# Shared meeting entry points — 2026-10-30

The macOS and Windows application use the same library markup, handlers and
feature layout. Colors, typography, control appearance, icons and focus treatment
continue to come from the canonical `design-system/` sources. This change adds no
Windows-specific theme, component copy or generated-source override.

The library header exposes **새 회의 만들기** and **녹음 시작** even when the
navigation sidebar is collapsed. View filters and selection tools occupy the next
row. Empty-library, folder, search and trash messages explain their own state.

Create opens a draft and focuses its title. Quick recording checks model and
microphone readiness before creating a draft, retains the current folder, and
uses the same start helper as meeting details. An existing recording, paused or
finalizing session opens its current meeting without creating another one. A
failed native start preserves the new draft and displays the existing error UI.
The in-flight action guard blocks repeated clicks. Focus moves to the stable
recording control region, rather than a button replaced by subsequent rendering.

## Verification

- Frontend: 435 passing tests, including nine full-HTML/native-boundary flow tests.
- Native Windows desktop: 279 passing tests; five existing hardware acceptance
  tests explicitly ignored. Core and CLI suites also pass; desktop check and
  frontend production build pass.
- Actual production frontend in isolated Edge: 640/1200px, green/mono and
  light/dark (eight combinations). Checked visible actions, keyboard focus,
  sidebar open/close, creation, opened menu viewport bounds/Escape, quick record
  and return to the existing recording. No page exceptions. Native calls were
  controlled fixtures and never used the personal meeting library.
- The pinned Windows speech and summary executable packages pass staged integrity
  checks. Model weights are unchanged.

Browser checks establish layout and frontend behavior, not microphone capture,
real speech recognition, laptop memory/performance, OS permissions or macOS
native acceptance. Build38 is reserved for the Windows development installer;
Mac's separate native recording panel work and installed build37 are independent.
Generated screenshots, logs, fixture scripts and installation receipts stay in
ignored `.work/`; they are not product or private-library fixtures.
