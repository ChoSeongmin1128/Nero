import AppKit
import SwiftUI
import Testing
@testable import GroveApp

@MainActor
struct WorkspaceLayoutRenderingTests {
    @Test(.enabled(if: ProcessInfo.processInfo.environment["GROVE_WORKSPACE_LAYOUT_OUTPUT"] != nil))
    func renderWorkspaceAcrossNavigation() throws {
        _ = NSApplication.shared
        GroveTypography.registerFonts()
        let output = URL(fileURLWithPath: try #require(ProcessInfo.processInfo.environment["GROVE_WORKSPACE_LAYOUT_OUTPUT"]))
        let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: base) }
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        let store = GroveStore(baseDirectory: base)
        let meeting = MeetingRecord(title: "제품 검토 회의", startedAt: Date(), duration: 40, status: .failed,
            audioPath: nil, glossaryProfile: "", transcript: [], claims: [], errorMessage: "전사를 중단했습니다.")
        let processing = MeetingRecord(title: "가져온 녹음", startedAt: Date(), duration: 0, status: .processing,
            audioPath: nil, glossaryProfile: "", transcript: [], claims: [], errorMessage: nil)
        store.meetings = [meeting, processing]
        let host = NSHostingController(rootView: RootView(store: store).frame(minWidth: 980, minHeight: 640).environment(\.colorScheme, .dark))
        let window = NSWindow(contentViewController: host)
        window.styleMask = [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView]
        window.toolbarStyle = .unifiedCompact
        window.setFrame(NSRect(x: -10000, y: -10000, width: 1260, height: 780), display: false)
        window.appearance = NSAppearance(named: .darkAqua)
        defer { window.contentViewController = nil }
        let cases: [(String, SidebarDestination)] = [("library", .library), ("meeting", .meeting(meeting.id)), ("unfiled", .unfiled), ("processing", .meeting(processing.id)), ("meeting-again", .meeting(meeting.id))]
        for (name, selection) in cases {
            store.selection = selection
            RunLoop.main.run(until: Date().addingTimeInterval(0.3))
            window.contentView?.layoutSubtreeIfNeeded()
            func splits(in view: NSView) -> [NSSplitView] {
                (view as? NSSplitView).map { [$0] } ?? view.subviews.flatMap { splits(in: $0) }
            }
            let split = try #require(splits(in: host.view).first)
            let occupied = split.convert(split.bounds, to: host.view)
            #expect(occupied.minY >= host.view.bounds.minY - 1)
            #expect(occupied.maxY <= host.view.bounds.maxY + 1)
            let frame = try #require(window.contentView?.superview)
            let bitmap = try #require(frame.bitmapImageRepForCachingDisplay(in: frame.bounds))
            frame.cacheDisplay(in: frame.bounds, to: bitmap)
            try #require(bitmap.representation(using: .png, properties: [:])).write(to: output.appendingPathComponent(name + ".png"))
            print("workspace", name, "toolbar", window.toolbar != nil, "content", host.view.frame, "layout", window.contentLayoutRect, "safe", host.view.safeAreaInsets)
        }
    }
}
