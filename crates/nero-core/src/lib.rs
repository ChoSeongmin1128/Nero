pub mod agent_control;
mod capture;
mod diarization;
mod engine;
pub mod identity;
mod library;
mod nemotron;
pub mod people;
pub mod presentation;
pub use people::{MeetingAttendee, PeopleCatalog, Person, PersonGroup, PromotedAttendee};
pub mod reader;
pub mod speech_runtime;
mod store;
mod summary;
pub mod templates;
pub mod terminology;
mod transcript;
mod whisper;
pub use capture::CaptureReader;
pub use diarization::{SpeakerAnalysis, SpeakerAnalysisDraft, SpeakerPosterior};
pub use library::Folder;
pub use presentation::{DocumentFormat, DocumentKind, ExportedDocument};
pub use store::{CaptureQuality, CaptureQualityEvent, CaptureQualityEventKind};
pub use summary::{
    ExternalSummaryProducer, Summary, SummaryCitation, SummaryCitations, SummaryContext,
    SummaryDraft, SummaryInput, SummaryProvenance, SummaryView, MAX_SUMMARY_CITATION_REFERENCES,
};
pub use terminology::{
    discover_candidates, TerminologyCandidate, TerminologyDecision, TerminologyDraft,
    TerminologyKind, TerminologyOutcome, TerminologyProfile, TerminologyRule,
};
pub use transcript::{
    SpeakerNameOrigin, Transcript, TranscriptHistory, TranscriptHistoryEntry, TranscriptSegment,
    TranscriptSource, TranscriptSpeaker, TranscriptUpgradeReport, TranscriptVersion,
};

use engine::Engine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("회의를 찾을 수 없습니다.")]
    MeetingNotFound,
    #[error("이미 녹음 중인 회의가 있습니다.")]
    RecordingBusy,
    #[error("현재 녹음과 다른 요청입니다.")]
    StaleSession,
    #[error("{0}")]
    InvalidState(String),
    #[error("{0}")]
    Audio(String),
    #[error("저장할 수 없습니다: {0}")]
    Storage(String),
    #[error(transparent)]
    Reader(#[from] reader::ReadError),
    #[error("다른 변경이 저장되었습니다. 최신 정보를 읽고 다시 시도해 주세요.")]
    EditConflict,
    #[error("이미 이 저장소를 사용하는 Nero가 있습니다.")]
    StoreBusy,
    #[error("Nero가 종료되어 요청을 처리할 수 없습니다.")]
    Unavailable,
}

impl From<std::io::Error> for CoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MeetingStatus {
    Draft,
    Recording,
    Paused,
    Completed,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SessionState {
    Recording,
    Paused,
    Completed,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioArtifact {
    pub session_id: String,
    pub relative_path: String,
    pub frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CalendarReference {
    pub occurrence_key: String,
    pub title: String,
    pub start_at: u64,
    pub end_at: u64,
    pub calendar_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    pub join_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub notes: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attendees: Vec<MeetingAttendee>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar_reference: Option<CalendarReference>,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<u64>,
    pub status: MeetingStatus,
    pub audio: Option<AudioArtifact>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "store::deserialize_optional_capture_quality"
    )]
    pub capture_quality: Option<CaptureQuality>,
    #[serde(default)]
    pub transcript_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DraftAttachmentTarget {
    pub meeting_id: String,
    pub expected_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub id: String,
    pub meeting_id: String,
    pub device_id: String,
    pub device_name: String,
    pub state: SessionState,
    pub elapsed_ms: u64,
    pub saved_frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "store::deserialize_optional_capture_quality"
    )]
    pub capture_quality: Option<CaptureQuality>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub revision: u64,
    pub meetings: Vec<Meeting>,
    pub folders: Vec<Folder>,
    pub session: Option<SessionSnapshot>,
}

type Job = Box<dyn FnOnce(&mut Engine) + Send>;

enum Command {
    Run(Job),
    Shutdown,
}

struct HandleInner {
    commands: mpsc::SyncSender<Command>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
}

impl Drop for HandleInner {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Ok(worker) = self.worker.get_mut() {
            if let Some(worker) = worker.take() {
                let _ = worker.join();
            }
        }
    }
}

#[derive(Clone)]
pub struct CoreHandle {
    inner: Arc<HandleInner>,
}

