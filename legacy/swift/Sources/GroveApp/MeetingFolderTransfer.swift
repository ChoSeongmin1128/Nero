import CoreTransferable
import Foundation
import UniformTypeIdentifiers

extension UTType {
    static let groveRecordingReference = UTType(
        exportedAs: "io.github.choseongmin1128.grove.recording-reference", conformingTo: .data)
}

struct MeetingFolderTransfer: Codable, Equatable, Sendable, Transferable {
    let schemaVersion: Int
    let meetingID: UUID
    let scopeID: UUID

    init(meetingID: UUID, scopeID: UUID) {
        schemaVersion = 1
        self.meetingID = meetingID
        self.scopeID = scopeID
    }

    static var transferRepresentation: some TransferRepresentation {
        CodableRepresentation(contentType: .groveRecordingReference)
    }

    static func recordingIDs(in items: [Self], scopeID: UUID) throws -> [UUID] {
        guard !items.isEmpty, items.count <= 100,
              items.allSatisfy({ $0.schemaVersion == 1 && $0.scopeID == scopeID }),
              Set(items.map(\.meetingID)).count == items.count else {
            throw MeetingFolderMoveError.invalidDrag
        }
        return items.map(\.meetingID)
    }
}

struct MeetingFolderMoveFeedback: Identifiable, Equatable {
    let id = UUID()
    let message: String
}

enum MeetingFolderMoveError: Error, LocalizedError {
    case invalidDrag, missingRecording, missingFolder, processing, invalidFolderName, duplicateFolderName

    var errorDescription: String? {
        switch self {
        case .invalidDrag: "이 창의 녹음 목록에서 다시 끌어와 주세요. 외부 텍스트나 파일은 폴더 이동으로 처리하지 않습니다."
        case .missingRecording: "옮길 녹음을 찾을 수 없습니다. 목록을 확인하고 다시 시도해 주세요."
        case .missingFolder: "이동할 폴더를 찾을 수 없습니다. 다른 폴더를 선택해 주세요."
        case .processing: "작업 중인 녹음은 아직 옮길 수 없습니다. 녹음·전사·원본 저장이 끝난 뒤 다시 시도해 주세요."
        case .invalidFolderName: "폴더 이름을 한 줄로 입력해 주세요. ‘미분류’는 기본 보관 위치로 사용합니다."
        case .duplicateFolderName: "같은 이름의 폴더가 있습니다. 구분할 수 있는 다른 이름을 입력해 주세요."
        }
    }
}

enum MeetingFolderListing {
    static func recordings(_ meetings: [MeetingRecord], in folderID: UUID?, folders: [MeetingFolder]) -> [MeetingRecord] {
        if let folderID { return meetings.filter { $0.folderID == folderID } }
        let known = Set(folders.map(\.id))
        return meetings.filter {
            guard let id = $0.folderID else { return true }
            return !known.contains(id)
        }
    }

    static func search(_ meetings: [MeetingRecord], title query: String) -> [MeetingRecord] {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return query.isEmpty ? meetings : meetings.filter { $0.title.localizedStandardContains(query) }
    }

    static func folderName(_ name: String, excluding id: UUID? = nil, folders: [MeetingFolder]) throws -> String {
        let clean = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty, clean != "미분류",
              !clean.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) else {
            throw MeetingFolderMoveError.invalidFolderName
        }
        guard !folders.contains(where: { $0.id != id && $0.name.compare(clean, options: [.caseInsensitive, .diacriticInsensitive]) == .orderedSame }) else {
            throw MeetingFolderMoveError.duplicateFolderName
        }
        return clean
    }
}
