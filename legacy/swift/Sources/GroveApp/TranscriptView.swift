import AppKit
import SwiftUI

struct TranscriptView: View {
    @ObservedObject var store: GroveStore
    let meeting: MeetingRecord
    @Binding var reviewOnly: Bool
    @Binding var showsSpeakers: Bool
    var retryTranscription: (() -> Void)? = nil
    @Environment(\.displayScale) private var displayScale
    @Environment(\.colorSchemeContrast) private var colorSchemeContrast
    @StateObject private var player = AudioPlayerController()
    @State private var query = ""
    @State private var speakerFilter = "all"
    @State private var selectionMode = false
    @State private var selectedIDs: Set<UUID> = []
    @State private var editingUtterance: DocumentUtterance?
    @State private var splittingUtterance: DocumentUtterance?
    @State private var changingSpeaker: DocumentUtterance?
    @State private var renamingSpeaker: MeetingSpeaker?
    @State private var savingSpeaker: MeetingSpeaker?
    @State private var enrollingSpeaker: MeetingSpeaker?
    @State private var showsExport = false
    @State private var showsHistory = false
    @State private var notice: String?
    @AppStorage("transcriptTextScale") private var textScale = 1.0
    @AppStorage("exportIncludesSpeakers") private var includesSpeakers = true
    @AppStorage("exportIncludesTimestamps") private var includesTimestamps = true
    @AppStorage("exportFormat") private var exportFormat = TranscriptExportFormat.text

    private var document: TranscriptDocument? { store.transcriptDocuments[meeting.id] }
    private var visibleRows: [TranscriptDisplayRow] {
        guard let document else { return [] }
        let reviewIDs = reviewOnly ? Set(document.utterances.filter { document.speakerReview(for: $0).needsReview }.map(\.id)) : nil
        return TranscriptPresentation.rows(in: document, query: query, speakerFilter: speakerFilter, onlyUtteranceIDs: reviewIDs)
    }
    private var visibleUtterances: [DocumentUtterance] {
        visibleRows.map(\.utterance)
    }

