import Foundation
import Testing
@testable import GroveApp

struct OriginalRecordingFileTests {
    private func meeting(audio: String? = nil, microphone: String? = nil, system: String? = nil) -> MeetingRecord {
        MeetingRecord(title: "녹음", startedAt: Date(), duration: 2, status: .ready, audioPath: audio,
            systemAudioPath: system, microphoneAudioPath: microphone,
            glossaryProfile: "사전 없음", transcript: [], claims: [])
    }

    private func fixture() throws -> (root: URL, managed: URL, source: URL, destination: URL) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("grove-export-test-\(UUID().uuidString)")
        let managed = root.appendingPathComponent("Grove", isDirectory: true)
        try FileManager.default.createDirectory(at: managed, withIntermediateDirectories: true)
        let source = managed.appendingPathComponent("original.wav")
        try Data([0, 1, 2, 0, 255, 128]).write(to: source)
        return (root, managed, source, root.appendingPathComponent("export.wav"))
    }

    @Test func primaryFileWinsAndLegacyChannelsAreExplicitAndDeduplicated() {
        let primary = meeting(audio: "/tmp/audio.wav", microphone: "/tmp/mic.caf", system: "/tmp/system.caf")
        #expect(primary.originalRecordingFiles.map(\.id) == ["recording"])
        #expect(primary.originalRecordingFiles.first?.url.path == "/tmp/audio.wav")
        let dual = meeting(microphone: "/tmp/mic.caf", system: "/tmp/system.caf").originalRecordingFiles
        #expect(dual.map(\.id) == ["microphone", "system"])
        #expect(dual.map(\.label) == ["마이크 원본", "컴퓨터 소리 원본"])
        #expect(meeting(microphone: "/tmp/original.caf", system: "/tmp/sub/../original.caf").originalRecordingFiles.count == 1)
        #expect(meeting(audio: "relative.wav", microphone: " ", system: "relative.caf").originalRecordingFiles.isEmpty)
        #expect(meeting(audio: "", microphone: "/tmp/mic.caf").originalRecordingFiles.count == 1)
    }

    @Test func filenamesKeepExtensionsWithoutPathOrControlCharacters() {
        let source = OriginalRecordingFile(id: "recording", label: "원본", url: URL(fileURLWithPath: "/tmp/input.WAV"))
        let name = OriginalRecordingExporter.suggestedFilename(title: "../회의/검토\\결과:\n\0", source: source)
        #expect(name.hasSuffix(".WAV"))
        #expect(!name.contains("/") && !name.contains("\\") && !name.contains(":"))
        #expect(!name.unicodeScalars.contains { CharacterSet.controlCharacters.contains($0) })
        #expect(OriginalRecordingExporter.suggestedFilename(title: "회의.wav", source: source) == "회의.WAV")
        #expect(OriginalRecordingExporter.suggestedFilename(title: "  ", source: source) == "녹음.WAV")
        let long = OriginalRecordingExporter.suggestedFilename(title: String(repeating: "회의😀", count: 100), source: source)
        #expect(long.utf8.count <= 255)
        #expect(long.hasSuffix(".WAV"))
    }

    @Test func exportCopiesOriginalBytesAndNeedsExplicitReplacement() async throws {
        let files = try fixture()
        defer { try? FileManager.default.removeItem(at: files.root) }
        let original = try Data(contentsOf: files.source)
        try await OriginalRecordingExporter.export(source: files.source, to: files.destination, protectedDirectory: files.managed)
        #expect(try Data(contentsOf: files.destination) == original)
        try Data("keep existing".utf8).write(to: files.destination)
        await #expect(throws: OriginalRecordingExportError.self) {
            try await OriginalRecordingExporter.export(source: files.source, to: files.destination, protectedDirectory: files.managed)
        }
        #expect(try String(contentsOf: files.destination, encoding: .utf8) == "keep existing")
        try await OriginalRecordingExporter.export(source: files.source, to: files.destination,
                                                   protectedDirectory: files.managed, replacingExisting: true)
        #expect(try Data(contentsOf: files.destination) == original)
        #expect(try Data(contentsOf: files.source) == original)
    }

    @Test func sameFileHardlinkAndSymbolicLinkAreRejectedWithoutMutation() async throws {
        let files = try fixture()
        defer { try? FileManager.default.removeItem(at: files.root) }
        let original = try Data(contentsOf: files.source)
        try FileManager.default.linkItem(at: files.source, to: files.destination)
        let symbolic = files.root.appendingPathComponent("link.wav")
        try FileManager.default.createSymbolicLink(at: symbolic, withDestinationURL: files.source)
        for destination in [files.source, files.destination, symbolic] {
            await #expect(throws: OriginalRecordingExportError.self) {
                try await OriginalRecordingExporter.export(source: files.source, to: destination,
                                                           protectedDirectory: files.managed, replacingExisting: true)
            }
        }
        #expect(try Data(contentsOf: files.source) == original)
        #expect(try Data(contentsOf: files.destination) == original)
    }

    @Test func managedDirectoryAndSymlinkedParentsAreRejectedButSiblingIsAllowed() async throws {
        let files = try fixture()
        defer { try? FileManager.default.removeItem(at: files.root) }
        let redirected = files.root.appendingPathComponent("shortcut", isDirectory: true)
        try FileManager.default.createSymbolicLink(at: redirected, withDestinationURL: files.managed)
        for destination in [files.managed.appendingPathComponent("copy.wav"), redirected.appendingPathComponent("copy.wav")] {
            await #expect(throws: OriginalRecordingExportError.self) {
                try await OriginalRecordingExporter.export(source: files.source, to: destination, protectedDirectory: files.managed)
            }
            #expect(!FileManager.default.fileExists(atPath: destination.path))
        }
        let sibling = files.root.appendingPathComponent("Grove-other", isDirectory: true)
        try FileManager.default.createDirectory(at: sibling, withIntermediateDirectories: true)
        try await OriginalRecordingExporter.export(source: files.source, to: sibling.appendingPathComponent("copy.wav"),
                                                   protectedDirectory: files.managed)
    }

    @Test func finderAliasDestinationIsRejectedAndSourceAliasCopiesTargetBytes() async throws {
        let files = try fixture()
        defer { try? FileManager.default.removeItem(at: files.root) }
        let alias = files.root.appendingPathComponent("original alias")
        let bookmark = try files.source.bookmarkData(options: .suitableForBookmarkFile, includingResourceValuesForKeys: nil, relativeTo: nil)
        try URL.writeBookmarkData(bookmark, to: alias)
        await #expect(throws: OriginalRecordingExportError.self) {
            try await OriginalRecordingExporter.export(source: files.source, to: alias,
                                                       protectedDirectory: files.managed, replacingExisting: true)
        }
        try await OriginalRecordingExporter.export(source: alias, to: files.destination, protectedDirectory: files.managed)
        #expect(try Data(contentsOf: files.destination) == Data(contentsOf: files.source))
    }

    @Test func sourceChangeDuringCopyDoesNotReplaceDestinationAndCleansTemporaryFile() throws {
        let files = try fixture()
        defer { try? FileManager.default.removeItem(at: files.root) }
        try Data(repeating: 9, count: 2 * 1_048_576).write(to: files.source)
        try Data("old destination".utf8).write(to: files.destination)
        #expect(throws: OriginalRecordingExportError.self) {
            try OriginalRecordingExporter.copySynchronously(source: files.source, to: files.destination,
                protectedDirectory: files.managed, replacingExisting: true) { copied in
                    if copied == 1_048_576 {
                        let handle = try FileHandle(forWritingTo: files.source)
                        defer { try? handle.close() }
                        try handle.seekToEnd()
                        try handle.write(contentsOf: Data([1]))
                    }
                }
        }
        #expect(try String(contentsOf: files.destination, encoding: .utf8) == "old destination")
        #expect(try FileManager.default.contentsOfDirectory(atPath: files.root.path).allSatisfy { !$0.hasPrefix(".grove-export-") })
    }

    @Test func cancelledExportPreservesDestinationAndCleansPartialCopy() async throws {
        let files = try fixture()
        defer { try? FileManager.default.removeItem(at: files.root) }
        try Data("old destination".utf8).write(to: files.destination)
        let operation = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            try await OriginalRecordingExporter.export(source: files.source, to: files.destination,
                                                       protectedDirectory: files.managed, replacingExisting: true)
        }
        await #expect(throws: CancellationError.self) { try await operation.value }
        #expect(throws: CancellationError.self) {
            try OriginalRecordingExporter.copySynchronously(source: files.source, to: files.destination,
                protectedDirectory: files.managed, replacingExisting: true) { _ in throw CancellationError() }
        }
        #expect(try String(contentsOf: files.destination, encoding: .utf8) == "old destination")
        #expect(try FileManager.default.contentsOfDirectory(atPath: files.root.path).allSatisfy { !$0.hasPrefix(".grove-export-") })
    }

    @Test func changedDestinationOrNonRegularSourceCannotBePublished() throws {
        let files = try fixture()
        defer { try? FileManager.default.removeItem(at: files.root) }
        #expect(throws: OriginalRecordingExportError.self) {
            try OriginalRecordingExporter.copySynchronously(source: files.source, to: files.destination,
                protectedDirectory: files.managed) { _ in try Data("other writer".utf8).write(to: files.destination) }
        }
        #expect(try String(contentsOf: files.destination, encoding: .utf8) == "other writer")
        #expect(throws: OriginalRecordingExportError.self) {
            try OriginalRecordingExporter.copySynchronously(source: files.managed,
                to: files.root.appendingPathComponent("directory-copy"), protectedDirectory: files.managed)
        }
    }
}
