import SwiftUI
import GroveInference

enum VoiceEnrollmentPrompts {
    static let reading = [
        "오늘은 조금 일찍 나왔는데 길이 한산하더라고요. 그래서 천천히 걸어왔고, 도착해서 따뜻한 차를 한 잔 마셨어요.",
        "다음 회의는 수요일 오후 세 시쯤 하면 될 것 같아요. 자료는 금요일 오전까지 준비해 놓을게요. 확인할 건 일단 세 가지예요.",
        "이건 일단 작게 해 보고 결정하죠. 결과가 괜찮으면 그때 범위를 조금 넓히면 될 것 같아요. 혹시 다른 의견 있으세요?"
    ]
    static let natural = "최근 맡은 일이나 어제 있었던 일을 평소 회의에서 말하듯 설명해 주세요."
}

struct TeamMemberNameSheet: View {
    @ObservedObject var store: GroveStore
    let folderID: UUID
    var profile: SavedSpeakerProfile?
    @State private var name = ""
    @State private var error: String?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text(profile == nil ? "팀원 추가" : "이름 변경").font(GroveTypography.heading)
            TextField("이름", text: $name).textFieldStyle(.roundedBorder)
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
            HStack {
                Spacer()
                Button("취소") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(profile == nil ? "추가" : "저장") {
                    let saved = profile.map { store.libraryStore.renameTeamMember(profileID: $0.id, name: name) }
                        ?? (store.libraryStore.addTeamMember(folderID: folderID, name: name) != nil)
                    if saved { dismiss() } else { error = "이름을 저장하지 못했습니다." }
                }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                    .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || store.isBusy)
            }
        }.padding(24).frame(width: 360)
            .onAppear { name = profile?.name ?? "" }
    }
}

struct MeetingAttendanceView: View {
    @ObservedObject var store: GroveStore
    let folderID: UUID?
    @Binding var attendance: MeetingAttendance
    @State private var name = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("참석자").font(GroveTypography.label)
                Spacer()
                Text("\(attendance.count)명").monospacedDigit().foregroundStyle(.secondary)
            }
            if let folderID {
                ForEach(store.speakerProfiles(in: folderID)) { profile in
                    Toggle(isOn: Binding(get: { attendance.profileIDs.contains(profile.id) }, set: { selected in
                        attendance.profileIDs.removeAll { $0 == profile.id }
                        if selected { attendance.profileIDs.append(profile.id) }
                    })) {
                        HStack {
                            Text(profile.name)
                            Spacer()
                            Text(store.voice.needsReenrollment(profile.id) ? "재등록 필요"
                                 : store.voice.isRegistered(profile.id) ? "목소리 등록됨" : "이름만 등록")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }.toggleStyle(.checkbox)
                }
            }
            ForEach(Array(attendance.names.enumerated()), id: \.offset) { index, value in
                HStack {
                    Text(value)
                    Spacer()
                    Button { attendance.names.remove(at: index) } label: { Image(systemName: "xmark") }
                        .buttonStyle(.borderless).accessibilityLabel("\(value) 참석자에서 제외")
                }
            }
            HStack {
                TextField("참석자 이름", text: $name).textFieldStyle(.roundedBorder)
                Button("추가") {
                    let cleaned = name.trimmingCharacters(in: .whitespacesAndNewlines)
                    if !cleaned.isEmpty { attendance.names.append(cleaned); name = "" }
                }.modifier(GroveActionAppearance()).controlSize(.regular)
                    .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || attendance.count >= 99)
            }
            Stepper("이름 없는 참석자 \(attendance.guestCount)명", value: $attendance.guestCount, in: 0...max(0, 99 - attendance.profileIDs.count - attendance.names.count))
                .font(GroveTypography.bodySmall)
        }
    }
}

struct RecordedVoiceEnrollmentSheet: View {
    @ObservedObject var store: GroveStore
    @ObservedObject var session: VoiceEnrollmentSession
    @ObservedObject var models: ModelManager
    let profile: SavedSpeakerProfile
    @State private var permissionConfirmed = false
    @State private var submitting = false
    @Environment(\.dismiss) private var dismiss

