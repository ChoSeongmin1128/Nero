import SwiftUI

struct RecordingFolderPicker: View {
    @ObservedObject var store: GroveStore
    @Binding var folderID: UUID?

    var body: some View {
        Picker("폴더", selection: $folderID) {
            Text("미분류").tag(nil as UUID?)
            ForEach(store.library.folders) { folder in
                Text(folder.name).tag(Optional(folder.id))
            }
        }
    }
}

struct MeetingMoveMenu: View {
    @ObservedObject var store: GroveStore
    let meeting: MeetingRecord

    var body: some View {
        Menu("폴더로 이동") {
            Button("미분류") { store.moveMeeting(id: meeting.id, to: nil) }
                .disabled(meeting.folderID == nil)
            ForEach(store.library.folders) { folder in
                Button(folder.name) { store.moveMeeting(id: meeting.id, to: folder.id) }
                    .disabled(meeting.folderID == folder.id)
            }
        }
        .disabled(!store.canMoveMeeting(id: meeting.id))
    }
}

struct MeetingDragSource: ViewModifier {
    @ObservedObject var store: GroveStore
    let meeting: MeetingRecord

    @ViewBuilder
    func body(content: Content) -> some View {
        if store.canMoveMeeting(id: meeting.id) {
            content.draggable(store.folderTransfer(for: meeting.id)) {
                Label(meeting.title, systemImage: "waveform")
                    .font(GroveTypography.label).lineLimit(1)
                    .padding(12).background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
            }
        } else { content }
    }
}

struct MeetingFolderDropZone: ViewModifier {
    @ObservedObject var store: GroveStore
    let folderID: UUID?
    var isEnabled = true
    @State private var isTargeted = false

    @ViewBuilder
    func body(content: Content) -> some View {
        if isEnabled {
            content
                .contentShape(Rectangle())
                .background(isTargeted ? GroveTheme.grove.opacity(0.12) : .clear,
                            in: RoundedRectangle(cornerRadius: 6))
                .overlay {
                    RoundedRectangle(cornerRadius: 6)
                        .stroke(isTargeted ? GroveTheme.grove : .clear, lineWidth: 1.5)
                        .allowsHitTesting(false)
                }
                .dropDestination(for: MeetingFolderTransfer.self) { items, _ in
                    store.acceptFolderDrop(items, to: folderID)
                } isTargeted: { isTargeted = $0 }
                .accessibilityHint("이 위치로 녹음을 끌어 옮길 수 있습니다. 녹음의 폴더로 이동 메뉴도 사용할 수 있습니다.")
        } else { content }
    }
}

struct FolderMoveFeedbackView: View {
    @ObservedObject var store: GroveStore

    var body: some View {
        if let feedback = store.folderMoveFeedback {
            HStack(spacing: 10) {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(GroveTheme.grove)
                Text(feedback.message).font(GroveTypography.label)
                Button { store.dismissFolderMoveFeedback(id: feedback.id) } label: {
                    Image(systemName: "xmark").font(.caption)
                }.buttonStyle(.plain).accessibilityLabel("이동 알림 닫기")
            }
            .padding(.horizontal, 14).padding(.vertical, 11)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
            .overlay { RoundedRectangle(cornerRadius: 10).stroke(GroveTheme.divider) }
            .frame(maxWidth: 560)
            .padding(12)
            .task(id: feedback.id) {
                do { try await Task.sleep(for: .seconds(4)) }
                catch { return }
                store.dismissFolderMoveFeedback(id: feedback.id)
            }
        }
    }
}

struct FolderNameEditor: View {
    let title: String
    let save: (String) -> Bool
    @State var name: String
    @State private var failed = false
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text(title).font(GroveTypography.heading)
            TextField("폴더 이름", text: $name).textFieldStyle(.roundedBorder)
            if failed { Text("저장하지 못했습니다. 입력한 이름은 유지됩니다.").foregroundStyle(.red).font(.caption) }
            HStack {
                Spacer()
                Button("취소") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("저장") { if save(name) { dismiss() } else { failed = true } }
                    .buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                    .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }.padding(24).frame(width: 380)
    }
}

