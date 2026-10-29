import Foundation
import GroveInference
import Testing
@testable import GroveApp

private func unitVoice(_ dimension: Int, model: String = "test-model") -> SpeakerVoicePrint {
    var embedding = [Float](repeating: 0, count: 256)
    embedding[dimension] = 1
    return SpeakerVoicePrint(modelIdentifier: model, embedding: embedding, speechDuration: 8, sampleCount: 2)
}

private func savedProfile(_ dimension: Int, folder: UUID, name: String) -> SavedSpeakerProfile {
    SavedSpeakerProfile(folderID: folder, name: name, voice: unitVoice(dimension), sourceMeetingID: UUID(),
        sourceRevisionID: UUID(), sourceSpeakerID: UUID(), createdAt: Date())
}

struct SavedSpeakerProfileTests {
    @Test func inferredNameIsReversibleAndManualRenameClearsIdentity() throws {
        let speaker = MeetingSpeaker(name: "화자 1", order: 0)
        let utterance = DocumentUtterance(id: UUID(), startTime: 0, endTime: 4, rawText: "원문", sourceChannelID: "recording", engineClusterID: "0", speakerID: speaker.id)
        var doc = try TranscriptDocument(speakers: [speaker], utterances: [utterance])
        let profile = savedProfile(0, folder: UUID(), name: "A")
        try doc.applySpeakerProfile(profile, to: speaker.id, similarity: 0.9, confirmed: false)
        #expect(doc.schemaVersion == 5)
        #expect(doc.speakers[0].name == "A")
        #expect(doc.speakers[0].profileMatch?.isConfirmed == false)
        #expect(TranscriptRenderer.render(doc).contains("A (추정)"))
        #expect(TranscriptRenderer.render(doc, options: .init(format: .markdown, selectedUtteranceIDs: [utterance.id])).contains("A \\(추정\\)"))
        try doc.undo()
        #expect(doc.speakers[0].name == "화자 1")
        #expect(doc.speakers[0].profileMatch == nil)
        try doc.redo()
        try doc.renameSpeaker(speaker.id, to: "다른 사람")
        #expect(doc.speakers[0].profileMatch == nil)
        #expect(doc.utterances[0].rawText == "원문")
        let restored = try JSONDecoder().decode(TranscriptDocument.self, from: JSONEncoder().encode(doc))
        #expect(restored == doc)
    }

    @Test func removedVoiceprintsDoNotSurviveInMetadataBackup() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let storage = MeetingLibraryStorage(url: root.appendingPathComponent("library.json"))
        let folder = MeetingFolder(name: "폴더")
        let profile = savedProfile(0, folder: folder.id, name: "A")
        var library = MeetingLibrary()
        library.folders = [folder]
        library.speakerProfiles = [profile]
        try storage.save(library)
        library.speakerProfiles = []
        try storage.save(library)
        let backup = try JSONDecoder().decode(MeetingLibrary.self, from: Data(contentsOf: storage.url.appendingPathExtension("backup")))
        #expect(backup.speakerProfiles?.isEmpty == true)
        #expect(try storage.load().speakerProfiles?.isEmpty == true)
    }
}
