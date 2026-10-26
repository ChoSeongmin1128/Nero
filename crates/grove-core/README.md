# Grove core

Development recorder for the cross-platform rewrite. This crate does not load the
legacy Swift library, migrate recordings, run models, connect accounts, or capture
system output. The host supplies its separate application data directory.

## Ownership

`CoreHandle::open(root)` starts a command thread. That thread owns the CPAL stream,
meeting state and storage lock. Opening the store does not enumerate or start audio.
`devices()` enumerates input devices; only an explicit `start()` opens the microphone.
The final handle drop joins the command thread and finalizes an active recording as
interrupted. UI windows do not own recording lifetime.

The callback copies native PCM bytes into a preallocated, bounded SPSC queue. It
performs no disk writes, allocations, mutex operations, logging or model calls.
A separate writer drains the queue, tracks written frames and syncs a checkpoint
once per second and on pause/stop. Queue overflow, stream errors, invalid frame
boundaries and storage errors stop acceptance and mark the recording interrupted.
A partial file-write failure permanently fences that queue from being written again:
the complete prefix is recovered, while the ambiguous queued batch is never replayed.
The original PCM remains available even when playback-file creation fails.

The queue holds up to two seconds at the selected device format, capped at 8 MiB to
bound allocation even for unusual multichannel devices. This is a capacity bound,
not a guaranteed recovery-loss bound. A process crash can lose bytes still in memory;
recovery reads complete frames actually present on disk. Power-loss durability and
faulty or stalled drivers require separate native acceptance testing.

## API

Methods return `grove_core::Result<T>` and block the caller until the command finishes.
A UI host should call them off its event/render thread.

| Method | Result |
| --- | --- |
| `CoreHandle::open(root)` | `CoreHandle` |
| `list_meetings()` | `Vec<Meeting>` |
| `create_meeting(title, notes)` | `Meeting` |
| `update_meeting(id, title, notes)` | `Meeting` |
| `devices()` | `Vec<Device>` |
| `start(meeting_id, device_id)` | `Snapshot` |
| `pause(session_id)`, `resume(session_id)`, `stop(session_id)` | `Snapshot` |
| `snapshot()` | `Snapshot` |
| `audio_path(meeting_id)` | Validated `PathBuf` for finalized playback |
| `import_audio(title, original, prepared_wav)` | `Meeting`; original plus mono 16 kHz PCM/playback |
| `transcript(meeting_id)` | `Option<Transcript>` |
| `commit_transcript(meeting_id, expected_version, raw)` | `Transcript`; retained MOSS output contract |
| `commit_whole_transcript(meeting_id, expected_version, raw)` | `Transcript`; MOSS whole-input EOS/budget/completeness contract |
| `commit_nemotron_transcript(meeting_id, expected_version, raw)` | `Transcript`; pinned Nemotron 3.5 ASR input/completion/word validation |
| `edit_transcript(meeting_id, expected_version, texts)` | `Transcript`; separate correction revision |
| `prepare_shutdown(expected_session_id)` | `Snapshot` |

DTOs serialize as camelCase. Dates are Unix epoch milliseconds. `Snapshot.revision`
increases on the command thread for every snapshot. The host must reject older
responses when polling and action replies arrive out of order. `Snapshot.session`
contains the current or most recently finished session in this process. After reopen,
meeting rows retain durable state, while `session` starts empty.

`prepare_shutdown` checks the expected session, finishes it and fences new starts in
one command. `None` requires there to be no active session. The host may exit after
success; a failed final metadata save clears the shutdown fence and returns an error.
The main window and floating controls should hide independently of this method.

Transcripts bind to the finalized audio session. The default native host uses pinned
Nemotron 3.5 Q8 ASR through one complete-input stateful stream, then commits its result
with `commit_nemotron_transcript`. The core checks model/runtime identity, full input
frame/duration identity, native completion evidence and valid word timestamps, and
projects sentence rows without inventing speaker labels. A completed native stream
does not establish complete recognition of every spoken word.

Retained MOSS APIs keep their incomplete-coverage, parsing, token-cap and whole-input
EOS/budget validation. Existing MOSS raw output, edits and history retain their saved
revisions and speaker scope; ordinary reads never rerun recognition. All recognition
commits reject stale expected versions and preserve raw output even when it cannot
become the active transcript. Corrections preserve segment identities, times and
original text; optimistic versions reject stale edits.
Import retains the original file and its hash separately from normalized PCM/playback.
The host owns format conversion, model preparation and worker cancellation.

After durable ASR and optional terminology correction, the native host runs independent
whole-recording Nemotron 3 diarization. Speaker output binds to the saved transcript
revision and preserves words/times; a speaker-stage failure leaves recognition intact.
The host persists the source-bound continuation under the same job ID and retries that
stage without rerunning ASR. Existing meetings are not automatically reprocessed.