impl CoreHandle {
    pub fn upgrade_transcript_sources(&self) -> Result<TranscriptUpgradeReport> {
        self.call(|engine| engine.upgrade_transcript_sources())
    }
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let (commands, incoming) = mpsc::sync_channel(32);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("nero-core".into())
            .spawn(move || {
                let mut engine = match Engine::open(root) {
                    Ok(engine) => {
                        let _ = ready_tx.send(Ok(()));
                        engine
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                loop {
                    engine.poll_capture();
                    match incoming.recv_timeout(std::time::Duration::from_millis(40)) {
                        Ok(Command::Run(job)) => job(&mut engine),
                        Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
                engine.shutdown();
            })?;
        ready_rx.recv().map_err(|_| CoreError::Unavailable)??;
        Ok(Self {
            inner: Arc::new(HandleInner {
                commands,
                worker: Mutex::new(Some(worker)),
            }),
        })
    }

    fn call<T: Send + 'static>(
        &self,
        action: impl FnOnce(&mut Engine) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        // Check on the actor, not the sending thread: requests already queued
        // when update preparation closes the gate must not write afterwards.
        self.call_unfenced(move |engine| {
            if engine.quitting {
                return Err(CoreError::Unavailable);
            }
            action(engine)
        })
    }

    fn call_unfenced<T: Send + 'static>(
        &self,
        action: impl FnOnce(&mut Engine) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (reply, result) = mpsc::sync_channel(1);
        self.inner
            .commands
            .send(Command::Run(Box::new(move |engine| {
                engine.poll_capture();
                let _ = reply.send(action(engine));
            })))
            .map_err(|_| CoreError::Unavailable)?;
        result.recv().map_err(|_| CoreError::Unavailable)?
    }

    pub fn agent_validate(&self, mutation: agent_control::Mutation) -> Result<()> {
        self.call(move |engine| agent_control::validate(engine, &mutation))
    }

    pub fn agent_mutate(&self, mutation: agent_control::Mutation) -> Result<serde_json::Value> {
        self.call(move |engine| agent_control::apply(engine, mutation))
    }

    pub fn list_meetings(&self) -> Result<Vec<Meeting>> {
        self.call(|engine| Ok(engine.meetings()))
    }

    pub fn create_meeting(&self, title: &str, notes: &str) -> Result<Meeting> {
        let (title, notes) = (title.to_owned(), notes.to_owned());
        self.call(move |engine| engine.create_meeting(title, notes))
    }

    pub fn draft_attachment_target(&self, id: &str) -> Result<DraftAttachmentTarget> {
        let id = id.to_owned();
        self.call(move |engine| engine.draft_attachment_target(&id))
    }

    pub fn validate_draft_attachment(&self, target: DraftAttachmentTarget) -> Result<()> {
        self.call(move |engine| engine.require_draft_attachment(&target).map(|_| ()))
    }

    pub fn attach_calendar_to_draft(
        &self,
        target: DraftAttachmentTarget,
        reference: CalendarReference,
    ) -> Result<Meeting> {
        self.call(move |engine| engine.attach_calendar_to_draft(target, reference))
    }

    pub fn create_meeting_from_calendar(&self, reference: CalendarReference) -> Result<Meeting> {
        self.call(move |engine| engine.create_meeting_from_calendar(reference))
    }

    pub fn create_folder(&self, name: &str) -> Result<Folder> {
        let name = name.to_owned();
        self.call(move |engine| engine.create_folder(name))
    }
    pub fn rename_folder(&self, id: &str, name: &str) -> Result<Folder> {
        let (id, name) = (id.to_owned(), name.to_owned());
        self.call(move |engine| engine.rename_folder(&id, name))
    }
    pub fn delete_folder(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.call(move |engine| engine.delete_folder(&id))
    }
    pub fn move_meetings(&self, ids: Vec<String>, folder_id: Option<String>) -> Result<()> {
        self.call(move |engine| engine.move_meetings(&ids, folder_id))
    }
    pub fn trash_meetings(&self, ids: Vec<String>) -> Result<()> {
        self.call(move |engine| engine.trash_meetings(&ids))
    }
    pub fn restore_meetings(&self, ids: Vec<String>) -> Result<()> {
        self.call(move |engine| engine.restore_meetings(&ids))
    }
    pub fn retry_purge_cleanup(&self) -> Result<()> {
        self.call(|engine| engine.retry_purge_cleanup())
    }
    pub fn purge_meetings(&self, ids: Vec<String>) -> Result<()> {
        self.call(move |engine| engine.purge_meetings(&ids))
    }

    pub fn import_audio(&self, title: &str, source: &Path, prepared: &Path) -> Result<Meeting> {
        let (title, source, prepared) = (title.to_owned(), source.to_owned(), prepared.to_owned());
        self.call(move |engine| engine.import_audio(title, source, prepared))
    }

    pub fn import_audio_into_draft(
        &self,
        target: DraftAttachmentTarget,
        source: &Path,
        prepared: &Path,
    ) -> Result<Meeting> {
        let (source, prepared) = (source.to_owned(), prepared.to_owned());
        self.call(move |engine| engine.import_audio_into_draft(target, source, prepared))
    }

    pub fn transcript(&self, id: &str) -> Result<Option<Transcript>> {
        let id = id.to_owned();
        self.call(move |engine| engine.transcript(&id))
    }

    pub fn transcript_history(&self, id: &str) -> Result<TranscriptHistory> {
        let id = id.to_owned();
        self.call(move |engine| engine.transcript_history(&id))
    }

    pub fn restore_transcript(
        &self,
        id: &str,
        expected: TranscriptVersion,
        target: TranscriptVersion,
    ) -> Result<Transcript> {
        let id = id.to_owned();
        self.call(move |engine| engine.restore_transcript(&id, expected, target))
    }

    pub fn commit_transcript(
        &self,
        id: &str,
        expected: Option<TranscriptVersion>,
        raw: Vec<u8>,
    ) -> Result<Transcript> {
        let id = id.to_owned();
        self.call(move |engine| engine.commit_transcript(&id, expected, raw))
    }

    pub fn commit_whole_transcript(
        &self,
        id: &str,
        expected: Option<TranscriptVersion>,
        raw: Vec<u8>,
    ) -> Result<Transcript> {
        let id = id.to_owned();
        self.call(move |engine| engine.commit_whole_transcript(&id, expected, raw))
    }

    pub fn commit_nemotron_transcript(
        &self,
        id: &str,
        expected: Option<TranscriptVersion>,
        raw: Vec<u8>,
    ) -> Result<Transcript> {
        let id = id.to_owned();
        self.call(move |engine| engine.commit_nemotron_transcript(&id, expected, raw))
    }

    pub fn commit_whisper_transcript(
        &self,
        id: &str,
        expected: Option<TranscriptVersion>,
        raw: Vec<u8>,
    ) -> Result<Transcript> {
        let id = id.to_string();
        self.call(move |engine| engine.commit_whisper_transcript(&id, expected, raw))
    }

    pub fn edit_transcript(
        &self,
        id: &str,
        expected: TranscriptVersion,
        texts: Vec<String>,
    ) -> Result<Transcript> {
        let id = id.to_owned();
        self.call(move |engine| engine.edit_transcript(&id, expected, texts))
    }

    pub fn commit_terminology(&self, id: &str, draft: TerminologyDraft) -> Result<Transcript> {
        let id = id.to_owned();
        self.call(move |engine| engine.commit_terminology(&id, draft))
    }

    pub fn apply_diarization(&self, id: &str, draft: SpeakerAnalysisDraft) -> Result<Transcript> {
        let id = id.to_owned();
        self.call(move |engine| engine.apply_diarization(&id, draft))
    }

    pub fn edit_speakers(
        &self,
        id: &str,
        expected: TranscriptVersion,
        speakers: Vec<TranscriptSpeaker>,
        assignments: Vec<Option<String>>,
    ) -> Result<Transcript> {
        let id = id.to_owned();
        self.call(move |engine| engine.edit_speakers(&id, expected, speakers, assignments))
    }

    pub fn summary(&self, id: &str) -> Result<Option<SummaryView>> {
        let id = id.to_owned();
        self.call(move |engine| engine.summary(&id))
    }
    pub fn export_document(
        &self,
        meeting_id: &str,
        kind: DocumentKind,
        format: DocumentFormat,
        include_citations: bool,
    ) -> Result<ExportedDocument> {
        let id = meeting_id.to_owned();
        self.call(move |engine| engine.export_document(&id, kind, format, include_citations))
    }
    pub fn summary_citations(
        &self,
        id: &str,
        summary_id: &str,
        references: Vec<u32>,
    ) -> Result<SummaryCitations> {
        let (id, summary_id) = (id.to_owned(), summary_id.to_owned());
        self.call(move |engine| engine.summary_citations(&id, &summary_id, &references))
    }
    pub fn summary_input(&self, id: &str) -> Result<SummaryInput> {
        let id = id.to_owned();
        self.call(move |engine| engine.summary_input(&id))
    }
    pub fn commit_summary(&self, id: &str, draft: SummaryDraft) -> Result<Summary> {
        let id = id.to_owned();
        self.call(move |engine| engine.commit_summary(&id, draft))
    }

    pub fn update_meeting(&self, id: &str, title: &str, notes: &str) -> Result<Meeting> {
        let (id, title, notes) = (id.to_owned(), title.to_owned(), notes.to_owned());
        self.call(move |engine| engine.update_meeting(&id, title, notes))
    }

    pub fn set_meeting_date(&self, id: &str, meeting_date: Option<String>) -> Result<Meeting> {
        let id = id.to_owned();
        self.call(move |engine| engine.set_meeting_date(&id, meeting_date))
    }

    pub fn devices(&self) -> Result<Vec<Device>> {
        self.call(|engine| engine.devices())
    }

    pub fn start(&self, meeting_id: &str, device_id: &str) -> Result<Snapshot> {
        let (meeting_id, device_id) = (meeting_id.to_owned(), device_id.to_owned());
        self.call(move |engine| engine.start(&meeting_id, &device_id))
    }

    /// Creates an explicitly requested recording without a gap between the busy
    /// check and creation. A failed capture preserves the requested draft.
    pub fn create_recording(&self, title: &str, notes: &str, device_id: &str) -> Result<Snapshot> {
        let (title, notes, device_id) = (title.to_owned(), notes.to_owned(), device_id.to_owned());
        self.call(move |engine| {
            if engine.capture.is_some() {
                return Err(CoreError::RecordingBusy);
            }
            let meeting = engine.create_meeting(title, notes)?;
            engine.start(&meeting.id, &device_id)
        })
    }

    pub fn create_calendar_recording(
        &self,
        reference: CalendarReference,
        device_id: &str,
    ) -> Result<(Meeting, Snapshot)> {
        let device_id = device_id.to_owned();
        self.call(move |engine| {
            if engine.capture.is_some() {
                return Err(CoreError::RecordingBusy);
            }
            let meeting = engine.create_meeting_from_calendar(reference)?;
            if meeting.deleted_at.is_some() {
                return Err(CoreError::InvalidState(
                    "휴지통에 있는 회의입니다. 복원 후 열어 주세요.".into(),
                ));
            }
            let snapshot = if meeting.status == MeetingStatus::Draft && meeting.audio.is_none() {
                engine.start(&meeting.id, &device_id)?
            } else {
                engine.snapshot()
            };
            Ok((meeting, snapshot))
        })
    }

    pub fn pause(&self, session_id: &str) -> Result<Snapshot> {
        let session_id = session_id.to_owned();
        self.call(move |engine| engine.pause(&session_id))
    }

    pub fn resume(&self, session_id: &str) -> Result<Snapshot> {
        let session_id = session_id.to_owned();
        self.call(move |engine| engine.resume(&session_id))
    }

    pub fn stop(&self, session_id: &str) -> Result<Snapshot> {
        let session_id = session_id.to_owned();
        self.call(move |engine| engine.stop(&session_id))
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        self.call_unfenced(|engine| Ok(engine.snapshot()))
    }

    pub fn capture_reader(&self, session_id: &str) -> Result<CaptureReader> {
        let session_id = session_id.to_owned();
        self.call(move |engine| {
            let capture = engine
                .capture
                .as_ref()
                .filter(|capture| capture.manifest.session_id == session_id)
                .ok_or(CoreError::StaleSession)?;
            capture.reader(&engine.store.root)
        })
    }

    pub fn prepare_shutdown(&self, expected_session_id: Option<&str>) -> Result<Snapshot> {
        let expected_session_id = expected_session_id.map(str::to_owned);
        self.call_unfenced(move |engine| engine.prepare_shutdown(expected_session_id.as_deref()))
    }

    pub fn resume_after_failed_shutdown(&self) -> Result<()> {
        self.call_unfenced(|engine| engine.resume_after_failed_shutdown())
    }

    pub fn audio_path(&self, meeting_id: &str) -> Result<PathBuf> {
        let meeting_id = meeting_id.to_owned();
        self.call(move |engine| engine.audio_path(&meeting_id))
    }
}

#[cfg(test)]
mod shutdown_gate_tests {
    use super::*;

    #[test]
    fn update_shutdown_fences_all_actor_requests_and_failure_reopens_them() {
        let temporary = tempfile::tempdir().unwrap();
        let core = CoreHandle::open(temporary.path().canonicalize().unwrap()).unwrap();
        let meeting = core.create_meeting("보존", "메모").unwrap();
        core.prepare_shutdown(None).unwrap();
        assert!(matches!(
            core.create_folder("설치 중 변경"),
            Err(CoreError::Unavailable)
        ));
        assert!(matches!(core.list_meetings(), Err(CoreError::Unavailable)));
        assert_eq!(core.snapshot().unwrap().meetings[0].id, meeting.id);
        assert!(core.prepare_shutdown(None).is_ok());
        core.resume_after_failed_shutdown().unwrap();
        core.create_folder("작업 재개").unwrap();
        assert_eq!(core.list_meetings().unwrap()[0].notes, "메모");
    }
}
