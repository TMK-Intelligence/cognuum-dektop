# Local speech regression fixtures

Synthetic speech generated for this feature with macOS `say` and converted to
16 kHz mono PCM using `afconvert`. No person or microphone was recorded.

| File | Synthetic voice | Phrase |
| --- | --- | --- |
| hey-max-us.wav | Samantha (US English) | Hey Max, show Apple over five years. |
| hey-max-uk.wav | Daniel (UK English) | Hey Max. Open analysis. |
| hey-max-india.wav | Aman (Indian English) | Hey Max. Open settings. |
| listen.wav | Samantha | Listen. Load Apple. |
| hold.wav | Samantha | Load Apple. |
| no-wake.wav | Samantha | The hay market is closed. Show Apple over five years. |

The live recognizer and activation parser run against these fixtures in the
normal Rust suite. They caught three defects during implementation: lost initial
syllables without acoustic context, clipped final words with too little tail
padding, and the model's phonetic spelling “HAEMAKS” for “Hey Max.” The parser
accepts only a small whole-token spelling set; it never uses fuzzy substring
matching. Silence and the similar-sounding negative phrase must not activate.

These checks are regression coverage, not an accent-accuracy certification.
Real microphones, varied speakers, room noise and playback need release testing.
