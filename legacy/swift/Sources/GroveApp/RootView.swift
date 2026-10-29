import SwiftUI
import UniformTypeIdentifiers

struct RootView: View {
    @ObservedObject var store: GroveStore
    @State private var columnVisibility: NavigationSplitViewVisibility = .all

    var body: some View {
        NavigationSplitView(columnVisibility: $columnVisibility) {
            GroveSidebar(store: store)
                .navigationSplitViewColumnWidth(min: 220, ideal: 250, max: 310)
        } detail: {
            NavigationStack {
                detail
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(GroveTheme.canvas)
                    .navigationTitle(navigationTitle)
                    .toolbar {
                        ToolbarSpacer(.flexible, placement: .primaryAction)
                        ToolbarItemGroup(placement: .primaryAction) {
                            Button {
                                store.requestFileImport()
                            } label: {
                                Label("파일 가져오기", systemImage: "square.and.arrow.down")
                            }
                            .disabled(!store.canPresentNewMeeting)

                            Button {
                                store.present(.newMeeting())
                            } label: {
                                Label("새 회의", systemImage: "record.circle")
                            }
                            .buttonStyle(.borderedProminent)
                            .disabled(!store.canPresentNewMeeting)
                        }
                    }
            }
        }
        #if DEBUG
        .background(WorkspaceLayoutProbe(state: "\(String(describing: store.selection)):\(store.isProcessing)"))
        #endif
        .sheet(item: $store.workspaceSheet, onDismiss: store.workspaceSheetDidDismiss) { sheet in
            switch sheet {
            case .newMeeting(let event): NewMeetingSheet(store: store, calendarEvent: event)
            case .importRecording(let url): ImportRecordingOptionsSheet(store: store, source: url)
            case .renameRecording(let id):
                if let meeting = store.meetings.first(where: { $0.id == id }) {
                    RecordingNameEditor(store: store, meeting: meeting)
                }
            case .originalRecording(let id): OriginalRecordingFilesSheet(store: store, meetingID: id)
            }
        }
        .fileImporter(
            isPresented: $store.isPresentingImporter,
            allowedContentTypes: [.audio, .movie],
            allowsMultipleSelection: false
        ) { result in
            switch result {
            case .success(let urls):
                if let url = urls.first {
                    store.present(.importRecording(url))
                }
            case .failure(let error):
                store.alertMessage = error.localizedDescription
            }
        }
        .alert(
            "Grove",
            isPresented: Binding(
                get: { store.alertMessage != nil },
                set: { if !$0 { store.alertMessage = nil } }
            )
        ) {
            Button("확인", role: .cancel) { store.alertMessage = nil }
        } message: {
            Text(store.alertMessage ?? "")
        }
        .overlay(alignment: .bottom) {
            VStack(spacing: 8) {
                FolderMoveFeedbackView(store: store)
                if store.isRecording, let meeting = store.activeMeeting {
                    RecordingHUD(store: store, meeting: meeting)
                        .transition(.move(edge: .bottom).combined(with: .opacity))
                }
            }
            .padding(.bottom, 18)
        }
    }

    private var navigationTitle: String {
        switch store.selection {
        case .library, .none: "모든 녹음"
        case .unfiled: "미분류"
        case .folder(let id): store.folderName(id)
        case .meeting(let id): store.meetings.first(where: { $0.id == id })?.title ?? "Grove"
        }
    }

    @ViewBuilder
    private var detail: some View {
        switch store.selection {
        case .library, .none:
            if store.needsModelSetup { FirstRunSetupView(store: store) }
            else { LibraryHomeView(store: store) }
        case .unfiled:
            LibraryHomeView(store: store, showsUnfiled: true)
        case .folder(let id):
            LibraryHomeView(store: store, folderID: id)
        case .meeting(let id):
            if let meeting = store.meetings.first(where: { $0.id == id }) {
                MeetingDetailView(store: store, meeting: meeting)
            } else {
                ContentUnavailableView("회의를 찾을 수 없습니다", systemImage: "waveform.slash")
            }
        }
    }
}
