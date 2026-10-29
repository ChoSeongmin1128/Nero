# Grove handoff

AI-assisted working document. Current user instructions, live state, source and tests
take precedence. This public handoff describes code, not a user's installed app,
machine, recordings or evaluation results. Machine-local installation receipts and
detailed validation logs remain in ignored `.work/`. The root `HANDOFF.md` points to
the private installation and evaluation receipts.

## Release state

Nero 0.0.1 build 28/agent23 has signed/notarized macOS arm64 artifacts and is locally
installed. The canonical installer replaced only the app by rename under the writer
lock. Existing library hashes, model metadata and installation-time connection state
were preserved without copying audio, models or a full data snapshot. Native settings
navigation, copy-format menus, citation defaults, focus restoration and existing meeting
access have been observed. The rollback app was retired after acceptance; current/previous
DMGs and small provenance receipts are retained.

External agents can read a summary context and explicitly save their generated summary
through the running application's writer. Source/notes/current-summary checks reject
stale output, prior summaries remain archived and open summaries reload after writes.
Host package updates and actual live tool acceptance remain separate from app packaging;
client configuration was not rewritten. Windows-device and model-quality qualification
remain separate. Source commits and hashes are in [Release history](release-history.md).

Build 28 source tests and installed-screen checks do not establish new microphone
capture, Google account authorization/reminder delivery or Windows-device qualification.
Host package installation and existing live sessions remain separate; a current enabled
installation remains connected while per-conversation sessions start or stop. Older-only
sessions have a new-conversation notice without requiring reinstall or whole-app restart.

## Notarized candidate

Build29/source91a8561 retains the signed agent23 payload and improves stable installation
readiness, ChatGPT naming and Google Desktop authorization/callback completion. App/DMG
notarization is accepted with verified embedded-app identity. Installation remains pending
while the installed app records. Actual Google reauthorization, callback auto-close and
reminder delivery remain distinct from mocked/endpoint and unit checks.

## Native rewrite source

- Meeting lists use direct checkbox/modifier selection without a selection mode.
  Visible order owns range selection; selection count owns the header actions.
  Toolbar icon controls, contextual deletion names and confirmation boundaries
  follow the shared design-system rules.

- The shared UI foundation uses pinned React/Base UI primitives adapted from the
  MIT shadcn/ui registry, with Korean DayPicker calendars and Nero's semantic tokens.
  React owns dedicated select/date/dialog roots; existing recording, persistence,
  processing, revision and playback controllers retain their ownership. The app and
  static catalog build the same canonical source rather than maintaining two controls.
- Neutral surfaces are independent of the six accent colors. Recent meetings use
  recording/creation order; detail navigation selects only the current recent meeting,
  retaining the previous folder filter for returning to the list. The summary remains
  the primary document beside a non-modal voice record, with compact playback and
  labeled icon actions. No unimplemented summary-template selector is displayed.
- Settings > Integrations keeps direct Claude/ChatGPT installation and connection
  evidence. Other agents open a connection dialog with the full prompt, copy action
  and paste destination together. Its code area scrolls independently of the instruction;
  copy failure and unavailable connection metadata have explicit recovery states.
- Connection preparation and verified initialization are separate. Verified enabled host
  installation represents connection readiness independently of lazy per-conversation
  sessions. Inactive sessions do not change a current connection's label or restore a
  recheck button. Older-only sessions explain that the update is complete and applies
  to new conversations. Unknown host installation still requires current live evidence.
  Explicit status checks refresh bounded host observations. Client-specific observation errors do not block
  the other row, and retrying an observation never runs installation. Codex uses a
  stable catalog location pointing at preserved generation-specific executables.
  Catalog metadata alone does not establish a running client's installed version.
- Summary-only reading uses the available detail width and shrinks when its inspector
  opens. Shared neutral surfaces use the previous warm gray palette; accent variants
  remain independent of the large background planes.
- Settings share grouped rows with aligned labels, current values and controls across
  general, recording, integrations and summary templates. Optional summary preparation
  is an inline general-setting row. Confirmed connections omit a redundant
  connect button and restore it when reconnection is needed. Tab keyboard navigation,
  focus restoration, native connection evidence and preference persistence are retained.
  Browser layout validation does not establish installed-app or Windows-device validation.
- Whole voice records and summaries share native TXT/Markdown copy/export formatting. Summary
  output omits validated internal numeric citations by default; users can include them.
  Canonical summaries, citation sources and spoken numeric text remain intact. External
  template context carries the same output policy independently of user-edited instructions.
