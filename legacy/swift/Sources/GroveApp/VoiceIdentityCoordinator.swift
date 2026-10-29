import Foundation
import GroveInference

@MainActor
final class VoiceIdentityCoordinator: ObservableObject {
    @Published private(set) var isSavingSpeakerProfile = false
    @Published private(set) var isRecognizingVoices = false
    @Published private(set) var isCommittingVoiceEnrollment = false
    @Published private(set) var registeredVoiceProfileIDs: Set<UUID> = []
    // Registered with a different voice model; the policy skips them, so the person must re-enroll.
    @Published private(set) var staleVoiceProfileIDs: Set<UUID> = []
    @Published private(set) var identificationMessages: [UUID: String] = [:]
    let isAvailable: Bool
    let enrollmentSession: VoiceEnrollmentSession
    private(set) var registryLoaded = false

    var meeting: (UUID) -> MeetingRecord? = { _ in nil }
    var isBusy: () -> Bool = { false }
    var mayIdentify: () -> Bool = { true }
    var onError: ((String) -> Void)?

    private let library: LibraryStore
    private let transcripts: TranscriptDocumentStore
    private let vault: SpeakerVoiceVault
    private let extractor: any SpeakerVoiceExtracting
    private let workDirectory: URL
    private let persists: Bool
    private var task: Task<Bool, Never>?

    init(library: LibraryStore, transcripts: TranscriptDocumentStore, vault: SpeakerVoiceVault,
         extractor: any SpeakerVoiceExtracting, enrollmentSession: VoiceEnrollmentSession,
         workDirectory: URL, persists: Bool, isAvailable: Bool) {
        self.library = library
        self.transcripts = transcripts
        self.vault = vault
        self.extractor = extractor
        self.enrollmentSession = enrollmentSession
        self.workDirectory = workDirectory
        self.persists = persists
        self.isAvailable = isAvailable
    }

    var isWorking: Bool { isSavingSpeakerProfile || isRecognizingVoices }

    func isRegistered(_ profileID: UUID) -> Bool { registeredVoiceProfileIDs.contains(profileID) }
    func needsReenrollment(_ profileID: UUID) -> Bool { staleVoiceProfileIDs.contains(profileID) }

    func hasStorage(_ profileID: UUID) -> Bool {
        isRegistered(profileID) || library.profile(profileID)?.voiceStorageReferenced == true
    }

    func message(for meetingID: UUID) -> String? { identificationMessages[meetingID] }

    func canForget(_ profiles: [SavedSpeakerProfile]) -> Bool {
        profiles.isEmpty || (registryLoaded && profiles.allSatisfy { !hasStorage($0.id) })
    }

    func automaticIdentificationEnabled(folderID: UUID) -> Bool {
        isAvailable && library.folders.first(where: { $0.id == folderID })?.automaticSpeakerIdentification == true
    }

    func setAutomaticIdentification(folderID: UUID, enabled: Bool) -> Bool {
        guard !enabled || isAvailable else { onError?(VoiceIdentityReleaseGate.message); return false }
        guard !isBusy(), let index = library.folders.firstIndex(where: { $0.id == folderID }) else { return false }
        return library.edit { $0.folders[index].automaticSpeakerIdentification = enabled }
    }

    func refreshEnrollmentStatus() async {
        let profiles = library.library.speakerProfiles ?? []
        do {
            var found: Set<UUID> = []
            for profile in profiles where try await vault.hasRecord(profileID: profile.id) { found.insert(profile.id) }
            guard profiles == library.library.speakerProfiles ?? [] else { return }
            registeredVoiceProfileIDs = found
            registryLoaded = true
        } catch {
            registryLoaded = false
            onError?("목소리 등록 상태를 읽지 못했습니다. 기존 자료는 보존됩니다. \(error.localizedDescription)")
        }
    }

