import Foundation
import GroveInference
import Testing
@testable import GroveApp

private struct FolderASRFixture: MeetingInferenceRunning {
    func run(source: URL, configuration: InferenceConfiguration, directory: URL,
             progress: @Sendable (String) async -> Void) async throws -> InferenceResult {
        let utterances: [RecognizedUtterance] = (0..<6).map { (index: Int) in
            let start = Double(index) * 5.0
            return RecognizedUtterance(start: start, end: start + 4.0, text: "기준 발화 \(index)", asrClusterID: nil)
        }
        let turns: [DiarizationTurn] = (0..<6).map { (index: Int) in
            let start = Double(index) * 5.0
            return DiarizationTurn(start: start, end: start + 4.0, clusterID: String(index / 2))
        }
        return try InferenceResult(duration: 29, configuration: configuration, transcription: .init(utterances: utterances), rawDiarization: turns)
    }
}

private struct FolderVoiceFixture {
    static func voice(_ dimension: Int) -> SpeakerVoicePrint {
        var vector = [Float](repeating: 0, count: 256)
        vector[dimension] = 1
        return .init(modelIdentifier: "fixture", embedding: vector, speechDuration: 8, sampleCount: 2)
    }
}

@MainActor
struct FolderProfileIntegrationTests {
    @Test func savingANameDoesNotCollectVoiceprints() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = GroveStore(baseDirectory: root, inferenceService: FolderASRFixture())
        let folder = try #require(store.createFolder(name: "회의"))
        let input = root.appendingPathComponent("input.wav")
        try Data("fixture".utf8).write(to: input)
        await store.importRecording(from: input, folderID: folder)
        let meeting = try #require(store.meetings.first)
        let speaker = try #require(store.transcriptDocuments[meeting.id]?.speakers.first)
        #expect(await store.voice.saveSpeakerProfile(meetingID: meeting.id, speakerID: speaker.id, name: "저장한 이름"))
        let reopened = GroveStore(baseDirectory: root)
        let profile = try #require(reopened.speakerProfiles(in: folder).first)
        #expect(profile.name == "저장한 이름")
        #expect(profile.voice == nil)
        #expect(reopened.transcriptDocuments[meeting.id]?.speakers.first?.profileMatch?.profileID == profile.id)
        #expect(!FileManager.default.fileExists(atPath: root.appendingPathComponent("VoiceJobs").path))
    }

    @Test func importDoesNotGuessNamesAndManualReuseIsFolderScopedAndPersistent() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let folder = MeetingFolder(name: "회의 폴더")
        let other = MeetingFolder(name: "다른 폴더")
        func profile(_ index: Int, _ name: String, _ id: UUID) -> SavedSpeakerProfile {
            .init(folderID: id, name: name, voice: FolderVoiceFixture.voice(index), sourceMeetingID: UUID(),
                  sourceRevisionID: UUID(), sourceSpeakerID: UUID(), createdAt: Date())
        }
        var library = MeetingLibrary()
        library.folders = [folder, other]
        library.speakerProfiles = [profile(0, "A", folder.id), profile(1, "B", folder.id), profile(2, "C", folder.id), profile(3, "다른 폴더의 사람", other.id)]
        try MeetingLibraryStorage(url: root.appendingPathComponent("library.json")).save(library)
        let input = root.appendingPathComponent("input.wav")
        try Data("fake audio; no hardware".utf8).write(to: input)
        let store = GroveStore(baseDirectory: root, inferenceService: FolderASRFixture())
        await store.importRecording(from: input, folderID: folder.id)
        let meeting = try #require(store.meetings.first)
        let doc = try #require(store.transcriptDocuments[meeting.id])
        #expect(doc.speakers.count == 3)
        #expect(doc.speakers.allSatisfy { $0.profileMatch == nil })
        #expect(!doc.speakers.contains { $0.name == "다른 폴더의 사람" })
        let a = try #require(doc.speakers.first)
        let profileA = try #require(store.speakerProfiles(in: folder.id).first { $0.name == "A" })
        let foreignProfile = try #require(store.speakerProfiles(in: other.id).first)
        #expect(!store.voice.applySavedSpeaker(profileID: foreignProfile.id, meetingID: meeting.id, speakerID: a.id))
        #expect(store.voice.applySavedSpeaker(profileID: profileA.id, meetingID: meeting.id, speakerID: a.id))
        #expect(await store.voice.saveSpeakerProfile(meetingID: meeting.id, speakerID: a.id, name: "A") == false)
        #expect(store.speakerProfiles(in: folder.id).count == 3)
        let reopened = GroveStore(baseDirectory: root)
        let restored = try #require(reopened.transcriptDocuments[meeting.id])
        #expect(restored.speakers.first { $0.name == "A" }?.profileMatch?.isConfirmed == true)
        #expect(!TranscriptRenderer.render(restored).contains("A (추정)"))
        #expect(try Data(contentsOf: input) == Data("fake audio; no hardware".utf8))
    }

    @Test func profileStorageFailureLeavesCurrentTranscriptAndInputUntouched() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = GroveStore(baseDirectory: root, inferenceService: FolderASRFixture())
        let folder = try #require(store.createFolder(name: "회의"))
        let input = root.appendingPathComponent("input.wav")
        try Data("original".utf8).write(to: input)
        await store.importRecording(from: input, folderID: folder)
        let id = try #require(store.meetings.first?.id)
        let doc = try #require(store.transcriptDocuments[id])
        let speaker = try #require(doc.speakers.first)
        let bad = Data("malformed metadata".utf8)
        try bad.write(to: root.appendingPathComponent("library.json"))
        #expect(await store.voice.saveSpeakerProfile(meetingID: id, speakerID: speaker.id, name: "A") == false)
        #expect(store.transcriptDocuments[id] == doc)
        #expect(try Data(contentsOf: root.appendingPathComponent("library.json")) == bad)
        #expect(try Data(contentsOf: input) == Data("original".utf8))
    }
}