- Calendar integrations use a desktop public-client PKCE grant and the platform credential
  store. Selected Google calendars supply a compact five-minute reminder while Nero runs.
  Recording starts only from an explicit reminder action; no helper wakes a quit app.
  Calendar-state corruption is isolated from the meeting library. Complete Desktop
  client configuration is required before offering connection. The callback renders
  shared Nero styles, waits for token exchange and durable connection state before
  success, returns focus to Nero and attempts to close the tab. Browser restrictions
  retain a clear completion message; failures stay visible with an actionable reason.
- Recording controls remain visible throughout capture when enabled, including while
  the main window is foreground. Closing the control hides it for that capture only.
  Bounded capture diagnostics record observed writer progress rather than inventing a
  lost-audio interval. Malformed optional diagnostics do not corrupt completed meetings.
- Recording policy is separate from the processing slot/queue. Downloads survive capture
  startup, inference is joined, and persisted jobs never automatically rerun at startup.
  Source-bound retry, durable cancellation intent and uncertain-saving review preserve
  previous results. Alerts can be acknowledged without changing outcomes/history.
  See [Task ownership](task-management.md).
- Individual session-manifest corruption no longer blocks all meetings. Originals and
  valid playback remain intact; zero complete frames are not reported as recovered audio.
- Summary input freezes meeting notes with transcript/current-summary identity. Notes
  have their own hash, archive, CAS and stale checks; legacy transcript hashes keep their
  meaning. Whole/map/merge prompts treat notes as reference data, not meeting evidence.
- Folder drag uses a bounded single-line title preview, with a count for multiple
  meetings. It omits full row metadata, clears temporary DOM on completion/cancel,
  and revalidates placement/destination after edit flush. Windows HTML drag uses
  `dragDropEnabled: false`; import keeps the native picker. Recorder physical placement
  recovers after display changes while preserving visible user placement and focus policy.

- Default recognition uses the pinned Nemotron 3.5 Q8 GGUF and NeMo-Speech.cpp
  runtime. One stateful stream consumes the complete prepared WAV with endpointing
  disabled and finishes at EOF. The adapter preserves native output and execution
  evidence separately; the core checks model/runtime identity, full input-frame
  identity, native completion and valid word timestamps before saving sentence rows.
  ASR does not invent speaker labels. A successful process/stream finish is not proof
  that every spoken word was recognized. Cancellation serializes with the baseline
  save and explicitly reaps the worker. Existing recordings are never automatically
  rerun. See [Native speech contract](../scripts/native-diarization/README.md).
- Saved MOSS raw output, corrections and recognition history keep their original
  identities and remain readable/restorable. Its whole-recording artifacts retain the
  `nero-moss-whole-v2` runtime/schema-2 EOS, measured-budget, single-generation and
  complete-input validation contract.
  Existing v1 artifacts use the validated stored-artifact policy, not a new-worker
  acceptance path. MOSS is not a fallback for new recognition. See
  [Retained MOSS worker contract](../scripts/native-moss-harness/README.md).


- After durable recognition and optional terminology correction, the default route
  runs independent whole-recording Nemotron 3 speaker analysis through the shared
  NeMo-Speech.cpp package. Its source-bound continuation is persisted under the same
  job ID; a speaker-stage retry does not rerun recognition. It preserves raw output/
  provenance and rejects stale source revisions. V3 frame probabilities are collected
  before native compaction and projected with the pinned word-onset argmax rule.
  Word turns form a new validated generation; original ASR, text corrections, manual
  speaker edits and old generations are preserved. Failure/cancellation retains the saved transcript and audio.
  Speaker management also supports explicit reanalysis. Target-hardware accuracy and
  distribution acceptance remain unqualified; this is not a release/installation receipt.
- The shared speech-input adapter binds original and normalized audio by hashes and
  verifies mono/16kHz/PCM16 WAV, including the container tag. Automatic speaker analysis
  reuses recognition's prepared input; standalone analysis/retry uses the same contract.
  Already-normalized input needs no platform converter. The native converter re-encodes
  EXTENSIBLE inputs instead of preserving an unsupported container. Failures retain
  typed input/engine/storage/cleanup categories and safe speaker-specific UI messages.
- Speaker display names record automatic/user origin when known; missing legacy origin
  remains unknown. Summary input separates chunk scope from whole-recording groups,
  never derives an attendee roster/count from labels, and allows user-assigned names
  to reference first-person commitments without claiming verified voice identity.


- Summary generation measures the remaining output budget inside the unchanged context,
  preserves typed output-limit failures and repartitions only truncated source ranges.
  Request/outcome artifacts retain phase and token evidence. Normal feedback uses a
  single stage label without internal utterance counters or advisory memory warnings;
  failures expose a cause, retry action and collapsed diagnostic details. Prior-summary
  preservation text depends on an actual prior result. See [Runtime policy](runtime-memory.md).

