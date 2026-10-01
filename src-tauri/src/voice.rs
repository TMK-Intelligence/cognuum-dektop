//! Native microphone + local streaming ASR. No network/audio persistence.
use crate::{desktop, voice_protocol, workspace::WorkspaceState};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;
use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig};
use std::{
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

#[derive(Default)]
pub struct VoiceState {
    next: AtomicU64,
    session: Mutex<Option<Session>>,
    // Keep the model warm after opt-in. Holding this lock serializes capture
    // across windows; microphone/audio buffers are never kept in this cache.
    recognizer: Arc<Mutex<Option<OnlineRecognizer>>>,
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
    session: u64,
    sequence: u64,
    phase: &'static str,
    text: String,
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

fn eligible(window: &WebviewWindow, generation: u64, owner: &str) -> bool {
    desktop::verify(window).is_ok()
        && window.is_focused().unwrap_or(false)
        && window.state::<WorkspaceState>().generation(window.label()) == generation
        && window
            .state::<WorkspaceState>()
            .auth
            .lock()
            .ok()
            .and_then(|auth| auth.owner())
            .as_deref()
            == Some(owner)
}

#[tauri::command]
pub fn desktop_voice_start(
    window: WebviewWindow,
    wake: bool,
    listen_word: bool,
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
    stop(window.app_handle(), None);
    let state = window.state::<VoiceState>();
    let id = state.next.fetch_add(1, Ordering::SeqCst) + 1;
    let model = state.recognizer.clone();
    let stop_signal = Arc::new(AtomicBool::new(false));
    let action = Arc::new(AtomicU8::new(IDLE));
    *state.session.lock().map_err(|_| "Voice unavailable.")? = Some(Session {
        id,
        label: window.label().into(),
        stop: stop_signal.clone(),
        action: action.clone(),
    });
    std::thread::spawn(move || {
        let mut sequence = 0;
        let mut emit = |phase, text: String| {
            // A result can never cross a reload, account switch or focus change.
            if !stop_signal.load(Ordering::SeqCst) && eligible(&window, generation, &owner) {
                sequence += 1;
                let _ = window.emit(
                    EVENT,
                    Event {
                        session: id,
                        sequence,
                        phase,
                        text,
                    },
                );
            }
        };
        let result = (|| {
            let mut model = model.lock().map_err(|_| "Voice engine unavailable.")?;
            if stop_signal.load(Ordering::SeqCst) || !eligible(&window, generation, &owner) {
                return Ok(());
            }
            if model.is_none() {
                *model = Some(recognizer(&resources.join("voice-model"))?);
            }
            run(
                model.as_ref().unwrap(),
                wake,
                listen_word,
                &stop_signal,
                &action,
                || eligible(&window, generation, &owner),
                &mut emit,
            )
        })();
        if let Err(error) = result {
            emit("error", error);
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
    config.rule2_min_trailing_silence = 0.65;
    config.rule3_min_utterance_length = 20.0;
    OnlineRecognizer::create(&config).ok_or("Could not load local speech recognition.".into())
}

fn microphone(
    tx: mpsc::SyncSender<Vec<f32>>,
    failed: Arc<AtomicBool>,
) -> Result<(cpal::Stream, i32), String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("No microphone found. Connect one and try again.")?;
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
        ($type:ty, $convert:expr) => {
            device.build_input_stream(
                &config,
                move |data: &[$type], _| {
                    let mono = data
                        .chunks(channels)
                        .map(|frame| frame.iter().map($convert).sum::<f32>() / channels as f32)
                        .collect();
                    // Bound memory and reject dropped audio rather than run a corrupt command.
                    if tx.try_send(mono).is_err() {
                        failed.store(true, Ordering::SeqCst);
                    }
                },
                on_error,
                None,
            )
        };
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
    Ok((stream, config.sample_rate.0 as i32))
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

fn run(
    recognizer: &OnlineRecognizer,
    wake: bool,
    listen: bool,
    stop: &AtomicBool,
    action: &AtomicU8,
    eligible: impl Fn() -> bool,
    emit: &mut impl FnMut(&'static str, String),
) -> Result<(), String> {
    let mut stream = new_stream(recognizer);
    let (tx, rx) = mpsc::sync_channel(32);
    let failed = Arc::new(AtomicBool::new(false));
    let mut mic = None;
    let mut held = false;
    let mut waiting: Option<Instant> = None;
    let mut started: Option<Instant> = None;
    let mut last_partial = String::new();
    let mut segment_started = Instant::now();
    let ready = if wake { "armed" } else { "ready" };
    if stop.load(Ordering::SeqCst) || !eligible() {
        return Ok(());
    }
    if wake {
        mic = Some(microphone(tx.clone(), failed.clone())?);
    }
    emit(ready, String::new());
    loop {
        if stop.load(Ordering::SeqCst) || !eligible() {
            break;
        }
        let next = action.swap(IDLE, Ordering::SeqCst);
        if next == HOLD {
            if mic.is_none() {
                mic = Some(microphone(tx.clone(), failed.clone())?);
            }
            while rx.try_recv().is_ok() {}
            stream = new_stream(recognizer);
            held = true;
            waiting = None;
            started = Some(Instant::now());
            last_partial.clear();
            emit("listening", String::new());
        }
        if next == CANCEL
            || started.is_some_and(|t| t.elapsed() > Duration::from_secs(20))
            || waiting.is_some_and(|t| t.elapsed() > Duration::from_secs(5))
        {
            held = false;
            waiting = None;
            started = None;
            last_partial.clear();
            stream = new_stream(recognizer);
            while rx.try_recv().is_ok() {}
            if !wake {
                mic = None;
            }
            emit(ready, String::new());
        }
        if failed.load(Ordering::SeqCst) {
            return Err(
                "Microphone interrupted. Check your input device, then enable Max again.".into(),
            );
        }
        if let Some((_, rate)) = mic.as_ref() {
            if let Ok(samples) = rx.recv_timeout(Duration::from_millis(20)) {
                stream.accept_waveform(*rate, &samples);
            }
        } else {
            std::thread::sleep(Duration::from_millis(20));
        }
        if next == RELEASE && held {
            // Flush model look-ahead without an extra silence/VAD wait on key-up.
            if let Some((_, rate)) = mic.as_ref() {
                while let Ok(samples) = rx.try_recv() {
                    stream.accept_waveform(*rate, &samples);
                }
            }
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
        let spoken = if held || waiting.is_some() {
            Some(text.clone())
        } else {
            voice_protocol::after_wake(&text, listen)
        };
        if let Some(ref spoken) = spoken {
            if started.is_none() {
                started = Some(Instant::now());
                emit("listening", String::new());
            }
            if spoken != &last_partial {
                emit("partial", spoken.clone());
                last_partial = spoken.clone();
            }
        }
        if (next == RELEASE && held) || (!held && recognizer.is_endpoint(&stream)) {
            if let Some(spoken) = spoken {
                if !held && spoken.is_empty() {
                    if waiting.is_none() {
                        waiting = Some(Instant::now());
                    }
                } else {
                    if let Some(command) = voice_protocol::command(&spoken) {
                        emit("final", command);
                    }
                    waiting = None;
                    started = None;
                    emit(ready, String::new());
                }
            } else {
                started = None;
                emit(ready, String::new());
            }
            held = false;
            stream = new_stream(recognizer);
            last_partial.clear();
            segment_started = Instant::now();
            if !wake {
                mic = None;
            }
        }
        // Bounded idle decoder history, including silence / background noise.
        if !held
            && waiting.is_none()
            && started.is_none()
            && segment_started.elapsed() > Duration::from_secs(20)
        {
            stream = new_stream(recognizer);
            segment_started = Instant::now();
        }
    }
    // Dropping cpal::Stream closes the microphone; audio queues die here.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn speech_regressions_preserve_wake_commands_and_short_hold_utterances() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let recognizer = recognizer(&root.join("voice-model")).unwrap();
        for (file, expected, wake) in [
            ("hey-max-us.wav", Some("show apple over five years"), true),
            ("hey-max-uk.wav", Some("open analysis"), true),
            ("hey-max-india.wav", Some("open settings"), true),
            ("listen.wav", Some("load apple"), true),
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
            assert_eq!(command.as_deref(), expected, "{file}: {text}");
        }
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
