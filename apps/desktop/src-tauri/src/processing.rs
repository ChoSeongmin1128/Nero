use grove_core::{CoreHandle, SessionState, Snapshot};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use uuid::Uuid;

const MODEL: &str = include_str!("../../../../Sources/GroveApp/Resources/Models/moss.json");

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
        Arc::new(Self {
            root,
            resources,
            progress: Mutex::new(Progress::default()),
            startup_issues: Mutex::new(Vec::new()),
            running: Mutex::new(None),
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
        let mut progress = self.progress.lock().unwrap().clone();
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
        progress.revision = progress.revision.saturating_add(1);
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
        let revision = progress.revision.saturating_add(1);
        action(&mut progress);
        progress.revision = revision;
    }
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        let running = self.running.lock().unwrap();
        let job = running
            .as_ref()
            .filter(|r| r.id == id && !r.handle.is_finished())
            .ok_or("현재 작업과 다른 요청입니다.")?;
        let mut progress = self.progress.lock().unwrap();
        if progress.committing || progress.summary_committing {
            return Err(if progress.kind == "summary" {
                "회의록을 저장하고 있습니다."
            } else {
                "음성 기록을 저장하고 있습니다."
            }
            .into());
        }
        job.cancel.store(true, Ordering::Release);
        progress.message = "취소 중".into();
        progress.revision = progress.revision.saturating_add(1);
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

    fn commit_result<T>(
        &self,
        cancel: &AtomicBool,
        summary: bool,
        commit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        {
            let mut progress = self.progress.lock().unwrap();
            cancelled(cancel)?;
            progress.committing = true;
            progress.summary_committing = summary;
            progress.phase = Some("saving".into());
            progress.message = if summary {
                "회의록 저장 중"
            } else {
                "음성 기록 저장 중"
            }
            .into();
            progress.revision = progress.revision.saturating_add(1);
        }
        commit()
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
        self.worker_ready()
            .map_err(grove_core::CoreError::InvalidState)?;
        action()
    }

    fn bind_job(path: &Path, meeting_id: &str) -> Result<(), String> {
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
            job.cancel.store(true, Ordering::Release);
            job.handle
                .join()
                .map_err(|_| "음성 처리 종료를 확인하지 못했습니다.".to_string())?;
        }
        self.worker_ready()
    }
    pub fn resume(&self) {
        self.shutting_down.store(false, Ordering::Release);
    }
    pub(super) fn start(
        self: &Arc<Self>,
        kind: &str,
        meeting_id: Option<String>,
        action: impl FnOnce(Arc<Self>, Arc<AtomicBool>, PathBuf) -> Result<Option<String>, String>
            + Send
            + 'static,
    ) -> Result<Progress, String> {
        self.start_scoped(kind, meeting_id, None, action)
    }
    fn start_scoped(
        self: &Arc<Self>,
        kind: &str,
        meeting_id: Option<String>,
        importing_core: Option<CoreHandle>,
        action: impl FnOnce(Arc<Self>, Arc<AtomicBool>, PathBuf) -> Result<Option<String>, String>
            + Send
            + 'static,
    ) -> Result<Progress, String> {
        let mut running = self.running.lock().unwrap();
        self.worker_ready()?;
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
        if running.as_ref().is_some_and(|r| !r.handle.is_finished()) {
            return Err("다른 작업이 진행 중입니다.".into());
        }
        if let Some(old) = running.take() {
            let _ = old.handle.join();
        }
        self.worker_ready()?;
        let import_existing = importing_core
            .map(|core| {
                core.list_meetings()
                    .map(|meetings| {
                        meetings
                            .into_iter()
                            .map(|meeting| meeting.id)
                            .collect::<HashSet<_>>()
                    })
                    .map_err(message)
            })
            .transpose()?;
        let id = Uuid::new_v4().to_string();
        let path = directory(&self.root, &format!("processing/{id}"))?;
        if let Some(meeting_id) = &meeting_id {
            Self::bind_job(&path, meeting_id)?;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.update(|p| {
            *p = Progress {
                id: Some(id.clone()),
                kind: kind.into(),
                meeting_id,
                status: "running".into(),
                message: "준비 중".into(),
                ..Progress::default()
            }
        });
        let this = self.clone();
        let flag = cancel.clone();
        let handle = thread::spawn(move || {
            let result = action(this.clone(), flag.clone(), path);
            this.update(|p| match result {
                Ok(meeting) => {
                    if meeting.is_some() {
                        p.meeting_id = meeting;
                    }
                    p.status = "completed".into();
                    p.message = "완료".into();
                }
                Err(error) => {
                    p.status = if flag.load(Ordering::Acquire)
                        && !this.worker_cleanup_failed.load(Ordering::Acquire)
                    {
                        "cancelled"
                    } else {
                        "failed"
                    }
                    .into();
                    p.message = error;
                }
            });
        });
        *running = Some(Running {
            id,
            cancel,
            handle,
            import_existing,
        });
        Ok(self.snapshot())
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
        self.start("setup", None, move |this, cancel, _path| {
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
        })
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
        let mut child = Worker(
            command
                .spawn()
                .map_err(|_| "음성 인식을 시작하지 못했습니다.".to_string())?,
        );
        loop {
            cancelled(cancel)?;
            if let Some(status) = child.0.try_wait().map_err(message)? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(
                        "음성 처리를 완료하지 못했습니다. 원본과 기존 음성 기록은 보관했습니다."
                            .into(),
                    )
                };
            }
            thread::sleep(Duration::from_millis(100));
        }
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
        if let Some(job) = running.take() {
            job.cancel.store(true, Ordering::Release);
            let _ = job.handle.join();
        }
        self.worker_ready()?;
        action()
    }
    pub fn import(self: &Arc<Self>, core: CoreHandle, source: PathBuf) -> Result<Progress, String> {
        self.start_scoped(
            "import",
            None,
            Some(core.clone()),
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
                let meeting = core
                    .import_audio(&title, &copied, &prepared)
                    .map_err(message)?;
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
        let audio = core.audio_path(&meeting_id).map_err(message)?;
        let expected = core
            .transcript(&meeting_id)
            .map_err(message)?
            .map(|d| d.version);
        if !self.snapshot().model_ready {
            return Err("먼저 음성 인식 모델을 준비해 주세요.".into());
        }
        self.start(
            "transcribe",
            Some(meeting_id.clone()),
            move |this, cancel, path| {
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
                let source = core
                    .commit_transcript(&meeting_id, expected, raw)
                    .map_err(message)?;
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
        self.transcribe(core, meeting_id).map(Some)
    }
}

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
