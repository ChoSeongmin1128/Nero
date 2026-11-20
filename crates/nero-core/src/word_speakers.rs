use crate::asr_provider::{decode_native, AsrProvider, Projection};
use crate::nemotron::Word;
use crate::{
    CoreError, Result, Transcript, TranscriptSegment, TranscriptSource, TranscriptVersion,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub(crate) const WORD_PROJECTOR_VERSION: &str = "nemotron-word-speakers-v1";
pub(crate) const WHISPER_WORD_PROJECTOR_VERSION: &str = "whisper-word-speakers-v1";
pub(crate) const QWEN_WORD_PROJECTOR_VERSION: &str = "qwen3-word-speakers-v1";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WordOutput {
    input_audio_frames: u64,
    processing_policy: String,
    words: Vec<Word>,
}

fn zero_length_unknown(model: &str) -> bool {
    AsrProvider::of_model(model).is_some_and(AsrProvider::zero_length_unknown)
}

// Unknown models take Nemotron's projector, which their replay then refuses.
fn word_projector(model: &str) -> &'static str {
    AsrProvider::of_model(model)
        .and_then(AsrProvider::word_projector)
        .unwrap_or(WORD_PROJECTOR_VERSION)
}

fn unassigned_cause(atom: &WordAtom, model: &str, words: &[Word]) -> String {
    if !atom.preserved
        && AsrProvider::of_model(model).is_some_and(AsrProvider::native_segment_context)
        && words[atom.word_start..atom.word_end]
            .iter()
            .any(Word::context_only)
    {
        "native_segment_context".into()
    } else if !atom.preserved
        && zero_length_unknown(model)
        && words[atom.word_start..atom.word_end]
            .iter()
            .all(|w| w.start == w.end)
    {
        "native_zero_duration".into()
    } else if atom.preserved {
        "preserved_parent_assignment".into()
    } else {
        "no_output".into()
    }
}

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

