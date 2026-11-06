// Qwen3-ASR-ko runs on pause-bounded chunks; word times come only from the forced aligner.
use crate::nemotron::{Word, WordProjection};
use crate::{CoreError, Result, Transcript};
use serde::Deserialize;

pub(crate) const MODEL: &str = "seongmin0127/nero-qwen3-asr-ko-gguf";
pub(crate) const PROJECTOR_VERSION: &str = "qwen3-aligned-words-sentences-v1";
pub(crate) const POLICY: &str = "qwen3-pause-chunks-v1";
const COMPLETION: &str = "crispasr-exit0-v1";
const TIMESTAMPS: &str = "forced-aligner-text-units-v1";
const GAIN: &str = "window-gain-v1";
const SAMPLE_RATE: u64 = 16000;
const MAX_CHUNK_FRAMES: u64 = 60 * SAMPLE_RATE;
const MAX_PAD_FRAMES: u64 = 2 * SAMPLE_RATE;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Output {
    #[serde(default, rename = "glossaryContext")]
    _glossary_context: Option<serde_json::Value>,
    schema_version: u32,
    model: String,
    model_revision: String,
    model_sha256: String,
    aligner_sha256: String,
    runtime: String,
    target: String,
    processing_policy: String,
    completion_evidence: String,
    timestamp_policy: String,
    gain_policy: String,
    pad_frames: u64,
    completed: bool,
    input_audio_frames: u64,
    audio_sample_rate: u64,
    audio_duration_seconds: f64,
    input_pcm_sha256: String,
    chunks: Vec<Chunk>,
    words: Vec<Word>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Chunk {
    index: usize,
    start_frame: u64,
    end_frame: u64,
    input_pcm_sha256: String,
    native_result_sha256: String,
    native_exit_code: i32,
    actual_backend: String,
    backend_evidence: String,
    word_start: usize,
    word_count: usize,
}

fn invalid() -> CoreError {
    CoreError::InvalidInput("Qwen 전사의 전체 입력, 완료 정보 또는 정렬 단어 시각을 확인할 수 없습니다. 원음과 기존 기록은 유지됩니다.".into())
}

fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn decode_asr(raw: &[u8], duration: f64, session: &str, id: &str) -> Result<Transcript> {
    if raw.len() > 16 * 1024 * 1024 {
        return Err(invalid());
    }
    let output: Output = serde_json::from_slice(raw).map_err(|_| invalid())?;
    let pin: serde_json::Value =
        serde_json::from_str(include_str!("../../../config/models/qwen3-asr-ko.json"))
            .map_err(|_| invalid())?;
    let runtime: serde_json::Value = serde_json::from_str(include_str!(
        "../../../scripts/native-crispasr/runtime.json"
    ))
    .map_err(|_| invalid())?;
    let input_duration = output.input_audio_frames as f64 / SAMPLE_RATE as f64;
    if output.schema_version != 1
        || output.model != MODEL
        || Some(output.model.as_str()) != pin["repository"].as_str()
        || Some(output.model_revision.as_str()) != pin["revision"].as_str()
        || Some(output.model_sha256.as_str()) != pin["files"][0]["sha256"].as_str()
        || Some(output.aligner_sha256.as_str()) != pin["files"][1]["sha256"].as_str()
        || Some(output.runtime.as_str()) != runtime["identity"].as_str()
        || output.target != "macos-arm64"
        || output.processing_policy != POLICY
        || output.completion_evidence != COMPLETION
        || output.timestamp_policy != TIMESTAMPS
        || output.gain_policy != GAIN
        || output.pad_frames > MAX_PAD_FRAMES
        || !output.completed
        || output.input_audio_frames == 0
        || output.audio_sample_rate != SAMPLE_RATE
        || !output.audio_duration_seconds.is_finite()
        || (output.audio_duration_seconds - input_duration).abs() > f64::EPSILON * input_duration
        || !duration.is_finite()
        || duration <= 0.0
        || (duration - input_duration).abs() > 1.0 / SAMPLE_RATE as f64
        || !hash(&output.input_pcm_sha256)
        || output.chunks.is_empty()
        || output.chunks.len() > 10000
    {
        return Err(invalid());
    }
    let mut next_frame = 0;
    let mut next_word = 0_usize;
    for (index, chunk) in output.chunks.iter().enumerate() {
        let stop = next_word
            .checked_add(chunk.word_count)
            .ok_or_else(invalid)?;
        let backend_valid = matches!(
            (
                chunk.actual_backend.as_str(),
                chunk.backend_evidence.as_str()
            ),
            ("metal", "metal-backend-initialized") | ("cpu", "confirmed-gpu-unavailable")
        );
        if chunk.index != index
            || chunk.start_frame != next_frame
            || chunk.end_frame <= chunk.start_frame
            || chunk.end_frame > output.input_audio_frames
            || chunk.end_frame - chunk.start_frame > MAX_CHUNK_FRAMES
            || !hash(&chunk.input_pcm_sha256)
            || !hash(&chunk.native_result_sha256)
            || chunk.native_exit_code != 0
            || !backend_valid
            || chunk.word_start != next_word
            || stop > output.words.len()
        {
            return Err(invalid());
        }
        for word in &output.words[next_word..stop] {
            let start_frame = (word.start * SAMPLE_RATE as f64).round();
            let end_frame = (word.end * SAMPLE_RATE as f64).round();
            if word.confidence.is_some()
                || word.timing_scope.is_some()
                || word.native_segment_end_frame.is_some()
                || (word.start * SAMPLE_RATE as f64 - start_frame).abs() > 1e-6
                || (word.end * SAMPLE_RATE as f64 - end_frame).abs() > 1e-6
                || start_frame < chunk.start_frame as f64
                || end_frame > chunk.end_frame as f64
                || end_frame < start_frame
            {
                return Err(invalid());
            }
        }
        next_frame = chunk.end_frame;
        next_word = stop;
    }
    if next_frame != output.input_audio_frames || next_word != output.words.len() {
        return Err(invalid());
    }
    crate::nemotron::project_words(
        raw,
        duration,
        session,
        id,
        output.words,
        WordProjection {
            model: output.model,
            revision: output.model_revision,
            policy: output.processing_policy,
            projector: PROJECTOR_VERSION,
            input_duration,
        },
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::{json, Value};

    pub(crate) fn value() -> Value {
        let pin: Value =
            serde_json::from_str(include_str!("../../../config/models/qwen3-asr-ko.json")).unwrap();
        let runtime: Value = serde_json::from_str(include_str!(
            "../../../scripts/native-crispasr/runtime.json"
        ))
        .unwrap();
        json!({"schemaVersion":1,"model":MODEL,"modelRevision":pin["revision"],
            "modelSha256":pin["files"][0]["sha256"],"alignerSha256":pin["files"][1]["sha256"],
            "runtime":runtime["identity"],"target":"macos-arm64","processingPolicy":POLICY,
            "completionEvidence":COMPLETION,"timestampPolicy":TIMESTAMPS,"gainPolicy":GAIN,
            "padFrames":16000,"completed":true,"inputAudioFrames":32000,"audioSampleRate":16000,
            "audioDurationSeconds":2.0,"inputPcmSha256":"a".repeat(64),
            "chunks":[
                {"index":0,"startFrame":0,"endFrame":16000,"inputPcmSha256":"a".repeat(64),
                 "nativeResultSha256":"b".repeat(64),"nativeExitCode":0,"actualBackend":"metal",
                 "backendEvidence":"metal-backend-initialized","wordStart":0,"wordCount":2},
                {"index":1,"startFrame":16000,"endFrame":32000,"inputPcmSha256":"c".repeat(64),
                 "nativeResultSha256":"d".repeat(64),"nativeExitCode":0,"actualBackend":"metal",
                 "backendEvidence":"metal-backend-initialized","wordStart":2,"wordCount":1}],
            "words":[{"word":"예시","start":0.1,"end":0.5},{"word":"문장.","start":0.5,"end":0.5},
                {"word":"네.","start":1.2,"end":1.5}]})
    }

    fn decode(v: &Value) -> Result<Transcript> {
        decode_asr(&serde_json::to_vec(v).unwrap(), 2.0, "session", "id")
    }

    #[test]
    fn pinned_complete_chunks_project_sentences_and_forgeries_fail() {
        let document = decode(&value()).unwrap();
        assert_eq!(document.model, MODEL);
        assert_eq!(document.segments.len(), 2);
        assert_eq!(
            document.source.as_ref().unwrap().projector_version,
            PROJECTOR_VERSION
        );
        for (pointer, forged) in [
            ("/schemaVersion", json!(2)),
            ("/modelSha256", json!("0".repeat(64))),
            ("/alignerSha256", json!("0".repeat(64))),
            ("/runtime", json!("other")),
            ("/target", json!("windows-x64")),
            ("/processingPolicy", json!("whisper-context-windows-v1")),
            ("/gainPolicy", json!("smooth-gain-v1")),
            ("/padFrames", json!(48000)),
            ("/completed", json!(false)),
            ("/inputAudioFrames", json!(32001)),
            ("/chunks/1/startFrame", json!(15999)),
            ("/chunks/1/endFrame", json!(31999)),
            ("/chunks/0/nativeExitCode", json!(1)),
            ("/chunks/0/backendEvidence", json!("explicit-no-gpu")),
            ("/chunks/1/wordStart", json!(1)),
            ("/chunks/1/wordCount", json!(2)),
            ("/words/2/start", json!(0.9)),
            ("/words/0/end", json!(1.1)),
        ] {
            let mut forged_value = value();
            *forged_value.pointer_mut(pointer).unwrap() = forged;
            assert!(decode(&forged_value).is_err(), "{pointer}");
        }
        let mut scored = value();
        scored["words"][0]["confidence"] = json!(0.5);
        assert!(decode(&scored).is_err());
        let mut long = value();
        long["inputAudioFrames"] = json!(61 * SAMPLE_RATE + 16000);
        long["audioDurationSeconds"] = json!(62.0);
        long["chunks"][1]["endFrame"] = json!(61 * SAMPLE_RATE + 16000);
        assert!(decode_asr(&serde_json::to_vec(&long).unwrap(), 62.0, "s", "id").is_err());
    }

    fn one_second_value() -> Value {
        let mut v = value();
        v["inputAudioFrames"] = json!(16000);
        v["audioDurationSeconds"] = json!(1.0);
        v["chunks"][0]["endFrame"] = json!(8000);
        v["chunks"][1]["startFrame"] = json!(8000);
        v["chunks"][1]["endFrame"] = json!(16000);
        v["words"] = json!([{"word":"예시","start":0.1,"end":0.4},
            {"word":"문장.","start":0.4,"end":0.45},{"word":"네.","start":0.7,"end":0.7}]);
        v
    }

    #[test]
    fn stored_qwen_word_speakers_keep_zero_length_words_unknown_and_history_readable() {
        let (_temp, mut engine, id, _audio) = crate::transcript::tests::setup();
        let nemo = engine
            .commit_nemotron_transcript(&id, None, crate::nemotron::tests::raw())
            .unwrap();
        let baseline = engine
            .commit_qwen_transcript(
                &id,
                Some(nemo.version.clone()),
                serde_json::to_vec(&one_second_value()).unwrap(),
            )
            .unwrap();
        assert_eq!(engine.transcript(&id).unwrap().unwrap().model, MODEL);
        let after = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(baseline))
            .unwrap();
        assert_eq!(
            after.source.as_ref().unwrap().projector_version,
            crate::nemotron::QWEN_WORD_PROJECTOR_VERSION
        );
        let last = after.segments.last().unwrap();
        assert_eq!(last.text, "네.");
        assert!(last.speaker_id.is_none());
        assert!(after.segments[0].speaker_id.is_some());
        assert_eq!(
            after
                .speaker_analysis
                .as_ref()
                .unwrap()
                .review_causes
                .get("native_zero_duration"),
            Some(&1)
        );
        assert_eq!(engine.transcript_history(&id).unwrap().entries.len(), 3);
        let restored = engine
            .restore_transcript(&id, after.version, nemo.version)
            .unwrap();
        assert_eq!(restored.model, "nvidia/nemotron-3.5-asr-streaming-0.6b");
    }

    #[test]
    fn empty_chunks_are_explicit_and_cpu_needs_confirmed_unavailability() {
        let mut v = value();
        v["chunks"][1]["wordCount"] = json!(0);
        v["words"].as_array_mut().unwrap().pop();
        assert!(decode(&v).is_ok());
        v["chunks"][1]["actualBackend"] = json!("cpu");
        v["chunks"][1]["backendEvidence"] = json!("confirmed-gpu-unavailable");
        assert!(decode(&v).is_ok());
    }
}
