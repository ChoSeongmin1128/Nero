import CryptoKit
import Foundation
import GroveInference

enum VoiceIdentityReleaseGate {
    // Integration and synthetic safety tests are not evidence of open-set accuracy.
    // Keep meeting-derived enrollment and automatic naming off until a recipe passes
    // independent-session known/unknown validation and its storage release gates.
    static let isEnabled = false
    static let message = "자동 이름 연결은 정확도 검증 중입니다."
}

protocol SpeakerVoiceExtracting: Sendable {
    func extractSamples(source: URL, ranges: [VoiceSampleRange], workingDirectory: URL) async throws -> [VoiceEmbeddingSample]
    /// The identifier new samples would carry, or nil when it cannot be determined.
    func currentModelIdentifier() throws -> String?
}

extension SpeakerVoiceExtracting {
    func currentModelIdentifier() throws -> String? { nil }
}

struct LocalSpeakerVoiceService: SpeakerVoiceExtracting {
    let modelDirectory: URL
    var recipe: VoiceEmbeddingRecipe = .activeFrameCenteredV2

    func extractSamples(source: URL, ranges: [VoiceSampleRange], workingDirectory: URL) async throws -> [VoiceEmbeddingSample] {
        for name in ["FBank.mlmodelc", "Embedding.mlmodelc"] {
            guard FileManager.default.isReadableFile(atPath: modelDirectory.appendingPathComponent(name).path) else {
                throw VoiceIdentityError.modelsUnavailable
            }
        }
        return try await VoiceEmbeddingExtractor(modelDirectory: modelDirectory, recipe: recipe)
            .extractSamples(source: source, ranges: ranges, workingDirectory: workingDirectory)
    }

    func currentModelIdentifier() throws -> String? {
        guard FileManager.default.fileExists(atPath: modelDirectory.appendingPathComponent("Embedding.mlmodelc").path) else { return nil }
        return try VoiceEmbeddingExtractor.modelFingerprint(directory: modelDirectory, recipe: recipe)
    }
}

enum VoiceIdentityError: LocalizedError {
    case consentRequired, insufficientSelection, invalidSelection, changed, missingSource
    case modelsUnavailable, namesOnlyConflict, noEnrolledVoices, sourceEnrollmentOnly

    var errorDescription: String? {
        switch self {
        case .consentRequired: "선택한 발화가 같은 사람의 목소리인지, 목소리를 등록할 권한이 있는지 확인해 주세요."
        case .insufficientSelection: "단독 발화를 3개 이상, 실제 사용할 음성이 합계 10초 이상이 되도록 선택해 주세요."
        case .invalidSelection: "선택한 발화에 다른 화자의 활동이 있거나 유효한 음성 근거가 부족합니다. 발화를 확인한 뒤 다시 선택해 주세요."
        case .changed: "처리 중 녹음·화자·폴더 또는 등록 정보가 바뀌어 적용하지 않았습니다. 다시 시도해 주세요."
        case .missingSource: "목소리 등록·식별에는 단일 원본 녹음과 화자 활동이 있는 전사가 필요합니다."
        case .modelsUnavailable: "로컬 목소리 특징 모델이 준비되지 않았습니다. 이름만 저장하는 기능은 계속 사용할 수 있습니다."
        case .namesOnlyConflict: "이미 저장한 화자와 이름이 다릅니다. 이름이나 연결을 먼저 수정한 뒤 등록해 주세요."
        case .noEnrolledVoices: "이 폴더에 등록한 목소리가 없습니다. 화자 목록에서 목소리를 먼저 등록해 주세요."
        case .sourceEnrollmentOnly: "이 녹음으로 등록한 목소리는 같은 녹음의 자동 식별 비교에서 제외합니다. 다른 회의에서 확인해 주세요."
        }
    }
}

enum VoiceIdentitySelection {
    static func range(for utterance: DocumentUtterance) -> VoiceSampleRange {
        .init(start: utterance.startTime + 0.15, end: min(utterance.endTime - 0.15, utterance.startTime + 10.15))
    }