    func refreshModelCompatibility() async {
        let extractor = extractor
        guard let current = try? await Task.detached(priority: .utility, operation: { try extractor.currentModelIdentifier() }).value
        else { return }
        let profiles = (library.library.speakerProfiles ?? []).filter { registeredVoiceProfileIDs.contains($0.id) }
        guard let metadata = try? await vault.metadata(profiles: profiles) else { return }
        staleVoiceProfileIDs = Set(metadata.filter { $0.modelIdentifier != current }.map(\.profileID))
    }

    func enrollmentCandidates(meetingID: UUID, speakerID: UUID) -> [DocumentUtterance] {
        guard let document = transcripts.documents[meetingID] else { return [] }
        return VoiceIdentitySelection.candidates(in: document, speakerID: speakerID)
    }

    func enrollmentDuration(_ utterance: DocumentUtterance) -> Double {
        let range = VoiceIdentitySelection.range(for: utterance)
        return max(0, range.end - range.start)
    }

    func cancelWork() {
        guard !isCommittingVoiceEnrollment else { return }
        task?.cancel()
    }

    func waitForWork() async { _ = await task?.value }

    func confirmIdentity(meetingID: UUID, speakerID: UUID) -> Bool {
        transcripts.edit(meetingID: meetingID) { try $0.confirmSpeakerIdentity(speakerID) }
    }

    func rejectIdentity(meetingID: UUID, speakerID: UUID) -> Bool {
        transcripts.edit(meetingID: meetingID) { try $0.rejectSpeakerIdentity(speakerID) }
    }

    func applySavedSpeaker(profileID: UUID, meetingID: UUID, speakerID: UUID) -> Bool {
        guard let meeting = meeting(meetingID),
              let profile = library.profile(profileID), profile.folderID == meeting.folderID else { return false }
        return transcripts.edit(meetingID: meetingID) { try $0.applySpeakerProfile(profile, to: speakerID, similarity: nil, confirmed: true) }
    }

    func removeSpeakerProfile(id: UUID) -> Bool {
        guard !isBusy() else { return false }
        guard registryLoaded, !hasStorage(id) else {
            onError?("등록한 목소리를 먼저 삭제한 뒤 이름을 삭제해 주세요.")
            return false
        }
        return library.edit { $0.speakerProfiles?.removeAll { $0.id == id } }
    }

    func saveSpeakerProfile(meetingID: UUID, speakerID: UUID, name: String) async -> Bool {
        guard !isBusy(), let meeting = meeting(meetingID), let original = transcripts.documents[meetingID] else { return false }
        guard let folderID = meeting.folderID, library.containsFolder(folderID) else {
            onError?(VoiceProfileError.folderRequired.localizedDescription); return false
        }
        if let linked = original.speakers.first(where: { $0.id == speakerID })?.profileMatch,
           library.speakerProfiles(in: folderID).contains(where: { $0.id == linked.profileID }) {
            onError?("이미 저장한 화자가 연결되어 있습니다. 다른 사람이라면 먼저 화자 이름을 변경해 연결을 해제해 주세요.")
            return false
        }
        guard !library.speakerProfiles(in: folderID).contains(where: {
            $0.sourceMeetingID == meetingID && $0.sourceSpeakerID == speakerID
        }) else {
            onError?("이 화자에서 저장한 이름이 이미 있습니다. 폴더의 기존 화자를 적용하거나, 등록 목소리와 이름을 먼저 삭제해 주세요.")
            return false
        }
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { return false }
        isSavingSpeakerProfile = true
        defer { isSavingSpeakerProfile = false }
        do {
            guard original.speakers.contains(where: { $0.id == speakerID }) else { throw TranscriptEditError.missingSpeaker }
            guard transcripts.documents[meetingID] == original,
                  self.meeting(meetingID)?.folderID == folderID else { throw VoiceProfileError.recordingChanged }
            let profile = SavedSpeakerProfile(folderID: folderID, name: name,
                sourceMeetingID: meetingID, sourceRevisionID: original.revisionID, sourceSpeakerID: speakerID, createdAt: Date())
            guard library.edit({
                if $0.speakerProfiles == nil { $0.speakerProfiles = [] }
                $0.speakerProfiles?.removeAll { $0.folderID == folderID && $0.sourceMeetingID == meetingID && $0.sourceSpeakerID == speakerID }
                $0.speakerProfiles?.append(profile)
            }) else { return false }
            let applied = transcripts.edit(meetingID: meetingID) { try $0.applySpeakerProfile(profile, to: speakerID, similarity: nil, confirmed: true) }
            if !applied { onError?("화자는 폴더에 저장했지만 현재 전사에 이름을 적용하지 못했습니다. 폴더의 화자 목록에서 다시 적용할 수 있습니다.") }
            return true
        } catch {
            onError?("화자를 저장하지 못했습니다. \(error.localizedDescription)")
            return false
        }
    }

