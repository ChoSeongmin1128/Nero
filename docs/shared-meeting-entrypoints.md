# Shared meeting entry points

The macOS and Windows application use the same library markup, handlers and
feature layout. Colors, typography, control appearance, icons and focus treatment
continue to come from the canonical `design-system/` sources. This change adds no
Windows-specific theme, component copy or generated-source override.

Home is the first screen: today's calendar events, each with a record action, and
**녹음 시작**, **빈 회의** and **파일 가져오기** without an event. The central library
header (목록 크게 보기) also exposes **새 회의 만들기** and **녹음 시작**. View filters and selection tools occupy the next
row. Empty-library, folder, search and trash messages explain their own state.

Create opens a draft and focuses its title. Quick recording checks model and
microphone readiness before creating a draft, retains the current folder, and
uses the same start helper as meeting details. An existing recording, paused or
finalizing session opens its current meeting without creating another one. A
failed native start preserves the new draft and displays the existing error UI.
The in-flight action guard blocks repeated clicks. Focus moves to the stable
recording control region, rather than a button replaced by subsequent rendering.
