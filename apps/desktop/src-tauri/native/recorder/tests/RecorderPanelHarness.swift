import AppKit

@MainActor private var harness: PanelHarness?

@main struct PanelHarnessMain {
    @MainActor static func main() {
        NSApplication.shared.setActivationPolicy(.accessory)
        harness = PanelHarness()
        harness?.start()
        NSApplication.shared.run()
    }
}

@MainActor final class PanelHarness {
    var presenter: RecorderPresenter!
    var revision: UInt64 = 1
    var phase = "recording"
    var elapsed: UInt64 = 95_000
    private var geometry = ""
    private var timer: Timer?
    private var events: Any?

    func start() {
        if ProcessInfo.processInfo.arguments.contains("--long-time") { elapsed = 5_025_000 }
        presenter = RecorderPresenter(callback: { action, session in
            let action = String(cString: action), session = String(cString: session)
            MainActor.assumeIsolated { harness?.perform(action, session: session) }
        }, defaults: nil)
        if ProcessInfo.processInfo.arguments.contains("--notch-display"),
           let screen = NSScreen.screens.first(where: { RecorderDisplay(screen: $0).notch != nil }) {
            presenter.placement.displayID = RecorderDisplay(screen: screen).id
        }
        presenter.panel.title = "Nero 녹음 패널 테스트"
        if ProcessInfo.processInfo.arguments.contains("--self-test") { exerciseOrdering() }
        update()
        if ProcessInfo.processInfo.arguments.contains("--self-test") { exercisePrimaryControls() }
        events = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .leftMouseDragged, .leftMouseUp]) { [weak self] event in
            MainActor.assumeIsolated { self?.report(["event": "mouse", "type": event.type.rawValue, "window": event.windowNumber, "point": NSStringFromPoint(event.locationInWindow)]) }
            return event
        }
        report(["event": "launched", "pid": ProcessInfo.processInfo.processIdentifier, "window": presenter.panel.windowNumber,
                "displays": NSScreen.screens.map { screen in
                    let display = RecorderDisplay(screen: screen)
                    return ["id": display.id, "notch": display.notch != nil, "frame": NSStringFromRect(display.frame), "workArea": NSStringFromRect(display.workArea)] as [String: Any]
                }])
        timer = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                let signature = "\(self.presenter.panel.frame):\(self.presenter.expanded):\(self.presenter.placement.floating):\(self.presenter.panel.isVisible)"
                if signature != self.geometry {
                    self.geometry = signature
                    self.report(["event": "geometry", "frame": NSStringFromRect(self.presenter.panel.frame), "expanded": self.presenter.expanded, "floating": self.presenter.placement.floating, "visible": self.presenter.panel.isVisible,
                                 "key": self.presenter.panel.isKeyWindow, "main": self.presenter.panel.isMainWindow,
                                 "frontmostPID": NSWorkspace.shared.frontmostApplication?.processIdentifier ?? 0])
                }
            }
        }
        if ProcessInfo.processInfo.arguments.contains("--exercise-drag") {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) { self.exerciseDrag() }
        }
    }

    func exerciseOrdering() {
        func value(_ generation: UInt64, _ revision: UInt64, _ visible: Bool) -> RecorderUpdate {
            RecorderUpdate(generation: generation, revision: revision, visible: visible, sessionID: "ordering-test", meetingTitle: "패널 상태 검증", phase: "recording", elapsedMS: 95_000, error: nil, warning: nil, preferredDisplay: 0)
        }
        presenter.update(value(10, 10, true))
        assert(presenter.panel.isVisible)
        presenter.hide(generation: 12)
        presenter.update(value(11, 11, true))
        assert(!presenter.panel.isVisible)
        presenter.update(value(13, 13, true))
        assert(presenter.panel.isVisible)
        presenter.update(value(14, 12, false))
        assert(presenter.panel.isVisible)
        presenter.update(value(15, 15, false))
        assert(!presenter.panel.isVisible)
        revision = 16
        report(["event": "ordering-tests", "passed": true, "cases": ["late update after hide", "explicit reopen", "older snapshot", "capture ends"]])
    }

    func exercisePrimaryControls() {
        func button(_ id: String) -> NSButton? {
            presenter.surface.subviews.compactMap { $0 as? NSButton }.first { $0.identifier?.rawValue == id }
        }
        assert(!presenter.expanded)
        assert(button("pause")?.isEnabled == true && button("stop")?.isEnabled == true)
        assert(button("open") == nil && button("hide") == nil)
        button("pause")!.performClick(nil)
        assert(phase == "paused" && button("pause")?.accessibilityLabel() == "계속 녹음")
        button("pause")!.performClick(nil)
        assert(phase == "recording")
        phase = "finalizing"; revision += 1; update()
        assert(button("pause")?.isEnabled == false && button("stop")?.isEnabled == false)
        phase = "recording"; revision += 1; update()
        presenter.toggleControls()
        assert(button("open") != nil && button("hide") != nil)
        presenter.toggleControls()
        assert(button("open") == nil && button("pause") != nil)
        report(["event": "primary-control-tests", "passed": true,
                "cases": ["collapsed controls", "pause", "resume", "finalizing disabled", "auxiliary expansion"]])
    }

    func exerciseDrag() {
        let start = NSPoint(x: presenter.panel.frame.minX + 20, y: presenter.panel.frame.maxY - 15)
        let end = NSPoint(x: start.x + 100, y: start.y - 180)
        for (kind, point) in [(NSEvent.EventType.leftMouseDown, start), (.leftMouseDragged, end), (.leftMouseUp, end)] {
            let event = NSEvent.mouseEvent(with: kind, location: presenter.panel.convertPoint(fromScreen: point), modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: presenter.panel.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1)!
            presenter.panel.sendEvent(event)
        }
        report(["event": "native-event-drag", "floating": presenter.placement.floating, "frame": NSStringFromRect(presenter.panel.frame), "input": "local AppKit events, no global mouse injection"])
    }

    func update(visible: Bool = true) {
        presenter.update(RecorderUpdate(generation: revision, revision: revision, visible: visible, sessionID: "isolated-test", meetingTitle: "녹음 패널 테스트", phase: phase, elapsedMS: elapsed, error: nil, warning: nil, preferredDisplay: 0))
    }

    func perform(_ action: String, session: String) {
        report(["event": "action", "action": action, "session": session])
        revision += 1
        if action == "pause" { phase = "paused" }
        if action == "resume" { phase = "recording" }
        if action == "stop" || action == "hide" { presenter.hide() }
        else { update() }
        presenter.complete(session: session, error: nil)
    }

    func report(_ data: [String: Any]) {
        if let bytes = try? JSONSerialization.data(withJSONObject: data, options: [.sortedKeys]), let text = String(data: bytes, encoding: .utf8) {
            print(text); fflush(stdout)
        }
    }
}
