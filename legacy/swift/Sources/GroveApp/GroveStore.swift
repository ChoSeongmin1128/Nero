import Foundation
import Combine
import GroveInference

@MainActor
final class GroveStore: ObservableObject {
    @Published var meetings: [MeetingRecord] = []
    @Published var selection: SidebarDestination? = .library
    @Published var workspaceSheet: WorkspaceSheet?
    @Published var isPresentingImporter = false
    @Published var alertMessage: String?
    @Published private(set) var exportingOriginalMeetingID: UUID?
    @Published private(set) var isPreparingToQuit = false
    @Published var activeMeetingID: UUID?
    @Published var processingStage: String?
    @Published private(set) var processingMeetingID: UUID?
    @Published private var captureStartID: UUID?
    @Published private(set) var isImportingRecording = false
    @Published private(set) var folderMoveFeedback: MeetingFolderMoveFeedback?
    @Published private var pendingWorkspaceStart: WorkspaceStartAction?
    @Published private var packagingCaptures = 0

    let recorder: AudioRecorder
    let calendarSchedule: CalendarSchedule
    let modelManager: ModelManager
    let voiceEnrollmentSession: VoiceEnrollmentSession
    let notionExporter: NotionExporter
    let notionConnection: NotionConnection
    let applePreparation = AppleTranscriptionPreparation()
    let transcripts: TranscriptDocumentStore
    let libraryStore: LibraryStore
    let voice: VoiceIdentityCoordinator

    private let storageURL: URL
    private let audioDirectory: URL
    private let isDemoMode: Bool
    private var meetingIndexError: String?
    private let inferenceService: any MeetingInferenceRunning
    private var processingTask: Task<Void, Never>?
    private var preparationTask: (id: UUID, task: Task<Void, Error>)?
    private let folderTransferScopeID = UUID()
    private var subscriptions: Set<AnyCancellable> = []

    init(baseDirectory: URL? = nil, inferenceService: (any MeetingInferenceRunning)? = nil,
         recorder: AudioRecorder? = nil,
         voiceVault: SpeakerVoiceVault? = nil, voiceExtractor: (any SpeakerVoiceExtracting)? = nil,
         voiceEnrollmentSession: VoiceEnrollmentSession? = nil,
         voiceIdentificationAvailable: Bool = VoiceIdentityReleaseGate.isEnabled) {
        self.recorder = recorder ?? AudioRecorder()
        let base = baseDirectory ?? FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first!.appendingPathComponent("Grove", isDirectory: true)
        isDemoMode = CommandLine.arguments.contains("--demo")
        storageURL = base.appendingPathComponent("meetings.json")
        calendarSchedule = CalendarSchedule(defaults: baseDirectory == nil ? .standard : UserDefaults(suiteName: "Grove.Tests.\(UUID().uuidString)")!)
        let models = ModelManager(baseDirectory: base)
        modelManager = models
        let session = voiceEnrollmentSession ?? VoiceEnrollmentSession(directory: base.appendingPathComponent("VoiceWork/Capture", isDirectory: true))
        self.voiceEnrollmentSession = session
        let notionDiagnostics = NotionDiagnostics(file: base.appendingPathComponent("Diagnostics/notion-requests.json"))
        notionExporter = NotionExporter(directory: base.appendingPathComponent("Exports/Notion"), diagnostics: notionDiagnostics)
        notionConnection = baseDirectory == nil ? NotionConnection(diagnostics: notionDiagnostics)
            : NotionConnection(storage: MemoryNotionConnectionStore(), defaults: UserDefaults(suiteName: "Grove.Notion.QA.\(UUID().uuidString)")!, diagnostics: notionDiagnostics)
        audioDirectory = base.appendingPathComponent("Audio", isDirectory: true)
        transcripts = TranscriptDocumentStore(directory: base.appendingPathComponent("Documents", isDirectory: true), persists: !isDemoMode)
        libraryStore = LibraryStore(url: base.appendingPathComponent("library.json"), persists: !isDemoMode)
        self.inferenceService = inferenceService ?? BundledMeetingInferenceService(
            appBundle: Bundle.main.bundleURL, applicationSupport: base, modelsReady: { models.isReadyForUse })
        voice = VoiceIdentityCoordinator(library: libraryStore, transcripts: transcripts,
            vault: voiceVault ?? SpeakerVoiceVault(directory: base.appendingPathComponent("VoiceRegistry", isDirectory: true),
                                                   keyStore: KeychainSpeakerVoiceKeyStore(storage: .login)),
            extractor: voiceExtractor ?? LocalSpeakerVoiceService(modelDirectory: base.appendingPathComponent("Models/VoiceIdentity")),
            enrollmentSession: session, workDirectory: base.appendingPathComponent("VoiceWork", isDirectory: true),
            persists: !isDemoMode, isAvailable: voiceIdentificationAvailable)

        try? FileManager.default.createDirectory(at: audioDirectory, withIntermediateDirectories: true)
        if let error = libraryStore.loadError {
            alertMessage = "폴더와 설정을 읽지 못했습니다. 기존 파일은 보존됩니다. \(error)"
        }
        if isDemoMode {
            meetings = [Self.demoMeeting]
            selection = .meeting(Self.demoMeeting.id)
        } else {
            loadMeetings()
            if meetingIndexError == nil {
                recoverInterruptedCaptures()
                recoverInterruptedProcessing()
            }
        }
        transcripts.load(meetings)
        wireComponents()
        Task { [weak self] in
            await self?.voice.refreshEnrollmentStatus()
            await self?.packageInterruptedRecordings()
        }
    }

