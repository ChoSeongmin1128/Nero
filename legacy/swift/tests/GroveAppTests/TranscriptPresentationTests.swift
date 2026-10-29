import Foundation
import Testing
@testable import GroveApp

struct TranscriptPresentationTests {
    private let a = MeetingSpeaker(name: "화자 1", order: 0)
    private let b = MeetingSpeaker(name: "화자 2", order: 1)

    private func utterance(_ start: Double, _ end: Double, _ speaker: UUID?,
                           text: String = "내용", source: String? = "recording") -> DocumentUtterance {
        .init(id: UUID(), startTime: start, endTime: end, rawText: text, sourceChannelID: source,
              engineClusterID: nil, speakerID: speaker, editedText: nil)
    }

    @Test func adjacentUtterancesStayIndividuallyAddressable() throws {
        let originals = [utterance(0, 2, a.id), utterance(2, 4, a.id), utterance(5, 6, a.id), utterance(7, 8, b.id)]
        let document = try TranscriptDocument(speakers: [a, b], utterances: originals)
        let rows = TranscriptPresentation.rows(in: document)
        #expect(rows.map(\.continuesPrevious) == [false, true, true, false])
        #expect(rows.map(\.utterance) == originals)
        #expect(rows.map(\.id) == originals.map(\.id))
        #expect(document.utterances == originals)
    }

    @Test func gapsOverlapChannelsAndUnknownSpeakersBreakContinuation() throws {
        let document = try TranscriptDocument(speakers: [a], utterances: [
            utterance(0, 2, a.id), utterance(1, 3, a.id), utterance(6, 8, a.id),
            utterance(8, 9, a.id, source: "microphone"), utterance(10, 11, nil), utterance(11, 12, nil)
        ])
        #expect(TranscriptPresentation.rows(in: document).allSatisfy { !$0.continuesPrevious })
    }

    @Test func filteredOutInterveningSpeakerDoesNotLookContinuous() throws {
        let document = try TranscriptDocument(speakers: [a, b], utterances: [
            utterance(0, 1, a.id), utterance(1, 2, b.id), utterance(2, 3, a.id)
        ])
        let rows = TranscriptPresentation.rows(in: document, speakerFilter: a.id.uuidString)
        #expect(rows.count == 2)
        #expect(rows.allSatisfy { !$0.continuesPrevious })
    }

    @Test func searchDoesNotConnectHiddenUtterances() throws {
        let document = try TranscriptDocument(speakers: [a], utterances: [
            utterance(0, 1, a.id, text: "검색 대상"), utterance(1, 2, a.id, text: "다른 내용"),
            utterance(2, 3, a.id, text: "검색 대상")
        ])
        #expect(TranscriptPresentation.rows(in: document, query: "검색").allSatisfy { !$0.continuesPrevious })
        #expect(TranscriptPresentation.rows(in: document, query: "화자 1").count == 3)
    }

    @Test func simultaneousTimesRetainSourceOrder() throws {
        let late = utterance(8, 9, a.id)
        let first = utterance(1, 2, b.id)
        let second = utterance(1, 3, a.id)
        let document = try TranscriptDocument(speakers: [a, b], utterances: [late, first, second])
        #expect(TranscriptPresentation.rows(in: document).map(\.id) == [first.id, second.id, late.id])
        let reviewRows = TranscriptPresentation.rows(in: document, onlyUtteranceIDs: [first.id, late.id])
        #expect(reviewRows.map(\.id) == [first.id, late.id])
        #expect(reviewRows.allSatisfy { !$0.continuesPrevious })
    }

    @Test func basicTimeRangeUsesWholeSecondsWithoutRoundingIntoTheNextSecond() {
        #expect(TranscriptPresentation.timeRange(utterance(12.34, 18.76, a.id)) == "00:12 – 00:18")
        #expect(TranscriptPresentation.timeRange(utterance(22.24, 22.64, a.id)) == "00:22 – 00:22")
        #expect(TranscriptPresentation.timestamp(0.08) == "00:00")
        #expect(TranscriptPresentation.timestamp(59.999) == "00:59")
        #expect(TranscriptPresentation.timestamp(60) == "01:00")
        #expect(TranscriptPresentation.timestamp(3599.999) == "59:59")
        #expect(TranscriptPresentation.timestamp(3600) == "01:00:00")
        #expect(TranscriptPresentation.timestamp(3661.25) == "01:01:01")
        #expect(TranscriptRenderer.timestamp(22.24) == "00:22")
    }

    @Test func displayFormattingDoesNotChangeStoredPlaybackOrSplitBoundaries() throws {
        let original = utterance(12.34, 18.76, a.id)
        var document = try TranscriptDocument(speakers: [a], utterances: [original])
        _ = TranscriptPresentation.timeRange(original)
        #expect(document.utterances.first?.startTime == 12.34)
        #expect(document.utterances.first?.endTime == 18.76)
        try document.splitUtterance(original.id, at: 14.125, firstText: "앞", secondText: "뒤")
        #expect(document.utterances.first?.endTime == 14.125)
        #expect(document.utterances.last?.startTime == 14.125)
    }

    @Test func invalidTimestampsNeverLookLikeTheStartOfARecording() {
        for value in [Double.nan, .infinity, -.infinity, -.leastNonzeroMagnitude, .greatestFiniteMagnitude] {
            #expect(TranscriptPresentation.timestamp(value) == "—")
        }
    }
}
