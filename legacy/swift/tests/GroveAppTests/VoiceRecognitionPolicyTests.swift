import CryptoKit
import Foundation
import GroveInference
import Testing
@testable import GroveApp

struct VoiceRecognitionPolicyTests {
    private func voice(angle: Double = 0, duration: Double = 4, model: String = "synthetic-voice-v1") -> SpeakerVoicePrint {
        var vector = [Float](repeating: 0, count: 256)
        vector[0] = Float(cos(angle)); vector[1] = Float(sin(angle))
        return SpeakerVoicePrint(modelIdentifier: model, embedding: vector, speechDuration: duration, sampleCount: 1)
    }

    private func record(angle: Double = 0, count: Int = 3, duration: Double = 4,
                        model: String = "synthetic-voice-v1") -> VoiceEnrollmentRecord {
        let meeting = UUID(), revision = UUID()
        let samples = (0..<count).map { index in
            VoiceEnrollmentSample(utteranceID: UUID(), start: Double(index) * 12,
                end: Double(index) * 12 + duration, voice: voice(angle: angle, duration: duration, model: model),
                sourceMeetingID: meeting, sourceRevisionID: revision, audioSHA256: String(repeating: "a", count: 64))
        }
        return VoiceEnrollmentRecord(profileID: UUID(), folderID: UUID(), modelIdentifier: model, samples: samples, createdAt: Date())
    }

    @Test func explicitEnrollmentNeedsThreeSpansAndTenSeconds() throws {
        try VoiceRecognitionPolicy.validateEnrollment(samples: record().samples)
        #expect(throws: VoiceRecognitionPolicy.PolicyError.self) { try VoiceRecognitionPolicy.validateEnrollment(samples: record(count: 2).samples) }
        #expect(throws: VoiceRecognitionPolicy.PolicyError.self) { try VoiceRecognitionPolicy.validateEnrollment(samples: record(duration: 3).samples) }
    }

    @Test func enrollmentCannotCountImportedDuplicateAudioAsIndependent() {
        let base = record()
        var samples = base.samples
        let original = samples[0]
        samples[1] = VoiceEnrollmentSample(utteranceID: UUID(), start: original.start, end: original.end,
            voice: original.voice, sourceMeetingID: UUID(), sourceRevisionID: UUID(), audioSHA256: original.audioSHA256)
        #expect(throws: VoiceRecognitionPolicy.PolicyError.self) { try VoiceRecognitionPolicy.validateEnrollment(samples: samples) }
    }

    @Test func heterogeneousEnrollmentIsRejectedBeforeCentroidCanHideIt() {
        let base = record()
        var samples = base.samples
        let replaced = samples[2]
        samples[2] = VoiceEnrollmentSample(utteranceID: replaced.utteranceID, start: replaced.start, end: replaced.end,
            voice: voice(angle: .pi / 2), sourceMeetingID: replaced.sourceMeetingID,
            sourceRevisionID: replaced.sourceRevisionID, audioSHA256: replaced.audioSHA256)
        #expect(throws: VoiceRecognitionPolicy.PolicyError.self) { try VoiceRecognitionPolicy.validateEnrollment(samples: samples) }
    }

    @Test func stableKnownSpeakerIsSuggestedButDecisionIsNotConfirmation() {
        let known = record(), other = record(angle: .pi / 2)
        let decision = VoiceRecognitionPolicy.evaluate(samples: [voice(), voice(angle: 0.05)], enrollments: [known, other])
        #expect(decision.profileID == known.profileID)
        #expect(decision.supportingSamples == 2)
        #expect(decision.reason == nil)
        #expect(decision.policyVersion == VoiceRecognitionPolicy.version)
    }

    @Test func unknownAndLeaveOnePersonOutStayUnmatched() {
        let known = record(), other = record(angle: .pi / 2)
        let unknown = VoiceRecognitionPolicy.evaluate(samples: [voice(angle: .pi), voice(angle: .pi)], enrollments: [known, other])
        #expect(unknown.profileID == nil)
        let removedKnown = VoiceRecognitionPolicy.evaluate(samples: [voice(), voice()], enrollments: [other])
        #expect(removedKnown.profileID == nil)
    }

    @Test func closeRunnerUpPreventsForcedBestName() {
        let result = VoiceRecognitionPolicy.evaluate(samples: [voice(), voice()],
            enrollments: [record(), record(angle: 0.1)])
        #expect(result.profileID == nil)
        #expect(result.reason == .ambiguousProfiles)
    }

    @Test func allQuerySpansMustAgreeRatherThanMajorityAverage() {
        let angle = Double.pi / 10
        let result = VoiceRecognitionPolicy.evaluate(samples: [voice(angle: -angle), voice(angle: angle)],
            enrollments: [record(angle: -angle), record(angle: angle)])
        #expect(result.profileID == nil)
        #expect(result.reason == .conflictingVotes)
    }

    @Test func oneStrongQueryCannotCompensateForWeakSecond() {
        let result = VoiceRecognitionPolicy.evaluate(samples: [voice(), voice(angle: 0.60)], enrollments: [record()])
        #expect(result.profileID == nil)
        #expect(result.reason == .belowSimilarity)
    }

