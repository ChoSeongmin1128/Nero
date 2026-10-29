import SwiftUI

struct VoiceIdentityAvailabilityNotice: View {
    var body: some View {
        Text("목소리 자동 식별은 정확도 검증 중입니다. 이름 저장·직접 연결은 사용할 수 있습니다.")
            .font(.caption).foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

struct VoiceEnrollmentSheet: View {
    @ObservedObject var store: GroveStore
    let meeting: MeetingRecord
    let speaker: MeetingSpeaker
    @ObservedObject var player: AudioPlayerController
    @State private var name: String
    @State private var selectedIDs: Set<UUID> = []
    @State private var permissionConfirmed = false
    @State private var enableAutomaticIdentification = false
    @State private var submitting = false
    @State private var cancelling = false
    @State private var error: String?
    @Environment(\.dismiss) private var dismiss

    init(store: GroveStore, meeting: MeetingRecord, speaker: MeetingSpeaker, player: AudioPlayerController) {
        self.store = store
        self.meeting = meeting
        self.speaker = speaker
        self.player = player
        _name = State(initialValue: speaker.name)
    }

    private var candidates: [DocumentUtterance] {
        store.voice.enrollmentCandidates(meetingID: meeting.id, speakerID: speaker.id)
    }

    private var selectedDuration: Double {
        candidates.filter { selectedIDs.contains($0.id) }
            .reduce(0) { $0 + store.voice.enrollmentDuration($1) }
    }

    private var existingProfile: SavedSpeakerProfile? {
        guard let folderID = meeting.folderID else { return nil }
        let profiles = store.speakerProfiles(in: folderID)
        if let linked = profiles.first(where: { $0.id == speaker.profileMatch?.profileID }) { return linked }
        return profiles.first {
            $0.sourceMeetingID == meeting.id && $0.sourceSpeakerID == speaker.id
        }
    }

    private var canRegister: Bool {
        (3...5).contains(selectedIDs.count) && selectedDuration >= 10 && permissionConfirmed
            && !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !submitting && !store.isBusy && store.voice.isAvailable
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            VStack(alignment: .leading, spacing: 7) {
                Text("목소리 등록").font(GroveTypography.heading)
                Text("‘\(store.folderName(meeting.folderID))’의 다른 녹음에서 이 사람의 이름을 찾을 때 사용합니다. 이름만 저장하는 것과는 별개입니다.")
                    .font(GroveTypography.bodySmall).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if !store.voice.isAvailable { VoiceIdentityAvailabilityNotice() }
            VStack(alignment: .leading, spacing: 6) {
                TextField("화자 이름", text: $name).textFieldStyle(.roundedBorder)
                if let profile = existingProfile {
                    Text(store.voice.isRegistered(profile.id)
                         ? "‘\(profile.name)’에 등록한 목소리를 이번에 선택한 발화로 교체합니다. 이름을 바꾸려면 먼저 화자 이름을 수정해 주세요."
                         : "저장한 이름 ‘\(profile.name)’에 목소리를 추가합니다. 이름을 바꾸려면 먼저 화자 이름을 수정해 주세요.")
                        .font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .disabled(submitting || !store.voice.isAvailable)
            VStack(alignment: .leading, spacing: 8) {
                Text("단독 발화를 듣고 선택해 주세요").font(GroveTypography.label)
                Text("선택한 모든 구간이 이 사람의 목소리인지 확인해 주세요. 3~5개를 선택하고, 실제 사용하는 음성의 합계가 10초 이상이어야 합니다.")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                if candidates.isEmpty {
                    Text("등록할 만한 단독 발화가 없습니다. 화자 배정을 먼저 확인하거나, 더 길고 겹치지 않는 발화가 있는 녹음에서 등록해 주세요.")
                        .font(GroveTypography.bodySmall).foregroundStyle(GroveTheme.evidence)
                        .padding(.vertical, 12)
                } else {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 0) {
                            ForEach(candidates) { utterance in
                                sampleRow(utterance)
                                if utterance.id != candidates.last?.id { Divider().padding(.vertical, 8) }
                            }
                        }.padding(12)
                    }
                    .frame(maxHeight: 260)
                    .background(GroveTheme.canvas, in: RoundedRectangle(cornerRadius: 8))
                }
                Text("\(selectedIDs.count)개 선택 · 사용할 음성 \(Int(selectedDuration))초")
                    .font(.caption).foregroundStyle(.secondary)
                    .accessibilityAddTraits(.updatesFrequently)
            }
            .disabled(submitting || !store.voice.isAvailable)
            VStack(alignment: .leading, spacing: 10) {
                Toggle("선택한 발화가 같은 사람의 목소리이며, 이 목소리를 등록할 권한이 있습니다", isOn: $permissionConfirmed)
                    .toggleStyle(.checkbox).font(GroveTypography.bodySmall)
                    .fixedSize(horizontal: false, vertical: true)
                Toggle("이 폴더의 새 녹음에서 화자 자동 식별", isOn: $enableAutomaticIdentification)
                    .toggleStyle(.checkbox).font(GroveTypography.bodySmall)
                Text("음성 특징은 이 Mac에 암호화해 저장합니다. 애매한 목소리는 이름을 붙이지 않으며, 제안한 이름도 틀릴 수 있습니다. 등록한 목소리는 폴더에서 삭제할 수 있습니다.")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .disabled(submitting || !store.voice.isAvailable)
            if let error {
                Text(error).font(.caption).foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.updatesFrequently)
            }
            if let error = player.errorMessage {
                Text(error).font(.caption).foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: 12) {
                if submitting {
                    ProgressView().controlSize(.small)
                    Text(store.voice.isCommittingVoiceEnrollment ? "암호화 저장 중"
                         : cancelling ? "취소 중" : "선택한 목소리 확인 중")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button(submitting ? "등록 취소" : "취소") {
                    if submitting {
                        guard !store.voice.isCommittingVoiceEnrollment else { return }
                        cancelling = true
                        store.voice.cancelWork()
                    } else { dismiss() }
                }
                .keyboardShortcut(.cancelAction).disabled(cancelling || store.voice.isCommittingVoiceEnrollment)
                Button(existingProfile.map { store.voice.isRegistered($0.id) } == true ? "목소리 교체" : "목소리 등록") {
                    register()
                }
                .buttonStyle(.borderedProminent).disabled(!canRegister)
            }
        }
        .padding(24).frame(width: 580)
        .interactiveDismissDisabled(submitting)
        .onDisappear { player.pause() }
        .onChange(of: candidates.map(\.id)) { _, ids in selectedIDs.formIntersection(ids) }
    }

    private func sampleRow(_ utterance: DocumentUtterance) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Toggle("\(TranscriptPresentation.timeRange(utterance)), 같은 사람의 단독 발화로 확인", isOn: Binding(
                get: { selectedIDs.contains(utterance.id) },
                set: { if $0 { selectedIDs.insert(utterance.id) } else { selectedIDs.remove(utterance.id) } }
            ))
            .labelsHidden().toggleStyle(.checkbox).padding(.top, 3)
            .disabled(selectedIDs.count >= 5 && !selectedIDs.contains(utterance.id))
            VStack(alignment: .leading, spacing: 5) {
                HStack(spacing: 8) {
                    Text(TranscriptPresentation.timeRange(utterance)).monospacedDigit().font(.caption)
                    Button {
                        if player.playingSegmentID == utterance.id { player.pause() }
                        else { player.play(meeting: meeting, utterance: utterance) }
                    } label: {
                        Label(player.playingSegmentID == utterance.id ? "정지" : "듣기",
                              systemImage: player.playingSegmentID == utterance.id ? "pause.fill" : "play.fill")
                    }
                    .buttonStyle(.plain).font(.caption)
                    .accessibilityLabel("\(TranscriptPresentation.timeRange(utterance)) 발화 \(player.playingSegmentID == utterance.id ? "정지" : "듣기")")
                }
                Text(utterance.displayedText).font(GroveTypography.bodySmall)
                    .lineLimit(3).frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    private func register() {
        guard canRegister else { return }
        error = nil
        submitting = true
        player.pause()
        Task {
            let saved = await store.voice.enrollVoice(meetingID: meeting.id, speakerID: speaker.id, name: name,
                                               selectedUtteranceIDs: selectedIDs, permissionConfirmed: permissionConfirmed,
                                               enableAutomaticIdentification: enableAutomaticIdentification)
            submitting = false
            if saved || cancelling { dismiss() }
            else {
                error = store.alertMessage ?? "목소리를 등록하지 못했습니다. 선택한 발화와 입력한 이름은 유지됩니다."
                store.alertMessage = nil
            }
        }
    }
}
