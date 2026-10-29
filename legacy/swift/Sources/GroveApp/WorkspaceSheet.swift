import Foundation

enum WorkspaceSheet: Identifiable, Equatable {
    case newMeeting(ScheduledMeeting? = nil)
    case importRecording(URL)
    case renameRecording(UUID)
    case originalRecording(UUID)

    var id: String {
        switch self {
        case .newMeeting(let event): "new:\(event?.id ?? "manual")"
        case .importRecording(let url): "import:\(url.absoluteString)"
        case .renameRecording(let id): "rename:\(id)"
        case .originalRecording(let id): "original:\(id)"
        }
    }
}

enum WorkspaceStartAction {
    case recording(title: String, plan: MeetingInferencePlan, folderID: UUID?, calendarEvent: ScheduledMeeting?, attendance: MeetingAttendance)
    case importing(source: URL, plan: MeetingInferencePlan, folderID: UUID?)
}
