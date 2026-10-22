import Foundation

enum MossOutputValidation {
    private struct Segment: Equatable {
        let start: Double
        let end: Double
        let text: String
        let speaker: String
    }

    static func validatedSegments(_ raw: String, segments: [[String: Any]]) -> [[String: Any]]? {
        isComplete(raw, segments: segments) ? segments : nil
    }

    static func isComplete(_ raw: String, segments: [[String: Any]]) -> Bool {
        let pattern = #"\[(\d+(?:[\.,]\d+)?)\]\[(S\d+)\](.*?)\[(\d+(?:[\.,]\d+)?)\]"#
        guard let regex = try? NSRegularExpression(pattern: pattern, options: [.dotMatchesLineSeparators]) else { return false }
        let source = raw as NSString
        let range = NSRange(location: 0, length: source.length)
        let remainder = regex.stringByReplacingMatches(in: raw, range: range, withTemplate: "")
        guard remainder.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }

        var expected: [Segment] = []
        for match in regex.matches(in: raw, range: range) {
            guard let start = timestamp(source.substring(with: match.range(at: 1))),
                  let end = timestamp(source.substring(with: match.range(at: 4))),
                  start >= 0, end >= start else { return false }
            let speaker = source.substring(with: match.range(at: 2))
            let body = source.substring(with: match.range(at: 3))
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if body.isEmpty { continue }
            expected.append(Segment(start: start, end: end, text: "[\(speaker)] \(body)", speaker: speaker))
        }

        var actual: [Segment] = []
        for segment in segments {
            guard let start = segment["start"] as? Double, start.isFinite,
                  let end = segment["end"] as? Double, end.isFinite,
                  start >= 0, end >= start,
                  let text = segment["text"] as? String else { return false }
            // The pinned parser returns an empty fallback for silence; the worker omits it.
            if text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { continue }
            guard let speaker = segment["speaker_id"] as? String else { return false }
            actual.append(Segment(start: start, end: end, text: text, speaker: speaker))
        }
        return actual == expected
    }

    private static func timestamp(_ value: String) -> Double? {
        guard let number = Double(value.replacingOccurrences(of: ",", with: ".")), number.isFinite else { return nil }
        return number
    }
}