- Optional machine-local terminology profiles run after baseline recognition and create
  separate bounded correction revisions. Contextual names require nearby literal cues
  and an acceptance from an already prepared optional model; uncertainty keeps the
  original. Acronym formatting does not require that model. The profile, source and
  decisions are preserved; concurrent manual edits prevent automatic replacement.
  Recording settings exposes the toggle only when a local profile exists. See
  [Local terminology correction](terminology.md). This source contract does not state
  that the installed app or existing meetings have changed.

- Settings > Connections opens a native Claude Desktop MCPB or the Codex local plugin
  installation page. Client registration, install files and a live initialized MCP session
  are distinct: only the matching live session of the current package is shown connected.
  Agent and template changes enable a package update; copies preserve the signed CLI and
  do not depend on the app staying at its initial path. Session management files live
  outside the meeting library. Existing client settings are owned by the client's install
  flow; advanced manual config/CLI copy remains available. Native host acceptance and
  Windows execution are separate validation gates. See [Local agent integration](local-agent.md).

- CLI and stdio MCP queries share `LibraryReader`, without opening the writer,
  recovery, migration, microphone or model paths. Folder/date filters, IANA timezones,
  version-bound paging and byte limits keep queries scoped. Template lists return metadata;
  the selected template returns the current user prompt. Explicit template writes use the
  catalog lock/revision, and meeting metadata/folder/trash mutations use the running app's
  authenticated writer bridge with edit-version checks and GUI draft flushing. Meeting
  deletion is recoverable trash; permanent purge is not exposed. The host client owns tool
  approval policy. Settings > Summary templates manages defaults and custom prompts for
  connected agents; this does not select a template for the internal summary pipeline.
  See [Local agent integration](local-agent.md) for contracts and platform boundaries.

- Rebuilding an existing voice record requires confirmation; verified previous runs are
  available in Previous records. Restore compares both current and target versions,
  makes a new revision, preserves raw/initial/audio, and leaves the existing summary stale.
  History lists the latest corrected document for each recognition run, not every edit.
- Content search shows matching context in the library and opens the matching section/
  utterance. Detail search highlights literal matches and moves through transcript,
  summary and notes. Transcript search anchors validate version and segment identity;
  delayed reads cannot replace a newer saved/restored document.
- Speaker changes on a reading row edit only that utterance, with range-limited replay.
  Full speaker management separates names/merges from utterance assignment. Its footer
  remains outside the single scrolling body. Completed meeting headers/player/toolbars
  are compact; settings keeps fixed navigation and dimensions across categories.
- Shared summary rendering separates headings, paragraphs and lists, escapes raw markup,
  and groups frozen source references into compact evidence controls. Korean text wraps at word boundaries with
  a fallback for long unspaced text. Month navigation preserves a keyboard date entry
  without changing the selected date until confirmation. Familiar secondary actions
  (copy, search, settings, close and search navigation) use shared icons with Korean
  accessible names/tooltips; generation, export-format and destructive actions retain text.

- Meeting details keep the summary as the primary reading surface. Header actions open
  the full voice record or notes in one non-modal inspector; the summary stays readable
  beside it. Only a narrow available detail area switches to a single inspector surface
  with a return action. Playback uses the same media state in both layouts.
  Existing edits, exports, speaker/history actions, search and per-meeting reading
  positions retain their existing owners. Search tools remain outside scrolling text.
  Clicking a summary reference opens that same full record and reads its frozen archive.
  Only verified exact-source or direct-parent word-turn lineage permits current-row
  navigation. Matched utterances are highlighted without playing audio. Unmapped
  references retain the summary-time text in the inspector; current text remains
  accessible. Invalid archives never fall back to ordinal/time proximity. Storage,
  summary generation and recognition models are unchanged by this layout.
- Meeting management now includes single-folder placement, batch move, trash/restore,
  permanent deletion with tombstone recovery, and content search. Speaker names and
  assignments share transcript revision checks and preserve raw results/corrections.
  Source identity is explicit (raw hash, initial IDs/order, source index); legacy
  upgrades run as a separate startup operation with immutable before/after evidence.
  Ordinary reads do not reconstruct or migrate. Workspace JSON float parsing preserves
  persisted coordinates; timestamps are checked as ASR sample addresses, never epsilon identity.
  Old MOSS labels retain their original independent chunk or verified recording scope.
  New Nemotron ASR has no speaker labels; the separate whole-recording stage supplies
  recording-scoped assignments. Explicit reanalysis can replace assignments while
  preserving text. Qualified eight-person diarization remains unverified. See
  [Meeting library and speakers](meeting-library-and-speakers.md) for storage, cancellation,
  recovery and validation boundaries; this source contract is not an installed-app receipt.

- Trash has an Empty trash action independent of current search and selection. Its
  confirmation freezes IDs/deletion timestamps, revalidates before submission, excludes
  newly trashed meetings, and uses the existing protected purge path. Busy feedback,
  failure refresh and focus restoration remain available; the CLI/MCP expose no deletion.

