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
The runtime identity is `mlx-audio-swift-v0.1.3+nero-moss-whole-v2`; its processing
policy remains `whole-recording-v1`. The app's expected model snapshot is configured
separately in the [shared MOSS manifest](../../Sources/GroveApp/Resources/Models/moss.json),
which the desktop processing route and core also consume.
Use the matching model/tokenizer/configuration files and do not substitute unpinned weights.

Inference requires the Metal resources produced by the matching Xcode Release build.
A successful Swift compilation alone does not establish a runnable Metal package.
If Xcode requires dependency plug-in approval, inspect and authorize that dependency
through the appropriate build workflow; do not disable global validation or reuse
an unrelated binary's metallib.

The worker requires the checked [runtime patch](patches/README.md) before building.
Resolve the retained dependency lock into a task-owned directory, verify/apply the
patch with `apply-runtime-patch.py`, then compile against that exact checkout.
The patched model exposes actual prompt/decode budgets, terminal token/EOS
definitions, acoustic feature coverage and EOS/length/repetition termination.
The upstream commit and original/patch/result hashes are retained.
A build without that API fails instead of silently substituting an old worker.

Build Release/macOS using at most two jobs and task-local build outputs. Keep the
matching MLX Metal resource provenance with the package; compilation alone does not
establish a runnable Metal bundle. See [MLX Audio Swift](https://github.com/Blaizzy/mlx-audio-swift)
and the [upstream model](https://huggingface.co/OpenMOSS-Team/MOSS-Transcribe-Diarize).

## Processing contract

The default requests at most 65,536 output tokens, temperature=0, topP=1,
minChunkDuration=0 and repetitionContextSize=100. The decoder uses the actual
multimodal prompt count to cap output within the model context, reserving one EOS
position. Greedy argmax and the pinned repetition guard remain unchanged.
EOS comes from the pinned model's generation configuration and must agree with
the tokenizer; padding is not an additional stop rule.

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
count, metadata schema 2, prompt/context/requested/effective output budgets, actual terminal token,
configured/tokenizer EOS, acoustic feature lengths, actual embedding count and
complete PCM coverage. The core only publishes a new recognition result when the
whole-input evidence is consistent, exactly one decoder call ended with the
pinned model's actual EOS (151645),
output syntax is complete and the source version is unchanged. A length/repetition
stop is incomplete even when its final text is syntactically valid. Raw/initial
results and previous edits remain separate immutable artifacts.
Every raw utterance must have finite, ordered timestamps and correspond to the
returned start, end, text and speaker. A parser silently dropping an utterance
therefore cannot turn a partially decoded recording into a successful result.

The complete-input feature check expects one token-length entry for each 30-second
acoustic chunk: 375 tokens for a full chunk and `ceil(samples / 1280)` for the final
partial chunk. Their sum must match both prompt audio placeholders and the actual
merged audio embedding count. This is independent of timestamp proximity to the end
of the recording and still does not prove complete speech recognition.

Previously stored `nero-moss-whole-v1` results retain their original evidence and remain
available through history validation after their raw hash and initial/current source
association are verified. The compatibility path does not accept a new v1 result,
automatically reprocess a meeting, or upgrade old evidence into schema 2.

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
`RuntimeProvenance.json`. The source inventory includes `Package.swift` and every Swift
helper; the full patch receipt, canonical resolution lock, bundle contents and matching
Metal resource are checked separately. Weights are not copied into the helper.
The `XCODE_RELEASE_DIRECTORY` contract requires macOS resource bundles with only
`Contents` at their root, including `Contents/Info.plist` and `Contents/Resources`.
Mixed native/Xcode layouts and flat bundles fail before copying or running the worker;
identical duplicate bytes do not make such a bundle signable. The public packager does
not silently normalize resource layouts or remove an input resource.

The desktop preparation/build paths use the same staged-package verifier. Static
verification does not execute the worker, so a build host does not need to run the
macOS 26 worker just to compare package inputs. A present partial, stale or modified
package fails; an absent optional package leaves recognition unavailable. Windows
builds do not require this macOS-only runtime.

```bash
python3 -B scripts/native-moss-harness/verify_worker_package.py \
  verify-staged NEW_WORKER_DIRECTORY --runtime-info
```

On a supported host, `--runtime-info` also executes the helper's bounded metadata
query without loading weights. The macOS signing preflight requires that query.
Developer ID signing changes Mach-O bytes and resource signature envelopes, so
`RuntimeProvenance.json` keeps its pre-sign executable hash and is never rewritten
to match the new signature. A separate release receipt binds its digest and pre-sign
executable hash to the post-sign executable hash, Team/Identifier/CDHash, runtime
metadata and package tree digest. Only `CodeDirectory`, `CodeRequirements`,
`CodeResources` and `CodeSignature` under each of the three known resource bundles'
exact `Contents/_CodeSignature` directory may be added or changed as external
signature components. Other signature paths/files are rejected. Each resource
bundle must pass strict code-signature verification with its expected identifier
and the application's signing team; its signed identity is retained and rechecked
from the release receipt. Non-signature resources must remain unchanged.
The final release receipt also records the app tree digest; `--dmg-only` verifies the
preserved signed receipt instead of comparing signed bytes with an ad-hoc hash.

Source hashes record packaging inputs, not an embedded binary-to-source attestation.
Signed identity and matching runtime metadata do not prove semantic equivalence,
full-app readiness or long-recording recognition quality. Existing artifacts/receipts
are not overwritten. Worker symlinks/hardlinks and app links escaping the bundle are
rejected.

Validate resource discovery after relocation, cancellation and incomplete-output handling
with synthetic fixtures and separately authorized local test data. Keep all recording-derived
parity reports, scores, timing, and memory observations out of public documents.
Helper packaging alone is not a full-app smoke test or a notarized release.
