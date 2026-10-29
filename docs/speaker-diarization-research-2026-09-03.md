# Speaker diarization architecture and evaluation guide

Status: public technical notes. Current product behavior is defined by source code;
external model support and licenses must be checked against the linked upstream release.

This AI-assisted document contains no private recordings, reference annotations,
recording-derived scores, runtime measurements, or meeting-specific error analysis.
Keep those materials in ignored local data/result directories.

## Current Grove routing

Grove uses a native MOSS worker for post-meeting text transcription and a separate
diarizer for speaker activities. The current automatic preset selects Ultra8 for an
unknown count or an entered count of 1–8, and Community-1 with an exact count for 9+.
Sortformer4, Ultra8, and Community-1 can also be chosen explicitly per recording.
These are application settings and model-capacity limits, not measured meeting results.

- Ultra8 supports at most eight output speaker slots; Sortformer4 at most four.
- Entered counts are advisory for these Sortformer routes, not an exact-N constraint.
- Explicit Community-1 can compare unconstrained estimation with a supplied count.
- An unknown count is not evidence that the recording fits a model's capacity.
- Unsupported capacity, missing assets, invalid model identity, and malformed output
  must fail explicitly; do not silently substitute a different engine.
- The current default does not establish general Korean accuracy or larger-meeting
  quality. A model's supported capacity is distinct from correct speaker recovery.

See [engine/count options](ultra8-option-and-speaker-count.md) and
[app inference configuration](../legacy/swift/Sources/GroveInference/InferenceConfiguration.swift).

## Distinct responsibilities

ASR determines the words; diarization determines when anonymous speakers are active.
A derived assignment maps transcript spans onto those activities. None of these
alone establishes a person's real identity or produces human-verified training data.

Two broad diarization designs should be distinguished:

- Segmentation, speaker embeddings, and clustering create global speaker groups.
  Counts or clustering parameters may constrain that grouping where supported.
- End-to-end activity models predict speaker activity over time with a checkpoint-specific
  output capacity. Their capacity is not interchangeable with a clustering count argument.

Overlapping activities should remain in raw output even when an exclusive assignment
is useful for displaying words. Preserve source channels, machine cluster IDs,
user-confirmed profiles, and human corrections as separate concepts.

## Upstream reference map

The links below are implementation/research references, not a quality ranking.

