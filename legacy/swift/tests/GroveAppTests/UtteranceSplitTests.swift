import Foundation
import Testing
@testable import GroveApp

struct UtteranceSplitTests {
    private func fixture() throws -> TranscriptDocument {
        let speaker = MeetingSpeaker(name: "화자 1", order: 0)
        return try TranscriptDocument(speakers: [speaker], utterances: [
            .init(id: UUID(), startTime: 5, endTime: 9, rawText: "안녕하세요. 네 반갑습니다.",
                  sourceChannelID: "import", engineClusterID: "A", speakerID: speaker.id, editedText: nil)
        ])
    }

    @Test func splitPreservesRawTextAndUsesExplicitTime() throws {
        var document = try fixture()
        let original = document.utterances[0]
        try document.splitUtterance(original.id, at: 6.25, firstText: "안녕하세요.", secondText: "네 반갑습니다.")
        #expect(document.utterances.count == 2)
        #expect(document.utterances[0].endTime == 6.25)
        #expect(document.utterances[1].startTime == 6.25)
        #expect(document.utterances.allSatisfy { $0.rawText == original.rawText && $0.parentUtteranceID == original.id })
        #expect(TranscriptRenderer.render(document).components(separatedBy: "안녕하세요.").count == 2)
        let children = document.utterances
        try document.undo()
        #expect(document.utterances == [original])
        try document.redo()
        #expect(document.utterances == children)
    }

    @Test func childTextAndSpeakerCanBeEditedThenUndoRestoresTheOriginal() throws {
        var document = try fixture()
        let original = document.utterances[0]
        try document.splitUtterance(original.id, at: 7, firstText: "안녕하세요.", secondText: "네 반갑습니다.")
        try document.reassign(from: document.utterances[1].id, to: .new("화자 2"), scope: .utterance)
        try document.editText(document.utterances[1].id, to: "반갑습니다!")
        #expect(document.speakerName(for: document.utterances[1]) == "화자 2")
        #expect(TranscriptRenderer.render(document).contains("반갑습니다!"))
        try document.undo()
        try document.undo()
        try document.undo()
        #expect(document.utterances == [original])
        #expect(document.speakers.count == 1)
    }

    @Test func invalidSplitIsAtomicAndHistorySurvivesSerialization() throws {
        var document = try fixture()
        let original = document
        for time in [5.0, 9.0, Double.nan, Double.infinity] {
            #expect(throws: TranscriptEditError.self) {
                try document.splitUtterance(document.utterances[0].id, at: time, firstText: "앞", secondText: "뒤")
            }
            #expect(document == original)
        }
        try document.splitUtterance(document.utterances[0].id, at: 7, firstText: "앞", secondText: "뒤")
        var decoded = try JSONDecoder().decode(TranscriptDocument.self, from: JSONEncoder().encode(document))
        try decoded.validate()
        try decoded.undo()
        #expect(decoded.utterances == original.utterances)
    }
}
