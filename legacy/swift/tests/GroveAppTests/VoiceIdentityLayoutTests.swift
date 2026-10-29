import AppKit
import GroveInference
import SwiftUI
import Testing
@testable import GroveApp

@MainActor
struct VoiceIdentityLayoutTests {
    // Synthetic, offscreen only. Rendering does not enroll a voice or run identification.
    @Test(.enabled(if: ProcessInfo.processInfo.environment["GROVE_VOICE_LAYOUT_OUTPUT"] != nil))
    func renderEnrollmentAndFolderLibrary() async throws {
        let output = URL(fileURLWithPath: try #require(ProcessInfo.processInfo.environment["GROVE_VOICE_LAYOUT_OUTPUT"]))
        let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: base) }
        _ = NSApplication.shared
        GroveTypography.registerFonts()
        let store = GroveStore(baseDirectory: base, voiceIdentificationAvailable: true)
        let folderID = try #require(store.createFolder(name: "함께하는 회의"))
        var meeting = MeetingRecord(title: "화자 등록 화면 테스트", startedAt: Date(), duration: 35, status: .ready,
                                    audioPath: base.appendingPathComponent("synthetic.wav").path,
                                    glossaryProfile: "", transcript: [], claims: [], errorMessage: nil)
        meeting.folderID = folderID
        store.meetings = [meeting]
        let result = try InferenceResult(duration: 35, configuration: .init(expectedSpeakerCount: 1),
            transcription: .init(utterances: [
                .init(start: 0, end: 5, text: "첫 번째 안건부터 차례대로 말씀드리겠습니다."),
                .init(start: 6, end: 12, text: "다음 회의에서 논의할 내용도 함께 정리하겠습니다."),
                .init(start: 13, end: 19, text: "이 발화의 목소리가 같은 사람인지 듣고 확인합니다."),
                .init(start: 20, end: 27, text: "목소리 등록은 이름만 저장하는 기능과 별개이며, 등록한 자료는 폴더에서 삭제할 수 있습니다.")
            ]), rawDiarization: [
                .init(start: 0, end: 5, clusterID: "A"), .init(start: 6, end: 12, clusterID: "A"),
                .init(start: 13, end: 19, clusterID: "A"), .init(start: 20, end: 27, clusterID: "A")
            ])
        try store.acceptInferenceResult(result, meetingID: meeting.id, sourceChannelID: "recording")
        let speaker = try #require(store.transcriptDocuments[meeting.id]?.speakers.first)
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        let player = AudioPlayerController()
        for scheme in [ColorScheme.light, .dark] {
            try render(VoiceEnrollmentSheet(store: store, meeting: meeting, speaker: speaker, player: player)
                .environment(\.colorScheme, scheme),
                size: CGSize(width: 580, height: 720),
                destination: output.appendingPathComponent("voice-enrollment-\(scheme == .dark ? "dark" : "light").png"))
        }
        let missingSpeaker = MeetingSpeaker(name: "아직 발화가 없는 화자", order: 1)
        try render(VoiceEnrollmentSheet(store: store, meeting: meeting, speaker: missingSpeaker, player: player)
            .environment(\.colorScheme, .dark),
            size: CGSize(width: 580, height: 540),
            destination: output.appendingPathComponent("voice-enrollment-empty.png"))
        #expect(await store.voice.saveSpeakerProfile(meetingID: meeting.id, speakerID: speaker.id, name: "등록 전 이름"))
        try render(FolderSpeakerLibraryView(store: store, folderID: folderID).environment(\.colorScheme, .dark),
                   size: CGSize(width: 540, height: 300),
                   destination: output.appendingPathComponent("voice-library-names-only.png"))
        let gatedStore = GroveStore(baseDirectory: base)
        try render(FolderSpeakerLibraryView(store: gatedStore, folderID: folderID).environment(\.colorScheme, .dark),
                   size: CGSize(width: 540, height: 360),
                   destination: output.appendingPathComponent("voice-library-production-gate.png"))
        let profileID = try #require(gatedStore.libraryStore.addTeamMember(folderID: folderID, name: "등록할 팀원"))
        let profile = try #require(gatedStore.speakerProfiles(in: folderID).first { $0.id == profileID })
        if let modelPath = ProcessInfo.processInfo.environment["GROVE_VOICE_IDENTITY_MODELS"] {
            let destination = base.appendingPathComponent("Models/VoiceIdentity")
            try FileManager.default.createDirectory(at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
            try FileManager.default.copyItem(at: URL(fileURLWithPath: modelPath), to: destination)
            gatedStore.modelManager.install(groups: [.voiceIdentity])
            while gatedStore.modelManager.isInstalling { try await Task.sleep(for: .milliseconds(50)) }
            #expect(gatedStore.modelManager.isVoiceModelReady)
        }
        for scheme in [ColorScheme.light, .dark] {
            try render(RecordedVoiceEnrollmentSheet(store: gatedStore, profile: profile).environment(\.colorScheme, scheme),
                       size: CGSize(width: 560, height: 520),
                       destination: output.appendingPathComponent("voice-reading-\(scheme == .dark ? "dark" : "light").png"))
        }
        var attendance = MeetingAttendance(profileIDs: [profileID], names: ["이번 회의 참석자"], guestCount: 3)
        try render(MeetingAttendanceView(store: gatedStore, folderID: folderID, attendance: Binding(get: { attendance }, set: { attendance = $0 }))
            .environment(\.colorScheme, .dark), size: CGSize(width: 500, height: 230),
            destination: output.appendingPathComponent("attendance.png"))
        let event = ScheduledMeeting(id: "preview", calendarID: "preview", title: "함께하는 회의", start: Date().addingTimeInterval(120),
            end: Date().addingTimeInterval(3600), calendarName: "일정")
        try render(MeetingReminderBanner(meeting: event, record: {}, snooze: {}, close: {}),
                   size: CGSize(width: 440, height: 40), destination: output.appendingPathComponent("calendar-pill.png"))
    }

    private func render(_ view: some View, size: CGSize, destination: URL) throws {
        let host = NSHostingView(rootView: view.frame(width: size.width, height: size.height)
            .background(Color(nsColor: .windowBackgroundColor)))
        let window = NSWindow(contentRect: NSRect(origin: CGPoint(x: -10000, y: -10000), size: size),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.appearance = NSAppearance(named: destination.lastPathComponent.contains("light") ? .aqua : .darkAqua)
        window.contentView = host
        host.frame = NSRect(origin: .zero, size: size)
        host.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.15))
        host.layoutSubtreeIfNeeded()
        let bitmap = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        let png = try #require(bitmap.representation(using: .png, properties: [:]))
        try png.write(to: destination, options: .atomic)
        #expect(bitmap.pixelsWide >= Int(size.width))
        window.contentView = nil
    }
}
