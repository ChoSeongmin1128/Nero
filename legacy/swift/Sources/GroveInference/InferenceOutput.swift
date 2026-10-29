import Foundation

public struct RecognizedUtterance: Identifiable, Codable, Hashable, Sendable {
    public let id: UUID
    public let start: Double
    public let end: Double
    public let text: String
    public let asrClusterID: String?
    public let asrChunkIndex: Int?

    public init(id: UUID = UUID(), start: Double, end: Double, text: String, asrClusterID: String? = nil, asrChunkIndex: Int? = nil) {
        self.id = id
        self.start = start
        self.end = end
        self.text = text
        self.asrClusterID = asrClusterID
        self.asrChunkIndex = asrChunkIndex
    }
}

public struct DiarizationTurn: Codable, Hashable, Sendable {
    public let start: Double
    public let end: Double
    public let clusterID: String

    public init(start: Double, end: Double, clusterID: String) {
        self.start = start
        self.end = end
        self.clusterID = clusterID
    }
}

public struct RawTranscription: Codable, Hashable, Sendable {
    public let utterances: [RecognizedUtterance]
    public let hitTokenLimit: Bool

    public init(utterances: [RecognizedUtterance], hitTokenLimit: Bool = false) {
        self.utterances = utterances
        self.hitTokenLimit = hitTokenLimit
    }

    public func validate(duration: Double) throws {
        guard !hitTokenLimit else { throw InferenceError.incompleteTranscription }
        guard duration.isFinite, duration > 0, !utterances.isEmpty,
              Set(utterances.map(\.id)).count == utterances.count,
              utterances.allSatisfy({
                  $0.start.isFinite && $0.end.isFinite && $0.start >= 0 && $0.end >= $0.start
                      && $0.end <= duration + 0.5 && !$0.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
              }) else { throw InferenceError.invalidOutput("전사 구간 또는 시각이 올바르지 않습니다.") }
    }
}

public enum AssignmentReviewReason: String, Codable, Hashable, Sendable {
    case noDiarizationCoverage
    case ambiguousSpeakers
    case multipleSpeakersInUtterance
}

public struct UtteranceAssignment: Codable, Hashable, Sendable {
    public let utteranceID: UUID
    public let clusterID: String?
    public let overlapSecondsByCluster: [String: Double]
    public let reviewReasons: Set<AssignmentReviewReason>
}

public struct InferenceResult: Codable, Hashable, Sendable {
    public let schemaVersion: Int
    public let jobID: UUID
    public let duration: Double
    public let configuration: InferenceConfiguration
    public let diarizationEngine: DiarizationEngine
    public let transcription: RawTranscription
    public let rawDiarization: [DiarizationTurn]
    public let assignments: [UtteranceAssignment]

    public init(jobID: UUID = UUID(), duration: Double, configuration: InferenceConfiguration,
                transcription: RawTranscription, rawDiarization: [DiarizationTurn]) throws {
        try transcription.validate(duration: duration)
        let engine = try configuration.resolvedEngine()
        try SpeakerProjection.validate(rawDiarization, duration: duration, engine: engine)
        schemaVersion = 1
        self.jobID = jobID
        self.duration = duration
        self.configuration = configuration
        self.diarizationEngine = engine
        self.transcription = transcription
        self.rawDiarization = rawDiarization
        self.assignments = Self.project(transcription.utterances, turns: rawDiarization, engine: engine)
    }

    public func validate() throws {
        var recordedConfiguration = configuration
        if configuration.diarizationPreference == .automatic {
            // Automatic routing can change; saved results retain their actual engine.
            switch diarizationEngine {
            case .none: recordedConfiguration.diarizationPreference = .none
            case .nemotron3: recordedConfiguration.diarizationPreference = .nemotron3
            case .sortformerStreaming: recordedConfiguration.diarizationPreference = .sortformerStreaming
            case .ultra8: recordedConfiguration.diarizationPreference = .ultra8
            case .community1: recordedConfiguration.diarizationPreference = .community1
            }
        }
        guard schemaVersion == 1, try recordedConfiguration.resolvedEngine() == diarizationEngine else {
            throw InferenceError.invalidOutput("처리 결과의 버전 또는 엔진이 일치하지 않습니다.")
        }
        try transcription.validate(duration: duration)
        try SpeakerProjection.validate(rawDiarization, duration: duration, engine: diarizationEngine)
        guard assignments == Self.project(transcription.utterances, turns: rawDiarization, engine: diarizationEngine) else {
            throw InferenceError.invalidOutput("원본 화자 구간과 발화 배정이 일치하지 않습니다.")
        }
    }

    private static func project(_ utterances: [RecognizedUtterance], turns: [DiarizationTurn], engine: DiarizationEngine) -> [UtteranceAssignment] {
        if engine == .none {
            return utterances.map { .init(utteranceID: $0.id, clusterID: nil, overlapSecondsByCluster: [:], reviewReasons: []) }
        }
        return SpeakerProjection.assign(utterances, turns: turns)
    }
}
