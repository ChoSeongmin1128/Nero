use crate::{
    CoreError, Result, Transcript, TranscriptSegment, TranscriptSource, TranscriptVersion,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub(crate) const PROJECTOR_VERSION: &str = "nemotron-asr-sentences-v1";
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
    #[serde(default)]
    stream_frames_fed: Option<u64>,
    #[serde(default)]
    native_stream_finished: bool,
    #[serde(default)]
    input_pcm_sha256: Option<String>,
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
    if raw.len() > 16 * 1024 * 1024 {
        return Err(derived_invalid());
    }
    let value: serde_json::Value = serde_json::from_slice(raw).map_err(|_| derived_invalid())?;
    if value["schemaVersion"] == 2 {
        return decode_derived(raw, duration, session_id, id);
    }
    decode_asr(raw, duration, session_id, id)
}

pub(crate) fn decode_asr(
    raw: &[u8],
    duration: f64,
    session_id: &str,
    id: &str,
) -> Result<Transcript> {
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
        || !crate::speech_runtime::known(&output.runtime)
        || !((output.processing_policy == POLICY
            && output.completion_evidence == "native-stream-finish-exit0-v1")
            || (output.processing_policy == "whole-recording-stateful-live-v1"
                && output.completion_evidence == "native-stream-finish-cabi-v1"
                && output.stream_frames_fed == Some(output.input_audio_frames)
                && output.native_stream_finished
                && output.input_pcm_sha256.as_ref().is_some_and(|sha| {
                    sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit())
                })))
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
            chunking_policy: Some(output.processing_policy.clone()),
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

pub(crate) const WORD_PROJECTOR_VERSION: &str = "nemotron-word-speakers-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WordAtom {
    id: String,
    parent_index: usize,
    word_start: usize,
    word_end: usize,
    speaker_id: Option<String>,
    preserved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PosteriorSource {
    input_frames: u64,
    input_sample_rate: u32,
    prepared_frames: u64,
    frames: u64,
    speakers: u32,
    seconds_per_frame: f64,
    bytes: u64,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DerivedOutput {
    schema_version: u32,
    projector_version: String,
    canonical_asr_raw: String,
    canonical_asr_sha256: String,
    parent: Transcript,
    parent_sha256: String,
    configuration: serde_json::Value,
    analysis: crate::SpeakerAnalysis,
    posterior: PosteriorSource,
    word_slots: Vec<u32>,
    atoms: Vec<WordAtom>,
}

pub(crate) struct SpeakerTurnGeneration {
    pub raw: Vec<u8>,
    pub initial: Transcript,
    pub current: Transcript,
    pub evidence: crate::diarization::SpeakerAnalysisEvidence,
}

fn derived_invalid() -> CoreError {
    CoreError::InvalidInput(
        "화자 분석과 원본 단어의 연결을 확인할 수 없습니다. 기존 기록은 유지됩니다.".into(),
    )
}

fn hash(raw: &[u8]) -> String {
    format!("{:x}", Sha256::digest(raw))
}

pub(crate) fn canonical_asr(raw: &[u8]) -> Result<Vec<u8>> {
    if raw.len() > 16 * 1024 * 1024 {
        return Err(derived_invalid());
    }
    let value: serde_json::Value = serde_json::from_slice(raw).map_err(|_| derived_invalid())?;
    if value["schemaVersion"] == 1 {
        return Ok(raw.to_vec());
    }
    let output: DerivedOutput = serde_json::from_value(value).map_err(|_| derived_invalid())?;
    if output.schema_version != 2
        || output.projector_version != WORD_PROJECTOR_VERSION
        || hash(output.canonical_asr_raw.as_bytes()) != output.canonical_asr_sha256
    {
        return Err(derived_invalid());
    }
    let inner: serde_json::Value =
        serde_json::from_str(&output.canonical_asr_raw).map_err(|_| derived_invalid())?;
    if inner["schemaVersion"] != 1 {
        return Err(derived_invalid());
    }
    Ok(output.canonical_asr_raw.into_bytes())
}

fn word_ranges(parent: &Transcript, words: &[Word], duration: f64) -> Result<Vec<(usize, usize)>> {
    let mut cursor = 0;
    let mut ranges = Vec::new();
    for segment in &parent.segments {
        let start = cursor;
        let mut text = String::new();
        while cursor < words.len() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&words[cursor].word);
            cursor += 1;
            if text == segment.original_text {
                break;
            }
            if !segment.original_text.starts_with(&text) {
                return Err(derived_invalid());
            }
        }
        if cursor == start
            || text != segment.original_text
            || (segment.start * 16000.0).round()
                != (words[start].start.min(duration) * 16000.0).round()
            || (segment.end * 16000.0).round()
                != (words[cursor - 1].end.min(duration) * 16000.0).round()
        {
            return Err(derived_invalid());
        }
        ranges.push((start, cursor));
    }
    if cursor != words.len() {
        return Err(derived_invalid());
    }
    Ok(ranges)
}

