# Pinned MOSS runtime patch

Runtime identity: `mlx-audio-swift-v0.1.3+nero-moss-whole-v1`.

The patch changes one MOSS source file in MLXAudio 0.1.3 at commit
`d302a5c6080d2bb97bae38c7418f82abb76013b6`. `manifest.json` binds the original source,
patch and resulting source to SHA-256 digests. The shared `STTOutput` contract is
unchanged. Sampling, EOS IDs and the existing repetition guard are unchanged.

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

## Checks

Root validation runs this lightweight suite separately from model execution:

```bash
python3 -m unittest discover -s scripts/native-moss-harness -p test_runtime_patch.py
```

These checks cover patch integrity, metadata/budget/stop-rule patch content,
idempotence, provenance and rejected paths/states with simulated Git application.
They do not compile Swift or run the model. The pinned dependency patch must still
be applied, compiled and exercised by the native validation workflow.