    @Test func withheldDecisionsKeepScoresForCalibration() {
        let first = record(), close = record(angle: 0.1)
        let ambiguous = VoiceRecognitionPolicy.evaluate(samples: [voice(), voice(angle: 0.02)], enrollments: [first, close])
        #expect(ambiguous.reason == .ambiguousProfiles && ambiguous.profileID == nil)
        #expect(ambiguous.bestSimilarity != nil && ambiguous.runnerUpSimilarity != nil)
        #expect((ambiguous.bestSimilarity ?? 0) >= (ambiguous.runnerUpSimilarity ?? 1))
        let accepted = VoiceRecognitionPolicy.evaluate(samples: [voice(), voice(angle: 0.05)], enrollments: [first, record(angle: .pi / 2)])
        #expect(accepted.profileID == first.profileID && accepted.bestSimilarity == accepted.similarity)
    }

    @Test func singleProfileDoesNotInventRunnerUpConfidence() {
        let result = VoiceRecognitionPolicy.evaluate(samples: [voice(), voice()], enrollments: [record()])
        #expect(result.profileID != nil)
        #expect(result.runnerUpMargin == nil)
    }

    @Test func insufficientAndMixedQueriesFailClosed() {
        #expect(VoiceRecognitionPolicy.evaluate(samples: [voice()], enrollments: [record()]).reason == .insufficientSamples)
        #expect(VoiceRecognitionPolicy.evaluate(samples: [voice(), voice(model: "other")], enrollments: [record()]).reason == .mixedModels)
        #expect(VoiceRecognitionPolicy.evaluate(samples: [voice(), voice(angle: .pi / 2)], enrollments: [record()]).reason == .inconsistentSamples)
        #expect(VoiceRecognitionPolicy.evaluate(samples: [voice(), voice()], enrollments: []).reason == .noCompatibleProfiles)
    }

    @Test func malformedOrDuplicatedProfilesDoNotImproveRunnerUpMargin() {
        let good = record()
        #expect(VoiceRecognitionPolicy.evaluate(samples: [voice(), voice()], enrollments: [good, good]).reason == .invalidEnrollment)
        #expect(VoiceRecognitionPolicy.evaluate(samples: [voice(), voice()], enrollments: [good, record(count: 2)]).reason == .invalidEnrollment)
    }

    @Test func averagedLegacyPrintCannotPretendToBeIndependentSpan() {
        let sample = voice()
        let centroid = SpeakerVoicePrint(modelIdentifier: sample.modelIdentifier, embedding: sample.embedding,
            speechDuration: 8, sampleCount: 2)
        #expect(VoiceRecognitionPolicy.evaluate(samples: [centroid, sample], enrollments: [record()]).reason == .invalidSample)
        let invalid = SpeakerVoicePrint(modelIdentifier: sample.modelIdentifier, embedding: [Float](repeating: .nan, count: 256),
            speechDuration: 4, sampleCount: 1)
        #expect(VoiceRecognitionPolicy.evaluate(samples: [invalid, sample], enrollments: [record()]).reason == .invalidSample)
    }
}

struct VoiceRecognitionDevelopmentTests {
    struct Speaker: Codable {
        let id: String
        let enrollment: [VoiceSampleRange]
        let heldOut: [VoiceSampleRange]
    }
    struct Manifest: Codable { let samples: [Speaker]; let audioSHA256: String }
    struct Row: Codable {
        let id: String
        let enrollmentSamples: [VoiceEmbeddingSample]
        let heldOutSamples: [VoiceEmbeddingSample]
        let enrollmentAccepted: Bool
        let enrollmentFailure: String?
        let allProfiles: VoiceRecognitionPolicy.Decision
        let leaveOnePersonOut: VoiceRecognitionPolicy.Decision
        let allProfilesMatchedReference: String?
        let leaveOnePersonOutMatchedReference: String?
    }
    struct Output: Codable {
        let developmentOnly: Bool
        let crossMeetingValidated: Bool
        let usedReferenceForCleanSpanSelection: Bool
        let policyVersion: String
        let recipe: String
        let inputSHA256: String
        let manifestSHA256: String
        let elapsedSeconds: Double
        let acceptedProfileIDs: [String]
        let rows: [Row]
    }

