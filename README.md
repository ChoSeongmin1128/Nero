# Nero

Nero (formerly Grove) is a local meeting recorder for macOS and Windows.
Audio and meeting metadata stay on the device unless the user explicitly exports them.

## Native rewrite

Nero release versions and data compatibility follow the [version policy](docs/versioning.md).
Source commits and distribution status are recorded in the [release history](docs/release-history.md).

The new [desktop application](apps/desktop/README.md) uses Tauri and a shared
[Rust recording core](crates/grove-core/README.md). It has its own application identity
and storage; it does not import the legacy library automatically.

The first development slice contains meeting creation, notes, microphone selection,
recording/pause/stop, PCM preservation, interrupted-file recovery, WAV playback and
a compact recording window. With packaged workers, Apple silicon/macOS 26 also
supports local audio import, pinned Nemotron 3.5 Q8 speech recognition, independent
whole-recording Nemotron 3 speaker analysis and separately versioned corrections.
Existing MOSS results and edits remain readable without automatic reprocessing.
With a packaged llama.cpp worker, explicit model
preparation enables on-demand Gemma 4 E4B Q4 meeting summaries linked to transcript
revisions. Windows speech runtimes have separate Vulkan/CPU packaging; native audio
preparation and end-to-end Windows acceptance remain incomplete. Native macOS and Windows acceptance results must
be recorded separately.

The Swift source below is retained temporarily as the legacy implementation.

> Nero is in development. Packaging requires prepared native workers. Model weights
> are downloaded and verified only after explicit preparation in the app.

## Rewrite design system

The [design system](design-system/README.md) contains shared UI, interactive examples,
and macOS/Windows browser prototypes for the rewrite. It is separate from the current
native app described below. Native Windows recording is not implemented by these examples.

## Legacy Swift capabilities

- native SwiftUI macOS app
- first-run choice between precision transcription and Mac basic transcription
- microphone-only recording with pause/resume, captured as ADTS AAC so an interrupted
  recording stays readable, then repackaged without re-encoding to M4A
- audio and video file import
- Korean post-meeting transcription with MOSS or Apple SpeechAnalyzer
- independent diarization with Nemotron 3 (up to eight speakers; an entered count is advisory)
- Calendar reminders from accounts connected to the Mac Calendar app
- rich-text copy for Notion and divider / meeting-note child page added at a chosen page end
- in-app model preparation with byte progress, pause / resume, capacity checks, pinned versions, SHA256 and a runtime check
- per-recording transcription/count options; persistent defaults live in Settings
- local folders, recording-title editing, and explicit reuse of saved speaker names
- utterance text/speaker editing, splitting, undo/redo and TXT/Markdown export
- original audio preservation, path access and byte-preserving file export

Results produced by the retired Sortformer4, Ultra8, automatic and Community-1 routes
remain readable; reprocessing them uses Nemotron 3.
Speaker names are not automatically matched across meetings. Nemotron 3 count
entry is advisory, not an exact cluster-count constraint. A successful transcription
or an acknowledged speaker warning does not certify transcription accuracy or approve
training data. AI summaries and dataset-quality annotation/export are not shipped.

## Legacy Swift requirements

- macOS 26 or newer
- Apple Silicon
- Swift 6.2 / Xcode 26

## Legacy Swift build

```bash
swift test
python3 -m unittest discover -s tests -v
./scripts/package_app.sh release
codesign --verify --deep --strict .work/dist/Grove.app
```

Mac basic transcription uses Apple-managed Korean assets and does not separate speakers.
The packaged application is written to `.work/dist/Grove.app`. Packaging expects qualified
workers in `.work/native-workers` (or `GROVE_NATIVE_WORKERS_DIR`), including their
matching resources and license notices. This ignored directory is not part of a fresh
checkout. See the [MOSS worker](scripts/native-moss-harness/README.md) and
[Nemotron worker](scripts/native-nemotron/README.md) notes and the
[third-party notices](THIRD_PARTY_NOTICES.md).
Calendar access and Notion credentials are configured in Settings. Notion export sends
text only when the user clicks Apply; copying does not require a Notion connection.
Models are stored separately under Grove's Application Support directory and are not
bundled in the app or this repository. Source builds/tests do not establish that the
runtime models are installed or that a release is notarized.

## Optional glossary experiments

Grove starts with no built-in organization terms or personal names, and the app does not
apply glossaries. The Apple/Whisper benchmark scripts take a glossary file through
`--glossary`; [`config/glossary.example.json`](config/glossary.example.json) shows the
format. Keep real glossaries machine-local and out of Git.

## Private evaluation data

Audio, video, transcripts, RTTM/SRT files, licensed datasets, benchmark outputs, and
machine-local glossary profiles are excluded by `.gitignore`. The public repository
contains only generic test fixtures.

If you add your own dataset, keep it under `test-data/` and generated outputs under
`results/`. Neither directory is tracked.

## Design principles

- preserve original audio and raw transcripts
- keep input source, anonymous speaker cluster, and confirmed human identity separate
- treat summaries and corrections as derived revisions with source references
- support local microphone recording and file import, without system-output capture
- keep processing completion, speaker-review issues and dataset approval separate
- do not ship organization-specific terms, names, examples, or demo meetings
- measure Korean multi-speaker quality on private reference data before selecting a
  default diarization engine

Research notes are indexed in [`AGENTS.md`](AGENTS.md). They are AI-assisted working
documents and must be checked against current code and primary sources.

See [setup and integrations](docs/setup-and-integrations.md) and [distribution](docs/distribution.md).
