# Desktop task ownership and recovery

This is a source contract, not an installed-app or hardware acceptance receipt.

## Ownership

Rust core owns capture, original audio, placement and versioned results. Processing owns
one execution slot and a bounded request queue. A recording start keeps model downloads,
but cancels and joins an active inference before capture starts. Requested inference waits
until recording/current processing ends. Changing the selected meeting does not change
job ownership. Folder movement changes placement, never original paths or job ownership.

## Persistence

New jobs append immutable generations at `processing/<job-id>/job-state/<generation>.json`.
Existing owner metadata, worker evidence and meeting files are preserved. Writes use
create-new files and file synchronization instead of replacement rename. Unix also syncs
the directory; Windows power-loss durability still needs device validation.

A damaged latest generation never reverts to an older actionable state. Startup isolates
individual damaged jobs, preserves their bytes and recovers valid jobs. Unsafe common
processing paths remain shared storage errors. Legacy jobs receive no invented status.

| Stored state | Restart |
| --- | --- |
| Queued/running | Interrupted, explicit retry when supported |
| Running with saving phase | Result confirmation required, no replay |
| Cancellation requested | Cancelled, no restart |
| Completed/failed/cancelled | Outcome retained |
| Acknowledged notice | Outcome/history retained, notice hidden |

No recovered job loads a model or starts recording automatically. Retry creates a new job;
it does not restore an in-memory decoder. Downloads reuse existing Range/hash verification.

## Source and cancellation

- Requests bind audio and transcript identity; summaries also bind notes/current summary.
  Retry passes the original binding through input capture and execution preflight.
- Changed/deleted inputs reject an old request. Core revision/CAS checks remain the final
  publication authority. Preflight success never grants overwrite permission.
- Cancellation intent is saved before acceptance. Queued closures are removed only after
  persistence succeeds. User cancellation is not promoted to a recording interruption.
- Baseline recognition, final publication and imported-audio publication save their phase
  before applying effects. Cancellation is rejected during final publication.
- Unconfirmed worker termination blocks new capture/inference. Typed failure feedback is
  preserved; acknowledging an alert neither erases the outcome nor retries its operation.

Core output validation, source-version conflicts and missing meetings reject publication
with their typed cause. They are failed jobs, not uncertain saves. Storage errors and
an unavailable actor during publication retain result-confirmation protection.

## Recording recovery

PCM checkpoints, complete frames and originals remain protected. Invalid session manifests
fail at the owning meeting boundary, preserving other meetings and valid existing playback.
Recovery metadata write failures remain errors. Zero complete frames are not called recovery.

## Whole-recording recognition

New desktop recognition publishes only `nero-moss-whole-v2` output under the unchanged
`whole-recording-v1` policy. Metadata schema 2 must bind one decoder call to complete
PCM, actual prompt/output budgets, the configured/tokenizer EOS and actual terminal
token, and acoustic feature lengths/embedding counts for the complete input. Incomplete
parsing, length/repetition stops or inconsistent evidence preserve raw output and prior
results. A structural acceptance is not a speech-coverage or speaker-quality guarantee.

Existing v1 whole-recording artifacts remain readable/restorable only through stored
history validation, including their raw hash and initial/current source association.
This does not accept newly supplied v1 results, rewrite old evidence, or rerun a job.
An explicit retry retains its original source binding, revalidates current inputs
and must satisfy the current publication contract; recovery cannot promote an old
worker or chunk-local speaker label.

macOS packaging checks the included worker against canonical source/lock/patch/resource
receipts before signing and connects its preserved ad-hoc provenance to a separate
Developer ID receipt. An absent worker remains unavailable. The MOSS guard is not a
Windows recognition implementation and is not required by Windows builds.

## Summary context

`summary_input` freezes transcript, notes and expected summary ID in one actor call. New
summaries archive notes and a separate notes hash. Existing transcript hashes retain their
citation/read-only-client meaning. Notes edits stale notes-aware summaries; old summaries
without that hash retain their transcript-only meaning without rewriting.

Whole/map/merge prompts include notes as reference data and measure their token cost.
Notes alone are not evidence of meeting decisions, people or deadlines. Oversized notes
fail rather than being silently omitted. Correction-rule descriptions are not approved
term definitions and are not automatically injected as such.

## Presentation and platforms

Waiting, interruption, failure and uncertain saving have distinct feedback and ID-based
actions. Unrelated downloads do not block existing meetings. List updates preserve edits,
playback and selected tabs. Read-only CLI/MCP never initialize this job writer/recovery.

The recorder remains non-focused and always-on-top. Physical work-area checks recover
an offscreen window without moving fully visible user placement. Movement grace avoids
fighting an active drag. Windows shares no macOS-notch assumption. The main window disables
native file-drop interception for HTML folder drag; audio import keeps its native picker.

Tests use synthetic temporary libraries. The CI matrix includes macOS and Windows native
tests. Mac builds/scripted GUI tests do not establish Windows devices, permissions, drivers,
display behavior, power-loss durability or model performance. Long ASR output and automatic
speaker identity remain separate qualification gates. No installation/release is implied.
