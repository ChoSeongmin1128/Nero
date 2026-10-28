# Nero Agent Instructions

This file applies to the entire repository.

## Native rewrite

- `apps/desktop/` is the new Grove application. `crates/grove-core/` owns its
  platform-independent storage and recording state; CPAL provides native microphone
  input on macOS and Windows. Browser prototypes do not establish native support.
- The new identity is `io.github.choseongmin1128.grove.desktop`, with storage resolved
  from that application's data directory. Never probe, migrate or initialize the
  legacy `Grove` library implicitly. The old installed app and data are archived
  outside active application locations; do not launch the archive during validation.
- New capture preserves native PCM and creates a separate float32 WAV for playback.
  The legacy AAC/M4A and model rules below apply to the retained Swift implementation,
  not this new format. Never discard original PCM after playback conversion.
- The native app provides meetings, notes, microphone recording and playback. With
  packaged workers, Apple silicon/macOS 26 supports native file import and local
  Nemotron 3.5 Q8 speech recognition. Explicit setup prepares the pinned ASR model
  and the smaller Nemotron 3 speaker model; speech readiness gates new meetings,
  recording and import. Packaged llama.cpp enables on-demand Gemma 4 E4B Q4 summaries
  with separately stored source revisions; summary preparation remains optional.
  Do not fabricate transcripts or summary results.
- Default recognition sends the complete recording through one stateful
  NeMo-Speech.cpp stream with endpointing disabled, then finishes the stream at EOF.
  `commit_nemotron_transcript` requires pinned model/runtime identity, full input-frame
  identity, successful native completion evidence and valid word timestamps. Preserve
  native output, the execution receipt and original audio; completion evidence does
  not prove that every spoken word was recognized. Never silently switch to chunk ASR.
- Save recognition before optional terminology correction and the separate
  whole-recording Nemotron 3 speaker stage. Persist the source-bound diarization
  continuation under the same job ID so a speaker-stage retry does not rerun ASR.
  Failure or cancellation retains the saved text and audio. Existing meetings are
  never automatically reprocessed. Speaker management can explicitly rerun analysis.
  The isolated V3 C ABI child preserves every probability frame before compaction.
  Whole-file contiguous activity uses p > 0.5, while timestamped words use the pinned
  native onset argmax rule. Store new word-turn projections as separate generations,
  preserving canonical ASR, corrections, manual assignments and prior history. Do not
  force the number of model slots to match an attendee list.
- Stored MOSS output, edits and history remain readable with their original source
  and revision identities. Preserve the MOSS EOS, measured-budget, single-generation
  and complete-input checks for its whole-recording artifacts; the stored-artifact
  policy does not authorize an old worker as a new recognition fallback.
- ASR and diarization share the pinned native speech package. GPU is preferred;
  only a confirmed unavailable GPU backend permits the packaged CPU route. Allocation
  and generic runtime failures do not authorize CPU fallback. Windows packages contain
  Vulkan and CPU runtimes, but the current AudioPrepare path requires Apple silicon/
  macOS 26. Windows end-to-end recognition availability remains unqualified.
- Existing chunk speaker labels retain their `asr_chunk` scope; never equate the same
  label across independently recognized chunks. Verified whole-recording labels use
  a separate recording scope. Nemotron ASR starts without speaker labels; the independent
  full-recording speaker stage supplies recording-scoped assignments. Manual names/merges
  are transcript revisions, not automatic identity verification. Diarization does not
  establish qualified eight-person support. Keep automatic/user/unknown name origin separate; user names
  are useful references, not automatic voice-identity verification. See `docs/meeting-library-and-speakers.md`.
- Transcript source identity uses raw hashes, immutable initial utterance IDs/order and
  source indices. Playback time is validated at the ASR 16kHz sample address, not used
  as an approximate identity. Legacy source upgrades are a separate startup operation,
  with immutable before/after evidence; ordinary reads must not migrate or reconstruct.
  Keep float_roundtrip in the shared workspace JSON dependency for stable persistence.
