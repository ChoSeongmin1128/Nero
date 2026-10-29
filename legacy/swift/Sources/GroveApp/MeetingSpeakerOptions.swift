import Foundation
import GroveInference

struct MeetingInferencePlan: Equatable, Sendable {
    let configuration: InferenceConfiguration
    var channelConfigurations: [String: InferenceConfiguration]? = nil

    func configurations(for sourceIDs: [String]) throws -> [String: InferenceConfiguration] {
        guard !sourceIDs.isEmpty, Set(sourceIDs).count == sourceIDs.count else { throw TranscriptEditError.invalidDocument }
        _ = try configuration.resolvedEngine()
        if let channelConfigurations {
            guard Set(channelConfigurations.keys) == Set(sourceIDs) else {
                throw InferenceError.invalidOutput("각 녹음 채널의 화자 수를 지정해 주세요.")
            }
            for value in channelConfigurations.values { _ = try value.resolvedEngine() }
            return channelConfigurations
        }
        guard sourceIDs.count == 1 || configuration.expectedSpeakerCount == nil else {
            throw InferenceError.invalidOutput("전체 인원수를 두 채널에 중복 적용할 수 없습니다. 채널별로 지정해 주세요.")
        }
        return Dictionary(uniqueKeysWithValues: sourceIDs.map { ($0, configuration) })
    }
}

struct MeetingSpeakerOptions: Codable, Equatable {
    var mode: MeetingProcessingMode = .automatic
    var transcriptionEngine: TranscriptionEngine = .moss
    var countText = ""
    var systemCountText = ""
    var microphoneCountText = ""

    init() {}

    private enum CodingKeys: String, CodingKey { case mode, countText, systemCountText, microphoneCountText, transcriptionEngine }

    init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        mode = try values.decodeIfPresent(MeetingProcessingMode.self, forKey: .mode) ?? .automatic
        transcriptionEngine = try values.decodeIfPresent(TranscriptionEngine.self, forKey: .transcriptionEngine) ?? .moss
        countText = try values.decodeIfPresent(String.self, forKey: .countText) ?? ""
        systemCountText = try values.decodeIfPresent(String.self, forKey: .systemCountText) ?? ""
        microphoneCountText = try values.decodeIfPresent(String.self, forKey: .microphoneCountText) ?? ""
    }

    init(configuration: InferenceConfiguration?, channels: [String: InferenceConfiguration]? = nil) {
        transcriptionEngine = configuration?.transcriptionEngine ?? .moss
        mode = .init(configuration: configuration)
        if let count = configuration?.expectedSpeakerCount { countText = String(count) }
        if let channels {
            if channels.values.contains(where: { $0.expectedSpeakerCount != nil }) { mode = .manualCount }
            if let count = channels["system"]?.expectedSpeakerCount { systemCountText = String(count) }
            if let count = channels["microphone"]?.expectedSpeakerCount { microphoneCountText = String(count) }
        }
    }

    var summary: String {
        if transcriptionEngine == .apple { return "Mac 기본 전사" }
        guard mode == .manualCount else { return mode.label }
        if let count = try? Self.parseCount(countText) {
            return "\(count)명 (참고)"
        }
        return "인원 입력 필요"
    }

    func plan(isDual: Bool) throws -> MeetingInferencePlan {
        if transcriptionEngine == .apple {
            return MeetingInferencePlan(configuration: .init(diarizationPreference: .none, transcriptionEngine: .apple))
        }
        if mode == .manualCount, isDual {
            let system = try Self.configuration(count: Self.parseCount(systemCountText))
            let microphone = try Self.configuration(count: Self.parseCount(microphoneCountText))
            let base = try Self.configuration(count: nil)
            return MeetingInferencePlan(configuration: base,
                channelConfigurations: ["system": system, "microphone": microphone])
        }
        return MeetingInferencePlan(configuration: try Self.configuration(
            count: mode == .manualCount ? Self.parseCount(countText) : nil))
    }

    // Saved results from retired engines stay readable, but every new run uses Nemotron 3;
    // an entered count is only compared with the result.
    static func configuration(count: Int?) throws -> InferenceConfiguration {
        let result = InferenceConfiguration(expectedSpeakerCount: count, diarizationPreference: .nemotron3)
        _ = try result.resolvedEngine()
        return result
    }

    func validationMessage(isDual: Bool) -> String? {
        do { _ = try plan(isDual: isDual); return nil }
        catch { return error.localizedDescription }
    }

    private static func parseCount(_ input: String) throws -> Int {
        let normalized = input.precomposedStringWithCompatibilityMapping.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty, normalized.allSatisfy({ $0.isASCII && $0.isNumber }),
              let count = Int(normalized), count > 0 else { throw InferenceError.invalidSpeakerCount }
        return count
    }
}