struct FolderSpeakerLibraryView: View {
    @ObservedObject var store: GroveStore
    let folderID: UUID
    @State private var removingProfile: SavedSpeakerProfile?
    @State private var removingVoice: SavedSpeakerProfile?
    @State private var deletingVoice = false
    @State private var error: String?
    @State private var addingMember = false
    @State private var renamingMember: SavedSpeakerProfile?
    @State private var enrollingMember: SavedSpeakerProfile?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("팀원").font(GroveTypography.heading)
                Spacer()
                Button("팀원 추가") { addingMember = true }.modifier(GroveActionAppearance()).disabled(store.isBusy)
            }
            let profiles = store.speakerProfiles(in: folderID)
            let hasVoices = profiles.contains { store.voice.isRegistered($0.id) }
            if !profiles.isEmpty {
                if store.voice.isAvailable {
                    Toggle("새 녹음에서 이름 자동 연결", isOn: Binding(
                        get: { store.voice.automaticIdentificationEnabled(folderID: folderID) },
                        set: { enabled in
                            if !store.voice.setAutomaticIdentification(folderID: folderID, enabled: enabled) {
                                error = store.alertMessage ?? "설정을 저장하지 못했습니다."
                                store.alertMessage = nil
                            }
                        }
                    ))
                    .toggleStyle(.checkbox).font(GroveTypography.bodySmall)
                    .disabled(store.isBusy || deletingVoice || !hasVoices || !store.voice.isAvailable)
                }
                ForEach(profiles) { profile in
                    let hasVoiceStorage = store.voice.hasStorage(profile.id)
                    HStack(spacing: 10) {
                        Image(systemName: store.voice.isRegistered(profile.id) ? "waveform" : "person")
                            .foregroundStyle(.secondary)
                        VStack(alignment: .leading, spacing: 3) {
                            Text(profile.name).font(GroveTypography.label)
                            Text(store.voice.needsReenrollment(profile.id) ? "재등록 필요"
                                 : store.voice.isRegistered(profile.id) ? "목소리 등록됨"
                                 : hasVoiceStorage ? "등록 정리 필요" : "이름만 저장")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button(store.voice.isRegistered(profile.id) ? "다시 등록" : "목소리 등록") { enrollingMember = profile }
                            .modifier(GroveActionAppearance()).controlSize(.regular).disabled(store.isBusy || deletingVoice)
                        Menu {
                            Button("이름 변경...") { renamingMember = profile }
                            if hasVoiceStorage {
                                Button("등록한 목소리만 삭제…", role: .destructive) { removingVoice = profile }
                            } else {
                                Button("저장한 이름 삭제…", role: .destructive) { removingProfile = profile }
                            }
                        } label: { Image(systemName: "ellipsis") }
                        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
                        .accessibilityLabel("\(profile.name) 관리")
                        .disabled(store.isBusy || deletingVoice)
                    }
                    .padding(.vertical, 4)
                }
            }
            if deletingVoice {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("등록한 목소리 삭제 중").font(.caption).foregroundStyle(.secondary)
                }
            }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
        }
        .padding(20)
        .background(GroveTheme.surface, in: RoundedRectangle(cornerRadius: 10))
        .sheet(isPresented: $addingMember) { TeamMemberNameSheet(store: store, folderID: folderID) }
        .sheet(item: $renamingMember) { TeamMemberNameSheet(store: store, folderID: folderID, profile: $0) }
        .sheet(item: $enrollingMember) { RecordedVoiceEnrollmentSheet(store: store, profile: $0) }
        .alert("저장한 이름을 삭제할까요?", isPresented: Binding(get: { removingProfile != nil }, set: { if !$0 { removingProfile = nil } })) {
            Button("취소", role: .cancel) { removingProfile = nil }
            Button("이름 삭제", role: .destructive) {
                if let profile = removingProfile, !store.voice.removeSpeakerProfile(id: profile.id) {
                    error = store.alertMessage ?? "화자를 삭제하지 못했습니다."
                    store.alertMessage = nil
                }
                removingProfile = nil
            }
        } message: {
            Text("이 폴더에서 재사용할 이름을 삭제합니다. 기존 녹음과 전사의 화자 이름은 바뀌지 않습니다.")
        }
        .alert("등록한 목소리만 삭제할까요?", isPresented: Binding(get: { removingVoice != nil }, set: { if !$0 { removingVoice = nil } })) {
            Button("취소", role: .cancel) { removingVoice = nil }
            Button("목소리 삭제", role: .destructive) {
                if let profile = removingVoice {
                    deletingVoice = true
                    Task {
                        if !(await store.voice.removeVoiceEnrollment(profileID: profile.id)) {
                            error = store.alertMessage ?? "등록한 목소리를 삭제하지 못했습니다."
                            store.alertMessage = nil
                        }
                        deletingVoice = false
                    }
                }
                removingVoice = nil
            }
        } message: {
            Text("목소리로 이 사람을 찾는 데 쓰는 음성 특징을 삭제합니다. 저장한 이름과 기존 전사는 유지됩니다. 다시 자동 식별하려면 목소리를 새로 등록해야 합니다.")
        }
        .task {
            await store.voice.refreshEnrollmentStatus()
            await store.voice.refreshModelCompatibility()
        }
    }
}

struct SaveSpeakerProfileSheet: View {
    @ObservedObject var store: GroveStore
    let meeting: MeetingRecord
    let speaker: MeetingSpeaker
    @State private var name: String
    @State private var confirmed = false
    @State private var error: String?
    @Environment(\.dismiss) private var dismiss

    init(store: GroveStore, meeting: MeetingRecord, speaker: MeetingSpeaker) {
        self.store = store
        self.meeting = meeting
        self.speaker = speaker
        _name = State(initialValue: speaker.name)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("폴더에 이름 저장").font(GroveTypography.heading)
            Text("‘\(store.folderName(meeting.folderID))’ 안의 다음 녹음에서 같은 사람에게 이 이름을 연결할 수 있습니다.")
                .foregroundStyle(.secondary)
            TextField("화자 이름", text: $name).textFieldStyle(.roundedBorder)
            Toggle("이 화자의 이름과 발화를 확인했습니다", isOn: $confirmed)
                .toggleStyle(.checkbox)
            if store.voice.isAvailable {
                Text("여기서는 이름만 저장합니다. 음성 특징을 저장하거나 자동 식별에 사용하려면 별도로 ‘목소리 등록’을 선택해 주세요.")
                    .font(.callout).foregroundStyle(.secondary)
            } else { VoiceIdentityAvailabilityNotice() }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
            HStack {
                if store.voice.isSavingSpeakerProfile { ProgressView().controlSize(.small); Text("화자 저장 중").font(.caption) }
                Spacer()
                Button("취소") { dismiss() }.keyboardShortcut(.cancelAction).disabled(store.voice.isSavingSpeakerProfile)
                Button("화자 저장") {
                    Task {
                        if await store.voice.saveSpeakerProfile(meetingID: meeting.id, speakerID: speaker.id, name: name) { dismiss() }
                        else { error = store.alertMessage ?? "화자를 저장하지 못했습니다."; store.alertMessage = nil }
                    }
                }.buttonStyle(.borderedProminent)
                    .disabled(!confirmed || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || store.isBusy)
            }
        }.padding(24).frame(width: 480).interactiveDismissDisabled(store.voice.isSavingSpeakerProfile)
    }
}
