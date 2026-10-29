import Foundation

public enum DiarizationEngine: String, Codable, CaseIterable, Sendable {
    case none
    case nemotron3
    case sortformerStreaming
    case ultra8
    case community1
}

public enum DiarizationPreference: String, Codable, CaseIterable, Sendable {
    case none
    case nemotron3
    case automatic
    case sortformerStreaming
    case ultra8
    case community1
}

public enum SpeakerCountPolicy: String, Codable, Sendable {
    case advisory
    case exact
}

public enum TranscriptionEngine: String, Codable, CaseIterable, Sendable {
    case moss, apple
}

public struct InferenceConfiguration: Codable, Hashable, Sendable {
    public var expectedSpeakerCount: Int?
    public var diarizationPreference: DiarizationPreference
    public var speakerCountPolicy: SpeakerCountPolicy
    public var transcriptionEngine: TranscriptionEngine

    public init(expectedSpeakerCount: Int? = nil, diarizationPreference: DiarizationPreference = .automatic,
                speakerCountPolicy: SpeakerCountPolicy = .advisory, transcriptionEngine: TranscriptionEngine = .moss) {
        self.expectedSpeakerCount = expectedSpeakerCount
        self.diarizationPreference = diarizationPreference
        self.speakerCountPolicy = speakerCountPolicy
        self.transcriptionEngine = transcriptionEngine
    }

    private enum CodingKeys: String, CodingKey { case expectedSpeakerCount, diarizationPreference, speakerCountPolicy, transcriptionEngine }

    public init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        expectedSpeakerCount = try values.decodeIfPresent(Int.self, forKey: .expectedSpeakerCount)
        diarizationPreference = try values.decodeIfPresent(DiarizationPreference.self, forKey: .diarizationPreference) ?? .automatic
        speakerCountPolicy = try values.decodeIfPresent(SpeakerCountPolicy.self, forKey: .speakerCountPolicy) ?? .advisory
        transcriptionEngine = try values.decodeIfPresent(TranscriptionEngine.self, forKey: .transcriptionEngine) ?? .moss
    }

    public func resolvedEngine() throws -> DiarizationEngine {
        if let count = expectedSpeakerCount, count < 1 { throw InferenceError.invalidSpeakerCount }
        if speakerCountPolicy == .exact && expectedSpeakerCount == nil { throw InferenceError.invalidSpeakerCount }
        switch diarizationPreference {
        case .none:
            guard expectedSpeakerCount == nil, speakerCountPolicy == .advisory else { throw InferenceError.unsupportedCountConstraint }
            return .none
        case .nemotron3:
            if let count = expectedSpeakerCount, count > 8 { throw InferenceError.nemotronCapacity }
            if speakerCountPolicy == .exact { throw InferenceError.unsupportedCountConstraint }
            return .nemotron3
        case .automatic:
            if speakerCountPolicy == .exact { return .community1 }
            return expectedSpeakerCount.map { $0 <= 8 ? .ultra8 : .community1 } ?? .ultra8
        case .sortformerStreaming:
            if let count = expectedSpeakerCount, count > 4 { throw InferenceError.sortformerCapacity }
            if speakerCountPolicy == .exact { throw InferenceError.unsupportedCountConstraint }
            return .sortformerStreaming
        case .ultra8:
            if let count = expectedSpeakerCount, count > 8 { throw InferenceError.ultra8Capacity }
            if speakerCountPolicy == .exact { throw InferenceError.unsupportedCountConstraint }
            return .ultra8
        case .community1:
            return .community1
        }
    }
}

public enum InferenceError: Error, LocalizedError, Sendable {
    case invalidSpeakerCount
    case sortformerCapacity
    case ultra8Capacity
    case nemotronCapacity
    case unsupportedCountConstraint
    case invalidOutput(String)
    case incompleteTranscription
    case missingExecutable(String)
    case workerFailed(Int32)
    case jobAlreadyRunning
    case noSpeech
    case unsupportedEngine

    public var errorDescription: String? {
        switch self {
        case .invalidSpeakerCount: "발화자 수는 1명 이상이어야 합니다."
        case .sortformerCapacity: "이 Sortformer 모델은 최대 4명을 지원합니다. 5명 이상은 다인원 엔진을 선택해 주세요."
        case .ultra8Capacity: "Ultra8은 최대 8명을 지원합니다."
        case .nemotronCapacity: "Nemotron 3는 최대 8명까지 구분합니다. 8명 이하로 입력하거나 자동 추정을 선택해 주세요."
        case .unsupportedCountConstraint: "이 모델은 인원수 강제를 지원하지 않습니다."
        case .invalidOutput(let detail): "처리 결과를 확인할 수 없습니다. \(detail)"
        case .incompleteTranscription: "녹음 전체를 전사하지 못했습니다. 불완전한 결과를 완료된 전사로 저장하지 않았습니다."
        case .missingExecutable(let name): "처리에 필요한 구성 요소가 없습니다: \(name)"
        case .workerFailed(let status): "음성 처리 프로세스가 종료됐습니다. 종료 코드: \(status)"
        case .jobAlreadyRunning: "다른 회의를 처리하고 있습니다. 처리가 끝난 뒤 다시 시도해 주세요."
        case .noSpeech: "전사할 음성을 찾지 못했습니다. 원본 녹음은 보존됩니다."
        case .unsupportedEngine: "이 화자 분리 엔진은 더 이상 제공하지 않습니다. Nemotron 3로 다시 처리해 주세요."
        }
    }
}
