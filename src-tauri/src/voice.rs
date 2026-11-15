//! Native capture and local wake detection. Activated PCM is sent to the hosted UI; never persisted.
use crate::{desktop, voice_protocol, workspace::WorkspaceState};
use base64::Engine;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;
use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig};
use std::{
    cell::Cell,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager, WebviewWindow};

const EVENT: &str = "cognuum-desktop-voice";
const IDLE: u8 = 0;
const HOLD: u8 = 1;
const RELEASE: u8 = 2;
const CANCEL: u8 = 3;
const FOLLOWUP: u8 = 4;
/// Window focus and URL are synchronous event-loop round trips. Blur, page
/// loads, account changes and window close already stop a session through
/// events, so the audio loop re-polls them as a bounded-staleness backstop
/// instead of for every buffer (~100/s). Command-bearing phases always re-check.
const NATIVE_RECHECK: Duration = Duration::from_millis(100);

#[derive(Default)]
pub struct VoiceState {
    next: AtomicU64,
    owner: Mutex<Option<String>>,
    session: Mutex<Option<Session>>,
    // Keep the models warm after opt-in. Holding this lock serializes capture
    // across windows; microphone/audio buffers are never kept in this cache.
    models: Arc<Mutex<Models>>,
}

#[derive(Default)]
struct Models {
    recognizer: Option<OnlineRecognizer>,
    /// Two keyword-spotting models, keyed by the short "Max" trigger setting.
    /// Reloading them on every refocus/re-enable cost a visible start delay.
    wake: Option<(bool, crate::voice_wake::WakeDetector)>,
}
struct Session {
    id: u64,
    label: String,
    stop: Arc<AtomicBool>,
    action: Arc<AtomicU8>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    #[serde(flatten)]
    detail: Option<Detail>,
    session: u64,
    sequence: u64,
    phase: &'static str,
    text: String,
}

#[derive(Clone, Serialize)]
struct InputSignal {
    level: f32,
    device: String,
}

