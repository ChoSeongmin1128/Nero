import Foundation
import GroveInference

protocol MeetingInferenceRunning: Sendable {
    @MainActor func preparationIssue(configuration: InferenceConfiguration, appleReady: Bool) -> String?
    @MainActor func validatePreparation(configuration: InferenceConfiguration) async throws
    func run(source: URL, configuration: InferenceConfiguration, directory: URL,
             progress: @Sendable (String) async -> Void) async throws -> InferenceResult
}

extension MeetingInferenceRunning {
    @MainActor func preparationIssue(configuration: InferenceConfiguration, appleReady: Bool) -> String? { nil }
    @MainActor func validatePreparation(configuration: InferenceConfiguration) async throws { _ = try configuration.resolvedEngine() }
}

struct BundledMeetingInferenceService: MeetingInferenceRunning {
    let appBundle: URL
    let applicationSupport: URL
    // UI checks read the app's shared model state instead of rescanning model files on every render.
    var modelsReady: (@MainActor @Sendable () -> Bool)? = nil

    @MainActor func preparationIssue(configuration: InferenceConfiguration, appleReady: Bool) -> String? {
        issue(configuration: configuration, appleReady: appleReady, modelsReady: modelsReady?() ?? freshModelsReady)
    }

    @MainActor func validatePreparation(configuration: InferenceConfiguration) async throws {
        if configuration.transcriptionEngine == .apple {
            try await AppleTranscriptionService.validatePreparation()
        } else if let issue = issue(configuration: configuration, appleReady: false, modelsReady: freshModelsReady) {
            throw InferenceError.invalidOutput(issue)
        }
    }

    @MainActor private var freshModelsReady: Bool { ModelManager(baseDirectory: applicationSupport).isReadyForUse }

    @MainActor private func issue(configuration: InferenceConfiguration, appleReady: Bool, modelsReady: Bool) -> String? {
        if configuration.transcriptionEngine == .apple {
            return appleReady ? nil : TranscriptionError.assetsMissing.localizedDescription
        }
        guard modelsReady else { return "설정의 모델 화면에서 정밀 전사 모델을 준비해 주세요." }
        do {
            _ = try backend(configuration: configuration)
            return nil
        } catch { return error.localizedDescription }
    }

    func run(source: URL, configuration: InferenceConfiguration, directory: URL,
             progress: @Sendable (String) async -> Void) async throws -> InferenceResult {
        if configuration.transcriptionEngine == .apple {
            await progress("Mac 기본 전사로 대화를 옮기고 있습니다")
            let output = try await AppleTranscriptionService.transcribe(file: source, contextualStrings: [])
            guard !output.utterances.isEmpty else { throw InferenceError.noSpeech }
            let result = try InferenceResult(duration: output.duration, configuration: configuration,
                transcription: .init(utterances: output.utterances), rawDiarization: [])
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try JSONEncoder().encode(output).write(to: directory.appendingPathComponent("apple-transcript.json"), options: .withoutOverwriting)
            return result
        }
        let backend = try backend(configuration: configuration)
        return try await NativeInferencePipeline(backend: backend).runRecording(
            source: source, configuration: configuration, directory: directory, progress: progress)
    }

    func backend(configuration: InferenceConfiguration) throws -> NativeInferenceBackend {
        let helpers = appBundle.appendingPathComponent("Contents/Helpers", isDirectory: true)
        let moss = helpers.appendingPathComponent("Moss.bundle/Contents/MacOS/MossHarness")
        let nemotron = helpers.appendingPathComponent("Nemotron/bin/nemo-speech")
        let engine = try configuration.resolvedEngine()
        let diarizer: URL
        switch engine {
        case .none: diarizer = moss
        case .nemotron3: diarizer = nemotron
        case .sortformerStreaming, .ultra8, .community1: throw InferenceError.unsupportedEngine
        }
        for executable in [moss, diarizer] {
            guard FileManager.default.isExecutableFile(atPath: executable.path) else {
                throw InferenceError.missingExecutable(executable.lastPathComponent)
            }
        }
        let nemotronModel = NemotronModel.url(in: applicationSupport)
        if engine == .nemotron3 && !FileManager.default.isReadableFile(atPath: nemotronModel.path) {
            throw InferenceError.invalidOutput("설정의 모델 화면에서 기본 모델을 준비해 주세요.")
        }
        let model = MossModel.directory(in: applicationSupport)
        for file in MossModel.requiredFiles {
            guard FileManager.default.isReadableFile(atPath: model.appendingPathComponent(file).path) else {
                throw InferenceError.invalidOutput("설정의 모델 화면에서 전사 모델을 먼저 준비해 주세요.")
            }
        }
        return NativeInferenceBackend(mossExecutable: moss, mossModelDirectory: model,
                                      nemotronExecutable: nemotron, nemotronModel: nemotronModel)
    }
}

enum MeetingProcessingMode: String, Codable, CaseIterable, Identifiable {
    case automatic
    case manualCount

    var id: String { rawValue }
    var label: String {
        switch self {
        case .automatic: "자동 추정"
        case .manualCount: "인원 입력"
        }
    }
    init(configuration: InferenceConfiguration?) {
        if configuration?.expectedSpeakerCount != nil {
            self = .manualCount
        } else { self = .automatic }
    }
}

struct ChannelInference: Codable, Sendable {
    let sourceID: String
    let result: InferenceResult

    var speakerCountWarning: String? {
        guard let expected = result.configuration.expectedSpeakerCount else { return nil }
        let detected = Set(result.rawDiarization.map(\.clusterID)).count
        guard expected != detected else { return nil }
        let prefix = sourceID == "system" ? "컴퓨터 소리: " : sourceID == "microphone" ? "마이크: " : ""
        return "\(prefix)입력한 인원은 \(expected)명이지만 감지된 화자는 \(detected)명입니다. 화자 배정을 확인해 주세요."
    }
}

struct MeetingInferenceArchive: Codable {
    let revisionID: UUID
    let channels: [ChannelInference]
}

extension TranscriptDocument {
    static func preservingChannels(_ channels: [ChannelInference]) throws -> TranscriptDocument {
        guard !channels.isEmpty, Set(channels.map(\.sourceID)).count == channels.count else {
            throw TranscriptEditError.invalidDocument
        }
        if channels.count == 1, let channel = channels.first {
            return try preservingInference(channel.result, sourceChannelID: channel.sourceID)
        }
        var speakers: [MeetingSpeaker] = []
        var utterances: [DocumentUtterance] = []
        for channel in channels {
            let document = try preservingInference(channel.result, sourceChannelID: channel.sourceID)
            // Channel-local clusters do not identify the same person across recordings.
            // Keep separate IDs even when both workers label a cluster "Speaker 0".
            for var speaker in document.speakers {
                speaker.order = speakers.count
                speaker.name = MeetingSpeaker.defaultName(order: speakers.count)
                speakers.append(speaker)
            }
            utterances.append(contentsOf: document.utterances)
        }
        return try TranscriptDocument(speakers: speakers, utterances: utterances.sorted { $0.startTime < $1.startTime },
            sourceDiarizationEngines: Dictionary(uniqueKeysWithValues: channels.map { ($0.sourceID, $0.result.diarizationEngine) }))
    }
}
