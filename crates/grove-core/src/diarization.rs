use crate::{CoreError, Result, Transcript, TranscriptSpeaker};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const PROJECTION_VERSION: &str = "majority-coverage-v1";
pub const WORD_PROJECTION_VERSION: &str = "nemo-word-160ms-argmax-v1";
const TIMEBASE: u64 = 16_000;
const NEMO_RUNTIME: &str = "nemo-speech-0.2.0@6a3ca369370782acee0dd155e34394ba60b920c4";
const NEMO_MODEL: &str = "nvidia/Nemotron-3-Diarization";
const NEMO_REVISION: &str = "f667ed73aee57d40cc39428eb768b4fd87a0a29e";
const NEMO_SHA256: &str = "08456d9e22cd9a323c0364d98375f3746d6e68507ebb705cd46438c534c7a3a1";
const NEMO_TAIL_RULE: &str = "nemo-v3-centered-stft-terminal-tail-v1";

fn pinned_nemo(draft: &SpeakerAnalysisDraft) -> bool {
    let config = &draft.configuration;
    draft.runtime == NEMO_RUNTIME
        && draft.model == NEMO_MODEL
        && draft.model_revision == NEMO_REVISION
        && draft.model_sha256 == NEMO_SHA256
        && config["scope"] == "recording"
        && config["preset"] == "v3-offline"
        && config["fullAttention"] == false
        && config["batching"] == false
        && matches!(config["backend"].as_str(), Some("cpu" | "metal" | "vulkan"))
        && config["postprocessingSource"] == "pinned-runtime-defaults"
        && config["onset"] == serde_json::json!(0.641_f32)
        && config["offset"] == serde_json::json!(0.561_f32)
        && config["padOnset"] == serde_json::json!(0.229)
        && config["padOffset"] == serde_json::json!(0.079)
        && config["minDurationOn"] == serde_json::json!(0.511)
        && config["minDurationOff"] == serde_json::json!(0.296)
}

