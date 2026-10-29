# Nero Agent Instructions

This file applies to the entire repository.

## Native rewrite

- `apps/desktop/` is the new Nero application. `crates/nero-core/` owns its
  platform-independent storage and recording state; CPAL provides native microphone
  input on macOS and Windows. Browser prototypes do not establish native support.
- The current identity is `io.github.choseongmin1128.nero.desktop`. Identity changes require the explicit installer migration and strict completed receipt. Ordinary reads must not migrate or initialize a library. Never open the separately archived original Swift library implicitly. The old installed app and data are archived
  outside active application locations; do not launch the archive during validation.
- New capture preserves native PCM and creates a separate float32 WAV for playback.
  The legacy AAC/M4A and model rules in `legacy/swift/AGENTS.md` apply to the retired implementation,
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
- During new recording, a background saved-PCM reader may feed one persistent ASR
  stream and an independent speaker stream. Publish provisional replacements at
  60–180 seconds without resetting models, finish only after successful capture EOF,
  and bind the exact fed PCM hash/frame count to the finalized analysis WAV.
  Interim C ABI text has no word offsets: do not invent times or speaker labels.
  Save native ASR before speaker finalization and reuse it through the normal
  checkpoint/continuation path. See `docs/live-speech.md`; source checks do not
  establish microphone, low-memory laptop or macOS acceptance.
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
  Vulkan and CPU runtimes. Windows WAV preparation uses the portable bounded converter;
  compressed imports remain unavailable there. macOS conversion requires Apple silicon/
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
- Validate this scope with `cargo test -p nero-core --locked`, the desktop JavaScript
  tests and a native Tauri build, sequentially. Windows needs a Windows runner/device;
  a macOS or WSL build does not establish Windows recording, permissions or packaging.
- A development bundle is not a signed/notarized distribution. Native capture,
  device interruption, shutdown and long recordings require hardware acceptance tests.

## Rewrite design system

- `design-system/` owns the shared browser design system and examples for the rewrite.
  Its target platforms are macOS and Windows. The retired Swift implementation is isolated in `legacy/swift/`; it is not a current installation target.
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
5. Nero planning and handoff documents

If documentation and code differ, the code explains current behavior but does not
automatically define the correct product requirement. Separate verified facts,
inference, proposals, and unverified items.

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

## Source and historical evidence

- Current desktop contracts use Nero package, event, UI, storage and build names.
- Previous identifiers remain only in explicit migration boundaries and tests, immutable old transcript provenance, release receipts and the retired `legacy/swift` source archive. Do not rewrite signed release metadata or measured evidence.
- The macOS converter is compiled from `scripts/audio-prepare`; the retained MOSS metadata is `crates/nero-core/resources/models/moss.json`. The independent optional `scripts/native-moss-harness` is not the retired root Swift app.
- The native macOS app and Windows package must be validated separately. macOS tests and the browser prototype do not establish Windows capture, imports or model execution.

## Git and verification

- Use the personal ChoSeongmin1128 account without switching global authentication. Product push and PR creation require an explicit user request.
- Read `HANDOFF.md` and task-specific current docs. Keep private meeting data and evaluation results outside product Git.
- Sequential validation: `cargo test -p nero-core --locked -- --test-threads=1`, `cargo test -p nero-cli --locked -- --test-threads=1`, `npm --prefix apps/desktop test`, `npm --prefix apps/desktop run build`, `cargo check -p nero-desktop --locked`.
- Native bundle, recording hardware, live OAuth and Windows execution are separate checks; record omissions explicitly. Do not initialize dummy user meetings to validate a release.
