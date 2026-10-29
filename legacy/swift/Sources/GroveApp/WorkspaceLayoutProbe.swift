#if DEBUG
import AppKit
import SwiftUI

struct WorkspaceLayoutProbe: NSViewRepresentable {
    let state: String
    func makeNSView(context: Context) -> ProbeView { ProbeView() }
    func updateNSView(_ view: ProbeView, context: Context) { view.state = state; view.record() }

    final class ProbeView: NSView {
        var state = ""
        override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); record() }
        override func layout() { super.layout(); record() }
        func record() {
            let args = CommandLine.arguments
            guard let i = args.firstIndex(of: "--qa-profile"), args.indices.contains(i + 1), let window,
                  let content = window.contentView else { return }
            func tree(_ view: NSView, depth: Int) -> [String: Any] {
                var value: [String: Any] = ["type": String(describing: type(of: view)), "frame": NSStringFromRect(view.convert(view.bounds, to: nil)), "safeTop": view.safeAreaInsets.top]
                if depth < 6 { value["children"] = view.subviews.map { tree($0, depth: depth + 1) } }
                return value
            }
            let row: [String: Any] = ["date": Date().timeIntervalSince1970, "state": state, "views": tree(content, depth: 0), "layoutRect": NSStringFromRect(window.contentLayoutRect),
                "contentFrame": NSStringFromRect(content.frame), "safeTop": content.safeAreaInsets.top,
                "fullSize": window.styleMask.contains(.fullSizeContentView), "sheetAttached": window.attachedSheet != nil,
                "superviewSafeTop": superview?.safeAreaInsets.top ?? -1]
            guard let data = try? JSONSerialization.data(withJSONObject: row) else { return }
            let file = URL(fileURLWithPath: args[i + 1]).appendingPathComponent("window-layout.jsonl")
            if !FileManager.default.fileExists(atPath: file.path) { FileManager.default.createFile(atPath: file.path, contents: nil) }
            guard let handle = try? FileHandle(forWritingTo: file) else { return }
            defer { try? handle.close() }
            _ = try? handle.seekToEnd(); try? handle.write(contentsOf: data + Data([10]))
        }
    }
}
#endif
