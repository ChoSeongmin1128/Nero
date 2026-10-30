import AppKit

typealias RecorderCallback = @convention(c) (UnsafePointer<CChar>, UnsafePointer<CChar>) -> Void

struct RecorderUpdate: Decodable {
    let generation: UInt64
    let revision: UInt64
    let visible: Bool
    let sessionID: String?
    let meetingTitle: String
    let phase: String
    let elapsedMS: UInt64
    let error: String?
    let warning: String?
    let preferredDisplay: UInt32
}

@MainActor final class RecorderPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    override func animationResizeTime(_ newFrame: NSRect) -> TimeInterval { 0.16 }
}

@MainActor final class RecorderSurface: NSView {
    weak var presenter: RecorderPresenter?
    private var tracking: NSTrackingArea?
    private var down: NSPoint?
    private var dragged = false
    override var isFlipped: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func updateTrackingAreas() {
        if let tracking { removeTrackingArea(tracking) }
        tracking = NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self)
        addTrackingArea(tracking!)
        super.updateTrackingAreas()
    }
    override func mouseEntered(with event: NSEvent) { presenter?.hover(true) }
    override func mouseExited(with event: NSEvent) { presenter?.hover(false) }
    override func mouseDown(with event: NSEvent) {
        down = window?.convertPoint(toScreen: event.locationInWindow) ?? NSEvent.mouseLocation
        dragged = false
    }
    override func mouseDragged(with event: NSEvent) {
        guard let down else { return }
        let point = window?.convertPoint(toScreen: event.locationInWindow) ?? NSEvent.mouseLocation
        if !dragged {
            guard hypot(point.x - down.x, point.y - down.y) >= 4 else { return }
            dragged = true; presenter?.beginDrag(at: down)
        }
        presenter?.moveDrag(to: point)
    }
    override func mouseUp(with event: NSEvent) {
        if dragged { presenter?.endDrag(at: window?.convertPoint(toScreen: event.locationInWindow) ?? NSEvent.mouseLocation) }
        else if down != nil { presenter?.toggleControls() }
        down = nil
    }
    override func accessibilityPerformPress() -> Bool { presenter?.toggleControls(); return true }

    override func draw(_ dirtyRect: NSRect) {
        guard let presenter, let state = presenter.state else { return }
        let floating = presenter.placement.floating
        (floating ? RecorderTokens.surface : NSColor.black).setFill()
        NSBezierPath(roundedRect: bounds, xRadius: floating ? 21 : 12, yRadius: floating ? 21 : 12).fill()
        if !floating {
            NSRect(x: 0, y: 0, width: bounds.width, height: 12).fill()
        } else {
            RecorderTokens.edge.setStroke()
            NSBezierPath(roundedRect: bounds.insetBy(dx: 0.5, dy: 0.5), xRadius: 21, yRadius: 21).stroke()
        }
        let header = floating ? 44 : presenter.display?.headerHeight ?? 30
        let centerY = header / 2
        let markX: CGFloat = floating ? 24 : presenter.display?.notch == nil ? 17 : 20
        if presenter.failure != nil || state.warning != nil {
            NSImage(systemSymbolName: "exclamationmark.triangle", accessibilityDescription: "녹음 확인 필요")?
                .withSymbolConfiguration(.init(paletteColors: [RecorderTokens.warning]))?
                .draw(in: NSRect(x: markX - 7, y: centerY - 7, width: 14, height: 14))
        } else {
            (state.phase == "paused" ? RecorderTokens.warning : RecorderTokens.recording).setFill()
            NSBezierPath(ovalIn: NSRect(x: markX - 3, y: centerY - 3, width: 6, height: 6)).fill()
        }
        let seconds = state.elapsedMS / 1000
        let time = seconds >= 3600 ? String(format: "%llu:%02llu:%02llu", seconds / 3600, seconds / 60 % 60, seconds % 60)
                                  : String(format: "%02llu:%02llu", seconds / 60, seconds % 60)
        let text = presenter.failure != nil ? "오류 확인" : time
        (text as NSString).draw(in: presenter.display?.timeFrame(in: presenter.panel.frame, floating: floating) ?? .zero,
                               withAttributes: [.font: NSFont.monospacedDigitSystemFont(ofSize: RecorderTokens.caption, weight: .semibold), .foregroundColor: RecorderTokens.text])
        if !floating && presenter.expanded {
            RecorderTokens.muted.setFill()
            for x in [CGFloat(20), 24] {
                for y in [header + 14, header + 19, header + 24] {
                    NSBezierPath(ovalIn: NSRect(x: x, y: y, width: 1.5, height: 1.5)).fill()
                }
            }
        }
    }
}