Only one recording can own a store. Session commands require the exact current ID;
stale IDs cannot control a newer recording. Existing recordings cannot be overwritten
by calling start again; create another meeting. Meeting title/notes can still be edited
during recording without resetting audio ownership.

## Files

```text
root/
  store.lock
  meetings/
    <meeting UUID>/
      meeting.json
      sessions/
        <session UUID>/
          manifest.json
          audio.pcm
          audio.wav
```

Each `meeting.json` is authoritative; there is no second global meeting index.
Directory and metadata IDs must match. Metadata is replaced through a newly created
and synced temporary file. A file lock excludes a second process using this store.
Paths are derived from canonical UUIDs and fixed filenames. Existing symlinks/reparse
points and Unix hard-linked files are rejected; leaf opens use no-follow flags.
These checks are not a sandbox against another process with permission to rename
application directories concurrently. Keep the store under the host's private app
location and scope renderer asset access to finalized meeting audio.

The manifest records format, native rate/channels, owner IDs, durable checkpoint
frames and state. On reopen, a previously active capture becomes interrupted. Complete
raw frames are retained, including a partial final write's complete prefix. Interrupted
captures with missing or invalid playback are retried on reopen, including failures
that happened after stop. Invalid derived WAV files are preserved as
`audio.invalid.<UUID>.wav` before regeneration. An unreadable source is reported as a
recovery failure rather than a successful repair. Completed, valid recordings are not
reprocessed. Partial
trailing bytes are preserved in the raw file and omitted from playback. An incomplete
or invalid manifest is reported instead of guessing an audio format or deleting it.

## Audio format

This rewrite slice uses native-rate, native-channel PCM for crash recovery. The
supported device defaults are little-endian F32/F64/I16/I32. It requests input devices
only and does not resample, downmix, denoise, identify speakers, or silently switch to
another microphone. Unsupported formats fail while the meeting remains a draft.

`audio.pcm` preserves bytes from the callback in the manifest's source format.
`audio.wav` is a derived 32-bit floating-point WAV for playback. Integer and F64 inputs
are converted only for this derived file; it is not a bit-exact replacement for the
source. PCM is never deleted after conversion. Standard RIFF WAV has a 4 GiB boundary;
if exceeded, the raw recording stays preserved and playback generation reports an
interruption. RF64/segmented playback is not implemented in this slice.

The legacy app's ADTS AAC/M4A contract continues to apply to legacy recordings. No
existing audio is re-encoded or moved by this crate.

## Verification

Unit tests use synthetic bytes and temporary directories. They cover written-frame
accounting, pause exclusion, overflow, checkpoint failures, stale session ownership,
recovery, original-byte retention, path rejection, two-owner exclusion, shutdown
fencing and final-handle cleanup. They do not start the microphone.

Actual microphone permission, device disconnection, default-device changes, Bluetooth,
long recording, app close/reopen, audio rendering and installed binaries still need
macOS and Windows checks. Synthetic tests do not establish native audio quality.

## Dependencies

CPAL is pinned to 0.18.2 and rtrb to 0.3.2. CPAL 0.18 passes `StreamConfig` by value,
uses stable device IDs and exposes raw callback bytes. The crate follows that API
rather than examples for older CPAL releases. Native requirements include the
platform audio backend and the host application's microphone permission description.
Windows development requires the MSVC toolchain; Linux development additionally
requires ALSA development libraries. End users receive native dependencies with the
host package and are not expected to install Rust or Python.

References checked 2026-10-19: [CPAL 0.18.2 traits](https://docs.rs/cpal/0.18.2/cpal/traits/index.html),
[CPAL raw data](https://docs.rs/cpal/0.18.2/cpal/struct.Data.html),
[rtrb producer](https://docs.rs/rtrb/0.3.2/rtrb/struct.Producer.html).

## Native CI

`.github/workflows/desktop.yml` defines macOS 15 and Windows Server 2025 native checks.
It runs frontend dependency installation/tests/build, synthetic recording-core tests
and a desktop-host compile check. The OS matrix is sequential, and Cargo uses two
build jobs. No CI step opens a microphone, loads a model, installs the app or publishes
a release. Windows Server is a native compile/test environment, not a Windows 11
laptop acceptance test.

A manual dispatch can additionally build an unsigned Windows NSIS installer and
check that an executable was generated. The installer is not run or uploaded. Its
packaging check does not establish signing, SmartScreen behavior, installation,
WebView2 first-run setup, audio quality, IME behavior or tray/window usability.
Those require an actual target computer and explicit recording interaction.

The workflow uses Node 24, the native stable Rust toolchain and locked dependency
files. It adds no package/build cache. Action pins were checked against the official
releases on 2026-10-19:
[checkout v7.0.1](https://github.com/actions/checkout/releases/tag/v7.0.1) and
[setup-node v7.1.0](https://github.com/actions/setup-node/releases/tag/v7.1.0).
The workflow file is prepared locally; no remote run is claimed by this document.
