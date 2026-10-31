//! Whisper uses bounded context windows, independently of the NeMo stream protocol.
use crate::nemotron::{Word, WordProjection};
use crate::{CoreError, Result, Transcript};
use serde::Deserialize;

pub(crate) const MODEL: &str = "ggerganov/whisper.cpp";
pub(crate) const PROJECTOR_VERSION: &str = "whisper-native-token-sentences-v1";
pub(crate) const POLICY: &str = "whisper-context-windows-v1";
const COMPLETION: &str = "whisper-cli-windows-exit0-v1";
const SAMPLE_RATE: u64 = 16000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Output {
    schema_version: u32,
    model: String,
    model_revision: String,
    model_sha256: String,
    runtime: String,
    target: String,
    processing_policy: String,
    completion_evidence: String,
    token_timestamp_policy: String,
    completed: bool,
    input_audio_frames: u64,
    audio_sample_rate: u64,
    audio_duration_seconds: f64,
    input_pcm_sha256: String,
    windows: Vec<Window>,
    words: Vec<Word>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Window {
    index: usize,
    input_start_frame: u64,
    input_end_frame: u64,
    owned_start_frame: u64,
    owned_end_frame: u64,
    input_pcm_sha256: String,
    native_result_sha256: String,
    native_exit_code: i32,
    actual_backend: String,
    backend_evidence: String,
    #[serde(default)]
    execution: Option<Execution>,
    word_start: usize,
    word_count: usize,
}

// v2 adds target/package/executable provenance without rewriting v1 artifacts.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Execution {
    runtime_identity: String,
    runtime_receipt_sha256: String,
    executable_sha256: String,
    stderr_sha256: String,
    probe_result_sha256: String,
}

