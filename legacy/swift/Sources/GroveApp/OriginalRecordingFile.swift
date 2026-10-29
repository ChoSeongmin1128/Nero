import Darwin
import Foundation

struct OriginalRecordingFile: Identifiable, Hashable, Sendable {
    let id: String
    let label: String
    let url: URL
}

extension MeetingRecord {
    var originalRecordingFiles: [OriginalRecordingFile] {
        func file(_ path: String?, id: String, label: String) -> OriginalRecordingFile? {
            guard let path, path.hasPrefix("/"), !path.contains("\0"),
                  !path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
            return OriginalRecordingFile(id: id, label: label,
                url: URL(fileURLWithPath: path).standardizedFileURL)
        }
        if let original = file(audioPath, id: "recording", label: "원본 녹음") { return [original] }
        let channels = [file(microphoneAudioPath, id: "microphone", label: "마이크 원본"),
                        file(systemAudioPath, id: "system", label: "컴퓨터 소리 원본")]
        var paths: Set<String> = []
        return channels.compactMap { $0 }.filter { paths.insert($0.url.path).inserted }
    }
}

enum OriginalRecordingExportError: Error, LocalizedError {
    case invalidSource, invalidDestination, sameFile, protectedDestination
    case destinationExists, destinationChanged, sourceChanged, linkDestination

    var errorDescription: String? {
        switch self {
        case .invalidSource: "읽을 수 있는 원본 파일이 필요합니다. 파일 위치를 확인해 주세요."
        case .invalidDestination: "저장할 폴더와 파일 이름을 확인해 주세요."
        case .sameFile: "원본과 같은 파일에는 저장할 수 없습니다. 다른 이름이나 위치를 선택해 주세요."
        case .protectedDestination: "Grove가 관리하는 폴더 안에는 내보낼 수 없습니다. 다른 위치를 선택해 주세요."
        case .destinationExists: "같은 이름의 파일이 있습니다. 덮어쓰기를 확인하거나 다른 이름을 선택해 주세요."
        case .destinationChanged: "복사 중 저장 위치나 대상 파일이 바뀌었습니다. 기존 파일은 덮어쓰지 않았습니다."
        case .sourceChanged: "복사 중 원본 파일이 바뀌었습니다. 녹음이나 파일 변경을 마친 뒤 다시 저장해 주세요."
        case .linkDestination: "바로가기나 심볼릭 링크에는 저장할 수 없습니다. 다른 이름이나 위치를 선택해 주세요."
        }
    }
}

