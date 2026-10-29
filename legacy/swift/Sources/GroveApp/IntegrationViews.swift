import SwiftUI

struct CalendarHomeSection: View {
    @ObservedObject var schedule: CalendarSchedule
    @ObservedObject var models: ModelManager
    let isBusy: Bool
    var requiresModels = true
    @Environment(\.openSettings) private var openSettings
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if requiresModels && !models.isReadyForUse {
                HStack(spacing: 14) {
                    Image(systemName: "arrow.down.circle").font(.title2).foregroundStyle(GroveTheme.grove)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("전사 모델 설치").font(GroveTypography.heading)
                        Text(models.message ?? "다운로드 약 1.9 GB. 설치 후 오프라인 전사 가능.")
                            .font(GroveTypography.bodySmall).foregroundStyle(.secondary)
                    }
                    Spacer()
                    if models.isInstalling { ProgressView().controlSize(.small) }
                    else { Button("모델 설치") { models.install() }.modifier(GroveActionAppearance()).disabled(isBusy) }
                }.padding(18).background(GroveTheme.grove.opacity(0.06), in: RoundedRectangle(cornerRadius: 12))
            }
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Label("다가오는 회의", systemImage: "calendar").font(GroveTypography.heading)
                    Spacer()
                    Button("일정 설정") { UserDefaults.standard.set("calendar", forKey: "settingsTab"); openSettings() }
                        .modifier(GroveActionAppearance()).controlSize(.regular)
                }
                if !schedule.enabled {
                    HStack {
                        Text("Mac 캘린더에서 회의 일정을 가져옵니다.").foregroundStyle(.secondary)
                        Spacer()
                        Button("캘린더 연결") { Task { await schedule.setEnabled(true) } }
                            .modifier(GroveActionAppearance())
                    }
                } else if let message = schedule.message {
                    Text(message).foregroundStyle(.secondary)
                } else if schedule.meetings.isEmpty {
                    Text("선택한 캘린더에 예정된 회의가 없습니다.").foregroundStyle(.secondary)
                } else {
                    ForEach(Array(schedule.meetings.prefix(3))) { event in
                        CalendarMeetingRow(meeting: event, isBusy: isBusy) { schedule.record(event) }
                    }
                }
            }.padding(18).background(GroveTheme.surface, in: RoundedRectangle(cornerRadius: 12))
                .overlay { RoundedRectangle(cornerRadius: 12).stroke(GroveTheme.divider) }
        }
    }
}

struct CalendarMeetingRow: View {
    let meeting: ScheduledMeeting
    let isBusy: Bool
    let record: () -> Void

    var body: some View {
        HStack(spacing: 16) {
            Text(meeting.start, format: .dateTime.hour().minute()).font(GroveTypography.heading).monospacedDigit()
                .fixedSize().frame(minWidth: 80, alignment: .leading)
            VStack(alignment: .leading, spacing: 4) {
                Text(meeting.title).font(GroveTypography.body).lineLimit(1)
                Text("\(meeting.start.formatted(date: .abbreviated, time: .omitted)) / \(meeting.calendarName)")
                    .font(GroveTypography.bodySmall).foregroundStyle(.secondary)
            }
            Spacer()
            Button("녹음 시작", action: record).modifier(GroveActionAppearance()).disabled(isBusy)
        }.padding(.vertical, 7)
    }
}

