import Foundation

enum TranscriptExportFormat: String, CaseIterable, Identifiable, Sendable {
    case text
    case markdown

    var id: String { rawValue }
    var fileExtension: String { self == .text ? "txt" : "md" }
}

struct TranscriptExportOptions: Sendable {
    var format: TranscriptExportFormat = .text
    var includesSpeakers = true
    var includesTimestamps = true
    var selectedUtteranceIDs: Set<UUID>?
    var usesOriginalText = false
}

enum TranscriptRenderer {
    static func render(_ document: TranscriptDocument, options: TranscriptExportOptions = .init()) -> String {
        let document = exportDocument(document, original: options.usesOriginalText)
        let utterances = document.utterances.enumerated().filter {
            options.selectedUtteranceIDs?.contains($0.element.id) ?? true
        }.sorted {
            $0.element.startTime == $1.element.startTime ? $0.offset < $1.offset : $0.element.startTime < $1.element.startTime
        }.map(\.element)
        let blocks = utterances.map { utterance in
            var heading: [String] = []
            if options.includesTimestamps { heading.append("[\(timestamp(utterance.startTime))]") }
            if options.includesSpeakers {
                let name = speakerLabel(in: document, for: utterance)
                heading.append(options.format == .markdown ? "**\(escapeMarkdown(name))**" : name)
            }
            let body = options.usesOriginalText ? utterance.rawText : utterance.displayedText
            let text = options.format == .markdown ? escapeMarkdown(body) : body
            guard !heading.isEmpty else { return text }
            return heading.joined(separator: " ") + "\n" + text
        }
        return blocks.isEmpty ? "" : blocks.joined(separator: "\n\n") + "\n"
    }

    static func speakerLabel(in document: TranscriptDocument, for utterance: DocumentUtterance) -> String {
        let isInferred = document.speakers.first(where: { $0.id == utterance.speakerID })?.profileMatch?.isConfirmed == false
        return document.speakerName(for: utterance) + (isInferred ? " (추정)" : "")
    }

    static func timestamp(_ seconds: TimeInterval) -> String {
        let value = seconds.isFinite && seconds >= 0 && seconds < Double(Int.max) ? Int(seconds) : 0
        if value >= 3600 { return String(format: "%02d:%02d:%02d", value / 3600, value / 60 % 60, value % 60) }
        return String(format: "%02d:%02d", value / 60, value % 60)
    }

    static func exportDocument(_ document: TranscriptDocument, original: Bool) -> TranscriptDocument {
        guard original else { return document }
        var source = document
        while !source.undoHistory.isEmpty {
            do { try source.undo() } catch { break }
        }
        return source
    }

    static func escapeMarkdown(_ value: String) -> String {
        let special: Set<Character> = ["\\", "`", "*", "_", "{", "}", "[", "]", "(", ")", "#", "+", "-", ".", "!", "<", ">", "|", "~"]
        return value.map { special.contains($0) ? "\\\($0)" : String($0) }.joined()
    }
}
