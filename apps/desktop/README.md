# Nero desktop

First native development slice for macOS and Windows. The application is named
Nero. It has a separate identity and never opens the legacy Swift library.

## Build

Install a supported Node.js version (see `package.json`), Rust and the native Tauri
prerequisites for the target OS. Windows builds need MSVC Build Tools, the Windows
SDK and WebView2; use a native Windows environment rather than WSL for acceptance.

From this directory:

```sh
npm ci
npm test
npm run build
npm run tauri -- build --debug --bundles app --ci -- --locked
```

On Windows use `--bundles nsis` instead of `--bundles app`. Build outputs are under
the workspace's `.work/rust-target`. Run Rust tests from the repository root with
`cargo test -p grove-core --locked`. Build/test one native target at a time.

For a macOS local test bundle, apply ad hoc signing after bundling and then run
`codesign --verify --deep --strict` on that bundle. This does not provide Developer ID
signing or notarization; use a separate distribution configuration for a release.

## Version and macOS distribution

The product version is owned by `package.json`. `npm run version:check` verifies
Tauri, Rust and lockfile mirrors, and runs before a frontend build. Update them
together from the repository root:

```sh
node apps/desktop/scripts/version.mjs set 0.0.1 --build 3
```

Before first user sharing, development changes may keep `0.0.1` and advance the build.
The [version policy](../../docs/versioning.md) separates product versions, monotonic
build numbers and data formats. [Release history](../../docs/release-history.md)
records source hashes and distribution status. Generated `build-info.json` records
the exact Git commit/tree, dirty state, version/build and build host. It is packaged
as an app resource; signing a distribution refuses a dirty or mismatched source.

For macOS distribution, build `app` from a clean commit with prepared workers, copy
the resulting bundle to a new work directory, then use:

```sh
python3 apps/desktop/scripts/sign-macos.py --app WORK/Nero.app \
  --output WORK/Nero-0.0.1-build2-arm64.zip \
  --dmg-output WORK/Nero-0.0.1-build2-arm64.dmg --notarize
```

Developer ID and notary credentials come from the invoking environment. `dmgbuild`
is a build-time prerequisite, optionally selected with `--dmg-builder`; it is not
required on the user's Mac. The script signs/notarizes/staples the app before making
a ZIP/DMG, then independently signs/notarizes/staples the DMG. The image contains
Nero and an Applications link. Verification mounts it read-only, checks the embedded
app against the signed source, and detaches it. Receipts contain source commit,
version/build, notarization IDs and final artifact SHA256. Output paths must be new.
With `--dmg-only`, `--output` points to an existing ZIP and receipt. The script
verifies its SHA256 and exact embedded app before making a new DMG, without
rebuilding/resigning the app. The DMG receipt separately records the packaging
commit. Archive rejected, undistributed containers before reusing an output path;
never replace an already distributed artifact.

`npm run tauri -- dev` starts a development app. `npm run dev` alone serves its
frontend, whose actions require the native bridge; it does not supply fake meetings.
`prepare:shared` copies canonical design-system files into generated build inputs.

## Current behavior

- Read-only local CLI/MCP query meetings, notes, current corrected voice records and
  summaries without initializing recording, models or store recovery. Settings > Agent
  integration copies the installed executable's MCP configuration and CLI example.
  Tauri builds bundle the native tool; see [local agent integration](../../docs/local-agent.md).

- Create meetings, edit title/notes and reopen them after restarting.
- Select an input device and explicitly start microphone recording.
- When no input device is available, recording stays disabled; connect a microphone
  and refresh the device list to enable it. Existing recording controls stay available.
- Pause/resume/stop and use a separate compact control window.
- Preserve raw PCM and prepare a separate WAV for playback.
- Recover complete stored frames after interruption; retain originals on failure.
- On Apple silicon with macOS 26 or later and packaged workers, import local audio
  through the native file picker, retaining the original bytes and separate playback WAV.
