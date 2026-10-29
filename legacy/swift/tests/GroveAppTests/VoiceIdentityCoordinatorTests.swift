@preconcurrency import AVFoundation
import Foundation
import Darwin
import GroveInference
import Testing
@testable import GroveApp


@MainActor
private final class EnrollmentRecordingDevice: AudioRecordingDevice {
    var isRecording = false
    var currentTime: TimeInterval = 0
    var isMeteringEnabled = false
    func prepareToRecord() -> Bool { true }
    func record() -> Bool { isRecording = true; return true }
    func pause() { isRecording = false }
    func stop() { isRecording = false }
    func updateMeters() {}
    func averagePower(forChannel channelNumber: Int) -> Float { -20 }
}

private enum CoordinatorVoiceFailure: Error { case injected }

private final class CoordinatorVoiceKeys: SpeakerVoiceKeyStore, @unchecked Sendable {
    private let lock = NSLock()
    private var values: [UUID: [UUID: Data]] = [:]
    private var rejectDeletion = false

    func load(profileID: UUID, generationID: UUID) throws -> Data? {
        lock.withLock { values[profileID]?[generationID] }
    }
    func insert(_ key: Data, profileID: UUID, generationID: UUID) throws {
        lock.withLock { values[profileID, default: [:]][generationID] = key }
    }
    func remove(profileID: UUID, generationID: UUID) throws {
        try lock.withLock {
            if rejectDeletion { throw CoordinatorVoiceFailure.injected }
            values[profileID]?[generationID] = nil
        }
    }
    func generations(profileID: UUID) throws -> [UUID] {
        lock.withLock { Array(values[profileID, default: [:]].keys) }
    }
    func failDeletion(_ value: Bool) { lock.withLock { rejectDeletion = value } }
}

private actor CoordinatorVoiceExtractor: SpeakerVoiceExtracting {
    enum Behavior { case normal, blocked, failure }
    private(set) var calls = 0
    private var behavior: Behavior = .normal

    func setBehavior(_ value: Behavior) { behavior = value }

    func extractSamples(source: URL, ranges: [VoiceSampleRange], workingDirectory: URL) async throws -> [VoiceEmbeddingSample] {
        calls += 1
        try Task.checkCancellation()
        if behavior == .failure { throw CoordinatorVoiceFailure.injected }
        while behavior == .blocked { try await Task.sleep(for: .milliseconds(5)) }
        try Task.checkCancellation()
        var vector = [Float](repeating: 0, count: 256)
        vector[0] = 1
        return ranges.map {
            VoiceEmbeddingSample(range: $0,
                voicePrint: SpeakerVoicePrint(modelIdentifier: "synthetic-coordinator-voice-v1", embedding: vector,
                                              speechDuration: $0.end - $0.start, sampleCount: 1))
        }
    }
}

private struct ReplacedModelExtractor: SpeakerVoiceExtracting {
    func extractSamples(source: URL, ranges: [VoiceSampleRange], workingDirectory: URL) async throws -> [VoiceEmbeddingSample] {
        throw CoordinatorVoiceFailure.injected
    }
    func currentModelIdentifier() throws -> String? { "synthetic-coordinator-voice-v2" }
}

private func coordinatorResult(configuration: InferenceConfiguration = .init(expectedSpeakerCount: 1),
                               mixedThirdSpan: Bool = false) throws -> InferenceResult {
    var activities: [DiarizationTurn] = [
        .init(start: 0, end: 5, clusterID: "A"),
        .init(start: 6, end: 11, clusterID: "A"),
        .init(start: 12, end: 17, clusterID: "A")
    ]
    if mixedThirdSpan { activities.append(.init(start: 14, end: 15, clusterID: "B")) }
    return try InferenceResult(duration: 18, configuration: configuration,
        transcription: .init(utterances: [
            .init(start: 0, end: 5, text: "합성 테스트의 첫 발화입니다."),
            .init(start: 6, end: 11, text: "저장과 식별 연결만 검증합니다."),
            .init(start: 12, end: 17, text: "실제 사람의 음성은 사용하지 않습니다.")
        ]), rawDiarization: activities)
}

