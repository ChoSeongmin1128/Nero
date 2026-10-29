import AVFoundation
import Foundation
import Speech

struct BenchmarkRecord: Codable {
    let id: String
    let engine: String
    let locale: String
    let audioPath: String
    let audioDurationSeconds: Double
    let processingSeconds: Double
    let realTimeFactor: Double
    let conditioningMode: String
    let contextualStringCount: Int
    let text: String?
    let meanConfidence: Double?
    let minimumConfidence: Double?
    let confidenceSampleCount: Int
    let error: String?
}

private struct TranscriptionOutput: Sendable {
    let text: String
    let confidences: [Double]
}

enum BenchmarkError: Error, CustomStringConvertible {
    case usage(String)
    case unsupportedLocale(String)

    var description: String {
        switch self {
        case .usage(let message), .unsupportedLocale(let message): message
        }
    }
}

@main
struct AppleBenchmark {
    static func main() async {
        do {
            let configuration = try parseArguments()
            try await run(configuration: configuration)
        } catch {
            FileHandle.standardError.write(Data("error: \(error)\n".utf8))
            exit(1)
        }
    }

    private struct Configuration {
        let audioDirectory: URL
        let outputFile: URL
        let localeIdentifier: String
        let limit: Int?
        let contextFile: URL?
        let conditioningMode: String
    }

    private static func parseArguments() throws -> Configuration {
        var arguments = Array(CommandLine.arguments.dropFirst())
        var values: [String: String] = [:]

        while !arguments.isEmpty {
            let key = arguments.removeFirst()
            guard key.hasPrefix("--"), !arguments.isEmpty else {
                throw BenchmarkError.usage(usage)
            }
            values[key] = arguments.removeFirst()
        }

        guard let audioDirectory = values["--audio-dir"],
              let outputFile = values["--output"] else {
            throw BenchmarkError.usage(usage)
        }

        return Configuration(
            audioDirectory: URL(fileURLWithPath: audioDirectory),
            outputFile: URL(fileURLWithPath: outputFile),
            localeIdentifier: values["--locale"] ?? "ko-KR",
            limit: values["--limit"].flatMap(Int.init),
            contextFile: values["--context-file"].map(URL.init(fileURLWithPath:)),
            conditioningMode: values["--conditioning-mode"] ?? "baseline"
        )
    }

    private static let usage = """
    Usage: grove-apple-benchmark \\
      --audio-dir <directory> \\
      --output <results.jsonl> \\
      [--locale ko-KR] [--limit count]
      [--context-file <json-array>] [--conditioning-mode <label>]
    """