- CLI and stdio MCP queries share `reader::LibraryReader`. Never call `CoreHandle::open`
  from a query tool: opening the writer acquires its lock and runs recovery. Ordinary
  reads must not initialize directories, migrate, load models or start capture. Keep
  paging/source revisions and read-only tool annotations consistent across both entry
  points. Explicit meeting mutations use the running app's authenticated local writer
  bridge and edit-version checks; template mutations use the template catalog lock
  and revision checks. Host clients own tool approval policy. Meeting deletion means
  recoverable trash; no permanent purge tool is exposed. See `docs/local-agent.md`;
  do not edit another AI client's settings implicitly.
- Model selection will prefer qualified GPU paths and retain CPU compatibility.
  Initial diarization support targets up to eight actual speakers; more attendees
  must not block recording or transcription. Preserve transcription on speaker-stage failure.
- Validate this scope with `cargo test -p grove-core --locked`, the desktop JavaScript
  tests and a native Tauri build, sequentially. Windows needs a Windows runner/device;
  a macOS or WSL build does not establish Windows recording, permissions or packaging.
- A development bundle is not a signed/notarized distribution. Native capture,
  device interruption, shutdown and long recordings require hardware acceptance tests.

## Rewrite design system

- `design-system/` owns the shared browser design system and examples for the rewrite.
  Its target platforms are macOS and Windows. The existing Swift app rules below
  continue to describe the retained legacy Swift implementation.
- Edit shared controls and styles at their canonical source, then rebuild the preview.
  Do not fix only the catalog while leaving the product example different.
- `design-system/typography.css` owns font families and text roles;
  `components.css` owns shared control styles; `components.js` owns repeated views.
  Feature state and event handlers stay in their consuming screen. Keep platform
  font contracts shared and preserve explicit component layout when composing icons.
- Check opened menus, calendar navigation and confirmation, focus restoration,
  content padding and narrow-window boundaries. Initial closed screenshots are not
  interaction verification. Browser checks do not establish native OS behavior.
- Legacy code retention is temporary. Moving code or renaming an installed app must
  preserve the existing library and keep the new app's identity and storage separate.

## Document trust

Documents under `docs/` are wholly or partly AI-assisted working notes. Read them
critically. Verify claims in this order:

1. the current user request
2. running app state, local files, logs, and measurements
3. current source code and tests
4. primary documentation and upstream repositories
5. Grove planning and handoff documents

If documentation and code differ, the code explains current behavior but does not
automatically define the correct product requirement. Separate verified facts,
inference, proposals, and unverified items.

## Legacy Swift product boundaries

- Grove is a general-purpose local macOS meeting recorder and transcription app.
- New recording is microphone-only. Do not reintroduce computer/system-output capture;
  file import and read-only compatibility with existing recordings remain supported.
- Do not hardcode organization names, project terms, people, or private meeting examples.
- Notion OAuth uses the official hosted MCP service with public-client PKCE and dynamic
  registration; do not require a Grove authentication server or a separate CLI. Do not
  add workspace, organization or email allowlists. Obtain identities from authorization
  and destinations from user selection. If REST public OAuth is added, use Any workspace.
- AuthenticationServices completion callbacks may arrive on an XPC queue. Keep the
  callback explicitly Sendable and move state changes to MainActor inside it. Exercise
  that same callback from a background queue; a mock browser alone does not verify it.
- Research Notion link forms and destination entity semantics before changing URL
  rules. Use `docs/notion-link-handling.md`; apply the same normalizer to input and
  API-returned URLs. Do not guess page IDs from slugs or confuse database views / rows.
- User glossaries are optional machine-local profiles and never public defaults.
- Preserve original audio and raw transcripts. Store corrections, speaker assignments,
  and summaries as separate revisions.
- AI summaries and ChatGPT / Claude provider sign-in are not implemented. Provider
  research is not implementation approval. Require a new user request before adding
  provider authentication or transmitting transcripts for summaries.
- New recordings are captured as ADTS AAC and repackaged without re-encoding to M4A only
  after the frame count matches. Never delete a capture before the index points at its
  replacement, and never repackage imported files.
- New processing uses Nemotron 3 only, and an entered speaker count is advisory. Retired
  engines (Sortformer4, Ultra8, automatic routing, Community-1) stay readable in saved
  results but are not run again.
- Unexpected recording completion and encoding failures terminate only their own capture,
  preserve meeting audio and publish an interrupted state. Pausing is not completion.
- Prepare playback after the recording file is finalized, including the same detail view
  remaining open through transcription. Do not cache an actively written recording.
