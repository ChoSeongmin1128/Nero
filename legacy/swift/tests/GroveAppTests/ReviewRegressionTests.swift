@preconcurrency import AVFoundation
import Foundation
import GroveInference
import Testing
@testable import GroveApp

struct PreparedFixtureInference: MeetingInferenceRunning {
    func run(source: URL, configuration: InferenceConfiguration, directory: URL,
             progress: @Sendable (String) async -> Void) async throws -> InferenceResult { throw InferenceError.noSpeech }
}

@MainActor
private final class ReviewRecordingDevice: AudioRecordingDevice {
    var isRecording = false
    var currentTime: TimeInterval = 0
    var isMeteringEnabled = false
    var starts = 0
    var termination: (@Sendable () -> Void)?
    func prepareToRecord() -> Bool { true }
    func record() -> Bool { starts += 1; isRecording = true; return true }
    func pause() { isRecording = false }
    func stop() { isRecording = false; currentTime = 0 }
    func updateMeters() {}
    func averagePower(forChannel channelNumber: Int) -> Float { -20 }
    func observeTermination(_ handler: @escaping @Sendable () -> Void) { termination = handler }
}

@MainActor
private final class SelectedPreparationFixture: MeetingInferenceRunning {
    var blockedEngine: TranscriptionEngine? = .apple
    var suspendPreparation = false
    var preparationRequests = 0
    var runs = 0
    var continuation: CheckedContinuation<Void, Error>?
    func preparationIssue(configuration: InferenceConfiguration, appleReady: Bool) -> String? {
        if configuration.transcriptionEngine == blockedEngine { return "선택한 모델 준비 필요" }
        return nil
    }
    func validatePreparation(configuration: InferenceConfiguration) async throws {
        preparationRequests += 1
        if suspendPreparation { try await withCheckedThrowingContinuation { continuation = $0 } }
        if let issue = preparationIssue(configuration: configuration, appleReady: false) { throw InferenceError.invalidOutput(issue) }
    }
    func run(source: URL, configuration: InferenceConfiguration, directory: URL,
             progress: @Sendable (String) async -> Void) async throws -> InferenceResult {
        runs += 1
        return try InferenceResult(duration: 1, configuration: configuration,
            transcription: .init(utterances: [.init(start: 0, end: 1, text: "합성 발화")]),
            rawDiarization: configuration.transcriptionEngine == .apple ? [] : [.init(start: 0, end: 1, clusterID: "speaker0")])
    }
}

@MainActor
private final class CalendarPermissionFixture {
    var requests: [CheckedContinuation<Bool, Error>] = []
    func request() async throws -> Bool { try await withCheckedThrowingContinuation { requests.append($0) } }
}

