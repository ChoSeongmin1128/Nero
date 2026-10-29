import SwiftUI

struct SettingsView: View {
    @ObservedObject var store: GroveStore
    @AppStorage("settingsTab") private var selectedTab = "processing"

    var body: some View {
        TabView(selection: $selectedTab) {
            Form {
                LabeledContent("전사 시점", value: "녹음 종료 후")
                LabeledContent("기본 전사", value: store.defaultSpeakerOptions.transcriptionEngine == .apple ? "Mac 기본 전사" : "MOSS")
                Section("새 녹음·가져오기의 기본값") {
                    MeetingSpeakerOptionsView(options: $store.defaultSpeakerOptions)
                    Text("각 녹음에서 따로 변경할 수 있습니다. 기존 녹음에는 적용되지 않습니다.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                LabeledContent("원본 보관", value: "녹음 파일과 전사 원문 유지")
                Text("텍스트와 화자를 수정해도 원문은 바뀌지 않습니다. 대화 화면에서 변경을 되돌릴 수 있습니다.")
                    .font(.callout).foregroundStyle(.secondary)
            }
            .formStyle(.grouped)
            .tabItem { Label("전사", systemImage: "waveform") }
            .tag("processing")

            ModelSettingsView(models: store.modelManager, apple: store.applePreparation, isBusy: store.isBusy)
                .tabItem { Label("모델", systemImage: "externaldrive") }
                .tag("models")
            CalendarSettingsView(schedule: store.calendarSchedule)
                .tabItem { Label("일정", systemImage: "calendar") }
                .tag("calendar")
            NotionSettingsView(connection: store.notionConnection, isBusy: store.notionExporter.isSaving)
                .tabItem { Label("Notion", systemImage: "square.and.arrow.up") }
                .tag("notion")

            Form {
                Label("녹음 파일과 전사문은 Mac에 저장됩니다.", systemImage: "lock.shield")
                LabeledContent("저장 위치", value: "Application Support/Grove")
                Text("Notion 내보내기에서 적용을 누르면 선택한 전사문이 Notion에 전송됩니다. 음성 파일은 전송하지 않습니다.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .formStyle(.grouped)
            .tabItem { Label("개인정보", systemImage: "hand.raised") }
            .tag("privacy")
        }
        .padding(12)
    }
}