    private func wireComponents() {
        transcripts.onError = { [weak self] in self?.alertMessage = $0 }
        libraryStore.onError = { [weak self] in self?.alertMessage = $0 }
        libraryStore.canEditPeople = { [weak self] in self?.isBusy == false }
        voice.onError = { [weak self] in self?.alertMessage = $0 }
        voice.meeting = { [weak self] id in self?.meetings.first { $0.id == id } }
        voice.isBusy = { [weak self] in self?.isBusy ?? true }
        voice.mayIdentify = { [weak self] in
            guard let self else { return false }
            return !self.isRecording && !self.isPreparingToQuit
        }
        recorder.onUnexpectedStop = { [weak self] duration in self?.recordingStoppedUnexpectedly(duration: duration) }
        calendarSchedule.canRecord = { [weak self] in self?.canPresentNewMeeting == true }
        calendarSchedule.onRecord = { [weak self] event in self?.present(.newMeeting(event)) }
        // Views that show levels or download progress observe those objects directly; the
        // workspace only needs the state transitions that change what can be started.
        forward(recorder.$isRecording)
        forward(recorder.$isPaused)
        forward(modelManager.$isInstalling)
        forward(modelManager.$isReadyForUse)
        forward(modelManager.$isVoiceModelReady)
        forward(modelManager.$readyGroups)
        forward(voiceEnrollmentSession.$isStarting)
        forward(voiceEnrollmentSession.$isAnalyzing)
        forward(voiceEnrollmentSession.recorder.$isRecording)
        forward(applePreparation.$isPreparing)
        forward(applePreparation.$isReady)
        forward(notionConnection.$isConnecting)
        forward(notionExporter.$isSaving)
        for publisher in [transcripts.objectWillChange, libraryStore.objectWillChange, voice.objectWillChange] {
            publisher.sink { [weak self] in self?.objectWillChange.send() }.store(in: &subscriptions)
        }
    }

    private func forward<Value: Equatable>(_ publisher: Published<Value>.Publisher) {
        publisher.removeDuplicates().dropFirst().sink { [weak self] _ in self?.objectWillChange.send() }.store(in: &subscriptions)
    }

    var transcriptDocuments: [UUID: TranscriptDocument] { transcripts.documents }
    var transcriptDocumentErrors: [UUID: String] { transcripts.errors }
    var library: MeetingLibrary { libraryStore.library }

    var activeMeeting: MeetingRecord? {
        guard let activeMeetingID else { return nil }
        return meetings.first { $0.id == activeMeetingID }
    }

    var isRecording: Bool { recorder.isRecording }
    var isProcessing: Bool { processingMeetingID != nil }
    var isStartingCapture: Bool { captureStartID != nil }
    var isExportingOriginal: Bool { exportingOriginalMeetingID != nil }
    var isBusy: Bool {
        notionConnection.isConnecting || pendingWorkspaceStart != nil || isRecording || isProcessing || isStartingCapture
            || isImportingRecording || packagingCaptures > 0 || voice.isWorking || isPreparingToQuit || modelManager.isInstalling
            || applePreparation.isPreparing || voiceEnrollmentSession.isActive
    }
    var needsModelSetup: Bool { (try? defaultSpeakerOptions.plan(isDual: false)).map { preparationMessage(for: $0) != nil } ?? true }
    var canPresentNewMeeting: Bool { !isBusy && !needsModelSetup && workspaceSheet == nil && !isPresentingImporter }

    func preparationMessage(for plan: MeetingInferencePlan, sourceIDs: [String] = ["recording"]) -> String? {
        do {
            let configurations = try plan.configurations(for: sourceIDs)
            for source in sourceIDs {
                if let configuration = configurations[source],
                   let issue = inferenceService.preparationIssue(configuration: configuration, appleReady: applePreparation.isReady) {
                    return issue
                }
            }
            return nil
        } catch { return error.localizedDescription }
    }

    private func validatePreparation(_ configurations: [String: InferenceConfiguration]) async throws {
        let id = UUID()
        let service = inferenceService
        let task = Task {
            for source in configurations.keys.sorted() {
                try Task.checkCancellation()
                if let configuration = configurations[source] { try await service.validatePreparation(configuration: configuration) }
                try Task.checkCancellation()
            }
        }
        preparationTask = (id, task)
        defer { if preparationTask?.id == id { preparationTask = nil } }
        try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
    }

    func present(_ sheet: WorkspaceSheet) {
        guard workspaceSheet == nil, pendingWorkspaceStart == nil, !isPreparingToQuit else { return }
        switch sheet {
        case .newMeeting, .importRecording: guard !isBusy else { return }
        case .renameRecording(let id), .originalRecording(let id):
            guard meetings.contains(where: { $0.id == id }) else { return }
        }
        workspaceSheet = sheet
    }

    func requestFileImport() {
        guard canPresentNewMeeting else { return }
        isPresentingImporter = true
    }

    func cancelRecordingStart() {
        if captureStartID != nil { preparationTask?.task.cancel() }
        captureStartID = nil
    }

    func startAfterDismissingSheet(_ action: WorkspaceStartAction) {
        guard workspaceSheet != nil, pendingWorkspaceStart == nil, !isBusy else { return }
        pendingWorkspaceStart = action
        workspaceSheet = nil
    }

    func workspaceSheetDidDismiss() {
        cancelRecordingStart()
        guard let action = pendingWorkspaceStart else { return }
        Task {
            pendingWorkspaceStart = nil
            switch action {
            case .recording(let title, let plan, let folderID, let event, let attendance):
                await beginRecording(title: title, plan: plan, folderID: folderID, calendarEvent: event, attendance: attendance)
            case .importing(let source, let plan, let folderID):
                await importRecording(from: source, plan: plan, folderID: folderID)
            }
        }
    }

