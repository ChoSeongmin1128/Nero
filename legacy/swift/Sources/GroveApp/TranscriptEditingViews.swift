import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct UtteranceTextEditor: View {
    let utterance: DocumentUtterance
    let save: (String) -> Bool
    @Environment(\.dismiss) private var dismiss
    @State private var text: String
    @State private var saveFailed = false

    init(utterance: DocumentUtterance, save: @escaping (String) -> Bool) {
        self.utterance = utterance
        self.save = save
        _text = State(initialValue: utterance.displayedText)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("발화 내용 수정").font(GroveTypography.heading)
            Text("\(utterance.startTime.clockString)–\(utterance.endTime.clockString)")
                .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
            TextEditor(text: $text).font(GroveTypography.body).lineSpacing(5)
                .padding(8).frame(minHeight: 150)
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(GroveTheme.divider))
                .accessibilityLabel("수정할 발화 내용")
            DisclosureGroup(utterance.parentUtteranceID == nil ? "원문 보기" : "분할 전 원문 보기") {
                ScrollView { Text(utterance.rawText).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }
                    .frame(maxHeight: 120).font(.callout).foregroundStyle(.secondary)
            }
            Text("원문과 녹음은 유지됩니다. 저장 후에도 되돌릴 수 있습니다.")
                .font(.caption).foregroundStyle(.secondary)
            if saveFailed { Text("저장하지 못했습니다. 입력한 내용은 유지됩니다. 다시 시도해 주세요.").font(.caption).foregroundStyle(.red) }
            HStack {
                Spacer()
                Button("취소") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("저장") { if save(text) { dismiss() } else { saveFailed = true } }
                    .buttonStyle(.borderedProminent).keyboardShortcut(.return, modifiers: .command)
                    .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }.padding(24).frame(width: 550)
    }
}

struct UtteranceSplitEditor: View {
    let meeting: MeetingRecord
    let utterance: DocumentUtterance
    @ObservedObject var player: AudioPlayerController
    let save: (Double, String, String) -> Bool
    @Environment(\.dismiss) private var dismiss
    @State private var splitTime: Double
    @State private var firstText: String
    @State private var secondText = ""
    @State private var saveFailed = false

    init(meeting: MeetingRecord, utterance: DocumentUtterance, player: AudioPlayerController,
         save: @escaping (Double, String, String) -> Bool) {
        self.meeting = meeting
        self.utterance = utterance
        self.player = player
        self.save = save
        let current = player.position
        _splitTime = State(initialValue: current > utterance.startTime && current < utterance.endTime ? current : utterance.startTime)
        _firstText = State(initialValue: utterance.displayedText)
    }

    private var isValid: Bool {
        splitTime.isFinite && utterance.startTime < splitTime && splitTime < utterance.endTime
            && !firstText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !secondText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("발화 나누기").font(GroveTypography.heading)
            Text("화자가 바뀌는 지점을 듣고 시각과 앞뒤 내용을 지정해 주세요.")
                .font(.callout).foregroundStyle(.secondary)
            HStack {
                Button(player.isPlaying ? "일시 정지" : "이 발화 듣기") {
                    if player.isPlaying { player.pause() } else { player.play(meeting: meeting, utterance: utterance) }
                }
                Text(player.position.clockString).monospacedDigit().foregroundStyle(.secondary)
                Button("현재 재생 위치 사용") { splitTime = player.position }
                    .disabled(player.position <= utterance.startTime || player.position >= utterance.endTime)
            }
            HStack {
                Text("분할 시각 (초)")
                TextField("초", value: $splitTime, format: .number.precision(.fractionLength(2)))
                    .textFieldStyle(.roundedBorder).frame(width: 110)
                Text("\(utterance.startTime.clockString)–\(utterance.endTime.clockString) 사이")
                    .font(.caption).foregroundStyle(.secondary)
                Spacer()
            }
            VStack(alignment: .leading, spacing: 6) {
                Text("앞 발화").font(GroveTypography.label)
                TextEditor(text: $firstText).font(GroveTypography.body).frame(height: 100).padding(6)
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(GroveTheme.divider))
                    .accessibilityLabel("분할할 앞 발화")
            }
            VStack(alignment: .leading, spacing: 6) {
                Text("뒤 발화").font(GroveTypography.label)
                TextEditor(text: $secondText).font(GroveTypography.body).frame(height: 100).padding(6)
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(GroveTheme.divider))
                    .accessibilityLabel("분할할 뒤 발화")
            }
            Text("뒤에 해당하는 내용을 앞 칸에서 잘라 옮겨 주세요. 나눈 뒤 각 발화의 화자를 변경할 수 있습니다.")
                .font(.caption).foregroundStyle(.secondary)
            if saveFailed { Text("분할을 저장하지 못했습니다. 기존 발화는 유지됩니다.").foregroundStyle(.red).font(.caption) }
            HStack {
                Spacer()
                Button("취소") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("두 발화로 나누기") {
                    if save(splitTime, firstText, secondText) { dismiss() } else { saveFailed = true }
                }.buttonStyle(.borderedProminent).disabled(!isValid)
            }
        }.padding(24).frame(width: 620)
        .onDisappear { player.pause() }
    }
}

