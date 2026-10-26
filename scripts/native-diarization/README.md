# Shared native speech worker

This build-only preparation stages official NVIDIA NeMo-Speech.cpp v0.2.0
release archives for both Nemotron ASR and speaker analysis. `runtime.json` pins the upstream commit, archive sizes and
SHA256 values. It never downloads model weights or reads a user library.

```sh
python3 scripts/native-diarization/prepare.py --target macos-arm64
python3 scripts/native-diarization/prepare.py --verify-staged apps/desktop/.generated/native/diarization --target macos-arm64 --execute-runtime
```

On native Windows builds use `--target windows-x64`. Packaging includes Vulkan
and CPU executables with app-local Microsoft C++ runtime DLLs, following the
summary worker's redistributable discovery. macOS packages the Metal archive,
whose executable also supports CPU operation. The original `bin` / `lib` layout
and third-party licenses are retained, while dylib aliases become ordinary files
for signing. The existing Tauri `native/` resource mapping includes this output.

Existing output directories are never replaced. The generated receipt records
archive verification and pre-signing file hashes. No compiler, Python, separate
CLI, GPU SDK or model download tool is required on an end user's machine.

The desktop build verifies staged files against that receipt, rejecting linked,
unexpected or changed files and a mismatched target. An absent optional package
leaves recognition unavailable; it does not make a MOSS package a replacement.
macOS signing checks the original hashes before signing and preserves separate
post-signing hashes. The native resource mapping includes this directory once,
so recognition and speaker analysis share one executable/library package.

ASR uses `transcribe --stream` with the verified local Nemotron 3.5 Q8 GGUF,
`ko-KR`, concurrency 1, endpointing disabled, no dynamic batching/warmup and
RNNT right context 13. The runtime consumes the complete supplied WAV through
one stateful stream and calls `finish()` after EOF. It emits word timestamps
without speaker labels. CLI `duration` is rounded recognition metadata, not a
sample-accurate count of audio consumed or proof of complete spoken coverage.
The adapter must retain the original raw result and separate execution evidence.
GPU/CPU selection and failure handling belong to the native app adapter.

The native adapter sends one complete audio file to one stateful diarization
invocation with `v3-offline`, pinned runtime segmentation defaults and disabled
dynamic batching. Custom segmentation overrides are deliberately absent: this runtime
freezes long-recording prefixes with library defaults during compaction, so a different
tail configuration would describe a mixed-policy result. The request receipt records
the actual defaults. It does not use the short-recording full-attention `--offline` option.
The model has at most eight speaker slots; slots are not registered identities.
Inference receives only a verified local model path and cannot implicitly search
an archived application's models. GPU availability failures may use the packaged
CPU path. An allocation or unrelated runtime failure does not prove a missing GPU.

Packaging does not establish Windows hardware qualification. The complete app
also needs its own supported audio preparation path and target-device acceptance.
