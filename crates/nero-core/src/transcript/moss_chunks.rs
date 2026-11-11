//! Versioned product output. Historical MOSS whole-recording/diagnostic contracts
//! remain in the parent module and are never accepted through this decoder.
use super::*;

pub const POLICY: &str = "moss-independent-chunks-v1";
pub(crate) const PROJECTOR: &str = "moss-independent-chunks-projector-v2";
const STORED_PROJECTOR: &str = "moss-independent-chunks-projector-v1";
pub const CHUNK_FRAMES: u64 = 60 * SOURCE_TIMEBASE;
const MAX_RAW_BYTES: usize = 16 * 1024 * 1024;
// Two centisecond units observed with the pinned model, expressed at the input rate.
const TERMINAL_OVERRUN_FRAMES: u64 = 320;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecognitionChunkState {
    Complete,
    Failed,
    NotProcessed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecognitionChunk {
    pub index: u32,
    pub start_frame: u64,
    pub end_frame: u64,
    pub state: RecognitionChunkState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecognitionReport {
    pub sample_rate: u32,
    pub input_frames: u64,
    pub chunks: Vec<RecognitionChunk>,
    pub timing_context_segments: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub normalized_end_segments: Vec<u32>,
}
impl RecognitionReport {
    pub fn text_complete(&self) -> bool {
        self.chunks
            .iter()
            .all(|chunk| chunk.state == RecognitionChunkState::Complete)
    }
    pub fn needs_review(&self) -> bool {
        !self.text_complete() || !self.timing_context_segments.is_empty()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProductOutput {
    schema_version: u32,
    model: String,
    model_revision: String,
    runtime: String,
    processing_policy: String,
    input_audio_frames: u64,
    input_audio_sha256: String,
    chunk_frames: u64,
    chunks: Vec<ChunkRecord>,
    #[serde(default)]
    glossary_context: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChunkRecord {
    index: u32,
    start_frame: u64,
    end_frame: u64,
    // Exact UTF-8 JSON bytes from the worker, including rejected native output.
    raw_bytes: Vec<u8>,
    #[serde(default)]
    not_processed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeChunk {
    index: u32,
    start_frame: u64,
    end_frame: u64,
    input_audio_sha256: String,
    raw_text: String,
    segments: Vec<MossSegment>,
    parsed_completely: bool,
    generation_metadata: Vec<serde_json::Value>,
}

fn invalid() -> CoreError {
    CoreError::InvalidInput(
        "MOSS 구간 결과의 입력 또는 출처를 확인할 수 없습니다. 원본 결과는 보관했습니다.".into(),
    )
}
fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn timestamp(input: &str) -> Option<(f64, &str)> {
    let (number, tail) = input.strip_prefix('[')?.split_once(']')?;
    let normalized = number.replace(',', ".");
    let mut pieces = normalized.split('.');
    let whole = pieces.next()?;
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if let Some(decimal) = pieces.next() {
        if decimal.is_empty() || !decimal.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    if pieces.next().is_some() {
        return None;
    }
    let value: f64 = normalized.parse().ok()?;
    value.is_finite().then_some((value, tail))
}
fn complete_text(raw: &str, segments: &[MossSegment]) -> bool {
    let mut rest = raw.trim();
    let mut index = 0;
    while !rest.is_empty() {
        let Some((start, mut body)) = timestamp(rest) else {
            return false;
        };
        let mut speaker = None;
        if body.starts_with("[S") {
            let Some((label, tail)) = body[1..].split_once(']') else {
                return false;
            };
            if label.len() < 2 || !label[1..].bytes().all(|b| b.is_ascii_digit()) {
                return false;
            }
            speaker = Some(label);
            body = tail;
        }
        let Some((offset, end, tail)) = body
            .match_indices('[')
            .find_map(|(i, _)| timestamp(&body[i..]).map(|(end, tail)| (i, end, tail)))
        else {
            return false;
        };
        let text = body[..offset].trim();
        if end < start {
            return false;
        }
        if !text.is_empty() {
            let Some(segment) = segments.get(index) else {
                return false;
            };
            let expected = speaker
                .map(|label| format!("[{label}] {text}"))
                .unwrap_or_else(|| text.into());
            if segment.start != start
                || segment.end != end
                || segment.text != expected
                || model_speaker_id(segment.speaker_id.as_ref()).as_deref() != speaker
            {
                return false;
            }
            index += 1;
        }
        rest = tail.trim();
    }
    index == segments.len()
}

fn generation_complete(value: &serde_json::Value, frames: u64) -> bool {
    let Ok(evidence) = serde_json::from_value::<MossGenerationEvidence>(value.clone()) else {
        return false;
    };
    let (Some(prompt), Some(context), Some(requested), Some(effective), Some(generated)) = (
        value["promptTokens"].as_u64(),
        value["contextSize"].as_u64(),
        value["maxTokens"].as_u64(),
        value["effectiveMaxTokens"].as_u64(),
        value["generationTokens"].as_u64(),
    ) else {
        return false;
    };
    evidence.complete_for(frames)
        && value["terminationReason"] == "eos"
        && prompt > 0
        && context.saturating_sub(prompt) > 1
        && requested == 8192
        && effective == requested.min(context - prompt - 1)
        && generated <= effective
}

pub(super) fn decode(
    raw: &[u8],
    source_frames: u64,
    source_rate: u32,
    session: &str,
    id: &str,
) -> Result<Transcript> {
    decode_with_projector(raw, source_frames, source_rate, session, id, PROJECTOR)
}

pub(super) fn is_projector(version: &str) -> bool {
    matches!(version, PROJECTOR | STORED_PROJECTOR)
}

pub(super) fn decode_with_projector(
    raw: &[u8],
    source_frames: u64,
    source_rate: u32,
    session: &str,
    id: &str,
    projector: &str,
) -> Result<Transcript> {
    if !is_projector(projector) {
        return Err(invalid());
    }
    let normalize_end = projector == PROJECTOR;
    if raw.len() > MAX_RAW_BYTES {
        return Err(invalid());
    }
    let output: ProductOutput = serde_json::from_slice(raw).map_err(|_| invalid())?;
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../resources/models/moss.json"))
            .map_err(|_| invalid())?;
    let expected_frames =
        crate::audio_normalize::output_frames(source_frames, source_rate).map_err(|_| invalid())?;
    if output.schema_version != 1
        || output.model != manifest["repository"]
        || output.model_revision != manifest["revision"]
        || output.runtime != WHOLE_RECORDING_RUNTIME
        || output.processing_policy != POLICY
        || output.chunk_frames != CHUNK_FRAMES
        || expected_frames == 0
        || output.input_audio_frames != expected_frames
        || !digest(&output.input_audio_sha256)
        || output.chunks.len() as u64 != expected_frames.div_ceil(CHUNK_FRAMES)
    {
        return Err(invalid());
    }
    let _ = output.glossary_context;
    let duration = source_frames as f64 / source_rate as f64;
    let mut report = RecognitionReport {
        sample_rate: SOURCE_TIMEBASE as u32,
        input_frames: expected_frames,
        chunks: Vec::new(),
        timing_context_segments: Vec::new(),
        normalized_end_segments: Vec::new(),
    };
    let mut segments = Vec::new();
    for (index, record) in output.chunks.into_iter().enumerate() {
        let start = index as u64 * CHUNK_FRAMES;
        let end = (start + CHUNK_FRAMES).min(expected_frames);
        if record.index as usize != index
            || record.start_frame != start
            || record.end_frame != end
            || record.raw_bytes.len() > 2 * 1024 * 1024
            || (record.not_processed && !record.raw_bytes.is_empty())
        {
            return Err(invalid());
        }
        let mut chunk = RecognitionChunk {
            index: record.index,
            start_frame: start,
            end_frame: end,
            state: RecognitionChunkState::Failed,
            issue: Some("nativeOutputInvalid".into()),
        };
        if record.not_processed {
            chunk.state = RecognitionChunkState::NotProcessed;
            chunk.issue = Some("notProcessed".into());
            report.chunks.push(chunk);
            continue;
        }
        let native = serde_json::from_slice::<NativeChunk>(&record.raw_bytes).ok();
        if let Some(native) = native {
            if native.index != record.index
                || native.start_frame != start
                || native.end_frame != end
                || native.input_audio_sha256 != output.input_audio_sha256
            {
                return Err(invalid());
            }
            if native.parsed_completely
                && complete_text(&native.raw_text, &native.segments)
                && native.generation_metadata.len() == 1
                && generation_complete(&native.generation_metadata[0], end - start)
                && (native.raw_text.trim().is_empty()
                    || native.generation_metadata[0]["generationTokens"]
                        .as_u64()
                        .is_some_and(|tokens| tokens > 0))
                && native.raw_text.len() <= 1024 * 1024
                && !native.raw_text.contains('\0')
            {
                let local_duration = (end - start) as f64 / SOURCE_TIMEBASE as f64;
                let offset = start as f64 / SOURCE_TIMEBASE as f64;
                let playback_end = (end as f64 / SOURCE_TIMEBASE as f64).min(duration);
                let end_limit = if normalize_end {
                    // Subtract exact frame addresses before converting the final tail to seconds.
                    let source_local = u128::from(source_frames) * u128::from(SOURCE_TIMEBASE)
                        - u128::from(start) * u128::from(source_rate);
                    let source_limit = (source_local
                        + u128::from(TERMINAL_OVERRUN_FRAMES) * u128::from(source_rate))
                        as f64
                        / (u128::from(source_rate) * u128::from(SOURCE_TIMEBASE)) as f64;
                    ((end - start + TERMINAL_OVERRUN_FRAMES) as f64 / SOURCE_TIMEBASE as f64)
                        .min(source_limit)
                } else {
                    local_duration + 0.02
                };
                let mut projected = Vec::new();
                let mut contexts = Vec::new();
                let mut normalized = Vec::new();
                let mut valid = true;
                for segment in native.segments {
                    if !segment.start.is_finite()
                        || !segment.end.is_finite()
                        || segment.start < 0.0
                        || segment.start > local_duration
                        || segment.end < segment.start
                        || segment.end > end_limit
                        || segment.text.len() > 1024 * 1024
                        || segment.text.contains('\0')
                    {
                        valid = false;
                        break;
                    }
                    if segment.text.trim().is_empty() {
                        continue;
                    }
                    let label = model_speaker_id(segment.speaker_id.as_ref());
                    let text = label
                        .as_ref()
                        .and_then(|label| segment.text.strip_prefix(&format!("[{label}] ")))
                        .unwrap_or(&segment.text)
                        .trim()
                        .to_owned();
                    if text.is_empty() {
                        continue;
                    }
                    let source_index =
                        u32::try_from(segments.len() + projected.len()).map_err(|_| invalid())?;
                    let overrun = segment.end > local_duration || offset + segment.end > duration;
                    let projected_start = offset + segment.start;
                    let projected_end = (offset + segment.end).min(playback_end);
                    if normalize_end && overrun {
                        if projected_end <= projected_start {
                            valid = false;
                            break;
                        }
                        normalized.push(source_index);
                    }
                    let context = segment.start == segment.end || (!normalize_end && overrun);
                    if context {
                        contexts.push(source_index);
                    }
                    projected.push(TranscriptSegment {
                        id: Uuid::new_v4().to_string(),
                        start: if context { offset } else { projected_start },
                        end: if context { playback_end } else { projected_end },
                        original_text: text.clone(),
                        text,
                        speaker_id: None,
                        source_index: Some(source_index),
                    });
                }
                // Empty output is silence only when both native forms are empty.
                if valid && (!projected.is_empty() || native.raw_text.trim().is_empty()) {
                    report.timing_context_segments.extend(contexts);
                    report.normalized_end_segments.extend(normalized);
                    segments.extend(projected);
                    chunk.state = RecognitionChunkState::Complete;
                    chunk.issue = None;
                }
            }
        }
        report.chunks.push(chunk);
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
            raw_sha256: sha256(raw),
            projector_version: projector.into(),
            association: "direct".into(),
            speaker_scope: Some("chunk".into()),
            chunking_policy: Some(POLICY.into()),
            recognition_report: Some(report),
        }),
        speakers: Vec::new(),
        segments,
        speaker_analysis: None,
    })
}

pub(super) fn can_publish(document: &Transcript) -> bool {
    !document.segments.is_empty()
        || document
            .source
            .as_ref()
            .and_then(|source| source.recognition_report.as_ref())
            .is_some_and(RecognitionReport::text_complete)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn native(index: u32, start: u64, end: u64) -> Value {
        let lengths: Vec<u64> = (0..end - start)
            .step_by(480_000)
            .map(|n| (end - start - n).min(480_000).div_ceil(1280))
            .collect();
        let tokens: u64 = lengths.iter().sum();
        json!({"index":index,"startFrame":start,"endFrame":end,"inputAudioSha256":"a".repeat(64),
            "rawText":"[0][S00]합성 문장[0.5]","segments":[{"start":0.0,"end":0.5,"text":"[S00] 합성 문장","speaker_id":"S00"}],"parsedCompletely":true,
            "generationMetadata":[{"metadataSchemaVersion":2,"eosPolicy":"model-generation-config+tokenizer-v1","samplingPolicy":"greedy-argmax-v1","terminationReason":"eos","terminalTokenId":151645,"configuredEosTokenIds":[151645],"tokenizerEosTokenId":151645,"inputAudioSamples":end-start,"featureChunkCount":lengths.len(),"featureTokenLengths":lengths,"audioTokenCount":tokens,"audioEmbeddingCount":tokens,"promptTokens":1000,"contextSize":32768,"maxTokens":8192,"effectiveMaxTokens":8192,"generationTokens":12}]})
    }
    fn output(frames: u64) -> Value {
        let manifest: Value =
            serde_json::from_str(include_str!("../../resources/models/moss.json")).unwrap();
        let chunks: Vec<_> = (0..frames.div_ceil(CHUNK_FRAMES)).map(|index| {
            let start=index*CHUNK_FRAMES; let end=(start+CHUNK_FRAMES).min(frames);
            json!({"index":index,"startFrame":start,"endFrame":end,"rawBytes":serde_json::to_vec(&native(index as u32,start,end)).unwrap(),"notProcessed":false})
        }).collect();
        json!({"schemaVersion":1,"model":manifest["repository"],"modelRevision":manifest["revision"],"runtime":WHOLE_RECORDING_RUNTIME,"processingPolicy":POLICY,"inputAudioFrames":frames,"inputAudioSha256":"a".repeat(64),"chunkFrames":CHUNK_FRAMES,"chunks":chunks})
    }
    fn change(value: &mut Value, index: usize, action: impl FnOnce(&mut Value)) {
        let bytes: Vec<u8> =
            serde_json::from_value(value["chunks"][index]["rawBytes"].clone()).unwrap();
        let mut raw: Value = serde_json::from_slice(&bytes).unwrap();
        action(&mut raw);
        value["chunks"][index]["rawBytes"] = json!(serde_json::to_vec(&raw).unwrap());
    }
    fn project(value: &Value) -> Result<Transcript> {
        decode(
            &serde_json::to_vec(value).unwrap(),
            value["inputAudioFrames"].as_u64().unwrap(),
            16000,
            "session",
            "generation",
        )
    }

    #[test]
    fn timed_text_without_a_speaker_is_valid_but_omitted_or_extra_text_is_not() {
        let mut value = output(CHUNK_FRAMES);
        change(&mut value, 0, |native| {
            native["rawText"] = json!("[0]합성 문장[0.5]");
            native["segments"][0]["text"] = json!("합성 문장");
            native["segments"][0]
                .as_object_mut()
                .unwrap()
                .remove("speaker_id");
        });
        assert!(project(&value)
            .unwrap()
            .source
            .unwrap()
            .recognition_report
            .unwrap()
            .text_complete());
        change(&mut value, 0, |native| {
            native["rawText"] = json!("[0]합성 문장[0.5] 누락된 꼬리")
        });
        assert!(!can_publish(&project(&value).unwrap()));
    }

    #[test]
    fn a_failed_middle_chunk_retains_the_later_text_and_explicit_missing_range() {
        let mut value = output(3 * CHUNK_FRAMES);
        change(&mut value, 1, |native| {
            native["rawText"] = json!("[S00][][][S00][][]");
            native["parsedCompletely"] = json!(false);
        });
        let doc = project(&value).unwrap();
        assert_eq!(doc.segments.len(), 2);
        assert_eq!(doc.segments[1].start, 120.0);
        let report = doc.source.unwrap().recognition_report.unwrap();
        assert_eq!(report.chunks[1].state, RecognitionChunkState::Failed);
        assert_eq!(
            (report.chunks[1].start_frame, report.chunks[1].end_frame),
            (CHUNK_FRAMES, 2 * CHUNK_FRAMES)
        );
        assert!(!report.text_complete());
        assert!(report.needs_review());
    }
    #[test]
    fn bounded_native_terminal_overrun_keeps_start_and_raw_without_review() {
        let mut value = output(CHUNK_FRAMES);
        change(&mut value, 0, |native| {
            native["segments"][0]["start"] = json!(59.5);
            native["segments"][0]["end"] = json!(60.02);
            native["rawText"] = json!("[59.5][S00]합성 문장[60.02]");
        });
        let original = serde_json::to_vec(&value).unwrap();
        let doc = project(&value).unwrap();
        assert_eq!((doc.segments[0].start, doc.segments[0].end), (59.5, 60.0));
        let source = doc.source.unwrap();
        assert_eq!(source.projector_version, PROJECTOR);
        assert_eq!(source.raw_sha256, sha256(&original));
        let report = source.recognition_report.unwrap();
        assert!(report.text_complete());
        assert!(!report.needs_review());
        assert!(report.timing_context_segments.is_empty());
        assert_eq!(report.normalized_end_segments, vec![0]);
        let bytes: Vec<u8> =
            serde_json::from_value(value["chunks"][0]["rawBytes"].clone()).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap()["segments"][0]["end"],
            json!(60.02)
        );
    }

    fn set_time(value: &mut Value, chunk: usize, start: f64, end: f64) {
        change(value, chunk, |native| {
            native["segments"][0]["start"] = json!(start);
            native["segments"][0]["end"] = json!(end);
            native["rawText"] = json!(format!("[{start}][S00]합성 문장[{end}]"));
        });
    }

    #[test]
    fn terminal_allowance_has_a_hard_limit_and_cannot_create_a_playback_range() {
        for (start, end, accepted) in [
            (59.5, 60.0, true),
            (59.5, 60.01, true),
            (59.5, 60.02, true),
            (59.5, 60.020001, false),
            (59.5, 61.0, false),
            (60.0, 60.01, false),
            (60.01, 60.02, false),
            (-0.01, 0.5, false),
            (0.5, 0.4, false),
        ] {
            let mut value = output(2 * CHUNK_FRAMES);
            set_time(&mut value, 0, start, end);
            let doc = project(&value).unwrap();
            let report = doc.source.unwrap().recognition_report.unwrap();
            assert_eq!(
                report.chunks[0].state == RecognitionChunkState::Complete,
                accepted,
                "{start}..{end}"
            );
            assert_eq!(report.chunks[1].state, RecognitionChunkState::Complete);
            assert_eq!(doc.segments.last().unwrap().start, 60.0);
            assert_eq!(report.needs_review(), !accepted);
            assert_eq!(
                report.normalized_end_segments,
                if accepted && end > 60.0 {
                    vec![0]
                } else {
                    vec![]
                }
            );
        }
    }

    #[test]
    fn last_partial_chunk_uses_its_exact_audio_end_at_the_original_rate() {
        let frames = CHUNK_FRAMES + 3 * SOURCE_TIMEBASE + 1;
        let mut value = output(frames);
        set_time(&mut value, 1, 2.5, 3.02);
        // 32kHz input rounds up by half a 16kHz sample during normalization.
        let source_frames = 2 * frames - 1;
        let raw = serde_json::to_vec(&value).unwrap();
        let doc = decode(&raw, source_frames, 32000, "session", "generation").unwrap();
        assert_eq!(doc.segments[1].start, 62.5);
        assert_eq!(doc.segments[1].end, source_frames as f64 / 32000.0);
        let report = doc.source.unwrap().recognition_report.unwrap();
        assert!(!report.needs_review());
        assert_eq!(report.normalized_end_segments, vec![1]);
        set_time(&mut value, 1, 2.5, 3.021);
        let doc = decode(
            &serde_json::to_vec(&value).unwrap(),
            source_frames,
            32000,
            "session",
            "generation",
        )
        .unwrap();
        assert_eq!(doc.segments.len(), 1);
        assert!(doc
            .source
            .unwrap()
            .recognition_report
            .unwrap()
            .needs_review());
    }

    #[test]
    fn exact_twenty_millisecond_tail_is_not_rejected_by_large_offset_subtraction() {
        let frames = 103 * CHUNK_FRAMES + 189760;
        let mut value = output(frames);
        set_time(&mut value, 103, 11.5, 11.88);
        let raw = serde_json::to_vec(&value).unwrap();
        let doc = decode(&raw, 3 * frames, 48000, "session", "generation").unwrap();
        let report = doc.source.unwrap().recognition_report.unwrap();
        assert!(!report.needs_review());
        assert_eq!(report.normalized_end_segments, vec![103]);
        assert_eq!(
            (doc.segments[103].start, doc.segments[103].end),
            (6191.5, 6191.86)
        );
    }

    #[test]
    fn unusable_native_zero_duration_remains_context_and_is_not_normalized() {
        let mut value = output(CHUNK_FRAMES);
        set_time(&mut value, 0, 1.0, 1.0);
        let doc = project(&value).unwrap();
        let report = doc.source.unwrap().recognition_report.unwrap();
        assert_eq!(report.timing_context_segments, vec![0]);
        assert!(report.normalized_end_segments.is_empty());
        assert!(report.needs_review());
    }

    #[test]
    fn saved_v1_context_and_v2_normalization_remain_readable_after_restart() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().canonicalize().unwrap();
        let wav = directory.join("synthetic.wav");
        let mut writer = hound::WavWriter::create(
            &wav,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..32000 {
            writer.write_sample(10i16).unwrap();
        }
        writer.finalize().unwrap();
        let root = directory.join("store");
        let mut engine = Engine::open(root.clone()).unwrap();
        let meeting = engine
            .import_audio("합성 회의".into(), wav.clone(), wav)
            .unwrap();
        let mut value = output(32000);
        set_time(&mut value, 0, 1.5, 2.02);
        let raw = serde_json::to_vec(&value).unwrap();
        let current = engine
            .commit_moss_chunk_transcript(&meeting.id, None, raw.clone())
            .unwrap();
        drop(engine);
        let engine = Engine::open(root.clone()).unwrap();
        let read = engine.transcript(&meeting.id).unwrap().unwrap();
        assert_eq!(read.source, current.source);
        assert_eq!((read.segments[0].start, read.segments[0].end), (1.5, 2.0));
        let directory = transcript_dir(&meeting.id, &current.version.id).unwrap();
        let mut legacy = decode_with_projector(
            &raw,
            32000,
            16000,
            &current.source_session_id,
            &current.version.id,
            STORED_PROJECTOR,
        )
        .unwrap();
        legacy.segments[0].id = current.segments[0].id.clone();
        drop(engine);
        for name in ["initial.json", "current.json"] {
            std::fs::write(
                root.join(&directory).join(name),
                serde_json::to_vec(&legacy).unwrap(),
            )
            .unwrap();
        }
        let mut engine = Engine::open(root).unwrap();
        let read = engine.transcript(&meeting.id).unwrap().unwrap();
        assert_eq!((read.segments[0].start, read.segments[0].end), (0.0, 2.0));
        assert_eq!(read.source, legacy.source);
        assert!(read
            .source
            .as_ref()
            .unwrap()
            .recognition_report
            .as_ref()
            .unwrap()
            .needs_review());
        let edited = engine
            .edit_transcript(
                &meeting.id,
                read.version,
                vec!["사용자가 편집한 본문".into()],
            )
            .unwrap();
        assert_eq!(edited.segments[0].text, "사용자가 편집한 본문");
        assert_eq!(edited.source, legacy.source);
    }

    #[test]
    #[ignore = "requires an explicit private replay fixture, without re-running inference"]
    fn replay_recorded_native_chunks_through_current_projection() {
        let path = std::env::var_os("NERO_MOSS_REPLAY_JSON")
            .expect("private product-envelope replay fixture");
        let raw = std::fs::read(path).unwrap();
        let value: Value = serde_json::from_slice(&raw).unwrap();
        let frames = value["inputAudioFrames"].as_u64().unwrap();
        let doc = decode(&raw, frames, 16000, "replay", "replay").unwrap();
        let report = doc
            .source
            .as_ref()
            .unwrap()
            .recognition_report
            .as_ref()
            .unwrap();
        assert!(report.text_complete());
        assert!(!report.needs_review());
        let mut original = Vec::new();
        for record in value["chunks"].as_array().unwrap() {
            let bytes: Vec<u8> = serde_json::from_value(record["rawBytes"].clone()).unwrap();
            let native: NativeChunk = serde_json::from_slice(&bytes).unwrap();
            original.extend(native.segments.into_iter().map(|segment| {
                (
                    native.start_frame as f64 / 16000.0,
                    native.end_frame as f64 / 16000.0,
                    segment,
                )
            }));
        }
        assert_eq!(doc.segments.len(), original.len());
        let mut corrections = Vec::new();
        for (index, (segment, (offset, chunk_end, native))) in
            doc.segments.iter().zip(&original).enumerate()
        {
            assert_eq!(segment.start, offset + native.start);
            assert_eq!(segment.end, (offset + native.end).min(*chunk_end));
            assert!(segment.speaker_id.is_none());
            let text = model_speaker_id(native.speaker_id.as_ref())
                .map(|label| native.text.strip_prefix(&format!("[{label}] ")).unwrap())
                .unwrap_or(&native.text);
            assert_eq!(segment.text, text.trim());
            if report.normalized_end_segments.contains(&(index as u32)) {
                corrections.push(json!({"sourceIndex":index,"nativeStart":offset+native.start,"nativeEnd":offset+native.end,"start":segment.start,"end":segment.end}));
            }
        }
        let expected: usize = std::env::var("NERO_MOSS_REPLAY_NORMALIZED")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(corrections.len(), expected);
        if let Some(path) = std::env::var_os("NERO_MOSS_REPLAY_REPORT") {
            std::fs::write(path, serde_json::to_vec_pretty(&json!({"projector":PROJECTOR,"fixtureSha256":sha256(&raw),"coverage":report,"segmentCount":doc.segments.len(),"corrections":corrections,"rawUnchanged":true,"startsUnchanged":true,"textsUnchanged":true,"inferenceRerun":false})).unwrap()).unwrap();
        }
    }
    #[test]
    fn gross_timing_and_false_eos_cannot_become_complete_text() {
        let mut value = output(CHUNK_FRAMES);
        change(&mut value, 0, |native| {
            native["segments"][0]["end"] = json!(61.0)
        });
        let doc = project(&value).unwrap();
        assert!(!can_publish(&doc));
        let mut value = output(CHUNK_FRAMES);
        change(&mut value, 0, |native| {
            native["generationMetadata"][0]["terminalTokenId"] = json!(0)
        });
        assert!(!can_publish(&project(&value).unwrap()));
    }
    #[test]
    fn missing_input_identity_or_chunk_ranges_cannot_publish_any_result() {
        let mut value = output(2 * CHUNK_FRAMES);
        value["chunks"][1]["startFrame"] = json!(CHUNK_FRAMES + 1);
        assert!(project(&value).is_err());
        let mut value = output(CHUNK_FRAMES);
        change(&mut value, 0, |native| {
            native["inputAudioSha256"] = json!("b".repeat(64))
        });
        assert!(project(&value).is_err());
        let mut value = output(CHUNK_FRAMES);
        value["processingPolicy"] = json!("explicit-diagnostic-chunks-v1");
        assert!(project(&value).is_err());
    }
    #[test]
    fn silence_and_unprocessed_audio_are_distinct() {
        let mut value = output(CHUNK_FRAMES);
        change(&mut value, 0, |native| {
            native["rawText"] = json!("");
            native["segments"] = json!([]);
            native["generationMetadata"][0]["generationTokens"] = json!(0);
        });
        let silence = project(&value).unwrap();
        assert!(silence.segments.is_empty());
        assert!(can_publish(&silence));
        value["chunks"][0]["rawBytes"] = json!([]);
        value["chunks"][0]["notProcessed"] = json!(true);
        let missing = project(&value).unwrap();
        assert!(!can_publish(&missing));
        assert_eq!(
            missing.source.unwrap().recognition_report.unwrap().chunks[0].state,
            RecognitionChunkState::NotProcessed
        );
    }

    #[test]
    fn generation_capacity_reserves_the_native_eos_position() {
        let mut value = output(CHUNK_FRAMES);
        change(&mut value, 0, |native| {
            native["generationMetadata"][0]["contextSize"] = json!(1300);
            native["generationMetadata"][0]["effectiveMaxTokens"] = json!(299);
        });
        assert!(project(&value)
            .unwrap()
            .source
            .unwrap()
            .recognition_report
            .unwrap()
            .text_complete());
        change(&mut value, 0, |native| {
            native["generationMetadata"][0]["effectiveMaxTokens"] = json!(300)
        });
        assert!(!can_publish(&project(&value).unwrap()));
    }

    #[test]
    fn partial_storage_survives_restart_and_a_retry_cannot_overwrite_manual_edits() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().canonicalize().unwrap();
        let wav = directory.join("synthetic.wav");
        let mut writer = hound::WavWriter::create(
            &wav,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..64 * 16000 {
            writer.write_sample(10i16).unwrap();
        }
        writer.finalize().unwrap();
        let root = directory.join("store");
        let mut engine = Engine::open(root.clone()).unwrap();
        let meeting = engine
            .import_audio("합성 회의".into(), wav.clone(), wav)
            .unwrap();
        let mut partial = output(64 * 16000);
        change(&mut partial, 1, |native| {
            native["parsedCompletely"] = json!(false)
        });
        let initial = engine
            .commit_moss_chunk_transcript(&meeting.id, None, serde_json::to_vec(&partial).unwrap())
            .unwrap();
        assert_eq!(initial.segments.len(), 1);
        assert!(!initial
            .source
            .as_ref()
            .unwrap()
            .recognition_report
            .as_ref()
            .unwrap()
            .text_complete());
        drop(engine);
        let mut engine = Engine::open(root).unwrap();
        let read = engine.transcript(&meeting.id).unwrap().unwrap();
        assert_eq!(read.version, initial.version);
        let edited = engine
            .edit_transcript(
                &meeting.id,
                read.version,
                vec!["사용자가 편집한 본문".into()],
            )
            .unwrap();
        let complete = serde_json::to_vec(&output(64 * 16000)).unwrap();
        assert!(engine
            .commit_moss_chunk_transcript(&meeting.id, Some(initial.version), complete)
            .is_err());
        assert_eq!(
            engine.transcript(&meeting.id).unwrap().unwrap().segments[0].text,
            "사용자가 편집한 본문"
        );
        let history = engine.transcript_history(&meeting.id).unwrap();
        let completed = history
            .entries
            .iter()
            .find(|entry| entry.segment_count == 2)
            .unwrap();
        let restored = engine
            .restore_transcript(&meeting.id, edited.version, completed.version.clone())
            .unwrap();
        assert_eq!(restored.segments.len(), 2);
        assert!(restored
            .source
            .unwrap()
            .recognition_report
            .unwrap()
            .text_complete());
    }
}