struct SpeakerNameEditor: View {
    let speaker: MeetingSpeaker
    let save: (String) -> Bool
    @Environment(\.dismiss) private var dismiss
    @State private var name: String
    @State private var saveFailed = false

    init(speaker: MeetingSpeaker, save: @escaping (String) -> Bool) {
        self.speaker = speaker
        self.save = save
        _name = State(initialValue: speaker.name)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("화자 이름 변경").font(GroveTypography.heading)
            TextField("화자 이름", text: $name).textFieldStyle(.roundedBorder)
            Text("이 화자의 모든 발화와 내보내기에 적용됩니다. 같은 이름의 다른 화자와 합쳐지지는 않습니다.")
                .font(.callout).foregroundStyle(.secondary)
            if saveFailed { Text("이름을 저장하지 못했습니다. 기존 이름은 유지됩니다.").font(.caption).foregroundStyle(.red) }
            HStack {
                Spacer()
                Button("취소") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("변경") { if save(name) { dismiss() } else { saveFailed = true } }
                    .buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                    .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }.padding(24).frame(width: 420)
    }
}

struct SpeakerAssignmentEditor: View {
    let document: TranscriptDocument
    let utterance: DocumentUtterance
    let selectedIDs: Set<UUID>
    let save: (SpeakerEditTarget, SpeakerEditScope, Bool) -> Bool
    @Environment(\.dismiss) private var dismiss
    @State private var targetID: UUID?
    @State private var createsSpeaker = false
    @State private var newName = ""
    @State private var scopeChoice = 0
    @State private var saveFailed = false
    @State private var confirmsReview = false

    private var scope: SpeakerEditScope {
        switch scopeChoice {
        case 1: .followingSameSpeaker
        case 2: .allSameSpeaker
        case 3: .selected(selectedIDs)
        default: .utterance
        }
    }

    private var affectedIDs: Set<UUID> { (try? document.reassignmentIDs(from: utterance.id, scope: scope)) ?? [] }
    private var changedCount: Int {
        document.utterances.filter { affectedIDs.contains($0.id) && (createsSpeaker || $0.speakerID != targetID) }.count
    }
    private var isValid: Bool {
        changedCount > 0 && (createsSpeaker ? !newName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty : targetID != nil)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("화자 변경").font(GroveTypography.heading)
            Text("\(utterance.startTime.clockString)  \(document.speakerName(for: utterance))")
                .font(.callout).foregroundStyle(.secondary)
            Text(utterance.displayedText).lineLimit(3).font(GroveTypography.body)
            Divider()
            Toggle("새 화자 만들기", isOn: $createsSpeaker)
            if createsSpeaker {
                TextField("새 화자 이름", text: $newName).textFieldStyle(.roundedBorder)
            } else {
                Picker("변경할 화자", selection: $targetID) {
                    Text("화자 선택").tag(nil as UUID?)
                    ForEach(document.speakers.sorted { $0.order < $1.order }) { speaker in
                        Text(speaker.name).tag(Optional(speaker.id))
                    }
                }
            }
            Picker("적용 범위", selection: $scopeChoice) {
                Text("이 발화만").tag(0)
                Text("이 시점부터 같은 화자 발화").tag(1)
                Text("같은 화자의 전체 발화").tag(2)
                if !selectedIDs.isEmpty { Text("선택한 \(selectedIDs.count)개 발화").tag(3) }
            }.pickerStyle(.radioGroup)
            if utterance.speakerID == nil && (scopeChoice == 1 || scopeChoice == 2) {
                Text("화자 미확정 발화에는 여러 사람의 발화가 포함될 수 있습니다.")
                    .font(.caption).foregroundStyle(GroveTheme.evidence)
            }
            Text("\(changedCount)개 발화가 변경됩니다. 검색·필터로 숨겨진 발화도 적용 범위에 포함됩니다.")
                .font(.callout).foregroundStyle(.secondary)
            if scopeChoice == 0 && document.speakerReview(for: utterance).needsReview {
                Toggle("이 발화의 화자 배정을 확인함", isOn: $confirmsReview)
                    .toggleStyle(.checkbox)
                Text("체크하면 변경한 화자로 확인을 마칩니다. 텍스트 검수나 다른 발화의 확인에는 적용되지 않습니다.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if saveFailed { Text("화자 변경을 저장하지 못했습니다. 기존 배정은 유지됩니다.").font(.caption).foregroundStyle(.red) }
            HStack {
                Spacer()
                Button("취소") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(confirmsReview && scopeChoice == 0 ? "화자 변경 및 확인" : "화자 변경") {
                    let target: SpeakerEditTarget
                    if createsSpeaker { target = .new(newName) }
                    else if let targetID { target = .existing(targetID) }
                    else { return }
                    if save(target, scope, confirmsReview && scopeChoice == 0) { dismiss() } else { saveFailed = true }
                }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction).disabled(!isValid)
            }
        }.padding(24).frame(width: 480)
            .onChange(of: scopeChoice) { _, _ in confirmsReview = false }
    }
}