    var defaultSpeakerOptions: MeetingSpeakerOptions {
        get { libraryStore.defaultSpeakerOptions }
        set { libraryStore.defaultSpeakerOptions = newValue }
    }

    var selectedFolderID: UUID? {
        let candidate: UUID?
        switch selection {
        case .folder(let id): candidate = id
        case .meeting(let id): candidate = meetings.first(where: { $0.id == id })?.folderID
        default: candidate = nil
        }
        return libraryStore.containsFolder(candidate) ? candidate : nil
    }

    func folderName(_ id: UUID?) -> String { libraryStore.folderName(id) }
    func speakerProfiles(in folderID: UUID) -> [SavedSpeakerProfile] { libraryStore.speakerProfiles(in: folderID) }
    func recentAttendance(in folderID: UUID?) -> MeetingAttendance { libraryStore.recentAttendance(in: folderID) }

    func recordings(in folderID: UUID?) -> [MeetingRecord] {
        MeetingFolderListing.recordings(meetings, in: folderID, folders: library.folders)
    }

    func folderTransfer(for meetingID: UUID) -> MeetingFolderTransfer {
        MeetingFolderTransfer(meetingID: meetingID, scopeID: folderTransferScopeID)
    }

    func canMoveMeeting(id: UUID) -> Bool {
        guard meetingIndexError == nil, libraryStore.loadError == nil, !isPreparingToQuit,
              !isStartingCapture, !voice.isWorking,
              let meeting = meetings.first(where: { $0.id == id }) else { return false }
        return id != activeMeetingID && id != processingMeetingID && id != exportingOriginalMeetingID
            && meeting.status != .recording && meeting.status != .processing
    }

    @discardableResult
    func acceptFolderDrop(_ items: [MeetingFolderTransfer], to folderID: UUID?) -> Bool {
        do {
            return moveMeetings(ids: try MeetingFolderTransfer.recordingIDs(in: items, scopeID: folderTransferScopeID), to: folderID)
        } catch {
            folderMoveFeedback = nil
            alertMessage = error.localizedDescription
            return false
        }
    }

    func dismissFolderMoveFeedback(id: UUID) {
        if folderMoveFeedback?.id == id { folderMoveFeedback = nil }
    }

    @discardableResult
    func createFolder(name: String) -> UUID? {
        guard let folder = libraryStore.createFolder(name: name) else { return nil }
        selection = .folder(folder.id)
        return folder.id
    }

    @discardableResult
    func renameMeeting(id: UUID, title: String) -> Bool {
        guard canModifyMeetingIndex else { return false }
        guard let cleaned = LibraryStore.singleLineName(title) else {
            alertMessage = "녹음 이름을 한 줄로 입력해 주세요."
            return false
        }
        guard let index = meetings.firstIndex(where: { $0.id == id }) else {
            alertMessage = "이름을 변경할 녹음을 찾을 수 없습니다."
            return false
        }
        guard meetings[index].title != cleaned else { return true }
        var updated = meetings
        updated[index].title = cleaned
        do {
            if !isDemoMode { try MeetingRecordStorage(url: storageURL).save(updated) }
            meetings = updated
            return true
        } catch {
            alertMessage = "녹음 이름을 저장하지 못했습니다. 기존 이름과 원본 파일은 유지됩니다. \(error.localizedDescription)"
            return false
        }
    }

    func canExportOriginal(meetingID: UUID) -> Bool {
        guard let meeting = meetings.first(where: { $0.id == meetingID }) else { return false }
        return meeting.status != .recording && activeMeetingID != meetingID && !isExportingOriginal && !isPreparingToQuit
    }

    func exportOriginal(meetingID: UUID, sourceID: String, to destination: URL, replacingExisting: Bool) async throws {
        guard !isPreparingToQuit else { throw RecordingManagementError.appClosing }
        guard let meeting = meetings.first(where: { $0.id == meetingID }),
              let source = meeting.originalRecordingFiles.first(where: { $0.id == sourceID }) else {
            throw RecordingManagementError.missingSource
        }
        guard meeting.status != .recording, activeMeetingID != meetingID else { throw RecordingManagementError.recordingInProgress }
        guard !isExportingOriginal else { throw RecordingManagementError.exportInProgress }
        exportingOriginalMeetingID = meetingID
        defer { exportingOriginalMeetingID = nil }
        try await OriginalRecordingExporter.export(source: source.url, to: destination,
            protectedDirectory: storageURL.deletingLastPathComponent(), replacingExisting: replacingExisting)
    }

    @discardableResult
    func moveMeeting(id: UUID, to folderID: UUID?) -> Bool {
        moveMeetings(ids: [id], to: folderID)
    }

    @discardableResult
    func moveMeetings(ids: [UUID], to folderID: UUID?) -> Bool {
        guard canModifyMeetingIndex else { return false }
        guard libraryStore.loadError == nil else {
            alertMessage = "폴더 정보를 읽지 못해 이동을 중단했습니다. 기존 녹음과 폴더 배치는 보존됩니다."
            return false
        }
        do {
            guard folderID == nil || libraryStore.containsFolder(folderID) else {
                throw MeetingFolderMoveError.missingFolder
            }
            guard !ids.isEmpty, Set(ids).count == ids.count,
                  ids.allSatisfy({ id in meetings.contains { $0.id == id } }) else {
                throw MeetingFolderMoveError.missingRecording
            }
            guard ids.allSatisfy({ canMoveMeeting(id: $0) }) else { throw MeetingFolderMoveError.processing }
            let moving = Set(ids)
            let changedCount = meetings.filter { moving.contains($0.id) && $0.folderID != folderID }.count
            guard changedCount > 0 else { return true }
            var updated = meetings
            for index in updated.indices where moving.contains(updated[index].id) { updated[index].folderID = folderID }
            do {
                if !isDemoMode { try MeetingRecordStorage(url: storageURL).save(updated) }
            } catch {
                folderMoveFeedback = nil
                alertMessage = "폴더 이동을 저장하지 못했습니다. 기존 폴더 배치와 원본 파일은 유지됩니다. \(error.localizedDescription)"
                return false
            }
            // Keep IDs, order, selection, playback and transcript identity unchanged.
            meetings = updated
            let destination = folderID == nil ? "미분류로" : "‘\(folderName(folderID))’ 폴더로"
            folderMoveFeedback = MeetingFolderMoveFeedback(message: "녹음 \(changedCount)개를 \(destination) 옮겼습니다.")
            return true
        } catch {
            folderMoveFeedback = nil
            alertMessage = error.localizedDescription
            return false
        }
    }

