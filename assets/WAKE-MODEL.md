# Max local phonetic wake model

Model: sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20, chunk 8 int8 and chunk 16 fp32.
Two acoustic resolutions run locally; either can propose a wake capture. Each
uses two bounded streams: one also refreshes on a new speech onset after
quiet, while the other preserves context across soft speech and room noise.
Staggered six-second refreshes avoid cutting both streams through a phrase. Two models and four bounded streams add no
network calls. Candidates still require anchored final cloud confirmation.
Original publisher: pkufool; license: Apache License 2.0 (see LICENSE.txt).
Publisher model card: https://modelscope.cn/models/pkufool/icefall-kws-zipformer-zh-en-3M-2025-12-20
Distribution: https://github.com/k2-fsa/sherpa-onnx/releases/tag/kws-models
Usage: https://k2-fsa.github.io/sherpa/onnx/kws/pretrained_models/index.html

English phonemes from the publisher's en.phone lexicon:
HEY = HH EY1; MAX = M AE1 K S. Wake hypotheses authorize capture only.
Final transcription still must confirm the complete, anchored wake phrase.
Archive and every bundled model file are SHA-256 pinned by the build scripts.
