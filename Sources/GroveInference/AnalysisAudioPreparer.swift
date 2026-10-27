@preconcurrency import AVFoundation
import Foundation

public struct PreparedAnalysisAudio: Codable, Sendable {
    public let url: URL
    public let frameCount: Int64
    public let sampleRate: Double
    public var duration: Double { Double(frameCount) / sampleRate }
}

public enum AnalysisAudioPreparer {
    public static func prepare(source: URL, destination: URL) async throws -> PreparedAnalysisAudio {
        try Task.checkCancellation()
        let task = Task.detached(priority: .userInitiated) {
            try convert(source: source, destination: destination)
        }
        return try await withTaskCancellationHandler {
            try await task.value
        } onCancel: { task.cancel() }
    }

    private static func convert(source: URL, destination: URL) throws -> PreparedAnalysisAudio {
        let files = FileManager.default
        guard source.isFileURL, destination.isFileURL, files.fileExists(atPath: source.path),
              !files.fileExists(atPath: destination.path),
              source.resolvingSymlinksInPath() != destination.resolvingSymlinksInPath() else {
            throw InferenceError.invalidOutput("원본과 다른 새 분석 파일 경로가 필요합니다.")
        }
        let input = try AVAudioFile(forReading: source)
        guard input.length > 0, input.processingFormat.sampleRate > 0, input.processingFormat.channelCount > 0 else {
            throw InferenceError.invalidOutput("녹음 파일에 읽을 수 있는 음성이 없습니다.")
        }
        let directory = destination.deletingLastPathComponent()
        try files.createDirectory(at: directory, withIntermediateDirectories: true)
        let temporary = directory.appendingPathComponent(".analysis-\(UUID().uuidString).wav")
        defer { if files.fileExists(atPath: temporary.path) { try? files.removeItem(at: temporary) } }
        try Task.checkCancellation()
        let bits = input.fileFormat.settings[AVLinearPCMBitDepthKey] as? Int
        if source.pathExtension.lowercased() == "wav", input.fileFormat.sampleRate == 16_000,
           input.fileFormat.channelCount == 1, bits == 16, try isCanonicalPCM16WAV(source) {
            // Preserve the already-normalized benchmark waveform byte for byte.
            try files.copyItem(at: source, to: temporary)
        } else {
            try resample(input: input, output: temporary)
        }
        try Task.checkCancellation()
        let normalized = try AVAudioFile(forReading: temporary)
        guard normalized.processingFormat.sampleRate == 16_000,
              normalized.processingFormat.channelCount == 1, normalized.length > 0 else {
            throw InferenceError.invalidOutput("분석용 음성 파일을 만들지 못했습니다.")
        }
        let frames = normalized.length
        try files.moveItem(at: temporary, to: destination)
        return PreparedAnalysisAudio(url: destination, frameCount: frames, sampleRate: 16_000)
    }

    private static func isCanonicalPCM16WAV(_ source: URL) throws -> Bool {
        let file = try FileHandle(forReadingFrom: source)
        defer { try? file.close() }
        guard let header = try file.read(upToCount: 12), header.count == 12,
              header.prefix(4) == Data("RIFF".utf8), header.suffix(4) == Data("WAVE".utf8) else { return false }
        while true {
            try Task.checkCancellation()
            guard let chunk = try file.read(upToCount: 8), chunk.count == 8 else { return false }
            let size = chunk.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 4, as: UInt32.self)) }
            if chunk.prefix(4) == Data("fmt ".utf8) {
                guard size >= 16, let format = try file.read(upToCount: 2), format.count == 2 else { return false }
                return format == Data([1, 0])
            }
            let offset = try file.offset()
            try file.seek(toOffset: offset + UInt64(size) + UInt64(size % 2))
        }
    }

    private static func resample(input: AVAudioFile, output: URL) throws {
        guard let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false),
              let converter = AVAudioConverter(from: input.processingFormat, to: format),
              let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 4096) else {
            throw InferenceError.invalidOutput("이 음성 형식의 변환을 지원하지 않습니다.")
        }
        converter.sampleRateConverterQuality = AVAudioQuality.max.rawValue
        converter.downmix = true
        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: 16_000,
            AVNumberOfChannelsKey: 1, AVLinearPCMBitDepthKey: 16,
            AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false,
            AVLinearPCMIsNonInterleaved: false,
        ]
        let writer = try AVAudioFile(forWriting: output, settings: settings, commonFormat: .pcmFormatFloat32, interleaved: false)
        var stalledReads = 0
        while true {
            try Task.checkCancellation()
            buffer.frameLength = 0
            let readError = ConversionReadError()
            var conversionError: NSError?
            let status = converter.convert(to: buffer, error: &conversionError) { requested, state in
                do {
                    try Task.checkCancellation()
                    if input.framePosition >= input.length { state.pointee = .endOfStream; return nil }
                    guard let chunk = AVAudioPCMBuffer(pcmFormat: input.processingFormat, frameCapacity: min(32_768, max(1, requested))) else {
                        throw InferenceError.invalidOutput("음성 버퍼를 할당하지 못했습니다.")
                    }
                    try input.read(into: chunk)
                    if chunk.frameLength == 0 { state.pointee = .endOfStream; return nil }
                    state.pointee = .haveData
                    return chunk
                } catch {
                    readError.store(error)
                    state.pointee = .endOfStream
                    return nil
                }
            }
            if let error = readError.value { throw error }
            if let conversionError { throw conversionError }
            if status == .error { throw InferenceError.invalidOutput("음성 변환 중 오류가 발생했습니다.") }
            if buffer.frameLength > 0 {
                try writer.write(from: buffer)
                stalledReads = 0
            } else { stalledReads += 1 }
            if status == .endOfStream { break }
            if stalledReads > 3 { throw InferenceError.invalidOutput("음성 변환이 더 이상 진행되지 않습니다.") }
        }
    }
}

private final class ConversionReadError: @unchecked Sendable {
    private let lock = NSLock()
    private var error: (any Error)?
    var value: (any Error)? { lock.withLock { error } }
    func store(_ error: any Error) { lock.withLock { self.error = error } }
}
