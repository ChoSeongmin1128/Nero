import Foundation

struct MeetingAttendance: Codable, Hashable, Sendable {
    var profileIDs: [UUID] = []
    var names: [String] = []
    var guestCount = 0

    var count: Int { profileIDs.count + names.count + guestCount }

    func validate() throws {
        guard Set(profileIDs).count == profileIDs.count, (0...99).contains(guestCount), count <= 99,
              names.allSatisfy({ !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                  && !$0.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) }) else {
            throw TranscriptEditError.invalidDocument
        }
    }

    func candidates(in profiles: [SavedSpeakerProfile]) -> [SavedSpeakerProfile] {
        let selected = Set(profileIDs)
        return profiles.filter { selected.contains($0.id) }
    }
}