    init(store: GroveStore, profile: SavedSpeakerProfile) {
        self.store = store; self.profile = profile
        session = store.voiceEnrollmentSession; models = store.modelManager
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack {
                Text("목소리 등록").font(GroveTypography.heading)
                Spacer()
                Text(profile.name).font(GroveTypography.label)
            }
            if !models.isVoiceModelReady {
                VStack(alignment: .leading, spacing: 12) {
                    Text("다운로드 \(ByteCountFormatter.string(fromByteCount: models.voiceDownloadBytes, countStyle: .file))")
                        .font(GroveTypography.bodySmall).foregroundStyle(.secondary)
                    if models.isInstalling {
                        ProgressView(value: models.fraction)
                        Button("중단") { models.cancel() }.modifier(GroveActionAppearance())
                    } else {
                        Button("목소리 모델 준비") { models.install(groups: [.voiceIdentity]) }
                            .modifier(GroveActionAppearance()).disabled(store.isBusy)
                    }
                    if let message = models.message { Text(message).font(.caption).foregroundStyle(.secondary) }
                }
            } else if let review = session.review {
                VStack(alignment: .leading, spacing: 12) {
                    Text("녹음 확인").font(GroveTypography.label)
                    LabeledContent("녹음 시간", value: "\(Int(review.duration))초")
                    LabeledContent("음량이 충분한 구간", value: "\(Int(review.signalSeconds))초")
                    if !review.canExtract {
                        Text(review.clippedFraction >= 0.005 ? "소리가 잘렸습니다. 마이크에서 조금 떨어져 다시 녹음해 주세요."
                             : "녹음 구간이 부족합니다. 예문과 자유 발화를 더 길게 녹음해 주세요.")
                            .font(.caption).foregroundStyle(GroveTheme.evidence)
                    }
                    HStack {
                        Button(session.isPlaying ? "재생 중지" : "녹음 듣기") { session.togglePreview() }.modifier(GroveActionAppearance())
                        Button("다시 녹음") { permissionConfirmed = false; session.discard() }.modifier(GroveActionAppearance())
                    }.disabled(submitting)
                    Toggle("본인 동의를 받았고, 한 사람의 목소리만 녹음했습니다.", isOn: $permissionConfirmed)
                        .toggleStyle(.checkbox).font(GroveTypography.bodySmall).disabled(submitting)
                }
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        Text(session.naturalSpeechStart == nil ? "예문 읽기" : "자유 발화").font(GroveTypography.label)
                        Text(session.naturalSpeechStart == nil ? "평소 회의에서 말하는 속도와 억양으로 읽어 주세요." : VoiceEnrollmentPrompts.natural)
                            .font(GroveTypography.bodySmall).foregroundStyle(.secondary)
                        if session.naturalSpeechStart == nil {
                            ForEach(VoiceEnrollmentPrompts.reading, id: \.self) { Text($0).font(GroveTypography.body).lineSpacing(6) }
                        } else {
                            Text("20-30초 정도 이야기해 주세요.").font(GroveTypography.body)
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }.frame(maxHeight: 250)
                if session.recorder.isRecording {
                    HStack {
                        Image(systemName: "record.circle.fill").foregroundStyle(.red)
                        Text("\(Int(session.recorder.elapsed))초").monospacedDigit()
                        ProgressView(value: session.recorder.level).frame(width: 100)
                        Spacer()
                        if session.naturalSpeechStart == nil {
                            Button("다음: 자유 발화") { session.beginNaturalSpeech() }.modifier(GroveActionAppearance())
                        } else {
                            Button("녹음 완료") { Task { await session.finish() } }.modifier(GroveActionAppearance())
                        }
                    }.font(GroveTypography.bodySmall)
                } else if session.isAnalyzing {
                    HStack { ProgressView().controlSize(.small); Text("녹음 확인 중").font(.caption) }
                } else {
                    Button("녹음 시작") { Task { if !store.isBusy { await session.start() } } }
                        .modifier(GroveActionAppearance()).disabled(store.isBusy)
                }
            }
            if let error = session.error { Text(error).font(.caption).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true) }
            if store.voice.isRegistered(profile.id) {
                Text("등록하면 이전 목소리가 교체됩니다.").font(.caption).foregroundStyle(.secondary)
            }
            Divider()
            HStack {
                Text("등록 후 녹음 파일은 저장되지 않습니다.")
                    .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                Spacer()
                Button("취소") { session.discard(); dismiss() }.keyboardShortcut(.cancelAction).disabled(submitting)
                if session.review != nil {
                    Button(submitting ? "등록 중" : "등록") {
                        submitting = true
                        Task {
                            if await store.voice.enrollRecordedVoice(profileID: profile.id, permissionConfirmed: permissionConfirmed) { dismiss() }
                            submitting = false
                        }
                    }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                        .disabled(submitting || store.isBusy || !permissionConfirmed || session.review?.canExtract != true)
                }
            }
        }.padding(24).frame(width: 540)
            .interactiveDismissDisabled(submitting)
            .onDisappear { if !submitting { session.discard() } }
            .task {
                while !Task.isCancelled {
                    do { try await Task.sleep(for: .milliseconds(250)) } catch { return }
                    if session.recorder.isRecording && session.recorder.elapsed >= VoiceEnrollmentAudio.maximumDuration { await session.finish() }
                }
            }
    }
}