fn normalization(
    draft: &SpeakerAnalysisDraft,
    frames: u64,
    sample_rate: u32,
) -> Result<(Option<f64>, serde_json::Value)> {
    if draft.posterior.is_some() && posterior_profile(draft) {
        let prepared =
            (u128::from(frames) * 16_000 + u128::from(sample_rate / 2)) / u128::from(sample_rate);
        let prepared = u64::try_from(prepared).map_err(|_| invalid())?;
        return Ok((
            Some(prepared as f64 / 16_000.0),
            serde_json::json!({
                "rule":"nemo-v3-probability-frame-boundaries-v1","inputFrames":frames,
                "inputSampleRate":sample_rate,"preparedFrames":prepared,"preparedSampleRate":16000,
                "nativeFrames":prepared/160+1,"secondsPerFrame":f64::from(0.01_f32),"clipped":[]
            }),
        ));
    }
    if !pinned_nemo(draft) {
        return Ok((
            None,
            serde_json::json!({"rule":"strict-audio-bounds-v1","clipped":[]}),
        ));
    }
    // This exception is tied to the model/runtime pins above, not a generic epsilon.
    // src/common/audio_resampler.cpp: rounded_output_count uses nearest/ties-up.
    let resampled = (u128::from(frames) * u128::from(TIMEBASE) + u128::from(sample_rate / 2))
        / u128::from(sample_rate);
    let resampled = u64::try_from(resampled).map_err(|_| invalid())?;
    if resampled == 0 {
        return Err(invalid());
    }
    // Pinned GGUF: sr=16000, n_fft=512, hop=160, output_subsampling_factor=1.
    // finish() appends n_fft/2, and FE i_max removes the same half window:
    // floor((N + n_fft/2 - n_fft/2) / hop) + 1 native frames.
    // See diar_pipeline.cpp:128-150,202-218 and features/fe.cpp:668-706.
    let native_frames = resampled / 160 + 1;
    let stride = f64::from(0.01_f32);
    let horizon = native_frames as f64 * stride;
    // app/diarize.cpp renders each endpoint with %.3f. Reproduce that decimal
    // precision for the model's terminal horizon; do not widen all timestamps.
    let serialized_horizon: f64 = format!("{horizon:.3}").parse().map_err(|_| invalid())?;
    if !serialized_horizon.is_finite() {
        return Err(invalid());
    }
    Ok((
        Some(serialized_horizon),
        serde_json::json!({
            "rule":NEMO_TAIL_RULE,"inputFrames":frames,"inputSampleRate":sample_rate,
            "modelSampleRate":TIMEBASE,"resampledFrames":resampled,"fftSize":512,
            "frontendHopSamples":160,"nativeFrames":native_frames,"secondsPerFrame":stride,
            "serializedHorizonSeconds":serialized_horizon,"clipped":[]
        }),
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpeakerAnalysis {
    pub run_id: String,
    pub scope: String,
    pub model: String,
    pub model_revision: String,
    pub model_sha256: String,
    pub runtime: String,
    pub projection_version: String,
    pub raw_sha256: String,
    pub assigned_count: usize,
    pub unassigned_count: usize,
    pub review_causes: BTreeMap<String, usize>,
    #[serde(default)]
    pub assignments_edited: bool,
}

#[derive(Debug, Clone)]
pub struct SpeakerPosterior {
    pub frames: u64,
    pub speakers: u32,
    pub seconds_per_frame: f64,
    /// Little-endian, frame-major float32 sigmoid probabilities.
    pub probabilities: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SpeakerAnalysisDraft {
    pub source: Transcript,
    pub model: String,
    pub model_revision: String,
    pub model_sha256: String,
    pub runtime: String,
    pub configuration: serde_json::Value,
    pub raw: Vec<u8>,
    pub posterior: Option<SpeakerPosterior>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Output {
    #[serde(default, rename = "file")]
    _file: Option<String>,
    segments: Vec<Activity>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Activity {
    start: f64,
    end: f64,
    speaker: u32,
}

pub(crate) struct SpeakerAnalysisEvidence {
    pub run_id: String,
    pub raw: Vec<u8>,
    pub document: serde_json::Value,
    pub posterior: Option<Vec<u8>>,
}

pub(crate) fn posterior_profile(draft: &SpeakerAnalysisDraft) -> bool {
    let c = &draft.configuration;
    !(draft.model != NEMO_MODEL
        || draft.model_revision != NEMO_REVISION
        || draft.model_sha256 != NEMO_SHA256
        || draft.runtime != NEMO_RUNTIME
        || c["scope"] != "recording"
        || c["fullAttention"] != false
        || c["batching"] != false
        || !matches!(c["backend"].as_str(), Some("cpu" | "metal" | "vulkan"))
        || c["postprocessingSource"] != "nvidia-v3-probability-v1"
        || c["assignment"] != WORD_PROJECTION_VERSION
        || c["geometry"]
            != serde_json::json!({"spkcache":264,"fifo":40,"chunk":340,
            "updatePeriod":300,"leftContext":0,"rightContext":40}))
}

pub(crate) fn posterior_slots(
    draft: &SpeakerAnalysisDraft,
    words: &[(f64, f64)],
    frames: u64,
    sample_rate: u32,
) -> Result<Vec<u32>> {
    let p = draft.posterior.as_ref().ok_or_else(invalid)?;
    if frames == 0
        || sample_rate == 0
        || !posterior_profile(draft)
        || p.speakers != 8
        || p.seconds_per_frame != f64::from(0.01_f32)
    {
        return Err(invalid());
    }
    let resampled =
        (u128::from(frames) * 16_000 + u128::from(sample_rate / 2)) / u128::from(sample_rate);
    let expected = u64::try_from(resampled).map_err(|_| invalid())? / 160 + 1;
    let bytes = expected
        .checked_mul(8)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(invalid)?;
    if p.frames != expected
        || bytes != p.probabilities.len() as u64
        || bytes > 128 * 1024 * 1024
        || resampled == 0
    {
        return Err(invalid());
    }
    if p.probabilities.chunks_exact(4).any(|b| {
        let value = f32::from_le_bytes(b.try_into().unwrap());
        !value.is_finite() || !(0.0..=1.0).contains(&value)
    }) {
        return Err(invalid());
    }
    let output: Output = serde_json::from_slice(&draft.raw).map_err(|_| invalid())?;
    if output.segments.len() > 100_000 {
        return Err(invalid());
    }
    // V2 activity is an exact view of the archived full timeline, with only the
    // centered-STFT terminal tail clipped to the prepared 16 kHz sample count.
    let duration = resampled as f64 / 16_000.0;
    let mut expected_activity = Vec::new();
    for speaker in 0..8_usize {
        let mut start = None;
        for frame in 0..=p.frames {
            let active = if frame == p.frames {
                false
            } else {
                let offset = (frame as usize * 8 + speaker) * 4;
                f32::from_le_bytes(p.probabilities[offset..offset + 4].try_into().unwrap()) > 0.5
            };
            if active && start.is_none() {
                start = Some(frame);
            }
            if !active {
                if let Some(first) = start.take() {
                    let a = (first as f64 * p.seconds_per_frame).min(duration);
                    let b = (frame as f64 * p.seconds_per_frame).min(duration);
                    if a < b {
                        expected_activity.push((a, b, speaker as u32 + 1));
                    }
                }
            }
        }
    }
    let mut actual_activity = output
        .segments
        .into_iter()
        .map(|a| (a.start, a.end, a.speaker))
        .collect::<Vec<_>>();
    let order = |a: &(f64, f64, u32), b: &(f64, f64, u32)| {
        a.0.total_cmp(&b.0)
            .then(a.2.cmp(&b.2))
            .then(a.1.total_cmp(&b.1))
    };
    expected_activity.sort_by(order);
    actual_activity.sort_by(order);
    if expected_activity != actual_activity {
        return Err(invalid());
    }
    let mut result = Vec::with_capacity(words.len());
    for &(start, end) in words {
        if !start.is_finite() || !end.is_finite() || start < 0.0 || end < start {
            return Err(invalid());
        }
        // Pinned sortformer_model.h computes ceil(2560 / 160) on the sample grid.
        // The word API has no activity/confidence threshold and lower-slot ties win.
        let f0 = (start / p.seconds_per_frame) as u64;
        let f1 = ((end / p.seconds_per_frame).ceil() as u64).min(f0.saturating_add(16));
        let first = f0.min(p.frames - 1);
        let last = f1.min(p.frames).max(first + 1);
        let mut sums = [0.0_f64; 8];
        for frame in first..last {
            for (slot, sum) in sums.iter_mut().enumerate() {
                let offset = (frame as usize * 8 + slot) * 4;
                *sum += f64::from(f32::from_le_bytes(
                    p.probabilities[offset..offset + 4].try_into().unwrap(),
                ));
            }
        }
        let mut best = 0;
        for slot in 1..8 {
            if sums[slot] > sums[best] {
                best = slot;
            }
        }
        result.push(best as u32 + 1);
    }
    Ok(result)
}

fn invalid() -> CoreError {
    CoreError::InvalidInput("화자 분석 결과 또는 실행 정보가 올바르지 않습니다.".into())
}

fn metadata(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= 512
        && !value.chars().any(char::is_control)
}

fn sample(seconds: f64, duration: f64, samples: u64) -> Result<u64> {
    if !seconds.is_finite() || seconds < 0.0 || seconds > duration {
        return Err(invalid());
    }
    let value = (seconds * TIMEBASE as f64).round();
    if !value.is_finite() || value >= u64::MAX as f64 || value > samples as f64 {
        return Err(invalid());
    }
    Ok(value as u64)
}

pub(crate) fn project(
    draft: SpeakerAnalysisDraft,
    frames: u64,
    sample_rate: u32,
) -> Result<(Transcript, SpeakerAnalysisEvidence)> {
    project_with_source_assignments(draft, frames, sample_rate, false)
}

pub(crate) fn project_with_source_assignments(
    draft: SpeakerAnalysisDraft,
    frames: u64,
    sample_rate: u32,
    preserve_source_assignments: bool,
) -> Result<(Transcript, SpeakerAnalysisEvidence)> {
    if preserve_source_assignments
        && (draft.posterior.is_none() || draft.source.speaker_analysis.is_some())
    {
        return Err(invalid());
    }
    if draft.posterior.is_some() {
        posterior_slots(&draft, &[], frames, sample_rate)?;
    }
    let duration = frames as f64 / f64::from(sample_rate);
    if frames == 0
        || sample_rate == 0
        || !duration.is_finite()
        || duration <= 0.0
        || duration * TIMEBASE as f64 >= u64::MAX as f64
        || draft.raw.len() > 16 * 1024 * 1024
        || ![&draft.model, &draft.model_revision, &draft.runtime]
            .into_iter()
            .all(|value| metadata(value))
        || draft.model_sha256.len() != 64
        || !draft.model_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || !draft.configuration.is_object()
        || serde_json::to_vec(&draft.configuration)
            .map_err(|_| invalid())?
            .len()
            > 128 * 1024
    {
        return Err(invalid());
    }
    let samples = (duration * TIMEBASE as f64).round() as u64;
    let output: Output = serde_json::from_slice(&draft.raw).map_err(|_| invalid())?;
    if output.segments.len() > 100_000 {
        return Err(invalid());
    }
    let mut ranges: BTreeMap<u32, Vec<(u64, u64)>> = BTreeMap::new();
    let mut previous = 0.0;
    let (terminal_horizon, mut normalization) = normalization(&draft, frames, sample_rate)?;
    for (index, activity) in output.segments.into_iter().enumerate() {
        if activity.start < previous
            || activity.end <= activity.start
            || !(1..=8).contains(&activity.speaker)
        {
            return Err(invalid());
        }
        let clipped_start = if activity.start > duration
            && draft.posterior.is_some()
            && terminal_horizon.is_some_and(|horizon| activity.start <= horizon)
        {
            normalization["clipped"].as_array_mut().unwrap().push(serde_json::json!({
                "activityIndex":index,"speaker":activity.speaker,"originalStartSeconds":activity.start,
                "clippedStartSeconds":duration
            }));
            duration
        } else {
            activity.start
        };
        let start = sample(clipped_start, duration, samples)?;
        let clipped_end =
            if activity.end > duration {
                if !activity.end.is_finite()
                    || !terminal_horizon.is_some_and(|horizon| activity.end <= horizon)
                {
                    return Err(invalid());
                }
                normalization["clipped"].as_array_mut().unwrap().push(serde_json::json!({
                "activityIndex":index,"speaker":activity.speaker,"startSeconds":activity.start,
                "originalEndSeconds":activity.end,"clippedEndSeconds":duration
            }));
                duration
            } else {
                activity.end
            };
        let end = sample(clipped_end, duration, samples)?;
        previous = activity.start;
        if end == start {
            continue;
        }
        let speaker_ranges = ranges.entry(activity.speaker).or_default();
        if let Some(last) = speaker_ranges.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        speaker_ranges.push((start, end));
    }
    let run_id = uuid::Uuid::new_v4().to_string();
    let mut after = draft.source.clone();
    let mut decisions = Vec::new();
    let mut slots = BTreeSet::new();
    let mut review_causes = BTreeMap::new();
    let mut assigned_count = 0;
    let preserve_assignments = draft.posterior.is_some()
        && (preserve_source_assignments
            || draft
                .source
                .speaker_analysis
                .as_ref()
                .is_some_and(|a| a.assignments_edited));
    let protected_speakers: BTreeMap<_, _> = draft
        .source
        .speakers
        .iter()
        .filter(|speaker| {
            draft.posterior.is_some()
                && (preserve_assignments
                    || speaker.name_origin != Some(crate::SpeakerNameOrigin::Automatic))
        })
        .map(|speaker| (speaker.id.clone(), speaker.clone()))
        .collect();
    for segment in &mut after.segments {
        let start = sample(segment.start, duration, samples)?;
        let end = sample(segment.end, duration, samples)?;
        if end < start {
            return Err(invalid());
        }
        let mut covered = BTreeMap::new();
        if end > start {
            for (speaker, speaker_ranges) in &ranges {
                let first = speaker_ranges.partition_point(|range| range.1 <= start);
                let frames: u64 = speaker_ranges[first..]
                    .iter()
                    .take_while(|range| range.0 < end)
                    .map(|range| range.1.min(end) - range.0.max(start))
                    .sum();
                if frames > 0 {
                    covered.insert(*speaker, frames);
                }
            }
        }
        let (slot, reason) = if end == start {
            (None, "zero_duration")
        } else if covered.is_empty() {
            (None, "no_activity")
        } else if covered.len() > 1 {
            (None, "multiple_speakers")
        } else {
            let (speaker, coverage) = covered.iter().next().unwrap();
            if *coverage > (end - start) / 2 {
                (Some(*speaker), "assigned")
            } else {
                (None, "insufficient_coverage")
            }
        };
        let protected = preserve_assignments
            || segment
                .speaker_id
                .as_ref()
                .is_some_and(|id| protected_speakers.contains_key(id));
        let reason = if protected {
            if preserve_source_assignments {
                "preserved_source_assignment"
            } else {
                "preserved_manual_assignment"
            }
        } else {
            reason
        };
        if !protected {
            segment.speaker_id = slot.map(|speaker| format!("diarization:{run_id}:{speaker}"));
            if let Some(slot) = slot {
                slots.insert(slot);
            }
        }
        if segment.speaker_id.is_some() {
            assigned_count += 1;
        } else {
            *review_causes.entry(reason.into()).or_insert(0) += 1;
        }
        decisions.push(serde_json::json!({
            "segmentId":segment.id,"sourceIndex":segment.source_index,
            "startSample":start,"endSample":end,"coverageSamples":covered,
            "assignment":segment.speaker_id,"reason":reason
        }));
    }
    after.speakers = protected_speakers
        .into_values()
        .chain(slots.into_iter().map(|slot| TranscriptSpeaker {
            id: format!("diarization:{run_id}:{slot}"),
            name: format!("화자 {slot}"),
            name_origin: Some(crate::SpeakerNameOrigin::Automatic),
        }))
        .collect();
    after.speaker_analysis = Some(SpeakerAnalysis {
        run_id: run_id.clone(),
        scope: "recording".into(),
        model: draft.model.clone(),
        model_revision: draft.model_revision.clone(),
        model_sha256: draft.model_sha256.clone(),
        runtime: draft.runtime.clone(),
        projection_version: PROJECTION_VERSION.into(),
        raw_sha256: format!("{:x}", Sha256::digest(&draft.raw)),
        assigned_count,
        unassigned_count: after.segments.len() - assigned_count,
        review_causes,
        assignments_edited: preserve_assignments,
    });
    let document = serde_json::json!({
        "schemaVersion":1,"analysis":after.speaker_analysis,"source":draft.source,
        "sourceSha256":crate::summary::transcript_digest(&draft.source)?,
        "configuration":draft.configuration,"timebase":TIMEBASE,"decisions":decisions,
        "assignmentPreservation":{"verifiedInitialAssignmentsDiffer":preserve_source_assignments,
            "preserveSourceAssignments":preserve_assignments},
        "activityNormalization":normalization,
        "rawBytes":draft.raw.len(),"rawPath":"raw.json"
    });
    Ok((
        after,
        SpeakerAnalysisEvidence {
            run_id,
            raw: draft.raw,
            document,
            posterior: draft.posterior.map(|p| p.probabilities),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(segments: serde_json::Value, activity: serde_json::Value) -> SpeakerAnalysisDraft {
        let source = Transcript {
            version: crate::TranscriptVersion {
                id: "transcript".into(),
                revision: 0,
            },
            source_session_id: "session".into(),
            model: "example/asr".into(),
            model_revision: "revision-1".into(),
            source: None,
            speakers: vec![TranscriptSpeaker {
                id: "S1".into(),
                name: "화자 1".into(),
                name_origin: None,
            }],
            segments: segments
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(index, item)| crate::TranscriptSegment {
                    id: format!("utterance-{index}"),
                    start: item["start"].as_f64().unwrap(),
                    end: item["end"].as_f64().unwrap(),
                    original_text: item["text"].as_str().unwrap().into(),
                    text: item["text"].as_str().unwrap().into(),
                    speaker_id: Some("S1".into()),
                    source_index: Some(index as u32),
                })
                .collect(),
            speaker_analysis: None,
        };
        SpeakerAnalysisDraft {
            source,
            model: "example/diarizer".into(),
            model_revision: "revision-1".into(),
            model_sha256: "a".repeat(64),
            runtime: "example-runtime".into(),
            configuration: serde_json::json!({"scope":"recording"}),
            raw: serde_json::to_vec(&serde_json::json!({
                "file":"input.wav", "segments":activity
            }))
            .unwrap(),
            posterior: None,
        }
    }

    fn utterance(start: f64, end: f64) -> serde_json::Value {
        serde_json::json!({"start":start,"end":end,"text":"원문","speaker_id":"S1"})
    }

    fn pinned_draft(activity: serde_json::Value) -> SpeakerAnalysisDraft {
        let mut draft = draft(serde_json::json!([utterance(0.0, 1.0)]), activity);
        draft.model = NEMO_MODEL.into();
        draft.model_revision = NEMO_REVISION.into();
        draft.model_sha256 = NEMO_SHA256.into();
        draft.runtime = NEMO_RUNTIME.into();
        draft.configuration = serde_json::json!({
            "scope":"recording","preset":"v3-offline","fullAttention":false,
            "backend":"cpu","batching":false,"postprocessingSource":"pinned-runtime-defaults",
            "onset":0.641_f32,"offset":0.561_f32,"padOnset":0.229,"padOffset":0.079,
            "minDurationOn":0.511,"minDurationOff":0.296
        });
        draft
    }

    #[test]
    fn pinned_nemo_terminal_padding_is_intersected_without_changing_source_or_raw() {
        let request = pinned_draft(serde_json::json!([
            {"start":0.0,"end":1.01,"speaker":1}
        ]));
        let raw = request.raw.clone();
        let source = request.source.clone();
        let (after, evidence) = project(request, 48018, 48000).unwrap();
        assert_eq!(evidence.raw, raw);
        assert_eq!(after.segments[0].start, source.segments[0].start);
        assert_eq!(after.segments[0].end, source.segments[0].end);
        assert_eq!(after.segments[0].text, source.segments[0].text);
        assert_eq!(
            after.segments[0].original_text,
            source.segments[0].original_text
        );
        assert!(after.segments[0].speaker_id.is_some());
        let policy = &evidence.document["activityNormalization"];
        assert_eq!(policy["rule"], NEMO_TAIL_RULE);
        assert_eq!(policy["inputFrames"], 48018);
        assert_eq!(policy["inputSampleRate"], 48000);
        assert_eq!(policy["resampledFrames"], 16006);
        assert_eq!(policy["nativeFrames"], 101);
        assert_eq!(policy["serializedHorizonSeconds"], 1.01);
        assert_eq!(policy["clipped"][0]["originalEndSeconds"], 1.01);
        assert_eq!(policy["clipped"][0]["clippedEndSeconds"], 48018.0 / 48000.0);
        assert_eq!(policy["clipped"][0]["activityIndex"], 0);
    }

    #[test]
    fn pinned_nemo_tail_allowance_requires_matching_pins_and_configuration() {
        let activity = serde_json::json!([{"start":0.0,"end":1.01,"speaker":1}]);
        let base = pinned_draft(activity.clone());
        let mut variants = vec![];
        let mut other = base.clone();
        other.model_sha256 = "b".repeat(64);
        variants.push(other);
        let mut other = base.clone();
        other.runtime = "other-runtime".into();
        variants.push(other);
        let mut other = base.clone();
        other.configuration["onset"] = 0.5.into();
        variants.push(other);
        let mut other = base;
        other.configuration["preset"] = "v3-streaming".into();
        variants.push(other);
        variants.push(draft(serde_json::json!([utterance(0.0, 1.0)]), activity));
        for request in variants {
            assert!(project(request, 16000, 16000).is_err());
        }
        for invalid in [
            serde_json::json!([{"start":0.0,"end":1.011,"speaker":1}]),
            serde_json::json!([{"start":1.001,"end":1.01,"speaker":1}]),
            serde_json::json!([{"start":-0.001,"end":1.01,"speaker":1}]),
        ] {
            assert!(project(pinned_draft(invalid), 16000, 16000).is_err());
        }
        let mut nonfinite = pinned_draft(serde_json::json!([]));
        nonfinite.raw = br#"{"segments":[{"start":0,"end":NaN,"speaker":1}]}"#.to_vec();
        assert!(project(nonfinite, 16000, 16000).is_err());
    }

    #[test]
    fn source_frame_geometry_is_validated_and_resampling_uses_nearest_ties_up() {
        let request = pinned_draft(serde_json::json!([]));
        let (_, policy) = normalization(&request, 1, 32000).unwrap();
        assert_eq!(policy["resampledFrames"], 1);
        let (_, policy) = normalization(&request, 1439, 48000).unwrap();
        assert_eq!(policy["resampledFrames"], 480);
        assert_eq!(policy["nativeFrames"], 4);
        let (_, policy) = normalization(&request, 44102, 44100).unwrap();
        assert_eq!(policy["resampledFrames"], 16001);
        assert_eq!(policy["nativeFrames"], 101);
        for (frames, rate) in [(0, 16000), (16000, 0), (u64::MAX, 1), (1, 96000)] {
            assert!(project(request.clone(), frames, rate).is_err());
        }
    }

    #[test]
    fn union_requires_one_speaker_with_strict_majority_and_keeps_source_fields() {
        let mut request = draft(
            serde_json::json!([utterance(0.0, 1.0)]),
            serde_json::json!([
                {"start":0.0,"end":0.4,"speaker":2},
                {"start":0.2,"end":0.6,"speaker":2}
            ]),
        );
        request.source.segments[0].text = "사용자 수정".into();
        let source = request.source.clone();
        let raw = request.raw.clone();
        let (after, evidence) = project(request, 16000, 16000).unwrap();
        assert_eq!(after.version, source.version);
        assert_eq!(after.source_session_id, source.source_session_id);
        assert_eq!(after.source, source.source);
        assert_eq!(after.model, source.model);
        assert_eq!(after.model_revision, source.model_revision);
        let actual = &after.segments[0];
        let expected = &source.segments[0];
        assert_eq!(actual.id, expected.id);
        assert_eq!(actual.source_index, expected.source_index);
        assert_eq!((actual.start, actual.end), (expected.start, expected.end));
        assert_eq!(actual.text, "사용자 수정");
        assert_eq!(actual.original_text, "원문");
        assert_eq!(after.speakers[0].name, "화자 2");
        assert_eq!(actual.speaker_id, Some(after.speakers[0].id.clone()));
        assert_eq!(evidence.raw, raw);
        assert_eq!(
            evidence.document["decisions"][0]["coverageSamples"]["2"],
            9600
        );
        assert_eq!(after.speaker_analysis.unwrap().assigned_count, 1);

        let (half, _) = project(
            draft(
                serde_json::json!([utterance(0.0, 1.0)]),
                serde_json::json!([
                    {"start":0.0,"end":0.3,"speaker":1},
                    {"start":0.2,"end":0.5,"speaker":1}
                ]),
            ),
            16000,
            16000,
        )
        .unwrap();
        assert!(half.segments[0].speaker_id.is_none());
        assert_eq!(
            half.speaker_analysis.unwrap().review_causes["insufficient_coverage"],
            1
        );
    }

    #[test]
    fn sequential_or_overlapping_multiple_speakers_never_assign_whole_utterance() {
        for activities in [
            serde_json::json!([
                {"start":0.0,"end":0.8,"speaker":1},
                {"start":0.8,"end":1.0,"speaker":2}
            ]),
            serde_json::json!([
                {"start":0.0,"end":1.0,"speaker":1},
                {"start":0.4,"end":0.5,"speaker":2}
            ]),
        ] {
            let (after, _) = project(
                draft(serde_json::json!([utterance(0.0, 1.0)]), activities),
                16000,
                16000,
            )
            .unwrap();
            assert!(after.segments[0].speaker_id.is_none());
            assert!(after.speakers.is_empty());
            assert_eq!(
                after.speaker_analysis.unwrap().review_causes["multiple_speakers"],
                1
            );
        }
    }

    #[test]
    fn touching_boundaries_and_zero_duration_do_not_invent_activity() {
        let (after, _) = project(
            draft(
                serde_json::json!([
                    utterance(0.2, 0.6),
                    utterance(0.6, 1.0),
                    utterance(0.8, 0.8)
                ]),
                serde_json::json!([
                    {"start":0.0,"end":0.2,"speaker":1},
                    {"start":0.2,"end":0.6,"speaker":2}
                ]),
            ),
            16000,
            16000,
        )
        .unwrap();
        assert_eq!(
            after.segments[0].speaker_id,
            Some(after.speakers[0].id.clone())
        );
        assert!(after.segments[1..].iter().all(|s| s.speaker_id.is_none()));
        let analysis = after.speaker_analysis.unwrap();
        assert_eq!(analysis.assigned_count, 1);
        assert_eq!(analysis.unassigned_count, 2);
        assert_eq!(analysis.review_causes["no_activity"], 1);
        assert_eq!(analysis.review_causes["zero_duration"], 1);
    }

    #[test]
    fn invalid_activity_is_rejected_without_clamping_or_renumbering() {
        for activities in [
            serde_json::json!([{"start":-0.1,"end":0.5,"speaker":1}]),
            serde_json::json!([{"start":0.0,"end":1.01,"speaker":1}]),
            serde_json::json!([{"start":0.5,"end":0.5,"speaker":1}]),
            serde_json::json!([{"start":0.0,"end":1.0,"speaker":0}]),
            serde_json::json!([{"start":0.0,"end":1.0,"speaker":9}]),
            serde_json::json!([
                {"start":0.5,"end":0.8,"speaker":1},
                {"start":0.4,"end":0.9,"speaker":2}
            ]),
        ] {
            assert!(project(
                draft(serde_json::json!([utterance(0.0, 1.0)]), activities),
                16000,
                16000
            )
            .is_err());
        }
        let mut invalid = draft(
            serde_json::json!([utterance(0.0, 1.0)]),
            serde_json::json!([]),
        );
        invalid.raw = br#"{"segments":[],"unexpected":true}"#.to_vec();
        assert!(project(invalid, 16000, 16000).is_err());
    }
    #[test]
    fn new_posterior_keeps_unknown_origin_names_without_linking_new_slot_numbers() {
        let source = draft(
            serde_json::json!([utterance(0.0, 1.0)]),
            serde_json::json!([]),
        )
        .source;
        let mut request = crate::nemotron::tests::posterior_draft(source.clone());
        request.source.speakers[0].name = "이름 출처 미확인".into();
        let (after, evidence) = project(request, 16000, 16000).unwrap();
        assert_eq!(after.segments[0].speaker_id.as_deref(), Some("S1"));
        assert_eq!(after.speakers.len(), 1);
        assert_eq!(after.speakers[0].name, "이름 출처 미확인");
        assert_eq!(after.speakers[0].name_origin, None);
        assert_eq!(
            evidence.document["decisions"][0]["reason"],
            "preserved_manual_assignment"
        );
        assert_eq!(
            after.segments[0].original_text,
            source.segments[0].original_text
        );
    }
}
