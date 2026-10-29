import Foundation
import GroveInference
import Testing
@testable import GroveApp

struct SpeakerReviewTests {
    private func fixture() throws -> TranscriptDocument {
        let first = MeetingSpeaker(name: "화자 1", order: 0)
        let second = MeetingSpeaker(name: "화자 2", order: 1)
        return try TranscriptDocument(speakers: [first, second], utterances: [
            .init(id: UUID(), startTime: 0, endTime: 4, rawText: "첫 번째 합성 발화", sourceChannelID: "recording",
                  engineClusterID: "A", speakerID: first.id, assignmentReviewReasons: [.ambiguousSpeakers]),
            .init(id: UUID(), startTime: 5, endTime: 9, rawText: "두 번째 합성 발화", sourceChannelID: "recording",
                  engineClusterID: "A", speakerID: first.id, assignmentReviewReasons: [.noDiarizationCoverage]),
            .init(id: UUID(), startTime: 10, endTime: 12, rawText: "세 번째 합성 발화", sourceChannelID: "recording",
                  engineClusterID: "B", speakerID: second.id)
        ])
    }

    private func generated(turns: [DiarizationTurn], start: Double = 0, end: Double = 4) throws -> (InferenceResult, TranscriptDocument) {
        let result = try InferenceResult(duration: 5, configuration: .init(diarizationPreference: .ultra8),
            transcription: .init(utterances: [.init(start: start, end: end, text: "합성 문장")]), rawDiarization: turns)
        return (result, try TranscriptDocument.preservingInference(result, sourceChannelID: "recording"))
    }

    @Test func smallSecondaryOverlapIsNotAWarningOrAnAutomaticConfirmation() throws {
        let (result, document) = try generated(turns: [.init(start: 0, end: 4, clusterID: "A"), .init(start: 1, end: 1.1, clusterID: "B")])
        let utterance = try #require(document.utterances.first)
        #expect(result.assignments[0].reviewReasons == [.multipleSpeakersInUtterance])
        #expect(utterance.assignmentReviewReasons == result.assignments[0].reviewReasons)
        #expect(utterance.assignmentEvidence?.overlapSecondsByCluster == result.assignments[0].overlapSecondsByCluster)
        let assessment = document.speakerReview(for: utterance)
        #expect(assessment.hasRawAssignmentEvidence)
        #expect(!assessment.needsReview && !assessment.isConfirmed && assessment.canConfirm)
        #expect(document.speakerReviewCount == 0)
        try result.validate()
    }

    @Test func lowCoverageAndCloseCompetitorAreSelectedFromMeasuredEvidence() throws {
        let (_, low) = try generated(turns: [.init(start: 0, end: 1, clusterID: "A")])
        #expect(low.speakerReview(for: low.utterances[0]).reasons == [.lowCoverage])
        let (_, close) = try generated(turns: [.init(start: 0, end: 3, clusterID: "A"), .init(start: 1, end: 3.7, clusterID: "B")])
        #expect(close.speakerReview(for: close.utterances[0]).reasons == [.competingSpeakers])
        let (_, tie) = try generated(turns: [.init(start: 0, end: 3, clusterID: "A"), .init(start: 1, end: 4, clusterID: "B")])
        #expect(tie.speakerReview(for: tie.utterances[0]).reasons.contains(.ambiguousSpeakers))
        #expect(!tie.speakerReview(for: tie.utterances[0]).canConfirm)
        let (_, empty) = try generated(turns: [])
        #expect(empty.speakerReview(for: empty.utterances[0]).reasons.contains(.noDiarizationCoverage))
    }

    @Test func legacyMultipleOnlyAndEditedTextAreNotSilentlyApproved() throws {
        var document = try fixture()
        var item = document.utterances[0]
        item.assignmentReviewReasons = [.multipleSpeakersInUtterance]
        item.editedText = "예전에 수정한 합성 문장"
        document = try TranscriptDocument(speakers: document.speakers, utterances: [item])
        let assessment = document.speakerReview(for: item)
        #expect(!assessment.needsReview && !assessment.isConfirmed && !assessment.hasRawAssignmentEvidence)
        #expect(document.utterances[0].editedText == item.editedText)
        #expect(document.utterances[0].assignmentReviewReasons == [.multipleSpeakersInUtterance])
    }

