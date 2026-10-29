import Foundation
import GroveInference

enum MeetingStatus: String, Codable, CaseIterable, Sendable {
    case recording
    case processing
    case ready
    case needsReview
    case failed

    var label: String {
        switch self {
        case .recording: "녹음 중"
        case .processing: "정확 전사 중"
        case .ready: "전사 완료"
        case .needsReview: "확인 필요"
        case .failed: "처리 실패"
        }
    }

    var symbol: String {
        switch self {
        case .recording: "record.circle.fill"
        case .processing: "waveform.badge.magnifyingglass"
        case .ready: "checkmark.circle.fill"
        case .needsReview: "exclamationmark.circle.fill"
        case .failed: "xmark.octagon.fill"
        }
    }
}

// The second case is retained solely to decode recordings made by older versions.
enum CaptureMode: String, Codable, Sendable {
    case microphone = "마이크"
    case systemAndMicrophone = "시스템 + 마이크"

}

struct TranscriptSegment: Identifiable, Codable, Hashable, Sendable {
    var id: UUID = UUID()
    var startTime: TimeInterval
    var endTime: TimeInterval
    var speaker: String
    var text: String
    var confidence: Double?
    var revisedText: String?

    var displayedText: String { revisedText ?? text }
    var isRevised: Bool { revisedText != nil && revisedText != text }
}

struct EvidenceClaim: Identifiable, Codable, Hashable, Sendable {
    enum Kind: String, Codable, Sendable {
        case decision
        case action
        case summary
    }

    var id: UUID = UUID()
    var kind: Kind
    var text: String
    var owner: String?
    var sourceSegmentIDs: [UUID]
    var isReviewed: Bool
}

struct MeetingRecord: Identifiable, Codable, Hashable, Sendable {
    var id: UUID = UUID()
    var title: String
    var startedAt: Date
    var duration: TimeInterval
    var status: MeetingStatus
    var audioPath: String?
    var systemAudioPath: String? = nil
    var microphoneAudioPath: String? = nil
    var captureManifestPath: String? = nil
    var captureMode: CaptureMode? = nil
    var inferenceConfiguration: InferenceConfiguration? = nil
    var channelInferenceConfigurations: [String: InferenceConfiguration]? = nil
    var processingOutcome: MeetingProcessingOutcome? = nil
    var completedResult: MeetingCompletedResult? = nil
    var folderID: UUID? = nil
    var calendarEvent: ScheduledMeeting? = nil
    var attendance: MeetingAttendance? = nil
    var glossaryProfile: String
    var transcript: [TranscriptSegment]
    var claims: [EvidenceClaim]
    var errorMessage: String?

    // Stored with each record by earlier versions; the app no longer applies glossaries.
    static let noGlossaryProfile = "사전 없음"
}

enum SidebarDestination: Hashable {
    case library
    case unfiled
    case folder(UUID)
    case meeting(UUID)
}