    var body: some View {
        Group {
            if let error = store.transcriptDocumentErrors[meeting.id] {
                ContentUnavailableView("전사 문서를 열지 못했습니다", systemImage: "exclamationmark.triangle",
                                       description: Text("기존 파일은 보존되어 있습니다. \(error)"))
            } else if let document {
                VStack(spacing: 0) {
                    toolbar(document)
                    Divider()
                    HStack(spacing: 0) {
                        transcript(document)
                        if showsSpeakers {
                            Divider()
                            speakerList(document).frame(width: 220)
                        }
                    }
                    Divider()
                    playbackBar
                }
                .background(GroveTheme.surface)
            } else {
                ContentUnavailableView {
                    Label(emptyTitle, systemImage: emptySymbol)
                        .foregroundStyle(GroveTheme.ink)
                } description: {
                    Text(meeting.errorMessage ?? emptyDescription)
                        .font(GroveTypography.body)
                        .foregroundStyle(GroveTheme.ink)
                } actions: {
                    if let retryTranscription, MeetingPresentationStatus(meeting: meeting, document: nil).canRetry {
                        Button("전사 다시 시도…", action: retryTranscription)
                            .modifier(GroveActionAppearance())
                            .disabled(store.isBusy)
                    }
                }
                .frame(maxWidth: 600)
                .padding(.horizontal, 24)
                .padding(.top, 48)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task(id: MeetingPlaybackSource(meeting: meeting)) {
            player.prepare(meeting: meeting)
        }
        .task {
            await store.voice.refreshEnrollmentStatus()
        }
        .onDisappear { player.stop() }
        .onChange(of: document?.revisionID) { _, _ in
            selectedIDs.removeAll()
            speakerFilter = "all"
            player.pause()
        }
        .onChange(of: reviewOnly) { _, enabled in
            if enabled {
                query = ""
                speakerFilter = "all"
                selectionMode = false
                selectedIDs.removeAll()
            }
        }
        .sheet(item: $editingUtterance) { utterance in
            UtteranceTextEditor(utterance: utterance) { text in
                store.transcripts.updateUtteranceText(meetingID: meeting.id, utteranceID: utterance.id, text: text)
            }
        }
        .sheet(item: $changingSpeaker) { utterance in
            if let document {
                SpeakerAssignmentEditor(document: document, utterance: utterance, selectedIDs: selectedIDs) { target, scope, confirming in
                    store.transcripts.reassignSpeaker(meetingID: meeting.id, utteranceID: utterance.id, target: target, scope: scope,
                                          confirmingAnchor: confirming)
                }
            }
        }
        .sheet(item: $splittingUtterance) { utterance in
            UtteranceSplitEditor(meeting: meeting, utterance: utterance, player: player) { time, first, second in
                let saved = store.transcripts.splitUtterance(meetingID: meeting.id, utteranceID: utterance.id,
                                                time: time, firstText: first, secondText: second)
                if saved { selectedIDs.remove(utterance.id) }
                return saved
            }
        }
        .sheet(item: $renamingSpeaker) { speaker in
            SpeakerNameEditor(speaker: speaker) { name in
                store.transcripts.renameSpeaker(meetingID: meeting.id, speakerID: speaker.id, name: name)
            }
        }
        .sheet(item: $savingSpeaker) { speaker in
            SaveSpeakerProfileSheet(store: store, meeting: meeting, speaker: speaker)
        }
        .sheet(item: $enrollingSpeaker) { speaker in
            VoiceEnrollmentSheet(store: store, meeting: meeting, speaker: speaker, player: player)
        }
        .sheet(isPresented: $showsExport) {
            if let document {
                TranscriptExportSheet(document: document, title: meeting.title, selectedIDs: selectedIDs,
                                      visibleIDs: Set(visibleUtterances.map(\.id)))
            }
        }
        .sheet(isPresented: $showsHistory) {
            TranscriptHistorySheet(store: store, meetingID: meeting.id)
        }
    }

    private func toolbar(_ document: TranscriptDocument) -> some View {
        VStack(spacing: 10) {
            if reviewOnly {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Text("화자 확인 \(document.speakerReviewCount)곳")
                        .font(GroveTypography.label).foregroundStyle(GroveTheme.evidence)
                    Text("화자 배정만 확인합니다. 텍스트 검수와는 별개입니다.")
                        .font(.caption).foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                    Button("전체 대화 보기") { reviewOnly = false }
                        .buttonStyle(.plain).font(.caption)
                }
            }
            HStack(spacing: 12) {
                TextField("대화 검색", text: $query)
                    .textFieldStyle(.roundedBorder)
                    .frame(minWidth: 110, maxWidth: 240)
                Picker("화자", selection: $speakerFilter) {
                    Text("모든 화자").tag("all")
                    ForEach(document.speakers.sorted { $0.order < $1.order }) { speaker in
                        Text(speaker.name).tag(speaker.id.uuidString)
                    }
                    if document.utterances.contains(where: { $0.speakerID == nil }) {
                        Text("화자 미확정").tag("unassigned")
                    }
                }
                .labelsHidden().frame(maxWidth: 150)
                Spacer(minLength: 0)
                Button { showsSpeakers.toggle() } label: { Label("화자", systemImage: "person.2") }
                    .help("화자 목록과 이름 편집")
                Menu {
                    Button("전체 대화 복사") { copy(document, ids: nil) }
                    Button("선택한 발화 복사 (\(selectedIDs.count)개)") { copy(document, ids: selectedIDs) }
                        .disabled(selectedIDs.isEmpty)
                    Button("현재 표시된 발화 복사 (\(visibleUtterances.count)개)") {
                        copy(document, ids: Set(visibleUtterances.map(\.id)))
                    }.disabled(visibleUtterances.isEmpty)
                    Divider()
                    Button("파일로 저장 및 형식 설정…") { showsExport = true }
                    Divider()
                    Button("이전 전사 보기…") { showsHistory = true }
                } label: { Label("복사·저장", systemImage: "square.and.arrow.up") }
            }
            HStack(spacing: 12) {
                Button(selectionMode ? "선택 완료" : "발화 선택") {
                    selectionMode.toggle()
                    if !selectionMode { selectedIDs.removeAll() }
                }
                .buttonStyle(.plain)
                if selectionMode {
                    Button("표시된 발화 선택") { selectedIDs.formUnion(visibleUtterances.map(\.id)) }
                        .buttonStyle(.plain).disabled(visibleUtterances.isEmpty)
                    Button("선택 해제") { selectedIDs.removeAll() }.buttonStyle(.plain)
                    Text("\(selectedIDs.count)개 선택").foregroundStyle(.secondary)
                } else {
                    Text("\(visibleUtterances.count)개 발화").foregroundStyle(.secondary)
                }
                Spacer()
                Menu {
                    ForEach([1.0, 1.25, 1.5, 2.0], id: \.self) { scale in
                        Button("\(Int(scale * 100))%") { textScale = scale }
                    }
                } label: { Image(systemName: "textformat.size") }
                .menuStyle(.borderlessButton).fixedSize().help("본문 글자 크기")
                if let notice { Text(notice).foregroundStyle(.secondary).accessibilityAddTraits(.updatesFrequently) }
                Button { store.transcripts.undo(meetingID: meeting.id) } label: {
                    Image(systemName: "arrow.uturn.backward")
                }
                .help(document.undoHistory.last.map { "되돌리기: \($0.label)" } ?? "되돌리기")
                .accessibilityLabel("변경 되돌리기")
                .disabled(document.undoHistory.isEmpty)
                Button { store.transcripts.redo(meetingID: meeting.id) } label: {
                    Image(systemName: "arrow.uturn.forward")
                }
                .help("다시 실행").accessibilityLabel("변경 다시 실행")
                .disabled(document.redoHistory.isEmpty)
            }
            .font(.caption)
        }
        .padding(.horizontal, 24).padding(.vertical, 12)
        .tint(.primary)
    }

    private func transcript(_ document: TranscriptDocument) -> some View {
        let rows = visibleRows
        let metadataWidth: CGFloat = rows.contains { $0.utterance.endTime >= 3600 } ? 148 : 112
        return ScrollViewReader { proxy in
            ScrollView {
                if reviewOnly && document.speakerReviewCount == 0 {
                    ContentUnavailableView {
                        Label("남은 화자 확인 항목이 없습니다", systemImage: "checkmark.circle")
                    } actions: {
                        Button("전체 대화 보기") { reviewOnly = false }
                    }
                } else if rows.isEmpty {
                    ContentUnavailableView("일치하는 발화가 없습니다", systemImage: "magnifyingglass",
                                           description: Text("검색어 또는 화자 필터를 변경해 주세요."))
                } else {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(rows) { row in
                            utteranceRow(row, document: document, metadataWidth: metadataWidth)
                                .padding(.top, row.id == rows.first?.id || row.continuesPrevious ? 0 : 8)
                                .overlay(alignment: .top) {
                                    if row.id != rows.first?.id {
                                        Rectangle()
                                            .fill(GroveTheme.ink.opacity(separatorOpacity(for: row)))
                                            .frame(height: 1 / max(1, displayScale))
                                            .padding(.horizontal, 10)
                                            .offset(y: row.continuesPrevious ? 0 : 4)
                                            .allowsHitTesting(false)
                                            .accessibilityHidden(true)
                                    }
                                }
                                .id(row.id)
                        }
                    }
                    .frame(maxWidth: 980, alignment: .leading)
                    .padding(.horizontal, 20).padding(.vertical, 16)
                    .frame(maxWidth: .infinity)
                }
            }
            .onChange(of: reviewOnly) { _, _ in
                if let id = rows.first?.id { proxy.scrollTo(id, anchor: .top) }
            }
        }
    }

