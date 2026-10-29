import Foundation
import Testing
@testable import GroveApp

struct CaptureModelTests {
    @Test
    func legacyMeetingDecodesWithoutCaptureFields() throws {
        let json = #"""
        {
          "id": "11111111-1111-1111-1111-111111111111",
          "title": "기존 회의",
          "startedAt": "2026-09-02T00:00:00Z",
          "duration": 12,
          "status": "ready",
          "audioPath": null,
          "glossaryProfile": "Legacy profile",
          "transcript": [],
          "claims": [],
          "errorMessage": null
        }
        """#
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let meeting = try decoder.decode(MeetingRecord.self, from: Data(json.utf8))
        #expect(meeting.captureMode == nil)
        #expect(meeting.systemAudioPath == nil)
        #expect(meeting.title == "기존 회의")
    }

    @Test
    func captureManifestRoundTripsChannelEvidence() throws {
        let manifest = CaptureSessionManifest(
            sessionID: UUID(uuidString: "22222222-2222-2222-2222-222222222222")!,
            status: .recording,
            createdAt: Date(timeIntervalSince1970: 100),
            startedAt: Date(timeIntervalSince1970: 101),
            endedAt: nil,
            selectedContentDescription: "테스트 화면",
            systemAudio: CaptureChannelStats(
                path: "/tmp/system.caf",
                firstPresentationSeconds: 50,
                lastPresentationSeconds: 55,
                bufferCount: 10,
                detectedGapCount: 0,
                writtenFrameCount: 240_000
            ),
            microphone: CaptureChannelStats(
                path: "/tmp/microphone.caf",
                firstPresentationSeconds: 50.02,
                lastPresentationSeconds: 55.02,
                bufferCount: 10,
                detectedGapCount: 1,
                writtenFrameCount: 240_000
            ),
            errorMessage: nil
        )
        let data = try JSONEncoder().encode(manifest)
        let decoded = try JSONDecoder().decode(CaptureSessionManifest.self, from: data)
        #expect(decoded.status == .recording)
        #expect(decoded.systemAudio.bufferCount == 10)
        #expect(decoded.microphone.detectedGapCount == 1)
    }
}
