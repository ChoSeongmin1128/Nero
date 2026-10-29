import Foundation

@MainActor
final class LibraryStore: ObservableObject {
    @Published private(set) var library = MeetingLibrary()
    private(set) var loadError: String?
    var onError: ((String) -> Void)?
    var canEditPeople: () -> Bool = { true }
    private let storage: MeetingLibraryStorage
    private let persists: Bool

    init(url: URL, persists: Bool) {
        storage = MeetingLibraryStorage(url: url)
        self.persists = persists
        guard persists else { return }
        do { library = try storage.load() }
        catch { loadError = error.localizedDescription }
    }

    var folders: [MeetingFolder] { library.folders }

    var defaultSpeakerOptions: MeetingSpeakerOptions {
        get { library.defaultSpeakerOptions }
        set { edit { $0.defaultSpeakerOptions = newValue } }
    }

    func containsFolder(_ id: UUID?) -> Bool {
        guard let id else { return false }
        return library.folders.contains { $0.id == id }
    }

    func folderName(_ id: UUID?) -> String {
        library.folders.first { $0.id == id }?.name ?? "미분류"
    }

    @discardableResult
    func edit(_ mutation: (inout MeetingLibrary) -> Void) -> Bool {
        guard loadError == nil else {
            onError?("폴더·설정 파일을 먼저 복구해야 합니다. 기존 파일은 보존됩니다.")
            return false
        }
        var updated = library
        mutation(&updated)
        updated.speakerProfiles = updated.speakerProfiles?.map { profile in
            var namesOnly = profile; namesOnly.voice = nil; return namesOnly
        }
        do {
            if persists { try storage.save(updated) }
            library = updated
            return true
        } catch {
            onError?("폴더·설정을 저장하지 못했습니다. \(error.localizedDescription)")
            return false
        }
    }

    func createFolder(name: String) -> MeetingFolder? {
        do {
            let folder = MeetingFolder(name: try MeetingFolderListing.folderName(name, folders: library.folders))
            return edit({ $0.folders.append(folder) }) ? folder : nil
        } catch { onError?(error.localizedDescription); return nil }
    }

    func renameFolder(id: UUID, name: String) -> Bool {
        guard let index = library.folders.firstIndex(where: { $0.id == id }) else {
            onError?(MeetingFolderMoveError.missingFolder.localizedDescription); return false
        }
        do {
            let name = try MeetingFolderListing.folderName(name, excluding: id, folders: library.folders)
            guard library.folders[index].name != name else { return true }
            return edit { $0.folders[index].name = name }
        } catch { onError?(error.localizedDescription); return false }
    }

    func speakerProfiles(in folderID: UUID) -> [SavedSpeakerProfile] {
        (library.speakerProfiles ?? []).filter { $0.folderID == folderID }
    }

    func profile(_ id: UUID) -> SavedSpeakerProfile? {
        library.speakerProfiles?.first { $0.id == id }
    }

    func addTeamMember(folderID: UUID, name: String) -> UUID? {
        guard canEditPeople(), containsFolder(folderID) else { return nil }
        guard let name = Self.singleLineName(name) else { return nil }
        let profile = SavedSpeakerProfile(folderID: folderID, name: name, sourceMeetingID: nil,
            sourceRevisionID: nil, sourceSpeakerID: nil, createdAt: Date())
        return edit({
            if $0.speakerProfiles == nil { $0.speakerProfiles = [] }
            $0.speakerProfiles?.append(profile)
        }) ? profile.id : nil
    }

    func renameTeamMember(profileID: UUID, name: String) -> Bool {
        guard canEditPeople(), let name = Self.singleLineName(name),
              let index = library.speakerProfiles?.firstIndex(where: { $0.id == profileID }) else { return false }
        return edit { $0.speakerProfiles?[index].name = name }
    }

    func recentAttendance(in folderID: UUID?) -> MeetingAttendance {
        var result = library.recentAttendance?[Self.attendanceKey(folderID)] ?? MeetingAttendance()
        let available = Set(folderID.map { speakerProfiles(in: $0).map(\.id) } ?? [])
        result.profileIDs.removeAll { !available.contains($0) }
        return result
    }

    func rememberAttendance(_ attendance: MeetingAttendance, folderID: UUID?) {
        edit {
            if $0.recentAttendance == nil { $0.recentAttendance = [:] }
            $0.recentAttendance?[Self.attendanceKey(folderID)] = attendance
        }
    }

    static func singleLineName(_ value: String) -> String? {
        let name = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, !name.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) else { return nil }
        return name
    }

    private static func attendanceKey(_ folderID: UUID?) -> String { folderID?.uuidString ?? "unfiled" }
}