    func deleteFolder(id: UUID) -> Bool {
        guard canModifyMeetingIndex, libraryStore.loadError == nil, !isBusy else { return false }
        guard voice.canForget(libraryStore.speakerProfiles(in: id)) else {
            alertMessage = "폴더의 저장한 화자에서 등록 목소리를 먼저 삭제해 주세요. 음성 정보를 남긴 채 폴더를 삭제하지 않습니다."
            return false
        }
        let previous = meetings
        for index in meetings.indices where meetings[index].folderID == id { meetings[index].folderID = nil }
        guard saveMeetings() else { meetings = previous; return false }
        // Detach the index before removing metadata; restore it if metadata fails.
        guard libraryStore.edit({
            $0.folders.removeAll { $0.id == id }
            $0.speakerProfiles?.removeAll { $0.folderID == id }
        }) else {
            let detached = meetings
            meetings = previous
            if !saveMeetings() {
                meetings = detached
                alertMessage = "폴더 삭제를 마치지 못했습니다. 녹음은 ‘미분류’로 옮겨졌고 빈 폴더가 남아 있습니다. 원본과 전사는 보존됩니다."
            } else {
                alertMessage = "폴더를 삭제하지 못해 녹음의 원래 폴더 배치를 복원했습니다. 원본과 전사는 보존됩니다."
            }
            return false
        }
        if selection == .folder(id) { selection = .unfiled }
        return true
    }

    func acceptInferenceResult(_ result: InferenceResult, meetingID: UUID, sourceChannelID: String) throws {
        guard meetings.contains(where: { $0.id == meetingID }) else { throw TranscriptEditError.invalidDocument }
        try transcripts.accept(result, meetingID: meetingID, sourceChannelID: sourceChannelID)
    }

    func toggleRecordingPause() {
        guard isRecording else { return }
        if recorder.isPaused {
            do {
                try recorder.resume()
            } catch { alertMessage = "녹음을 재개하지 못했습니다. 일시정지 상태를 유지합니다. \(error.localizedDescription)" }
        } else {
            recorder.pause()
        }
    }

    func cancelProcessing() {
        if isProcessing { preparationTask?.task.cancel() }
        processingTask?.cancel()
        if processingTask != nil { processingStage = "처리를 중단하고 있습니다" }
    }

    func prepareToQuit() async -> Bool {
        guard !isPreparingToQuit else { return false }
        notionConnection.cancelConnect()
        guard !notionExporter.isSaving else {
            alertMessage = "Notion 저장 결과를 확인한 뒤 앱을 닫아 주세요."
            return false
        }
        guard !isImportingRecording else {
            alertMessage = "파일 가져오기를 마친 뒤 앱을 닫아 주세요."
            return false
        }
        isPreparingToQuit = true
        defer { isPreparingToQuit = false }
        await modelManager.cancelAndWait()
        await applePreparation.cancelAndWait()
        guard !isExportingOriginal else {
            alertMessage = "원본 파일 저장을 마치거나 취소한 뒤 앱을 닫아 주세요."
            return false
        }
        guard !voice.isSavingSpeakerProfile else {
            alertMessage = "화자 저장을 마친 뒤 앱을 닫아 주세요."
            return false
        }
        guard !isRecording, !isStartingCapture, !voiceEnrollmentSession.isActive else {
            alertMessage = "녹음을 종료한 뒤 앱을 닫아 주세요."
            return false
        }
        cancelProcessing()
        voice.cancelWork()
        _ = try? await preparationTask?.task.value
        await processingTask?.value
        await voice.waitForWork()
        guard !isExportingOriginal, !isRecording, !isStartingCapture, !isProcessing, !voice.isWorking, packagingCaptures == 0 else {
            alertMessage = "진행 중인 저장이나 녹음을 마친 뒤 앱을 닫아 주세요."
            return false
        }
        await notionConnection.diagnostics.flush()
        guard !notionExporter.isSaving, !notionConnection.isConnecting else {
            alertMessage = "Notion 작업을 마친 뒤 앱을 닫아 주세요."
            return false
        }
        voiceEnrollmentSession.discard()
        return true
    }

