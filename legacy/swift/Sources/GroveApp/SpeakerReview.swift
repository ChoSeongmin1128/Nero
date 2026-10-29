import Foundation
import GroveInference

// These are immutable machine-assignment measurements, not a probability or a
// human verdict. Their time/source binding prevents reuse after a split/edit.
struct SpeakerAssignmentEvidence: Codable, Hashable, Sendable {
    let version: Int
    let utteranceID: UUID
    let startTime: TimeInterval
    let endTime: TimeInterval
    let sourceChannelID: String?
    let assignedClusterID: String?
    let overlapSecondsByCluster: [String: Double]

    init(utteranceID: UUID, startTime: TimeInterval, endTime: TimeInterval,
         sourceChannelID: String?, assignedClusterID: String?, overlapSecondsByCluster: [String: Double]) {
        version = 1
        self.utteranceID = utteranceID
        self.startTime = startTime
        self.endTime = endTime
        self.sourceChannelID = sourceChannelID
        self.assignedClusterID = assignedClusterID
        self.overlapSecondsByCluster = overlapSecondsByCluster
    }

    var isValid: Bool {
        version == 1 && startTime.isFinite && endTime.isFinite && startTime >= 0 && endTime >= startTime
            && overlapSecondsByCluster.allSatisfy { key, value in
                !key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    && value.isFinite && value >= 0 && value <= endTime - startTime + 0.000001
            }
    }

    func applies(to utterance: DocumentUtterance) -> Bool {
        isValid && utteranceID == utterance.id && startTime == utterance.startTime && endTime == utterance.endTime
            && sourceChannelID == utterance.sourceChannelID && assignedClusterID == utterance.engineClusterID
    }
}

struct SpeakerReviewBinding: Codable, Hashable, Sendable {
    let documentRevisionID: UUID
    let utteranceID: UUID
    let speakerID: UUID
    let text: String
    let startTime: TimeInterval
    let endTime: TimeInterval
    let sourceChannelID: String?
    let engineClusterID: String?
    let assignmentReviewReasons: Set<AssignmentReviewReason>?
    let assignmentEvidence: SpeakerAssignmentEvidence?
    let rulesVersion: String

    init(utterance: DocumentUtterance, speakerID: UUID, documentRevisionID: UUID,
         rulesVersion: String = SpeakerReviewPolicy.rulesVersion) {
        self.documentRevisionID = documentRevisionID
        utteranceID = utterance.id
        self.speakerID = speakerID
        text = utterance.displayedText
        startTime = utterance.startTime
        endTime = utterance.endTime
        sourceChannelID = utterance.sourceChannelID
        engineClusterID = utterance.engineClusterID
        assignmentReviewReasons = utterance.assignmentReviewReasons
        assignmentEvidence = utterance.assignmentEvidence
        self.rulesVersion = rulesVersion
    }

    var isValid: Bool {
        startTime.isFinite && endTime.isFinite && startTime >= 0 && endTime >= startTime
            && !rulesVersion.isEmpty && (assignmentEvidence?.isValid ?? true)
    }
}

struct SpeakerReviewEvidence: Identifiable, Codable, Hashable, Sendable {
    enum Action: String, Codable, Hashable, Sendable {
        case confirmedCurrentSpeaker
        case confirmedAssignmentChange
    }

    let id: UUID
    let reviewedAt: Date
    let action: Action
    let binding: SpeakerReviewBinding

    init(binding: SpeakerReviewBinding, action: Action, reviewedAt: Date = Date()) {
        id = UUID()
        self.reviewedAt = reviewedAt
        self.action = action
        self.binding = binding
    }

    var isValid: Bool { reviewedAt.timeIntervalSinceReferenceDate.isFinite && binding.isValid }
}

enum SpeakerReviewReason: String, Hashable, Sendable {
    case unresolvedSpeaker, noDiarizationCoverage, ambiguousSpeakers
    case lowCoverage, competingSpeakers, changedSinceConfirmation, reviewRulesChanged
    case staleAssignmentEvidence, splitUtterance

    var message: String {
        switch self {
        case .unresolvedSpeaker: "화자가 지정되지 않았습니다."
        case .noDiarizationCoverage: "발화 구간에서 화자 활동을 찾지 못했습니다."
        case .ambiguousSpeakers: "화자 후보를 하나로 정하지 못했습니다."
        case .lowCoverage: "자동 배정 근거가 되는 화자 활동이 발화 구간의 절반보다 짧습니다."
        case .competingSpeakers: "다른 화자 후보의 활동 길이가 비슷합니다."
        case .changedSinceConfirmation: "확인 이후 발화 내용·시각·화자 또는 출처가 변경되었습니다."
        case .reviewRulesChanged: "화자 확인 기준이 변경되어 다시 확인이 필요합니다."
        case .staleAssignmentEvidence: "현재 구간에 맞는 화자 활동 근거가 없습니다."
        case .splitUtterance: "분할한 발화의 화자를 확인해 주세요."
        }
    }
}

