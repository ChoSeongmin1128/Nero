@preconcurrency import AVFoundation
import Combine
import Darwin
import Foundation
import GroveInference

@MainActor
final class VoiceEnrollmentSession: ObservableObject {
    let recorder: AudioRecorder
    @Published private(set) var source: URL?
    @Published private(set) var naturalSpeechStart: Double?
    @Published private(set) var review: VoiceEnrollmentAudioReview?
    @Published private(set) var isAnalyzing = false
    @Published private(set) var isStarting = false
    @Published private(set) var isPlaying = false
    @Published var error: String?
    private let directory: URL
    private let requestPermission: @MainActor () async -> Bool
    private var preview: AVAudioPlayer?
    private var previewTask: Task<Void, Never>?
    private var analysisTask: Task<Void, Never>?
    private var subscription: AnyCancellable?
    private var generation = UUID()
    var isActive: Bool { isStarting || recorder.isRecording || isAnalyzing }

    init(directory: URL, recorder: AudioRecorder = AudioRecorder(), requestPermission: (@MainActor () async -> Bool)? = nil) {
        self.directory = directory
        self.recorder = recorder
        self.requestPermission = requestPermission ?? { await recorder.requestPermission() }
        Self.clearInterruptedCaptures(in: directory)
        subscription = recorder.objectWillChange.sink { [weak self] in self?.objectWillChange.send() }
        recorder.onUnexpectedStop = { [weak self] _ in
            self?.discard()
            self?.error = "녹음이 중단됐습니다. 마이크 연결 상태를 확인하고 다시 녹음해 주세요."
        }
    }

    func start() async {
        guard !isActive else { return }
        discard()
        let token = generation
        isStarting = true
        defer { if generation == token { isStarting = false } }
        let granted = await requestPermission()
        guard generation == token else { return }
        guard granted else { error = RecordingError.permissionDenied.localizedDescription; return }
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: directory.path)
            let url = directory.appendingPathComponent("enrollment-\(getpid())-\(UUID().uuidString).m4a")
            source = url
            try recorder.start(to: url)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
        } catch {
            _ = recorder.stop()
            if let source { try? FileManager.default.removeItem(at: source) }
            source = nil
            self.error = error.localizedDescription
        }
    }

    func beginNaturalSpeech() { naturalSpeechStart = recorder.elapsed }

    func finish() async {
        guard recorder.isRecording, let source else { return }
        _ = recorder.stop()
        guard let naturalSpeechStart else { error = "자유 발화 단계까지 녹음해 주세요."; return }
        isAnalyzing = true
        analysisTask = Task {
            defer { isAnalyzing = false }
            do {
                let result = try await VoiceEnrollmentAudio.inspect(source: source, naturalSpeechStart: naturalSpeechStart, workingDirectory: directory)
                try Task.checkCancellation()
                guard self.source == source else { return }
                review = result
                error = nil
            } catch is CancellationError {} catch { self.error = error.localizedDescription }
        }
        await analysisTask?.value
        analysisTask = nil
    }

    func togglePreview() {
        if isPlaying { stopPreview(); return }
        guard let source else { return }
        do {
            let player = try AVAudioPlayer(contentsOf: source)
            guard player.play() else { return }
            preview = player
            isPlaying = true
            previewTask = Task {
                while player.isPlaying {
                    do { try await Task.sleep(for: .milliseconds(150)) } catch { return }
                }
                isPlaying = false
            }
        } catch { self.error = error.localizedDescription }
    }

    func stopPreview() {
        previewTask?.cancel(); previewTask = nil
        preview?.stop(); preview = nil; isPlaying = false
    }

    func discard() {
        generation = UUID()
        analysisTask?.cancel(); stopPreview()
        _ = recorder.stop()
        isStarting = false
        if let source { try? FileManager.default.removeItem(at: source) }
        source = nil; review = nil; naturalSpeechStart = nil; error = nil
    }

    private static func clearInterruptedCaptures(in directory: URL) {
        let files = FileManager.default
        guard let contents = try? files.contentsOfDirectory(at: directory, includingPropertiesForKeys: [.isRegularFileKey, .isSymbolicLinkKey]) else { return }
        for url in contents where url.pathExtension == "m4a" {
            let parts = url.deletingPathExtension().lastPathComponent.split(separator: "-", maxSplits: 2)
            guard parts.count == 3, parts[0] == "enrollment", let process = Int32(parts[1]), process > 0,
                  UUID(uuidString: String(parts[2])) != nil,
                  let values = try? url.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey]),
                  values.isRegularFile == true, values.isSymbolicLink != true else { continue }
            if kill(process, 0) != 0 && errno == ESRCH { try? files.removeItem(at: url) }
        }
    }
}
