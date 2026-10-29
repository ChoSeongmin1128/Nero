import Combine
import Foundation
import GroveInference
import Testing
import UniformTypeIdentifiers
@testable import GroveApp

@MainActor
struct MeetingFolderTransferTests {
    private struct Fixture {
        let root: URL
        let store: GroveStore
        let first: MeetingRecord
        let second: MeetingRecord
        let folderA: UUID
        let folderB: UUID
        var indexURL: URL { root.appendingPathComponent("meetings.json") }
    }

    private func fixture() throws -> Fixture {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("grove-folder-test-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let audio = root.appendingPathComponent("original.wav")
        try Data([0, 2, 4, 8]).write(to: audio)
        let first = MeetingRecord(title: "디자인 회의", startedAt: Date(), duration: 2, status: .ready,
            audioPath: audio.path, glossaryProfile: "사전 없음", transcript: [], claims: [])
        let second = MeetingRecord(title: "배포 검토", startedAt: Date().addingTimeInterval(-60), duration: 2, status: .ready,
            audioPath: audio.path, glossaryProfile: "사전 없음", transcript: [], claims: [])
        try MeetingRecordStorage(url: root.appendingPathComponent("meetings.json")).save([first, second])
        let store = GroveStore(baseDirectory: root)
        let folderA = try #require(store.createFolder(name: "디자인"))
        let folderB = try #require(store.createFolder(name: "제품"))
        #expect(store.moveMeeting(id: first.id, to: folderA))
        return Fixture(root: root, store: store, first: first, second: second, folderA: folderA, folderB: folderB)
    }

    @Test func privatePayloadRoundTripsAndRejectsForeignMalformedOrDuplicateReferences() throws {
        let scope = UUID()
        let payload = MeetingFolderTransfer(meetingID: UUID(), scopeID: scope)
        let encoded = try JSONEncoder().encode(payload)
        let restored = try JSONDecoder().decode(MeetingFolderTransfer.self, from: encoded)
        #expect(restored == payload)
        #expect(try MeetingFolderTransfer.recordingIDs(in: [restored], scopeID: scope) == [payload.meetingID])
        #expect(!UTType.plainText.conforms(to: .groveRecordingReference))
        #expect(!UTType.fileURL.conforms(to: .groveRecordingReference))
        #expect(throws: MeetingFolderMoveError.self) { try MeetingFolderTransfer.recordingIDs(in: [payload], scopeID: UUID()) }
        #expect(throws: MeetingFolderMoveError.self) { try MeetingFolderTransfer.recordingIDs(in: [], scopeID: scope) }
        #expect(throws: MeetingFolderMoveError.self) { try MeetingFolderTransfer.recordingIDs(in: [payload, payload], scopeID: scope) }
        var object = try #require(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        object["schemaVersion"] = 99
        let unsupported = try JSONDecoder().decode(MeetingFolderTransfer.self, from: JSONSerialization.data(withJSONObject: object))
        #expect(throws: MeetingFolderMoveError.self) { try MeetingFolderTransfer.recordingIDs(in: [unsupported], scopeID: scope) }
        #expect(throws: (any Error).self) { try JSONDecoder().decode(MeetingFolderTransfer.self, from: Data("file:///external.wav".utf8)) }
    }

    @Test func dropPersistsBeforePublishingAndPreservesSelectionOrderAudioAndTranscriptIdentity() throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let store = fixture.store
        let result = try InferenceResult(duration: 2, configuration: .init(),
            transcription: .init(utterances: [.init(start: 0, end: 1, text: "확인한 발화")]),
            rawDiarization: [.init(start: 0, end: 1, clusterID: "A")])
        try store.acceptInferenceResult(result, meetingID: fixture.first.id, sourceChannelID: "recording")
        let document = try #require(store.transcriptDocuments[fixture.first.id])
        #expect(store.transcripts.renameSpeaker(meetingID: fixture.first.id, speakerID: document.speakers[0].id, name: "발화자"))
        let savedDocument = try #require(store.transcriptDocuments[fixture.first.id])
        let documentURL = TranscriptDocumentStorage(directory: fixture.root.appendingPathComponent("Documents")).fileURL(for: fixture.first.id)
        let originalDocumentData = try Data(contentsOf: documentURL)
        let audioURL = URL(fileURLWithPath: try #require(fixture.first.audioPath))
        let originalAudioData = try Data(contentsOf: audioURL)
        let before = store.meetings
        store.selection = .meeting(fixture.first.id)
        let defaults = store.defaultSpeakerOptions
        var publications = 0
        var diskMatchedPublication = true
        let observation = store.$meetings.dropFirst().sink { value in
            publications += 1
            diskMatchedPublication = diskMatchedPublication && (try? MeetingRecordStorage(url: fixture.indexURL).load()) == value
        }
        defer { observation.cancel() }
        #expect(store.acceptFolderDrop([store.folderTransfer(for: fixture.first.id)], to: fixture.folderB))
        #expect(publications == 1 && diskMatchedPublication)
        var expected = before
        expected[0].folderID = fixture.folderB
        #expect(store.meetings == expected)
        #expect(store.selection == .meeting(fixture.first.id))
        #expect(store.defaultSpeakerOptions == defaults)
        #expect(store.transcriptDocuments[fixture.first.id] == savedDocument)
        #expect(try Data(contentsOf: documentURL) == originalDocumentData)
        #expect(try Data(contentsOf: audioURL) == originalAudioData)
        #expect(store.meetings.prefix(12).map(\.id) == before.prefix(12).map(\.id))
        #expect(store.folderMoveFeedback?.message.contains("제품") == true)
        let reopened = GroveStore(baseDirectory: fixture.root)
        #expect(reopened.meetings.first?.folderID == fixture.folderB)
        #expect(reopened.transcriptDocuments[fixture.first.id] == savedDocument)
    }

    @Test func failedIndexSavePublishesNoTransientMoveAndKeepsOriginalData() throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let store = fixture.store
        let before = store.meetings
        let brokenIndex = Data("invalid index must be preserved".utf8)
        try brokenIndex.write(to: fixture.indexURL)
        var publications = 0
        let observation = store.$meetings.dropFirst().sink { _ in publications += 1 }
        defer { observation.cancel() }
        #expect(!store.moveMeeting(id: fixture.first.id, to: fixture.folderB))
        #expect(publications == 0)
        #expect(store.meetings == before)
        #expect(try Data(contentsOf: fixture.indexURL) == brokenIndex)
        #expect(store.alertMessage?.contains("기존 폴더 배치") == true)
        #expect(store.folderMoveFeedback == nil)
    }