    @Test func explicitConfirmationIsPerUtteranceAndPreservesRawReasons() throws {
        var document = try fixture()
        let original = document.utterances
        let reviewedAt = Date(timeIntervalSince1970: 1_700_000_000)
        #expect(document.speakerReviewCount == 2)
        try document.confirmUtteranceSpeaker(original[0].id, reviewedAt: reviewedAt)
        #expect(document.speakerReviewCount == 1)
        #expect(document.speakerReview(for: original[0].id)?.isConfirmed == true)
        #expect(document.utterances[1] == original[1] && document.utterances[2] == original[2])
        #expect(document.utterances[0].rawText == original[0].rawText && document.utterances[0].editedText == nil)
        #expect(document.utterances[0].assignmentReviewReasons == original[0].assignmentReviewReasons)
        let evidence = try #require(document.utterances[0].speakerReviewEvidence)
        #expect(evidence.reviewedAt == reviewedAt)
        #expect(evidence.binding.speakerID == original[0].speakerID)
        #expect(evidence.binding.text == original[0].displayedText)
        #expect(evidence.binding.sourceChannelID == "recording")
        #expect(evidence.binding.rulesVersion == SpeakerReviewPolicy.rulesVersion)
        guard case .speakerReview(let id, nil, _) = document.undoHistory.last?.mutations.first else {
            Issue.record("Confirmation must have its own undo mutation"); return
        }
        #expect(id == original[0].id)
    }

    @Test func confirmationUndoRedoAndRepeatedConfirmationAreStable() throws {
        var document = try fixture()
        let id = document.utterances[0].id
        try document.confirmUtteranceSpeaker(id)
        let confirmed = document.utterances[0].speakerReviewEvidence
        try document.confirmUtteranceSpeaker(id)
        #expect(document.undoHistory.count == 1)
        try document.undo()
        #expect(document.speakerReview(for: id)?.needsReview == true)
        #expect(document.utterances[0].speakerReviewEvidence == nil)
        try document.redo()
        #expect(document.speakerReview(for: id)?.isConfirmed == true)
        #expect(document.utterances[0].speakerReviewEvidence == confirmed)
    }

    @Test func unassignedOrMissingUtteranceCannotBeConfirmed() throws {
        var document = try fixture()
        var item = document.utterances[0]
        item.speakerID = nil
        document = try TranscriptDocument(speakers: document.speakers, utterances: [item])
        let before = document
        #expect(!document.speakerReview(for: item).canConfirm)
        #expect(throws: TranscriptEditError.self) { try document.confirmUtteranceSpeaker(item.id) }
        #expect(throws: TranscriptEditError.self) { try document.confirmUtteranceSpeaker(UUID()) }
        #expect(document == before)
    }

    @Test func textChangeInvalidatesOnlyThatConfirmationAndUndoRestoresIt() throws {
        var document = try fixture()
        let first = document.utterances[0].id
        let second = document.utterances[1].id
        try document.confirmUtteranceSpeaker(first)
        try document.confirmUtteranceSpeaker(second)
        let previousEvidence = document.utterances[0].speakerReviewEvidence
        try document.editText(first, to: "변경한 합성 문장")
        #expect(document.speakerReview(for: first)?.reasons.contains(.changedSinceConfirmation) == true)
        #expect(document.speakerReview(for: second)?.isConfirmed == true)
        #expect(document.speakerReviewCount == 1)
        #expect(document.utterances[0].speakerReviewEvidence == previousEvidence)
        try document.undo()
        #expect(document.speakerReview(for: first)?.isConfirmed == true)
        try document.redo()
        #expect(document.speakerReview(for: first)?.needsReview == true)
    }

