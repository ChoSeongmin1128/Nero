@preconcurrency import AVFoundation
import Foundation
import Darwin
@preconcurrency import Speech

enum Variant: String, Codable { case speechTranscriber, dictationTranscriber }
struct Availability: Codable {
    let supported: Bool
    let ready: Bool
    let locale: String
    let assetStatus: String
    let message: String
    let variant: Variant?
    let speechAvailable: Bool
    let speechLocaleSupported: Bool
    let dictationLocaleSupported: Bool
}
struct NativeSegment: Codable {
    let start: Double
    let end: Double
    let text: String
}
struct NativeOutput: Codable {
    let schemaVersion: Int
    let variant: Variant
    let locale: String
    let systemVersion: String
    let inputFrames: Int64
    let consumedInputFrames: Int64
    let sampleRate: Double
    let finalized: Bool
    let finalizationTimeSeconds: Double
    let segments: [NativeSegment]
}
enum Failure: Error { case unsupported, assetsMissing, invalidInput, incomplete, invalidArguments }
enum Module {
    case speech(SpeechTranscriber)
    case dictation(DictationTranscriber)
    var base: any LocaleDependentSpeechModule {
        switch self { case .speech(let module): module; case .dictation(let module): module }
    }
    func collect() async throws -> [NativeSegment] {
        var segments: [NativeSegment] = []
        switch self {
        case .speech(let module):
            for try await result in module.results {
                guard result.isFinal else { continue }
                let text = String(result.text.characters)
                guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { continue }
                segments.append(NativeSegment(start: CMTimeGetSeconds(result.range.start), end: CMTimeGetSeconds(CMTimeRangeGetEnd(result.range)), text: text))
            }
        case .dictation(let module):
            for try await result in module.results {
                guard result.isFinal else { continue }
                let text = String(result.text.characters)
                guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { continue }
                segments.append(NativeSegment(start: CMTimeGetSeconds(result.range.start), end: CMTimeGetSeconds(CMTimeRangeGetEnd(result.range)), text: text))
            }
        }
        return segments
    }
}
@main struct AppleSpeech {
    static func module(_ variant: Variant) async throws -> Module {
        let locale = Locale(identifier: "ko-KR")
        switch variant {
        case .speechTranscriber:
            guard SpeechTranscriber.isAvailable,
                  let locale = await SpeechTranscriber.supportedLocale(equivalentTo: locale) else { throw Failure.unsupported }
            return .speech(SpeechTranscriber(locale: locale, transcriptionOptions: [], reportingOptions: [], attributeOptions: [.audioTimeRange]))
        case .dictationTranscriber:
            guard let locale = await DictationTranscriber.supportedLocale(equivalentTo: locale) else { throw Failure.unsupported }
            return .dictation(DictationTranscriber(locale: locale, preset: .timeIndexedLongDictation))
        }
    }
    static func availability(_ preferred: Variant?) async -> Availability {
        let available = SpeechTranscriber.isAvailable
        let speechLocale = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: "ko-KR")) != nil
        let dictationLocale = await DictationTranscriber.supportedLocale(equivalentTo: Locale(identifier: "ko-KR")) != nil
        let selected = preferred ?? (available && speechLocale ? .speechTranscriber : dictationLocale ? .dictationTranscriber : nil)
        var name = "unsupported"
        if let selected, let module = try? await module(selected) {
            switch await AssetInventory.status(forModules: [module.base]) {
            case .unsupported: name = "unsupported"
            case .supported: name = "supported"
            case .downloading: name = "downloading"
            case .installed: name = "installed"
            @unknown default: name = "unknown"
            }
        }
        let supported = name != "unsupported" && name != "unknown"
        return Availability(supported: supported, ready: name == "installed", locale: "ko-KR", assetStatus: name,
            message: !supported ? "이 환경에서는 한국어 기본 전사를 사용할 수 없습니다." : name == "installed" ? "macOS 기본 전사를 사용할 수 있습니다." : "준비 시 macOS 한국어 언어 자산을 다운로드할 수 있습니다.",
            variant: selected, speechAvailable: available, speechLocaleSupported: speechLocale, dictationLocaleSupported: dictationLocale)
    }
    static func prepare(_ variant: Variant) async throws -> Availability {
        let module = try await module(variant)
        if await AssetInventory.status(forModules: [module.base]) != .installed {
            guard let locale = module.base.selectedLocales.first, try await AssetInventory.reserve(locale: locale) else { throw Failure.assetsMissing }
            if let request = try await AssetInventory.assetInstallationRequest(supporting: [module.base]) {
                try await withTaskCancellationHandler { try await request.downloadAndInstall() }
                    onCancel: { request.progress.cancel() }
            }
        }
        try Task.checkCancellation()
        let status = await availability(variant)
        guard status.ready else { throw Failure.assetsMissing }
        return status
    }
    static func transcribe(_ path: String, variant: Variant) async throws -> NativeOutput {
        let module = try await module(variant)
        guard await AssetInventory.status(forModules: [module.base]) == .installed else { throw Failure.assetsMissing }
        let audio = try AVAudioFile(forReading: URL(fileURLWithPath: path))
        guard audio.length > 0, audio.processingFormat.sampleRate == 16000,
              audio.processingFormat.channelCount == 1 else { throw Failure.invalidInput }
        let frames = audio.length
        let analyzer = SpeechAnalyzer(modules: [module.base])
        let results = Task { try await module.collect() }
        return try await withTaskCancellationHandler {
            do {
                guard let last = try await analyzer.analyzeSequence(from: audio), audio.framePosition == frames else { throw Failure.incomplete }
                try await analyzer.finalizeAndFinish(through: last)
                let segments = try await results.value
                try Task.checkCancellation()
                return NativeOutput(schemaVersion: 1, variant: variant, locale: "ko-KR", systemVersion: ProcessInfo.processInfo.operatingSystemVersionString,
                    inputFrames: frames, consumedInputFrames: audio.framePosition, sampleRate: audio.processingFormat.sampleRate,
                    finalized: true, finalizationTimeSeconds: CMTimeGetSeconds(last), segments: segments)
            } catch {
                results.cancel()
                await analyzer.cancelAndFinishNow()
                throw error
            }
        } onCancel: {
            results.cancel()
            Task { await analyzer.cancelAndFinishNow() }
        }
    }
    static func write<T: Encodable>(_ value: T, path: String?) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(value)
        if let path {
            let descriptor = Darwin.open(path, O_WRONLY | O_CREAT | O_EXCL, 0o600)
            guard descriptor >= 0 else { throw Failure.invalidArguments }
            let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
            try handle.write(contentsOf: data)
            try handle.synchronize()
            try handle.close()
        } else { FileHandle.standardOutput.write(data) }
    }
    static func main() async {
        do {
            let arguments = CommandLine.arguments
            let operation = Task {
                switch arguments.dropFirst().first {
                case "status" where (2...3).contains(arguments.count):
                    let preferred = arguments.count == 3 ? Variant(rawValue: arguments[2]) : nil
                    if arguments.count == 3 && preferred == nil { throw Failure.invalidArguments }
                    try write(await availability(preferred), path: nil)
                case "prepare" where arguments.count == 4:
                    guard let variant = Variant(rawValue: arguments[2]) else { throw Failure.invalidArguments }
                    try write(try await prepare(variant), path: arguments[3])
                case "transcribe" where arguments.count == 5:
                    guard let variant = Variant(rawValue: arguments[2]) else { throw Failure.invalidArguments }
                    try write(try await transcribe(arguments[3], variant: variant), path: arguments[4])
                default: throw Failure.invalidArguments
                }
            }
            let cancellation = Task.detached {
                if let data = try? FileHandle.standardInput.read(upToCount: 1), !data.isEmpty { operation.cancel() }
            }
            defer { cancellation.cancel() }
            try await operation.value
        } catch {
            FileHandle.standardError.write(Data("Apple speech operation failed: \(error)\n".utf8))
            exit(1)
        }
    }
}
