@preconcurrency import AVFoundation
import Foundation

// New recordings are written as ADTS AAC so a crash keeps every completed frame. Once the
// capture is finished the same AAC packets are repackaged, without re-encoding, into M4A,
// which players can time and seek accurately.
enum RecordingFinalizer {
    static func captureURL(id: UUID, in directory: URL) -> URL {
        directory.appendingPathComponent("\(id.uuidString).capture.aac")
    }

    enum FinalizeError: LocalizedError {
        case destinationExists, lengthMismatch
        var errorDescription: String? {
            switch self {
            case .destinationExists: "정리된 녹음 파일이 이미 있습니다. 원본은 그대로 둡니다."
            case .lengthMismatch: "녹음 파일을 정리하는 중 길이가 달라졌습니다. 원본은 그대로 둡니다."
            }
        }
    }

    // Imported .aac files keep their container; only Grove's own captures are repackaged.
    static func needsFinalizing(_ path: String) -> Bool { path.hasSuffix(".capture.aac") }

    static func duration(of url: URL) -> TimeInterval? {
        guard let file = try? AVAudioFile(forReading: url), file.processingFormat.sampleRate > 0 else { return nil }
        return Double(file.length) / file.processingFormat.sampleRate
    }

    /// Writes `<name>.m4a` next to the capture and leaves the capture in place; the caller
    /// removes it after the meeting index points at the packaged file.
    static func package(_ capture: URL) async throws -> (url: URL, duration: TimeInterval) {
        let destination = capture.deletingPathExtension().deletingPathExtension().appendingPathExtension("m4a")
        guard !FileManager.default.fileExists(atPath: destination.path) else { throw FinalizeError.destinationExists }
        let temporary = capture.deletingLastPathComponent().appendingPathComponent(".\(UUID().uuidString).m4a")
        defer { try? FileManager.default.removeItem(at: temporary) }
        guard let session = AVAssetExportSession(asset: AVURLAsset(url: capture), presetName: AVAssetExportPresetPassthrough) else {
            throw FinalizeError.lengthMismatch
        }
        try await session.export(to: temporary, as: .m4a)
        let original = try AVAudioFile(forReading: capture)
        let packaged = try AVAudioFile(forReading: temporary)
        guard original.length > 0, packaged.length == original.length,
              packaged.processingFormat.sampleRate == original.processingFormat.sampleRate else {
            throw FinalizeError.lengthMismatch
        }
        try FileManager.default.moveItem(at: temporary, to: destination)
        return (destination, Double(packaged.length) / packaged.processingFormat.sampleRate)
    }
}