    func enrollRecordedVoice(profileID: UUID, permissionConfirmed: Bool) async -> Bool {
        let session = enrollmentSession
        guard !isBusy(), permissionConfirmed, let source = session.source,
              let review = session.review, review.canExtract,
              let profile = library.profile(profileID) else { return false }
        isSavingSpeakerProfile = true
        defer { isSavingSpeakerProfile = false; isCommittingVoiceEnrollment = false }
        let before = library.library
        let task = Task { () -> Bool in
            do {
                session.stopPreview()
                let hash = try await VoiceIdentitySelection.audioHash(source)
                let extracted = try await extractor.extractSamples(source: source, ranges: review.ranges,
                    workingDirectory: source.deletingLastPathComponent())
                guard extracted.count == review.ranges.count,
                      zip(extracted, review.ranges).allSatisfy({ $0.0.range == $0.1 }) else { throw VoiceIdentityError.invalidSelection }
                let samples = extracted.map { sample in
                    VoiceEnrollmentSample(utteranceID: UUID(), start: sample.range.start, end: sample.range.end,
                        voice: sample.voicePrint, sourceMeetingID: nil, sourceRevisionID: nil, audioSHA256: hash)
                }
                try VoiceRecognitionPolicy.validateEnrollment(samples: samples)
                try Task.checkCancellation()
                guard library.library == before, session.source == source,
                      try await VoiceIdentitySelection.audioHash(source) == hash else { throw VoiceIdentityError.changed }
                isCommittingVoiceEnrollment = true
                guard library.edit({ updated in
                    if let index = updated.speakerProfiles?.firstIndex(where: { $0.id == profileID }) {
                        updated.speakerProfiles?[index].voiceStorageReferenced = true
                    }
                }) else { return false }
                do {
                    try await vault.put(.init(profileID: profileID, folderID: profile.folderID,
                        modelIdentifier: samples[0].voice.modelIdentifier, samples: samples, createdAt: Date()))
                } catch SpeakerVoiceVaultError.publishedButCleanupFailed(let reason) {
                    onError?(SpeakerVoiceVaultError.publishedButCleanupFailed(reason).localizedDescription)
                }
                await refreshEnrollmentStatus()
                session.discard()
                return true
            } catch is CancellationError { return false }
            catch {
                await refreshEnrollmentStatus()
                session.error = error.localizedDescription
                return false
            }
        }
        return await track(task)
    }

    func removeVoiceEnrollment(profileID: UUID) async -> Bool {
        guard !isBusy(), let profile = library.profile(profileID) else { return false }
        isSavingSpeakerProfile = true
        defer { isSavingSpeakerProfile = false }
        do {
            try await vault.remove(profileID: profileID, folderID: profile.folderID)
            guard library.edit({ updated in
                if let index = updated.speakerProfiles?.firstIndex(where: { $0.id == profileID }) {
                    updated.speakerProfiles?[index].voiceStorageReferenced = nil
                }
            }) else { return false }
            await refreshEnrollmentStatus()
            return !registeredVoiceProfileIDs.contains(profileID)
        } catch {
            await refreshEnrollmentStatus()
            onError?("목소리 삭제를 마치지 못했습니다. 이름과 기존 전사는 유지됩니다. \(error.localizedDescription)")
            return false
        }
    }

