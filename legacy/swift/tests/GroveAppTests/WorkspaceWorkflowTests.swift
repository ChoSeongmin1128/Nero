import Foundation
import Testing
@testable import GroveApp

@MainActor
private final class WorkflowRecordingDevice: AudioRecordingDevice {
    var isRecording = false
    var currentTime: TimeInterval = 0
    var isMeteringEnabled = false
    var recordCount = 0
    func prepareToRecord() -> Bool { true }
    func record() -> Bool { recordCount += 1; isRecording = true; return true }
    func pause() { isRecording = false }
    func stop() { isRecording = false }
    func updateMeters() {}
    func averagePower(forChannel channelNumber: Int) -> Float { -20 }
}

@MainActor
private final class WorkflowPermissionPrompt {
    var responses: [CheckedContinuation<Bool, Never>] = []
    func request() async -> Bool { await withCheckedContinuation { responses.append($0) } }
    func reply(_ value: Bool) { responses.removeFirst().resume(returning: value) }
}

@MainActor
struct WorkspaceWorkflowTests {
    @Test func recordingStartsOnlyAfterTheSetupSheetHasClosed() async throws {
        let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: base) }
        let prompt = WorkflowPermissionPrompt()
        let device = WorkflowRecordingDevice()
        let store = GroveStore(baseDirectory: base, inferenceService: PreparedFixtureInference(), recorder: AudioRecorder(makeRecorder: { _, _ in device }, permissionRequester: { await prompt.request() }))
        store.present(.newMeeting())
        store.startAfterDismissingSheet(.recording(title: "대기", plan: try MeetingSpeakerOptions().plan(isDual: false),
            folderID: nil, calendarEvent: nil, attendance: .init()))
        await Task.yield()
        #expect(store.workspaceSheet == nil)
        #expect(store.isBusy)
        #expect(prompt.responses.isEmpty && store.meetings.isEmpty)
        store.present(.newMeeting())
        #expect(store.workspaceSheet == nil)
        store.workspaceSheetDidDismiss()
        while prompt.responses.isEmpty { await Task.yield() }
        #expect(store.isStartingCapture)
        prompt.reply(false)
        while store.isStartingCapture { await Task.yield() }
        #expect(!store.isBusy && device.recordCount == 0)
    }

    @Test func dismissingAnUnsubmittedSetupDoesNotStartRecording() async {
        let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: base) }
        let device = WorkflowRecordingDevice()
        let store = GroveStore(baseDirectory: base, inferenceService: PreparedFixtureInference(), recorder: AudioRecorder(makeRecorder: { _, _ in device }, permissionRequester: { true }))
        store.present(.newMeeting())
        store.workspaceSheet = nil
        store.workspaceSheetDidDismiss()
        await Task.yield()
        #expect(!store.isBusy && store.meetings.isEmpty && device.recordCount == 0)
    }

    @Test func closingRecordingSetupWhilePermissionIsPendingDoesNotStartMicrophone() async throws {
        let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: base) }
        let prompt = WorkflowPermissionPrompt()
        let device = WorkflowRecordingDevice()
        let recorder = AudioRecorder(makeRecorder: { _, _ in device }, permissionRequester: { await prompt.request() })
        let store = GroveStore(baseDirectory: base, inferenceService: PreparedFixtureInference(), recorder: recorder)
        store.present(.newMeeting())
        let task = Task { await store.beginRecording(title: "취소할 녹음") }
        while prompt.responses.isEmpty { await Task.yield() }
        #expect(store.isStartingCapture)
        store.cancelRecordingStart()
        store.workspaceSheet = nil
        prompt.reply(true)
        await task.value
        #expect(device.recordCount == 0)
        #expect(!store.isRecording && !store.isStartingCapture)
        #expect(store.meetings.isEmpty)
    }

    @Test func cancelledPermissionReplyCannotClearOrStartANewerRecordingAttempt() async throws {
        let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: base) }
        let prompt = WorkflowPermissionPrompt()
        let device = WorkflowRecordingDevice()
        let store = GroveStore(baseDirectory: base, inferenceService: PreparedFixtureInference(), recorder: AudioRecorder(makeRecorder: { _, _ in device }, permissionRequester: { await prompt.request() }))
        let first = Task { await store.beginRecording(title: "이전") }
        while prompt.responses.isEmpty { await Task.yield() }
        store.cancelRecordingStart()
        let second = Task { await store.beginRecording(title: "새 시도") }
        while prompt.responses.count < 2 { await Task.yield() }
        prompt.reply(true)
        await first.value
        #expect(store.isStartingCapture)
        #expect(device.recordCount == 0)
        prompt.reply(false)
        await second.value
        #expect(!store.isStartingCapture)
        #expect(device.recordCount == 0)
    }

    @Test func importWriteFailureRestoresNavigationAndKeepsTheSource() async throws {
        let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: base) }
        let index = base.appendingPathComponent("meetings.json")
        try MeetingRecordStorage(url: index).save([])
        let store = GroveStore(baseDirectory: base, inferenceService: PreparedFixtureInference())
        store.selection = .unfiled
        let source = base.appendingPathComponent("input.m4a")
        let bytes = Data([1, 2, 3, 4]); try bytes.write(to: source)
        try FileManager.default.createDirectory(at: index.appendingPathExtension("backup"), withIntermediateDirectories: false)
        await store.importRecording(from: source)
        #expect(store.selection == .unfiled)
        #expect(store.meetings.isEmpty)
        #expect(!store.isBusy)
        #expect(try Data(contentsOf: source) == bytes)
        #expect(try MeetingRecordStorage(url: index).load().isEmpty)
    }

    @Test func recordingIndexFailureStopsTheDeviceAndRestoresNavigation() async throws {
        let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: base) }
        let index = base.appendingPathComponent("meetings.json")
        try MeetingRecordStorage(url: index).save([])
        let device = WorkflowRecordingDevice()
        let store = GroveStore(baseDirectory: base, inferenceService: PreparedFixtureInference(), recorder: AudioRecorder(makeRecorder: { _, _ in device }, permissionRequester: { true }))
        store.selection = .unfiled
        try FileManager.default.createDirectory(at: index.appendingPathExtension("backup"), withIntermediateDirectories: false)
        await store.beginRecording(title: "저장 실패")
        #expect(store.selection == .unfiled)
        #expect(!store.isRecording && !device.isRecording)
        #expect(store.activeMeetingID == nil)
        #expect(store.meetings.isEmpty)
    }

    @Test func existingSheetCannotBeReplacedByAnUnrelatedAction() {
        let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: base) }
        let store = GroveStore(baseDirectory: base, inferenceService: PreparedFixtureInference())
        let first = WorkspaceSheet.newMeeting()
        store.present(first)
        store.present(.importRecording(base.appendingPathComponent("other.m4a")))
        #expect(store.workspaceSheet == first)
        #expect(!store.canPresentNewMeeting)
    }
}
