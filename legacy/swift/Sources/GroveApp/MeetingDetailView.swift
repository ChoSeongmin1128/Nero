import GroveInference
import SwiftUI

struct MeetingDetailView: View {
    @ObservedObject var store: GroveStore
    let meeting: MeetingRecord
    @State private var showsTranscriptionOptions = false
    @State private var speakerOptions = MeetingSpeakerOptions()
    @State private var reviewOnly = false
    @State private var showsSpeakers = false
    @State private var showsNotionExport = false
    private var isDual: Bool { meeting.audioPath == nil && meeting.systemAudioPath != nil && meeting.microphoneAudioPath != nil }
    private var document: TranscriptDocument? { store.transcriptDocuments[meeting.id] }
    private var retryPlan: MeetingInferencePlan? { try? speakerOptions.plan(isDual: isDual) }
    private var preparationMessage: String? {
        let sources = meeting.audioPath != nil ? ["recording"] : [("system", meeting.systemAudioPath), ("microphone", meeting.microphoneAudioPath)]
            .compactMap { $0.1 == nil ? nil : $0.0 }
        return retryPlan.flatMap { store.preparationMessage(for: $0, sourceIDs: sources) }
    }
    private var presentation: MeetingPresentationStatus { .init(meeting: meeting, document: document) }
    private var currentCounts: [MeetingSpeakerCount] {
        guard let result = meeting.completedResult, result.revisionID == document?.revisionID else { return [] }
        return result.speakerCounts
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            if meeting.captureMode == .systemAndMicrophone {
                Text("베타: 두 채널은 각각 전사합니다. 시각은 각 파일 기준이며 채널 간 화자는 자동 병합하지 않습니다.")
                    .font(.caption).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 28).padding(.bottom, 10)
            }
            if let detail = presentation.detail, document != nil || !meeting.transcript.isEmpty {
                Text(detail).font(GroveTypography.bodySmall).foregroundStyle(GroveTheme.ink)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 24).padding(.bottom, 16)
            }
            Divider()
            TranscriptView(store: store, meeting: meeting, reviewOnly: $reviewOnly, showsSpeakers: $showsSpeakers,
                           retryTranscription: presentTranscriptionOptions)
                .id(meeting.id)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .onChange(of: meeting.id) { _, _ in reviewOnly = false; showsSpeakers = false }
        .sheet(isPresented: $showsNotionExport) {
            if let document { NotionExportSheet(meeting: meeting, document: document, exporter: store.notionExporter, connection: store.notionConnection) }
        }
        .sheet(isPresented: $showsTranscriptionOptions) {
            VStack(alignment: .leading, spacing: 18) {
                Text("다시 전사").font(GroveTypography.title)
                Text("원본 녹음으로 새 전사를 만듭니다. 현재 수정 내용은 이전 전사에 보관됩니다.")
                    .foregroundStyle(.secondary)
                MeetingSpeakerOptionsView(options: $speakerOptions, isDual: isDual)
                if let preparationMessage { MeetingPreparationNotice(message: preparationMessage) { showsTranscriptionOptions = false } }
                HStack {
                    Spacer()
                    Button("취소") { showsTranscriptionOptions = false }.keyboardShortcut(.cancelAction)
                    Button("전사 시작") {
                        guard let plan = try? speakerOptions.plan(isDual: isDual) else { return }
                        showsTranscriptionOptions = false
                        Task { await store.transcribeMeeting(id: meeting.id, plan: plan) }
                    }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                        .disabled(store.isBusy || retryPlan == nil || preparationMessage != nil)
                }
            }.padding(24).frame(width: 440)
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .center, spacing: 14) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(meeting.title)
                        .font(GroveTypography.title)
                        .foregroundStyle(GroveTheme.ink)
                        .lineLimit(2)
                    Button { store.present(.renameRecording(meeting.id)) } label: {
                        Image(systemName: "pencil").font(.body)
                    }
                    .modifier(GroveActionAppearance())
                    .accessibilityLabel("녹음 이름 변경")
                    .help("녹음 이름 변경")
                }
                Spacer(minLength: 8)
            }
            HStack(spacing: 14) {
                Text(store.folderName(meeting.folderID))
                Label(presentation.label, systemImage: presentation.symbol)
                if let engines = store.transcriptDocuments[meeting.id]?.sourceDiarizationEngines, !engines.isEmpty {
                    Text(Set(engines.values.map(\.displayName)).sorted().joined(separator: ", "))
                }
                Text(meeting.startedAt, format: .dateTime.year().month().day().hour().minute())
                if meeting.duration > 0 { Text(meeting.duration.clockString).monospacedDigit() }
            }
            .font(GroveTypography.label)
            .foregroundStyle(.secondary)
            HStack(spacing: 10) {
                Button("원본 파일…") { store.present(.originalRecording(meeting.id)) }
                    .modifier(GroveActionAppearance())
                MeetingMoveMenu(store: store, meeting: meeting)
                    .modifier(GroveActionAppearance())
                Spacer(minLength: 12)
                processingControls
            }
            if let document {
                HStack(spacing: 12) {
                    if document.sourceDiarizationEngines?.values.allSatisfy({ $0 == .none }) != true {
                        Button("배정된 화자 \(Set(document.utterances.compactMap(\.speakerID)).count)명") { showsSpeakers = true }
                    } else { Text("화자 구분 없음").foregroundStyle(.secondary) }
                    ForEach(currentCounts.filter(\.isMismatch)) { count in
                        Button(count.label) { showsSpeakers = true }
                            .help("이번 전사의 입력 인원과 모델 감지 인원입니다. 화자 목록에서 확인할 수 있습니다.")
                    }
                    if document.speakerReviewCount > 0 {
                        Button("화자 확인 \(document.speakerReviewCount)곳") { reviewOnly = true }
                    }
                    Spacer()
                    Button("Notion 내보내기") { showsNotionExport = true }.disabled(store.isBusy)
                }
                .font(GroveTypography.label)
                .buttonStyle(.bordered)
                .tint(.primary)
            }
        }
        .padding(24)
        .background(GroveTheme.surface)
    }

    @ViewBuilder
    private var processingControls: some View {
        if store.processingMeetingID == meeting.id {
            VStack(alignment: .trailing, spacing: 7) {
                HStack {
                    ProgressView().controlSize(.small)
                    Text(store.processingStage ?? "처리 중").font(.caption)
                }
                Button("처리 중단") { store.cancelProcessing() }
                    .modifier(GroveActionAppearance())
            }
        } else if (meeting.audioPath != nil || meeting.systemAudioPath != nil || meeting.microphoneAudioPath != nil)
            && (document != nil || !presentation.canRetry || store.transcriptDocumentErrors[meeting.id] != nil) {
            Button(presentation.canRetry ? "전사 다시 시도…" : "다시 전사…") {
                presentTranscriptionOptions()
            }.modifier(GroveActionAppearance()).disabled(store.isBusy)
        }
    }

    private func presentTranscriptionOptions() {
        speakerOptions = .init(configuration: meeting.inferenceConfiguration, channels: meeting.channelInferenceConfigurations)
        showsTranscriptionOptions = true
    }
}

private extension DiarizationEngine {
    var displayName: String {
        switch self {
        case .none: "Mac 기본 전사"
        case .nemotron3: "Nemotron 3"
        case .community1: "Community-1"
        case .ultra8: "Ultra8"
        case .sortformerStreaming: "Sortformer"
        }
    }
}
