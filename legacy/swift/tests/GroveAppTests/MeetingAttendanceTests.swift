import Foundation
import GroveInference
import Testing
@testable import GroveApp

struct MeetingAttendanceTests {
    @Test func attendanceDoesNotBecomeTheRequiredSpeakerCount() throws {
        let attendance = MeetingAttendance(profileIDs: [UUID(), UUID()], names: ["참석자"], guestCount: 2)
        try attendance.validate()
        #expect(attendance.count == 5)
        let plan = try MeetingSpeakerOptions().plan(isDual: false)
        #expect(plan.configuration.expectedSpeakerCount == nil)
        #expect(throws: TranscriptEditError.self) { try MeetingAttendance(guestCount: -1).validate() }
        let id = UUID()
        #expect(throws: TranscriptEditError.self) { try MeetingAttendance(profileIDs: [id, id]).validate() }
    }

    @Test func oldRecordsDecodeWithoutAttendanceAndNewRecordsPreserveIt() throws {
        let old = MeetingRecord(title: "회의", startedAt: Date(), duration: 3, status: .ready,
            audioPath: nil, glossaryProfile: "", transcript: [], claims: [], errorMessage: nil)
        let data = try JSONEncoder().encode(old)
        #expect(try JSONDecoder().decode(MeetingRecord.self, from: data).attendance == nil)
        var new = old
        new.attendance = .init(names: ["참석자"], guestCount: 4)
        #expect(try JSONDecoder().decode(MeetingRecord.self, from: JSONEncoder().encode(new)).attendance?.count == 5)
    }

    @MainActor @Test func teamMemberRegistrationAndRecentRosterSurviveRestart() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = GroveStore(baseDirectory: root)
        let folder = try #require(store.createFolder(name: "팀 회의"))
        let person = try #require(store.libraryStore.addTeamMember(folderID: folder, name: "참석자"))
        #expect(store.speakerProfiles(in: folder).first?.sourceMeetingID == nil)
        #expect(store.libraryStore.renameTeamMember(profileID: person, name: "변경한 이름"))
        let reopened = GroveStore(baseDirectory: root)
        #expect(reopened.speakerProfiles(in: folder).first?.name == "변경한 이름")
        #expect(!reopened.voice.isAvailable)
        let selected = MeetingAttendance(profileIDs: [person])
        #expect(selected.candidates(in: reopened.speakerProfiles(in: folder)).count == 1)
        #expect(MeetingAttendance().candidates(in: reopened.speakerProfiles(in: folder)).isEmpty)
    }

    @Test func overlappingClustersCannotBothReceiveOneIdentity() throws {
        let people = [MeetingSpeaker(name: "화자 1", order: 0), MeetingSpeaker(name: "화자 2", order: 1)]
        func utterance(_ index: Int, start: Double, end: Double) -> DocumentUtterance {
            .init(id: UUID(), startTime: start, endTime: end, rawText: "발화", sourceChannelID: "recording",
                engineClusterID: "\(index)", speakerID: people[index].id, editedText: nil)
        }
        let same = UUID()
        let mapping = [people[0].id: same, people[1].id: same]
        let overlapping = try TranscriptDocument(speakers: people, utterances: [utterance(0, start: 0, end: 5), utterance(1, start: 4, end: 7)])
        #expect(VoiceIdentitySelection.conflictingAssignments(mapping, in: overlapping) == Set(people.map(\.id)))
        let separated = try TranscriptDocument(speakers: people, utterances: [utterance(0, start: 0, end: 5), utterance(1, start: 6, end: 9)])
        #expect(VoiceIdentitySelection.conflictingAssignments(mapping, in: separated).isEmpty)
    }
}
