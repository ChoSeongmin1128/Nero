import Foundation
import GroveInference

struct ArchivedTranscript: Identifiable, Codable {
    let id: UUID
    let meetingID: UUID
    let savedAt: Date
    let document: TranscriptDocument
}

struct TranscriptDocumentStorage {
    let directory: URL

    private struct Envelope: Codable {
        let meetingID: UUID
        let document: TranscriptDocument
    }

    func load(for meetingID: UUID) throws -> TranscriptDocument? {
        try loadCurrent(for: meetingID)?.document
    }

    private func loadCurrent(for meetingID: UUID) throws -> (document: TranscriptDocument, data: Data)? {
        let url = fileURL(for: meetingID)
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        let data = try Data(contentsOf: url)
        let envelope = try JSONDecoder().decode(Envelope.self, from: data)
        guard envelope.meetingID == meetingID else { throw TranscriptEditError.invalidDocument }
        try envelope.document.validate()
        return (envelope.document, data)
    }

    func save(_ document: TranscriptDocument, for meetingID: UUID) throws {
        let current = try loadCurrent(for: meetingID)
        if let current, current.document.revisionID != document.revisionID {
            throw TranscriptEditError.invalidDocument
        }
        try write(document, for: meetingID, replacing: current?.data)
    }

    func activate(_ document: TranscriptDocument, for meetingID: UUID, rawResult: InferenceResult? = nil) throws {
        try document.validate()
        if let rawResult {
            try rawResult.validate()
            guard rawResult.jobID == document.revisionID else { throw TranscriptEditError.invalidDocument }
        }
        let stored = try loadCurrent(for: meetingID)
        let current = stored?.document
        if current == document { return }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        if let current {
            let archive = ArchivedTranscript(id: UUID(), meetingID: meetingID, savedAt: Date(), document: current)
            let folder = archiveDirectory(for: meetingID)
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try encoder.encode(archive).write(to: folder.appendingPathComponent("\(archive.id.uuidString).json"), options: .withoutOverwriting)
        }
        if let rawResult {
            try preserveRawResult(rawResult, for: meetingID)
        }
        try write(document, for: meetingID, replacing: stored?.data)
    }

    func preserveRawResult(_ result: InferenceResult, for meetingID: UUID) throws {
        try result.validate()
        let folder = directory.appendingPathComponent("EngineResults", isDirectory: true).appendingPathComponent(meetingID.uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let url = folder.appendingPathComponent("\(result.jobID.uuidString).json")
        if FileManager.default.fileExists(atPath: url.path) {
            let existing = try JSONDecoder().decode(InferenceResult.self, from: Data(contentsOf: url))
            try existing.validate()
            guard existing == result else { throw TranscriptEditError.invalidDocument }
        } else {
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            try encoder.encode(result).write(to: url, options: .withoutOverwriting)
        }
    }

    func archives(for meetingID: UUID) throws -> [ArchivedTranscript] {
        let folder = archiveDirectory(for: meetingID)
        guard FileManager.default.fileExists(atPath: folder.path) else { return [] }
        return try FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)
            .filter { $0.pathExtension == "json" }
            .map { url in
                let archive = try JSONDecoder().decode(ArchivedTranscript.self, from: Data(contentsOf: url))
                guard archive.meetingID == meetingID, url.deletingPathExtension().lastPathComponent == archive.id.uuidString else {
                    throw TranscriptEditError.invalidDocument
                }
                try archive.document.validate()
                return archive
            }.sorted { $0.savedAt > $1.savedAt }
    }

    private func archiveDirectory(for meetingID: UUID) -> URL {
        directory.appendingPathComponent("History", isDirectory: true).appendingPathComponent(meetingID.uuidString, isDirectory: true)
    }

    /// `previous` is the validated bytes the caller just read; they become the backup.
    private func write(_ document: TranscriptDocument, for meetingID: UUID, replacing previous: Data?) throws {
        try document.validate()
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = fileURL(for: meetingID)
        if let previous {
            try previous.write(to: url.appendingPathExtension("backup"), options: .atomic)
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let data = try encoder.encode(Envelope(meetingID: meetingID, document: document))
        try data.write(to: url, options: .atomic)
    }

    func fileURL(for meetingID: UUID) -> URL {
        directory.appendingPathComponent(meetingID.uuidString).appendingPathExtension("json")
    }
}

extension TranscriptDocument {
    static func preservingLegacy(_ segments: [TranscriptSegment]) throws -> TranscriptDocument {
        var speakers: [MeetingSpeaker] = []
        var ids: [String: UUID] = [:]
        var utterances: [DocumentUtterance] = []
        for segment in segments {
            let source: String?
            switch segment.speaker {
            case "내 마이크": source = "microphone"
            case "원격 오디오": source = "system"
            default: source = nil
            }
            let label = segment.speaker.trimmingCharacters(in: .whitespacesAndNewlines)
            var speakerID: UUID?
            if source == nil && !label.isEmpty {
                if let existing = ids[label] {
                    speakerID = existing
                } else {
                    let speaker = MeetingSpeaker(name: label, order: speakers.count)
                    speakers.append(speaker)
                    ids[label] = speaker.id
                    speakerID = speaker.id
                }
            }
            utterances.append(DocumentUtterance(
                id: segment.id, startTime: segment.startTime, endTime: segment.endTime,
                rawText: segment.text, sourceChannelID: source, engineClusterID: nil,
                speakerID: speakerID, editedText: segment.revisedText
            ))
        }
        return try TranscriptDocument(speakers: speakers, utterances: utterances)
    }
}
