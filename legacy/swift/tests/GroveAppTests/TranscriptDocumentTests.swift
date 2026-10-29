import Foundation
import Testing
@testable import GroveApp

struct TranscriptDocumentTests {
    private func fixture() throws -> TranscriptDocument {
        let a = MeetingSpeaker(name: "화자 1", order: 0)
        let b = MeetingSpeaker(name: "화자 2", order: 1)
        return try TranscriptDocument(speakers: [a, b], utterances: [
            DocumentUtterance(id: UUID(), startTime: 0, endTime: 2, rawText: "안녕하세요.", sourceChannelID: "import", engineClusterID: "A", speakerID: a.id, editedText: nil),
            DocumentUtterance(id: UUID(), startTime: 3, endTime: 5, rawText: "첫 번째 의견입니다.", sourceChannelID: "import", engineClusterID: "A", speakerID: a.id, editedText: nil),
            DocumentUtterance(id: UUID(), startTime: 4, endTime: 6, rawText: "동의합니다.", sourceChannelID: "import", engineClusterID: "B", speakerID: b.id, editedText: nil),
            DocumentUtterance(id: UUID(), startTime: 8, endTime: 9, rawText: "다음 내용입니다.", sourceChannelID: "import", engineClusterID: "A", speakerID: a.id, editedText: nil),
        ])
    }

    @Test func singleAssignmentDoesNotChangeOtherUtterances() throws {
        var document = try fixture()
        let original = document.utterances
        let count = try document.reassign(from: original[1].id, to: .existing(document.speakers[1].id), scope: .utterance)
        #expect(count == 1)
        #expect(document.utterances[0] == original[0])
        #expect(document.utterances[2] == original[2])
        #expect(document.utterances[1].engineClusterID == "A")
        #expect(document.utterances[1].rawText == original[1].rawText)
    }

    @Test func allAssignmentPreservesAnEarlierManualCorrection() throws {
        var document = try fixture()
        let a = document.speakers[0].id
        let b = document.speakers[1].id
        try document.reassign(from: document.utterances[0].id, to: .existing(b), scope: .utterance)
        let changed = try document.reassign(from: document.utterances[1].id, to: .new("검토자"), scope: .allSameSpeaker)
        #expect(changed == 2)
        #expect(document.utterances[0].speakerID == b)
        #expect(document.utterances[2].speakerID == b)
        #expect(document.utterances.allSatisfy { $0.speakerID != a })
        try document.undo()
        #expect(document.speakers.count == 2)
        #expect(document.utterances[1].speakerID == a)
        #expect(document.utterances[3].speakerID == a)
        #expect(document.utterances[0].speakerID == b)
        try document.redo()
        #expect(document.speakers.count == 3)
        #expect(document.speakerName(for: document.utterances[1]) == "검토자")
    }

    @Test func followingScopeOnlyIncludesSameSpeaker() throws {
        var document = try fixture()
        let a = document.speakers[0].id
        let b = document.speakers[1].id
        let ids = try document.reassignmentIDs(from: document.utterances[1].id, scope: .followingSameSpeaker)
        #expect(ids == [document.utterances[1].id, document.utterances[3].id])
        try document.reassign(from: document.utterances[1].id, to: .existing(b), scope: .followingSameSpeaker)
        #expect(document.utterances[0].speakerID == a)
    }

    @Test func invalidSelectionAndTargetAreAtomic() throws {
        var document = try fixture()
        let before = document
        #expect(throws: TranscriptEditError.self) {
            try document.reassign(from: document.utterances[0].id, to: .new("새 화자"), scope: .selected([UUID()]))
        }
        #expect(document == before)
        #expect(throws: TranscriptEditError.self) {
            try document.reassign(from: document.utterances[0].id, to: .existing(UUID()), scope: .allSameSpeaker)
        }
        #expect(document == before)
    }

    @Test func textEditsKeepRawAndCanBeReset() throws {
        var document = try fixture()
        let id = document.utterances[0].id
        try document.editText(id, to: "안녕하세요! 👋\n수정했습니다.")
        #expect(document.utterances[0].rawText == "안녕하세요.")
        #expect(document.utterances[0].displayedText.contains("👋"))
        try document.undo()
        #expect(document.utterances[0].editedText == nil)
        try document.redo()
        try document.editText(id, to: "안녕하세요.")
        #expect(document.utterances[0].editedText == nil)
    }

    @Test func newEditAfterUndoClearsRedo() throws {
        var document = try fixture()
        try document.renameSpeaker(document.speakers[0].id, to: "가")
        try document.undo()
        try document.editText(document.utterances[0].id, to: "새 내용")
        #expect(document.redoHistory.isEmpty)
    }

    @Test func namesAreIdentityIndependentAndRenderAtExportTime() throws {
        var document = try fixture()
        let originalIDs = document.utterances.map(\.speakerID)
        try document.renameSpeaker(document.speakers[0].id, to: " 같은 이름 ")
        try document.renameSpeaker(document.speakers[1].id, to: "같은 이름")
        #expect(document.speakers.count == 2)
        #expect(document.utterances.map(\.speakerID) == originalIDs)
        let output = TranscriptRenderer.render(document)
        #expect(output.contains("[00:00] 같은 이름"))
        #expect(!output.contains("화자 1"))
    }

    @Test func selectionExportKeepsOrderAndLatestText() throws {
        var document = try fixture()
        try document.editText(document.utterances[1].id, to: "수정한 본문")
        let output = TranscriptRenderer.render(document, options: .init(selectedUtteranceIDs: [document.utterances[1].id, document.utterances[3].id]))
        #expect(output == "[00:03] 화자 1\n수정한 본문\n\n[00:08] 화자 1\n다음 내용입니다.\n")
        #expect(TranscriptRenderer.render(document, options: .init(selectedUtteranceIDs: [])) == "")
    }

    @Test func markdownEscapesSpeakerAndTranscriptSyntax() throws {
        var document = try fixture()
        try document.renameSpeaker(document.speakers[0].id, to: "이름*[1]")
        try document.editText(document.utterances[0].id, to: "# 내용 `코드` 👋")
        let output = TranscriptRenderer.render(document, options: .init(format: .markdown, selectedUtteranceIDs: [document.utterances[0].id]))
        #expect(output == "[00:00] **이름\\*\\[1\\]**\n\\# 내용 \\`코드\\` 👋\n")
    }

    @Test func documentAndUndoHistoryRoundTrip() throws {
        var document = try fixture()
        try document.reassign(from: document.utterances[0].id, to: .new("새 화자"), scope: .allSameSpeaker)
        let encoded = try JSONEncoder().encode(document)
        var decoded = try JSONDecoder().decode(TranscriptDocument.self, from: encoded)
        try decoded.validate()
        #expect(decoded == document)
        try decoded.undo()
        #expect(decoded.speakers.count == 2)
        #expect(decoded.utterances[0].speakerID == decoded.speakers[0].id)
    }

    @Test func invalidTimesAndMissingSpeakersAreRejected() throws {
        let utterance = DocumentUtterance(id: UUID(), startTime: .infinity, endTime: 2, rawText: "내용", sourceChannelID: nil, engineClusterID: nil, speakerID: UUID(), editedText: nil)
        #expect(throws: TranscriptEditError.self) { try TranscriptDocument(speakers: [], utterances: [utterance]) }
        #expect(TranscriptRenderer.timestamp(3661) == "01:01:01")
        #expect(TranscriptRenderer.timestamp(.infinity) == "00:00")
        #expect(TranscriptRenderer.timestamp(Double.greatestFiniteMagnitude) == "00:00")
    }
}
