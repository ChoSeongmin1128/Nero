// What the app's windows receive: saved documents with the judgements Core makes about
// them, so a screen shows them without repeating the rules. Saved documents stay as they
// are; these fields exist only in responses.
use crate::{
    startup_notice_frames, CaptureQuality, Meeting, RecognitionChunk, SessionSnapshot, Snapshot,
    Transcript,
};
use serde::ser::{Error as _, SerializeStruct};
use serde::{Serialize, Serializer};
use serde_json::Value;

// `value` is `quality` as saved. A recording without a known sample rate counts only
// notices before anything was saved.
fn annotate_capture_quality(value: &mut Value, quality: &CaptureQuality, sample_rate: Option<u32>) {
    let startup_frames = sample_rate.map_or(0, startup_notice_frames);
    if let Some(events) = value.get_mut("events").and_then(Value::as_array_mut) {
        for (projected, event) in events.iter_mut().zip(&quality.events) {
            projected["startup"] = event.startup_xrun(startup_frames).into();
        }
    }
    value["startupXrunCount"] = quality.startup_xruns(startup_frames).into();
}

pub fn capture_quality(quality: &CaptureQuality, sample_rate: Option<u32>) -> Value {
    let mut value = serde_json::to_value(quality).unwrap_or_default();
    annotate_capture_quality(&mut value, quality, sample_rate);
    value
}

fn with_capture_quality<S: Serializer>(
    document: &impl Serialize,
    quality: Option<&CaptureQuality>,
    sample_rate: Option<u32>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let Some(quality) = quality else {
        return document.serialize(serializer);
    };
    let mut value = serde_json::to_value(document).map_err(S::Error::custom)?;
    annotate_capture_quality(&mut value["captureQuality"], quality, sample_rate);
    value.serialize(serializer)
}

#[derive(Clone, Copy)]
pub struct ScreenMeeting<'a>(pub &'a Meeting);

impl Serialize for ScreenMeeting<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let meeting = self.0;
        with_capture_quality(
            meeting,
            meeting.capture_quality.as_ref(),
            meeting.audio.as_ref().map(|audio| audio.sample_rate),
            serializer,
        )
    }
}

#[derive(Clone, Copy)]
pub struct ScreenSession<'a>(pub &'a SessionSnapshot);

impl Serialize for ScreenSession<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let session = self.0;
        with_capture_quality(
            session,
            session.capture_quality.as_ref(),
            Some(session.sample_rate),
            serializer,
        )
    }
}

// Snapshots are sent only to the app's windows.
impl Serialize for Snapshot {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut snapshot = serializer.serialize_struct("Snapshot", 4)?;
        snapshot.serialize_field("revision", &self.revision)?;
        snapshot.serialize_field(
            "meetings",
            &self.meetings.iter().map(ScreenMeeting).collect::<Vec<_>>(),
        )?;
        snapshot.serialize_field("folders", &self.folders)?;
        snapshot.serialize_field("session", &self.session.as_ref().map(ScreenSession))?;
        snapshot.end()
    }
}

// Each recognition chunk also carries `settled`: nothing more can be recognized there.
#[derive(Clone)]
pub struct ScreenTranscript(pub Transcript);

