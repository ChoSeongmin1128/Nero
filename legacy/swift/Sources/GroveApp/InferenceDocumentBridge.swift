import Foundation
import GroveInference

extension TranscriptDocument {
    static func preservingInference(_ result: InferenceResult, sourceChannelID: String) throws -> TranscriptDocument {
        try result.validate()
        let orderedClusters = result.rawDiarization.sorted { $0.start < $1.start }.reduce(into: [String]()) { ids, turn in
            if !ids.contains(turn.clusterID) { ids.append(turn.clusterID) }
        }
        let speakers = orderedClusters.enumerated().map { MeetingSpeaker(name: MeetingSpeaker.defaultName(order: $0.offset), order: $0.offset) }
        let speakerByCluster = Dictionary(uniqueKeysWithValues: zip(orderedClusters, speakers.map(\.id)))
        let assignmentByUtterance = Dictionary(uniqueKeysWithValues: result.assignments.map { ($0.utteranceID, $0) })
        let utterances = result.transcription.utterances.map { utterance in
            let assignment = assignmentByUtterance[utterance.id]
            return DocumentUtterance(id: utterance.id, startTime: utterance.start, endTime: utterance.end,
                rawText: utterance.text, sourceChannelID: sourceChannelID,
                engineClusterID: assignment?.clusterID,
                speakerID: assignment?.clusterID.flatMap { speakerByCluster[$0] }, editedText: nil,
                asrClusterID: utterance.asrClusterID, asrChunkIndex: utterance.asrChunkIndex,
                assignmentReviewReasons: assignment?.reviewReasons,
                assignmentEvidence: assignment.map {
                    SpeakerAssignmentEvidence(utteranceID: utterance.id, startTime: utterance.start, endTime: utterance.end,
                        sourceChannelID: sourceChannelID, assignedClusterID: $0.clusterID,
                        overlapSecondsByCluster: $0.overlapSecondsByCluster)
                })
        }
        return try TranscriptDocument(speakers: speakers, utterances: utterances, revisionID: result.jobID,
                                      sourceDiarizationEngines: [sourceChannelID: result.diarizationEngine])
    }
}