- Explicitly prepare the pinned MOSS model, then recognize each finalized recording
  with one complete-input decoder context. The checked worker reports EOS, measured
  context/output budgets and one-generation evidence before a result can be saved.
  It does not silently switch to chunk recognition on failure. The upstream 90-minute
  input range is not an ordinary-laptop memory/quality qualification. Original audio
  and previous results remain available after an unsupported or failed input.
  Downloads are checked against the shared manifest; inference uploads no audio.
- Optional machine-local terminology profiles create a separate correction revision
  after recognition. Uncertain names remain unchanged; an absent optional contextual
  model does not block recognition. See [terminology correction](../../docs/terminology.md).
- With separately packaged NeMo-Speech.cpp and explicit speaker-model preparation,
  speaker management can analyze the complete finalized recording in one stateful run.
  It preserves words/times/originals, commits only against the unchanged source and
  leaves mixed or insufficiently covered utterances unassigned. This optional route
  is not yet part of automatic post-recording recognition; target-hardware accuracy
  and Windows acceptance remain separate checks.
- Edit transcript text, seek to a segment, and reopen corrections after restarting.
  Navigation saves and closes the editor; failed persistence retains its input.
- Confirm re-transcription and restore a verified previous recognition run with version
  checks. This preserves original audio and raw/initial recognition output.
- Search matching context in the library or highlight/move through literal matches in
  a meeting. Speaker changes on one utterance include replay; full speaker management
  keeps its actions outside the scrolling content.
  Raw worker output and correction revisions are retained separately. Cancelling or
  failing a rerun preserves the previous transcript.
- Hide the main window without ending recording. Explicit quit flushes notes and
  finalizes the active recording after confirmation.

The application identifier is `io.github.choseongmin1128.grove.desktop`.
Tauri resolves its own OS application data directory. No automatic legacy migration,
Keychain access, automatic model installation or account connection occurs.

## First use

New meetings, new recordings and file import require speech recognition to be
prepared. Meeting summary is optional: an unchecked choice includes it in the initial
download, or it can be downloaded separately later. Preparation starts only after a
click, verifies each model and retains completed models and partial downloads.
Normal controls show no model sizes. Available disk space is checked before transfer
and periodically while downloading; insufficient space surfaces a recovery message.
The UI calls the generated text "음성 기록" and the summary "회의록".

Saved meetings, notes, playback and existing results remain accessible when models
are unavailable. Active recording pause/resume/stop does not depend on readiness.
Missing platform workers show an unsupported state instead of offering a download;
the current complete meeting pipeline therefore still requires Apple silicon/macOS 26.

## Appearance

Settings / Screen exposes six color themes and independent Light / Dark / System
brightness. The main app and compact recorder use the same shared appearance code
and app-local preference. System follows the OS; an explicit brightness remains fixed.
Theme changes do not change meetings or model preparation.

## Recording and dates

macOS permission is resolved before opening the stream. The meeting and capture are
prepared before input begins. Recoverable CPAL overload, scheduling and route notices
keep recording active with a separately saved quality warning; device failure, queue
overflow and write failure still interrupt capture. A warning is not a measured loss
duration or evidence that all speech was preserved.

Normal recording completion automatically starts speech recognition for that finalized
session. Cancellation retains audio; reopening or cancelling does not repeatedly restart
recognition. The main window exposes retry/re-recognition only when needed.

During recording, hiding or minimizing the main window automatically shows the compact
control window. Once shown, it stays above other windows even after the main window
returns. Only recording completion, an explicit dismissal or disabling the setting hides it.
An explicit dismissal lasts for that session. Settings / Recording can disable
this behavior; the separate grip moves the native transparent window, while the timer
opens the meeting. Hiding the compact window does not stop recording.

Meeting date is a separately editable calendar day. Actual microphone start time remains
read-only; imported files do not acquire an invented recording timestamp. Creation and
update timestamps keep their storage semantics. Default meeting titles get a free numeric
suffix; explicitly named recurring meetings may share a title.
The meeting list shows local date and 24-hour time, ordered by actual microphone
start time or creation time when recording time is unavailable. Hover distinguishes
recording from creation and shows the separately editable meeting date. Imported
files are not assigned an inferred original recording time.