- Validate the selected per-meeting inference plan before microphone permission, import
  copying or replacing processing metadata. Preserve prior results on preparation failure.
- Calendar permission replies apply only to the latest enable/disable request.
- Keep source channel, anonymous speaker cluster, and confirmed human profile separate.
- Do not use ScreenCaptureKit or request screen-recording permission for an audio-only
  capture route.
- Do not expose internal implementation terms as the default user-facing language.
- Use short, concrete UI labels. Remove descriptions that repeat a heading or button,
  promotional taglines and routine implementation explanations. Keep information needed
  to choose a transcription mode, grant permissions, understand a limit or recover from errors.
- Voice enrollment contains no development / validation-status notices or storage
  implementation explanations. Its retention notice is only: "등록 후 녹음 파일은 저장되지 않습니다."
- A single workspace owns navigation and shared dialogs. Start recording / import after
  setup dismissal. Empty transcript states must accept the available height; never force
  a vertically fixed size onto a container-filling view. Verify native file import through
  processing and completion, not only initial or component rendering.
- Meeting actions use legible adaptive text, native bordered controls and visible hover
  feedback. Preserve native keyboard / disabled behavior. Align the detail header and
  transcript to the same content margin; show an empty-result error in one place.

## Private data

- Never commit audio, video, transcripts, speaker labels, voice embeddings, licensed
  datasets, benchmark outputs, user glossaries, or local paths.
- Keep private fixtures under ignored `test-data/` and generated outputs under ignored
  `results/`.
- Before every public push, inspect `git status`, tracked files, and staged diffs for
  private names, paths, credentials, and media.

## Native installation and artifacts

- Nero release versions follow `docs/versioning.md`; keep version mirrors consistent
  with `node apps/desktop/scripts/version.mjs check`. Nero has not yet been shared
  with actual users: internal architecture and stored formats may change, and each
  feature does not require a product-version bump from 0.0.1. Compatibility obligations
  begin with the first build actually shared with users; record that baseline.
  Preserve existing personal meeting data through explicit migration or separate archival
  when a development change affects it. Do not initialize or discard it implicitly.
  Preserve historical commits, tags and release metadata rather than rewriting labels.
  Undistributed development binaries follow the bounded retention rules below.
- Record release source commits in `docs/release-history.md`. A dirty build must not
  be attributed to its HEAD alone. Distribution bundles require clean source provenance;
  keep generated binaries, signing/notary receipts and private compatibility evidence in `.work/`.

- `/Applications/Nero.app` is the macOS installation target, not proof that the
  new app is installed. Development bundles stay in `.work/rust-target`.
- Build/test instructions for the new app are in `apps/desktop/README.md`.
  Never install or launch an archived legacy app as part of rewrite validation.
- Move discarded artifacts to Trash only. The user empties Trash personally; do not
  ask to purge cleanup groups or permanently delete them.
- Ordinary app replacement does not authorize a full library snapshot. Do not copy
  the user's audio, models or complete data directory for each build. A schema/data
  migration must declare its affected paths and preserve only the data it changes.
  Verify an app-only replacement with the writer lock, data hashes and model metadata.
- Use the canonical `apps/desktop/scripts/install-macos.py` installer. On one filesystem,
  move the verified candidate and previous app by rename, with one rollback slot.
  After GUI/data verification, retire that rollback app; keep the current and immediately
  previous notarized DMGs plus small provenance receipts. Older undistributed app/ZIP/DMG
  copies are disposable after their source/hash history is recorded. Do not accumulate
  per-release `.app` copies, `data-before` folders or standalone installer clones.
- Use `.work/rust-target` as the single Cargo target cache. Retire temporary alternate
  targets when their experiment is finished. Temporary notary upload archives belong
  to an auto-cleaned temporary directory and are not retained beside final artifacts.
- During macOS GUI/release verification, keep a bounded temporary display/idle/user-active
  assertion while working and release it when finished. Do not change persistent
  screen-lock, power or security settings.
- Keep synthetic fixtures in auto-cleaned temporary directories. Never add dummy
  meetings or templates to the user's library during validation. Reuse model/audio
  inputs without making copies. After recording the needed result, move obsolete
  upload ZIPs, intermediate app copies and one-off logs to Trash. Preserve the installed
  release, necessary rollback, original user data and unique validation evidence.
