import Foundation

struct MeetingRecordStorage {
    let url: URL

    func load() throws -> [MeetingRecord] {
        guard FileManager.default.fileExists(atPath: url.path) else { return [] }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let records = try decoder.decode([MeetingRecord].self, from: Data(contentsOf: url))
        guard Set(records.map(\.id)).count == records.count else { throw TranscriptEditError.invalidDocument }
        try records.forEach { try $0.validateProcessingMetadata(); try $0.attendance?.validate() }
        return records
    }

    func save(_ records: [MeetingRecord]) throws {
        guard Set(records.map(\.id)).count == records.count else { throw TranscriptEditError.invalidDocument }
        try records.forEach { try $0.validateProcessingMetadata(); try $0.attendance?.validate() }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        encoder.dateEncodingStrategy = .iso8601
        let data = try encoder.encode(records)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        if FileManager.default.fileExists(atPath: url.path) {
            _ = try load()
            try Data(contentsOf: url).write(to: url.appendingPathExtension("backup"), options: .atomic)
        }
        try data.write(to: url, options: .atomic)
    }
}