fn preserve_atom(parent: &Transcript, index: usize) -> bool {
    let segment = &parent.segments[index];
    segment.text != segment.original_text
        || parent
            .speaker_analysis
            .as_ref()
            .is_some_and(|a| a.assignments_edited)
        || segment.speaker_id.as_ref().is_some_and(|id| {
            parent
                .speakers
                .iter()
                .any(|s| &s.id == id && s.name_origin != Some(crate::SpeakerNameOrigin::Automatic))
        })
}

fn atoms(
    parent: &Transcript,
    words: &[Word],
    slots: &[u32],
    duration: f64,
    run: &str,
) -> Result<Vec<WordAtom>> {
    let ranges = word_ranges(parent, words, duration)?;
    let mut result = Vec::new();
    for (parent_index, (start, end)) in ranges.into_iter().enumerate() {
        if preserve_atom(parent, parent_index) {
            result.push(WordAtom {
                id: String::new(),
                parent_index,
                word_start: start,
                word_end: end,
                speaker_id: parent.segments[parent_index].speaker_id.clone(),
                preserved: true,
            });
            continue;
        }
        let mut begin = start;
        while begin < end {
            let mut stop = begin + 1;
            while stop < end && slots[stop] == slots[begin] {
                stop += 1;
            }
            result.push(WordAtom {
                id: String::new(),
                parent_index,
                word_start: begin,
                word_end: stop,
                speaker_id: Some(format!("diarization:{run}:{}", slots[begin])),
                preserved: false,
            });
            begin = stop;
        }
    }
    Ok(result)
}

