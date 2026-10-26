# Third-party notices

## Pretendard

Grove bundles the unmodified Regular, Medium, and SemiBold fonts from Pretendard 1.3.9.
Copyright (c) 2021, Kil Hyung-jin. Distributed under the SIL Open Font License 1.1.
The complete license is included with the fonts in the app resource bundle.
Source: https://github.com/orioncactus/pretendard/tree/v1.3.9

## Native local beta workers

- MOSS worker: MLXAudio Swift 0.1.3 and MLX Swift 0.31.6. Dependency resolution and
  collected upstream license notices are included under `Contents/Resources/Licenses/Moss/`.
  Sources: https://github.com/Blaizzy/mlx-audio-swift and https://github.com/ml-explore/mlx-swift.
- Nemotron 3: official NeMo-Speech.cpp 0.2.0 Metal helper. Runtime notices are included
  under `Contents/Resources/Licenses/Nemotron/`. Source: https://github.com/NVIDIA/NeMo-Speech.cpp.
  The separately downloaded Q8 model is distributed under OpenMDW 1.1:
  https://huggingface.co/nvidia/Nemotron-3-Diarization.

Model weights remain in machine-local caches, not the app or public repository.
Signing and notarization status must be established for each packaged build; these
notices do not certify a public binary distribution.

## Nero native speech runtime

The rewritten desktop app shares the official NVIDIA NeMo-Speech.cpp 0.2.0 runtime
between Nemotron 3.5 speech recognition and optional Nemotron 3 speaker analysis.
The runtime licenses are packaged under
`Contents/Resources/native/diarization/<backend>/share/licenses/` on macOS and the
equivalent resource directory on Windows. Model weights are separate downloads,
with repository revisions and licenses recorded in their model manifests.
Runtime source: https://github.com/NVIDIA/NeMo-Speech.cpp.

Optional team voice enrollment downloads the pinned FBANK / WeSpeaker Core ML models from
https://huggingface.co/FluidInference/speaker-diarization-coreml (metadata: CC BY 4.0) on
request. Voice features are stored encrypted; automatic naming remains disabled.
