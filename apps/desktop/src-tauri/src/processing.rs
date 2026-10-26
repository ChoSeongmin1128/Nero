use crate::job_journal::{JobRecord, JobStatus, Journal};
use grove_core::{CoreHandle, SessionState, Snapshot};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex, OnceLock,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use uuid::Uuid;

const MODEL: &str = include_str!("../../../../Sources/GroveApp/Resources/Models/moss.json");

fn moss_worker_failure(log: &str) -> Option<(crate::summary_failure::FailureKind, &'static str)> {
    use crate::summary_failure::FailureKind;
    for line in log.lines().rev() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value["worker"] != "moss" {
            continue;
        }
        // Only the worker's fixed kind is trusted. Its message may contain a local path.
        return match value["kind"].as_str()? {
            "inputLimit" => Some((FailureKind::InputLimit,
                "90분을 넘는 녹음은 아직 전체 음성 인식을 지원하지 않습니다. 원음과 기존 음성 기록은 유지했습니다.")),
            "allocation" => Some((FailureKind::Allocation,
                "음성 인식에 필요한 메모리를 확보하지 못했습니다. 다른 작업을 마친 뒤 다시 시도해 주세요.")),
            "input" => Some((FailureKind::Input,
                "음성 인식에 사용할 녹음 파일을 읽지 못했습니다.")),
            "output" => Some((FailureKind::Output,
                "음성 인식 결과가 완성되지 않았습니다. 원음과 기존 음성 기록은 유지했습니다.")),
            "storage" => Some((FailureKind::Storage,
                "음성 인식 결과를 저장하지 못했습니다.")),
            "engineUnavailable" => Some((FailureKind::EngineUnavailable,
                "음성 인식 실행 중 오류가 발생했습니다. 원음과 기존 음성 기록은 유지했습니다.")),
            _ => None,
        };
    }
    None
}

fn platform_supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        cfg!(all(target_os = "macos", target_arch = "aarch64"))
            && Command::new("/usr/bin/sw_vers")
                .arg("-productVersion")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .and_then(|version| version.trim().split('.').next()?.parse::<u32>().ok())
                .is_some_and(|major| major >= 26)
    })
}

#[derive(Deserialize)]
pub(super) struct ModelFile {
    pub(super) name: String,
    pub(super) bytes: u64,
    pub(super) sha256: String,
}
#[derive(Deserialize)]
pub(super) struct Model {
    pub(super) repository: String,
    pub(super) revision: String,
    pub(super) files: Vec<ModelFile>,
}

#[derive(Clone, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub revision: u64,
    pub id: Option<String>,
    pub kind: String,
    pub meeting_id: Option<String>,
    pub status: String,
    pub message: String,
    pub bytes: u64,
    pub total_bytes: u64,
    pub model_bytes: u64,
    pub model_ready: bool,
    pub supported: bool,
    pub summary_supported: bool,
    pub summary_ready: bool,
    pub summary_bytes: u64,
    pub speaker_supported: bool,
    pub speaker_ready: bool,
    pub speaker_bytes: u64,
    pub model_stage: Option<String>,
    pub memory_state: Option<crate::memory::MemoryState>,
    pub phase: Option<String>,
    pub failure_kind: Option<crate::summary_failure::FailureKind>,
    pub failure_detail: Option<crate::summary_failure::FailureDetail>,
    pub has_previous_summary: bool,
    pub retry_count: u8,
    pub committing: bool,
    pub summary_committing: bool,
    pub jobs: Vec<JobView>,
}

#[derive(Clone, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: String,
    pub kind: String,
    pub meeting_id: Option<String>,
    pub status: String,
    pub phase: Option<String>,
    pub message: String,
    pub retryable: bool,
    pub acknowledged: bool,
    pub failure_kind: Option<crate::summary_failure::FailureKind>,
}

type Action = Box<
    dyn FnOnce(Arc<Processing>, Arc<AtomicBool>, PathBuf) -> Result<Option<String>, String> + Send,
>;
struct Pending {
    record: JobRecord,
    action: Action,
    import_existing: Option<HashSet<String>>,
    validate: Option<Box<dyn Fn() -> Result<(), String> + Send>>,
}

fn model_job(kind: &str) -> bool {
    matches!(kind, "setup" | "model" | "summary-model" | "speaker-model")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn meeting_models_ready(progress: &Progress) -> Result<(), String> {
    if !progress.supported {
        return Err("이 환경에서는 음성 인식을 아직 사용할 수 없습니다.".into());
    }
    if !progress.model_ready {
        return Err("음성 인식을 먼저 준비해 주세요.".into());
    }
    Ok(())
}
struct Running {
    id: String,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<()>,
    import_existing: Option<HashSet<String>>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JobOwner {
    meeting_id: String,
}

#[derive(Default)]
struct RecordingTranscription {
    attempted: HashSet<String>,
}

impl RecordingTranscription {
    fn claim(&mut self, snapshot: &Snapshot) -> Option<String> {
        let session = snapshot.session.as_ref()?;
        if session.state != SessionState::Completed {
            return None;
        }
        let meeting = snapshot
            .meetings
            .iter()
            .find(|meeting| meeting.id == session.meeting_id)?;
        if meeting.audio.as_ref()?.session_id != session.id
            || !self.attempted.insert(session.id.clone())
        {
            return None;
        }
        Some(session.meeting_id.clone())
    }
}

pub struct Processing {
    pub(super) root: PathBuf,
    pub(super) resources: PathBuf,
    progress: Mutex<Progress>,
    startup_issues: Mutex<Vec<Progress>>,
    running: Mutex<Option<Running>>,
    pending: Mutex<VecDeque<Pending>>,
    records: Mutex<Vec<JobRecord>>,
    journal: Journal,
    journal_error: Mutex<Option<String>>,
    recording: AtomicBool,
    interrupted_jobs: Mutex<HashSet<String>>,
    revision: AtomicU64,
    recording_transcription: Mutex<RecordingTranscription>,
    shutting_down: AtomicBool,
    worker_cleanup_failed: AtomicBool,
}

pub(super) fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}
pub(super) fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("처리를 취소했습니다.".into())
    } else {
        Ok(())
    }
}
pub(super) async fn interruptible<F: std::future::Future>(
    cancel: &AtomicBool,
    future: F,
) -> Result<F::Output, String> {
    tokio::pin!(future);
    loop {
        cancelled(cancel)?;
        tokio::select! { result = &mut future => return Ok(result), _ = tokio::time::sleep(Duration::from_millis(100)) => () }
    }
}

pub(super) fn directory(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let mut path = root.to_path_buf();
    for part in Path::new(relative).components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return Err("저장 경로가 올바르지 않습니다.".into());
        }
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                return Err("저장 폴더에 연결된 경로나 다른 항목이 있습니다.".into())
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&path).map_err(message)?
            }
            Err(e) => return Err(message(e)),
        }
    }
    Ok(path)
}
pub(super) fn plain_file(path: &Path) -> Result<File, String> {
    let meta = fs::symlink_metadata(path).map_err(message)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("일반 파일이 필요합니다.".into());
    }
    File::open(path).map_err(message)
}
pub(super) fn matches_file(path: &Path, item: &ModelFile, cancel: &AtomicBool) -> bool {
    let Ok(mut file) = plain_file(path) else {
        return false;
    };
    if file.metadata().map(|m| m.len()).ok() != Some(item.bytes) {
        return false;
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        if cancel.load(Ordering::Acquire) {
            return false;
        }
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => hash.update(&buffer[..n]),
            Err(_) => return false,
        }
    }
    format!("{:x}", hash.finalize()) == item.sha256
}
pub(super) fn invalidate_readiness(directory: &Path) -> Result<(), String> {
    let marker = directory.join("verified.json");
    if marker.exists() {
        plain_file(&marker)?;
        fs::rename(
            marker,
            directory.join(format!("verified.invalid-{}.json", Uuid::new_v4())),
        )
        .map_err(message)?;
    }
    Ok(())
}
pub(super) struct Worker(pub(super) std::process::Child);
impl Worker {
    pub(super) fn stop(&mut self) -> Result<std::process::ExitStatus, String> {
        if let Some(status) = self.0.try_wait().map_err(message)? {
            return Ok(status);
        }
        match self.0.kill() {
            Ok(()) => self.0.wait().map_err(message),
            Err(error) => match self.0.try_wait() {
                Ok(Some(status)) => Ok(status),
                _ => Err(message(error)),
            },
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Processing {
    pub fn new(root: PathBuf, resources: PathBuf) -> Arc<Self> {
        let journal = Journal::new(root.clone());
        let (records, issues, journal_error) = match journal.recover_available() {
            Ok(report) => (report.records, report.issues, None),
            Err(error) => (Vec::new(), Vec::new(), Some(error)),
        };
        let startup_issues = journal_error
            .as_ref()
            .map(|error| {
                vec![Progress {
                    id: Some(Uuid::new_v4().to_string()),
                    kind: "recovery".into(),
                    status: "failed".into(),
                    message: error.clone(),
                    ..Default::default()
                }]
            })
            .unwrap_or_default();
        let mut startup_issues = startup_issues;
        if !issues.is_empty() {
            startup_issues.push(Progress {
                id: Some(Uuid::new_v4().to_string()),
                kind: "recovery".into(),
                status: "failed".into(),
                message: "이전 작업의 상태를 복구하지 못했습니다. 기존 회의 자료는 유지했습니다."
                    .into(),
                ..Default::default()
            });
        }
        Arc::new(Self {
            root,
            resources,
            progress: Mutex::new(Progress::default()),
            startup_issues: Mutex::new(startup_issues),
            running: Mutex::new(None),
            pending: Mutex::new(VecDeque::new()),
            records: Mutex::new(records),
            journal,
            journal_error: Mutex::new(journal_error),
            recording: AtomicBool::new(false),
            interrupted_jobs: Mutex::new(HashSet::new()),
            revision: AtomicU64::new(0),
            recording_transcription: Mutex::new(RecordingTranscription::default()),
            shutting_down: AtomicBool::new(false),
            worker_cleanup_failed: AtomicBool::new(false),
        })
    }
    fn model(&self) -> Model {
        serde_json::from_str(MODEL).expect("bundled MOSS manifest")
    }
    fn model_dir(&self) -> Result<PathBuf, String> {
        directory(
            &self.root,
            &format!("models/MOSS-{}", self.model().revision),
        )
    }
    fn worker(&self, name: &str) -> PathBuf {
        self.resources.join("native").join(name)
    }
    fn supported(&self) -> bool {
        platform_supported()
            && self.worker("moss/MossHarness").is_file()
            && self.worker("AudioPrepare").is_file()
    }
    pub fn snapshot(&self) -> Progress {
        let mut progress = {
            let stored = self.progress.lock().unwrap();
            let records = self.records.lock().unwrap();
            let mut progress = stored.clone();
            progress.jobs = records
                .iter()
                .map(|record| JobView {
                    id: record.id.clone(),
                    kind: record.kind.clone(),
                    meeting_id: record.meeting_id.clone(),
                    status: serde_json::to_value(record.status)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .into(),
                    phase: record.phase.clone(),
                    message: record.message.clone(),
                    acknowledged: record.acknowledged,
                    failure_kind: record.failure_kind,
                    retryable: matches!(record.status, JobStatus::Interrupted | JobStatus::Failed)
                        && record.kind != "import",
                })
                .collect();
            if records
                .iter()
                .any(|record| Some(&record.id) == progress.id.as_ref() && record.acknowledged)
            {
                progress.status = "acknowledged".into();
            }
            progress.revision = self.revision.load(Ordering::Acquire);
            progress
        };
        progress.supported = self.supported();
        (
            progress.summary_supported,
            progress.summary_ready,
            progress.summary_bytes,
        ) = crate::summary::availability(self);
        (
            progress.speaker_supported,
            progress.speaker_ready,
            progress.speaker_bytes,
        ) = crate::diarization::availability(self);
        let model = self.model();
        progress.model_bytes = model.files.iter().map(|file| file.bytes).sum();
        let Ok(path) = self.model_dir() else {
            return progress;
        };
        progress.model_ready = plain_file(&path.join("verified.json"))
            .and_then(|file| {
                let mut marker = String::new();
                file.take(64 * 1024)
                    .read_to_string(&mut marker)
                    .map_err(message)?;
                Ok(marker)
            })
            .ok()
            .as_deref()
            == Some(MODEL)
            && model.files.iter().all(|f| {
                fs::symlink_metadata(path.join(&f.name))
                    .map(|m| m.is_file() && !m.file_type().is_symlink() && m.len() == f.bytes)
                    .unwrap_or(false)
            });
        progress
    }
    pub(super) fn observe_memory(&self, state: crate::memory::MemoryState) -> bool {
        let mut progress = self.progress.lock().unwrap();
        if progress.memory_state == Some(state) {
            return false;
        }
        progress.memory_state = Some(state);
        progress.revision = self
            .revision
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        true
    }
    pub(super) fn cleanup_failed(&self) {
        self.worker_cleanup_failed.store(true, Ordering::Release);
    }
    fn worker_ready(&self) -> Result<(), String> {
        if self.worker_cleanup_failed.load(Ordering::Acquire) {
            return Err("이전 음성 처리의 종료를 확인하지 못했습니다.".into());
        }
        Ok(())
    }
    pub(super) fn update(&self, action: impl FnOnce(&mut Progress)) {
        let mut progress = self.progress.lock().unwrap();
        action(&mut progress);
        progress.revision = self
            .revision
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        if let Some(id) = &progress.id {
            let record = self
                .records
                .lock()
                .unwrap()
                .iter()
                .find(|record| &record.id == id)
                .cloned();
            if let Some(mut record) = record {
                let status = match progress.status.as_str() {
                    "running" => JobStatus::Running,
                    "completed" => JobStatus::Completed,
                    "cancelled" => JobStatus::Cancelled,
                    "interrupted" => JobStatus::Interrupted,
                    "failed" => JobStatus::Failed,
                    "needsReview" => JobStatus::NeedsReview,
                    _ => record.status,
                };
                if record.status != status
                    || record.phase != progress.phase
                    || record.message != progress.message
                    || record.meeting_id != progress.meeting_id
                    || record.failure_kind != progress.failure_kind
                {
                    record.status = if record.status == JobStatus::Cancelled
                        || (status == JobStatus::Running
                            && matches!(
                                record.status,
                                JobStatus::Interrupted | JobStatus::Cancelling
                            )) {
                        record.status
                    } else {
                        status
                    };
                    record.phase = progress.phase.clone();
                    record.failure_kind = progress.failure_kind;
                    record.message = progress.message.clone();
                    record.meeting_id = progress.meeting_id.clone();
                    record.updated_at = now().max(record.updated_at);
                    if let Err(error) = self.save_job(record) {
                        *self.journal_error.lock().unwrap() = Some(error.clone());
                        progress.status = "failed".into();
                        progress.message = error;
                    }
                }
            }
        }
    }

    fn save_job(&self, record: JobRecord) -> Result<(), String> {
        self.journal.write(&record)?;
        let mut records = self.records.lock().unwrap();
        if let Some(previous) = records.iter_mut().find(|r| r.id == record.id) {
            *previous = record;
        } else {
            records.push(record);
        }
        self.revision.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn job_ready(&self) -> Result<(), String> {
        self.worker_ready()?;
        if let Some(error) = self.journal_error.lock().unwrap().as_ref() {
            return Err(error.clone());
        }
        Ok(())
    }

    pub(super) fn source_fingerprint(
        core: &CoreHandle,
        id: &str,
        kind: &str,
    ) -> Result<String, String> {
        let snapshot = core.snapshot().map_err(message)?;
        let meeting = snapshot
            .meetings
            .iter()
            .find(|m| m.id == id && m.deleted_at.is_none())
            .ok_or("회의를 찾을 수 없습니다.")?;
        let transcript = core.transcript(id).map_err(message)?;
        let summary = if kind == "summary" {
            core.summary(id).map_err(message)?.map(|s| s.document.id)
        } else {
            None
        };
        let input = serde_json::json!({"audio":meeting.audio,"transcript":transcript,"summary":summary,
            "notes": if kind == "summary" { Some(&meeting.notes) } else { None }});
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&input).map_err(message)?)
        ))
    }