    func enrollVoice(meetingID: UUID, speakerID: UUID, name: String,
                     selectedUtteranceIDs: Set<UUID>, permissionConfirmed: Bool,
                     enableAutomaticIdentification: Bool) async -> Bool {
        guard !isBusy() else { return false }
        guard isAvailable else { onError?(VoiceIdentityReleaseGate.message); return false }
        guard permissionConfirmed else { onError?(VoiceIdentityError.consentRequired.localizedDescription); return false }
        isSavingSpeakerProfile = true
        defer { isSavingSpeakerProfile = false }
        let task = Task { await self.performEnrollment(meetingID: meetingID, speakerID: speakerID, name: name,
            selectedUtteranceIDs: selectedUtteranceIDs, enableAutomaticIdentification: enableAutomaticIdentification) }
        return await track(task)
    }

    func identifySpeakers(meetingID: UUID) async {
        guard !isBusy() else { return }
        guard isAvailable else {
            identificationMessages[meetingID] = VoiceIdentityReleaseGate.message
            return
        }
        await runIdentification(meetingID: meetingID, automatic: false)
    }

    func identifyAfterProcessing(meetingID: UUID) async {
        guard let folderID = meeting(meetingID)?.folderID, automaticIdentificationEnabled(folderID: folderID) else { return }
        await runIdentification(meetingID: meetingID, automatic: true)
    }

    private func track(_ task: Task<Bool, Never>) async -> Bool {
        self.task = task
        let result = await withTaskCancellationHandler { await task.value } onCancel: { task.cancel() }
        self.task = nil
        return result
    }

    private func performEnrollment(meetingID: UUID, speakerID: UUID, name: String,
                                   selectedUtteranceIDs: Set<UUID>, enableAutomaticIdentification: Bool) async -> Bool {
        defer { isCommittingVoiceEnrollment = false }
        do {
            guard persists, let meeting = meeting(meetingID),
                  let document = transcripts.documents[meetingID], transcripts.errors[meetingID] == nil,
                  let speaker = document.speakers.first(where: { $0.id == speakerID }),
                  let path = meeting.audioPath else { throw VoiceIdentityError.missingSource }
            guard let folderID = meeting.folderID, library.containsFolder(folderID) else {
                throw VoiceProfileError.folderRequired
            }
            guard let name = LibraryStore.singleLineName(name) else { throw TranscriptEditError.emptyName }
            let beforeLibrary = library.library
            let candidates = try VoiceIdentitySelection.selected(in: document, speakerID: speakerID, ids: selectedUtteranceIDs)
            let profiles = library.speakerProfiles(in: folderID)
            let existing = profiles.first(where: { $0.id == speaker.profileMatch?.profileID })
                ?? profiles.first(where: { $0.sourceMeetingID == meetingID && $0.sourceSpeakerID == speakerID })
            guard existing == nil || existing?.name == name else { throw VoiceIdentityError.namesOnlyConflict }
            var profile = existing ?? SavedSpeakerProfile(folderID: folderID, name: name, sourceMeetingID: meetingID,
                sourceRevisionID: document.revisionID, sourceSpeakerID: speakerID, createdAt: Date())
            profile.voiceStorageReferenced = true
            let source = URL(fileURLWithPath: path)
            let hash = try await VoiceIdentitySelection.audioHash(source)
            let extracted = try await extractSamples(source: source, utterances: candidates)
            let samples = zip(candidates, extracted).map { utterance, sample in
                VoiceEnrollmentSample(utteranceID: utterance.id, start: sample.range.start, end: sample.range.end,
                    voice: sample.voicePrint, sourceMeetingID: meetingID, sourceRevisionID: document.revisionID, audioSHA256: hash)
            }
            try VoiceRecognitionPolicy.validateEnrollment(samples: samples)
            guard try await VoiceIdentitySelection.audioHash(source) == hash else { throw VoiceIdentityError.changed }
            try Task.checkCancellation()
            guard transcripts.documents[meetingID] == document, library.library == beforeLibrary,
                  self.meeting(meetingID)?.audioPath == path,
                  self.meeting(meetingID)?.folderID == folderID else { throw VoiceIdentityError.changed }

            // Publish names first: a crash before the encrypted commit can leave only
            // a harmless names-only entry, never a hidden/orphaned enrollment.
            isCommittingVoiceEnrollment = true
            guard library.edit({
                if $0.speakerProfiles == nil { $0.speakerProfiles = [] }
                $0.speakerProfiles?.removeAll { $0.id == profile.id }
                $0.speakerProfiles?.append(profile)
            }) else { return false }
            var cleanupWarning: String?
            do {
                try await vault.put(VoiceEnrollmentRecord(profileID: profile.id, folderID: folderID,
                    modelIdentifier: samples[0].voice.modelIdentifier, samples: samples, createdAt: Date()))
            } catch SpeakerVoiceVaultError.publishedButCleanupFailed(let reason) {
                cleanupWarning = SpeakerVoiceVaultError.publishedButCleanupFailed(reason).localizedDescription
            } catch {
                // Retaining a names-only profile keeps any failed-generation cleanup
                // addressable. Never claim the encrypted commit succeeded.
                await refreshEnrollmentStatus()
                throw error
            }
            await refreshEnrollmentStatus()
            var warnings: [String] = cleanupWarning.map { [$0] } ?? []
            if transcripts.documents[meetingID] == document, self.meeting(meetingID)?.folderID == folderID {
                if !transcripts.edit(meetingID: meetingID, mutation: {
                    try $0.applySpeakerProfile(profile, to: speakerID, similarity: nil, confirmed: true)
                }) { warnings.append("등록은 저장했지만 현재 전사에 이름을 적용하지 못했습니다.") }
            } else { warnings.append("등록은 저장했지만 처리 중 바뀐 전사에는 이름을 적용하지 않았습니다.") }
            if let index = library.folders.firstIndex(where: { $0.id == folderID }),
               !library.edit({ $0.folders[index].automaticSpeakerIdentification = enableAutomaticIdentification }) {
                warnings.append("폴더의 자동 식별 설정은 저장하지 못했습니다.")
            }
            let message = warnings.isEmpty ? "목소리를 등록했습니다. 같은 폴더의 다른 녹음에서 이름을 추정할 수 있습니다." : warnings.joined(separator: " ")
            identificationMessages[meetingID] = message
            if !warnings.isEmpty { onError?(message) }
            return true
        } catch is CancellationError {
            onError?("목소리 등록을 취소했습니다.")
            return false
        } catch {
            onError?("목소리를 등록하지 못했습니다. \(error.localizedDescription)")
            return false
        }
    }

