# Pinned MOSS runtime patch

Runtime identity: `mlx-audio-swift-v0.1.3+nero-moss-whole-v2`.
Processing policy: `whole-recording-v1`. The policy name is unchanged; it does not
mean that v1 runtime evidence can satisfy the new-publication contract.

The patch changes one MOSS source file in MLXAudio 0.1.3 at commit
`d302a5c6080d2bb97bae38c7418f82abb76013b6`. `manifest.json` binds the original source,
patch and resulting source to SHA-256 digests. The shared `STTOutput` contract is
unchanged. Greedy argmax and the existing repetition guard are unchanged. The
decoder stop IDs now come from the loaded model's `generation_config.json` and
must agree with the loaded tokenizer's EOS. A missing or conflicting definition
fails before decoding, without a hardcoded substitute.

The pinned model snapshot `704aa4a9c304e8520be88901e0d1960158ef5b15` defines
`eos_token_id=151645` and `pad_token_id=bos_token_id=151643` in
[generation_config.json](https://huggingface.co/OpenMOSS-Team/MOSS-Transcribe-Diarize/blob/704aa4a9c304e8520be88901e0d1960158ef5b15/generation_config.json).
Its [tokenizer configuration](https://huggingface.co/OpenMOSS-Team/MOSS-Transcribe-Diarize/blob/704aa4a9c304e8520be88901e0d1960158ef5b15/tokenizer_config.json)
defines `<|im_end|>` as EOS and `<|endoftext|>` as padding. The manifest retains both
configuration hashes, matching the app's pinned model manifest. The upstream Swift
stop set included padding as a second EOS; this patch removes that inherited policy
and records the actual terminal token to distinguish model EOS from other stops.
It does not suppress padding output, alter model weights or force continuation
after a configured EOS. The caller's explicit output budget still takes precedence
over `max_new_tokens` in the model's generation defaults.

## Build order

1. Resolve the retained package lock into a new task-owned directory beneath this
   repository's `.work/`. Do not use the canonical package cache or another task's
   checkout.
2. Apply the patch before compiling the worker:

   ```bash
   python3 scripts/native-moss-harness/apply-runtime-patch.py \
     --task-root .work/TASK \
     --checkout .work/TASK/PACKAGES/checkouts/mlx-audio-swift \
     --receipt .work/TASK/moss-runtime-patch.json
   ```

3. Run the existing Xcode Release build with that same package checkout directory,
   its matching Metal resources and retained resolution lock. Automatic package
   updates must not replace the patched checkout between verification and compilation.
4. Retain the patch receipt with the worker's build/package provenance. Verify the
   executable after relocation before treating the package as runnable.

The worker reads `model.generationMetadata`, which is absent from unpatched
MLXAudio. A canonical build that omits this patch is expected to fail compilation;
do not remove the metadata checks or substitute an older executable to make it pass.

The application tool refuses an unexpected dependency commit, unknown source hash,
other staged/modified/untracked files, symlinked paths, receipts inside the dependency
checkout and a path outside the selected task. An already patched source with the
expected digest is verified without reapplying. A conflicting receipt is retained
and rejected. A failed post-application check must not be followed by a build.

## Metadata API

`MossTranscribeDiarizeModel.GenerationMetadata` is a public `Sendable` struct.
`model.generationMetadata` is a publicly readable array with a private setter.

| Field | Swift type | Meaning |
| --- | --- | --- |
| `promptTokens` | `Int` | Actual prepared multimodal prompt token count |
| `requestedMaxTokens` | `Int` | Caller-requested decode cap |
| `effectiveMaxTokens` | `Int` | `min(requestedMaxTokens, contextSize - promptTokens - 1)` |
| `contextSize` | `Int` | Pinned model text configuration's maximum positions |
| `generatedTokens` | `Int` | Returned non-EOS token count |
| `terminationReason` | `String` | `eos`, `length` or `repetition` |
| `terminalTokenId` | `Int?` | Last token selected by the decoder, including the actual stop token |
| `configuredEosTokenIds` | `[Int]` | EOS IDs loaded from the model generation configuration |
| `tokenizerEosTokenId` | `Int` | Loaded tokenizer EOS; must agree with the generation configuration |
| `inputAudioSamples` | `Int` | Actual mono input sample count used for feature extraction |
| `featureChunkCount` | `Int` | Actual feature tensor batch size |
| `featureTokenLengths` | `[Int]` | Token lengths for every acoustic chunk, in source order |
| `audioTokenCount` | `Int` | Audio placeholders inserted into the decoder prompt |
| `audioEmbeddingCount` | `Int` | Actual merged audio embedding sequence length before injection |
| `samplingPolicy` | `String` | `greedy-argmax-v1`; no stochastic sampler is added |

One context position is reserved for EOS. A nonpositive request or no remaining
decode capacity throws the existing MOSS input error. The existing synchronous
entry point retains its upstream fatal-error behavior when generation throws.

The array resets at the beginning of synchronous generation and stream generation.
The `STTGenerateParameters` overload delegates to the synchronous entry point.
Each completed internal decoder call appends exactly one record. A whole-input
worker must check that the array contains exactly one record; a duration setting
that unexpectedly causes internal splitting cannot be claimed as one shared context.

`eos` is a decoder stop reason, not proof of transcription accuracy or complete
speech coverage. `length` and `repetition` must be treated as incomplete results,
including syntactically complete timestamped text.

The worker emits metadata schema 2 and `model-generation-config+tokenizer-v1`.
New whole-recording persistence requires the actual stop token to be 151645, both
EOS definitions to agree, and every acoustic chunk's expected length to match the
complete 16 kHz input. A 30-second chunk contains 375 merged audio tokens; a partial
chunk uses `ceil(samples / 1280)`. Their sum must equal the actual audio embedding
count. These are structural coverage checks, not proof that every spoken word was
recognized. A syntactically incomplete ending still fails even after a true EOS.
Old v1 evidence cannot qualify a new whole-recording result; existing stored
transcripts and raw artifacts are retained without automatic reprocessing.
History validation retains the v1 contract only after the existing raw hash and
initial/current source association are verified. This does not authorize saving
a newly supplied v1 result or rewriting the existing artifacts.

## Package consumption

The worker's bounded `--runtime-info` query reports runtime v2, metadata schema 2,
the unchanged whole-recording policy, input/output limits, EOS policy and greedy
sampling policy without loading a model. `verify_worker_package.py` checks that
identity together with canonical Swift/helper hashes, the resolution lock, full
patch receipt, resource bundles, Metal resource and staged executable hash.
macOS build consumers perform static package checks; signing additionally executes
the query. A present stale package fails rather than falling back to a previous
worker. An absent optional package remains unavailable; Windows builds do not use
this runtime.

The original `RuntimeProvenance.json` describes the ad-hoc package before Developer
ID signing and remains unchanged. The release receipt separately links its digest
and pre-sign executable hash to post-sign hashes, signed identity, runtime metadata
and package/app tree digests. Expected Mach-O/signature-envelope changes are not
treated as changed model resources. This is a packaging/signature linkage, not an
embedded source attestation or a model-quality acceptance result.

## Checks

Root validation runs this lightweight suite separately from model execution:

```bash
python3 -m unittest discover -s scripts/native-moss-harness -p test_runtime_patch.py
```

These checks cover patch integrity, metadata/budget/stop-rule patch content,
idempotence, provenance and rejected paths/states with simulated Git application.
When the retained upstream source is available, a temporary copy also verifies
real patch application and the resulting source hash without editing that checkout.
They do not compile Swift or run the model. The pinned dependency patch must still
be applied, compiled and exercised by the native validation workflow.
