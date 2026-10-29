import Foundation
import GroveInference

struct MeetingSpeaker: Identifiable, Codable, Hashable, Sendable {
    var id: UUID = UUID()
    var name: String
    var order: Int
    var profileMatch: SpeakerProfileMatch? = nil

    static func defaultName(order: Int) -> String { "화자 \(order + 1)" }
    var hasDefaultName: Bool { name == Self.defaultName(order: order) }
}

struct DocumentUtterance: Identifiable, Codable, Hashable, Sendable {
    let id: UUID
    let startTime: TimeInterval
    let endTime: TimeInterval
    let rawText: String
    let sourceChannelID: String?
    let engineClusterID: String?
    var speakerID: UUID?
    var editedText: String?
    var asrClusterID: String? = nil
    var asrChunkIndex: Int? = nil
    var assignmentReviewReasons: Set<AssignmentReviewReason>? = nil
    var parentUtteranceID: UUID? = nil
    var assignmentEvidence: SpeakerAssignmentEvidence? = nil
    var speakerReviewEvidence: SpeakerReviewEvidence? = nil

    var displayedText: String { editedText ?? rawText }
}

enum SpeakerEditScope: Sendable {
    case utterance
    case followingSameSpeaker
    case allSameSpeaker
    case selected(Set<UUID>)
}

enum SpeakerEditTarget: Sendable {
    case existing(UUID)
    case new(String)
}

enum TranscriptEditError: Error, LocalizedError {
    case missingUtterance
    case missingSpeaker
    case emptyName
    case emptyText
    case invalidDocument
    case invalidSplit
    case invalidReviewScope

    var errorDescription: String? {
        switch self {
        case .missingUtterance: "변경할 발화를 찾을 수 없습니다."
        case .missingSpeaker: "변경할 화자를 찾을 수 없습니다."
        case .emptyName: "화자 이름을 입력해 주세요."
        case .emptyText: "발화 내용을 입력해 주세요. 원문은 삭제되지 않았습니다."
        case .invalidDocument: "전사 문서의 식별자나 시각이 올바르지 않습니다."
        case .invalidSplit: "발화 안의 분할 시각과 앞뒤 내용을 모두 지정해 주세요."
        case .invalidReviewScope: "화자 변경과 확인은 한 발화에만 적용할 수 있습니다."
        }
    }
}

struct SpeakerAssignmentDelta: Codable, Hashable, Sendable {
    let utteranceID: UUID
    let before: UUID?
    let after: UUID?
}

enum TranscriptMutation: Codable, Hashable, Sendable {
    case assignments([SpeakerAssignmentDelta])
    case speakerName(id: UUID, before: String, after: String)
    case speakerProfile(id: UUID, before: SpeakerProfileMatch?, after: SpeakerProfileMatch?)
    case text(id: UUID, before: String?, after: String?)
    case speakerReview(id: UUID, before: SpeakerReviewEvidence?, after: SpeakerReviewEvidence?)
    case createdSpeaker(MeetingSpeaker)
    case split(before: DocumentUtterance, children: [DocumentUtterance], index: Int)
}

struct TranscriptChange: Identifiable, Codable, Hashable, Sendable {
    var id: UUID = UUID()
    var createdAt: Date = Date()
    let label: String
    let mutations: [TranscriptMutation]
}

struct TranscriptDocument: Codable, Hashable, Sendable {
    private(set) var schemaVersion: Int
    let revisionID: UUID
    let sourceDiarizationEngines: [String: DiarizationEngine]?
    private(set) var speakers: [MeetingSpeaker]
    private(set) var utterances: [DocumentUtterance]
    private(set) var undoHistory: [TranscriptChange]
    private(set) var redoHistory: [TranscriptChange]

    init(speakers: [MeetingSpeaker], utterances: [DocumentUtterance], revisionID: UUID = UUID(),
         sourceDiarizationEngines: [String: DiarizationEngine]? = nil) throws {
        schemaVersion = 5
        self.revisionID = revisionID
        self.sourceDiarizationEngines = sourceDiarizationEngines
        self.speakers = speakers
        self.utterances = utterances
        undoHistory = []
        redoHistory = []
        try validate()
    }