#[derive(Clone, Serialize)]
#[serde(untagged)]
enum Detail {
    Input { input: InputSignal },
    Audio { audio: String },
    Activation { activation: &'static str },
}

pub fn stop(app: &tauri::AppHandle, label: Option<&str>) {
    let state = app.state::<VoiceState>();
    let mut session = state.session.lock().expect("voice session");
    if session
        .as_ref()
        .is_some_and(|s| label.is_none_or(|label| s.label == label))
    {
        if let Some(session) = session.take() {
            session.stop.store(true, Ordering::SeqCst);
        }
    }
}

struct Eligibility {
    window: WebviewWindow,
    generation: u64,
    owner: String,
    confirmed: Cell<Option<Instant>>,
}

impl Eligibility {
    /// In-process checks (generation, account) run every time; native window
    /// checks run when `fresh` or when the last confirmation is stale.
    fn check(&self, fresh: bool) -> bool {
        let state = self.window.state::<WorkspaceState>();
        if state.generation(self.window.label()) != self.generation
            || !state
                .auth
                .lock()
                .is_ok_and(|auth| auth.is_owner(&self.owner))
        {
            self.confirmed.set(None);
            return false;
        }
        if !fresh
            && self
                .confirmed
                .get()
                .is_some_and(|at| at.elapsed() < NATIVE_RECHECK)
        {
            return true;
        }
        let native =
            desktop::verify(&self.window).is_ok() && self.window.is_focused().unwrap_or(false);
        self.confirmed.set(native.then(Instant::now));
        native
    }
}

#[tauri::command]
pub fn desktop_voice_start(
    window: WebviewWindow,
    wake: bool,
    listen_word: bool,
    streaming: Option<bool>,
    take_ownership: Option<bool>,
) -> Result<u64, String> {
    desktop::verify(&window)?;
    if !window.is_focused().unwrap_or(false) {
        return Err("Focus Cognuum to use Max.".into());
    }
    let workspace = window.state::<WorkspaceState>();
    let owner = workspace
        .auth
        .lock()
        .map_err(|_| "Sign in first.")?
        .owner()
        .ok_or("Sign in first.")?;
    let generation = workspace.generation(window.label());
    let resources = window
        .app_handle()
        .path()
        .resource_dir()
        .map_err(|_| "Voice resources unavailable.")?;
    let state = window.state::<VoiceState>();
    let mut voice_owner = state.owner.lock().map_err(|_| "Voice unavailable.")?;
    {
        let owner = &mut voice_owner;
        if owner
            .as_deref()
            .is_some_and(|label| label != window.label())
            && !take_ownership.unwrap_or(false)
        {
            return Err("Max belongs to another window. Enable Max here to move it.".into());
        }
        **owner = Some(window.label().into());
    }
    // One owner for all windows; notify the old UI even when it is unfocused.
    let _ = window
        .app_handle()
        .emit("cognuum-voice-owner", window.label());
    stop(window.app_handle(), None);
    let id = state.next.fetch_add(1, Ordering::SeqCst) + 1;
    let models = state.models.clone();
    let stop_signal = Arc::new(AtomicBool::new(false));
    let action = Arc::new(AtomicU8::new(IDLE));
    *state.session.lock().map_err(|_| "Voice unavailable.")? = Some(Session {
        id,
        label: window.label().into(),
        stop: stop_signal.clone(),
        action: action.clone(),
    });
    drop(voice_owner);
    std::thread::spawn(move || {
        let eligibility = Eligibility {
            window,
            generation,
            owner,
            confirmed: Cell::new(None),
        };
        let mut sequence = 0;
        let mut emit = |phase, text: String, detail: Option<Detail>| {
            // A result can never cross a reload, account switch or focus change.
            // Streamed audio/meter frames rely on the event-driven stop signal
            // plus a bounded-staleness native check; every other phase re-checks.
            let fresh = !matches!(phase, "audio" | "level");
            if !stop_signal.load(Ordering::SeqCst) && eligibility.check(fresh) {
                sequence += 1;
                let _ = eligibility.window.emit(
                    EVENT,
                    Event {
                        session: id,
                        sequence,
                        detail,
                        phase,
                        text,
                    },
                );
            }
        };
        let result = (|| {
            let mut models = models.lock().map_err(|_| "Voice engine unavailable.")?;
            if stop_signal.load(Ordering::SeqCst) || !eligibility.check(true) {
                return Ok(());
            }
            let Models {
                recognizer: cached_recognizer,
                wake: cached_wake,
            } = &mut *models;
            if cached_recognizer.is_none() {
                *cached_recognizer = Some(recognizer(&resources.join("voice-model"))?);
            }
            let cloud = streaming.unwrap_or(false);
            let detector = if wake && cloud {
                match cached_wake {
                    Some((short, detector)) if *short == listen_word => detector.reset(),
                    _ => {
                        // Release the old models before loading the other variant.
                        *cached_wake = None;
                        *cached_wake = Some((
                            listen_word,
                            crate::voice_wake::WakeDetector::new(
                                &resources.join("wake-model"),
                                listen_word,
                            )?,
                        ));
                    }
                }
                cached_wake.as_mut().map(|(_, detector)| detector)
            } else {
                None
            };
            run(
                cached_recognizer.as_ref().expect("loaded recognizer"),
                detector,
                RunOptions {
                    wake,
                    short_wake: listen_word,
                    cloud,
                },
                &stop_signal,
                &action,
                || eligibility.check(false),
                &mut emit,
            )
        })();
        if let Err(error) = result {
            emit("error", error, None);
        }
    });
    Ok(id)
}

#[tauri::command]
pub fn desktop_voice_control(
    window: WebviewWindow,
    session: u64,
    action: String,
) -> Result<(), String> {
    desktop::verify(&window)?;
    if action == "stop" {
        let state = window.state::<VoiceState>();
        let mut current = state.session.lock().map_err(|_| "Voice unavailable.")?;
        if current
            .as_ref()
            .is_some_and(|s| s.id == session && s.label == window.label())
        {
            current.take().unwrap().stop.store(true, Ordering::SeqCst);
        }
        return Ok(());
    }
    if !window.is_focused().unwrap_or(false) {
        return Err("Focus Cognuum to use Max.".into());
    }
    let value = match action.as_str() {
        "hold" => HOLD,
        "release" => RELEASE,
        "cancel" => CANCEL,
        "followup" => FOLLOWUP,
        _ => return Err("Unknown voice action.".into()),
    };
    let state = window.state::<VoiceState>();
    let current = state.session.lock().map_err(|_| "Voice unavailable.")?;
    let current = current
        .as_ref()
        .filter(|s| s.id == session && s.label == window.label() && !s.stop.load(Ordering::SeqCst))
        .ok_or("Voice session ended.")?;
    // Do not lose a release that follows hold before the next audio callback.
    if value == RELEASE
        && current
            .action
            .compare_exchange(HOLD, CANCEL, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    {
        return Err("Max was not ready. Hold the shortcut again when Ready appears.".into());
    }
    current.action.store(value, Ordering::SeqCst);
    Ok(())
}

pub fn recognizer(path: &Path) -> Result<OnlineRecognizer, String> {
    OnlineRecognizer::create(&recognizer_config(path)?)
        .ok_or("Could not load local speech recognition.".into())
}

fn recognizer_config(path: &Path) -> Result<OnlineRecognizerConfig, String> {
    let file = |name: &str| -> Result<String, String> {
        let path = path.join(name);
        if !path.is_file() {
            return Err("Voice model is missing. Reinstall Cognuum.".into());
        }
        Ok(path.to_string_lossy().into_owned())
    };
    let mut config = OnlineRecognizerConfig::default();
    config.model_config.transducer.encoder =
        Some(file("encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx")?);
    config.model_config.transducer.decoder =
        Some(file("decoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx")?);
    config.model_config.transducer.joiner =
        Some(file("joiner-epoch-99-avg-1-chunk-16-left-128.int8.onnx")?);
    config.model_config.tokens = Some(file("tokens.txt")?);
    config.model_config.modeling_unit = Some("bpe".into());
    config.model_config.bpe_vocab = Some(file("bpe.vocab")?);
    config.model_config.num_threads = 2;
    config.model_config.provider = Some("cpu".into());
    config.decoding_method = Some("modified_beam_search".into());
    config.max_active_paths = 4;
    config.hotwords_buf = Some("HEY MAX\n".as_bytes().to_vec());
    config.hotwords_score = 4.0;
    config.enable_endpoint = true;
    config.rule1_min_trailing_silence = 2.4;
    config.rule2_min_trailing_silence = 0.35;
    config.rule3_min_utterance_length = 20.0;
    Ok(config)
}

fn microphone(
    tx: mpsc::SyncSender<Vec<f32>>,
    failed: Arc<AtomicBool>,
) -> Result<(cpal::Stream, i32, String), String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("No microphone found. Connect one and try again.")?;
    let name = device.name().unwrap_or_else(|_| "System microphone".into());
    let supported = device
        .default_input_config()
        .map_err(|_| "Allow microphone access to Cognuum in system settings.")?;
    let config = supported.config();
    let channels = config.channels as usize;
    let on_error = {
        let failed = failed.clone();
        move |_| {
            failed.store(true, Ordering::SeqCst);
        }
    };
    macro_rules! build {
        ($type:ty, $convert:expr) => {{
            let mut input = crate::voice_audio::InputChannel::default();
            device.build_input_stream(
                &config,
                move |data: &[$type], _| {
                    let mono = input.mono(data, channels, $convert);
                    // Bound memory and reject dropped audio rather than run a corrupt command.
                    if tx.try_send(mono).is_err() {
                        failed.store(true, Ordering::SeqCst);
                    }
                },
                on_error,
                None,
            )
        }};
    }
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => build!(f32, |s: &f32| *s),
        cpal::SampleFormat::I16 => build!(i16, |s: &i16| *s as f32 / 32768.0),
        cpal::SampleFormat::U16 => build!(u16, |s: &u16| (*s as f32 - 32768.0) / 32768.0),
        _ => {
            return Err(
                "This microphone format is not supported. Select another input device.".into(),
            )
        }
    }
    .map_err(|_| "Allow microphone access to Cognuum in system settings, then try again.")?;
    stream
        .play()
        .map_err(|_| "Could not start the microphone. Check system microphone access.")?;
    Ok((stream, config.sample_rate.0 as i32, name))
}