    func beginRecording(
        title: String,
        plan: MeetingInferencePlan? = nil,
        folderID: UUID? = nil,
        calendarEvent: ScheduledMeeting? = nil,
        attendance: MeetingAttendance? = nil
    ) async {
        guard canModifyMeetingIndex else { return }
        guard folderID == nil || libraryStore.containsFolder(folderID) else {
            alertMessage = "저장할 폴더를 다시 선택해 주세요."; return
        }
        guard !isBusy else {
            alertMessage = "현재 녹음 또는 전사를 먼저 마쳐 주세요."
            return
        }
        let selectedPlan: MeetingInferencePlan
        do {
            try attendance?.validate()
            if let attendance {
                let available = Set(folderID.map { libraryStore.speakerProfiles(in: $0).map(\.id) } ?? [])
                guard Set(attendance.profileIDs).isSubset(of: available) else { throw TranscriptEditError.invalidDocument }
            }
            selectedPlan = try plan ?? defaultSpeakerOptions.plan(isDual: false)
            _ = try selectedPlan.configurations(for: ["recording"])
        } catch { alertMessage = error.localizedDescription; return }
        let startID = UUID()
        captureStartID = startID
        defer { if captureStartID == startID { captureStartID = nil } }
        do { try await validatePreparation(selectedPlan.configurations(for: ["recording"])) }
        catch {
            if captureStartID == startID, !Task.isCancelled { alertMessage = error.localizedDescription }
            return
        }
        guard captureStartID == startID, !Task.isCancelled else { return }
        let allowed = await recorder.requestPermission()
        guard captureStartID == startID, !Task.isCancelled else { return }
        guard allowed else {
            alertMessage = RecordingError.permissionDenied.localizedDescription
            return
        }

        let id = UUID()
        let audioURL = RecordingFinalizer.captureURL(id: id, in: audioDirectory)
        var meeting = MeetingRecord(
            id: id,
            title: title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                ? "제목 없음"
                : title.trimmingCharacters(in: .whitespacesAndNewlines),
            startedAt: Date(),
            duration: 0,
            status: .recording,
            audioPath: audioURL.path,
            captureMode: .microphone,
            glossaryProfile: MeetingRecord.noGlossaryProfile,
            transcript: [],
            claims: [],
            errorMessage: nil
        )
        meeting.inferenceConfiguration = selectedPlan.configuration
        meeting.channelInferenceConfigurations = selectedPlan.channelConfigurations
        meeting.folderID = folderID
        meeting.calendarEvent = calendarEvent
        meeting.attendance = attendance
        let previousSelection = selection

        do {
            try recorder.start(to: audioURL)
        } catch {
            alertMessage = error.localizedDescription
            return
        }
        meetings.insert(meeting, at: 0)
        activeMeetingID = id
        selection = .meeting(id)
        if !saveMeetings() {
            _ = recorder.stop()
            activeMeetingID = nil
            meetings.removeAll { $0.id == id }
            selection = previousSelection
        } else if let attendance {
            libraryStore.rememberAttendance(attendance, folderID: folderID)
        }
        if activeMeetingID == id { workspaceSheet = nil }
    }

    func stopRecording() async {
        guard let id = activeMeetingID,
              let index = meetings.firstIndex(where: { $0.id == id }) else { return }
        meetings[index].duration = recorder.stop()
        meetings[index].status = .processing
        processingStage = "저장한 녹음을 준비하고 있습니다"
        activeMeetingID = nil
        saveMeetings()
        await packageRecording(id: id)
        await transcribeMeeting(id: id)
        settleUnstartedProcessing(id: id)
    }

    private func recordingStoppedUnexpectedly(duration: TimeInterval) {
        guard let id = activeMeetingID else { return }
        activeMeetingID = nil
        processingStage = nil
        let message = RecordingError.unexpectedlyStopped.localizedDescription
        if let index = meetings.firstIndex(where: { $0.id == id }) {
            meetings[index].duration = duration
            meetings[index].status = .failed
            meetings[index].errorMessage = message
            meetings[index].processingOutcome = .init(kind: .interrupted, message: message)
            if !saveMeetings() { return }
        }
        alertMessage = message
        Task { await packageRecording(id: id) }
    }

    /// Replaces a finished ADTS capture with the losslessly repackaged M4A. Failure keeps the
    /// capture as the meeting's audio; it remains readable for transcription.
    private func packageRecording(id: UUID) async {
        guard !isDemoMode, let path = meetings.first(where: { $0.id == id })?.audioPath,
              RecordingFinalizer.needsFinalizing(path) else { return }
        let capture = URL(fileURLWithPath: path)
        packagingCaptures += 1
        defer { packagingCaptures -= 1 }
        guard let packaged = try? await RecordingFinalizer.package(capture),
              let index = meetings.firstIndex(where: { $0.id == id }), meetings[index].audioPath == path else { return }
        let previous = meetings[index]
        meetings[index].audioPath = packaged.url.path
        meetings[index].duration = max(previous.duration, packaged.duration)
        guard saveMeetings() else {
            meetings[index] = previous
            try? FileManager.default.removeItem(at: packaged.url)
            return
        }
        if processingMeetingID != id { try? FileManager.default.removeItem(at: capture) }
    }

    private func packageInterruptedRecordings() async {
        for id in meetings.filter({ $0.status != .recording && $0.audioPath.map(RecordingFinalizer.needsFinalizing) == true }).map(\.id) {
            await packageRecording(id: id)
        }
    }

    // A meeting stays `.processing` only while its job runs. If transcription could not even
    // start, record that instead of leaving a stalled state until the next launch.
    private func settleUnstartedProcessing(id: UUID) {
        guard processingMeetingID != id, let index = meetings.firstIndex(where: { $0.id == id }),
              meetings[index].status == .processing else { return }
        let message = "전사를 시작하지 못했습니다. 원본 녹음은 보존됩니다. 다시 전사해 주세요."
        meetings[index].status = .failed
        meetings[index].errorMessage = message
        meetings[index].processingOutcome = .init(kind: .failed, message: message)
        processingStage = nil
        saveMeetings()
    }

