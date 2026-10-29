import Foundation
import Testing
@testable import GroveInference

struct ExternalOutputDecoderTests {
    @Test func confirmedEmptyCompletedAudioHasAnExplicitNoSpeechOutcome() throws {
        let input = Data(#"{"hitTokenLimit":false,"hasUnparsedText":false,"processedAudioFrames":32000,"inputAudioFrames":32000,"segments":[]}"#.utf8)
        do {
            _ = try ExternalOutputDecoder.moss(input, duration: 2)
            Issue.record("Expected noSpeech")
        } catch InferenceError.noSpeech {
            // The app may omit a genuinely silent channel, never a capped/partial one.
        }
    }

    @Test func mossFormattingLabelIsMetadataNotSpokenText() throws {
        let input = Data(#"{"segments":[{"start":0,"end":1,"text":"[S01] 실제 [S01] 표기는 유지","speaker_id":"S01"},{"start":1,"end":2,"text":"[S02] 다른 표기는 유지","speaker_id":"S01"},{"start":2,"end":3,"text":"[S01] 메타데이터 없음"}]}"#.utf8)
        let result = try ExternalOutputDecoder.moss(input, duration: 3)
        #expect(result.utterances.map(\.text) == ["실제 [S01] 표기는 유지", "[S02] 다른 표기는 유지", "[S01] 메타데이터 없음"])
        #expect(result.utterances[0].asrClusterID == "S01")
    }

    @Test func parsesBothNumericAndStringClusterLabels() throws {
        let moss = Data(#"{"segments":[{"start":0,"end":1,"text":"원문","speaker_id":2,"asr_chunk":0},{"start":1,"end":2,"text":"다음","speaker_id":"A","asr_chunk":1}]}"#.utf8)
        let output = try ExternalOutputDecoder.moss(moss, duration: 2)
        #expect(output.utterances.map(\.asrClusterID) == ["2", "A"])
        #expect(output.utterances.map(\.asrChunkIndex) == [0, 1])
        let nemotron = Data(#"{"segments":[{"start":0,"end":1,"speaker":5}]}"#.utf8)
        #expect(try ExternalOutputDecoder.diarization(nemotron, engine: .nemotron3, duration: 2).first?.clusterID == "5")
    }

    @Test func decoderRejectsAnotherEngineShape() throws {
        let input = Data(#"{"segments":[{"startTimeSeconds":0,"endTimeSeconds":1,"speaker":"speaker_0"}]}"#.utf8)
        #expect(throws: (any Error).self) { try ExternalOutputDecoder.diarization(input, engine: .nemotron3, duration: 2) }
        let retired = Data(#"{"segments":[{"start":0,"end":1,"speaker":1}]}"#.utf8)
        #expect(throws: InferenceError.self) { try ExternalOutputDecoder.diarization(retired, engine: .community1, duration: 2) }
    }

    @Test func rejectsPartialMalformedAndOutOfBoundsResults() {
        let capped = Data(#"{"hitTokenLimit":true,"segments":[{"start":0,"end":1,"text":"미완료"}]}"#.utf8)
        #expect(throws: InferenceError.self) { try ExternalOutputDecoder.moss(capped, duration: 2) }
        let partial = Data(#"{"hasUnparsedText":true,"segments":[{"start":0,"end":1,"text":"뒤 문장 미완료"}]}"#.utf8)
        #expect(throws: InferenceError.self) { try ExternalOutputDecoder.moss(partial, duration: 2) }
        let unfinished = Data(#"{"processedAudioFrames":16000,"inputAudioFrames":32000,"segments":[{"start":0,"end":1,"text":"일부만 처리"}]}"#.utf8)
        #expect(throws: InferenceError.self) { try ExternalOutputDecoder.moss(unfinished, duration: 2) }
        let accumulated = Data(#"{"generationTokens":9000,"hitTokenLimit":false,"hasUnparsedText":false,"processedAudioFrames":32000,"inputAudioFrames":32000,"segments":[{"start":0,"end":1,"text":"정상 완료"}]}"#.utf8)
        #expect(throws: Never.self) { try ExternalOutputDecoder.moss(accumulated, duration: 2) }
        let late = Data(#"{"segments":[{"start":0,"end":20,"speaker":1}]}"#.utf8)
        #expect(throws: InferenceError.self) { try ExternalOutputDecoder.diarization(late, engine: .nemotron3, duration: 2) }
    }
}