fn new_stream(recognizer: &OnlineRecognizer) -> sherpa_onnx::OnlineStream {
    let stream = recognizer.create_stream();
    // Supply left acoustic context immediately (no wall-clock wait). Without
    // it this streaming model clips initial syllables on hold-to-talk starts.
    stream.accept_waveform(16000, &[0.0; 16000]);
    while recognizer.is_ready(&stream) {
        recognizer.decode(&stream);
    }
    stream
}

struct RunOptions {
    wake: bool,
    short_wake: bool,
    cloud: bool,
}
/// `detector` is present only for wake + cloud sessions and arrives reset.
fn run(
    recognizer: &OnlineRecognizer,
    mut detector: Option<&mut crate::voice_wake::WakeDetector>,
    options: RunOptions,
    stop: &AtomicBool,
    action: &AtomicU8,
    eligible: impl Fn() -> bool,
    emit: &mut impl FnMut(&'static str, String, Option<Detail>),
) -> Result<(), String> {
    let RunOptions {
        wake,
        short_wake,
        cloud,
    } = options;
    let mut stream = new_stream(recognizer);
    let (tx, rx) = mpsc::sync_channel(32);
    let failed = Arc::new(AtomicBool::new(false));
    let mut mic = None;
    let mut held = false;
    let mut waiting: Option<Instant> = None;
    let mut started: Option<Instant> = None;
    let mut last_partial = String::new();
    let mut segment_started = Instant::now();
    let mut gain = crate::voice_audio::InputGain::default();
    let mut last_meter = Instant::now();
    let mut resampler = None;
    let mut buffered = Vec::<f32>::new();
    let mut sent = false;
    let mut cloud_samples = 0;
    let mut outgoing = Vec::<f32>::new();
    let ready = if wake { "armed" } else { "ready" };
    if stop.load(Ordering::SeqCst) || !eligible() {
        return Ok(());
    }
    if wake {
        mic = Some(microphone(tx.clone(), failed.clone())?);
    }
    emit(ready, String::new(), None);
    loop {
        if stop.load(Ordering::SeqCst) || !eligible() {
            break;
        }
        let next = action.swap(IDLE, Ordering::SeqCst);
        if next == HOLD || next == FOLLOWUP {
            if mic.is_none() {
                mic = Some(microphone(tx.clone(), failed.clone())?);
            }
            while rx.try_recv().is_ok() {}
            stream = new_stream(recognizer);
            if let Some(detector) = &mut detector {
                detector.reset();
            }
            resampler = None;
            buffered.clear();
            sent = false;
            cloud_samples = 0;
            outgoing.clear();
            held = next == HOLD;
            waiting = if next == FOLLOWUP {
                Some(Instant::now())
            } else {
                None
            };
            started = Some(Instant::now());
            last_partial.clear();
            emit(
                "listening",
                String::new(),
                Some(Detail::Activation {
                    activation: if next == FOLLOWUP { "followup" } else { "hold" },
                }),
            );
        }
        if next == CANCEL
            || started.is_some_and(|t| t.elapsed() > Duration::from_secs(20))
            || waiting.is_some_and(|t| t.elapsed() > Duration::from_secs(5))
        {
            held = false;
            waiting = None;
            started = None;
            last_partial.clear();
            buffered.clear();
            sent = false;
            cloud_samples = 0;
            outgoing.clear();
            resampler = None;
            stream = new_stream(recognizer);
            if let Some(detector) = &mut detector {
                detector.reset();
            }
            while rx.try_recv().is_ok() {}
            if !wake {
                mic = None;
            }
            emit("cancelled", String::new(), None);
            emit(ready, String::new(), None);
        }
        if failed.load(Ordering::SeqCst) {
            return Err(
                "Microphone interrupted. Check your input device, then enable Max again.".into(),
            );
        }
        let mut detected = false;
        if let Some((_, rate, device)) = mic.as_ref() {
            let mut received = rx
                .recv_timeout(Duration::from_millis(20))
                .ok()
                .into_iter()
                .collect::<Vec<_>>();
            if next == RELEASE && held {
                received.extend(rx.try_iter());
            }
            for mut samples in received {
                let level = gain.process(&mut samples, *rate);
                if last_meter.elapsed() >= Duration::from_millis(250) {
                    emit(
                        "level",
                        String::new(),
                        Some(Detail::Input {
                            input: InputSignal {
                                level,
                                device: device.clone(),
                            },
                        }),
                    );
                    last_meter = Instant::now();
                }
                stream.accept_waveform(*rate, &samples);
                if let Some(detector) = &mut detector {
                    detected |= detector.accept(*rate, &samples);
                }
                if cloud {
                    if resampler.is_none() {
                        resampler = sherpa_onnx::LinearResampler::create(*rate, 24000);
                    }
                    let converted = resampler
                        .as_ref()
                        .ok_or("Could not prepare microphone audio.")?
                        .resample(&samples, false);
                    if sent {
                        cloud_samples += converted.len();
                        if cloud_samples > 24000 * 20 {
                            return Err(
                                "Voice request is too long. Please try a shorter request.".into()
                            );
                        }
                        outgoing.extend(converted);
                        while outgoing.len() >= 2400 {
                            emit_pcm(&outgoing[..2400], emit);
                            outgoing.drain(..2400);
                        }
                    } else {
                        buffered.extend(converted);
                        // At most 2 seconds of local pre-roll; no idle audio leaves the process.
                        if buffered.len() > 48000 {
                            buffered.drain(..buffered.len() - 48000);
                        }
                    }
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(20));
        }
        if next == RELEASE && held {
            stream.accept_waveform(16000, &[0.0; 16000]);
            stream.input_finished();
        }
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }
        let text = recognizer
            .get_result(&stream)
            .map(|r| r.text)
            .unwrap_or_default();
        if waiting.is_some() && !text.trim().is_empty() {
            waiting = None;
        }
        let spoken = if held || waiting.is_some() || (cloud && sent) {
            Some(text.clone())
        } else {
            voice_protocol::after_wake(&text, short_wake)
        };
        if spoken.is_some() || (cloud && detected && wake && started.is_none()) {
            if started.is_none() {
                started = Some(Instant::now());
                emit(
                    "listening",
                    String::new(),
                    Some(Detail::Activation { activation: "wake" }),
                );
            }
            if cloud && !sent {
                sent = true;
                cloud_samples = buffered.len();
                emit_pcm(&buffered, emit);
                buffered.clear();
            }
            if !cloud {
                if let Some(ref spoken) = spoken {
                    if spoken != &last_partial {
                        emit("partial", spoken.clone(), None);
                        last_partial = spoken.clone();
                    }
                }
            }
        }
        if (next == RELEASE && held) || (!held && recognizer.is_endpoint(&stream)) {
            if cloud && sent {
                emit_pcm(&outgoing, emit);
                outgoing.clear();
                emit("commit", String::new(), None);
                sent = false;
                cloud_samples = 0;
                outgoing.clear();
                waiting = None;
                started = None;
                emit(ready, String::new(), None);
            } else if !cloud {
                if let Some(spoken) = spoken {
                    if !held && spoken.is_empty() {
                        if waiting.is_none() {
                            waiting = Some(Instant::now());
                        }
                    } else {
                        if let Some(command) = voice_protocol::command(&spoken) {
                            emit("final", command, None);
                        }
                        waiting = None;
                        started = None;
                        emit(ready, String::new(), None);
                    }
                } else {
                    started = None;
                    emit(ready, String::new(), None);
                }
            }
            held = false;
            stream = new_stream(recognizer);
            last_partial.clear();
            segment_started = Instant::now();
            if !wake {
                mic = None;
            }
        }
        if !held
            && waiting.is_none()
            && started.is_none()
            && segment_started.elapsed() > Duration::from_secs(20)
        {
            stream = new_stream(recognizer);
            segment_started = Instant::now();
        }
    }
    Ok(())
}

fn emit_pcm(samples: &[f32], emit: &mut impl FnMut(&'static str, String, Option<Detail>)) {
    for chunk in samples.chunks(2400) {
        let bytes: Vec<u8> = chunk
            .iter()
            .flat_map(|v| ((*v * 32767.0).clamp(-32768.0, 32767.0) as i16).to_le_bytes())
            .collect();
        if !bytes.is_empty() {
            emit(
                "audio",
                String::new(),
                Some(Detail::Audio {
                    audio: base64::engine::general_purpose::STANDARD.encode(bytes),
                }),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn speech_regressions_preserve_wake_commands_and_short_hold_utterances() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let recognizer = recognizer(&root.join("voice-model")).unwrap();
        let mut failures = Vec::new();
        for (file, expected, wake) in [
            ("hey-max-us.wav", Some("show apple over five years"), true),
            ("hey-max-uk.wav", Some("open analysis"), true),
            ("hey-max-india.wav", Some("open settings"), true),
            ("listen.wav", None, true),
            ("maximum.wav", None, true),
            ("conversation.wav", None, true),
            ("hold.wav", Some("load apple"), false),
            ("no-wake.wav", None, true),
        ] {
            let wave =
                sherpa_onnx::Wave::read(root.join("../tests/voice").join(file).to_str().unwrap())
                    .unwrap();
            assert!(wave.num_samples() > 0, "Empty fixture: {file}");
            let stream = new_stream(&recognizer);
            for chunk in wave.samples().chunks((wave.sample_rate() / 10) as usize) {
                stream.accept_waveform(wave.sample_rate(), chunk);
                while recognizer.is_ready(&stream) {
                    recognizer.decode(&stream);
                }
            }
            stream.accept_waveform(16000, &[0.0; 16000]);
            stream.input_finished();
            while recognizer.is_ready(&stream) {
                recognizer.decode(&stream);
            }
            let text = recognizer.get_result(&stream).unwrap().text.to_lowercase();
            let command = if wake {
                voice_protocol::after_wake(&text, true)
            } else {
                voice_protocol::command(&text)
            };
            if command.as_deref() != expected {
                failures.push(format!(
                    "{file}: {text} -> {command:?}; expected {expected:?}"
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        let silence = new_stream(&recognizer);
        silence.accept_waveform(16000, &[0.0; 32000]);
        silence.input_finished();
        while recognizer.is_ready(&silence) {
            recognizer.decode(&silence);
        }
        assert_eq!(
            voice_protocol::after_wake(&recognizer.get_result(&silence).unwrap().text, true),
            None
        );
    }

    /// Run explicitly with a local, consented audio fixture; never opens a mic.
    #[test]
    #[ignore = "requires COGNUUM_VOICE_FIXTURE WAV; recognition benchmark"]
    fn speech_fixture() {
        let path = std::env::var("COGNUUM_VOICE_FIXTURE").expect("WAV fixture path");
        let wave = sherpa_onnx::Wave::read(&path).expect("PCM WAV");
        let start = Instant::now();
        let model = std::env::var("COGNUUM_VOICE_MODEL")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("voice-model"));
        let recognizer = recognizer(&model).unwrap();
        let loaded = start.elapsed();
        let stream = new_stream(&recognizer);
        for chunk in wave.samples().chunks((wave.sample_rate() / 10) as usize) {
            stream.accept_waveform(wave.sample_rate(), chunk);
            while recognizer.is_ready(&stream) {
                recognizer.decode(&stream);
            }
        }
        let flush = Instant::now();
        stream.accept_waveform(16000, &[0.0; 16000]);
        stream.input_finished();
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }
        let text = recognizer.get_result(&stream).unwrap().text;
        println!(
            "model_load={loaded:?} compute={:?} release_flush={:?} transcript={text}",
            start.elapsed(),
            flush.elapsed()
        );
        let expected = std::env::var("COGNUUM_VOICE_EXPECT").expect("expected phrase");
        assert_eq!(text.trim().to_lowercase(), expected.to_lowercase());
    }
}