    @Test func reconfirmationUndoRestoresThePriorStaleEvidenceBeforeUndoingText() throws {
        var document = try fixture()
        let id = document.utterances[0].id
        try document.confirmUtteranceSpeaker(id)
        let firstEvidence = document.utterances[0].speakerReviewEvidence
        try document.editText(id, to: "다시 확인한 합성 문장")
        try document.confirmUtteranceSpeaker(id)
        #expect(document.speakerReview(for: id)?.isConfirmed == true)
        #expect(document.utterances[0].speakerReviewEvidence?.id != firstEvidence?.id)
        try document.undo()
        #expect(document.utterances[0].speakerReviewEvidence == firstEvidence)
        #expect(document.speakerReview(for: id)?.needsReview == true)
        try document.undo()
        #expect(document.speakerReview(for: id)?.isConfirmed == true)
    }

    @Test func bulkReassignmentDoesNotConfirmHiddenPendingItems() throws {
        var document = try fixture()
        let first = document.utterances[0].id
        let second = document.utterances[1].id
        let unaffected = document.utterances[2]
        try document.confirmUtteranceSpeaker(first)
        let originalEvidence = document.utterances[0].speakerReviewEvidence
        #expect(try document.reassign(from: first, to: .existing(document.speakers[1].id), scope: .allSameSpeaker) == 2)
        #expect(document.speakerReview(for: first)?.needsReview == true)
        #expect(document.utterances[0].speakerReviewEvidence == originalEvidence)
        #expect(document.speakerReview(for: second)?.needsReview == true)
        #expect(document.utterances[1].speakerReviewEvidence == nil)
        #expect(document.utterances[2] == unaffected)
        try document.undo()
        #expect(document.speakerReview(for: first)?.isConfirmed == true)
        #expect(document.speakerReview(for: second)?.needsReview == true)
        #expect(document.utterances[1].speakerReviewEvidence == nil)
        try document.redo()
        #expect(document.speakerReview(for: first)?.needsReview == true)
        #expect(document.speakerReview(for: second)?.needsReview == true)
    }

    @Test func explicitSingleCorrectionAndConfirmationAreOneUndoableCommand() throws {
        var document = try fixture()
        let first = document.utterances[0].id
        let before = document.utterances
        let count = try document.reassign(from: first, to: .existing(document.speakers[1].id),
            scope: .utterance, confirmingAnchor: true)
        #expect(count == 1)
        #expect(document.speakerReview(for: first)?.isConfirmed == true)
        #expect(document.utterances[0].speakerReviewEvidence?.action == .confirmedAssignmentChange)
        #expect(document.utterances[1] == before[1])
        #expect(document.undoHistory.count == 1 && document.undoHistory[0].mutations.count == 2)
        try document.undo()
        #expect(document.utterances == before)
        try document.redo()
        #expect(document.speakerReview(for: first)?.isConfirmed == true)
    }

