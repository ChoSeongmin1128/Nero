# Shared meeting entry points

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
