import AVFoundation
import CryptoKit
import Darwin
import Foundation
import MLX
import MLXAudioSTT

// The product mode is explicit and separate from the retained whole-recording
// and diagnostic entry points. It never decides whether a transcript is complete.
enum ProductMoss {
    static let policy = "moss-independent-chunks-v1"
    static let chunkFrames: Int64 = 60 * 16_000

    struct Request: Decodable {
        let schemaVersion: Int
        let processingPolicy: String
        let inputPath: String
        let inputAudioSha256: String
        let inputAudioFrames: Int64
        let modelDirectory: String
        let outputDirectory: String
        let selectedChunks: [Int]
    }

    static func information() -> [String: Any] {
        ["schemaVersion": 1, "processingPolicy": policy, "chunkFrames": chunkFrames,
         "runtime": "mlx-audio-swift-v0.1.3+nero-moss-whole-v2",
         "maxRequestedTokens": 8192, "resume": "caller-validated-independent-chunks-v1"]
    }

    static func generationEvidence(_ metadata: [MossTranscribeDiarizeModel.GenerationMetadata]) -> [[String: Any]] {
        metadata.map { item in
            ["promptTokens": item.promptTokens, "maxTokens": item.requestedMaxTokens,
             "effectiveMaxTokens": item.effectiveMaxTokens, "contextSize": item.contextSize,
             "generationTokens": item.generatedTokens, "terminationReason": item.terminationReason,
             "terminalTokenId": item.terminalTokenId.map { $0 as Any } ?? NSNull(),
             "configuredEosTokenIds": item.configuredEosTokenIds,
             "tokenizerEosTokenId": item.tokenizerEosTokenId,
             "inputAudioSamples": item.inputAudioSamples,
             "featureChunkCount": item.featureChunkCount,
             "featureTokenLengths": item.featureTokenLengths,
             "audioTokenCount": item.audioTokenCount,
             "audioEmbeddingCount": item.audioEmbeddingCount,
             "metadataSchemaVersion": 2,
             "eosPolicy": "model-generation-config+tokenizer-v1",
             "samplingPolicy": item.samplingPolicy]
        }
    }

    private static func failure(_ code: Int) -> NSError { NSError(domain: "MossHarness", code: code) }

    private static func fileHash(_ url: URL) throws -> String {
        let file = try FileHandle(forReadingFrom: url)
        defer { try? file.close() }
        var hash = SHA256()
        while let bytes = try file.read(upToCount: 1024 * 1024), !bytes.isEmpty { hash.update(data: bytes) }
        return hash.finalize().map { String(format: "%02x", $0) }.joined()
    }

    // JSON cannot represent non-finite native floating values. Preserve their
    // exact bits as an explicit invalid value instead of inventing a valid time.
    static func jsonValue(_ value: Any) -> Any {
        if let number = value as? Double, !number.isFinite {
            return ["nativeFloat64Bits": String(number.bitPattern, radix: 16)]
        }
        if let array = value as? [Any] { return array.map(jsonValue) }
        if let dictionary = value as? [String: Any] { return dictionary.mapValues(jsonValue) }
        return value
    }

    private static func save(_ value: [String: Any], to target: URL) throws {
        let parent = target.deletingLastPathComponent()
        let temporary = parent.appendingPathComponent(".moss-\(UUID().uuidString).tmp")
        do {
            let bytes = try JSONSerialization.data(withJSONObject: jsonValue(value), options: [.sortedKeys])
            guard bytes.count <= 2 * 1024 * 1024, !FileManager.default.fileExists(atPath: target.path) else { throw failure(6) }
            try bytes.write(to: temporary, options: [.withoutOverwriting])
            let file = try FileHandle(forWritingTo: temporary)
            try file.synchronize()
            try file.close()
            try FileManager.default.moveItem(at: temporary, to: target)
            let directory = Darwin.open(parent.path, O_RDONLY)
            guard directory >= 0 else { throw failure(6) }
            defer { Darwin.close(directory) }
            guard Darwin.fsync(directory) == 0 else { throw failure(6) }
        } catch {
            // Leave an incomplete temporary file for the owning app to classify
            // and clean. Never delete a previously committed chunk.
            throw failure(6)
        }
    }