struct SpeakerReviewAssessment: Equatable, Sendable {
    let isConfirmed: Bool
    let canConfirm: Bool
    let reasons: [SpeakerReviewReason]
    let hasRawAssignmentEvidence: Bool
    var needsReview: Bool { !isConfirmed && !reasons.isEmpty }
}

enum SpeakerReviewPolicy {
    static let rulesVersion = "speaker-assignment-review-v1"

    // UI triage heuristics, NOT calibrated accuracy/confidence thresholds:
    // retain low winner coverage (< 50%) or a near competitor (>= 80% of
    // the winner). A tiny secondary interval alone is not a warning.
    static func assess(_ utterance: DocumentUtterance, documentRevisionID: UUID, speakerExists: Bool,
                       rulesVersion: String = SpeakerReviewPolicy.rulesVersion) -> SpeakerReviewAssessment {
        let rawEvidence = utterance.assignmentEvidence.flatMap { $0.applies(to: utterance) ? $0 : nil }
        let currentBinding = utterance.speakerID.map {
            SpeakerReviewBinding(utterance: utterance, speakerID: $0, documentRevisionID: documentRevisionID,
                                 rulesVersion: rulesVersion)
        }
        if speakerExists, let evidence = utterance.speakerReviewEvidence,
           evidence.isValid, evidence.binding == currentBinding {
            return .init(isConfirmed: true, canConfirm: true, reasons: [], hasRawAssignmentEvidence: rawEvidence != nil)
        }
        var reasons: [SpeakerReviewReason] = []
        if !speakerExists { reasons.append(.unresolvedSpeaker) }
        if let evidence = utterance.speakerReviewEvidence {
            reasons.append(evidence.binding.rulesVersion == rulesVersion ? .changedSinceConfirmation : .reviewRulesChanged)
        }
        let rawReasons = utterance.assignmentReviewReasons ?? []
        if rawReasons.contains(.noDiarizationCoverage) { reasons.append(.noDiarizationCoverage) }
        if rawReasons.contains(.ambiguousSpeakers) { reasons.append(.ambiguousSpeakers) }
        if utterance.parentUtteranceID != nil { reasons.append(.splitUtterance) }
        if utterance.assignmentEvidence != nil && rawEvidence == nil { reasons.append(.staleAssignmentEvidence) }
        if let rawEvidence, utterance.endTime > utterance.startTime {
            let ranked = rawEvidence.overlapSecondsByCluster.values.filter { $0 > 0 }.sorted(by: >)
            if let winner = ranked.first {
                if winner / (utterance.endTime - utterance.startTime) < 0.5 { reasons.append(.lowCoverage) }
                if ranked.count > 1 && ranked[1] / winner >= 0.8 { reasons.append(.competingSpeakers) }
            } else if !reasons.contains(.noDiarizationCoverage) {
                reasons.append(.noDiarizationCoverage)
            }
        }
        // Legacy multipleSpeakers-only flags have no overlap measurements.
        // Suppressing that coarse UI warning does not verify the assignment.
        return .init(isConfirmed: false, canConfirm: speakerExists, reasons: reasons,
                     hasRawAssignmentEvidence: rawEvidence != nil)
    }
}

extension TranscriptDocument {
    var speakerReviewCount: Int { utterances.filter { speakerReview(for: $0).needsReview }.count }

    func speakerReview(for utterance: DocumentUtterance) -> SpeakerReviewAssessment {
        if utterance.speakerID == nil, let source = utterance.sourceChannelID, sourceDiarizationEngines?[source] == DiarizationEngine.none {
            return .init(isConfirmed: false, canConfirm: false, reasons: [], hasRawAssignmentEvidence: false)
        }
        return SpeakerReviewPolicy.assess(utterance, documentRevisionID: revisionID,
            speakerExists: utterance.speakerID.map { id in speakers.contains { $0.id == id } } ?? false)
    }

    func speakerReview(for utteranceID: UUID) -> SpeakerReviewAssessment? {
        utterances.first(where: { $0.id == utteranceID }).map { speakerReview(for: $0) }
    }
}