    func importRecording(from sourceURL: URL, plan: MeetingInferencePlan? = nil, folderID: UUID? = nil) async {
        guard canModifyMeetingIndex else { return }
        guard folderID == nil || libraryStore.containsFolder(folderID) else {
            alertMessage = "저장할 폴더를 다시 선택해 주세요."; return
        }
        guard !isBusy else {
            alertMessage = "현재 녹음 또는 전사를 먼저 마쳐 주세요."
            return
        }
        let selectedPlan: MeetingInferencePlan
        do {
            selectedPlan = try plan ?? defaultSpeakerOptions.plan(isDual: false)
            _ = try selectedPlan.configurations(for: ["recording"])
        } catch { alertMessage = error.localizedDescription; return }
        isImportingRecording = true
        defer { isImportingRecording = false }
        do { try await validatePreparation(selectedPlan.configurations(for: ["recording"])) }
        catch { if !Task.isCancelled { alertMessage = error.localizedDescription }; return }
        let access = sourceURL.startAccessingSecurityScopedResource()
        defer {
            if access { sourceURL.stopAccessingSecurityScopedResource() }
        }

        let id = UUID()
        let ext = sourceURL.pathExtension.isEmpty ? "m4a" : sourceURL.pathExtension
        let target = audioDirectory.appendingPathComponent("\(id.uuidString).\(ext)")
        let previousSelection = selection
        do {
            try await Task.detached(priority: .utility) {
                try FileManager.default.copyItem(at: sourceURL, to: target)
            }.value
        } catch {
            alertMessage = "파일을 가져오지 못했습니다: \(error.localizedDescription)"
            return
        }
        var meeting = MeetingRecord(
            id: id,
            title: sourceURL.deletingPathExtension().lastPathComponent,
            startedAt: Date(),
            duration: 0,
            status: .processing,
            audioPath: target.path,
            captureMode: .microphone,
            glossaryProfile: MeetingRecord.noGlossaryProfile,
            transcript: [],
            claims: [],
            errorMessage: nil
        )
        meeting.inferenceConfiguration = selectedPlan.configuration
        meeting.channelInferenceConfigurations = selectedPlan.channelConfigurations
        meeting.folderID = folderID
        meetings.insert(meeting, at: 0)
        selection = .meeting(id)
        processingStage = "가져온 녹음을 준비하고 있습니다"
        guard saveMeetings() else {
            meetings.removeAll { $0.id == id }
            processingStage = nil
            selection = previousSelection
            // The copy was never referenced by the index; the user's source file is untouched.
            try? FileManager.default.removeItem(at: target)
            return
        }
        isImportingRecording = false
        await transcribeMeeting(id: id)
        settleUnstartedProcessing(id: id)
    }

    func deleteMeeting(_ meeting: MeetingRecord) {
        guard canModifyMeetingIndex, meeting.id != activeMeetingID || !isRecording,
              meeting.id != processingMeetingID, meeting.id != exportingOriginalMeetingID else { return }
        let previousMeetings = meetings
        let previousSelection = selection
        meetings.removeAll { $0.id == meeting.id }
        if selection == .meeting(meeting.id) { selection = .library }
        // Removing a list entry never deletes audio or corrections. A separate, confirmed
        // retention workflow is required before those files can be removed.
        if !saveMeetings() {
            meetings = previousMeetings
            selection = previousSelection
        }
    }

    func transcribeMeeting(id: UUID, plan: MeetingInferencePlan? = nil) async {
        guard canModifyMeetingIndex, !isBusy,
              let initialIndex = meetings.firstIndex(where: { $0.id == id }) else { return }
        let meeting = meetings[initialIndex]
        let sources: [(String, URL)]
        if let path = meeting.audioPath {
            sources = [("recording", URL(fileURLWithPath: path))]
        } else {
            sources = [("system", meeting.systemAudioPath), ("microphone", meeting.microphoneAudioPath)]
                .compactMap { key, path in path.map { (key, URL(fileURLWithPath: $0)) } }
        }
        guard !sources.isEmpty else { alertMessage = "전사할 원본 녹음이 없습니다."; return }
        let selectedPlan: MeetingInferencePlan
        let configurations: [String: InferenceConfiguration]
        do {
            if let plan { selectedPlan = plan }
            else if let configuration = meeting.inferenceConfiguration {
                selectedPlan = MeetingInferencePlan(configuration: configuration, channelConfigurations: meeting.channelInferenceConfigurations)
            } else { selectedPlan = try defaultSpeakerOptions.plan(isDual: sources.count > 1) }
            configurations = try selectedPlan.configurations(for: sources.map(\.0))
        } catch { alertMessage = error.localizedDescription; return }
        processingMeetingID = id
        do { try await validatePreparation(configurations) }
        catch {
            processingMeetingID = nil
            processingStage = nil
            if meeting.status == .processing, let current = meetings.firstIndex(where: { $0.id == id }) {
                let message = error is CancellationError ? "전사를 중단했습니다. 원본 녹음은 보존됩니다." : error.localizedDescription
                meetings[current].status = .failed
                meetings[current].errorMessage = message
                meetings[current].processingOutcome = .init(kind: error is CancellationError ? .cancelled : .failed, message: message)
                _ = saveMeetings()
            }
            if !(error is CancellationError), !Task.isCancelled { alertMessage = error.localizedDescription }
            return
        }
        processingMeetingID = nil
        guard !Task.isCancelled, let index = meetings.firstIndex(where: { $0.id == id }) else { return }
        let previousRecord = meetings[index]
        meetings[index].inferenceConfiguration = selectedPlan.configuration
        meetings[index].channelInferenceConfigurations = selectedPlan.channelConfigurations
        meetings[index].status = .processing
        meetings[index].errorMessage = nil
        meetings[index].processingOutcome = nil
        guard saveMeetings() else { meetings[index] = previousRecord; return }
        processingMeetingID = id
        processingStage = "녹음 파일을 준비하고 있습니다"
        let job = storageURL.deletingLastPathComponent().appendingPathComponent("Jobs/\(id.uuidString)/\(UUID().uuidString)", isDirectory: true)
        let task = Task { [weak self] in
            guard let self else { return }
            await self.process(id: id, meeting: meeting, sources: sources, configurations: configurations, job: job)
        }
        processingTask = task
        await task.value
    }

