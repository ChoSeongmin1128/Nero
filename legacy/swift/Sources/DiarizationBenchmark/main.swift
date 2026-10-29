@preconcurrency import AVFoundation
import Foundation
import Darwin
import GroveInference

@main
struct DiarizationBenchmark {
    private struct Result: Encodable {
        let durationSeconds: Double
        let stageWallSeconds: Double
        let configuration: InferenceConfiguration
        let engine: DiarizationEngine
        let clusterCount: Int
        let turns: [DiarizationTurn]
    }

    static func main() async {
        do { try await run() }
        catch {
            FileHandle.standardError.write(Data("Grove benchmark: \(error.localizedDescription)\n".utf8))
            exit(1)
        }
    }

    private static func run() async throws {
        let args = Array(CommandLine.arguments.dropFirst())
        if args.count == 8, args[0] == "--pipeline", args[3] == "auto" || Int(args[3]) != nil {
            let directory = URL(fileURLWithPath: args[2], isDirectory: true)
            let backend = NativeInferenceBackend(mossExecutable: URL(fileURLWithPath: args[4]),
                mossModelDirectory: URL(fileURLWithPath: args[5], isDirectory: true),
                nemotronExecutable: URL(fileURLWithPath: args[6]), nemotronModel: URL(fileURLWithPath: args[7]))
            let pipeline = NativeInferencePipeline(backend: backend)
            let started = Date()
            let result = try await pipeline.runRecording(source: URL(fileURLWithPath: args[1]),
                configuration: .init(expectedSpeakerCount: Int(args[3]), diarizationPreference: .nemotron3),
                directory: directory) { stage in
                    print(stage)
                }
            try result.validate()
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            try encoder.encode(result).write(to: directory.appendingPathComponent("inference-result.json"), options: .withoutOverwriting)
            print("Completed native pipeline in \(Date().timeIntervalSince(started))s")
            return
        }
        if args.count == 3, args[0] == "--prepare-only" {
            let prepared = try await AnalysisAudioPreparer.prepare(source: URL(fileURLWithPath: args[1]),
                                                                   destination: URL(fileURLWithPath: args[2]))
            print("Prepared \(prepared.frameCount) frames at \(prepared.sampleRate)Hz; \(prepared.duration)s")
            return
        }
        guard args.count == 5, args[2] == "auto" || Int(args[2]) != nil else {
            throw NSError(domain: "GroveBenchmark", code: 1, userInfo: [NSLocalizedDescriptionKey:
                "Usage: grove-diarization-benchmark INPUT_WAV OUTPUT_DIRECTORY auto|SPEAKER_COUNT NEMOTRON_EXECUTABLE NEMOTRON_MODEL"])
        }
        let audio = URL(fileURLWithPath: args[0])
        let directory = URL(fileURLWithPath: args[1])
        let configuration = InferenceConfiguration(expectedSpeakerCount: Int(args[2]), diarizationPreference: .nemotron3)
        let file = try AVAudioFile(forReading: audio)
        let duration = Double(file.length) / file.processingFormat.sampleRate
        let backend = NativeInferenceBackend(mossExecutable: directory.appendingPathComponent("not-used"),
            mossModelDirectory: directory.appendingPathComponent("not-used-model"),
            nemotronExecutable: URL(fileURLWithPath: args[3]), nemotronModel: URL(fileURLWithPath: args[4]))
        let start = Date()
        let turns = try await backend.diarize(audio: audio, duration: duration, configuration: configuration, directory: directory)
        let result = Result(durationSeconds: duration, stageWallSeconds: Date().timeIntervalSince(start),
            configuration: configuration, engine: try configuration.resolvedEngine(),
            clusterCount: Set(turns.map { $0.clusterID }).count, turns: turns)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        try encoder.encode(result).write(to: directory.appendingPathComponent("native-result.json"), options: .withoutOverwriting)
        print("Completed \(result.engine.rawValue): \(result.clusterCount) clusters, \(result.stageWallSeconds)s")
    }
}