    static func run(_ requestURL: URL) async throws {
        let metadata = try requestURL.resourceValues(forKeys: [.fileSizeKey, .isSymbolicLinkKey])
        guard metadata.isSymbolicLink != true, let size = metadata.fileSize, size <= 128 * 1024 else { throw failure(1) }
        let request = try JSONDecoder().decode(Request.self, from: Data(contentsOf: requestURL))
        let input = URL(fileURLWithPath: request.inputPath)
        let destination = URL(fileURLWithPath: request.outputDirectory, isDirectory: true)
        let folder = try destination.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
        guard request.schemaVersion == 1, request.processingPolicy == policy,
              folder.isDirectory == true, folder.isSymbolicLink != true,
              request.inputAudioFrames > 0, request.inputAudioFrames <= 4096 * chunkFrames,
              request.inputAudioSha256.count == 64,
              !request.selectedChunks.isEmpty,
              request.selectedChunks == Array(Set(request.selectedChunks)).sorted() else { throw failure(1) }
        let file = try AVAudioFile(forReading: input)
        guard file.processingFormat.sampleRate == 16_000, file.processingFormat.channelCount == 1,
              file.length == request.inputAudioFrames,
              try fileHash(input) == request.inputAudioSha256 else { throw failure(4) }
        let count = Int((file.length + chunkFrames - 1) / chunkFrames)
        guard count <= 4096, request.selectedChunks.allSatisfy({ $0 >= 0 && $0 < count }) else { throw failure(1) }
        for index in request.selectedChunks {
            guard !FileManager.default.fileExists(atPath: destination.appendingPathComponent("chunk-\(index).json").path) else { throw failure(6) }
        }

        let model = try await MossTranscribeDiarizeModel.fromModelDirectory(URL(fileURLWithPath: request.modelDirectory, isDirectory: true))
        let parameters = STTGenerateParameters(maxTokens: 8192, temperature: 0, topP: 1, chunkDuration: 60, minChunkDuration: 0, repetitionContextSize: 100)
        for index in request.selectedChunks {
            try Task.checkCancellation()
            let start = Int64(index) * chunkFrames
            let end = min(start + chunkFrames, file.length)
            let frames = AVAudioFrameCount(end - start)
            file.framePosition = start
            guard let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: frames) else { throw failure(3) }
            try file.read(into: buffer, frameCount: frames)
            guard buffer.frameLength == frames, let samples = buffer.floatChannelData?[0] else { throw failure(4) }
            let audio = MLXArray(Array(UnsafeBufferPointer(start: samples, count: Int(frames))))
            let output = model.generate(audio: audio, generationParameters: parameters)
            let parsed = MossOutputValidation.productSegments(output.text)
            let native: [String: Any] = [
                "index": index, "startFrame": start, "endFrame": end,
                "inputAudioSha256": request.inputAudioSha256,
                "rawText": output.text, "nativeSegments": output.segments ?? [],
                "segments": parsed ?? [], "parserVersion": "moss-timestamped-text-optional-speaker-v1",
                "parsedCompletely": parsed != nil,
                "generationMetadata": generationEvidence(model.generationMetadata),
                "peakMemoryUsageGB": output.peakMemoryUsage,
            ]
            try save(native, to: destination.appendingPathComponent("chunk-\(index).json"))
            // A malformed native result is durable, but does not stop later input.
            // EOS/parse/coverage judgement and retries belong to the Core projector.
            print("MOSS product chunk \(index + 1)/\(count) saved")
            fflush(stdout)
        }
        guard try fileHash(input) == request.inputAudioSha256 else { throw failure(4) }
    }
}
