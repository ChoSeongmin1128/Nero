import Foundation

public enum SpeakerProjection {
    public static func validate(_ turns: [DiarizationTurn], duration: Double, engine: DiarizationEngine) throws {
        if engine == .none && !turns.isEmpty { throw InferenceError.invalidOutput("화자 구분 없는 전사에 화자 결과를 넣을 수 없습니다.") }
        guard duration.isFinite, duration > 0,
              turns.allSatisfy({
                  $0.start.isFinite && $0.end.isFinite && $0.start >= 0 && $0.end > $0.start
                      && $0.end <= duration + 0.5 && !$0.clusterID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
              }) else { throw InferenceError.invalidOutput("화자 구간 또는 시각이 올바르지 않습니다.") }
        if engine == .sortformerStreaming && Set(turns.map(\.clusterID)).count > 4 {
            throw InferenceError.invalidOutput("Sortformer의 지원 인원수를 넘는 결과입니다.")
        }
        if (engine == .ultra8 || engine == .nemotron3) && Set(turns.map(\.clusterID)).count > 8 {
            throw InferenceError.invalidOutput("지원 인원수를 넘는 결과입니다.")
        }
    }

    public static func assign(_ utterances: [RecognizedUtterance], turns: [DiarizationTurn]) -> [UtteranceAssignment] {
        let grouped = Dictionary(grouping: turns, by: \.clusterID)
        return utterances.map { utterance in
            if utterance.start == utterance.end {
                let active = Set(turns.filter { $0.start <= utterance.start && utterance.start < $0.end }.map(\.clusterID))
                return UtteranceAssignment(utteranceID: utterance.id, clusterID: active.count == 1 ? active.first : nil,
                    overlapSecondsByCluster: [:], reviewReasons: active.count == 1 ? [] : [active.isEmpty ? .noDiarizationCoverage : .ambiguousSpeakers])
            }
            var overlaps: [String: Double] = [:]
            for (cluster, intervals) in grouped {
                let clipped = intervals.compactMap { turn -> (Double, Double)? in
                    let start = max(utterance.start, turn.start)
                    let end = min(utterance.end, turn.end)
                    return end > start ? (start, end) : nil
                }.sorted { $0.0 < $1.0 }
                // Union duplicate/overlapping intervals from one cluster before scoring.
                var total = 0.0
                var merged: (Double, Double)?
                for interval in clipped {
                    if let previous = merged {
                        if interval.0 <= previous.1 { merged = (previous.0, max(previous.1, interval.1)) }
                        else { total += previous.1 - previous.0; merged = interval }
                    } else { merged = interval }
                }
                if let merged { total += merged.1 - merged.0 }
                if total > 0 { overlaps[cluster] = total }
            }
            let ranked = overlaps.sorted { $0.value == $1.value ? $0.key < $1.key : $0.value > $1.value }
            guard let winner = ranked.first else {
                return UtteranceAssignment(utteranceID: utterance.id, clusterID: nil,
                    overlapSecondsByCluster: [:], reviewReasons: [.noDiarizationCoverage])
            }
            var reasons: Set<AssignmentReviewReason> = []
            if ranked.count > 1 {
                reasons.insert(.multipleSpeakersInUtterance)
                if abs(winner.value - ranked[1].value) < 0.001 { reasons.insert(.ambiguousSpeakers) }
            }
            return UtteranceAssignment(utteranceID: utterance.id,
                clusterID: reasons.contains(.ambiguousSpeakers) ? nil : winner.key,
                overlapSecondsByCluster: overlaps, reviewReasons: reasons)
        }
    }
}
