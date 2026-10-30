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
`cargo test -p nero-core --locked`. Build/test one native target at a time.

## Windows installation and updates

Prepare both native packages before creating a Windows installer. These commands
download pinned runtime binaries, never model weights:

```sh
python scripts/native-summary/prepare.py --target windows-x64
python scripts/native-summary/prepare.py --target windows-x64 --verify-staged apps/desktop/.generated/native/summary --execute-runtime
python scripts/native-diarization/prepare.py --target windows-x64
python scripts/native-diarization/prepare.py --verify-staged apps/desktop/.generated/native/diarization --target windows-x64 --execute-runtime
```

Run them from the repository root. The summary packager needs a licensed MSVC
redistributable directory with the required x64 DLLs at or above the qualified
14.44.35211.0 baseline; `--msvc-redist` selects it
explicitly when the newest Build Tools installation lacks that component. The
speech packager preserves the CRT from its verified official archives. Do not
substitute arbitrary DLLs or infer compatibility from filenames alone. The summary
packager rejects older CRT metadata and copies only known CRT names. Verification
checks the exact package files and the pinned recipe; CI additionally starts both
Windows summary backends with `--version`. That startup check loads no model and
does not establish summary quality or GPU inference.

NSIS installs for the current Windows user and registers an uninstaller. Its
running-app check is replaced by `src-tauri/windows/hooks.nsh`: installation,
manual update and removal stop when the target executable is in use or the
Restart Manager check fails. Neither interactive nor silent mode may force-close
the app. An early GUI check also precedes maintenance paths which can invoke an
older uninstaller. Finish recording/processing and quit Nero before proceeding. This is a
point-in-time process check, not a lock preventing a later app start.

Initially, updates use a new verified setup executable for the same application
identity. Automatic updates are not implemented: there is no updater plugin,
public verification key or update endpoint. A future automatic route must retain
the app's normal save/quit boundary and verify signed update artifacts.
Downgrades across product versions are disabled. Rebuilds sharing `0.0.1` are
still the same NSIS version; the separate build receipt does not prevent an older
development build from being installed. Follow the first-user-sharing version
policy rather than claiming build-order protection.

Ordinary removal preserves the data directory, including recordings, transcripts
and downloaded models. Tauri's optional **Delete app data** checkbox permanently
removes the identifier's Roaming and Local data folders. It is not a separate
model-only cleaner and does not remove the separately prepared agent connections
or revoke OAuth credentials. Disconnect those integrations explicitly in their
own clients before a full personal-data cleanup. No new model-only or complete
cleanup interface is provided by this source change.

The packaged CLI and generated integration copies use different paths: prepared
connections retain generation-specific executables outside the app installation.
The Windows MSVC agent is built with static CRT linkage and CI inspects its PE
imports. A copied MCP/CLI executable must not need a separately installed MSVC
runtime; DLLs inside Nero's speech package cannot satisfy that standalone use.
They survive an app update/removal, and existing reader integrations can still
read retained meeting data. Mutations requiring the running app fail while it is
absent. Do not erase a host client's configuration from an installer.

The guard tests compile tiny NSIS fixtures and run both installation and removal
checks against an active process, an unopened file and a missing file. They do
not install Nero or access a real library. Set `NERO_NSIS_COMPILER` if `makensis`
is outside PATH and Tauri's normal cache, then run:

```sh
python -B -m unittest discover -s apps/desktop/tests -p test_windows_installer.py -v
```

The Windows CI prepares and verifies speech packages. A manually requested NSIS
build also runs the guard tests with missing-compiler skips forbidden. A compiled
fixture or source check does not establish full app installation, update, removal,
code signing, live client connection or 8 GB laptop acceptance.

