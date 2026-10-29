import Foundation
import Testing
@testable import GroveApp

@MainActor
struct MeetingLibraryTests {
    @Test func foldersAndDefaultsPersistWithoutChangingExistingMeetingOptions() throws {
        let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: base) }
        let store = GroveStore(baseDirectory: base)
        let folder = try #require(store.createFolder(name: "  디자인  "))
        var options = MeetingSpeakerOptions()
        options.mode = .manualCount
        options.countText = "4"
        let record = MeetingRecord(title: "녹음", startedAt: Date(), duration: 10, status: .ready,
            audioPath: "/tmp/original.m4a", inferenceConfiguration: try options.plan(isDual: false).configuration,
            glossaryProfile: "사전 없음", transcript: [], claims: [])
        store.meetings = [record]
        #expect(store.moveMeeting(id: record.id, to: folder))
        options.countText = "8"
        store.defaultSpeakerOptions = options
        let reopened = GroveStore(baseDirectory: base)
        #expect(reopened.folderName(folder) == "디자인")
        #expect(reopened.defaultSpeakerOptions.countText == "8")
        #expect(reopened.meetings.first?.inferenceConfiguration?.expectedSpeakerCount == 4)
        #expect(reopened.meetings.first?.folderID == folder)
        #expect(reopened.deleteFolder(id: folder))
        let unfiled = GroveStore(baseDirectory: base)
        #expect(unfiled.library.folders.isEmpty)
        #expect(unfiled.meetings.first?.folderID == nil)
        #expect(unfiled.meetings.first?.audioPath == "/tmp/original.m4a")
    }

    @Test func localDraftDoesNotChangeDefaultsAndMalformedLibraryIsNotOverwritten() throws {
        let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: base) }
        let store = GroveStore(baseDirectory: base)
        var draft = store.defaultSpeakerOptions
        draft.mode = .manualCount
        draft.countText = "2"
        #expect(store.defaultSpeakerOptions.mode == .automatic)
        let invalid = Data("not-json".utf8)
        let url = base.appendingPathComponent("library.json")
        try invalid.write(to: url)
        let reopened = GroveStore(baseDirectory: base)
        #expect(reopened.createFolder(name: "새 폴더") == nil)
        #expect(try Data(contentsOf: url) == invalid)
    }

    @Test func legacyRecordsHaveNoFolderAndUnsafeMetadataFailsValidation() throws {
        let folder = MeetingFolder(name: "하나")
        var library = MeetingLibrary()
        library.folders = [folder, folder]
        #expect(throws: TranscriptEditError.self) { try library.validate() }
    }
}