- Before the first actual user sharing, architecture and stored formats may change.
  Personal meeting data still requires explicit preservation/migration. The first
  user-shared build establishes the compatibility baseline; notarization or personal
  installation alone does not establish it.
- Native voice records default to compact reading rows, with timestamps for playback.
  Only the selected row becomes an auto-sizing editor. Completion waits for persistence;
  failure keeps the editor/input, and cancellation restores the last saved text.
  Successful save before navigation also closes the editor, so returning cannot expose
  a stale cancellation action.
  Original recognition text is disclosed only on corrected rows. Shared markup/styles
  live in `design-system/components.js` and `components.css`.
- Processing shows a shared activity indicator and actual backend stages, with real
  byte progress for model downloads only. Cancellation stays active/disabled until
  the backend finishes. Inference does not present an invented completion percentage.

- Nero release numbering starts at 0.0.1 with an independent macOS build number.
  `docs/versioning.md` owns data/version compatibility rules and
  `docs/release-history.md` records exact source commits and distribution status.
  The version helper checks all app/Rust/lockfile mirrors before builds. Distribution
  resources preserve the source commit/tree and dirty state; release signing requires
  a clean matching source. macOS packaging can produce separately signed/notarized
  ZIP and DMG artifacts, with distinct receipts and read-only embedded-app verification.
  macOS build/preparation and sign-time gates reject a present retained MOSS worker whose
  canonical sources, lock, patch receipt, resources or executable have changed. The
  shared speech package verifies pinned archive provenance, target and staged file
  hashes; signing retains separate pre/post-signing hashes and checks its runtime version
  without loading models. MOSS signing also runs its no-model runtime metadata check.
  Developer ID signing preserves the original
  ad-hoc provenance and records separate pre/post executable hashes, signed identity,
  runtime metadata and package/app tree digests. These bindings do not establish
  binary/source semantic equivalence or model quality. An absent shared speech package
  leaves ASR/diarization unavailable; a retained MOSS package cannot substitute for it.
  Windows builds do not require the unsupported MOSS runtime.

- Settings contains General / Recording / Integrations / Summary templates. Shared appearance behavior is packaged
  for the main, recorder and reminder windows, with six colors and independent Light / Dark / System brightness.
  The native app uses its own saved preference instead of forcing green on every launch.
  Meeting title/notes autosave without a permanent Saved label or manual Save button;
  pending feedback and failed-save recovery remain available.

- Initial setup gates creation, recording start and import on speech recognition
  readiness. Meeting summary is optional, unchecked initially and downloadable later.
  Setup includes the smaller pinned Nemotron 3 speaker model after the Nemotron 3.5
  Q8 ASR model when the speaker runtime is supported.
  Preparation is explicit/cancellable; selected models download sequentially. Available
  disk space is checked before and during transfer, with partial files retained. Normal
  controls omit model sizes. Existing meeting access and active pause/resume/stop stay
  available. Unsupported runtimes cannot appear ready by downloading files alone.
- User-facing results are "음성 기록" and "회의록". Before recording, the detail view
  exposes recording and notes; empty result sections appear only when their inputs
  exist. Initial preparation has its own screen; a missing optional summary model can
  also be downloaded from the meeting summary section.

- Playback uses Korean app controls rather than native webview media controls. Text
  and summary can be copied or exported as UTF-8 TXT/Markdown through a native save
  dialog, after pending edits finish saving. A stale summary carries a source-change
  notice in exported output. Selected files are replaced only after a complete temp
  write. Optional summary-model downloads show percentage and do not block new meetings.
  The macOS app declares Korean localization and translates its existing standard menu;
  Windows retains its previous menu configuration.

- The approved Nero mark is 23-B r7 A, with source in `apps/desktop/brand/mark.svg`.
  Desktop app icons compose it with the approved light `brand/app-background.svg`;
  macOS menu-bar uses a transparent monochrome template and
  Windows tray selects monochrome assets from the reported window theme. Icon
  source/build integration does not establish Windows shell appearance or installation.

- `apps/desktop` is the new Grove Tauri app, with identity
  `io.github.choseongmin1128.grove.desktop`. It uses only its own application data
  directory. Legacy app/data archival receipts remain private.
- `crates/grove-core` owns meetings and recording on a dedicated command thread.
  A bounded callback queue and separate file writer retain native PCM; finalization
  creates a distinct float32 WAV. Original audio survives conversion failures.
