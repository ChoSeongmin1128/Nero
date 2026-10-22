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

The worker generates text, timestamps, and upstream anonymous tags. Each independent
generation has its own speaker scope. The retained Swift application can apply an
independent diarizer afterward; the new desktop's default recognition route does not
yet do so. Its separately invoked whole-recording adapter requires packaged workers
and explicit preparation. Never stitch ASR-local tags into global identities across
chunks or source channels. See the [application README](../../README.md).

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

Build the `MossHarness` scheme for Release/macOS with a task-local DerivedData directory.
Retain the dependency lock, model identity, build configuration, and resource provenance
for reproducibility. See [MLX Audio Swift](https://github.com/Blaizzy/mlx-audio-swift)
and the [upstream model](https://huggingface.co/OpenMOSS-Team/MOSS-Transcribe-Diarize).

## Processing contract

The current implementation uses maxTokens=8192 per chunk, temperature=0, topP=1,
minChunkDuration=0, and repetitionContextSize=100. Files up to 180 seconds use one
chunk; longer files use 60-second chunks. The optional fourth argument overrides the
chunk duration within 1...180 seconds. These are source configuration values, not
recording-derived performance measurements or a demonstrated optimum. The upstream
model supports long single-pass inputs; this wrapper limit is an application policy,
not an upstream model limit. Long-input support in the model does not qualify this
Swift runtime or ordinary laptop memory use.

The reader loads a bounded PCM chunk, records raw text/token counts, and offsets
timestamps onto the original timeline. Chunk boundaries and model-generated timestamps
still require independent validation; successful execution is not a quality certificate.

`hitTokenLimit` is true if any individual chunk hits its cap. A sum of tokens across
multiple completed chunks must not be compared with one chunk's cap. Unparsed output
and incomplete PCM coverage are also reported and rejected rather than silently
published as complete transcription.

Upstream speaker-formatting prefixes remain in the raw JSON. Grove's decoder separates
a matching leading formatting label from the spoken body without rewriting the raw output.

## Packaging

```text
package_worker.sh XCODE_RELEASE_DIRECTORY CHECKOUTS_DIRECTORY NEW_WORKER_DIRECTORY
```

The script copies the executable, matching resource bundles, dependency notices, and
resolution lock into a non-overwriting directory and ad-hoc signs the helper.
The app packager consumes that staged worker. Weights are not copied into the helper.

Validate resource discovery after relocation, cancellation and incomplete-output handling
with synthetic fixtures and separately authorized local test data. Keep all recording-derived
parity reports, scores, timing, and memory observations out of public documents.
Helper packaging alone is not a full-app smoke test or a notarized release.
