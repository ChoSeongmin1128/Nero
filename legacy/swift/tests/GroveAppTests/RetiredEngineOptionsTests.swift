import Foundation
import GroveInference
import Testing
@testable import GroveApp

struct RetiredEngineOptionsTests {
    @Test func freshDefaultIsNemotronWithoutInventingACount() throws {
        let configuration = try MeetingSpeakerOptions().plan(isDual: false).configuration
        #expect(try configuration.resolvedEngine() == .nemotron3)
        #expect(configuration.expectedSpeakerCount == nil && configuration.speakerCountPolicy == .advisory)
    }

    @Test(arguments: ["automatic", "ultra8", "sortformerStreaming", "community1", "nemotron3", nil])
    func savedEngineChoicesOpenAsNemotron(_ saved: String?) throws {
        let field = saved.map { #","engineChoice":"\#($0)""# } ?? ""
        let json = Data(#"{"mode":"manualCount","countText":"5"\#(field)}"#.utf8)
        let options = try JSONDecoder().decode(MeetingSpeakerOptions.self, from: json)
        let configuration = try options.plan(isDual: false).configuration
        #expect(try configuration.resolvedEngine() == .nemotron3)
        #expect(configuration.expectedSpeakerCount == 5 && configuration.speakerCountPolicy == .advisory)
    }

    @Test func reopeningRetiredConfigurationsKeepsTheCountAsAdvisory() throws {
        let retired: [InferenceConfiguration] = [
            .init(diarizationPreference: .ultra8),
            .init(expectedSpeakerCount: 4, diarizationPreference: .sortformerStreaming),
            .init(expectedSpeakerCount: 5, speakerCountPolicy: .exact),
            .init(expectedSpeakerCount: 5, diarizationPreference: .community1, speakerCountPolicy: .exact),
        ]
        for configuration in retired {
            let next = try MeetingSpeakerOptions(configuration: configuration).plan(isDual: false).configuration
            #expect(try next.resolvedEngine() == .nemotron3)
            #expect(next.expectedSpeakerCount == configuration.expectedSpeakerCount)
            #expect(next.speakerCountPolicy == .advisory)
        }
    }

    @Test func countsAboveEightReportTheLimit() throws {
        var options = MeetingSpeakerOptions()
        options.mode = .manualCount
        options.countText = "8"
        let eight = try options.plan(isDual: false).configuration
        #expect(try MeetingSpeakerOptions(configuration: eight).plan(isDual: false).configuration == eight)
        options.countText = "9"
        #expect(options.validationMessage(isDual: false)?.contains("최대 8명") == true)
        let nine = InferenceConfiguration(expectedSpeakerCount: 9, diarizationPreference: .community1, speakerCountPolicy: .exact)
        #expect(MeetingSpeakerOptions(configuration: nine).validationMessage(isDual: false)?.contains("최대 8명") == true)
    }

    @Test func optionsSurvivePersistence() throws {
        var options = MeetingSpeakerOptions()
        options.mode = .manualCount
        options.countText = "5"
        let copy = try JSONDecoder().decode(MeetingSpeakerOptions.self, from: JSONEncoder().encode(options))
        #expect(copy == options)
    }
}