pub(crate) fn derived_invalid() -> CoreError {
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
        || AsrProvider::of_projector(&output.projector_version)
            .is_none_or(|(_, projection)| projection != Projection::WordSpeakers)
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
        // The words joined by spaces must spell the segment; `rest` is what they have not
        // spelled yet, so each word is compared once.
        let original = segment.original_text.as_str();
        let mut rest = original;
        while cursor < words.len() {
            if rest.len() != original.len() {
                rest = rest.strip_prefix(' ').ok_or_else(derived_invalid)?;
            }
            rest = rest
                .strip_prefix(words[cursor].word.as_str())
                .ok_or_else(derived_invalid)?;
            cursor += 1;
            if rest.is_empty() {
                break;
            }
        }
        if cursor == start
            || !rest.is_empty()
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
                speaker_id: (slots[begin] != 0)
                    .then(|| format!("diarization:{run}:{}", slots[begin])),
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
        || output.projector_version != word_projector(&output.parent.model)
        || output.parent.source_session_id != session
        || output.parent_sha256 != crate::summary::transcript_digest(&output.parent)?
        || hash(output.canonical_asr_raw.as_bytes()) != output.canonical_asr_sha256
        || output.word_slots.len() != words.len()
        || output.word_slots.iter().zip(words).any(|(slot, word)| {
            if zero_length_unknown(&output.parent.model)
                && (word.start == word.end || word.context_only())
            {
                *slot != 0
            } else {
                !(1..=8).contains(slot)
            }
        })
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
    if !crate::diarization::posterior_profile_of(
        &output.analysis.model,
        &output.analysis.model_revision,
        &output.analysis.model_sha256,
        &output.analysis.runtime,
        &output.configuration,
    ) {
        return Err(derived_invalid());
    }
    if output.posterior.input_frames == 0
        || output.posterior.input_sample_rate == 0
        || duration
            != output.posterior.input_frames as f64 / f64::from(output.posterior.input_sample_rate)
    {
        return Err(derived_invalid());
    }
    let prepared = crate::audio_normalize::output_frames(
        output.posterior.input_frames,
        output.posterior.input_sample_rate,
    )
    .map_err(|_| derived_invalid())?;
    if prepared != output.posterior.prepared_frames
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
                .entry(unassigned_cause(atom, &output.parent.model, words))
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

pub(crate) fn decode_derived(
    raw: &[u8],
    duration: f64,
    session: &str,
    id: &str,
) -> Result<Transcript> {
    if raw.len() > 16 * 1024 * 1024 {
        return Err(derived_invalid());
    }
    let output: DerivedOutput = serde_json::from_slice(raw).map_err(|_| derived_invalid())?;
    replay(&output, raw, duration, session, id)
}

// `output` is `raw` parsed; `raw` itself is hashed into the generation's identity.
fn replay(
    output: &DerivedOutput,
    raw: &[u8],
    duration: f64,
    session: &str,
    id: &str,
) -> Result<Transcript> {
    let canonical = decode_native(output.canonical_asr_raw.as_bytes(), duration, session, id)?;
    if canonical.model != output.parent.model
        || canonical.model_revision != output.parent.model_revision
    {
        return Err(derived_invalid());
    }
    let words: WordOutput =
        serde_json::from_str(&output.canonical_asr_raw).map_err(|_| derived_invalid())?;
    if words.input_audio_frames != output.posterior.prepared_frames {
        return Err(derived_invalid());
    }
    let mut transcript = materialize(output, &words.words, duration, session, id)?;
    transcript.source = Some(TranscriptSource {
        raw_sha256: hash(raw),
        projector_version: word_projector(&canonical.model).into(),
        association: "direct".into(),
        speaker_scope: Some("recording".into()),
        chunking_policy: Some(words.processing_policy),
        recognition_report: None,
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
            identity.projector_version != word_projector(&current.model)
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
    let replay = replay(
        &output,
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
    // The replay checked `parent_sha256` against the whole stored parent, so an equal
    // digest of `source` is the same transcript.
    if output.parent.version != source.version
        || output.parent_sha256 != crate::summary::transcript_digest(source)?
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
            || AsrProvider::of_projector(&s.projector_version)
                .is_none_or(|(provider, _)| !provider.word_speakers())
    }) {
        return Err(derived_invalid());
    }
    let duration = frames as f64 / f64::from(sample_rate);
    let canonical = canonical_asr(parent_raw)?;
    let baseline = decode_native(&canonical, duration, &draft.source.source_session_id, id)?;
    if baseline.model != draft.source.model
        || baseline.model_revision != draft.source.model_revision
    {
        return Err(derived_invalid());
    }
    let parsed: WordOutput = serde_json::from_slice(&canonical).map_err(|_| derived_invalid())?;
    if crate::audio_normalize::output_frames(frames, sample_rate).ok()
        != Some(parsed.input_audio_frames)
    {
        return Err(derived_invalid());
    }
    let positions = parsed
        .words
        .iter()
        .map(|w| (w.start, w.end))
        .collect::<Vec<_>>();
    let mut slots = crate::diarization::posterior_slots(&draft, &positions, frames, sample_rate)?;
    if zero_length_unknown(&baseline.model) {
        for (slot, word) in slots.iter_mut().zip(&parsed.words) {
            if word.start == word.end || word.context_only() {
                *slot = 0;
            }
        }
    }
    let run = Uuid::new_v4().to_string();
    let mut plans = atoms(&draft.source, &parsed.words, &slots, duration, &run)?;
    for atom in &mut plans {
        atom.id = Uuid::new_v4().to_string();
    }
    let assigned = plans.iter().filter(|a| a.speaker_id.is_some()).count();
    let mut causes = BTreeMap::new();
    for atom in plans.iter().filter(|a| a.speaker_id.is_none()) {
        *causes
            .entry(unassigned_cause(atom, &baseline.model, &parsed.words))
            .or_insert(0) += 1;
    }
    let posterior = draft.posterior.as_ref().ok_or_else(derived_invalid)?;
    let parent_sha256 = crate::summary::transcript_digest(&draft.source)?;
    let assignments_edited = draft
        .source
        .speaker_analysis
        .as_ref()
        .is_some_and(|a| a.assignments_edited);
    // The parent moves into the output, so one copy of it stays alive below.
    let output = DerivedOutput {
        schema_version: 2,
        projector_version: word_projector(&baseline.model).into(),
        canonical_asr_raw: String::from_utf8(canonical).map_err(|_| derived_invalid())?,
        canonical_asr_sha256: baseline.source.as_ref().unwrap().raw_sha256.clone(),
        parent: draft.source,
        parent_sha256,
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
            assignments_edited,
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
    let initial = decode_derived(&raw, duration, &output.parent.source_session_id, id)?;
    let mut current = initial.clone();
    for (segment, atom) in current.segments.iter_mut().zip(&output.atoms) {
        if atom.preserved {
            segment.text = output.parent.segments[atom.parent_index].text.clone();
        }
    }
    if current.segments.iter().any(|s| s.text != s.original_text) {
        current.version.revision = 1;
    }
    let document = serde_json::json!({"schemaVersion":2,"analysis":output.analysis,"source":output.parent,"sourceSha256":output.parent_sha256,
        "configuration":output.configuration,"posterior":{"path":"posterior.f32le","frames":output.posterior.frames,"speakers":output.posterior.speakers,"secondsPerFrame":output.posterior.seconds_per_frame,"bytes":output.posterior.bytes,"sha256":output.posterior.sha256},
        "activityNormalization":{"rule":"nemo-v3-probability-frame-boundaries-v1","inputFrames":frames,"inputSampleRate":sample_rate,
            "preparedFrames":output.posterior.prepared_frames,"preparedSampleRate":16000,"nativeFrames":output.posterior.frames,
            "secondsPerFrame":output.posterior.seconds_per_frame,"terminalEndClippedToPreparedDuration":true},
        "projection":{"projectorVersion":output.projector_version,"assignment":crate::diarization::WORD_PROJECTION_VERSION,"wordSpans":output.atoms},"rawBytes":draft.raw.len(),"rawPath":"raw.json"});
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