| Family | Role to inspect | Primary references |
|---|---|---|
| MOSS-Transcribe-Diarize | Joint text/timestamp/speaker-tag model; Grove uses a separate diarizer for final assignment | [Repository](https://github.com/OpenMOSS/MOSS-Transcribe-Diarize), [model](https://huggingface.co/OpenMOSS-Team/MOSS-Transcribe-Diarize) |
| Sortformer | End-to-end speaker activity and streaming state contracts | [NeMo diarization configuration](https://docs.nvidia.com/nemo-framework/user-guide/latest/nemotoolkit/asr/speaker_diarization/configs.html), [FluidAudio](https://github.com/FluidInference/FluidAudio) |
| Ultra-Sortformer | Expanded-capacity checkpoint and separate ONNX conversion provenance | [Repository](https://github.com/mago-research/Ultra-Sortformer), [checkpoint](https://huggingface.co/mago-ai/ultra_diar_streaming_sortformer_8spk_v1), [ONNX artifact](https://huggingface.co/investguy/ultra_diar_streaming_sortformer_8spk_v1_onnx) |
| Community-1 | Segmentation/embedding/clustering and regular versus exclusive activities | [Model](https://huggingface.co/pyannote/speaker-diarization-community-1), [speech-swift integration](https://github.com/soniqo/speech-swift/blob/main/docs/inference/speaker-diarization.md) |
| SpeakerKit | Native API and output reconciliation contracts | [Argmax repository](https://github.com/argmaxinc/argmax-oss-swift#speakerkit) |
| NeMo clustering/MSDD | Speaker embeddings, count constraints, and multiscale decoding | [NeMo configuration](https://docs.nvidia.com/nemo-framework/user-guide/latest/nemotoolkit/asr/speaker_diarization/configs.html) |
| DiariZen | Local segmentation, embeddings, clustering, and model-specific usage conditions | [Repository](https://github.com/BUTSpeechFIT/DiariZen), [model](https://huggingface.co/BUT-FIT/diarizen-wavlm-large-s80-md-v2) |
| Qwen3-ASR | Text transcription and optional alignment, separate from a diarizer | [Repository](https://github.com/QwenLM/Qwen3-ASR) |

Inspect the exact model and conversion licenses before packaging. A repository's code
license does not automatically license its weights, datasets, or dependent artifacts.

## macOS integration patterns

- **MLX/Metal:** a Swift or Python worker can load an Apple-oriented model conversion.
  Pin model/runtime revisions and package matching Metal resources.
- **Core ML:** neural stages can run through Core ML while clustering and timeline
  reconstruction remain in native code. Check shapes, data types, transforms, and
  compute-unit support for the exact converted model.
- **Native CPU runtime:** C++, Rust, or ONNX Runtime helpers can isolate inference from
  the SwiftUI process. Backend choice and thread settings are part of the execution contract.
- **Reference implementation:** an upstream Python pipeline can provide a comparison
  route. Its results do not prove a converted backend is numerically equivalent.

Keep worker processes headless and scoped to one job. Release one stage's allocations
before starting the next, support cancellation, and retain raw output on failure.
A successful build does not prove that weights or runtime resources are installed.

Grove's implementations are documented in the [MOSS worker](../scripts/native-moss-harness/README.md)
and [Ultra8 worker](../scripts/native-ultra8-harness/README.md) notes.
[Third-party notices](../THIRD_PARTY_NOTICES.md) describe redistributed dependencies.

## Evaluation method

Prepare authorized, independently reviewed reference material before comparing candidates.
Keep inputs, references, exact model/runtime identities, settings, and raw results local.

| Question | Measurement contract |
|---|---|
| Are the words correct? | Text CER/WER under a versioned normalization policy; do not include speaker labels in text scoring |
| Is the speaker correct? | Speaker confusion separately from missed speech and false alarms; document mapping and overlap policy |
| Is the timeline correct? | DER/JER with declared collar and evaluation map; retain component totals |
| Is the attributed transcript correct? | A defined speaker-attributed metric such as cpCER, not an average of text CER and DER |
| Is execution practical? | Cold/warm wall time, real-time factor, sampled process memory, pressure, and optional energy under a documented collection method |

Evaluation maps describe reviewed audio regions, including reviewed silence where
false alarms can be assessed. They must not be generated merely from the union of
reference speech. See [pyannote evaluation-map documentation](https://pyannote.github.io/pyannote-metrics/basics.html#evaluation-map).

Keep automatic and supplied-count conditions on the same engine when comparing the
effect of count input. Do not describe an advisory-only count field as an oracle-count
experiment. Preserve overlapping raw activities and document any exclusive projection.

Process RSS is not a complete attribution of GPU/ANE unified memory. Pipeline stage
times must not be reported as end-to-end application latency, and build/model-download
time should be distinguished from inference time.

Repeatedly used development material is not a fresh holdout. Group derivatives of the
same original together and use independent meetings for generalization checks. Do not
tune labels, normalization, or thresholds against a holdout and then report it as unseen.

## Product safeguards and remaining work

Manual speaker correction and explicit name reuse are available; automatic cross-meeting
voice identity matching is not shipped. A voice profile cannot by itself recover separate
people from an already merged diarization cluster. Cross-recording matching requires its
own enrollment, rejection, privacy, and independent-evaluation contract.

Speaker-issue acknowledgment is distinct from complete annotation or permission to train.
See the planned [review workflow](review-mode-spec.md), [annotation schema](annotation-schema.md),
and [dataset export](dataset-export.md). Hardware behavior, long-file resource limits,
and broader accuracy must be validated independently before making release claims.
