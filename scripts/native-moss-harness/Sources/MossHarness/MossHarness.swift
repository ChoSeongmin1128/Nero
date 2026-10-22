import AVFoundation
import Darwin
import Foundation
import MLX
import MLXAudioSTT

@main
struct MossHarness {
    static func main() async {
        do { try await run() }
        catch {
            let error = error as NSError
            let kind = error.domain == "MossHarness" ? [1: "input", 2: "input", 3: "allocation", 4: "input", 5: "output", 6: "storage", 7: "inputLimit"][error.code] ?? "engineUnavailable" : "engineUnavailable"
            let message = ["input": "음성 파일 또는 실행 설정을 확인해 주세요.", "allocation": "음성 인식에 필요한 메모리를 확보하지 못했습니다.", "output": "음성 인식 결과를 읽지 못했습니다.", "storage": "음성 인식 결과를 저장하지 못했습니다.", "inputLimit": "전체 음성 인식은 90분 이하의 녹음을 지원합니다."][kind] ?? "음성 인식을 완료하지 못했습니다."
            if let data = try? JSONSerialization.data(withJSONObject: ["worker": "moss", "kind": kind, "message": message], options: [.sortedKeys]) {
                FileHandle.standardError.write(data)
                FileHandle.standardError.write(Data("\n".utf8))
            }
            exit(1)
        }
    }

