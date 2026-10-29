import Foundation
import Testing
@testable import GroveApp

struct MeetingRecordStorageTests {
    @Test func corruptIndexCannotBeReplacedWithEmptyList() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let url = root.appendingPathComponent("meetings.json")
        let original = Data("invalid meeting index".utf8)
        try original.write(to: url)
        let storage = MeetingRecordStorage(url: url)
        #expect(throws: (any Error).self) { try storage.load() }
        #expect(throws: (any Error).self) { try storage.save([]) }
        #expect(try Data(contentsOf: url) == original)
    }

    @Test func previousIndexIsBackedUp() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let storage = MeetingRecordStorage(url: root.appendingPathComponent("meetings.json"))
        #expect(try storage.load().isEmpty)
        try storage.save([])
        let first = try Data(contentsOf: storage.url)
        try storage.save([])
        #expect(try Data(contentsOf: storage.url.appendingPathExtension("backup")) == first)
    }
}
