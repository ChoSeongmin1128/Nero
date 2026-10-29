import Foundation
import GroveInference
import Testing
@testable import GroveApp

struct VoiceIdentityQualificationTests {
    struct AudioSample: Codable {
        let id: String
        let personID: String?
        let sessionID: String
        let audioPath: String
        let ranges: [VoiceSampleRange]
        let candidates: [String]?
        let conditions: [String]?
    }
    struct Manifest: Codable {
        let enrollments: [AudioSample]
        let queries: [AudioSample]
    }
    struct Row: Codable {
        let id: String
        let expected: String?
        let predicted: String?
        let reason: String?
        let similarity: Float?
        let margin: Float?
        let best: Float?
        let runnerUp: Float?
    }
    struct Trial: Codable {
        let thresholds: VoiceRecognitionPolicy.Thresholds
        let knownQueries: Int
        let correctNames: Int
        let wrongNames: Int
        let missedNames: Int
        let unknownQueries: Int
        let unknownFalseAccepts: Int
        let rows: [Row]
    }
    struct Report: Encodable {
        let modelIdentifier: String
        let extractionSeconds: Double
        let enrollmentPeople: Int
        let acceptedEnrollments: Int
        let enrollmentErrors: [String: String]
        let querySessions: [String]
        let conditions: [String]
        let audioHashes: [String: String]
        let trials: [Trial]
        let releaseEnabled = false
    }

    @Test(.enabled(if: ProcessInfo.processInfo.environment["GROVE_VOICE_IDENTITY_MANIFEST"] != nil))
    func evaluateIndependentRecordings() async throws {
        let env = ProcessInfo.processInfo.environment
        let manifestURL = URL(fileURLWithPath: try #require(env["GROVE_VOICE_IDENTITY_MANIFEST"]))
        let output = URL(fileURLWithPath: try #require(env["GROVE_VOICE_IDENTITY_OUTPUT"]))
        let models = URL(fileURLWithPath: try #require(env["GROVE_VOICE_IDENTITY_MODELS"]))
        let manifest = try JSONDecoder().decode(Manifest.self, from: Data(contentsOf: manifestURL))
        try #require(!manifest.enrollments.isEmpty && !manifest.queries.isEmpty)
        try #require(Set(manifest.enrollments.compactMap(\.personID)).count == manifest.enrollments.count)
        try #require(manifest.enrollments.allSatisfy { $0.personID != nil })
        try #require(Set((manifest.enrollments + manifest.queries).map(\.id)).count == manifest.enrollments.count + manifest.queries.count)
        try #require(!FileManager.default.fileExists(atPath: output.path))
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        let extractor = VoiceEmbeddingExtractor(modelDirectory: models, recipe: .activeFrameCenteredV2)
        var hashes: [String: String] = [:]
        var features: [String: [VoiceEmbeddingSample]] = [:]
        let started = Date()
        for sample in manifest.enrollments + manifest.queries {
            let audio = URL(fileURLWithPath: sample.audioPath, relativeTo: manifestURL.deletingLastPathComponent()).standardizedFileURL
            hashes[sample.id] = try await VoiceIdentitySelection.audioHash(audio)
            features[sample.id] = try await extractor.extractSamples(source: audio, ranges: sample.ranges, workingDirectory: output)
            try #require(try await VoiceIdentitySelection.audioHash(audio) == hashes[sample.id], "평가 중 음성이 변경되었습니다.")
        }
        for enrollment in manifest.enrollments {
            for query in manifest.queries {
                try #require(enrollment.sessionID != query.sessionID, "등록과 평가는 다른 날의 녹음으로 진행해 주세요.")
                try #require(hashes[enrollment.id] != hashes[query.id], "등록에 쓴 음성을 평가에 재사용할 수 없습니다.")
            }
        }
        let elapsed = Date().timeIntervalSince(started)
        var records: [String: VoiceEnrollmentRecord] = [:]
        var enrollmentErrors: [String: String] = [:]
        for sample in manifest.enrollments {
            let voices = try #require(features[sample.id])
            let samples = voices.map { voice in
                VoiceEnrollmentSample(utteranceID: UUID(), start: voice.range.start, end: voice.range.end,
                    voice: voice.voicePrint, sourceMeetingID: nil, sourceRevisionID: nil, audioSHA256: hashes[sample.id]!)
            }
            do { try VoiceRecognitionPolicy.validateEnrollment(samples: samples) }
            catch { enrollmentErrors[sample.personID!] = error.localizedDescription; continue }
            records[sample.personID!] = .init(profileID: UUID(), folderID: UUID(), modelIdentifier: samples[0].voice.modelIdentifier,
                samples: samples, createdAt: Date())
        }
        let modelIDs = Set(features.values.flatMap { $0.map { $0.voicePrint.modelIdentifier } })
        try #require(modelIDs.count == 1)
        var trials: [Trial] = []
        for similarity: Float in [0.60, 0.65, 0.70, 0.75, 0.80, 0.85, 0.90] {
            for margin: Float in [0.05, 0.10, 0.15, 0.20] {
                let thresholds = VoiceRecognitionPolicy.Thresholds(similarity: similarity, margin: margin)
                var rows: [Row] = []
                for query in manifest.queries {
                    let registered = Set(manifest.enrollments.compactMap(\.personID))
                    let candidates = query.candidates ?? registered.sorted()
                    try #require(Set(candidates).isSubset(of: registered))
                    let expected = query.personID.flatMap { candidates.contains($0) ? $0 : nil }
                    let selected = candidates.compactMap { records[$0] }
                    let result = VoiceRecognitionPolicy.evaluate(samples: features[query.id]!.map(\.voicePrint), enrollments: selected, thresholds: thresholds)
                    let predicted = records.first { $0.value.profileID == result.profileID }?.key
                    rows.append(.init(id: query.id, expected: expected, predicted: predicted, reason: result.reason?.rawValue,
                        similarity: result.similarity, margin: result.runnerUpMargin,
                        best: result.bestSimilarity, runnerUp: result.runnerUpSimilarity))
                }
                let known = rows.filter { $0.expected != nil }, unknown = rows.filter { $0.expected == nil }
                trials.append(.init(thresholds: thresholds, knownQueries: known.count,
                    correctNames: known.filter { $0.expected == $0.predicted }.count,
                    wrongNames: known.filter { $0.predicted != nil && $0.expected != $0.predicted }.count,
                    missedNames: known.filter { $0.predicted == nil }.count,
                    unknownQueries: unknown.count, unknownFalseAccepts: unknown.filter { $0.predicted != nil }.count, rows: rows))
            }
        }
        let report = Report(modelIdentifier: modelIDs.first!, extractionSeconds: elapsed,
            enrollmentPeople: manifest.enrollments.count, acceptedEnrollments: records.count, enrollmentErrors: enrollmentErrors,
            querySessions: Set(manifest.queries.map(\.sessionID)).sorted(),
            conditions: Set(manifest.queries.flatMap { $0.conditions ?? [] }).sorted(), audioHashes: hashes, trials: trials)
        let encoder = JSONEncoder(); encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        try encoder.encode(report).write(to: output.appendingPathComponent("evaluation.json"), options: .atomic)
    }
}
