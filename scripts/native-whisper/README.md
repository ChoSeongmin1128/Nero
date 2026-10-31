# Native Whisper runtime

Build-time preparation stages the pinned whisper.cpp CLI and its required dynamic
libraries, C++ runtime on Windows, and source license. It never downloads model
weights. `runtime.json` owns source/archive hashes and the per-platform recipe.

```sh
python3 scripts/native-whisper/prepare.py --target macos-arm64
python3 scripts/native-whisper/prepare.py --verify-staged apps/desktop/.generated/native/whisper --target macos-arm64 --execute-runtime
```

Windows uses `--target windows-x64` and requires the licensed Microsoft C++ runtime
from the installed build tools or `--msvc-redist`. It packages the CPU CLI and CPU
dispatch libraries, excluding servers, examples and other model families.

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