    func validate() throws {
        let speakerIDs = Set(speakers.map(\.id))
        guard (3...5).contains(schemaVersion),
              speakerIDs.count == speakers.count,
              Set(utterances.map(\.id)).count == utterances.count,
              speakers.allSatisfy({ speaker in
                  guard !speaker.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
                  guard let match = speaker.profileMatch else { return true }
                  return match.similarity.map { $0.isFinite && (-1...1).contains($0) } ?? match.isConfirmed
              }),
              utterances.allSatisfy({
                  $0.startTime.isFinite && $0.endTime.isFinite && $0.startTime >= 0 && $0.endTime >= $0.startTime
                      && ($0.speakerID.map { speakerIDs.contains($0) } ?? true)
                      && ($0.assignmentEvidence?.isValid ?? true) && ($0.speakerReviewEvidence?.isValid ?? true)
                      && ($0.parentUtteranceID == nil || ($0.parentUtteranceID != $0.id && !($0.editedText ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty))
              }) else { throw TranscriptEditError.invalidDocument }
    }

    func speakerName(for utterance: DocumentUtterance) -> String {
        if let speaker = speakers.first(where: { $0.id == utterance.speakerID }) { return speaker.name }
        if let source = utterance.sourceChannelID, sourceDiarizationEngines?[source] == DiarizationEngine.none { return "화자 구분 없음" }
        return "화자 미확정"
    }

    func reassignmentIDs(from utteranceID: UUID, scope: SpeakerEditScope) throws -> Set<UUID> {
        guard let anchor = utterances.first(where: { $0.id == utteranceID }) else {
            throw TranscriptEditError.missingUtterance
        }
        switch scope {
        case .utterance:
            return [utteranceID]
        case .followingSameSpeaker:
            return Set(utterances.filter { $0.speakerID == anchor.speakerID && $0.startTime >= anchor.startTime }.map(\.id))
        case .allSameSpeaker:
            return Set(utterances.filter { $0.speakerID == anchor.speakerID }.map(\.id))
        case .selected(let ids):
            guard !ids.isEmpty, ids.isSubset(of: Set(utterances.map(\.id))) else {
                throw TranscriptEditError.missingUtterance
            }
            return ids
        }
    }

    @discardableResult
    mutating func reassign(from utteranceID: UUID, to target: SpeakerEditTarget, scope: SpeakerEditScope,
                          confirmingAnchor: Bool = false) throws -> Int {
        if confirmingAnchor {
            guard case .utterance = scope else { throw TranscriptEditError.invalidReviewScope }
        }
        let ids = try reassignmentIDs(from: utteranceID, scope: scope)
        var mutations: [TranscriptMutation] = []
        let targetID: UUID
        switch target {
        case .existing(let id):
            guard speakers.contains(where: { $0.id == id }) else { throw TranscriptEditError.missingSpeaker }
            targetID = id
        case .new(let name):
            let cleaned = name.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !cleaned.isEmpty else { throw TranscriptEditError.emptyName }
            let speaker = MeetingSpeaker(name: cleaned, order: (speakers.map(\.order).max() ?? -1) + 1)
            targetID = speaker.id
            mutations.append(.createdSpeaker(speaker))
        }
        let changes = utterances.filter { ids.contains($0.id) && $0.speakerID != targetID }.map {
            SpeakerAssignmentDelta(utteranceID: $0.id, before: $0.speakerID, after: targetID)
        }
        if !changes.isEmpty { mutations.append(.assignments(changes)) }
        if confirmingAnchor, let original = utterances.first(where: { $0.id == utteranceID }) {
            var corrected = original
            corrected.speakerID = targetID
            let binding = SpeakerReviewBinding(utterance: corrected, speakerID: targetID, documentRevisionID: revisionID)
            if original.speakerReviewEvidence?.binding != binding || original.speakerReviewEvidence?.isValid != true {
                let evidence = SpeakerReviewEvidence(binding: binding,
                    action: changes.isEmpty ? .confirmedCurrentSpeaker : .confirmedAssignmentChange)
                mutations.append(.speakerReview(id: original.id, before: original.speakerReviewEvidence, after: evidence))
            }
        }
        guard !mutations.isEmpty else { return 0 }
        let label = confirmingAnchor ? "발화 화자 변경 및 확인" : "화자 변경 \(changes.count)개"
        try commit(TranscriptChange(label: label, mutations: mutations))
        return changes.count
    }

    mutating func confirmUtteranceSpeaker(_ id: UUID, reviewedAt: Date = Date()) throws {
        guard let utterance = utterances.first(where: { $0.id == id }) else { throw TranscriptEditError.missingUtterance }
        guard let speakerID = utterance.speakerID, speakers.contains(where: { $0.id == speakerID }) else {
            throw TranscriptEditError.missingSpeaker
        }
        guard !speakerReview(for: utterance).isConfirmed else { return }
        let evidence = SpeakerReviewEvidence(
            binding: .init(utterance: utterance, speakerID: speakerID, documentRevisionID: revisionID),
            action: .confirmedCurrentSpeaker, reviewedAt: reviewedAt)
        try commit(TranscriptChange(createdAt: reviewedAt, label: "발화 화자 확인", mutations: [
            .speakerReview(id: id, before: utterance.speakerReviewEvidence, after: evidence)
        ]))
    }

    mutating func renameSpeaker(_ id: UUID, to name: String) throws {
        let cleaned = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleaned.isEmpty else { throw TranscriptEditError.emptyName }
        guard let speaker = speakers.first(where: { $0.id == id }) else { throw TranscriptEditError.missingSpeaker }
        guard speaker.name != cleaned || speaker.profileMatch != nil else { return }
        try commit(TranscriptChange(label: "화자 이름 변경", mutations: [
            .speakerName(id: id, before: speaker.name, after: cleaned),
            .speakerProfile(id: id, before: speaker.profileMatch, after: nil)
        ]))
    }

    mutating func applySpeakerProfile(_ profile: SavedSpeakerProfile, to id: UUID, similarity: Float?, confirmed: Bool,
                                     identityEvidence: SpeakerIdentityEvidence? = nil) throws {
        guard let speaker = speakers.first(where: { $0.id == id }) else { throw TranscriptEditError.missingSpeaker }
        let match = SpeakerProfileMatch(folderID: profile.folderID, profileID: profile.id, similarity: similarity,
                                        isConfirmed: confirmed, originalName: speaker.profileMatch?.originalName ?? speaker.name,
                                        identityEvidence: identityEvidence)
        try commit(TranscriptChange(label: confirmed ? "저장한 화자 적용" : "저장한 목소리로 이름 추정", mutations: [
            .speakerName(id: id, before: speaker.name, after: profile.name),
            .speakerProfile(id: id, before: speaker.profileMatch, after: match)
        ]))
    }

    mutating func confirmSpeakerIdentity(_ id: UUID) throws {
        guard let speaker = speakers.first(where: { $0.id == id }), let old = speaker.profileMatch else {
            throw TranscriptEditError.missingSpeaker
        }
        guard !old.isConfirmed else { return }
        let confirmed = SpeakerProfileMatch(folderID: old.folderID, profileID: old.profileID,
            similarity: old.similarity, isConfirmed: true, originalName: old.originalName, identityEvidence: old.identityEvidence)
        try commit(TranscriptChange(label: "추정한 화자 이름 확인", mutations: [
            .speakerProfile(id: id, before: old, after: confirmed)
        ]))
    }

    mutating func rejectSpeakerIdentity(_ id: UUID) throws {
        guard let speaker = speakers.first(where: { $0.id == id }), let old = speaker.profileMatch,
              !old.isConfirmed else { throw TranscriptEditError.missingSpeaker }
        try commit(TranscriptChange(label: "추정한 화자 이름 해제", mutations: [
            .speakerName(id: id, before: speaker.name, after: old.originalName ?? MeetingSpeaker.defaultName(order: speaker.order)),
            .speakerProfile(id: id, before: old, after: nil)
        ]))
    }

    mutating func editText(_ id: UUID, to text: String) throws {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw TranscriptEditError.emptyText }
        guard let utterance = utterances.first(where: { $0.id == id }) else { throw TranscriptEditError.missingUtterance }
        let edited = text == utterance.rawText && utterance.parentUtteranceID == nil ? nil : text
        guard utterance.editedText != edited else { return }
        try commit(TranscriptChange(label: "발화 내용 수정", mutations: [.text(id: id, before: utterance.editedText, after: edited)]))
    }

