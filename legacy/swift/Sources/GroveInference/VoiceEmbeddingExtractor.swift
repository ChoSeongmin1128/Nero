@preconcurrency import AVFoundation
@preconcurrency import CoreML
import CryptoKit
import Foundation

public struct VoiceSampleRange: Codable, Equatable, Sendable {
    public let start: Double
    public let end: Double

    public init(start: Double, end: Double) {
        self.start = start
        self.end = end
    }
}

public struct SpeakerVoicePrint: Codable, Hashable, Sendable {
    public let modelIdentifier: String
    public let embedding: [Float]
    public let speechDuration: Double
    /// Number of distinct spans contributing to the vector, not PCM frame count.
    public let sampleCount: Int

    public init(modelIdentifier: String, embedding: [Float], speechDuration: Double, sampleCount: Int) {
        self.modelIdentifier = modelIdentifier
        self.embedding = embedding
        self.speechDuration = speechDuration
        self.sampleCount = sampleCount
    }

    public func cosineSimilarity(to other: SpeakerVoicePrint) throws -> Double {
        guard !modelIdentifier.isEmpty, modelIdentifier == other.modelIdentifier else {
            throw InferenceError.invalidOutput("서로 다른 음성 특징 모델의 결과는 비교할 수 없습니다.")
        }
        guard embedding.count == other.embedding.count else {
            throw InferenceError.invalidOutput("음성 특징의 차원이 서로 다릅니다.")
        }
        let left = try VoiceEmbeddingMath.normalized(embedding, dimension: embedding.count)
        let right = try VoiceEmbeddingMath.normalized(other.embedding, dimension: embedding.count)
        let dot = zip(left, right).reduce(0.0) { $0 + Double($1.0) * Double($1.1) }
        return min(1, max(-1, dot))
    }
}

public struct VoiceEmbeddingSample: Codable, Equatable, Sendable {
    public let range: VoiceSampleRange
    public let voicePrint: SpeakerVoicePrint

    public init(range: VoiceSampleRange, voicePrint: SpeakerVoicePrint) {
        self.range = range
        self.voicePrint = voicePrint
    }
}

public enum VoiceEmbeddingRecipe: String, Codable, Sendable {
    case rawPaddedV1
    /// Diagnostic recipe until independently qualified for production use.
    case activeFrameCenteredV2
}

