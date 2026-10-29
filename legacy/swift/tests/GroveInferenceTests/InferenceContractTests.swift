import Foundation
import Testing
@testable import GroveInference

struct InferenceContractTests {
    @Test func automaticRoutingUsesUltraThroughEightAndCommunityAboveCapacity() throws {
        for count in 1...8 {
            #expect(try InferenceConfiguration(expectedSpeakerCount: count).resolvedEngine() == .ultra8)
        }
        for count in [9, 12, 40, Int.max] {
            #expect(try InferenceConfiguration(expectedSpeakerCount: count).resolvedEngine() == .community1)
        }
        #expect(try InferenceConfiguration().resolvedEngine() == .ultra8)
    }

    @Test func explicitSelectionNeverSilentlySwitchesEngine() throws {
        #expect(try InferenceConfiguration(diarizationPreference: .sortformerStreaming).resolvedEngine() == .sortformerStreaming)
        #expect(try InferenceConfiguration(expectedSpeakerCount: 4, diarizationPreference: .community1).resolvedEngine() == .community1)
        #expect(throws: InferenceError.self) {
            try InferenceConfiguration(expectedSpeakerCount: 5, diarizationPreference: .sortformerStreaming).resolvedEngine()
        }
        #expect(throws: InferenceError.self) { try InferenceConfiguration(expectedSpeakerCount: 0).resolvedEngine() }
    }

    @Test func sixSpeakerResultKeepsEveryIdentityWithoutClamping() throws {
        let utterances = (0..<6).map { RecognizedUtterance(start: Double($0 * 2), end: Double($0 * 2 + 1), text: "테스트 발화") }
        let turns = (0..<6).map { DiarizationTurn(start: Double($0 * 2), end: Double($0 * 2 + 1), clusterID: "c\($0)") }
        let result = try InferenceResult(duration: 12, configuration: .init(expectedSpeakerCount: 6),
                                         transcription: .init(utterances: utterances), rawDiarization: turns)
        #expect(Set(result.assignments.compactMap(\.clusterID)).count == 6)
        #expect(result.rawDiarization == turns)
        #expect(result.transcription.utterances == utterances)
    }

    @Test func savedAutomaticResultsRetainTheirHistoricalEngine() throws {
        let configurations: [InferenceConfiguration] = [
            .init(diarizationPreference: .community1),
            .init(expectedSpeakerCount: 4, diarizationPreference: .sortformerStreaming),
            .init(expectedSpeakerCount: 5, diarizationPreference: .community1, speakerCountPolicy: .exact),
        ]
        for configuration in configurations {
            let result = try InferenceResult(duration: 2, configuration: configuration,
                transcription: .init(utterances: [.init(start: 0, end: 1, text: "과거 전사")]),
                rawDiarization: [.init(start: 0, end: 1, clusterID: "A")])
            var object = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(result)) as? [String: Any])
            var legacyConfiguration = try #require(object["configuration"] as? [String: Any])
            legacyConfiguration["diarizationPreference"] = "automatic"
            object["configuration"] = legacyConfiguration
            let decoded = try JSONDecoder().decode(InferenceResult.self, from: JSONSerialization.data(withJSONObject: object))
            try decoded.validate()
            #expect(decoded.configuration.diarizationPreference == .automatic)
            #expect(decoded.diarizationEngine == result.diarizationEngine)
            #expect(decoded.transcription == result.transcription)
            #expect(decoded.rawDiarization == result.rawDiarization)
        }
    }

    @Test func recordedEngineValidationStillRejectsExplicitMismatchAndImpossibleCounts() throws {
        let result = try InferenceResult(duration: 2,
            configuration: .init(expectedSpeakerCount: 4, diarizationPreference: .sortformerStreaming),
            transcription: .init(utterances: [.init(start: 0, end: 1, text: "전사")]),
            rawDiarization: [.init(start: 0, end: 1, clusterID: "A")])
        let original = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(result)) as? [String: Any])
        var mismatch = original
        mismatch["diarizationEngine"] = "ultra8"
        let invalid = try JSONDecoder().decode(InferenceResult.self, from: JSONSerialization.data(withJSONObject: mismatch))
        #expect(throws: InferenceError.self) { try invalid.validate() }
        for (count, policy) in [(5, "advisory"), (4, "exact"), (0, "advisory")] {
            var object = original
            object["configuration"] = ["expectedSpeakerCount": count,
                "diarizationPreference": "automatic", "speakerCountPolicy": policy]
            let decoded = try JSONDecoder().decode(InferenceResult.self, from: JSONSerialization.data(withJSONObject: object))
            #expect(throws: InferenceError.self) { try decoded.validate() }
        }
    }

    @Test func silenceAndTiesStayUnassignedInsteadOfGuessing() {
        let turns = [DiarizationTurn(start: 0, end: 1, clusterID: "A"), DiarizationTurn(start: 1, end: 2, clusterID: "B")]
        let result = SpeakerProjection.assign([
            .init(start: 0, end: 2, text: "겹친 발화"), .init(start: 4, end: 5, text: "범위 밖")
        ], turns: turns)
        #expect(result[0].clusterID == nil)
        #expect(result[0].reviewReasons.contains(.ambiguousSpeakers))
        #expect(result[1].reviewReasons == [.noDiarizationCoverage])
    }

    @Test func duplicateIntervalsDoNotDoubleCount() {
        let result = SpeakerProjection.assign([.init(start: 0, end: 3, text: "발화")], turns: [
            .init(start: 0, end: 1, clusterID: "A"), .init(start: 0, end: 1, clusterID: "A"),
            .init(start: 1, end: 3, clusterID: "B"),
        ])
        #expect(result[0].clusterID == "B")
        #expect(result[0].overlapSecondsByCluster["A"] == 1)
        #expect(result[0].reviewReasons.contains(.multipleSpeakersInUtterance))
    }

    @Test func zeroDurationUsesOnlyOneActiveSpeakerAndHalfOpenBoundaries() {
        let result = SpeakerProjection.assign([.init(start: 1, end: 1, text: "네")], turns: [
            .init(start: 0, end: 1, clusterID: "A"), .init(start: 1, end: 2, clusterID: "B"),
        ])
        #expect(result[0].clusterID == "B")
    }

    @Test func partialAndInvalidOutputsAreRejected() {
        let text = RawTranscription(utterances: [.init(start: 0, end: 1, text: "내용")], hitTokenLimit: true)
        #expect(throws: InferenceError.self) { try text.validate(duration: 2) }
        #expect(throws: InferenceError.self) {
            try SpeakerProjection.validate([.init(start: 0, end: .infinity, clusterID: "A")], duration: 2, engine: .community1)
        }
    }
}
