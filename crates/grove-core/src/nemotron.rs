use crate::{
    CoreError, Result, Transcript, TranscriptSegment, TranscriptSource, TranscriptVersion,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(crate) const PROJECTOR_VERSION: &str = "nemotron-asr-sentences-v1";
const RUNTIME: &str = "nemo-speech-0.2.0@6a3ca369370782acee0dd155e34394ba60b920c4";
const POLICY: &str = "whole-recording-stateful-v1";
const SAMPLE_RATE: u32 = 16000;
// Pinned transcribe.cpp uses defaultfloat's six significant digits. Duration is
// float32; word endpoints first pass through integer milliseconds.
fn decimal_rounding(seconds: f64) -> f64 {
    if seconds == 0.0 {
        0.0
    } else {
        0.5 * 10_f64.powi(seconds.abs().log10().floor() as i32 - 5)
    }
}

fn float_rounding(value: f64) -> f64 {
    let rounded = value as f32;
    let next = f32::from_bits(rounded.to_bits() + 1);
    f64::from(next - rounded) / 2.0
}

fn duration_rounding(frames: u64, rendered: f64) -> f64 {
    let seconds = frames as f64 / SAMPLE_RATE as f64;
    decimal_rounding(seconds.max(rendered))
        + float_rounding(seconds)
        + float_rounding(frames as f64) / SAMPLE_RATE as f64
}

fn word_rounding(seconds: f64, rendered: f64) -> f64 {
    decimal_rounding(seconds.max(rendered)) + 0.5 / 1000.0
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Output {
    schema_version: u32,
    model: String,
    model_revision: String,
    runtime: String,
    processing_policy: String,
    completion_evidence: String,
    completed: bool,
    input_audio_frames: u64,
    audio_sample_rate: u32,
    audio_duration_seconds: f64,
    native_duration_seconds: f64,
    words: Vec<Word>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Word {
    word: String,
    start: f64,
    end: f64,
    #[serde(default)]
    confidence: Option<f64>,
}

pub(crate) fn decode(raw: &[u8], duration: f64, session_id: &str, id: &str) -> Result<Transcript> {
    let invalid = || {
        CoreError::InvalidInput(
            "음성 인식의 전체 입력 또는 완료 정보를 확인할 수 없습니다. 모델 원본은 보관했습니다."
                .into(),
        )
    };
    if raw.len() > 16 * 1024 * 1024 {
        return Err(CoreError::InvalidInput(
            "음성 인식 결과가 너무 큽니다.".into(),
        ));
    }
    let output: Output = serde_json::from_slice(raw).map_err(|_| invalid())?;
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../../config/models/nemotron-asr.json"))
            .map_err(|error| CoreError::Storage(error.to_string()))?;
    let input_duration = output.input_audio_frames as f64 / SAMPLE_RATE as f64;
    if output.schema_version != 1
        || Some(output.model.as_str()) != manifest["repository"].as_str()
        || Some(output.model_revision.as_str()) != manifest["revision"].as_str()
        || output.runtime != RUNTIME
        || output.processing_policy != POLICY
        || output.completion_evidence != "native-stream-finish-exit0-v1"
        || !output.completed
        || output.input_audio_frames == 0
        || output.audio_sample_rate != SAMPLE_RATE
        || !duration.is_finite()
        || duration <= 0.0
        || !output.audio_duration_seconds.is_finite()
        || (output.audio_duration_seconds - input_duration).abs() > f64::EPSILON * input_duration
        || (duration - input_duration).abs() > 1.0 / SAMPLE_RATE as f64
        || !output.native_duration_seconds.is_finite()
        || (output.native_duration_seconds - input_duration).abs()
            > duration_rounding(output.input_audio_frames, output.native_duration_seconds)
    {
        return Err(invalid());
    }
    if output.words.is_empty() {
        return Err(CoreError::InvalidInput(
            "인식된 발화가 없습니다. 원본 음성은 보관했습니다.".into(),
        ));
    }
    let mut segments = Vec::new();
    let mut sentence = Vec::new();
    let mut sentence_start = 0.0;
    let mut previous_start = 0.0;
    let mut previous_end = 0.0;
    let mut sentence_end = 0.0;
    for word in output.words {
        if word.word.trim().is_empty()
            || word.word.contains('\0')
            || word.word.len() > 2 * 1024 * 1024
            || !word.start.is_finite()
            || !word.end.is_finite()
            || word.start < 0.0
            || word.end < word.start
            || word.start < previous_start
            || word.end < previous_end
            || word.end > input_duration + word_rounding(input_duration, word.end)
            || word.confidence.is_some_and(|value| !value.is_finite())
        {
            return Err(CoreError::InvalidInput(
                "음성 인식의 단어 또는 시각이 올바르지 않습니다. 모델 원본은 보관했습니다.".into(),
            ));
        }
        if sentence.is_empty() {
            sentence_start = word.start.min(duration);
        }
        previous_start = word.start;
        previous_end = word.end;
        sentence_end = word.end.min(duration);
        let ends_sentence = word.word.trim_end().ends_with(['.', '!', '?']);
        sentence.push(word.word);
        if ends_sentence {
            push_sentence(&mut segments, &mut sentence, sentence_start, sentence_end)?;
        }
    }
    if !sentence.is_empty() {
        push_sentence(&mut segments, &mut sentence, sentence_start, sentence_end)?;
    }
    Ok(Transcript {
        version: TranscriptVersion {
            id: id.into(),
            revision: 0,
        },
        source_session_id: session_id.into(),
        model: output.model,
        model_revision: output.model_revision,
        source: Some(TranscriptSource {
            raw_sha256: format!("{:x}", Sha256::digest(raw)),
            projector_version: PROJECTOR_VERSION.into(),
            association: "direct".into(),
            speaker_scope: None,
            chunking_policy: Some(POLICY.into()),
        }),
        speakers: Vec::new(),
        segments,
        speaker_analysis: None,
    })
}

fn push_sentence(
    segments: &mut Vec<TranscriptSegment>,
    words: &mut Vec<String>,
    start: f64,
    end: f64,
) -> Result<()> {
    let text = words.join(" ");
    if text.len() > 2 * 1024 * 1024 {
        return Err(CoreError::InvalidInput(
            "음성 인식의 문장이 너무 깁니다.".into(),
        ));
    }
    segments.push(TranscriptSegment {
        id: Uuid::new_v4().to_string(),
        start,
        end,
        original_text: text.clone(),
        text,
        speaker_id: None,
        source_index: Some(
            u32::try_from(segments.len()).map_err(|_| {
                CoreError::InvalidInput("음성 인식의 문장 수가 너무 많습니다.".into())
            })?,
        ),
    });
    words.clear();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::tests::{output as moss_output, setup};

    fn value() -> serde_json::Value {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../config/models/nemotron-asr.json")).unwrap();
        serde_json::json!({
            "schemaVersion":1,
            "model":manifest["repository"],"modelRevision":manifest["revision"],
            "runtime":RUNTIME,"processingPolicy":POLICY,
            "completionEvidence":"native-stream-finish-exit0-v1","completed":true,
            "inputAudioFrames":16000,"audioSampleRate":16000,
            "audioDurationSeconds":1.0,"nativeDurationSeconds":1.0,
            "words":[
                {"word":"첫","start":0.0,"end":0.15,"confidence":1.0},
                {"word":"문장.","start":0.15,"end":0.4},
                {"word":"둘째","start":0.45,"end":0.6,"confidence":null},
                {"word":"문장?","start":0.6,"end":0.8},
                {"word":"마지막","start":0.85,"end":1.0}
            ]
        })
    }

    fn raw() -> Vec<u8> {
        serde_json::to_vec(&value()).unwrap()
    }

    fn project(value: serde_json::Value) -> Result<Transcript> {
        decode(&serde_json::to_vec(&value).unwrap(), 1.0, "session", "id")
    }

    #[test]
    fn punctuation_projection_preserves_words_native_ranges_and_absent_speakers() {
        let raw = raw();
        let projected = decode(&raw, 1.0, "session", "id").unwrap();
        assert_eq!(projected.segments.len(), 3);
        assert_eq!(projected.segments[0].text, "첫 문장.");
        assert_eq!(projected.segments[0].start, 0.0);
        assert_eq!(projected.segments[0].end, 0.4);
        assert_eq!(projected.segments[1].text, "둘째 문장?");
        assert_eq!(projected.segments[1].start, 0.45);
        assert_eq!(projected.segments[1].end, 0.8);
        assert_eq!(projected.segments[2].text, "마지막");
        assert!(projected.speakers.is_empty());
        assert!(projected.segments.iter().all(|segment| {
            segment.speaker_id.is_none() && segment.text == segment.original_text
        }));
        assert_eq!(projected.segments[2].source_index, Some(2));
        let source = projected.source.unwrap();
        assert_eq!(source.projector_version, PROJECTOR_VERSION);
        assert_eq!(source.raw_sha256, format!("{:x}", Sha256::digest(&raw)));
        assert_eq!(source.association, "direct");
        assert_eq!(source.speaker_scope, None);
        assert_eq!(source.chunking_policy.as_deref(), Some(POLICY));
    }

    #[test]
    fn whole_input_identity_and_terminal_evidence_are_required() {
        for (field, invalid) in [
            ("schemaVersion", serde_json::json!(2)),
            ("model", serde_json::json!("other-model")),
            ("modelRevision", serde_json::json!("other-revision")),
            ("runtime", serde_json::json!("other-runtime")),
            ("processingPolicy", serde_json::json!("chunked")),
            (
                "completionEvidence",
                serde_json::json!("inferred-from-last-word"),
            ),
            ("completed", serde_json::json!(false)),
            ("inputAudioFrames", serde_json::json!(0)),
            ("inputAudioFrames", serde_json::json!(8000)),
            ("audioSampleRate", serde_json::json!(48000)),
            ("audioDurationSeconds", serde_json::json!(1.01)),
            ("nativeDurationSeconds", serde_json::json!(0.98)),
            ("nativeDurationSeconds", serde_json::json!(1.02)),
        ] {
            let mut candidate = value();
            candidate[field] = invalid;
            assert!(project(candidate).is_err(), "accepted field {field}");
        }
        let mut missing = value();
        missing
            .as_object_mut()
            .unwrap()
            .remove("completionEvidence");
        assert!(project(missing).is_err());
        assert!(decode(&raw(), 2.0, "session", "id").is_err());
    }

    #[test]
    fn malformed_word_ranges_and_bodies_are_rejected() {
        for (field, invalid) in [
            ("start", serde_json::json!(-0.01)),
            ("start", serde_json::json!(0.3)),
            ("end", serde_json::json!(-0.01)),
            ("end", serde_json::json!(1.02)),
            ("word", serde_json::json!(" ")),
            ("word", serde_json::json!("bad\0word")),
            ("confidence", serde_json::json!("not-a-number")),
        ] {
            let mut candidate = value();
            candidate["words"][0][field] = invalid;
            assert!(project(candidate).is_err(), "accepted word field {field}");
        }
        for field in ["start", "end"] {
            let mut candidate = value();
            candidate["words"][2][field] = serde_json::json!(0.01);
            assert!(
                project(candidate).is_err(),
                "accepted non-monotonic {field}"
            );
        }
        let mut empty = value();
        empty["words"] = serde_json::json!([]);
        assert!(project(empty).is_err());
        let mut invalid = String::from_utf8(raw()).unwrap();
        invalid = invalid.replace(
            "\"nativeDurationSeconds\":1.0",
            "\"nativeDurationSeconds\":1e999",
        );
        assert!(decode(invalid.as_bytes(), 1.0, "session", "id").is_err());
    }

    #[test]
    fn only_native_rounding_can_be_normalized_and_silence_is_not_a_token_cap() {
        let mut rounded = value();
        rounded["inputAudioFrames"] = serde_json::json!(16008);
        rounded["audioDurationSeconds"] = serde_json::json!(1.0005);
        rounded["nativeDurationSeconds"] = serde_json::json!(1.0005);
        rounded["words"][4]["end"] = serde_json::json!(1.001);
        let projected = decode(
            &serde_json::to_vec(&rounded).unwrap(),
            1.0005,
            "session",
            "id",
        )
        .unwrap();
        assert_eq!(projected.segments.last().unwrap().end, 1.0005);
        let mut not_rounding = value();
        not_rounding["nativeDurationSeconds"] = serde_json::json!(1.01);
        assert!(project(not_rounding).is_err());
        let mut silent_tail = value();
        silent_tail["words"] = serde_json::json!([
            {"word":"짧은 발언.","start":0.0,"end":0.1}
        ]);
        assert!(project(silent_tail).is_ok());
    }

    #[test]
    fn six_significant_digit_rounding_supports_recordings_longer_than_ten_thousand_seconds() {
        let frames = 160_002_800_u64;
        let duration = frames as f64 / SAMPLE_RATE as f64;
        let mut candidate = value();
        candidate["inputAudioFrames"] = serde_json::json!(frames);
        candidate["audioDurationSeconds"] = serde_json::json!(duration);
        candidate["nativeDurationSeconds"] = serde_json::json!(10000.2);
        candidate["words"] = serde_json::json!([
            {"word":"긴 녹음.","start":9999.0,"end":10000.2}
        ]);
        let projected = decode(
            &serde_json::to_vec(&candidate).unwrap(),
            duration,
            "session",
            "id",
        )
        .unwrap();
        assert_eq!(projected.segments[0].end, duration);
        candidate["nativeDurationSeconds"] = serde_json::json!(10000.3);
        assert!(decode(
            &serde_json::to_vec(&candidate).unwrap(),
            duration,
            "session",
            "id"
        )
        .is_err());
        candidate["nativeDurationSeconds"] = serde_json::json!(10000.2);
        candidate["words"][0]["end"] = serde_json::json!(10000.3);
        assert!(decode(
            &serde_json::to_vec(&candidate).unwrap(),
            duration,
            "session",
            "id"
        )
        .is_err());
    }

    #[test]
    fn model_switch_and_restore_keep_both_edited_histories_and_immutable_raw() {
        let (_temp, mut engine, id, _) = setup();
        let moss = engine.commit_transcript(&id, None, moss_output()).unwrap();
        let moss = engine
            .edit_transcript(&id, moss.version, vec!["기존 수정".into()])
            .unwrap();
        let nemo = engine
            .commit_nemotron_transcript(&id, Some(moss.version.clone()), raw())
            .unwrap();
        let initial_source = nemo.source.clone();
        let nemo = engine
            .edit_transcript(
                &id,
                nemo.version,
                vec!["첫 문장 수정.".into(), "둘째 문장?".into(), "마지막".into()],
            )
            .unwrap();
        assert_eq!(nemo.source, initial_source);
        assert_eq!(engine.transcript_history(&id).unwrap().entries.len(), 2);
        let restored = engine
            .restore_transcript(&id, nemo.version.clone(), moss.version)
            .unwrap();
        assert_eq!(restored.segments[0].text, "기존 수정");
        let restored = engine
            .restore_transcript(&id, restored.version, nemo.version)
            .unwrap();
        assert_eq!(restored.segments[0].text, "첫 문장 수정.");
        assert_eq!(restored.source, initial_source);
        assert!(restored.speakers.is_empty());
        let directory = engine
            .store
            .root
            .join("meetings")
            .join(&id)
            .join("transcripts")
            .join(&restored.version.id);
        assert_eq!(std::fs::read(directory.join("raw.json")).unwrap(), raw());
        assert_eq!(engine.transcript_history(&id).unwrap().unavailable, 0);
    }

    #[test]
    fn stale_or_invalid_recognition_preserves_current_result_and_failed_raw() {
        let (_temp, mut engine, id, _) = setup();
        let first = engine.commit_nemotron_transcript(&id, None, raw()).unwrap();
        let edited = engine
            .edit_transcript(
                &id,
                first.version.clone(),
                vec!["사용자 수정".into(), "둘째".into(), "마지막".into()],
            )
            .unwrap();
        assert!(engine
            .commit_nemotron_transcript(&id, Some(first.version), raw())
            .is_err());
        assert!(engine
            .commit_nemotron_transcript(&id, Some(edited.version.clone()), b"invalid".to_vec())
            .is_err());
        let current = engine.transcript(&id).unwrap().unwrap();
        assert_eq!(current.version, edited.version);
        assert_eq!(current.segments[0].text, "사용자 수정");
        let directory = engine
            .store
            .root
            .join("meetings")
            .join(&id)
            .join("transcripts");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 3);
        assert_eq!(engine.transcript_history(&id).unwrap().unavailable, 2);
    }

    #[test]
    fn raw_models_require_their_own_adapter_and_tampering_is_not_repaired() {
        let (_temp, mut engine, id, _) = setup();
        assert!(engine.commit_transcript(&id, None, raw()).is_err());
        assert!(engine.commit_whole_transcript(&id, None, raw()).is_err());
        assert!(engine
            .commit_nemotron_transcript(&id, None, moss_output())
            .is_err());
        let nemo = engine.commit_nemotron_transcript(&id, None, raw()).unwrap();
        let changed = engine
            .commit_nemotron_transcript(&id, Some(nemo.version.clone()), raw())
            .unwrap();
        let path = engine
            .store
            .root
            .join("meetings")
            .join(&id)
            .join("transcripts")
            .join(&nemo.version.id)
            .join("raw.json");
        let mut forged = value();
        forged["words"][0]["word"] = serde_json::json!("다른");
        let forged = serde_json::to_vec(&forged).unwrap();
        std::fs::write(&path, &forged).unwrap();
        assert!(engine
            .restore_transcript(&id, changed.version.clone(), nemo.version)
            .is_err());
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            changed.version
        );
        assert_eq!(std::fs::read(path).unwrap(), forged);
    }
}