    @Test func sameFolderIsSuccessfulNoOpWithoutDiskOrPublicationChanges() throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let data = try Data(contentsOf: fixture.indexURL)
        let backupURL = fixture.indexURL.appendingPathExtension("backup")
        let backup = try Data(contentsOf: backupURL)
        var publications = 0
        let observation = fixture.store.$meetings.dropFirst().sink { _ in publications += 1 }
        defer { observation.cancel() }
        #expect(fixture.store.moveMeeting(id: fixture.first.id, to: fixture.folderA))
        #expect(publications == 0)
        #expect(try Data(contentsOf: fixture.indexURL) == data)
        #expect(try Data(contentsOf: backupURL) == backup)
    }

    @Test func invalidIDsOrBusyItemsNeverPartiallyMoveTheBatch() throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let store = fixture.store
        let before = store.meetings
        #expect(!store.moveMeeting(id: UUID(), to: fixture.folderB))
        #expect(!store.moveMeeting(id: fixture.first.id, to: UUID()))
        #expect(!store.moveMeetings(ids: [fixture.first.id, UUID()], to: fixture.folderB))
        #expect(!store.acceptFolderDrop([.init(meetingID: fixture.first.id, scopeID: UUID())], to: fixture.folderB))
        #expect(store.meetings == before)
        store.activeMeetingID = fixture.first.id
        #expect(!store.canMoveMeeting(id: fixture.first.id))
        #expect(!store.moveMeetings(ids: [fixture.first.id, fixture.second.id], to: fixture.folderB))
        #expect(store.meetings == before)
        store.activeMeetingID = nil
        store.meetings[0].status = .processing
        #expect(!store.moveMeeting(id: fixture.first.id, to: fixture.folderB))
        store.meetings[0].status = .recording
        #expect(!store.moveMeeting(id: fixture.first.id, to: fixture.folderB))
        #expect(store.meetings[0].folderID == fixture.folderA)
        #expect(store.meetings[1].folderID == nil)
    }

    @Test func movingToUnfiledDoesNotRemoveTheRecordingFromRecentOrAllViews() throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let store = fixture.store
        let ids = store.meetings.map(\.id)
        #expect(store.recordings(in: fixture.folderA).count == 1)
        #expect(store.recordings(in: nil).count == 1)
        #expect(store.acceptFolderDrop([store.folderTransfer(for: fixture.first.id)], to: nil))
        #expect(store.recordings(in: fixture.folderA).isEmpty)
        #expect(store.recordings(in: nil).count == 2)
        #expect(store.meetings.map(\.id) == ids)
        #expect(store.folderName(nil) == "미분류")
    }

    @Test func searchAndOrphanFolderClassificationAreReadOnlyAndStable() throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        var orphan = fixture.second
        orphan.folderID = UUID()
        orphan.title = "Café 회의"
        let rows = [fixture.first, orphan]
        #expect(MeetingFolderListing.search(rows, title: "  CAFE  ").map(\.id) == [orphan.id])
        #expect(MeetingFolderListing.search(rows, title: "없는 제목").isEmpty)
        #expect(MeetingFolderListing.search(rows, title: " ") == rows)
        #expect(MeetingFolderListing.recordings([orphan], in: nil, folders: fixture.store.library.folders) == [orphan])
        #expect(orphan.folderID != nil)
    }

    @Test func oldFeedbackCannotDismissANewerMoveAndNamesRemainUnambiguous() throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let store = fixture.store
        let old = try #require(store.folderMoveFeedback?.id)
        #expect(store.moveMeeting(id: fixture.first.id, to: fixture.folderB))
        let new = try #require(store.folderMoveFeedback?.id)
        store.dismissFolderMoveFeedback(id: old)
        #expect(store.folderMoveFeedback?.id == new)
        store.dismissFolderMoveFeedback(id: new)
        #expect(store.folderMoveFeedback == nil)
        #expect(store.createFolder(name: "미분류") == nil)
        #expect(store.createFolder(name: "디자인") == nil)
        #expect(store.createFolder(name: "여러\n줄") == nil)
        #expect(!store.libraryStore.renameFolder(id: fixture.folderA, name: "제품"))
        #expect(store.libraryStore.renameFolder(id: fixture.folderA, name: "디자인"))
    }

    @Test func orphanFolderDoesNotBecomeAnInvalidDefaultForANewRecording() throws {
        let fixture = try fixture()
        defer { try? FileManager.default.removeItem(at: fixture.root) }
        let missingFolder = UUID()
        fixture.store.meetings[1].folderID = missingFolder
        fixture.store.selection = .meeting(fixture.second.id)
        #expect(fixture.store.selectedFolderID == nil)
        #expect(fixture.store.meetings[1].folderID == missingFolder)
        fixture.store.selection = .folder(fixture.folderA)
        #expect(fixture.store.selectedFolderID == fixture.folderA)
    }
}