## Playback and exports

Playback uses Korean controls for play/pause, seek, mute and speed instead of the
webview's native media controls. Source text is labelled "음성 기록" and the generated
summary "회의록". Both support copying and UTF-8 TXT/Markdown export through the native
save dialog. Pending corrections are saved before export, original model text is not
substituted, and an outdated summary retains a source-change warning in the exported file.
Writes complete in a temporary file before atomically replacing the chosen destination.

The optional summary model is prepared separately from result export. Model download
shows a byte-based percentage; it does not represent recognition or summarization
completion. Creating a meeting remains available while that optional download runs.

## Recording settings

Input-device selection and device refresh live under Settings / Recording. The
meeting view contains recording actions and an error recovery link when no usable
input is available. The device ID/name preference persists in the app webview's
local storage; it is separate from meeting data and never written to the repository.
A missing explicit selection blocks the next recording instead of silently switching
to another device. Recording start refreshes availability. Existing capture keeps its
input and disables preference changes until it ends. Settings / Models links to the
existing preparation screen without duplicating download logic.

## Brand assets

The application and tray icons use the approved Nero 23-B r7 A mark.
`brand/mark.svg` owns the geometry; `brand/app-background.svg` supplies the light
application-icon background. `npm run prepare:icons` composes these for macOS/Windows
app assets using the pinned Tauri CLI. Tray assets retain the transparent mark. macOS uses a monochrome menu-bar template.
Windows switches the monochrome tray asset on reported window-theme changes;
contrast on separately configured taskbar/application themes needs Windows validation.

## Optional macOS transcription workers

Build the pinned Release/macOS `MossHarness` scheme as described in
`scripts/native-moss-harness/README.md`, using task-local build and checkout directories.
Then, from the repository root, stage its matching resources before the Tauri build:

```sh
apps/desktop/scripts/prepare-native.sh XCODE_RELEASE_DIRECTORY CHECKOUTS_DIRECTORY PATCH_RECEIPT
```

This compiles the shared audio preparer and packages the worker, its matching Metal
resources, dependency notices and resolution lock under `.generated/native`.
The worker destination must be new; existing packages are never silently replaced.
No development tools are needed by the resulting app. The recording app itself retains
its macOS 13 minimum; this optional transcription route requires Apple silicon/macOS 26.
Absent or unsupported workers leave transcription unavailable. Windows transcription
is not implemented. Models remain outside the application bundle in its own data directory.

The model snapshot and file hashes are shared with the retained Swift implementation
through `Sources/GroveApp/Resources/Models/moss.json`. Preparation starts only from the
app's explicit model button. The downloader retains interrupted partial files, resumes verified Range responses,
and checks the complete file SHA256 before publishing a readiness marker.

The product name is Nero. Packaging uses the approved 23-B r7 A mark and light
application-icon background. The existing application identifier and
data location intentionally remain stable so the rename does not hide existing data.

## Remaining acceptance work

Native compilation does not prove recording reliability. On both operating systems,
test actual microphone permission, device removal, pause/resume, reopen/playback,
sleep/resume, low disk space, force termination and long recordings alongside a
meeting app. Windows NSIS installation and WebView2 provisioning need native checks.
Code signing, notarization and release-license review are separate release gates.

The current recorder has a RIFF WAV size limit and no automatic device reconnect.
It interrupts rather than silently claiming success on device/queue/write errors.
See the core README for queue capacity and file recovery details.

Diarization, domain dictionaries and reference-based summary context remain
future integrations. MOSS worker execution does not establish transcript accuracy or
speaker identities; model qualification and release acceptance remain separate work.


## On-demand summaries

Package the pinned native runtime before building the app (build-time Python only):

```sh
python3 scripts/native-summary/prepare.py --target macos-arm64
# On Windows x64, from the repository root:
python scripts/native-summary/prepare.py --target windows-x64
```

