@preconcurrency import AVFoundation
import Foundation
import Testing
@testable import GroveInference

struct VoiceEnrollmentAudioTests {
    @Test func silenceClippingAndMissingNaturalSpeechCannotPass() async throws {
        for amplitude: Float in [0, 1] {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: root) }
            let url = root.appendingPathComponent("input.wav")
            try writeSignal(to: url, amplitude: amplitude)
            let result = try await VoiceEnrollmentAudio.inspect(source: url, naturalSpeechStart: 16, workingDirectory: root)
            #expect(!result.canExtract)
            #expect(try FileManager.default.contentsOfDirectory(atPath: root.path) == ["input.wav"])
        }
    }

    @Test func selectedRangesIncludeBothPartsAndStayWithinTheAudio() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let url = root.appendingPathComponent("input.wav")
        try writeSignal(to: url, amplitude: 0.1)
        let result = try await VoiceEnrollmentAudio.inspect(source: url, naturalSpeechStart: 16, workingDirectory: root)
        #expect(result.canExtract)
        #expect(result.ranges.count == 4)
        #expect(result.ranges.allSatisfy { $0.start >= 0 && $0.end <= 32 && $0.end - $0.start <= 10 })
        #expect(result.ranges.contains { $0.start >= 16 })
        #expect(result.ranges.contains { $0.end <= 16 })
        await #expect(throws: InferenceError.self) {
            try await VoiceEnrollmentAudio.inspect(source: url, naturalSpeechStart: 0, workingDirectory: root)
        }
    }

    @Test func delayedAutomaticStopStillUsesOnlyTheBoundedEnrollmentWindow() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let url = root.appendingPathComponent("input.wav")
        try writeSignal(to: url, amplitude: 0.1, duration: 120.32)
        let result = try await VoiceEnrollmentAudio.inspect(source: url, naturalSpeechStart: 60, workingDirectory: root)
        #expect(result.canExtract)
        #expect(result.duration > 120)
        #expect(result.ranges.allSatisfy { $0.end <= VoiceEnrollmentAudio.maximumDuration })
        #expect(result.signalSeconds <= VoiceEnrollmentAudio.maximumDuration)
    }

    private func writeSignal(to url: URL, amplitude: Float, duration: Double = 32) throws {
        let format = try #require(AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false))
        let frames = AVAudioFrameCount(duration * 16_000)
        let buffer = try #require(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames))
        buffer.frameLength = frames
        for i in 0..<Int(frames) { buffer.floatChannelData![0][i] = i.isMultiple(of: 2) ? amplitude : -amplitude }
        let file = try AVAudioFile(forWriting: url, settings: [AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: 16_000,
            AVNumberOfChannelsKey: 1, AVLinearPCMBitDepthKey: 16, AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false],
            commonFormat: .pcmFormatFloat32, interleaved: false)
        try file.write(from: buffer)
    }
}
