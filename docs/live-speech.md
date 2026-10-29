# Speech processing during recording

This is a source contract, not a microphone, target-laptop or macOS acceptance receipt.

The recorder retains its device callback, bounded queue, original PCM writer and
separate playback WAV. A background reader consumes only complete frames already
written to disk. Saved PCM is the backlog when inference falls behind; inference
never waits inside the device callback. Original audio is retained.

One isolated child holds a Nemotron 3.5 Q8 ASR stream and an independent Nemotron 3
speaker stream for the capture session. Pause/resume retains both states. ASR
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

On successful recording stop, the reader drains saved frames and flushes its
bounded resampler into a mono 16 kHz PCM16 analysis WAV. It closes and synchronizes
the WAV before sending an explicit final frame count. The child checks that the
finalized WAV's PCM hash/count equal the samples actually fed, verifies model and
runtime identity, and finishes ASR once. It saves the ASR artifact before speaker
finalization; a later speaker failure preserves recognition. The parent verifies
its independently accumulated input hash/count and atomically records source,
analysis WAV and ASR hashes as soon as the synchronized ASR artifact is announced.
The ordinary job checkpoints canonical text before waiting for speaker completion,
then uses the existing terminology and speaker continuation paths. An explicit
retry after restart can recover a source/session/hash-matched ASR artifact without
replaying ASR; changed or unrelated artifacts are not reused. Speaker retry does not
rerun saved ASR. Final word times and speaker assignments are projected after EOF.

Final publication retains source/CAS checks, canonical words, revision history and
manual corrections. Live artifacts use `whole-recording-stateful-live-v1` with
`native-stream-finish-cabi-v1`, exact fed-frame count, explicit native finish and
the input PCM SHA-256. Core validation still checks the native final duration and
word ranges. A valid final ASR artifact can survive a subsequent child failure;
ASR completion does not claim successful speaker analysis.

Stop returns after audio finalization while speech drains in the background.
The finalized recording immediately becomes an ordinary durable, cancellable job.
Its action waits for the matching session; other inference waits while the live
child is active. A queued or running cancellation stops only its owned live child.
Shutdown/removal terminates and joins it. A new capture can interrupt a prior finishing child; completed ASR
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