private actor CoordinatorInference: MeetingInferenceRunning {
    func run(source: URL, configuration: InferenceConfiguration, directory: URL,
             progress: @Sendable (String) async -> Void) async throws -> InferenceResult {
        try coordinatorResult(configuration: configuration)
    }
}

@MainActor
private struct VoiceCoordinatorFixture {
    let root: URL
    let keys: CoordinatorVoiceKeys
    let vault: SpeakerVoiceVault
    let extractor: CoordinatorVoiceExtractor
    let inference: CoordinatorInference
    let store: GroveStore
    let folderID: UUID
    let voiceIdentificationAvailable: Bool

    init(voiceIdentificationAvailable: Bool = true,
         vaultFailure: @escaping @Sendable (VoiceVaultIOPoint) throws -> Void = { _ in }) throws {
        self.voiceIdentificationAvailable = voiceIdentificationAvailable
        let canonicalTemporary = try #require(realpath(FileManager.default.temporaryDirectory.path, nil))
        defer { free(canonicalTemporary) }
        root = URL(fileURLWithPath: String(cString: canonicalTemporary), isDirectory: true)
            .appendingPathComponent("grove-voice-coordinator-" + UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false,
                                               attributes: [.posixPermissions: 0o700])
        keys = CoordinatorVoiceKeys()
        vault = SpeakerVoiceVault(directory: root.appendingPathComponent("VoiceRegistry"), keyStore: keys,
                                  ioFailure: vaultFailure)
        extractor = CoordinatorVoiceExtractor()
        inference = CoordinatorInference()
        store = GroveStore(baseDirectory: root, inferenceService: inference, voiceVault: vault, voiceExtractor: extractor,
                           voiceIdentificationAvailable: voiceIdentificationAvailable)
        folderID = try #require(store.createFolder(name: "합성 회의 폴더"))
    }

    func clean() { try? FileManager.default.removeItem(at: root) }

    func addMeeting(in folder: UUID? = nil, payload: Data? = nil, mixedThirdSpan: Bool = false) throws -> MeetingRecord {
        let source = root.appendingPathComponent(UUID().uuidString + ".wav")
        // This is a fake file for hashing and protocol wiring, not playable audio.
        try (payload ?? Data(("synthetic payload " + UUID().uuidString).utf8)).write(to: source, options: .withoutOverwriting)
        var meeting = MeetingRecord(title: "합성 회의", startedAt: Date(), duration: 18, status: .ready,
            audioPath: source.path, glossaryProfile: "", transcript: [], claims: [], errorMessage: nil)
        meeting.folderID = folder ?? folderID
        store.meetings.append(meeting)
        try store.acceptInferenceResult(coordinatorResult(mixedThirdSpan: mixedThirdSpan), meetingID: meeting.id,
                                        sourceChannelID: "recording")
        return meeting
    }

    func document(_ meeting: MeetingRecord) throws -> TranscriptDocument {
        try #require(store.transcriptDocuments[meeting.id])
    }

    func enroll(_ meeting: MeetingRecord, name: String = "저장한 사람", automatic: Bool = false) async throws -> SavedSpeakerProfile {
        let document = try document(meeting)
        let speaker = try #require(document.speakers.first)
        let ids = Set(store.voice.enrollmentCandidates(meetingID: meeting.id, speakerID: speaker.id).map(\.id))
        let saved = await store.voice.enrollVoice(meetingID: meeting.id, speakerID: speaker.id, name: name,
                                           selectedUtteranceIDs: ids, permissionConfirmed: true,
                                           enableAutomaticIdentification: automatic)
        try #require(saved, Comment(rawValue: store.alertMessage ?? "enrollment should succeed"))
        return try #require(store.speakerProfiles(in: folderID).first)
    }

    func reopened() -> GroveStore {
        GroveStore(baseDirectory: root, inferenceService: inference, voiceVault: vault, voiceExtractor: extractor,
                   voiceIdentificationAvailable: voiceIdentificationAvailable)
    }
}

