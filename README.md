# Nero

Nero is a local meeting recorder for macOS and Windows.
Audio and meeting metadata stay on the device unless the user explicitly exports them.

## Native rewrite

Nero release versions and data compatibility follow the [version policy](docs/versioning.md).
Source commits and distribution status are recorded in the [release history](docs/release-history.md).

The new [desktop application](apps/desktop/README.md) uses Tauri and a shared
[Rust recording core](crates/nero-core/README.md). It has its own application identity
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

The retired Swift source is retained temporarily under `legacy/swift/`.

> Nero is in development. Packaging requires prepared native workers. Model weights
> are downloaded and verified only after explicit preparation in the app.

## Rewrite design system

The [design system](design-system/README.md) contains shared UI, interactive examples,
and macOS/Windows browser prototypes for the rewrite. Its canonical controls and tokens are consumed by the native desktop app. Native Windows recording is not implemented by these examples.

## Retired implementation

The previous Swift application is isolated in [legacy/swift](legacy/swift/README.md).
It is not the Nero installation target. Original identifiers are retained only for
historical source and explicit migration of existing personal data.

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