@MainActor final class RecorderPresenter: NSObject {
    let panel = RecorderPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
    let surface = RecorderSurface()
    let callback: RecorderCallback
    let defaults: UserDefaults?
    var placement: RecorderPlacement
    var state: RecorderUpdate?
    var display: RecorderDisplay?
    private(set) var expanded = false
    private var dragging = false
    private var dragOffset = NSPoint.zero
    private var pendingSession: String?
    private var actionError: String?
    private var collapse: DispatchWorkItem?
    private var shown = false
    private var explicitExpansion = false
    private var generation: UInt64 = 0
    private var observers: [(NotificationCenter, NSObjectProtocol)] = []
    private var buttons: [NSButton] = []
    private var lastControls = ""
    var failure: String? { actionError ?? state?.error }

    init(callback: @escaping RecorderCallback, defaults: UserDefaults? = .standard) {
        self.callback = callback; self.defaults = defaults
        placement = defaults?.data(forKey: "nero.recorderPlacement.v1")
            .flatMap { try? JSONDecoder().decode(RecorderPlacement.self, from: $0) } ?? RecorderPlacement()
        super.init()
        panel.isOpaque = false; panel.backgroundColor = .clear; panel.hasShadow = false
        panel.isFloatingPanel = true; panel.hidesOnDeactivate = false; panel.becomesKeyOnlyIfNeeded = true
        panel.acceptsMouseMovedEvents = true
        panel.level = NSWindow.Level(rawValue: NSWindow.Level.statusBar.rawValue + 1)
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        panel.isReleasedWhenClosed = false
        panel.title = "Nero 녹음 제어"
        panel.contentView = surface; surface.presenter = self
        surface.setAccessibilityElement(true); surface.setAccessibilityRole(.button)
        let center = NotificationCenter.default
        observers.append((center, center.addObserver(forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.refreshEnvironment() }
        }))
        let workspace = NSWorkspace.shared.notificationCenter
        for name in [NSWorkspace.activeSpaceDidChangeNotification, NSWorkspace.didWakeNotification] {
            observers.append((workspace, workspace.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.refreshEnvironment() }
            }))
        }
    }

    func update(_ next: RecorderUpdate) {
        guard next.generation >= generation else { return }
        generation = next.generation
        if let state, next.revision < state.revision { return }
        if state?.sessionID != next.sessionID { pendingSession = nil; actionError = nil; expanded = false; explicitExpansion = false }
        state = next
        guard next.visible, next.sessionID != nil else { hide(); return }
        shown = true
        refreshEnvironment()
    }

    func hide() { shown = false; collapse?.cancel(); expanded = false; explicitExpansion = false; panel.orderOut(nil) }

    func hide(generation next: UInt64) {
        guard next >= generation else { return }
        generation = next; hide()
    }

    func refreshEnvironment() {
        guard shown, !dragging else { return }
        let displays = NSScreen.screens.map(RecorderDisplay.init)
        let preferredScreen = NSApp.windows.first { !($0 is NSPanel) && $0.isVisible }?.screen ?? NSScreen.main
        let preferred = preferredScreen.map { RecorderDisplay(screen: $0).id } ?? state?.preferredDisplay ?? 0
        guard let resolved = placement.resolve(displays: displays, preferred: preferred) else { return }
        display = resolved
        layout(animated: false)
        panel.orderFrontRegardless()
    }

    private func layout(animated: Bool) {
        guard let display else { return }
        let target = placement.floating ? NSRect(x: placement.x, y: placement.y, width: 276, height: 44)
                                        : display.docked(expanded: expanded)
        let animate = animated && panel.isVisible && !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        if panel.frame != target { panel.setFrame(target, display: true, animate: animate) }
        let controlsKey = "\(placement.floating):\(expanded):\(target.width):\(display.headerHeight):\(state?.phase ?? ""):\(pendingSession != nil)"
        if controlsKey != lastControls {
            lastControls = controlsKey
            for button in buttons { button.removeFromSuperview() }; buttons = []
            let primary: [(String, String, String)] = [
                ("pause", state?.phase == "paused" ? "play.fill" : "pause", state?.phase == "paused" ? "계속 녹음" : "일시정지"),
                ("stop", "stop", "녹음 종료"),
            ]
            let auxiliary: [(String, String, String)] = [("open", "arrow.up.right.square", "회의 열기")]
                + (placement.floating ? [("dock", "arrow.up.to.line", "상단에 붙이기")] : [])
                + [("hide", "xmark", "녹음 표시 숨기기")]
            let actions = placement.floating ? [auxiliary[0]] + primary + Array(auxiliary.dropFirst())
                                             : primary + (expanded ? auxiliary : [])
            for (index, action) in actions.enumerated() {
                let button = NSButton(image: NSImage(systemSymbolName: action.1, accessibilityDescription: action.2)!, target: self, action: #selector(pressed(_:)))
                button.identifier = NSUserInterfaceItemIdentifier(action.0)
                button.image = button.image?.withSymbolConfiguration(.init(pointSize: RecorderTokens.caption, weight: .medium))
                button.isBordered = false; button.contentTintColor = action.0 == "stop" ? RecorderTokens.recording : RecorderTokens.text
                button.setAccessibilityLabel(action.2); button.toolTip = action.2
                button.isEnabled = action.0 == "hide" || action.0 == "dock" || (pendingSession == nil && (action.0 == "open" || state?.phase != "finalizing"))
                if placement.floating {
                    button.frame = NSRect(x: 112 + CGFloat(index) * 32, y: 8, width: 28, height: 28)
                } else if index < primary.count {
                    button.frame = display.primaryControlFrame(index: index, in: target)
                } else {
                    button.frame = NSRect(x: target.width - CGFloat(auxiliary.count) * 32 - 10 + CGFloat(index - primary.count) * 32,
                                          y: display.headerHeight + 6, width: 28, height: 28)
                }
                surface.addSubview(button); buttons.append(button)
            }
        }
        surface.toolTip = [state?.meetingTitle, actionError ?? state?.error ?? state?.warning].compactMap { $0 }.joined(separator: "\n")
        surface.setAccessibilityRole(placement.floating ? .group : .button)
        surface.setAccessibilityLabel("\(state?.meetingTitle ?? "회의"), \(state?.phase == "paused" ? "일시정지" : "녹음 중"), 녹음 제어" + (placement.floating ? "" : explicitExpansion ? " 접기" : " 펼치기"))
        surface.needsDisplay = true
    }

    func hover(_ inside: Bool) {
        collapse?.cancel()
        guard !placement.floating, !dragging, !explicitExpansion else { return }
        if inside { expanded = true; layout(animated: true) }
        else {
            let task = DispatchWorkItem { [weak self] in
                guard let self, !self.panel.frame.contains(NSEvent.mouseLocation), !self.dragging else { return }
                self.expanded = false; self.layout(animated: true)
            }
            collapse = task; DispatchQueue.main.asyncAfter(deadline: .now() + 0.2, execute: task)
        }
    }

    func toggleControls() {
        guard !placement.floating else { return }
        collapse?.cancel(); explicitExpansion.toggle(); expanded = explicitExpansion
        layout(animated: true)
    }

    func beginDrag(at pointer: NSPoint) {
        guard display != nil else { return }
        collapse?.cancel(); dragging = true; expanded = false; explicitExpansion = false
        dragOffset = NSPoint(x: min(max(pointer.x - panel.frame.minX, 12), 264), y: min(max(pointer.y - panel.frame.minY, 8), 36))
        placement.floating = true
        moveDrag(to: pointer)
    }

    func moveDrag(to pointer: NSPoint) {
        guard dragging else { return }
        placement.x = pointer.x - dragOffset.x; placement.y = pointer.y - dragOffset.y
        layout(animated: false)
    }

    func endDrag(at point: NSPoint) {
        guard dragging else { return }
        dragging = false
        let targetScreen = NSScreen.screens.first { $0.frame.contains(point) }
        if let targetScreen { self.display = RecorderDisplay(screen: targetScreen) }
        guard let target = self.display else { return }
        placement.displayID = target.id
        placement.floating = !target.acceptsDock(at: point)
        let position = target.clamped(panel.frame)
        placement.x = position.minX; placement.y = position.minY
        persist(); layout(animated: true)
    }

    @objc private func pressed(_ sender: NSButton) { if let action = sender.identifier?.rawValue { send(action) } }

    func send(_ action: String) {
        if action == "dock" { placement.floating = false; expanded = false; persist(); layout(animated: true); return }
        guard let id = state?.sessionID else { return }
        if action != "hide" {
            guard pendingSession == nil else { return }
            pendingSession = id
        }
        actionError = nil
        layout(animated: false)
        let request = action == "pause" && state?.phase == "paused" ? "resume" : action
        request.withCString { actionPointer in id.withCString { callback(actionPointer, $0) } }
    }

    func complete(session: String, error: String?) {
        guard pendingSession == session else { return }
        pendingSession = nil
        if state?.sessionID == session { actionError = error }
        layout(animated: false)
        if let error { NSAccessibility.post(element: surface, notification: .announcementRequested, userInfo: [.announcement: error, .priority: NSAccessibilityPriorityLevel.high.rawValue]) }
    }

    private func persist() { if let data = try? JSONEncoder().encode(placement) { defaults?.set(data, forKey: "nero.recorderPlacement.v1") } }
}

@MainActor private var recorderPresenter: RecorderPresenter?

@_cdecl("nero_recorder_update") @MainActor
func neroRecorderUpdate(_ bytes: UnsafePointer<UInt8>, _ count: Int, _ callback: @escaping RecorderCallback) {
    guard let state = try? JSONDecoder().decode(RecorderUpdate.self, from: Data(bytes: bytes, count: count)) else { return }
    if recorderPresenter == nil { recorderPresenter = RecorderPresenter(callback: callback) }
    recorderPresenter?.update(state)
}

@_cdecl("nero_recorder_hide") @MainActor
func neroRecorderHide(_ generation: UInt64) { recorderPresenter?.hide(generation: generation) }

@_cdecl("nero_recorder_complete") @MainActor
func neroRecorderComplete(_ session: UnsafePointer<CChar>, _ error: UnsafePointer<CChar>?) {
    recorderPresenter?.complete(session: String(cString: session), error: error.map { String(cString: $0) })
}
