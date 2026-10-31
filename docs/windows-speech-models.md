# Windows speech model selection

Setup and Settings share the same selector. Nemotron 3.5 streaming ASR 0.6B Q8
is the default. Whisper large-v3-turbo Q5 is an explicit alternative using the
packaged whisper.cpp CLI on Windows CPU. Both use the independent Nemotron 3 Q8
speaker model. SenseVoice is not a product engine.

Selection is stored locally for subsequent work. Selection does not download
weights: preparation verifies the selected ASR model and shared speaker model.
Summary preparation remains optional. Recording, import and new meetings use the
selected engine's readiness gate. Selection is rejected during recording, running
or queued processing. Each job/session captures its engine, including explicit
retry after restart. A damaged setting requires explicit reselection.

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

Nemotron GPU preference and error handling are unchanged. A confirmed unavailable GPU
backend permits CPU operation. Allocation or generic engine failures retain the
recording and failed job; they do not silently choose another backend or model.
The CPU source package requires the pinned AVX2/FMA/F16C instruction set.

Nemotron retains one stateful ASR stream. Whisper normally owns 90–120 seconds
per exact PCM window with up to three seconds of context on either side; the last
window completes after EOF. Neither choice resets the independent speaker stream.
Saved PCM is the backlog, so Whisper can leave work after recording stops when
the device cannot keep up. Language results may differ; a model name does not
guarantee accuracy.

Whisper retains raw CLI output and per-window completion/input evidence. Native
token bytes are reconstructed before UTF-8 validation. Word times use native
anchors without interpolation. Unusable word spans retain the complete native
segment as context with an unknown speaker; invalid generated UTF-8 is rejected
with audio and raw output preserved. ASR is saved before speaker finalization;
speaker retry reuses that artifact. See [live speech](live-speech.md).

Only a pinned final sub-10 ms terminal tick can be retained as explicitly audited
segment context: the exact PCM end closes its context range, the original native
end remains recorded, and its speaker is unknown. This does not interpolate word
times or allow arbitrary out-of-input results.

Memory optimization does not establish acceptance on every 8 GB or low-power
notebook, long microphone recording, battery life, Korean meeting accuracy or
speaker accuracy. Those limits must remain separate from native file-fed runtime
and persistence checks. Private device/evaluation receipts stay outside product Git.