- Current UI supports meeting creation/content search, folders and trash/restore,
  title/notes editing, microphone choice
  in Settings / Recording (persisted app-local preference),
  pause/resume/stop, finalized playback and a 236 x 44 draggable transparent control window.
  Hiding/minimizing the main window first shows the control window. It then stays on top
  for the active recording despite main-window restoration, until stop, explicit dismissal
  or an opt-out in Recording settings. Default names receive free numeric
  suffixes; meeting date is separate from real microphone recording time.
  Sidebar timestamps include local 24-hour time and sort by the displayed recording
  or creation timestamp. Their tooltip identifies its meaning and the separate meeting day;
  importing a file does not invent an original recording timestamp.
  Shared controls, fonts, icons and appearance come from `design-system` at build time.
- An empty input-device list disables new recording and explains how to refresh after
  connecting a microphone. It does not disable pause/stop for an existing session.
- Notes are flushed before switching, hiding or quitting. Delayed saves cannot discard
  newer input. Normal recording completion starts recognition once for its finalized audio session;
  cancellation preserves audio and does not schedule another automatic attempt.
  macOS microphone authorization settles before CPAL opens an input device.
  Capture storage is activated before the stream starts. Recoverable overload,
  scheduling and route notices retain recording with persisted quality warnings;
  terminal device/storage/queue failures still interrupt. Exact missing-frame counts
  cannot be inferred from those notices. Compact-window creation failures are reported
  once per state transition and allow a deliberate retry after restoration/settings.
  A native atomic shutdown operation checks the active session and fences
  new recording starts before the app exits.
- Apple silicon/macOS 26 can import audio with the native picker when AudioPrepare and
  the shared native speech runtime are packaged. Original bytes/hash, normalized PCM
  and playback remain separate. The base recorder retains its macOS 13 minimum.
- Explicit model preparation downloads the pinned Nemotron 3.5 Q8 ASR snapshot and
  the smaller Nemotron 3 speaker model, checking each size and SHA256 before readiness.
  The ASR manifest is `config/models/nemotron-asr.json`; the retained Swift MOSS catalog
  remains separate. Native executables, libraries and Metal resources are packaged;
  runtime developer tools and audio uploads are unnecessary.
- Transcription binds complete output to the finalized audio session. Raw output,
  initial transcript and correction revisions remain separate. Stale edits/reruns cannot
  replace current corrections. Text editing, timestamp playback and reopen are connected;
  UI transitions and quit flush pending edits, including composition completion.
- Processing is cancellable. Normal quit cancels/joins workers before shutting down the
  core. Failure or cancellation keeps audio and prior transcripts. Interrupted model
  downloads are retained; validated partial-byte resume and full-file checks are implemented.
- Model hash verification checks cancellation between read blocks. The development
  profile optimizes SHA256, and PCM import uses buffered reads/writes with an explicit
  flush and sync before finalization.
- Synthetic tests cover ownership, persistence, queue overflow, partial-write failure,
  recovery, transcript version conflicts, cancellation and delayed UI saves. These tests
  do not establish actual microphone capture, power-loss durability, long-duration
  performance or Windows behavior.
- The current product name is Nero; its existing identifier/data location remains stable.
  The repository was renamed separately; retaining internal grove crate/event names is
  intentional compatibility, not a second installed product. The approved mark is 23-B r7 A.
- Packaged llama.cpp b11734 supports explicit preparation of pinned Gemma 4 E4B Q4_0 and
  on-demand local summaries. GPU startup is preferred; confirmed GPU backend unavailability
  permits a bundled CPU fallback.
  Per-process loopback authentication, serial processing, cancellation/quit/recording-start
  worker teardown and bounded token budgets are connected.
- OS memory observations no longer gate summary startup or execution. macOS uses an
  undocumented pressure sysctl as a best-effort observation; Windows uses the official
  low-memory resource notification. Read failures become `unknown`, not task failures.
  Strict pinned runtime diagnostics distinguish allocation failure and unavailable GPU
  backends from generic engine failure; OS warnings or unexplained exits do not prove OOM.
  Recovery is at most one retry in the same job after the direct worker is stopped/reaped.
  Allocation and generic engine failures retain the backend and generation settings;
  only confirmed GPU backend unavailability permits CPU fallback. Cleanup failure blocks
  subsequent processing/recording and successful normal quit. Cancellation prevents a retry;
  accepted cancellation and final summary commit are mutually exclusive. See
  [runtime-memory.md](runtime-memory.md). Synthetic worker/loopback coverage does not
  establish real OOM recovery, Windows execution or 8GB/16GB hardware qualification.
