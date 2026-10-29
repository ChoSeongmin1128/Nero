@preconcurrency import AVFoundation
import Foundation
import GroveInference

enum VoiceModelCheck {
    static func run(directory: URL) async throws {
        let work = FileManager.default.temporaryDirectory.appendingPathComponent("grove-voice-check-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: work, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: work) }
        let url = work.appendingPathComponent("check.wav")
        try writeSignal(to: url)
        let samples = try await VoiceEmbeddingExtractor(modelDirectory: directory, recipe: .activeFrameCenteredV2)
            .extractSamples(source: url, ranges: [.init(start: 0, end: 2)], workingDirectory: work)
        guard samples.count == 1, samples[0].voicePrint.embedding.count == 256 else {
            throw ModelPreparationError.integrity
        }
    }

    private static func writeSignal(to url: URL) throws {
        guard let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false),
              let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 32_000) else {
            throw ModelPreparationError.integrity
        }
        buffer.frameLength = 32_000
        for i in 0..<32_000 { buffer.floatChannelData![0][i] = Float(sin(Double(i) * 0.07) * 0.1) }
        let file = try AVAudioFile(forWriting: url, settings: [AVFormatIDKey: kAudioFormatLinearPCM,
            AVSampleRateKey: 16_000, AVNumberOfChannelsKey: 1, AVLinearPCMBitDepthKey: 16,
            AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false],
            commonFormat: .pcmFormatFloat32, interleaved: false)
        try file.write(from: buffer)
    }
}
