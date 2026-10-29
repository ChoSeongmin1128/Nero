import Foundation
import GroveInference
import Testing
@testable import GroveApp

struct TranscriptRevisionTests {
    private func document(_ text: String) throws -> TranscriptDocument {
        try TranscriptDocument(speakers: [], utterances: [
            .init(id: UUID(), startTime: 0, endTime: 1, rawText: text, sourceChannelID: "import", engineClusterID: nil, speakerID: nil, editedText: nil)
        ])
    }

    @Test func reprocessingAndRestoreBothPreserveManualEdits() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let storage = TranscriptDocumentStorage(directory: root)
        let meeting = UUID()
        var first = try document("첫 원문")
        try first.editText(first.utterances[0].id, to: "사용자 수정")
        try storage.save(first, for: meeting)
        let second = try document("새 모델의 원문")
        try storage.activate(second, for: meeting)
        #expect(try storage.load(for: meeting) == second)
        let archived = try #require(storage.archives(for: meeting).first)
        #expect(archived.document == first)
        try storage.activate(archived.document, for: meeting)
        #expect(try storage.load(for: meeting) == first)
        #expect(try storage.archives(for: meeting).contains { $0.document == second })
    }

    @Test func ordinarySaveCannotReplaceADifferentInferenceRevision() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let storage = TranscriptDocumentStorage(directory: root)
        let meeting = UUID()
        let first = try document("기존")
        try storage.save(first, for: meeting)
        #expect(throws: TranscriptEditError.self) { try storage.save(document("다른 추론"), for: meeting) }
        #expect(try storage.load(for: meeting) == first)
    }

    @Test func corruptCurrentFileCannotBeOverwrittenByActivation() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let storage = TranscriptDocumentStorage(directory: root)
        let meeting = UUID()
        let bytes = Data("not a valid document".utf8)
        try bytes.write(to: storage.fileURL(for: meeting))
        #expect(throws: (any Error).self) { try storage.activate(document("새 전사"), for: meeting) }
        #expect(try Data(contentsOf: storage.fileURL(for: meeting)) == bytes)
    }

    @Test func rawInferenceResultSurvivesManualEdits() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let storage = TranscriptDocumentStorage(directory: root)
        let meeting = UUID()
        let result = try InferenceResult(duration: 2, configuration: .init(expectedSpeakerCount: 1),
            transcription: .init(utterances: [.init(start: 0, end: 1, text: "원문")]),
            rawDiarization: [.init(start: 0, end: 1, clusterID: "A")])
        var current = try TranscriptDocument.preservingInference(result, sourceChannelID: "import")
        try storage.activate(current, for: meeting, rawResult: result)
        let rawURL = root.appendingPathComponent("EngineResults/\(meeting.uuidString)/\(result.jobID.uuidString).json")
        let original = try Data(contentsOf: rawURL)
        try current.editText(current.utterances[0].id, to: "편집한 내용")
        try storage.save(current, for: meeting)
        #expect(try Data(contentsOf: rawURL) == original)
        let restoredResult = try JSONDecoder().decode(InferenceResult.self, from: original)
        try restoredResult.validate()
    }
}