    private static func run() async throws {
        if CommandLine.arguments.count == 2, CommandLine.arguments[1] == "--runtime-info" {
            let data = try JSONSerialization.data(withJSONObject: [
                "runtime": "mlx-audio-swift-v0.1.3+nero-moss-whole-v1",
                "defaultPolicy": "whole-recording-v1",
                "maxInputSeconds": 5400,
                "maxRequestedTokens": 65536,
            ], options: [.sortedKeys])
            FileHandle.standardOutput.write(data)
            FileHandle.standardOutput.write(Data("\n".utf8))
            return
        }
        guard (4...5).contains(CommandLine.arguments.count) else {
            throw NSError(domain: "MossHarness", code: 1, userInfo: [NSLocalizedDescriptionKey: "Expected input WAV, new output JSON, local model directory, and optional diagnostic chunk seconds (1...180)"])
        }
        let input = URL(fileURLWithPath: CommandLine.arguments[1])
        let outputURL = URL(fileURLWithPath: CommandLine.arguments[2])
        let modelDirectory = URL(fileURLWithPath: CommandLine.arguments[3], isDirectory: true)
        guard !FileManager.default.fileExists(atPath: outputURL.path) else {
            throw NSError(domain: "MossHarness", code: 6, userInfo: [NSLocalizedDescriptionKey: "Output already exists"])
        }
        let file = try AVAudioFile(forReading: input)
        guard file.processingFormat.sampleRate == 16_000,
              file.processingFormat.channelCount == 1,
              file.length > 0 else {
            throw NSError(domain: "MossHarness", code: 2, userInfo: [NSLocalizedDescriptionKey: "Expected nonempty mono 16kHz input"])
        }
        let wholeRecording = CommandLine.arguments.count == 4
        let chunkSeconds = wholeRecording ? Int(ceil(Double(file.length) / 16_000)) : (Int(CommandLine.arguments[4]) ?? 0)
        if wholeRecording {
            guard file.length <= 90 * 60 * 16_000 else {
                throw NSError(domain: "MossHarness", code: 7, userInfo: [NSLocalizedDescriptionKey: "Whole-recording input exceeds the model's 90-minute range"])
            }
        } else if !(1...180).contains(chunkSeconds) {
            throw NSError(domain: "MossHarness", code: 1, userInfo: [NSLocalizedDescriptionKey: "Diagnostic chunk seconds must be 1...180"])
        }
        let started = Date()
        let model = try await MossTranscribeDiarizeModel.fromModelDirectory(modelDirectory)
        let loaded = Date()
        let parameters = STTGenerateParameters(maxTokens: wholeRecording ? 65536 : 8192, temperature: 0, topP: 1, chunkDuration: Float(chunkSeconds), minChunkDuration: 0, repetitionContextSize: 100)
        let chunkFrames = AVAudioFrameCount(wholeRecording ? file.length : Int64(chunkSeconds * 16_000))
        var segments: [[String: Any]] = []
        var chunks: [[String: Any]] = []
        var generationTokens = 0
        var hitTokenLimit = false
        var hasUnparsedText = false
        var generationCount = 0
        var generationEvidence: [[String: Any]] = []
        while file.framePosition < file.length {
            try Task.checkCancellation()
            let startFrame = file.framePosition
            let count = AVAudioFrameCount(min(Int64(chunkFrames), file.length - startFrame))
            guard let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: count) else {
                throw NSError(domain: "MossHarness", code: 3, userInfo: [NSLocalizedDescriptionKey: "PCM allocation failed"])
            }
            try file.read(into: buffer, frameCount: count)
            guard buffer.frameLength > 0, let samples = buffer.floatChannelData?[0] else {
                throw NSError(domain: "MossHarness", code: 4, userInfo: [NSLocalizedDescriptionKey: "PCM read ended before the audio file"])
            }
            let audio = MLXArray(Array(UnsafeBufferPointer(start: samples, count: Int(buffer.frameLength))))
            print(wholeRecording ? "Processing complete recording" : "Processing diagnostic chunk \(chunks.count + 1)")
            fflush(stdout)
            let output = model.generate(audio: audio, generationParameters: parameters)
            let offset = Double(startFrame) / 16_000
            let metadata = model.generationMetadata
            generationCount += metadata.count
            let evidence: [[String: Any]] = metadata.map { item in
                ["promptTokens": item.promptTokens, "maxTokens": item.requestedMaxTokens,
                 "effectiveMaxTokens": item.effectiveMaxTokens, "contextSize": item.contextSize,
                 "generationTokens": item.generatedTokens, "terminationReason": item.terminationReason]
            }
            generationEvidence.append(contentsOf: evidence)
            let capped = metadata.contains { $0.terminationReason == "length" }
            let validatedSegments = MossOutputValidation.validatedSegments(output.text, segments: output.segments ?? [])
            let parsedCompletely = validatedSegments != nil
            hasUnparsedText = hasUnparsedText || !parsedCompletely
            let chunkIndex = chunks.count
            for var segment in validatedSegments ?? [] {
                guard let start = segment["start"] as? Double, let end = segment["end"] as? Double,
                      let text = segment["text"] as? String else {
                    throw NSError(domain: "MossHarness", code: 5, userInfo: [NSLocalizedDescriptionKey: "Malformed MOSS segment"])
                }
                if text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { continue }
                segment["start"] = start + offset
                segment["end"] = end + offset
                segment["asr_chunk"] = chunkIndex
                segments.append(segment)
            }
            var chunk: [String: Any] = [
                "index": chunkIndex, "start": offset,
                "end": Double(file.framePosition) / 16_000,
                "rawText": output.text, "generationTokens": output.generationTokens,
                "hitTokenLimit": capped,
                "parsedCompletely": parsedCompletely,
                "generationCount": metadata.count,
                "generationMetadata": evidence,
                "peakMemoryUsageGB": output.peakMemoryUsage,
            ]
            if let single = evidence.first, evidence.count == 1 {
                for (key, value) in single { chunk[key] = value }
            }
            chunks.append(chunk)
            generationTokens += output.generationTokens
            hitTokenLimit = hitTokenLimit || capped
            if capped || !parsedCompletely || metadata.count != 1 || metadata.first?.terminationReason != "eos" { break }
        }
        let finished = Date()
        var document: [String: Any] = [
            "model": "OpenMOSS-Team/MOSS-Transcribe-Diarize",
            "runtime": "mlx-audio-swift-v0.1.3+nero-moss-whole-v1",
            "chunkDurationSeconds": chunkSeconds,
            "chunkingPolicy": wholeRecording ? "whole-recording-v1" : "explicit-diagnostic-chunks-v1",
            "speakerScope": wholeRecording ? "recording" : "chunk",
            "generationCount": generationCount,
            "generationMetadata": generationEvidence,
            "maxTokens": parameters.maxTokens,
            "modelLoadSeconds": loaded.timeIntervalSince(started),
            "inferenceSeconds": finished.timeIntervalSince(loaded),
            "audioDurationSeconds": Double(file.length) / 16000,
            "text": segments.compactMap { $0["text"] as? String }.joined(separator: "\n"),
            "segments": segments,
            "chunks": chunks,
            "generationTokens": generationTokens,
            "hitTokenLimit": hitTokenLimit,
            "hasUnparsedText": hasUnparsedText,
            "processedAudioFrames": file.framePosition,
            "inputAudioFrames": file.length,
        ]
        if wholeRecording, let single = generationEvidence.first, generationEvidence.count == 1 {
            for (key, value) in single { document[key] = value }
        }
        let data = try JSONSerialization.data(withJSONObject: document, options: [.prettyPrinted, .sortedKeys])
        do { try data.write(to: outputURL, options: .atomic) }
        catch { throw NSError(domain: "MossHarness", code: 6) }
        print("Saved Swift MOSS output; chunks=\(chunks.count), tokens=\(generationTokens), inference=\(finished.timeIntervalSince(loaded))")
    }
}
