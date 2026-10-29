import Foundation

struct MeetingProcessingOutcome: Codable, Hashable, Sendable {
    enum Kind: String, Codable, Sendable { case completed, failed, cancelled, interrupted }
    var kind: Kind
    var message: String? = nil
    var previousResultRetained = false
    var retainedRevisionID: UUID? = nil
    var recordedAt = Date()
}

struct MeetingSpeakerCount: Codable, Hashable, Sendable, Identifiable {
    let sourceID: String
    let expected: Int?
    let detected: Int
    var id: String { sourceID }
    var isMismatch: Bool { expected.map { $0 != detected } ?? false }
    var label: String {
        let prefix = sourceID == "system" ? "컴퓨터 소리 " : sourceID == "microphone" ? "마이크 " : ""
        if let expected { return "\(prefix)입력 \(expected)명 · 감지 \(detected)명" }
        return "\(prefix)감지 \(detected)명"
    }
}

struct MeetingCompletedResult: Codable, Hashable, Sendable {
    let revisionID: UUID
    let speakerCounts: [MeetingSpeakerCount]
}

/// Presentation only: legacy error strings are never parsed into inferred failures.
struct MeetingPresentationStatus {
    let label: String
    let symbol: String
    let isFailure: Bool
    let detail: String?
    let canRetry: Bool

    init(meeting: MeetingRecord, document: TranscriptDocument?) {
        let hasResult = document != nil || !meeting.transcript.isEmpty
        let hasAudio = meeting.audioPath != nil || meeting.systemAudioPath != nil || meeting.microphoneAudioPath != nil
        if meeting.status == .recording || meeting.status == .processing {
            label = meeting.status == .recording ? "녹음 중" : "전사 중"
            symbol = meeting.status.symbol
            isFailure = false
            detail = nil
            canRetry = false
        } else if let outcome = meeting.processingOutcome, outcome.kind != .completed {
            let retained = hasResult && outcome.previousResultRetained
                && (outcome.retainedRevisionID == nil || outcome.retainedRevisionID == document?.revisionID)
            switch outcome.kind {
            case .failed: label = retained ? "재전사 실패 · 이전 결과 유지" : "전사 실패"
            case .cancelled: label = retained ? "재전사 중단 · 이전 결과 유지" : "전사 중단"
            case .interrupted: label = hasResult ? "처리 중단 · 저장된 결과 유지" : "처리 중단"
            case .completed: label = "전사 완료"
            }
            symbol = outcome.kind == .failed ? "exclamationmark.circle" : "pause.circle"
            isFailure = outcome.kind == .failed
            detail = outcome.message
            canRetry = hasAudio
        } else if meeting.status == .failed {
            label = hasResult ? "이전 처리 실패 · 전사 보존" : "처리 실패"
            symbol = "exclamationmark.circle"
            isFailure = true
            detail = meeting.errorMessage
            canRetry = hasAudio
        } else if meeting.status == .ready || meeting.processingOutcome?.kind == .completed
                    || (meeting.status == .needsReview && hasResult && meeting.errorMessage == nil) {
            label = "전사 완료"
            symbol = "checkmark.circle"
            isFailure = false
            detail = meeting.processingOutcome == nil ? meeting.errorMessage.map { "이전 처리 안내: \($0)" } : nil
            canRetry = false
        } else {
            label = hasResult ? "전사 결과 있음" : "이전 처리 확인"
            symbol = "info.circle"
            isFailure = false
            detail = meeting.errorMessage.map { "이전 처리 안내: \($0)" }
            canRetry = hasAudio && !hasResult
        }
    }
}

extension MeetingRecord {
    func validateProcessingMetadata() throws {
        if let result = completedResult {
            guard !result.speakerCounts.isEmpty,
                  Set(result.speakerCounts.map(\.sourceID)).count == result.speakerCounts.count,
                  result.speakerCounts.allSatisfy({
                      !$0.sourceID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && $0.detected >= 0
                          && ($0.expected.map { $0 > 0 } ?? true)
                  }) else { throw TranscriptEditError.invalidDocument }
        }
        if let outcome = processingOutcome {
            guard outcome.recordedAt.timeIntervalSince1970.isFinite,
                  outcome.kind != .completed || !outcome.previousResultRetained else { throw TranscriptEditError.invalidDocument }
            if outcome.kind == .completed, let revision = outcome.retainedRevisionID, let result = completedResult {
                guard revision == result.revisionID else { throw TranscriptEditError.invalidDocument }
            }
        }
    }
}
