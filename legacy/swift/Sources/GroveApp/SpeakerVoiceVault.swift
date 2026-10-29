import CryptoKit
import Darwin
import Foundation
import GroveInference
import Security

struct VoiceEnrollmentSample: Codable, Equatable, Sendable {
    let utteranceID: UUID
    let start: Double
    let end: Double
    let voice: SpeakerVoicePrint
    let sourceMeetingID: UUID?
    let sourceRevisionID: UUID?
    let audioSHA256: String
}

struct VoiceEnrollmentRecord: Codable, Equatable, Sendable {
    let profileID: UUID
    let folderID: UUID
    let modelIdentifier: String
    let samples: [VoiceEnrollmentSample]
    let createdAt: Date
}

struct VoiceEnrollmentMetadata: Equatable, Sendable {
    let profileID: UUID
    let folderID: UUID
    let modelIdentifier: String
    let sampleCount: Int
    let speechDuration: Double
    let createdAt: Date
}

enum SpeakerVoiceVaultError: LocalizedError, Equatable {
    case invalidRecord
    case unsafePath
    case oversizedFile
    case corruptRecord
    case folderMismatch
    case missingKey
    case keychain(operation: String, status: Int32)
    case fileSystem(operation: String, code: Int32)
    case publishedButCleanupFailed(String)
    case saveFailedAndKeyCleanupFailed(String)

    var errorDescription: String? {
        switch self {
        case .invalidRecord: "목소리 등록 정보가 유효하지 않습니다. 원음을 확인하고 다시 등록해 주세요."
        case .unsafePath: "목소리 저장 경로의 연결이나 접근 권한이 안전하지 않습니다."
        case .oversizedFile: "목소리 등록 파일이 허용된 크기를 초과했습니다."
        case .corruptRecord: "목소리 등록 파일이 손상되었거나 내용이 변경되었습니다. 자동으로 초기화하지 않습니다."
        case .folderMismatch: "이 목소리 등록은 다른 폴더에 속합니다."
        case .missingKey: "목소리 암호화 키를 찾을 수 없습니다. 기존 등록을 자동으로 덮어쓰지 않습니다."
        case let .keychain(operation, status): "목소리 키체인 \(operation)에 실패했습니다 (\(status))."
        case let .fileSystem(operation, code): "목소리 파일 \(operation)에 실패했습니다 (\(code))."
        case let .publishedButCleanupFailed(reason):
            "새 목소리는 저장했지만 이전 암호화 자료 정리를 마치지 못했습니다. 다시 삭제하거나 등록해 주세요. \(reason)"
        case let .saveFailedAndKeyCleanupFailed(reason):
            "기존 목소리는 유지했습니다. 실패한 저장의 암호화 키 정리를 마치지 못했습니다. \(reason)"
        }
    }
}

/// Implementations must not prompt for authentication or return a newly created key
/// when an existing key is absent. The vault serializes every operation on this store.
protocol SpeakerVoiceKeyStore: Sendable {
    func load(profileID: UUID, generationID: UUID) throws -> Data?
    func insert(_ key: Data, profileID: UUID, generationID: UUID) throws
    func remove(profileID: UUID, generationID: UUID) throws
    func generations(profileID: UUID) throws -> [UUID]
}

struct KeychainSpeakerVoiceKeyStore: SpeakerVoiceKeyStore {
    enum Storage: Equatable, Sendable {
        case dataProtection
        case login
    }

    let service: String
    let storage: Storage

    init(storage: Storage = .dataProtection, service: String = "local.grove.speaker-voice-vault.v1") {
        self.storage = storage
        self.service = service
    }

