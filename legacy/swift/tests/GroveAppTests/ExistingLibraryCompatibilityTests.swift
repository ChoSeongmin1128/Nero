import Foundation
import GroveInference
import Testing
@testable import GroveApp

struct ExistingLibraryCompatibilityTests {
    @Test(.enabled(if: ProcessInfo.processInfo.environment["GROVE_COMPATIBILITY_LIBRARY"] != nil))
    func existingLibraryAndRawResultsRemainReadableWithoutRewriting() throws {
        let base = URL(fileURLWithPath: try #require(ProcessInfo.processInfo.environment["GROVE_COMPATIBILITY_LIBRARY"]))
        let records = try MeetingRecordStorage(url: base.appendingPathComponent("meetings.json")).load()
        let documents = TranscriptDocumentStorage(directory: base.appendingPathComponent("Documents"))
        let decoder = JSONDecoder()
        var checked = 0
        for record in records {
            if let document = try documents.load(for: record.id) {
                let encoded = try JSONEncoder().encode(document)
                let decoded = try decoder.decode(TranscriptDocument.self, from: encoded)
                try decoded.validate()
                #expect(decoded == document)
                checked += 1
            }
            for archive in try documents.archives(for: record.id) { try archive.document.validate() }
            let rawDirectory = documents.directory.appendingPathComponent("EngineResults/\(record.id.uuidString)")
            if FileManager.default.fileExists(atPath: rawDirectory.path) {
                for url in try FileManager.default.contentsOfDirectory(at: rawDirectory, includingPropertiesForKeys: nil)
                    where url.pathExtension == "json" {
                    try decoder.decode(InferenceResult.self, from: Data(contentsOf: url)).validate()
                }
            }
        }
        #expect(checked > 0)
    }
}