References: [Tauri Windows installers](https://v2.tauri.app/distribute/windows-installer/),
[Tauri CLI 2.12.3 NSIS template](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.3/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi),
[upstream running-app helper](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.3/crates/tauri-bundler/src/bundle/windows/nsis/utils.nsh),
[signed automatic updates](https://v2.tauri.app/plugin/updater/).

For a macOS local test bundle, apply ad hoc signing after bundling and then run
`codesign --verify --deep --strict` on that bundle. This does not provide Developer ID
signing or notarization; use a separate distribution configuration for a release.

## Google Calendar registration

Release builds enabling Google Calendar require `NERO_GOOGLE_CLIENT_ID` and
`NERO_GOOGLE_CLIENT_SECRET` for the registered Desktop OAuth client. Load these from
the credential store through the build environment; never commit the credential JSON.
An incomplete registration leaves Calendar unavailable instead of opening a flow that
cannot exchange its authorization code. Tokens remain in the platform credential store.

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

For macOS distribution, build `app` from a clean commit with prepared workers and
sign the canonical bundle in place, then use:

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

For local replacement, use the canonical installer after normal application exit:

```sh
python3 -B apps/desktop/scripts/install-macos.py --app WORK/Nero.app \
  --dmg WORK/Nero.dmg --work WORK/installation
```

The installer verifies the release and idle writer, then renames apps on the same
filesystem with one rollback slot. It records data hashes and model metadata without
copying recordings, models or the data directory. After installed-app acceptance,
retire the rollback app and retain only current/previous notarized DMGs and small
release receipts. Unshared older builds do not require full binary archives.

The CLI/integration component version is owned by `agent/release.json`, independently
of the app build. Advance it when the CLI, protocol, skill or package templates change.
For a macOS UI-only release, `NERO_REUSE_SIGNED_AGENT_FROM=/path/to/Nero.app` reuses the
previous signed CLI only when the freshly compiled binary hash, target, API and all
generated integration files match its signed build metadata. The source app's full
Developer ID signature is verified; distribution signing preserves the exact CLI
bytes and requires the same signing team as the new app. Changes reject reuse.
This preserves actual live connection evidence without weakening generation checks.

`npm run tauri -- dev` starts a development app. `npm run dev` alone serves its
frontend, whose actions require the native bridge; it does not supply fake meetings.
`prepare:shared` copies canonical design-system files into generated build inputs.

## Current behavior

- Local CLI/MCP queries read meetings, folders, notes, corrected voice records and
  summaries without initializing capture, models or recovery. Explicit meeting edits
  use the running app's writer; template edits share the local catalog with settings.
  Revision checks protect both. Deletes use recoverable trash. Tauri builds bundle the
  native tool; see [local agent integration](../../docs/local-agent.md).

- Create meetings, edit title/notes and reopen them after restarting.
- Select an input device and explicitly start microphone recording.
- When no input device is available, recording stays disabled; connect a microphone
  and refresh the device list to enable it. Existing recording controls stay available.
- Pause/resume/stop and use a separate compact control window.
- Preserve raw PCM and prepare a separate WAV for playback.
- During new recordings, consume saved PCM in the background with persistent ASR
  and independent speaker states. Publish provisional text every 60–180 seconds of
  processed audio; pauses retain state and stop drains/finalizes once. Reuse completed
  ASR and speaker evidence through the existing source-bound publication path.
  Interim text has no fabricated word times/speakers and remains read-only.
  See [live speech contract](../../docs/live-speech.md) for limits and acceptance gaps.
- Recover complete stored frames after interruption; retain originals on failure.
- On Apple silicon with macOS 26 or later and packaged workers, import local audio
  through the native file picker, retaining the original bytes and separate playback WAV.
- Explicitly prepare pinned Nemotron 3.5 Q8 ASR and recognize each finalized recording
  through one stateful NeMo-Speech.cpp stream over the complete prepared WAV, with
  endpointing disabled and a finish call at EOF. Native output and execution evidence
  are retained; saving checks pinned identity, full input-frame identity, completion
  evidence and word timestamps. The core projects sentence rows without speaker labels.
  There is no silent chunk-ASR fallback. Original audio and previous results remain
  available after an unsupported or failed input. Inference uploads no audio.
- Optional machine-local terminology profiles create a separate correction revision
  after recognition. Uncertain names remain unchanged; an absent optional contextual
  model does not block recognition. See [terminology correction](../../docs/terminology.md).
- After recognition is saved and optional terminology correction finishes, independent
  Nemotron 3 speaker analysis processes the complete finalized recording in one
  stateful run using the shared native speech package. The source-bound continuation
  is durable under the same job ID; a speaker-stage retry does not rerun ASR. Failure
  or cancellation preserves the saved transcript and audio. Analysis preserves words/
  times/originals, commits against the unchanged source and leaves mixed or insufficiently
  covered utterances unassigned. Speaker management also supports explicit reanalysis;
  target-hardware accuracy and Windows acceptance remain separate checks.
- Recognition and speaker analysis share an immutable analysis-input contract: mono,
  16kHz PCM16 WAV. Supported normalized files are verified and read directly, including
  on Windows without a platform converter. Other Windows WAV inputs use a bounded
  hound/rubato converter for channel averaging, anti-alias resampling and PCM16 output.
  Windows imports currently accept WAV only; compressed inputs are rejected before copying.
  macOS retains its packaged converter. Automatic speaker analysis reuses recognition's prepared input. Standalone
  analysis/retry prepares and verifies the same format, preserving original audio and
  checking original/prepared hashes before save. Container format is checked as well
  as rate/depth; EXTENSIBLE WAV is re-encoded rather than copied by the converter.
  Typed input, engine, storage and cleanup failures remain separate in the UI.
- Edit transcript text, seek to a segment, and reopen corrections after restarting.
  Navigation saves and closes the editor; failed persistence retains its input.
- Confirm re-transcription and restore a verified previous recognition run with version
  checks. This preserves original audio and raw/initial recognition output.
  Existing MOSS results, edits and history remain readable/restorable with their
  original model revisions and chunk/recording scope. Their whole-input EOS and
  completeness contract is retained; existing meetings are not automatically rerun.
- Search matching context in the library or highlight/move through literal matches in
  a meeting. Speaker changes on one utterance include replay; full speaker management
  keeps its actions outside the scrolling content.
  Raw worker output and correction revisions are retained separately. Cancelling or
  failing a rerun preserves the previous transcript.
- Hide the main window without ending recording. Explicit quit flushes notes and
  finalizes the active recording after confirmation.

The application identifier is `io.github.choseongmin1128.nero.desktop`.
Tauri resolves its own OS application data directory. No automatic legacy migration,
automatic model installation or account connection occurs. An explicit Google Calendar
connection stores its grant in the platform credential store.

## First use

New meetings, new recordings and file import require speech recognition to be
prepared. Initial setup includes Nemotron 3.5 Q8 ASR and the smaller Nemotron 3 speaker
model when its runtime is supported. Meeting summary is optional: select its initially
unchecked choice to include it in the initial download, or download it separately
later. Preparation starts only after a click, verifies each model and retains completed
models and partial downloads.
Normal controls show no model sizes. Available disk space is checked before transfer
and periodically while downloading; insufficient space surfaces a recovery message.
The UI calls the generated text "음성 기록" and the summary "회의록".

Saved meetings, notes, playback and existing results remain accessible when models
are unavailable. Active recording pause/resume/stop does not depend on readiness.
Missing platform workers show an unsupported state instead of offering a download;
the current complete meeting pipeline therefore still requires Apple silicon/macOS 26.

## Appearance

The shared design-system adapts pinned shadcn/ui Base UI primitives for select,
date/popover and the agent connection dialog. React owns these dedicated roots;
the native input events and existing app controllers remain the interface for saving,
recording and processing. The component catalog builds the same source. Upstream MIT
and runtime license notices are included in the generated app assets.

Settings / Screen exposes six color themes and independent Light / Dark / System
brightness. The main app and compact recorder use the same shared appearance code
and app-local preference. System follows the OS; an explicit brightness remains fixed.
Theme changes do not change meetings or model preparation.
Large surfaces retain a warm neutral tone across accent colors. Recent meetings show recording
or creation time; opening a detail selects only that meeting in the recent list.
Returning to the library restores its previous folder filter.

Settings / Connections offers direct Claude/Codex installation and a separate action
for other agents. The latter opens a dialog containing the full connection prompt,
copy action and destination instruction. It does not change external client settings.
An existing integration that needs a newer package shows an update action. Preparing
the package does not complete the update; a verified current initialization does.
Nero retains that observation independently of a live connection. Codex catalog paths
stay stable while older generation executables remain readable. Client metadata
observation failures have a query-only retry and do not block another integration.

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

## Summary workspace

Completed meetings open on the summary. Header actions open the full voice record or
notes alongside it in a non-modal inspector. The voice record remains one continuous
editable document with its existing search, speaker/history, copy and export controls.
The summary uses its available width rather than a fixed character-width cap.
The inspector starts at the detail header; closing it preserves the summary position.
Narrow available detail space uses a single surface with a return action and shared
playback control rather than dimming the summary behind a modal.

A summary citation opens this same voice record and highlights verified current
utterances without playing audio. Source identity, current revision and meeting identity
are checked before navigation. If the original utterance no longer maps to the current
record, its summary-time text remains readable in the inspector. Citation numbers or
nearby timestamps are never used to guess a current row. Existing audio, corrections,
notes and summary archives are unchanged by the workspace layout.

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
input and disables preference changes until it ends. Settings / General provides inline
optional summary preparation and download progress without a separate model page.

## Brand assets

The application and tray icons use the approved Nero 23-B r7 A mark.
`brand/mark.svg` owns the geometry; `brand/app-background.svg` supplies the light
application-icon background. `npm run prepare:icons` composes these for macOS/Windows
app assets using the pinned Tauri CLI. Tray assets retain the transparent mark. macOS uses a monochrome menu-bar template.
Windows switches the monochrome tray asset on reported window-theme changes;
contrast on separately configured taskbar/application themes needs Windows validation.

## Native speech workers

From the repository root, stage the pinned shared NeMo-Speech.cpp package and compile
AudioPrepare before the Tauri build:

```sh
python3 scripts/native-diarization/prepare.py --target macos-arm64
apps/desktop/scripts/prepare-native.sh
```

The shared package supplies ASR and diarization executables, libraries, Metal resources
and licenses once under `.generated/native/diarization`. Preparation/build/signing
verify pinned archive provenance, target and file hashes; signing preserves separate
post-signing hashes. The worker destination must be new; existing packages are never
silently replaced. End users need no compiler, Python or separate CLI. The recorder
retains its macOS 13 minimum; AudioPrepare and the complete recognition path currently
require Apple silicon/macOS 26. Absent or unsupported workers leave recognition unavailable.

The same preparation tool supports `--target windows-x64` with app-local Microsoft C++
runtime files as described in [the runtime contract](../../scripts/native-diarization/README.md).
It packages separate Vulkan and CPU bundles. The Windows app prepares WAV through its
portable converter; compressed imports and end-to-end hardware acceptance remain unavailable
or unqualified respectively.
ASR and diarization prefer GPU; only confirmed GPU-backend unavailability permits the
CPU route. Allocation or generic runtime errors do not trigger a CPU fallback.

ASR model files and hashes are pinned in `config/models/nemotron-asr.json`, and the
speaker model in `apps/desktop/models/diarization.json`. Models remain outside the app
bundle in its own data directory. Preparation starts only from the app's explicit
model action. The downloader retains interrupted partial files, resumes verified Range
responses and checks the complete file SHA256 before publishing a readiness marker.

Retained MOSS packaging remains available through
`scripts/native-moss-harness/README.md` and the three-argument `prepare-native.sh` route.
It does not replace the shared speech package or make MOSS the default ASR. Stored
MOSS whole-input output keeps its EOS/budget/complete-input validation and versioned
history; loading it does not start recognition or download a model.

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

Automatic voice identity matching and reference-based summary context remain future
integrations. Native worker completion does not establish transcript accuracy,
eight-person diarization quality or speaker identities; model qualification and release
acceptance remain separate work.


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

In the app, add the optional `회의 요약` feature in Settings / General, then
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

The context remains 8,192 tokens with a 128-token input margin. The actual rendered
system/user template is tokenized before each request. Short input uses a direct
summary with its remaining output space. Long input uses source-bound fact extraction,
explicit later-change linking, agenda grouping and complete agenda sections. Each
structured request reserves 2,048 output tokens; input or output overflow partitions
only the affected work. Oversized single utterances are divided at UTF-8 boundaries
without changing their canonical source number or stored transcript.

Every structured result requires a normal stop, schema validation and valid original
source references. Agenda grouping accounts for every extracted topic; section writing
requires one sentence under each fixed fact ID. The app retains the extracted fact's
category and original source references; the writer cannot reassign IDs or promote a
report into a decision. It assembles completed sections without one final model rewrite. This
prevents the final document from competing with all intermediate summaries for one
output window. Structural coverage and a normal stop do not establish factual accuracy
or guarantee that extraction retained every important statement.

Validated stage responses are checkpointed under each meeting's `summary-work`
directory. Identity binds the frozen transcript, notes, pinned model/runtime, prompts
and request settings. Retry reuses matching complete results; changed input invalidates
reuse. Incomplete or corrupt responses are never reused, and corrupt files are retained
when replaced. Meeting deletion includes these private intermediate files. The source,
notes, audio and earlier summary remain untouched until the existing final commit gate.
References retain original 1-based segment order and are rendered in bounded clickable
groups. Source changes preserve the frozen archive rather than rebinding old numbers.

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
source playback are connected. Local summary generation selects the same built-in/user
template catalog as CLI/MCP. It freezes the selected revision before enqueue and verifies
it again before execution. A changed/deleted selection fails without replacing an earlier
summary. Long-input writing assigns headings/order under accounted fact IDs, then assembles
template headings before agenda subheadings. Template changes invalidate stage reuse.
Structural completion tests do not establish model instruction-following or factual quality.
Manual editing in the app remains separate; explicit agent summary import is available.


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
