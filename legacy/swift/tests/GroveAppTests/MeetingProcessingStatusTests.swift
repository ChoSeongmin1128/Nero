import Foundation
import GroveInference
import Testing
@testable import GroveApp

private actor ProcessingStatusFixture: MeetingInferenceRunning {
    enum Behavior { case success, failure, wait, blockIndexBackup }
    var behavior: Behavior = .success
    private(set) var calls = 0
    let base: URL
    init(base: URL) { self.base = base }
    func setBehavior(_ behavior: Behavior) { self.behavior = behavior }

    func run(source: URL, configuration: InferenceConfiguration, directory: URL,
             progress: @Sendable (String) async -> Void) async throws -> InferenceResult {
        calls += 1
        if behavior == .failure { throw InferenceError.workerFailed(42) }
        if behavior == .wait { try await Task.sleep(for: .seconds(60)) }
        if behavior == .blockIndexBackup {
            let backup = base.appendingPathComponent("meetings.json.backup")
            if FileManager.default.fileExists(atPath: backup.path) { try FileManager.default.removeItem(at: backup) }
            try FileManager.default.createDirectory(at: backup, withIntermediateDirectories: true)
        }
        return try InferenceResult(duration: 2, configuration: configuration,
            transcription: .init(utterances: [.init(start: 0, end: 1, text: "합성 발화 \(calls)")]),
            rawDiarization: [.init(start: 1.2, end: 1.8, clusterID: "0")])
    }
}