/// Extracts local voice features only. Callers must supply single-speaker spans and
/// apply enrollment/identity-confirmation policy separately; this does not name people.
public actor VoiceEmbeddingExtractor {
    private let modelDirectory: URL
    private let recipe: VoiceEmbeddingRecipe

    public init(modelDirectory: URL, recipe: VoiceEmbeddingRecipe = .rawPaddedV1) {
        self.modelDirectory = modelDirectory
        self.recipe = recipe
    }

    /// Returns one vector per supplied span, in caller order. Invalid or oversized ranges
    /// are rejected, never silently dropped. One span is allowed for diagnostics; the app
    /// owns enrollment/query minimums.
    public func extractSamples(source: URL, ranges: [VoiceSampleRange], workingDirectory: URL) async throws -> [VoiceEmbeddingSample] {
        try Task.checkCancellation()
        let selected = try VoiceEmbeddingMath.strictRanges(ranges)
        guard modelDirectory.isFileURL, source.isFileURL, workingDirectory.isFileURL else {
            throw InferenceError.invalidOutput("음성 특징 추출에는 로컬 파일 경로가 필요합니다.")
        }
        let files = FileManager.default
        let scratch = workingDirectory.appendingPathComponent("voiceprint-\(UUID().uuidString)", isDirectory: true)
        try files.createDirectory(at: scratch, withIntermediateDirectories: true)
        defer { try? files.removeItem(at: scratch) }
        let prepared = try await AnalysisAudioPreparer.prepare(source: source, destination: scratch.appendingPathComponent("analysis.wav"))
        let spans = try VoiceEmbeddingMath.frameRanges(selected, frameCount: prepared.frameCount)
        try Task.checkCancellation()
        let identifier = try Self.modelFingerprint(directory: modelDirectory, recipe: recipe)
        let useActiveFrameCentering = recipe == .activeFrameCenteredV2
        let configuration = MLModelConfiguration()
        configuration.computeUnits = .cpuAndNeuralEngine
        let fbank = try MLModel(contentsOf: modelDirectory.appendingPathComponent("FBank.mlmodelc"), configuration: configuration)
        let embedder = try MLModel(contentsOf: modelDirectory.appendingPathComponent("Embedding.mlmodelc"), configuration: configuration)
        let layout = try ModelLayout(fbank: fbank, embedder: embedder)
        let file = try AVAudioFile(forReading: prepared.url, commonFormat: .pcmFormatFloat32, interleaved: false)
        var samples: [VoiceEmbeddingSample] = []
        for (range, span) in zip(selected, spans) {
            try Task.checkCancellation()
            let vector = try autoreleasepool {
                let audio = try Self.readSpan(file: file, startFrame: span.start, frameCount: span.count)
                let input = try VoiceEmbeddingMath.multiArray(values: audio, shape: [1, 1, 160_000], dataType: layout.audioType)
                let fbankResult = try fbank.prediction(from: MLDictionaryFeatureProvider(dictionary: ["audio": input]))
                guard let features = fbankResult.featureValue(for: "fbank_features")?.multiArrayValue,
                      features.shape.map(\.intValue) == [1, 1, 80, 998] else {
                    throw InferenceError.invalidOutput("음성 특징 모델의 주파수 출력 형식이 다릅니다.")
                }
                let preparedFeatures = useActiveFrameCentering
                    ? try VoiceEmbeddingMath.centerActiveFrames(features, activeSamples: span.count) : features
                let featuresInput = try VoiceEmbeddingMath.convert(preparedFeatures, to: layout.featureType)
                let weights = try VoiceEmbeddingMath.multiArray(
                    values: VoiceEmbeddingMath.weights(activeSamples: span.count), shape: [1, 589], dataType: layout.weightType
                )
                try Task.checkCancellation()
                let result = try embedder.prediction(from: MLDictionaryFeatureProvider(dictionary: [
                    "fbank_features": featuresInput, "weights": weights,
                ]))
                guard let output = result.featureValue(for: "embedding")?.multiArrayValue,
                      [.float16, .float32].contains(output.dataType), output.count == 256 else {
                    throw InferenceError.invalidOutput("화자 음성 특징은 256차원 Float 배열이어야 합니다.")
                }
                return try VoiceEmbeddingMath.normalized((0..<output.count).map { output[$0].floatValue })
            }
            samples.append(VoiceEmbeddingSample(range: range,
                voicePrint: SpeakerVoicePrint(modelIdentifier: identifier, embedding: vector,
                    speechDuration: Double(span.count) / 16_000, sampleCount: 1)))
        }
        try Task.checkCancellation()
        return samples
    }

    private static func readSpan(file: AVAudioFile, startFrame: Int64, frameCount: Int) throws -> [Float] {
        guard file.processingFormat.sampleRate == 16_000, file.processingFormat.channelCount == 1,
              frameCount >= 32_000, frameCount <= 160_000,
              let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(frameCount)) else {
            throw InferenceError.invalidOutput("음성 특징 구간을 위한 제한된 오디오 버퍼를 만들지 못했습니다.")
        }
        file.framePosition = startFrame
        try file.read(into: buffer, frameCount: AVAudioFrameCount(frameCount))
        guard Int(buffer.frameLength) == frameCount, let channel = buffer.floatChannelData?[0] else {
            throw InferenceError.invalidOutput("음성 특징 구간을 끝까지 읽지 못했습니다.")
        }
        var audio = [Float](repeating: 0, count: 160_000)
        var energy = 0.0
        for index in 0..<frameCount {
            let value = channel[index]
            guard value.isFinite else { throw InferenceError.invalidOutput("음성 파일에 유효하지 않은 샘플이 있습니다.") }
            audio[index] = value
            energy += Double(value) * Double(value)
        }
        guard energy / Double(frameCount) > 1e-10 else {
            throw InferenceError.invalidOutput("무음 구간에서는 화자 음성 특징을 저장할 수 없습니다.")
        }
        return audio
    }

    public static func modelFingerprint(directory: URL, recipe: VoiceEmbeddingRecipe = .rawPaddedV1) throws -> String {
        var digest = SHA256()
        let files = FileManager.default
        let names = ["FBank.mlmodelc", "Embedding.mlmodelc"]
        for name in names {
            let root = directory.appendingPathComponent(name).standardizedFileURL
            let rootValues = try root.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
            guard rootValues.isDirectory == true, rootValues.isSymbolicLink != true,
                  let enumerator = files.enumerator(at: root, includingPropertiesForKeys: [.isRegularFileKey, .isSymbolicLinkKey]) else {
                throw InferenceError.invalidOutput("로컬 음성 특징 모델 \(name)이 없습니다.")
            }
            let entries = try enumerator.compactMap { value -> URL? in
                guard let url = value as? URL else { return nil }
                let attributes = try url.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey])
                guard attributes.isSymbolicLink != true else {
                    throw InferenceError.invalidOutput("음성 특징 모델 내부의 심볼릭 링크는 지원하지 않습니다.")
                }
                return attributes.isRegularFile == true ? url : nil
            }.sorted { $0.path < $1.path }
            guard entries.contains(where: { $0.lastPathComponent == "weight.bin" }), !entries.isEmpty else {
                throw InferenceError.invalidOutput("음성 특징 모델의 실제 가중치 파일이 없습니다.")
            }
            for entry in entries {
                try Task.checkCancellation()
                let relative = name + "/" + String(entry.path.dropFirst(root.path.count + 1))
                digest.update(data: Data(relative.utf8))
                digest.update(data: Data([0]))
                let handle = try FileHandle(forReadingFrom: entry)
                defer { try? handle.close() }
                while let data = try handle.read(upToCount: 1_048_576), !data.isEmpty {
                    try Task.checkCancellation()
                    digest.update(data: data)
                }
                digest.update(data: Data([0]))
            }
        }
        let hash = digest.finalize().map { String(format: "%02x", $0) }.joined()
        if recipe == .activeFrameCenteredV2 { return "grove-fbank-embedding-active-centered-span-v2:" + hash }
        return "grove-fbank-embedding-span-v1:" + hash
    }

    private struct ModelLayout {
        let audioType: MLMultiArrayDataType
        let featureType: MLMultiArrayDataType
        let weightType: MLMultiArrayDataType

        init(fbank: MLModel, embedder: MLModel) throws {
            func type(_ model: MLModel, name: String, shape: [Int]) throws -> MLMultiArrayDataType {
                guard let constraint = model.modelDescription.inputDescriptionsByName[name]?.multiArrayConstraint,
                      constraint.shape.map(\.intValue) == shape,
                      [.float16, .float32].contains(constraint.dataType) else {
                    throw InferenceError.invalidOutput("음성 특징 모델 입력 \(name)의 차원 또는 형식이 다릅니다.")
                }
                return constraint.dataType
            }
            audioType = try type(fbank, name: "audio", shape: [1, 1, 160_000])
            featureType = try type(embedder, name: "fbank_features", shape: [1, 1, 80, 998])
            weightType = try type(embedder, name: "weights", shape: [1, 589])
        }
    }
}