- Summary records retain source revisions, model/prompt provenance and receipts. Changed
  sources or competing results cannot replace current summaries. The UI reads/copies them
  and links valid references to transcript playback, with stale-source links disabled.
  Long input uses source-bound fact extraction, bounded global change linking, agenda
  grouping and complete section writing. The app assembles sections without another
  whole-document model rewrite. Input/output overflow partitions affected work; every
  topic/fact must be accounted for and source IDs are app-derived from validated inputs.
  UTF-8 splits preserve the original source number for oversized utterances. Complete
  validated responses are checkpointed in meeting-owned summary-work, bound to frozen
  source/notes/model/runtime/prompts/settings, and reused on retry. Partial/corrupt
  responses are not reused. Original audio, text, edits and summaries retain the same
  final CAS/cancellation boundary. Structural coverage does not establish semantic
  summary quality or Windows/low-memory hardware qualification.
- Model preparation now resumes validated Range responses and verifies full-file SHA256.
  Existing partial/invalid files are preserved; no automatic preparation or private uploads.
- Cross-chunk identity matching, Calendar and Notion remain unconnected. Windows has
  pinned Vulkan/CPU speech runtime packages, but the current AudioPrepare path requires
  Apple silicon/macOS 26. End-to-end Windows recognition is currently unavailable;
  native acceptance remains unqualified. No weights are downloaded automatically.
  No notch panel or platform-specific nonactivating floating-window contract is implemented yet.
- A macOS development bundle builds. Windows-native CI is prepared separately;
  actual Windows execution, installer, permissions and driver tests remain pending.
  This is not a release distribution or replacement for qualified recording tests.

## Legacy Swift source

- One workspace window owns navigation, its toolbar and recording / import / file-management
  dialogs. Recording and import begin after setup dismissal. Empty transcript states accept
  the available height; they must not enlarge the split view beyond the window.
- Pending microphone permission and file-copy work have explicit ownership. Cancelling
  an earlier setup cannot start or clear a newer attempt. Failed index writes restore the
  previous selection and keep source audio.
- Recording-device completion and encoding-error callbacks are bridged from background
  queues to MainActor and bound to the current capture ID. Unexpected stops finalize the
  recorded file, clear active capture and persist an interrupted meeting without starting
  transcription. Enrollment failures discard their temporary capture and allow retry.
- New recordings are captured as `<id>.capture.aac` (ADTS), which stays readable after a
  crash. After stop, or at the next launch for an interrupted capture, the AAC packets are
  repackaged without re-encoding into `<id>.m4a`; the index switches only after the frame
  count matches, and the capture is removed afterwards. Failure keeps the capture as the
  meeting audio. Imported `.aac` files are not repackaged.
- If transcription cannot start after a recording or import, the meeting is marked failed
  instead of staying in processing until the next launch.
- Recording and processing hold an idle-sleep activity assertion. Ninety seconds of digital
  silence (at or below -120 dB; an all-zero input reads -160 dB) shows a warning in the
  recording bar; pausing or any live signal resets it.
- Playback observes the original file's finalized state and source paths. A recording
  remaining open through completion gets a prepared player without navigating away;
  transcript edits do not reset playback position or rate.
- Per-meeting recording, import and retranscription validate the selected plan before
  microphone permission, file copying or changing prior result metadata. Setup shows
  required preparation and links to Settings; default options do not stand in for the draft.
- Calendar permission responses are owned by the latest enable/disable request.
- Enrollment inspection uses at most the first120 seconds, allowing timer-stop latency
  without rejecting an otherwise valid recording. Rich-text export preserves inferred
  speaker labels, and an asynchronous Notion task is not a confirmed created page.

- Team voice enrollment records prompt reading and free speech, checks amplitude / clipping,
  supports preview and stores encrypted per-span features. Enrollments made with another
  voice model are listed as **재등록 필요**. Policy decisions keep the deciding query's best
  and runner-up scores even when no name is given; the qualification harness records them
  per query over a 0.60 to 0.90 similarity and 0.05 to 0.20 margin grid. The optional 15.3 MB voice
  model has pinned hashes and its own CoreML readiness check. Automatic naming remains
  gated; independent other-day five-person evaluation data is still required.
- Optional attendance is remembered per folder and stored separately from speaker-count
  options. Calendar recording opens the same meeting setup. Anonymous split clusters
  can share an identity if no retained overlap evidence conflicts.
- Calendar reminders use a single 440 x 40 row with title, countdown and recording action.

The source targets version 0.4.0 / local-beta.27. This is a personal local beta, a
source build whose signing / notarization status must be checked from local receipts. Consult `App/Info.plist` for the
build identifier and local receipts for actual installation state.

- Basic transcript and accessibility time labels show whole seconds. Stored playback
  and split boundaries retain fractional precision. The metadata column is112pt
  (148pt for hour-long timestamps), with a12pt gap before transcript text.
- Thin horizontal separators are lighter between continuous same-speaker turns.
  The first visible row remains unruled; lines add no height or interaction targets.
- Detail titles, metadata and actions have separate rows with a shared content margin.
  Ordinary action buttons use native bordered controls, adaptive text and hover outlines.
  Empty-result failure / cancellation appears once with an explicit retry action; retained
  transcripts still show the processing warning above the prior result.