    static func candidates(in document: TranscriptDocument, speakerID: UUID) -> [DocumentUtterance] {
        document.utterances.filter { utterance in
            guard utterance.speakerID == speakerID, utterance.parentUtteranceID == nil,
                  utterance.sourceChannelID == "recording", utterance.endTime - utterance.startTime >= 2.3,
                  let evidence = utterance.assignmentEvidence, evidence.applies(to: utterance),
                  let cluster = utterance.engineClusterID,
                  evidence.overlapSecondsByCluster[cluster, default: 0] >= (utterance.endTime - utterance.startTime) * 0.8,
                  !evidence.overlapSecondsByCluster.contains(where: { $0.key != cluster && $0.value > 0 }),
                  (utterance.assignmentReviewReasons ?? []).isEmpty else { return false }
            let selected = range(for: utterance)
            return !document.utterances.contains { other in
                other.id != utterance.id && other.startTime < selected.end && other.endTime > selected.start
            }
        }.sorted { $0.startTime < $1.startTime }
    }

    static func selected(in document: TranscriptDocument, speakerID: UUID, ids: Set<UUID>) throws -> [DocumentUtterance] {
        let candidates = candidates(in: document, speakerID: speakerID)
        guard ids.isSubset(of: Set(candidates.map(\.id))) else { throw VoiceIdentityError.invalidSelection }
        let chosen = candidates.filter { ids.contains($0.id) }
        guard (3...5).contains(chosen.count), chosen.reduce(0, { $0 + range(for: $1).end - range(for: $1).start }) >= 10 else {
            throw VoiceIdentityError.insufficientSelection
        }
        return chosen
    }

    static func query(in document: TranscriptDocument, speakerID: UUID) -> [DocumentUtterance] {
        Array(candidates(in: document, speakerID: speakerID).sorted {
            $0.endTime - $0.startTime > $1.endTime - $1.startTime
        }.prefix(5)).sorted { $0.startTime < $1.startTime }
    }

    static func permitsAutomaticName(_ speaker: MeetingSpeaker, in document: TranscriptDocument) -> Bool {
        guard speaker.profileMatch == nil, speaker.hasDefaultName else { return false }
        for change in document.undoHistory + document.redoHistory {
            for mutation in change.mutations {
                switch mutation {
                case .speakerName(let id, _, _), .speakerProfile(let id, _, _): if id == speaker.id { return false }
                case .createdSpeaker(let created): if created.id == speaker.id { return false }
                case .assignments(let changes): if changes.contains(where: { $0.before == speaker.id || $0.after == speaker.id }) { return false }
                default: break
                }
            }
        }
        return true
    }

    static func conflictingAssignments(_ assignments: [UUID: UUID], in document: TranscriptDocument) -> Set<UUID> {
        var rejected = Set<UUID>()
        for group in Dictionary(grouping: assignments.keys, by: { assignments[$0]! }).values where group.count > 1 {
            let ids = Set(group)
            let utterances = document.utterances.filter { $0.speakerID.map(ids.contains) == true }
            let clusters = Set(utterances.compactMap(\.engineClusterID))
            let activityConflict = utterances.contains { utterance in
                utterance.assignmentEvidence?.overlapSecondsByCluster.contains(where: {
                    $0.key != utterance.engineClusterID && clusters.contains($0.key) && $0.value > 0
                }) == true
            }
            let timeConflict = utterances.contains { first in
                utterances.contains { second in
                    first.speakerID != second.speakerID && first.sourceChannelID == second.sourceChannelID
                        && first.startTime < second.endTime && second.startTime < first.endTime
                }
            }
            if activityConflict || timeConflict { rejected.formUnion(ids) }
        }
        return rejected
    }

    static func audioHash(_ url: URL) async throws -> String {
        let task = Task.detached(priority: .utility) {
            guard url.isFileURL else { throw VoiceIdentityError.missingSource }
            let handle = try FileHandle(forReadingFrom: url)
            defer { try? handle.close() }
            var hash = SHA256()
            while true {
                try Task.checkCancellation()
                guard let bytes = try handle.read(upToCount: 1_048_576), !bytes.isEmpty else { break }
                hash.update(data: bytes)
            }
            return hash.finalize().map { String(format: "%02x", $0) }.joined()
        }
        return try await withTaskCancellationHandler(operation: { try await task.value }, onCancel: { task.cancel() })
    }
}