    mutating func splitUtterance(_ id: UUID, at time: Double, firstText: String, secondText: String) throws {
        guard let index = utterances.firstIndex(where: { $0.id == id }) else { throw TranscriptEditError.missingUtterance }
        let original = utterances[index]
        guard time.isFinite, original.startTime < time, time < original.endTime,
              !firstText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !secondText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw TranscriptEditError.invalidSplit }
        func child(start: Double, end: Double, text: String) -> DocumentUtterance {
            DocumentUtterance(id: UUID(), startTime: start, endTime: end, rawText: original.rawText,
                sourceChannelID: original.sourceChannelID, engineClusterID: original.engineClusterID,
                speakerID: original.speakerID, editedText: text, asrClusterID: original.asrClusterID,
                asrChunkIndex: original.asrChunkIndex, assignmentReviewReasons: original.assignmentReviewReasons,
                parentUtteranceID: original.parentUtteranceID ?? original.id)
        }
        let children = [child(start: original.startTime, end: time, text: firstText),
                        child(start: time, end: original.endTime, text: secondText)]
        try commit(TranscriptChange(label: "발화 분할", mutations: [.split(before: original, children: children, index: index)]))
    }

    mutating func undo() throws {
        guard let change = undoHistory.last else { return }
        var copy = self
        copy.schemaVersion = 5
        try copy.apply(change, forward: false)
        try copy.validate()
        copy.undoHistory.removeLast()
        copy.redoHistory.append(change)
        self = copy
    }

    mutating func redo() throws {
        guard let change = redoHistory.last else { return }
        var copy = self
        copy.schemaVersion = 5
        try copy.apply(change, forward: true)
        try copy.validate()
        copy.redoHistory.removeLast()
        copy.undoHistory.append(change)
        self = copy
    }

    private mutating func commit(_ change: TranscriptChange) throws {
        var copy = self
        copy.schemaVersion = 5
        try copy.apply(change, forward: true)
        try copy.validate()
        copy.undoHistory.append(change)
        copy.redoHistory.removeAll()
        self = copy
    }

    private mutating func apply(_ change: TranscriptChange, forward: Bool) throws {
        let mutations = forward ? change.mutations : Array(change.mutations.reversed())
        for mutation in mutations {
            switch mutation {
            case .assignments(let changes):
                for delta in changes {
                    guard let index = utterances.firstIndex(where: { $0.id == delta.utteranceID }) else {
                        throw TranscriptEditError.missingUtterance
                    }
                    utterances[index].speakerID = forward ? delta.after : delta.before
                }
            case .speakerName(let id, let before, let after):
                guard let index = speakers.firstIndex(where: { $0.id == id }) else { throw TranscriptEditError.missingSpeaker }
                speakers[index].name = forward ? after : before
            case .speakerProfile(let id, let before, let after):
                guard let index = speakers.firstIndex(where: { $0.id == id }) else { throw TranscriptEditError.missingSpeaker }
                speakers[index].profileMatch = forward ? after : before
            case .text(let id, let before, let after):
                guard let index = utterances.firstIndex(where: { $0.id == id }) else { throw TranscriptEditError.missingUtterance }
                utterances[index].editedText = forward ? after : before
            case .speakerReview(let id, let before, let after):
                guard let index = utterances.firstIndex(where: { $0.id == id }) else { throw TranscriptEditError.missingUtterance }
                utterances[index].speakerReviewEvidence = forward ? after : before
            case .createdSpeaker(let speaker):
                if forward { speakers.append(speaker) } else { speakers.removeAll { $0.id == speaker.id } }
            case .split(let before, let children, let index):
                guard children.count == 2, index >= 0 else { throw TranscriptEditError.invalidDocument }
                if forward {
                    guard utterances.indices.contains(index), utterances[index].id == before.id else {
                        throw TranscriptEditError.missingUtterance
                    }
                    utterances.replaceSubrange(index...index, with: children)
                } else {
                    guard index <= utterances.count - children.count,
                          Array(utterances[index..<(index + children.count)]).map(\.id) == children.map(\.id) else {
                        throw TranscriptEditError.missingUtterance
                    }
                    utterances.replaceSubrange(index..<(index + children.count), with: [before])
                }
            }
        }
    }
}
