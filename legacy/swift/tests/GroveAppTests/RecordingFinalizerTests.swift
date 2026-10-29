@preconcurrency import AVFoundation
import Foundation
import Testing
@testable import GroveApp

struct RecordingFinalizerTests {
    private func writeCapture(to url: URL, seconds: Int) throws {
        let settings: [String: Any] = [AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 48_000,
                                       AVNumberOfChannelsKey: 1, AVEncoderBitRateKey: 128_000]
        let file = try AVAudioFile(forWriting: url, settings: settings, commonFormat: .pcmFormatFloat32, interleaved: false)
        let buffer = try #require(AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 48_000))
        buffer.frameLength = 48_000
        for index in 0..<48_000 { buffer.floatChannelData![0][index] = Float(sin(Double(index) * 0.06) * 0.2) }
        for _ in 0..<seconds { try file.write(from: buffer) }
    }

    @Test func captureIsRepackagedWithoutLosingFramesAndLeftForTheCaller() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let capture = RecordingFinalizer.captureURL(id: UUID(), in: root)
        try writeCapture(to: capture, seconds: 3)
        #expect(RecordingFinalizer.needsFinalizing(capture.path))
        let frames = try AVAudioFile(forReading: capture).length
        let packaged = try await RecordingFinalizer.package(capture)
        #expect(packaged.url.pathExtension == "m4a" && !RecordingFinalizer.needsFinalizing(packaged.url.path))
        #expect(try AVAudioFile(forReading: packaged.url).length == frames)
        #expect(abs(packaged.duration - Double(frames) / 48_000) < 0.001)
        #expect(FileManager.default.fileExists(atPath: capture.path))
        await #expect(throws: RecordingFinalizer.FinalizeError.self) { try await RecordingFinalizer.package(capture) }
    }

    @Test func importedAACFilesAreNotTreatedAsCaptures() {
        #expect(!RecordingFinalizer.needsFinalizing("/tmp/\(UUID().uuidString).aac"))
        #expect(!RecordingFinalizer.needsFinalizing("/tmp/\(UUID().uuidString).m4a"))
    }
}
