import Foundation
import Testing
@testable import GroveApp

@MainActor
private final class EnrollmentTestDevice: AudioRecordingDevice {
    var isRecording = false
    var currentTime: TimeInterval = 0
    var isMeteringEnabled = false
    var starts = 0
    func prepareToRecord() -> Bool { true }
    func record() -> Bool { starts += 1; isRecording = true; return true }
    func pause() { isRecording = false }
    func stop() { isRecording = false }
    func updateMeters() {}
    func averagePower(forChannel channelNumber: Int) -> Float { -20 }
}

@MainActor
struct VoiceEnrollmentSessionTests {
    @Test func cancellingDuringPermissionCannotStartRecordingAfterTheSheetClosed() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let device = EnrollmentTestDevice()
        let recorder = AudioRecorder(makeRecorder: { _, _ in device })
        var permission: CheckedContinuation<Bool, Never>?
        let session = VoiceEnrollmentSession(directory: root, recorder: recorder, requestPermission: {
            await withCheckedContinuation { permission = $0 }
        })
        let start = Task { await session.start() }
        while permission == nil { await Task.yield() }
        #expect(session.isActive)
        session.discard()
        permission?.resume(returning: true)
        await start.value
        #expect(device.starts == 0)
        #expect(session.source == nil)
        #expect(!session.isActive)
    }

    @Test func cancellationStopsTheMicrophoneAndRemovesOnlyTheOwnedCapture() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let sentinel = root.appendingPathComponent("keep.txt")
        try Data("keep".utf8).write(to: sentinel)
        let device = EnrollmentTestDevice()
        let recorder = AudioRecorder(makeRecorder: { url, _ in
            try Data([1, 2, 3]).write(to: url)
            return device
        })
        let session = VoiceEnrollmentSession(directory: root, recorder: recorder, requestPermission: { true })
        await session.start()
        let capture = try #require(session.source)
        #expect(session.isActive)
        #expect(FileManager.default.fileExists(atPath: capture.path))
        session.discard()
        #expect(!device.isRecording)
        #expect(!FileManager.default.fileExists(atPath: capture.path))
        #expect(try Data(contentsOf: sentinel) == Data("keep".utf8))
    }
}
