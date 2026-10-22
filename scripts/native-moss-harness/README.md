# Native MOSS worker

This is the headless worker used by Grove's post-recording/import transcription route.
It loads explicitly supplied local weights, does not download models or upload recordings,
and keeps machine output separate from human corrections.

## Invocation

```text
MossHarness INPUT_WAV NEW_OUTPUT_JSON MODEL_DIRECTORY [CHUNK_SECONDS]
```

Input is mono 16 kHz WAV. Use a new output path and keep recordings, transcripts,
references, and generated results under ignored local directories. Model files are
separate from the app and repository.

The default invocation reads the complete recording and passes it to one decoder
context. Anonymous labels belong to that recording. There is no automatic fallback
to independent chunks. The optional fourth argument is for explicit diagnostic
comparisons, not a product setting; those results retain chunk-local labels and
cannot be saved through the desktop's new-recognition path.

## Runtime and model resources

The retained dependency lock pins MLXAudio 0.1.3 and MLX Swift 0.31.6.
The app's expected model snapshot is configured separately in
[MeetingInferenceService](../../Sources/GroveApp/MeetingInferenceService.swift).
Use the matching model/tokenizer/configuration files and do not substitute unpinned weights.

Inference requires the Metal resources produced by the matching Xcode Release build.
A successful Swift compilation alone does not establish a runnable Metal package.
If Xcode requires dependency plug-in approval, inspect and authorize that dependency
through the appropriate build workflow; do not disable global validation or reuse
an unrelated binary's metallib.

The worker requires the checked [runtime patch](patches/README.md) before building.
Resolve the retained dependency lock into a task-owned directory, verify/apply the
patch with `apply-runtime-patch.py`, then compile against that exact checkout.
The patched model exposes actual prompt/decode budgets and EOS/length/repetition
termination. The upstream commit and original/patch/result hashes are retained.
A build without that API fails instead of silently substituting an old worker.

Build Release/macOS using at most two jobs and task-local build outputs. Keep the
matching MLX Metal resource provenance with the package; compilation alone does not
establish a runnable Metal bundle. See [MLX Audio Swift](https://github.com/Blaizzy/mlx-audio-swift)
and the [upstream model](https://huggingface.co/OpenMOSS-Team/MOSS-Transcribe-Diarize).

## Processing contract

The default requests at most 65,536 output tokens, temperature=0, topP=1,
minChunkDuration=0 and repetitionContextSize=100. The decoder uses the actual
multimodal prompt count to cap output within the model context, reserving one EOS
position. Sampling and the pinned repetition guard remain unchanged.

`chunkDuration` is the ceiling of the complete recording length. It therefore
bypasses the library's default 30-minute decoder splitting. Internal 30-second
acoustic feature extraction is retained and is assembled into the same decoder
input. Neither reading all PCM nor reaching a timestamp near the end proves
transcription accuracy.

Default input is limited to the upstream model's advertised 90-minute range.
That is not a memory or quality qualification on an ordinary laptop. Longer input
fails explicitly without splitting, truncating or replacing the previous transcript.
Within that range, allocation/inference failure also preserves originals.

Raw output records `whole-recording-v1`, recording speaker scope, actual generation
count, prompt/context/requested/effective output budgets, completion reason and
complete PCM coverage. The core only publishes a new recognition result when the
whole-input evidence is consistent, exactly one decoder call ended with EOS,
output syntax is complete and the source version is unchanged. A length/repetition
stop is incomplete even when its final text is syntactically valid. Raw/initial
results and previous edits remain separate immutable artifacts.
Every raw utterance must have finite, ordered timestamps and correspond to the
returned start, end, text and speaker. A parser silently dropping an utterance
therefore cannot turn a partially decoded recording into a successful result.

Explicit diagnostic chunks retain 1...180-second input and an 8192 requested output
cap; their speaker labels must not be equated across chunks. They are not an
alternative selected automatically after a whole-input failure.

Upstream formatting prefixes remain in raw JSON. The decoder separates matching
speaker prefixes from the spoken body without modifying the raw result.

## Packaging

```text
package_worker.sh XCODE_RELEASE_DIRECTORY CHECKOUTS_DIRECTORY NEW_WORKER_DIRECTORY PATCH_RECEIPT
```

The script copies the executable, matching resource bundles, dependency notices, and
resolution lock into a non-overwriting directory and ad-hoc signs the helper.
It verifies the pinned runtime patch receipt before copying, checks the relocated
worker's runtime identity, and records source, executable and resource hashes in
`RuntimeProvenance.json`. Source hashes record packaging inputs, not an embedded
binary-to-source attestation; release validation also checks the canonical build
and staged worker. The app packager consumes that staged worker. Weights are not
copied into the helper.

Validate resource discovery after relocation, cancellation and incomplete-output handling
with synthetic fixtures and separately authorized local test data. Keep all recording-derived
parity reports, scores, timing, and memory observations out of public documents.
Helper packaging alone is not a full-app smoke test or a notarized release.