    func load(profileID: UUID, generationID: UUID) throws -> Data? {
        var query = query(profileID: profileID, generationID: generationID)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = try withoutLoginInteraction { SecItemCopyMatching(query as CFDictionary, &result) }
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw error("읽기", status) }
        guard let data = result as? Data, data.count == 32 else { throw SpeakerVoiceVaultError.corruptRecord }
        return data
    }

    func insert(_ key: Data, profileID: UUID, generationID: UUID) throws {
        guard key.count == 32 else { throw SpeakerVoiceVaultError.invalidRecord }
        var attributes = query(profileID: profileID, generationID: generationID)
        if storage == .dataProtection {
            attributes[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        }
        attributes[kSecValueData as String] = key
        let status = try withoutLoginInteraction { SecItemAdd(attributes as CFDictionary, nil) }
        guard status == errSecSuccess else { throw error("저장", status) }
    }

    func remove(profileID: UUID, generationID: UUID) throws {
        let status = try withoutLoginInteraction {
            SecItemDelete(query(profileID: profileID, generationID: generationID) as CFDictionary)
        }
        guard status == errSecSuccess || status == errSecItemNotFound else { throw error("삭제", status) }
    }

    func generations(profileID: UUID) throws -> [UUID] {
        var query = baseQuery
        query[kSecReturnAttributes as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitAll
        var result: CFTypeRef?
        let status = try withoutLoginInteraction { SecItemCopyMatching(query as CFDictionary, &result) }
        if status == errSecItemNotFound { return [] }
        guard status == errSecSuccess else { throw error("목록 읽기", status) }
        guard let attributes = result as? [[String: Any]] else { throw SpeakerVoiceVaultError.corruptRecord }
        let prefix = profileID.uuidString.lowercased() + "."
        return try attributes.compactMap { item in
            guard let account = item[kSecAttrAccount as String] as? String else {
                throw SpeakerVoiceVaultError.corruptRecord
            }
            guard account.hasPrefix(prefix) else { return nil }
            guard let generation = UUID(uuidString: String(account.dropFirst(prefix.count))) else {
                throw SpeakerVoiceVaultError.corruptRecord
            }
            return generation
        }
    }

    private func withoutLoginInteraction<T>(_ operation: () throws -> T) throws -> T {
        guard storage == .login else { return try operation() }
        return try LoginKeychainInteraction.run(readState: {
            var previous = DarwinBoolean(false)
            let status = SecKeychainGetUserInteractionAllowed(&previous)
            guard status == errSecSuccess else { throw error("상호작용 설정 읽기", status) }
            return previous.boolValue
        }, setState: { enabled in
            let status = SecKeychainSetUserInteractionAllowed(enabled)
            guard status == errSecSuccess else { throw error("상호작용 설정 변경", status) }
        }, operation: operation)
    }

    private var baseQuery: [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: service,
         kSecAttrSynchronizable as String: false,
         kSecUseDataProtectionKeychain as String: storage == .dataProtection,
         kSecUseAuthenticationUI as String: kSecUseAuthenticationUIFail]
    }

    private func query(profileID: UUID, generationID: UUID) -> [String: Any] {
        var result = baseQuery
        result[kSecAttrAccount as String] = profileID.uuidString.lowercased() + "." + generationID.uuidString.lowercased()
        return result
    }

    private func error(_ operation: String, _ status: OSStatus) -> SpeakerVoiceVaultError {
        .keychain(operation: operation, status: status)
    }
}

/// The legacy login Keychain ignores the query-only UI suppression in some ACL
/// cases. This deprecated macOS switch is process-local, not system-wide. Serialize
/// Grove's use and restore it even after a failed query; unrelated framework work in
/// the same process may briefly observe interaction disabled. There is no silent
/// fallback between this explicitly selected backend and the entitlement-gated one.
enum LoginKeychainInteraction {
    private static let lock = NSLock()

    static func run<T>(readState: () throws -> Bool, setState: (Bool) throws -> Void,
                       operation: () throws -> T) throws -> T {
        try lock.withLock {
            let previous = try readState()
            try setState(false)
            let result = Result { try operation() }
            try setState(previous)
            return try result.get()
        }
    }
}

enum VoiceVaultIOPoint: Equatable, Sendable {
    case beforeWrite
    case beforePublish
    case beforeRemoveCiphertext
}

