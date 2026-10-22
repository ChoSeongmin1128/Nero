# Native diarization worker

This build-only preparation stages official NVIDIA NeMo-Speech.cpp v0.2.0
release archives. `runtime.json` pins the upstream commit, archive sizes and
SHA256 values. It never downloads model weights or reads a user library.

```sh
python3 scripts/native-diarization/prepare.py --target macos-arm64
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

This optional adapter does not establish a default recognition policy or a
Windows hardware qualification. Default integration and target-hardware quality
remain separate release decisions.
