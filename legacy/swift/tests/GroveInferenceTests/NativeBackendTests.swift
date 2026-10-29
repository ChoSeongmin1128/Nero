import Foundation
import Testing
@testable import GroveInference

struct NativeBackendTests {
    @Test func retiredEnginesAreReadableButNeverRunAgain() async throws {
        let audio = URL(fileURLWithPath: "/tmp/input.wav")
        let output = URL(fileURLWithPath: "/tmp/output.json")
        for configuration in [InferenceConfiguration(expectedSpeakerCount: 4, diarizationPreference: .sortformerStreaming),
                              .init(diarizationPreference: .ultra8), .init(expectedSpeakerCount: 4),
                              .init(diarizationPreference: .community1),
                              .init(expectedSpeakerCount: 5, speakerCountPolicy: .exact)] {
            #expect(throws: InferenceError.self) {
                try NativeInferenceBackend.diarizationArguments(audio: audio, output: output, configuration: configuration)
            }
        }
        let turns = Data(#"{"segments":[{"start":0,"end":1,"speaker":"0"}]}"#.utf8)
        #expect(throws: InferenceError.self) { try ExternalOutputDecoder.diarization(turns, engine: .ultra8, duration: 2) }
    }

    @Test func knownCountIsOnlyAdvisory() throws {
        let advisory = try NativeInferenceBackend.diarizationArguments(audio: URL(fileURLWithPath: "/tmp/input.wav"),
            output: URL(fileURLWithPath: "/tmp/output.json"),
            configuration: .init(expectedSpeakerCount: 5, diarizationPreference: .nemotron3),
            nemotronModel: URL(fileURLWithPath: "/tmp/model.gguf"))
        #expect(!advisory.contains("--num-speakers"))
    }

    @Test func unsupportedExactCountDoesNotSilentlyFallBackToAdvisory() throws {
        #expect(try InferenceConfiguration(expectedSpeakerCount: 3, speakerCountPolicy: .exact).resolvedEngine() == .community1)
        #expect(throws: InferenceError.self) {
            try InferenceConfiguration(expectedSpeakerCount: 3, diarizationPreference: .sortformerStreaming,
                                       speakerCountPolicy: .exact).resolvedEngine()
        }
        #expect(throws: InferenceError.self) { try InferenceConfiguration(speakerCountPolicy: .exact).resolvedEngine() }
    }

    @Test func runnerCapturesOutputAndDoesNotOverwriteLogs() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let log = root.appendingPathComponent("result.log")
        let runner = NativeProcessRunner()
        try await runner.run(executable: URL(fileURLWithPath: "/usr/bin/printf"), arguments: ["test output"], directory: root, log: log)
        #expect(try String(contentsOf: log, encoding: .utf8) == "test output")
        await #expect(throws: (any Error).self) {
            try await runner.run(executable: URL(fileURLWithPath: "/usr/bin/printf"), arguments: ["replacement"], directory: root, log: log)
        }
        #expect(try String(contentsOf: log, encoding: .utf8) == "test output")
    }

    @Test func cancelledWorkerTerminatesWithoutPublishingSuccess() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let task = Task {
            try await NativeProcessRunner().run(executable: URL(fileURLWithPath: "/bin/sleep"), arguments: ["30"],
                                                directory: root, log: root.appendingPathComponent("cancelled.log"))
        }
        try await Task.sleep(for: .milliseconds(100))
        task.cancel()
        await #expect(throws: CancellationError.self) { try await task.value }
    }
}