enum OriginalRecordingExporter {
    static func suggestedFilename(title: String, source: OriginalRecordingFile) -> String {
        func cleaned(_ value: String) -> String {
            String(value.unicodeScalars.map { scalar -> Character in
                if CharacterSet.controlCharacters.contains(scalar) || "/\\:".unicodeScalars.contains(scalar) { return "-" }
                return Character(String(scalar))
            }).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        let extensionName = cleaned(source.url.pathExtension)
        let suffix = extensionName.isEmpty ? "" : ".\(extensionName)"
        var name = cleaned(title)
        if !suffix.isEmpty, name.lowercased().hasSuffix(suffix.lowercased()) { name.removeLast(suffix.count) }
        name = name.trimmingCharacters(in: CharacterSet(charactersIn: " .-_"))
        if name.isEmpty { name = "녹음" }
        var bounded = ""
        let budget = max(0, 255 - suffix.utf8.count)
        for character in name {
            guard bounded.utf8.count + String(character).utf8.count <= budget else { break }
            bounded.append(character)
        }
        // A real file's extension fits its filesystem component. Normal audio
        // extensions leave ample room for the fallback even with multibyte titles.
        if bounded.isEmpty, budget >= 1 { bounded = "_" }
        return bounded + suffix
    }

    static func export(source: URL, to destination: URL, protectedDirectory: URL,
                       replacingExisting: Bool = false) async throws {
        try Task.checkCancellation()
        let operation = Task.detached(priority: .utility) {
            try copySynchronously(source: source, to: destination, protectedDirectory: protectedDirectory,
                                  replacingExisting: replacingExisting)
        }
        try await withTaskCancellationHandler {
            try await operation.value
        } onCancel: {
            operation.cancel()
        }
    }

    // Internal seam lets tests deterministically mutate/cancel between copy chunks.
    // Production calls have no callback. All production file I/O runs detached.
    static func copySynchronously(source: URL, to destination: URL, protectedDirectory: URL,
                                  replacingExisting: Bool = false,
                                  afterChunk: (@Sendable (Int64) throws -> Void)? = nil) throws {
        try Task.checkCancellation()
        guard source.isFileURL, destination.isFileURL, protectedDirectory.isFileURL else {
            throw OriginalRecordingExportError.invalidDestination
        }
        let sourceURL = try resolvedExistingURL(source)
        let input = sourceURL.path.withCString { Darwin.open($0, O_RDONLY | O_NONBLOCK | O_NOFOLLOW | O_CLOEXEC) }
        guard input >= 0 else { throw systemError("원본 파일을 열지 못했습니다") }
        defer { Darwin.close(input) }
        let original = try stamp(input)
        guard original.isRegular, original.size >= 0 else { throw OriginalRecordingExportError.invalidSource }

        let destinationURL = destination.standardizedFileURL
        let name = destinationURL.lastPathComponent
        guard !name.isEmpty, name != ".", name != "..", !name.contains("\0"), !name.contains("/") else {
            throw OriginalRecordingExportError.invalidDestination
        }
        let parentURL = try resolvedExistingURL(destinationURL.deletingLastPathComponent())
        let parent = try openDirectory(parentURL)
        defer { Darwin.close(parent) }
        let parentIdentity = try stamp(parent).identity
        try rejectProtectedAncestor(parent, protectedDirectory: protectedDirectory)
        let previousDestination = try destinationStamp(parent, name: name)
        try validateDestination(previousDestination, original: original, url: destinationURL,
                                replacingExisting: replacingExisting)

        let temporaryName = ".grove-export-\(UUID().uuidString).tmp"
        let output = temporaryName.withCString {
            Darwin.openat(parent, $0, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, mode_t(0o600))
        }
        guard output >= 0 else { throw systemError("저장할 임시 파일을 만들지 못했습니다") }
        defer {
            Darwin.close(output)
            _ = temporaryName.withCString { Darwin.unlinkat(parent, $0, 0) }
        }
        let capacity = 1_048_576
        let buffer = UnsafeMutableRawPointer.allocate(byteCount: capacity, alignment: MemoryLayout<UInt8>.alignment)
        defer { buffer.deallocate() }
        var remaining = original.size
        var copied: Int64 = 0
        while remaining > 0 {
            try Task.checkCancellation()
            let amount = Darwin.read(input, buffer, min(capacity, Int(remaining)))
            if amount < 0 {
                if errno == EINTR { continue }
                throw systemError("원본 파일을 읽지 못했습니다")
            }
            guard amount > 0 else { throw OriginalRecordingExportError.sourceChanged }
            var written = 0
            while written < amount {
                try Task.checkCancellation()
                let count = Darwin.write(output, buffer.advanced(by: written), amount - written)
                if count < 0 {
                    if errno == EINTR { continue }
                    throw systemError("원본 사본을 저장하지 못했습니다")
                }
                guard count > 0 else { throw OriginalRecordingExportError.invalidDestination }
                written += count
            }
            remaining -= Int64(amount)
            copied += Int64(amount)
            try afterChunk?(copied)
        }
        guard Darwin.fsync(output) == 0 else { throw systemError("원본 사본을 디스크에 저장하지 못했습니다") }
        try Task.checkCancellation()
        guard try stamp(input) == original else { throw OriginalRecordingExportError.sourceChanged }
        let currentSourceURL = try resolvedExistingURL(source)
        let currentInput = currentSourceURL.path.withCString { Darwin.open($0, O_RDONLY | O_NONBLOCK | O_NOFOLLOW | O_CLOEXEC) }
        guard currentInput >= 0 else { throw OriginalRecordingExportError.sourceChanged }
        defer { Darwin.close(currentInput) }
        guard try stamp(currentInput) == original else { throw OriginalRecordingExportError.sourceChanged }

        let currentParent = try openDirectory(resolvedExistingURL(destinationURL.deletingLastPathComponent()))
        defer { Darwin.close(currentParent) }
        guard try stamp(currentParent).identity == parentIdentity else { throw OriginalRecordingExportError.destinationChanged }
        try rejectProtectedAncestor(parent, protectedDirectory: protectedDirectory)
        let currentDestination = try destinationStamp(parent, name: name)
        guard currentDestination == previousDestination else { throw OriginalRecordingExportError.destinationChanged }
        try validateDestination(currentDestination, original: original, url: destinationURL,
                                replacingExisting: replacingExisting)
        try Task.checkCancellation()
        let published = temporaryName.withCString { temporary in
            name.withCString { final in
                if previousDestination == nil {
                    return Darwin.renameatx_np(parent, temporary, parent, final, UInt32(RENAME_EXCL))
                }
                return Darwin.renameat(parent, temporary, parent, final)
            }
        }
        guard published == 0 else { throw systemError("완성된 원본 사본을 저장하지 못했습니다") }
        // Atomic publication is the commit point. A later cancellation does not
        // turn a completed export into a reported failure.
    }

    private struct FileIdentity: Equatable {
        let device: dev_t
        let inode: ino_t
    }

    private struct FileStamp: Equatable {
        let identity: FileIdentity
        let size: Int64
        let mode: mode_t
        let modificationSeconds: Int64
        let modificationNanoseconds: Int64
        let changeSeconds: Int64
        let changeNanoseconds: Int64
        var isRegular: Bool { mode & mode_t(S_IFMT) == mode_t(S_IFREG) }
        var isSymbolicLink: Bool { mode & mode_t(S_IFMT) == mode_t(S_IFLNK) }

        init(_ status: stat) {
            identity = FileIdentity(device: status.st_dev, inode: status.st_ino)
            size = status.st_size
            mode = status.st_mode
            modificationSeconds = Int64(status.st_mtimespec.tv_sec)
            modificationNanoseconds = Int64(status.st_mtimespec.tv_nsec)
            changeSeconds = Int64(status.st_ctimespec.tv_sec)
            changeNanoseconds = Int64(status.st_ctimespec.tv_nsec)
        }
    }

    private static func stamp(_ descriptor: Int32) throws -> FileStamp {
        var status = stat()
        guard Darwin.fstat(descriptor, &status) == 0 else { throw systemError("파일 상태를 확인하지 못했습니다") }
        return FileStamp(status)
    }

    private static func destinationStamp(_ directory: Int32, name: String) throws -> FileStamp? {
        var status = stat()
        let result = name.withCString { Darwin.fstatat(directory, $0, &status, AT_SYMLINK_NOFOLLOW) }
        if result != 0 {
            if errno == ENOENT { return nil }
            throw systemError("저장 대상의 상태를 확인하지 못했습니다")
        }
        return FileStamp(status)
    }

    private static func validateDestination(_ destination: FileStamp?, original: FileStamp, url: URL,
                                            replacingExisting: Bool) throws {
        guard let destination else { return }
        if destination.identity == original.identity { throw OriginalRecordingExportError.sameFile }
        if destination.isSymbolicLink { throw OriginalRecordingExportError.linkDestination }
        if try url.resourceValues(forKeys: [.isAliasFileKey]).isAliasFile == true {
            throw OriginalRecordingExportError.linkDestination
        }
        guard destination.isRegular else { throw OriginalRecordingExportError.invalidDestination }
        guard replacingExisting else { throw OriginalRecordingExportError.destinationExists }
    }

    private static func resolvedExistingURL(_ url: URL) throws -> URL {
        var resolved = url.standardizedFileURL.resolvingSymlinksInPath()
        for _ in 0..<8 {
            let values = try resolved.resourceValues(forKeys: [.isAliasFileKey])
            guard values.isAliasFile == true else { return resolved }
            resolved = try URL(resolvingAliasFileAt: resolved, options: [.withoutUI, .withoutMounting])
                .standardizedFileURL.resolvingSymlinksInPath()
        }
        throw OriginalRecordingExportError.invalidSource
    }

    private static func openDirectory(_ url: URL) throws -> Int32 {
        let descriptor = url.path.withCString { Darwin.open($0, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC) }
        guard descriptor >= 0 else { throw systemError("저장 폴더를 열지 못했습니다") }
        return descriptor
    }

    private static func rejectProtectedAncestor(_ directory: Int32, protectedDirectory: URL) throws {
        let protected = try openDirectory(resolvedExistingURL(protectedDirectory))
        defer { Darwin.close(protected) }
        let protectedIdentity = try stamp(protected).identity
        var current = Darwin.dup(directory)
        guard current >= 0 else { throw systemError("저장 폴더를 확인하지 못했습니다") }
        defer { Darwin.close(current) }
        for _ in 0..<512 {
            let identity = try stamp(current).identity
            guard identity != protectedIdentity else { throw OriginalRecordingExportError.protectedDestination }
            let parent = Darwin.openat(current, "..", O_RDONLY | O_DIRECTORY | O_CLOEXEC)
            guard parent >= 0 else { throw systemError("저장 폴더의 상위 위치를 확인하지 못했습니다") }
            let parentIdentity: FileIdentity
            do { parentIdentity = try stamp(parent).identity }
            catch { Darwin.close(parent); throw error }
            if parentIdentity == identity { Darwin.close(parent); return }
            Darwin.close(current)
            current = parent
        }
        throw OriginalRecordingExportError.invalidDestination
    }

    private static func systemError(_ operation: String) -> NSError {
        let code = errno
        return NSError(domain: NSPOSIXErrorDomain, code: Int(code),
            userInfo: [NSLocalizedDescriptionKey: "\(operation). \(String(cString: strerror(code)))"])
    }
}
