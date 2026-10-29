import Foundation
import GroveInference

struct SavedSpeakerProfile: Identifiable, Codable, Hashable, Sendable {
    var id = UUID()
    let folderID: UUID
    var name: String
    // Decoded only so libraries from the experimental builds still open; saves always strip it.
    var voice: SpeakerVoicePrint? = nil
    // Written before touching Keychain. A failed first registration still has an
    // addressable cleanup action after restart, even if no ciphertext was published.
    var voiceStorageReferenced: Bool? = nil
    let sourceMeetingID: UUID?
    let sourceRevisionID: UUID?
    let sourceSpeakerID: UUID?
    let createdAt: Date
}

struct SpeakerProfileMatch: Codable, Hashable, Sendable {
    let folderID: UUID
    let profileID: UUID
    let similarity: Float?
    let isConfirmed: Bool
    var originalName: String? = nil
    var identityEvidence: SpeakerIdentityEvidence? = nil
}

struct SpeakerIdentityEvidence: Codable, Hashable, Sendable {
    let modelIdentifier: String
    let policyVersion: String
    let enrollmentCreatedAt: Date
    let queryUtteranceIDs: [UUID]
}

enum VoiceProfileError: LocalizedError {
    case recordingChanged
    case folderRequired
    var errorDescription: String? {
        switch self {
        case .recordingChanged: "처리 중 녹음이나 화자 정보가 바뀌었습니다. 변경 내용을 확인하고 다시 시도해 주세요."
        case .folderRequired: "녹음을 폴더에 먼저 옮겨 주세요. 저장한 목소리는 같은 폴더에서만 사용합니다."
        }
    }
}