fn materialize(
    output: &DerivedOutput,
    words: &[Word],
    duration: f64,
    session: &str,
    id: &str,
) -> Result<Transcript> {
    if output.schema_version != 2
        || output.projector_version != WORD_PROJECTOR_VERSION
        || output.parent.source_session_id != session
        || output.parent_sha256 != crate::summary::transcript_digest(&output.parent)?
        || hash(output.canonical_asr_raw.as_bytes()) != output.canonical_asr_sha256
        || output.word_slots.len() != words.len()
        || output.word_slots.iter().any(|s| !(1..=8).contains(s))
        || output.posterior.speakers != 8
        || output.posterior.seconds_per_frame != f64::from(0.01_f32)
        || output.posterior.bytes
            != output
                .posterior
                .frames
                .checked_mul(32)
                .ok_or_else(derived_invalid)?
        || output.posterior.bytes > 128 * 1024 * 1024
        || output.posterior.sha256.len() != 64
        || !output
            .posterior
            .sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
        || output.analysis.projection_version != crate::diarization::WORD_PROJECTION_VERSION
        || output.analysis.scope != "recording"
        || output.analysis.run_id.is_empty()
        || output
            .analysis
            .run_id
            .contains(|c: char| !c.is_ascii_alphanumeric() && c != '-')
    {
        return Err(derived_invalid());
    }
    let profile = crate::SpeakerAnalysisDraft {
        source: output.parent.clone(),
        model: output.analysis.model.clone(),
        model_revision: output.analysis.model_revision.clone(),
        model_sha256: output.analysis.model_sha256.clone(),
        runtime: output.analysis.runtime.clone(),
        configuration: output.configuration.clone(),
        raw: Vec::new(),
        posterior: None,
    };
    if !crate::diarization::posterior_profile(&profile) {
        return Err(derived_invalid());
    }
    if output.posterior.input_frames == 0
        || output.posterior.input_sample_rate == 0
        || duration
            != output.posterior.input_frames as f64 / f64::from(output.posterior.input_sample_rate)
    {
        return Err(derived_invalid());
    }
    let prepared = (u128::from(output.posterior.input_frames) * 16_000
        + u128::from(output.posterior.input_sample_rate / 2))
        / u128::from(output.posterior.input_sample_rate);
    if prepared != u128::from(output.posterior.prepared_frames)
        || output.posterior.frames != output.posterior.prepared_frames / 160 + 1
    {
        return Err(derived_invalid());
    }
    let expected = atoms(
        &output.parent,
        words,
        &output.word_slots,
        duration,
        &output.analysis.run_id,
    )?;
    if expected.len() != output.atoms.len() {
        return Err(derived_invalid());
    }
    let mut ids = BTreeSet::new();
    let mut speakers = BTreeMap::new();
    let mut segments = Vec::new();
    let mut causes = BTreeMap::new();
    for (index, (atom, expected)) in output.atoms.iter().zip(expected).enumerate() {
        if atom.id.is_empty()
            || atom.id.len() > 128
            || !atom
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || !ids.insert(atom.id.clone())
            || atom.parent_index != expected.parent_index
            || atom.word_start != expected.word_start
            || atom.word_end != expected.word_end
            || atom.speaker_id != expected.speaker_id
            || atom.preserved != expected.preserved
        {
            return Err(derived_invalid());
        }
        let parent = &output.parent.segments[atom.parent_index];
        let text = words[atom.word_start..atom.word_end]
            .iter()
            .map(|w| w.word.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(speaker) = &atom.speaker_id {
            let name = if atom.preserved {
                output
                    .parent
                    .speakers
                    .iter()
                    .find(|s| &s.id == speaker)
                    .cloned()
                    .ok_or_else(derived_invalid)?
            } else {
                crate::TranscriptSpeaker {
                    id: speaker.clone(),
                    name: format!("화자 {}", output.word_slots[atom.word_start]),
                    name_origin: Some(crate::SpeakerNameOrigin::Automatic),
                }
            };
            speakers.insert(speaker.clone(), name);
        } else {
            *causes
                .entry(if atom.preserved {
                    "preserved_parent_assignment".into()
                } else {
                    "no_output".into()
                })
                .or_insert(0) += 1;
        }
        segments.push(TranscriptSegment {
            id: atom.id.clone(),
            start: if atom.preserved {
                parent.start
            } else {
                words[atom.word_start].start.min(duration)
            },
            end: if atom.preserved {
                parent.end
            } else {
                words[atom.word_end - 1].end.min(duration)
            },
            original_text: text.clone(),
            text,
            speaker_id: atom.speaker_id.clone(),
            source_index: Some(u32::try_from(index).map_err(|_| derived_invalid())?),
        });
    }
    let assigned = segments.iter().filter(|s| s.speaker_id.is_some()).count();
    if output.analysis.assigned_count != assigned
        || output.analysis.unassigned_count != segments.len() - assigned
        || output.analysis.review_causes != causes
    {
        return Err(derived_invalid());
    }
    Ok(Transcript {
        version: TranscriptVersion {
            id: id.into(),
            revision: 0,
        },
        source_session_id: session.into(),
        model: output.parent.model.clone(),
        model_revision: output.parent.model_revision.clone(),
        source: None,
        speakers: speakers.into_values().collect(),
        segments,
        speaker_analysis: Some(output.analysis.clone()),
    })
}

fn decode_derived(raw: &[u8], duration: f64, session: &str, id: &str) -> Result<Transcript> {
    if raw.len() > 16 * 1024 * 1024 {
        return Err(derived_invalid());
    }
    let output: DerivedOutput = serde_json::from_slice(raw).map_err(|_| derived_invalid())?;
    let canonical = decode_asr(output.canonical_asr_raw.as_bytes(), duration, session, id)?;
    if canonical.model != output.parent.model
        || canonical.model_revision != output.parent.model_revision
    {
        return Err(derived_invalid());
    }
    let words: Output =
        serde_json::from_str(&output.canonical_asr_raw).map_err(|_| derived_invalid())?;
    if words.input_audio_frames != output.posterior.prepared_frames {
        return Err(derived_invalid());
    }
    let mut transcript = materialize(&output, &words.words, duration, session, id)?;
    transcript.source = Some(TranscriptSource {
        raw_sha256: hash(raw),
        projector_version: WORD_PROJECTOR_VERSION.into(),
        association: "direct".into(),
        speaker_scope: Some("recording".into()),
        chunking_policy: Some(words.processing_policy),
    });
    Ok(transcript)
}

pub(crate) fn posterior_source(raw: &[u8]) -> Result<(String, u64, String)> {
    let output: DerivedOutput = serde_json::from_slice(raw).map_err(|_| derived_invalid())?;
    Ok((
        output.analysis.run_id,
        output.posterior.bytes,
        output.posterior.sha256,
    ))
}

pub(crate) fn direct_parent_citation_targets(
    raw: &[u8],
    current: &Transcript,
    source: &Transcript,
) -> Result<Option<BTreeMap<String, Vec<String>>>> {
    if raw.len() > 16 * 1024 * 1024
        || current.source.as_ref().is_none_or(|identity| {
            identity.projector_version != WORD_PROJECTOR_VERSION
                || identity.association != "direct"
                || identity.raw_sha256 != hash(raw)
        })
    {
        return Err(derived_invalid());
    }
    let output: DerivedOutput = serde_json::from_slice(raw).map_err(|_| derived_invalid())?;
    if output.posterior.input_sample_rate == 0 {
        return Err(derived_invalid());
    }
    let duration =
        output.posterior.input_frames as f64 / f64::from(output.posterior.input_sample_rate);
    let replay = decode_derived(
        raw,
        duration,
        &current.source_session_id,
        &current.version.id,
    )?;
    if replay.source != current.source
        || replay.model != current.model
        || replay.model_revision != current.model_revision
        || replay.segments.len() != current.segments.len()
        || replay
            .segments
            .iter()
            .zip(&current.segments)
            .any(|(expected, actual)| {
                expected.id != actual.id
                    || expected.source_index != actual.source_index
                    || expected.start != actual.start
                    || expected.end != actual.end
                    || expected.original_text != actual.original_text
            })
    {
        return Err(derived_invalid());
    }
    if output.parent.version != source.version
        || output.parent_sha256 != crate::summary::transcript_digest(source)?
        || serde_json::to_value(&output.parent).map_err(|_| derived_invalid())?
            != serde_json::to_value(source).map_err(|_| derived_invalid())?
    {
        return Ok(None);
    }
    let mut targets: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (atom, segment) in output.atoms.iter().zip(&current.segments) {
        targets
            .entry(source.segments[atom.parent_index].id.clone())
            .or_default()
            .push(segment.id.clone());
    }
    Ok(Some(targets))
}

pub(crate) fn create_speaker_generation(
    draft: crate::SpeakerAnalysisDraft,
    parent_raw: &[u8],
    frames: u64,
    sample_rate: u32,
    id: &str,
) -> Result<SpeakerTurnGeneration> {
    if draft.source.source.as_ref().is_none_or(|s| {
        s.raw_sha256 != hash(parent_raw)
            || !matches!(
                s.projector_version.as_str(),
                PROJECTOR_VERSION | WORD_PROJECTOR_VERSION
            )
    }) {
        return Err(derived_invalid());
    }
    let duration = frames as f64 / f64::from(sample_rate);
    let canonical = canonical_asr(parent_raw)?;
    let baseline = decode_asr(&canonical, duration, &draft.source.source_session_id, id)?;
    if baseline.model != draft.source.model
        || baseline.model_revision != draft.source.model_revision
    {
        return Err(derived_invalid());
    }
    let parsed: Output = serde_json::from_slice(&canonical).map_err(|_| derived_invalid())?;
    let prepared = (u128::from(frames) * 16_000 + u128::from(sample_rate / 2))
        / u128::from(sample_rate.max(1));
    if sample_rate == 0 || prepared != u128::from(parsed.input_audio_frames) {
        return Err(derived_invalid());
    }
    let positions = parsed
        .words
        .iter()
        .map(|w| (w.start, w.end))
        .collect::<Vec<_>>();
    let slots = crate::diarization::posterior_slots(&draft, &positions, frames, sample_rate)?;
    let run = Uuid::new_v4().to_string();
    let mut plans = atoms(&draft.source, &parsed.words, &slots, duration, &run)?;
    for atom in &mut plans {
        atom.id = Uuid::new_v4().to_string();
    }
    let assigned = plans.iter().filter(|a| a.speaker_id.is_some()).count();
    let mut causes = BTreeMap::new();
    for atom in plans.iter().filter(|a| a.speaker_id.is_none()) {
        *causes
            .entry(if atom.preserved {
                "preserved_parent_assignment".into()
            } else {
                "no_output".into()
            })
            .or_insert(0) += 1;
    }
    let posterior = draft.posterior.as_ref().ok_or_else(derived_invalid)?;
    let output = DerivedOutput {
        schema_version: 2,
        projector_version: WORD_PROJECTOR_VERSION.into(),
        canonical_asr_raw: String::from_utf8(canonical).map_err(|_| derived_invalid())?,
        canonical_asr_sha256: baseline.source.as_ref().unwrap().raw_sha256.clone(),
        parent: draft.source.clone(),
        parent_sha256: crate::summary::transcript_digest(&draft.source)?,
        configuration: draft.configuration.clone(),
        analysis: crate::SpeakerAnalysis {
            run_id: run.clone(),
            scope: "recording".into(),
            model: draft.model.clone(),
            model_revision: draft.model_revision.clone(),
            model_sha256: draft.model_sha256.clone(),
            runtime: draft.runtime.clone(),
            projection_version: crate::diarization::WORD_PROJECTION_VERSION.into(),
            raw_sha256: hash(&draft.raw),
            assigned_count: assigned,
            unassigned_count: plans.len() - assigned,
            review_causes: causes,
            assignments_edited: draft
                .source
                .speaker_analysis
                .as_ref()
                .is_some_and(|a| a.assignments_edited),
        },
        posterior: PosteriorSource {
            input_frames: frames,
            input_sample_rate: sample_rate,
            prepared_frames: parsed.input_audio_frames,
            frames: posterior.frames,
            speakers: posterior.speakers,
            seconds_per_frame: posterior.seconds_per_frame,
            bytes: posterior.probabilities.len() as u64,
            sha256: hash(&posterior.probabilities),
        },
        word_slots: slots,
        atoms: plans,
    };
    let raw = serde_json::to_vec(&output).map_err(|_| derived_invalid())?;
    let initial = decode_derived(&raw, duration, &draft.source.source_session_id, id)?;
    let mut current = initial.clone();
    for (segment, atom) in current.segments.iter_mut().zip(&output.atoms) {
        if atom.preserved {
            segment.text = output.parent.segments[atom.parent_index].text.clone();
        }
    }
    if current.segments.iter().any(|s| s.text != s.original_text) {
        current.version.revision = 1;
    }
    let document = serde_json::json!({"schemaVersion":2,"analysis":output.analysis,"source":draft.source,"sourceSha256":output.parent_sha256,
        "configuration":output.configuration,"posterior":{"path":"posterior.f32le","frames":output.posterior.frames,"speakers":output.posterior.speakers,"secondsPerFrame":output.posterior.seconds_per_frame,"bytes":output.posterior.bytes,"sha256":output.posterior.sha256},
        "activityNormalization":{"rule":"nemo-v3-probability-frame-boundaries-v1","inputFrames":frames,"inputSampleRate":sample_rate,
            "preparedFrames":output.posterior.prepared_frames,"preparedSampleRate":16000,"nativeFrames":output.posterior.frames,
            "secondsPerFrame":output.posterior.seconds_per_frame,"terminalEndClippedToPreparedDuration":true},
        "projection":{"projectorVersion":WORD_PROJECTOR_VERSION,"assignment":crate::diarization::WORD_PROJECTION_VERSION,"wordSpans":output.atoms},"rawBytes":draft.raw.len(),"rawPath":"raw.json"});
    Ok(SpeakerTurnGeneration {
        raw,
        initial,
        current,
        evidence: crate::diarization::SpeakerAnalysisEvidence {
            run_id: run,
            raw: draft.raw,
            document,
            posterior: Some(draft.posterior.unwrap().probabilities),
        },
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::speech_runtime::current as runtime;
    use crate::transcript::tests::{output as moss_output, setup};

    fn value() -> serde_json::Value {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../config/models/nemotron-asr.json")).unwrap();
        serde_json::json!({
            "schemaVersion":1,
            "model":manifest["repository"],"modelRevision":manifest["revision"],
            "runtime":runtime(),"processingPolicy":POLICY,
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

    pub(crate) fn raw() -> Vec<u8> {
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
    fn historical_base_output_remains_readable_with_its_own_raw_identity() {
        let spec: serde_json::Value = serde_json::from_str(include_str!(
            "../../../scripts/native-diarization/runtime.json"
        ))
        .unwrap();
        let mut historical = value();
        historical["runtime"] = serde_json::json!(format!(
            "nemo-speech-{}@{}",
            spec["version"].as_str().unwrap().trim_start_matches('v'),
            spec["commit"].as_str().unwrap()
        ));
        let bytes = serde_json::to_vec(&historical).unwrap();
        let source = project(historical).unwrap().source.unwrap();
        assert_eq!(source.raw_sha256, format!("{:x}", Sha256::digest(bytes)));
        let patched = project(value()).unwrap();
        assert_eq!(patched.segments.len(), 3);
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert_ne!(source.raw_sha256, patched.source.unwrap().raw_sha256);
        }
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
    fn live_completion_requires_exact_fed_frames_finish_and_pcm_identity() {
        let mut live = value();
        live["processingPolicy"] = serde_json::json!("whole-recording-stateful-live-v1");
        live["completionEvidence"] = serde_json::json!("native-stream-finish-cabi-v1");
        live["streamFramesFed"] = serde_json::json!(16000);
        live["nativeStreamFinished"] = serde_json::json!(true);
        live["inputPcmSha256"] = serde_json::json!("a".repeat(64));
        assert_eq!(
            project(live.clone())
                .unwrap()
                .source
                .unwrap()
                .chunking_policy
                .as_deref(),
            Some("whole-recording-stateful-live-v1")
        );
        for (field, bad) in [
            ("streamFramesFed", serde_json::json!(15999)),
            ("nativeStreamFinished", serde_json::json!(false)),
            ("inputPcmSha256", serde_json::json!("unknown")),
            ("nativeDurationSeconds", serde_json::json!(0.5)),
            (
                "completionEvidence",
                serde_json::json!("native-stream-finish-exit0-v1"),
            ),
        ] {
            let mut candidate = live.clone();
            candidate[field] = bad;
            assert!(project(candidate).is_err(), "accepted {field}");
        }
        for field in ["streamFramesFed", "nativeStreamFinished", "inputPcmSha256"] {
            let mut candidate = live.clone();
            candidate.as_object_mut().unwrap().remove(field);
            assert!(project(candidate).is_err());
        }
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
    pub(crate) fn posterior_draft(source: Transcript) -> crate::SpeakerAnalysisDraft {
        let stride = f64::from(0.01_f32);
        let mut probabilities = Vec::new();
        for frame in 0..101 {
            for slot in 0..8 {
                let probability: f32 = if slot == if frame < 20 { 0 } else { 1 } {
                    0.9
                } else {
                    0.0
                };
                probabilities.extend_from_slice(&probability.to_le_bytes());
            }
        }
        crate::SpeakerAnalysisDraft {
            source,
            model: "nvidia/Nemotron-3-Diarization".into(),
            model_revision: "f667ed73aee57d40cc39428eb768b4fd87a0a29e".into(),
            model_sha256: "08456d9e22cd9a323c0364d98375f3746d6e68507ebb705cd46438c534c7a3a1".into(),
            runtime: runtime().into(),
            configuration: serde_json::json!({"scope":"recording","fullAttention":false,"batching":false,"backend":"cpu",
                "postprocessingSource":"nvidia-v3-probability-v1","assignment":"nemo-word-160ms-argmax-v1",
                "geometry":{"spkcache":264,"fifo":40,"chunk":340,"updatePeriod":300,"leftContext":0,"rightContext":40}}),
            raw: serde_json::to_vec(&serde_json::json!({"file":"test.wav","segments":[
                {"start":0.0,"end":20.0*stride,"speaker":1},
                {"start":20.0*stride,"end":1.0,"speaker":2}
            ]}))
            .unwrap(),
            posterior: Some(crate::SpeakerPosterior {
                frames: 101,
                speakers: 8,
                seconds_per_frame: stride,
                probabilities,
            }),
        }
    }

    #[test]
    fn posterior_generation_splits_only_native_word_boundaries_and_replays_exactly() {
        let raw = raw();
        let parent = decode(&raw, 1.0, "session", "parent").unwrap();
        let generation = create_speaker_generation(
            posterior_draft(parent.clone()),
            &raw,
            16000,
            16000,
            "derived",
        )
        .unwrap();
        assert_eq!(generation.initial.segments.len(), 4);
        assert_eq!(
            generation
                .initial
                .segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            vec!["첫", "문장.", "둘째 문장?", "마지막"]
        );
        assert_eq!(
            generation
                .initial
                .segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            parent
                .segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert_eq!(canonical_asr(&generation.raw).unwrap(), raw);
        let replay = decode(&generation.raw, 1.0, "session", "derived").unwrap();
        assert_eq!(
            serde_json::to_value(replay).unwrap(),
            serde_json::to_value(&generation.initial).unwrap()
        );
        assert!(generation
            .current
            .segments
            .iter()
            .all(|s| s.speaker_id.is_some()));
        assert_eq!(generation.current.version.revision, 0);
        assert!(generation.evidence.posterior.is_some());
        assert_eq!(
            generation.evidence.document["activityNormalization"]["preparedFrames"],
            16000
        );
    }

    #[test]
    fn posterior_never_guesses_alignment_for_an_edited_parent_block() {
        let raw = raw();
        let mut parent = decode(&raw, 1.0, "session", "parent").unwrap();
        parent.segments[0].text = "사용자가 문장 전체를 고쳤습니다".into();
        parent.version.revision = 1;
        let generation = create_speaker_generation(
            posterior_draft(parent.clone()),
            &raw,
            16000,
            16000,
            "derived",
        )
        .unwrap();
        assert_eq!(generation.initial.segments.len(), 3);
        assert_eq!(generation.initial.segments[0].text, "첫 문장.");
        assert_eq!(generation.current.segments[0].text, parent.segments[0].text);
        assert_eq!(generation.current.segments[0].original_text, "첫 문장.");
        assert_eq!(generation.current.segments[0].speaker_id, None);
        assert_eq!(generation.current.version.revision, 1);
        assert_eq!(
            generation.current.segments[0].start,
            parent.segments[0].start
        );
        assert_eq!(generation.current.segments[0].end, parent.segments[0].end);
    }

    #[test]
    fn user_names_and_manual_assignments_keep_their_own_namespace_on_reanalysis() {
        let raw = raw();
        let mut parent = decode(&raw, 1.0, "session", "parent").unwrap();
        parent.speakers.push(crate::TranscriptSpeaker {
            id: "user-alice".into(),
            name: "직접 지정한 이름".into(),
            name_origin: Some(crate::SpeakerNameOrigin::User),
        });
        parent.segments[0].speaker_id = Some("user-alice".into());
        let first = create_speaker_generation(posterior_draft(parent), &raw, 16000, 16000, "first")
            .unwrap();
        assert_eq!(
            first.initial.segments[0].speaker_id.as_deref(),
            Some("user-alice")
        );
        assert_eq!(first.initial.segments[0].text, "첫 문장.");
        assert_eq!(
            first
                .initial
                .speakers
                .iter()
                .filter(|s| s.name == "직접 지정한 이름")
                .count(),
            1
        );
        let mut manual = first.current.clone();
        manual.speaker_analysis.as_mut().unwrap().assignments_edited = true;
        manual.segments[1].speaker_id = None;
        let second = create_speaker_generation(
            posterior_draft(manual.clone()),
            &first.raw,
            16000,
            16000,
            "second",
        )
        .unwrap();
        assert_eq!(second.current.segments.len(), manual.segments.len());
        assert!(second
            .current
            .segments
            .iter()
            .map(|s| &s.speaker_id)
            .eq(manual.segments.iter().map(|s| &s.speaker_id)));
        assert!(second.current.speaker_analysis.unwrap().assignments_edited);
        assert_eq!(canonical_asr(&second.raw).unwrap(), raw);
    }

    #[test]
    fn posterior_uses_the_onset_anchor_and_keeps_official_lower_slot_ties() {
        let raw = raw();
        let parent = decode(&raw, 1.0, "session", "parent").unwrap();
        let draft = posterior_draft(parent.clone());
        assert_eq!(
            crate::diarization::posterior_slots(&draft, &[(0., 1.), (0.15, 0.4)], 16000, 16000)
                .unwrap(),
            vec![1, 2]
        );
        let mut tied = posterior_draft(parent);
        for bytes in tied
            .posterior
            .as_mut()
            .unwrap()
            .probabilities
            .chunks_exact_mut(4)
        {
            bytes.copy_from_slice(&0.2_f32.to_le_bytes());
        }
        tied.raw = br#"{"segments":[]}"#.to_vec();
        assert_eq!(
            crate::diarization::posterior_slots(&tied, &[(0., 0.9), (1.0, 1.0)], 16000, 16000)
                .unwrap(),
            vec![1, 1]
        );
    }

    #[test]
    fn posterior_rejects_shape_nan_mismatched_activity_and_unpinned_config() {
        let raw = raw();
        let parent = decode(&raw, 1.0, "session", "parent").unwrap();
        let base = posterior_draft(parent);
        let mut variants = Vec::new();
        let mut bad = base.clone();
        bad.posterior.as_mut().unwrap().probabilities.pop();
        variants.push(bad);
        let mut bad = base.clone();
        bad.posterior.as_mut().unwrap().probabilities[..4].copy_from_slice(&f32::NAN.to_le_bytes());
        variants.push(bad);
        let mut bad = base.clone();
        bad.posterior.as_mut().unwrap().frames = 100;
        variants.push(bad);
        let mut bad = base.clone();
        bad.raw = br#"{"segments":[]}"#.to_vec();
        variants.push(bad);
        let mut bad = base.clone();
        bad.configuration["geometry"]["chunk"] = 264.into();
        variants.push(bad);
        let mut bad = base;
        bad.configuration["assignment"] = "guessed-rule".into();
        variants.push(bad);
        for bad in variants {
            assert!(create_speaker_generation(bad, &raw, 16000, 16000, "derived").is_err());
        }
    }

    #[test]
    fn derived_replay_rejects_changed_word_spans_slots_parent_and_source_geometry() {
        let raw = raw();
        let parent = decode(&raw, 1.0, "session", "parent").unwrap();
        let generation =
            create_speaker_generation(posterior_draft(parent), &raw, 16000, 16000, "derived")
                .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&generation.raw).unwrap();
        let mut variants = Vec::new();
        let mut bad = value.clone();
        bad["atoms"][0]["wordEnd"] = 2.into();
        variants.push(bad);
        let mut bad = value.clone();
        bad["wordSlots"][0] = 0.into();
        variants.push(bad);
        let mut bad = value.clone();
        bad["parent"]["segments"][0]["originalText"] = "가짜 원문".into();
        variants.push(bad);
        let mut bad = value;
        bad["posterior"]["preparedFrames"] = 16001.into();
        variants.push(bad);
        for bad in variants {
            assert!(decode(
                &serde_json::to_vec(&bad).unwrap(),
                1.0,
                "session",
                "derived"
            )
            .is_err());
        }
    }

    #[test]
    fn posterior_binds_prepared_frames_to_integer_resampling_without_widening_audio_bounds() {
        let raw = raw();
        let physical_duration = 48001.0 / 48000.0;
        let parent = decode(&raw, physical_duration, "session", "parent").unwrap();
        let generation =
            create_speaker_generation(posterior_draft(parent), &raw, 48001, 48000, "derived")
                .unwrap();
        assert_eq!(
            generation.evidence.document["activityNormalization"]["preparedFrames"],
            16000
        );
        assert!(decode(&generation.raw, physical_duration, "session", "derived").is_ok());
        assert!(decode(&generation.raw, 1.0, "session", "derived").is_err());
    }
}
