//! Versioned product output. Historical MOSS whole-recording/diagnostic contracts
//! remain in the parent module and are never accepted through this decoder.
use super::*;

pub const POLICY: &str = "moss-independent-chunks-v1";
pub(crate) const PROJECTOR: &str = "moss-independent-chunks-projector-v3";
const PROJECTOR_V2: &str = "moss-independent-chunks-projector-v2";
const STORED_PROJECTOR: &str = "moss-independent-chunks-projector-v1";
pub const CHUNK_FRAMES: u64 = 60 * SOURCE_TIMEBASE;
// A failed chunk shorter than this is not recognized again in halves.
pub const SPLIT_MINIMUM_FRAMES: u64 = 2 * SOURCE_TIMEBASE;
const MAX_RAW_BYTES: usize = 16 * 1024 * 1024;
// v2: two centisecond units observed with the pinned model, at the input rate.
const TERMINAL_OVERRUN_FRAMES_V2: u64 = 320;
// v3: 100 ms.
const TERMINAL_OVERRUN_FRAMES: u64 = 1600;

// What each stored projector version accepts. Documents keep the version they were
// projected with, so history re-decodes them with the same rules.
#[derive(Clone, Copy)]
struct Rules {
    // Clamp a bounded terminal overrun to the input end instead of a context range.
    normalize_end: bool,
    overrun_frames: u64,
    // Ignore what starts at or after the input end, and allow chunks split in pieces.
    after_end: bool,
}

impl Rules {
    fn of(projector: &str) -> Option<Self> {
        Some(match projector {
            PROJECTOR => Self {
                normalize_end: true,
                overrun_frames: TERMINAL_OVERRUN_FRAMES,
                after_end: true,
            },
            PROJECTOR_V2 => Self {
                normalize_end: true,
                overrun_frames: TERMINAL_OVERRUN_FRAMES_V2,
                after_end: false,
            },
            STORED_PROJECTOR => Self {
                normalize_end: false,
                overrun_frames: TERMINAL_OVERRUN_FRAMES_V2,
                after_end: false,
            },
            _ => return None,
        })
    }
}

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
impl RecognitionChunk {
    // Nothing more can be recognized here: complete, failed again in pieces, or failed and
    // too short to split, so a retry would give the deterministic model the same input.
    // Decided here rather than in the stored projection, so saved reports stay as they are.
    pub fn settled(&self) -> bool {
        self.state == RecognitionChunkState::Complete
            || self.issue.as_deref() == Some("splitFailed")
            || (self.state == RecognitionChunkState::Failed
                && self.end_frame.saturating_sub(self.start_frame) < SPLIT_MINIMUM_FRAMES)
    }
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
    // Schema 2: the worker read this chunk from a separate WAV recorded while the
    // meeting was still being captured.
    #[serde(default)]
    input: Option<ChunkInput>,
    // Schema 3: after the whole chunk failed, its range recognized again as pieces,
    // each from its own WAV. `raw_bytes` keeps the failed whole-chunk output.
    #[serde(default)]
    pieces: Option<Vec<ChunkPiece>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChunkPiece {
    raw_bytes: Vec<u8>,
    input: ChunkInput,
}

/// Where a chunk recognized from its own WAV sits in the recording. `sha256` is the
/// digest of `canonical_wav_header(frames)` followed by the recording's PCM16 from
/// `offset_frames`, so it can be recomputed from the stored audio and the raw output.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChunkInput {
    sha256: String,
    frames: u64,
    offset_frames: u64,
}

/// The 44-byte header of a 16 kHz mono PCM16 WAV holding `frames` samples.
pub fn canonical_wav_header(frames: u64) -> Result<[u8; 44]> {
    let data = frames
        .checked_mul(2)
        .and_then(|bytes| u32::try_from(bytes).ok())
        .filter(|data| *data <= u32::MAX - 36)
        .ok_or_else(invalid)?;
    let rate = SOURCE_TIMEBASE as u32;
    let mut header = [0_u8; 44];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(36 + data).to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16_u32.to_le_bytes());
    header[20..22].copy_from_slice(&1_u16.to_le_bytes());
    header[22..24].copy_from_slice(&1_u16.to_le_bytes());
    header[24..28].copy_from_slice(&rate.to_le_bytes());
    header[28..32].copy_from_slice(&(rate * 2).to_le_bytes());
    header[32..34].copy_from_slice(&2_u16.to_le_bytes());
    header[34..36].copy_from_slice(&16_u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data.to_le_bytes());
    Ok(header)
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
    Rules::of(version).is_some()
}

