# Windows speech model selection

Windows retains Nemotron 3.5 streaming ASR 0.6B Q8 as the final transcription
model and Nemotron 3 Q8 as its independent speaker model. There is one supported
final-transcript path. SenseVoice drafts and Whisper turbo are not packaged as
alternate final engines: their chunking, backlog and word-time/speaker publication
contracts have not been qualified as product replacements.

The existing model pins, hashes and explicit preparation UI remain authoritative.
Previously downloaded valid weights are reused. Existing meetings, canonical
transcripts, corrections, speaker history and source identities are not rewritten
or automatically retranscribed.

The Windows CPU package builds the existing upstream base with the pinned PR47
mmap backport and a separate Windows scheduler-storage patch. File-backed weights
avoid a private staging copy where supported. Scheduler graph-copy allocation
starts at 2048 nodes, grows geometrically with headroom, and retains the original
65536-node maximum and explicit oversized-graph failure. Growth synchronizes the
backend and invalidates cached placements, retaining model and live-stream state.
The source archives, both patches, per-file hashes, compiler flags and composite
package identity are owned by `scripts/native-diarization/runtime.json`.

Vulkan retains the separately hash-verified official base executable/libraries.
The composite Windows identity does not claim that Vulkan received either patch.
macOS retains its existing PR47-only recipe. Stored base artifacts and both target
identities remain readable without changing their historical provenance.

GPU preference and error handling are unchanged. A confirmed unavailable GPU
backend permits CPU operation. Allocation or generic engine failures retain the
recording and failed job; they do not silently choose another backend or model.
The CPU source package requires the pinned AVX2/FMA/F16C instruction set.

Recording-time processing retains one stateful ASR stream and an independent
speaker stream across the recording. Saved PCM is the backlog, publication does
not reset inference, and finalization requires complete-input hash/frame evidence,
native EOF completion and valid native word times. Interim text has no invented
word times or speaker labels. ASR is saved before speaker finalization; speaker
retry reuses that artifact. See [live speech](live-speech.md).

Memory optimization does not establish acceptance on every 8 GB or low-power
notebook, long microphone recording, battery life, Korean meeting accuracy or
speaker accuracy. Those limits must remain separate from native file-fed runtime
and persistence checks. Private device/evaluation receipts stay outside product Git.