@MainActor
struct VoiceIdentityCoordinatorTests {
    @Test func productionDefaultGatePreventsEnrollmentManualIdentificationAndAutomaticHook() async throws {
        let fixture = try VoiceCoordinatorFixture(voiceIdentificationAvailable: false)
        defer { fixture.clean() }
        let meeting = try fixture.addMeeting()
        let document = try fixture.document(meeting)
        let speaker = try #require(document.speakers.first)
        // Omit the injection flag to exercise the shipped constructor default too.
        let productionDefault = GroveStore(baseDirectory: fixture.root, inferenceService: fixture.inference,
                                          voiceVault: fixture.vault, voiceExtractor: fixture.extractor)
        #expect(!productionDefault.voice.isAvailable)
        #expect(!fixture.store.voice.isAvailable)
        #expect(!(await fixture.store.voice.enrollVoice(meetingID: meeting.id, speakerID: speaker.id, name: "사용 불가 등록",
            selectedUtteranceIDs: Set(document.utterances.map(\.id)), permissionConfirmed: true,
            enableAutomaticIdentification: true)))
        #expect(!fixture.store.voice.setAutomaticIdentification(folderID: fixture.folderID, enabled: true))
        await fixture.store.voice.identifySpeakers(meetingID: meeting.id)
        await fixture.store.transcribeMeeting(id: meeting.id)
        #expect(await fixture.extractor.calls == 0)
        #expect(fixture.store.speakerProfiles(in: fixture.folderID).isEmpty)
        #expect(try fixture.document(meeting).speakers.allSatisfy { $0.profileMatch == nil })
        #expect(fixture.store.meetings.first?.status == .ready)
        let currentSpeakerID = try #require(fixture.document(meeting).speakers.first?.id)
        #expect(await fixture.store.voice.saveSpeakerProfile(meetingID: meeting.id,
            speakerID: currentSpeakerID, name: "계속 가능한 이름 저장"))
        #expect(await fixture.extractor.calls == 0)
    }

    @Test func productionGateSuppressesPreviouslyEnabledFolderButStillAllowsForgetting() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        let profile = try await fixture.enroll(fixture.addMeeting(), automatic: true)
        let query = try fixture.addMeeting()
        try MeetingRecordStorage(url: fixture.root.appendingPathComponent("meetings.json")).save(fixture.store.meetings)
        let gated = GroveStore(baseDirectory: fixture.root, inferenceService: fixture.inference,
                               voiceVault: fixture.vault, voiceExtractor: fixture.extractor)
        await gated.voice.refreshEnrollmentStatus()
        #expect(gated.library.folders.first?.automaticSpeakerIdentification == true)
        #expect(!gated.voice.automaticIdentificationEnabled(folderID: fixture.folderID))
        #expect(gated.voice.isRegistered(profile.id))
        await gated.voice.identifySpeakers(meetingID: query.id)
        await gated.transcribeMeeting(id: query.id)
        #expect(await fixture.extractor.calls == 1)
        #expect(gated.meetings.first(where: { $0.id == query.id })?.status == .ready)
        #expect(gated.transcriptDocuments[query.id]?.speakers.allSatisfy { $0.profileMatch == nil } == true)
        #expect(await gated.voice.removeVoiceEnrollment(profileID: profile.id))
        #expect(!gated.voice.hasStorage(profile.id))
        #expect(gated.speakerProfiles(in: fixture.folderID).first?.name == profile.name)
        #expect(gated.voice.removeSpeakerProfile(id: profile.id))
    }

    @Test func enrollmentFromAnotherVoiceModelIsMarkedForReenrollment() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        let profile = try await fixture.enroll(fixture.addMeeting(), automatic: false)
        let unchanged = GroveStore(baseDirectory: fixture.root, inferenceService: fixture.inference,
                                   voiceVault: fixture.vault, voiceExtractor: fixture.extractor)
        await unchanged.voice.refreshEnrollmentStatus()
        await unchanged.voice.refreshModelCompatibility()
        #expect(unchanged.voice.isRegistered(profile.id) && !unchanged.voice.needsReenrollment(profile.id))
        let replaced = GroveStore(baseDirectory: fixture.root, inferenceService: fixture.inference,
                                  voiceVault: fixture.vault, voiceExtractor: ReplacedModelExtractor())
        await replaced.voice.refreshEnrollmentStatus()
        await replaced.voice.refreshModelCompatibility()
        #expect(replaced.voice.needsReenrollment(profile.id))
    }

    @Test func savingNameNeverExtractsOrRegistersVoice() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        let meeting = try fixture.addMeeting()
        let speaker = try #require(fixture.document(meeting).speakers.first)
        #expect(await fixture.store.voice.saveSpeakerProfile(meetingID: meeting.id, speakerID: speaker.id, name: "이름만 저장"))
        let profile = try #require(fixture.store.speakerProfiles(in: fixture.folderID).first)
        #expect(await fixture.extractor.calls == 0)
        #expect(profile.voice == nil)
        #expect(try await !fixture.vault.hasRecord(profileID: profile.id))
        #expect(!fixture.store.voice.automaticIdentificationEnabled(folderID: fixture.folderID))
    }

    @Test func consentAndExplicitMinimumSelectionAreRequiredBeforeExtraction() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        let meeting = try fixture.addMeeting()
        let document = try fixture.document(meeting)
        let speaker = try #require(document.speakers.first)
        let ids = Set(document.utterances.map(\.id))
        #expect(!(await fixture.store.voice.enrollVoice(meetingID: meeting.id, speakerID: speaker.id, name: "합성 화자",
            selectedUtteranceIDs: ids, permissionConfirmed: false, enableAutomaticIdentification: true)))
        #expect(!(await fixture.store.voice.enrollVoice(meetingID: meeting.id, speakerID: speaker.id, name: "합성 화자",
            selectedUtteranceIDs: Set(document.utterances.prefix(2).map(\.id)), permissionConfirmed: true,
            enableAutomaticIdentification: true)))
        #expect(await fixture.extractor.calls == 0)
        #expect(fixture.store.speakerProfiles(in: fixture.folderID).isEmpty)
        #expect(!fixture.store.voice.automaticIdentificationEnabled(folderID: fixture.folderID))
    }

    @Test func rawMixedSpeakerEvidenceCannotBecomeEnrollmentBySelection() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        let meeting = try fixture.addMeeting(mixedThirdSpan: true)
        let document = try fixture.document(meeting)
        let speaker = try #require(document.speakers.first)
        let candidates = fixture.store.voice.enrollmentCandidates(meetingID: meeting.id, speakerID: speaker.id)
        #expect(candidates.count == 2)
        #expect(candidates.allSatisfy { $0.endTime < 12 })
        #expect(!(await fixture.store.voice.enrollVoice(meetingID: meeting.id, speakerID: speaker.id, name: "합성 화자",
            selectedUtteranceIDs: Set(document.utterances.map(\.id)), permissionConfirmed: true,
            enableAutomaticIdentification: false)))
        #expect(await fixture.extractor.calls == 0)
    }

    @Test func enrollmentPersistsEncryptedAndLeavesLibraryBackupsNamesOnly() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        let meeting = try fixture.addMeeting()
        let original = try fixture.document(meeting)
        let profile = try await fixture.enroll(meeting)
        let record = try #require(await fixture.vault.load(profileID: profile.id, folderID: fixture.folderID))
        #expect(record.samples.count == 3)
        #expect(record.samples.allSatisfy { $0.sourceMeetingID == meeting.id && $0.sourceRevisionID == original.revisionID })
        #expect(record.samples.allSatisfy { $0.audioSHA256.count == 64 })
        #expect(await fixture.extractor.calls == 1)
        #expect(profile.voice == nil)
        for file in ["library.json", "library.json.backup"] {
            let data = try Data(contentsOf: fixture.root.appendingPathComponent(file))
            let library = try JSONDecoder().decode(MeetingLibrary.self, from: data)
            #expect(library.speakerProfiles?.allSatisfy { $0.voice == nil } ?? true)
            let text = String(decoding: data, as: UTF8.self)
            #expect(!text.contains("embedding"))
            #expect(!text.contains("synthetic-coordinator-voice-v1"))
            #expect(!text.contains("audioSHA256"))
        }
        let restarted = fixture.reopened()
        await restarted.voice.refreshEnrollmentStatus()
        #expect(restarted.voice.isRegistered(profile.id))
        #expect(restarted.speakerProfiles(in: fixture.folderID).first?.id == profile.id)
        #expect(!restarted.voice.automaticIdentificationEnabled(folderID: fixture.folderID))
    }

    @Test func automaticHookRunsOnlyForExplicitlyEnabledFolder() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        let source = try fixture.addMeeting()
        let profile = try await fixture.enroll(source, automatic: false)
        let disabled = try fixture.addMeeting()
        await fixture.store.transcribeMeeting(id: disabled.id)
        #expect(await fixture.extractor.calls == 1)
        #expect(try fixture.document(disabled).speakers.first?.profileMatch == nil)
        #expect(fixture.store.voice.setAutomaticIdentification(folderID: fixture.folderID, enabled: true))
        let enabled = try fixture.addMeeting()
        await fixture.store.transcribeMeeting(id: enabled.id)
        #expect(await fixture.extractor.calls == 2)
        #expect(try fixture.document(enabled).speakers.first?.profileMatch?.profileID == profile.id)
        #expect(try fixture.document(enabled).speakers.first?.profileMatch?.isConfirmed == false)
        #expect(!fixture.store.isBusy)
        let otherFolder = try #require(fixture.store.createFolder(name: "다른 합성 폴더"))
        let outside = try fixture.addMeeting(in: otherFolder)
        await fixture.store.transcribeMeeting(id: outside.id)
        #expect(await fixture.extractor.calls == 2)
        #expect(try fixture.document(outside).speakers.first?.profileMatch == nil)
        #expect(fixture.reopened().voice.automaticIdentificationEnabled(folderID: fixture.folderID))
    }

    @Test func suggestedIdentityConfirmRejectAndUndoDoNotEnrollOrChangeText() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        let source = try fixture.addMeeting()
        let profile = try await fixture.enroll(source)
        let query = try fixture.addMeeting()
        let original = try fixture.document(query)
        let originalSpeaker = try #require(original.speakers.first)
        let enrollmentBefore = try await fixture.vault.load(profileID: profile.id, folderID: fixture.folderID)
        await fixture.store.voice.identifySpeakers(meetingID: query.id)
        let proposed = try fixture.document(query)
        #expect(proposed.speakers.first?.profileMatch?.isConfirmed == false)
        #expect(proposed.speakers.first?.name == profile.name)
        #expect(proposed.utterances == original.utterances)
        #expect(fixture.store.voice.confirmIdentity(meetingID: query.id, speakerID: originalSpeaker.id))
        #expect(try fixture.document(query).speakers.first?.profileMatch?.isConfirmed == true)
        fixture.store.transcripts.undo(meetingID: query.id)
        #expect(try fixture.document(query).speakers.first?.profileMatch?.isConfirmed == false)
        #expect(fixture.store.voice.rejectIdentity(meetingID: query.id, speakerID: originalSpeaker.id))
        #expect(try fixture.document(query).speakers.first?.name == originalSpeaker.name)
        #expect(try fixture.document(query).speakers.first?.profileMatch == nil)
        fixture.store.transcripts.undo(meetingID: query.id)
        #expect(try fixture.document(query).speakers.first?.profileMatch?.isConfirmed == false)
        #expect(try await fixture.vault.load(profileID: profile.id, folderID: fixture.folderID) == enrollmentBefore)
        #expect(await fixture.extractor.calls == 2)
    }

    @Test func manualNameAndEvenUndoneIdentityHistoryPreventAutomaticOverwrite() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        let source = try fixture.addMeeting()
        _ = try await fixture.enroll(source)
        let query = try fixture.addMeeting()
        let speaker = try #require(fixture.document(query).speakers.first)
        #expect(fixture.store.transcripts.renameSpeaker(meetingID: query.id, speakerID: speaker.id, name: "직접 정한 이름"))
        await fixture.store.voice.identifySpeakers(meetingID: query.id)
        #expect(try fixture.document(query).speakers.first?.name == "직접 정한 이름")
        fixture.store.transcripts.undo(meetingID: query.id)
        #expect(try fixture.document(query).speakers.first?.name == speaker.name)
        await fixture.store.voice.identifySpeakers(meetingID: query.id)
        #expect(try fixture.document(query).speakers.first?.profileMatch == nil)
        #expect(await fixture.extractor.calls == 1)
    }

    @Test func importedCopyOfEnrollmentAudioCannotCountAsAnotherMeeting() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        let payload = Data("same synthetic bytes across two recording records".utf8)
        let source = try fixture.addMeeting(payload: payload)
        _ = try await fixture.enroll(source)
        let copy = try fixture.addMeeting(payload: payload)
        await fixture.store.voice.identifySpeakers(meetingID: copy.id)
        #expect(try fixture.document(copy).speakers.first?.profileMatch == nil)
        #expect(await fixture.extractor.calls == 1)
    }

    @Test func forgettingVoiceKeepsNamesAndHistoryUntilExplicitNameDeletion() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        let source = try fixture.addMeeting()
        let profile = try await fixture.enroll(source)
        let before = try fixture.document(source)
        #expect(!fixture.store.voice.removeSpeakerProfile(id: profile.id))
        #expect(await fixture.store.voice.removeVoiceEnrollment(profileID: profile.id))
        #expect(try await !fixture.vault.hasRecord(profileID: profile.id))
        #expect(try fixture.keys.generations(profileID: profile.id).isEmpty)
        #expect(fixture.store.speakerProfiles(in: fixture.folderID).first?.name == profile.name)
        #expect(try fixture.document(source) == before)
        #expect(fixture.store.voice.removeSpeakerProfile(id: profile.id))
        #expect(fixture.store.speakerProfiles(in: fixture.folderID).isEmpty)
        #expect(try fixture.document(source) == before)
    }

    @Test(arguments: [false, true])
    func failedEncryptedCommitRemainsAddressableForExplicitCleanup(_ keyCleanupFails: Bool) async throws {
        let fixture = try VoiceCoordinatorFixture(vaultFailure: { point in
            if case .beforeWrite = point { throw CoordinatorVoiceFailure.injected }
        })
        defer { fixture.clean() }
        let meeting = try fixture.addMeeting()
        let document = try fixture.document(meeting)
        let speaker = try #require(document.speakers.first)
        fixture.keys.failDeletion(keyCleanupFails)
        #expect(!(await fixture.store.voice.enrollVoice(meetingID: meeting.id, speakerID: speaker.id, name: "실패한 등록",
            selectedUtteranceIDs: Set(document.utterances.map(\.id)), permissionConfirmed: true,
            enableAutomaticIdentification: true)))
        let profile = try #require(fixture.store.speakerProfiles(in: fixture.folderID).first)
        #expect(fixture.store.voice.hasStorage(profile.id))
        #expect(!fixture.store.voice.isRegistered(profile.id))
        #expect(!fixture.store.voice.removeSpeakerProfile(id: profile.id))
        #expect(!(await fixture.store.voice.saveSpeakerProfile(meetingID: meeting.id, speakerID: speaker.id, name: "다른 이름")))
        #expect(fixture.store.speakerProfiles(in: fixture.folderID).map(\.id) == [profile.id])
        #expect(try fixture.keys.generations(profileID: profile.id).count == (keyCleanupFails ? 1 : 0))
        let restarted = fixture.reopened()
        await restarted.voice.refreshEnrollmentStatus()
        #expect(restarted.voice.hasStorage(profile.id))
        #expect(!restarted.voice.removeSpeakerProfile(id: profile.id))
        fixture.keys.failDeletion(false)
        #expect(await restarted.voice.removeVoiceEnrollment(profileID: profile.id))
        #expect(!restarted.voice.hasStorage(profile.id))
        #expect(try fixture.keys.generations(profileID: profile.id).isEmpty)
        #expect(restarted.voice.removeSpeakerProfile(id: profile.id))
        #expect(restarted.speakerProfiles(in: fixture.folderID).isEmpty)
        #expect(!restarted.voice.automaticIdentificationEnabled(folderID: fixture.folderID))
        #expect(try fixture.document(meeting) == document)
    }

    @Test(arguments: ["name", "text"])
    func editsWhileExtractingInvalidateOnlyTheProposedResult(_ edit: String) async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        _ = try await fixture.enroll(fixture.addMeeting())
        let query = try fixture.addMeeting()
        let original = try fixture.document(query)
        let speaker = try #require(original.speakers.first)
        let utterance = try #require(original.utterances.first)
        await fixture.extractor.setBehavior(.blocked)
        let task = Task { await fixture.store.voice.identifySpeakers(meetingID: query.id) }
        defer { task.cancel() }
        try await waitForCalls(2, on: fixture.extractor)
        if edit == "name" {
            #expect(fixture.store.transcripts.renameSpeaker(meetingID: query.id, speakerID: speaker.id, name: "처리 중 바꾼 이름"))
        } else {
            #expect(fixture.store.transcripts.updateUtteranceText(meetingID: query.id, utteranceID: utterance.id, text: "처리 중 수정한 발화"))
        }
        let edited = try fixture.document(query)
        await fixture.extractor.setBehavior(.normal)
        await task.value
        #expect(try fixture.document(query) == edited)
        #expect(try fixture.document(query).speakers.first?.profileMatch == nil)
        #expect(!fixture.store.isBusy)
    }

    @Test func cancellingEnrollmentBeforePublishLeavesNoSavedIdentity() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        let meeting = try fixture.addMeeting()
        let original = try fixture.document(meeting)
        let speaker = try #require(original.speakers.first)
        await fixture.extractor.setBehavior(.blocked)
        let task = Task {
            await fixture.store.voice.enrollVoice(meetingID: meeting.id, speakerID: speaker.id, name: "취소할 이름",
                selectedUtteranceIDs: Set(original.utterances.map(\.id)), permissionConfirmed: true,
                enableAutomaticIdentification: true)
        }
        defer { task.cancel() }
        try await waitForCalls(1, on: fixture.extractor)
        fixture.store.voice.cancelWork()
        #expect(await task.value == false)
        #expect(fixture.store.speakerProfiles(in: fixture.folderID).isEmpty)
        #expect(!fixture.store.voice.automaticIdentificationEnabled(folderID: fixture.folderID))
        #expect(try fixture.document(meeting) == original)
        #expect(!fixture.store.isBusy)
    }

    @Test func externalTaskCancellationDoesNotApplyNamesOrChangeEnrollment() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        let profile = try await fixture.enroll(fixture.addMeeting())
        let enrollment = try await fixture.vault.load(profileID: profile.id, folderID: fixture.folderID)
        let query = try fixture.addMeeting()
        let original = try fixture.document(query)
        await fixture.extractor.setBehavior(.blocked)
        let task = Task { await fixture.store.voice.identifySpeakers(meetingID: query.id) }
        defer { task.cancel() }
        try await waitForCalls(2, on: fixture.extractor)
        task.cancel()
        await task.value
        #expect(try fixture.document(query) == original)
        #expect(try await fixture.vault.load(profileID: profile.id, folderID: fixture.folderID) == enrollment)
        #expect(!fixture.store.isBusy)
    }

    @Test func automaticIdentityFailureDoesNotConvertSuccessfulTranscriptionIntoFailure() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        _ = try await fixture.enroll(fixture.addMeeting(), automatic: true)
        let query = try fixture.addMeeting()
        await fixture.extractor.setBehavior(.failure)
        await fixture.store.transcribeMeeting(id: query.id)
        let updated = try #require(fixture.store.meetings.first { $0.id == query.id })
        #expect(updated.status == .ready)
        #expect(updated.processingOutcome?.kind == .completed)
        #expect(updated.errorMessage == nil)
        #expect(try fixture.document(query).utterances.count == 3)
        #expect(try fixture.document(query).speakers.first?.profileMatch == nil)
        #expect(fixture.store.voice.message(for: query.id) != nil)
        #expect(!fixture.store.isBusy)
    }

    @Test func separatedClustersCanBothIdentifyTheSameRegisteredPerson() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        _ = try await fixture.enroll(fixture.addMeeting())
        let query = try fixture.addMeeting()
        var utterances: [RecognizedUtterance] = []
        var turns: [DiarizationTurn] = []
        for index in 0..<6 {
            let start = Double(index) * 6
            utterances.append(RecognizedUtterance(start: start, end: start + 5, text: "합성 발화 \(index + 1)"))
            turns.append(DiarizationTurn(start: start, end: start + 5, clusterID: index < 3 ? "A" : "B"))
        }
        let result = try InferenceResult(duration: 36, configuration: .init(expectedSpeakerCount: 2),
            transcription: RawTranscription(utterances: utterances), rawDiarization: turns)
        try fixture.store.acceptInferenceResult(result, meetingID: query.id, sourceChannelID: "recording")
        await fixture.store.voice.identifySpeakers(meetingID: query.id)
        #expect(await fixture.extractor.calls == 3)
        let identified = try fixture.document(query)
        #expect(identified.speakers.count == 2)
        #expect(identified.speakers.allSatisfy { $0.profileMatch != nil && $0.profileMatch?.isConfirmed == false })
        #expect(Set(identified.speakers.compactMap { $0.profileMatch?.profileID }).count == 1)
    }

    @Test func explicitEmptyAttendanceNeverSearchesUnselectedTeamMembers() async throws {
        let fixture = try VoiceCoordinatorFixture()
        defer { fixture.clean() }
        _ = try await fixture.enroll(fixture.addMeeting())
        var query = try fixture.addMeeting()
        query.attendance = MeetingAttendance()
        fixture.store.meetings = fixture.store.meetings.map { $0.id == query.id ? query : $0 }
        let before = try fixture.document(query)
        await fixture.store.voice.identifySpeakers(meetingID: query.id)
        #expect(await fixture.extractor.calls == 1)
        #expect(try fixture.document(query) == before)
    }

    @Test func dedicatedRecordingEnrollsWithoutEnablingAutomaticNames() async throws {
        let canonicalTemporary = try #require(realpath(FileManager.default.temporaryDirectory.path, nil))
        defer { free(canonicalTemporary) }
        let root = URL(fileURLWithPath: String(cString: canonicalTemporary), isDirectory: true).appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: root) }
        let device = EnrollmentRecordingDevice()
        let recorder = AudioRecorder(makeRecorder: { url, _ in
            let format = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 48_000, channels: 1, interleaved: false)!
            let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 48_000 * 32)!
            buffer.frameLength = 48_000 * 32
            for index in 0..<Int(buffer.frameLength) { buffer.floatChannelData![0][index] = Float(sin(Double(index) * 0.07) * 0.1) }
            let file = try AVAudioFile(forWriting: url, settings: [AVFormatIDKey: kAudioFormatMPEG4AAC,
                AVSampleRateKey: 48_000, AVNumberOfChannelsKey: 1, AVEncoderBitRateKey: 128_000],
                commonFormat: .pcmFormatFloat32, interleaved: false)
            try file.write(from: buffer)
            return device
        })
        let session = VoiceEnrollmentSession(directory: root.appendingPathComponent("Capture"), recorder: recorder, requestPermission: { true })
        let keys = CoordinatorVoiceKeys()
        let vault = SpeakerVoiceVault(directory: root.appendingPathComponent("VoiceRegistry"), keyStore: keys)
        let store = GroveStore(baseDirectory: root, voiceVault: vault, voiceExtractor: CoordinatorVoiceExtractor(), voiceEnrollmentSession: session)
        let folder = try #require(store.createFolder(name: "팀 회의"))
        let person = try #require(store.libraryStore.addTeamMember(folderID: folder, name: "등록할 팀원"))
        await session.start()
        #expect(store.isBusy)
        device.currentTime = 16
        recorder.pause()
        session.beginNaturalSpeech()
        try recorder.resume()
        device.currentTime = 32
        let source = try #require(session.source)
        await session.finish()
        #expect(session.review?.canExtract == true)
        #expect(!(await store.voice.enrollRecordedVoice(profileID: person, permissionConfirmed: false)))
        #expect(await store.voice.enrollRecordedVoice(profileID: person, permissionConfirmed: true))
        #expect(store.voice.isRegistered(person))
        #expect(!store.voice.isAvailable)
        #expect(!FileManager.default.fileExists(atPath: source.path))
        #expect(store.meetings.isEmpty)
        let stored = try #require(await vault.load(profileID: person, folderID: folder))
        #expect(stored.samples.count >= 3)
        #expect(stored.samples.allSatisfy { $0.sourceMeetingID == nil && $0.sourceRevisionID == nil })
        let index = String(decoding: try Data(contentsOf: root.appendingPathComponent("library.json")), as: UTF8.self)
        #expect(!index.contains("embedding"))
        #expect(await store.voice.removeVoiceEnrollment(profileID: person))
        #expect(!store.voice.isRegistered(person))
    }

    private func waitForCalls(_ count: Int, on extractor: CoordinatorVoiceExtractor) async throws {
        for _ in 0..<200 {
            if await extractor.calls >= count { return }
            try await Task.sleep(for: .milliseconds(5))
        }
        try #require(await extractor.calls >= count)
    }
}