    private func process(id: UUID, meeting: MeetingRecord, sources: [(String, URL)],
                         configurations: [String: InferenceConfiguration], job: URL) async {
        var previousDocument: TranscriptDocument?
        var activatedDocument: TranscriptDocument?
        let activity = ProcessInfo.processInfo.beginActivity(options: [.userInitiated, .idleSystemSleepDisabled],
                                                             reason: "Transcribing a meeting")
        defer {
            ProcessInfo.processInfo.endActivity(activity)
            processingStage = nil
            processingMeetingID = nil
            processingTask = nil
        }
        do {
            var channels: [ChannelInference] = []
            for (sourceID, source) in sources {
                try Task.checkCancellation()
                do {
                    guard let sourceConfiguration = configurations[sourceID] else { throw TranscriptEditError.invalidDocument }
                    let result = try await inferenceService.run(source: source, configuration: sourceConfiguration,
                        directory: job.appendingPathComponent(sourceID)) { [weak self] stage in
                            await self?.setProcessingStage(stage, meetingID: id)
                        }
                    try Task.checkCancellation()
                    guard result.configuration == sourceConfiguration else { throw TranscriptEditError.invalidDocument }
                    try transcripts.preserveRawResult(result, for: id)
                    channels.append(ChannelInference(sourceID: sourceID, result: result))
                } catch InferenceError.noSpeech {
                    continue
                }
            }
            guard !channels.isEmpty else { throw InferenceError.noSpeech }
            try Task.checkCancellation()
            let document = try TranscriptDocument.preservingChannels(channels)
            previousDocument = transcripts.documents[id]
            if !isDemoMode {
                try FileManager.default.createDirectory(at: job, withIntermediateDirectories: true)
                let archive = MeetingInferenceArchive(revisionID: document.revisionID, channels: channels)
                try JSONEncoder().encode(archive).write(to: job.appendingPathComponent("result.json"), options: .withoutOverwriting)
            }
            try transcripts.activateOnDisk(document, for: id)
            activatedDocument = document
            guard let updated = meetings.firstIndex(where: { $0.id == id }) else { return }
            var completed = meetings[updated]
            completed.duration = max(meeting.duration, channels.map { $0.result.duration }.max() ?? 0)
            completed.status = .ready
            completed.errorMessage = nil
            completed.processingOutcome = .init(kind: .completed, retainedRevisionID: document.revisionID)
            completed.completedResult = MeetingCompletedResult(revisionID: document.revisionID, speakerCounts: channels.map {
                MeetingSpeakerCount(sourceID: $0.sourceID, expected: $0.result.configuration.expectedSpeakerCount,
                                    detected: Set($0.result.rawDiarization.map(\.clusterID)).count)
            })
            try publishProcessingRecord(completed)
            transcripts.publish(document, for: id)
            await voice.identifyAfterProcessing(meetingID: id)
        } catch {
            guard let updated = meetings.firstIndex(where: { $0.id == id }) else { return }
            var message = error is CancellationError ? "전사를 중단했습니다. 원본 녹음은 보존됩니다." : error.localizedDescription
            var retentionVerified = true
            if let activatedDocument {
                do {
                    try transcripts.rollback(to: previousDocument, from: activatedDocument, for: id)
                    message = "새 처리 결과를 저장하지 못했습니다. \(message)"
                } catch {
                    retentionVerified = false
                    transcripts.markUnreadable(id, reason: error.localizedDescription)
                    message = "새 결과 저장 및 이전 전사 복원 상태를 확인해야 합니다. 원본과 처리 출력은 보존됩니다. \(error.localizedDescription)"
                }
            }
            let retained = transcripts.documents[id]
            var failed = meetings[updated]
            failed.status = .failed
            failed.errorMessage = message
            failed.processingOutcome = .init(kind: error is CancellationError ? .cancelled : .failed,
                message: message, previousResultRetained: retentionVerified && retained != nil,
                retainedRevisionID: retentionVerified ? retained?.revisionID : nil)
            do { try publishProcessingRecord(failed) }
            catch {
                failed.processingOutcome?.message = "\(message) 처리 상태도 저장하지 못했습니다: \(error.localizedDescription)"
                meetings[updated] = failed
                alertMessage = failed.processingOutcome?.message
            }
            if !(error is CancellationError) { alertMessage = failed.processingOutcome?.message ?? message }
        }
    }

    private func setProcessingStage(_ stage: String, meetingID: UUID) {
        if processingMeetingID == meetingID { processingStage = stage }
    }

    private func publishProcessingRecord(_ record: MeetingRecord) throws {
        guard let index = meetings.firstIndex(where: { $0.id == record.id }) else { throw TranscriptEditError.invalidDocument }
        var next = meetings
        next[index] = record
        if !isDemoMode { try MeetingRecordStorage(url: storageURL).save(next) }
        meetings = next
    }

    private func recoverInterruptedProcessing() {
        var changed = false
        for index in meetings.indices where meetings[index].status == .processing || meetings[index].status == .recording {
            let revisionID = transcripts.storedRevisionID(for: meetings[index].id)
            meetings[index].status = .failed
            meetings[index].errorMessage = "이전 작업이 중단됐습니다. 원본을 확인하고 다시 전사해 주세요."
            meetings[index].processingOutcome = .init(kind: .interrupted, message: meetings[index].errorMessage,
                previousResultRetained: revisionID != nil || !meetings[index].transcript.isEmpty,
                retainedRevisionID: revisionID)
            changed = true
        }
        if changed { saveMeetings() }
    }