fn invalid() -> CoreError {
    CoreError::InvalidInput("Whisper의 전체 입력, 완료 정보 또는 원본 단어 시각을 확인할 수 없습니다. 원음과 기존 기록은 유지됩니다.".into())
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
    let model: serde_json::Value =
        serde_json::from_str(include_str!("../../../config/models/whisper-turbo.json"))
            .map_err(|_| invalid())?;
    let runtime: serde_json::Value =
        serde_json::from_str(include_str!("../../../scripts/native-whisper/runtime.json"))
            .map_err(|_| invalid())?;
    let input_duration = output.input_audio_frames as f64 / SAMPLE_RATE as f64;
    if !matches!(output.schema_version, 1 | 2)
        || output.schema_version == 2 && output.target != "windows-x64"
        || output.model != MODEL
        || Some(output.model_revision.as_str()) != model["revision"].as_str()
        || Some(output.model_sha256.as_str()) != model["files"][0]["sha256"].as_str()
        || Some(output.runtime.as_str()) != runtime["identity"].as_str()
        || !matches!(output.target.as_str(), "windows-x64" | "macos-arm64")
        || output.processing_policy != POLICY
        || output.completion_evidence != COMPLETION
        || !matches!(
            output.token_timestamp_policy.as_str(),
            "native-token-span-v1" | "native-token-span-terminal-context-v2"
        )
        || !output.completed
        || output.input_audio_frames == 0
        || output.audio_sample_rate != SAMPLE_RATE
        || !output.audio_duration_seconds.is_finite()
        || (output.audio_duration_seconds - input_duration).abs() > f64::EPSILON * input_duration
        || !duration.is_finite()
        || duration <= 0.0
        || (duration - input_duration).abs() > 1.0 / SAMPLE_RATE as f64
        || !hash(&output.input_pcm_sha256)
        || output.windows.is_empty()
        || output.windows.len() > 10000
    {
        return Err(invalid());
    }
    let mut next_frame = 0;
    let mut next_word = 0_usize;
    for (index, window) in output.windows.iter().enumerate() {
        let stop = next_word
            .checked_add(window.word_count)
            .ok_or_else(invalid)?;
        let backend_valid = match (
            output.schema_version,
            output.target.as_str(),
            window.actual_backend.as_str(),
            window.backend_evidence.as_str(),
        ) {
            (1, "windows-x64", "cpu", "explicit-no-gpu") => true,
            (1, "macos-arm64", "metal", "metal-backend-initialized") => true,
            (1, "macos-arm64", "cpu", "confirmed-gpu-unavailable") => true,
            (2, "windows-x64", "vulkan", "vulkan-backend-initialized") => true,
            (2, "windows-x64", "cpu", "confirmed-gpu-unavailable") => true,
            _ => false,
        };
        let execution_valid = match (output.schema_version, &window.execution) {
            (1, None) => true,
            (2, Some(execution)) => {
                Some(execution.runtime_identity.as_str())
                    == runtime["targets"]["windows-x64"]["identity"].as_str()
                    && [
                        &execution.runtime_receipt_sha256,
                        &execution.executable_sha256,
                        &execution.stderr_sha256,
                        &execution.probe_result_sha256,
                    ]
                    .iter()
                    .all(|value| hash(value))
            }
            _ => false,
        };
        if window.index != index
            || window.owned_start_frame != next_frame
            || window.owned_end_frame <= next_frame
            || window.owned_end_frame > output.input_audio_frames
            || window.input_start_frame > window.owned_start_frame
            || window.input_end_frame < window.owned_end_frame
            || window.input_end_frame > output.input_audio_frames
            || window.input_end_frame - window.input_start_frame > 180 * SAMPLE_RATE
            || window.owned_start_frame - window.input_start_frame > 3 * SAMPLE_RATE
            || window.input_end_frame - window.owned_end_frame > 3 * SAMPLE_RATE
            || !hash(&window.input_pcm_sha256)
            || !hash(&window.native_result_sha256)
            || window.native_exit_code != 0
            || !backend_valid
            || !execution_valid
            || window.word_start != next_word
            || stop > output.words.len()
        {
            return Err(invalid());
        }
        for word in &output.words[next_word..stop] {
            let start_frame = (word.start * SAMPLE_RATE as f64).round();
            let end_frame = (word.end * SAMPLE_RATE as f64).round();
            let center = (start_frame + end_frame) * 0.5;
            let terminal_context_valid = word.native_segment_end_frame.is_none_or(|native_end| {
                let local_frames = window.input_end_frame - window.input_start_frame;
                let native_tick = local_frames.div_ceil(160) * 160;
                output.token_timestamp_policy == "native-token-span-terminal-context-v2"
                    && word.timing_scope.as_deref() == Some("nativeSegmentContext")
                    && end_frame == window.input_end_frame as f64
                    && native_tick > local_frames
                    && native_tick - local_frames < 160
                    && window.input_start_frame.checked_add(native_tick) == Some(native_end)
                    && start_frame <= end_frame
                    && (start_frame - window.input_start_frame as f64) % 160.0 == 0.0
            });
            if word.confidence.is_some()
                || !terminal_context_valid
                || word
                    .timing_scope
                    .as_deref()
                    .is_some_and(|scope| scope != "nativeSegmentContext")
                || !center.is_finite()
                || (word.start * SAMPLE_RATE as f64 - start_frame).abs() > 1e-6
                || (word.end * SAMPLE_RATE as f64 - end_frame).abs() > 1e-6
                || start_frame < window.input_start_frame as f64
                || end_frame > window.input_end_frame as f64
                || center < window.owned_start_frame as f64
                || center >= window.owned_end_frame as f64
                    && !(index + 1 == output.windows.len()
                        && center == window.owned_end_frame as f64)
            {
                return Err(invalid());
            }
        }
        next_frame = window.owned_end_frame;
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
        let model: Value =
            serde_json::from_str(include_str!("../../../config/models/whisper-turbo.json"))
                .unwrap();
        let runtime: Value =
            serde_json::from_str(include_str!("../../../scripts/native-whisper/runtime.json"))
                .unwrap();
        json!({"schemaVersion":1,"model":MODEL,"modelRevision":model["revision"],"modelSha256":model["files"][0]["sha256"],
            "runtime":runtime["identity"],"target":"windows-x64","processingPolicy":POLICY,"completionEvidence":COMPLETION,
            "tokenTimestampPolicy":"native-token-span-v1","completed":true,"inputAudioFrames":32000,"audioSampleRate":16000,
            "audioDurationSeconds":2.0,"inputPcmSha256":"a".repeat(64),
            "windows":[{"index":0,"inputStartFrame":0,"inputEndFrame":32000,"ownedStartFrame":0,"ownedEndFrame":32000,
                "inputPcmSha256":"a".repeat(64),"nativeResultSha256":"b".repeat(64),"nativeExitCode":0,
                "actualBackend":"cpu","backendEvidence":"explicit-no-gpu","wordStart":0,"wordCount":2}],
            "words":[{"word":"예시","start":0.1,"end":0.6,"confidence":null},{"word":"문장.","start":0.7,"end":1.5,"confidence":null}]})
    }

    pub(crate) fn raw() -> Vec<u8> {
        serde_json::to_vec(&value()).unwrap()
    }

    #[test]
    fn windows_vulkan_v2_requires_target_runtime_and_execution_receipts_while_v1_stays_readable() {
        let mut v = value();
        v["schemaVersion"] = 2.into();
        v["windows"][0]["actualBackend"] = "vulkan".into();
        v["windows"][0]["backendEvidence"] = "vulkan-backend-initialized".into();
        let runtime: Value =
            serde_json::from_str(include_str!("../../../scripts/native-whisper/runtime.json"))
                .unwrap();
        v["windows"][0]["execution"] = json!({"runtimeIdentity":runtime["targets"]["windows-x64"]["identity"],
            "runtimeReceiptSha256":"a".repeat(64),"executableSha256":"b".repeat(64),
            "stderrSha256":"c".repeat(64),"probeResultSha256":"d".repeat(64)});
        let decode = |v: &Value| decode_asr(&serde_json::to_vec(v).unwrap(), 2.0, "session", "id");
        assert!(decode(&v).is_ok());
        let valid = v.clone();
        for (pointer, forged) in [
            ("/schemaVersion", json!(1)),
            ("/target", json!("macos-arm64")),
            ("/windows/0/execution", json!(null)),
            ("/windows/0/execution/runtimeIdentity", json!("other")),
            ("/windows/0/execution/probeResultSha256", json!("missing")),
            ("/windows/0/execution/executableSha256", json!("missing")),
            ("/windows/0/backendEvidence", json!("explicit-no-gpu")),
        ] {
            let mut forged_value = valid.clone();
            *forged_value.pointer_mut(pointer).unwrap() = forged;
            assert!(decode(&forged_value).is_err(), "{pointer}");
        }
        v["windows"][0]["actualBackend"] = "cpu".into();
        v["windows"][0]["backendEvidence"] = "confirmed-gpu-unavailable".into();
        assert!(decode(&v).is_ok());
        assert!(decode(&value()).is_ok());
        let mut metal = value();
        metal["target"] = "macos-arm64".into();
        metal["windows"][0]["actualBackend"] = "metal".into();
        metal["windows"][0]["backendEvidence"] = "metal-backend-initialized".into();
        assert!(decode(&metal).is_ok());
    }

    fn one_second_value() -> Value {
        let mut v = value();
        v["inputAudioFrames"] = 16000.into();
        v["audioDurationSeconds"] = 1.0.into();
        v["windows"][0]["inputEndFrame"] = 16000.into();
        v["windows"][0]["ownedEndFrame"] = 16000.into();
        v["words"][1]["end"] = 0.9.into();
        v
    }

    #[test]
    fn stored_whisper_and_word_speaker_generations_preserve_prior_engine_and_read_only_queries() {
        let (_temp, mut engine, id, _audio) = crate::transcript::tests::setup();
        let nemo_raw = crate::nemotron::tests::raw();
        let nemo = engine
            .commit_nemotron_transcript(&id, None, nemo_raw)
            .unwrap();
        let raw = serde_json::to_vec(&one_second_value()).unwrap();
        let baseline = engine
            .commit_whisper_transcript(&id, Some(nemo.version.clone()), raw.clone())
            .unwrap();
        let reread = engine.transcript(&id).unwrap().unwrap();
        assert_eq!(reread.model, MODEL);
        let after = engine
            .apply_diarization(
                &id,
                crate::nemotron::tests::posterior_draft(baseline.clone()),
            )
            .unwrap();
        assert_eq!(after.model, MODEL);
        assert_eq!(
            after.source.as_ref().unwrap().projector_version,
            crate::nemotron::WHISPER_WORD_PROJECTOR_VERSION
        );
        assert!(after.segments.iter().all(|s| s.speaker_id.is_some()));
        let reader = crate::reader::LibraryReader::open(&engine.store.root).unwrap();
        let read = reader
            .query("transcript_get", json!({"meetingId":id}))
            .unwrap();
        assert!(!read["segments"].as_array().unwrap().is_empty());
        assert_eq!(engine.transcript_history(&id).unwrap().entries.len(), 3);
        let restored = engine
            .restore_transcript(&id, after.version, nemo.version)
            .unwrap();
        assert_eq!(restored.model, "nvidia/nemotron-3.5-asr-streaming-0.6b");
    }

    #[test]
    fn zero_duration_native_word_is_preserved_without_assigning_a_speaker() {
        let (_temp, mut engine, id, _audio) = crate::transcript::tests::setup();
        let mut v = one_second_value();
        v["words"][0]["end"] = v["words"][0]["start"].clone();
        let baseline = engine
            .commit_whisper_transcript(&id, None, serde_json::to_vec(&v).unwrap())
            .unwrap();
        let after = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(baseline))
            .unwrap();
        assert_eq!(
            after
                .segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            vec!["예시", "문장."]
        );
        assert!(after.segments[0].speaker_id.is_none());
        assert_eq!(after.segments[0].start, after.segments[0].end);
        assert!(after.segments[1].speaker_id.is_some());
        assert_eq!(
            after
                .speaker_analysis
                .as_ref()
                .unwrap()
                .review_causes
                .get("native_zero_duration"),
            Some(&1)
        );
        let read = engine.transcript(&id).unwrap().unwrap();
        assert_eq!(read.version, after.version);
    }

    #[test]
    fn native_segment_context_preserves_text_and_is_never_projected_as_word_timing() {
        let (_temp, mut engine, id, _audio) = crate::transcript::tests::setup();
        let mut v = one_second_value();
        v["words"][0]["timingScope"] = "nativeSegmentContext".into();
        let baseline = engine
            .commit_whisper_transcript(&id, None, serde_json::to_vec(&v).unwrap())
            .unwrap();
        let after = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(baseline))
            .unwrap();
        assert!(after.segments[0].speaker_id.is_none());
        assert_eq!(after.segments[0].text, "예시");
        assert_eq!(
            after
                .speaker_analysis
                .as_ref()
                .unwrap()
                .review_causes
                .get("native_segment_context"),
            Some(&1)
        );
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            after.version
        );
    }

    fn terminal_value() -> Value {
        let mut v = one_second_value();
        v["tokenTimestampPolicy"] = "native-token-span-terminal-context-v2".into();
        v["windows"][0]["inputEndFrame"] = 8001.into();
        v["windows"][0]["ownedEndFrame"] = 8001.into();
        v["windows"][0]["wordCount"] = 0.into();
        let mut last = v["windows"][0].clone();
        last["index"] = 1.into();
        last["inputStartFrame"] = 1.into();
        last["inputEndFrame"] = 16000.into();
        last["ownedStartFrame"] = 8001.into();
        last["ownedEndFrame"] = 16000.into();
        last["wordCount"] = 1.into();
        v["windows"].as_array_mut().unwrap().push(last);
        v["words"] = json!([{"word":"예시","start":8001_f64/16000.0,"end":1.0,
            "timingScope":"nativeSegmentContext","nativeSegmentEndFrame":16001}]);
        v
    }

    #[test]
    fn explicit_terminal_context_keeps_native_end_and_replays_as_unknown_speaker() {
        let (_temp, mut engine, id, _audio) = crate::transcript::tests::setup();
        let raw = serde_json::to_vec(&terminal_value()).unwrap();
        let baseline = engine
            .commit_whisper_transcript(&id, None, raw.clone())
            .unwrap();
        let after = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(baseline))
            .unwrap();
        assert_eq!(after.segments.len(), 1);
        assert_eq!(after.segments[0].end, 1.0);
        assert!(after.segments[0].speaker_id.is_none());
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            after.version
        );
        let reader = crate::reader::LibraryReader::open(&engine.store.root).unwrap();
        assert_eq!(
            reader
                .query("transcript_get", json!({"meetingId":id}))
                .unwrap()["segments"][0]["text"],
            "예시"
        );
    }

    #[test]
    fn terminal_context_rejects_forged_metadata_and_never_changes_nemo_policy() {
        for (pointer, value) in [
            ("/tokenTimestampPolicy", json!("native-token-span-v1")),
            ("/words/0/timingScope", json!(null)),
            ("/words/0/nativeSegmentEndFrame", json!(16161)),
            ("/words/0/nativeSegmentEndFrame", json!(16000)),
            ("/words/0/end", json!(0.99)),
            ("/words/0/start", json!(8002_f64 / 16000.0)),
            ("/windows/1/inputStartFrame", json!(0)),
        ] {
            let mut v = terminal_value();
            *v.pointer_mut(pointer).unwrap() = value;
            assert!(
                decode_asr(&serde_json::to_vec(&v).unwrap(), 1.0, "session", "id").is_err(),
                "{pointer}"
            );
        }
        let mut nemo: Value = serde_json::from_slice(&crate::nemotron::tests::raw()).unwrap();
        nemo["words"][0]["nativeSegmentEndFrame"] = 16001.into();
        assert!(crate::nemotron::decode_asr(
            &serde_json::to_vec(&nemo).unwrap(),
            1.0,
            "session",
            "id"
        )
        .is_err());
    }

    #[test]
    fn whisper_source_is_distinct_from_nemo_and_readable_on_other_hosts() {
        let transcript = decode_asr(&raw(), 2.0, "session", "id").unwrap();
        assert_eq!(transcript.model, MODEL);
        assert_eq!(
            transcript.source.unwrap().projector_version,
            PROJECTOR_VERSION
        );
        assert_eq!(transcript.segments[0].text, "예시 문장.");
        assert!(crate::nemotron::decode_asr(&raw(), 2.0, "session", "id").is_err());
    }

    #[test]
    fn unaligned_recording_eof_is_context_with_the_original_native_tick() {
        let frames = 2_879_931_u64;
        let mut v = value();
        v["tokenTimestampPolicy"] = "native-token-span-terminal-context-v2".into();
        v["inputAudioFrames"] = frames.into();
        v["audioDurationSeconds"] = json!(frames as f64 / 16000.0);
        v["windows"][0]["inputEndFrame"] = frames.into();
        v["windows"][0]["ownedEndFrame"] = frames.into();
        v["windows"][0]["wordCount"] = 1.into();
        v["words"] = json!([{"word":"예시","start":179.0,"end":frames as f64/16000.0,
            "timingScope":"nativeSegmentContext","nativeSegmentEndFrame":2_880_000}]);
        let decoded = decode_asr(
            &serde_json::to_vec(&v).unwrap(),
            frames as f64 / 16000.0,
            "session",
            "id",
        )
        .unwrap();
        assert_eq!(decoded.segments.len(), 1);
        assert_eq!(decoded.segments[0].end, frames as f64 / 16000.0);
        assert!(decoded.segments[0].speaker_id.is_none());
        v["words"][0]["nativeSegmentEndFrame"] = 2_880_160.into();
        assert!(decode_asr(
            &serde_json::to_vec(&v).unwrap(),
            frames as f64 / 16000.0,
            "session",
            "id"
        )
        .is_err());
    }

    #[test]
    fn missing_or_unfinished_window_and_forged_word_times_are_rejected() {
        for change in [
            ("/windows/0/ownedEndFrame", json!(31999)),
            ("/windows/0/ownedStartFrame", json!(1)),
            ("/windows/0/nativeExitCode", json!(1)),
            ("/windows/0/nativeResultSha256", json!("missing")),
            ("/windows/0/wordCount", json!(1)),
            ("/words/1/end", json!(2.1)),
            ("/words/0/confidence", json!(0.9)),
            ("/runtime", json!("other")),
            ("/completed", json!(false)),
        ] {
            let mut value = value();
            *value
                .pointer_mut(change.0)
                .unwrap_or_else(|| panic!("fixture pointer {}", change.0)) = change.1;
            assert!(
                decode_asr(&serde_json::to_vec(&value).unwrap(), 2.0, "session", "id").is_err(),
                "{}",
                change.0
            );
        }
    }

    #[test]
    fn adjacent_owned_windows_allow_context_but_require_no_gap_or_duplicate_words() {
        let mut v = value();
        v["windows"][0]["ownedEndFrame"] = 16000.into();
        v["windows"][0]["wordCount"] = 1.into();
        let mut second = v["windows"][0].clone();
        second["index"] = 1.into();
        second["ownedStartFrame"] = 16000.into();
        second["ownedEndFrame"] = 32000.into();
        second["wordStart"] = 1.into();
        v["windows"].as_array_mut().unwrap().push(second);
        // The second native word has its midpoint after the ownership boundary.
        assert!(decode_asr(&serde_json::to_vec(&v).unwrap(), 2.0, "session", "id").is_ok());
        v["windows"][1]["ownedStartFrame"] = 15999.into();
        assert!(decode_asr(&serde_json::to_vec(&v).unwrap(), 2.0, "session", "id").is_err());
    }
}
