import SwiftUI

struct NewMeetingSheet: View {
    @ObservedObject var store: GroveStore
    let calendarEvent: ScheduledMeeting?
    @State private var title = ""
    @State private var options: MeetingSpeakerOptions
    @State private var folderID: UUID?
    @State private var showsOptions = false
    @State private var attendance: MeetingAttendance
    @State private var showsAttendance = false
    private var plan: MeetingInferencePlan? { try? options.plan(isDual: false) }
    private var preparationMessage: String? { plan.flatMap { store.preparationMessage(for: $0) } }

    init(store: GroveStore, calendarEvent: ScheduledMeeting? = nil) {
        self.store = store
        self.calendarEvent = calendarEvent
        _title = State(initialValue: calendarEvent?.title ?? "")
        _options = State(initialValue: store.defaultSpeakerOptions)
        _folderID = State(initialValue: store.selectedFolderID)
        _attendance = State(initialValue: store.recentAttendance(in: store.selectedFolderID))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 24) {
            Text("새 회의")
                .font(GroveTypography.title)

            Form {
                TextField("회의 제목", text: $title, prompt: Text("회의 제목 입력"))
                RecordingFolderPicker(store: store, folderID: $folderID)
                LabeledContent("입력", value: "마이크")
                DisclosureGroup("참석자 \(attendance.count)명", isExpanded: $showsAttendance) {
                    MeetingAttendanceView(store: store, folderID: folderID, attendance: $attendance)
                }
                DisclosureGroup("전사 옵션", isExpanded: $showsOptions) {
                    MeetingSpeakerOptionsView(options: $options)
                }
            }
            .formStyle(.grouped)

            if let preparationMessage { MeetingPreparationNotice(message: preparationMessage) { store.workspaceSheet = nil } }

            HStack {
                Spacer()
                Button("취소") { store.cancelRecordingStart(); store.workspaceSheet = nil }
                    .keyboardShortcut(.cancelAction)
                Button("녹음 시작") {
                    guard let plan = try? options.plan(isDual: false) else { return }
                    store.startAfterDismissingSheet(.recording(title: title, plan: plan, folderID: folderID,
                        calendarEvent: calendarEvent, attendance: attendance))
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.defaultAction)
                .disabled(store.isBusy || plan == nil || preparationMessage != nil)
            }
        }
        .padding(28)
        .frame(width: 520)
        .onChange(of: folderID) { _, value in attendance = store.recentAttendance(in: value) }
        .onDisappear { store.cancelRecordingStart() }
    }
}