    pub fn drive(self: &Arc<Self>, core: &CoreHandle) -> Result<(), String> {
        let mut running = self.running.lock().unwrap();
        self.recording.store(
            core.snapshot()
                .map_err(message)?
                .session
                .is_some_and(|s| matches!(s.state, SessionState::Recording | SessionState::Paused)),
            Ordering::Release,
        );
        if running.as_ref().is_some_and(|r| !r.handle.is_finished())
            || self.shutting_down.load(Ordering::Acquire)
        {
            return Ok(());
        }
        if let Some(old) = running.take() {
            old.handle
                .join()
                .map_err(|_| "처리 작업의 종료를 확인하지 못했습니다.")?;
        }
        self.job_ready()?;
        let mut queue = self.pending.lock().unwrap();
        let index = queue
            .iter()
            .position(|job| !self.recording.load(Ordering::Acquire) || model_job(&job.record.kind));
        if let Some(index) = index {
            let job = queue.remove(index).unwrap();
            drop(queue);
            *running = Some(self.launch_checked(job)?);
        }
        Ok(())
    }
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        let running = self.running.lock().unwrap();
        let mut pending = self.pending.lock().unwrap();
        if let Some(index) = pending.iter().position(|job| job.record.id == id) {
            let mut record = pending[index].record.clone();
            record.status = JobStatus::Cancelled;
            record.message = "취소했습니다.".into();
            record.updated_at = now().max(record.updated_at);
            self.save_job(record)?;
            pending.remove(index);
            drop(pending);
            self.update(|_| ());
            return Ok(());
        }
        drop(pending);
        let job = running
            .as_ref()
            .filter(|r| r.id == id && !r.handle.is_finished())
            .ok_or("현재 작업과 다른 요청입니다.")?;
        let mut progress = self.progress.lock().unwrap();
        if progress.id.as_deref() == Some(id) && progress.status != "running" {
            return Err("이미 종료된 작업입니다.".into());
        }
        if progress.committing || progress.summary_committing {
            return Err(if progress.kind == "summary" {
                "회의록을 저장하고 있습니다."
            } else {
                "음성 기록을 저장하고 있습니다."
            }
            .into());
        }
        let record = self
            .records
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.id == id)
            .cloned();
        if let Some(mut record) = record {
            record.status = JobStatus::Cancelling;
            record.updated_at = now().max(record.updated_at);
            record.message = "취소 요청을 받았습니다.".into();
            self.save_job(record)?;
        }
        job.cancel.store(true, Ordering::Release);
        progress.message = "취소 중".into();
        progress.revision = self
            .revision
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        Ok(())
    }

    pub(super) fn commit_summary<T>(
        &self,
        cancel: &AtomicBool,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.commit_result(cancel, true, commit)
    }

    pub(super) fn commit_transcript_result<T>(
        &self,
        cancel: &AtomicBool,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.commit_result(cancel, false, commit)
    }

    fn checkpoint_transcript<T>(
        &self,
        cancel: &AtomicBool,
        commit: impl FnOnce() -> grove_core::Result<T>,
    ) -> Result<T, String> {
        // cancel() takes running -> progress. This checkpoint takes only progress,
        // then calls the core actor, which never calls back into Processing.
        // An accepted cancellation therefore wins before the baseline CAS write;
        // cancellation arriving during that write is accepted only after it returns.
        // Unlike the final commit gate, this ends before optional postprocessing.
        let mut progress = self.progress.lock().unwrap();
        cancelled(cancel)?;
        self.job_ready()?;
        self.save_phase(&progress, "saving")?;
        progress.phase = Some("saving".into());
        let result = commit();
        let rejection = match &result {
            Err(grove_core::CoreError::InvalidInput(_)) => Some((
                "outputRejected",
                crate::summary_failure::FailureKind::Output,
            )),
            Err(grove_core::CoreError::InvalidState(_)) => Some((
                "sourceChanged",
                crate::summary_failure::FailureKind::SourceChanged,
            )),
            Err(grove_core::CoreError::MeetingNotFound) => Some((
                "meetingUnavailable",
                crate::summary_failure::FailureKind::MeetingUnavailable,
            )),
            _ => None,
        };
        if result.is_ok() {
            self.save_phase(&progress, "postprocessing")?;
            progress.phase = Some("postprocessing".into());
        } else if let Some((phase, kind)) = rejection {
            // These core transcript errors precede current-document publication.
            self.save_phase(&progress, phase)?;
            progress.phase = Some(phase.into());
            progress.failure_kind = Some(kind);
        }
        progress.revision = self
            .revision
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        drop(progress);
        result.map_err(message)
    }

    fn commit_result<T>(
        &self,
        cancel: &AtomicBool,
        summary: bool,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        {
            let mut progress = self.progress.lock().unwrap();
            cancelled(cancel)?;
            self.job_ready()?;
            self.save_phase(&progress, "saving")?;
            progress.committing = true;
            progress.summary_committing = summary;
            progress.phase = Some("saving".into());
            progress.message = if progress.kind == "import" {
                "음성 저장 중"
            } else if summary {
                "회의록 저장 중"
            } else {
                "음성 기록 저장 중"
            }
            .into();
            progress.revision = self
                .revision
                .fetch_add(1, Ordering::AcqRel)
                .saturating_add(1);
        }
        commit()
    }

    fn save_phase(&self, progress: &Progress, phase: &str) -> Result<(), String> {
        let record = self
            .records
            .lock()
            .unwrap()
            .iter()
            .find(|r| Some(&r.id) == progress.id.as_ref())
            .cloned();
        if let Some(mut record) = record {
            record.phase = Some(phase.into());
            record.updated_at = now().max(record.updated_at);
            self.save_job(record).map_err(|error| {
                *self.journal_error.lock().unwrap() = Some(error.clone());
                error
            })?;
        }
        Ok(())
    }

    pub fn transcript_restore<T>(
        &self,
        meeting_id: &str,
        action: impl FnOnce() -> grove_core::Result<T>,
    ) -> grove_core::Result<T> {
        // Hold the same ownership gate used by new jobs and capture startup through commit.
        let running = self.running.lock().unwrap();
        self.worker_ready()
            .map_err(grove_core::CoreError::InvalidState)?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(grove_core::CoreError::Unavailable);
        }
        if running
            .as_ref()
            .is_some_and(|job| !job.handle.is_finished())
            && self.progress.lock().unwrap().meeting_id.as_deref() == Some(meeting_id)
            || self
                .pending
                .lock()
                .unwrap()
                .iter()
                .any(|job| job.record.meeting_id.as_deref() == Some(meeting_id))
        {
            return Err(grove_core::CoreError::InvalidState(
                "이 회의의 처리가 끝난 뒤 이전 기록으로 되돌려 주세요.".into(),
            ));
        }
        action()
    }

    pub fn meeting_removal<T>(
        &self,
        core: &CoreHandle,
        ids: &[String],
        action: impl FnOnce() -> grove_core::Result<T>,
    ) -> grove_core::Result<T> {
        let mut running = self.running.lock().unwrap();
        self.worker_ready()
            .map_err(grove_core::CoreError::InvalidState)?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(grove_core::CoreError::InvalidState(
                "앱을 종료하고 있습니다.".into(),
            ));
        }
        let snapshot = core.snapshot()?;
        if ids.is_empty()
            || ids
                .iter()
                .any(|id| !snapshot.meetings.iter().any(|meeting| &meeting.id == id))
        {
            return Err(grove_core::CoreError::MeetingNotFound);
        }
        if snapshot.session.as_ref().is_some_and(|session| {
            matches!(
                session.state,
                SessionState::Recording | SessionState::Paused
            ) && ids.contains(&session.meeting_id)
        }) {
            return Err(grove_core::CoreError::InvalidState(
                "녹음을 마친 뒤 삭제해 주세요.".into(),
            ));
        }
        let affected = self
            .progress
            .lock()
            .unwrap()
            .meeting_id
            .as_ref()
            .is_some_and(|id| ids.contains(id))
            || running
                .as_ref()
                .and_then(|job| job.import_existing.as_ref())
                .is_some_and(|existing| {
                    snapshot.meetings.iter().any(|meeting| {
                        ids.contains(&meeting.id)
                            && !existing.contains(&meeting.id)
                            && meeting.audio.is_some()
                    })
                });
        if affected {
            if let Some(job) = running.take() {
                job.cancel.store(true, Ordering::Release);
                self.update(|progress| progress.message = "취소 중".into());
                job.handle.join().map_err(|_| {
                    grove_core::CoreError::InvalidState(
                        "음성 처리 종료를 확인하지 못했습니다. 다시 시도해 주세요.".into(),
                    )
                })?;
            }
        }
        {
            let mut pending = self.pending.lock().unwrap();
            let mut index = 0;
            while index < pending.len() {
                if pending[index]
                    .record
                    .meeting_id
                    .as_ref()
                    .is_some_and(|id| ids.contains(id))
                {
                    let mut record = pending[index].record.clone();
                    record.status = JobStatus::Cancelled;
                    record.message = "회의가 삭제되어 작업을 취소했습니다.".into();
                    record.updated_at = now().max(record.updated_at);
                    self.save_job(record)
                        .map_err(grove_core::CoreError::Storage)?;
                    pending.remove(index);
                } else {
                    index += 1;
                }
            }
        }
        self.worker_ready()
            .map_err(grove_core::CoreError::InvalidState)?;
        action()
    }

    fn bind_job(path: &Path, meeting_id: &str) -> Result<(), String> {
        if path.join("owner.json").exists() {
            let file = plain_file(&path.join("owner.json"))?;
            if file.metadata().map_err(message)?.len() > 4096 {
                return Err("작업 소유 정보가 올바르지 않습니다.".into());
            }
            let owner: JobOwner = serde_json::from_reader(file.take(4096)).map_err(message)?;
            return if owner.meeting_id == meeting_id {
                Ok(())
            } else {
                Err("다른 회의의 작업입니다.".into())
            };
        }
        let mut owner = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.join("owner.json"))
            .map_err(message)?;
        serde_json::to_writer(
            &mut owner,
            &JobOwner {
                meeting_id: meeting_id.into(),
            },
        )
        .map_err(message)?;
        owner.sync_all().map_err(message)
    }

    fn artifact_hash(path: &Path) -> Result<String, String> {
        let mut file = plain_file(path)?;
        if file.metadata().map_err(message)?.len() > 16 * 1024 * 1024 {
            return Err("처리 결과가 너무 큽니다.".into());
        }
        let mut hash = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        let mut total = 0;
        loop {
            let bytes = file.read(&mut buffer).map_err(message)?;
            if bytes == 0 {
                break;
            }
            total += bytes;
            if total > 16 * 1024 * 1024 {
                return Err("처리 결과가 너무 큽니다.".into());
            }
            hash.update(&buffer[..bytes]);
        }
        Ok(format!("{:x}", hash.finalize()))
    }

    pub fn bind_legacy_jobs(&self, core: &CoreHandle) -> grove_core::Result<()> {
        let mut owners: HashMap<String, Option<String>> = HashMap::new();
        for meeting in core.list_meetings()? {
            let path = self
                .root
                .join("meetings")
                .join(&meeting.id)
                .join("transcripts");
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => (),
                _ => return Ok(()),
            }
            let Ok(entries) = fs::read_dir(path) else {
                return Ok(());
            };
            for entry in entries {
                let Ok(entry) = entry else {
                    return Ok(());
                };
                let Ok(kind) = entry.file_type() else {
                    return Ok(());
                };
                if !kind.is_dir() || Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err()
                {
                    return Ok(());
                }
                let raw = entry.path().join("raw.json");
                let Ok(hash) = Self::artifact_hash(&raw) else {
                    return Ok(());
                };
                owners
                    .entry(hash)
                    .and_modify(|owner| {
                        if owner.as_deref() != Some(meeting.id.as_str()) {
                            *owner = None;
                        }
                    })
                    .or_insert(Some(meeting.id.clone()));
            }
        }
        let path = self.root.join("processing");
        if !path.exists() {
            return Ok(());
        }
        let path = directory(&self.root, "processing").map_err(grove_core::CoreError::Storage)?;
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir()
                || Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err()
            {
                continue;
            }
            let path = entry.path();
            if fs::symlink_metadata(path.join("owner.json")).is_ok() {
                continue;
            }
            let Ok(hash) = Self::artifact_hash(&path.join("moss.json")) else {
                continue;
            };
            if let Some(Some(meeting_id)) = owners.get(&hash) {
                Self::bind_job(&path, meeting_id).map_err(grove_core::CoreError::Storage)?;
            }
        }
        Ok(())
    }

    fn owned_jobs(&self) -> grove_core::Result<Vec<(PathBuf, JobOwner)>> {
        let mut jobs = Vec::new();
        let root = self.root.join("processing");
        if !root.exists() {
            return Ok(jobs);
        }
        let root = directory(&self.root, "processing").map_err(grove_core::CoreError::Storage)?;
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err()
                || !entry.file_type()?.is_dir()
            {
                continue;
            }
            let path = entry.path();
            let owner_path = path.join("owner.json");
            let Ok(owner) = plain_file(&owner_path) else {
                continue;
            };
            if owner.metadata()?.len() > 4096 {
                continue;
            }
            let Ok(owner) = serde_json::from_reader::<_, JobOwner>(owner.take(4096)) else {
                continue;
            };
            if Uuid::parse_str(&owner.meeting_id).is_err() {
                continue;
            }
            jobs.push((path, owner));
        }
        Ok(jobs)
    }

    pub fn purge_meeting_jobs(&self, ids: &[String]) -> grove_core::Result<()> {
        for (path, owner) in self.owned_jobs()? {
            if ids.contains(&owner.meeting_id) {
                fs::remove_dir_all(path)?;
            }
        }
        self.records
            .lock()
            .unwrap()
            .retain(|r| !r.meeting_id.as_ref().is_some_and(|id| ids.contains(id)));
        self.revision.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    pub fn cleanup_removed_meeting_jobs(&self, core: &CoreHandle) {
        let canonical_cleanup = core.retry_purge_cleanup();
        let result = (|| -> grove_core::Result<()> {
            let existing = core
                .list_meetings()?
                .into_iter()
                .map(|meeting| meeting.id)
                .collect::<HashSet<_>>();
            for (path, owner) in self.owned_jobs()? {
                if !existing.contains(&owner.meeting_id) {
                    fs::remove_dir_all(path)?;
                }
            }
            self.records.lock().unwrap().retain(|record| {
                record
                    .meeting_id
                    .as_ref()
                    .is_none_or(|id| existing.contains(id))
            });
            self.revision.fetch_add(1, Ordering::AcqRel);
            Ok(())
        })();
        if canonical_cleanup.is_err() || result.is_err() {
            self.update(|progress| {
                *progress = Progress {
                    id: Some(Uuid::new_v4().to_string()),
                    kind: "cleanup".into(),
                    status: "failed".into(),
                    message: "삭제한 회의의 임시 파일을 정리하지 못했습니다.".into(),
                    ..Default::default()
                };
            });
        }
    }

    pub fn upgrade_transcript_sources(&self, core: &CoreHandle) {
        self.record_transcript_upgrade(core.upgrade_transcript_sources());
    }

    fn record_transcript_upgrade(
        &self,
        result: grove_core::Result<grove_core::TranscriptUpgradeReport>,
    ) {
        if result.is_ok_and(|report| report.unavailable == 0) {
            return;
        }
        self.startup_issues.lock().unwrap().push(Progress {
            id: Some(Uuid::new_v4().to_string()),
            kind: "transcript-upgrade".into(),
            status: "failed".into(),
            message: "일부 음성 기록의 화자 정보를 복구하지 못했습니다. 원문과 수정본은 유지했습니다. 앱을 다시 열면 재시도합니다.".into(),
            ..Default::default()
        });
    }

    pub fn notify_startup_issues(&self, notify: impl FnOnce(&[Progress]) -> bool) {
        let mut issues = self.startup_issues.lock().unwrap();
        if !issues.is_empty() && notify(&issues) {
            issues.clear();
        }
    }

    pub fn shutdown(&self) -> Result<(), String> {
        self.shutting_down.store(true, Ordering::Release);
        if let Some(job) = self.running.lock().unwrap().take() {
            self.mark_interrupted(&job);
            job.cancel.store(true, Ordering::Release);
            job.handle
                .join()
                .map_err(|_| "음성 처리 종료를 확인하지 못했습니다.".to_string())?;
        }
        let mut pending = self.pending.lock().unwrap();
        while let Some(job) = pending.front() {
            let mut record = job.record.clone();
            record.status = JobStatus::Interrupted;
            record.message = "앱을 종료하여 작업이 중단되었습니다.".into();
            record.updated_at = now().max(record.updated_at);
            self.save_job(record)?;
            pending.pop_front();
        }
        self.worker_ready()
    }
    pub fn resume(&self) {
        self.shutting_down.store(false, Ordering::Release);
    }

    fn mark_interrupted(&self, job: &Running) {
        if job.handle.is_finished()
            || self.records.lock().unwrap().iter().any(|r| {
                r.id == job.id && matches!(r.status, JobStatus::Cancelled | JobStatus::Cancelling)
            })
        {
            return;
        }
        self.interrupted_jobs.lock().unwrap().insert(job.id.clone());
    }
    pub(super) fn start(
        self: &Arc<Self>,
        kind: &str,
        meeting_id: Option<String>,
        action: impl FnOnce(Arc<Self>, Arc<AtomicBool>, PathBuf) -> Result<Option<String>, String>
            + Send
            + 'static,
    ) -> Result<Progress, String> {
        self.start_scoped(kind, meeting_id, None, None, false, action)
    }

    pub(super) fn start_meeting(
        self: &Arc<Self>,
        kind: &str,
        meeting_id: String,
        core: &CoreHandle,
        required_fingerprint: Option<String>,
        action: impl FnOnce(Arc<Self>, Arc<AtomicBool>, PathBuf) -> Result<Option<String>, String>
            + Send
            + 'static,
    ) -> Result<Progress, String> {
        let fingerprint = Self::source_fingerprint(core, &meeting_id, kind)?;
        if required_fingerprint
            .as_ref()
            .is_some_and(|required| required != &fingerprint)
        {
            return Err("회의 내용이 변경되어 이전 작업을 다시 실행하지 않았습니다.".into());
        }
        self.start_scoped(
            kind,
            Some(meeting_id),
            None,
            Some((core.clone(), fingerprint)),
            false,
            action,
        )
    }

    fn start_scoped(
        self: &Arc<Self>,
        kind: &str,
        meeting_id: Option<String>,
        importing_core: Option<CoreHandle>,
        source: Option<(CoreHandle, String)>,
        include_summary: bool,
        action: impl FnOnce(Arc<Self>, Arc<AtomicBool>, PathBuf) -> Result<Option<String>, String>
            + Send
            + 'static,
    ) -> Result<Progress, String> {
        let mut running = self.running.lock().unwrap();
        self.job_ready()?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("앱을 종료하고 있습니다.".into());
        }
        let supported = if kind.starts_with("summary") {
            crate::summary::availability(self).0
        } else if matches!(kind, "diarize" | "speaker-model") {
            crate::diarization::availability(self).0
        } else {
            self.supported()
        };
        if !supported {
            return Err("이 기기의 처리 실행기가 아직 준비되지 않았습니다.".into());
        }
        let mut queue = self.pending.lock().unwrap();
        if queue.len() >= 32 {
            return Err("대기 중인 작업을 마치거나 취소한 뒤 다시 시도해 주세요.".into());
        }
        if queue
            .iter()
            .any(|job| job.record.kind == kind && job.record.meeting_id == meeting_id)
            || (running
                .as_ref()
                .is_some_and(|job| !job.handle.is_finished())
                && {
                    let p = self.progress.lock().unwrap();
                    p.kind == kind && p.meeting_id == meeting_id
                })
        {
            return Err("이미 같은 작업이 진행 중이거나 대기 중입니다.".into());
        }
        let import_existing = importing_core
            .map(|core| {
                core.list_meetings()
                    .map(|meetings| meetings.into_iter().map(|m| m.id).collect::<HashSet<_>>())
                    .map_err(message)
            })
            .transpose()?;
        let id = Uuid::new_v4().to_string();
        let created_at = now();
        let record = JobRecord {
            schema_version: 1,
            id: id.clone(),
            kind: kind.into(),
            meeting_id: meeting_id.clone(),
            source_fingerprint: source.as_ref().map(|(_, fingerprint)| fingerprint.clone()),
            include_summary,
            acknowledged: false,
            failure_kind: None,
            status: JobStatus::Queued,
            phase: None,
            message: "대기 중".into(),
            created_at,
            updated_at: created_at,
        };
        if let Some(meeting_id) = &record.meeting_id {
            let path = directory(&self.root, &format!("processing/{}", record.id))?;
            Self::bind_job(&path, meeting_id)?;
        }
        let validate = source.map(|(core, fingerprint)| {
            let id = meeting_id.unwrap(); let kind = kind.to_string();
            Box::new(move || {
                if Self::source_fingerprint(&core, &id, &kind)? != fingerprint {
                    return Err("회의 내용이 변경되어 대기하던 작업을 실행하지 않았습니다. 현재 내용을 확인해 주세요.".into());
                }
                Ok(())
            }) as Box<dyn Fn() -> Result<(), String> + Send>
        });
        self.save_job(record.clone())?;
        let job = Pending {
            record,
            action: Box::new(action),
            import_existing,
            validate,
        };
        if !queue.is_empty()
            || running.as_ref().is_some_and(|r| !r.handle.is_finished())
            || (self.recording.load(Ordering::Acquire) && !model_job(kind))
        {
            queue.push_back(job);
            drop(queue);
            self.update(|_| ());
        } else {
            if let Some(old) = running.take() {
                old.handle
                    .join()
                    .map_err(|_| "이전 작업 종료를 확인하지 못했습니다.")?;
            }
            drop(queue);
            *running = Some(self.launch_checked(job)?);
        }
        Ok(self.snapshot())
    }

    fn launch(self: &Arc<Self>, mut job: Pending) -> Result<Running, String> {
        self.job_ready()?;
        let id = job.record.id.clone();
        let path = directory(&self.root, &format!("processing/{id}"))?;
        if let Some(meeting_id) = &job.record.meeting_id {
            Self::bind_job(&path, meeting_id)?;
        }
        job.record.status = JobStatus::Running;
        job.record.message = "준비 중".into();
        job.record.updated_at = now().max(job.record.updated_at);
        self.save_job(job.record.clone())?;
        self.update(|p| {
            *p = Progress {
                revision: p.revision,
                id: Some(id.clone()),
                kind: job.record.kind.clone(),
                meeting_id: job.record.meeting_id.clone(),
                status: "running".into(),
                message: "준비 중".into(),
                ..Progress::default()
            }
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let this = self.clone();
        let job_id = id.clone();
        let handle = thread::Builder::new()
            .name(format!("nero-job-{id}"))
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if let Some(validate) = job.validate {
                        validate()?;
                    }
                    (job.action)(this.clone(), flag.clone(), path)
                }))
                .unwrap_or_else(|_| {
                    this.cleanup_failed();
                    Err("처리 작업이 예기치 않게 종료되었습니다. 기존 결과는 유지했습니다.".into())
                });
                let interrupted = this.interrupted_jobs.lock().unwrap().remove(&job_id);
                let user_cancelled = this.records.lock().unwrap().iter().any(|r| {
                    r.id == job_id
                        && matches!(r.status, JobStatus::Cancelled | JobStatus::Cancelling)
                });
                if user_cancelled {
                    this.update(|p| {
                        p.status = "cancelled".into();
                        p.message = "취소했습니다.".into();
                    });
                    return;
                }
                this.update(|p| match result {
                    Ok(meeting) => {
                        if meeting.is_some() {
                            p.meeting_id = meeting;
                        }
                        p.status = "completed".into();
                        p.message = "완료".into();
                    }
                    Err(error) => {
                        p.status = if p.phase.as_deref() == Some("saving") {
                            "needsReview"
                        } else if interrupted {
                            "interrupted"
                        } else if flag.load(Ordering::Acquire)
                            && !this.worker_cleanup_failed.load(Ordering::Acquire)
                        {
                            "cancelled"
                        } else {
                            "failed"
                        }
                        .into();
                        p.message = if p.phase.as_deref() == Some("saving") {
                            "저장 완료 여부를 확인하지 못했습니다. 회의의 결과를 확인해 주세요."
                                .into()
                        } else if interrupted {
                            "녹음 또는 앱 종료로 작업이 중단되었습니다. 다시 시도할 수 있습니다."
                                .into()
                        } else {
                            error
                        };
                    }
                });
            })
            .map_err(message)?;
        Ok(Running {
            id,
            cancel,
            handle,
            import_existing: job.import_existing,
        })
    }

    fn launch_checked(self: &Arc<Self>, job: Pending) -> Result<Running, String> {
        let mut record = job.record.clone();
        self.launch(job).map_err(|error| {
            record.status = JobStatus::Failed;
            record.message = error.clone();
            record.updated_at = now().max(record.updated_at);
            if let Err(failure) = self.save_job(record) {
                *self.journal_error.lock().unwrap() = Some(failure);
            }
            error
        })
    }
    pub fn prepare_model(self: &Arc<Self>) -> Result<Progress, String> {
        self.prepare_named_model(MODEL, "model", "MOSS")
    }
    pub fn require_meeting_models(&self) -> Result<(), String> {
        meeting_models_ready(&self.snapshot())
    }
    pub fn prepare_meeting_models(
        self: &Arc<Self>,
        include_summary: bool,
    ) -> Result<Progress, String> {
        let progress = self.snapshot();
        if !progress.supported || (include_summary && !progress.summary_supported) {
            return Err("이 환경에서는 음성 인식과 회의 요약을 아직 사용할 수 없습니다.".into());
        }
        let required = self.remaining_model_bytes(MODEL, "MOSS")?
            + if include_summary {
                self.remaining_model_bytes(crate::summary::MANIFEST, "Gemma4-E4B")?
            } else {
                0
            };
        crate::storage_space::check(&self.root, required)?;
        self.start_scoped(
            "setup",
            None,
            None,
            None,
            include_summary,
            move |this, cancel, _path| {
                let progress = this.snapshot();
                let total = progress.model_bytes
                    + if include_summary {
                        progress.summary_bytes
                    } else {
                        0
                    };
                let first = this.prepare_files(MODEL, "MOSS", &cancel, 0, total)?;
                if include_summary {
                    this.prepare_files(
                        crate::summary::MANIFEST,
                        "Gemma4-E4B",
                        &cancel,
                        first,
                        total,
                    )?;
                }
                Ok(None)
            },
        )
    }
    pub(super) fn prepare_named_model(
        self: &Arc<Self>,
        manifest: &'static str,
        kind: &str,
        label: &'static str,
    ) -> Result<Progress, String> {
        crate::storage_space::check(&self.root, self.remaining_model_bytes(manifest, label)?)?;
        self.start(kind, None, move |this, cancel, _path| {
            let total = serde_json::from_str::<Model>(manifest)
                .map_err(message)?
                .files
                .iter()
                .map(|file| file.bytes)
                .sum();
            this.prepare_files(manifest, label, &cancel, 0, total)?;
            Ok(None)
        })
    }
    fn remaining_model_bytes(&self, manifest: &str, label: &str) -> Result<u64, String> {
        let model: Model = serde_json::from_str(manifest).map_err(message)?;
        let target = self.root.join(format!("models/{label}-{}", model.revision));
        let length = |path: PathBuf| {
            fs::symlink_metadata(path)
                .ok()
                .filter(|meta| meta.is_file() && !meta.file_type().is_symlink())
                .map(|meta| meta.len())
        };
        Ok(model
            .files
            .iter()
            .map(|file| {
                if length(target.join(&file.name)) == Some(file.bytes) {
                    return 0;
                }
                let partial = length(target.join(format!("{}.part", file.name)))
                    .filter(|bytes| *bytes <= file.bytes)
                    .unwrap_or(0);
                file.bytes - partial
            })
            .sum())
    }
    fn prepare_files(
        &self,
        manifest: &str,
        label: &str,
        cancel: &AtomicBool,
        offset: u64,
        total: u64,
    ) -> Result<u64, String> {
        let model: Model = serde_json::from_str(manifest).map_err(message)?;
        let target = directory(&self.root, &format!("models/{label}-{}", model.revision))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(message)?;
        runtime.block_on(async {
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(20))
                .read_timeout(Duration::from_secs(30))
                .build()
                .map_err(message)?;

            let mut done = 0u64;
            for item in &model.files {
                cancelled(cancel)?;
                self.update(|p| {
                    p.model_stage = Some(
                        match label {
                            "MOSS" => "speech",
                            "Nemotron3" => "speakers",
                            _ => "summary",
                        }
                        .into(),
                    );
                    p.message = match label {
                        "MOSS" => "음성 인식 모델 확인 중",
                        "Nemotron3" => "화자 구분 모델 확인 중",
                        _ => "요약 모델 확인 중",
                    }
                    .into();
                    p.bytes = offset + done;
                    p.total_bytes = total;
                });
                let destination = target.join(&item.name);
                let matches = matches_file(&destination, item, cancel);
                cancelled(cancel)?;
                if !matches {
                    self.update(|p| {
                        p.message = match label {
                            "MOSS" => "음성 인식 모델 다운로드 중",
                            "Nemotron3" => "화자 구분 모델 다운로드 중",
                            _ => "요약 모델 다운로드 중",
                        }
                        .into()
                    });
                    let url = format!(
                        "https://huggingface.co/{}/resolve/{}/{}",
                        model.repository, model.revision, item.name
                    );
                    crate::downloads::item(&client, &url, &target, item, cancel, |bytes| {
                        self.update(|p| p.bytes = offset + done + bytes)
                    })
                    .await?;
                }
                done += item.bytes;
                self.update(|p| p.bytes = offset + done);
            }
            Ok::<(), String>(())
        })?;
        cancelled(cancel)?;
        let marker = target.join(format!(".verified-{}", Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
            .map_err(message)?;
        file.write_all(manifest.as_bytes())
            .map_err(crate::storage_space::write_error)?;
        file.sync_all().map_err(crate::storage_space::write_error)?;
        drop(file);
        fs::rename(marker, target.join("verified.json")).map_err(message)?;
        Ok(model.files.iter().map(|file| file.bytes).sum())
    }
    fn run_worker(
        &self,
        executable: &Path,
        args: &[&Path],
        log: &Path,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        cancelled(cancel)?;
        let output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(log)
            .map_err(message)?;
        let mut command = Command::new(executable);
        command
            .args(args)
            .current_dir(log.parent().unwrap())
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .stdout(Stdio::from(output.try_clone().map_err(message)?))
            .stderr(Stdio::from(output));
        let moss = executable
            .file_name()
            .is_some_and(|name| name == "MossHarness");
        let mut child = Worker(command.spawn().map_err(|_| {
            if moss {
                self.update(|progress| {
                    progress.failure_kind =
                        Some(crate::summary_failure::FailureKind::EngineUnavailable)
                });
            }
            "음성 인식을 시작하지 못했습니다.".to_string()
        })?);
        loop {
            if cancelled(cancel).is_err() {
                self.stop_worker(&mut child)?;
                return Err("처리를 취소했습니다.".into());
            }
            match child.0.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(_)) => return Err(self.worker_failure_message(log, moss)),
                Ok(None) => thread::sleep(Duration::from_millis(100)),
                Err(_) => {
                    self.stop_worker(&mut child)?;
                    return Err(self.worker_failure_message(log, moss));
                }
            }
        }
    }

    fn stop_worker(&self, worker: &mut Worker) -> Result<(), String> {
        worker.stop().map(|_| ()).map_err(|_| {
            self.cleanup_failed();
            self.update(|progress| {
                progress.failure_kind = Some(crate::summary_failure::FailureKind::Cleanup)
            });
            "음성 처리 실행기의 종료를 확인하지 못했습니다. 앱을 다시 열어 주세요.".into()
        })
    }

    fn worker_failure_message(&self, log: &Path, moss: bool) -> String {
        if moss {
            let diagnostic = plain_file(log)
                .and_then(|mut file| {
                    let length = file.metadata().map_err(message)?.len();
                    file.seek(SeekFrom::Start(length.saturating_sub(64 * 1024)))
                        .map_err(message)?;
                    let mut bytes = Vec::new();
                    file.take(64 * 1024)
                        .read_to_end(&mut bytes)
                        .map_err(message)?;
                    Ok(String::from_utf8_lossy(&bytes).into_owned())
                })
                .ok()
                .and_then(|log| moss_worker_failure(&log));
            if let Some((kind, text)) = diagnostic {
                self.update(|progress| progress.failure_kind = Some(kind));
                return text.into();
            }
            self.update(|progress| {
                progress.failure_kind = Some(crate::summary_failure::FailureKind::EngineUnavailable)
            });
        }
        "음성 처리를 완료하지 못했습니다. 원본과 기존 음성 기록은 보관했습니다.".into()
    }
    pub fn recording_start<T>(
        &self,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let mut running = self.running.lock().unwrap();
        self.worker_ready()?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("앱을 종료하고 있습니다.".into());
        }
        if let Some(job) = running.as_ref() {
            if !job.handle.is_finished() && self.progress.lock().unwrap().kind == "import" {
                return Err("음성 가져오기가 끝난 뒤 녹음을 시작해 주세요.".into());
            }
        }
        let downloading = model_job(&self.progress.lock().unwrap().kind);
        if !downloading {
            if let Some(job) = running.take() {
                self.mark_interrupted(&job);
                job.cancel.store(true, Ordering::Release);
                job.handle
                    .join()
                    .map_err(|_| "처리 작업의 종료를 확인하지 못했습니다.")?;
            }
        }
        self.worker_ready()?;
        let result = action();
        if result.is_ok() {
            self.recording.store(true, Ordering::Release);
        }
        result
    }

    pub fn acknowledge(&self, id: &str) -> Result<Progress, String> {
        let _running = self.running.lock().unwrap();
        let mut record = self
            .records
            .lock()
            .unwrap()
            .iter()
            .find(|record| record.id == id)
            .cloned()
            .ok_or("작업을 찾을 수 없습니다.")?;
        if !matches!(
            record.status,
            JobStatus::Failed | JobStatus::Interrupted | JobStatus::NeedsReview
        ) {
            return Err("진행 중인 작업의 알림은 닫을 수 없습니다.".into());
        }
        record.acknowledged = true;
        record.updated_at = now().max(record.updated_at);
        self.save_job(record)?;
        Ok(self.snapshot())
    }

    pub fn retry(self: &Arc<Self>, core: CoreHandle, id: &str) -> Result<Progress, String> {
        let record = self
            .records
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.id == id)
            .cloned()
            .ok_or("작업을 찾을 수 없습니다.")?;
        if !matches!(record.status, JobStatus::Interrupted | JobStatus::Failed)
            || record.kind == "import"
        {
            return Err(
                "현재 작업은 다시 실행할 수 없습니다. 회의의 저장 결과를 먼저 확인해 주세요."
                    .into(),
            );
        }
        if let Some(meeting_id) = &record.meeting_id {
            if record.source_fingerprint.as_ref()
                != Some(&Self::source_fingerprint(&core, meeting_id, &record.kind)?)
            {
                return Err("회의 내용이 변경되어 이전 작업을 다시 실행하지 않았습니다. 현재 회의에서 작업을 시작해 주세요.".into());
            }
        }
        match record.kind.as_str() {
            "setup" => self.prepare_meeting_models(record.include_summary)?,
            "model" => self.prepare_model()?,
            "summary-model" => self.prepare_summary()?,
            "speaker-model" => self.prepare_diarization()?,
            "transcribe" => self.transcribe_bound(
                core,
                record.meeting_id.clone().ok_or("회의가 없습니다.")?,
                record.source_fingerprint.clone(),
            )?,
            "summary" => self.summarize_bound(
                core,
                record.meeting_id.clone().ok_or("회의가 없습니다.")?,
                record.source_fingerprint.clone(),
            )?,
            "diarize" => {
                let meeting_id = record.meeting_id.clone().ok_or("회의가 없습니다.")?;
                let expected = core
                    .transcript(&meeting_id)
                    .map_err(message)?
                    .ok_or("음성 기록이 없습니다.")?
                    .version;
                self.diarize_bound(
                    core,
                    meeting_id,
                    expected,
                    record.source_fingerprint.clone(),
                )?
            }
            _ => return Err("지원하지 않는 작업입니다.".into()),
        };
        let mut old = record;
        old.status = JobStatus::Cancelled;
        old.message = "새 작업으로 다시 시도했습니다.".into();
        old.updated_at = now().max(old.updated_at);
        self.save_job(old)?;
        self.update(|_| ());
        Ok(self.snapshot())
    }
    pub fn import(self: &Arc<Self>, core: CoreHandle, source: PathBuf) -> Result<Progress, String> {
        self.start_scoped(
            "import",
            None,
            Some(core.clone()),
            None,
            false,
            move |this, cancel, path| {
                let source = source.canonicalize().map_err(message)?;
                let mut input = plain_file(&source)?;
                let ext = source
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("audio");
                if !["wav", "m4a", "mp3", "aac", "aiff", "aif", "caf"]
                    .contains(&ext.to_ascii_lowercase().as_str())
                {
                    return Err("지원하지 않는 음성 파일입니다.".into());
                }
                let copied = directory(&path, "source")?
                    .join(source.file_name().ok_or("음성 파일 이름이 필요합니다.")?);
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&copied)
                    .map_err(message)?;
                let before = input.metadata().map_err(message)?;
                let mut buffer = [0u8; 128 * 1024];
                let mut bytes = 0u64;
                loop {
                    cancelled(&cancel)?;
                    let n = input.read(&mut buffer).map_err(message)?;
                    if n == 0 {
                        break;
                    }
                    output.write_all(&buffer[..n]).map_err(message)?;
                    bytes += n as u64;
                }
                output.sync_all().map_err(message)?;
                drop(output);
                if bytes != before.len()
                    || input.metadata().map_err(message)?.modified().ok() != before.modified().ok()
                {
                    return Err("가져오는 동안 파일이 변경됐습니다. 다시 선택해 주세요.".into());
                }
                this.update(|p| p.message = "음성을 준비하고 있습니다.".into());
                let prepared = path.join("analysis.wav");
                this.run_worker(
                    &this.worker("AudioPrepare"),
                    &[&copied, &prepared],
                    &path.join("prepare.log"),
                    &cancel,
                )?;
                cancelled(&cancel)?;
                let title = source
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("가져온 회의")
                    .chars()
                    .take(200)
                    .collect::<String>();
                let meeting = this.commit_transcript_result(&cancel, || {
                    core.import_audio(&title, &copied, &prepared)
                        .map_err(message)
                })?;
                this.update(|progress| progress.meeting_id = Some(meeting.id.clone()));
                Self::bind_job(&path, &meeting.id)?;
                Ok(Some(meeting.id))
            },
        )
    }
    pub fn transcribe(
        self: &Arc<Self>,
        core: CoreHandle,
        meeting_id: String,
    ) -> Result<Progress, String> {
        self.transcribe_bound(core, meeting_id, None)
    }
    fn transcribe_bound(
        self: &Arc<Self>,
        core: CoreHandle,
        meeting_id: String,
        required_fingerprint: Option<String>,
    ) -> Result<Progress, String> {
        let audio = core.audio_path(&meeting_id).map_err(message)?;
        let expected = core
            .transcript(&meeting_id)
            .map_err(message)?
            .map(|d| d.version);
        if !self.snapshot().model_ready {
            return Err("먼저 음성 인식 모델을 준비해 주세요.".into());
        }
        let source_core = core.clone();
        self.start_meeting(
            "transcribe",
            meeting_id.clone(),
            &source_core,
            required_fingerprint,
            move |this, cancel, path| {
                if core.audio_path(&meeting_id).map_err(message)? != audio
                    || core
                        .transcript(&meeting_id)
                        .map_err(message)?
                        .map(|d| d.version)
                        != expected
                {
                    return Err("회의 내용이 변경되어 음성 인식을 시작하지 않았습니다.".into());
                }
                // Freeze the local profile for this recognition run. A missing or invalid
                // optional profile must never prevent preserving the recognized baseline.
                let terminology = crate::terminology::load_snapshot(&this);
                core.audio_path(&meeting_id).map_err(message)?;
                if core.snapshot().map_err(message)?.session.is_some_and(|s| {
                    matches!(
                        s.state,
                        grove_core::SessionState::Recording | grove_core::SessionState::Paused
                    )
                }) {
                    return Err("녹음을 마친 뒤 음성 인식을 시작해 주세요.".into());
                }
                this.update(|p| {
                    p.phase = Some("preparing".into());
                    p.message = "모델을 확인하고 있습니다.".into();
                });
                let model = this.model();
                let directory = this.model_dir()?;
                for item in &model.files {
                    cancelled(&cancel)?;
                    let matches = matches_file(&directory.join(&item.name), item, &cancel);
                    cancelled(&cancel)?;
                    if !matches {
                        invalidate_readiness(&directory)?;
                        return Err("모델 파일을 다시 준비해 주세요.".into());
                    }
                }
                let prepared = path.join("analysis.wav");
                this.run_worker(
                    &this.worker("AudioPrepare"),
                    &[&audio, &prepared],
                    &path.join("prepare.log"),
                    &cancel,
                )?;
                this.update(|p| {
                    p.phase = Some("recognizing".into());
                    p.message = "텍스트로 변환 중".into();
                });
                let output = path.join("moss.json");
                this.run_worker(
                    &this.worker("moss/MossHarness"),
                    &[&prepared, &output, &directory],
                    &path.join("moss.log"),
                    &cancel,
                )?;
                cancelled(&cancel)?;
                if fs::metadata(&output).map_err(message)?.len() > 16 * 1024 * 1024 {
                    return Err("음성 기록 결과가 너무 큽니다. 원문 결과는 보관했습니다.".into());
                }
                let raw = fs::read(output).map_err(message)?;
                cancelled(&cancel)?;
                let source = this.checkpoint_transcript(&cancel, || {
                    core.commit_whole_transcript(&meeting_id, expected, raw)
                })?;
                crate::terminology::correct(
                    &this,
                    &core,
                    &meeting_id,
                    source,
                    terminology,
                    &path,
                    &cancel,
                )?;
                Ok(Some(meeting_id))
            },
        )
    }

    pub fn transcribe_recording(
        self: &Arc<Self>,
        core: CoreHandle,
        snapshot: &Snapshot,
    ) -> Result<Option<Progress>, String> {
        let Some(meeting_id) = self.recording_transcription.lock().unwrap().claim(snapshot) else {
            return Ok(None);
        };
        self.recording.store(false, Ordering::Release);
        self.transcribe(core, meeting_id).map(Some)
    }
}

