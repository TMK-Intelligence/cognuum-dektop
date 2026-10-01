//! Dedicated local wake detector. Its hypotheses activate capture, never execute a command.
use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig, OnlineStream};
use std::path::Path;
pub fn detector(path: &Path, short: bool) -> Result<KeywordSpotter, String> {
    let file = |name: &str| path.join(name).to_string_lossy().into_owned();
    let mut c = KeywordSpotterConfig::default();
    c.model_config.transducer.encoder =
        Some(file("encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx"));
    c.model_config.transducer.decoder = Some(file("decoder-epoch-12-avg-2-chunk-16-left-64.onnx"));
    c.model_config.transducer.joiner =
        Some(file("joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx"));
    c.model_config.tokens = Some(file("tokens.txt"));
    c.model_config.num_threads = 2;
    c.keywords_buf = Some(
        if short {
            "▁HE Y ▁MA X @hey-max\n▁MA X @max\n"
        } else {
            "▁HE Y ▁MA X @hey-max\n"
        }
        .into(),
    );
    c.num_trailing_blanks = 1;
    c.keywords_score = 2.0;
    c.keywords_threshold = 0.25;
    KeywordSpotter::create(&c).ok_or("Wake model unavailable. Reinstall Cognuum.".into())
}
pub fn stream(kws: &KeywordSpotter) -> OnlineStream {
    let stream = kws.create_stream();
    stream.accept_waveform(16000, &[0.0; 16000]);
    stream
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_hey_max_activates_without_a_following_command() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let kws = detector(&root.join("wake-model"), false).unwrap();
        let wav = sherpa_onnx::Wave::read(
            root.join("../tests/voice/hey-max-alone.wav")
                .to_str()
                .unwrap(),
        )
        .unwrap();
        let stream = stream(&kws);
        let mut found = false;
        for chunk in wav.samples().chunks(1600).chain([&[0.0; 16000][..]]) {
            stream.accept_waveform(wav.sample_rate(), chunk);
            while kws.is_ready(&stream) {
                kws.decode(&stream);
                found |= kws
                    .get_result(&stream)
                    .is_some_and(|r| r.keyword == "hey-max");
            }
        }
        assert!(found);
    }
}