    @Test func confirmationCannotBeBundledWithAnyBulkScope() throws {
        var document = try fixture()
        let before = document
        let first = document.utterances[0].id
        for scope in [SpeakerEditScope.allSameSpeaker, .followingSameSpeaker, .selected([first])] {
            #expect(throws: TranscriptEditError.self) {
                try document.reassign(from: first, to: .new("새 합성 화자"), scope: scope, confirmingAnchor: true)
            }
            #expect(document == before)
        }
    }

    @Test func explicitConfirmationCanKeepTheSameAssignedSpeaker() throws {
        var document = try fixture()
        let item = document.utterances[0]
        let speakerID = try #require(item.speakerID)
        #expect(try document.reassign(from: item.id, to: .existing(speakerID),
            scope: .utterance, confirmingAnchor: true) == 0)
        #expect(document.speakerReview(for: item.id)?.isConfirmed == true)
        #expect(document.utterances[0].speakerReviewEvidence?.action == .confirmedCurrentSpeaker)
        #expect(document.undoHistory.count == 1)
    }

    @Test func reassignmentOfAnUnflaggedItemDoesNotInventHumanConfirmation() throws {
        var document = try fixture()
        let id = document.utterances[2].id
        try document.reassign(from: id, to: .existing(document.speakers[0].id), scope: .utterance)
        #expect(document.utterances[2].speakerReviewEvidence == nil)
        #expect(document.speakerReview(for: id)?.isConfirmed == false)
    }

    @Test func speakerDisplayNameDoesNotInvalidateAssignmentReview() throws {
        var document = try fixture()
        let id = document.utterances[0].id
        try document.confirmUtteranceSpeaker(id)
        let evidence = document.utterances[0].speakerReviewEvidence
        try document.renameSpeaker(document.speakers[0].id, to: "새 표시 이름")
        #expect(document.speakerReview(for: id)?.isConfirmed == true)
        #expect(document.utterances[0].speakerReviewEvidence == evidence)
    }

    @Test func timeSourceGenerationAndRulesChangesCannotReuseConfirmation() throws {
        var (_, document) = try generated(turns: [.init(start: 0, end: 4, clusterID: "A")])
        let id = document.utterances[0].id
        try document.confirmUtteranceSpeaker(id)
        for (key, value) in [("startTime", 0.5 as Any), ("endTime", 3.5 as Any), ("sourceChannelID", "another-source" as Any)] {
            var object = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(document)) as? [String: Any])
            var utterances = try #require(object["utterances"] as? [[String: Any]])
            utterances[0][key] = value
            object["utterances"] = utterances
            let changed = try JSONDecoder().decode(TranscriptDocument.self, from: JSONSerialization.data(withJSONObject: object))
            try changed.validate()
            #expect(changed.speakerReview(for: id)?.needsReview == true)
            #expect(changed.speakerReview(for: id)?.hasRawAssignmentEvidence == false)
        }
        let newGeneration = try TranscriptDocument(speakers: document.speakers, utterances: document.utterances)
        #expect(newGeneration.speakerReview(for: id)?.isConfirmed == false)
        let nextPolicy = SpeakerReviewPolicy.assess(document.utterances[0], documentRevisionID: document.revisionID,
            speakerExists: true, rulesVersion: "speaker-assignment-review-v2-test")
        #expect(nextPolicy.reasons.contains(.reviewRulesChanged) && nextPolicy.needsReview)
    }

    @Test func splitChildrenRequireReviewAndUndoRestoresParentEvidence() throws {
        var (_, document) = try generated(turns: [.init(start: 0, end: 4, clusterID: "A")])
        let id = document.utterances[0].id
        try document.confirmUtteranceSpeaker(id)
        let parent = document.utterances[0]
        try document.splitUtterance(id, at: 2, firstText: "합성 앞 문장", secondText: "합성 뒤 문장")
        #expect(document.speakerReviewCount == 2)
        #expect(document.utterances.allSatisfy { $0.speakerReviewEvidence == nil && $0.assignmentEvidence == nil })
        let children = document.utterances
        try document.undo()
        #expect(document.utterances == [parent])
        #expect(document.speakerReview(for: id)?.isConfirmed == true)
        try document.redo()
        #expect(document.utterances == children && document.speakerReviewCount == 2)
    }

    @Test func optionalFieldsAndOldUndoSplitSnapshotsDecodeForSchemaThreeAndFour() throws {
        var original = try fixture()
        try original.editText(original.utterances[0].id, to: "과거 합성 수정문")
        try original.splitUtterance(original.utterances[0].id, at: 2, firstText: "과거 앞 문장", secondText: "과거 뒤 문장")
        for version in [3, 4] {
            var object = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(original)) as? [String: Any])
            object["schemaVersion"] = version
            var decoded = try JSONDecoder().decode(TranscriptDocument.self, from: JSONSerialization.data(withJSONObject: object))
            try decoded.validate()
            #expect(decoded.schemaVersion == version)
            #expect(decoded.utterances.allSatisfy { $0.speakerReviewEvidence == nil && $0.assignmentEvidence == nil })
            #expect(decoded.utterances.map(\.displayedText) == original.utterances.map(\.displayedText))
            try decoded.undo()
            #expect(decoded.schemaVersion == 5)
            #expect(decoded.utterances[0].displayedText == "과거 합성 수정문")
            try decoded.redo()
            #expect(decoded.utterances == original.utterances)
        }
    }

    @Test func newReviewEvidenceAndUndoHistoryRoundTripTogether() throws {
        var document = try fixture()
        let id = document.utterances[0].id
        try document.confirmUtteranceSpeaker(id)
        try document.editText(id, to: "재확인할 합성 문장")
        let data = try JSONEncoder().encode(document)
        var decoded = try JSONDecoder().decode(TranscriptDocument.self, from: data)
        try decoded.validate()
        #expect(decoded == document)
        #expect(decoded.speakerReview(for: id)?.needsReview == true)
        try decoded.undo()
        #expect(decoded.speakerReview(for: id)?.isConfirmed == true)
        try decoded.undo()
        #expect(decoded.speakerReview(for: id)?.isConfirmed == false)
    }
}