// The start of every timed span in native text, with whether it carries text. Only
// called after `complete_text` accepted the text.
fn span_starts(raw: &str) -> Vec<(f64, bool)> {
    let mut spans = Vec::new();
    let mut rest = raw.trim();
    while let Some((start, mut body)) = timestamp(rest) {
        if body.starts_with("[S") {
            if let Some((_, tail)) = body[1..].split_once(']') {
                body = tail;
            }
        }
        let Some((offset, tail)) = body
            .match_indices('[')
            .find_map(|(i, _)| timestamp(&body[i..]).map(|(_, tail)| (i, tail)))
        else {
            break;
        };
        spans.push((start, !body[..offset].trim().is_empty()));
        rest = tail.trim();
    }
    spans
}

struct Projected {
    segments: Vec<TranscriptSegment>,
    contexts: Vec<u32>,
    normalized: Vec<u32>,
    after_end: bool,
}

// One native output read from the input `[start, end)` of the recording. `None` when
// the output does not form a complete, valid recognition of that input.
#[allow(clippy::too_many_arguments)]
fn project_native(
    native: NativeChunk,
    start: u64,
    end: u64,
    source_frames: u64,
    source_rate: u32,
    first_index: usize,
    rules: Rules,
) -> Result<Option<Projected>> {
    if !(native.parsed_completely
        && complete_text(&native.raw_text, &native.segments)
        && native.generation_metadata.len() == 1
        && generation_complete(&native.generation_metadata[0], end - start)
        && (native.raw_text.trim().is_empty()
            || native.generation_metadata[0]["generationTokens"]
                .as_u64()
                .is_some_and(|tokens| tokens > 0))
        && native.raw_text.len() <= 1024 * 1024
        && !native.raw_text.contains('\0'))
    {
        return Ok(None);
    }
    let duration = source_frames as f64 / source_rate as f64;
    let local_duration = (end - start) as f64 / SOURCE_TIMEBASE as f64;
    let offset = start as f64 / SOURCE_TIMEBASE as f64;
    let playback_end = (end as f64 / SOURCE_TIMEBASE as f64).min(duration);
    let end_limit = if rules.normalize_end {
        // Subtract exact frame addresses before converting the final tail to seconds.
        let source_local = u128::from(source_frames) * u128::from(SOURCE_TIMEBASE)
            - u128::from(start) * u128::from(source_rate);
        let source_limit = (source_local + u128::from(rules.overrun_frames) * u128::from(source_rate))
            as f64
            / (u128::from(source_rate) * u128::from(SOURCE_TIMEBASE)) as f64;
        ((end - start + rules.overrun_frames) as f64 / SOURCE_TIMEBASE as f64).min(source_limit)
    } else {
        local_duration + 0.02
    };
    let mut projected = Projected {
        segments: Vec::new(),
        contexts: Vec::new(),
        normalized: Vec::new(),
        after_end: false,
    };
    for segment in native.segments {
        // Text the model placed at or after the end of the audio it was given cannot
        // be in that audio.
        if rules.after_end && segment.start.is_finite() && segment.start >= local_duration {
            projected.after_end = true;
            continue;
        }
        if !segment.start.is_finite()
            || !segment.end.is_finite()
            || segment.start < 0.0
            || segment.start > local_duration
            || segment.end < segment.start
            || segment.end > end_limit
            || segment.text.len() > 1024 * 1024
            || segment.text.contains('\0')
        {
            return Ok(None);
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
            u32::try_from(first_index + projected.segments.len()).map_err(|_| invalid())?;
        let overrun = segment.end > local_duration || offset + segment.end > duration;
        let projected_start = offset + segment.start;
        let projected_end = (offset + segment.end).min(playback_end);
        if rules.normalize_end && overrun {
            if projected_end <= projected_start {
                return Ok(None);
            }
            projected.normalized.push(source_index);
        }
        let context = segment.start == segment.end || (!rules.normalize_end && overrun);
        if context {
            projected.contexts.push(source_index);
        }
        projected.segments.push(TranscriptSegment {
            id: Uuid::new_v4().to_string(),
            start: if context { offset } else { projected_start },
            end: if context { playback_end } else { projected_end },
            original_text: text.clone(),
            text,
            speaker_id: None,
            source_index: Some(source_index),
        });
    }
    // Empty output is silence only when both native forms are empty; from v3 a timed
    // span that starts at or after the input end does not count.
    let silent = if rules.after_end {
        span_starts(&native.raw_text)
            .iter()
            .all(|(start, _)| *start >= local_duration)
    } else {
        native.raw_text.trim().is_empty()
    };
    if projected.segments.is_empty() && !silent {
        return Ok(None);
    }
    if rules.after_end
        && span_starts(&native.raw_text)
            .iter()
            .any(|(start, _)| *start >= local_duration)
    {
        projected.after_end = true;
    }
    Ok(Some(projected))
}

pub(super) fn decode_with_projector(
    raw: &[u8],
    source_frames: u64,
    source_rate: u32,
    session: &str,
    id: &str,
    projector: &str,
) -> Result<Transcript> {
    let rules = Rules::of(projector).ok_or_else(invalid)?;
    if raw.len() > MAX_RAW_BYTES {
        return Err(invalid());
    }
    let output: ProductOutput = serde_json::from_slice(raw).map_err(|_| invalid())?;
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../resources/models/moss.json"))
            .map_err(|_| invalid())?;
    let expected_frames =
        crate::audio_normalize::output_frames(source_frames, source_rate).map_err(|_| invalid())?;
    if !matches!(output.schema_version, 1 | 2 | 3)
        || (output.schema_version == 3 && !rules.after_end)
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
            || (record.input.is_some() && (output.schema_version < 2 || record.not_processed))
            || (record.pieces.is_some()
                && (output.schema_version < 3 || record.not_processed || record.input.is_some()))
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
        if record.input.is_some() && native.is_none() {
            return Err(invalid());
        }
        if let Some(native) = native {
            let identified = match &record.input {
                None => {
                    native.index == record.index
                        && native.start_frame == start
                        && native.end_frame == end
                        && native.input_audio_sha256 == output.input_audio_sha256
                }
                // A whole chunk read from its own WAV, at a chunk boundary of that file.
                Some(input) => {
                    digest(&input.sha256)
                        && native.input_audio_sha256 == input.sha256
                        && end - start == CHUNK_FRAMES
                        && input.offset_frames % CHUNK_FRAMES == 0
                        && u64::from(native.index) * CHUNK_FRAMES == native.start_frame
                        && native.start_frame.checked_add(input.offset_frames) == Some(start)
                        && native.end_frame.checked_add(input.offset_frames) == Some(end)
                        && native.end_frame <= input.frames
                }
            };
            if !identified {
                return Err(invalid());
            }
            // A chunk split in pieces takes its result from them; the whole-chunk
            // output stays as the evidence of the first attempt.
            if record.pieces.is_none() {
                if let Some(projected) = project_native(
                    native,
                    start,
                    end,
                    source_frames,
                    source_rate,
                    segments.len(),
                    rules,
                )? {
                    chunk.state = RecognitionChunkState::Complete;
                    chunk.issue = projected.after_end.then(|| "afterEndIgnored".into());
                    report.timing_context_segments.extend(projected.contexts);
                    report.normalized_end_segments.extend(projected.normalized);
                    segments.extend(projected.segments);
                }
            }
        }
        if let Some(pieces) = record.pieces {
            if pieces.len() < 2 {
                return Err(invalid());
            }
            let mut at = start;
            let mut parts = Vec::new();
            for piece in pieces {
                let input = &piece.input;
                let piece_end = at.checked_add(input.frames).ok_or_else(invalid)?;
                if !digest(&input.sha256)
                    || input.offset_frames != at
                    || input.frames == 0
                    || piece_end > end
                    || piece.raw_bytes.len() > 2 * 1024 * 1024
                {
                    return Err(invalid());
                }
                // Unreadable output is a failed piece, kept as evidence; output that
                // claims another input is not this piece at all.
                if let Ok(native) = serde_json::from_slice::<NativeChunk>(&piece.raw_bytes) {
                    if native.index != 0
                        || native.start_frame != 0
                        || native.end_frame != input.frames
                        || native.input_audio_sha256 != input.sha256
                    {
                        return Err(invalid());
                    }
                    parts.push(Some((native, at, piece_end)));
                } else {
                    parts.push(None);
                }
                at = piece_end;
            }
            if at != end {
                return Err(invalid());
            }
            let mut pieced = Vec::new();
            let mut complete = true;
            let mut after_end = false;
            for part in parts {
                let projection = match part {
                    Some((native, from, to)) => project_native(
                        native,
                        from,
                        to,
                        source_frames,
                        source_rate,
                        segments.len() + pieced.iter().map(|p: &Projected| p.segments.len()).sum::<usize>(),
                        rules,
                    )?,
                    None => None,
                };
                match projection {
                    Some(projected) => {
                        after_end |= projected.after_end;
                        pieced.push(projected);
                    }
                    None => complete = false,
                }
            }
            if complete {
                chunk.state = RecognitionChunkState::Complete;
                chunk.issue = after_end.then(|| "afterEndIgnored".into());
                for projected in pieced {
                    report.timing_context_segments.extend(projected.contexts);
                    report.normalized_end_segments.extend(projected.normalized);
                    segments.extend(projected.segments);
                }
            } else {
                chunk.issue = Some("splitFailed".into());
            }
        }
        // Only finished chunks are carried over from a separate WAV.
        if record.input.is_some() && chunk.state != RecognitionChunkState::Complete {
            return Err(invalid());
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

// An empty transcript is saved once nothing more can be recognized: every chunk is
// complete, or failed again in pieces, so its missing ranges are on record instead of
// a retry that would repeat the same result.
pub(super) fn can_publish(document: &Transcript) -> bool {
    !document.segments.is_empty()
        || document
            .source
            .as_ref()
            .and_then(|source| source.recognition_report.as_ref())
            .is_some_and(|report| {
                report.chunks.iter().all(RecognitionChunk::settled)
            })
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
    fn a_failed_chunk_too_short_to_split_is_settled_and_a_longer_one_stays_open() {
        let failed = |frames: u64| {
            let mut value = output(frames);
            change(&mut value, 1, |native| {
                native["parsedCompletely"] = json!(false)
            });
            project(&value)
                .unwrap()
                .source
                .unwrap()
                .recognition_report
                .unwrap()
                .chunks[1]
                .clone()
        };
        // The stored projection keeps its issue; only the judgement changes.
        let short = failed(CHUNK_FRAMES + SPLIT_MINIMUM_FRAMES / 2);
        assert_eq!(short.issue.as_deref(), Some("nativeOutputInvalid"));
        assert!(short.settled());
        let long = failed(2 * CHUNK_FRAMES);
        assert!(!long.settled());
        // A damaged report with its range reversed has nothing to split.
        let reversed = RecognitionChunk {
            start_frame: long.end_frame,
            end_frame: long.start_frame,
            ..long
        };
        assert!(reversed.settled());
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
        // (start, end, accepted, kept as a segment)
        for (start, end, accepted, kept) in [
            (59.5, 60.0, true, true),
            (59.5, 60.04, true, true),
            (59.5, 60.1, true, true),
            (59.5, 60.100001, false, false),
            (59.5, 61.0, false, false),
            // Starting at or after the input end: not in this audio, ignored.
            (60.0, 60.01, true, false),
            (60.01, 60.06, true, false),
            (-0.01, 0.5, false, false),
            (0.5, 0.4, false, false),
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
            assert_eq!(doc.segments.len(), if kept { 2 } else { 1 }, "{start}..{end}");
            assert_eq!(doc.segments.last().unwrap().start, 60.0);
            assert_eq!(report.needs_review(), !accepted);
            assert_eq!(
                report.chunks[0].issue.as_deref(),
                if !accepted {
                    Some("nativeOutputInvalid")
                } else if kept {
                    None
                } else {
                    Some("afterEndIgnored")
                }
            );
            assert_eq!(
                report.normalized_end_segments,
                if kept && end > 60.0 { vec![0] } else { vec![] }
            );
        }
        // Documents projected with v2 keep its 20 ms limit.
        let mut value = output(2 * CHUNK_FRAMES);
        set_time(&mut value, 0, 59.5, 60.04);
        let raw = serde_json::to_vec(&value).unwrap();
        let v2 = decode_with_projector(&raw, 2 * CHUNK_FRAMES, 16000, "s", "g", PROJECTOR_V2)
            .unwrap();
        assert!(v2.source.unwrap().recognition_report.unwrap().needs_review());
    }

    // The four ways a 60 s chunk ends badly.
    #[test]
    fn observed_end_of_chunk_failures_keep_their_text_and_a_repetition_still_fails() {
        let mut value = output(5 * CHUNK_FRAMES);
        // Text, then a punctuation span starting exactly at the end.
        change(&mut value, 0, |native| {
            native["rawText"] = json!("[3.84][S01]문장.[5.12][60.00][S02]...[60.02]");
            native["segments"] = json!([
                {"start":3.84,"end":5.12,"text":"[S01] 문장.","speaker_id":"S01"},
                {"start":60.0,"end":60.02,"text":"[S02] ...","speaker_id":"S02"}]);
        });
        // The last segment ends 40 ms past the end.
        change(&mut value, 1, |native| {
            native["rawText"] = json!("[58.44][S02]마지막 문장[60.04]");
            native["segments"] =
                json!([{"start":58.44,"end":60.04,"text":"[S02] 마지막 문장","speaker_id":"S02"}]);
        });
        // Only an empty span after the end.
        change(&mut value, 2, |native| {
            native["rawText"] = json!("[60.00][S01][60.06]");
            native["segments"] = json!([]);
        });
        // A repetition stopped by the model's guard.
        change(&mut value, 3, |native| {
            native["rawText"] = json!("[0.00][S01] 응 응 응 \u{fffd}");
            native["segments"] = json!([]);
            native["parsedCompletely"] = json!(false);
            native["generationMetadata"][0]["terminationReason"] = json!("repetition");
        });
        let doc = project(&value).unwrap();
        let report = doc.source.unwrap().recognition_report.unwrap();
        let states: Vec<_> = report.chunks.iter().map(|chunk| chunk.state.clone()).collect();
        assert_eq!(
            states,
            [
                RecognitionChunkState::Complete,
                RecognitionChunkState::Complete,
                RecognitionChunkState::Complete,
                RecognitionChunkState::Failed,
                RecognitionChunkState::Complete
            ]
        );
        assert_eq!(report.chunks[0].issue.as_deref(), Some("afterEndIgnored"));
        assert_eq!(report.chunks[2].issue.as_deref(), Some("afterEndIgnored"));
        let texts: Vec<_> = doc.segments.iter().map(|s| (s.start, s.end, s.text.as_str())).collect();
        assert_eq!(texts[0], (3.84, 5.12, "문장."));
        assert_eq!(texts[1], (60.0 + 58.44, 120.0, "마지막 문장"));
        assert_eq!(texts.len(), 3);
        assert_eq!(report.normalized_end_segments, vec![1]);
    }

    fn piece(start: u64, frames: u64, sha: &str, action: impl FnOnce(&mut Value)) -> Value {
        let mut raw = native(0, 0, frames);
        raw["inputAudioSha256"] = json!(sha);
        action(&mut raw);
        json!({"rawBytes":serde_json::to_vec(&raw).unwrap(),"input":{"sha256":sha,"frames":frames,"offsetFrames":start}})
    }

    #[test]
    fn a_failed_chunk_split_in_two_pieces_takes_their_text_at_their_places() {
        let frames = 2 * CHUNK_FRAMES;
        let half = CHUNK_FRAMES / 2;
        let mut value = output(frames);
        value["schemaVersion"] = json!(3);
        change(&mut value, 1, |native| native["parsedCompletely"] = json!(false));
        let pieces = |second: Box<dyn FnOnce(&mut Value)>| {
            json!([
                piece(CHUNK_FRAMES, half, &"b".repeat(64), |_| {}),
                piece(CHUNK_FRAMES + half, half, &"c".repeat(64), second)
            ])
        };
        value["chunks"][1]["pieces"] = pieces(Box::new(|_| {}));
        let doc = project(&value).unwrap();
        let report = doc.source.as_ref().unwrap().recognition_report.as_ref().unwrap();
        assert!(report.text_complete());
        let starts: Vec<_> = doc.segments.iter().map(|s| (s.start, s.source_index)).collect();
        assert_eq!(starts, [(0.0, Some(0)), (60.0, Some(1)), (90.0, Some(2))]);
        // The whole-chunk output is still the first attempt's evidence.
        assert_eq!(report.chunks[1].issue, None);

        let mut failed = value.clone();
        failed["chunks"][1]["pieces"] =
            pieces(Box::new(|raw| raw["parsedCompletely"] = json!(false)));
        let doc = project(&failed).unwrap();
        let report = doc.source.unwrap().recognition_report.unwrap();
        assert_eq!(report.chunks[1].state, RecognitionChunkState::Failed);
        assert_eq!(report.chunks[1].issue.as_deref(), Some("splitFailed"));
        assert_eq!(doc.segments.len(), 1);

        let mut cases: Vec<Box<dyn Fn(&mut Value)>> = vec![
            Box::new(|v| v["schemaVersion"] = json!(2)),
            Box::new(|v| v["chunks"][1]["pieces"][1]["input"]["offsetFrames"] = json!(CHUNK_FRAMES + half + 1)),
            Box::new(|v| v["chunks"][1]["pieces"][1]["input"]["frames"] = json!(half - 1)),
            Box::new(|v| v["chunks"][1]["pieces"][1]["input"]["sha256"] = json!("d".repeat(64))),
            Box::new(|v| {
                let first = v["chunks"][1]["pieces"][0].clone();
                v["chunks"][1]["pieces"] = json!([first]);
            }),
            Box::new(|v| v["chunks"][1]["input"] = json!({"sha256":"b".repeat(64),"frames":CHUNK_FRAMES,"offsetFrames":CHUNK_FRAMES})),
        ];
        cases.push(Box::new(move |v| {
            v["chunks"][1]["pieces"][0] = piece(CHUNK_FRAMES, half, &"b".repeat(64), |raw| raw["index"] = json!(1));
        }));
        for (case, mutate) in cases.into_iter().enumerate() {
            let mut broken = value.clone();
            mutate(&mut broken);
            assert!(project(&broken).is_err(), "case {case}");
        }
        let raw = serde_json::to_vec(&value).unwrap();
        assert!(decode_with_projector(&raw, frames, 16000, "s", "g", PROJECTOR_V2).is_err());
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
        set_time(&mut value, 1, 2.5, 3.11);
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
            .commit_transcript(
                &meeting.id,
                None,
                raw.clone(),
                crate::AsrProvider::MossChunks,
            )
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
        // A chunk that already failed in pieces is never tried again, so the empty
        // transcript is recorded rather than left waiting for a retry.
        let mut settled = missing.clone();
        let report = settled.source.as_mut().unwrap().recognition_report.as_mut().unwrap();
        report.chunks[0].issue = Some("splitFailed".into());
        assert!(can_publish(&settled));
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
            .commit_transcript(
                &meeting.id,
                None,
                serde_json::to_vec(&partial).unwrap(),
                crate::AsrProvider::MossChunks,
            )
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
            .commit_transcript(
                &meeting.id,
                Some(initial.version),
                complete,
                crate::AsrProvider::MossChunks
            )
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

    // Chunk `index` recognized from its own one-chunk WAV, mapped back into the recording.
    fn separate(value: &mut Value, index: usize) {
        let start = index as u64 * CHUNK_FRAMES;
        let mut raw = native(0, 0, CHUNK_FRAMES);
        raw["inputAudioSha256"] = json!("b".repeat(64));
        value["schemaVersion"] = json!(2);
        value["chunks"][index]["rawBytes"] = json!(serde_json::to_vec(&raw).unwrap());
        value["chunks"][index]["input"] =
            json!({"sha256":"b".repeat(64),"frames":CHUNK_FRAMES,"offsetFrames":start});
    }

    #[test]
    fn a_chunk_from_its_own_wav_projects_like_the_same_chunk_of_the_whole_recording() {
        let frames = 2 * CHUNK_FRAMES + 16000;
        let whole = project(&output(frames)).unwrap();
        let mut value = output(frames);
        separate(&mut value, 1);
        let mapped = project(&value).unwrap();
        let shape = |document: &Transcript| {
            document
                .segments
                .iter()
                .map(|segment| (segment.start, segment.end, segment.text.clone(), segment.source_index))
                .collect::<Vec<_>>()
        };
        assert_eq!(shape(&mapped), shape(&whole));
        assert_eq!(mapped.segments[1].start, 60.0);
        assert_eq!(
            mapped.source.as_ref().unwrap().recognition_report,
            whole.source.as_ref().unwrap().recognition_report
        );
        assert_eq!(
            mapped.source.unwrap().chunking_policy.as_deref(),
            Some(POLICY)
        );
    }

    #[test]
    fn a_mapped_chunk_must_match_its_own_wav_and_be_a_finished_whole_chunk() {
        let frames = 2 * CHUNK_FRAMES + 16000;
        let mut cases: Vec<Box<dyn Fn(&mut Value)>> = vec![
            Box::new(|value| value["chunks"][1]["input"]["sha256"] = json!("c".repeat(64))),
            Box::new(|value| value["chunks"][1]["input"]["offsetFrames"] = json!(CHUNK_FRAMES + 1)),
            Box::new(|value| value["chunks"][1]["input"]["offsetFrames"] = json!(0)),
            Box::new(|value| value["chunks"][1]["input"]["frames"] = json!(CHUNK_FRAMES - 1)),
            Box::new(|value| change(value, 1, |native| native["index"] = json!(1))),
            Box::new(|value| change(value, 1, |native| native["parsedCompletely"] = json!(false))),
            Box::new(|value| value["schemaVersion"] = json!(1)),
            Box::new(|value| {
                value["chunks"][1]["notProcessed"] = json!(true);
                value["chunks"][1]["rawBytes"] = json!([]);
            }),
            Box::new(|value| value["chunks"][1]["input"]["unknown"] = json!(1)),
        ];
        // The last, partial chunk is never taken from a separate WAV.
        cases.push(Box::new(|value| {
            value["chunks"][2]["input"] =
                json!({"sha256":"a".repeat(64),"frames":16000,"offsetFrames":2 * CHUNK_FRAMES});
        }));
        for (case, mutate) in cases.into_iter().enumerate() {
            let mut value = output(frames);
            separate(&mut value, 1);
            mutate(&mut value);
            assert!(project(&value).is_err(), "case {case}");
        }
        let mut batch = output(frames);
        separate(&mut batch, 1);
        // Two chunks read from one WAV starting at chunk 0 of the recording.
        let mut raw = native(1, CHUNK_FRAMES, 2 * CHUNK_FRAMES);
        raw["inputAudioSha256"] = json!("b".repeat(64));
        batch["chunks"][1]["rawBytes"] = json!(serde_json::to_vec(&raw).unwrap());
        batch["chunks"][1]["input"] =
            json!({"sha256":"b".repeat(64),"frames":2 * CHUNK_FRAMES,"offsetFrames":0});
        assert!(project(&batch).is_ok());
    }

    #[test]
    fn the_canonical_chunk_header_is_what_a_wav_writer_produces_for_16khz_mono_pcm16() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chunk.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for sample in 0..1000_i16 {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(bytes.len(), 44 + 2000);
        assert_eq!(&bytes[..44], &canonical_wav_header(1000).unwrap());
        assert!(canonical_wav_header(u64::MAX).is_err());
    }
}
