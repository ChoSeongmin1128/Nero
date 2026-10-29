import Foundation
import Testing
@testable import GroveApp

struct TranscriptStorageTests {
    @Test func legacySourcesDoNotBecomePeople() throws {
        let segments = [
            TranscriptSegment(startTime: 0, endTime: 2, speaker: "내 마이크", text: "마이크", confidence: nil, revisedText: nil),
            TranscriptSegment(startTime: 0, endTime: 2, speaker: "원격 오디오", text: "원격", confidence: nil, revisedText: nil),
            TranscriptSegment(startTime: 3, endTime: 4, speaker: "화자 1", text: "원문", confidence: nil, revisedText: "수정문"),
        ]
        let document = try TranscriptDocument.preservingLegacy(segments)
        #expect(document.speakers.count == 1)
        #expect(document.utterances[0].speakerID == nil)
        #expect(document.utterances[0].sourceChannelID == "microphone")
        #expect(document.utterances[1].sourceChannelID == "system")
        #expect(document.utterances[2].id == segments[2].id)
        #expect(document.utterances[2].rawText == "원문")
        #expect(document.utterances[2].displayedText == "수정문")
    }

    @Test func savedEditsRoundTripAndPreviousVersionIsBackedUp() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("grove-storage-test-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        let storage = TranscriptDocumentStorage(directory: directory)
        let id = UUID()
        let segment = TranscriptSegment(startTime: 0, endTime: 1, speaker: "화자 1", text: "원문", confidence: nil, revisedText: nil)
        var document = try TranscriptDocument.preservingLegacy([segment])
        try storage.save(document, for: id)
        let before = try Data(contentsOf: storage.fileURL(for: id))
        try document.editText(segment.id, to: "수정문")
        try storage.save(document, for: id)
        #expect(try storage.load(for: id) == document)
        #expect(try Data(contentsOf: storage.fileURL(for: id).appendingPathExtension("backup")) == before)
    }

    @Test func corruptExistingDocumentIsNotOverwritten() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("grove-corrupt-test-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let storage = TranscriptDocumentStorage(directory: directory)
        let id = UUID()
        let damaged = Data("invalid-json".utf8)
        try damaged.write(to: storage.fileURL(for: id))
        let document = try TranscriptDocument(speakers: [], utterances: [])
        #expect(throws: (any Error).self) { try storage.save(document, for: id) }
        #expect(try Data(contentsOf: storage.fileURL(for: id)) == damaged)
    }
}
