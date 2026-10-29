@preconcurrency import AVFoundation
import Foundation

@MainActor
protocol AudioRecordingDevice: AnyObject {
    var isRecording: Bool { get }
    var currentTime: TimeInterval { get }
    var isMeteringEnabled: Bool { get set }
    func prepareToRecord() -> Bool
    func record() -> Bool
    func pause()
    func stop()
    func updateMeters()
    func averagePower(forChannel channelNumber: Int) -> Float
    func observeTermination(_ handler: @escaping @Sendable () -> Void)
}

extension AudioRecordingDevice {
    func observeTermination(_ handler: @escaping @Sendable () -> Void) {}
}

final class RecordingDelegate: NSObject, AVAudioRecorderDelegate, Sendable {
    private let onTermination: @Sendable () -> Void
    init(onTermination: @escaping @Sendable () -> Void) { self.onTermination = onTermination }
    func audioRecorderDidFinishRecording(_ recorder: AVAudioRecorder, successfully flag: Bool) { onTermination() }
    func audioRecorderEncodeErrorDidOccur(_ recorder: AVAudioRecorder, error: (any Error)?) { onTermination() }
}

@MainActor
private final class SystemRecordingDevice: AudioRecordingDevice {
    private let device: AVAudioRecorder
    private var delegate: RecordingDelegate?
    init(url: URL, settings: [String: Any]) throws { device = try AVAudioRecorder(url: url, settings: settings) }
    var isRecording: Bool { device.isRecording }
    var currentTime: TimeInterval { device.currentTime }
    var isMeteringEnabled: Bool {
        get { device.isMeteringEnabled }
        set { device.isMeteringEnabled = newValue }
    }
    func prepareToRecord() -> Bool { device.prepareToRecord() }
    func record() -> Bool { device.record() }
    func pause() { device.pause() }
    func stop() { device.stop() }
    func updateMeters() { device.updateMeters() }
    func averagePower(forChannel channelNumber: Int) -> Float { device.averagePower(forChannel: channelNumber) }
    func observeTermination(_ handler: @escaping @Sendable () -> Void) {
        delegate = RecordingDelegate(onTermination: handler)
        device.delegate = delegate
    }
}

@MainActor
final class AudioRecorder: NSObject, ObservableObject {
    @Published private(set) var isRecording = false
    @Published private(set) var isPaused = false
    @Published private(set) var elapsed: TimeInterval = 0
    @Published private(set) var level: Double = 0
    @Published private(set) var isInputSilent = false
    var onUnexpectedStop: ((TimeInterval) -> Void)?

    // An all-zero input reads as -160 dB. Ninety seconds of it before warning is the default
    // of meeting-transcriber's channel fault monitor; a quiet but live microphone stays above it.
    static let digitalSilenceFloor: Float = -120
    static let silenceWarningDelay: TimeInterval = 90

    private var recorder: (any AudioRecordingDevice)?
    private var meterTimer: Timer?
    private var captureID: UUID?
    private var silentSince: TimeInterval?
    private var activity: (any NSObjectProtocol)?
    private let makeRecorder: (URL, [String: Any]) throws -> any AudioRecordingDevice
    private let permissionRequester: () async -> Bool
    private let clock: () -> TimeInterval

    init(makeRecorder: @escaping (URL, [String: Any]) throws -> any AudioRecordingDevice = { try SystemRecordingDevice(url: $0, settings: $1) },
         permissionRequester: @escaping () async -> Bool = AudioRecorder.requestSystemPermission,
         clock: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }) {
        self.makeRecorder = makeRecorder
        self.permissionRequester = permissionRequester
        self.clock = clock
        super.init()
    }

    func requestPermission() async -> Bool {
        await permissionRequester()
    }

    private static func requestSystemPermission() async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized:
            return true
        case .denied, .restricted:
            return false
        case .notDetermined:
            return await AVCaptureDevice.requestAccess(for: .audio)
        @unknown default:
            return false
        }
    }

    func start(to url: URL) throws {
        guard !isRecording else { throw RecordingError.couldNotStart }
        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 48_000,
            AVNumberOfChannelsKey: 1,
            AVEncoderAudioQualityKey: AVAudioQuality.high.rawValue,
            AVEncoderBitRateKey: 128_000,
        ]
        let recorder = try makeRecorder(url, settings)
        let captureID = UUID()
        recorder.observeTermination { [weak self] in
            Task { @MainActor in self?.recordingTerminated(captureID: captureID) }
        }
        recorder.isMeteringEnabled = true
        guard recorder.prepareToRecord(), recorder.record() else {
            throw RecordingError.couldNotStart
        }
        self.recorder = recorder
        self.captureID = captureID
        isPaused = false
        elapsed = 0
        level = 0
        resetSilence()
        isRecording = true
        activity = ProcessInfo.processInfo.beginActivity(options: [.userInitiated, .idleSystemSleepDisabled],
                                                         reason: "Recording a meeting")
        meterTimer?.invalidate()
        meterTimer = Timer.scheduledTimer(
            timeInterval: 0.08,
            target: self,
            selector: #selector(updateMeter),
            userInfo: nil,
            repeats: true
        )
    }

    @discardableResult
    func stop() -> TimeInterval {
        let finalDuration = max(recorder?.currentTime ?? 0, elapsed)
        captureID = nil
        recorder?.stop()
        recorder = nil
        meterTimer?.invalidate()
        meterTimer = nil
        isRecording = false
        isPaused = false
        elapsed = finalDuration
        level = 0
        resetSilence()
        if let activity { ProcessInfo.processInfo.endActivity(activity) }
        activity = nil
        return finalDuration
    }

    func pause() {
        guard isRecording, !isPaused, let recorder else { return }
        recorder.pause()
        elapsed = recorder.currentTime
        isPaused = true
        level = 0
        resetSilence()
    }

    func resume() throws {
        guard isRecording, isPaused, let recorder else { return }
        guard recorder.record() else { throw RecordingError.couldNotStart }
        isPaused = false
    }

    @objc func updateMeter() {
        guard let recorder, isRecording, !isPaused else { return }
        guard recorder.isRecording else {
            if let captureID { recordingTerminated(captureID: captureID) }
            return
        }
        recorder.updateMeters()
        let decibels = recorder.averagePower(forChannel: 0)
        level = max(0, min(1, pow(10, Double(decibels) / 28)))
        elapsed = recorder.currentTime
        if decibels <= Self.digitalSilenceFloor {
            let now = clock()
            let since = silentSince ?? now
            silentSince = since
            if !isInputSilent && now - since >= Self.silenceWarningDelay { isInputSilent = true }
        } else {
            resetSilence()
        }
    }

    private func resetSilence() {
        silentSince = nil
        if isInputSilent { isInputSilent = false }
    }

    private func recordingTerminated(captureID: UUID) {
        guard self.captureID == captureID, isRecording else { return }
        let duration = stop()
        onUnexpectedStop?(duration)
    }
}

enum RecordingError: LocalizedError {
    case permissionDenied
    case couldNotStart
    case unexpectedlyStopped

    var errorDescription: String? {
        switch self {
        case .permissionDenied:
            "마이크 접근이 꺼져 있습니다. 시스템 설정 → 개인정보 보호 및 보안 → 마이크에서 Grove를 허용해 주세요."
        case .couldNotStart:
            "선택한 마이크로 녹음을 시작하지 못했습니다. 입력 장치 연결 상태를 확인해 주세요."
        case .unexpectedlyStopped:
            "녹음이 예기치 않게 중단됐습니다. 저장된 녹음은 보존됩니다. 마이크 연결 상태를 확인해 주세요."
        }
    }
}