@MainActor
struct ReviewRegressionTests {
    @Test func deviceFailureEndsTheMeetingPreservesAudioAndSurvivesReopening() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let device = ReviewRecordingDevice()
        let bytes = Data("synthetic capture".utf8)
        let recorder = AudioRecorder(makeRecorder: { url, _ in try bytes.write(to: url); return device }, permissionRequester: { true })
        let store = GroveStore(baseDirectory: root, inferenceService: PreparedFixtureInference(), recorder: recorder)
        await store.beginRecording(title: "합성 회의")
        let meeting = try #require(store.activeMeeting)
        device.currentTime = 8
        recorder.updateMeter()
        device.stop()
        recorder.updateMeter()
        #expect(!store.isRecording && !store.isBusy && store.activeMeetingID == nil)
        #expect(store.meetings.first?.status == .failed && store.meetings.first?.duration == 8)
        #expect(store.meetings.first?.processingOutcome?.kind == .interrupted)
        await store.stopRecording()
        let restored = GroveStore(baseDirectory: root, inferenceService: PreparedFixtureInference())
        #expect(restored.meetings.first?.status == .failed)
        #expect(try Data(contentsOf: URL(fileURLWithPath: try #require(meeting.audioPath))) == bytes)
    }

    @Test func enrollmentFailureEndsTheSessionAndDiscardsItsTemporaryCapture() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let device = ReviewRecordingDevice()
        let session = VoiceEnrollmentSession(directory: root,
            recorder: AudioRecorder(makeRecorder: { url, _ in try Data([1]).write(to: url); return device }), requestPermission: { true })
        await session.start()
        let source = try #require(session.source)
        device.stop()
        session.recorder.updateMeter()
        #expect(!session.isActive && session.source == nil && session.error != nil)
        #expect(!FileManager.default.fileExists(atPath: source.path))
    }

    @Test func finalizedFileIsLoadedWithoutChangingMeetingsAndCorrectionsDoNotResetPlayback() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let source = root.appendingPathComponent("capture.wav")
        try Data().write(to: source)
        var meeting = MeetingRecord(title: "합성", startedAt: .now, duration: 0, status: .recording,
            audioPath: source.path, glossaryProfile: "", transcript: [], claims: [], errorMessage: nil)
        let player = AudioPlayerController()
        let captureSource = MeetingPlaybackSource(meeting: meeting)
        player.prepare(meeting: meeting)
        #expect(player.duration == 0 && player.errorMessage == nil)
        let format = try #require(AVAudioFormat(standardFormatWithSampleRate: 16_000, channels: 1))
        let buffer = try #require(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 16_000))
        buffer.frameLength = 16_000
        if let channel = buffer.floatChannelData?[0] { channel.initialize(repeating: 0, count: 16_000) }
        do { let file = try AVAudioFile(forWriting: source, settings: format.settings); try file.write(from: buffer) }
        meeting.status = .processing
        #expect(captureSource != MeetingPlaybackSource(meeting: meeting))
        player.prepare(meeting: meeting)
        #expect(abs(player.duration - 1) < 0.01)
        player.seek(to: 0.5)
        player.setRate(1.5)
        let finalizedSource = MeetingPlaybackSource(meeting: meeting)
        meeting.status = .ready
        meeting.title = "수정된 제목"
        #expect(finalizedSource == MeetingPlaybackSource(meeting: meeting))
        player.prepare(meeting: meeting)
        #expect(player.position == 0.5 && player.rate == 1.5)
        meeting.audioPath = nil
        player.prepare(meeting: meeting)
        #expect(player.duration == 0 && player.sourceChannel == nil && player.errorMessage != nil)
    }

    @Test func unpreparedSelectedPlansDoNotAskForMicrophoneOrCopyFiles() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let fixture = SelectedPreparationFixture()
        let device = ReviewRecordingDevice()
        var permissions = 0
        let store = GroveStore(baseDirectory: root, inferenceService: fixture,
            recorder: AudioRecorder(makeRecorder: { _, _ in device }, permissionRequester: { permissions += 1; return true }))
        var options = MeetingSpeakerOptions()
        #expect(store.preparationMessage(for: try options.plan(isDual: false)) == nil)
        options.transcriptionEngine = .apple
        let selected = try options.plan(isDual: false)
        #expect(store.preparationMessage(for: selected) != nil)
        await store.beginRecording(title: "준비 안 됨", plan: selected)
        let source = root.appendingPathComponent("input.m4a")
        try Data([1, 2, 3]).write(to: source)
        await store.importRecording(from: source, plan: selected)
        #expect(permissions == 0 && device.starts == 0 && store.meetings.isEmpty && fixture.runs == 0)
        #expect(!store.isBusy)
        #expect(try FileManager.default.contentsOfDirectory(atPath: root.appendingPathComponent("Audio").path).isEmpty)
        #expect(try Data(contentsOf: source) == Data([1, 2, 3]))
        fixture.blockedEngine = .moss
        options.transcriptionEngine = .moss
        await store.beginRecording(title: "반대 방식", plan: try options.plan(isDual: false))
        #expect(device.starts == 0 && permissions == 0)
    }

    @Test func unpreparedRetranscriptionPreservesTheCompletedResultAndSavedOptions() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let fixture = SelectedPreparationFixture()
        let store = GroveStore(baseDirectory: root, inferenceService: fixture)
        let source = root.appendingPathComponent("input.m4a")
        try Data([1]).write(to: source)
        await store.importRecording(from: source)
        let meeting = try #require(store.meetings.first)
        let document = try #require(store.transcriptDocuments[meeting.id])
        let indexURL = root.appendingPathComponent("meetings.json")
        let savedIndex = try Data(contentsOf: indexURL)
        var options = MeetingSpeakerOptions()
        options.transcriptionEngine = .apple
        await store.transcribeMeeting(id: meeting.id, plan: try options.plan(isDual: false))
        #expect(store.meetings.first == meeting)
        #expect(store.transcriptDocuments[meeting.id]?.revisionID == document.revisionID)
        #expect(fixture.runs == 1 && !store.isBusy)
        #expect(try Data(contentsOf: indexURL) == savedIndex)
    }

    @Test func cancellationDuringPreparationNeverStartsInferenceOrChangesPriorResult() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let fixture = SelectedPreparationFixture()
        let store = GroveStore(baseDirectory: root, inferenceService: fixture)
        let source = root.appendingPathComponent("input.m4a")
        try Data([1]).write(to: source)
        await store.importRecording(from: source)
        let meeting = try #require(store.meetings.first)
        fixture.suspendPreparation = true
        let retry = Task { await store.transcribeMeeting(id: meeting.id) }
        while fixture.continuation == nil { await Task.yield() }
        #expect(store.isBusy)
        store.cancelProcessing()
        fixture.continuation?.resume()
        await retry.value
        #expect(store.meetings.first == meeting && fixture.runs == 1 && !store.isBusy)
    }

    @Test func preparationDoesNotKeepAnArrayIndexAfterAnotherMeetingIsDeleted() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let fixture = SelectedPreparationFixture()
        let store = GroveStore(baseDirectory: root, inferenceService: fixture)
        let source = root.appendingPathComponent("input.m4a")
        try Data([1]).write(to: source)
        await store.importRecording(from: source)
        let target = try #require(store.meetings.first)
        await store.importRecording(from: source)
        let other = try #require(store.meetings.first)
        fixture.suspendPreparation = true
        let retry = Task { await store.transcribeMeeting(id: target.id) }
        while fixture.continuation == nil { await Task.yield() }
        store.deleteMeeting(other)
        #expect(store.renameMeeting(id: target.id, title: "준비 중 변경"))
        fixture.continuation?.resume()
        await retry.value
        #expect(store.meetings.count == 1 && store.meetings.first?.id == target.id)
        #expect(store.meetings.first?.title == "준비 중 변경" && store.meetings.first?.status == .ready)
    }

    @Test func bundledReadinessRejectsMissingModelsAndUnpreparedAppleAssets() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let service = BundledMeetingInferenceService(appBundle: root, applicationSupport: root)
        let moss = try MeetingSpeakerOptions().plan(isDual: false).configuration
        #expect(service.preparationIssue(configuration: moss, appleReady: true) != nil)
        await #expect(throws: InferenceError.self) { try await service.validatePreparation(configuration: moss) }
        var options = MeetingSpeakerOptions()
        options.transcriptionEngine = .apple
        #expect(service.preparationIssue(configuration: try options.plan(isDual: false).configuration, appleReady: false) != nil)
    }

    @Test(arguments: [true, false]) func lateCalendarGrantOrDenialCannotOverrideDisable(allowed: Bool) async throws {
        let defaults = try #require(UserDefaults(suiteName: "Grove.Calendar.Tests.\(UUID().uuidString)"))
        let permission = CalendarPermissionFixture()
        let schedule = CalendarSchedule(defaults: defaults, requestPermission: { try await permission.request() })
        let enabling = Task { await schedule.setEnabled(true) }
        while permission.requests.isEmpty { await Task.yield() }
        await schedule.setEnabled(false)
        permission.requests.removeFirst().resume(returning: allowed)
        await enabling.value
        #expect(!schedule.enabled && !defaults.bool(forKey: "calendarEnabled") && schedule.message == nil)
    }

    @Test func lateCalendarErrorCannotOverrideDisableOrNewerEnable() async throws {
        let defaults = try #require(UserDefaults(suiteName: "Grove.Calendar.Tests.\(UUID().uuidString)"))
        let permission = CalendarPermissionFixture()
        let schedule = CalendarSchedule(defaults: defaults, requestPermission: { try await permission.request() })
        let old = Task { await schedule.setEnabled(true) }
        while permission.requests.isEmpty { await Task.yield() }
        await schedule.setEnabled(false)
        let latest = Task { await schedule.setEnabled(true) }
        while permission.requests.count < 2 { await Task.yield() }
        permission.requests.removeLast().resume(returning: true)
        await latest.value
        let latestMessage = schedule.message
        permission.requests.removeFirst().resume(throwing: InferenceError.noSpeech)
        await old.value
        #expect(schedule.enabled && defaults.bool(forKey: "calendarEnabled") && schedule.message == latestMessage)
    }
}