    private static func run(configuration: Configuration) async throws {
        let requestedLocale = Locale(identifier: configuration.localeIdentifier)
        guard let locale = await SpeechTranscriber.supportedLocale(equivalentTo: requestedLocale) else {
            throw BenchmarkError.unsupportedLocale(
                "SpeechTranscriber does not support \(configuration.localeIdentifier) on this Mac"
            )
        }

        var audioFiles = try FileManager.default.contentsOfDirectory(
            at: configuration.audioDirectory,
            includingPropertiesForKeys: nil,
            options: [.skipsHiddenFiles]
        ).filter { $0.pathExtension.lowercased() == "wav" }
        audioFiles.sort { $0.lastPathComponent < $1.lastPathComponent }
        if let limit = configuration.limit {
            audioFiles = Array(audioFiles.prefix(limit))
        }
        guard !audioFiles.isEmpty else {
            throw BenchmarkError.usage("No WAV files found at \(configuration.audioDirectory.path)")
        }
        let contextualStrings = try loadContextualStrings(from: configuration.contextFile)

        try FileManager.default.createDirectory(
            at: configuration.outputFile.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        FileManager.default.createFile(atPath: configuration.outputFile.path, contents: nil)
        let output = try FileHandle(forWritingTo: configuration.outputFile)
        defer { try? output.close() }

        // Warm the system-managed model and shared backing engine. The measured
        // pass below still includes per-file analyzer setup, which is part of the
        // app-visible cost for this clip-oriented dataset.
        _ = try? await transcribe(
            file: audioFiles[0],
            locale: locale,
            contextualStrings: contextualStrings
        )

        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]

        for (index, file) in audioFiles.enumerated() {
            let id = file.deletingPathExtension().lastPathComponent
            let audioFile = try AVAudioFile(forReading: file)
            let duration = Double(audioFile.length) / audioFile.processingFormat.sampleRate
            let started = ContinuousClock.now

            let record: BenchmarkRecord
            do {
                let transcription = try await transcribe(
                    file: file,
                    locale: locale,
                    contextualStrings: contextualStrings
                )
                let elapsed = seconds(since: started)
                record = BenchmarkRecord(
                    id: id,
                    engine: "apple-speechtranscriber",
                    locale: locale.identifier,
                    audioPath: file.path,
                    audioDurationSeconds: duration,
                    processingSeconds: elapsed,
                    realTimeFactor: duration > 0 ? elapsed / duration : 0,
                    conditioningMode: configuration.conditioningMode,
                    contextualStringCount: contextualStrings.count,
                    text: transcription.text,
                    meanConfidence: transcription.confidences.isEmpty
                        ? nil
                        : transcription.confidences.reduce(0, +) / Double(transcription.confidences.count),
                    minimumConfidence: transcription.confidences.min(),
                    confidenceSampleCount: transcription.confidences.count,
                    error: nil
                )
            } catch {
                let elapsed = seconds(since: started)
                record = BenchmarkRecord(
                    id: id,
                    engine: "apple-speechtranscriber",
                    locale: locale.identifier,
                    audioPath: file.path,
                    audioDurationSeconds: duration,
                    processingSeconds: elapsed,
                    realTimeFactor: duration > 0 ? elapsed / duration : 0,
                    conditioningMode: configuration.conditioningMode,
                    contextualStringCount: contextualStrings.count,
                    text: nil,
                    meanConfidence: nil,
                    minimumConfidence: nil,
                    confidenceSampleCount: 0,
                    error: String(describing: error)
                )
            }

            output.write(try encoder.encode(record))
            output.write(Data("\n".utf8))
            print("[\(index + 1)/\(audioFiles.count)] \(id) \(String(format: "%.3fs", record.processingSeconds))")
        }
    }

    private static func transcribe(
        file: URL,
        locale: Locale,
        contextualStrings: [String]
    ) async throws -> TranscriptionOutput {
        let audioFile = try AVAudioFile(forReading: file)
        let transcriber = SpeechTranscriber(
            locale: locale,
            transcriptionOptions: [],
            reportingOptions: [],
            attributeOptions: [.audioTimeRange, .transcriptionConfidence]
        )

        let analyzer = SpeechAnalyzer(modules: [transcriber])
        if !contextualStrings.isEmpty {
            let context = AnalysisContext()
            context.contextualStrings[.general] = contextualStrings
            try await analyzer.setContext(context)
        }

        let resultTask = Task { () throws -> TranscriptionOutput in
            var text = ""
            var confidences: [Double] = []
            for try await result in transcriber.results {
                text += String(result.text.characters)
                for run in result.text.runs {
                    if let confidence = run.transcriptionConfidence {
                        confidences.append(confidence)
                    }
                }
            }
            return TranscriptionOutput(text: text, confidences: confidences)
        }

        do {
            if let lastSample = try await analyzer.analyzeSequence(from: audioFile) {
                try await analyzer.finalizeAndFinish(through: lastSample)
            } else {
                await analyzer.cancelAndFinishNow()
            }
            return try await resultTask.value
        } catch {
            resultTask.cancel()
            await analyzer.cancelAndFinishNow()
            throw error
        }
    }

    private static func loadContextualStrings(from file: URL?) throws -> [String] {
        guard let file else { return [] }
        let values = try JSONDecoder().decode([String].self, from: Data(contentsOf: file))
        var seen: Set<String> = []
        return values.compactMap { value in
            let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty, seen.insert(trimmed).inserted else { return nil }
            return trimmed
        }
    }

    private static func seconds(since start: ContinuousClock.Instant) -> Double {
        let duration = start.duration(to: .now)
        return Double(duration.components.seconds)
            + Double(duration.components.attoseconds) / 1_000_000_000_000_000_000
    }
}
