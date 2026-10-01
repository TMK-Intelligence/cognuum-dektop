//! Dedicated local wake detector. Its hypotheses activate capture, never execute a command.
use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig, OnlineStream};
use std::path::Path;
pub fn detector(path: &Path, short: bool, accurate: bool) -> Result<KeywordSpotter, String> {
    let file = |name: &str| path.join(name).to_string_lossy().into_owned();
    let mut c = KeywordSpotterConfig::default();
    c.model_config.transducer.encoder = Some(file(if accurate {
        "encoder-epoch-13-avg-2-chunk-16-left-64.onnx"
    } else {
        "encoder-epoch-13-avg-2-chunk-8-left-64.int8.onnx"
    }));
    c.model_config.transducer.decoder = Some(file(if accurate {
        "decoder-epoch-13-avg-2-chunk-16-left-64.onnx"
    } else {
        "decoder-epoch-13-avg-2-chunk-8-left-64.onnx"
    }));
    c.model_config.transducer.joiner = Some(file(if accurate {
        "joiner-epoch-13-avg-2-chunk-16-left-64.onnx"
    } else {
        "joiner-epoch-13-avg-2-chunk-8-left-64.int8.onnx"
    }));
    c.model_config.tokens = Some(file("tokens.txt"));
    c.model_config.num_threads = 2;
    c.keywords_buf = Some(
        if short {
            "HH EY1 M AE1 K S @hey-max\nM AE1 K S @max\n"
        } else {
            "HH EY1 M AE1 K S @hey-max\n"
        }
        .into(),
    );
    c.num_trailing_blanks = 1;
    c.keywords_score = 2.0;
    c.keywords_threshold = 0.25;
    c.max_active_paths = 8;
    KeywordSpotter::create(&c).ok_or("Wake model unavailable. Reinstall Cognuum.".into())
}
pub fn stream(kws: &KeywordSpotter) -> OnlineStream {
    let stream = kws.create_stream();
    stream.accept_waveform(16000, &[0.0; 16000]);
    while kws.is_ready(&stream) {
        kws.decode(&stream);
    }
    stream
}
/// Overlapping bounded acoustic windows keep wake detection independent from
/// ASR endpoints. Staggered refreshes always leave one window hearing the whole
/// phrase, including when it starts during a refresh or after a long silence.
pub struct WakeDetector {
    streams: [OnlineStream; 4],
    kws: [KeywordSpotter; 2],
    ages: [f64; 4],
    cooldown: f64,
    rate: i32,
    quiet_seconds: f64,
}
impl WakeDetector {
    pub fn new(path: &Path, short: bool) -> Result<Self, String> {
        let kws = [detector(path, short, false)?, detector(path, short, true)?];
        let streams = [
            stream(&kws[0]),
            stream(&kws[0]),
            stream(&kws[1]),
            stream(&kws[1]),
        ];
        Ok(Self {
            streams,
            kws,
            ages: [0.0, -3.0, 0.0, -3.0],
            cooldown: 0.0,
            rate: 16000,
            quiet_seconds: 0.0,
        })
    }
    pub fn reset(&mut self) {
        self.streams = [
            stream(&self.kws[0]),
            stream(&self.kws[0]),
            stream(&self.kws[1]),
            stream(&self.kws[1]),
        ];
        self.ages = [0.0, -3.0, 0.0, -3.0];
        self.cooldown = 0.0;
        self.quiet_seconds = 0.0;
    }
    pub fn accept(&mut self, rate: i32, samples: &[f32]) -> bool {
        if rate <= 0 || samples.is_empty() {
            return false;
        }
        if rate != self.rate {
            self.reset();
            self.rate = rate;
        }
        let seconds = samples.len() as f64 / rate as f64;
        self.cooldown = (self.cooldown - seconds).max(0.0);
        let rms = (samples.iter().map(|sample| sample * sample).sum::<f32>()
            / samples.len() as f32)
            .sqrt();
        // A fresh onset lane avoids conditioning a short wake word on minutes
        // of silence. The independent rolling lane remains live for soft speech
        // and noisy rooms where no clean silence boundary exists.
        let onset = rms > 0.003 && self.quiet_seconds >= 0.4;
        if rms <= 0.003 {
            self.quiet_seconds += seconds;
        } else {
            self.quiet_seconds = 0.0;
        }
        let mut found = false;
        for (index, decoder) in self.streams.iter_mut().enumerate() {
            let kws = &self.kws[index / 2];
            if self.ages[index] >= 6.0 || (onset && index % 2 == 0) {
                *decoder = stream(kws);
                self.ages[index] = 0.0;
            }
            decoder.accept_waveform(rate, samples);
            self.ages[index] += seconds;
            while kws.is_ready(decoder) {
                kws.decode(decoder);
                if kws
                    .get_result(decoder)
                    .is_some_and(|r| !r.keyword.is_empty())
                {
                    found = true;
                    kws.reset(decoder);
                }
            }
        }
        if found && self.cooldown == 0.0 {
            self.cooldown = 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn continuous_quiet_wake_survives_silence_and_repeated_commands() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut wake = WakeDetector::new(&root.join("wake-model"), true).unwrap();
        let mut gain = crate::voice_audio::InputGain::default();
        let mut misses = Vec::new();
        // Real streaming chunks, multiple utterances in the SAME session, across
        // the bounded stream refresh. These are synthetic regressions, not an
        // accent or microphone accuracy claim.
        for repetition in 0..4 {
            for name in [
                "hey-max-alone.wav",
                "hey-max-us.wav",
                "hey-max-uk.wav",
                "hey-max-india.wav",
                "hey-max-au.wav",
                "hey-max-fr.wav",
                "max-us.wav",
                "max-uk.wav",
                "max-fr.wav",
            ] {
                let wav = sherpa_onnx::Wave::read(
                    root.join("../tests/voice").join(name).to_str().unwrap(),
                )
                .unwrap();
                let rate = wav.sample_rate();
                for _ in 0..(17 + repetition) {
                    wake.accept(rate, &vec![0.0; rate as usize / 10]);
                }
                let scale = if repetition % 2 == 0 { 1.0 } else { 0.125 };
                let samples: Vec<_> = wav
                    .samples()
                    .iter()
                    .map(|v| v * scale)
                    .chain(std::iter::repeat_n(0.0, rate as usize))
                    .collect();
                let mut detected = false;
                for chunk in samples.chunks(512) {
                    let mut conditioned = chunk.to_vec();
                    gain.process(&mut conditioned, rate);
                    detected |= wake.accept(rate, &conditioned);
                }
                if !detected {
                    misses.push(format!("{repetition} {name}"));
                }
            }
        }
        assert!(misses.is_empty(), "Missed: {misses:?}");
    }
    #[test]
    fn silence_and_non_wake_speech_do_not_activate() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut wake = WakeDetector::new(&root.join("wake-model"), true).unwrap();
        for _ in 0..650 {
            assert!(!wake.accept(16000, &[0.0; 1600]));
        }
        for name in ["no-wake.wav", "hold.wav", "listen.wav"] {
            wake.reset();
            let wav =
                sherpa_onnx::Wave::read(root.join("../tests/voice").join(name).to_str().unwrap())
                    .unwrap();
            for chunk in wav.samples().chunks(512).chain([&[0.0; 16000][..]]) {
                assert!(!wake.accept(wav.sample_rate(), chunk), "{name}");
            }
        }
    }
}
