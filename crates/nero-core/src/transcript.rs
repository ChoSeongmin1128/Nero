use crate::engine::Engine;
use crate::store::{self, CapturePhase, Encoding, Manifest};
use crate::{CoreError, Meeting, MeetingStatus, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const PROJECTOR_VERSION: &str = "moss-chunk-speakers-v1";
const RECORDING_PROJECTOR_VERSION: &str = "moss-recording-speakers-v1";
const WHOLE_RECORDING_POLICY: &str = "whole-recording-v1";
const WHOLE_RECORDING_RUNTIME: &str = "mlx-audio-swift-v0.1.3+nero-moss-whole-v2";
const STORED_WHOLE_RECORDING_RUNTIME_V1: &str = "mlx-audio-swift-v0.1.3+nero-moss-whole-v1";
const MOSS_PARSE_INCOMPLETE: &str = "일부 발언을 끝까지 읽지 못했습니다. 모델 원본은 보관했습니다.";
const MOSS_PCM_INCOMPLETE: &str = "전사가 끝까지 완료되지 않았습니다. 원본 결과는 보관했습니다.";
const LEGACY_ASSOCIATION: &str = "legacy-initial-order-v1";
const SOURCE_TIMEBASE: u64 = 16000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscriptVersion {
    pub id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscriptSegment {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub original_text: String,
    pub text: String,
    #[serde(default)]
    pub speaker_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscriptSpeaker {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_origin: Option<SpeakerNameOrigin>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SpeakerNameOrigin {
    Automatic,
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscriptSource {
    pub raw_sha256: String,
    pub projector_version: String,
    pub association: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunking_policy: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptUpgradeReport {
    pub upgraded: u64,
    pub unchanged: u64,
    pub unavailable: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transcript {
    pub version: TranscriptVersion,
    pub source_session_id: String,
    pub model: String,
    pub model_revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<TranscriptSource>,
    #[serde(default)]
    pub speakers: Vec<TranscriptSpeaker>,
    pub segments: Vec<TranscriptSegment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_analysis: Option<crate::SpeakerAnalysis>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptHistoryEntry {
    pub version: TranscriptVersion,
    pub updated_at: u64,
    pub segment_count: usize,
    pub speaker_count: usize,
    pub edited: bool,
    pub current: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptHistory {
    pub entries: Vec<TranscriptHistoryEntry>,
    pub unavailable: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MossOutput {
    model: String,
    segments: Vec<MossSegment>,
    hit_token_limit: bool,
    has_unparsed_text: bool,
    processed_audio_frames: u64,
    input_audio_frames: u64,
    audio_duration_seconds: f64,
    #[serde(default, deserialize_with = "present_chunk")]
    speaker_scope: Option<serde_json::Value>,
    #[serde(default)]
    chunking_policy: Option<serde_json::Value>,
    #[serde(default)]
    generation_count: Option<serde_json::Value>,
    #[serde(default)]
    max_tokens: Option<serde_json::Value>,
    #[serde(default)]
    chunk_duration_seconds: Option<serde_json::Value>,
    #[serde(default)]
    generation_tokens: Option<serde_json::Value>,
    #[serde(default)]
    chunks: Option<serde_json::Value>,
    #[serde(default)]
    runtime: Option<serde_json::Value>,
    #[serde(default)]
    effective_max_tokens: Option<serde_json::Value>,
    #[serde(default)]
    prompt_tokens: Option<serde_json::Value>,
    #[serde(default)]
    context_size: Option<serde_json::Value>,
    #[serde(default)]
    termination_reason: Option<serde_json::Value>,
    #[serde(default)]
    generation_metadata: Option<serde_json::Value>,
    #[serde(flatten)]
    generation_evidence: MossGenerationEvidence,
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct MossGenerationEvidence {
    #[serde(default)]
    metadata_schema_version: Option<serde_json::Value>,
    #[serde(default)]
    eos_policy: Option<serde_json::Value>,
    #[serde(default)]
    sampling_policy: Option<serde_json::Value>,
    #[serde(default)]
    terminal_token_id: Option<serde_json::Value>,
    #[serde(default)]
    configured_eos_token_ids: Option<serde_json::Value>,
    #[serde(default)]
    tokenizer_eos_token_id: Option<serde_json::Value>,
    #[serde(default)]
    input_audio_samples: Option<serde_json::Value>,
    #[serde(default)]
    feature_chunk_count: Option<serde_json::Value>,
    #[serde(default)]
    feature_token_lengths: Option<serde_json::Value>,
    #[serde(default)]
    audio_token_count: Option<serde_json::Value>,
    #[serde(default)]
    audio_embedding_count: Option<serde_json::Value>,
}

impl MossGenerationEvidence {
    fn complete_for(&self, frames: u64) -> bool {
        if frames == 0 || frames > 90 * 60 * SOURCE_TIMEBASE {
            return false;
        }
        let lengths: Vec<u64> = (0..frames)
            .step_by(480_000)
            .map(|start| (frames - start).min(480_000).div_ceil(1280))
            .collect();
        let audio_tokens: u64 = lengths.iter().sum();
        self.metadata_schema_version
            .as_ref()
            .and_then(|v| v.as_u64())
            == Some(2)
            && self.eos_policy.as_ref().and_then(|v| v.as_str())
                == Some("model-generation-config+tokenizer-v1")
            && self.sampling_policy.as_ref().and_then(|v| v.as_str()) == Some("greedy-argmax-v1")
            && self.terminal_token_id.as_ref().and_then(|v| v.as_u64()) == Some(151645)
            && self
                .tokenizer_eos_token_id
                .as_ref()
                .and_then(|v| v.as_u64())
                == Some(151645)
            && self.configured_eos_token_ids.as_ref() == Some(&serde_json::json!([151645]))
            && self.input_audio_samples.as_ref().and_then(|v| v.as_u64()) == Some(frames)
            && self.feature_chunk_count.as_ref().and_then(|v| v.as_u64())
                == Some(lengths.len() as u64)
            && self.feature_token_lengths.as_ref() == Some(&serde_json::json!(lengths))
            && self.audio_token_count.as_ref().and_then(|v| v.as_u64()) == Some(audio_tokens)
            && self.audio_embedding_count.as_ref().and_then(|v| v.as_u64()) == Some(audio_tokens)
    }
}

#[derive(Deserialize)]
struct MossSegment {
    start: f64,
    end: f64,
    text: String,
    speaker_id: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "present_chunk")]
    asr_chunk: Option<serde_json::Value>,
}

#[derive(PartialEq, Eq)]
enum MossValidationPolicy {
    NewRecognition,
    StoredArtifact,
}

enum TranscriptCommitPolicy {
    Moss,
    WholeMoss,
    Nemotron,
}

fn present_chunk<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde_json::Value::deserialize(deserializer).map(Some)
}

impl MossOutput {
    fn recording_scope(&self, policy: MossValidationPolicy) -> Result<bool> {
        let whole_policy = self
            .chunking_policy
            .as_ref()
            .and_then(|value| value.as_str())
            == Some(WHOLE_RECORDING_POLICY);
        let scope = self.speaker_scope.as_ref().and_then(|value| value.as_str());
        let claimed_recording = whole_policy || scope == Some("recording");
        let stored_v1 = policy == MossValidationPolicy::StoredArtifact
            && self.runtime.as_ref().and_then(|value| value.as_str())
                == Some(STORED_WHOLE_RECORDING_RUNTIME_V1);
        if !claimed_recording {
            if self.speaker_scope.is_some() && scope != Some("chunk") {
                return Err(CoreError::InvalidInput(
                    "화자 범위 정보가 올바르지 않습니다.".into(),
                ));
            }
            return Ok(false);
        }
        let incomplete = || {
            let message = "녹음 전체를 한 번에 처리한 실행 정보를 확인할 수 없습니다. 모델 원본은 보관했습니다.";
            CoreError::InvalidInput(message.into())
        };
        let chunks = self
            .chunks
            .as_ref()
            .and_then(|value| value.as_array())
            .ok_or_else(incomplete)?;
        let full_seconds = self.input_audio_frames as f64 / SOURCE_TIMEBASE as f64;
        let chunk_duration = self
            .chunk_duration_seconds
            .as_ref()
            .and_then(|value| value.as_f64());
        let max_tokens = self.max_tokens.as_ref().and_then(|value| value.as_u64());
        let tokens = self
            .generation_tokens
            .as_ref()
            .and_then(|value| value.as_u64());
        let prompt = self.prompt_tokens.as_ref().and_then(|value| value.as_u64());
        let context = self.context_size.as_ref().and_then(|value| value.as_u64());
        let effective = self
            .effective_max_tokens
            .as_ref()
            .and_then(|value| value.as_u64());
        let remaining = context
            .zip(prompt)
            .and_then(|(context, prompt)| context.checked_sub(prompt)?.checked_sub(1));
        match self.termination_reason.as_ref().and_then(|value| value.as_str()) {
            Some("eos") => (),
            Some("length") => return Err(CoreError::InvalidInput("음성 기록이 모델의 생성 한도에 도달해 끝까지 완료되지 않았습니다. 모델 원본은 보관했습니다.".into())),
            Some("repetition") => return Err(CoreError::InvalidInput("음성 기록에서 반복 출력이 감지되어 처리가 멈췄습니다. 모델 원본은 보관했습니다.".into())),
            _ => return Err(incomplete()),
        }
        if self.has_unparsed_text {
            return Err(CoreError::InvalidInput(MOSS_PARSE_INCOMPLETE.into()));
        }
        if self.input_audio_frames == 0 || self.processed_audio_frames != self.input_audio_frames {
            return Err(CoreError::InvalidInput(MOSS_PCM_INCOMPLETE.into()));
        }
        if scope != Some("recording")
            || !whole_policy
            || self
                .generation_count
                .as_ref()
                .and_then(|value| value.as_u64())
                != Some(1)
            || self.runtime.as_ref().and_then(|value| value.as_str())
                != Some(WHOLE_RECORDING_RUNTIME)
                && !stored_v1
            || max_tokens != Some(65536)
            || context != Some(131072)
            || !prompt.is_some_and(|prompt| prompt > 0)
            || !effective.is_some_and(|effective| {
                effective > 0
                    && remaining.is_some_and(|remaining| effective == remaining.min(65536))
            })
            || !tokens
                .zip(effective)
                .is_some_and(|(tokens, effective)| tokens < effective)
            || !chunk_duration.is_some_and(|value| value.is_finite() && value >= full_seconds)
            || self.audio_duration_seconds != full_seconds
            || chunks.len() != 1
            || !stored_v1
                && !self
                    .generation_evidence
                    .complete_for(self.input_audio_frames)
        {
            return Err(incomplete());
        }
        let chunk = &chunks[0];
        if chunk["index"].as_u64() != Some(0)
            || chunk["generationCount"].as_u64() != Some(1)
            || chunk["start"].as_f64() != Some(0.0)
            || chunk["end"].as_f64() != Some(full_seconds)
            || chunk["hitTokenLimit"].as_bool() != Some(false)
            || chunk["parsedCompletely"].as_bool() != Some(true)
            || chunk["generationTokens"].as_u64() != tokens
            || chunk["maxTokens"].as_u64() != max_tokens
            || chunk["effectiveMaxTokens"].as_u64() != effective
            || chunk["promptTokens"].as_u64() != prompt
            || chunk["contextSize"].as_u64() != context
            || chunk["terminationReason"].as_str() != Some("eos")
            || !stored_v1 && !self.generation_matches_chunk(chunk)
            || chunk["rawText"].as_str().is_none()
            || self.segments.iter().any(|segment| {
                segment.asr_chunk.as_ref().and_then(|value| value.as_u64()) != Some(0)
            })
        {
            return Err(incomplete());
        }
        Ok(true)
    }

    fn generation_matches_chunk(&self, chunk: &serde_json::Value) -> bool {
        let Some(generation) = self
            .generation_metadata
            .as_ref()
            .and_then(|value| value.as_array())
            .filter(|items| items.len() == 1)
        else {
            return false;
        };
        let Ok(chunk_evidence) = serde_json::from_value::<MossGenerationEvidence>(chunk.clone())
        else {
            return false;
        };
        let Ok(generation_evidence) =
            serde_json::from_value::<MossGenerationEvidence>(generation[0].clone())
        else {
            return false;
        };
        chunk_evidence == self.generation_evidence
            && generation_evidence == self.generation_evidence
            && chunk.get("generationMetadata") == self.generation_metadata.as_ref()
            && [
                "promptTokens",
                "maxTokens",
                "effectiveMaxTokens",
                "contextSize",
                "generationTokens",
                "terminationReason",
            ]
            .iter()
            .all(|field| generation[0].get(*field) == chunk.get(*field))
    }

    fn decode(raw: &[u8], duration: f64, session_id: &str, id: &str) -> Result<Transcript> {
        Self::decode_with_policy(
            raw,
            duration,
            session_id,
            id,
            MossValidationPolicy::NewRecognition,
        )
    }

    fn decode_with_policy(
        raw: &[u8],
        duration: f64,
        session_id: &str,
        id: &str,
        policy: MossValidationPolicy,
    ) -> Result<Transcript> {
        if raw.len() > 16 * 1024 * 1024 {
            return Err(CoreError::InvalidInput("전사 결과가 너무 큽니다.".into()));
        }
        let output: Self = serde_json::from_slice(raw)
            .map_err(|_| CoreError::InvalidInput("전사 결과를 읽을 수 없습니다.".into()))?;
        let recording_scope = output.recording_scope(policy)?;
        if output.hit_token_limit {
            return Err(CoreError::InvalidInput(
                "음성 기록의 생성 한도에 도달했습니다. 모델 원본은 보관했습니다.".into(),
            ));
        }
        if output.has_unparsed_text {
            return Err(CoreError::InvalidInput(MOSS_PARSE_INCOMPLETE.into()));
        }
        if output.model != "OpenMOSS-Team/MOSS-Transcribe-Diarize"
            || output.input_audio_frames == 0
            || output.processed_audio_frames != output.input_audio_frames
            || !output.audio_duration_seconds.is_finite()
            || (output.audio_duration_seconds - duration).abs() > 0.5
            || (output.input_audio_frames as f64 / 16000.0 - duration).abs() > 0.5
        {
            return Err(CoreError::InvalidInput(MOSS_PCM_INCOMPLETE.into()));
        }
        if output.segments.is_empty() {
            return Err(CoreError::InvalidInput(
                "인식된 발화가 없습니다. 원본 음성은 보관했습니다.".into(),
            ));
        }
        let mut segments = Vec::new();
        let mut speakers: Vec<TranscriptSpeaker> = Vec::new();
        let mut speaker_ids = BTreeSet::new();
        let chunked = output
            .segments
            .iter()
            .any(|segment| segment.asr_chunk.is_some());
        for (index, segment) in output.segments.into_iter().enumerate() {
            if !segment.start.is_finite()
                || !segment.end.is_finite()
                || segment.start < 0.0
                || segment.end < segment.start
                || segment.end > duration + 0.5
                || segment.text.trim().is_empty()
                || segment.text.len() > 2 * 1024 * 1024
                || segment.text.contains('\0')
            {
                return Err(CoreError::InvalidInput(
                    "전사 구간 또는 시각이 올바르지 않습니다.".into(),
                ));
            }
            let model_label = model_speaker_id(segment.speaker_id.as_ref());
            let speaker_id = model_label
                .as_ref()
                .and_then(|label| match &segment.asr_chunk {
                    _ if recording_scope => Some(format!("moss:recording:{label}")),
                    Some(value) => value
                        .as_u64()
                        .map(|chunk| format!("moss:chunk:{chunk}:{label}")),
                    None if !chunked => Some(label.clone()),
                    None => None,
                });
            let mut text = segment.text;
            if let Some(label) = &model_label {
                if let Some(body) = text.strip_prefix(&format!("[{label}] ")) {
                    text = body.to_owned();
                }
            }
            if let Some(label) = &speaker_id {
                if speaker_ids.insert(label.clone()) {
                    speakers.push(TranscriptSpeaker {
                        id: label.clone(),
                        name: format!("화자 {}", speakers.len() + 1),
                        name_origin: Some(SpeakerNameOrigin::Automatic),
                    });
                }
            }
            segments.push(TranscriptSegment {
                id: Uuid::new_v4().to_string(),
                start: segment.start.min(duration),
                end: segment.end.min(duration),
                original_text: text.clone(),
                text,
                speaker_id,
                source_index: Some(u32::try_from(index).map_err(|_| {
                    CoreError::InvalidInput("전사 구간 수가 너무 많습니다.".into())
                })?),
            });
        }
        Ok(Transcript {
            version: TranscriptVersion {
                id: id.into(),
                revision: 0,
            },
            source_session_id: session_id.into(),
            model: output.model,
            model_revision: serde_json::from_str::<serde_json::Value>(include_str!(
                "../resources/models/moss.json"
            ))
            .map_err(|e| CoreError::Storage(e.to_string()))?["revision"]
                .as_str()
                .unwrap_or_default()
                .into(),
            source: Some(TranscriptSource {
                raw_sha256: sha256(raw),
                projector_version: if recording_scope {
                    RECORDING_PROJECTOR_VERSION
                } else {
                    PROJECTOR_VERSION
                }
                .into(),
                association: "direct".into(),
                speaker_scope: recording_scope.then(|| "recording".into()),
                chunking_policy: recording_scope.then(|| WHOLE_RECORDING_POLICY.into()),
            }),
            speakers,
            segments,
            speaker_analysis: None,
        })
    }
}

fn model_speaker_id(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(label)
            if label.len() > 1
                && label.len() <= 64
                && label.starts_with('S')
                && label[1..].bytes().all(|b| b.is_ascii_digit()) =>
        {
            Some(label.clone())
        }
        serde_json::Value::Number(number) => number
            .as_u64()
            .filter(|id| *id > 0)
            .map(|id| format!("S{id}")),
        _ => None,
    }
}

fn validate_speakers(speakers: &[TranscriptSpeaker], assignments: &[Option<String>]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for speaker in speakers {
        if speaker.id.is_empty()
            || speaker.id.len() > 128
            || !speaker
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b':')
            || speaker.name.trim().is_empty()
            || speaker.name.len() > 256
            || speaker.name.chars().any(char::is_control)
            || !ids.insert(speaker.id.as_str())
        {
            return Err(CoreError::InvalidInput(
                "화자 이름 또는 ID가 올바르지 않습니다.".into(),
            ));
        }
    }
    if speakers.len() > 4096
        || assignments
            .iter()
            .flatten()
            .any(|id| !ids.contains(id.as_str()))
    {
        return Err(CoreError::InvalidInput(
            "화자 배정이 올바르지 않습니다.".into(),
        ));
    }
    Ok(())
}

fn transcript_dir(meeting_id: &str, transcript_id: &str) -> Result<PathBuf> {
    store::validate_id(meeting_id)?;
    store::validate_id(transcript_id)?;
    Ok(Path::new("meetings")
        .join(meeting_id)
        .join("transcripts")
        .join(transcript_id))
}

fn immutable_json(root: &Path, relative: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = store::create_new(&store::checked_path(root, relative)?)?;
    serde_json::to_writer(&mut file, value).map_err(|e| CoreError::Storage(e.to_string()))?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read_bytes(root: &Path, relative: &Path) -> Result<Vec<u8>> {
    let path = store::checked_path(root, relative)?;
    let file = store::open_read(&path)?;
    if file.metadata()?.len() > 16 * 1024 * 1024 {
        return Err(CoreError::Storage("저장 정보가 너무 큽니다.".into()));
    }
    let mut bytes = Vec::new();
    file.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(CoreError::Storage("저장 정보가 너무 큽니다.".into()));
    }
    Ok(bytes)
}

fn immutable_bytes(root: &Path, relative: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = store::create_new(&store::checked_path(root, relative)?)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn source_position(seconds: f64, total_samples: u64) -> Option<u64> {
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    let position = (seconds * SOURCE_TIMEBASE as f64).round();
    if !position.is_finite() || position >= u64::MAX as f64 || position > total_samples as f64 {
        return None;
    }
    Some(position as u64)
}

fn same_playback_range(a: &TranscriptSegment, b: &TranscriptSegment, samples: u64) -> bool {
    if a.end < a.start || b.end < b.start {
        return false;
    }
    match (
        source_position(a.start, samples),
        source_position(a.end, samples),
        source_position(b.start, samples),
        source_position(b.end, samples),
    ) {
        (Some(a_start), Some(a_end), Some(b_start), Some(b_end)) => {
            a_start == b_start && a_end == b_end
        }
        _ => false,
    }
}

struct TranscriptUpgradeEvidence {
    before: Vec<u8>,
    raw_sha256: String,
    initial_sha256: String,
}

impl Engine {
    pub fn import_audio(
        &mut self,
        title: String,
        source: PathBuf,
        prepared: PathBuf,
    ) -> Result<Meeting> {
        self.import_audio_target(title, source, prepared, None)
    }

    pub fn import_audio_into_draft(
        &mut self,
        target: crate::DraftAttachmentTarget,
        source: PathBuf,
        prepared: PathBuf,
    ) -> Result<Meeting> {
        self.require_draft_attachment(&target)?;
        self.import_audio_target(String::new(), source, prepared, Some(target))
    }

    fn import_audio_target(
        &mut self,
        title: String,
        source: PathBuf,
        prepared: PathBuf,
        target: Option<crate::DraftAttachmentTarget>,
    ) -> Result<Meeting> {
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        if self.capture.is_some() {
            return Err(CoreError::RecordingBusy);
        }
        if !source.is_absolute() || !prepared.is_absolute() {
            return Err(CoreError::InvalidInput(
                "음성 파일의 절대 경로가 필요합니다.".into(),
            ));
        }
        let mut wav = hound::WavReader::new(BufReader::new(store::open_read(&prepared)?))
            .map_err(|e| CoreError::Audio(e.to_string()))?;
        let spec = wav.spec();
        if spec.channels != 1
            || spec.sample_rate != 16000
            || spec.bits_per_sample != 16
            || spec.sample_format != hound::SampleFormat::Int
            || wav.duration() == 0
        {
            return Err(CoreError::Audio(
                "분석용 음성 형식이 올바르지 않습니다.".into(),
            ));
        }
        let source_file = store::open_read(&source)?;
        let metadata = source_file.metadata()?;
        let attaching = target.is_some();
        let mut stored = match &target {
            Some(target) => self.require_draft_attachment(target)?.clone(),
            None => self.store.create(title, String::new())?,
        };
        let mut manifest = Manifest::new(
            &stored.meeting.id,
            "import",
            "파일 가져오기",
            16000,
            1,
            Encoding::I16,
        );
        let result = (|| -> Result<()> {
            let mut pcm = BufWriter::new(self.store.prepare_capture(&manifest)?);
            stored.capture_id = Some(manifest.session_id.clone());
            stored.meeting.status = MeetingStatus::Interrupted;
            if !attaching {
                self.store.save(&stored)?;
            }
            let directory = manifest.relative_dir();
            let extension = source
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or("audio")
                .to_ascii_lowercase();
            if !["wav", "m4a", "mp3", "aac", "aiff", "aif", "caf", "audio"]
                .contains(&extension.as_str())
            {
                return Err(CoreError::Audio(
                    "지원하지 않는 음성 파일 형식입니다.".into(),
                ));
            }
            let destination = store::checked_path(
                &self.store.root,
                &directory.join(format!("source.{extension}")),
            )?;
            let mut original = store::create_new(&destination)?;
            let mut reader = source_file;
            let mut digest = Sha256::new();
            let mut buffer = [0u8; 64 * 1024];
            let mut bytes = 0u64;
            loop {
                let count = reader.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                original.write_all(&buffer[..count])?;
                digest.update(&buffer[..count]);
                bytes += count as u64;
            }
            original.sync_all()?;
            if bytes != metadata.len()
                || reader.metadata()?.modified().ok() != metadata.modified().ok()
            {
                return Err(CoreError::Audio(
                    "가져오는 동안 음성 파일이 변경되었습니다. 다시 선택해 주세요.".into(),
                ));
            }
            immutable_json(
                &self.store.root,
                &directory.join("source.json"),
                &serde_json::json!({"filename":source.file_name().and_then(|x| x.to_str()), "bytes":bytes,"sha256":format!("{:x}",digest.finalize())}),
            )?;
            for sample in wav.samples::<i16>() {
                pcm.write_all(
                    &sample
                        .map_err(|e| CoreError::Audio(e.to_string()))?
                        .to_le_bytes(),
                )?;
            }
            pcm.flush()?;
            pcm.get_ref().sync_all()?;
            drop(pcm);
            manifest.phase = CapturePhase::Completed;
            stored.meeting.audio = store::finalize_audio(&self.store.root, &mut manifest)?;
            stored.meeting.status = MeetingStatus::Completed;
            stored.meeting.updated_at = store::now();
            if attaching {
                self.commit_meeting(stored.clone())?;
            } else {
                self.store.save(&stored)?;
            }
            Ok(())
        })();
        if attaching {
            return result.map(|_| stored.meeting);
        }
        if let Err(error) = &result {
            stored.meeting.status = MeetingStatus::Interrupted;
            stored.meeting.error = Some(error.to_string());
            let _ = self.store.save(&stored);
        }
        let meeting = stored.meeting.clone();
        self.meetings.insert(meeting.id.clone(), stored);
        result?;
        Ok(meeting)
    }

    pub fn transcript(&self, id: &str) -> Result<Option<Transcript>> {
        Ok(self
            .read_transcript_document(id)?
            .map(|(document, _)| document))
    }

    fn read_transcript_document(&self, id: &str) -> Result<Option<(Transcript, Vec<u8>)>> {
        store::validate_id(id)?;
        let meeting = &self
            .meetings
            .get(id)
            .ok_or(CoreError::MeetingNotFound)?
            .meeting;
        let Some(transcript_id) = &meeting.transcript_id else {
            return Ok(None);
        };
        let directory = transcript_dir(id, transcript_id)?;
        let bytes = read_bytes(&self.store.root, &directory.join("current.json"))?;
        let doc: Transcript = serde_json::from_slice(&bytes)
            .map_err(|_| CoreError::Storage("저장 정보의 형식이 올바르지 않습니다.".into()))?;
        if doc.version.id != *transcript_id
            || meeting.audio.as_ref().map(|a| a.session_id.as_str())
                != Some(doc.source_session_id.as_str())
        {
            return Err(CoreError::Storage(
                "전사와 원음의 연결이 올바르지 않습니다.".into(),
            ));
        }
        Ok(Some((doc, bytes)))
    }

    // History reads only complete documents below this meeting's transcript directory.
    // It does not recover partial runs or reconstruct missing source associations.
    pub fn transcript_history(&self, id: &str) -> Result<TranscriptHistory> {
        self.require_active_meeting(id)?;
        let relative = Path::new("meetings").join(id).join("transcripts");
        let path = store::checked_path(&self.store.root, &relative)?;
        if !path.exists() {
            return Ok(TranscriptHistory::default());
        }
        let current = self.meetings[id].meeting.transcript_id.as_deref();
        let mut history = TranscriptHistory::default();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let Some(transcript_id) = entry.file_name().to_str().map(str::to_owned) else {
                history.unavailable += 1;
                continue;
            };
            if store::validate_id(&transcript_id).is_err() {
                history.unavailable += 1;
                continue;
            }
            match self.history_document(id, &transcript_id) {
                Ok((document, edited, updated_at)) => {
                    history.entries.push(TranscriptHistoryEntry {
                        current: current == Some(transcript_id.as_str()),
                        version: document.version,
                        updated_at,
                        segment_count: document.segments.len(),
                        speaker_count: document.speakers.len(),
                        edited,
                    })
                }
                Err(_) => history.unavailable += 1,
            }
        }
        history.entries.sort_by(|a, b| {
            b.current
                .cmp(&a.current)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
                .then_with(|| a.version.id.cmp(&b.version.id))
        });
        Ok(history)
    }

    fn history_document(&self, id: &str, transcript_id: &str) -> Result<(Transcript, bool, u64)> {
        let meeting = &self
            .meetings
            .get(id)
            .ok_or(CoreError::MeetingNotFound)?
            .meeting;
        let audio = meeting
            .audio
            .as_ref()
            .ok_or_else(|| CoreError::Storage("원음 정보가 없습니다.".into()))?;
        let directory = transcript_dir(id, transcript_id)?;
        let current_path = store::checked_path(&self.store.root, &directory.join("current.json"))?;
        let current_bytes = read_bytes(&self.store.root, &directory.join("current.json"))?;
        let document: Transcript = serde_json::from_slice(&current_bytes)
            .map_err(|_| CoreError::Storage("음성 기록을 읽을 수 없습니다.".into()))?;
        let initial_bytes = read_bytes(&self.store.root, &directory.join("initial.json"))?;
        let initial: Transcript = serde_json::from_slice(&initial_bytes)
            .map_err(|_| CoreError::Storage("원본 음성 기록을 읽을 수 없습니다.".into()))?;
        let raw = read_bytes(&self.store.root, &directory.join("raw.json"))?;
        let duration = audio.frames as f64 / audio.sample_rate as f64;
        let samples = source_position(duration, u64::MAX)
            .ok_or_else(|| CoreError::Storage("원음 시각이 올바르지 않습니다.".into()))?;
        let source = document.source.as_ref().ok_or_else(|| {
            CoreError::Storage("음성 기록의 원본 연결을 확인할 수 없습니다.".into())
        })?;
        let valid_source = matches!(
            source.projector_version.as_str(),
            PROJECTOR_VERSION
                | RECORDING_PROJECTOR_VERSION
                | crate::nemotron::PROJECTOR_VERSION
                | crate::nemotron::WORD_PROJECTOR_VERSION
        ) && source.raw_sha256 == sha256(&raw)
            && match source.association.as_str() {
                "direct" => initial.source.as_ref() == Some(source),
                LEGACY_ASSOCIATION => initial.source.is_none(),
                _ => false,
            };
        if !valid_source
            || document.version.id != transcript_id
            || initial.version.id != transcript_id
            || initial.version.revision != 0
            || document.source_session_id != audio.session_id
            || initial.source_session_id != document.source_session_id
            || initial.model != document.model
            || initial.model_revision != document.model_revision
            || document.segments.is_empty()
            || initial.segments.len() != document.segments.len()
        {
            return Err(CoreError::Storage(
                "음성 기록의 원본 연결을 확인할 수 없습니다.".into(),
            ));
        }
        let projection = if matches!(
            source.projector_version.as_str(),
            crate::nemotron::PROJECTOR_VERSION | crate::nemotron::WORD_PROJECTOR_VERSION
        ) {
            crate::nemotron::decode(&raw, duration, &audio.session_id, transcript_id)?
        } else {
            MossOutput::decode_with_policy(
                &raw,
                duration,
                &audio.session_id,
                transcript_id,
                MossValidationPolicy::StoredArtifact,
            )?
        };
        if projection.model != initial.model
            || (matches!(
                source.projector_version.as_str(),
                crate::nemotron::PROJECTOR_VERSION | crate::nemotron::WORD_PROJECTOR_VERSION
            ) && projection.model_revision != initial.model_revision)
            || projection.segments.len() != initial.segments.len()
            || (source.association == "direct" && projection.source.as_ref() != Some(source))
            || (source.association == LEGACY_ASSOCIATION
                && (source.projector_version != PROJECTOR_VERSION
                    || source.speaker_scope.is_some()
                    || source.chunking_policy.is_some()))
        {
            return Err(CoreError::Storage(
                "모델 원본과 발언의 연결을 확인할 수 없습니다.".into(),
            ));
        }
        if source.projector_version == crate::nemotron::WORD_PROJECTOR_VERSION {
            if initial.speaker_analysis != projection.speaker_analysis
                || initial.speakers != projection.speakers
                || initial
                    .segments
                    .iter()
                    .map(|s| &s.speaker_id)
                    .ne(projection.segments.iter().map(|s| &s.speaker_id))
            {
                return Err(CoreError::Storage(
                    "화자 분석의 원본 연결을 확인할 수 없습니다.".into(),
                ));
            }
            let (run, bytes, expected_hash) = crate::nemotron::posterior_source(&raw)?;
            store::validate_id(&run)?;
            let path = store::checked_path(
                &self.store.root,
                &directory
                    .join("speaker-analysis")
                    .join(run)
                    .join("posterior.f32le"),
            )?;
            let mut file = store::open_read(&path)?;
            if bytes > 128 * 1024 * 1024 || file.metadata()?.len() != bytes {
                return Err(CoreError::Storage(
                    "화자 확률 원본의 크기가 다릅니다.".into(),
                ));
            }
            let mut digest = Sha256::new();
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                digest.update(&buffer[..count]);
            }
            if format!("{:x}", digest.finalize()) != expected_hash {
                return Err(CoreError::Storage(
                    "화자 확률 원본을 확인할 수 없습니다.".into(),
                ));
            }
        }
        let mut ids = BTreeSet::new();
        for (index, ((original, current), projected)) in initial
            .segments
            .iter()
            .zip(&document.segments)
            .zip(&projection.segments)
            .enumerate()
        {
            store::validate_id(&original.id)?;
            if !ids.insert(&original.id)
                || original.id != current.id
                || original.original_text != current.original_text
                || original.text != original.original_text
                || projected.original_text != original.original_text
                || current.source_index != u32::try_from(index).ok()
                || (source.association == "direct" && original.source_index != current.source_index)
                || !same_playback_range(original, current, samples)
                || !same_playback_range(projected, original, samples)
                || current.text.contains('\0')
                || current.text.len() > 2 * 1024 * 1024
            {
                return Err(CoreError::Storage(
                    "수정본과 원본 발언의 연결을 확인할 수 없습니다.".into(),
                ));
            }
        }
        validate_speakers(
            &document.speakers,
            &document
                .segments
                .iter()
                .map(|segment| segment.speaker_id.clone())
                .collect::<Vec<_>>(),
        )?;
        let edited = document
            .segments
            .iter()
            .any(|segment| segment.text != segment.original_text)
            || document.speakers != projection.speakers
            || document
                .segments
                .iter()
                .map(|segment| &segment.speaker_id)
                .ne(projection
                    .segments
                    .iter()
                    .map(|segment| &segment.speaker_id));
        let updated_at = store::open_read(&current_path)?
            .metadata()?
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| CoreError::Storage("저장 시각을 확인할 수 없습니다.".into()))?
            .as_millis()
            .min(u64::MAX as u128) as u64;
        Ok((document, edited, updated_at))
    }

    pub fn restore_transcript(
        &mut self,
        id: &str,
        expected: TranscriptVersion,
        target: TranscriptVersion,
    ) -> Result<Transcript> {
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        self.require_active_meeting(id)?;
        if self.capture.is_some() {
            return Err(CoreError::RecordingBusy);
        }
        store::validate_id(&expected.id)?;
        store::validate_id(&target.id)?;
        let current_id = self.meetings[id]
            .meeting
            .transcript_id
            .as_deref()
            .ok_or_else(|| CoreError::InvalidState("음성 기록이 없습니다.".into()))?;
        let (current, _, _) = self.history_document(id, current_id)?;
        if current.version != expected {
            return Err(CoreError::InvalidState(
                "음성 기록이 변경되었습니다. 이전 기록을 다시 열어 주세요.".into(),
            ));
        }
        if target.id == expected.id {
            return Err(CoreError::InvalidState(
                "현재 사용 중인 음성 기록입니다.".into(),
            ));
        }
        let (document, _, _) = self.history_document(id, &target.id)?;
        if document.version != target {
            return Err(CoreError::InvalidState(
                "선택한 이전 기록이 변경되었습니다. 목록을 다시 열어 주세요.".into(),
            ));
        }
        // A -> B -> A is a new revision, so an old A request cannot pass expected-version checks.
        let restored = self.save_transcript_edit(id, document)?;
        let mut stored = self.meetings[id].clone();
        stored.meeting.transcript_id = Some(restored.version.id.clone());
        stored.meeting.updated_at = store::now();
        self.store.save(&stored)?;
        self.meetings.insert(id.into(), stored);
        Ok(restored)
    }

    pub fn upgrade_transcript_sources(&mut self) -> Result<TranscriptUpgradeReport> {
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        if self.capture.is_some() {
            return Err(CoreError::RecordingBusy);
        }
        let ids: Vec<_> = self
            .meetings
            .values()
            .filter(|stored| stored.meeting.transcript_id.is_some())
            .map(|stored| stored.meeting.id.clone())
            .collect();
        let mut report = TranscriptUpgradeReport::default();
        for id in ids {
            match self.upgrade_transcript_source(&id) {
                Ok(true) => report.upgraded += 1,
                Ok(false) => report.unchanged += 1,
                Err(_) => report.unavailable += 1,
            }
        }
        Ok(report)
    }

    fn upgrade_transcript_source(&self, id: &str) -> Result<bool> {
        let (mut doc, before) = self
            .read_transcript_document(id)?
            .ok_or_else(|| CoreError::InvalidState("음성 기록이 없습니다.".into()))?;
        if doc.source.is_some() {
            return Ok(false);
        }
        let audio = self.meetings[id]
            .meeting
            .audio
            .as_ref()
            .ok_or_else(|| CoreError::Storage("원음 정보가 없습니다.".into()))?;
        let duration = audio.frames as f64 / audio.sample_rate as f64;
        let samples = source_position(duration, u64::MAX)
            .ok_or_else(|| CoreError::Storage("원음 시각이 올바르지 않습니다.".into()))?;
        let directory = transcript_dir(id, &doc.version.id)?;
        let current_value: serde_json::Value = serde_json::from_slice(&before)
            .map_err(|_| CoreError::Storage("저장 정보의 형식이 올바르지 않습니다.".into()))?;
        let explicit_speakers = current_value.get("speakers").is_some();
        let initial_bytes = read_bytes(&self.store.root, &directory.join("initial.json"))?;
        let initial: Transcript = serde_json::from_slice(&initial_bytes)
            .map_err(|_| CoreError::Storage("원본 음성 기록을 읽을 수 없습니다.".into()))?;
        if initial.version.id != doc.version.id
            || initial.version.revision != 0
            || initial.source_session_id != doc.source_session_id
            || initial.model != doc.model
            || initial.model_revision != doc.model_revision
            || initial.segments.len() != doc.segments.len()
        {
            return Err(CoreError::Storage(
                "음성 기록의 원본 연결을 확인할 수 없습니다.".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        for (original, current) in initial.segments.iter().zip(&doc.segments) {
            store::validate_id(&original.id)?;
            if !ids.insert(&original.id)
                || original.id != current.id
                || original.original_text != current.original_text
                || original.text != original.original_text
                || !same_playback_range(original, current, samples)
            {
                return Err(CoreError::Storage(
                    "수정본과 원본 발언의 연결을 확인할 수 없습니다.".into(),
                ));
            }
        }
        let raw = read_bytes(&self.store.root, &directory.join("raw.json"))?;
        let projection =
            MossOutput::decode(&raw, duration, &doc.source_session_id, &doc.version.id)?;
        if projection
            .source
            .as_ref()
            .is_some_and(|source| source.projector_version == RECORDING_PROJECTOR_VERSION)
        {
            return Err(CoreError::Storage(
                "기존 음성 기록을 녹음 전체 화자 범위로 승격할 수 없습니다.".into(),
            ));
        }
        if projection.model != initial.model
            || projection.segments.len() != initial.segments.len()
            || !projection
                .segments
                .iter()
                .zip(&initial.segments)
                .all(|(projected, original)| {
                    projected.original_text == original.original_text
                        && same_playback_range(projected, original, samples)
                })
        {
            return Err(CoreError::Storage(
                "모델 원본과 발언의 연결을 확인할 수 없습니다.".into(),
            ));
        }
        if !explicit_speakers {
            doc.speakers = projection.speakers;
            for (segment, projected) in doc.segments.iter_mut().zip(&projection.segments) {
                segment.speaker_id = projected.speaker_id.clone();
            }
        }
        validate_speakers(
            &doc.speakers,
            &doc.segments
                .iter()
                .map(|segment| segment.speaker_id.clone())
                .collect::<Vec<_>>(),
        )?;
        for (index, segment) in doc.segments.iter_mut().enumerate() {
            segment.source_index = Some(
                u32::try_from(index)
                    .map_err(|_| CoreError::Storage("발언 수가 너무 많습니다.".into()))?,
            );
        }
        let raw_sha256 = sha256(&raw);
        doc.source = Some(TranscriptSource {
            raw_sha256: raw_sha256.clone(),
            projector_version: PROJECTOR_VERSION.into(),
            association: LEGACY_ASSOCIATION.into(),
            speaker_scope: None,
            chunking_policy: None,
        });
        self.save_transcript_revision(
            id,
            doc,
            Some(TranscriptUpgradeEvidence {
                before,
                raw_sha256,
                initial_sha256: sha256(&initial_bytes),
            }),
        )?;
        Ok(true)
    }

    pub fn commit_transcript(
        &mut self,
        id: &str,
        expected: Option<TranscriptVersion>,
        raw: Vec<u8>,
    ) -> Result<Transcript> {
        self.commit_transcript_with_policy(id, expected, raw, TranscriptCommitPolicy::Moss)
    }

    pub fn commit_whole_transcript(
        &mut self,
        id: &str,
        expected: Option<TranscriptVersion>,
        raw: Vec<u8>,
    ) -> Result<Transcript> {
        self.commit_transcript_with_policy(id, expected, raw, TranscriptCommitPolicy::WholeMoss)
    }

    pub fn commit_nemotron_transcript(
        &mut self,
        id: &str,
        expected: Option<TranscriptVersion>,
        raw: Vec<u8>,
    ) -> Result<Transcript> {
        self.commit_transcript_with_policy(id, expected, raw, TranscriptCommitPolicy::Nemotron)
    }

    fn commit_transcript_with_policy(
        &mut self,
        id: &str,
        expected: Option<TranscriptVersion>,
        raw: Vec<u8>,
        policy: TranscriptCommitPolicy,
    ) -> Result<Transcript> {
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        store::validate_id(id)?;
        self.require_active_meeting(id)?;
        let mut stored = self
            .meetings
            .get(id)
            .ok_or(CoreError::MeetingNotFound)?
            .clone();
        let audio = stored.meeting.audio.as_ref().ok_or_else(|| {
            CoreError::InvalidState("먼저 녹음을 종료하거나 음성을 가져와 주세요.".into())
        })?;
        let transcript_id = Uuid::new_v4().to_string();
        let directory = transcript_dir(id, &transcript_id)?;
        store::ensure_directory(&store::checked_path(&self.store.root, &directory)?)?;
        let mut file = store::create_new(&store::checked_path(
            &self.store.root,
            &directory.join("raw.json"),
        )?)?;
        file.write_all(&raw)?;
        file.sync_all()?;
        let duration = audio.frames as f64 / audio.sample_rate as f64;
        let document = if matches!(policy, TranscriptCommitPolicy::Nemotron) {
            crate::nemotron::decode_asr(&raw, duration, &audio.session_id, &transcript_id)?
        } else {
            MossOutput::decode(&raw, duration, &audio.session_id, &transcript_id)?
        };
        immutable_json(&self.store.root, &directory.join("initial.json"), &document)?;
        if matches!(policy, TranscriptCommitPolicy::WholeMoss)
            && !document.source.as_ref().is_some_and(|source| {
                source.projector_version == RECORDING_PROJECTOR_VERSION
                    && source.speaker_scope.as_deref() == Some("recording")
                    && source.chunking_policy.as_deref() == Some(WHOLE_RECORDING_POLICY)
            })
        {
            return Err(CoreError::InvalidInput(
                "녹음 전체를 한 번에 처리하는 실행기가 필요합니다. 새 원문 결과는 보관했으며 현재 음성 기록은 변경하지 않았습니다.".into(),
            ));
        }
        if self.transcript(id)?.map(|d| d.version) != expected {
            return Err(CoreError::InvalidState("전사가 변경되어 새 결과를 적용하지 않았습니다. 기존 수정과 새 원문 결과는 보관했습니다.".into()));
        }
        store::write_json(&self.store.root, &directory.join("current.json"), &document)?;
        stored.meeting.transcript_id = Some(transcript_id);
        stored.meeting.updated_at = store::now();
        self.store.save(&stored)?;
        self.meetings.insert(id.into(), stored);
        Ok(document)
    }

    pub fn edit_transcript(
        &mut self,
        id: &str,
        expected: TranscriptVersion,
        texts: Vec<String>,
    ) -> Result<Transcript> {
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        self.require_active_meeting(id)?;
        let mut doc = self
            .transcript(id)?
            .ok_or_else(|| CoreError::InvalidState("저장할 전사가 없습니다.".into()))?;
        if doc.version != expected {
            return Err(CoreError::InvalidState(
                "전사가 변경되었습니다. 현재 입력을 보존하고 새로고침해 주세요.".into(),
            ));
        }
        if texts.len() != doc.segments.len()
            || texts.iter().map(String::len).sum::<usize>() > 2 * 1024 * 1024
            || texts.iter().any(|s| s.contains('\0'))
        {
            return Err(CoreError::InvalidInput(
                "전사 수정 내용이 올바르지 않습니다.".into(),
            ));
        }
        for (segment, text) in doc.segments.iter_mut().zip(texts) {
            segment.text = text;
        }
        self.save_transcript_edit(id, doc)
    }

    pub fn commit_terminology(
        &mut self,
        id: &str,
        draft: crate::terminology::TerminologyDraft,
    ) -> Result<Transcript> {
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        self.require_active_meeting(id)?;
        let current = self
            .transcript(id)?
            .ok_or_else(|| CoreError::InvalidState("보정할 음성 기록이 없습니다.".into()))?;
        if current.version != draft.source.version
            || crate::terminology::transcript_digest(&current)?
                != crate::terminology::transcript_digest(&draft.source)?
        {
            return Err(CoreError::InvalidState(
                "음성 기록이 변경되어 용어 보정을 적용하지 않았습니다.".into(),
            ));
        }
        let (after, evidence) = crate::terminology::prepare(&draft)?;
        if current
            .segments
            .iter()
            .map(|s| &s.text)
            .eq(after.segments.iter().map(|s| &s.text))
        {
            return Ok(current);
        }
        let (verified, _, _) = self.history_document(id, &current.version.id)?;
        if crate::terminology::transcript_digest(&verified)?
            != crate::terminology::transcript_digest(&current)?
        {
            return Err(CoreError::InvalidState(
                "음성 기록이 변경되었습니다.".into(),
            ));
        }
        self.save_transcript_revision_with_evidence(id, after, None, Some(evidence), None)
    }

    pub fn apply_diarization(
        &mut self,
        id: &str,
        draft: crate::SpeakerAnalysisDraft,
    ) -> Result<Transcript> {
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        self.require_active_meeting(id)?;
        if self.capture.is_some() {
            return Err(CoreError::RecordingBusy);
        }
        let current = self
            .transcript(id)?
            .ok_or_else(|| CoreError::InvalidState("분석할 음성 기록이 없습니다.".into()))?;
        if current.version != draft.source.version
            || crate::summary::transcript_digest(&current)?
                != crate::summary::transcript_digest(&draft.source)?
        {
            return Err(CoreError::InvalidState(
                "음성 기록이 변경되어 화자 분석을 적용하지 않았습니다.".into(),
            ));
        }
        let (verified, _, _) = self.history_document(id, &current.version.id)?;
        if crate::summary::transcript_digest(&verified)?
            != crate::summary::transcript_digest(&current)?
        {
            return Err(CoreError::InvalidState(
                "음성 기록이 변경되었습니다.".into(),
            ));
        }
        let audio = self.meetings[id]
            .meeting
            .audio
            .as_ref()
            .ok_or_else(|| CoreError::InvalidState("원음 정보가 없습니다.".into()))?;
        if draft.posterior.is_some()
            && draft.source.model == "nvidia/nemotron-3.5-asr-streaming-0.6b"
        {
            let before = draft.source.version.clone();
            let before_digest = crate::summary::transcript_digest(&draft.source)?;
            let raw = read_bytes(
                &self.store.root,
                &transcript_dir(id, &before.id)?.join("raw.json"),
            )?;
            let next_id = Uuid::new_v4().to_string();
            let generation = crate::nemotron::create_speaker_generation(
                draft,
                &raw,
                audio.frames,
                audio.sample_rate,
                &next_id,
            )?;
            let directory = transcript_dir(id, &next_id)?;
            store::ensure_directory(&store::checked_path(&self.store.root, &directory)?)?;
            immutable_bytes(
                &self.store.root,
                &directory.join("raw.json"),
                &generation.raw,
            )?;
            immutable_json(
                &self.store.root,
                &directory.join("initial.json"),
                &generation.initial,
            )?;
            validate_speakers(
                &generation.current.speakers,
                &generation
                    .current
                    .segments
                    .iter()
                    .map(|s| s.speaker_id.clone())
                    .collect::<Vec<_>>(),
            )?;
            let after_path = if generation.current.version.revision > 0 {
                let name = format!("edit-{}.json", Uuid::new_v4());
                immutable_json(
                    &self.store.root,
                    &directory.join(&name),
                    &generation.current,
                )?;
                name
            } else {
                "initial.json".into()
            };
            let evidence = generation.evidence;
            let receipts = directory.join("speaker-analysis").join(&evidence.run_id);
            store::ensure_directory(&store::checked_path(&self.store.root, &receipts)?)?;
            immutable_bytes(&self.store.root, &receipts.join("raw.json"), &evidence.raw)?;
            immutable_bytes(
                &self.store.root,
                &receipts.join("posterior.f32le"),
                evidence
                    .posterior
                    .as_deref()
                    .ok_or_else(|| CoreError::InvalidState("화자 확률 원본이 없습니다.".into()))?,
            )?;
            immutable_json(
                &self.store.root,
                &receipts.join("run.json"),
                &serde_json::json!({
                    "status":"prepared","beforeVersion":before,"afterVersion":generation.current.version,
                    "afterPath":after_path,"provenance":evidence.document
                }),
            )?;
            // All immutable artifacts are complete before replacing the meeting pointer.
            // A concurrent edit keeps its active generation and leaves this candidate archived.
            let latest = self
                .transcript(id)?
                .ok_or_else(|| CoreError::InvalidState("음성 기록이 변경되었습니다.".into()))?;
            if latest.version != before
                || crate::summary::transcript_digest(&latest)? != before_digest
            {
                return Err(CoreError::InvalidState(
                    "음성 기록이 변경되어 새 화자 분석을 적용하지 않았습니다.".into(),
                ));
            }
            store::write_json(
                &self.store.root,
                &directory.join("current.json"),
                &generation.current,
            )?;
            let mut stored = self.meetings[id].clone();
            stored.meeting.transcript_id = Some(next_id);
            stored.meeting.updated_at = store::now();
            self.store.save(&stored)?;
            self.meetings.insert(id.into(), stored);
            return Ok(generation.current);
        }
        let preserve_source_assignments =
            if draft.posterior.is_some() && draft.source.speaker_analysis.is_none() {
                // history_document above verified the initial/current segment identities.
                // Before the first analysis there is no assignments_edited receipt yet.
                let initial: Transcript = serde_json::from_slice(&read_bytes(
                    &self.store.root,
                    &transcript_dir(id, &current.version.id)?.join("initial.json"),
                )?)
                .map_err(|_| CoreError::Storage("원본 음성 기록을 읽을 수 없습니다.".into()))?;
                current
                    .segments
                    .iter()
                    .map(|s| &s.speaker_id)
                    .ne(initial.segments.iter().map(|s| &s.speaker_id))
            } else {
                false
            };
        let (after, evidence) = if preserve_source_assignments {
            crate::diarization::project_with_source_assignments(
                draft,
                audio.frames,
                audio.sample_rate,
                true,
            )?
        } else {
            crate::diarization::project(draft, audio.frames, audio.sample_rate)?
        };
        self.save_transcript_revision_with_evidence(id, after, None, None, Some(evidence))
    }

    pub fn edit_speakers(
        &mut self,
        id: &str,
        expected: TranscriptVersion,
        mut speakers: Vec<TranscriptSpeaker>,
        assignments: Vec<Option<String>>,
    ) -> Result<Transcript> {
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        self.require_active_meeting(id)?;
        let mut doc = self
            .transcript(id)?
            .ok_or_else(|| CoreError::InvalidState("수정할 음성 기록이 없습니다.".into()))?;
        if doc.version != expected {
            return Err(CoreError::InvalidState(
                "음성 기록이 변경되었습니다. 현재 수정을 보존하고 다시 열어 주세요.".into(),
            ));
        }
        if assignments.len() != doc.segments.len() {
            return Err(CoreError::InvalidInput(
                "발언 수와 화자 배정 수가 다릅니다.".into(),
            ));
        }
        validate_speakers(&speakers, &assignments)?;
        for speaker in &mut speakers {
            speaker.name_origin = doc
                .speakers
                .iter()
                .find(|previous| previous.id == speaker.id && previous.name == speaker.name)
                .map(|previous| previous.name_origin)
                .unwrap_or(Some(SpeakerNameOrigin::User));
        }
        if doc.speakers == speakers
            && doc
                .segments
                .iter()
                .map(|segment| &segment.speaker_id)
                .eq(assignments.iter())
        {
            return Ok(doc);
        }
        doc.speakers = speakers;
        if let Some(analysis) = &mut doc.speaker_analysis {
            analysis.assignments_edited = true;
        }
        for (segment, assignment) in doc.segments.iter_mut().zip(assignments) {
            segment.speaker_id = assignment;
        }
        self.save_transcript_edit(id, doc)
    }

    fn save_transcript_edit(&self, id: &str, doc: Transcript) -> Result<Transcript> {
        self.save_transcript_revision(id, doc, None)
    }

    fn save_transcript_revision(
        &self,
        id: &str,
        doc: Transcript,
        upgrade: Option<TranscriptUpgradeEvidence>,
    ) -> Result<Transcript> {
        self.save_transcript_revision_with_evidence(id, doc, upgrade, None, None)
    }

    fn save_transcript_revision_with_evidence(
        &self,
        id: &str,
        mut doc: Transcript,
        upgrade: Option<TranscriptUpgradeEvidence>,
        terminology: Option<serde_json::Value>,
        speaker_analysis: Option<crate::diarization::SpeakerAnalysisEvidence>,
    ) -> Result<Transcript> {
        let before_version = doc.version.clone();
        doc.version.revision = doc
            .version
            .revision
            .checked_add(1)
            .ok_or(CoreError::Unavailable)?;
        let directory = transcript_dir(id, &doc.version.id)?;
        let transaction = Uuid::new_v4();
        let edit_name = format!("edit-{transaction}.json");
        let mut after =
            serde_json::to_vec(&doc).map_err(|error| CoreError::Storage(error.to_string()))?;
        after.push(b'\n');
        if let Some(evidence) = &upgrade {
            let current = read_bytes(&self.store.root, &directory.join("current.json"))?;
            if current != evidence.before {
                return Err(CoreError::InvalidState(
                    "이관 중 음성 기록이 변경되었습니다.".into(),
                ));
            }
            immutable_bytes(
                &self.store.root,
                &directory.join(format!("upgrade-before-{transaction}.json")),
                &evidence.before,
            )?;
        }
        immutable_bytes(&self.store.root, &directory.join(&edit_name), &after)?;
        if let Some(evidence) = upgrade {
            // 적용 뒤 영수증 쓰기 실패가 성공한 교체를 숨기지 않도록 증거를 먼저 보존한다.
            immutable_json(
                &self.store.root,
                &directory.join(format!("upgrade-receipt-{transaction}.json")),
                &serde_json::json!({
                    "status":"prepared",
                    "ruleVersion":LEGACY_ASSOCIATION,
                    "timebase":SOURCE_TIMEBASE,
                    "rawSha256":evidence.raw_sha256,
                    "initialSha256":evidence.initial_sha256,
                    "beforeCurrentSha256":sha256(&evidence.before),
                    "afterCurrentSha256":sha256(&after),
                    "beforePath":format!("upgrade-before-{transaction}.json"),
                    "afterPath":edit_name,
                    "beforeVersion":before_version,
                    "afterVersion":doc.version,
                }),
            )?;
        }
        if let Some(evidence) = terminology {
            let receipts = directory.join("terminology");
            store::ensure_directory(&store::checked_path(&self.store.root, &receipts)?)?;
            immutable_json(
                &self.store.root,
                &receipts.join(format!("receipt-{transaction}.json")),
                &serde_json::json!({
                    "status":"prepared",
                    "beforeVersion":before_version,
                    "afterVersion":doc.version,
                    "afterCurrentSha256":sha256(&after),
                    "afterPath":edit_name,
                    "provenance":evidence,
                }),
            )?;
        }
        if let Some(evidence) = speaker_analysis {
            let receipts = directory.join("speaker-analysis").join(&evidence.run_id);
            store::ensure_directory(&store::checked_path(&self.store.root, &receipts)?)?;
            immutable_bytes(&self.store.root, &receipts.join("raw.json"), &evidence.raw)?;
            if let Some(posterior) = &evidence.posterior {
                immutable_bytes(
                    &self.store.root,
                    &receipts.join("posterior.f32le"),
                    posterior,
                )?;
            }
            immutable_json(
                &self.store.root,
                &receipts.join("run.json"),
                &serde_json::json!({
                    "status":"prepared","beforeVersion":before_version,"afterVersion":doc.version,
                    "afterCurrentSha256":sha256(&after),"afterPath":edit_name,"provenance":evidence.document
                }),
            )?;
        }
        store::write_json(&self.store.root, &directory.join("current.json"), &doc)?;
        Ok(doc)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn setup() -> (tempfile::TempDir, Engine, String, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let wav_path = root.join("input.wav");
        let mut writer = hound::WavWriter::create(
            &wav_path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..16000 {
            writer.write_sample(100i16).unwrap();
        }
        writer.finalize().unwrap();
        let mut engine = Engine::open(root.join("store")).unwrap();
        let meeting = engine
            .import_audio("회의".into(), wav_path.clone(), wav_path.clone())
            .unwrap();
        (temp, engine, meeting.id, wav_path)
    }
    #[test]
    fn draft_import_keeps_identity_notes_folder_and_calendar() {
        let (_temp, mut engine, _, wav) = setup();
        let draft = engine
            .create_meeting("Manual title".into(), "Manual notes".into())
            .unwrap();
        let folder = engine.create_folder("Folder".into()).unwrap();
        engine
            .move_meetings(&[draft.id.clone()], Some(folder.id.clone()))
            .unwrap();
        let reference = crate::CalendarReference {
            occurrence_key: format!("google:{}", "a".repeat(64)),
            title: "Event".into(),
            start_at: 1,
            end_at: 2,
            calendar_name: "Calendar".into(),
            join_urls: vec![],
        };
        let target = engine.draft_attachment_target(&draft.id).unwrap();
        engine
            .attach_calendar_to_draft(target, reference.clone())
            .unwrap();
        let target = engine.draft_attachment_target(&draft.id).unwrap();
        let imported = engine
            .import_audio_into_draft(target.clone(), wav.clone(), wav.clone())
            .unwrap();
        assert_eq!(imported.id, draft.id);
        assert_eq!(imported.title, draft.title);
        assert_eq!(imported.notes, draft.notes);
        assert_eq!(imported.folder_id, Some(folder.id));
        assert_eq!(imported.calendar_reference, Some(reference));
        assert_eq!(imported.status, MeetingStatus::Completed);
        assert!(imported.audio.is_some());
        assert_eq!(engine.meetings.len(), 2);
        assert!(matches!(
            engine.import_audio_into_draft(target, wav.clone(), wav.clone()),
            Err(CoreError::EditConflict)
        ));
        assert!(engine.draft_attachment_target(&draft.id).is_err());
        let root = engine.store.root.clone();
        drop(engine);
        let reopened = Engine::open(root).unwrap();
        assert_eq!(reopened.meetings[&draft.id].meeting, imported);
    }

    #[test]
    fn draft_import_keeps_published_audio_after_directory_sync_failure() {
        let (_temp, mut engine, _, wav) = setup();
        let draft = engine
            .create_meeting("Draft".into(), "Notes".into())
            .unwrap();
        let folder = engine.create_folder("Folder".into()).unwrap();
        engine
            .move_meetings(&[draft.id.clone()], Some(folder.id.clone()))
            .unwrap();
        let target = engine.draft_attachment_target(&draft.id).unwrap();
        let path = engine
            .store
            .root
            .join("meetings")
            .join(&draft.id)
            .join("meeting.json");
        store::fail_next_json_sync(path);
        assert!(matches!(
            engine.import_audio_into_draft(target.clone(), wav.clone(), wav.clone()),
            Err(CoreError::Storage(_))
        ));
        let published = engine.meetings[&draft.id].meeting.clone();
        assert_eq!(published.status, MeetingStatus::Completed);
        assert!(published.audio.is_some());
        assert_eq!(published.title, draft.title);
        assert_eq!(published.notes, draft.notes);
        assert_eq!(published.folder_id, Some(folder.id));
        assert!(engine
            .store
            .meeting_matches(&engine.meetings[&draft.id])
            .unwrap());
        let audio = engine.audio_path(&draft.id).unwrap();
        let audio_before = std::fs::read(&audio).unwrap();
        assert!(matches!(
            engine.import_audio_into_draft(target, wav.clone(), wav),
            Err(CoreError::EditConflict)
        ));
        assert!(engine.draft_attachment_target(&draft.id).is_err());
        assert_eq!(engine.meetings[&draft.id].meeting, published);
        assert_eq!(std::fs::read(&audio).unwrap(), audio_before);
        let root = engine.store.root.clone();
        drop(engine);
        let reopened = Engine::open(root).unwrap();
        assert_eq!(reopened.meetings[&draft.id].meeting, published);
        assert_eq!(std::fs::read(&audio).unwrap(), audio_before);
    }

    #[test]
    fn failed_draft_import_preserves_document_and_remains_attachable() {
        let (_temp, mut engine, _, wav) = setup();
        let draft = engine
            .create_meeting("Draft".into(), "Notes".into())
            .unwrap();
        let target = engine.draft_attachment_target(&draft.id).unwrap();
        let path = engine
            .store
            .root
            .join("meetings")
            .join(&draft.id)
            .join("meeting.json");
        let before = std::fs::read(&path).unwrap();
        let unsupported = wav.with_extension("unsupported");
        std::fs::write(&unsupported, b"Synthetic source").unwrap();
        assert!(engine
            .import_audio_into_draft(target.clone(), unsupported, wav.clone())
            .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(engine.meetings[&draft.id].meeting, draft);
        assert_eq!(engine.draft_attachment_target(&draft.id).unwrap(), target);
        let root = engine.store.root.clone();
        drop(engine);
        let reopened = Engine::open(root).unwrap();
        assert_eq!(reopened.meetings[&draft.id].meeting, draft);
    }

    #[test]
    fn draft_import_rejects_intervening_edits_trash_and_existing_audio() {
        let (_temp, mut engine, audio_id, wav) = setup();
        let draft = engine
            .create_meeting("Draft".into(), String::new())
            .unwrap();
        let target = engine.draft_attachment_target(&draft.id).unwrap();
        let edited = engine
            .update_meeting(&draft.id, "Edited".into(), "Notes".into())
            .unwrap();
        assert!(matches!(
            engine.import_audio_into_draft(target, wav.clone(), wav.clone()),
            Err(CoreError::EditConflict)
        ));
        assert_eq!(engine.meetings[&draft.id].meeting, edited);
        let target = engine.draft_attachment_target(&draft.id).unwrap();
        engine.trash_meetings(&[draft.id.clone()]).unwrap();
        assert!(matches!(
            engine.import_audio_into_draft(target, wav.clone(), wav.clone()),
            Err(CoreError::MeetingNotFound)
        ));
        let target = crate::DraftAttachmentTarget {
            meeting_id: audio_id.clone(),
            expected_version: crate::agent_control::meeting_edit_version(
                &engine.meetings[&audio_id].meeting,
            )
            .unwrap(),
        };
        let before = engine.meetings[&audio_id].meeting.clone();
        assert!(matches!(
            engine.import_audio_into_draft(target, wav.clone(), wav),
            Err(CoreError::InvalidState(_))
        ));
        assert_eq!(engine.meetings[&audio_id].meeting, before);
        assert_eq!(engine.meetings.len(), 2);
    }

    pub(crate) fn output() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"model":"OpenMOSS-Team/MOSS-Transcribe-Diarize","segments":[{"start":0.0,"end":1.0,"text":"[S1] 원문","speaker_id":"S1"}],"hitTokenLimit":false,"hasUnparsedText":false,"processedAudioFrames":16000,"inputAudioFrames":16000,"audioDurationSeconds":1.0})).unwrap()
    }
    fn whole_output() -> Vec<u8> {
        let mut raw: serde_json::Value = serde_json::from_slice(&output()).unwrap();
        raw["speakerScope"] = "recording".into();
        raw["chunkingPolicy"] = WHOLE_RECORDING_POLICY.into();
        raw["runtime"] = WHOLE_RECORDING_RUNTIME.into();
        raw["generationCount"] = 1.into();
        raw["maxTokens"] = 65536.into();
        raw["effectiveMaxTokens"] = 65536.into();
        raw["promptTokens"] = 64.into();
        raw["contextSize"] = 131072.into();
        raw["terminationReason"] = "eos".into();
        raw["generationTokens"] = 12.into();
        let evidence = serde_json::json!({
            "metadataSchemaVersion":2,"eosPolicy":"model-generation-config+tokenizer-v1",
            "samplingPolicy":"greedy-argmax-v1","terminalTokenId":151645,
            "configuredEosTokenIds":[151645],"tokenizerEosTokenId":151645,
            "inputAudioSamples":16000,"featureChunkCount":1,"featureTokenLengths":[13],
            "audioTokenCount":13,"audioEmbeddingCount":13
        });
        for (field, value) in evidence.as_object().unwrap() {
            raw[field] = value.clone();
        }
        raw["chunkDurationSeconds"] = 1.into();
        raw["segments"][0]["asr_chunk"] = 0.into();
        raw["chunks"] = serde_json::json!([{
            "index":0,"start":0.0,"end":1.0,"rawText":"예시 모델 원본","generationCount":1,
            "generationTokens":12,"maxTokens":65536,"effectiveMaxTokens":65536,
            "promptTokens":64,"contextSize":131072,"terminationReason":"eos",
            "hitTokenLimit":false,"parsedCompletely":true
        }]);
        for (field, value) in evidence.as_object().unwrap() {
            raw["chunks"][0][field] = value.clone();
        }
        let mut generation = evidence;
        for field in [
            "promptTokens",
            "maxTokens",
            "effectiveMaxTokens",
            "contextSize",
            "generationTokens",
            "terminationReason",
        ] {
            generation[field] = raw[field].clone();
        }
        raw["generationMetadata"] = serde_json::json!([generation]);
        raw["chunks"][0]["generationMetadata"] = raw["generationMetadata"].clone();
        serde_json::to_vec(&raw).unwrap()
    }
    fn stored_whole_v1_output() -> Vec<u8> {
        let mut raw: serde_json::Value = serde_json::from_slice(&whole_output()).unwrap();
        raw["runtime"] = STORED_WHOLE_RECORDING_RUNTIME_V1.into();
        for field in [
            "metadataSchemaVersion",
            "eosPolicy",
            "samplingPolicy",
            "terminalTokenId",
            "configuredEosTokenIds",
            "tokenizerEosTokenId",
            "inputAudioSamples",
            "featureChunkCount",
            "featureTokenLengths",
            "audioTokenCount",
            "audioEmbeddingCount",
        ] {
            raw.as_object_mut().unwrap().remove(field);
            raw["chunks"][0].as_object_mut().unwrap().remove(field);
            raw["generationMetadata"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
        }
        raw["chunks"][0]["generationMetadata"] = raw["generationMetadata"].clone();
        serde_json::to_vec(&raw).unwrap()
    }
    fn speaker_output(labels: &[serde_json::Value]) -> Vec<u8> {
        let mut raw: serde_json::Value = serde_json::from_slice(&output()).unwrap();
        raw["segments"] = labels
            .iter()
            .enumerate()
            .map(|(index, label)| {
                serde_json::json!({"start":0.0,"end":1.0,"text":format!("예시 발언 {}",index + 1),"speaker_id":label})
            })
            .collect();
        serde_json::to_vec(&raw).unwrap()
    }
    fn chunk_output() -> Vec<u8> {
        let mut raw: serde_json::Value = serde_json::from_slice(&output()).unwrap();
        raw["segments"] = serde_json::json!([
            {"start":0.0,"end":0.2,"text":"[S1] 첫 구간","speaker_id":"S1","asr_chunk":0},
            {"start":0.2,"end":0.3,"text":"[S1] 같은 구간","speaker_id":"S1","asr_chunk":0},
            {"start":0.4,"end":0.5,"text":"[S1] 다음 구간","speaker_id":"S1","asr_chunk":1},
            {"start":0.6,"end":0.7,"text":"[S2] 다른 화자","speaker_id":2,"asr_chunk":1}
        ]);
        serde_json::to_vec(&raw).unwrap()
    }
    fn legacy_value(mut value: serde_json::Value, keep_speakers: bool) -> serde_json::Value {
        value.as_object_mut().unwrap().remove("source");
        if !keep_speakers {
            value.as_object_mut().unwrap().remove("speakers");
        }
        for segment in value["segments"].as_array_mut().unwrap() {
            segment.as_object_mut().unwrap().remove("sourceIndex");
            if !keep_speakers {
                segment.as_object_mut().unwrap().remove("speakerId");
            }
        }
        value
    }
    fn make_legacy(engine: &Engine, id: &str, keep_speakers: bool) -> (PathBuf, Vec<u8>) {
        let doc = engine.transcript(id).unwrap().unwrap();
        let directory = engine
            .store
            .root
            .join(transcript_dir(id, &doc.version.id).unwrap());
        let initial: serde_json::Value = store::read_json(&directory.join("initial.json")).unwrap();
        std::fs::write(
            directory.join("initial.json"),
            serde_json::to_vec_pretty(&legacy_value(initial, keep_speakers)).unwrap(),
        )
        .unwrap();
        let before = serde_json::to_vec_pretty(&legacy_value(
            serde_json::to_value(doc).unwrap(),
            keep_speakers,
        ))
        .unwrap();
        std::fs::write(directory.join("current.json"), &before).unwrap();
        (directory, before)
    }
    #[test]
    fn import_preserves_source_and_reopens_playback() {
        let (temp, engine, id, source) = setup();
        let bytes = std::fs::read(&source).unwrap();
        let meeting = &engine.meetings[&id].meeting;
        let audio = engine.audio_path(&id).unwrap();
        assert!(audio.is_file());
        let saved_source = engine
            .store
            .root
            .join("meetings")
            .join(&id)
            .join("sessions")
            .join(&meeting.audio.as_ref().unwrap().session_id)
            .join("source.wav");
        assert_eq!(std::fs::read(saved_source).unwrap(), bytes);
        drop(engine);
        let engine = Engine::open(temp.path().canonicalize().unwrap().join("store")).unwrap();
        assert!(engine.audio_path(&id).is_ok());
        assert_eq!(std::fs::read(source).unwrap(), bytes);
    }
    #[test]
    fn pre_transcript_meeting_reopens_without_rewriting_its_metadata() {
        let (_temp, engine, id, _) = setup();
        let root = engine.store.root.clone();
        let path = root.join("meetings").join(&id).join("meeting.json");
        let mut old: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        old["meeting"]
            .as_object_mut()
            .unwrap()
            .remove("transcriptId");
        let bytes = serde_json::to_vec(&old).unwrap();
        drop(engine);
        std::fs::write(&path, &bytes).unwrap();
        let reopened = Engine::open(root).unwrap();
        assert!(reopened.transcript(&id).unwrap().is_none());
        assert!(reopened.audio_path(&id).is_ok());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    #[test]
    fn transcript_edits_are_versioned_and_raw_is_immutable_across_reopen() {
        let (temp, mut engine, id, _) = setup();
        let raw = output();
        let doc = engine.commit_transcript(&id, None, raw.clone()).unwrap();
        let edited = engine
            .edit_transcript(&id, doc.version.clone(), vec!["수정한 문장".into()])
            .unwrap();
        assert_eq!(edited.segments[0].original_text, "원문");
        assert!(engine
            .edit_transcript(&id, doc.version, vec!["stale".into()])
            .is_err());
        let raw_path = engine
            .store
            .root
            .join(transcript_dir(&id, &edited.version.id).unwrap())
            .join("raw.json");
        assert_eq!(std::fs::read(&raw_path).unwrap(), raw);
        drop(engine);
        let engine = Engine::open(temp.path().canonicalize().unwrap().join("store")).unwrap();
        let reopened = engine.transcript(&id).unwrap().unwrap();
        assert_eq!(reopened.version, edited.version);
        assert_eq!(reopened.segments[0].text, "수정한 문장");
        assert_eq!(std::fs::read(raw_path).unwrap(), raw);
    }
    #[test]
    fn late_retranscription_preserves_edits_and_candidate_raw() {
        let (_temp, mut engine, id, _) = setup();
        let first = engine.commit_transcript(&id, None, output()).unwrap();
        let edited = engine
            .edit_transcript(&id, first.version.clone(), vec!["수정".into()])
            .unwrap();
        assert!(engine
            .commit_transcript(&id, Some(first.version), output())
            .is_err());
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            edited.version
        );
        let dir = engine
            .store
            .root
            .join("meetings")
            .join(&id)
            .join("transcripts");
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), 2);
    }

    #[test]
    fn whole_commit_rejects_stale_and_diagnostic_workers_without_replacing_user_edits() {
        let (_temp, mut engine, id, _) = setup();
        let first = engine
            .commit_whole_transcript(&id, None, whole_output())
            .unwrap();
        let edited = engine
            .edit_transcript(&id, first.version, vec!["사용자 수정".into()])
            .unwrap();
        let directory = engine
            .store
            .root
            .join(transcript_dir(&id, &edited.version.id).unwrap());
        let before = std::fs::read(directory.join("current.json")).unwrap();
        let original_raw = std::fs::read(directory.join("raw.json")).unwrap();
        let rejected = [output(), chunk_output()];
        for raw in &rejected {
            assert!(engine
                .commit_whole_transcript(&id, Some(edited.version.clone()), raw.clone())
                .is_err());
            assert_eq!(
                std::fs::read(directory.join("current.json")).unwrap(),
                before
            );
        }
        let parent = directory.parent().unwrap();
        let candidates: Vec<_> = std::fs::read_dir(parent)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path != &directory)
            .collect();
        assert_eq!(candidates.len(), 2);
        for candidate in candidates {
            let retained_raw = std::fs::read(candidate.join("raw.json")).unwrap();
            assert!(rejected.contains(&retained_raw));
            assert!(candidate.join("initial.json").is_file());
            assert!(!candidate.join("current.json").exists());
        }
        let current = engine.transcript(&id).unwrap().unwrap();
        assert_eq!(current.version, edited.version);
        assert_eq!(current.segments[0].text, "사용자 수정");
        assert_eq!(
            std::fs::read(directory.join("raw.json")).unwrap(),
            original_raw
        );
        assert_eq!(
            engine
                .history_document(&id, &current.version.id)
                .unwrap()
                .0
                .version,
            current.version
        );
    }

    #[test]
    fn whole_commit_keeps_source_cas_and_legacy_commit_reading_available() {
        let (_temp, mut engine, id, _) = setup();
        assert!(engine.commit_whole_transcript(&id, None, output()).is_err());
        assert!(engine.transcript(&id).unwrap().is_none());
        let legacy = engine.commit_transcript(&id, None, chunk_output()).unwrap();
        assert_eq!(
            legacy.segments[0].speaker_id.as_deref(),
            Some("moss:chunk:0:S1")
        );
        let legacy = engine
            .edit_transcript(
                &id,
                legacy.version,
                vec![
                    "수정1".into(),
                    "수정2".into(),
                    "수정3".into(),
                    "수정4".into(),
                ],
            )
            .unwrap();
        assert!(engine.history_document(&id, &legacy.version.id).is_ok());
        let whole = engine
            .commit_whole_transcript(&id, Some(legacy.version), whole_output())
            .unwrap();
        let edited = engine
            .edit_transcript(&id, whole.version.clone(), vec!["최신 수정".into()])
            .unwrap();
        let candidate_raw = whole_output();
        assert!(engine
            .commit_whole_transcript(&id, Some(whole.version), candidate_raw.clone())
            .is_err());
        let current = engine.transcript(&id).unwrap().unwrap();
        assert_eq!(current.version, edited.version);
        assert_eq!(current.segments[0].text, "최신 수정");
        let parent = engine
            .store
            .root
            .join("meetings")
            .join(&id)
            .join("transcripts");
        let retained_candidate = std::fs::read_dir(parent)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .any(|path| {
                !path.join("current.json").exists()
                    && path.join("initial.json").is_file()
                    && std::fs::read(path.join("raw.json")).unwrap() == candidate_raw
            });
        assert!(retained_candidate);
    }
    #[test]
    fn incomplete_malformed_and_wrong_audio_outputs_do_not_replace_transcript() {
        let (_temp, mut engine, id, _) = setup();
        let first = engine.commit_transcript(&id, None, output()).unwrap();
        for field in [
            "hitTokenLimit",
            "hasUnparsedText",
            "inputAudioFrames",
            "audioDurationSeconds",
        ] {
            let mut bad: serde_json::Value = serde_json::from_slice(&output()).unwrap();
            bad[field] = match field {
                "inputAudioFrames" => 32000.into(),
                "audioDurationSeconds" => 10.into(),
                _ => true.into(),
            };
            assert!(engine
                .commit_transcript(
                    &id,
                    Some(first.version.clone()),
                    serde_json::to_vec(&bad).unwrap()
                )
                .is_err());
            assert_eq!(
                engine.transcript(&id).unwrap().unwrap().version,
                first.version
            );
        }
        assert!(engine
            .commit_transcript(&id, Some(first.version.clone()), b"invalid".to_vec())
            .is_err());
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            first.version
        );
    }
    #[test]
    fn model_speakers_preserve_clusters_and_unknown_metadata_does_not_lose_text() {
        let labels = vec![
            serde_json::json!("S02"),
            serde_json::json!("S1"),
            serde_json::json!("S02"),
            serde_json::json!(null),
            serde_json::json!({"invalid": true}),
            serde_json::json!("unknown"),
            serde_json::json!(3),
        ];
        let doc =
            MossOutput::decode(&speaker_output(&labels), 1.0, "session", "transcript").unwrap();
        assert_eq!(doc.segments.len(), labels.len());
        assert_eq!(
            doc.speakers,
            vec![
                TranscriptSpeaker {
                    id: "S02".into(),
                    name: "화자 1".into(),
                    name_origin: Some(SpeakerNameOrigin::Automatic),
                },
                TranscriptSpeaker {
                    id: "S1".into(),
                    name: "화자 2".into(),
                    name_origin: Some(SpeakerNameOrigin::Automatic),
                },
                TranscriptSpeaker {
                    id: "S3".into(),
                    name: "화자 3".into(),
                    name_origin: Some(SpeakerNameOrigin::Automatic),
                },
            ]
        );
        assert_eq!(doc.segments[0].speaker_id.as_deref(), Some("S02"));
        assert_eq!(doc.segments[2].speaker_id.as_deref(), Some("S02"));
        assert!(doc.segments[3..6]
            .iter()
            .all(|segment| segment.speaker_id.is_none()));
        assert_eq!(doc.segments[4].text, "예시 발언 5");
        let many: Vec<_> = (1..=10)
            .map(|n| serde_json::json!(format!("S{n}")))
            .collect();
        assert_eq!(
            MossOutput::decode(&speaker_output(&many), 1.0, "s", "t")
                .unwrap()
                .speakers
                .len(),
            10
        );
    }
    #[test]
    fn independent_asr_chunks_never_link_reused_model_labels_automatically() {
        let doc = MossOutput::decode(&chunk_output(), 1.0, "session", "transcript").unwrap();
        assert_eq!(doc.source.as_ref().unwrap().association, "direct");
        assert_eq!(
            doc.source.as_ref().unwrap().raw_sha256,
            sha256(&chunk_output())
        );
        assert_eq!(
            doc.segments
                .iter()
                .map(|segment| segment.source_index)
                .collect::<Vec<_>>(),
            vec![Some(0), Some(1), Some(2), Some(3)]
        );
        let ids: Vec<_> = doc
            .segments
            .iter()
            .map(|segment| segment.speaker_id.as_deref())
            .collect();
        assert_eq!(
            ids,
            vec![
                Some("moss:chunk:0:S1"),
                Some("moss:chunk:0:S1"),
                Some("moss:chunk:1:S1"),
                Some("moss:chunk:1:S2")
            ]
        );
        assert_eq!(
            doc.speakers
                .iter()
                .map(|speaker| speaker.name.as_str())
                .collect::<Vec<_>>(),
            vec!["화자 1", "화자 2", "화자 3"]
        );
        assert_eq!(doc.segments[0].text, "첫 구간");
        assert_eq!(doc.segments[2].original_text, "다음 구간");
        validate_speakers(
            &doc.speakers,
            &doc.segments
                .iter()
                .map(|segment| segment.speaker_id.clone())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let single = MossOutput::decode(&output(), 1.0, "session", "transcript").unwrap();
        assert_eq!(single.segments[0].speaker_id.as_deref(), Some("S1"));
    }

    #[test]
    fn verified_whole_recording_labels_have_recording_scope_without_chunk_identity() {
        let mut raw: serde_json::Value = serde_json::from_slice(&whole_output()).unwrap();
        raw["segments"] = serde_json::json!([
            {"start":0.0,"end":0.3,"text":"[S1] 첫 발언","speaker_id":"S1","asr_chunk":0},
            {"start":0.3,"end":0.6,"text":"[S2] 다른 발언","speaker_id":"S2","asr_chunk":0},
            {"start":0.6,"end":1.0,"text":"[S1] 이어진 발언","speaker_id":"S1","asr_chunk":0}
        ]);
        let raw = serde_json::to_vec(&raw).unwrap();
        let doc = MossOutput::decode(&raw, 1.0, "session", "transcript").unwrap();
        let ids: Vec<_> = doc
            .segments
            .iter()
            .map(|segment| segment.speaker_id.as_deref())
            .collect();
        assert_eq!(
            ids,
            vec![
                Some("moss:recording:S1"),
                Some("moss:recording:S2"),
                Some("moss:recording:S1")
            ]
        );
        assert_eq!(doc.speakers.len(), 2);
        assert!(doc
            .speakers
            .iter()
            .all(|speaker| speaker.name_origin == Some(SpeakerNameOrigin::Automatic)));
        let source = doc.source.as_ref().unwrap();
        assert_eq!(source.projector_version, RECORDING_PROJECTOR_VERSION);
        assert_eq!(source.speaker_scope.as_deref(), Some("recording"));
        assert_eq!(
            source.chunking_policy.as_deref(),
            Some(WHOLE_RECORDING_POLICY)
        );
        assert_eq!(source.raw_sha256, sha256(&raw));
        assert_eq!(doc.segments[2].text, "이어진 발언");
        assert_eq!(doc.segments[2].source_index, Some(2));
        let mut explicit: serde_json::Value = serde_json::from_slice(&chunk_output()).unwrap();
        explicit["speakerScope"] = "chunk".into();
        explicit["chunkingPolicy"] = "explicit".into();
        explicit["generationCount"] = 2.into();
        let old = MossOutput::decode(
            &serde_json::to_vec(&explicit).unwrap(),
            1.0,
            "session",
            "transcript",
        )
        .unwrap();
        assert_eq!(
            old.segments[0].speaker_id.as_deref(),
            Some("moss:chunk:0:S1")
        );
        assert_eq!(
            old.segments[2].speaker_id.as_deref(),
            Some("moss:chunk:1:S1")
        );
        assert_eq!(
            old.source.as_ref().unwrap().projector_version,
            PROJECTOR_VERSION
        );
        assert!(old.source.as_ref().unwrap().speaker_scope.is_none());
        assert!(serde_json::to_value(&old).unwrap()["source"]
            .get("speakerScope")
            .is_none());
        let mut single_chunk: serde_json::Value = serde_json::from_slice(&output()).unwrap();
        single_chunk["segments"][0]["asr_chunk"] = 0.into();
        let single_chunk = MossOutput::decode(
            &serde_json::to_vec(&single_chunk).unwrap(),
            1.0,
            "session",
            "transcript",
        )
        .unwrap();
        assert_eq!(
            single_chunk.segments[0].speaker_id.as_deref(),
            Some("moss:chunk:0:S1")
        );
        assert!(single_chunk
            .source
            .as_ref()
            .unwrap()
            .speaker_scope
            .is_none());
    }

    #[test]
    fn whole_scope_requires_single_complete_generation_with_measured_context_budget() {
        let valid: serde_json::Value = serde_json::from_slice(&whole_output()).unwrap();
        for (field, value) in [
            ("speakerScope", serde_json::json!("chunk")),
            ("speakerScope", serde_json::Value::Null),
            ("chunkingPolicy", serde_json::json!("explicit")),
            ("generationCount", serde_json::json!(2)),
            ("maxTokens", serde_json::json!(8192)),
            ("effectiveMaxTokens", serde_json::json!(65535)),
            ("generationTokens", serde_json::json!(65537)),
            ("generationTokens", serde_json::json!(65536)),
            ("promptTokens", serde_json::json!(0)),
            ("promptTokens", serde_json::json!(131072)),
            ("contextSize", serde_json::json!(131073)),
            ("runtime", serde_json::json!("mlx-audio-swift-v0.1.3")),
            ("terminationReason", serde_json::json!("unknown")),
            ("chunkDurationSeconds", serde_json::json!(0.5)),
            ("audioDurationSeconds", serde_json::json!(0.5)),
            ("processedAudioFrames", serde_json::json!(15999)),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            assert!(
                MossOutput::decode(
                    &serde_json::to_vec(&invalid).unwrap(),
                    1.0,
                    "session",
                    "transcript"
                )
                .is_err(),
                "field {field}"
            );
        }
        for field in [
            "speakerScope",
            "chunkingPolicy",
            "generationCount",
            "effectiveMaxTokens",
            "terminationReason",
        ] {
            let mut invalid = valid.clone();
            invalid.as_object_mut().unwrap().remove(field);
            assert!(
                MossOutput::decode(
                    &serde_json::to_vec(&invalid).unwrap(),
                    1.0,
                    "session",
                    "transcript"
                )
                .is_err(),
                "missing {field}"
            );
        }
        for (field, value) in [
            ("index", serde_json::json!(1)),
            ("generationCount", serde_json::json!(2)),
            ("start", serde_json::json!(0.1)),
            ("end", serde_json::json!(0.5)),
            ("generationTokens", serde_json::json!(11)),
            ("effectiveMaxTokens", serde_json::json!(65535)),
            ("terminationReason", serde_json::json!("length")),
            ("parsedCompletely", serde_json::json!(false)),
            ("hitTokenLimit", serde_json::json!(true)),
        ] {
            let mut invalid = valid.clone();
            invalid["chunks"][0][field] = value;
            assert!(
                MossOutput::decode(
                    &serde_json::to_vec(&invalid).unwrap(),
                    1.0,
                    "session",
                    "transcript"
                )
                .is_err(),
                "chunk {field}"
            );
        }
        let mut capped_eos = valid.clone();
        capped_eos["generationTokens"] = 65536.into();
        capped_eos["chunks"][0]["generationTokens"] = 65536.into();
        assert!(MossOutput::decode(
            &serde_json::to_vec(&capped_eos).unwrap(),
            1.0,
            "session",
            "transcript"
        )
        .is_err());
        let mut missing_count = valid.clone();
        missing_count["chunks"][0]
            .as_object_mut()
            .unwrap()
            .remove("generationCount");
        assert!(MossOutput::decode(
            &serde_json::to_vec(&missing_count).unwrap(),
            1.0,
            "session",
            "transcript"
        )
        .is_err());
        let mut multiple = valid.clone();
        multiple["chunks"]
            .as_array_mut()
            .unwrap()
            .push(valid["chunks"][0].clone());
        assert!(MossOutput::decode(
            &serde_json::to_vec(&multiple).unwrap(),
            1.0,
            "session",
            "transcript"
        )
        .is_err());
        for chunk in [serde_json::json!(1), serde_json::Value::Null] {
            let mut invalid = valid.clone();
            invalid["segments"][0]["asr_chunk"] = chunk;
            assert!(MossOutput::decode(
                &serde_json::to_vec(&invalid).unwrap(),
                1.0,
                "session",
                "transcript"
            )
            .is_err());
        }
        let mut missing = valid.clone();
        missing["segments"][0]
            .as_object_mut()
            .unwrap()
            .remove("asr_chunk");
        assert!(MossOutput::decode(
            &serde_json::to_vec(&missing).unwrap(),
            1.0,
            "session",
            "transcript"
        )
        .is_err());
        let mut remaining = valid;
        remaining["promptTokens"] = 122880.into();
        remaining["effectiveMaxTokens"] = 8191.into();
        remaining["chunks"][0]["promptTokens"] = 122880.into();
        remaining["chunks"][0]["effectiveMaxTokens"] = 8191.into();
        remaining["generationMetadata"][0]["promptTokens"] = 122880.into();
        remaining["generationMetadata"][0]["effectiveMaxTokens"] = 8191.into();
        remaining["chunks"][0]["generationMetadata"] = remaining["generationMetadata"].clone();
        assert!(MossOutput::decode(
            &serde_json::to_vec(&remaining).unwrap(),
            1.0,
            "session",
            "transcript"
        )
        .is_ok());
    }

    #[test]
    fn stored_v1_whole_history_reader_summary_and_restore_remain_available_without_accepting_new_v1(
    ) {
        let (_temp, mut engine, id, _) = setup();
        let initial = engine
            .commit_whole_transcript(&id, None, whole_output())
            .unwrap();
        let directory = engine
            .store
            .root
            .join(transcript_dir(&id, &initial.version.id).unwrap());
        let v1 = stored_whole_v1_output();
        std::fs::write(directory.join("raw.json"), &v1).unwrap();
        for name in ["initial.json", "current.json"] {
            let path = directory.join(name);
            let mut doc: Transcript = store::read_json(&path).unwrap();
            doc.source.as_mut().unwrap().raw_sha256 = sha256(&v1);
            std::fs::write(path, serde_json::to_vec(&doc).unwrap()).unwrap();
        }
        let initial_bytes = std::fs::read(directory.join("initial.json")).unwrap();
        let current_bytes = std::fs::read(directory.join("current.json")).unwrap();
        let root = engine.store.root.clone();
        drop(engine);
        let mut engine = Engine::open(root.clone()).unwrap();
        let stored = engine.transcript(&id).unwrap().unwrap();
        assert_eq!(
            stored.segments[0].speaker_id.as_deref(),
            Some("moss:recording:S1")
        );
        assert_eq!(
            engine
                .summary_input(&id)
                .unwrap()
                .source
                .source
                .unwrap()
                .speaker_scope
                .as_deref(),
            Some("recording")
        );
        let history = engine.transcript_history(&id).unwrap();
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.unavailable, 0);
        let reader = crate::reader::LibraryReader::open(&root).unwrap();
        let read = reader
            .query("transcript_get", serde_json::json!({"meetingId":id}))
            .unwrap();
        assert_eq!(read["segments"][0]["speakerId"], "moss:recording:S1");
        assert_eq!(
            std::fs::read(directory.join("initial.json")).unwrap(),
            initial_bytes
        );
        assert_eq!(
            std::fs::read(directory.join("current.json")).unwrap(),
            current_bytes
        );
        assert_eq!(std::fs::read(directory.join("raw.json")).unwrap(), v1);
        let edited = engine
            .edit_transcript(&id, stored.version, vec!["수정한 이전 기록".into()])
            .unwrap();
        let next = engine
            .commit_whole_transcript(&id, Some(edited.version.clone()), whole_output())
            .unwrap();
        let restored = engine
            .restore_transcript(&id, next.version, edited.version)
            .unwrap();
        assert_eq!(restored.segments[0].text, "수정한 이전 기록");
        assert_eq!(
            restored.segments[0].speaker_id.as_deref(),
            Some("moss:recording:S1")
        );
        assert_eq!(engine.transcript_history(&id).unwrap().unavailable, 0);
        assert_eq!(
            std::fs::read(directory.join("initial.json")).unwrap(),
            initial_bytes
        );
        assert_eq!(std::fs::read(directory.join("raw.json")).unwrap(), v1);
        assert!(engine
            .commit_whole_transcript(&id, Some(restored.version.clone()), v1.clone())
            .is_err());
        assert!(engine
            .commit_transcript(&id, Some(restored.version.clone()), v1.clone())
            .is_err());
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            restored.version
        );
        for (field, value) in [
            ("hasUnparsedText", serde_json::json!(true)),
            ("terminationReason", serde_json::json!("length")),
            ("processedAudioFrames", serde_json::json!(15999)),
        ] {
            let mut incomplete: serde_json::Value = serde_json::from_slice(&v1).unwrap();
            incomplete[field] = value;
            assert!(MossOutput::decode_with_policy(
                &serde_json::to_vec(&incomplete).unwrap(),
                1.0,
                "session",
                "transcript",
                MossValidationPolicy::StoredArtifact
            )
            .is_err());
        }
    }

    #[test]
    fn whole_scope_requires_true_model_eos_and_all_acoustic_features() {
        let valid: serde_json::Value = serde_json::from_slice(&whole_output()).unwrap();
        for (field, value) in [
            ("metadataSchemaVersion", serde_json::json!(1)),
            ("eosPolicy", serde_json::json!("hardcoded-stop-set")),
            ("samplingPolicy", serde_json::json!("unknown")),
            ("terminalTokenId", serde_json::json!(151643)),
            ("terminalTokenId", serde_json::Value::Null),
            ("terminalTokenId", serde_json::json!("151645")),
            ("configuredEosTokenIds", serde_json::json!([151643, 151645])),
            ("tokenizerEosTokenId", serde_json::json!(151643)),
            ("inputAudioSamples", serde_json::json!(15999)),
            ("featureChunkCount", serde_json::json!(0)),
            ("featureTokenLengths", serde_json::json!([12])),
            ("audioTokenCount", serde_json::json!(12)),
            ("audioEmbeddingCount", serde_json::json!(12)),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value.clone();
            invalid["chunks"][0][field] = value.clone();
            invalid["generationMetadata"][0][field] = value.clone();
            invalid["chunks"][0]["generationMetadata"][0][field] = value;
            assert!(
                MossOutput::decode(
                    &serde_json::to_vec(&invalid).unwrap(),
                    1.0,
                    "session",
                    "transcript"
                )
                .is_err(),
                "invalid {field} cannot be made valid by consistent mirrors"
            );
        }
        for field in [
            "metadataSchemaVersion",
            "eosPolicy",
            "samplingPolicy",
            "terminalTokenId",
            "configuredEosTokenIds",
            "tokenizerEosTokenId",
            "inputAudioSamples",
            "featureChunkCount",
            "featureTokenLengths",
            "audioTokenCount",
            "audioEmbeddingCount",
            "generationMetadata",
        ] {
            for location in ["root", "chunk", "generation"] {
                let mut invalid = valid.clone();
                let target = match location {
                    "root" => &mut invalid,
                    "chunk" => &mut invalid["chunks"][0],
                    _ => &mut invalid["generationMetadata"][0],
                };
                if target.as_object_mut().unwrap().remove(field).is_none() {
                    continue;
                }
                assert!(
                    MossOutput::decode(
                        &serde_json::to_vec(&invalid).unwrap(),
                        1.0,
                        "session",
                        "transcript"
                    )
                    .is_err(),
                    "missing {location} {field}"
                );
            }
        }
        let mut complete = valid;
        complete["inputAudioFrames"] = 480001.into();
        complete["processedAudioFrames"] = 480001.into();
        let duration = 480001.0 / 16000.0;
        complete["audioDurationSeconds"] = duration.into();
        complete["chunkDurationSeconds"] = 31.into();
        complete["chunks"][0]["end"] = duration.into();
        for (field, value) in [
            ("inputAudioSamples", serde_json::json!(480001)),
            ("featureChunkCount", serde_json::json!(2)),
            ("featureTokenLengths", serde_json::json!([375, 1])),
            ("audioTokenCount", serde_json::json!(376)),
            ("audioEmbeddingCount", serde_json::json!(376)),
        ] {
            complete[field] = value.clone();
            complete["chunks"][0][field] = value.clone();
            complete["generationMetadata"][0][field] = value.clone();
            complete["chunks"][0]["generationMetadata"][0][field] = value;
        }
        assert!(MossOutput::decode(
            &serde_json::to_vec(&complete).unwrap(),
            duration,
            "session",
            "transcript"
        )
        .is_ok());
        complete["audioEmbeddingCount"] = 375.into();
        assert!(MossOutput::decode(
            &serde_json::to_vec(&complete).unwrap(),
            duration,
            "session",
            "transcript"
        )
        .is_err());
    }

    #[test]
    fn whole_generation_length_repetition_parse_and_pcm_failures_have_distinct_causes() {
        for (field, value, expected) in [
            (
                "terminationReason",
                serde_json::json!("length"),
                "생성 한도",
            ),
            (
                "terminationReason",
                serde_json::json!("repetition"),
                "반복 출력",
            ),
            ("hasUnparsedText", serde_json::json!(true), "일부 발언"),
            (
                "processedAudioFrames",
                serde_json::json!(15999),
                "끝까지 완료",
            ),
        ] {
            let mut raw: serde_json::Value = serde_json::from_slice(&whole_output()).unwrap();
            raw[field] = value;
            if field == "hasUnparsedText" {
                raw["chunks"][0]["parsedCompletely"] = false.into();
            }
            if field == "processedAudioFrames" {
                raw["chunks"][0]["end"] = (15999.0 / 16000.0).into();
            }
            let error = MossOutput::decode(
                &serde_json::to_vec(&raw).unwrap(),
                1.0,
                "session",
                "transcript",
            )
            .unwrap_err();
            assert!(error.to_string().contains(expected));
        }
    }

    #[test]
    fn whole_recording_scope_preserves_current_history_and_never_upgrades_missing_source() {
        let (_temp, mut engine, id, _) = setup();
        let raw = whole_output();
        let source = engine.commit_transcript(&id, None, raw.clone()).unwrap();
        let edited = engine
            .edit_transcript(&id, source.version, vec!["사용자 수정".into()])
            .unwrap();
        assert_eq!(
            engine
                .history_document(&id, &edited.version.id)
                .unwrap()
                .0
                .segments[0]
                .text,
            "사용자 수정"
        );
        let directory = engine
            .store
            .root
            .join(transcript_dir(&id, &edited.version.id).unwrap());
        let initial = std::fs::read(directory.join("initial.json")).unwrap();
        assert_eq!(std::fs::read(directory.join("raw.json")).unwrap(), raw);
        assert_eq!(engine.upgrade_transcript_sources().unwrap().upgraded, 0);
        assert_eq!(
            std::fs::read(directory.join("initial.json")).unwrap(),
            initial
        );
        let (_directory, before) = make_legacy(&engine, &id, true);
        let report = engine.upgrade_transcript_sources().unwrap();
        assert_eq!(report.upgraded, 0);
        assert_eq!(report.unavailable, 1);
        assert_eq!(
            std::fs::read(directory.join("current.json")).unwrap(),
            before
        );
        assert_eq!(std::fs::read(directory.join("raw.json")).unwrap(), raw);
    }

    #[test]
    fn recording_scope_cannot_be_added_to_stored_chunk_raw_by_source_metadata_alone() {
        let (_temp, mut engine, id, _) = setup();
        let doc = engine.commit_transcript(&id, None, chunk_output()).unwrap();
        let directory = engine
            .store
            .root
            .join(transcript_dir(&id, &doc.version.id).unwrap());
        for name in ["initial.json", "current.json"] {
            let path = directory.join(name);
            let mut value: serde_json::Value = store::read_json(&path).unwrap();
            value["source"]["projectorVersion"] = RECORDING_PROJECTOR_VERSION.into();
            value["source"]["speakerScope"] = "recording".into();
            value["source"]["chunkingPolicy"] = WHOLE_RECORDING_POLICY.into();
            std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        assert!(engine.history_document(&id, &doc.version.id).is_err());
    }
    #[test]
    fn missing_or_invalid_chunk_scope_keeps_text_without_unscoped_identity_links() {
        let mut raw: serde_json::Value = serde_json::from_slice(&chunk_output()).unwrap();
        raw["segments"][0]["asr_chunk"] = serde_json::Value::Null;
        raw["segments"][1]["asr_chunk"] = (-1).into();
        raw["segments"][2]
            .as_object_mut()
            .unwrap()
            .remove("asr_chunk");
        raw["segments"][3]["asr_chunk"] = "1".into();
        let doc = MossOutput::decode(
            &serde_json::to_vec(&raw).unwrap(),
            1.0,
            "session",
            "transcript",
        )
        .unwrap();
        assert!(doc.speakers.is_empty());
        assert!(doc
            .segments
            .iter()
            .all(|segment| segment.speaker_id.is_none()));
        assert_eq!(doc.segments.len(), 4);
        assert_eq!(doc.segments[0].text, "첫 구간");
        let invalid_ids = speaker_output(&[
            serde_json::json!(0),
            serde_json::json!(-1),
            serde_json::json!(1.5),
        ]);
        let unknown = MossOutput::decode(&invalid_ids, 1.0, "session", "transcript").unwrap();
        assert!(unknown.speakers.is_empty());
        assert_eq!(unknown.segments.len(), 3);
    }
    #[test]
    fn legacy_chunk_labels_are_recovered_locally_and_manual_merge_is_explicit() {
        let (_temp, mut engine, id, _) = setup();
        let raw = chunk_output();
        let first = engine.commit_transcript(&id, None, raw.clone()).unwrap();
        let mut texts: Vec<_> = first
            .segments
            .iter()
            .map(|segment| segment.text.clone())
            .collect();
        texts[2] = "사용자가 고친 구간".into();
        let edited = engine.edit_transcript(&id, first.version, texts).unwrap();
        let (directory, before) = make_legacy(&engine, &id, false);
        let current = directory.join("current.json");
        assert!(engine.transcript(&id).unwrap().unwrap().speakers.is_empty());
        assert_eq!(std::fs::read(&current).unwrap(), before);
        assert_eq!(engine.upgrade_transcript_sources().unwrap().upgraded, 1);
        let recovered = engine.transcript(&id).unwrap().unwrap();
        assert_ne!(
            recovered.segments[0].speaker_id,
            recovered.segments[2].speaker_id
        );
        assert_eq!(recovered.segments[2].text, "사용자가 고친 구간");
        assert_eq!(recovered.segments[2].original_text, "다음 구간");
        assert_eq!(recovered.version.revision, edited.version.revision + 1);
        assert_eq!(recovered.version.id, edited.version.id);
        let merged = engine
            .edit_speakers(
                &id,
                recovered.version,
                vec![recovered.speakers[0].clone()],
                vec![recovered.segments[0].speaker_id.clone(); 4],
            )
            .unwrap();
        assert!(merged
            .segments
            .iter()
            .all(|segment| segment.speaker_id.as_deref() == Some("moss:chunk:0:S1")));
        assert_eq!(merged.segments[2].text, "사용자가 고친 구간");
        assert_eq!(std::fs::read(directory.join("raw.json")).unwrap(), raw);
    }
    #[test]
    fn speaker_only_changes_stale_the_summary_and_reject_late_results_without_losing_minutes() {
        let (_temp, mut engine, id, _) = setup();
        let source = engine.commit_transcript(&id, None, chunk_output()).unwrap();
        let draft = |source: Transcript, expected_id: Option<String>| crate::SummaryDraft {
            source,
            notes: None,
            expected_id,
            text: "예시 회의록 [1]".into(),
            provenance: crate::SummaryProvenance {
                producer: None,
                model: Some("test".into()),
                model_revision: Some("1".into()),
                model_sha256: Some("a".repeat(64)),
                runtime: Some("test".into()),
                prompt_version: Some("test".into()),
                template: Some("minutes".into()),
            },
            receipt: serde_json::json!({"finish_reason":"stop"}),
        };
        let first = engine
            .commit_summary(&id, draft(source.clone(), None))
            .unwrap();
        assert!(!engine.summary(&id).unwrap().unwrap().stale);
        let root = engine.store.root.clone();
        let archive = root
            .join("meetings")
            .join(&id)
            .join("summaries")
            .join(&first.id)
            .join("run.json");
        let original_minutes = std::fs::read(&archive).unwrap();
        let texts: Vec<_> = source
            .segments
            .iter()
            .map(|segment| segment.text.clone())
            .collect();
        let assignments: Vec<_> = source
            .segments
            .iter()
            .map(|segment| segment.speaker_id.clone())
            .collect();
        let mut names = source.speakers.clone();
        names[0].name = "진행자".into();
        let renamed = engine
            .edit_speakers(&id, source.version.clone(), names, assignments.clone())
            .unwrap();
        assert!(engine.summary(&id).unwrap().unwrap().stale);
        assert_eq!(
            renamed
                .segments
                .iter()
                .map(|segment| segment.text.clone())
                .collect::<Vec<_>>(),
            texts
        );
        assert!(engine
            .commit_summary(&id, draft(source, Some(first.id.clone())))
            .is_err());
        assert_eq!(engine.summary(&id).unwrap().unwrap().document.id, first.id);
        assert_eq!(std::fs::read(&archive).unwrap(), original_minutes);
        let second = engine
            .commit_summary(&id, draft(renamed.clone(), Some(first.id)))
            .unwrap();
        assert!(!engine.summary(&id).unwrap().unwrap().stale);
        let mut next_assignments = assignments;
        next_assignments[2] = renamed.segments[0].speaker_id.clone();
        let reassigned = engine
            .edit_speakers(
                &id,
                renamed.version.clone(),
                renamed.speakers.clone(),
                next_assignments,
            )
            .unwrap();
        assert!(engine.summary(&id).unwrap().unwrap().stale);
        assert!(engine
            .commit_summary(&id, draft(renamed, Some(second.id.clone())))
            .is_err());
        assert_eq!(engine.summary(&id).unwrap().unwrap().document.id, second.id);
        let third = engine
            .commit_summary(&id, draft(reassigned.clone(), Some(second.id)))
            .unwrap();
        drop(engine);
        let reopened = Engine::open(root).unwrap();
        let minutes = reopened.summary(&id).unwrap().unwrap();
        assert_eq!(minutes.document.id, third.id);
        assert!(!minutes.stale);
        assert_eq!(
            reopened.transcript(&id).unwrap().unwrap().version,
            reassigned.version
        );
        assert_eq!(std::fs::read(archive).unwrap(), original_minutes);
    }
    #[test]
    fn speaker_name_origin_is_optional_without_changing_legacy_serialization() {
        let old = r#"{"id":"S1","name":"예시 이름"}"#;
        let speaker: TranscriptSpeaker = serde_json::from_str(old).unwrap();
        assert_eq!(speaker.name_origin, None);
        assert_eq!(serde_json::to_string(&speaker).unwrap(), old);
        for (origin, expected) in [
            (SpeakerNameOrigin::Automatic, "automatic"),
            (SpeakerNameOrigin::User, "user"),
        ] {
            let mut explicit = speaker.clone();
            explicit.name_origin = Some(origin);
            assert_eq!(
                serde_json::to_value(explicit).unwrap()["nameOrigin"],
                expected
            );
        }
    }
    #[test]
    fn speaker_edits_preserve_existing_origin_and_derive_new_origin_from_name_changes() {
        let (_temp, mut engine, id, _) = setup();
        let original = engine.commit_transcript(&id, None, output()).unwrap();
        assert_eq!(
            original.speakers[0].name_origin,
            Some(SpeakerNameOrigin::Automatic)
        );
        let assignments = vec![original.segments[0].speaker_id.clone()];
        let original_digest = crate::summary::transcript_digest(&original).unwrap();
        for requested_origin in [None, Some(SpeakerNameOrigin::User)] {
            let mut omitted_or_forged = original.speakers.clone();
            omitted_or_forged[0].name_origin = requested_origin;
            let unchanged = engine
                .edit_speakers(
                    &id,
                    original.version.clone(),
                    omitted_or_forged,
                    assignments.clone(),
                )
                .unwrap();
            assert_eq!(unchanged.version, original.version);
            assert_eq!(
                crate::summary::transcript_digest(&unchanged).unwrap(),
                original_digest
            );
            assert_eq!(
                unchanged.speakers[0].name_origin,
                Some(SpeakerNameOrigin::Automatic)
            );
        }
        let mut renamed = original.speakers.clone();
        renamed[0].name = "사용자 표시명".into();
        renamed[0].name_origin = Some(SpeakerNameOrigin::Automatic);
        let renamed = engine
            .edit_speakers(&id, original.version, renamed, assignments.clone())
            .unwrap();
        assert_eq!(
            renamed.speakers[0].name_origin,
            Some(SpeakerNameOrigin::User)
        );
        let mut no_origin = renamed.speakers.clone();
        no_origin[0].name_origin = None;
        let unchanged = engine
            .edit_speakers(&id, renamed.version.clone(), no_origin, assignments)
            .unwrap();
        assert_eq!(unchanged.version, renamed.version);
        assert_eq!(
            unchanged.speakers[0].name_origin,
            Some(SpeakerNameOrigin::User)
        );
        let mut new_speakers = unchanged.speakers.clone();
        new_speakers.push(TranscriptSpeaker {
            id: "manual-new".into(),
            name: "화자 1".into(),
            name_origin: Some(SpeakerNameOrigin::Automatic),
        });
        let added = engine
            .edit_speakers(
                &id,
                unchanged.version,
                new_speakers,
                vec![Some("manual-new".into())],
            )
            .unwrap();
        assert_eq!(added.speakers[1].name_origin, Some(SpeakerNameOrigin::User));
        let root = engine.store.root.clone();
        drop(engine);
        let reopened = Engine::open(root)
            .unwrap()
            .transcript(&id)
            .unwrap()
            .unwrap();
        assert_eq!(reopened.speakers, added.speakers);
    }
    #[test]
    fn speaker_rename_merge_split_and_restore_are_versioned_without_changing_content() {
        let (_temp, mut engine, id, _) = setup();
        let raw = speaker_output(&[serde_json::json!("S1"), serde_json::json!("S2")]);
        let original = engine.commit_transcript(&id, None, raw.clone()).unwrap();
        let edited = engine
            .edit_transcript(
                &id,
                original.version,
                vec!["수정한 발언".into(), "두 번째 발언".into()],
            )
            .unwrap();
        let saved_segments = serde_json::to_value(&edited.segments).unwrap();
        let saved_speakers = edited.speakers.clone();
        let saved_assignments: Vec<_> = edited
            .segments
            .iter()
            .map(|segment| segment.speaker_id.clone())
            .collect();
        let mut renamed_speakers = edited.speakers.clone();
        renamed_speakers[0].name = "진행자".into();
        let renamed = engine
            .edit_speakers(
                &id,
                edited.version.clone(),
                renamed_speakers,
                saved_assignments.clone(),
            )
            .unwrap();
        assert_eq!(renamed.version.revision, edited.version.revision + 1);
        assert_eq!(
            serde_json::to_value(&renamed.segments).unwrap(),
            saved_segments
        );
        assert!(engine
            .edit_transcript(
                &id,
                edited.version,
                vec!["오래된 수정".into(), "오래된 수정".into()]
            )
            .is_err());
        let merged = engine
            .edit_speakers(
                &id,
                renamed.version,
                vec![renamed.speakers[0].clone()],
                vec![Some("S1".into()), Some("S1".into())],
            )
            .unwrap();
        let split_id = Uuid::new_v4().to_string();
        let mut split_speakers = merged.speakers.clone();
        split_speakers.push(TranscriptSpeaker {
            id: split_id.clone(),
            name: "새 화자".into(),
            name_origin: None,
        });
        let split = engine
            .edit_speakers(
                &id,
                merged.version,
                split_speakers,
                vec![Some("S1".into()), Some(split_id)],
            )
            .unwrap();
        assert_eq!(split.segments[0].text, "수정한 발언");
        assert_eq!(split.segments[0].original_text, "예시 발언 1");
        let restored = engine
            .edit_speakers(&id, split.version, saved_speakers, saved_assignments)
            .unwrap();
        assert_eq!(
            serde_json::to_value(&restored.segments).unwrap(),
            saved_segments
        );
        let same = engine
            .edit_speakers(
                &id,
                restored.version.clone(),
                restored.speakers.clone(),
                restored
                    .segments
                    .iter()
                    .map(|segment| segment.speaker_id.clone())
                    .collect(),
            )
            .unwrap();
        assert_eq!(same.version, restored.version);
        let root = engine.store.root.clone();
        let raw_path = root
            .join(transcript_dir(&id, &restored.version.id).unwrap())
            .join("raw.json");
        assert_eq!(std::fs::read(&raw_path).unwrap(), raw);
        drop(engine);
        let reopened = Engine::open(root)
            .unwrap()
            .transcript(&id)
            .unwrap()
            .unwrap();
        assert_eq!(reopened.version, restored.version);
        assert_eq!(reopened.speakers, restored.speakers);
        assert_eq!(
            serde_json::to_value(reopened.segments).unwrap(),
            saved_segments
        );
    }
    #[test]
    fn invalid_or_stale_speaker_edits_do_not_replace_current_document() {
        let (_temp, mut engine, id, _) = setup();
        let doc = engine.commit_transcript(&id, None, output()).unwrap();
        let before = serde_json::to_vec(&doc).unwrap();
        for (speakers, assignments) in [
            (doc.speakers.clone(), vec![]),
            (doc.speakers.clone(), vec![Some("missing".into())]),
            (
                vec![doc.speakers[0].clone(), doc.speakers[0].clone()],
                vec![Some("S1".into())],
            ),
            (
                vec![TranscriptSpeaker {
                    id: "../invalid".into(),
                    name: "화자".into(),
                    name_origin: None,
                }],
                vec![None],
            ),
            (
                vec![TranscriptSpeaker {
                    id: "S1".into(),
                    name: " ".into(),
                    name_origin: None,
                }],
                vec![None],
            ),
            (
                vec![TranscriptSpeaker {
                    id: "S1".into(),
                    name: "줄\n바꿈".into(),
                    name_origin: None,
                }],
                vec![None],
            ),
            (
                vec![TranscriptSpeaker {
                    id: "S1".into(),
                    name: "a".repeat(257),
                    name_origin: None,
                }],
                vec![None],
            ),
        ] {
            assert!(engine
                .edit_speakers(&id, doc.version.clone(), speakers, assignments)
                .is_err());
            assert_eq!(
                serde_json::to_vec(&engine.transcript(&id).unwrap().unwrap()).unwrap(),
                before
            );
        }
        let changed = engine
            .edit_speakers(&id, doc.version.clone(), vec![], vec![None])
            .unwrap();
        assert!(engine
            .edit_speakers(&id, doc.version, changed.speakers, vec![None])
            .is_err());
    }
    #[test]
    fn old_document_is_only_upgraded_explicitly_and_preserves_user_edits() {
        let (_temp, mut engine, id, _) = setup();
        let raw = output();
        let first = engine.commit_transcript(&id, None, raw.clone()).unwrap();
        let edited = engine
            .edit_transcript(&id, first.version, vec!["사용자 수정".into()])
            .unwrap();
        let root = engine.store.root.clone();
        let (directory, before) = make_legacy(&engine, &id, false);
        let current = directory.join("current.json");
        let initial_before = std::fs::read(directory.join("initial.json")).unwrap();
        drop(engine);
        let mut engine = Engine::open(root.clone()).unwrap();
        assert!(engine.transcript(&id).unwrap().unwrap().speakers.is_empty());
        assert_eq!(std::fs::read(&current).unwrap(), before);
        assert_eq!(engine.upgrade_transcript_sources().unwrap().upgraded, 1);
        let recovered = engine.transcript(&id).unwrap().unwrap();
        assert_eq!(recovered.version.id, edited.version.id);
        assert_eq!(recovered.version.revision, edited.version.revision + 1);
        assert_eq!(recovered.segments[0].id, edited.segments[0].id);
        assert_eq!(recovered.segments[0].text, "사용자 수정");
        assert_eq!(recovered.segments[0].original_text, "원문");
        assert_eq!(recovered.segments[0].speaker_id.as_deref(), Some("S1"));
        assert_eq!(recovered.speakers[0].name, "화자 1");
        assert_eq!(
            std::fs::read(directory.join("initial.json")).unwrap(),
            initial_before
        );
        assert_eq!(std::fs::read(directory.join("raw.json")).unwrap(), raw);
        let receipt_path = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("upgrade-receipt-")
            })
            .unwrap();
        let receipt: serde_json::Value = store::read_json(&receipt_path).unwrap();
        assert_eq!(receipt["status"], "prepared");
        assert_eq!(receipt["timebase"], 16000);
        assert_eq!(receipt["ruleVersion"], LEGACY_ASSOCIATION);
        assert_eq!(receipt["beforeCurrentSha256"], sha256(&before));
        assert_eq!(
            receipt["afterCurrentSha256"],
            sha256(&std::fs::read(&current).unwrap())
        );
        assert_eq!(receipt["rawSha256"], sha256(&raw));
        assert_eq!(receipt["initialSha256"], sha256(&initial_before));
        assert_eq!(
            std::fs::read(directory.join(receipt["beforePath"].as_str().unwrap())).unwrap(),
            before
        );
        let current_bytes = std::fs::read(&current).unwrap();
        let count = std::fs::read_dir(&directory).unwrap().count();
        let repeated = engine.upgrade_transcript_sources().unwrap();
        assert_eq!(repeated.upgraded, 0);
        assert_eq!(repeated.unchanged, 1);
        assert_eq!(std::fs::read(&current).unwrap(), current_bytes);
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), count);
        let cleared = engine
            .edit_speakers(&id, recovered.version, vec![], vec![None])
            .unwrap();
        drop(engine);
        let engine = Engine::open(root).unwrap();
        let reopened = engine.transcript(&id).unwrap().unwrap();
        assert_eq!(reopened.version, cleared.version);
        assert!(reopened.speakers.is_empty());
        assert!(reopened.segments[0].speaker_id.is_none());
    }
    #[test]
    fn old_document_without_usable_raw_remains_readable_without_invented_speakers() {
        let (_temp, mut engine, id, _) = setup();
        engine.commit_transcript(&id, None, output()).unwrap();
        let (directory, before) = make_legacy(&engine, &id, false);
        std::fs::write(directory.join("raw.json"), b"unreadable").unwrap();
        assert_eq!(engine.upgrade_transcript_sources().unwrap().unavailable, 1);
        let recovered = engine.transcript(&id).unwrap().unwrap();
        assert_eq!(recovered.segments[0].text, "원문");
        assert!(recovered.speakers.is_empty());
        assert!(recovered.segments[0].speaker_id.is_none());
        assert_eq!(
            std::fs::read(directory.join("current.json")).unwrap(),
            before
        );
        let mut mismatch: serde_json::Value = serde_json::from_slice(&output()).unwrap();
        mismatch["segments"][0]["text"] = "다른 원문".into();
        std::fs::write(
            directory.join("raw.json"),
            serde_json::to_vec(&mismatch).unwrap(),
        )
        .unwrap();
        assert_eq!(engine.upgrade_transcript_sources().unwrap().unavailable, 1);
        assert!(engine.transcript(&id).unwrap().unwrap().speakers.is_empty());
    }
    #[test]
    fn legacy_upgrade_uses_16khz_addresses_and_preserves_the_saved_times() {
        let (_temp, mut engine, id, _) = setup();
        let mut raw: serde_json::Value = serde_json::from_slice(&output()).unwrap();
        raw["segments"][0]["start"] = serde_json::json!(0.5799999999999998);
        raw["segments"][0]["end"] = serde_json::json!(0.8);
        engine
            .commit_transcript(&id, None, serde_json::to_vec(&raw).unwrap())
            .unwrap();
        let (directory, _) = make_legacy(&engine, &id, false);
        let mut initial: serde_json::Value =
            store::read_json(&directory.join("initial.json")).unwrap();
        initial["segments"][0]["end"] = serde_json::json!(0.7999999999999999);
        initial["modelRevision"] = "historical-revision".into();
        std::fs::write(
            directory.join("initial.json"),
            serde_json::to_vec(&initial).unwrap(),
        )
        .unwrap();
        let mut current: serde_json::Value =
            store::read_json(&directory.join("current.json")).unwrap();
        current["segments"][0]["start"] = serde_json::json!(0.58);
        current["modelRevision"] = "historical-revision".into();
        std::fs::write(
            directory.join("current.json"),
            serde_json::to_vec(&current).unwrap(),
        )
        .unwrap();
        let before = engine.transcript(&id).unwrap().unwrap();
        assert_ne!(0.5799999999999998_f64, before.segments[0].start);
        assert_eq!(
            source_position(15.199999999999998, u64::MAX),
            source_position(15.2, u64::MAX)
        );
        assert_eq!(engine.upgrade_transcript_sources().unwrap().upgraded, 1);
        let after = engine.transcript(&id).unwrap().unwrap();
        assert_eq!(
            after.segments[0].start.to_bits(),
            before.segments[0].start.to_bits()
        );
        assert_eq!(
            after.segments[0].end.to_bits(),
            before.segments[0].end.to_bits()
        );
        assert_eq!(after.segments[0].id, before.segments[0].id);
        assert_eq!(after.model_revision, "historical-revision");
        assert_eq!(
            after.source.as_ref().unwrap().association,
            LEGACY_ASSOCIATION
        );
        assert_eq!(after.segments[0].speaker_id.as_deref(), Some("S1"));
        let history = engine.transcript_history(&id).unwrap();
        assert_eq!(history.unavailable, 0);
        assert_eq!(history.entries.len(), 1);
        let next = engine
            .commit_transcript(&id, Some(after.version.clone()), output())
            .unwrap();
        let restored = engine
            .restore_transcript(&id, next.version, after.version)
            .unwrap();
        assert_eq!(restored.model_revision, "historical-revision");
        assert_eq!(
            restored.segments[0].start.to_bits(),
            before.segments[0].start.to_bits()
        );
        assert_eq!(
            restored.segments[0].end.to_bits(),
            before.segments[0].end.to_bits()
        );
        assert_eq!(source_position(f64::NAN, 16000), None);
        assert_eq!(source_position(f64::INFINITY, 16000), None);
        assert_eq!(source_position(-0.1, 16000), None);
        assert_eq!(source_position(1.001, 16000), None);
    }
    #[test]
    fn changed_sample_addresses_or_identity_anchors_fail_without_writes() {
        for case in [
            "current-sample",
            "initial-sample",
            "raw-sample",
            "current-id",
            "current-order",
            "current-original",
            "raw-text",
            "initial-model",
            "missing-initial",
            "current-outside",
        ] {
            let (_temp, mut engine, id, _) = setup();
            engine.commit_transcript(&id, None, chunk_output()).unwrap();
            let (directory, _) = make_legacy(&engine, &id, false);
            let mut current: serde_json::Value =
                store::read_json(&directory.join("current.json")).unwrap();
            let mut initial: serde_json::Value =
                store::read_json(&directory.join("initial.json")).unwrap();
            let mut raw: serde_json::Value = store::read_json(&directory.join("raw.json")).unwrap();
            match case {
                "current-sample" => {
                    current["segments"][0]["start"] = serde_json::json!(1.0 / 16000.0)
                }
                "initial-sample" => initial["segments"][0]["start"] = serde_json::json!(0.001),
                "raw-sample" => raw["segments"][0]["start"] = serde_json::json!(0.001),
                "current-id" => current["segments"][0]["id"] = Uuid::new_v4().to_string().into(),
                "current-order" => current["segments"].as_array_mut().unwrap().swap(0, 1),
                "current-original" => current["segments"][0]["originalText"] = "다른 원문".into(),
                "raw-text" => raw["segments"][0]["text"] = "[S1] 다른 원문".into(),
                "initial-model" => initial["modelRevision"] = "다른 실행".into(),
                "current-outside" => current["segments"][0]["end"] = serde_json::json!(2.0),
                _ => (),
            }
            std::fs::write(
                directory.join("current.json"),
                serde_json::to_vec(&current).unwrap(),
            )
            .unwrap();
            std::fs::write(
                directory.join("initial.json"),
                serde_json::to_vec(&initial).unwrap(),
            )
            .unwrap();
            std::fs::write(
                directory.join("raw.json"),
                serde_json::to_vec(&raw).unwrap(),
            )
            .unwrap();
            if case == "missing-initial" {
                std::fs::rename(
                    directory.join("initial.json"),
                    directory.join("held-initial.json"),
                )
                .unwrap();
            }
            let files = || {
                let mut entries: Vec<_> = std::fs::read_dir(&directory)
                    .unwrap()
                    .map(|entry| {
                        let path = entry.unwrap().path();
                        let bytes = std::fs::read(&path).unwrap();
                        (path, bytes)
                    })
                    .collect();
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                entries
            };
            let before = files();
            let report = engine.upgrade_transcript_sources().unwrap();
            assert_eq!(report.upgraded, 0, "{case}");
            assert_eq!(report.unavailable, 1, "{case}");
            assert_eq!(files(), before, "{case}");
            assert!(
                engine.transcript(&id).unwrap().unwrap().source.is_none(),
                "{case}"
            );
        }
    }
    #[test]
    fn explicit_unknown_speakers_survive_upgrade_and_the_prior_summary_is_retained_as_stale() {
        let (_temp, mut engine, id, _) = setup();
        let first = engine.commit_transcript(&id, None, output()).unwrap();
        engine
            .edit_speakers(&id, first.version, vec![], vec![None])
            .unwrap();
        let (directory, _) = make_legacy(&engine, &id, true);
        let before = engine.transcript(&id).unwrap().unwrap();
        let minutes = engine
            .commit_summary(
                &id,
                crate::SummaryDraft {
                    source: before.clone(),
                    notes: None,
                    expected_id: None,
                    text: "기존 회의록".into(),
                    provenance: crate::SummaryProvenance {
                        producer: None,
                        model: Some("test".into()),
                        model_revision: Some("1".into()),
                        model_sha256: Some("a".repeat(64)),
                        runtime: Some("test".into()),
                        prompt_version: Some("test".into()),
                        template: Some("minutes".into()),
                    },
                    receipt: serde_json::json!({"finish_reason":"stop"}),
                },
            )
            .unwrap();
        assert!(!engine.summary(&id).unwrap().unwrap().stale);
        let raw = std::fs::read(directory.join("raw.json")).unwrap();
        assert_eq!(engine.upgrade_transcript_sources().unwrap().upgraded, 1);
        let after = engine.transcript(&id).unwrap().unwrap();
        assert!(after.speakers.is_empty());
        assert!(after.segments[0].speaker_id.is_none());
        assert_eq!(after.segments[0].source_index, Some(0));
        assert_eq!(after.segments[0].text, before.segments[0].text);
        assert_eq!(
            after.segments[0].original_text,
            before.segments[0].original_text
        );
        assert_eq!(std::fs::read(directory.join("raw.json")).unwrap(), raw);
        let saved = engine.summary(&id).unwrap().unwrap();
        assert!(saved.stale);
        assert_eq!(saved.document.id, minutes.id);
        assert_eq!(saved.document.text, "기존 회의록");
    }
    #[test]
    fn one_unavailable_legacy_document_does_not_block_other_documents_or_direct_results() {
        let (_temp, mut engine, id, wav) = setup();
        engine.commit_transcript(&id, None, output()).unwrap();
        make_legacy(&engine, &id, false);
        let missing = engine
            .import_audio("다른 회의".into(), wav.clone(), wav.clone())
            .unwrap();
        engine
            .commit_transcript(&missing.id, None, output())
            .unwrap();
        let (missing_directory, before) = make_legacy(&engine, &missing.id, false);
        std::fs::rename(
            missing_directory.join("raw.json"),
            missing_directory.join("held-raw.json"),
        )
        .unwrap();
        let direct = engine
            .import_audio("새 회의".into(), wav.clone(), wav)
            .unwrap();
        engine
            .commit_transcript(&direct.id, None, output())
            .unwrap();
        let report = engine.upgrade_transcript_sources().unwrap();
        assert_eq!(
            report,
            TranscriptUpgradeReport {
                upgraded: 1,
                unchanged: 1,
                unavailable: 1
            }
        );
        assert!(engine.transcript(&id).unwrap().unwrap().source.is_some());
        assert!(engine
            .transcript(&missing.id)
            .unwrap()
            .unwrap()
            .source
            .is_none());
        assert_eq!(
            std::fs::read(missing_directory.join("current.json")).unwrap(),
            before
        );
        assert_eq!(
            engine
                .transcript(&direct.id)
                .unwrap()
                .unwrap()
                .source
                .unwrap()
                .association,
            "direct"
        );
    }
    #[test]
    fn retranscription_history_restores_edits_and_speakers_as_a_new_revision() {
        let (_temp, mut engine, id, _) = setup();
        let first = engine.commit_transcript(&id, None, output()).unwrap();
        let edited = engine
            .edit_transcript(&id, first.version.clone(), vec!["수정한 문장".into()])
            .unwrap();
        let mut speakers = edited.speakers.clone();
        speakers[0].name = "참석자".into();
        let edited = engine
            .edit_speakers(
                &id,
                edited.version.clone(),
                speakers,
                edited
                    .segments
                    .iter()
                    .map(|segment| segment.speaker_id.clone())
                    .collect(),
            )
            .unwrap();
        let second = engine
            .commit_transcript(&id, Some(edited.version.clone()), output())
            .unwrap();
        let root = engine.store.root.clone();
        let old_directory = root.join(transcript_dir(&id, &edited.version.id).unwrap());
        let second_path = root
            .join(transcript_dir(&id, &second.version.id).unwrap())
            .join("current.json");
        let second_bytes = std::fs::read(&second_path).unwrap();
        let raw = std::fs::read(old_directory.join("raw.json")).unwrap();
        let initial = std::fs::read(old_directory.join("initial.json")).unwrap();
        let audio = std::fs::read(engine.audio_path(&id).unwrap()).unwrap();
        let history = engine.transcript_history(&id).unwrap();
        assert_eq!(history.unavailable, 0);
        assert_eq!(history.entries.len(), 2);
        assert_eq!(history.entries[0].version, second.version);
        assert!(history.entries[0].current && !history.entries[0].edited);
        assert!(history.entries[1].edited && !history.entries[1].current);
        let restored = engine
            .restore_transcript(&id, second.version.clone(), edited.version.clone())
            .unwrap();
        assert_eq!(restored.version.id, edited.version.id);
        assert_eq!(restored.version.revision, edited.version.revision + 1);
        assert_eq!(restored.source, edited.source);
        assert_eq!(restored.segments[0].id, edited.segments[0].id);
        assert_eq!(
            restored.segments[0].source_index,
            edited.segments[0].source_index
        );
        assert_eq!(restored.segments[0].text, "수정한 문장");
        assert_eq!(restored.speakers[0].name, "참석자");
        assert_eq!(std::fs::read(&second_path).unwrap(), second_bytes);
        assert_eq!(std::fs::read(old_directory.join("raw.json")).unwrap(), raw);
        assert_eq!(
            std::fs::read(old_directory.join("initial.json")).unwrap(),
            initial
        );
        assert_eq!(
            std::fs::read(engine.audio_path(&id).unwrap()).unwrap(),
            audio
        );
        assert!(engine
            .edit_transcript(&id, edited.version, vec!["stale".into()])
            .is_err());
        drop(engine);
        let engine = Engine::open(root).unwrap();
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            restored.version
        );
        assert_eq!(engine.transcript_history(&id).unwrap().entries.len(), 2);
    }

    #[test]
    fn history_restore_requires_both_current_and_target_versions() {
        let (_temp, mut engine, id, _) = setup();
        let first = engine.commit_transcript(&id, None, output()).unwrap();
        let second = engine
            .commit_transcript(&id, Some(first.version.clone()), output())
            .unwrap();
        let changed = engine
            .edit_transcript(&id, second.version.clone(), vec!["새 수정".into()])
            .unwrap();
        assert!(engine
            .restore_transcript(&id, second.version, first.version.clone())
            .is_err());
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            changed.version
        );
        let restored = engine
            .restore_transcript(&id, changed.version.clone(), first.version.clone())
            .unwrap();
        assert!(engine
            .restore_transcript(&id, restored.version.clone(), first.version)
            .is_err());
        let second_again = engine
            .restore_transcript(&id, restored.version, changed.version.clone())
            .unwrap();
        assert_eq!(second_again.version.revision, changed.version.revision + 1);
        let current = engine.transcript(&id).unwrap().unwrap();
        assert!(engine
            .restore_transcript(
                &id,
                current.version.clone(),
                TranscriptVersion {
                    id: "../outside".into(),
                    revision: 0
                }
            )
            .is_err());
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            current.version
        );
    }

    #[test]
    fn history_rejects_damaged_source_identity_without_repair_or_partial_activation() {
        for corruption in ["raw", "id", "session", "sourceIndex", "original", "time"] {
            let (_temp, mut engine, id, _) = setup();
            let first = engine.commit_transcript(&id, None, output()).unwrap();
            let second = engine
                .commit_transcript(&id, Some(first.version.clone()), output())
                .unwrap();
            let directory = engine
                .store
                .root
                .join(transcript_dir(&id, &first.version.id).unwrap());
            let current = directory.join("current.json");
            let before_current = std::fs::read(&current).unwrap();
            if corruption == "raw" {
                std::fs::write(directory.join("raw.json"), b"{broken}").unwrap();
            } else {
                let mut value: serde_json::Value = serde_json::from_slice(&before_current).unwrap();
                match corruption {
                    "id" => value["version"]["id"] = Uuid::new_v4().to_string().into(),
                    "session" => value["sourceSessionId"] = Uuid::new_v4().to_string().into(),
                    "sourceIndex" => value["segments"][0]["sourceIndex"] = 1.into(),
                    "original" => value["segments"][0]["originalText"] = "다른 원문".into(),
                    "time" => value["segments"][0]["start"] = 0.001.into(),
                    _ => unreachable!(),
                }
                std::fs::write(&current, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            let damaged = std::fs::read(&current).unwrap();
            let history = engine.transcript_history(&id).unwrap();
            assert_eq!(history.entries.len(), 1, "{corruption}");
            assert_eq!(history.unavailable, 1, "{corruption}");
            assert!(
                engine
                    .restore_transcript(&id, second.version.clone(), first.version.clone())
                    .is_err(),
                "{corruption}"
            );
            assert_eq!(
                engine.transcript(&id).unwrap().unwrap().version,
                second.version
            );
            assert_eq!(std::fs::read(&current).unwrap(), damaged);
        }
    }

    #[test]
    fn history_is_read_only_and_does_not_guess_legacy_or_incomplete_documents() {
        let (_temp, mut engine, id, _) = setup();
        let first = engine.commit_transcript(&id, None, output()).unwrap();
        let (directory, legacy) = make_legacy(&engine, &id, true);
        assert_eq!(engine.transcript_history(&id).unwrap().unavailable, 1);
        assert_eq!(
            std::fs::read(directory.join("current.json")).unwrap(),
            legacy
        );
        assert_eq!(engine.upgrade_transcript_sources().unwrap().upgraded, 1);
        assert_eq!(engine.transcript_history(&id).unwrap().entries.len(), 1);
        let current = engine.transcript(&id).unwrap().unwrap();
        assert!(engine
            .commit_transcript(&id, Some(current.version), b"invalid".to_vec())
            .is_err());
        let history = engine.transcript_history(&id).unwrap();
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].version.id, first.version.id);
        assert_eq!(history.unavailable, 1);
        engine.trash_meetings(&[id.clone()]).unwrap();
        assert!(engine.transcript_history(&id).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn history_never_follows_transcript_directory_or_document_links() {
        use std::os::unix::fs::symlink;
        let (temp, mut engine, id, _) = setup();
        let first = engine.commit_transcript(&id, None, output()).unwrap();
        let second = engine
            .commit_transcript(&id, Some(first.version.clone()), output())
            .unwrap();
        let directory = engine
            .store
            .root
            .join(transcript_dir(&id, &first.version.id).unwrap());
        let raw = directory.join("raw.json");
        let preserved = temp.path().join("linked-source.json");
        std::fs::rename(&raw, &preserved).unwrap();
        symlink(&preserved, &raw).unwrap();
        let external = temp.path().join("outside");
        std::fs::create_dir(&external).unwrap();
        std::fs::write(external.join("current.json"), b"secret").unwrap();
        let link_id = Uuid::new_v4().to_string();
        let link = engine
            .store
            .root
            .join(transcript_dir(&id, &link_id).unwrap());
        symlink(&external, link).unwrap();
        let history = engine.transcript_history(&id).unwrap();
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.unavailable, 2);
        assert!(engine
            .restore_transcript(&id, second.version.clone(), first.version)
            .is_err());
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            second.version
        );
        assert_eq!(
            std::fs::read(external.join("current.json")).unwrap(),
            b"secret"
        );
    }
    #[test]
    fn restore_rejects_recording_shutdown_and_deleted_meetings_before_writing() {
        let (_temp, mut engine, id, _) = setup();
        let first = engine.commit_transcript(&id, None, output()).unwrap();
        let second = engine
            .commit_transcript(&id, Some(first.version.clone()), output())
            .unwrap();
        let recording = engine.create_meeting("녹음 중".into(), "".into()).unwrap();
        let manifest = Manifest::new(
            &recording.id,
            "synthetic",
            "시험 입력",
            16000,
            1,
            Encoding::I16,
        );
        let raw = engine.store.prepare_capture(&manifest).unwrap();
        let (capture, sink) =
            crate::capture::Capture::prepare(&engine.store.root, manifest, raw).unwrap();
        engine.capture = Some(capture);
        assert!(matches!(
            engine.restore_transcript(&id, second.version.clone(), first.version.clone()),
            Err(CoreError::RecordingBusy)
        ));
        engine.capture = None;
        drop(sink);
        engine.quitting = true;
        assert!(matches!(
            engine.restore_transcript(&id, second.version.clone(), first.version.clone()),
            Err(CoreError::Unavailable)
        ));
        engine.quitting = false;
        engine.trash_meetings(&[id.clone()]).unwrap();
        assert!(matches!(
            engine.restore_transcript(&id, second.version.clone(), first.version),
            Err(CoreError::MeetingNotFound)
        ));
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            second.version
        );
    }

    fn terminology_draft(source: Transcript) -> crate::TerminologyDraft {
        let profile = crate::TerminologyProfile {
            schema_version: 1,
            id: "synthetic".into(),
            revision: "1".into(),
            rules: vec![crate::TerminologyRule {
                id: "letters".into(),
                canonical: "ABC".into(),
                source_forms: vec!["A B C".into()],
                kind: crate::TerminologyKind::Acronym,
                context_cues: vec![],
                exclusion_cues: vec![],
                description: String::new(),
            }],
        };
        let decisions = crate::discover_candidates(&profile, &source)
            .unwrap()
            .into_iter()
            .map(|candidate| crate::TerminologyDecision {
                candidate_id: candidate.id,
                outcome: crate::TerminologyOutcome::Accept,
            })
            .collect();
        crate::TerminologyDraft {
            source,
            profile,
            decisions,
            receipt: serde_json::json!({"runtime":"test"}),
        }
    }

    fn terminology_raw() -> Vec<u8> {
        let mut raw: serde_json::Value = serde_json::from_slice(&output()).unwrap();
        raw["segments"][0]["text"] = "[S1] A B C는 수정하지 않습니다. 10개.".into();
        serde_json::to_vec(&raw).unwrap()
    }

    #[test]
    fn terminology_persists_profile_decisions_and_source_before_current_switch() {
        let (temp, mut engine, id, _) = setup();
        let raw = terminology_raw();
        let source = engine.commit_transcript(&id, None, raw.clone()).unwrap();
        let directory = engine
            .store
            .root
            .join(transcript_dir(&id, &source.version.id).unwrap());
        let initial = std::fs::read(directory.join("initial.json")).unwrap();
        let corrected = engine
            .commit_terminology(&id, terminology_draft(source.clone()))
            .unwrap();
        assert_eq!(corrected.version.revision, 1);
        assert_eq!(corrected.segments[0].text, "ABC는 수정하지 않습니다. 10개.");
        let mut unchanged_fields = corrected.clone();
        unchanged_fields.version = source.version.clone();
        unchanged_fields.segments[0].text = source.segments[0].text.clone();
        assert_eq!(
            serde_json::to_value(unchanged_fields).unwrap(),
            serde_json::to_value(&source).unwrap()
        );
        assert_eq!(std::fs::read(directory.join("raw.json")).unwrap(), raw);
        assert_eq!(
            std::fs::read(directory.join("initial.json")).unwrap(),
            initial
        );
        let receipts: Vec<_> = std::fs::read_dir(directory.join("terminology"))
            .unwrap()
            .collect();
        assert_eq!(receipts.len(), 1);
        let receipt: serde_json::Value =
            store::read_json(&receipts[0].as_ref().unwrap().path()).unwrap();
        assert_eq!(
            receipt["beforeVersion"],
            serde_json::to_value(&source.version).unwrap()
        );
        assert_eq!(
            receipt["afterVersion"],
            serde_json::to_value(&corrected.version).unwrap()
        );
        assert_eq!(
            receipt["provenance"]["sourceSha256"],
            crate::terminology::transcript_digest(&source).unwrap()
        );
        assert_eq!(
            receipt["provenance"]["source"],
            serde_json::to_value(&source).unwrap()
        );
        assert_eq!(receipt["provenance"]["profile"]["id"], "synthetic");
        assert_eq!(receipt["provenance"]["decisions"][0]["outcome"], "accept");
        let current = std::fs::read(directory.join("current.json")).unwrap();
        assert_eq!(receipt["afterCurrentSha256"], sha256(&current));
        assert_eq!(
            std::fs::read(directory.join(receipt["afterPath"].as_str().unwrap())).unwrap(),
            current
        );
        drop(engine);
        let engine = Engine::open(temp.path().canonicalize().unwrap().join("store")).unwrap();
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            corrected.version
        );
    }

    #[test]
    fn terminology_rejects_stale_version_and_same_version_changed_digest() {
        let (_temp, mut engine, id, _) = setup();
        let source = engine
            .commit_transcript(&id, None, terminology_raw())
            .unwrap();
        let mut altered = source.clone();
        altered.segments[0].original_text.push('!');
        altered.segments[0].text.push('!');
        assert!(engine
            .commit_terminology(&id, terminology_draft(altered))
            .is_err());
        let draft = terminology_draft(source.clone());
        let edited = engine
            .edit_transcript(&id, source.version, vec!["사용자 수정".into()])
            .unwrap();
        assert!(engine.commit_terminology(&id, draft).is_err());
        let current = engine.transcript(&id).unwrap().unwrap();
        assert_eq!(current.version, edited.version);
        assert_eq!(current.segments[0].text, "사용자 수정");
    }

    #[test]
    fn terminology_missing_acceptance_is_a_noop_without_revision_or_receipt() {
        let (_temp, mut engine, id, _) = setup();
        let source = engine
            .commit_transcript(&id, None, terminology_raw())
            .unwrap();
        let mut draft = terminology_draft(source.clone());
        draft.decisions.clear();
        let unchanged = engine.commit_terminology(&id, draft).unwrap();
        assert_eq!(unchanged.version, source.version);
        assert_eq!(unchanged.segments[0].text, source.segments[0].text);
        let directory = engine
            .store
            .root
            .join(transcript_dir(&id, &source.version.id).unwrap());
        assert!(!directory.join("terminology").exists());
        assert!(!std::fs::read_dir(directory).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("edit-")));
    }

    #[test]
    fn terminology_receipt_storage_failure_keeps_current_and_raw() {
        let (_temp, mut engine, id, _) = setup();
        let raw = terminology_raw();
        let source = engine.commit_transcript(&id, None, raw.clone()).unwrap();
        let directory = engine
            .store
            .root
            .join(transcript_dir(&id, &source.version.id).unwrap());
        let before = std::fs::read(directory.join("current.json")).unwrap();
        std::fs::write(directory.join("terminology"), b"blocked receipt directory").unwrap();
        assert!(engine
            .commit_terminology(&id, terminology_draft(source.clone()))
            .is_err());
        assert_eq!(
            std::fs::read(directory.join("current.json")).unwrap(),
            before
        );
        assert_eq!(std::fs::read(directory.join("raw.json")).unwrap(), raw);
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            source.version
        );
    }

    fn diarization_draft(source: Transcript) -> crate::SpeakerAnalysisDraft {
        crate::SpeakerAnalysisDraft {
            source,
            model: "example/diarizer".into(),
            model_revision: "revision-1".into(),
            model_sha256: "a".repeat(64),
            runtime: "example-runtime".into(),
            configuration: serde_json::json!({"scope":"recording"}),
            raw: br#"{"file":"input.wav","segments":[{"start":0.0,"end":1.0,"speaker":2}]}"#
                .to_vec(),
            posterior: None,
        }
    }

    #[test]
    fn diarization_preserves_audio_and_edited_text_with_immutable_run_evidence() {
        let (temp, mut engine, id, source_audio) = setup();
        let audio_bytes = std::fs::read(&source_audio).unwrap();
        let raw = output();
        let initial = engine.commit_transcript(&id, None, raw.clone()).unwrap();
        let directory = engine
            .store
            .root
            .join(transcript_dir(&id, &initial.version.id).unwrap());
        let initial_bytes = std::fs::read(directory.join("initial.json")).unwrap();
        let source = engine
            .edit_transcript(&id, initial.version, vec!["사용자 수정".into()])
            .unwrap();
        let draft = diarization_draft(source.clone());
        let activity_bytes = draft.raw.clone();
        let result = engine.apply_diarization(&id, draft).unwrap();
        assert_eq!(result.version.revision, source.version.revision + 1);
        assert_eq!(result.version.id, source.version.id);
        assert_eq!(result.source_session_id, source.source_session_id);
        assert_eq!(result.source, source.source);
        assert_eq!(result.segments[0].id, source.segments[0].id);
        assert_eq!(
            result.segments[0].source_index,
            source.segments[0].source_index
        );
        assert_eq!(
            (result.segments[0].start, result.segments[0].end),
            (source.segments[0].start, source.segments[0].end)
        );
        assert_eq!(result.segments[0].text, "사용자 수정");
        assert_eq!(result.segments[0].original_text, "원문");
        let analysis = result.speaker_analysis.as_ref().unwrap();
        assert_eq!(analysis.scope, "recording");
        assert!(!analysis.assignments_edited);
        let evidence_dir = directory.join("speaker-analysis").join(&analysis.run_id);
        assert_eq!(
            std::fs::read(evidence_dir.join("raw.json")).unwrap(),
            activity_bytes
        );
        let receipt: serde_json::Value = store::read_json(&evidence_dir.join("run.json")).unwrap();
        assert_eq!(
            receipt["beforeVersion"],
            serde_json::to_value(&source.version).unwrap()
        );
        assert_eq!(
            receipt["afterVersion"],
            serde_json::to_value(&result.version).unwrap()
        );
        assert_eq!(
            receipt["provenance"]["source"],
            serde_json::to_value(&source).unwrap()
        );
        assert_eq!(
            receipt["provenance"]["sourceSha256"],
            crate::summary::transcript_digest(&source).unwrap()
        );
        let edit_name = receipt["afterPath"].as_str().unwrap();
        assert_eq!(
            receipt["afterCurrentSha256"],
            sha256(&std::fs::read(directory.join(edit_name)).unwrap())
        );
        assert_eq!(
            std::fs::read(directory.join("initial.json")).unwrap(),
            initial_bytes
        );
        assert_eq!(std::fs::read(directory.join("raw.json")).unwrap(), raw);
        assert_eq!(std::fs::read(source_audio).unwrap(), audio_bytes);
        let mut names = result.speakers.clone();
        names[0].name = "참석자".into();
        let assignment = result.segments[0].speaker_id.clone();
        let edited = engine
            .edit_speakers(&id, result.version, names, vec![assignment])
            .unwrap();
        assert!(edited.speaker_analysis.as_ref().unwrap().assignments_edited);
        drop(engine);
        let reopened = Engine::open(temp.path().canonicalize().unwrap().join("store")).unwrap();
        let current = reopened.transcript(&id).unwrap().unwrap();
        assert_eq!(current.version, edited.version);
        assert_eq!(current.speakers[0].name, "참석자");
        assert_eq!(current.segments[0].text, "사용자 수정");
        assert_eq!(
            std::fs::read(evidence_dir.join("raw.json")).unwrap(),
            activity_bytes
        );
    }

    #[test]
    fn diarization_source_digest_and_version_reject_concurrent_or_forged_source() {
        let (_temp, mut engine, id, _) = setup();
        let source = engine.commit_transcript(&id, None, output()).unwrap();
        let mut forged = diarization_draft(source.clone());
        forged.source.segments[0].text = "수정 위조".into();
        assert!(engine.apply_diarization(&id, forged).is_err());
        let pending = diarization_draft(source.clone());
        let edited = engine
            .edit_transcript(&id, source.version, vec!["사용자 수정".into()])
            .unwrap();
        assert!(engine.apply_diarization(&id, pending).is_err());
        let current = engine.transcript(&id).unwrap().unwrap();
        assert_eq!(current.version, edited.version);
        assert_eq!(current.segments[0].text, "사용자 수정");
        assert!(current.speaker_analysis.is_none());
    }

    #[test]
    fn diarization_evidence_storage_failure_never_switches_current() {
        let (_temp, mut engine, id, _) = setup();
        let raw = output();
        let source = engine.commit_transcript(&id, None, raw.clone()).unwrap();
        let directory = engine
            .store
            .root
            .join(transcript_dir(&id, &source.version.id).unwrap());
        let before = std::fs::read(directory.join("current.json")).unwrap();
        std::fs::write(
            directory.join("speaker-analysis"),
            b"blocked evidence directory",
        )
        .unwrap();
        assert!(engine
            .apply_diarization(&id, diarization_draft(source.clone()))
            .is_err());
        assert_eq!(
            std::fs::read(directory.join("current.json")).unwrap(),
            before
        );
        assert_eq!(std::fs::read(directory.join("raw.json")).unwrap(), raw);
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            source.version
        );
    }
    #[test]
    fn posterior_generation_preserves_edited_parent_and_roundtrips_history_restore() {
        let (_temp, mut engine, id, _audio) = setup();
        let raw = crate::nemotron::tests::raw();
        let baseline = engine
            .commit_nemotron_transcript(&id, None, raw.clone())
            .unwrap();
        let old_dir = transcript_dir(&id, &baseline.version.id).unwrap();
        let original_initial =
            read_bytes(&engine.store.root, &old_dir.join("initial.json")).unwrap();
        let mut texts = baseline
            .segments
            .iter()
            .map(|s| s.text.clone())
            .collect::<Vec<_>>();
        texts[0] = "사용자가 수정한 문장".into();
        let edited = engine
            .edit_transcript(&id, baseline.version, texts)
            .unwrap();
        let edited_version = edited.version.clone();
        let after = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(edited))
            .unwrap();
        assert_ne!(after.version.id, edited_version.id);
        assert_eq!(after.version.revision, 1);
        assert_eq!(after.segments[0].text, "사용자가 수정한 문장");
        assert_eq!(after.segments[0].original_text, "첫 문장.");
        let new_dir = transcript_dir(&id, &after.version.id).unwrap();
        let initial: Transcript = serde_json::from_slice(
            &read_bytes(&engine.store.root, &new_dir.join("initial.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(initial.segments[0].text, "첫 문장.");
        assert_eq!(
            read_bytes(&engine.store.root, &old_dir.join("raw.json")).unwrap(),
            raw
        );
        assert_eq!(
            read_bytes(&engine.store.root, &old_dir.join("initial.json")).unwrap(),
            original_initial
        );
        assert_eq!(engine.transcript_history(&id).unwrap().entries.len(), 2);
        let restored = engine
            .restore_transcript(&id, after.version.clone(), edited_version)
            .unwrap();
        assert_eq!(restored.segments[0].text, "사용자가 수정한 문장");
        let back = engine
            .restore_transcript(&id, restored.version, after.version)
            .unwrap();
        assert_eq!(back.segments.len(), after.segments.len());
        assert_eq!(back.segments[0].text, "사용자가 수정한 문장");
        assert_eq!(
            engine.summary_input(&id).unwrap().source.version,
            back.version
        );
    }

    #[test]
    fn posterior_rerun_is_a_new_generation_with_canonical_words_and_no_old_mutation() {
        let (_temp, mut engine, id, _audio) = setup();
        let raw = crate::nemotron::tests::raw();
        let baseline = engine
            .commit_nemotron_transcript(&id, None, raw.clone())
            .unwrap();
        let first = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(baseline))
            .unwrap();
        let first_dir = transcript_dir(&id, &first.version.id).unwrap();
        let first_raw = read_bytes(&engine.store.root, &first_dir.join("raw.json")).unwrap();
        let first_current =
            read_bytes(&engine.store.root, &first_dir.join("current.json")).unwrap();
        let second = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(first.clone()))
            .unwrap();
        assert_ne!(second.version.id, first.version.id);
        assert_eq!(
            second
                .segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            first
                .segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            read_bytes(&engine.store.root, &first_dir.join("raw.json")).unwrap(),
            first_raw
        );
        assert_eq!(
            read_bytes(&engine.store.root, &first_dir.join("current.json")).unwrap(),
            first_current
        );
        let raw2 = read_bytes(
            &engine.store.root,
            &transcript_dir(&id, &second.version.id)
                .unwrap()
                .join("raw.json"),
        )
        .unwrap();
        assert_eq!(crate::nemotron::canonical_asr(&raw2).unwrap(), raw);
        assert_eq!(engine.transcript_history(&id).unwrap().entries.len(), 3);
    }

    #[test]
    fn invalid_posterior_or_missing_evidence_never_replaces_the_active_generation() {
        let (_temp, mut engine, id, _audio) = setup();
        let raw = crate::nemotron::tests::raw();
        let baseline = engine.commit_nemotron_transcript(&id, None, raw).unwrap();
        let mut bad = crate::nemotron::tests::posterior_draft(baseline.clone());
        bad.posterior.as_mut().unwrap().frames = 100;
        assert!(engine.apply_diarization(&id, bad).is_err());
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            baseline.version
        );
        let after = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(baseline))
            .unwrap();
        let directory = transcript_dir(&id, &after.version.id).unwrap();
        let run = after.speaker_analysis.as_ref().unwrap().run_id.clone();
        let posterior = store::checked_path(
            &engine.store.root,
            &directory
                .join("speaker-analysis")
                .join(run)
                .join("posterior.f32le"),
        )
        .unwrap();
        let mut bytes = std::fs::read(&posterior).unwrap();
        bytes[0] ^= 1;
        std::fs::write(&posterior, bytes).unwrap();
        assert!(engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(after.clone()))
            .is_err());
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            after.version
        );
        assert_eq!(engine.transcript_history(&id).unwrap().unavailable, 1);
    }
    #[test]
    fn legacy_moss_reanalysis_validates_new_posterior_but_keeps_the_legacy_generation() {
        let (_temp, mut engine, id, _audio) = setup();
        let raw = output();
        let baseline = engine.commit_transcript(&id, None, raw.clone()).unwrap();
        let after = engine
            .apply_diarization(
                &id,
                crate::nemotron::tests::posterior_draft(baseline.clone()),
            )
            .unwrap();
        assert_eq!(after.version.id, baseline.version.id);
        assert_eq!(after.version.revision, baseline.version.revision + 1);
        assert_eq!(after.segments.len(), baseline.segments.len());
        assert_eq!(
            after.segments[0].original_text,
            baseline.segments[0].original_text
        );
        assert_eq!(after.source, baseline.source);
        let run = &after.speaker_analysis.as_ref().unwrap().run_id;
        assert!(store::checked_path(
            &engine.store.root,
            &transcript_dir(&id, &after.version.id)
                .unwrap()
                .join("speaker-analysis")
                .join(run)
                .join("posterior.f32le")
        )
        .unwrap()
        .exists());
        assert_eq!(
            read_bytes(
                &engine.store.root,
                &transcript_dir(&id, &after.version.id)
                    .unwrap()
                    .join("raw.json")
            )
            .unwrap(),
            raw
        );
        let mut bad = crate::nemotron::tests::posterior_draft(after.clone());
        bad.posterior.as_mut().unwrap().probabilities.pop();
        assert!(engine.apply_diarization(&id, bad).is_err());
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            after.version
        );
    }
    #[test]
    fn new_moss_posterior_preserves_user_names_and_manual_unassignment_through_reopen_restore() {
        let (_temp, mut engine, id, _audio) = setup();
        let root = engine.store.root.clone();
        let raw = output();
        let baseline = engine.commit_transcript(&id, None, raw.clone()).unwrap();
        let directory = transcript_dir(&id, &baseline.version.id).unwrap();
        let original_initial = read_bytes(&root, &directory.join("initial.json")).unwrap();
        let mut names = baseline.speakers.clone();
        names[0].name = "사용자가 정한 이름".into();
        let assignments = baseline
            .segments
            .iter()
            .map(|s| s.speaker_id.clone())
            .collect();
        let named = engine
            .edit_speakers(&id, baseline.version, names, assignments)
            .unwrap();
        let named = engine
            .edit_transcript(&id, named.version, vec!["사용자 수정문".into()])
            .unwrap();
        let first = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(named.clone()))
            .unwrap();
        assert_eq!(first.speakers, named.speakers);
        assert_eq!(first.segments[0].speaker_id, named.segments[0].speaker_id);
        assert_eq!(first.segments[0].text, "사용자 수정문");
        let unassigned = engine
            .edit_speakers(&id, first.version, first.speakers, vec![None])
            .unwrap();
        let after = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(unassigned))
            .unwrap();
        assert_eq!(after.segments[0].speaker_id, None);
        assert_eq!(after.speakers[0].name, "사용자가 정한 이름");
        assert!(after.speaker_analysis.as_ref().unwrap().assignments_edited);
        assert_eq!(after.version.id, named.version.id);
        assert_eq!(read_bytes(&root, &directory.join("raw.json")).unwrap(), raw);
        assert_eq!(
            read_bytes(&root, &directory.join("initial.json")).unwrap(),
            original_initial
        );
        drop(engine);
        let mut engine = Engine::open(root.clone()).unwrap();
        let reopened = engine.transcript(&id).unwrap().unwrap();
        assert_eq!(reopened.version, after.version);
        assert_eq!(reopened.speakers, after.speakers);
        assert_eq!(reopened.segments[0].speaker_id, None);
        let other = engine
            .commit_transcript(&id, Some(after.version.clone()), output())
            .unwrap();
        let restored = engine
            .restore_transcript(&id, other.version, after.version)
            .unwrap();
        assert_eq!(restored.speakers, reopened.speakers);
        assert_eq!(restored.segments[0].speaker_id, None);
        assert_eq!(restored.segments[0].text, "사용자 수정문");
        assert!(
            restored
                .speaker_analysis
                .as_ref()
                .unwrap()
                .assignments_edited
        );
        assert_eq!(engine.transcript_history(&id).unwrap().unavailable, 0);
    }

    #[test]
    fn asr_commit_rejects_a_derived_envelope_without_publishing_incomplete_posterior_history() {
        let (_temp, mut engine, id, _audio) = setup();
        let baseline = engine
            .commit_nemotron_transcript(&id, None, crate::nemotron::tests::raw())
            .unwrap();
        let derived = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(baseline))
            .unwrap();
        let directory = transcript_dir(&id, &derived.version.id).unwrap();
        let raw = read_bytes(&engine.store.root, &directory.join("raw.json")).unwrap();
        let before = read_bytes(&engine.store.root, &directory.join("current.json")).unwrap();
        assert!(engine
            .commit_nemotron_transcript(&id, Some(derived.version.clone()), raw.clone())
            .is_err());
        assert_eq!(
            engine.transcript(&id).unwrap().unwrap().version,
            derived.version
        );
        assert_eq!(
            read_bytes(&engine.store.root, &directory.join("raw.json")).unwrap(),
            raw
        );
        assert_eq!(
            read_bytes(&engine.store.root, &directory.join("current.json")).unwrap(),
            before
        );
        assert!(engine.history_document(&id, &derived.version.id).is_ok());
    }
    #[test]
    fn first_moss_posterior_preserves_manual_unassignment_without_a_name_edit() {
        let (_temp, mut engine, id, _audio) = setup();
        let raw = output();
        let initial = engine.commit_transcript(&id, None, raw.clone()).unwrap();
        let manual = engine
            .edit_speakers(&id, initial.version, initial.speakers.clone(), vec![None])
            .unwrap();
        assert!(manual.speaker_analysis.is_none());
        assert!(manual
            .speakers
            .iter()
            .all(|s| s.name_origin == Some(SpeakerNameOrigin::Automatic)));
        let digest = crate::summary::transcript_digest(&manual).unwrap();
        let after = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(manual.clone()))
            .unwrap();
        assert_eq!(after.version.id, manual.version.id);
        assert_eq!(after.speakers, manual.speakers);
        assert_eq!(after.segments[0].speaker_id, None);
        assert!(after.speaker_analysis.as_ref().unwrap().assignments_edited);
        let run = &after.speaker_analysis.as_ref().unwrap().run_id;
        let provenance: serde_json::Value = store::read_json(
            &store::checked_path(
                &engine.store.root,
                &transcript_dir(&id, &after.version.id)
                    .unwrap()
                    .join("speaker-analysis")
                    .join(run)
                    .join("run.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(provenance["provenance"]["sourceSha256"], digest);
        assert_eq!(
            provenance["provenance"]["assignmentPreservation"]["verifiedInitialAssignmentsDiffer"],
            true
        );
        assert_eq!(
            provenance["provenance"]["decisions"][0]["reason"],
            "preserved_source_assignment"
        );
        assert_eq!(
            read_bytes(
                &engine.store.root,
                &transcript_dir(&id, &after.version.id)
                    .unwrap()
                    .join("raw.json")
            )
            .unwrap(),
            raw
        );
    }

    #[test]
    fn first_moss_posterior_preserves_a_manual_existing_automatic_speaker_assignment() {
        let (_temp, mut engine, id, _audio) = setup();
        let mut value: serde_json::Value = serde_json::from_slice(&output()).unwrap();
        value["segments"] = serde_json::json!([
            {"start":0.0,"end":0.4,"text":"[S1] 첫 발언","speaker_id":"S1"},
            {"start":0.5,"end":1.0,"text":"[S2] 둘째 발언","speaker_id":"S2"}
        ]);
        let initial = engine
            .commit_transcript(&id, None, serde_json::to_vec(&value).unwrap())
            .unwrap();
        let target = initial.segments[1].speaker_id.clone();
        let manual = engine
            .edit_speakers(
                &id,
                initial.version,
                initial.speakers.clone(),
                vec![target.clone(), target],
            )
            .unwrap();
        assert!(manual.speaker_analysis.is_none());
        let after = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(manual.clone()))
            .unwrap();
        assert_eq!(after.speakers, manual.speakers);
        assert!(after
            .segments
            .iter()
            .map(|s| &s.speaker_id)
            .eq(manual.segments.iter().map(|s| &s.speaker_id)));
        assert!(after.speaker_analysis.as_ref().unwrap().assignments_edited);
    }

    #[test]
    fn previous_automatic_moss_analysis_is_not_mistaken_for_a_manual_initial_difference() {
        let (_temp, mut engine, id, _audio) = setup();
        let initial = engine.commit_transcript(&id, None, output()).unwrap();
        let first = engine
            .apply_diarization(&id, crate::nemotron::tests::posterior_draft(initial))
            .unwrap();
        assert_eq!(first.segments[0].speaker_id, None);
        assert!(!first.speaker_analysis.as_ref().unwrap().assignments_edited);
        let mut next = crate::nemotron::tests::posterior_draft(first);
        let p = next.posterior.as_mut().unwrap();
        for (index, b) in p.probabilities.chunks_exact_mut(4).enumerate() {
            let value = if index % 8 == 0 { 0.9_f32 } else { 0.0_f32 };
            b.copy_from_slice(&value.to_le_bytes());
        }
        next.raw = br#"{"segments":[{"start":0.0,"end":1.0,"speaker":1}]}"#.to_vec();
        let second = engine.apply_diarization(&id, next).unwrap();
        assert!(second.segments[0].speaker_id.is_some());
        assert!(!second.speaker_analysis.as_ref().unwrap().assignments_edited);
    }
}