- General-purpose microphone recording, pause/resume and file import are supported.
  System-output capture is removed; older dual-channel files remain read-compatible.
- First-run precision mode uses bundled MOSS + Nemotron 3 and verified model downloads.
  Mac basic transcription uses Apple SpeechAnalyzer with no speaker separation.
  New processing uses Nemotron 3 only; an entered speaker count is advisory and counts
  above eight are rejected before the job starts.
  Results from the retired Sortformer4, Ultra8, automatic and Community-1 routes stay
  readable; their saved options reopen as Nemotron 3 with the saved count.
- Calendar reminders read selected Mac calendars and start microphone recording.
  Home actions use the same neutral bordered controls as detail actions. The root no
  longer forces a fixed dark-green tint onto ordinary native buttons.
- Notion rich-text copy / parent-end divider and child-page export are explicit actions.
  Destination lookup accepts explicit blank-page MCP responses and rejects missing /
  truncated bodies, conflicting body markers and mismatched page identities.
  MCP insertion uses the `content` field; request validation rejections remain distinct
  from uncertain writes, so a rejected create does not leave an uncertain receipt.
  OAuth lookup and save reuse the same grant's initialized MCP client. A fresh parent
  fetch validates the destination and its last divider together; no body is cached across
  actions. Expired read sessions initialize once and retry once; writes are not replayed.
  Async updates honor `poll_after_seconds` with a bounded wait budget and retain checkpoints.
  Export progress identifies the current stage. Bounded private diagnostics record request,
  authentication, poll-wait and total export time without content / URLs / credentials.
  First-run downloads support byte progress, pause / resume, capacity and runtime checks.
  Only an `invalid_grant` refresh response ends a Notion OAuth grant; network or server
  failures fail that request and keep the connection. A divider whose result was never
  confirmed can be discarded by the user (**처음부터 다시**) because no page was created.
- Per-recording options are local drafts; persistent defaults live in Settings.
- `GroveStore` owns navigation, the meeting index, recording / import / processing and
  quit coordination. Transcript documents (`TranscriptDocumentStore`), folders / people /
  defaults (`LibraryStore`) and voice enrollment / identification (`VoiceIdentityCoordinator`)
  are separate objects. Meter levels and download progress are observed by their own views;
  the workspace only re-renders on state transitions that change what can be started.
- Recording names are editable. **원본 파일…** provides path access, Finder reveal and
  byte-preserving export with source/destination protection and cancellation.
  Internal names and full paths are collapsed under **파일 위치**; **사본 저장…** opens
  the existing save-copy flow without moving or changing the app's original file.
- Rows place speaker/start–end time left and text right. Pretendard and body scaling
  remain. Each utterance is independently editable; continuation never merges text.
- Folders and 미분류 share one section. 모든 녹음 is an aggregate; recent recordings are
  shortcuts. Drag/drop uses typed IDs and publishes changes only after successful save.
- Speaker/text editing, split, undo/redo, history and TXT/Markdown copy/export exist.
  Names can be manually reused within a folder. Dedicated microphone enrollment is
  available from team profiles. Automatic naming and legacy enrollment from meeting
  utterances remain behind the disabled release gate.
- Completion, speaker issues, count mismatch and failure/cancellation are separate.
  Failed retranscription retains and labels the prior transcript.

## Data and compatibility

- Preserve original audio, raw outputs, source-channel/cluster identity and corrections.
- Schema5 adds assignment measurements and explicit speaker-confirmation history.
  Schema3/4 and old split/undo history remain readable, with no startup rewrite. After
  saving schema5 corrections, older betas cannot safely read the new history.
- Acknowledgement binds text, speaker ID, times/source, generation and rule version.
  Relevant edits invalidate only related evidence; title/display-name edits do not.
  Bulk reassignment never acknowledges hidden issues. This is not dataset approval.
- Raw flags remain unchanged. UI triage is heuristic, not a calibrated confidence score.
- Voice registration is separate from names-only reuse. Microphone enrollment uses
  encrypted storage and explicit consent / single-speaker confirmation. Gated matching
  uses selected attendance candidates, ambiguous-result deferral, user-name protection
  and confirm/reject/undo without auto-learning.
  Persist cleanup references before Keychain access; deletion/failed registration must
  not orphan keys. Ordinary library saves/backups strip legacy plaintext embeddings.
- Signed-app replacements need a qualified Keychain access / reauthorization flow;
  never silently reset missing keys.
- Full audio-hash/correction-head/snapshot identities, activity/coverage verification,
  UEM/RTTM datasets and training approval remain planned, not implemented.

## Validation and release work