#[cfg(test)]
#[path = "processing_job_tests.rs"]
mod job_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn completed_recording(session_id: &str, meeting_id: &str) -> Snapshot {
        serde_json::from_value(serde_json::json!({
            "revision": 1,
            "folders": [],
            "meetings": [{
                "id": meeting_id, "title": "Meeting", "notes": "", "createdAt": 1, "updatedAt": 2,
                "status": "completed", "error": null,
                "audio": { "sessionId": session_id, "relativePath": "audio.wav", "frames": 16000, "sampleRate": 16000, "channels": 1 }
            }],
            "session": {
                "id": session_id, "meetingId": meeting_id, "deviceId": "mic", "deviceName": "Microphone",
                "state": "completed", "elapsedMs": 1000, "savedFrames": 16000,
                "sampleRate": 16000, "channels": 1, "error": null
            }
        })).unwrap()
    }

    #[test]
    fn automatic_transcription_claims_each_finalized_recording_once() {
        let mut automatic = RecordingTranscription::default();
        let first = completed_recording("first", "one");
        assert_eq!(automatic.claim(&first), Some("one".into()));
        assert_eq!(automatic.claim(&first), None);
        let second = completed_recording("second", "two");
        assert_eq!(automatic.claim(&second), Some("two".into()));
        assert_eq!(automatic.claim(&first), None);
        // Cancellation or start failure never releases a recording for an automatic retry.
        assert_eq!(automatic.claim(&second), None);
    }

    #[test]
    fn automatic_transcription_requires_completed_audio_from_the_same_session() {
        for state in [
            SessionState::Recording,
            SessionState::Paused,
            SessionState::Interrupted,
        ] {
            let mut automatic = RecordingTranscription::default();
            let mut snapshot = completed_recording("capture", "one");
            snapshot.session.as_mut().unwrap().state = state;
            assert_eq!(automatic.claim(&snapshot), None);
            snapshot.session.as_mut().unwrap().state = SessionState::Completed;
            snapshot.meetings[0].audio.as_mut().unwrap().session_id = "previous".into();
            assert_eq!(automatic.claim(&snapshot), None);
            snapshot.meetings[0].audio.as_mut().unwrap().session_id = "capture".into();
            assert_eq!(automatic.claim(&snapshot), Some("one".into()));
        }
        let mut automatic = RecordingTranscription::default();
        let mut snapshot = completed_recording("capture", "one");
        snapshot.meetings[0].audio = None;
        assert_eq!(automatic.claim(&snapshot), None);
        snapshot.meetings.clear();
        assert_eq!(automatic.claim(&snapshot), None);
    }

    #[test]
    fn identical_processing_progress_does_not_need_a_new_event() {
        let first = Progress {
            kind: "transcribe".into(),
            status: "running".into(),
            ..Default::default()
        };
        assert!(first == first.clone());
        let mut changed = first.clone();
        changed.status = "cancelled".into();
        assert!(first != changed);
        changed = first.clone();
        changed.model_ready = true;
        assert!(first != changed);
    }

    fn setup() -> (tempfile::TempDir, Arc<Processing>) {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let process = Processing::new(root.clone(), root.clone());
        (temporary, process)
    }

    fn raw_transcript(text: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "model":"OpenMOSS-Team/MOSS-Transcribe-Diarize",
            "segments":[{"start":0.0,"end":1.0,"text":text,"speaker_id":"S1"}],
            "hitTokenLimit":false,"hasUnparsedText":false,
            "processedAudioFrames":16000,"inputAudioFrames":16000,"audioDurationSeconds":1.0
        }))
        .unwrap()
    }

    fn edited_baseline(process: &Processing) -> (CoreHandle, String, grove_core::Transcript) {
        let core = CoreHandle::open(&process.root).unwrap();
        let input = process.root.join("synthetic-input.wav");
        let data_size = 16000u32 * 2;
        let mut wav = Vec::from(b"RIFF".as_slice());
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend_from_slice(&32000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        wav.resize(wav.len() + data_size as usize, 0);
        fs::write(&input, wav).unwrap();
        let meeting = core
            .import_audio("Synthetic meeting", &input, &input)
            .unwrap();
        let source = core
            .commit_transcript(&meeting.id, None, raw_transcript("원문"))
            .unwrap();
        let edited = core
            .edit_transcript(&meeting.id, source.version, vec!["사용자 수정".into()])
            .unwrap();
        (core, meeting.id, edited)
    }

    #[test]
    fn accepted_cancel_before_baseline_checkpoint_preserves_existing_edits_and_audio() {
        let (_temporary, process) = setup();
        let (core, id, source) = edited_baseline(&process);
        let current = process
            .root
            .join("meetings")
            .join(&id)
            .join("transcripts")
            .join(&source.version.id)
            .join("current.json");
        let before = fs::read(&current).unwrap();
        let audio = core.audio_path(&id).unwrap();
        let original_audio = fs::read(&audio).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let (finish, wait) = std::sync::mpsc::channel();
        *process.running.lock().unwrap() = Some(Running {
            id: "recognition".into(),
            cancel: cancel.clone(),
            handle: thread::spawn(move || {
                wait.recv().unwrap();
            }),
            import_existing: None,
        });
        process.cancel("recognition").unwrap();
        assert!(process
            .checkpoint_transcript(&cancel, || {
                core.commit_transcript(&id, Some(source.version.clone()), raw_transcript("새 결과"))
            })
            .is_err());
        assert_eq!(fs::read(current).unwrap(), before);
        assert_eq!(fs::read(audio).unwrap(), original_audio);
        assert_eq!(
            core.transcript(&id).unwrap().unwrap().segments[0].text,
            "사용자 수정"
        );
        finish.send(()).unwrap();
        process.shutdown().unwrap();
    }

    #[test]
    fn baseline_checkpoint_rejects_a_stale_source_without_overwriting_new_edits() {
        let (_temporary, process) = setup();
        let (core, id, source) = edited_baseline(&process);
        let edited = core
            .edit_transcript(&id, source.version.clone(), vec!["새 사용자 수정".into()])
            .unwrap();
        let current = process
            .root
            .join("meetings")
            .join(&id)
            .join("transcripts")
            .join(&source.version.id)
            .join("current.json");
        let before = fs::read(&current).unwrap();
        let cancel = AtomicBool::new(false);
        assert!(process
            .checkpoint_transcript(&cancel, || {
                core.commit_transcript(&id, Some(source.version), raw_transcript("새 모델 결과"))
            })
            .is_err());
        assert_eq!(fs::read(current).unwrap(), before);
        assert_eq!(
            core.transcript(&id).unwrap().unwrap().version,
            edited.version
        );
        assert_eq!(
            core.transcript(&id).unwrap().unwrap().segments[0].text,
            "새 사용자 수정"
        );
        assert!(!process.progress.lock().unwrap().committing);
    }

    #[test]
    fn cancelling_optional_postprocessing_retains_the_new_baseline_and_previous_edited_version() {
        let (_temporary, process) = setup();
        let (core, id, source) = edited_baseline(&process);
        let previous = process
            .root
            .join("meetings")
            .join(&id)
            .join("transcripts")
            .join(&source.version.id)
            .join("current.json");
        let previous_bytes = fs::read(&previous).unwrap();
        let audio = core.audio_path(&id).unwrap();
        let audio_before = fs::read(&audio).unwrap();
        let cancel = AtomicBool::new(false);
        let baseline = process
            .checkpoint_transcript(&cancel, || {
                core.commit_transcript(&id, Some(source.version), raw_transcript("새 음성 기록"))
            })
            .unwrap();
        cancel.store(true, Ordering::Release);
        assert!(process
            .commit_transcript_result(&cancel, || -> Result<(), String> {
                panic!("cancelled postprocessing cannot replace the baseline")
            })
            .is_err());
        let current = core.transcript(&id).unwrap().unwrap();
        assert_eq!(current.version, baseline.version);
        assert_eq!(current.segments[0].text, "새 음성 기록");
        assert_eq!(fs::read(previous).unwrap(), previous_bytes);
        assert_eq!(fs::read(audio).unwrap(), audio_before);
    }

    #[test]
    fn baseline_checkpoint_finishes_before_accepting_cancel_and_keeps_postprocessing_cancellable() {
        let (_temporary, process) = setup();
        let (entered, in_commit) = std::sync::mpsc::channel();
        let (release, commit_release) = std::sync::mpsc::channel();
        let (finish, finish_job) = std::sync::mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let saved = Arc::new(AtomicBool::new(false));
        let worker_process = process.clone();
        let worker_cancel = cancel.clone();
        let worker_saved = saved.clone();
        let handle = thread::spawn(move || {
            worker_process
                .checkpoint_transcript(&worker_cancel, || {
                    entered.send(()).unwrap();
                    commit_release.recv().unwrap();
                    assert!(!worker_cancel.load(Ordering::Acquire));
                    worker_saved.store(true, Ordering::Release);
                    Ok(())
                })
                .unwrap();
            finish_job.recv().unwrap();
        });
        *process.running.lock().unwrap() = Some(Running {
            id: "recognition".into(),
            cancel: cancel.clone(),
            handle,
            import_existing: None,
        });
        in_commit.recv_timeout(Duration::from_secs(3)).unwrap();
        let (attempting, attempted) = std::sync::mpsc::channel();
        let (accepted, accepted_cancel) = std::sync::mpsc::channel();
        let cancelling_process = process.clone();
        let cancelling = thread::spawn(move || {
            attempting.send(()).unwrap();
            let result = cancelling_process.cancel("recognition");
            accepted.send(result).unwrap();
        });
        attempted.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(accepted_cancel
            .recv_timeout(Duration::from_millis(50))
            .is_err());
        assert!(!cancel.load(Ordering::Acquire));
        release.send(()).unwrap();
        accepted_cancel
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        assert!(saved.load(Ordering::Acquire));
        assert!(cancel.load(Ordering::Acquire));
        assert!(!process.progress.lock().unwrap().committing);
        assert!(process
            .commit_transcript_result(&cancel, || -> Result<(), String> {
                panic!("cancel after baseline must prevent an optional correction commit")
            })
            .is_err());
        finish.send(()).unwrap();
        cancelling.join().unwrap();
        process.shutdown().unwrap();
    }
    #[test]
    fn restoring_transcripts_rejects_related_processing_without_cancelling_it() {
        let (_temp, process) = setup();
        let (release, wait) = std::sync::mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let handle = thread::spawn(move || {
            wait.recv().unwrap();
        });
        *process.running.lock().unwrap() = Some(Running {
            id: "summary".into(),
            cancel: cancel.clone(),
            handle,
            import_existing: None,
        });
        process.update(|progress| progress.meeting_id = Some("meeting-a".into()));
        assert!(process
            .transcript_restore("meeting-a", || -> grove_core::Result<()> {
                panic!("a related running job must hold restore ownership")
            })
            .is_err());
        assert!(!cancel.load(Ordering::Acquire));
        assert!(process.transcript_restore("meeting-b", || Ok(())).is_ok());
        release.send(()).unwrap();
        process
            .running
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .handle
            .join()
            .unwrap();
        assert!(process.transcript_restore("meeting-a", || Ok(())).is_ok());
        process.cleanup_failed();
        assert!(process
            .transcript_restore("meeting-a", || -> grove_core::Result<()> {
                panic!("unconfirmed worker termination must block restore")
            })
            .is_err());
    }

    #[test]
    fn shutdown_rejects_new_jobs_even_without_a_running_worker() {
        let (_temp, process) = setup();
        process.shutdown().unwrap();
        assert_eq!(
            process.prepare_model().err().unwrap(),
            "앱을 종료하고 있습니다."
        );
        assert!(process.running.lock().unwrap().is_none());
    }
    #[test]
    fn failed_worker_cleanup_blocks_new_capture_even_when_it_happens_during_cancellation() {
        let (_temp, process) = setup();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let observer = process.clone();
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(1));
            }
            observer.cleanup_failed();
        });
        *process.running.lock().unwrap() = Some(Running {
            id: "summary".into(),
            cancel,
            handle,
            import_existing: None,
        });
        process.update(|p| p.kind = "summary".into());
        assert!(process
            .recording_start(|| -> Result<(), String> { panic!("capture must not start") })
            .is_err());
        assert!(process.worker_ready().is_err());
    }
    #[test]
    fn cleanup_failure_during_cancellation_blocks_removal_and_successful_shutdown() {
        let (_temp, process) = setup();
        let core = CoreHandle::open(&process.root).unwrap();
        let meeting = core.create_meeting("Meeting", "").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let observer = process.clone();
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                thread::yield_now();
            }
            observer.cleanup_failed();
        });
        *process.running.lock().unwrap() = Some(Running {
            id: "summary".into(),
            cancel,
            handle,
            import_existing: None,
        });
        process.update(|p| p.meeting_id = Some(meeting.id.clone()));
        assert!(process
            .meeting_removal(&core, &[meeting.id], || -> grove_core::Result<()> {
                panic!("removal must not run after unconfirmed termination")
            })
            .is_err());
        assert!(process.shutdown().is_err());
    }
    #[test]
    fn accepted_cancellation_prevents_commit_and_commit_rejects_late_cancellation() {
        let (_temp, process) = setup();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                thread::yield_now();
            }
        });
        *process.running.lock().unwrap() = Some(Running {
            id: "summary".into(),
            cancel: cancel.clone(),
            handle,
            import_existing: None,
        });
        process.cancel("summary").unwrap();
        assert!(process
            .commit_summary(&cancel, || -> Result<(), String> {
                panic!("accepted cancellation must prevent commit")
            })
            .is_err());
        process.shutdown().unwrap();
        process.resume();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                thread::yield_now();
            }
        });
        *process.running.lock().unwrap() = Some(Running {
            id: "summary".into(),
            cancel: cancel.clone(),
            handle,
            import_existing: None,
        });
        assert_eq!(
            process
                .commit_summary(&cancel, || {
                    assert!(process.cancel("summary").is_err());
                    assert!(!cancel.load(Ordering::Acquire));
                    Ok("saved")
                })
                .unwrap(),
            "saved"
        );
        assert!(process.snapshot().summary_committing);
        process.shutdown().unwrap();
    }
    #[test]
    fn terminology_commit_shares_cancellation_gate_without_summary_state() {
        let (_temp, process) = setup();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                thread::yield_now();
            }
        });
        *process.running.lock().unwrap() = Some(Running {
            id: "recognition".into(),
            cancel: cancel.clone(),
            handle,
            import_existing: None,
        });
        process.update(|p| p.kind = "transcribe".into());
        assert_eq!(
            process
                .commit_transcript_result(&cancel, || {
                    let progress = process.snapshot();
                    assert!(progress.committing);
                    assert!(!progress.summary_committing);
                    assert_eq!(progress.message, "음성 기록 저장 중");
                    assert!(process.cancel("recognition").is_err());
                    assert!(!cancel.load(Ordering::Acquire));
                    Ok("saved")
                })
                .unwrap(),
            "saved"
        );
        process.shutdown().unwrap();

        let cancelled_flag = AtomicBool::new(true);
        assert!(process
            .commit_transcript_result(&cancelled_flag, || -> Result<(), String> {
                panic!("an accepted cancellation must preserve the baseline")
            })
            .is_err());
    }
    #[test]
    fn recording_waits_for_summary_cancellation_before_starting_capture() {
        let (_temp, process) = setup();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let finished = Arc::new(AtomicBool::new(false));
        let ended = finished.clone();
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(5));
            }
            ended.store(true, Ordering::Release);
        });
        *process.running.lock().unwrap() = Some(Running {
            id: "summary".into(),
            cancel,
            handle,
            import_existing: None,
        });
        process.update(|p| {
            p.kind = "summary".into();
            p.status = "running".into();
        });
        process
            .recording_start(|| {
                assert!(finished.load(Ordering::Acquire));
                Ok(())
            })
            .unwrap();
        assert!(process.running.lock().unwrap().is_none());
    }
    #[test]
    fn removing_a_meeting_waits_for_its_worker_and_blocks_new_start_until_storage_finishes() {
        let (_temp, process) = setup();
        let core = CoreHandle::open(&process.root).unwrap();
        let meeting = core.create_meeting("Meeting", "").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let finished = Arc::new(AtomicBool::new(false));
        let ended = finished.clone();
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                thread::yield_now();
            }
            ended.store(true, Ordering::Release);
        });
        *process.running.lock().unwrap() = Some(Running {
            id: "job".into(),
            cancel,
            handle,
            import_existing: None,
        });
        process.update(|progress| {
            progress.meeting_id = Some(meeting.id.clone());
            progress.kind = "transcribe".into();
            progress.status = "running".into();
        });
        process
            .meeting_removal(&core, &[meeting.id], || {
                assert!(finished.load(Ordering::Acquire));
                assert!(process.running.try_lock().is_err());
                Ok(())
            })
            .unwrap();
        assert!(process.running.lock().unwrap().is_none());
    }
    #[test]
    fn deleting_another_meeting_does_not_cancel_an_unrelated_job() {
        let (_temp, process) = setup();
        let core = CoreHandle::open(&process.root).unwrap();
        let meeting = core.create_meeting("Meeting", "").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                thread::yield_now();
            }
        });
        *process.running.lock().unwrap() = Some(Running {
            id: "download".into(),
            cancel: cancel.clone(),
            handle,
            import_existing: None,
        });
        process.update(|progress| {
            progress.kind = "summary-model".into();
            progress.status = "running".into();
        });
        process
            .meeting_removal(&core, &[meeting.id], || Ok(()))
            .unwrap();
        assert!(!cancel.load(Ordering::Acquire));
        assert!(process.running.lock().unwrap().is_some());
        process.shutdown().unwrap();
    }
    #[test]
    fn import_completion_is_joined_before_removing_its_new_meeting() {
        let (_temp, process) = setup();
        let core = CoreHandle::open(&process.root).unwrap();
        let audio = process.root.join("source.wav");
        let mut wave = Vec::from(b"RIFF".as_slice());
        wave.extend_from_slice(&38u32.to_le_bytes());
        wave.extend_from_slice(b"WAVEfmt ");
        wave.extend_from_slice(&16u32.to_le_bytes());
        for value in [1u16, 1u16] {
            wave.extend_from_slice(&value.to_le_bytes());
        }
        wave.extend_from_slice(&16000u32.to_le_bytes());
        wave.extend_from_slice(&32000u32.to_le_bytes());
        for value in [2u16, 16u16] {
            wave.extend_from_slice(&value.to_le_bytes());
        }
        wave.extend_from_slice(b"data");
        wave.extend_from_slice(&2u32.to_le_bytes());
        wave.extend_from_slice(&0i16.to_le_bytes());
        fs::write(&audio, wave).unwrap();
        let meeting = core.import_audio("Imported", &audio, &audio).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let ended = Arc::new(AtomicBool::new(false));
        let finished = ended.clone();
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                thread::yield_now();
            }
            finished.store(true, Ordering::Release);
        });
        *process.running.lock().unwrap() = Some(Running {
            id: "import".into(),
            cancel,
            handle,
            import_existing: Some(HashSet::new()),
        });
        process.update(|progress| {
            progress.kind = "import".into();
            progress.status = "running".into();
        });
        process
            .meeting_removal(&core, &[meeting.id], || {
                assert!(ended.load(Ordering::Acquire));
                Ok(())
            })
            .unwrap();
    }
    #[test]
    fn purge_removes_only_owned_job_copies_for_selected_meetings() {
        let (_temp, process) = setup();
        let selected_id = Uuid::new_v4().to_string();
        let other_id = Uuid::new_v4().to_string();
        let owned = directory(&process.root, &format!("processing/{}", Uuid::new_v4())).unwrap();
        Processing::bind_job(&owned, &selected_id).unwrap();
        fs::write(owned.join("analysis.wav"), b"audio").unwrap();
        let other = directory(&process.root, &format!("processing/{}", Uuid::new_v4())).unwrap();
        Processing::bind_job(&other, &other_id).unwrap();
        let unknown = directory(&process.root, &format!("processing/{}", Uuid::new_v4())).unwrap();
        fs::write(unknown.join("analysis.wav"), b"legacy").unwrap();
        let model = directory(&process.root, "models/shared").unwrap();
        fs::write(model.join("weights"), b"model").unwrap();
        process.purge_meeting_jobs(&[selected_id]).unwrap();
        assert!(!owned.exists());
        assert!(other.exists());
        assert!(unknown.exists());
        assert!(model.join("weights").exists());
    }
    #[test]
    fn startup_retries_owned_orphan_cleanup_and_preserves_existing_and_unknown_jobs() {
        let (_temp, process) = setup();
        let core = CoreHandle::open(&process.root).unwrap();
        let existing = core.create_meeting("Existing", "").unwrap();
        let job = |owner: Option<&str>| {
            let path = directory(&process.root, &format!("processing/{}", Uuid::new_v4())).unwrap();
            fs::write(path.join("analysis.wav"), b"audio").unwrap();
            if let Some(owner) = owner {
                Processing::bind_job(&path, owner).unwrap();
            }
            path
        };
        let orphan = job(Some(&Uuid::new_v4().to_string()));
        let active = job(Some(&existing.id));
        let unknown = job(None);
        let invalid = job(Some("invalid"));
        process.cleanup_removed_meeting_jobs(&core);
        assert!(!orphan.exists());
        assert!(active.exists());
        assert!(unknown.exists());
        assert!(invalid.exists());
    }
    #[cfg(unix)]
    #[test]
    fn startup_cleanup_failure_is_visible_and_does_not_take_the_processing_gate() {
        let (temp, process) = setup();
        let core = CoreHandle::open(&process.root).unwrap();
        std::os::unix::fs::symlink(temp.path(), process.root.join("processing")).unwrap();
        process.cleanup_removed_meeting_jobs(&core);
        let status = process.progress.lock().unwrap().clone();
        assert_eq!(status.kind, "cleanup");
        assert_eq!(status.status, "failed");
        assert_eq!(
            status.message,
            "삭제한 회의의 임시 파일을 정리하지 못했습니다."
        );
        assert!(status.id.is_some());
        assert!(process.running.lock().unwrap().is_none());
        process.recording_start(|| Ok(())).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn canonical_purge_retry_failure_remains_visible_after_job_cleanup_succeeds() {
        let (temp, process) = setup();
        let core = CoreHandle::open(&process.root).unwrap();
        let meeting = core.create_meeting("Removed", "").unwrap();
        drop(core);
        fs::write(
            process.root.join("library.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1, "folders": {}, "placements": {}, "pendingPurges": [meeting.id],
            }))
            .unwrap(),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            temp.path(),
            process
                .root
                .join("meetings")
                .join(&meeting.id)
                .join("linked"),
        )
        .unwrap();
        let core = CoreHandle::open(&process.root).unwrap();
        assert!(core.list_meetings().unwrap().is_empty());
        process.cleanup_removed_meeting_jobs(&core);
        let status = process.progress.lock().unwrap().clone();
        assert_eq!(status.kind, "cleanup");
        assert_eq!(status.status, "failed");
        assert!(!status.message.contains(&meeting.id));
        assert!(process.root.join("meetings").join(&meeting.id).exists());
    }
    #[test]
    fn successful_source_upgrade_does_not_add_a_startup_notice() {
        let (_temp, process) = setup();
        process.record_transcript_upgrade(Ok(grove_core::TranscriptUpgradeReport {
            upgraded: 1,
            unchanged: 1,
            unavailable: 0,
        }));
        assert!(process.startup_issues.lock().unwrap().is_empty());
        assert_eq!(process.progress.lock().unwrap().status, "");
    }
    #[test]
    fn unavailable_source_upgrade_remains_nonblocking_and_can_be_delivered_once() {
        let (_temp, process) = setup();
        process.record_transcript_upgrade(Ok(grove_core::TranscriptUpgradeReport {
            upgraded: 1,
            unchanged: 0,
            unavailable: 1,
        }));
        {
            let issues = process.startup_issues.lock().unwrap();
            assert_eq!(issues.len(), 1);
            assert_eq!(issues[0].kind, "transcript-upgrade");
            assert_eq!(issues[0].status, "failed");
        }
        process.recording_start(|| Ok(())).unwrap();
        let deliveries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handles = (0..4)
            .map(|_| {
                let process = process.clone();
                let deliveries = deliveries.clone();
                thread::spawn(move || {
                    process.notify_startup_issues(|_| {
                        deliveries.fetch_add(1, Ordering::AcqRel);
                        true
                    })
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(deliveries.load(Ordering::Acquire), 1);
        assert!(process.startup_issues.lock().unwrap().is_empty());
    }
    #[cfg(unix)]
    #[test]
    fn source_upgrade_failure_and_cleanup_failure_keep_separate_diagnostics() {
        let (temp, process) = setup();
        let core = CoreHandle::open(&process.root).unwrap();
        std::os::unix::fs::symlink(temp.path(), process.root.join("processing")).unwrap();
        process.cleanup_removed_meeting_jobs(&core);
        let cleanup = process.progress.lock().unwrap().clone();
        process.record_transcript_upgrade(Err(grove_core::CoreError::Storage(
            "private/source/path".into(),
        )));
        assert!(process.progress.lock().unwrap().clone() == cleanup);
        assert_eq!(cleanup.kind, "cleanup");
        assert_eq!(cleanup.status, "failed");
        process.notify_startup_issues(|issues| {
            assert_eq!(issues[0].kind, "transcript-upgrade");
            assert!(!issues[0].message.contains("private/source/path"));
            false
        });
        assert_eq!(process.startup_issues.lock().unwrap().len(), 1);
        process.notify_startup_issues(|_| true);
        assert!(process.startup_issues.lock().unwrap().is_empty());
        assert!(process.progress.lock().unwrap().clone() == cleanup);
    }
    #[test]
    fn historic_job_ownership_requires_a_unique_exact_raw_result() {
        let (_temp, process) = setup();
        let core = CoreHandle::open(&process.root).unwrap();
        let first = core.create_meeting("First", "").unwrap();
        let second = core.create_meeting("Second", "").unwrap();
        let canonical = |id: &str, bytes: &[u8]| {
            let path = directory(
                &process.root,
                &format!("meetings/{id}/transcripts/{}", Uuid::new_v4()),
            )
            .unwrap();
            fs::write(path.join("raw.json"), bytes).unwrap();
        };
        canonical(&first.id, b"unique");
        canonical(&first.id, b"shared");
        canonical(&second.id, b"shared");
        let job = |bytes: &[u8]| {
            let path = directory(&process.root, &format!("processing/{}", Uuid::new_v4())).unwrap();
            fs::write(path.join("moss.json"), bytes).unwrap();
            path
        };
        let unique = job(b"unique");
        let shared = job(b"shared");
        let unknown = job(b"unknown");
        process.bind_legacy_jobs(&core).unwrap();
        let owner: JobOwner =
            serde_json::from_reader(plain_file(&unique.join("owner.json")).unwrap()).unwrap();
        assert_eq!(owner.meeting_id, first.id);
        assert!(!shared.join("owner.json").exists());
        assert!(!unknown.join("owner.json").exists());
    }
    #[test]
    fn fixed_manifest_hash_rejects_changed_model_files() {
        let (temp, _) = setup();
        let file = temp.path().join("model");
        fs::write(&file, b"model").unwrap();
        let item = ModelFile {
            name: "model".into(),
            bytes: 5,
            sha256: format!("{:x}", Sha256::digest(b"model")),
        };
        let cancel = AtomicBool::new(false);
        assert!(matches_file(&file, &item, &cancel));
        cancel.store(true, Ordering::Release);
        assert!(!matches_file(&file, &item, &cancel));
        cancel.store(false, Ordering::Release);
        fs::write(&file, b"other").unwrap();
        assert!(!matches_file(&file, &item, &cancel));
    }

    #[test]
    fn moss_worker_kinds_are_allowlisted_without_exposing_worker_messages() {
        use crate::summary_failure::FailureKind;
        for (name, expected) in [
            ("inputLimit", FailureKind::InputLimit),
            ("allocation", FailureKind::Allocation),
            ("input", FailureKind::Input),
            ("output", FailureKind::Output),
            ("storage", FailureKind::Storage),
            ("engineUnavailable", FailureKind::EngineUnavailable),
        ] {
            let log = serde_json::json!({"worker":"moss","kind":name,"message":"/private/recording.wav arbitrary engine text"}).to_string();
            let (kind, text) = moss_worker_failure(&log).unwrap();
            assert_eq!(kind, expected);
            assert!(!text.contains("/private/"));
            assert!(!text.contains("arbitrary"));
        }
        for log in [
            r#"{"worker":"other","kind":"allocation","message":"anything"}"#,
            r#"{"worker":"moss","kind":"unknown","message":"out of memory"}"#,
            "memory warning",
            "std::bad_alloc",
        ] {
            assert!(moss_worker_failure(log).is_none());
        }
    }

    #[cfg(unix)]
    fn shell_worker(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn typed_moss_failure_preserves_the_existing_baseline_and_does_not_change_audio_prepare_errors()
    {
        use crate::summary_failure::FailureKind;
        let (_temporary, process) = setup();
        let (_core, id, source) = edited_baseline(&process);
        let current = process
            .root
            .join("meetings")
            .join(&id)
            .join("transcripts")
            .join(&source.version.id)
            .join("current.json");
        let before = fs::read(&current).unwrap();
        let worker = process.root.join("MossHarness");
        let script = "printf '%s\\n' '{\"worker\":\"moss\",\"kind\":\"output\",\"message\":\"/private/recording.wav\"}' >&2\nexit 1";
        shell_worker(&worker, script);
        let cancel = AtomicBool::new(false);
        let error = process
            .run_worker(
                &worker,
                &[],
                &process.root.join("moss-failure.log"),
                &cancel,
            )
            .unwrap_err();
        assert_eq!(
            process.progress.lock().unwrap().failure_kind,
            Some(FailureKind::Output)
        );
        assert!(!error.contains("/private/"));
        assert!(error.contains("완성되지 않았습니다"));
        assert_eq!(fs::read(&current).unwrap(), before);
        let prepare = process.root.join("AudioPrepare");
        shell_worker(&prepare, script);
        process.update(|progress| progress.failure_kind = None);
        assert_eq!(
            process
                .run_worker(
                    &prepare,
                    &[],
                    &process.root.join("prepare-failure.log"),
                    &cancel
                )
                .unwrap_err(),
            "음성 처리를 완료하지 못했습니다. 원본과 기존 음성 기록은 보관했습니다."
        );
        assert!(process.progress.lock().unwrap().failure_kind.is_none());
        assert_eq!(fs::read(current).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_confirms_worker_pid_is_gone_before_another_capture_can_start() {
        let (_temporary, process) = setup();
        let script = process.root.join("mini-worker");
        shell_worker(&script, "printf '%s' \"$$\" > \"$1\"\nexec /bin/sleep 30");
        let pid_file = process.root.join("worker.pid");
        let log = process.root.join("worker.log");
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker_process = process.clone();
        let worker_pid_file = pid_file.clone();
        let handle = thread::spawn(move || {
            worker_process.run_worker(&script, &[&worker_pid_file], &log, &worker_cancel)
        });
        let started = std::time::Instant::now();
        let pid = loop {
            if let Some(pid) = fs::read_to_string(&pid_file)
                .ok()
                .and_then(|text| text.parse::<u32>().ok())
                .filter(|pid| *pid > 1)
            {
                break pid.to_string();
            }
            assert!(started.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(5));
        };
        cancel.store(true, Ordering::Release);
        assert!(handle.join().unwrap().is_err());
        let exists = Command::new("/bin/kill")
            .args(["-0", pid.trim()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(
            !exists,
            "a stopped worker must not remain alive or unreaped"
        );
        assert!(process.worker_ready().is_ok());
        assert!(process.recording_start(|| Ok(())).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_kills_and_reaps_a_running_worker() {
        let (temp, process) = setup();
        let flag = Arc::new(AtomicBool::new(false));
        let worker_flag = flag.clone();
        let log = temp.path().join("sleep.log");
        let now = std::time::Instant::now();
        let worker = thread::spawn(move || {
            process.run_worker(
                Path::new("/bin/sleep"),
                &[Path::new("30")],
                &log,
                &worker_flag,
            )
        });
        thread::sleep(Duration::from_millis(150));
        flag.store(true, Ordering::Release);
        assert!(worker.join().unwrap().is_err());
        assert!(now.elapsed() < Duration::from_secs(3));
    }
    #[test]
    fn stalled_network_future_can_be_cancelled() {
        let flag = Arc::new(AtomicBool::new(false));
        let other = flag.clone();
        let setter = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            other.store(true, Ordering::Release);
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert!(runtime
            .block_on(interruptible(&flag, std::future::pending::<()>()))
            .is_err());
        setter.join().unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn linked_model_file_and_job_directories_are_rejected() {
        let (temp, process) = setup();
        let source = temp.path().join("source");
        fs::write(&source, b"test").unwrap();
        let link = temp.path().join("linked");
        std::os::unix::fs::symlink(&source, &link).unwrap();
        assert!(plain_file(&link).is_err());
        assert!(directory(&process.root, "../outside").is_err());
        let sub = temp.path().join("processing");
        std::os::unix::fs::symlink(temp.path(), sub).unwrap();
        assert!(directory(&process.root, "processing/job").is_err());
    }
}

#[cfg(test)]
mod setup_tests {
    use super::*;
    #[test]
    fn meeting_entry_requires_speech_but_not_optional_summary() {
        for supported in [false, true] {
            for summary_supported in [false, true] {
                for model_ready in [false, true] {
                    for summary_ready in [false, true] {
                        let status = Progress {
                            supported,
                            summary_supported,
                            model_ready,
                            summary_ready,
                            ..Default::default()
                        };
                        assert_eq!(
                            meeting_models_ready(&status).is_ok(),
                            supported && model_ready
                        );
                    }
                }
            }
        }
    }
}