impl Serialize for ScreenTranscript {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let Some(report) = self
            .0
            .source
            .as_ref()
            .and_then(|source| source.recognition_report.as_ref())
        else {
            return self.0.serialize(serializer);
        };
        // Only the source becomes a JSON tree; the segments are written straight from the
        // transcript. Naming every field keeps this in step with `Transcript`'s own form.
        let Transcript {
            version,
            source_session_id,
            model,
            model_revision,
            source,
            speakers,
            segments,
            speaker_analysis,
        } = &self.0;
        let mut source = serde_json::to_value(source).map_err(S::Error::custom)?;
        if let Some(chunks) = source["recognitionReport"]["chunks"].as_array_mut() {
            for (projected, chunk) in chunks.iter_mut().zip(&report.chunks) {
                projected["settled"] = RecognitionChunk::settled(chunk).into();
            }
        }
        let mut transcript = serializer.serialize_struct("Transcript", 8)?;
        transcript.serialize_field("version", version)?;
        transcript.serialize_field("sourceSessionId", source_session_id)?;
        transcript.serialize_field("model", model)?;
        transcript.serialize_field("modelRevision", model_revision)?;
        transcript.serialize_field("source", &source)?;
        transcript.serialize_field("speakers", speakers)?;
        transcript.serialize_field("segments", segments)?;
        match speaker_analysis {
            Some(analysis) => transcript.serialize_field("speakerAnalysis", analysis)?,
            None => transcript.skip_field("speakerAnalysis")?,
        }
        transcript.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CaptureQualityEvent, CaptureQualityEventKind, MeetingStatus, RecognitionChunkState,
        RecognitionReport, SessionState, TranscriptSource, TranscriptVersion,
    };

    fn event(kind: CaptureQualityEventKind, saved: u64, queued: u64) -> CaptureQualityEvent {
        CaptureQualityEvent {
            kind,
            count: 1,
            first_observed_at_ms: 0,
            last_observed_at_ms: 0,
            first_saved_frame: saved,
            last_saved_frame: saved,
            first_queued_frame: queued,
            last_queued_frame: queued,
            coalesced: false,
        }
    }

    fn quality() -> CaptureQuality {
        CaptureQuality {
            timing: None,
            xrun_count: 3,
            realtime_denied_count: 0,
            route_changed_count: 0,
            events: vec![
                event(CaptureQualityEventKind::Xrun, 0, 4800),
                event(CaptureQualityEventKind::Xrun, 2000, 4800),
                event(CaptureQualityEventKind::Xrun, 9000, 9600),
                event(CaptureQualityEventKind::RouteChanged, 0, 0),
            ],
        }
    }

    #[test]
    fn snapshots_carry_the_startup_judgement_beside_the_saved_diagnostics() {
        let quality = quality();
        let meeting = Meeting {
            id: "meeting".into(),
            title: "회의".into(),
            notes: String::new(),
            attendees: Vec::new(),
            calendar_reference: None,
            created_at: 1,
            updated_at: 1,
            meeting_date: None,
            recorded_at: None,
            status: MeetingStatus::Completed,
            audio: None,
            error: None,
            warning: None,
            capture_quality: Some(quality.clone()),
            transcript_id: None,
            folder_id: None,
            deleted_at: None,
        };
        let session = SessionSnapshot {
            id: "session".into(),
            meeting_id: "meeting".into(),
            device_id: "device".into(),
            device_name: "마이크".into(),
            state: SessionState::Recording,
            elapsed_ms: 0,
            saved_frames: 0,
            sample_rate: 48000,
            channels: 1,
            error: None,
            warning: None,
            capture_quality: Some(quality.clone()),
        };
        let snapshot = Snapshot {
            revision: 1,
            meetings: vec![meeting.clone()],
            folders: Vec::new(),
            session: Some(session.clone()),
        };
        let value = serde_json::to_value(&snapshot).unwrap();
        let macos = cfg!(target_os = "macos");
        let projected = &value["session"]["captureQuality"];
        assert_eq!(projected["startupXrunCount"], if macos { 2 } else { 0 });
        assert_eq!(
            projected["events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|event| event["startup"].as_bool().unwrap())
                .collect::<Vec<_>>(),
            [macos, macos, false, false]
        );
        // Without a sample rate only notices before anything was saved count.
        let meeting_quality = &value["meetings"][0]["captureQuality"];
        assert_eq!(meeting_quality["startupXrunCount"], u64::from(macos));
        let saved = serde_json::to_value(&meeting).unwrap();
        let mut shown = value["meetings"][0].clone();
        for event in shown["captureQuality"]["events"].as_array_mut().unwrap() {
            event.as_object_mut().unwrap().remove("startup");
        }
        shown["captureQuality"]
            .as_object_mut()
            .unwrap()
            .remove("startupXrunCount");
        assert_eq!(shown, saved);
        // Coalesced counts that each pass validation never overflow their sum.
        let mut huge = quality.clone();
        for event in &mut huge.events {
            event.count = u64::MAX / 2 + 1;
            event.coalesced = true;
        }
        huge.xrun_count = u64::MAX;
        assert!(huge.validate().is_ok());
        assert_eq!(
            huge.startup_xruns(u64::MAX),
            if macos { u64::MAX } else { 0 }
        );
        let mut plain = snapshot;
        plain.meetings[0].capture_quality = None;
        plain.session = None;
        assert_eq!(
            serde_json::to_value(&plain).unwrap()["meetings"][0],
            serde_json::to_value(&plain.meetings[0]).unwrap()
        );
    }

    #[test]
    fn transcripts_mark_each_chunk_that_cannot_be_recognized_again() {
        let chunk =
            |index: u32, start: u64, end: u64, state, issue: Option<&str>| RecognitionChunk {
                index,
                start_frame: start,
                end_frame: end,
                state,
                issue: issue.map(str::to_owned),
            };
        let report = RecognitionReport {
            sample_rate: 16000,
            input_frames: 4_000_000,
            chunks: vec![
                chunk(0, 0, 960_000, RecognitionChunkState::Complete, None),
                chunk(1, 960_000, 1_920_000, RecognitionChunkState::Failed, None),
                chunk(
                    2,
                    1_920_000,
                    2_880_000,
                    RecognitionChunkState::Failed,
                    Some("splitFailed"),
                ),
                chunk(3, 2_880_000, 2_890_000, RecognitionChunkState::Failed, None),
            ],
            timing_context_segments: Vec::new(),
            normalized_end_segments: Vec::new(),
        };
        let transcript = Transcript {
            version: TranscriptVersion {
                id: "transcript".into(),
                revision: 0,
            },
            source_session_id: "session".into(),
            model: "fixture".into(),
            model_revision: "1".into(),
            source: Some(TranscriptSource {
                raw_sha256: "a".repeat(64),
                projector_version: "fixture".into(),
                association: "direct".into(),
                speaker_scope: None,
                chunking_policy: None,
                recognition_report: Some(report.clone()),
            }),
            speakers: Vec::new(),
            segments: Vec::new(),
            speaker_analysis: None,
        };
        let value = serde_json::to_value(ScreenTranscript(transcript.clone())).unwrap();
        let chunks = value["source"]["recognitionReport"]["chunks"]
            .as_array()
            .unwrap();
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk["settled"].as_bool().unwrap())
                .collect::<Vec<_>>(),
            report
                .chunks
                .iter()
                .map(RecognitionChunk::settled)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk["settled"].as_bool().unwrap())
                .collect::<Vec<_>>(),
            [true, false, true, true]
        );
        // Apart from `settled`, the screen gets the transcript as saved, every field included.
        let mut full = transcript;
        full.speakers = vec![crate::TranscriptSpeaker {
            id: "speaker".into(),
            name: "화자 1".into(),
            name_origin: Some(crate::SpeakerNameOrigin::Automatic),
        }];
        full.segments = vec![crate::TranscriptSegment {
            id: "segment".into(),
            start: 0.5,
            end: 1.25,
            original_text: "원문".into(),
            text: "수정".into(),
            speaker_id: Some("speaker".into()),
            source_index: Some(0),
        }];
        full.speaker_analysis = Some(crate::SpeakerAnalysis {
            run_id: "run".into(),
            scope: "recording".into(),
            model: "diarizer".into(),
            model_revision: "1".into(),
            model_sha256: "b".repeat(64),
            runtime: "runtime".into(),
            projection_version: "projection".into(),
            raw_sha256: "c".repeat(64),
            assigned_count: 1,
            unassigned_count: 0,
            review_causes: Default::default(),
            assignments_edited: false,
        });
        for transcript in [
            full.clone(),
            Transcript {
                speaker_analysis: None,
                ..full.clone()
            },
        ] {
            let mut shown = serde_json::to_value(ScreenTranscript(transcript.clone())).unwrap();
            for chunk in shown["source"]["recognitionReport"]["chunks"]
                .as_array_mut()
                .unwrap()
            {
                chunk.as_object_mut().unwrap().remove("settled").unwrap();
            }
            assert_eq!(shown, serde_json::to_value(&transcript).unwrap());
        }
        let mut plain = full;
        plain.source = None;
        assert_eq!(
            serde_json::to_value(ScreenTranscript(plain.clone())).unwrap(),
            serde_json::to_value(&plain).unwrap()
        );
    }
}