/// Voice features never enter library.json, transcript backups, or a plaintext file.
/// Each replacement has a fresh key; ciphertext is published before old keys are
/// removed. Key deletion cannot erase a previously exported key, a keychain backup,
/// an in-memory copy, or the source recording. Do not promise forensic erasure.
actor SpeakerVoiceVault {
    private let directory: URL
    private let keyStore: any SpeakerVoiceKeyStore
    private let ioFailure: @Sendable (VoiceVaultIOPoint) throws -> Void
    private static let maximumFileBytes = 1_048_576

    init(directory: URL, keyStore: any SpeakerVoiceKeyStore = KeychainSpeakerVoiceKeyStore(),
         ioFailure: @escaping @Sendable (VoiceVaultIOPoint) throws -> Void = { _ in }) {
        self.directory = directory
        self.keyStore = keyStore
        self.ioFailure = ioFailure
    }

    /// Presence only, not integrity or eligibility. This performs no Keychain access.
    func hasRecord(profileID: UUID) throws -> Bool {
        guard let folder = try VaultDirectory(url: directory, create: false) else { return false }
        defer { folder.close() }
        return try folder.contains(fileName(profileID))
    }

    func load(profileID: UUID, folderID: UUID) throws -> VoiceEnrollmentRecord? {
        guard let folder = try VaultDirectory(url: directory, create: false) else { return nil }
        defer { folder.close() }
        guard let envelope = try readEnvelope(profileID: profileID, in: folder) else { return nil }
        return try decrypt(envelope, profileID: profileID, folderID: folderID)
    }

    /// The explicit profile list is the scope boundary; do not discover people by
    /// scanning the registry. An unreadable registration is an error, not a reset.
    func metadata(profiles: [SavedSpeakerProfile]) throws -> [VoiceEnrollmentMetadata] {
        var result: [VoiceEnrollmentMetadata] = []
        for profile in profiles {
            guard let record = try load(profileID: profile.id, folderID: profile.folderID) else { continue }
            result.append(.init(profileID: record.profileID, folderID: record.folderID,
                                modelIdentifier: record.modelIdentifier, sampleCount: record.samples.count,
                                speechDuration: record.samples.reduce(0) { $0 + $1.end - $1.start },
                                createdAt: record.createdAt))
        }
        return result
    }

    func put(_ record: VoiceEnrollmentRecord) throws {
        try validate(record)
        guard let folder = try VaultDirectory(url: directory, create: true) else { throw SpeakerVoiceVaultError.unsafePath }
        defer { folder.close() }
        if let previous = try readEnvelope(profileID: record.profileID, in: folder) {
            _ = try decrypt(previous, profileID: record.profileID, folderID: record.folderID)
        }
        let generation = UUID()
        let key = SymmetricKey(size: .bits256)
        let plaintext = try JSONEncoder().encode(record)
        guard plaintext.count <= Self.maximumFileBytes / 2 else { throw SpeakerVoiceVaultError.oversizedFile }
        let sealed = try AES.GCM.seal(plaintext, using: key,
                                     authenticating: aad(profileID: record.profileID, folderID: record.folderID, generationID: generation))
        guard let combined = sealed.combined else { throw SpeakerVoiceVaultError.corruptRecord }
        let envelope = Envelope(schemaVersion: 1, profileID: record.profileID, folderID: record.folderID,
                                generationID: generation, sealed: combined)
        let encoded = try JSONEncoder().encode(envelope)
        try keyStore.insert(key.withUnsafeBytes { Data($0) }, profileID: record.profileID, generationID: generation)
        var published = false
        do {
            try folder.publish(encoded, name: fileName(record.profileID),
                               stagingName: stagingName(profileID: record.profileID, generationID: generation),
                               failure: ioFailure, published: &published)
        } catch {
            if published { throw SpeakerVoiceVaultError.publishedButCleanupFailed(error.localizedDescription) }
            do { try keyStore.remove(profileID: record.profileID, generationID: generation) }
            catch { throw SpeakerVoiceVaultError.saveFailedAndKeyCleanupFailed(error.localizedDescription) }
            throw error
        }
        do {
            try removeKeys(profileID: record.profileID, keeping: generation)
            try folder.removePending(profileID: record.profileID)
        } catch {
            throw SpeakerVoiceVaultError.publishedButCleanupFailed(error.localizedDescription)
        }
    }

    /// Explicit forgetting also removes an unreadable ciphertext. Header identity
    /// must still match, so a caller cannot delete another folder's registration.
    func remove(profileID: UUID, folderID: UUID) throws {
        let folder = try VaultDirectory(url: directory, create: false)
        defer { folder?.close() }
        let envelope = try folder.flatMap { try readEnvelope(profileID: profileID, in: $0) }
        if let envelope {
            guard envelope.folderID == folderID else { throw SpeakerVoiceVaultError.folderMismatch }
        }
        // Stale keys first: on cleanup failure the current, usable registration stays.
        let keys = try keyStore.generations(profileID: profileID)
        for generation in keys where generation != envelope?.generationID {
            try keyStore.remove(profileID: profileID, generationID: generation)
        }
        if let generation = envelope?.generationID {
            try keyStore.remove(profileID: profileID, generationID: generation)
        }
        try ioFailure(.beforeRemoveCiphertext)
        if let folder {
            try folder.removePending(profileID: profileID)
            try folder.remove(fileName(profileID))
            try folder.sync()
        }
    }

    private struct Envelope: Codable {
        let schemaVersion: Int
        let profileID: UUID
        let folderID: UUID
        let generationID: UUID
        let sealed: Data
    }

    private func fileName(_ profileID: UUID) -> String { profileID.uuidString.lowercased() + ".voice" }

    private func stagingName(profileID: UUID, generationID: UUID) -> String {
        "." + profileID.uuidString.lowercased() + "." + generationID.uuidString.lowercased() + ".pending"
    }

    private func aad(profileID: UUID, folderID: UUID, generationID: UUID) -> Data {
        Data("GroveVoiceVault|1|\(folderID.uuidString)|\(profileID.uuidString)|\(generationID.uuidString)".utf8)
    }

    private func readEnvelope(profileID: UUID, in folder: VaultDirectory) throws -> Envelope? {
        guard let data = try folder.read(fileName(profileID), maximumBytes: Self.maximumFileBytes) else { return nil }
        let value: Envelope
        do { value = try JSONDecoder().decode(Envelope.self, from: data) }
        catch { throw SpeakerVoiceVaultError.corruptRecord }
        guard value.schemaVersion == 1, value.profileID == profileID, value.sealed.count >= 28 else {
            throw SpeakerVoiceVaultError.corruptRecord
        }
        return value
    }

    private func decrypt(_ envelope: Envelope, profileID: UUID, folderID: UUID) throws -> VoiceEnrollmentRecord {
        guard envelope.folderID == folderID else { throw SpeakerVoiceVaultError.folderMismatch }
        guard let key = try keyStore.load(profileID: profileID, generationID: envelope.generationID) else {
            throw SpeakerVoiceVaultError.missingKey
        }
        guard key.count == 32 else { throw SpeakerVoiceVaultError.corruptRecord }
        let record: VoiceEnrollmentRecord
        do {
            let sealed = try AES.GCM.SealedBox(combined: envelope.sealed)
            let plaintext = try AES.GCM.open(sealed, using: SymmetricKey(data: key),
                                            authenticating: aad(profileID: profileID, folderID: folderID,
                                                                generationID: envelope.generationID))
            record = try JSONDecoder().decode(VoiceEnrollmentRecord.self, from: plaintext)
            try validate(record)
        } catch { throw SpeakerVoiceVaultError.corruptRecord }
        guard record.profileID == profileID, record.folderID == folderID else { throw SpeakerVoiceVaultError.corruptRecord }
        return record
    }

    private func removeKeys(profileID: UUID, keeping: UUID) throws {
        for generation in try keyStore.generations(profileID: profileID) where generation != keeping {
            try keyStore.remove(profileID: profileID, generationID: generation)
        }
    }

    private func validate(_ record: VoiceEnrollmentRecord) throws {
        guard !record.modelIdentifier.isEmpty, record.modelIdentifier.utf8.count <= 1_024,
              record.createdAt.timeIntervalSince1970.isFinite, !record.samples.isEmpty, record.samples.count <= 32,
              let dimension = record.samples.first?.voice.embedding.count, (1...4_096).contains(dimension) else {
            throw SpeakerVoiceVaultError.invalidRecord
        }
        for sample in record.samples {
            guard sample.start.isFinite, sample.end.isFinite, sample.start >= 0, sample.end > sample.start,
                  sample.voice.modelIdentifier == record.modelIdentifier, sample.voice.embedding.count == dimension,
                  sample.voice.embedding.allSatisfy(\.isFinite), sample.voice.embedding.contains(where: { $0 != 0 }),
                  sample.voice.speechDuration.isFinite, sample.voice.speechDuration > 0, sample.voice.sampleCount > 0,
                  sample.audioSHA256.utf8.count == 64,
                  sample.audioSHA256.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else {
                throw SpeakerVoiceVaultError.invalidRecord
            }
        }
    }
}

