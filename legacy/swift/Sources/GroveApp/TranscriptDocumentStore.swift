import Foundation
import GroveInference

@MainActor
final class TranscriptDocumentStore: ObservableObject {
    @Published private(set) var documents: [UUID: TranscriptDocument] = [:]
    @Published private(set) var errors: [UUID: String] = [:]
    var onError: ((String) -> Void)?
    private let storage: TranscriptDocumentStorage
    private let persists: Bool

    init(directory: URL, persists: Bool) {
        storage = TranscriptDocumentStorage(directory: directory)
        self.persists = persists
    }

    func load(_ meetings: [MeetingRecord]) {
        for meeting in meetings {
            do {
                if persists, let document = try storage.load(for: meeting.id) {
                    documents[meeting.id] = document
                } else if !meeting.transcript.isEmpty {
                    documents[meeting.id] = try TranscriptDocument.preservingLegacy(meeting.transcript)
                }
            } catch {
                errors[meeting.id] = error.localizedDescription
            }
        }
    }

    func storedRevisionID(for meetingID: UUID) -> UUID? {
        (try? storage.load(for: meetingID))?.revisionID
    }

    func updateUtteranceText(meetingID: UUID, utteranceID: UUID, text: String) -> Bool {
        edit(meetingID: meetingID) { try $0.editText(utteranceID, to: text) }
    }

    func confirmUtteranceSpeaker(meetingID: UUID, utteranceID: UUID) -> Bool {
        edit(meetingID: meetingID) { try $0.confirmUtteranceSpeaker(utteranceID) }
    }

    func speakerReview(meetingID: UUID, utteranceID: UUID) -> SpeakerReviewAssessment? {
        documents[meetingID]?.speakerReview(for: utteranceID)
    }

    func renameSpeaker(meetingID: UUID, speakerID: UUID, name: String) -> Bool {
        edit(meetingID: meetingID) { try $0.renameSpeaker(speakerID, to: name) }
    }

    func splitUtterance(meetingID: UUID, utteranceID: UUID, time: Double, firstText: String, secondText: String) -> Bool {
        edit(meetingID: meetingID) {
            try $0.splitUtterance(utteranceID, at: time, firstText: firstText, secondText: secondText)
        }
    }

    func reassignSpeaker(meetingID: UUID, utteranceID: UUID, target: SpeakerEditTarget, scope: SpeakerEditScope,
                         confirmingAnchor: Bool = false) -> Bool {
        edit(meetingID: meetingID) {
            _ = try $0.reassign(from: utteranceID, to: target, scope: scope, confirmingAnchor: confirmingAnchor)
        }
    }

    func undo(meetingID: UUID) {
        _ = edit(meetingID: meetingID) { try $0.undo() }
    }

    func redo(meetingID: UUID) {
        _ = edit(meetingID: meetingID) { try $0.redo() }
    }

    func previousTranscripts(meetingID: UUID) throws -> [ArchivedTranscript] {
        guard persists else { return [] }
        return try storage.archives(for: meetingID)
    }

    func restore(_ archive: ArchivedTranscript, meetingID: UUID) -> Bool {
        guard archive.meetingID == meetingID else { return false }
        do {
            try storage.activate(archive.document, for: meetingID)
            documents[meetingID] = archive.document
            errors.removeValue(forKey: meetingID)
            return true
        } catch {
            onError?("전사를 전환하지 못했습니다. 현재 내용은 보존됩니다. \(error.localizedDescription)")
            return false
        }
    }

    func accept(_ result: InferenceResult, meetingID: UUID, sourceChannelID: String) throws {
        let document = try TranscriptDocument.preservingInference(result, sourceChannelID: sourceChannelID)
        if persists { try storage.activate(document, for: meetingID, rawResult: result) }
        publish(document, for: meetingID)
    }

    @discardableResult
    func edit(meetingID: UUID, mutation: (inout TranscriptDocument) throws -> Void) -> Bool {
        guard var document = documents[meetingID], errors[meetingID] == nil else { return false }
        do {
            try mutation(&document)
            if persists { try storage.save(document, for: meetingID) }
            documents[meetingID] = document
            return true
        } catch {
            onError?("변경을 저장하지 못했습니다. 기존 내용은 보존됩니다. \(error.localizedDescription)")
            return false
        }
    }

    // Processing writes the new revision to disk first and publishes it only after the
    // meeting index accepted the result; `rollback` restores the previous active revision.
    func preserveRawResult(_ result: InferenceResult, for meetingID: UUID) throws {
        if persists { try storage.preserveRawResult(result, for: meetingID) }
    }

    func activateOnDisk(_ document: TranscriptDocument, for meetingID: UUID) throws {
        guard persists else { return }
        // Migrate an in-memory legacy document before activating a new revision.
        if try storage.load(for: meetingID) == nil, let old = documents[meetingID] {
            try storage.save(old, for: meetingID)
        }
        try storage.activate(document, for: meetingID)
    }

    func publish(_ document: TranscriptDocument?, for meetingID: UUID) {
        documents[meetingID] = document
        errors.removeValue(forKey: meetingID)
    }

    func rollback(to previous: TranscriptDocument?, from activated: TranscriptDocument, for meetingID: UUID) throws {
        if persists {
            if let previous {
                try storage.activate(previous, for: meetingID)
            } else {
                guard try storage.load(for: meetingID)?.revisionID == activated.revisionID else {
                    throw TranscriptEditError.invalidDocument
                }
                // Roll back only this attempt's newly activated document. Raw results remain.
                try FileManager.default.removeItem(at: storage.fileURL(for: meetingID))
            }
        }
        documents[meetingID] = previous
    }

    func markUnreadable(_ meetingID: UUID, reason: String) {
        errors[meetingID] = reason
    }
}