    private func loadMeetings() {
        do { meetings = try MeetingRecordStorage(url: storageURL).load() }
        catch {
            meetingIndexError = error.localizedDescription
            alertMessage = "회의 목록을 읽지 못했습니다. 기존 파일을 덮어쓰지 않도록 저장을 중단했습니다. \(error.localizedDescription)"
        }
    }

    private func recoverInterruptedCaptures() {
        guard let directories = try? FileManager.default.contentsOfDirectory(
            at: audioDirectory,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles]
        ) else { return }

        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        var changed = false
        for directory in directories {
            let manifestURL = directory.appendingPathComponent("capture-manifest.json")
            guard let data = try? Data(contentsOf: manifestURL),
                  let manifest = try? decoder.decode(CaptureSessionManifest.self, from: data),
                  manifest.status == .recording || manifest.status == .failed,
                  manifest.systemAudio.bufferCount > 0 || manifest.microphone.bufferCount > 0
            else { continue }

            let duration: TimeInterval
            if let recordedDuration = manifest.recordedDuration {
                duration = recordedDuration
            } else if let startedAt = manifest.startedAt {
                duration = max(0, (manifest.endedAt ?? Date()).timeIntervalSince(startedAt))
            } else {
                duration = 0
            }
            if let index = meetings.firstIndex(where: { $0.id == manifest.sessionID }) {
                // A historical unfinished manifest must not overwrite a later transcription outcome.
                guard meetings[index].status == .recording else { continue }
                meetings[index].status = .failed
                meetings[index].duration = max(meetings[index].duration, duration)
                meetings[index].systemAudioPath = manifest.systemAudio.path
                meetings[index].microphoneAudioPath = manifest.microphone.path
                meetings[index].captureManifestPath = manifestURL.path
                meetings[index].captureMode = .systemAndMicrophone
                meetings[index].errorMessage = "이전 앱 실행이 녹음 중 종료됐습니다. 저장된 PCM 채널을 검토해 주세요."
                meetings[index].processingOutcome = .init(kind: .interrupted, message: meetings[index].errorMessage)
            } else {
                meetings.insert(
                    MeetingRecord(
                        id: manifest.sessionID,
                        title: "복구된 회의",
                        startedAt: manifest.startedAt ?? manifest.createdAt,
                        duration: duration,
                        status: .failed,
                        audioPath: nil,
                        systemAudioPath: manifest.systemAudio.path,
                        microphoneAudioPath: manifest.microphone.path,
                        captureManifestPath: manifestURL.path,
                        captureMode: .systemAndMicrophone,
                        processingOutcome: .init(kind: .interrupted, message: "이전 녹음 작업이 중단됐습니다. 저장된 원본 채널로 다시 전사할 수 있습니다."),
                        glossaryProfile: "복구된 snapshot",
                        transcript: [],
                        claims: [],
                        errorMessage: "이전 앱 실행이 녹음 중 종료됐습니다. 저장된 PCM 채널을 검토해 주세요."
                    ),
                    at: 0
                )
            }
            changed = true
        }
        if changed { saveMeetings() }
    }

    @discardableResult
    private func saveMeetings() -> Bool {
        if isDemoMode { return true }
        guard canModifyMeetingIndex else { return false }
        do { try MeetingRecordStorage(url: storageURL).save(meetings); return true }
        catch {
            alertMessage = "회의 목록을 저장하지 못했습니다. 녹음 파일은 유지됩니다. \(error.localizedDescription)"
            return false
        }
    }

    private var canModifyMeetingIndex: Bool {
        guard let meetingIndexError else { return true }
        alertMessage = "회의 목록 파일을 먼저 복구해야 합니다. 기존 파일은 보존되어 있습니다. \(meetingIndexError)"
        return false
    }

    private static let demoMeeting: MeetingRecord = {
        let segments = [
            TranscriptSegment(
                startTime: 0,
                endTime: 18,
                speaker: "화자 A",
                text: "이번 릴리스에서는 기존 저장 형식을 유지하고 누락된 이벤트부터 확인하겠습니다.",
                confidence: 0.94,
                revisedText: nil
            ),
            TranscriptSegment(
                startTime: 18,
                endTime: 38,
                speaker: "화자 B",
                text: "저장 구조는 바꾸지 않고 출력 결과만 별도로 검증하는 것이 좋겠습니다.",
                confidence: 0.91,
                revisedText: nil
            ),
            TranscriptSegment(
                startTime: 38,
                endTime: 58,
                speaker: "화자 C",
                text: "그럼 이벤트 전달을 확인하고 저는 캐시 영향 범위를 정리하겠습니다.",
                confidence: 0.66,
                revisedText: "그럼 이벤트 전달을 확인하고 저는 캐시 영향 범위를 정리하겠습니다."
            ),
        ]
        return MeetingRecord(
            title: "제품 검토 회의",
            startedAt: Date().addingTimeInterval(-3_600),
            duration: 58,
            status: .needsReview,
            audioPath: nil,
            captureMode: .microphone,
            glossaryProfile: MeetingRecord.noGlossaryProfile,
            transcript: segments,
            claims: [
                EvidenceClaim(
                    kind: .decision,
                    text: "기존 저장 형식을 유지합니다.",
                    owner: nil,
                    sourceSegmentIDs: [segments[0].id],
                    isReviewed: true
                ),
                EvidenceClaim(
                    kind: .action,
                    text: "이벤트 전달 누락 여부를 확인합니다.",
                    owner: nil,
                    sourceSegmentIDs: [segments[0].id, segments[2].id],
                    isReviewed: false
                ),
            ],
            errorMessage: nil
        )
    }()
}
