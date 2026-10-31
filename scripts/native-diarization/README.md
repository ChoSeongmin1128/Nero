# Shared native speech worker

This build-only preparation stages the shared NVIDIA NeMo-Speech.cpp runtime for
Nemotron ASR and speaker analysis. Windows uses a composite package: the official
v0.2.0 Vulkan archive and a pinned CPU source build with the nine-file PR47 mmap
backport plus `windows-scheduler.patch`. The Windows patch starts scheduler
storage at 2048 graph nodes and grows to the original 65536-node limit when
needed, synchronizing and invalidating cached placements without resetting model
or stream state. It changes storage capacity, not recognition parameters.
macOS arm64 retains only the nine-file PR47 mmap backport. Neither target includes
the rest of that upstream merge's changes.
`runtime.json` owns the base commit, target identities, source/archive hashes,
patch hash, exact patched file identities and compiler flags. It never downloads model weights or reads a user library.

```sh
python3 scripts/native-diarization/prepare.py --target macos-arm64
python3 scripts/native-diarization/prepare.py --verify-staged apps/desktop/.generated/native/diarization --target macos-arm64 --execute-runtime
```

On native Windows builds use `--target windows-x64` from an x64 MSVC developer
environment with CMake, Ninja and Git. CPU compilation uses the pinned AVX2 flags
and UTF-8 source decoding; it requires an AVX2/FMA/F16C-capable end-user CPU.
The target receipt distinguishes the source-built CPU backend from official Vulkan.
Packaging includes Vulkan and CPU executables with the app-local Microsoft C++ runtime DLLs already present
in the hash-verified official archives. They are never replaced by an older local
Visual Studio toolchain. An incomplete pinned archive is rejected instead of
mixing CRT versions. The CPU source build copies only the verified official
archive's app-local CRT, rather than its original speech binaries. `--msvc-redist` is retained as an ignored compatibility input
for this speech preparer; the separate summary preparer has its own contract. macOS builds the Metal runtime,
whose executable also supports CPU operation. Source preparation requires Python
3.12+, CMake, Xcode command-line tools and Git on the developer machine. If CMake
is installed as a Python module, `--cmake-python /path/to/python` selects it.
Pinned NeMo, llama.cpp and SentencePiece archives are verified before compilation.
Temporary source/build trees are automatically removed; weights are never fetched. The original `bin` / `lib` layout
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

Speaker analysis uses the app's isolated `--nero-speaker-worker` child process,
before Tauri windows or single-instance registration. It loads the same packaged
C ABI library as ASR. No Python, separate model runtime or compiler is needed by
users. One mono16k PCM16 recording is pushed through one continuous stream and
finished at EOF. The V3 geometry uses speaker cache 264, FIFO 40, chunk 340,
update period 300, left context 0 and right context 40 (80ms encoder frames).
It does not use short-recording full-attention `--offline`.

The child exports every new probability frame before native compaction can discard
it. It rejects missing frames, revised retained probabilities, nonfinite values and
incomplete input/output counts. Full-timeline `p > 0.5` contiguous activity has no
padding, gap filling or minimum-turn filtering. This bypasses the old runtime's
V2-derived segmentation defaults and its different prefix/tail configuration path.
Raw frame-major float32 probabilities, activity, source/model/library hashes and
completion evidence are retained. Probability output is bounded at 128MiB and
retained native snapshots at 64MiB; these are memory bounds, not accuracy guarantees.

For timestamped Nemotron words, the shared Core projects the model probabilities
using the pinned native rule: sum each slot over the word onset, up to 160ms,
then choose the largest sum. This is not a calibrated confidence score or verified
human identity. New word turns form a separate source-validated generation; old
recognition, corrections, manual speaker edits and history remain available.
No attendee count or name is supplied to inference. The model has eight slots,
which do not establish qualified eight-person meeting accuracy.

GPU is preferred. Only a confirmed unavailable GPU backend permits the packaged
CPU route; allocation and generic engine failures do not. Windows loads the selected
app-local DLL and dependencies only from its directory and System32. Backend choice,
cancellation, source-version checks and final publication remain owned by the app's
existing processing/job boundary. A speaker retry does not rerun ASR.

Packaging does not establish Windows hardware qualification. The complete app
also needs its own supported audio preparation path and target-device acceptance.

The minimal Mac backport has a composite base-plus-patch identity, not the upstream
merge commit identity. Shared Core runtime metadata derives from this same recipe.
Both registered base artifacts and patched artifacts remain readable with their
original provenance. Native workers emit the current target identity; unknown
identities remain invalid. Receipts require the exact target source recipe and
all staged file hashes before signing. Source package changes require re-preparing
the generated package; do not manually relabel a previously prepared binary.

Windows product model selection is documented in
[Windows speech models](../../docs/windows-speech-models.md). Existing model
weights and their download hashes are unchanged; runtime preparation never copies
or re-downloads user models. Stored official-base artifacts remain readable with
their original identity. Runtime changes do not authorize automatic reprocessing.
