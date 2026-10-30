# macOS recording panel

The macOS presenter is an AppKit nonactivating panel. Windows retains the Tauri
recorder window. Both presenters use `RecorderPolicy` for visibility and the same
pause, resume and stop action in `main.rs`; stop still enters the existing saved
recording/transcription path.

`macos_recorder.rs` sends only the current capture identity, title, elapsed time
and recording diagnostics. Swift owns drawing, hover, drag placement and monitor
recovery. Capture state and audio never live in the presenter. Actions include the
capture identity so an old panel cannot modify or dismiss a later recording.

`RecorderPlacement` uses AppKit global points. The dock attaches to the hardware
notch from `safeAreaInsets` and the auxiliary top areas. Without a notch it sits
at the top center of the visible work area. The recording mark and clock stay left of the notch; pause/resume and stop
are always visible on the right. Both wings reserve the measured clock width,
including hour-long values. Floating placement is clamped when a
monitor disappears. Only the placement is saved in `nero.recorderPlacement.v1`.

Hover reveals only auxiliary actions (open meeting and hide); clicking the
status also keeps those actions available for accessibility. Dragging detaches the panel, and dropping near the top
center or choosing "상단에 붙이기" docks it again. Hiding the panel does not stop
recording; the existing main-window recording action can show it again.

The panel cannot become the key or main window. Pause, resume, stop and movement
do not activate Nero. "회의 열기" deliberately opens the main window. Display,
wake and active-Space notifications refresh placement. Native resizing respects
Reduce Motion. Full-screen/Spaces flags require hardware acceptance beyond unit
tests before a release.

The build script compiles the Swift sources into a static library on macOS only.
Overlay colors and caption size are generated from the canonical
`design-system/desktop-preview.css` and `design-system/typography.css`. There is no
second palette or bundled model/runtime for this view.

## Local checks

From the repository root on macOS with Xcode:

```sh
python3 -B scripts/native-recorder/build.py --out .work/recorder-check --target aarch64-apple-darwin
xcrun swiftc -swift-version 5 apps/desktop/src-tauri/native/recorder/RecorderPlacement.swift .work/recorder-check/RecorderTokens.swift apps/desktop/src-tauri/native/recorder/tests/RecorderPlacementTests.swift -o .work/recorder-check/placement-tests
.work/recorder-check/placement-tests
```

The optional isolated native UI executable uses no user library or microphone:

```sh
xcrun swiftc -swift-version 5 apps/desktop/src-tauri/native/recorder/RecorderPlacement.swift apps/desktop/src-tauri/native/recorder/RecorderPanel.swift .work/recorder-check/RecorderTokens.swift apps/desktop/src-tauri/native/recorder/tests/RecorderPanelHarness.swift -o .work/recorder-check/NeroPanelTest
.work/recorder-check/NeroPanelTest --self-test --long-time
```

`--self-test` checks late-update/hide/reopen ordering against the actual panel.
`--long-time` displays an example `1:23:45` clock. `--exercise-drag` dispatches local
AppKit mouse events to the panel without injecting global input; it validates the
native event path but does not replace physical mouse/trackpad acceptance. All
times and actions in this executable are test fixtures. Its placement store is
disabled. Quit the helper and retire the generated files after validation.
