import Foundation
import GroveInference
import Testing
@testable import GroveApp

/// These injectable protocol fixtures test app wiring/persistence, not model accuracy.
private actor MeetingInferenceFixture: MeetingInferenceRunning {
    enum Behavior { case succeed, fail, waitForCancellation }
    var behavior: Behavior = .succeed
    private(set) var configurations: [InferenceConfiguration] = []

    func setBehavior(_ value: Behavior) { behavior = value }

    func run(source: URL, configuration: InferenceConfiguration, directory: URL,
             progress: @Sendable (String) async -> Void) async throws -> InferenceResult {
        configurations.append(configuration)
        await progress("테스트 처리 단계")
        if behavior == .fail { throw InferenceError.workerFailed(9) }
        if behavior == .waitForCancellation { try await Task.sleep(for: .seconds(30)) }
        return try InferenceResult(duration: 2, configuration: configuration,
            transcription: .init(utterances: [.init(start: 0, end: 1, text: "원본 발화", asrClusterID: "S01")]),
            rawDiarization: [.init(start: 0, end: 1, clusterID: "Speaker 0")])
    }
}

@MainActor
struct MeetingInferenceIntegrationTests {
    @Test(.enabled(if: ProcessInfo.processInfo.environment["GROVE_RUN_NATIVE_INTEGRATION"] == "1"))
    func packagedNativeAppImportWithoutLaunchingUI() async throws {
        let environment = ProcessInfo.processInfo.environment
        let app = URL(fileURLWithPath: try #require(environment["GROVE_BETA_APP_PATH"]))
        let input = URL(fileURLWithPath: try #require(environment["GROVE_BETA_AUDIO_PATH"]))
        let models = URL(fileURLWithPath: try #require(environment["GROVE_BETA_MODEL_SUPPORT_PATH"]))
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let service = BundledMeetingInferenceService(appBundle: app, applicationSupport: models)
        let store = GroveStore(baseDirectory: root, inferenceService: service)
        if let count = environment["GROVE_BETA_SPEAKER_COUNT"] {
            store.defaultSpeakerOptions.mode = .manualCount
            store.defaultSpeakerOptions.countText = count
        }
        if environment["GROVE_BETA_TRANSCRIPTION_ENGINE"] == "apple" {
            store.defaultSpeakerOptions.transcriptionEngine = .apple
            if environment["GROVE_BETA_PREPARE_APPLE"] == "1" {
                store.applePreparation.prepare()
                while store.applePreparation.isPreparing { try await Task.sleep(for: .milliseconds(200)) }
                #expect(store.applePreparation.isReady, "\(store.applePreparation.message ?? "기본 전사 준비 실패")")
            }
        }
        await store.importRecording(from: input)
        let meeting = try #require(store.meetings.first)
        #expect(meeting.status == .ready || meeting.status == .needsReview, "\(meeting.errorMessage ?? "처리 실패 사유 없음")")
        let document = try #require(store.transcriptDocuments[meeting.id])
        #expect(!document.utterances.isEmpty)
        #expect(store.defaultSpeakerOptions.transcriptionEngine == .apple ? document.speakers.isEmpty : (1...4).contains(document.speakers.count))
        #expect(document.utterances.allSatisfy { !$0.rawText.hasPrefix("[S0") })
        let configuration = try #require(meeting.inferenceConfiguration)
        #expect(document.sourceDiarizationEngines?["recording"] == (try configuration.resolvedEngine()))
        if environment["GROVE_BETA_TRANSCRIPTION_ENGINE"] != "apple" {
            #expect(configuration.diarizationPreference == .nemotron3)
            #expect(try configuration.resolvedEngine() == .nemotron3)
            #expect(configuration.expectedSpeakerCount == environment["GROVE_BETA_SPEAKER_COUNT"].flatMap(Int.init))
        }
        let utterance = try #require(document.utterances.first)
        #expect(store.transcripts.updateUtteranceText(meetingID: meeting.id, utteranceID: utterance.id, text: "베타 검수 수정"))
        let reopened = GroveStore(baseDirectory: root, inferenceService: service)
        #expect(reopened.transcriptDocuments[meeting.id]?.utterances.first?.displayedText == "베타 검수 수정")
    }

    private func fixture() throws -> (URL, URL, URL) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let audio = root.appendingPathComponent("fixture.wav")
        try Data("Original private audio fixture".utf8).write(to: audio)
        return (root, root.appendingPathComponent("Grove"), audio)
    }

    @Test func importRunsChosenNativeRouteAndPersistsDocumentWithoutChangingOriginal() async throws {
        let (root, base, audio) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        let service = MeetingInferenceFixture()
        let store = GroveStore(baseDirectory: base, inferenceService: service)
        store.defaultSpeakerOptions.mode = .manualCount
        store.defaultSpeakerOptions.countText = "4"
        let original = try Data(contentsOf: audio)
        await store.importRecording(from: audio)
        let meeting = try #require(store.meetings.first)
        let document = try #require(store.transcriptDocuments[meeting.id])
        #expect(document.utterances.first?.displayedText == "원본 발화")
        #expect(document.utterances.first?.sourceChannelID == "recording")
        #expect(meeting.status == .ready)
        #expect(meeting.errorMessage == nil)
        #expect(meeting.completedResult?.speakerCounts.first?.expected == 4)
        #expect(meeting.completedResult?.speakerCounts.first?.detected == 1)
        #expect(!store.isProcessing)
        #expect(try await service.configurations.first?.resolvedEngine() == .nemotron3)
        #expect(await service.configurations.first?.expectedSpeakerCount == 4)
        #expect(try Data(contentsOf: audio) == original)
        #expect(try Data(contentsOf: URL(fileURLWithPath: #require(meeting.audioPath))) == original)
        let reloaded = GroveStore(baseDirectory: base, inferenceService: service)
        #expect(reloaded.transcriptDocuments[meeting.id] == document)
        #expect(try reloaded.meetings.first?.inferenceConfiguration?.resolvedEngine() == .nemotron3)
        #expect(reloaded.meetings.first?.inferenceConfiguration?.expectedSpeakerCount == 4)
        let raw = base.appendingPathComponent("Documents/EngineResults/\(meeting.id.uuidString)/\(document.revisionID.uuidString).json")
        #expect(FileManager.default.fileExists(atPath: raw.path))
    }

    @Test func successfulReprocessingArchivesEditsAndFailureKeepsCurrentDocument() async throws {
        let (root, base, audio) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        let service = MeetingInferenceFixture()
        let store = GroveStore(baseDirectory: base, inferenceService: service)
        await store.importRecording(from: audio)
        let id = try #require(store.meetings.first?.id)
        let utterance = try #require(store.transcriptDocuments[id]?.utterances.first)
        #expect(store.transcripts.updateUtteranceText(meetingID: id, utteranceID: utterance.id, text: "사용자 수정"))
        let corrected = try #require(store.transcriptDocuments[id])
        await store.transcribeMeeting(id: id, plan: .init(configuration:
            .init(expectedSpeakerCount: 4, diarizationPreference: .sortformerStreaming)))
        #expect(try store.transcripts.previousTranscripts(meetingID: id).contains { $0.document == corrected })
        let next = try #require(store.transcriptDocuments[id])
        #expect(next.revisionID != corrected.revisionID)
        await service.setBehavior(.fail)
        await store.transcribeMeeting(id: id, plan: .init(configuration: .init(diarizationPreference: .ultra8)))
        #expect(store.transcriptDocuments[id] == next)
        #expect(store.meetings.first?.inferenceConfiguration?.diarizationPreference == .ultra8)
        #expect(store.transcriptDocuments[id]?.sourceDiarizationEngines?["recording"] == .sortformerStreaming)
        #expect(store.meetings.first?.status == .failed)
        #expect(store.meetings.first?.processingOutcome?.previousResultRetained == true)
        #expect(store.meetings.first?.errorMessage != nil)
    }

    @Test func cancellationBlocksDuplicateJobsAndRemovalThenPreservesAudio() async throws {
        let (root, base, audio) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        let service = MeetingInferenceFixture()
        await service.setBehavior(.waitForCancellation)
        let store = GroveStore(baseDirectory: base, inferenceService: service)
        let task = Task { await store.importRecording(from: audio) }
        for _ in 0..<100 {
            if await !service.configurations.isEmpty { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        let meeting = try #require(store.meetings.first)
        #expect(store.isProcessing)
        store.deleteMeeting(meeting)
        await store.importRecording(from: audio)
        #expect(store.meetings.count == 1)
        store.cancelProcessing()
        await task.value
        #expect(!store.isProcessing)
        #expect(store.transcriptDocuments[meeting.id] == nil)
        #expect(store.meetings.first?.status == .failed)
        #expect(FileManager.default.fileExists(atPath: try #require(meeting.audioPath)))
        #expect(await service.configurations.count == 1)
    }

    @Test func dualSourcesNeverMergeSameNamedMachineClusters() async throws {
        let (root, base, audio) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        let service = MeetingInferenceFixture()
        let meeting = MeetingRecord(title: "두 채널", startedAt: Date(), duration: 2, status: .ready,
            audioPath: nil, systemAudioPath: audio.path, microphoneAudioPath: audio.path,
            captureMode: .systemAndMicrophone, glossaryProfile: "사전 없음", transcript: [], claims: [], errorMessage: nil)
        try MeetingRecordStorage(url: base.appendingPathComponent("meetings.json")).save([meeting])
        let store = GroveStore(baseDirectory: base, inferenceService: service)
        await store.transcribeMeeting(id: meeting.id)
        let document = try #require(store.transcriptDocuments[meeting.id])
        #expect(document.speakers.count == 2)
        #expect(Set(document.utterances.compactMap(\.speakerID)).count == 2)
        #expect(Set(document.utterances.compactMap(\.sourceChannelID)) == ["system", "microphone"])
        #expect(store.meetings.first?.status == .ready)
        #expect(await service.configurations.count == 2)
    }

    @Test func interruptedJobIsRecoverableAndMalformedIndexBlocksWrites() async throws {
        let (root, base, audio) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        let service = MeetingInferenceFixture()
        let meeting = MeetingRecord(title: "중단된 회의", startedAt: Date(), duration: 2, status: .processing,
            audioPath: audio.path, glossaryProfile: "사전 없음", transcript: [], claims: [], errorMessage: nil)
        let index = base.appendingPathComponent("meetings.json")
        try MeetingRecordStorage(url: index).save([meeting])
        let restored = GroveStore(baseDirectory: base, inferenceService: service)
        #expect(restored.meetings.first?.status == .failed)
        #expect(restored.meetings.first?.processingOutcome?.kind == .interrupted)
        #expect(!restored.isProcessing)
        let broken = Data("malformed existing index".utf8)
        try broken.write(to: index)
        let blocked = GroveStore(baseDirectory: base, inferenceService: service)
        await blocked.importRecording(from: audio)
        #expect(try Data(contentsOf: index) == broken)
        #expect(await service.configurations.isEmpty)
    }

    @Test func nemotronConfigurationStopsAtEightSpeakers() throws {
        #expect(try MeetingSpeakerOptions.configuration(count: 8).resolvedEngine() == .nemotron3)
        #expect(throws: InferenceError.self) { try MeetingSpeakerOptions.configuration(count: 9) }
        #expect(MeetingProcessingMode(configuration: .init(expectedSpeakerCount: 8)) == .manualCount)
        #expect(MeetingProcessingMode(configuration: .init(expectedSpeakerCount: 4)) == .manualCount)
    }

    @Test(arguments: [5, 8])
    func enteredCountIsPersistedAndInvalidInputDoesNotImport(_ count: Int) async throws {
        let (root, base, audio) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        let service = MeetingInferenceFixture()
        let store = GroveStore(baseDirectory: base, inferenceService: service)
        store.defaultSpeakerOptions.mode = .manualCount
        store.defaultSpeakerOptions.countText = "0"
        await store.importRecording(from: audio)
        #expect(store.meetings.isEmpty)
        #expect(await service.configurations.isEmpty)
        store.defaultSpeakerOptions.countText = String(count)
        await store.importRecording(from: audio)
        let config = try #require(await service.configurations.first)
        #expect(config.expectedSpeakerCount == count)
        #expect(config.speakerCountPolicy == .advisory)
        #expect(try config.resolvedEngine() == .nemotron3)
        let reopened = GroveStore(baseDirectory: base, inferenceService: service)
        #expect(reopened.meetings.first?.inferenceConfiguration == config)
    }
}
