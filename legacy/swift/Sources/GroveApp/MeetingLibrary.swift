import Foundation

struct MeetingFolder: Identifiable, Codable, Hashable, Sendable {
    var id = UUID()
    var name: String
    var automaticSpeakerIdentification: Bool? = nil
}

struct MeetingLibrary: Codable, Equatable {
    var schemaVersion = 1
    var folders: [MeetingFolder] = []
    var defaultSpeakerOptions = MeetingSpeakerOptions()
    var speakerProfiles: [SavedSpeakerProfile]? = nil
    var recentAttendance: [String: MeetingAttendance]? = nil

    func validate() throws {
        try recentAttendance?.values.forEach { try $0.validate() }
        guard schemaVersion == 1, Set(folders.map(\.id)).count == folders.count,
              folders.allSatisfy({ !$0.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty })
        else { throw TranscriptEditError.invalidDocument }
        let profiles = speakerProfiles ?? []
        guard Set(profiles.map(\.id)).count == profiles.count,
              profiles.allSatisfy({ profile in
                  guard folders.contains(where: { $0.id == profile.folderID }),
                        !profile.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
                  guard let voice = profile.voice else { return true }
                  return !voice.modelIdentifier.isEmpty && voice.embedding.count == 256
                      && voice.embedding.allSatisfy(\.isFinite) && voice.embedding.contains { $0 != 0 }
                      && voice.speechDuration.isFinite && voice.speechDuration >= 6 && voice.sampleCount >= 2
              }) else { throw TranscriptEditError.invalidDocument }
    }
}

struct MeetingLibraryStorage {
    let url: URL

    func load() throws -> MeetingLibrary {
        guard FileManager.default.fileExists(atPath: url.path) else { return MeetingLibrary() }
        let library = try JSONDecoder().decode(MeetingLibrary.self, from: Data(contentsOf: url))
        try library.validate()
        return library
    }

    func save(_ library: MeetingLibrary) throws {
        try library.validate()
        var namesOnly = library
        namesOnly.speakerProfiles = library.speakerProfiles?.map { profile in
            var clean = profile
            clean.voice = nil
            return clean
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let data = try encoder.encode(namesOnly)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        if FileManager.default.fileExists(atPath: url.path) {
            var previous = try load()
            // A removed voiceprint must not linger in the automatic backup.
            let retainedProfiles = Set((library.speakerProfiles ?? []).map(\.id))
            previous.speakerProfiles?.removeAll { !retainedProfiles.contains($0.id) }
            // Legacy experimental voice fields are never written into new names or backups.
            previous.speakerProfiles = previous.speakerProfiles?.map { profile in
                var clean = profile
                clean.voice = nil
                return clean
            }
            try encoder.encode(previous).write(to: url.appendingPathExtension("backup"), options: .atomic)
        }
        try data.write(to: url, options: .atomic)
    }
}