    private func separatorOpacity(for row: TranscriptDisplayRow) -> Double {
        if colorSchemeContrast == .increased { return row.continuesPrevious ? 0.18 : 0.28 }
        return row.continuesPrevious ? 0.07 : 0.14
    }

    private func utteranceRow(_ row: TranscriptDisplayRow, document: TranscriptDocument, metadataWidth: CGFloat) -> some View {
        let utterance = row.utterance
        let isPlaying = player.playingSegmentID == utterance.id
        let isSelected = selectedIDs.contains(utterance.id)
        let review = document.speakerReview(for: utterance)
        return HStack(alignment: .top, spacing: 12) {
            if selectionMode {
                Toggle("발화 선택", isOn: Binding(
                    get: { selectedIDs.contains(utterance.id) },
                    set: { if $0 { selectedIDs.insert(utterance.id) } else { selectedIDs.remove(utterance.id) } }
                )).labelsHidden().toggleStyle(.checkbox).padding(.top, 3)
            }
            VStack(alignment: .leading, spacing: 5) {
                Button { changingSpeaker = utterance } label: {
                    HStack(spacing: 6) {
                        Circle().fill(speakerColor(utterance.speakerID, document: document)).frame(width: 6, height: 6)
                        Text(document.speakerName(for: utterance))
                            .font(row.continuesPrevious ? GroveTypography.bodySmall : GroveTypography.label)
                            .foregroundStyle(row.continuesPrevious ? Color.secondary : GroveTheme.ink)
                            .lineLimit(1).truncationMode(.tail)
                        Image(systemName: "chevron.down")
                            .font(.system(size: 8, weight: .medium)).foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, minHeight: 18, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("\(document.speakerName(for: utterance)) · 이 발화의 화자 변경")
                .accessibilityLabel("\(document.speakerName(for: utterance)), 화자 변경")
                Button { player.play(meeting: meeting, utterance: utterance) } label: {
                    Text(TranscriptPresentation.timeRange(utterance))
                        .font(.system(size: 11)).monospacedDigit()
                        .foregroundStyle(isPlaying ? GroveTheme.grove : Color.secondary)
                        .fixedSize(horizontal: true, vertical: false)
                        .frame(maxWidth: .infinity, minHeight: 18, alignment: .leading)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("시작–종료 시간 · 클릭하면 이 발화의 앞뒤 문맥을 함께 듣습니다")
                .accessibilityLabel("시작 \(TranscriptPresentation.timestamp(utterance.startTime)), 종료 \(TranscriptPresentation.timestamp(utterance.endTime)), 발화 듣기")
                if utterance.sourceChannelID == "system" || utterance.sourceChannelID == "microphone" {
                    Text(utterance.sourceChannelID == "system" ? "컴퓨터 소리" : "마이크")
                        .font(.caption2).foregroundStyle(.secondary)
                }
            }
            .frame(width: metadataWidth, alignment: .leading)
            VStack(alignment: .leading, spacing: 4) {
                Text(utterance.displayedText)
                    .font(.custom("Pretendard-Regular", size: 15 * min(2, max(1, textScale)))).foregroundStyle(GroveTheme.ink)
                    .lineSpacing(3).textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if utterance.parentUtteranceID != nil {
                    Text("분할됨").font(.caption2).foregroundStyle(.secondary)
                } else if utterance.editedText != nil {
                    Text("수정됨").font(.caption2).foregroundStyle(.secondary)
                }
                if review.needsReview { speakerReviewActions(utterance, review: review) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Menu {
                Button("발화 내용 수정…") { editingUtterance = utterance }
                Button("발화 나누기…") { splittingUtterance = utterance }
                    .disabled(utterance.endTime <= utterance.startTime)
                Button("화자 변경…") { changingSpeaker = utterance }
                Button("화자와 함께 복사") { copy(document, ids: [utterance.id], forcesSpeakers: true) }
                if utterance.editedText != nil && utterance.parentUtteranceID == nil {
                    Button("내용을 원문으로 되돌리기") {
                        _ = store.transcripts.updateUtteranceText(meetingID: meeting.id, utteranceID: utterance.id, text: utterance.rawText)
                    }
                }
            } label: {
                Image(systemName: "ellipsis")
                    .foregroundStyle(.secondary).frame(width: 24, height: 20)
            }
            .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
            .accessibilityLabel("발화 메뉴").help("발화 수정·나누기·복사")
        }
        .padding(.horizontal, 10).padding(.vertical, 7)
        .background(isPlaying ? GroveTheme.grove.opacity(0.08) : (isSelected ? GroveTheme.revision.opacity(0.07) : .clear),
                    in: RoundedRectangle(cornerRadius: 6))
    }

    private func speakerReviewActions(_ utterance: DocumentUtterance, review: SpeakerReviewAssessment) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(review.reasons.map(\.message).joined(separator: " "))
                .font(.caption).foregroundStyle(GroveTheme.evidence)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 12) {
                Button("듣기") { player.play(meeting: meeting, utterance: utterance) }
                if review.canConfirm {
                    Button("현재 화자가 맞음") {
                        if store.transcripts.confirmUtteranceSpeaker(meetingID: meeting.id, utteranceID: utterance.id) {
                            notice = "화자 배정을 확인했습니다"
                        }
                    }
                }
                Button("화자 변경…") { changingSpeaker = utterance }
            }
            .buttonStyle(.plain).font(.caption)
        }
        .padding(.top, 4)
    }

    private func speakerList(_ document: TranscriptDocument) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Text("화자").font(GroveTypography.heading)
                Text("이름을 바꾸면 해당 화자의 모든 발화에 적용됩니다.")
                    .font(.caption).foregroundStyle(.secondary)
                if let folderID = meeting.folderID,
                   store.voice.isAvailable,
                   store.speakerProfiles(in: folderID).contains(where: { store.voice.isRegistered($0.id) }) {
                    VStack(alignment: .leading, spacing: 7) {
                        if store.voice.isRecognizingVoices {
                            HStack(spacing: 8) {
                                ProgressView().controlSize(.small)
                                Text("등록한 목소리 확인 중").font(.caption)
                            }
                            Button("식별 취소") { store.voice.cancelWork() }.buttonStyle(.plain).font(.caption)
                        } else {
                            Button("목소리로 이름 찾기") {
                                player.pause()
                                Task { await store.voice.identifySpeakers(meetingID: meeting.id) }
                            }.disabled(store.isBusy || !store.voice.isAvailable)
                        }
                        if let message = store.voice.message(for: meeting.id) {
                            Text(message).font(.caption).foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                }
                ForEach(document.speakers.sorted { $0.order < $1.order }) { speaker in
                    let utterances = document.utterances.filter { $0.speakerID == speaker.id }
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            Circle().fill(speakerColor(speaker.id, document: document)).frame(width: 7, height: 7)
                            Text(speaker.name).font(GroveTypography.label)
                            if utterances.isEmpty { Text("미사용").font(.caption2).foregroundStyle(.secondary) }
                            Spacer()
                            Button { renamingSpeaker = speaker } label: { Image(systemName: "pencil") }
                                .buttonStyle(.plain).help("\(speaker.name) 이름 변경")
                        }
                        HStack {
                            Button("\(utterances.count)개 발화 보기") {
                                reviewOnly = false
                                query = ""
                                speakerFilter = speaker.id.uuidString
                            }
                                .buttonStyle(.plain)
                            Spacer()
                            if let sample = utterances.first {
                                Button { player.play(meeting: meeting, utterance: sample) } label: { Image(systemName: "play.circle") }
                                    .buttonStyle(.plain).help("목소리 듣기")
                            }
                        }.font(.caption).foregroundStyle(.secondary)
                        if let folderID = meeting.folderID {
                            if speaker.profileMatch?.isConfirmed == false {
                                Text("저장한 목소리로 추정한 이름입니다. 확인하거나 다른 화자로 바꿔 주세요.")
                                    .font(.caption).foregroundStyle(GroveTheme.evidence)
                                HStack(spacing: 12) {
                                    Button("맞음") {
                                        _ = store.voice.confirmIdentity(meetingID: meeting.id, speakerID: speaker.id)
                                    }
                                    .accessibilityLabel("\(speaker.name) 이름 제안 확인")
                                    Button("다른 사람") {
                                        _ = store.voice.rejectIdentity(meetingID: meeting.id, speakerID: speaker.id)
                                    }
                                    .accessibilityLabel("\(speaker.name) 이름 제안 거절")
                                }
                                .buttonStyle(.plain).font(.caption).disabled(store.isBusy)
                            }
                            Menu("폴더의 화자") {
                                ForEach(store.speakerProfiles(in: folderID)) { profile in
                                    Button("\(profile.name)로 확인") {
                                        _ = store.voice.applySavedSpeaker(profileID: profile.id, meetingID: meeting.id, speakerID: speaker.id)
                                    }
                                }
                                Divider()
                                Button("이 화자 이름 저장…") { savingSpeaker = speaker }
                                    .disabled(store.speakerProfiles(in: folderID).contains { $0.id == speaker.profileMatch?.profileID })
                                if store.voice.isAvailable {
                                    Button("목소리 등록…") {
                                        player.pause()
                                        enrollingSpeaker = speaker
                                    }.disabled(utterances.isEmpty)
                                }
                            }.disabled(store.isBusy)
                        }
                    }
                }
            }.padding(20)
        }.background(GroveTheme.canvas.opacity(0.6))
    }

    private var playbackBar: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let error = player.errorMessage { Text(error).font(.caption).foregroundStyle(.secondary) }
            if meeting.systemAudioPath != nil && meeting.microphoneAudioPath != nil {
                Picker("재생할 녹음", selection: Binding(
                    get: { player.sourceChannel ?? "system" },
                    set: { player.prepare(meeting: meeting, source: $0) }
                )) {
                    Text("컴퓨터 소리").tag("system")
                    Text("마이크").tag("microphone")
                }.pickerStyle(.segmented).frame(width: 210)
            }
            HStack(spacing: 14) {
                Button { player.toggle() } label: {
                    Image(systemName: player.isPlaying ? "pause.fill" : "play.fill").frame(width: 20, height: 20)
                }
                .buttonStyle(.plain).disabled(player.duration == 0)
                .accessibilityLabel(player.isPlaying ? "일시 정지" : "전체 재생")
                Text(player.position.clockString).monospacedDigit().font(.caption)
                Slider(value: Binding(get: { player.position }, set: { player.seek(to: $0) }), in: 0...max(1, player.duration))
                    .disabled(player.duration == 0).accessibilityLabel("재생 위치")
                Text(player.duration.clockString).monospacedDigit().font(.caption).foregroundStyle(.secondary)
                Menu {
                    ForEach([0.75, 1.0, 1.25, 1.5, 2.0], id: \.self) { speed in
                        Button("\(speed.formatted())×") { player.setRate(Float(speed)) }
                    }
                } label: { Text("\(Double(player.rate).formatted())×").monospacedDigit() }
                .frame(width: 65).help("재생 속도")
            }
        }.padding(.horizontal, 24).padding(.vertical, 16)
    }

    private func copy(_ document: TranscriptDocument, ids: Set<UUID>?, forcesSpeakers: Bool = false) {
        let text = TranscriptRenderer.render(document, options: .init(format: exportFormat,
            includesSpeakers: forcesSpeakers || includesSpeakers, includesTimestamps: includesTimestamps, selectedUtteranceIDs: ids))
        guard !text.isEmpty else { return }
        NSPasteboard.general.clearContents()
        if NSPasteboard.general.setString(text, forType: .string) { notice = "복사했습니다" }
    }

    private func speakerColor(_ id: UUID?, document: TranscriptDocument) -> Color {
        guard let speaker = document.speakers.first(where: { $0.id == id }) else { return .secondary }
        let colors: [Color] = [GroveTheme.grove, .blue, .orange, .purple, .teal, .pink, .indigo, .brown]
        return colors[abs(speaker.order % colors.count)]
    }

    private var emptyTitle: String {
        if meeting.processingOutcome?.kind == .cancelled { return "전사 중단" }
        if meeting.processingOutcome?.kind == .interrupted { return "처리 중단" }
        switch meeting.status {
        case .recording: return "녹음 중"
        case .processing: return "전사 중"
        case .failed: return "전사 실패"
        default: return "전사문이 없습니다"
        }
    }

    private var emptySymbol: String {
        if meeting.processingOutcome?.kind == .cancelled || meeting.processingOutcome?.kind == .interrupted { return "pause.circle" }
        return meeting.status == .failed ? "exclamationmark.triangle" : "waveform"
    }

    private var emptyDescription: String {
        switch meeting.status {
        case .recording: "회의를 종료하면 전사를 시작합니다."
        case .processing: ""
        default: "음성 파일을 가져오거나 새 회의를 녹음해 주세요."
        }
    }
}