The builder downloads only the checksum-pinned llama.cpp b11734 runtime and its libraries
into ignored `apps/desktop/.generated/native/summary`. No model is installed by this
script. The resulting app needs no Python, Homebrew, Ollama or separately installed
llama.cpp. The summary runtime requires Apple silicon/macOS 13.3+ or Windows x64.
Windows packaging also copies the app-local Microsoft C++ runtime from the installed
Visual Studio redistributables (`VCToolsRedistDir`, auto-detected VS, or `--msvc-redist`).
Packaging fails if required DLLs are absent; end users do not manually install them.
A missing system Vulkan loader selects the CPU route without installing a GPU driver.
Windows includes separate Vulkan and CPU bundles; unsupported architectures
remain unavailable. A runtime build does not prove Windows execution or GPU support.

In the app, prepare the optional `요약 모델 다운로드` in Settings / Models, then
select `요약하기` on a meeting with an existing voice record.
`models/summary.json` pins the exact Gemma 4 E4B QAT Q4_0 revision, size and SHA256.
Weights are kept in the app's own data directory. No recording or transcript is sent
to a provider. First preparation requires network access; verified models run offline.

Only one processing job runs at a time. A summary worker binds to loopback with an
unpredictable per-process API key, uses a single slot, two CPU threads and low priority,
and is explicitly stopped and reaped on completion/cancellation/quit. GPU startup is
preferred. Only a confirmed unavailable GPU backend selects the bundled CPU route;
other engine or allocation failures retry with the same backend and settings. All
recovery paths share a single retry budget within the original job, after the direct
worker's exit is confirmed. New recording cancels/joins inference before capture.
Unconfirmed worker cleanup blocks new processing, recording and successful normal quit.
Unexpected process/OS termination is not a validated parent-death cleanup guarantee.

The initial context is 8,192 tokens, response allowance 2,048 plus a 128-token input
margin. The actual rendered template is tokenized. Long transcripts are partitioned by
whole utterances, all chunks are included, then their summaries are hierarchically merged without dropping late chunks. Merging
is bounded to eight levels and must reduce the number of parts. Oversized
utterances/non-reducing merge inputs, incomplete responses and invalid numeric source references
fail without overwriting a prior summary. Syntactic reference checks do not verify meaning.
Source numbers link back to the selected transcript; links are disabled after edits.

OS memory observations are advisory and never block or cancel a summary. macOS reads
an undocumented pressure sysctl on a best-effort basis; Windows queries the official
low-memory resource notification. Failed or unrecognized observations become `unknown`,
not task failures. Pinned runtime allocation diagnostics are distinguished from generic
engine failure; OS warnings and unexplained process death do not prove OOM. See
[runtime memory policy](../../docs/runtime-memory.md) for failure and recovery rules.

Summaries archive their exact source transcript, provenance and response receipt separately.
A changed transcript or competing result cannot overwrite the current summary. Failed or
cancelled runs preserve audio, notes, transcript and prior summaries. Accepted cancellation
and final commit are mutually exclusive; once commit begins, cancellation is rejected and
recording/quit waits for the job. Read/reopen/copy and
source playback are connected; manual summary editing and additional templates are not
implemented in this slice.


## macOS signing

Build an optimized app with `npm run tauri -- build --bundles app --ci -- --locked`.
The release profile retains symbols to avoid the macOS 27/Rust Mach-O stripping issue;
optimization remains enabled. Signing credentials are never packaged or committed.

`apps/desktop/scripts/sign-macos.py --app APP_PATH --output NEW_ZIP_PATH --notarize`
signs all nested native code with the `NERO_SIGN_IDENTITY` Developer ID identity,
then signs the app with the microphone entitlement. Notarization uses either
`NERO_NOTARY_PROFILE` or `NERO_NOTARY_KEY_PATH`/`NERO_NOTARY_KEY_ID`/
`NERO_NOTARY_ISSUER_ID` from the invoking environment. It requires Accepted status,
stapling, and Gatekeeper verification before producing the final ZIP. It never
replaces an installed app or uploads source recordings/model weights.
