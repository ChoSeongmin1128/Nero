import SwiftUI
import GroveInference

struct MeetingSpeakerOptionsView: View {
    @Binding var options: MeetingSpeakerOptions
    var isDual = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Picker("전사 방식", selection: $options.transcriptionEngine) {
                Text("정밀 전사 / 화자 분리").tag(TranscriptionEngine.moss)
                Text("Mac 기본 전사").tag(TranscriptionEngine.apple)
            }
            if options.transcriptionEngine == .apple {
                Text("Grove 모델을 설치하지 않고 macOS 한국어 전사를 사용합니다. 화자 구분은 제공하지 않습니다.")
                    .font(.caption).foregroundStyle(.secondary)
            } else {
            Picker("화자 수", selection: $options.mode) {
                ForEach(MeetingProcessingMode.allCases) { mode in Text(mode.label).tag(mode) }
            }
            if options.mode == .manualCount {
                if isDual {
                    countField("컴퓨터 소리의 화자 수", text: $options.systemCountText)
                    countField("마이크의 화자 수", text: $options.microphoneCountText)
                    Text("각 채널에서 실제로 발언한 사람 수를 입력하세요. 전체 참석자 수를 양쪽에 반복 입력하지 않습니다.")
                        .font(.caption).foregroundStyle(.secondary)
                } else {
                    countField("발화한 사람 수", text: $options.countText)
                    Text("참석자 총원이 아니라 녹음에서 실제로 발언한 사람 수를 입력하세요.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                if let error = options.validationMessage(isDual: isDual) {
                    Text(error).font(.caption).foregroundStyle(GroveTheme.evidence)
                }
            }
            Text("최대 8명까지 자동으로 구분합니다. 입력한 인원은 결과 확인용이며 강제하지 않습니다.")
                .font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private func countField(_ label: String, text: Binding<String>) -> some View {
        HStack {
            Text(label)
            Spacer()
            TextField(label, text: text, prompt: Text("숫자 입력"))
                .labelsHidden().textFieldStyle(.roundedBorder).frame(width: 96)
                .accessibilityLabel(label)
            Text("명").foregroundStyle(.secondary)
        }
    }
}

struct ImportRecordingOptionsSheet: View {
    @ObservedObject var store: GroveStore
    let source: URL
    @State private var options: MeetingSpeakerOptions
    @State private var folderID: UUID?
    @Environment(\.dismiss) private var dismiss
    private var plan: MeetingInferencePlan? { try? options.plan(isDual: false) }
    private var preparationMessage: String? { plan.flatMap { store.preparationMessage(for: $0) } }

    init(store: GroveStore, source: URL) {
        self.store = store
        self.source = source
        _options = State(initialValue: store.defaultSpeakerOptions)
        _folderID = State(initialValue: store.selectedFolderID)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("녹음 파일 전사").font(GroveTypography.title)
            Text(source.lastPathComponent).lineLimit(2).foregroundStyle(.secondary)
            RecordingFolderPicker(store: store, folderID: $folderID)
            MeetingSpeakerOptionsView(options: $options)
            if let preparationMessage { MeetingPreparationNotice(message: preparationMessage) { dismiss() } }
            HStack {
                Spacer()
                Button("취소") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("전사 시작") {
                    guard let plan = try? options.plan(isDual: false) else { return }
                    store.startAfterDismissingSheet(.importing(source: source, plan: plan, folderID: folderID))
                }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                    .disabled(store.isBusy || plan == nil || preparationMessage != nil)
            }
        }.padding(24).frame(width: 460)
    }
}

struct MeetingPreparationNotice: View {
    let message: String
    let dismissSetup: () -> Void
    @Environment(\.openSettings) private var openSettings
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(message).font(.callout).foregroundStyle(.secondary)
            Button("준비 설정") {
                dismissSetup()
                UserDefaults.standard.set("models", forKey: "settingsTab")
                openSettings()
            }
        }
    }
}