    @Test(.enabled(if: ProcessInfo.processInfo.environment["GROVE_RUN_VOICE_RECOGNITION_DEVELOPMENT_TEST"] == "1"))
    func disjointSampleDevelopmentCheckNeverWritesAppProfiles() async throws {
        let environment = ProcessInfo.processInfo.environment
        let audio = URL(fileURLWithPath: try #require(environment["GROVE_VOICEPRINT_AUDIO"]))
        let models = URL(fileURLWithPath: try #require(environment["GROVE_VOICEPRINT_MODEL_DIRECTORY"]))
        let manifestURL = URL(fileURLWithPath: try #require(environment["GROVE_VOICEPRINT_EVAL_MANIFEST"]))
        let directory = URL(fileURLWithPath: try #require(environment["GROVE_VOICEPRINT_OUTPUT"]))
        let outputURL = directory.appendingPathComponent("development.json")
        guard !FileManager.default.fileExists(atPath: outputURL.path) else {
            throw InferenceError.invalidOutput("새 개발 평가 출력 폴더를 지정해 주세요.")
        }
        let original = try Data(contentsOf: audio)
        let audioHash = SHA256.hash(data: original).map { String(format: "%02x", $0) }.joined()
        let manifestBytes = try Data(contentsOf: manifestURL)
        let manifest = try JSONDecoder().decode(Manifest.self, from: manifestBytes)
        #expect(audioHash == manifest.audioSHA256)
        guard audioHash == manifest.audioSHA256 else { throw InferenceError.invalidOutput("개발 평가 음성 해시가 다릅니다.") }
        let recipeValue = environment["GROVE_VOICEPRINT_RECIPE"] ?? VoiceEmbeddingRecipe.rawPaddedV1.rawValue
        guard let recipe = VoiceEmbeddingRecipe(rawValue: recipeValue) else {
            throw InferenceError.invalidOutput("알 수 없는 개발 음성 특징 recipe입니다.")
        }
        let extractor = VoiceEmbeddingExtractor(modelDirectory: models, recipe: recipe)
        let started = Date()
        let folder = UUID(), meeting = UUID(), revision = UUID()
        var enrolled: [String: [VoiceEmbeddingSample]] = [:], heldOut: [String: [VoiceEmbeddingSample]] = [:]
        var records: [VoiceEnrollmentRecord] = [], failures: [String: String] = [:], identities: [UUID: String] = [:]
        for speaker in manifest.samples {
            #expect(speaker.enrollment.allSatisfy { first in
                speaker.heldOut.allSatisfy { second in first.end <= second.start || second.end <= first.start }
            })
            guard speaker.enrollment.allSatisfy({ first in
                speaker.heldOut.allSatisfy { second in first.end <= second.start || second.end <= first.start }
            }) else { throw InferenceError.invalidOutput("등록과 조회 구간이 겹칩니다.") }
            let enrollment = speaker.enrollment.isEmpty ? [] : try await extractor.extractSamples(
                source: audio, ranges: speaker.enrollment, workingDirectory: directory)
            let queries = speaker.heldOut.isEmpty ? [] : try await extractor.extractSamples(
                source: audio, ranges: speaker.heldOut, workingDirectory: directory)
            enrolled[speaker.id] = enrollment; heldOut[speaker.id] = queries
            let samples = enrollment.map { item in
                VoiceEnrollmentSample(utteranceID: UUID(), start: item.range.start, end: item.range.end, voice: item.voicePrint,
                    sourceMeetingID: meeting, sourceRevisionID: revision, audioSHA256: audioHash)
            }
            do {
                try VoiceRecognitionPolicy.validateEnrollment(samples: samples)
                let profile = UUID()
                records.append(VoiceEnrollmentRecord(profileID: profile, folderID: folder,
                    modelIdentifier: try #require(samples.first?.voice.modelIdentifier), samples: samples, createdAt: Date()))
                identities[profile] = speaker.id
            } catch { failures[speaker.id] = error.localizedDescription }
        }
        let rows = manifest.samples.map { speaker in
            let queries = (heldOut[speaker.id] ?? []).map(\.voicePrint)
            let all = VoiceRecognitionPolicy.evaluate(samples: queries, enrollments: records)
            let missingSelf = VoiceRecognitionPolicy.evaluate(samples: queries,
                enrollments: records.filter { identities[$0.profileID] != speaker.id })
            return Row(id: speaker.id, enrollmentSamples: enrolled[speaker.id] ?? [], heldOutSamples: heldOut[speaker.id] ?? [],
                enrollmentAccepted: failures[speaker.id] == nil, enrollmentFailure: failures[speaker.id],
                allProfiles: all, leaveOnePersonOut: missingSelf,
                allProfilesMatchedReference: all.profileID.flatMap { identities[$0] },
                leaveOnePersonOutMatchedReference: missingSelf.profileID.flatMap { identities[$0] })
        }
        let output = Output(developmentOnly: true, crossMeetingValidated: false, usedReferenceForCleanSpanSelection: true,
            policyVersion: VoiceRecognitionPolicy.version, recipe: recipe.rawValue, inputSHA256: audioHash,
            manifestSHA256: SHA256.hash(data: manifestBytes).map { String(format: "%02x", $0) }.joined(),
            elapsedSeconds: Date().timeIntervalSince(started), acceptedProfileIDs: identities.values.sorted(), rows: rows)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let encoder = JSONEncoder(); encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        try encoder.encode(output).write(to: outputURL, options: .atomic)
        #expect(try Data(contentsOf: audio) == original)
        #expect(!(try FileManager.default.contentsOfDirectory(atPath: directory.path)).contains { $0.hasPrefix("voiceprint-") })
    }
}
