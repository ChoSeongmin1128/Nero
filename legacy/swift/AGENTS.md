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


## Preservation

This retired source archive is not the active Nero app. Do not install or launch it during current app verification. Preserve original identifiers and provenance in historical artifacts.
