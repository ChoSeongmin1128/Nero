import Foundation

public enum ExternalOutputDecoder {
    private struct ClusterLabel: Decodable {
        let value: String
        init(from decoder: any Decoder) throws {
            let container = try decoder.singleValueContainer()
            if let text = try? container.decode(String.self) { value = text }
            else { value = String(try container.decode(Int.self)) }
        }
    }

    private struct MossEnvelope: Decodable {
        struct Segment: Decodable {
            let start: Double
            let end: Double
            let text: String
            let speaker_id: ClusterLabel?
            let asr_chunk: Int?
        }
        let segments: [Segment]
        let hitTokenLimit: Bool?
        let hasUnparsedText: Bool?
        let processedAudioFrames: Int64?
        let inputAudioFrames: Int64?
    }

    private struct SegmentsEnvelope: Decodable {
        struct Segment: Decodable {
            let start: Double
            let end: Double
            let speaker: ClusterLabel
        }
        let segments: [Segment]
    }

    public static func moss(_ data: Data, duration: Double) throws -> RawTranscription {
        let decoded = try JSONDecoder().decode(MossEnvelope.self, from: data)
        guard decoded.hasUnparsedText != true else { throw InferenceError.incompleteTranscription }
        if let processed = decoded.processedAudioFrames, let total = decoded.inputAudioFrames {
            guard total > 0, processed == total else { throw InferenceError.incompleteTranscription }
        } else if decoded.processedAudioFrames != nil || decoded.inputAudioFrames != nil {
            throw InferenceError.invalidOutput("전체 녹음 처리 여부를 확인할 수 없습니다.")
        }
        if decoded.segments.isEmpty, decoded.hitTokenLimit == false, decoded.hasUnparsedText == false,
           decoded.processedAudioFrames != nil {
            throw InferenceError.noSpeech
        }
        let result = RawTranscription(utterances: decoded.segments.map {
            RecognizedUtterance(start: $0.start, end: $0.end, text: mossBody($0.text, cluster: $0.speaker_id?.value),
                asrClusterID: $0.speaker_id?.value, asrChunkIndex: $0.asr_chunk)
        }, hitTokenLimit: decoded.hitTokenLimit ?? false)
        try result.validate(duration: duration)
        return result
    }

    private static func mossBody(_ text: String, cluster: String?) -> String {
        // MLXAudio 0.1.3 prepends this redundant formatting label in parseSegments.
        // Remove only the matching leading label, never bracketed words in the body.
        // The exact upstream text remains preserved in the worker's raw moss.json.
        guard let cluster, cluster.range(of: #"^S[0-9]+$"#, options: .regularExpression) != nil else { return text }
        let prefix = "[\(cluster)] "
        guard text.hasPrefix(prefix) else { return text }
        return String(text.dropFirst(prefix.count))
    }

    public static func diarization(_ data: Data, engine: DiarizationEngine, duration: Double) throws -> [DiarizationTurn] {
        let turns: [DiarizationTurn]
        switch engine {
        case .none: turns = []
        case .nemotron3:
            let decoded = try JSONDecoder().decode(SegmentsEnvelope.self, from: data)
            turns = decoded.segments.map { .init(start: $0.start, end: $0.end, clusterID: $0.speaker.value) }
            guard decoded.segments.allSatisfy({ (1...8).contains(Int($0.speaker.value) ?? 0) }) else {
                throw InferenceError.invalidOutput("화자 분리 모델의 화자 번호가 올바르지 않습니다.")
            }
        case .sortformerStreaming, .ultra8, .community1:
            throw InferenceError.unsupportedEngine
        }
        try SpeakerProjection.validate(turns, duration: duration, engine: engine)
        return turns
    }
}
