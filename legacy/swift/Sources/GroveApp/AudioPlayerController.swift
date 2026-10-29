@preconcurrency import AVFoundation
import Foundation

struct MeetingPlaybackSource: Equatable {
    let meetingID: UUID
    let audioPath: String?
    let microphoneAudioPath: String?
    let systemAudioPath: String?
    let isFinalized: Bool
    init(meeting: MeetingRecord) {
        meetingID = meeting.id
        audioPath = meeting.audioPath
        microphoneAudioPath = meeting.microphoneAudioPath
        systemAudioPath = meeting.systemAudioPath
        isFinalized = meeting.status != .recording
    }
}

@MainActor
final class AudioPlayerController: ObservableObject {
    @Published private(set) var playingSegmentID: UUID?
    @Published private(set) var position: TimeInterval = 0
    @Published private(set) var duration: TimeInterval = 0
    @Published private(set) var isPlaying = false
    @Published private(set) var errorMessage: String?
    @Published private(set) var rate: Float = 1
    @Published private(set) var sourceChannel: String?
    private var player: AVAudioPlayer?
    private var loadedPath: String?
    private var stopBoundary: TimeInterval?
    private var ticker: Task<Void, Never>?

    func play(meeting: MeetingRecord, segment: TranscriptSegment) {
        play(meeting: meeting, id: segment.id, start: segment.startTime, end: segment.endTime,
             source: segment.speaker == "내 마이크" ? "microphone" : segment.speaker == "원격 오디오" ? "system" : nil)
    }

    func play(meeting: MeetingRecord, utterance: DocumentUtterance) {
        play(meeting: meeting, id: utterance.id, start: utterance.startTime, end: utterance.endTime, source: utterance.sourceChannelID)
    }

    func prepare(meeting: MeetingRecord, source: String? = nil) {
        guard MeetingPlaybackSource(meeting: meeting).isFinalized else {
            stop()
            errorMessage = nil
            return
        }
        let path: String?
        switch source {
        case "microphone": path = meeting.microphoneAudioPath ?? meeting.audioPath
        case "system": path = meeting.systemAudioPath ?? meeting.audioPath
        default: path = meeting.audioPath ?? meeting.systemAudioPath ?? meeting.microphoneAudioPath
        }
        guard let path else {
            stop()
            errorMessage = "녹음 파일이 없습니다."
            return
        }
        guard loadedPath != path else { return }
        stop()
        do {
            let player = try AVAudioPlayer(contentsOf: URL(fileURLWithPath: path))
            player.enableRate = true
            player.rate = rate
            guard player.prepareToPlay() else {
                errorMessage = "녹음 파일을 재생할 준비를 하지 못했습니다."
                return
            }
            self.player = player
            loadedPath = path
            sourceChannel = path == meeting.microphoneAudioPath ? "microphone" : path == meeting.systemAudioPath ? "system" : nil
            // AVAudioPlayer only estimates the length of an ADTS capture that was never repackaged.
            duration = URL(fileURLWithPath: path).pathExtension.lowercased() == "aac"
                ? RecordingFinalizer.duration(of: URL(fileURLWithPath: path)) ?? player.duration : player.duration
            errorMessage = nil
        } catch {
            errorMessage = "녹음 파일을 열지 못했습니다. \(error.localizedDescription)"
        }
    }

    func toggle() {
        guard let player else { return }
        stopBoundary = nil
        playingSegmentID = nil
        if player.isPlaying {
            player.pause()
            isPlaying = false
        } else {
            if player.currentTime >= player.duration - 0.05 { player.currentTime = 0 }
            isPlaying = player.play()
            startTicker()
        }
    }

    func seek(to time: TimeInterval) {
        guard time.isFinite, let player else { return }
        stopBoundary = nil
        playingSegmentID = nil
        player.currentTime = min(max(0, time), player.duration)
        position = player.currentTime
    }

    func pause() {
        player?.pause()
        ticker?.cancel()
        isPlaying = false
        playingSegmentID = nil
        stopBoundary = nil
        position = player?.currentTime ?? position
    }

    func setRate(_ value: Float) {
        rate = value
        player?.rate = value
    }

    func stop() {
        ticker?.cancel()
        ticker = nil
        player?.stop()
        player = nil
        loadedPath = nil
        playingSegmentID = nil
        stopBoundary = nil
        isPlaying = false
        position = 0
        duration = 0
        sourceChannel = nil
    }

    private func play(meeting: MeetingRecord, id: UUID, start: TimeInterval, end: TimeInterval, source: String?) {
        prepare(meeting: meeting, source: source)
        guard let player else { return }
        player.currentTime = min(max(0, start - 0.2), player.duration)
        stopBoundary = min(end + 0.2, player.duration)
        playingSegmentID = id
        position = player.currentTime
        isPlaying = player.play()
        startTicker()
    }

    private func startTicker() {
        ticker?.cancel()
        ticker = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(100))
                guard !Task.isCancelled, let self, let player = self.player else { return }
                self.position = player.currentTime
                if let boundary = self.stopBoundary, player.currentTime >= boundary {
                    player.pause()
                    self.stopBoundary = nil
                }
                self.isPlaying = player.isPlaying
                if !player.isPlaying {
                    self.playingSegmentID = nil
                    return
                }
            }
        }
    }
}
