import Foundation
import Testing
@testable import GroveInference

struct NemotronContractTests {
    @Test func eightSpeakerAutomaticSupportsAdvisoryButRejectsExactAndNine() throws {
        let config = InferenceConfiguration(expectedSpeakerCount: 5, diarizationPreference: .nemotron3)
        #expect(try config.resolvedEngine() == .nemotron3)
        #expect(throws: InferenceError.self) { try InferenceConfiguration(expectedSpeakerCount: 9, diarizationPreference: .nemotron3).resolvedEngine() }
        #expect(throws: InferenceError.self) { try InferenceConfiguration(expectedSpeakerCount: 5, diarizationPreference: .nemotron3, speakerCountPolicy: .exact).resolvedEngine() }
        let args = try NativeInferenceBackend.diarizationArguments(audio: URL(fileURLWithPath: "/input.wav"),
            output: URL(fileURLWithPath: "/output.json"), configuration: config, nemotronModel: URL(fileURLWithPath: "/local.gguf"))
        #expect(args.contains("v3-offline") && args.contains("metal") && args.contains("--no-batching"))
        #expect(!args.contains("--num-speakers"))
    }
    @Test func decoderRejectsOutOfRangeSpeakersAndPreservesOverlaps() throws {
        let data = Data(#"{"segments":[{"start":0,"end":1,"speaker":1},{"start":0.5,"end":1.5,"speaker":8}]}"#.utf8)
        let turns = try ExternalOutputDecoder.diarization(data, engine: .nemotron3, duration: 2)
        #expect(turns.count == 2 && turns[1].clusterID == "8")
        for speaker in [0, 9] {
            let bad = Data("{\"segments\":[{\"start\":0,\"end\":1,\"speaker\":\(speaker)}]}".utf8)
            #expect(throws: (any Error).self) { try ExternalOutputDecoder.diarization(bad, engine: .nemotron3, duration: 2) }
        }
    }
}
