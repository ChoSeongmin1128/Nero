import AVFoundation
import Foundation
import Testing
@testable import GroveInference

/// The executables below are protocol fixtures, not MOSS/diarization models.
/// These tests verify orchestration and failure safety, never model accuracy.
struct PipelineLifecycleTests {
    private func source(at root: URL) throws -> URL {
        let url = root.appendingPathComponent("input.wav")
        let format = try #require(AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false))
        let buffer = try #require(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 32_000))
        buffer.frameLength = 32_000
        for index in 0..<32_000 { buffer.floatChannelData![0][index] = 0 }
        let writer = try AVAudioFile(forWriting: url, settings: format.settings)
        try writer.write(from: buffer)
        return url
    }

    private func executable(_ name: String, at root: URL, script: String) throws -> URL {
        let url = root.appendingPathComponent(name)
        try Data(("#!/bin/sh\n" + script + "\n").utf8).write(to: url)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: url.path)
        return url
    }

    @Test func recordingPipelinePreservesInputAndRemovesOnlyItsWorkingCopy() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let input = try source(at: root)
        let original = try Data(contentsOf: input)
        let model = root.appendingPathComponent("local model with spaces")
        let asr = try executable("fixture-asr", at: root, script: #"test "$#" -eq 3 && test "${3##*/}" = 'local model with spaces' || exit 4; printf '%s' '{"hitTokenLimit":false,"segments":[{"start":0,"end":1,"text":"테스트"}]}' > "$2""#)
        let diarizer = try executable("fixture-diarizer", at: root,
            script: #"while [ $# -gt 0 ]; do [ "$1" = --output ] && out="$2"; shift; done; printf '%s' '{"segments":[{"start":0,"end":1,"speaker":1}]}' > "$out""#)
        let pipeline = NativeInferencePipeline(backend: .init(mossExecutable: asr, mossModelDirectory: model,
            nemotronExecutable: diarizer, nemotronModel: root.appendingPathComponent("model.gguf")))
        let job = root.appendingPathComponent("job")
        let result = try await pipeline.runRecording(source: input,
            configuration: .init(diarizationPreference: .nemotron3), directory: job)
        try result.validate()
        #expect(result.assignments[0].clusterID == "1")
        #expect(try Data(contentsOf: input) == original)
        #expect(!FileManager.default.fileExists(atPath: job.appendingPathComponent("analysis.wav").path))
        #expect(FileManager.default.fileExists(atPath: job.appendingPathComponent("moss.json").path))
        #expect(FileManager.default.fileExists(atPath: job.appendingPathComponent("diarization.json").path))
    }

    @Test func cappedAsrDoesNotStartDiarizationOrPublishAResult() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let input = try source(at: root)
        let asr = try executable("fixture-capped", at: root, script: #"printf '%s' '{"hitTokenLimit":true,"segments":[{"start":0,"end":1,"text":"미완료"}]}' > "$2""#)
        let missing = root.appendingPathComponent("must-not-run")
        let pipeline = NativeInferencePipeline(backend: .init(mossExecutable: asr, mossModelDirectory: root,
            nemotronExecutable: missing, nemotronModel: root.appendingPathComponent("model.gguf")))
        let job = root.appendingPathComponent("job")
        await #expect(throws: InferenceError.self) {
            try await pipeline.runRecording(source: input,
                configuration: .init(expectedSpeakerCount: 1, diarizationPreference: .nemotron3), directory: job)
        }
        #expect(!FileManager.default.fileExists(atPath: job.appendingPathComponent("diarization.log").path))
        #expect(!FileManager.default.fileExists(atPath: job.appendingPathComponent("analysis.wav").path))
        #expect(FileManager.default.fileExists(atPath: input.path))
    }
}
