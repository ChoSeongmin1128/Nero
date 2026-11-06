import AppKit

// One outer margin on both sides in every state: the status starts at `inset` and the
// last button ends `inset` before the edge. Widths follow the clock that is shown, so the
// panel grows only when an hour passes instead of always reserving "99:59:59".
enum RecorderMetrics {
    static let inset: CGFloat = 14
    static let button: CGFloat = 28
    static let spacing: CGFloat = 4
    static let groupGap: CGFloat = 12
    static let mark: CGFloat = 6
    static let markGap: CGFloat = 8
    static let notchGap: CGFloat = 10
    static let detailHeight: CGFloat = 34
    static let floatingHeight: CGFloat = 44
    static let plainHeader: CGFloat = 32
    static var font: NSFont { .monospacedDigitSystemFont(ofSize: RecorderTokens.caption, weight: .semibold) }

    static func clock(_ seconds: UInt64) -> String {
        seconds >= 3600 ? String(format: "%llu:%02llu:%02llu", seconds / 3600, seconds / 60 % 60, seconds % 60)
                        : String(format: "%02llu:%02llu", seconds / 60, seconds % 60)
    }
    static func clockWidth(_ seconds: UInt64) -> CGFloat {
        let template = seconds >= 36000 ? "00:00:00" : seconds >= 3600 ? "0:00:00" : "00:00"
        return ceil((template as NSString).size(withAttributes: [.font: font]).width)
    }
    // The shown text decides the width: the clock, or the wider failure label.
    static func textWidth(seconds: UInt64, failure: Bool) -> CGFloat {
        let clock = clockWidth(seconds)
        return failure ? max(clock, ceil(("오류 확인" as NSString).size(withAttributes: [.font: font]).width)) : clock
    }
    static func status(_ text: CGFloat) -> CGFloat { mark + markGap + text }
    static func buttons(_ count: Int) -> CGFloat { CGFloat(count) * button + CGFloat(max(count - 1, 0)) * spacing }
    static func floatingWidth(_ text: CGFloat) -> CGFloat {
        inset + status(text) + groupGap + buttons(2) + groupGap + buttons(3) + inset
    }
    // Right-aligned buttons that end `inset` before the panel edge.
    static func trailing(index: Int, count: Int, width: CGFloat, y: CGFloat) -> NSRect {
        NSRect(x: width - inset - buttons(count) + CGFloat(index) * (button + spacing), y: y, width: button, height: button)
    }
}

struct RecorderDisplay: Equatable {
    let id: UInt32
    let frame: NSRect
    let workArea: NSRect
    let notch: NSRect?

