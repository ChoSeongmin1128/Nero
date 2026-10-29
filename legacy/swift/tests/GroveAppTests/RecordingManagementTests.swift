import Foundation
import GroveInference
import Testing
@testable import GroveApp

private actor PausedRenameInference: MeetingInferenceRunning {
    private var continuation: CheckedContinuation<Void, Never>?
    private(set) var isWaiting = false

    func finish() { continuation?.resume(); continuation = nil }

    func run(source: URL, configuration: InferenceConfiguration, directory: URL,
             progress: @Sendable (String) async -> Void) async throws -> InferenceResult {
        await withCheckedContinuation { continuation in
            self.continuation = continuation
            isWaiting = true
        }
        return try InferenceResult(duration: 2, configuration: configuration,
            transcription: .init(utterances: [.init(start: 0, end: 1, text: "원본 발화")]),
            rawDiarization: [.init(start: 0, end: 1, clusterID: "0")])
    }
}

@MainActor
struct RecordingManagementTests {
    @Test(.enabled(if: ProcessInfo.processInfo.environment["GROVE_RUN_ORIGINAL_EXPORT_INTEGRATION"] == "1"))
    func actualRecordingExportsByteForByteWithoutChangingSource() async throws {
        let environment = ProcessInfo.processInfo.environment
        let source = URL(fileURLWithPath: try #require(environment["GROVE_ORIGINAL_AUDIO_PATH"]))
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let managed = root.appendingPathComponent("Grove")
        try FileManager.default.createDirectory(at: managed, withIntermediateDirectories: true)
        let destination = root.appendingPathComponent("원본 사본.\(source.pathExtension)")
        let original = try Data(contentsOf: source)
        try await OriginalRecordingExporter.export(source: source, to: destination, protectedDirectory: managed)
        #expect(try Data(contentsOf: destination) == original)
        #expect(try Data(contentsOf: source) == original)
    }

    private func fixture() throws -> (root: URL, base: URL, meeting: MeetingRecord, store: GroveStore) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let base = root.appendingPathComponent("Grove")
        let audio = base.appendingPathComponent("Audio/original.wav")
        try FileManager.default.createDirectory(at: audio.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data("Original recording bytes".utf8).write(to: audio)
        let meeting = MeetingRecord(title: "가져온 녹음", startedAt: Date(timeIntervalSince1970: 1_700_000_000),
            duration: 2, status: .ready, audioPath: audio.path, glossaryProfile: "사전 없음",
            transcript: [], claims: [], errorMessage: nil)
        try MeetingRecordStorage(url: base.appendingPathComponent("meetings.json")).save([meeting])
        return (root, base, meeting, GroveStore(baseDirectory: base))
    }

    @Test func renamePersistsOnlyTheDisplayTitleAndPreservesOriginalAndTranscript() throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let result = try InferenceResult(duration: 2, configuration: .init(),
            transcription: .init(utterances: [.init(start: 0, end: 1, text: "수정 전")]),
            rawDiarization: [.init(start: 0, end: 1, clusterID: "0")])
        try fixture.store.acceptInferenceResult(result, meetingID: fixture.meeting.id, sourceChannelID: "recording")
        let originalDocument = try #require(fixture.store.transcriptDocuments[fixture.meeting.id])
        let utteranceID = try #require(originalDocument.utterances.first?.id)
        #expect(fixture.store.transcripts.updateUtteranceText(meetingID: fixture.meeting.id, utteranceID: utteranceID, text: "사용자 교정"))
        let corrected = try #require(fixture.store.transcriptDocuments[fixture.meeting.id])
        let original = try Data(contentsOf: URL(fileURLWithPath: #require(fixture.meeting.audioPath)))
        #expect(fixture.store.renameMeeting(id: fixture.meeting.id, title: "  새 녹음 이름  "))
        var expected = fixture.meeting
        expected.title = "새 녹음 이름"
        #expect(fixture.store.meetings.first == expected)
        let reopened = GroveStore(baseDirectory: fixture.base)
        #expect(reopened.meetings.first == expected)
        #expect(reopened.transcriptDocuments[fixture.meeting.id] == corrected)
        #expect(try Data(contentsOf: URL(fileURLWithPath: #require(expected.audioPath))) == original)
    }

    @Test func emptyMultilineAndMissingMeetingNamesDoNotWrite() throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let index = fixture.base.appendingPathComponent("meetings.json")
        let before = try Data(contentsOf: index)
        for input in ["", " \n ", "회의\n이름", "회의\t이름"] {
            #expect(!fixture.store.renameMeeting(id: fixture.meeting.id, title: input))
        }
        #expect(!fixture.store.renameMeeting(id: UUID(), title: "없는 녹음"))
        #expect(try Data(contentsOf: index) == before)
        #expect(fixture.store.meetings.first == fixture.meeting)
    }

    @Test func renameFailureKeepsTheOldNameAndCorruptIndex() throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let index = fixture.base.appendingPathComponent("meetings.json")
        let corrupt = Data("invalid current index".utf8)
        try corrupt.write(to: index)
        #expect(!fixture.store.renameMeeting(id: fixture.meeting.id, title: "새 이름"))
        #expect(fixture.store.meetings.first == fixture.meeting)
        #expect(try Data(contentsOf: index) == corrupt)
        #expect(fixture.store.alertMessage?.contains("기존 이름") == true)
    }