struct TranscriptExportSheet: View {
    let document: TranscriptDocument
    let title: String
    let selectedIDs: Set<UUID>
    let visibleIDs: Set<UUID>
    @Environment(\.dismiss) private var dismiss
    @AppStorage("exportIncludesSpeakers") private var includesSpeakers = true
    @AppStorage("exportIncludesTimestamps") private var includesTimestamps = true
    @AppStorage("exportFormat") private var format = TranscriptExportFormat.text
    @State private var scope = 0
    @State private var error: String?

    private var options: TranscriptExportOptions {
        .init(format: format, includesSpeakers: includesSpeakers, includesTimestamps: includesTimestamps,
              selectedUtteranceIDs: scope == 1 ? selectedIDs : scope == 2 ? visibleIDs : nil)
    }
    private var output: String { TranscriptRenderer.render(document, options: options) }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("대화 내보내기").font(GroveTypography.heading)
            Picker("범위", selection: $scope) {
                Text("전체 대화").tag(0)
                if !selectedIDs.isEmpty { Text("선택한 발화 (\(selectedIDs.count)개)").tag(1) }
                Text("현재 표시된 발화 (\(visibleIDs.count)개)").tag(2)
            }
            Picker("형식", selection: $format) {
                Text("텍스트 (.txt)").tag(TranscriptExportFormat.text)
                Text("Markdown (.md)").tag(TranscriptExportFormat.markdown)
            }
            Toggle("화자 이름 포함", isOn: $includesSpeakers)
            Toggle("시간 포함", isOn: $includesTimestamps)
            Text("수정한 내용과 현재 화자 이름으로 내보냅니다.").font(.caption).foregroundStyle(.secondary)
            ScrollView {
                Text(String(output.prefix(1600))).font(.callout).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(12)
            }.frame(height: 180).background(GroveTheme.canvas, in: RoundedRectangle(cornerRadius: 6))
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
            HStack {
                Button("취소") { dismiss() }.keyboardShortcut(.cancelAction)
                Spacer()
                Button("복사") {
                    NSPasteboard.general.clearContents()
                    if NSPasteboard.general.setString(output, forType: .string) { dismiss() }
                    else { error = "클립보드에 복사하지 못했습니다." }
                }.disabled(output.isEmpty)
                Button("파일로 저장…", action: saveFile)
                    .buttonStyle(.borderedProminent).disabled(output.isEmpty)
            }
        }.padding(24).frame(width: 500)
    }

    private func saveFile() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = format == .text ? [.plainText] : [UTType(filenameExtension: "md") ?? .plainText]
        let cleaned = title.components(separatedBy: CharacterSet(charactersIn: "/:\n\r")).joined(separator: " ")
        panel.nameFieldStringValue = (cleaned.isEmpty ? "대화" : cleaned) + "." + format.fileExtension
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try output.write(to: url, atomically: true, encoding: .utf8)
            dismiss()
        } catch { self.error = "파일을 저장하지 못했습니다. \(error.localizedDescription)" }
    }
}