struct CalendarSettingsView: View {
    @ObservedObject var schedule: CalendarSchedule
    var body: some View {
        Form {
            Section("회의 일정 알림") {
                Toggle("캘린더 일정 알림 사용", isOn: Binding(get: { schedule.enabled }, set: { value in Task { await schedule.setEnabled(value) } }))
                Text("Mac 캘린더에 등록된 일정을 사용합니다.")
                    .font(.callout).foregroundStyle(.secondary)
                Picker("미리 알림", selection: $schedule.leadMinutes) {
                    Text("시작 시각").tag(0); Text("5분 전").tag(5); Text("10분 전").tag(10)
                }
                Button("상단 알림 미리보기") { schedule.previewReminder() }
                if let message = schedule.message { Text(message).font(.caption).foregroundStyle(GroveTheme.evidence) }
            }
            if schedule.enabled {
                Section("알림을 받을 캘린더") {
                    ForEach(schedule.calendars) { calendar in
                        Toggle(calendar.title, isOn: Binding(get: { schedule.selectedCalendarIDs.contains(calendar.id) }, set: { value in
                            if value { schedule.selectedCalendarIDs.insert(calendar.id) }
                            else { schedule.selectedCalendarIDs.remove(calendar.id) }
                            schedule.saveSelection()
                        }))
                    }
                    Text("종일 일정, 취소된 일정과 참석을 거절한 일정은 제외합니다. Grove가 실행 중일 때 알림이 표시됩니다.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            Section("Google 계정 연결") {
                Text("Mac 캘린더 앱에서 Google 계정을 먼저 연결해 주세요. 일정 조회를 위해 macOS가 캘린더 전체 접근 권한을 요청합니다.")
                    .font(.callout).foregroundStyle(.secondary)
                Button("Mac 캘린더 열기") { NSWorkspace.shared.open(URL(fileURLWithPath: "/System/Applications/Calendar.app")) }
            }
        }.formStyle(.grouped)
    }
}

struct ModelSettingsView: View {
    @ObservedObject var models: ModelManager
    @ObservedObject var apple: AppleTranscriptionPreparation
    let isBusy: Bool
    var body: some View {
        Form {
            Section("Mac 기본 전사") {
                LabeledContent("한국어", value: apple.isReady ? "준비됨" : "준비 필요")
                if apple.isPreparing {
                    if let progress = apple.progress { ProgressView(progress) }
                    Button("준비 중단") { apple.cancel() }
                } else {
                    Button("Mac 기본 전사 준비") { apple.prepare() }.disabled(isBusy)
                }
                if let message = apple.message { Text(message).font(.caption).foregroundStyle(.secondary) }
            }
            Section("전사 모델") {
                ForEach(ModelGroup.allCases) { group in
                    LabeledContent(group.label) {
                        Text((group == .voiceIdentity ? models.isVoiceModelReady : models.isReadyForUse) ? "준비됨" : "준비 필요").foregroundStyle(.secondary)
                    }
                }
                Text("다운로드 약 1.9 GB. 설치 후에는 인터넷 없이 전사할 수 있습니다.")
                    .font(.callout).foregroundStyle(.secondary)
                if models.isInstalling {
                    ProgressView(value: models.fraction)
                    Button("준비 중단") { models.cancel() }
                } else {
                    HStack {
                        Button(models.isReadyForUse ? "모델 확인" : "모델 설치") { models.install() }.disabled(isBusy)
                        Button("목소리 모델 준비") { models.install(groups: [.voiceIdentity]) }.disabled(isBusy)
                    }
                }
                if let message = models.message { Text(message).font(.caption).foregroundStyle(.secondary) }
            }
            Section("화자 분리") {
                Text("Nemotron 3는 최대 8명을 자동으로 구분하며 인원수를 강제하지 않습니다. 전사 후 화자 배정을 확인해 주세요. 기존 녹음의 결과는 그대로 유지됩니다.")
                    .font(.callout).foregroundStyle(.secondary)
                Button("모델 폴더 열기") { NSWorkspace.shared.open(models.baseDirectory.appendingPathComponent("Models")) }
            }
        }.formStyle(.grouped)
        .task { models.refresh(); await apple.refresh() }
    }
}

struct NotionSettingsView: View {
    @ObservedObject var connection: NotionConnection
    var isBusy = false
    @State private var token = ""
    @State private var message: String?
    @State private var parentLink = ""
    var body: some View {
        Form {
            Section("Notion 연결") {
                if connection.isConnected {
                    Label(connection.isOAuth ? "Notion 연결됨" : "토큰으로 연결됨", systemImage: "checkmark.circle")
                }
                HStack(spacing: 12) {
                    if connection.isConnecting {
                        ProgressView().controlSize(.small)
                        Text("연결 중")
                        Button("취소") { connection.cancelConnect() }
                    } else {
                        Button(connection.isConnected ? "다시 연결" : "Notion 연결") {
                            message = nil
                            Task { await connection.connect() }
                        }.buttonStyle(.borderedProminent)
                            .disabled(isBusy || connection.isRefreshing)
                        if connection.isConnected || connection.requiresReconnect {
                            Button("연결 해제") {
                                do { try connection.disconnect() }
                                catch { message = error.localizedDescription }
                            }.disabled(isBusy || connection.isRefreshing)
                        }
                    }
                }
                if let message = message ?? connection.message { Text(message).font(.callout).foregroundStyle(.secondary) }
                DisclosureGroup("토큰으로 연결") {
                    SecureField("연결 토큰", text: $token)
                    Button("토큰으로 연결") {
                        do { try connection.saveManualToken(token); token = ""; message = nil }
                        catch { message = error.localizedDescription }
                    }.disabled(isBusy || connection.isConnecting || connection.isRefreshing || token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    Link("Notion 연결 설정 열기", destination: URL(string: "https://www.notion.so/profile/integrations")!)
                }
            }
            Section("기본 저장 위치") {
                TextField("추가할 페이지 링크", text: $parentLink, prompt: Text("https://app.notion.com/p/..."))
                    .disabled(isBusy || connection.isConnecting)
                if let error = NotionPageLink.validationMessage(for: parentLink) { Text(error).font(.callout).foregroundStyle(.secondary) }
                Text("기존 본문 맨 아래에 구분선과 회의록 하위 페이지를 추가합니다. 데이터베이스 저장은 이 버전에서 지원하지 않습니다.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }.formStyle(.grouped)
            .onAppear { parentLink = connection.parentLink }
            .onChange(of: connection.destinationKey) { _, _ in parentLink = connection.parentLink }
            .onChange(of: parentLink) { _, value in connection.parentLink = value }
    }
}

struct NotionExportSheet: View {
    let meeting: MeetingRecord
    let document: TranscriptDocument
    @ObservedObject var exporter: NotionExporter
    @ObservedObject var connection: NotionConnection
    @State private var parentLink = ""
    @State private var original = false
    @State private var message: String?
    @State private var savedURL: URL?
    @State private var recoveryLink = ""
    @State private var uncertain = false
    @State private var dividerPending = false
    @State private var checkingParent = false
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openSettings) private var openSettings
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack {
                VStack(alignment: .leading, spacing: 5) {
                    Text("Notion 내보내기").font(GroveTypography.title)
                    Text("화자와 시간을 포함한 전체 전사문을 내보냅니다.").foregroundStyle(.secondary)
                }
                Spacer()
                Button("닫기") { dismiss() }.disabled(exporter.isSaving)
            }
            Picker("내보낼 내용", selection: $original) {
                Text("검토한 전사문").tag(false); Text("최초 전사문").tag(true)
            }.pickerStyle(.segmented).disabled(exporter.isSaving)
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text(MeetingExportContent.title(meeting)).font(GroveTypography.heading)
                    let source = TranscriptRenderer.exportDocument(document, original: original)
                    ForEach(Array(source.utterances.sorted { $0.startTime < $1.startTime }.prefix(30))) { utterance in
                        VStack(alignment: .leading, spacing: 4) {
                            Text("[\(TranscriptRenderer.timestamp(utterance.startTime))] \(TranscriptRenderer.speakerLabel(in: source, for: utterance))")
                                .font(GroveTypography.label).foregroundStyle(.secondary)
                            Text(original ? utterance.rawText : utterance.displayedText).font(GroveTypography.bodySmall)
                        }
                    }
                }.textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
            }.padding(14).frame(height: 190).background(GroveTheme.canvas, in: RoundedRectangle(cornerRadius: 8))
            HStack {
                Button("Notion용 서식 복사") {
                    message = MeetingExportContent.copy(meeting: meeting, document: document, original: original)
                        ? "복사했습니다. Notion에 붙여넣어 주세요." : "클립보드에 복사하지 못했습니다."
                }
                Spacer()
                Text("복사는 계정 연결 없이 사용할 수 있습니다.").font(.caption).foregroundStyle(.secondary)
            }
            Divider()
            Text("페이지 하단에 회의록 추가").font(GroveTypography.heading)
            HStack {
                TextField("추가할 페이지 링크", text: $parentLink, prompt: Text("https://app.notion.com/p/..."))
                    .textFieldStyle(.roundedBorder).disabled(exporter.isSaving)
                Button("위치 확인") {
                    checkingParent = true
                    let link = parentLink, destinationKey = connection.destinationKey
                    Task {
                        defer { checkingParent = false }
                        do {
                            let title = try await client().parentTitle(id: NotionPageLink.id(from: link))
                            if parentLink == link && connection.destinationKey == destinationKey { message = "저장 위치: " + title }
                        } catch {
                            if parentLink == link && connection.destinationKey == destinationKey { message = error.localizedDescription }
                        }
                    }
                }.disabled(!connection.isConnected || connection.isConnecting || checkingParent || exporter.isSaving || (try? NotionPageLink.id(from: parentLink)) == nil)
            }
            if let message = NotionPageLink.validationMessage(for: parentLink) ?? message { Text(message).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true) }
            if uncertain {
                HStack {
                    TextField("생성된 회의록 페이지 링크", text: $recoveryLink).textFieldStyle(.roundedBorder)
                    Button("생성된 페이지 연결") {
                        Task {
                            do { savedURL = try await exporter.connectCreatedPage(meetingID: meeting.id, link: recoveryLink, client: client()); uncertain = false }
                            catch { message = error.localizedDescription }
                        }
                    }.disabled(exporter.isSaving || (try? NotionPageLink.id(from: recoveryLink)) == nil)
                }
                if let error = NotionPageLink.validationMessage(for: recoveryLink) { Text(error).font(.callout).foregroundStyle(.secondary) }
            }
            HStack {
                Button("Notion 연결 설정") { UserDefaults.standard.set("notion", forKey: "settingsTab"); openSettings() }
                Spacer()
                if let savedURL { Link("추가된 회의록 열기", destination: savedURL) }
                if exporter.isSaving {
                    ProgressView().controlSize(.small)
                    if let stage = exporter.stage { Text(stage.label).font(.caption).foregroundStyle(.secondary) }
                }
                if dividerPending && !exporter.isSaving {
                    Button("처음부터 다시") {
                        do {
                            try exporter.discardPendingDivider(meetingID: meeting.id)
                            dividerPending = false
                            message = nil
                        } catch { message = error.localizedDescription }
                    }.help("구분선이 추가되지 않은 것을 Notion에서 확인한 뒤 누르세요. 회의록 페이지는 만들어지지 않았습니다.")
                }
                Button(dividerPending ? "결과 확인" : "적용") {
                    Task {
                        do {
                            savedURL = try await exporter.save(meeting: meeting, document: document, parentLink: parentLink, original: original, client: client())
                            connection.parentLink = parentLink
                            message = "페이지 하단에 회의록을 추가했습니다."
                            dividerPending = false
                        } catch {
                            message = error.localizedDescription
                            dividerPending = (try? exporter.receipt(meetingID: meeting.id))?.phase == .dividerPending
                            if case NotionExportError.unknownResult = error { uncertain = true }
                        }
                    }
                }.buttonStyle(.borderedProminent)
                    .disabled(!connection.isConnected || connection.isConnecting || exporter.isSaving || checkingParent || uncertain || (try? NotionPageLink.id(from: parentLink)) == nil)
            }
        }.padding(26).frame(width: 640)
            .interactiveDismissDisabled(exporter.isSaving)
            .onAppear {
                parentLink = connection.parentLink
                if let receipt = try? exporter.receipt(meetingID: meeting.id) {
                    dividerPending = receipt.phase == .dividerPending && receipt.pageID == nil
                    if let id = receipt.pageID { savedURL = NotionPageLink.url(for: id) }
                }
            }
            .onChange(of: connection.destinationKey) { _, _ in
                if !exporter.isSaving { parentLink = connection.parentLink }
            }
            .onChange(of: parentLink) { _, _ in message = nil }
    }
    private func client() throws -> any NotionPageClient {
        try connection.client()
    }
}
