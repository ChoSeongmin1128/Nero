use crate::{
    CoreError, Result, Transcript, TranscriptSegment, TranscriptSource, TranscriptVersion,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(crate) const MODEL: &str = "apple/SpeechTranscriber";
pub(crate) const DICTATION_MODEL: &str = "apple/DictationTranscriber";
pub(crate) const PROJECTOR: &str = "apple-native-results-v1";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Output {
    #[serde(default, rename = "glossaryContext")]
    _glossary_context: Option<serde_json::Value>,
    schema_version: u32,
    model: String,
    model_revision: String,
    runtime: String,
    processing_policy: String,
    completion_evidence: String,
    completed: bool,
    input_audio_frames: u64,
    audio_sample_rate: u32,
    input_audio_sha256: String,
    helper_sha256: String,
    native_result_sha256: String,
    native_result_json: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Native {
    schema_version: u32,
    variant: String,
    locale: String,
    system_version: String,
    input_frames: u64,
    consumed_input_frames: u64,
    sample_rate: f64,
    finalized: bool,
    finalization_time_seconds: f64,
    segments: Vec<Segment>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Segment {
    start: f64,
    end: f64,
    text: String,
}
fn invalid() -> CoreError {
    CoreError::InvalidInput("macOS 전사의 전체 입력, 완료 정보 또는 원본 시각을 확인할 수 없습니다. 원음과 기존 기록은 유지됩니다.".into())
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
pub(crate) fn decode(raw: &[u8], duration: f64, session: &str, id: &str) -> Result<Transcript> {
    if raw.len() > 16 * 1024 * 1024 {
        return Err(invalid());
    }
    let output: Output = serde_json::from_slice(raw).map_err(|_| invalid())?;
    let native: Native = serde_json::from_str(&output.native_result_json).map_err(|_| invalid())?;
    let input_duration = output.input_audio_frames as f64 / 16000.0;
    if output.schema_version != 1
        || !matches!(
            (output.model.as_str(), native.variant.as_str()),
            (MODEL, "speechTranscriber") | (DICTATION_MODEL, "dictationTranscriber")
        )
        || output.model_revision != format!("{}:ko-KR", native.system_version)
        || native.system_version.is_empty()
        || native.system_version.len() > 256
        || output.runtime != "apple-speech-analyzer-v1"
        || output.processing_policy != "whole-recording-native-results-v1"
        || output.completion_evidence != "speech-analyzer-finalized-file-exit0-v1"
        || !output.completed
        || output.input_audio_frames == 0
        || output.audio_sample_rate != 16000
        || !duration.is_finite()
        || duration <= 0.0
        || (duration - input_duration).abs() > 1.0 / 16000.0
        || !hash(&output.input_audio_sha256)
        || !hash(&output.helper_sha256)
        || output.native_result_sha256
            != format!("{:x}", Sha256::digest(output.native_result_json.as_bytes()))
        || native.schema_version != 1
        || native.locale != "ko-KR"
        || !native.finalized
        || native.input_frames != output.input_audio_frames
        || native.consumed_input_frames != output.input_audio_frames
        || native.sample_rate != 16000.0
        || !native.finalization_time_seconds.is_finite()
        || native.finalization_time_seconds < 0.0
        || native.finalization_time_seconds > input_duration
        || native.segments.is_empty()
        || native.segments.len() > 100000
    {
        return Err(invalid());
    }
    let mut previous_start = 0.0;
    let mut previous_end = 0.0;
    let mut segments = Vec::new();
    for segment in native.segments {
        if segment.text.trim().is_empty()
            || segment.text.contains('\0')
            || segment.text.len() > 2 * 1024 * 1024
            || !segment.start.is_finite()
            || !segment.end.is_finite()
            || segment.start < previous_start
            || segment.end < previous_end
            || segment.end < segment.start
            || segment.end > input_duration
        {
            return Err(invalid());
        }
        previous_start = segment.start;
        previous_end = segment.end;
        // The analysis WAV may run up to one 16 kHz frame past the recording; speaker
        // projection measures against the recording, so times stop at its length.
        segments.push(TranscriptSegment {
            id: Uuid::new_v4().to_string(),
            start: segment.start.min(duration),
            end: segment.end.min(duration),
            original_text: segment.text.clone(),
            text: segment.text,
            speaker_id: None,
            source_index: Some(u32::try_from(segments.len()).map_err(|_| invalid())?),
        });
    }
    Ok(Transcript {
        version: TranscriptVersion {
            id: id.into(),
            revision: 0,
        },
        source_session_id: session.into(),
        model: output.model,
        model_revision: output.model_revision,
        source: Some(TranscriptSource {
            raw_sha256: format!("{:x}", Sha256::digest(raw)),
            projector_version: PROJECTOR.into(),
            association: "direct".into(),
            speaker_scope: None,
            chunking_policy: Some(output.processing_policy),
            recognition_report: None,
        }),
        speakers: Vec::new(),
        segments,
        speaker_analysis: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn raw() -> serde_json::Value {
        let native =
            serde_json::json!({"schemaVersion":1,"variant":"speechTranscriber","locale":"ko-KR","systemVersion":"macOS test",
            "inputFrames":32000,"consumedInputFrames":32000,"sampleRate":16000.0,"finalized":true,
            "finalizationTimeSeconds":2.0,"segments":[{"start":0.25,"end":1.5,"text":"회의 기록"}]})
            .to_string();
        serde_json::json!({"schemaVersion":1,"model":MODEL,"modelRevision":"macOS test:ko-KR",
            "runtime":"apple-speech-analyzer-v1","processingPolicy":"whole-recording-native-results-v1",
            "completionEvidence":"speech-analyzer-finalized-file-exit0-v1","completed":true,
            "inputAudioFrames":32000,"audioSampleRate":16000,"inputAudioSha256":"a".repeat(64),
            "helperSha256":"b".repeat(64),"nativeResultSha256":format!("{:x}",Sha256::digest(native.as_bytes())),"nativeResultJson":native})
    }
    #[test]
    fn preserves_native_result_ranges_without_word_timing_or_speakers() {
        let doc = decode(&serde_json::to_vec(&raw()).unwrap(), 2.0, "session", "id").unwrap();
        assert_eq!(doc.segments[0].start, 0.25);
        assert_eq!(doc.segments[0].end, 1.5);
        assert_eq!(doc.segments[0].text, "회의 기록");
        assert!(doc.segments[0].speaker_id.is_none());
        assert!(doc.speakers.is_empty());
    }
    #[test]
    fn dictation_provenance_is_distinct_and_cannot_claim_another_variant() {
        let mut value = raw();
        let mut native: serde_json::Value =
            serde_json::from_str(value["nativeResultJson"].as_str().unwrap()).unwrap();
        native["variant"] = serde_json::json!("dictationTranscriber");
        let native = native.to_string();
        value["nativeResultSha256"] =
            serde_json::json!(format!("{:x}", Sha256::digest(native.as_bytes())));
        value["nativeResultJson"] = serde_json::json!(native);
        assert!(decode(&serde_json::to_vec(&value).unwrap(), 2.0, "session", "id").is_err());
        value["model"] = serde_json::json!(DICTATION_MODEL);
        let doc = decode(&serde_json::to_vec(&value).unwrap(), 2.0, "session", "id").unwrap();
        assert_eq!(doc.model, DICTATION_MODEL);
    }
    #[test]
    fn stored_native_segments_support_edits_history_and_optional_coverage_speakers() {
        let (_temporary, mut engine, id, _) = crate::transcript::tests::setup();
        let mut value = raw();
        let mut native: serde_json::Value =
            serde_json::from_str(value["nativeResultJson"].as_str().unwrap()).unwrap();
        native["inputFrames"] = serde_json::json!(16000);
        native["consumedInputFrames"] = serde_json::json!(16000);
        native["finalizationTimeSeconds"] = serde_json::json!(1.0);
        native["segments"][0]["end"] = serde_json::json!(0.75);
        let native = native.to_string();
        value["inputAudioFrames"] = serde_json::json!(16000);
        value["nativeResultSha256"] =
            serde_json::json!(format!("{:x}", Sha256::digest(native.as_bytes())));
        value["nativeResultJson"] = serde_json::json!(native);
        let first = engine
            .commit_apple_transcript(&id, None, serde_json::to_vec(&value).unwrap())
            .unwrap();
        assert!(engine.transcript_history(&id).is_ok());
        assert!(first.speakers.is_empty());
        let edited = engine
            .edit_transcript(&id, first.version, vec!["수정한 기록".into()])
            .unwrap();
        let draft = crate::SpeakerAnalysisDraft {
            source: edited.clone(),
            model: "example/diarizer".into(),
            model_revision: "test".into(),
            model_sha256: "a".repeat(64),
            runtime: "test-runtime".into(),
            configuration: serde_json::json!({"scope":"recording"}),
            posterior: None,
            raw: serde_json::to_vec(
                &serde_json::json!({"segments":[{"start":0.25,"end":0.75,"speaker":1}]}),
            )
            .unwrap(),
        };
        let assigned = engine.apply_diarization(&id, draft).unwrap();
        assert_eq!(assigned.segments[0].start, 0.25);
        assert_eq!(assigned.segments[0].end, 0.75);
        assert_eq!(assigned.segments[0].text, "수정한 기록");
        assert!(assigned.segments[0].speaker_id.is_some());
        assert!(engine.transcript_history(&id).is_ok());
        let mut conflict = assigned.clone();
        conflict.segments[0].speaker_id = None;
        conflict.speakers.clear();
        let draft = crate::SpeakerAnalysisDraft {
            source: conflict,
            model: "example/diarizer".into(),
            model_revision: "test".into(),
            model_sha256: "a".repeat(64),
            runtime: "test-runtime".into(),
            configuration: serde_json::json!({"scope":"recording"}),
            posterior: None,
            raw: serde_json::to_vec(&serde_json::json!({"segments":[
                {"start":0.25,"end":0.5,"speaker":1},{"start":0.5,"end":0.75,"speaker":2}]}))
            .unwrap(),
        };
        let (unknown, _) = crate::diarization::project(draft, 16000, 16000).unwrap();
        assert!(unknown.segments[0].speaker_id.is_none());
        assert_eq!(
            unknown.speaker_analysis.unwrap().review_causes["multiple_speakers"],
            1
        );
        assert_eq!(
            serde_json::to_value(engine.transcript(&id).unwrap().unwrap()).unwrap(),
            serde_json::to_value(assigned).unwrap()
        );
    }
    #[test]
    fn rejects_incomplete_input_modified_native_output_and_wrong_source_duration() {
        let mut value = raw();
        value["inputAudioFrames"] = serde_json::json!(16000);
        assert!(decode(&serde_json::to_vec(&value).unwrap(), 2.0, "session", "id").is_err());
        let mut value = raw();
        value["nativeResultJson"] = serde_json::json!("{}");
        assert!(decode(&serde_json::to_vec(&value).unwrap(), 2.0, "session", "id").is_err());
        assert!(decode(&serde_json::to_vec(&raw()).unwrap(), 1.0, "session", "id").is_err());
    }
}
