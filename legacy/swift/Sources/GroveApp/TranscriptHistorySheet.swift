import SwiftUI

struct TranscriptHistorySheet: View {
    @ObservedObject var store: GroveStore
    let meetingID: UUID
    @Environment(\.dismiss) private var dismiss
    @State private var archives: [ArchivedTranscript] = []
    @State private var error: String?
    @State private var selected: ArchivedTranscript?
    @State private var confirmsSwitch = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("이전 전사").font(GroveTypography.heading)
            Text("전사를 전환해도 현재 내용과 수정 이력은 보관됩니다.")
                .font(.callout).foregroundStyle(.secondary)
            if let error {
                ContentUnavailableView("이전 전사를 열지 못했습니다", systemImage: "exclamationmark.triangle", description: Text(error))
            } else if archives.isEmpty {
                ContentUnavailableView("이전 전사가 없습니다", systemImage: "clock.arrow.circlepath",
                                       description: Text("다시 전사하거나 이전 기록으로 전환하면 여기에 보관됩니다."))
            } else {
                List(archives) { archive in
                    HStack(alignment: .top, spacing: 16) {
                        VStack(alignment: .leading, spacing: 6) {
                            Text(archive.savedAt, format: .dateTime.year().month().day().hour().minute().second())
                                .font(GroveTypography.label)
                            Text("\(archive.document.utterances.count)개 발화 · \(archive.document.speakers.count)개 화자 이름")
                                .font(.caption).foregroundStyle(.secondary)
                            if let first = archive.document.utterances.first {
                                Text(first.displayedText).font(.callout).lineLimit(2)
                            }
                        }
                        Spacer()
                        Button("이 전사로 전환") { selected = archive; confirmsSwitch = true }
                    }.padding(.vertical, 8)
                }.listStyle(.inset)
            }
            HStack { Spacer(); Button("닫기") { dismiss() }.keyboardShortcut(.cancelAction) }
        }
        .padding(24).frame(width: 620, height: 450)
        .task {
            do { archives = try store.transcripts.previousTranscripts(meetingID: meetingID) }
            catch { self.error = "기존 파일은 보존되어 있습니다. \(error.localizedDescription)" }
        }
        .confirmationDialog("선택한 이전 전사로 전환할까요?", isPresented: $confirmsSwitch, titleVisibility: .visible) {
            Button("전환") {
                if let selected, store.transcripts.restore(selected, meetingID: meetingID) { dismiss() }
                else { error = "전사를 전환하지 못했습니다. 현재 내용은 보존되어 있습니다." }
            }
            Button("취소", role: .cancel) {}
        } message: { Text("현재 전사와 수정 내용도 이전 전사에 보관합니다.") }
    }
}
