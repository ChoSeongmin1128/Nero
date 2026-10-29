import Foundation
import GroveInference
import Testing
@testable import GroveApp

struct MeetingIntegrationTests {
    @Test func appleChoiceHasNoModelRequirementOrInventedSpeakerReview() throws {
        var options = MeetingSpeakerOptions()
        options.transcriptionEngine = .apple
        let configuration = try options.plan(isDual: false).configuration
        #expect(configuration.transcriptionEngine == .apple)
        #expect(try configuration.resolvedEngine() == .none)
        let result = try InferenceResult(duration: 3, configuration: configuration,
            transcription: .init(utterances: [.init(start: 0.5, end: 2, text: "기본 전사 원문")]), rawDiarization: [])
        let document = try TranscriptDocument.preservingInference(result, sourceChannelID: "recording")
        #expect(document.speakers.isEmpty)
        #expect(document.speakerReviewCount == 0)
        #expect(document.speakerName(for: document.utterances[0]) == "화자 구분 없음")
        let reopened = try JSONDecoder().decode(InferenceResult.self, from: JSONEncoder().encode(result))
        try reopened.validate()
        #expect(throws: InferenceError.self) {
            try InferenceResult(duration: 3, configuration: configuration,
                transcription: result.transcription, rawDiarization: [.init(start: 0, end: 1, clusterID: "fake")])
        }
    }
    @Test func freshDefaultAndRetiredAutomaticChoiceUseNemotron() throws {
        #expect(try MeetingSpeakerOptions().plan(isDual: false).configuration.resolvedEngine() == .nemotron3)
        let json = Data(#"{"mode":"automatic","engineChoice":"automatic","countText":""}"#.utf8)
        #expect(try JSONDecoder().decode(MeetingSpeakerOptions.self, from: json).plan(isDual: false).configuration.resolvedEngine() == .nemotron3)
    }
    @Test func reminderOccurrenceWindowDoesNotIncludeAllExpiredOrFarFutureEvents() {
        let start = Date(timeIntervalSince1970: 2000)
        let event = ScheduledMeeting(id: "occurrence", calendarID: "calendar", title: "정기 회의", start: start,
            end: start.addingTimeInterval(1800), calendarName: "업무")
        #expect(!event.isReminderDue(at: start.addingTimeInterval(-301), leadMinutes: 5))
        #expect(event.isReminderDue(at: start.addingTimeInterval(-300), leadMinutes: 5))
        #expect(event.isReminderDue(at: start.addingTimeInterval(30), leadMinutes: 5))
        #expect(!event.isReminderDue(at: start.addingTimeInterval(601), leadMinutes: 5))
        #expect(!event.isReminderDue(at: start.addingTimeInterval(1800), leadMinutes: 5))
    }
    @Test func notionLinksIgnoreViewQueryAndRejectOtherHosts() throws {
        let id = "01234567-89ab-cdef-0123-456789abcdef"
        #expect(try NotionPageLink.id(from: "https://www.notion.so/회의-0123456789abcdef0123456789abcdef?v=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa") == id)
        #expect(try NotionPageLink.id(from: "https://workspace.notion.site/0123456789abcdef0123456789abcdef") == id)
        for link in ["https://notion.so.evil.test/0123456789abcdef0123456789abcdef", "javascript:alert(1)", "https://www.notion.so/no-page-id"] {
            #expect(throws: NotionExportError.self) { try NotionPageLink.id(from: link) }
        }
    }
    @Test func originalExportUndoesSplitAndHtmlPreservesLiteralMarkup() throws {
        var document = try TranscriptDocument(speakers: [], utterances: [.init(id: UUID(), startTime: 0,
            endTime: 4, rawText: "첫 문장 <page> & 다음 문장", sourceChannelID: "recording", engineClusterID: nil, speakerID: nil, editedText: nil)])
        try document.splitUtterance(document.utterances[0].id, at: 2, firstText: "첫 문장", secondText: "다음 문장")
        let original = TranscriptRenderer.render(document, options: .init(usesOriginalText: true))
        #expect(original.components(separatedBy: "첫 문장").count == 2)
        #expect(document.utterances.count == 2)
        let meeting = MeetingRecord(title: "예시", startedAt: Date(), duration: 4, status: .ready, glossaryProfile: "없음", transcript: [], claims: [])
        let html = MeetingExportContent.html(meeting: meeting, document: document, original: true)
        #expect(html.contains("&lt;page&gt; &amp;"))
        let markdown = MeetingExportContent.markdown(meeting: meeting, document: document, original: true)
        #expect(markdown.contains("\\<page\\>"))
    }
    @Test func modelChecksumRejectsSameSizeCorruption() async throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: url) }
        try Data("abc".utf8).write(to: url)
        let asset = ModelAsset(group: .moss, repository: "test", revision: "test", name: "test", bytes: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        #expect(try await ModelManager.matches(url, asset: asset))
        try Data("abd".utf8).write(to: url)
        #expect(try await !ModelManager.matches(url, asset: asset))
    }
}
