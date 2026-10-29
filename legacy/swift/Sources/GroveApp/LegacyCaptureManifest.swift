import Foundation

// Read-only compatibility for recordings created before microphone-only capture.
struct CaptureChannelStats: Codable, Sendable {
    var path: String
    var firstPresentationSeconds: Double?
    var lastPresentationSeconds: Double?
    var bufferCount: Int
    var detectedGapCount: Int
    var writtenFrameCount: Int64
}

struct CaptureSessionManifest: Codable, Sendable {
    enum Status: String, Codable, Sendable { case recording, finished, failed }
    var sessionID: UUID
    var status: Status
    var createdAt: Date
    var startedAt: Date?
    var endedAt: Date?
    var selectedContentDescription: String
    var systemAudio: CaptureChannelStats
    var microphone: CaptureChannelStats
    var errorMessage: String?
    var recordedDuration: TimeInterval? = nil
}