    @Test func renameWriteFailureDoesNotUpdateTheUIState() throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let index = fixture.base.appendingPathComponent("meetings.json")
        let before = try Data(contentsOf: index)
        try FileManager.default.createDirectory(at: index.appendingPathExtension("backup"), withIntermediateDirectories: false)
        #expect(!fixture.store.renameMeeting(id: fixture.meeting.id, title: "저장 실패"))
        #expect(fixture.store.meetings.first == fixture.meeting)
        #expect(try Data(contentsOf: index) == before)
    }

    @Test func recordingCanBeRenamedButCannotBeExportedUntilStopped() async throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        fixture.store.meetings[0].status = .recording
        fixture.store.activeMeetingID = fixture.meeting.id
        #expect(fixture.store.renameMeeting(id: fixture.meeting.id, title: "녹음 중 이름 변경"))
        #expect(fixture.store.meetings.first?.status == .recording)
        #expect(!fixture.store.canExportOriginal(meetingID: fixture.meeting.id))
        let target = fixture.root.appendingPathComponent("export.wav")
        await #expect(throws: RecordingManagementError.self) {
            try await fixture.store.exportOriginal(meetingID: fixture.meeting.id, sourceID: "recording",
                to: target, replacingExisting: false)
        }
        #expect(!FileManager.default.fileExists(atPath: target.path))
    }

    @Test func exportWorksWithoutATranscriptAndDoesNotChangeMeetingMetadata() async throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        fixture.store.meetings[0].status = .failed
        let before = fixture.store.meetings
        let index = fixture.base.appendingPathComponent("meetings.json")
        let bytes = try Data(contentsOf: index)
        let target = fixture.root.appendingPathComponent("export.wav")
        #expect(fixture.store.canExportOriginal(meetingID: fixture.meeting.id))
        try await fixture.store.exportOriginal(meetingID: fixture.meeting.id, sourceID: "recording",
            to: target, replacingExisting: false)
        #expect(try Data(contentsOf: target) == Data(contentsOf: URL(fileURLWithPath: #require(fixture.meeting.audioPath))))
        #expect(fixture.store.meetings == before)
        #expect(try Data(contentsOf: index) == bytes)
        #expect(!fixture.store.isExportingOriginal)
    }

    @Test func transcriptionCompletionDoesNotRestoreTheOldRecordingTitle() async throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let service = PausedRenameInference()
        let store = GroveStore(baseDirectory: fixture.base, inferenceService: service)
        let task = Task { await store.transcribeMeeting(id: fixture.meeting.id) }
        for _ in 0..<200 {
            if await service.isWaiting { break }
            try await Task.sleep(for: .milliseconds(5))
        }
        #expect(await service.isWaiting)
        #expect(store.processingMeetingID == fixture.meeting.id)
        #expect(store.renameMeeting(id: fixture.meeting.id, title: "처리 중 변경한 이름"))
        await service.finish()
        await task.value
        #expect(store.meetings.first?.title == "처리 중 변경한 이름")
        #expect(GroveStore(baseDirectory: fixture.base).meetings.first?.title == "처리 중 변경한 이름")
    }

    @Test func quittingWhileAwaitingInferenceCannotStartAnOriginalExport() async throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let service = PausedRenameInference()
        let store = GroveStore(baseDirectory: fixture.base, inferenceService: service)
        let processing = Task { await store.transcribeMeeting(id: fixture.meeting.id) }
        for _ in 0..<200 {
            if await service.isWaiting { break }
            try await Task.sleep(for: .milliseconds(5))
        }
        let quitting = Task { await store.prepareToQuit() }
        for _ in 0..<200 {
            if store.isPreparingToQuit { break }
            try await Task.sleep(for: .milliseconds(5))
        }
        #expect(store.isPreparingToQuit)
        #expect(store.isBusy)
        #expect(!store.canExportOriginal(meetingID: fixture.meeting.id))
        let target = fixture.root.appendingPathComponent("must-not-export.wav")
        await #expect(throws: RecordingManagementError.self) {
            try await store.exportOriginal(meetingID: fixture.meeting.id, sourceID: "recording",
                to: target, replacingExisting: false)
        }
        await service.finish()
        await processing.value
        #expect(await quitting.value)
        #expect(!FileManager.default.fileExists(atPath: target.path))
    }
}
