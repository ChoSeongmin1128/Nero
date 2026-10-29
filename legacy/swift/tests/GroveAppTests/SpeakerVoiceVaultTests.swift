import Darwin
import Foundation
import GroveInference
import Testing
@testable import GroveApp

private enum VaultTestFailure: Error { case injected }

private final class MemoryVoiceKeys: SpeakerVoiceKeyStore, @unchecked Sendable {
    private let lock = NSLock()
    private var values: [UUID: [UUID: Data]] = [:]
    private var rejectInsert = false
    private var rejectedDeletion: UUID?
    private var rejectEveryDeletion = false
    private var loads = 0

    func load(profileID: UUID, generationID: UUID) throws -> Data? {
        lock.withLock {
            loads += 1
            return values[profileID]?[generationID]
        }
    }

    func insert(_ key: Data, profileID: UUID, generationID: UUID) throws {
        try lock.withLock {
            if rejectInsert { throw VaultTestFailure.injected }
            #expect(values[profileID]?[generationID] == nil)
            values[profileID, default: [:]][generationID] = key
        }
    }

    func remove(profileID: UUID, generationID: UUID) throws {
        try lock.withLock {
            if rejectEveryDeletion || rejectedDeletion == generationID { throw VaultTestFailure.injected }
            values[profileID]?[generationID] = nil
        }
    }

    func generations(profileID: UUID) throws -> [UUID] {
        lock.withLock { Array(values[profileID, default: [:]].keys) }
    }

    func failInsert(_ value: Bool) { lock.withLock { rejectInsert = value } }
    func failDeleting(_ generation: UUID?) { lock.withLock { rejectedDeletion = generation } }
    func failEveryDeletion(_ value: Bool) { lock.withLock { rejectEveryDeletion = value } }
    func loadCount() -> Int { lock.withLock { loads } }
}

private struct VaultFixture {
    let root: URL
    let directory: URL
    let keys = MemoryVoiceKeys()

    init() throws {
        guard let resolved = realpath(NSTemporaryDirectory(), nil) else { throw VaultTestFailure.injected }
        defer { free(resolved) }
        root = URL(fileURLWithPath: String(cString: resolved), isDirectory: true)
            .appendingPathComponent("grove-vault-test-" + UUID().uuidString, isDirectory: true)
        directory = root.appendingPathComponent("Voices", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false,
                                               attributes: [.posixPermissions: 0o700])
    }

    func clean() { try? FileManager.default.removeItem(at: root) }
    func vault(failure: @escaping @Sendable (VoiceVaultIOPoint) throws -> Void = { _ in }) -> SpeakerVoiceVault {
        SpeakerVoiceVault(directory: directory, keyStore: keys, ioFailure: failure)
    }
    func file(_ record: VoiceEnrollmentRecord) -> URL {
        directory.appendingPathComponent(record.profileID.uuidString.lowercased() + ".voice")
    }
}

private func enrollment(profileID: UUID = UUID(), folderID: UUID = UUID(), dimension: Int = 0) -> VoiceEnrollmentRecord {
    let model = "synthetic-local-voice-model-not-plaintext"
    var values = [Float](repeating: 0, count: 256)
    values[dimension] = 1
    let voice = SpeakerVoicePrint(modelIdentifier: model, embedding: values, speechDuration: 4, sampleCount: 1)
    return .init(profileID: profileID, folderID: folderID, modelIdentifier: model,
                 samples: [.init(utteranceID: UUID(), start: 1, end: 5, voice: voice,
                                 sourceMeetingID: UUID(), sourceRevisionID: UUID(),
                                 audioSHA256: String(repeating: "a", count: 64))],
                 createdAt: Date(timeIntervalSince1970: 1_700_000_000))
}

private func profile(for record: VoiceEnrollmentRecord) -> SavedSpeakerProfile {
    .init(id: record.profileID, folderID: record.folderID, name: "Synthetic participant",
          sourceMeetingID: UUID(), sourceRevisionID: UUID(), sourceSpeakerID: UUID(), createdAt: record.createdAt)
}

