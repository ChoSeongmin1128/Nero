import AppKit

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

    var headerHeight: CGFloat { notch.map { $0.height + 4 } ?? 30 }
    var clockWidth: CGFloat {
        let clockWidth = ("99:59:59" as NSString).size(withAttributes: [.font: NSFont.monospacedDigitSystemFont(ofSize: RecorderTokens.caption, weight: .semibold)]).width
        return ceil(clockWidth)
    }
    var compactWidth: CGFloat {
        let wing = max(clockWidth + 38, 80)
        return notch.map { $0.width + wing * 2 } ?? clockWidth + 114
    }
    var center: CGFloat { notch?.midX ?? workArea.midX }

    func docked(expanded: Bool) -> NSRect {
        let width = min(workArea.width, expanded ? max(compactWidth, 276) : compactWidth)
        let height = headerHeight + (expanded ? 40 : 0)
        let top = notch == nil ? workArea.maxY : frame.maxY
        return NSRect(x: center - width / 2, y: top - height, width: width, height: height)
    }

    func clamped(_ rect: NSRect) -> NSRect {
        NSRect(x: min(max(rect.minX, workArea.minX), max(workArea.minX, workArea.maxX - rect.width)),
               y: min(max(rect.minY, workArea.minY), max(workArea.minY, workArea.maxY - rect.height)),
               width: rect.width, height: rect.height)
    }

    func acceptsDock(at point: NSPoint) -> Bool {
        abs(point.x - center) <= max(compactWidth / 2, 90) && point.y >= workArea.maxY - 52 && point.y <= frame.maxY
    }

    func timeFrame(in panel: NSRect, floating: Bool) -> NSRect {
        let x: CGFloat = floating ? 38 : 32
        return NSRect(x: x, y: (floating ? 44 : headerHeight) / 2 - 8,
                      width: floating ? 72 : clockWidth, height: 20)
    }
    func primaryControlFrame(index: Int, in panel: NSRect) -> NSRect {
        let start = notch.map { $0.maxX - panel.minX + 8 } ?? panel.width - 72
        return NSRect(x: start + CGFloat(index) * 32, y: (headerHeight - 28) / 2, width: 28, height: 28)
    }
}

struct RecorderPlacement: Codable, Equatable {
    var floating = false
    var displayID: UInt32 = 0
    var x: CGFloat = 0
    var y: CGFloat = 0

    mutating func resolve(displays: [RecorderDisplay], preferred: UInt32) -> RecorderDisplay? {
        guard let display = displays.first(where: { $0.id == displayID })
                ?? displays.first(where: { $0.id == preferred }) ?? displays.first else { return nil }
        displayID = display.id
        if floating {
            let position = display.clamped(NSRect(x: x, y: y, width: 276, height: 44))
            x = position.minX; y = position.minY
        }
        return display
    }
}