enum VoiceEmbeddingMath {
    /// The qualified FBank uses a valid400-sample window and160-sample hop.
    /// Its original global mean includes padded silence; subtracting the active
    /// mean cancels that offset. Neutral padding stays outside the pooling mask.
    static func centerActiveFrames(_ features: MLMultiArray, activeSamples: Int) throws -> MLMultiArray {
        guard features.shape.map(\.intValue) == [1, 1, 80, 998],
              [.float16, .float32].contains(features.dataType), (32_000...160_000).contains(activeSamples) else {
            throw InferenceError.invalidOutput("활성 음성 프레임을 정규화할 수 없습니다.")
        }
        let activeFrames = min(998, (activeSamples - 400) / 160 + 1)
        var output = [Float](repeating: 0, count: 80 * 998)
        for band in 0..<80 {
            var values = [Float]()
            values.reserveCapacity(activeFrames)
            for frame in 0..<activeFrames {
                let value = features[[0, 0, NSNumber(value: band), NSNumber(value: frame)]].floatValue
                guard value.isFinite else { throw InferenceError.invalidOutput("음성 특징에 유효하지 않은 값이 있습니다.") }
                values.append(value)
            }
            let mean = values.reduce(0.0) { $0 + Double($1) } / Double(activeFrames)
            for frame in 0..<activeFrames { output[band * 998 + frame] = Float(Double(values[frame]) - mean) }
        }
        return try multiArray(values: output, shape: [1, 1, 80, 998], dataType: features.dataType)
    }

