# Native Whisper runtime

Build-time preparation stages the pinned whisper.cpp CLI and its required dynamic
libraries, C++ runtime on Windows, and source license. It never downloads model
weights. `runtime.json` owns source/archive hashes and the per-platform recipe.

```sh
python3 scripts/native-whisper/prepare.py --target macos-arm64
python3 scripts/native-whisper/prepare.py --verify-staged apps/desktop/.generated/native/whisper --target macos-arm64 --execute-runtime
```

Windows uses `--target windows-x64` in an x64 MSVC build environment and requires
the licensed Microsoft C++ runtime from the build tools or `--msvc-redist`.
Prepare build-only tools before the runtime:

```sh
python scripts/native-whisper/prepare.py --prepare-tools
python scripts/native-whisper/prepare.py --target windows-x64
python scripts/native-whisper/prepare.py --verify-staged apps/desktop/.generated/native/whisper --target windows-x64
```

`--toolchain-root` selects a verified tool directory under ignored `.work`.
`--prepare-tools --download-only` only fetches and verifies archives. Tool versions,
archive sizes and SHA-256 pins live in `toolchain.json`. LunarG's documented
`copy_only=1` mode copies the build SDK without installing a loader, layers,
shortcuts or persistent environment/registry settings. CMake and Ninja are portable
archives. The SDK is a build input, never an end-user prerequisite. The existing
MSVC compiler is identified by its path/hash in the generated build receipt.

The official CPU CLI and CPU dispatch libraries remain in `bin/`. The same pinned
whisper.cpp source builds an isolated Vulkan CLI under `vulkan/bin/`, with static
backend/C++ libraries, portable CPU instructions and no OpenMP/CUDA/FFmpeg.
Pinned Vulkan and SPIR-V header archives supply headers and complete licenses.
The package records source/toolchain/compiler/cache/flag and file hashes plus the
PE dependency inventory; staging validates the dependency closure without loading
any DLL. Only the GPU executable may import the system Vulkan loader. Neither the
CPU executable nor `nero-vulkan-probe.exe` depends on its presence.

At runtime the model-free helper loads `vulkan-1.dll` from System32 and checks
Vulkan 1.2, GPU device type, 16-bit storage access and a compute queue. A discrete
GPU is preferred over an integrated GPU; the chosen physical index is frozen for
the job. This is capability evidence, not inference acceptance. Successful model,
backend and synchronized GPU-graph diagnostics plus native exit zero establish
actual Vulkan use. A pinned, hash-checked diagnostic patch reports a completed
non-empty Vulkan graph after synchronization. It adds one synchronization per CLI
process and does not change model math, word timestamps or transcription policy. A
separate pinned build patch caps internal shader compiler concurrency at one,
so the outer `-j2` does not hide sixteen extra compiler processes. Only
known missing/unsupported capability permits CPU; probe failure,
timeout, allocation failure or silent CPU initialization preserves an error.
Transcribed stdout is never parsed as backend evidence. All child processes retain
the existing cancellation/process-tree ownership. No CPU/GPU setting is added.

Windows v2 output preserves each window's actual backend and target runtime,
package receipt, executable, stderr and probe-result hashes. Legacy v1 CPU and
Metal artifacts remain readable under their original evidence contract. PCM
ownership, token-byte projection and the terminal-context time policy are unchanged.

macOS builds the CLI for Apple Silicon with an explicit ARM baseline, Metal and
embedded Metal assets. Host-specific CPU optimization is disabled. The common
preparer relocates and checks the dynamic dependency closure before hashing it.
Signing uses `signed_transition` and `verify_signed`; source hashes describe the
pre-signing inventory, while a separate signing receipt preserves signed hashes.
These checks do not establish execution on every supported Mac or laptop.

The optional quality model pin is `config/models/whisper-turbo.json`. Recognition
must retain native output and token timestamps, report the actual backend, and
validate full input coverage independently of the NeMo stateful-stream protocol.
Building this package alone does not enable a second engine in the product.