Run the commands in `AGENTS.md` before publishing code or replacing an application.
Use the opt-in titled-window layout check for transcript changes, and inspect actual
file-picker / setup-dismissal / processing / completion transitions when authorized.
Component snapshots do not establish workspace layout correctness. Keep code-test results separate from model quality.

Debug QA can use `--qa-profile <isolated-directory>`; release builds ignore that
argument. The QA library must contain no production Keychain references or Notion
connection. `window-layout.jsonl` records geometry only when this argument is present.
After exercising library / processing / completed or failed states, validate it with:

```bash
python3 scripts/check_workspace_layout.py <qa-directory>/window-layout.jsonl --require-transitions
```

A passing offscreen `NSHostingController` render cannot exclude a sizing fault in the
actual SwiftUI scene. Validate that captured workspace bounds stay inside the window;
keep the live before/after evidence private. In-window editing menus, real microphone
capture, pointer hover, IME and external Notion writes require their own runtime checks.

Tests cover formatting, exact split boundaries, typed folder moves, publication/storage
failure, explicit confirmation, schema compatibility and undo. Voice tests use synthetic
features/fake keys for encrypted storage, cancellation, stale-source deferral, unknown
and conflicting proposals, cleanup/restart and the disabled product gate. These are not
real-voice accuracy tests or a substitute for signed-app Keychain qualification. Hardware interruption,
pointer drag/drop, VoiceOver and full IME/export checks remain independent QA gates;
unit tests are not evidence that those workflows were manually tested.

Before replacing an app, confirm permission and check for active recording, inference,
export or editing. Preserve data, keep a recoverable prior bundle, verify signing and
confirm the new process. Never restore an old library over newer data. Do not leave
duplicate generated apps in Downloads or Applications.

Qualified native helpers are bundled. The app prepares pinned default weights after
user selection, or uses Apple-managed Korean assets for basic transcription. Do not
claim a developer-tool-free separate Mac test unless it was performed.
Research environments and build caches are regenerable; preserve source, locks,
licenses and private evidence before scoped cleanup.

## Research boundaries and next work

- AI summaries and ChatGPT / Claude provider authentication are not implemented.
  The current source only has evidence-claim data / display scaffolding; it does not
  generate summaries. Provider integration was researched and deferred. A future
  implementation requires new authorization, provider eligibility / billing checks,
  explicit transcript transmission and separate summary revisions linked to source
  utterances. No provider token or summary request was created during the research.
- Notion OAuth uses public-client PKCE / dynamic registration with the official hosted
  MCP endpoint. No Grove auth server, app secret or user-installed CLI is needed. Grant
  metadata and user-selected destinations replace team / workspace / email allowlists.
  Native browser consent and a live parent-page export require user-run qualification;
  synthetic protocol / storage checks do not establish live workspace write success.
  Manual tokens remain available under advanced connection settings.
  The AuthenticationServices bridge uses an explicitly Sendable completion and hops
  to MainActor for session state. Background-queue callback / cancellation / retry
  checks exercise the actual bridge rather than replacing the browser implementation.
  See `setup-and-integrations.md` for lifecycle and compatibility.
  Destination URL / entity handling is defined in `notion-link-handling.md`. Research
  upstream forms first; input parsing and returned server URLs share one normalizer.
- [Automatic speaker identification](automatic-speaker-identification.md) has implemented
  microphone enrollment and attendance selection. **Automatic naming remains gated**.
  A new embedding recipe needs independent-session known/unknown calibration and
  evaluation. Rejecting every speaker is not success. Enrollment alone does not qualify
  automatic naming. Names-only manual reuse remains usable.
- [Offline speaker-logic experiments](speaker-logic-experiments.md) separate unchanged-text
  alignment, word assignment and frozen-posterior postprocessing. They create research
  sidecars, not app documents; production projection-v1 and human edits remain unchanged.
- [Annotation plans](review-mode-spec.md) are separate from speaker-issue triage. Read
  their data contracts before implementing training/evaluation dataset production.
- Public Git contains code, synthetic tests and generic documentation only. Private
  audio, references, labels, voice features, measurements, qualitative diagnoses,
  screenshots, local paths and installation receipts remain ignored.
- Commit/push with the personal account and restore the active GitHub CLI account
  to `nathan-glorang`, as required by `AGENTS.md`.

See [transcript/library UX](transcript-library-ux.md), [file management](recording-file-management.md),
[release readiness](release-readiness.md), [saved speakers](microphone-folders-and-speaker-reuse.md)
and [artifact retention](artifact-retention.md). Private handoff and decision records
are reached through root `HANDOFF.md` / `.local`; generated installation evidence
belongs in `.work/`. Retained historical experiments may still exist in `results/`.

See [setup and integrations](setup-and-integrations.md) and [distribution](distribution.md).