    @MainActor init(screen: NSScreen) {
        id = (screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value ?? 0
        frame = screen.frame
        workArea = screen.visibleFrame
        if screen.safeAreaInsets.top > 0, let left = screen.auxiliaryTopLeftArea,
           let right = screen.auxiliaryTopRightArea, left.width > 0, right.minX > left.maxX {
            notch = NSRect(x: left.maxX, y: frame.maxY - screen.safeAreaInsets.top,
                           width: right.minX - left.maxX, height: screen.safeAreaInsets.top)
        } else { notch = nil }
    }

    init(id: UInt32, frame: NSRect, workArea: NSRect, notch: NSRect? = nil) {
        self.id = id; self.frame = frame; self.workArea = workArea; self.notch = notch
    }

    var headerHeight: CGFloat { notch.map { $0.height + 4 } ?? RecorderMetrics.plainHeader }
    var center: CGFloat { notch?.midX ?? workArea.midX }

    // Both wings take the wider side, so the panel stays centered on the notch.
    func compactWidth(text: CGFloat) -> CGFloat {
        let content = max(RecorderMetrics.status(text), RecorderMetrics.buttons(2))
        guard let notch else {
            return RecorderMetrics.inset * 2 + RecorderMetrics.status(text) + RecorderMetrics.groupGap + RecorderMetrics.buttons(2)
        }
        return notch.width + 2 * (RecorderMetrics.inset + content + RecorderMetrics.notchGap)
    }

    func docked(expanded: Bool, text: CGFloat = RecorderMetrics.clockWidth(0)) -> NSRect {
        let width = min(workArea.width, expanded ? max(compactWidth(text: text), 260) : compactWidth(text: text))
        let height = headerHeight + (expanded ? RecorderMetrics.detailHeight : 0)
        let top = notch == nil ? workArea.maxY : frame.maxY
        return NSRect(x: center - width / 2, y: top - height, width: width, height: height)
    }

    func clamped(_ rect: NSRect) -> NSRect {
        NSRect(x: min(max(rect.minX, workArea.minX), max(workArea.minX, workArea.maxX - rect.width)),
               y: min(max(rect.minY, workArea.minY), max(workArea.minY, workArea.maxY - rect.height)),
               width: rect.width, height: rect.height)
    }

    func acceptsDock(at point: NSPoint) -> Bool {
        abs(point.x - center) <= max(compactWidth(text: RecorderMetrics.clockWidth(0)) / 2, 90) && point.y >= workArea.maxY - 52 && point.y <= frame.maxY
    }

    func markFrame(floating: Bool) -> NSRect {
        let header = floating ? RecorderMetrics.floatingHeight : headerHeight
        return NSRect(x: RecorderMetrics.inset, y: header / 2 - RecorderMetrics.mark / 2, width: RecorderMetrics.mark, height: RecorderMetrics.mark)
    }
    func timeFrame(text: CGFloat, floating: Bool) -> NSRect {
        let header = floating ? RecorderMetrics.floatingHeight : headerHeight
        return NSRect(x: RecorderMetrics.inset + RecorderMetrics.mark + RecorderMetrics.markGap, y: header / 2 - 8,
                      width: text, height: 20)
    }
    func primaryControlFrame(index: Int, in panel: NSRect) -> NSRect {
        RecorderMetrics.trailing(index: index, count: 2, width: panel.width, y: (headerHeight - RecorderMetrics.button) / 2)
    }
    // The detail row shares the right edge with pause and stop above it.
    func detailControlFrame(index: Int, count: Int, in panel: NSRect) -> NSRect {
        RecorderMetrics.trailing(index: index, count: count, width: panel.width,
                                 y: headerHeight + (RecorderMetrics.detailHeight - RecorderMetrics.button) / 2)
    }
    func titleFrame(controls: Int, in panel: NSRect) -> NSRect {
        let width = panel.width - RecorderMetrics.inset * 2 - RecorderMetrics.buttons(controls) - RecorderMetrics.groupGap
        return NSRect(x: RecorderMetrics.inset, y: headerHeight + RecorderMetrics.detailHeight / 2 - 8, width: max(0, width), height: 18)
    }
    // Floating order: status, pause and stop, then open, dock and hide after a wider gap.
    static func floatingControlFrame(index: Int, width: CGFloat) -> NSRect {
        let y = (RecorderMetrics.floatingHeight - RecorderMetrics.button) / 2
        if index < 2 {
            let start = width - RecorderMetrics.inset - RecorderMetrics.buttons(3) - RecorderMetrics.groupGap - RecorderMetrics.buttons(2)
            return NSRect(x: start + CGFloat(index) * (RecorderMetrics.button + RecorderMetrics.spacing), y: y, width: RecorderMetrics.button, height: RecorderMetrics.button)
        }
        return RecorderMetrics.trailing(index: index - 2, count: 3, width: width, y: y)
    }
}

struct RecorderPlacement: Codable, Equatable {
    var floating = false
    var displayID: UInt32 = 0
    var x: CGFloat = 0
    var y: CGFloat = 0

    mutating func resolve(displays: [RecorderDisplay], preferred: UInt32,
                          floatingSize: NSSize = NSSize(width: RecorderMetrics.floatingWidth(RecorderMetrics.clockWidth(0)), height: RecorderMetrics.floatingHeight)) -> RecorderDisplay? {
        guard let display = displays.first(where: { $0.id == displayID })
                ?? displays.first(where: { $0.id == preferred }) ?? displays.first else { return nil }
        displayID = display.id
        if floating {
            let position = display.clamped(NSRect(origin: NSPoint(x: x, y: y), size: floatingSize))
            x = position.minX; y = position.minY
        }
        return display
    }
}
