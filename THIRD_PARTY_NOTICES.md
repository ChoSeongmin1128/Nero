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

## Portable WAV preparation

Windows WAV preparation uses rubato 0.16.2 (MIT) and its FFT/numeric dependencies.
The complete pinned dependency notices are included under
`Contents/Resources/native/licenses/portable-wav/` (equivalent resources on Windows).
Source: https://github.com/HEnquist/rubato/tree/v0.16.2 .

## Whisper transcription

The optional Whisper transcription engine uses whisper.cpp b5454, commit
`d1be6fde11ac6e0407606b4e42fe72d34add8037`, under the MIT License.
Copyright (c) 2023-2026 The ggml authors. The complete runtime license is included
at `native/whisper/share/licenses/whisper.cpp/LICENSE` in app resources.
[Runtime license](https://github.com/ggml-org/whisper.cpp/blob/d1be6fde11ac6e0407606b4e42fe72d34add8037/LICENSE).

The Windows Vulkan build also uses Khronos Vulkan-Headers commit
`2cd90f9d20df57eac214c148f3aed885372ddcfe` (Apache-2.0 OR MIT) and
SPIRV-Headers commit `2a611a970fdbc41ac2e3e328802aed9985352dca` (MIT for
the included SPIR-V headers). Their complete license texts are packaged beneath
`native/whisper/share/licenses/Vulkan-Headers/` and `SPIRV-Headers/`.
LunarG Vulkan SDK 1.4.321.0, CMake and Ninja are build-only tools; the SDK is not
distributed or installed on users' systems. Runtime Vulkan loading uses the
graphics driver's system loader, and CPU operation does not require Vulkan.

The separately downloaded large-v3-turbo Q5_0 model is pinned in
`config/models/whisper-turbo.json`. Its conversion repository identifies the model
as MIT-licensed. Model weights are not bundled with the application.
[Model card](https://huggingface.co/ggerganov/whisper.cpp/blob/5359861c739e955e79d9a303bcbc70fb988958b1/README.md).