- Keep generated native builds, dependencies and preview output under ignored
  `.work/`; a fresh checkout must not depend on this machine's Git excludes.

## Legacy Swift installation and artifacts

- The canonical installed app is `/Applications/Grove.app`.
- Validate tests, release packaging, code signing, and a smoke test before replacing it.
- An explicit user-approved local-beta exception may waive full UI QA. Record the
  exception and remaining limitations; never call unperformed checks a pass. Do not
  interrupt an active user test or overwrite the bundle they are running.
- Research-only work does not authorize replacing the installed app.
- Do not leave duplicate apps or generated artifacts in Downloads or `/Applications`.
- Prefer offscreen rendering for design checks. Explain live dialog checks before opening
  them, close validation dialogs promptly and avoid interrupting the user's app inspection.

## Git account and publication

- Grove commits and pushes use the personal `ChoSeongmin1128` account/repository.
- Use that account's public noreply commit identity; do not expose a private email or
  change global Git author settings for this project.
- Never change the global `gh` active account (`gh auth switch`, `gh auth login`); other
  sessions share it. Pushes go over the `github-seongmin` SSH remote. For gh commands use a
  command-scoped token: `GH_TOKEN="$(gh auth token --user ChoSeongmin1128)" gh ... -R ChoSeongmin1128/Grove`.
- Inspect the exact staged files and diff before every push. Raw/test meeting data,
  user libraries, local model files and generated result artifacts remain excluded.
- Personal-meeting-derived benchmark numbers and qualitative error analyses also stay
  private, even when names/audio have been removed. Public docs contain product behavior,
  generic methodology and upstream technical information, not private sample results.
- Meeting timing reports separate audio preparation, transcription model loading /
  inference, diarization and speaker projection. Report the sequential total and its
  scope; exclude unrelated idle gaps. Record first-use/cache, priority and memory-pressure
  conditions. A worker-stage sum is not a measured GUI save / rendering duration.

## Legacy Swift resume and integration rules

- Read root `HANDOFF.md` first when available. It points to the private `.local` handoff;
  `docs/handoff.md` describes source behavior, not machine-local installation receipts.
- Read `docs/setup-and-integrations.md` and `docs/distribution.md` for first-run models,
  Mac basic transcription, Calendar, Notion and Developer ID distribution.
- Bundle all inference executables / libraries / Metal resources. Never require Python,
  a separate CLI or development tools on the user's Mac.
- Keep pinned model versions and hashes in `ModelCatalog`. Download weights only after
  the user chooses precision transcription and starts preparation. Preserve partial
  downloads, verify completed bytes and run the readiness check before claiming ready.
- Team voice enrollment is microphone-only and explicit. Keep automatic naming gated until
  independent-day known/unknown evaluation; neither attendance counts nor synthetic model
  checks establish identity accuracy. Do not append inferred matches to registered voices.
- Mac basic transcription uses Apple-managed language assets and has no diarization.
  Store this as an explicit no-diarization result, not an invented speaker or confirmation.
- Calendar uses accounts already connected to Mac Calendar. No Meet capture or Google
  OAuth registration is in MVP scope. Request Calendar permission through the app UI.
- Notion tokens stay in Keychain. Copy works without a connection. Add a divider and child
  meeting-note page at the chosen parent's end; do not replace the existing parent body.
  Preserve creation receipts and never automatically retry an uncertain page creation.
- Reuse Notion MCP initialization only within the same OAuth grant. Inspect the parent
  once at save time; do not reuse an earlier lookup's body for insertion decisions.
  Respect server poll intervals, retain uncertain-write checkpoints and never automatically
  repeat a write when its MCP session expires. Diagnostics contain timing / sizes / status
  only, never tokens, page IDs, URLs or transcript content.
- Developer ID signing / notarization credentials are supplied locally; do not commit
  credentials or reuse another app's update keys / CloudKit profile for Grove.
- Use `.work` for new logs, builds, app bundles and native-worker preparation. Retained
  historical `results` are not permission to overwrite or clean unrelated evidence.

## Work sequence