private func overwrite(_ url: URL, with data: Data) throws {
    let file = try FileHandle(forWritingTo: url)
    defer { try? file.close() }
    try file.truncate(atOffset: 0)
    try file.write(contentsOf: data)
}

struct SpeakerVoiceVaultTests {
    @Test func loginInteractionGuardRestoresPreviousStateOnSuccess() throws {
        var enabled = true
        var states: [Bool] = []
        let result = try LoginKeychainInteraction.run(readState: { enabled }, setState: {
            enabled = $0; states.append($0)
        }, operation: {
            #expect(!enabled)
            return 42
        })
        #expect(result == 42)
        #expect(enabled)
        #expect(states == [false, true])
    }

    @Test func loginInteractionGuardRestoresAfterQueryFailure() {
        var enabled = true
        #expect(throws: VaultTestFailure.self) {
            try LoginKeychainInteraction.run(readState: { enabled }, setState: { enabled = $0 }, operation: {
                #expect(!enabled)
                throw VaultTestFailure.injected
            })
        }
        #expect(enabled)
    }

    @Test func loginInteractionGuardPreservesAlreadyDisabledState() throws {
        var enabled = false
        try LoginKeychainInteraction.run(readState: { enabled }, setState: { enabled = $0 }, operation: {
            #expect(!enabled)
        })
        #expect(!enabled)
    }

    @Test func loginInteractionGuardSurfacesRestorationFailure() {
        var operations = 0
        var writes = 0
        #expect(throws: VaultTestFailure.self) {
            try LoginKeychainInteraction.run(readState: { true }, setState: { _ in
                writes += 1
                if writes == 2 { throw VaultTestFailure.injected }
            }, operation: { operations += 1 })
        }
        #expect(operations == 1)
        #expect(writes == 2)
    }

    @Test func roundTripAndRestartKeepOnlyEncryptedFile() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        let vault = fixture.vault()
        #expect(try await vault.load(profileID: record.profileID, folderID: record.folderID) == nil)
        #expect(try await !vault.hasRecord(profileID: record.profileID))
        #expect(!FileManager.default.fileExists(atPath: fixture.directory.path))
        try await vault.put(record)
        #expect(try await fixture.vault().load(profileID: record.profileID, folderID: record.folderID) == record)
        let data = try Data(contentsOf: fixture.file(record))
        let text = try #require(String(data: data, encoding: .utf8))
        #expect(!text.contains(record.modelIdentifier))
        #expect(!text.contains(record.samples[0].audioSHA256))
        #expect(!text.contains("embedding"))
        #expect(!text.contains("sourceMeetingID"))
        #expect(try FileManager.default.contentsOfDirectory(atPath: fixture.directory.path).count == 1)
        let permissions = try FileManager.default.attributesOfItem(atPath: fixture.file(record).path)[.posixPermissions] as? Int
        #expect(permissions == 0o600)
        let directoryPermissions = try FileManager.default.attributesOfItem(atPath: fixture.directory.path)[.posixPermissions] as? Int
        #expect(directoryPermissions == 0o700)
    }

    @Test func metadataUsesExplicitProfilesAndPresenceDoesNotReadKeychain() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let first = enrollment(), second = enrollment()
        let vault = fixture.vault()
        try await vault.put(first)
        try await vault.put(second)
        let before = fixture.keys.loadCount()
        #expect(try await vault.hasRecord(profileID: first.profileID))
        #expect(fixture.keys.loadCount() == before)
        #expect(try await vault.metadata(profiles: []).isEmpty)
        let metadata = try await vault.metadata(profiles: [profile(for: first)])
        #expect(metadata.count == 1)
        #expect(metadata.first?.profileID == first.profileID)
        #expect(metadata.first?.sampleCount == 1)
        #expect(metadata.first?.speechDuration == 4)
    }

    @Test func replacementRotatesKeyAndMakesOldCiphertextUnreadable() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let first = enrollment()
        let second = enrollment(profileID: first.profileID, folderID: first.folderID, dimension: 1)
        let vault = fixture.vault()
        try await vault.put(first)
        let old = try Data(contentsOf: fixture.file(first))
        let oldGeneration = try #require(fixture.keys.generations(profileID: first.profileID).first)
        try await vault.put(second)
        #expect(try await vault.load(profileID: first.profileID, folderID: first.folderID) == second)
        #expect(try fixture.keys.load(profileID: first.profileID, generationID: oldGeneration) == nil)
        #expect(try fixture.keys.generations(profileID: first.profileID).count == 1)
        try overwrite(fixture.file(first), with: old)
        await #expect(throws: SpeakerVoiceVaultError.missingKey) {
            try await vault.load(profileID: first.profileID, folderID: first.folderID)
        }
    }

    @Test func deletionRemovesEveryGenerationAndRestoredCiphertextCannotLoad() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        let vault = fixture.vault()
        try await vault.put(record)
        let backup = try Data(contentsOf: fixture.file(record))
        try fixture.keys.insert(Data(repeating: 9, count: 32), profileID: record.profileID, generationID: UUID())
        try await vault.remove(profileID: record.profileID, folderID: record.folderID)
        #expect(try fixture.keys.generations(profileID: record.profileID).isEmpty)
        #expect(try await !vault.hasRecord(profileID: record.profileID))
        #expect(try FileManager.default.contentsOfDirectory(atPath: fixture.directory.path).isEmpty)
        #expect(FileManager.default.createFile(atPath: fixture.file(record).path, contents: backup,
                                              attributes: [.posixPermissions: 0o600]))
        await #expect(throws: SpeakerVoiceVaultError.missingKey) {
            try await vault.load(profileID: record.profileID, folderID: record.folderID)
        }
        // Explicit forgetting remains possible even when a key has already gone.
        try await vault.remove(profileID: record.profileID, folderID: record.folderID)
        try await vault.remove(profileID: record.profileID, folderID: record.folderID)
    }

    @Test func folderMismatchCannotReadReplaceOrDelete() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        let wrongFolder = UUID()
        let vault = fixture.vault()
        try await vault.put(record)
        let before = try Data(contentsOf: fixture.file(record))
        await #expect(throws: SpeakerVoiceVaultError.folderMismatch) {
            try await vault.load(profileID: record.profileID, folderID: wrongFolder)
        }
        await #expect(throws: SpeakerVoiceVaultError.folderMismatch) {
            try await vault.remove(profileID: record.profileID, folderID: wrongFolder)
        }
        await #expect(throws: SpeakerVoiceVaultError.folderMismatch) {
            try await vault.put(enrollment(profileID: record.profileID, folderID: wrongFolder))
        }
        #expect(try Data(contentsOf: fixture.file(record)) == before)
        #expect(try await vault.load(profileID: record.profileID, folderID: record.folderID) == record)
    }

    @Test func folderHeaderIsAuthenticatedNotJustCompared() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        let wrongFolder = UUID()
        let vault = fixture.vault()
        try await vault.put(record)
        var object = try #require(JSONSerialization.jsonObject(with: Data(contentsOf: fixture.file(record))) as? [String: Any])
        object["folderID"] = wrongFolder.uuidString
        try overwrite(fixture.file(record), with: JSONSerialization.data(withJSONObject: object))
        await #expect(throws: SpeakerVoiceVaultError.corruptRecord) {
            try await vault.load(profileID: record.profileID, folderID: wrongFolder)
        }
    }

    @Test func cipherTamperingFailsClosedAndDoesNotPermitReplacement() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        let vault = fixture.vault()
        try await vault.put(record)
        var object = try #require(JSONSerialization.jsonObject(with: Data(contentsOf: fixture.file(record))) as? [String: Any])
        let encodedSealed = try #require(object["sealed"] as? String)
        var sealed = try #require(Data(base64Encoded: encodedSealed))
        sealed[sealed.count / 2] ^= 1
        object["sealed"] = sealed.base64EncodedString()
        let changed = try JSONSerialization.data(withJSONObject: object)
        try overwrite(fixture.file(record), with: changed)
        await #expect(throws: SpeakerVoiceVaultError.corruptRecord) {
            try await vault.load(profileID: record.profileID, folderID: record.folderID)
        }
        await #expect(throws: SpeakerVoiceVaultError.corruptRecord) { try await vault.put(record) }
        #expect(try Data(contentsOf: fixture.file(record)) == changed)
        #expect(try fixture.keys.generations(profileID: record.profileID).count == 1)
    }

    @Test func missingKeyFailsClosedAndDoesNotGenerateReplacement() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        let vault = fixture.vault()
        try await vault.put(record)
        let generation = try #require(fixture.keys.generations(profileID: record.profileID).first)
        try fixture.keys.remove(profileID: record.profileID, generationID: generation)
        await #expect(throws: SpeakerVoiceVaultError.missingKey) { try await vault.put(record) }
        #expect(try fixture.keys.generations(profileID: record.profileID).isEmpty)
    }

    @Test func writeOrPublishFailurePreservesOldEnrollmentAndCleansNewKey() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        try await fixture.vault().put(record)
        let before = try Data(contentsOf: fixture.file(record))
        let generations = try fixture.keys.generations(profileID: record.profileID)
        for point in [VoiceVaultIOPoint.beforeWrite, .beforePublish] {
            let vault = fixture.vault { reached in
                if reached == point { throw VaultTestFailure.injected }
            }
            await #expect(throws: VaultTestFailure.self) { try await vault.put(record) }
            #expect(try Data(contentsOf: fixture.file(record)) == before)
            #expect(try fixture.keys.generations(profileID: record.profileID) == generations)
            #expect(try FileManager.default.contentsOfDirectory(atPath: fixture.directory.path).count == 1)
        }
    }

    @Test func keyInsertFailureLeavesExistingFileUntouched() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        let vault = fixture.vault()
        try await vault.put(record)
        let before = try Data(contentsOf: fixture.file(record))
        fixture.keys.failInsert(true)
        await #expect(throws: VaultTestFailure.self) { try await vault.put(record) }
        #expect(try Data(contentsOf: fixture.file(record)) == before)
    }

    @Test func failedSaveWithFailedKeyCleanupIsExplicitAndRecoverable() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        try await fixture.vault().put(record)
        let before = try Data(contentsOf: fixture.file(record))
        fixture.keys.failEveryDeletion(true)
        let failing = fixture.vault { if $0 == .beforePublish { throw VaultTestFailure.injected } }
        do {
            try await failing.put(record)
            Issue.record("Expected failed-save-and-cleanup failure")
        } catch SpeakerVoiceVaultError.saveFailedAndKeyCleanupFailed { }
        #expect(try Data(contentsOf: fixture.file(record)) == before)
        #expect(try await failing.load(profileID: record.profileID, folderID: record.folderID) == record)
        #expect(try fixture.keys.generations(profileID: record.profileID).count == 2)
        fixture.keys.failEveryDeletion(false)
        try await fixture.vault().put(record)
        #expect(try fixture.keys.generations(profileID: record.profileID).count == 1)
    }

    @Test func replacementCleanupFailureIsExplicitAndCanBeRetried() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        let replacement = enrollment(profileID: record.profileID, folderID: record.folderID, dimension: 2)
        let vault = fixture.vault()
        try await vault.put(record)
        let oldGeneration = try #require(fixture.keys.generations(profileID: record.profileID).first)
        fixture.keys.failDeleting(oldGeneration)
        do {
            try await vault.put(replacement)
            Issue.record("Expected published-but-cleanup-failed")
        } catch SpeakerVoiceVaultError.publishedButCleanupFailed { }
        #expect(try await vault.load(profileID: record.profileID, folderID: record.folderID) == replacement)
        #expect(try fixture.keys.generations(profileID: record.profileID).count == 2)
        fixture.keys.failDeleting(nil)
        try await vault.put(replacement)
        #expect(try fixture.keys.generations(profileID: record.profileID).count == 1)
    }

    @Test func deletionKeyFailurePreservesFileAndCurrentKey() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        let vault = fixture.vault()
        try await vault.put(record)
        let generation = try #require(fixture.keys.generations(profileID: record.profileID).first)
        fixture.keys.failDeleting(generation)
        await #expect(throws: VaultTestFailure.self) {
            try await vault.remove(profileID: record.profileID, folderID: record.folderID)
        }
        #expect(try await vault.load(profileID: record.profileID, folderID: record.folderID) == record)
    }

    @Test func deletionFileFailureDoesNotRestoreRemovedKeys() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        try await fixture.vault().put(record)
        let vault = fixture.vault { if $0 == .beforeRemoveCiphertext { throw VaultTestFailure.injected } }
        await #expect(throws: VaultTestFailure.self) {
            try await vault.remove(profileID: record.profileID, folderID: record.folderID)
        }
        #expect(try fixture.keys.generations(profileID: record.profileID).isEmpty)
        #expect(try await vault.hasRecord(profileID: record.profileID))
        await #expect(throws: SpeakerVoiceVaultError.missingKey) {
            try await vault.load(profileID: record.profileID, folderID: record.folderID)
        }
        try await fixture.vault().remove(profileID: record.profileID, folderID: record.folderID)
        #expect(try await !vault.hasRecord(profileID: record.profileID))
    }

    @Test func symlinkDirectoryAndAncestorAreRejected() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let destination = fixture.root.appendingPathComponent("Elsewhere", isDirectory: true)
        try FileManager.default.createDirectory(at: destination, withIntermediateDirectories: false,
                                               attributes: [.posixPermissions: 0o700])
        try FileManager.default.createSymbolicLink(at: fixture.directory, withDestinationURL: destination)
        await #expect(throws: SpeakerVoiceVaultError.unsafePath) { try await fixture.vault().put(enrollment()) }
        let nested = SpeakerVoiceVault(directory: fixture.directory.appendingPathComponent("Nested"), keyStore: fixture.keys)
        await #expect(throws: SpeakerVoiceVaultError.unsafePath) { try await nested.put(enrollment()) }
        #expect(try FileManager.default.contentsOfDirectory(atPath: destination.path).isEmpty)
    }

    @Test func symlinkCiphertextIsNotReadReplacedOrRemoved() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        let vault = fixture.vault()
        try await vault.put(record)
        let external = fixture.root.appendingPathComponent("DoNotChange")
        try FileManager.default.moveItem(at: fixture.file(record), to: external)
        try FileManager.default.createSymbolicLink(at: fixture.file(record), withDestinationURL: external)
        let before = try Data(contentsOf: external)
        await #expect(throws: SpeakerVoiceVaultError.unsafePath) {
            try await vault.load(profileID: record.profileID, folderID: record.folderID)
        }
        await #expect(throws: SpeakerVoiceVaultError.unsafePath) { try await vault.put(record) }
        await #expect(throws: SpeakerVoiceVaultError.unsafePath) {
            try await vault.remove(profileID: record.profileID, folderID: record.folderID)
        }
        #expect(try Data(contentsOf: external) == before)
        #expect(try fixture.keys.generations(profileID: record.profileID).count == 1)
    }

    @Test func hardlinksAndGroupReadableFilesAreRejected() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        let vault = fixture.vault()
        try await vault.put(record)
        let hardlink = fixture.root.appendingPathComponent("Linked")
        try FileManager.default.linkItem(at: fixture.file(record), to: hardlink)
        await #expect(throws: SpeakerVoiceVaultError.unsafePath) {
            try await vault.load(profileID: record.profileID, folderID: record.folderID)
        }
        try FileManager.default.removeItem(at: hardlink)
        try FileManager.default.setAttributes([.posixPermissions: 0o640], ofItemAtPath: fixture.file(record).path)
        await #expect(throws: SpeakerVoiceVaultError.unsafePath) { try await vault.hasRecord(profileID: record.profileID) }
    }

    @Test func pendingCleanupRejectsSymlinksInsteadOfFollowingThem() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        let vault = fixture.vault()
        try await vault.put(record)
        let outside = fixture.root.appendingPathComponent("SyntheticPrivateFile")
        let data = Data("preserve-external-file".utf8)
        #expect(FileManager.default.createFile(atPath: outside.path, contents: data,
                                              attributes: [.posixPermissions: 0o600]))
        let pendingName = "." + record.profileID.uuidString.lowercased() + "." + UUID().uuidString.lowercased() + ".pending"
        try FileManager.default.createSymbolicLink(at: fixture.directory.appendingPathComponent(pendingName),
                                                  withDestinationURL: outside)
        do {
            try await vault.put(record)
            Issue.record("Expected explicit cleanup failure")
        } catch SpeakerVoiceVaultError.publishedButCleanupFailed { }
        #expect(try Data(contentsOf: outside) == data)
        #expect(try await vault.load(profileID: record.profileID, folderID: record.folderID) == record)
    }

    @Test func oversizedAndMalformedFilesFailBeforeKeyRead() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let record = enrollment()
        let vault = fixture.vault()
        try await vault.put(record)
        let before = fixture.keys.loadCount()
        try overwrite(fixture.file(record), with: Data(repeating: 0, count: 1_048_577))
        await #expect(throws: SpeakerVoiceVaultError.oversizedFile) {
            try await vault.load(profileID: record.profileID, folderID: record.folderID)
        }
        try overwrite(fixture.file(record), with: Data("{not-json".utf8))
        await #expect(throws: SpeakerVoiceVaultError.corruptRecord) {
            try await vault.load(profileID: record.profileID, folderID: record.folderID)
        }
        #expect(fixture.keys.loadCount() == before)
    }

    @Test func invalidStructureDoesNotCreateFilesOrKeys() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let valid = enrollment()
        let invalid = VoiceEnrollmentRecord(profileID: valid.profileID, folderID: valid.folderID,
                                            modelIdentifier: valid.modelIdentifier, samples: [], createdAt: valid.createdAt)
        await #expect(throws: SpeakerVoiceVaultError.invalidRecord) { try await fixture.vault().put(invalid) }
        #expect(try fixture.keys.generations(profileID: valid.profileID).isEmpty)
        #expect(!FileManager.default.fileExists(atPath: fixture.directory.path))
    }

    @Test func nonFiniteOrDifferentModelFeaturesCannotBeSaved() async throws {
        let fixture = try VaultFixture()
        defer { fixture.clean() }
        let valid = enrollment()
        let sample = valid.samples[0]
        for voice in [SpeakerVoicePrint(modelIdentifier: valid.modelIdentifier, embedding: [.nan], speechDuration: 4, sampleCount: 1),
                      SpeakerVoicePrint(modelIdentifier: "different", embedding: [1], speechDuration: 4, sampleCount: 1)] {
            let invalidSample = VoiceEnrollmentSample(utteranceID: sample.utteranceID, start: sample.start, end: sample.end,
                                                     voice: voice, sourceMeetingID: sample.sourceMeetingID,
                                                     sourceRevisionID: sample.sourceRevisionID, audioSHA256: sample.audioSHA256)
            let invalid = VoiceEnrollmentRecord(profileID: valid.profileID, folderID: valid.folderID,
                                                modelIdentifier: valid.modelIdentifier, samples: [invalidSample], createdAt: valid.createdAt)
            await #expect(throws: SpeakerVoiceVaultError.invalidRecord) { try await fixture.vault().put(invalid) }
        }
    }
}
