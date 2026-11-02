# Speech processing during recording

This is a source contract, not a microphone, target-laptop or macOS acceptance receipt.

The recorder retains its device callback, bounded queue, original PCM writer and
separate playback WAV. A background reader consumes only complete frames already
written to disk. Saved PCM is the backlog when inference falls behind; inference
never waits inside the device callback. Original audio is retained.

Native capture frames/rate/channels remain the time coordinates even when a
native float32 playback WAV would exceed RIFF. Such recordings keep the original
PCM and publish a separately described mono/16kHz/PCM16 playback WAV. Core playback,
this saved-PCM reader and file analysis share the same bounded normalizer and
native-sample downmix. The canonical playback's PCM therefore matches the live
analysis PCM, including rounding, resampler delay and EOF frames. Source and
analysis file hashes remain separate identities; a matching duration is not a
substitute for either hash. Older float32 playback files are unchanged. See
`audio-import.md` for the persisted playback policy and recovery checks.

The engine is captured at session start. One isolated child holds an independent
Nemotron 3 speaker stream throughout capture. With the default engine it also
holds a Nemotron 3.5 Q8 ASR stream. Pause/resume retains both states. ASR
endpointing is disabled. Publication does not reset the models. The integrated ASR
diarizer is not used; the independent speaker stream exports probability frames
before native compaction using the existing geometry and final onset attribution.

After 60 seconds of newly processed audio, punctuation or 750 ms of quiet samples
can publish a replacement interim hypothesis. Publication is forced at 180 seconds
when a hypothesis exists. These intervals measure processed audio, not wall-clock
latency. Quiet samples still reach both models. The pinned 0.2.0 C ABI supplies
interim text without word offsets, so previews have no invented times or speaker
labels. They are read-only and bound to the meeting and capture session. A new
hypothesis replaces its predecessor; interim text and boundaries can change.

With Whisper large-v3-turbo Q5, the child instead accumulates bounded PCM for the
packaged CLI. It chooses an owned boundary near quiet samples between 90 and 120
seconds, with up to three seconds of context on either side. EOF processes the
remainder. Each temporary PCM WAV is source-bound; a CLI duration option against
a longer file is not an input boundary. A word belongs to one window by its
midpoint sample address. Ownership covers the full input without gaps. The
speaker stream receives every sample continuously. Inference can block this
saved-PCM reader, never the recorder callback.

Whisper previews use completed windows and actual native times. Native token bytes
are reconstructed before UTF-8 validation. No time or confidence is invented.
Unusable word anchors retain the complete native segment as
`nativeSegmentContext`, with its actual segment interval and no speaker.
Zero-duration words also remain unassigned. Invalid generated UTF-8, invalid
segment bounds or text mismatches fail processing with raw output/audio preserved.
Boundary wording may differ between native runs; this is not a guarantee that
every spoken boundary word is recognized.

The pinned b5454 CLI serializes segment/token times as native centiseconds times
10 ms. A partial final PCM tick has one explicit exception: only the last native
segment may end exactly at the next 10 ms tick, less than 160 samples beyond its
exact input. Its start must remain inside the input and every text-token offset
must remain on the same grid within that horizon. The entire segment is then
context with an unknown speaker, with its context end at exact PCM EOF and its
original absolute end retained as `nativeSegmentEndFrame`. Raw native JSON and
its hash are unchanged. This is not a normalized word timestamp. Aligned inputs,
larger overruns, other segments, invalid text and forged metadata remain rejected.

On successful recording stop, the reader drains saved frames and flushes its
bounded resampler into a mono 16 kHz PCM16 analysis WAV. It closes and synchronizes
the WAV before sending an explicit final frame count. The child checks that the
finalized WAV's PCM hash/count equal the samples actually fed, verifies model and
runtime identity, and finishes the selected ASR path once. It saves ASR before speaker
finalization; a later speaker failure preserves recognition. The parent verifies
its independently accumulated input hash/count and atomically records source,
analysis WAV and ASR hashes as soon as the synchronized ASR artifact is announced.
The ordinary job checkpoints canonical text before waiting for speaker completion,
then uses the existing terminology and speaker continuation paths. An explicit
retry after restart can recover a source/session/hash-matched ASR artifact without
replaying ASR; a different model, changed or unrelated artifacts are not reused. Speaker retry does not
rerun saved ASR. Final word times and speaker assignments are projected after EOF.

Final publication retains source/CAS checks, canonical words, revision history and
manual corrections. Nemotron live artifacts use `whole-recording-stateful-live-v1` with
`native-stream-finish-cabi-v1`, exact fed-frame count, explicit native finish and
the input PCM SHA-256. Core validation still checks the native final duration and
word ranges. A valid final ASR artifact can survive a subsequent child failure;
ASR completion does not claim successful speaker analysis.

Whisper uses `whisper-context-windows-v1`, `native-token-span-terminal-context-v2` and
`whisper-cli-windows-exit0-v1` (windows of audio on either platform). The envelope
binds the full PCM hash/frame count, pinned model/runtime/target, window input and
owned ranges, native-output hash, exit status, word ranges and backend evidence.
Windows explicitly disables GPU. macOS requires positive Metal initialization;
only confirmed unavailable GPU permits a separately recorded CPU attempt.
Allocation and generic failures do not permit fallback. Neither engine silently
changes the chosen model.
Stored `native-token-span-v1` results retain their original strict bounds; terminal
normalization metadata is allowed only by the explicit v2 policy.

Stop returns after audio finalization while speech drains in the background.
The finalized recording immediately becomes an ordinary durable, cancellable job.
Its action waits for the matching session; other inference waits while the live
child is active. A queued or running cancellation stops only its owned live child.
Shutdown/removal terminates and joins its owned process tree, including a running
Whisper CLI. A new capture can interrupt a prior finishing child; completed ASR
remains session-bound for the queued job. Interrupted or failed live processing preserves original audio and
uses the existing finalized-file processing path. Restart does not reconstruct
decoder state or automatically replay recordings. Interim text is not a saved,
editable or exportable transcript.

This reduces remaining work after a meeting when the device can keep up. It does
not establish real-time speed on an 8 GB office laptop, accuracy, eight-person
support, long microphone reliability or platform packaging. Those require separate
hardware and quality acceptance.

Completed native streams close speaker state/model, then ASR state/model, then
libraries. An unfinished native multi-model teardown can fault in the pinned
runtime. The isolated live child retains those native handles on failure and
exits without native static finalizers on Unix, after Rust file writers have
closed and required outputs have been synchronized. The OS reclaims only that
child's resources. This containment is not an upstream teardown repair or proof
of successful inference. Missing terminal input must still exit nonzero and
must not produce a successful ASR artifact.

On Windows, model/stream destruction does not establish that OpenMP worker threads
have stopped. The shared speech-library owner keeps Windows DLL modules loaded
until isolated process exit; it never unloads code underneath surviving native
threads. macOS retains normal library destruction after completed streams close.
This policy applies to both live and standalone speaker children and does not
keep a model resident in the main application.