/// Directory-relative operations prevent following a replaced file or ancestor
/// symlink. Only the vault leaf is created; application-support parents must exist.
private struct VaultDirectory {
    let descriptor: Int32

    init?(url: URL, create: Bool) throws {
        guard url.isFileURL, url.host == nil || url.host == "", url.path.hasPrefix("/"),
              !url.path.utf8.contains(0),
              url.query == nil, url.fragment == nil else { throw SpeakerVoiceVaultError.unsafePath }
        let components = url.path.split(separator: "/").map(String.init)
        guard !components.isEmpty, components.allSatisfy({ $0 != "." && $0 != ".." }) else {
            throw SpeakerVoiceVaultError.unsafePath
        }
        var current = Darwin.open("/", O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
        guard current >= 0 else { throw Self.failure("경로 열기") }
        do {
            for (index, component) in components.enumerated() {
                let isLeaf = index == components.count - 1
                var next = openat(current, component, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
                if next < 0, errno == ENOENT {
                    guard isLeaf, create else { Darwin.close(current); return nil }
                    guard mkdirat(current, component, 0o700) == 0 || errno == EEXIST else { throw Self.failure("폴더 만들기") }
                    next = openat(current, component, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
                }
                guard next >= 0 else { throw SpeakerVoiceVaultError.unsafePath }
                Darwin.close(current)
                current = next
            }
            var attributes = stat()
            guard fstat(current, &attributes) == 0 else { throw Self.failure("폴더 확인") }
            guard attributes.st_uid == geteuid(), attributes.st_mode & 0o077 == 0 else {
                throw SpeakerVoiceVaultError.unsafePath
            }
            descriptor = current
        } catch {
            Darwin.close(current)
            throw error
        }
    }

    func close() { Darwin.close(descriptor) }

    func contains(_ name: String) throws -> Bool {
        var attributes = stat()
        guard fstatat(descriptor, name, &attributes, AT_SYMLINK_NOFOLLOW) == 0 else {
            if errno == ENOENT { return false }
            throw Self.failure("파일 확인")
        }
        guard attributes.st_mode & mode_t(S_IFMT) == mode_t(S_IFREG), attributes.st_nlink == 1,
              attributes.st_uid == geteuid(), attributes.st_mode & 0o077 == 0 else {
            throw SpeakerVoiceVaultError.unsafePath
        }
        return true
    }

    func read(_ name: String, maximumBytes: Int) throws -> Data? {
        guard try contains(name) else { return nil }
        let file = openat(descriptor, name, O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK)
        guard file >= 0 else { throw Self.failure("파일 열기") }
        defer { Darwin.close(file) }
        var attributes = stat()
        guard fstat(file, &attributes) == 0 else { throw Self.failure("파일 확인") }
        guard attributes.st_mode & mode_t(S_IFMT) == mode_t(S_IFREG), attributes.st_nlink == 1,
              attributes.st_uid == geteuid(), attributes.st_mode & 0o077 == 0 else { throw SpeakerVoiceVaultError.unsafePath }
        guard attributes.st_size >= 0, attributes.st_size <= off_t(maximumBytes) else { throw SpeakerVoiceVaultError.oversizedFile }
        var output = Data()
        var buffer = [UInt8](repeating: 0, count: 16_384)
        while true {
            let count = Darwin.read(file, &buffer, buffer.count)
            if count < 0, errno == EINTR { continue }
            guard count >= 0 else { throw Self.failure("파일 읽기") }
            if count == 0 { break }
            guard output.count + count <= maximumBytes else { throw SpeakerVoiceVaultError.oversizedFile }
            output.append(contentsOf: buffer.prefix(count))
        }
        return output
    }

    func publish(_ encrypted: Data, name: String, stagingName: String,
                 failure: @Sendable (VoiceVaultIOPoint) throws -> Void, published: inout Bool) throws {
        _ = try contains(name)
        try failure(.beforeWrite)
        let file = openat(descriptor, stagingName, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
        guard file >= 0 else { throw Self.failure("암호문 만들기") }
        do {
            try encrypted.withUnsafeBytes { bytes in
                var offset = 0
                while offset < bytes.count {
                    let written = Darwin.write(file, bytes.baseAddress!.advanced(by: offset), bytes.count - offset)
                    if written < 0, errno == EINTR { continue }
                    guard written > 0 else { throw Self.failure("암호문 쓰기") }
                    offset += written
                }
            }
            guard fsync(file) == 0 else { throw Self.failure("암호문 동기화") }
            Darwin.close(file)
        } catch {
            Darwin.close(file)
            do { try remove(stagingName) }
            catch { throw error }
            throw error
        }
        do {
            try failure(.beforePublish)
            _ = try contains(name)
            guard renameat(descriptor, stagingName, descriptor, name) == 0 else { throw Self.failure("암호문 교체") }
            published = true
            try sync()
        } catch {
            if !published { try remove(stagingName) }
            throw error
        }
    }

    func remove(_ name: String) throws {
        guard try contains(name) else { return }
        guard unlinkat(descriptor, name, 0) == 0 else { throw Self.failure("암호문 삭제") }
    }

    func removePending(profileID: UUID) throws {
        let copy = dup(descriptor)
        guard copy >= 0 else { throw Self.failure("임시 암호문 확인") }
        guard let stream = fdopendir(copy) else { Darwin.close(copy); throw Self.failure("임시 암호문 확인") }
        defer { closedir(stream) }
        let prefix = "." + profileID.uuidString.lowercased() + "."
        while true {
            errno = 0
            guard let entry = readdir(stream) else {
                guard errno == 0 else { throw Self.failure("임시 암호문 목록 읽기") }
                break
            }
            let name = withUnsafePointer(to: &entry.pointee.d_name) { value in
                value.withMemoryRebound(to: CChar.self, capacity: Int(MAXNAMLEN) + 1) { String(cString: $0) }
            }
            guard name.hasPrefix(prefix), name.hasSuffix(".pending") else { continue }
            let generation = String(name.dropFirst(prefix.count).dropLast(".pending".count))
            guard UUID(uuidString: generation) != nil else { throw SpeakerVoiceVaultError.unsafePath }
            try remove(name)
        }
    }

    func sync() throws {
        guard fsync(descriptor) == 0 else { throw Self.failure("폴더 동기화") }
    }

    private static func failure(_ operation: String) -> SpeakerVoiceVaultError {
        .fileSystem(operation: operation, code: errno)
    }
}
