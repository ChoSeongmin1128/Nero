import Foundation
import GroveInference
import Testing
@testable import GroveApp

struct InferenceDocumentBridgeTests {
    @Test func historicalAutomaticResultKeepsRecordedEngineInDocumentProvenance() throws {
        let result = try InferenceResult(duration: 2,
            configuration: .init(expectedSpeakerCount: 4, diarizationPreference: .sortformerStreaming),
            transcription: .init(utterances: [.init(start: 0, end: 1, text: "과거 전사")]),
            rawDiarization: [.init(start: 0, end: 1, clusterID: "A")])
        var object = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(result)) as? [String: Any])
        object["configuration"] = ["expectedSpeakerCount": 4,
            "diarizationPreference": "automatic", "speakerCountPolicy": "advisory"]
        let historical = try JSONDecoder().decode(InferenceResult.self, from: JSONSerialization.data(withJSONObject: object))
        let document = try TranscriptDocument.preservingInference(historical, sourceChannelID: "recording")
        #expect(document.sourceDiarizationEngines?["recording"] == .sortformerStreaming)
        #expect(document.utterances.first?.rawText == "과거 전사")
    }

    @Test func malformedDecodedAssignmentsAreRejectedBeforeDictionaryConstruction() throws {
        let output = try InferenceResult(duration: 2, configuration: .init(expectedSpeakerCount: 1),
            transcription: .init(utterances: [.init(start: 0, end: 1, text: "발화")]),
            rawDiarization: [.init(start: 0, end: 1, clusterID: "A")])
        var object = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(output)) as? [String: Any])
        let assignments = try #require(object["assignments"] as? [[String: Any]])
        object["assignments"] = assignments + assignments
        let malformed = try JSONDecoder().decode(InferenceResult.self, from: JSONSerialization.data(withJSONObject: object))
        #expect(throws: InferenceError.self) { try TranscriptDocument.preservingInference(malformed, sourceChannelID: "import") }
    }

    @Test func eightPeopleRemainEditableAndExportableWithoutLosingRawLabels() throws {
        let segments = (0..<8).map { RecognizedUtterance(start: Double($0), end: Double($0 + 1), text: "발화 \($0)", asrClusterID: "moss") }
        let turns = (0..<8).map { DiarizationTurn(start: Double($0), end: Double($0 + 1), clusterID: "c\($0)") }
        let result = try InferenceResult(duration: 8, configuration: .init(expectedSpeakerCount: 8),
                                         transcription: .init(utterances: segments), rawDiarization: turns)
        var document = try TranscriptDocument.preservingInference(result, sourceChannelID: "import")
        #expect(document.speakers.count == 8)
        #expect(document.utterances[7].engineClusterID == "c7")
        #expect(document.utterances[7].asrClusterID == "moss")
        try document.editText(document.utterances[7].id, to: "수정한 여덟 번째 발화")
        try document.renameSpeaker(document.speakers[7].id, to: "참석자 여덟")
        let exported = TranscriptRenderer.render(document)
        #expect(exported.contains("참석자 여덟"))
        #expect(exported.contains("수정한 여덟 번째 발화"))
        #expect(document.utterances[7].rawText == "발화 7")
    }
}
