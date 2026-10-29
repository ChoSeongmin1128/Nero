@preconcurrency import AVFoundation
import Foundation

public struct VoiceEnrollmentAudioReview: Sendable {
    public let duration: Double
    public let signalSeconds: Double
    public let clippedFraction: Double
    public let ranges: [VoiceSampleRange]
    public let includesNaturalSpeech: Bool

    public var canExtract: Bool {
        ranges.count >= 3 && ranges.reduce(0, { $0 + $1.end - $1.start }) >= 10
            && includesNaturalSpeech && clippedFraction < 0.005
    }
}

public enum VoiceEnrollmentAudio {
    public static let maximumDuration: Double = 120
    public static func inspect(source: URL, naturalSpeechStart: Double, workingDirectory: URL) async throws -> VoiceEnrollmentAudioReview {
        let work = workingDirectory.appendingPathComponent("voice-quality-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: work, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: work) }
        let prepared = try await AnalysisAudioPreparer.prepare(source: source, destination: work.appendingPathComponent("analysis.wav"))
        let task = Task.detached(priority: .utility) { try inspectPrepared(prepared, naturalSpeechStart: naturalSpeechStart) }
        return try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
    }

    private static func inspectPrepared(_ audio: PreparedAnalysisAudio, naturalSpeechStart: Double) throws -> VoiceEnrollmentAudioReview {
        let analyzedDuration = min(audio.duration, maximumDuration)
        guard naturalSpeechStart.isFinite, naturalSpeechStart > 0,
              naturalSpeechStart < analyzedDuration else {
            throw InferenceError.invalidOutput("예문을 읽은 뒤 자유 발화도 녹음해 주세요. 한 번에 2분까지 등록할 수 있습니다.")
        }
        let file = try AVAudioFile(forReading: audio.url, commonFormat: .pcmFormatFloat32, interleaved: false)
        let lastFrame = min(file.length, AVAudioFramePosition(maximumDuration * audio.sampleRate))
        guard let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 320) else {
            throw InferenceError.invalidOutput("등록 음성을 읽지 못했습니다.")
        }
        var active: [Bool] = []
        var clipped = 0, frames = 0
        while file.framePosition < lastFrame {
            try Task.checkCancellation()
            try file.read(into: buffer, frameCount: AVAudioFrameCount(min(320, lastFrame - file.framePosition)))
            guard let channel = buffer.floatChannelData?[0], buffer.frameLength > 0 else { break }
            var energy = 0.0
            for i in 0..<Int(buffer.frameLength) {
                let value = channel[i]
                guard value.isFinite else { throw InferenceError.invalidOutput("등록 음성에 잘못된 샘플이 있습니다.") }
                energy += Double(value) * Double(value)
                if abs(value) >= 0.99 { clipped += 1 }
            }
            frames += Int(buffer.frameLength)
            // An amplitude check, not a speech detector or a guarantee of one speaker.
            active.append(energy / Double(buffer.frameLength) >= 0.00001)
        }
        let boundary = min(active.count, Int(naturalSpeechStart / 0.02))
        let reading = selectRanges(active: active, start: 0, end: boundary, limit: 3)
        let natural = selectRanges(active: active, start: boundary, end: active.count, limit: 2)
        return VoiceEnrollmentAudioReview(duration: audio.duration,
            signalSeconds: Double(active.filter { $0 }.count) * 0.02,
            clippedFraction: Double(clipped) / Double(max(1, frames)),
            ranges: (Array(reading.prefix(3)) + Array(natural.prefix(2))).sorted { $0.start < $1.start },
            includesNaturalSpeech: !natural.isEmpty)
    }

    private static func selectRanges(active: [Bool], start: Int, end: Int, limit: Int) -> [VoiceSampleRange] {
        var candidates: [(VoiceSampleRange, Int)] = []
        var cursor = start
        while cursor + 100 <= end {
            let stop = min(cursor + 400, end)
            let voiced = (cursor..<stop).filter { active[$0] }
            if voiced.count >= 100, let first = voiced.first, let last = voiced.last, last - first >= 100 {
                candidates.append((.init(start: Double(first) * 0.02, end: Double(last + 1) * 0.02), voiced.count))
            }
            cursor = stop
        }
        return candidates.sorted { $0.1 > $1.1 }.prefix(limit).map(\.0).sorted { $0.start < $1.start }
    }
}
