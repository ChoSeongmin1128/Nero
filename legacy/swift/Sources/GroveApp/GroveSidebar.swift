import SwiftUI

struct GroveSidebar: View {
    @ObservedObject var store: GroveStore
    @State private var createsFolder = false
    @State private var renamingFolder: MeetingFolder?
    @State private var deletingFolder: MeetingFolder?

    var body: some View {
        List(selection: $store.selection) {
            Section {
                RecordingLocationRow(title: "모든 녹음", symbol: "waveform", count: store.meetings.count)
                    .tag(SidebarDestination.library)
            }

            Section {
                RecordingLocationRow(title: "미분류", symbol: "tray", count: store.recordings(in: nil).count)
                    .modifier(MeetingFolderDropZone(store: store, folderID: nil))
                    .tag(SidebarDestination.unfiled)
                ForEach(store.library.folders) { folder in
                    RecordingLocationRow(title: folder.name, symbol: "folder", count: store.recordings(in: folder.id).count)
                        .modifier(MeetingFolderDropZone(store: store, folderID: folder.id))
                        .tag(SidebarDestination.folder(folder.id))
                        .contextMenu {
                            Button("이름 변경…") { renamingFolder = folder }
                            Button("폴더 삭제…") { deletingFolder = folder }.disabled(store.isBusy)
                        }
                }
                Button { createsFolder = true } label: { Label("새 폴더", systemImage: "folder.badge.plus") }
                    .buttonStyle(.plain)
            } header: {
                Text("폴더")
            }

            Section {
                ForEach(store.meetings.prefix(12)) { meeting in
                    MeetingSidebarRow(meeting: meeting, folderName: store.folderName(meeting.folderID),
                                      document: store.transcriptDocuments[meeting.id])
                        .modifier(MeetingDragSource(store: store, meeting: meeting))
                        .tag(SidebarDestination.meeting(meeting.id))
                        .contextMenu {
                            RecordingManagementActions(store: store, meeting: meeting)
                            Divider()
                            MeetingMoveMenu(store: store, meeting: meeting)
                            Button("목록에서 제거") {
                                store.deleteMeeting(meeting)
                            }
                            .disabled((store.isRecording && store.activeMeetingID == meeting.id)
                                      || store.processingMeetingID == meeting.id || store.exportingOriginalMeetingID == meeting.id)
                        }
                }
            } header: {
                HStack {
                    Text("최근 녹음")
                    Spacer()
                    Image(systemName: "clock").help("최근 녹음의 바로가기입니다. 폴더로 옮겨도 여기에서 계속 볼 수 있습니다.")
                }
            }
        }
        .listStyle(.sidebar)
        .navigationTitle("Grove")
        .sheet(isPresented: $createsFolder) {
            FolderNameEditor(title: "새 폴더", save: { store.createFolder(name: $0) != nil }, name: "")
        }
        .sheet(item: $renamingFolder) { folder in
            FolderNameEditor(title: "폴더 이름 변경", save: { store.libraryStore.renameFolder(id: folder.id, name: $0) }, name: folder.name)
        }
        .alert("폴더를 삭제할까요?", isPresented: Binding(get: { deletingFolder != nil }, set: { if !$0 { deletingFolder = nil } })) {
            Button("취소", role: .cancel) { deletingFolder = nil }
            Button("폴더 삭제", role: .destructive) {
                if let folder = deletingFolder { _ = store.deleteFolder(id: folder.id) }
                deletingFolder = nil
            }
        } message: {
            Text("녹음과 전사는 삭제하지 않고 ‘미분류’로 옮깁니다. 폴더에 저장한 화자 정보는 삭제합니다.")
        }
    }

}

private struct RecordingLocationRow: View {
    let title: String
    let symbol: String
    let count: Int

    var body: some View {
        HStack(spacing: 9) {
            Image(systemName: symbol).frame(width: 18)
            Text(title).lineLimit(1)
            Spacer(minLength: 8)
            Text(count.formatted()).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
        }
        .padding(.vertical, 3)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(title), 녹음 \(count)개")
    }
}

private struct MeetingSidebarRow: View {
    let meeting: MeetingRecord
    let folderName: String
    let document: TranscriptDocument?
    private var status: MeetingPresentationStatus { .init(meeting: meeting, document: document) }
    private var speakerReviewCount: Int { document?.speakerReviewCount ?? 0 }

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 7) {
                Image(systemName: status.symbol)
                    .font(.caption)
                    .foregroundStyle(statusColor)
                Text(meeting.title)
                    .lineLimit(1)
                    .fontWeight(.medium)
            }
            HStack {
                Text(folderName).lineLimit(1)
                Spacer(minLength: 6)
                Text(meeting.startedAt, format: .dateTime.month().day())
                if meeting.duration > 0 {
                    Text(meeting.duration.clockString).monospacedDigit()
                }
            }
            .font(.caption2)
            .foregroundStyle(.secondary)
            if speakerReviewCount > 0 {
                Text("화자 확인 \(speakerReviewCount)곳")
                    .font(.caption2).foregroundStyle(GroveTheme.evidence)
            }
        }
        .padding(.vertical, 4)
        .help("\(meeting.title) · \(status.label) · \(folderName)")
    }

    private var statusColor: Color {
        if status.isFailure || meeting.status == .recording { return .red }
        if meeting.status == .processing { return GroveTheme.revision }
        return .secondary
    }
}

extension TimeInterval {
    var clockString: String {
        TranscriptRenderer.timestamp(self)
    }
}
