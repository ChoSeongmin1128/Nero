import SwiftUI

struct RecordingHUD: View {
    @ObservedObject var store: GroveStore
    @ObservedObject private var recorder: AudioRecorder
    let meeting: MeetingRecord

    init(store: GroveStore, meeting: MeetingRecord) {
        self.store = store
        self.meeting = meeting
        recorder = store.recorder
    }

    var body: some View {
        HStack(spacing: 16) {
            Circle()
                .fill(recorder.isPaused ? Color.secondary : .red)
                .frame(width: 10, height: 10)
                .shadow(color: .red.opacity(0.35), radius: 5)
            VStack(alignment: .leading, spacing: 2) {
                Text(meeting.title)
                    .font(.callout.weight(.semibold))
                    .lineLimit(1)
                if recorder.isInputSilent {
                    Text("입력이 들어오지 않습니다. 마이크 연결과 음소거를 확인해 주세요.")
                        .font(.caption2)
                        .foregroundStyle(GroveTheme.evidence)
                } else {
                    Text(recorder.isPaused ? "일시정지됨 · 멈춘 시간은 저장하지 않습니다" : "원본 저장 중")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            Text(recorder.elapsed.clockString)
                .font(.body.monospacedDigit().weight(.medium))
            channelMeter(label: "마이크", symbol: "mic.fill", level: recorder.level)
            Divider().frame(height: 26)
            Button {
                store.toggleRecordingPause()
            } label: {
                Label(recorder.isPaused ? "계속 녹음" : "일시정지", systemImage: recorder.isPaused ? "play.fill" : "pause.fill")
            }
            .accessibilityIdentifier("recording.pause-resume")
            Button("종료") {
                Task { await store.stopRecording() }
            }
            .buttonStyle(.borderedProminent)
            .tint(.red)
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 12)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16))
        .overlay {
            RoundedRectangle(cornerRadius: 16).stroke(Color.white.opacity(0.22))
        }
        .shadow(color: .black.opacity(0.16), radius: 20, y: 7)
        .frame(maxWidth: 760)
    }

    private func channelMeter(label: String, symbol: String, level: Double) -> some View {
        HStack(spacing: 7) {
            LevelMeter(level: level)
                .frame(width: 58, height: 24)
            Label(label, systemImage: symbol)
                .font(.caption)
                .foregroundStyle(level > 0.03 ? GroveTheme.grove : .secondary)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(label) 입력 레벨")
        .accessibilityValue("\(Int(level * 100))퍼센트")
    }
}

private struct LevelMeter: View {
    let level: Double

    var body: some View {
        HStack(alignment: .center, spacing: 3) {
            ForEach(0..<8, id: \.self) { index in
                let threshold = Double(index + 1) / 8
                Capsule()
                    .fill(level >= threshold ? GroveTheme.grove : Color.secondary.opacity(0.18))
                    .frame(width: 4, height: 7 + CGFloat(index % 4) * 4)
            }
        }
    }
}