@MainActor
struct MeetingProcessingStatusTests {
    private func fixture() throws -> (URL, URL, URL, ProcessingStatusFixture, GroveStore) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let audio = root.appendingPathComponent("synthetic.wav")
        try Data("Synthetic protocol fixture; not model inference".utf8).write(to: audio)
        let base = root.appendingPathComponent("Grove")
        let service = ProcessingStatusFixture(base: base)
        return (root, base, audio, service, GroveStore(baseDirectory: base, inferenceService: service))
    }

    @Test func successfulProcessingIsReadyEvenWhenSpeakerCountAndAssignmentsNeedAttention() async throws {
        let (root, base, audio, _, store) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        let plan = MeetingInferencePlan(configuration: .init(expectedSpeakerCount: 4, diarizationPreference: .ultra8))
        await store.importRecording(from: audio, plan: plan)
        let meeting = try #require(store.meetings.first)
        let document = try #require(store.transcriptDocuments[meeting.id])
        #expect(meeting.status == .ready)
        #expect(meeting.errorMessage == nil)
        #expect(meeting.processingOutcome?.kind == .completed)
        #expect(meeting.completedResult?.revisionID == document.revisionID)
        #expect(meeting.completedResult?.speakerCounts == [.init(sourceID: "recording", expected: 4, detected: 1)])
        #expect(document.speakerReviewCount > 0)
        let status = MeetingPresentationStatus(meeting: meeting, document: document)
        #expect(status.label == "전사 완료" && !status.isFailure && !status.canRetry)
        let reopened = GroveStore(baseDirectory: base)
        #expect(reopened.meetings.first?.completedResult == meeting.completedResult)
        #expect(reopened.transcriptDocuments[meeting.id] == document)
    }

    @Test func failedRetryKeepsPreviousCountsDocumentEditsAndActualEngine() async throws {
        let (root, base, audio, service, store) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        await store.importRecording(from: audio, plan: .init(configuration: .init(expectedSpeakerCount: 4, diarizationPreference: .sortformerStreaming)))
        let old = try #require(store.meetings.first)
        let utterance = try #require(store.transcriptDocuments[old.id]?.utterances.first)
        #expect(store.transcripts.updateUtteranceText(meetingID: old.id, utteranceID: utterance.id, text: "사용자 교정 보존"))
        let corrected = try #require(store.transcriptDocuments[old.id])
        await service.setBehavior(.failure)
        await store.transcribeMeeting(id: old.id, plan: .init(configuration: .init(expectedSpeakerCount: 6, diarizationPreference: .ultra8)))
        let failed = try #require(store.meetings.first)
        #expect(failed.processingOutcome?.kind == .failed)
        #expect(failed.completedResult == old.completedResult)
        #expect(store.transcriptDocuments[old.id] == corrected)
        #expect(store.transcriptDocuments[old.id]?.sourceDiarizationEngines?["recording"] == .sortformerStreaming)
        #expect(failed.inferenceConfiguration?.expectedSpeakerCount == 6)
        let status = MeetingPresentationStatus(meeting: failed, document: corrected)
        #expect(status.label == "재전사 실패 · 이전 결과 유지")
        #expect(status.isFailure && status.canRetry)
        #expect(status.detail?.contains("42") == true)
        let reopened = GroveStore(baseDirectory: base)
        #expect(reopened.transcriptDocuments[old.id] == corrected)
        #expect(reopened.meetings.first?.completedResult == old.completedResult)
    }

    @Test func cancellationIsNotASpeakerWarningAndPreservesThePreviousResult() async throws {
        let (root, _, audio, service, store) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        await store.importRecording(from: audio)
        let old = try #require(store.meetings.first)
        let document = try #require(store.transcriptDocuments[old.id])
        await service.setBehavior(.wait)
        let task = Task { await store.transcribeMeeting(id: old.id) }
        for _ in 0..<100 {
            if await service.calls >= 2 { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        store.cancelProcessing()
        await task.value
        let record = try #require(store.meetings.first)
        #expect(record.processingOutcome?.kind == .cancelled)
        #expect(record.completedResult == old.completedResult)
        #expect(store.transcriptDocuments[old.id] == document)
        let status = MeetingPresentationStatus(meeting: record, document: document)
        #expect(status.label == "재전사 중단 · 이전 결과 유지")
        #expect(!status.isFailure && status.canRetry)
    }

    @Test func indexCommitFailureRollsBackActivatedDocumentWithoutChangingOldCounts() async throws {
        let (root, base, audio, service, store) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        await store.importRecording(from: audio)
        let old = try #require(store.meetings.first)
        let document = try #require(store.transcriptDocuments[old.id])
        let originalAudio = try Data(contentsOf: URL(fileURLWithPath: #require(old.audioPath)))
        await service.setBehavior(.blockIndexBackup)
        await store.transcribeMeeting(id: old.id)
        let storage = TranscriptDocumentStorage(directory: base.appendingPathComponent("Documents"))
        #expect(try storage.load(for: old.id) == document)
        #expect(store.transcriptDocuments[old.id] == document)
        #expect(store.meetings.first?.completedResult == old.completedResult)
        #expect(store.meetings.first?.processingOutcome?.kind == .failed)
        #expect(store.meetings.first?.processingOutcome?.message?.contains("상태도 저장하지 못했습니다") == true)
        #expect(try Data(contentsOf: URL(fileURLWithPath: #require(old.audioPath))) == originalAudio)
        #expect(try !storage.archives(for: old.id).isEmpty)
    }

    @Test func failureWithoutAnEarlierDocumentDoesNotClaimPreviousResultRetention() async throws {
        let (root, _, audio, service, store) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        await service.setBehavior(.failure)
        await store.importRecording(from: audio)
        let meeting = try #require(store.meetings.first)
        #expect(meeting.completedResult == nil)
        #expect(meeting.processingOutcome?.previousResultRetained == false)
        #expect(MeetingPresentationStatus(meeting: meeting, document: nil).label == "전사 실패")
        #expect(FileManager.default.fileExists(atPath: try #require(meeting.audioPath)))
    }

    @Test func firstResultIndexFailureRemovesOnlyItsUncommittedDocumentAndKeepsRawOutput() async throws {
        let (root, base, audio, service, store) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        let original = try Data(contentsOf: audio)
        await service.setBehavior(.blockIndexBackup)
        await store.importRecording(from: audio)
        let meeting = try #require(store.meetings.first)
        let storage = TranscriptDocumentStorage(directory: base.appendingPathComponent("Documents"))
        #expect(store.transcriptDocuments[meeting.id] == nil)
        #expect(try storage.load(for: meeting.id) == nil)
        #expect(meeting.completedResult == nil)
        #expect(meeting.processingOutcome?.kind == .failed)
        #expect(meeting.processingOutcome?.previousResultRetained == false)
        #expect(try Data(contentsOf: URL(fileURLWithPath: #require(meeting.audioPath))) == original)
        #expect(try Data(contentsOf: audio) == original)
        let raw = storage.directory.appendingPathComponent("EngineResults/\(meeting.id.uuidString)")
        let outputs = try FileManager.default.contentsOfDirectory(at: raw, includingPropertiesForKeys: nil)
        #expect(outputs.count == 1)
        try JSONDecoder().decode(InferenceResult.self, from: Data(contentsOf: #require(outputs.first))).validate()
    }

    @Test func legacyReviewFlagsAndMixedMessagesNeverInventAFailedAttempt() throws {
        let (root, base, audio, _, _) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        var record = MeetingRecord(title: "합성 이전 기록", startedAt: Date(), duration: 2, status: .needsReview,
            audioPath: audio.path, glossaryProfile: "", transcript: [.init(startTime: 0, endTime: 1, speaker: "화자 1", text: "원문")],
            claims: [], errorMessage: nil)
        let storage = MeetingRecordStorage(url: base.appendingPathComponent("meetings.json"))
        try storage.save([record])
        let bytes = try Data(contentsOf: storage.url)
        let reopened = GroveStore(baseDirectory: base)
        let document = try #require(reopened.transcriptDocuments[record.id])
        #expect(MeetingPresentationStatus(meeting: record, document: document).label == "전사 완료")
        #expect(reopened.meetings.first?.processingOutcome == nil)
        #expect(try Data(contentsOf: storage.url) == bytes)
        for message in ["입력한 인원은 4명이지만 감지된 화자는 3명입니다.", "이전 도구에서 남긴 실패 또는 안내일 수 있는 문구"] {
            record.errorMessage = message
            let status = MeetingPresentationStatus(meeting: record, document: document)
            #expect(status.label == "전사 결과 있음")
            #expect(!status.isFailure)
            #expect(status.detail == "이전 처리 안내: \(message)")
        }
    }

    @Test func interruptedProcessingHasExplicitRecoveryOutcomeAndRetainsMetadata() async throws {
        let (root, base, audio, _, store) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        await store.importRecording(from: audio)
        var record = try #require(store.meetings.first)
        let document = try #require(store.transcriptDocuments[record.id])
        let previousCounts = record.completedResult
        record.status = .processing
        record.processingOutcome = nil
        try MeetingRecordStorage(url: base.appendingPathComponent("meetings.json")).save([record])
        let reopened = GroveStore(baseDirectory: base)
        let recovered = try #require(reopened.meetings.first)
        #expect(recovered.processingOutcome?.kind == .interrupted)
        #expect(recovered.completedResult == previousCounts)
        #expect(reopened.transcriptDocuments[record.id] == document)
        #expect(MeetingPresentationStatus(meeting: recovered, document: document).label == "처리 중단 · 저장된 결과 유지")
    }

    @Test func invalidStructuredCountsCannotReplaceAnExistingIndex() throws {
        let (root, base, audio, _, _) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        var record = MeetingRecord(title: "합성", startedAt: Date(), duration: 2, status: .ready,
            audioPath: audio.path, glossaryProfile: "", transcript: [], claims: [], errorMessage: nil)
        let storage = MeetingRecordStorage(url: base.appendingPathComponent("meetings.json"))
        try storage.save([record])
        let before = try Data(contentsOf: storage.url)
        record.completedResult = .init(revisionID: UUID(), speakerCounts: [.init(sourceID: "recording", expected: 0, detected: -1)])
        #expect(throws: TranscriptEditError.self) { try storage.save([record]) }
        #expect(try Data(contentsOf: storage.url) == before)
    }
}
