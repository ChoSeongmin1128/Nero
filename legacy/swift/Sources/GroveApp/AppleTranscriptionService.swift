@preconcurrency import AVFoundation
import Foundation
import GroveInference
@preconcurrency import Speech

struct AppleTranscriptionOutput: Codable, Sendable {
    let text: String
    let meanConfidence: Double?
    let duration: TimeInterval
    let utterances: [RecognizedUtterance]
}

enum AppleTranscriptionService {
    static func validatePreparation() async throws {
        guard SpeechTranscriber.isAvailable,
              let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: "ko-KR")) else {
            throw TranscriptionError.unsupportedKorean
        }
        let transcriber = SpeechTranscriber(locale: locale, preset: .transcription)
        guard await AssetInventory.status(forModules: [transcriber]) == .installed else { throw TranscriptionError.assetsMissing }
    }
    static func transcribe(file: URL, contextualStrings: [String]) async throws -> AppleTranscriptionOutput {
        guard SpeechTranscriber.isAvailable,
              let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: "ko-KR")) else {
            throw TranscriptionError.unsupportedKorean
        }
        let transcriber = SpeechTranscriber(locale: locale, transcriptionOptions: [], reportingOptions: [],
            attributeOptions: [.audioTimeRange, .transcriptionConfidence])
        guard await AssetInventory.status(forModules: [transcriber]) == .installed else { throw TranscriptionError.assetsMissing }
        let audioFile = try AVAudioFile(forReading: file)
        let duration = Double(audioFile.length) / audioFile.processingFormat.sampleRate
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        if !contextualStrings.isEmpty {
            let context = AnalysisContext()
            context.contextualStrings[.general] = contextualStrings
            try await analyzer.setContext(context)
        }
        let resultTask = Task { () throws -> ([RecognizedUtterance], [Double]) in
            var utterances: [RecognizedUtterance] = []
            var confidences: [Double] = []
            for try await result in transcriber.results {
                guard result.isFinal else { continue }
                let text = String(result.text.characters).trimmingCharacters(in: .whitespacesAndNewlines)
                guard !text.isEmpty else { continue }
                utterances.append(.init(start: CMTimeGetSeconds(result.range.start),
                    end: CMTimeGetSeconds(CMTimeRangeGetEnd(result.range)), text: text))
                for run in result.text.runs {
                    if let confidence = run.transcriptionConfidence { confidences.append(confidence) }
                }
            }
            return (utterances, confidences)
        }
        return try await withTaskCancellationHandler {
            do {
                if let lastSample = try await analyzer.analyzeSequence(from: audioFile) {
                    try await analyzer.finalizeAndFinish(through: lastSample)
                } else { await analyzer.cancelAndFinishNow() }
                let (utterances, confidences) = try await resultTask.value
                try Task.checkCancellation()
                let confidence = confidences.isEmpty ? nil : confidences.reduce(0, +) / Double(confidences.count)
                return AppleTranscriptionOutput(text: utterances.map(\.text).joined(separator: " "),
                    meanConfidence: confidence, duration: duration, utterances: utterances)
            } catch {
                resultTask.cancel()
                await analyzer.cancelAndFinishNow()
                throw error
            }
        } onCancel: {
            resultTask.cancel()
            Task { await analyzer.cancelAndFinishNow() }
        }
    }
}

@MainActor
final class AppleTranscriptionPreparation: ObservableObject {
    @Published private(set) var isSupported = false
    @Published private(set) var isReady = false
    @Published private(set) var isPreparing = false
    @Published private(set) var hasChecked = false
    @Published private(set) var progress: Progress?
    @Published private(set) var message: String?
    private var task: Task<Void, Never>?

    func refresh() async {
        let candidate = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: "ko-KR"))
        isSupported = SpeechTranscriber.isAvailable && candidate != nil
        hasChecked = true
        guard let locale = candidate else { isReady = false; return }
        let module = SpeechTranscriber(locale: locale, preset: .transcription)
        isReady = await AssetInventory.status(forModules: [module]) == .installed
    }
    func prepare() {
        guard !isPreparing else { return }
        isPreparing = true
        task = Task {
            defer { isPreparing = false; task = nil }
            do {
                guard SpeechTranscriber.isAvailable, let locale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: "ko-KR")) else {
                    throw TranscriptionError.unsupportedKorean
                }
                guard try await AssetInventory.reserve(locale: locale) else { throw TranscriptionError.assetsMissing }
                let module = SpeechTranscriber(locale: locale, preset: .transcription)
                if let request = try await AssetInventory.assetInstallationRequest(supporting: [module]) {
                    progress = request.progress
                    message = "macOS 한국어 언어 자산을 준비하고 있습니다"
                    try await request.downloadAndInstall()
                }
                try Task.checkCancellation()
                await refresh()
                guard isReady else { throw TranscriptionError.assetsMissing }
                message = "Mac 기본 전사 사용 준비 완료"
            } catch is CancellationError { message = "준비를 중단했습니다." }
            catch { message = error.localizedDescription }
        }
    }
    func cancel() { progress?.cancel(); task?.cancel() }
    func cancelAndWait() async { cancel(); await task?.value }
}

enum TranscriptionError: LocalizedError {
    case unsupportedKorean, emptyResult, assetsMissing
    var errorDescription: String? {
        switch self {
        case .unsupportedKorean: "이 Mac에서는 Apple 한국어 기본 전사를 사용할 수 없습니다. 정밀 전사 모드를 선택해 주세요."
        case .emptyResult: "음성을 찾지 못했습니다. 입력 레벨과 마이크 장치를 확인해 주세요."
        case .assetsMissing: "macOS 한국어 언어 자산이 준비되지 않았습니다. 설정에서 Mac 기본 전사를 준비해 주세요."
        }
    }
}
