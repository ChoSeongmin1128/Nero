import Foundation
import GroveInference

enum ModelSmokeCheck {
    static func run(appBundle: URL, applicationSupport: URL) async throws {
        let directory = applicationSupport.appendingPathComponent("ModelChecks/\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let audio = directory.appendingPathComponent("silence.wav")
        try silentWAV().write(to: audio, options: .withoutOverwriting)
        let configuration = InferenceConfiguration(diarizationPreference: .nemotron3)
        let backend = try BundledMeetingInferenceService(appBundle: appBundle, applicationSupport: applicationSupport).backend(configuration: configuration)
        do { _ = try await backend.transcribe(audio: audio, duration: 1, directory: directory) }
        catch InferenceError.noSpeech { }
        try Task.checkCancellation()
        _ = try await backend.diarize(audio: audio, duration: 1, configuration: configuration, directory: directory)
    }

    static func silentWAV() -> Data {
        var data = Data()
        func text(_ value: String) { data.append(Data(value.utf8)) }
        func number<T: FixedWidthInteger>(_ value: T) {
            var littleEndian = value.littleEndian
            withUnsafeBytes(of: &littleEndian) { data.append(contentsOf: $0) }
        }
        text("RIFF"); number(UInt32(32036)); text("WAVEfmt "); number(UInt32(16))
        number(UInt16(1)); number(UInt16(1)); number(UInt32(16000)); number(UInt32(32000))
        number(UInt16(2)); number(UInt16(16)); text("data"); number(UInt32(32000))
        data.append(Data(repeating: 0, count: 32000))
        return data
    }
}
