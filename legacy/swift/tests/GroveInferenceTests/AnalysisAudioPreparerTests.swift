import AVFoundation
import Foundation
import Testing
@testable import GroveInference

struct AnalysisAudioPreparerTests {
    private func writeTone(at url: URL, rate: Double, channels: AVAudioChannelCount, frames: AVAudioFrameCount, silentFirstChannel: Bool = false) throws {
        let format = try #require(AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: rate, channels: channels, interleaved: false))
        let buffer = try #require(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames))
        buffer.frameLength = frames
        for channel in 0..<Int(channels) {
            for frame in 0..<Int(frames) {
                buffer.floatChannelData![channel][frame] = silentFirstChannel && channel == 0 ? 0 : Float(sin(2 * .pi * 440 * Double(frame) / rate) * 0.2)
            }
        }
        let writer = try AVAudioFile(forWriting: url, settings: [
            AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: rate, AVNumberOfChannelsKey: channels,
            AVLinearPCMBitDepthKey: 16, AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false,
        ], commonFormat: .pcmFormatFloat32, interleaved: false)
        try writer.write(from: buffer)
    }

    @Test func normalizedInputIsPreservedByteForByte() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("input.wav")
        let output = root.appendingPathComponent("output.wav")
        try writeTone(at: input, rate: 16_000, channels: 1, frames: 16_000)
        let original = try Data(contentsOf: input)
        let result = try await AnalysisAudioPreparer.prepare(source: input, destination: output)
        #expect(result.frameCount == 16_000)
        #expect(result.duration == 1)
        #expect(try Data(contentsOf: output) == original)
        #expect(try Data(contentsOf: input) == original)
    }

    @Test func stereo48kIsStreamedIntoMono16kWithoutLosingTheTail() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("stereo.wav")
        let output = root.appendingPathComponent("mono.wav")
        try writeTone(at: input, rate: 48_000, channels: 2, frames: 144_000)
        let result = try await AnalysisAudioPreparer.prepare(source: input, destination: output)
        #expect(abs(result.duration - 3) < 0.02)
        let file = try AVAudioFile(forReading: output)
        #expect(file.processingFormat.sampleRate == 16_000)
        #expect(file.processingFormat.channelCount == 1)
        file.framePosition = file.length - 1000
        let tail = try #require(AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 1000))
        try file.read(into: tail)
        let samples = Array(UnsafeBufferPointer(start: tail.floatChannelData![0], count: Int(tail.frameLength)))
        #expect(samples.map { abs($0) }.max()! > 0.05)
    }

    @Test func existingDestinationAndOriginalCannotBeOverwritten() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("input.wav")
        try writeTone(at: input, rate: 16_000, channels: 1, frames: 1000)
        let bytes = try Data(contentsOf: input)
        await #expect(throws: InferenceError.self) { try await AnalysisAudioPreparer.prepare(source: input, destination: input) }
        #expect(try Data(contentsOf: input) == bytes)
    }

    @Test func speechOnTheSecondChannelIsNotDropped() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let input = root.appendingPathComponent("right-only.wav")
        let output = root.appendingPathComponent("mono.wav")
        try writeTone(at: input, rate: 48_000, channels: 2, frames: 48_000, silentFirstChannel: true)
        _ = try await AnalysisAudioPreparer.prepare(source: input, destination: output)
        let file = try AVAudioFile(forReading: output)
        let buffer = try #require(AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 16_000))
        try file.read(into: buffer)
        let samples = Array(UnsafeBufferPointer(start: buffer.floatChannelData![0], count: Int(buffer.frameLength)))
        #expect(samples.map { abs($0) }.max()! > 0.02)
    }
}