    private func extractSamples(source: URL, utterances: [DocumentUtterance]) async throws -> [VoiceEmbeddingSample] {
        let directory = workDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
                                              attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: directory) }
        let ranges = utterances.map { VoiceIdentitySelection.range(for: $0) }
        let samples = try await extractor.extractSamples(source: source, ranges: ranges, workingDirectory: directory)
        guard samples.count == ranges.count, zip(samples, ranges).allSatisfy({
            $0.0.range.start == $0.1.start && $0.0.range.end == $0.1.end
        }) else { throw VoiceIdentityError.invalidSelection }
        return samples
    }

    private func runIdentification(meetingID: UUID, automatic: Bool) async {
        guard isAvailable, !isRecognizingVoices, !isSavingSpeakerProfile, mayIdentify() else { return }
        isRecognizingVoices = true
        _ = await track(Task { await self.performIdentification(meetingID: meetingID, automatic: automatic) })
        isRecognizingVoices = false
    }

    private func performIdentification(meetingID: UUID, automatic: Bool) async -> Bool {
        do {
            guard persists, let meeting = meeting(meetingID),
                  let document = transcripts.documents[meetingID], transcripts.errors[meetingID] == nil,
                  let path = meeting.audioPath else { throw VoiceIdentityError.missingSource }
            guard let folderID = meeting.folderID else { throw VoiceProfileError.folderRequired }
            if automatic && !automaticIdentificationEnabled(folderID: folderID) { return false }
            let beforeLibrary = library.library
            let availableProfiles = library.speakerProfiles(in: folderID)
            let profiles = meeting.attendance?.candidates(in: availableProfiles) ?? availableProfiles
            var records: [VoiceEnrollmentRecord] = []
            for profile in profiles {
                try Task.checkCancellation()
                if let record = try await vault.load(profileID: profile.id, folderID: folderID) { records.append(record) }
            }
            guard !records.isEmpty else {
                if automatic { return false }
                throw VoiceIdentityError.noEnrolledVoices
            }
            let source = URL(fileURLWithPath: path)
            let hash = try await VoiceIdentitySelection.audioHash(source)
            records.removeAll { $0.samples.contains(where: { $0.sourceMeetingID == meetingID || $0.audioSHA256.lowercased() == hash }) }
            guard !records.isEmpty else { throw VoiceIdentityError.sourceEnrollmentOnly }
            var proposals: [(UUID, SavedSpeakerProfile, Float, SpeakerIdentityEvidence)] = []
            var deferredReasons: [String] = []
            for speaker in document.speakers where VoiceIdentitySelection.permitsAutomaticName(speaker, in: document) {
                try Task.checkCancellation()
                let utterances = VoiceIdentitySelection.query(in: document, speakerID: speaker.id)
                guard utterances.count >= VoiceRecognitionPolicy.minimumQuerySamples else {
                    deferredReasons.append("\(speaker.name): 단독 발화 부족")
                    continue
                }
                let samples = try await extractSamples(source: source, utterances: utterances)
                let decision = VoiceRecognitionPolicy.evaluate(samples: samples.map(\.voicePrint), enrollments: records)
                if let id = decision.profileID, let score = decision.similarity,
                   let profile = profiles.first(where: { $0.id == id }),
                   let record = records.first(where: { $0.profileID == id }) {
                    let evidence = SpeakerIdentityEvidence(modelIdentifier: record.modelIdentifier,
                        policyVersion: decision.policyVersion, enrollmentCreatedAt: record.createdAt,
                        queryUtteranceIDs: utterances.map(\.id))
                    proposals.append((speaker.id, profile, score, evidence))
                }
                else { deferredReasons.append("\(speaker.name): \(decision.reason?.message ?? "이름 추정 보류")") }
            }
            let assignments = Dictionary(uniqueKeysWithValues: proposals.map { ($0.0, $0.1.id) })
            let rejected = VoiceIdentitySelection.conflictingAssignments(assignments, in: document)
            let unique = proposals.filter { !rejected.contains($0.0) }
            if unique.count != proposals.count { deferredReasons.append("겹친 발화가 같은 팀원으로 연결되어 이름을 적용하지 않았습니다.") }
            guard try await VoiceIdentitySelection.audioHash(source) == hash else { throw VoiceIdentityError.changed }
            try Task.checkCancellation()
            guard transcripts.documents[meetingID] == document, library.library == beforeLibrary,
                  self.meeting(meetingID)?.folderID == folderID,
                  self.meeting(meetingID)?.audioPath == path else { throw VoiceIdentityError.changed }
            if !unique.isEmpty {
                guard transcripts.edit(meetingID: meetingID, mutation: { updated in
                    for (id, profile, score, evidence) in unique {
                        try updated.applySpeakerProfile(profile, to: id, similarity: score, confirmed: false, identityEvidence: evidence)
                    }
                }) else { return false }
            }
            let linkedPeople = Set(unique.map { $0.1.id }).count
            let summary = unique.isEmpty ? "연결할 이름을 찾지 못했습니다." : "팀원 \(linkedPeople)명의 이름을 연결했습니다."
            identificationMessages[meetingID] = ([summary] + deferredReasons).joined(separator: "\n")
            return true
        } catch is CancellationError {
            identificationMessages[meetingID] = "이름 찾기를 취소했습니다. 전사문과 기존 이름은 유지됩니다."
            return false
        } catch {
            let message = "이름 찾기를 마치지 못했습니다. 전사문과 기존 이름은 유지됩니다. \(error.localizedDescription)"
            identificationMessages[meetingID] = message
            if !automatic { onError?(message) }
            return false
        }
    }
}
