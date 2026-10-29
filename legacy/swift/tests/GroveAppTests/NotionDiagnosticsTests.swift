import Foundation
import Testing
@testable import GroveApp

@MainActor
struct NotionDiagnosticsTests {
    @Test func diagnosticsKeepOnlyBoundedTimingMetadataAndFlushInOrder() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let file = root.appendingPathComponent("diagnostics.json")
        let diagnostics = NotionDiagnostics(file: file, capacity: 2)
        diagnostics.record(kind: .request, operation: "initialize", startedAt: .now, elapsed: 0.01)
        diagnostics.record(kind: .request, operation: "private-token-and-page-content", startedAt: .now, elapsed: 0.1,
            statusCode: 200, requestBytes: 128, responseBytes: 256)
        diagnostics.record(kind: .export, operation: "save", startedAt: .now, elapsed: 0.5, failed: true)
        await diagnostics.flush()
        let data = try Data(contentsOf: file)
        #expect(!String(decoding: data, as: UTF8.self).contains("private-token-and-page-content"))
        let reopened = NotionDiagnostics(file: file, capacity: 2)
        #expect(reopened.entries.count == 2)
        #expect(reopened.entries.map(\.operation) == ["other", "save"])
        #expect(reopened.entries.map(\.milliseconds) == [100, 500])
        #expect(reopened.entries.last?.failed == true)
        let objects = try #require(JSONSerialization.jsonObject(with: data) as? [[String: Any]])
        #expect(objects.allSatisfy { Set($0.keys).isSubset(of: ["kind", "operation", "startedAt", "milliseconds", "statusCode", "requestBytes", "responseBytes", "failed"]) })
    }
}
