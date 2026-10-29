import Foundation

struct NotionDiagnosticEntry: Codable, Sendable {
    enum Kind: String, Codable { case request, authentication, pollWait, export }
    let kind: Kind
    let operation: String
    let startedAt: Date
    let milliseconds: Double
    let statusCode: Int?
    let requestBytes: Int
    let responseBytes: Int
    let failed: Bool
}

@MainActor
final class NotionDiagnostics {
    private(set) var entries: [NotionDiagnosticEntry]
    private let file: URL?
    private let capacity: Int
    private let queue = DispatchQueue(label: "Grove.Notion.Diagnostics", qos: .utility)

    init(file: URL? = nil, capacity: Int = 100) {
        self.file = file
        self.capacity = max(1, capacity)
        let saved = file.flatMap { try? Data(contentsOf: $0) }
            .flatMap { try? JSONDecoder().decode([NotionDiagnosticEntry].self, from: $0) } ?? []
        entries = Array(saved.suffix(self.capacity))
    }

    func record(kind: NotionDiagnosticEntry.Kind, operation: String, startedAt: Date, elapsed: TimeInterval,
                statusCode: Int? = nil, requestBytes: Int = 0, responseBytes: Int = 0, failed: Bool = false) {
        let names: Set<String> = ["initialize", "notifications/initialized", "notion-fetch", "notion-update-page",
            "notion-create-pages", "notion-get-async-task", "save", "poll", "oauth-refresh"]
        entries.append(.init(kind: kind, operation: names.contains(operation) ? operation : "other", startedAt: startedAt,
            milliseconds: max(0, elapsed * 1000), statusCode: statusCode,
            requestBytes: requestBytes, responseBytes: responseBytes, failed: failed))
        entries = Array(entries.suffix(capacity))
        guard let file else { return }
        let snapshot = entries
        queue.async {
            do {
                try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
                try JSONEncoder().encode(snapshot).write(to: file, options: .atomic)
            } catch { }
        }
    }

    func flush() async {
        await withCheckedContinuation { continuation in queue.async { continuation.resume() } }
    }
}