@MainActor
struct SpeakerReviewStorageTests {
    private func fixture() throws -> (URL, GroveStore, MeetingRecord, UUID) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("grove-review-\(UUID().uuidString)")
        let meeting = MeetingRecord(title: "합성 테스트 녹음", startedAt: Date(timeIntervalSince1970: 1_700_000_000),
            duration: 4, status: .ready, audioPath: nil, glossaryProfile: "사전 없음", transcript: [], claims: [])
        try MeetingRecordStorage(url: root.appendingPathComponent("meetings.json")).save([meeting])
        let store = GroveStore(baseDirectory: root)
        let result = try InferenceResult(duration: 4, configuration: .init(diarizationPreference: .ultra8),
            transcription: .init(utterances: [.init(start: 0, end: 4, text: "합성 발화")]),
            rawDiarization: [.init(start: 0, end: 1, clusterID: "A")])
        try store.acceptInferenceResult(result, meetingID: meeting.id, sourceChannelID: "recording")
        return (root, store, meeting, result.transcription.utterances[0].id)
    }

    @Test func confirmationPersistsAndTitleChangeDoesNotInvalidateIt() throws {
        let (root, store, meeting, id) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        #expect(store.transcripts.confirmUtteranceSpeaker(meetingID: meeting.id, utteranceID: id))
        #expect(store.renameMeeting(id: meeting.id, title: "변경한 합성 제목"))
        let reopened = GroveStore(baseDirectory: root)
        #expect(reopened.transcripts.speakerReview(meetingID: meeting.id, utteranceID: id)?.isConfirmed == true)
        #expect(reopened.transcriptDocuments[meeting.id] == store.transcriptDocuments[meeting.id])
    }

    @Test func failedSaveDoesNotPublishReviewOrAdvanceUndo() throws {
        let (root, store, meeting, id) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        let before = try #require(store.transcriptDocuments[meeting.id])
        let file = TranscriptDocumentStorage(directory: root.appendingPathComponent("Documents")).fileURL(for: meeting.id)
        let originalBytes = try Data(contentsOf: file)
        try FileManager.default.createDirectory(at: file.appendingPathExtension("backup"), withIntermediateDirectories: false)
        #expect(!store.transcripts.confirmUtteranceSpeaker(meetingID: meeting.id, utteranceID: id))
        #expect(store.transcriptDocuments[meeting.id] == before)
        #expect(store.transcripts.speakerReview(meetingID: meeting.id, utteranceID: id)?.needsReview == true)
        #expect(try Data(contentsOf: file) == originalBytes)
    }

    @Test func combinedAssignmentConfirmationSaveFailurePublishesNeitherChange() throws {
        let (root, store, meeting, id) = try fixture()
        defer { try? FileManager.default.removeItem(at: root) }
        let before = try #require(store.transcriptDocuments[meeting.id])
        let file = TranscriptDocumentStorage(directory: root.appendingPathComponent("Documents")).fileURL(for: meeting.id)
        let originalBytes = try Data(contentsOf: file)
        try FileManager.default.createDirectory(at: file.appendingPathExtension("backup"), withIntermediateDirectories: false)
        #expect(!store.transcripts.reassignSpeaker(meetingID: meeting.id, utteranceID: id, target: .new("새 합성 화자"),
            scope: .utterance, confirmingAnchor: true))
        #expect(store.transcriptDocuments[meeting.id] == before)
        #expect(store.transcripts.speakerReview(meetingID: meeting.id, utteranceID: id)?.isConfirmed == false)
        #expect(try Data(contentsOf: file) == originalBytes)
    }
}
