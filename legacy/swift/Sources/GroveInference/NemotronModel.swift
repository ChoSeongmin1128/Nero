import Foundation

public enum NemotronModel {
    public static let revision = "f667ed73aee57d40cc39428eb768b4fd87a0a29e"
    public static let fileName = "Nemotron-3-Diarization.q8_0.gguf"
    public static let sha256 = "08456d9e22cd9a323c0364d98375f3746d6e68507ebb705cd46438c534c7a3a1"
    public static let workerRuntime = "nemo-speech-0.2.0"
    public static func url(in applicationSupport: URL) -> URL {
        applicationSupport.appendingPathComponent("Models/Nemotron3/\(revision)/\(fileName)")
    }
}