    static func strictRanges(_ ranges: [VoiceSampleRange]) throws -> [VoiceSampleRange] {
        guard (1...5).contains(ranges.count), ranges.allSatisfy({
            $0.start.isFinite && $0.end.isFinite && $0.start >= 0
                && $0.end - $0.start >= 2 - 1e-9 && $0.end - $0.start <= 10 + 1e-9
        }) else { throw InferenceError.invalidOutput("2초 이상 10초 이하의 단독 발화를 최대 5개 선택해 주세요.") }
        let sorted = ranges.sorted { $0.start < $1.start }
        guard zip(sorted, sorted.dropFirst()).allSatisfy({ pair in pair.0.end <= pair.1.start }) else {
            throw InferenceError.invalidOutput("겹친 음성 구간은 별개의 화자 등록 자료로 사용할 수 없습니다.")
        }
        return ranges
    }

    static func frameRanges(_ ranges: [VoiceSampleRange], frameCount: Int64) throws -> [(start: Int64, count: Int)] {
        try ranges.map { range in
            guard range.end <= Double(frameCount) / 16_000, range.start < Double(frameCount) / 16_000 else {
                throw InferenceError.invalidOutput("화자 음성 구간이 녹음 길이를 벗어납니다.")
            }
            let start = Int64((range.start * 16_000).rounded())
            let end = min(frameCount, Int64((range.end * 16_000).rounded()))
            let count = end - start
            guard count >= 32_000, count <= 160_000 else { throw InferenceError.invalidOutput("유효한 화자 음성 구간이 부족합니다.") }
            return (start, Int(count))
        }
    }

    static func weights(activeSamples: Int) -> [Float] {
        let count = min(589, max(0, Int((Double(activeSamples) / 160_000 * 589).rounded())))
        return [Float](repeating: 1, count: count) + [Float](repeating: 0, count: 589 - count)
    }

    static func normalized(_ values: [Float], dimension: Int = 256) throws -> [Float] {
        guard [128, 256].contains(dimension), values.count == dimension, values.allSatisfy(\.isFinite) else {
            throw InferenceError.invalidOutput("화자 음성 특징의 차원이나 값이 유효하지 않습니다.")
        }
        let norm = sqrt(values.reduce(0.0) { $0 + Double($1) * Double($1) })
        guard norm.isFinite, norm > 1e-12 else { throw InferenceError.invalidOutput("비어 있는 화자 음성 특징입니다.") }
        return values.map { Float(Double($0) / norm) }
    }

    static func multiArray(values: [Float], shape: [Int], dataType: MLMultiArrayDataType) throws -> MLMultiArray {
        guard [.float16, .float32].contains(dataType), shape.allSatisfy({ $0 > 0 }),
              shape.reduce(1, *) == values.count, values.allSatisfy(\.isFinite),
              dataType != .float16 || values.allSatisfy({ abs($0) <= Float(Float16.greatestFiniteMagnitude) }) else {
            throw InferenceError.invalidOutput("음성 특징 모델의 Float 배열을 만들 수 없습니다.")
        }
        let array = try MLMultiArray(shape: shape.map { NSNumber(value: $0) }, dataType: dataType)
        if dataType == .float32 {
            values.withUnsafeBufferPointer { array.dataPointer.assumingMemoryBound(to: Float.self).update(from: $0.baseAddress!, count: values.count) }
        } else {
            let destination = array.dataPointer.assumingMemoryBound(to: Float16.self)
            for (index, value) in values.enumerated() { destination[index] = Float16(value) }
        }
        return array
    }

    static func convert(_ value: MLMultiArray, to dataType: MLMultiArrayDataType) throws -> MLMultiArray {
        guard [.float16, .float32].contains(value.dataType), [.float16, .float32].contains(dataType) else {
            throw InferenceError.invalidOutput("음성 특징 모델의 Float 형식이 지원되지 않습니다.")
        }
        if value.dataType == dataType { return value }
        return try multiArray(values: (0..<value.count).map { value[$0].floatValue }, shape: value.shape.map(\.intValue), dataType: dataType)
    }
}