1. Read this file and `docs/handoff.md`.
2. Read only the task-relevant documents and verify important claims in code.
3. Distinguish research/review from implementation authorization.
4. Before stored-data changes, preserve personal data through explicit migration or
   archival. After first user sharing, also check compatibility with supported releases.
5. Add proportionate tests and run the relevant validation commands.
6. Update `docs/handoff.md` with results, remaining risks, and whether the installed app
   changed.

New desktop validation, run sequentially:

```bash
cargo test -p grove-core --locked -- --test-threads=1
cargo test -p nero-cli --locked -- --test-threads=1
npm --prefix apps/desktop test
npm --prefix apps/desktop run build
cargo check -p grove-desktop --locked
```

A native bundle build and hardware acceptance remain separate gates; follow
`apps/desktop/README.md` on the target operating system.

Legacy Swift validation:

```bash
swift test
python3 -m unittest discover -s tests -v
./scripts/package_app.sh release
codesign --verify --deep --strict .work/dist/Grove.app
```

## Document index

- [`docs/meeting-library-and-speakers.md`](docs/meeting-library-and-speakers.md): native
  folders, trash/purge and cancellation ownership, speaker revisions, chunk-scoped
  model labels, content search and platform/quality validation boundaries.
- [`docs/automatic-speaker-identification.md`](docs/automatic-speaker-identification.md):
  microphone enrollment, optional pinned voice model, attendance candidates and encrypted
  registry; automatic naming remains blocked pending independent-session qualification.
- [`docs/speaker-logic-experiments.md`](docs/speaker-logic-experiments.md): offline-only
  alignment/projection and limited-postprocessing ablations, identity checks, separate
  text/activity metrics and no production integration.
- [`docs/transcript-library-ux.md`](docs/transcript-library-ux.md): compact transcript rows,
  folder drag/drop and navigation, processing versus speaker-review states, explicit
  confirmation and schema-5 compatibility; distinct from dataset annotation.
- [`docs/recording-file-management.md`](docs/recording-file-management.md): recording title
  changes, original file export/path access, source protection and verification limits.
- [`docs/artifact-retention.md`](docs/artifact-retention.md): active app assets versus
  regenerable experiments; retained evidence and scoped cleanup.
- [`docs/review-mode-spec.md`](docs/review-mode-spec.md): planned native review workflow,
  edit/invalidation/undo matrix and phased completion gates; NOT implemented.
- [`docs/annotation-schema.md`](docs/annotation-schema.md): planned audio/model/correction/
  review/snapshot identities, timebases, content binding and migration.
- [`docs/labeling-guidelines-ko.md`](docs/labeling-guidelines-ko.md): planned Korean
  verbatim labeling, uncertainty, overlap and independent completeness rules.
- [`docs/dataset-export.md`](docs/dataset-export.md): planned backup versus approved dataset,
  immutable snapshots, RTTM/UEM, permission and development/holdout separation.

- [`docs/ultra8-option-and-speaker-count.md`](docs/ultra8-option-and-speaker-count.md):
  historical; retired Ultra8 automatic policy, count boundaries, same-engine comparisons, native worker
  and historical engine provenance.

- [`docs/microphone-folders-and-speaker-reuse.md`](docs/microphone-folders-and-speaker-reuse.md):
  microphone-only recording, pause/resume, per-file options, folders, manual speaker
  reuse, and the evidence-based hold on automatic voice identity matching.

- [`docs/release-readiness.md`](docs/release-readiness.md): requirement-by-requirement
  completion audit, pending authorization/runtime gates, and installation resume sequence.

- [`docs/grove-v0.3-implementation-plan-2026-09-03.md`](docs/grove-v0.3-implementation-plan-2026-09-03.md):
  approved v0.3 scope, typography, speaker/text editing, export, and engine-validation gates.
- [`docs/handoff.md`](docs/handoff.md): current implementation state and next work.
- [`docs/open-source-local-meeting-apps-2026-09-02.md`](docs/open-source-local-meeting-apps-2026-09-02.md):
  local meeting app source review.
- [`docs/speaker-diarization-research-2026-09-03.md`](docs/speaker-diarization-research-2026-09-03.md):
  diarization model research and Grove evaluation plan.
- [`docs/app-v0.2-implementation-2026-09-02.md`](docs/app-v0.2-implementation-2026-09-02.md):
  historical v0.2 implementation notes; current code takes precedence.
