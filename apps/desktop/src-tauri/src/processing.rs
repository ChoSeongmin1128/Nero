use crate::job_journal::{JobRecord, JobStatus, Journal};
use crate::speech_choice::{AppleVariant, SpeechChoice, SpeechEngine};
use nero_core::{CoreHandle, SessionState, Snapshot};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex, OnceLock,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use uuid::Uuid;

fn platform_supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        cfg!(all(target_os = "windows", target_arch = "x86_64"))
            || cfg!(all(target_os = "macos", target_arch = "aarch64"))
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
impl Model {
    pub(super) fn bytes(&self) -> u64 {
        self.files.iter().map(|file| file.bytes).sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModelStage {
    Speech,
    Speakers,
    Summary,
}
impl ModelStage {
    fn id(self) -> &'static str {
        match self {
            Self::Speech => "speech",
            Self::Speakers => "speakers",
            Self::Summary => "summary",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::Speech => "음성 인식 모델",
            Self::Speakers => "화자 구분 모델",
            Self::Summary => "요약 모델",
        }
    }
}

// A pinned model stored under `models/{label}-{revision}`. Speech engines derive theirs
// from `SpeechEngine`; the speaker and summary models declare a constant.
#[derive(Clone, Copy)]
pub(super) struct ModelSpec {
    pub(super) manifest: &'static str,
    pub(super) label: &'static str,
    pub(super) stage: ModelStage,
}
impl ModelSpec {
    pub(super) fn speech(engine: SpeechEngine) -> Self {
        Self {
            manifest: engine.manifest(),
            label: engine.label(),
            stage: ModelStage::Speech,
        }
    }
    pub(super) fn model(self) -> Model {
        serde_json::from_str(self.manifest).expect("bundled model manifest")
    }
    fn relative_dir(self) -> String {
        format!("models/{}-{}", self.label, self.model().revision)
    }
}

// Hashes every pinned file before use. A mismatch withdraws the readiness marker so the
// model is prepared again; the error kind is Cancelled, Storage or ModelNotReady.
pub(super) fn verify_model_files(
    directory: &Path,
    model: &Model,
    cancel: &AtomicBool,
) -> Result<(), crate::failure::Failure> {
    use crate::failure::{Failure, FailureKind};
    let cancelled = |cancel: &AtomicBool| {
        cancelled(cancel).map_err(|message| Failure::new(FailureKind::Cancelled, message))
    };
    for item in &model.files {
        cancelled(cancel)?;
        let matches = matches_file(&directory.join(&item.name), item, cancel);
        cancelled(cancel)?;
        if !matches {
            invalidate_readiness(directory)
                .map_err(|message| Failure::new(FailureKind::Storage, message))?;
            return Err(Failure::new(
                FailureKind::ModelNotReady,
                "모델 파일을 다시 준비해 주세요.",
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub speech_engine: Option<SpeechEngine>,
    pub speaker_enabled: bool,
    pub apple_speech: crate::apple_speech::Availability,
    pub speech_choices: Vec<SpeechChoice>,
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
    pub local_summary_enabled: bool,
    pub local_summary_message: String,
    pub summary_ready: bool,
    pub summary_bytes: u64,
    pub speaker_supported: bool,
    pub speaker_ready: bool,
    pub speaker_bytes: u64,
    pub model_stage: Option<String>,
    pub memory_state: Option<crate::memory::MemoryState>,
    pub phase: Option<String>,
    pub failure_kind: Option<crate::failure::FailureKind>,
    pub failure_detail: Option<crate::failure::FailureDetail>,
    pub has_previous_summary: bool,
    pub retry_count: u8,
    pub committing: bool,
    pub summary_committing: bool,
    pub jobs: Vec<JobView>,
    pub live_transcript: Option<serde_json::Value>,
    pub speech_switch: Option<crate::speech_switch::SwitchView>,
    pub speech_offer: Option<crate::speech_switch::OfferView>,
}

#[derive(Clone, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub speech_engine: SpeechEngine,
    pub speaker_enabled: bool,
    pub apple_speech_variant: Option<AppleVariant>,
    pub id: String,
    pub kind: String,
    pub meeting_id: Option<String>,
    pub status: String,
    pub phase: Option<String>,
    pub message: String,
    pub retryable: bool,
    pub acknowledged: bool,
    pub created_at: u64,
    pub retry_successor_id: Option<String>,
    pub failure_kind: Option<crate::failure::FailureKind>,
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

// A new job. Only `kind` is required. `source` binds a meeting job to the meeting's
// current content; a retry also names the fingerprint its predecessor captured.
#[derive(Default)]
pub(super) struct JobSpec {
    pub(super) kind: &'static str,
    pub(super) meeting_id: Option<String>,
    pub(super) source: Option<CoreHandle>,
    pub(super) required_fingerprint: Option<String>,
    // An import records the meetings that existed before it.
    pub(super) importing: Option<CoreHandle>,
    pub(super) include_summary: bool,
    pub(super) template: Option<crate::summary_template::TemplateRef>,
    pub(super) requested_id: Option<String>,
    pub(super) engine: Option<SpeechEngine>,
    pub(super) speaker_enabled: Option<bool>,
    pub(super) apple_variant: Option<AppleVariant>,
}

// The job phase recorded when Core refused a result before publishing anything.
fn rejection_phase(kind: crate::failure::FailureKind) -> Option<&'static str> {
    match kind {
        crate::failure::FailureKind::Output => Some("outputRejected"),
        crate::failure::FailureKind::SourceChanged => Some("sourceChanged"),
        crate::failure::FailureKind::MeetingUnavailable => Some("meetingUnavailable"),
        _ => None,
    }
}

fn model_job(kind: &str) -> bool {
    matches!(kind, "setup" | "model" | "summary-model" | "speaker-model")
}

// Why a job cannot be retried, if it cannot. The job list and retry use the same rule.
fn retry_block_reason(record: &JobRecord) -> Option<&'static str> {
    if !crate::release_capabilities::LOCAL_SUMMARY_ENABLED
        && (matches!(record.kind.as_str(), "summary" | "summary-model")
            || (record.kind == "setup" && record.include_summary))
    {
        return Some(crate::release_capabilities::LOCAL_SUMMARY_MESSAGE);
    }
    if !matches!(
        record.status,
        JobStatus::Interrupted | JobStatus::Failed | JobStatus::Cancelled
    ) || record.acknowledged
        || record.kind == "import"
    {
        return Some("현재 작업은 다시 실행할 수 없습니다. 회의의 저장 결과를 먼저 확인해 주세요.");
    }
    // Stopped after its transcript was saved (during terminology correction with speaker
    // analysis off): the meeting has changed since, so a retry would only be refused.
    if record.kind == "transcribe" && record.phase.as_deref() == Some("postprocessing") {
        return Some("음성 기록은 이미 저장되었습니다. 다시 변환하려면 회의에서 변환을 새로 시작해 주세요.");
    }
    None
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
    #[cfg(test)]
    platform_supported_override: Mutex<Option<bool>>,
    speech_engine: Mutex<SpeechEngine>,
    speaker_enabled: Mutex<bool>,
    apple_speech_variant: Mutex<Option<AppleVariant>>,
    apple_speech: Mutex<crate::apple_speech::Availability>,
    speech_preference_error: Mutex<Option<String>>,
    speech_switch: Mutex<Option<crate::speech_switch::Pending>>,
    speech_offers: Mutex<crate::speech_switch::Offers>,
    speech_switch_error: Mutex<Option<String>>,
    // A setup that already worked at startup; first-time setups are not offered a switch.
    offer_eligible: AtomicBool,
    pub(super) root: PathBuf,
    pub(super) resources: PathBuf,
    progress: Mutex<Progress>,
    startup_issues: Mutex<Vec<Progress>>,
    startup_issues_requested: AtomicBool,
    running: Mutex<Option<Running>>,
    pending: Mutex<VecDeque<Pending>>,
    records: Mutex<Vec<JobRecord>>,
    journal: Journal,
    journal_error: Mutex<Option<String>>,
    // Records whose journal write failed. New work waits until each one, or a newer
    // state of the same job, is written.
    unsaved: Mutex<HashMap<String, JobRecord>>,
    retry_gate: Mutex<()>,
    pub(super) recording: AtomicBool,
    interrupted_jobs: Mutex<HashSet<String>>,
    // Work that a recording start interrupted; queued again as retries.
    requeue: Mutex<Vec<String>>,
    revision: AtomicU64,
    recording_transcription: Mutex<RecordingTranscription>,
    shutting_down: AtomicBool,
    worker_cleanup_failed: AtomicBool,
    pub(super) live: Mutex<Option<Arc<crate::live_speech::LiveJob>>>,
    pub(super) live_completed: Mutex<HashMap<String, crate::live_speech::Completed>>,
}

fn agent_job_version(record: &JobRecord) -> String {
    let identity = serde_json::json!([record.id, record.kind, record.meeting_id, record.status]);
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&identity).unwrap())
    )
}

fn agent_job_view(record: &JobRecord) -> serde_json::Value {
    serde_json::json!({"id":record.id,"kind":record.kind,"meetingId":record.meeting_id,"status":record.status,"editVersion":agent_job_version(record)})
}

fn check_agent_job_version(
    record: &JobRecord,
    expected: Option<&str>,
) -> Result<(), nero_core::agent_control::ControlError> {
    if expected.is_some_and(|version| version != agent_job_version(record)) {
        return Err(nero_core::agent_control::ControlError::new(
            "edit_conflict",
            "작업 상태가 변경되었습니다. 다시 조회해 주세요.",
        ));
    }
    Ok(())
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
// A request refused before its first journal record leaves no job folder behind.
fn discard_unrecorded(job: &Path) {
    if !job.join("job-state").exists() {
        let _ = fs::remove_dir_all(job);
    }
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
        let (speech_engine, speaker_enabled, apple_speech_variant, speech_preference_error) =
            match crate::speech_choice::load(&root) {
                Ok(preference) => (
                    preference.engine,
                    preference.speaker_enabled,
                    preference.apple_speech_variant,
                    None,
                ),
                Err(error) => (SpeechEngine::default(), true, None, Some(error)),
            };
        // Auxiliary switch state never blocks recognition. Unreadable files are left as they
        // are: a broken pending switch is ignored and a broken offer record hides the offer.
        let speech_switch = crate::speech_switch::load_pending(&root).ok().flatten();
        let speech_offers = crate::speech_switch::load_offers(&root)
            .unwrap_or_else(|_| crate::speech_switch::Offers::suppressed());
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
        if let Some(error) = &speech_preference_error {
            startup_issues.push(Progress {
                id: Some(Uuid::new_v4().to_string()),
                kind: "speech-settings".into(),
                status: "failed".into(),
                message: error.clone(),
                ..Default::default()
            });
        }
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
        let process = Arc::new(Self {
            #[cfg(test)]
            platform_supported_override: Mutex::new(None),
            speech_engine: Mutex::new(speech_engine),
            speaker_enabled: Mutex::new(speaker_enabled),
            apple_speech_variant: Mutex::new(apple_speech_variant),
            // Only a saved Apple Speech selection needs its status before startup finishes.
            apple_speech: Mutex::new(if speech_engine == SpeechEngine::AppleSpeech {
                crate::apple_speech::probe(&resources, apple_speech_variant)
            } else {
                crate::apple_speech::Availability::default()
            }),
            speech_preference_error: Mutex::new(speech_preference_error),
            speech_switch: Mutex::new(speech_switch),
            speech_offers: Mutex::new(speech_offers),
            offer_eligible: AtomicBool::new(false),
            speech_switch_error: Mutex::new(None),
            root,
            resources,
            progress: Mutex::new(Progress::default()),
            startup_issues: Mutex::new(startup_issues),
            startup_issues_requested: AtomicBool::new(false),
            running: Mutex::new(None),
            pending: Mutex::new(VecDeque::new()),
            records: Mutex::new(records),
            journal,
            journal_error: Mutex::new(journal_error),
            unsaved: Mutex::new(HashMap::new()),
            retry_gate: Mutex::new(()),
            recording: AtomicBool::new(false),
            interrupted_jobs: Mutex::new(HashSet::new()),
            requeue: Mutex::new(Vec::new()),
            revision: AtomicU64::new(0),
            recording_transcription: Mutex::new(RecordingTranscription::default()),
            shutting_down: AtomicBool::new(false),
            worker_cleanup_failed: AtomicBool::new(false),
            live: Mutex::new(None),
            live_completed: Mutex::new(HashMap::new()),
        });
        let working = process.speech_preference_error.lock().unwrap().is_none()
            && process.model_ready_for(process.speech_engine());
        process.offer_eligible.store(working, Ordering::Release);
        if process.speech_engine() != SpeechEngine::AppleSpeech
            && crate::apple_speech::installed(&process.resources)
        {
            let background = process.clone();
            let _ = thread::Builder::new()
                .name("nero-apple-speech-status".into())
                .spawn(move || {
                    let status = crate::apple_speech::probe(&background.resources, None);
                    let mut current = background.apple_speech.lock().unwrap();
                    // A probe made since startup for an explicit choice is newer.
                    if *current == crate::apple_speech::Availability::default()
                        && *current != status
                    {
                        *current = status;
                        drop(current);
                        background.update(|_| ());
                    }
                });
        }
        process
    }
    pub(super) fn model(&self) -> Model {
        Self::model_for(self.speech_engine())
    }
    pub(super) fn model_for(engine: SpeechEngine) -> Model {
        serde_json::from_str(engine.manifest()).expect("bundled speech recognition manifest")
    }
    pub(super) fn speech_engine(&self) -> SpeechEngine {
        *self.speech_engine.lock().unwrap()
    }
    pub(super) fn speaker_enabled(&self) -> bool {
        *self.speaker_enabled.lock().unwrap()
    }
    pub(super) fn apple_speech_variant(&self) -> Option<AppleVariant> {
        *self.apple_speech_variant.lock().unwrap()
    }
    #[cfg(all(test, windows))]
    pub(super) fn model_dir(&self) -> Result<PathBuf, String> {
        self.model_dir_for(self.speech_engine())
    }
    pub(super) fn model_dir_for(&self, engine: SpeechEngine) -> Result<PathBuf, String> {
        self.model_dir_of(ModelSpec::speech(engine))
    }
    pub(super) fn model_dir_of(&self, spec: ModelSpec) -> Result<PathBuf, String> {
        directory(&self.root, &spec.relative_dir())
    }
    // The verified marker matches the manifest and every file has its pinned size.
    pub(super) fn model_ready(&self, spec: ModelSpec) -> bool {
        let Ok(path) = self.model_dir_of(spec) else {
            return false;
        };
        let marker = plain_file(&path.join("verified.json"))
            .ok()
            .and_then(|file| {
                let mut value = String::new();
                file.take(65536).read_to_string(&mut value).ok()?;
                Some(value)
            });
        marker.as_deref() == Some(spec.manifest)
            && spec.model().files.iter().all(|file| {
                fs::symlink_metadata(path.join(&file.name)).is_ok_and(|meta| {
                    meta.is_file() && !meta.file_type().is_symlink() && meta.len() == file.bytes
                })
            })
    }
    // The model directory after every pinned file is hashed again for this use.
    pub(super) fn verified_model_dir(
        &self,
        spec: ModelSpec,
        cancel: &AtomicBool,
    ) -> Result<PathBuf, crate::failure::Failure> {
        let directory = self.model_dir_of(spec).map_err(|message| {
            crate::failure::Failure::new(crate::failure::FailureKind::Storage, message)
        })?;
        verify_model_files(&directory, &spec.model(), cancel)?;
        Ok(directory)
    }
    pub(super) fn worker(&self, name: &str) -> PathBuf {
        self.resources.join("native").join(name)
    }
    fn supported(&self) -> bool {
        self.speech_preference_error.lock().unwrap().is_none()
            && self.selectable(self.speech_engine())
    }
    fn platform_supported(&self) -> bool {
        #[cfg(test)]
        if let Some(supported) = *self.platform_supported_override.lock().unwrap() {
            return supported;
        }
        platform_supported()
    }
    // A status check that failed is not a verdict: Apple Speech stays selectable, and
    // choosing or preparing it checks again before anything is saved or started.
    fn selectable(&self, engine: SpeechEngine) -> bool {
        self.supported_for(engine)
            || engine == SpeechEngine::AppleSpeech
                && self.apple_speech.lock().unwrap().asset_status == "unknown"
                && self.supported_with(engine, true)
    }
    // Uses the cached Apple Speech probe, so the frequent snapshot never runs the helper.
    fn supported_for(&self, engine: SpeechEngine) -> bool {
        let apple = self.apple_speech.lock().unwrap().supported;
        self.supported_with(engine, apple)
    }
    fn supported_with(&self, engine: SpeechEngine, apple_supported: bool) -> bool {
        self.platform_supported()
            && crate::speech_audio::supported(self)
            && match engine {
                SpeechEngine::Nemotron => crate::asr::supported(self),
                SpeechEngine::WhisperTurbo => crate::whisper_asr::supported(&self.resources),
                SpeechEngine::WhisperLargeV3 => {
                    cfg!(all(target_os = "macos", target_arch = "aarch64"))
                        && crate::whisper_asr::supported(&self.resources)
                }
                SpeechEngine::AppleSpeech => apple_supported,
                // Qwen needs the Nemotron stream for its recording preview.
                SpeechEngine::QwenKo => {
                    crate::qwen_asr::supported(&self.resources) && crate::asr::supported(self)
                }
            }
    }
    // (supported, ready) before starting work with `engine`. Apple Speech is probed
    // again for `variant` instead of trusting the cached status.
    fn engine_status(&self, engine: SpeechEngine, variant: Option<AppleVariant>) -> (bool, bool) {
        if engine != SpeechEngine::AppleSpeech {
            return (self.supported_for(engine), self.model_ready_for(engine));
        }
        let status = crate::apple_speech::probe(&self.resources, variant);
        (self.supported_with(engine, status.supported), status.ready)
    }
    // Every speech model a selection prepares, including Qwen's Nemotron preview model.
    fn speech_bytes(engine: SpeechEngine) -> u64 {
        engine
            .requirements()
            .into_iter()
            .map(|engine| Self::model_for(engine).bytes())
            .sum()
    }
    // Bytes still missing for every model `engine` needs; `.part` files count as received.
    fn remaining_speech_bytes(&self, engine: SpeechEngine) -> Result<u64, String> {
        if engine == SpeechEngine::AppleSpeech {
            return Ok(0);
        }
        engine
            .requirements()
            .into_iter()
            .try_fold(0, |sum, engine| {
                Ok(sum + self.remaining_model_bytes(ModelSpec::speech(engine))?)
            })
    }
    fn speakers_ready(&self, enabled: bool) -> bool {
        let (supported, ready, _) = crate::diarization::availability(self);
        !enabled || !supported || ready
    }
    fn model_ready_for(&self, engine: SpeechEngine) -> bool {
        if engine == SpeechEngine::AppleSpeech {
            return self.apple_speech.lock().unwrap().ready;
        }
        engine
            .requirements()
            .into_iter()
            .all(|engine| self.model_ready(ModelSpec::speech(engine)))
    }
    // Selects an engine. A ready engine becomes active now. Without a working engine
    // (first setup) the selection is only recorded; setup downloads it explicitly.
    // Otherwise the current engine stays active while the target downloads, and
    // `drive` switches once it is ready.
    pub fn request_speech_engine(
        self: &Arc<Self>,
        engine: SpeechEngine,
    ) -> Result<Progress, String> {
        let speaker_enabled = self.speaker_enabled();
        let variant = (engine == SpeechEngine::AppleSpeech)
            .then(|| self.apple_speech_variant())
            .flatten();
        if engine == SpeechEngine::AppleSpeech {
            *self.apple_speech.lock().unwrap() =
                crate::apple_speech::probe(&self.resources, variant);
        }
        let working = self.speech_preference_error.lock().unwrap().is_none()
            && self.model_ready_for(self.speech_engine());
        // A deliberate move to another engine, with Qwen listed as recommended, answers the offer.
        let answers_offer = engine != SpeechEngine::QwenKo && engine != self.speech_engine();
        *self.speech_switch_error.lock().unwrap() = None;
        if !working || (self.model_ready_for(engine) && self.speakers_ready(speaker_enabled)) {
            self.set_speech_selection(engine, speaker_enabled, variant)?;
            self.forget_speech_switch()?;
            if answers_offer {
                let _ = self.decide_speech_offer(crate::speech_switch::Decision::Declined);
            }
            return Ok(self.snapshot());
        }
        if !self.supported_for(engine) {
            return Err("이 기기에 선택한 음성 인식 실행기가 준비되지 않았습니다.".into());
        }
        let pending = crate::speech_switch::Pending::new(engine, speaker_enabled, variant);
        crate::speech_switch::save_pending(&self.root, &pending)?;
        *self.speech_switch.lock().unwrap() = Some(pending);
        if let Err(error) = self.prepare_meeting_models_with_id(
            false,
            None,
            Some(engine),
            Some(speaker_enabled),
            variant,
        ) {
            self.forget_speech_switch()?;
            return Err(error);
        }
        if answers_offer {
            let _ = self.decide_speech_offer(crate::speech_switch::Decision::Declined);
        }
        Ok(self.snapshot())
    }
    // Drops a waiting switch and cancels its download. The active engine is unchanged.
    pub fn cancel_speech_switch(&self) -> Result<Progress, String> {
        *self.speech_switch_error.lock().unwrap() = None;
        let target = self
            .speech_switch
            .lock()
            .unwrap()
            .map(|pending| pending.target);
        self.forget_speech_switch()?;
        if let Some(target) = target {
            let downloads: Vec<String> = self
                .records
                .lock()
                .unwrap()
                .iter()
                .filter(|record| {
                    record.kind == "setup"
                        && record.speech_engine == target
                        && matches!(record.status, JobStatus::Queued | JobStatus::Running)
                })
                .map(|record| record.id.clone())
                .collect();
            for id in downloads {
                self.cancel(&id)?;
            }
        }
        Ok(self.snapshot())
    }
    // Records the existing-user offer decision first, so a failed start never leaves an
    // unrecorded switch running. Accepting then starts the same flow as selecting.
    pub fn respond_speech_offer(self: &Arc<Self>, accept: bool) -> Result<Progress, String> {
        if accept {
            // The offer is used up only once the switch has started; a refused switch
            // (no space, the same work already queued) leaves it to accept again.
            let progress = self.request_speech_engine(SpeechEngine::QwenKo)?;
            self.decide_speech_offer(crate::speech_switch::Decision::Accepted)?;
            return Ok(progress);
        }
        self.decide_speech_offer(crate::speech_switch::Decision::Declined)?;
        Ok(self.snapshot())
    }
    fn decide_speech_offer(&self, decision: crate::speech_switch::Decision) -> Result<(), String> {
        let mut offers = self.speech_offers.lock().unwrap();
        if offers.decided(crate::speech_switch::QWEN_OFFER) {
            return Ok(());
        }
        crate::speech_switch::decide(
            &self.root,
            &mut offers,
            crate::speech_switch::QWEN_OFFER,
            decision,
            now(),
        )?;
        drop(offers);
        self.update(|_| ());
        Ok(())
    }
    fn forget_speech_switch(&self) -> Result<(), String> {
        let mut pending = self.speech_switch.lock().unwrap();
        if pending.is_some() {
            crate::speech_switch::clear_pending(&self.root)?;
            *pending = None;
            drop(pending);
            self.update(|_| ());
        }
        Ok(())
    }
    // Called by `drive` with the `running` lock held and no job running.
    fn reconcile_speech_switch(&self, running: &Option<Running>) {
        let Some(pending) = *self.speech_switch.lock().unwrap() else {
            return;
        };
        // A failed switch waits for the user instead of retrying on every poll.
        if self.speech_switch_error.lock().unwrap().is_some()
            || !self.idle_locked(running)
            || !self.model_ready_for(pending.target)
            || !self.speakers_ready(pending.speaker_enabled)
        {
            return;
        }
        if self.speech_preference_error.lock().unwrap().is_none()
            && self.speech_engine() == pending.target
        {
            let _ = self.forget_speech_switch();
            return;
        }
        match self.apply_speech_selection(
            pending.target,
            pending.speaker_enabled,
            pending.apple_speech_variant,
        ) {
            Ok(()) => {
                let _ = self.forget_speech_switch();
            }
            Err(error) => {
                *self.speech_switch_error.lock().unwrap() = Some(error);
                self.update(|_| ());
            }
        }
    }
    fn switch_view(&self) -> Option<crate::speech_switch::SwitchView> {
        let pending = (*self.speech_switch.lock().unwrap())?;
        let preparing = self.records.lock().unwrap().iter().any(|record| {
            record.kind == "setup"
                && record.speech_engine == pending.target
                && matches!(
                    record.status,
                    JobStatus::Queued | JobStatus::Running | JobStatus::Cancelling
                )
        });
        Some(crate::speech_switch::SwitchView {
            target: pending.target,
            preparing,
            error: self.speech_switch_error.lock().unwrap().clone(),
        })
    }
    // Offered only to setups that already worked when this process started, and only
    // while nothing is running: snapshot runs on every progress tick.
    fn offer_view(&self, progress: &Progress) -> Option<crate::speech_switch::OfferView> {
        let engine = SpeechEngine::QwenKo;
        let current = progress.speech_engine?;
        if !self.offer_eligible.load(Ordering::Acquire)
            || self
                .speech_offers
                .lock()
                .unwrap()
                .decided(crate::speech_switch::QWEN_OFFER)
            || current == engine
            || progress.speech_switch.is_some()
            || progress
                .jobs
                .iter()
                .any(|job| matches!(job.status.as_str(), "queued" | "running" | "cancelling"))
            || !progress.model_ready
            || !progress
                .speech_choices
                .iter()
                .any(|choice| choice.id == engine && choice.supported)
        {
            return None;
        }
        let speakers = if self.speaker_enabled() && !progress.speaker_ready {
            self.remaining_model_bytes(crate::diarization::SPEAKER_MODEL)
                .unwrap_or(0)
        } else {
            0
        };
        Some(crate::speech_switch::OfferView {
            id: crate::speech_switch::QWEN_OFFER,
            engine,
            title: engine.title(),
            description: engine.description(),
            download_bytes: self.remaining_speech_bytes(engine).unwrap_or(0) + speakers,
            memory_warning: progress
                .speech_choices
                .iter()
                .find(|choice| choice.id == engine)
                .and_then(|choice| choice.memory_warning.clone()),
        })
    }
    pub fn set_speech_engine(&self, engine: SpeechEngine) -> Result<Progress, String> {
        self.set_speech_selection(engine, self.speaker_enabled(), None)
    }
    pub fn set_speaker_enabled(&self, enabled: bool) -> Result<Progress, String> {
        self.set_speech_selection(self.speech_engine(), enabled, self.apple_speech_variant())
    }
    // True when nothing could observe a selection change: no job, queue, capture or live child.
    // The caller holds the `running` lock.
    fn idle_locked(&self, running: &Option<Running>) -> bool {
        !self.shutting_down.load(Ordering::Acquire)
            && !self.recording.load(Ordering::Acquire)
            && running.as_ref().is_none_or(|job| job.handle.is_finished())
            && self.pending.lock().unwrap().is_empty()
            && self
                .live
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|job| !job.running.load(Ordering::Acquire))
    }
    fn set_speech_selection(
        &self,
        engine: SpeechEngine,
        speaker_enabled: bool,
        preferred_variant: Option<AppleVariant>,
    ) -> Result<Progress, String> {
        let running = self.running.lock().unwrap();
        if !self.idle_locked(&running) {
            return Err("녹음과 처리 작업을 마친 뒤 음성 인식 모델을 변경해 주세요.".into());
        }
        self.apply_speech_selection(engine, speaker_enabled, preferred_variant)?;
        drop(running);
        Ok(self.snapshot())
    }
    // Persists a selection. The caller has proven `idle_locked` under the `running` lock.
    fn apply_speech_selection(
        &self,
        engine: SpeechEngine,
        speaker_enabled: bool,
        preferred_variant: Option<AppleVariant>,
    ) -> Result<(), String> {
        if engine == SpeechEngine::AppleSpeech {
            *self.apple_speech.lock().unwrap() =
                crate::apple_speech::probe(&self.resources, preferred_variant);
        }
        if !self.supported_for(engine) {
            return Err("이 기기에 선택한 음성 인식 실행기가 준비되지 않았습니다.".into());
        }
        let variant = if engine == SpeechEngine::AppleSpeech {
            self.apple_speech.lock().unwrap().variant
        } else {
            None
        };
        crate::speech_choice::save(&self.root, engine, speaker_enabled, variant)?;
        *self.apple_speech_variant.lock().unwrap() = variant;
        *self.speech_engine.lock().unwrap() = engine;
        *self.speaker_enabled.lock().unwrap() = speaker_enabled;
        *self.speech_preference_error.lock().unwrap() = None;
        self.update(|_| ());
        Ok(())
    }
    pub fn snapshot(&self) -> Progress {
        let mut progress = {
            let stored = self.progress.lock().unwrap();
            let records = self.records.lock().unwrap();
            let mut progress = stored.clone();
            progress.jobs = records
                .iter()
                .map(|record| JobView {
                    speech_engine: record.speech_engine,
                    speaker_enabled: record.speaker_enabled,
                    apple_speech_variant: record.apple_speech_variant,
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
                    created_at: record.created_at,
                    retry_successor_id: record.retry_successor_id.clone(),
                    failure_kind: record.failure_kind,
                    retryable: retry_block_reason(record).is_none(),
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
        progress.speaker_enabled = self.speaker_enabled();
        progress.apple_speech = self.apple_speech.lock().unwrap().clone();
        progress.local_summary_enabled = crate::release_capabilities::LOCAL_SUMMARY_ENABLED;
        progress.local_summary_message = crate::release_capabilities::LOCAL_SUMMARY_MESSAGE.into();
        progress.speech_engine = self
            .speech_preference_error
            .lock()
            .unwrap()
            .is_none()
            .then(|| self.speech_engine());
        progress.speech_choices = SpeechEngine::ALL
            .into_iter()
            .map(|engine| SpeechChoice {
                id: engine,
                label: engine.title(),
                description: engine.description(),
                model_bytes: Self::speech_bytes(engine),
                ready: self.model_ready_for(engine),
                supported: self.supported_for(engine),
                recommended: false,
                recommendation_reason: None,
                memory_warning: None,
            })
            .collect();
        crate::speech_recommendation::apply(&mut progress.speech_choices);
        for choice in &mut progress.speech_choices {
            choice.supported = self.selectable(choice.id);
        }
        (
            progress.summary_supported,
            progress.summary_ready,
            progress.summary_bytes,
        ) = crate::summary::availability(self);
        (
            progress.speaker_supported,
            progress.speaker_ready,
            progress.speaker_bytes,
        ) = if self.speaker_enabled() {
            crate::diarization::availability(self)
        } else {
            (false, false, 0)
        };
        let model = self.model();
        progress.model_bytes = model.bytes();
        progress.model_ready = progress
            .speech_engine
            .is_some_and(|engine| self.model_ready_for(engine));
        progress.speech_switch = self.switch_view();
        progress.speech_offer = self.offer_view(&progress);
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
    pub(crate) fn worker_ready(&self) -> Result<(), String> {
        if self.worker_cleanup_failed.load(Ordering::Acquire) {
            return Err("이전 음성 처리의 종료를 확인하지 못했습니다.".into());
        }
        Ok(())
    }
    // Records a failure's kind and detail on the current job and returns its message.
    pub(super) fn report(&self, error: crate::failure::Failure) -> String {
        self.update(|progress| {
            progress.failure_kind = Some(error.kind);
            progress.failure_detail = error.detail.clone();
        });
        error.message
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
                    if let Err(error) = self.save_job_or_keep(record) {
                        progress.status = "failed".into();
                        progress.message = error;
                    }
                }
            }
        }
    }

    fn save_job(&self, record: JobRecord) -> Result<(), String> {
        self.write_job(record, false)
    }

    // A failed write keeps its record to be written again before new work starts.
    fn save_job_or_keep(&self, record: JobRecord) -> Result<(), String> {
        self.write_job(record, true)
    }

    // `unsaved` is held across every write, so a kept record never lands after a newer
    // state of the same job.
    fn write_job(&self, record: JobRecord, keep: bool) -> Result<(), String> {
        let mut unsaved = self.unsaved.lock().unwrap();
        if let Err(error) = self.journal.write(&record) {
            if keep {
                unsaved.insert(record.id.clone(), record);
            }
            return Err(error);
        }
        unsaved.remove(&record.id);
        self.remember(record);
        Ok(())
    }

    fn remember(&self, record: JobRecord) {
        let mut records = self.records.lock().unwrap();
        if let Some(previous) = records.iter_mut().find(|r| r.id == record.id) {
            *previous = record;
        } else {
            records.push(record);
        }
        self.revision.fetch_add(1, Ordering::AcqRel);
    }

    fn job_ready(&self) -> Result<(), String> {
        self.worker_ready()?;
        if let Some(error) = self.journal_error.lock().unwrap().as_ref() {
            return Err(error.clone());
        }
        let mut unsaved = self.unsaved.lock().unwrap();
        while let Some(id) = unsaved.keys().next().cloned() {
            self.journal.write(&unsaved[&id])?;
            let record = unsaved.remove(&id).unwrap();
            self.remember(record);
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
        let mut input = serde_json::json!({"audio":meeting.audio,"transcript":transcript,"summary":summary,
            "notes": if kind == "summary" { Some(&meeting.notes) } else { None }});
        if kind == "summary" {
            input["attendees"] = serde_json::json!(meeting.attendees);
        }
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&input).map_err(message)?)
        ))
    }

    fn synchronize_recording(&self, core: &CoreHandle) -> Result<(), String> {
        self.recording.store(
            core.snapshot().map_err(message)?.active_session().is_some(),
            Ordering::Release,
        );
        Ok(())
    }

    pub fn drive(self: &Arc<Self>, core: &CoreHandle) -> Result<(), String> {
        // A job that finished anyway, or whose meeting changed, keeps its retry action.
        for id in std::mem::take(&mut *self.requeue.lock().unwrap()) {
            let _ = self.retry(core.clone(), &id);
        }
        let mut running = self.running.lock().unwrap();
        self.synchronize_recording(core)?;
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
        self.reconcile_speech_switch(&running);
        let mut queue = self.pending.lock().unwrap();
        let index = queue.iter().position(|job| {
            (!self.recording.load(Ordering::Acquire) || model_job(&job.record.kind))
                && self.live_allows(&job.record.kind, job.record.meeting_id.as_deref())
        });
        if let Some(index) = index {
            let job = queue.remove(index).unwrap();
            drop(queue);
            *running = Some(self.launch_checked(job)?);
        }
        Ok(())
    }
    pub fn agent_jobs(
        &self,
        meeting_id: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<serde_json::Value, nero_core::agent_control::ControlError> {
        if limit == 0 || limit > 200 {
            return Err(nero_core::agent_control::ControlError::invalid());
        }
        let records = self.records.lock().unwrap();
        let mut matching = records
            .iter()
            .filter(|record| meeting_id.is_none() || record.meeting_id.as_deref() == meeting_id)
            .collect::<Vec<_>>();
        matching.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        let jobs = matching
            .iter()
            .skip(offset)
            .take(limit)
            .map(|record| agent_job_view(record))
            .collect::<Vec<_>>();
        let next = offset.saturating_add(jobs.len());
        Ok(
            serde_json::json!({"jobs":jobs,"pagination":{"offset":offset,"returned":jobs.len(),"total":matching.len(),"hasMore":next < matching.len(),"nextOffset":(next < matching.len()).then_some(next)}}),
        )
    }

    pub fn agent_cancel(
        &self,
        id: &str,
        expected_version: &str,
    ) -> Result<serde_json::Value, nero_core::agent_control::ControlError> {
        self.cancel_checked(id, Some(expected_version))?;
        let records = self.records.lock().unwrap();
        let record = records.iter().find(|record| record.id == id);
        Ok(serde_json::json!({"applied":true,"job":record.map(agent_job_view)}))
    }

    pub fn cancel(&self, id: &str) -> Result<(), String> {
        self.cancel_checked(id, None).map_err(|error| error.message)
    }

    // An agent names the job state it saw; retry and acknowledge still check the state.
    pub fn agent_check_job(
        &self,
        id: &str,
        expected_version: &str,
    ) -> Result<(), nero_core::agent_control::ControlError> {
        let records = self.records.lock().unwrap();
        let record = records
            .iter()
            .find(|record| record.id == id)
            .ok_or_else(|| {
                nero_core::agent_control::ControlError::new("not_found", "작업을 찾을 수 없습니다.")
            })?;
        check_agent_job_version(record, Some(expected_version))
    }

    fn cancel_checked(
        &self,
        id: &str,
        expected_version: Option<&str>,
    ) -> Result<(), nero_core::agent_control::ControlError> {
        use nero_core::agent_control::ControlError;
        let invalid_state = |message: &str| ControlError::new("invalid_state", message);
        let storage_error = |message: String| ControlError::new("storage_error", message);
        let running = self.running.lock().unwrap();
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(invalid_state("앱을 종료하고 있습니다."));
        }
        let mut pending = self.pending.lock().unwrap();
        if let Some(index) = pending.iter().position(|job| job.record.id == id) {
            let mut record = pending[index].record.clone();
            check_agent_job_version(&record, expected_version)?;
            record.status = JobStatus::Cancelled;
            record.message = "취소했습니다.".into();
            record.updated_at = now().max(record.updated_at);
            self.save_job(record.clone()).map_err(storage_error)?;
            pending.remove(index);
            drop(pending);
            if record.kind == "transcribe" && !self.recording.load(Ordering::Acquire) {
                self.cancel_live_meeting(record.meeting_id.as_deref())
                    .map_err(storage_error)?;
            }
            self.update(|_| ());
            return Ok(());
        }
        drop(pending);
        let job = running
            .as_ref()
            .filter(|r| r.id == id && !r.handle.is_finished())
            .ok_or_else(|| invalid_state("현재 작업과 다른 요청입니다."))?;
        let mut progress = self.progress.lock().unwrap();
        if progress.id.as_deref() == Some(id) && progress.status != "running" {
            return Err(invalid_state("이미 종료된 작업입니다."));
        }
        if progress.committing || progress.summary_committing {
            return Err(invalid_state(if progress.kind == "summary" {
                "회의록을 저장하고 있습니다."
            } else {
                "음성 기록을 저장하고 있습니다."
            }));
        }
        let record = self
            .records
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.id == id)
            .cloned();
        if let Some(mut record) = record {
            check_agent_job_version(&record, expected_version)?;
            if expected_version.is_some() && record.status != JobStatus::Running {
                return Err(invalid_state("이미 취소되었거나 종료된 작업입니다."));
            }
            record.status = JobStatus::Cancelling;
            record.updated_at = now().max(record.updated_at);
            record.message = "취소 요청을 받았습니다.".into();
            self.save_job(record).map_err(storage_error)?;
        } else if expected_version.is_some() {
            return Err(invalid_state("작업을 찾을 수 없습니다."));
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

    #[cfg(test)]
    fn checkpoint_transcript<T>(
        &self,
        cancel: &AtomicBool,
        commit: impl FnOnce() -> nero_core::Result<T>,
    ) -> Result<T, String> {
        self.checkpoint_transcript_following(cancel, commit, |_, _| Ok(()))
    }

    fn checkpoint_transcript_following<T>(
        &self,
        cancel: &AtomicBool,
        commit: impl FnOnce() -> nero_core::Result<T>,
        following: impl FnOnce(&mut Progress, &T) -> Result<(), String>,
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
        match &result {
            Ok(value) => {
                following(&mut progress, value)?;
                self.save_phase(&progress, "postprocessing")?;
                progress.phase = Some("postprocessing".into());
            }
            // These core transcript errors precede current-document publication.
            Err(error) => self.apply_rejection(&mut progress, error)?,
        }
        progress.revision = self
            .revision
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        drop(progress);
        result.map_err(message)
    }

    // A core refusal at the save step stored nothing: the job ends with that reason
    // instead of leaving "saving" for the worker to report as needing a check.
    fn apply_rejection(
        &self,
        progress: &mut Progress,
        error: &nero_core::CoreError,
    ) -> Result<(), String> {
        let Some((phase, kind)) = crate::failure::core_failure_kind(error)
            .and_then(|kind| Some((rejection_phase(kind)?, kind)))
        else {
            return Ok(());
        };
        self.save_phase(progress, phase)?;
        progress.phase = Some(phase.into());
        progress.failure_kind = Some(kind);
        Ok(())
    }

    pub(super) fn record_rejection(&self, error: &nero_core::CoreError) -> Result<(), String> {
        let mut progress = self.progress.lock().unwrap();
        self.apply_rejection(&mut progress, error)?;
        progress.revision = self
            .revision
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        Ok(())
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
            self.save_job_or_keep(record)?;
        }
        Ok(())
    }

    pub fn meeting_idle<T>(
        &self,
        meeting_id: &str,
        action: impl FnOnce() -> nero_core::Result<T>,
    ) -> nero_core::Result<T> {
        // Hold the same ownership gate used by new jobs and capture startup through commit.
        let running = self.running.lock().unwrap();
        self.worker_ready()
            .map_err(nero_core::CoreError::InvalidState)?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(nero_core::CoreError::Unavailable);
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
            || self.live.lock().unwrap().as_ref().is_some_and(|job| {
                job.meeting_id == meeting_id && job.running.load(Ordering::Acquire)
            })
        {
            return Err(nero_core::CoreError::InvalidState(
                "이 회의의 처리가 끝난 뒤 다시 시도해 주세요.".into(),
            ));
        }
        action()
    }

    pub fn meeting_removal<T>(
        &self,
        core: &CoreHandle,
        ids: &[String],
        action: impl FnOnce() -> nero_core::Result<T>,
    ) -> nero_core::Result<T> {
        self.meeting_removal_with_preflight(core, ids, || Ok(()), action)
    }

    pub fn meeting_trash<T>(
        &self,
        core: &CoreHandle,
        ids: &[String],
        action: impl FnOnce() -> nero_core::Result<T>,
    ) -> nero_core::Result<T> {
        let mut running = self.running.lock().unwrap();
        let snapshot = self.removal_snapshot(core, ids)?;
        // Soft trash closes Core result admission before workers are joined.
        // A directory-sync error may still have committed the complete library.
        let result = action();
        let committed = result.is_ok()
            || (matches!(&result, Err(nero_core::CoreError::Storage(_)))
                && core.snapshot().ok().is_some_and(|current| {
                    ids.iter().all(|id| {
                        current
                            .meetings
                            .iter()
                            .any(|meeting| &meeting.id == id && meeting.deleted_at.is_some())
                    })
                }));
        if !committed {
            return result;
        }
        let cleanup = self.cancel_meeting_processing(&mut running, ids, &snapshot);
        match (result, cleanup) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(nero_core::CoreError::InvalidState(format!(
                "회의를 휴지통으로 이동했지만 처리 정리를 완료하지 못했습니다: {error}"
            ))),
            (Ok(value), Ok(())) => Ok(value),
        }
    }

    pub fn meeting_removal_with_preflight<T>(
        &self,
        core: &CoreHandle,
        ids: &[String],
        preflight: impl FnOnce() -> nero_core::Result<()>,
        action: impl FnOnce() -> nero_core::Result<T>,
    ) -> nero_core::Result<T> {
        let mut running = self.running.lock().unwrap();
        let snapshot = self.removal_snapshot(core, ids)?;
        preflight()?;
        self.cancel_meeting_processing(&mut running, ids, &snapshot)?;
        action()
    }

    fn removal_snapshot(&self, core: &CoreHandle, ids: &[String]) -> nero_core::Result<Snapshot> {
        self.worker_ready()
            .map_err(nero_core::CoreError::InvalidState)?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(nero_core::CoreError::InvalidState(
                "앱을 종료하고 있습니다.".into(),
            ));
        }
        let snapshot = core.snapshot()?;
        if ids.is_empty()
            || ids
                .iter()
                .any(|id| !snapshot.meetings.iter().any(|meeting| &meeting.id == id))
        {
            return Err(nero_core::CoreError::MeetingNotFound);
        }
        if snapshot
            .active_session()
            .is_some_and(|session| ids.contains(&session.meeting_id))
        {
            return Err(nero_core::CoreError::InvalidState(
                "녹음을 마친 뒤 삭제해 주세요.".into(),
            ));
        }
        Ok(snapshot)
    }

    fn cancel_meeting_processing(
        &self,
        running: &mut Option<Running>,
        ids: &[String],
        snapshot: &Snapshot,
    ) -> nero_core::Result<()> {
        let remove_live = self
            .live
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|job| ids.contains(&job.meeting_id));
        if remove_live {
            self.cancel_live()
                .map_err(nero_core::CoreError::InvalidState)?;
        }
        self.live_completed
            .lock()
            .unwrap()
            .retain(|_, result| !ids.contains(&result.meeting_id));
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
                    nero_core::CoreError::InvalidState(
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
                        .map_err(nero_core::CoreError::Storage)?;
                    pending.remove(index);
                } else {
                    index += 1;
                }
            }
        }
        self.worker_ready()
            .map_err(nero_core::CoreError::InvalidState)?;
        Ok(())
    }

    pub(super) fn bind_job(path: &Path, meeting_id: &str) -> Result<(), String> {
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

    pub fn bind_legacy_jobs(&self, core: &CoreHandle) -> nero_core::Result<()> {
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
        let path = directory(&self.root, "processing").map_err(nero_core::CoreError::Storage)?;
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
                Self::bind_job(&path, meeting_id).map_err(nero_core::CoreError::Storage)?;
            }
        }
        Ok(())
    }

    fn owned_jobs(&self) -> nero_core::Result<Vec<(PathBuf, JobOwner)>> {
        let mut jobs = Vec::new();
        let root = self.root.join("processing");
        if !root.exists() {
            return Ok(jobs);
        }
        let root = directory(&self.root, "processing").map_err(nero_core::CoreError::Storage)?;
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

    pub fn purge_meeting_jobs(&self, ids: &[String]) -> nero_core::Result<()> {
        for (path, owner) in self.owned_jobs()? {
            if ids.contains(&owner.meeting_id) {
                fs::remove_dir_all(path)?;
            }
        }
        let removed = |r: &JobRecord| r.meeting_id.as_ref().is_some_and(|id| ids.contains(id));
        self.unsaved.lock().unwrap().retain(|_, r| !removed(r));
        self.records.lock().unwrap().retain(|r| !removed(r));
        self.revision.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    pub fn cleanup_removed_meeting_jobs(&self, core: &CoreHandle) {
        let canonical_cleanup = core.retry_purge_cleanup();
        let result = (|| -> nero_core::Result<()> {
            let existing = core.kept_meeting_ids()?;
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
        // A startup notice, like other startup issues: as current progress it had no job
        // record to acknowledge and vanished when the next job started.
        if canonical_cleanup.is_err() || result.is_err() {
            self.report_startup_issue(
                "cleanup",
                "삭제한 회의의 임시 파일을 정리하지 못했습니다.".into(),
            );
        }
    }

    // Shown once the main window asks for processing status, or after that by the
    // poll loop for checks that finish later.
    pub fn report_startup_issue(&self, kind: &str, message: String) {
        self.startup_issues.lock().unwrap().push(Progress {
            id: Some(Uuid::new_v4().to_string()),
            kind: kind.into(),
            status: "failed".into(),
            message,
            ..Default::default()
        });
    }

    pub fn upgrade_transcript_sources(&self, core: &CoreHandle) {
        self.record_transcript_upgrade(core.upgrade_transcript_sources());
    }

    fn record_transcript_upgrade(
        &self,
        result: nero_core::Result<nero_core::TranscriptUpgradeReport>,
    ) {
        match result {
            Ok(report) if report.unavailable == 0 => return,
            // Quitting or a recording stopped the pass; the next launch runs it again.
            Err(nero_core::CoreError::Unavailable | nero_core::CoreError::RecordingBusy) => return,
            _ => {}
        }
        self.report_startup_issue(
            "transcript-upgrade",
            "일부 음성 기록의 화자 정보를 복구하지 못했습니다. 원문과 수정본은 유지했습니다. 앱을 다시 열면 재시도합니다.".into(),
        );
    }

    pub fn notify_startup_issues(&self, notify: impl FnOnce(&[Progress]) -> bool) {
        self.startup_issues_requested.store(true, Ordering::Release);
        self.notify_late_startup_issues(notify);
    }

    /// Delivers issues found after the main window first asked, once it listens.
    pub fn notify_late_startup_issues(&self, notify: impl FnOnce(&[Progress]) -> bool) {
        if !self.startup_issues_requested.load(Ordering::Acquire) {
            return;
        }
        let mut issues = self.startup_issues.lock().unwrap();
        if !issues.is_empty() && notify(&issues) {
            issues.clear();
        }
    }

    pub fn shutdown(&self) -> Result<(), String> {
        self.shutting_down.store(true, Ordering::Release);
        self.cancel_live()?;
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
    pub fn quiesce_for_update(&self) -> Result<(), String> {
        // Capture admission, queue admission and drive all take running first.
        // Keep that lock until the fence is set; do not cancel any user's work.
        let _retry = self.retry_gate.lock().unwrap();
        let running = self.running.lock().unwrap();
        let pending = self.pending.lock().unwrap();
        self.worker_ready()?;
        if self.shutting_down.load(Ordering::Acquire)
            || self.recording.load(Ordering::Acquire)
            || running
                .as_ref()
                .is_some_and(|job| !job.handle.is_finished())
            || !pending.is_empty()
            || self
                .live
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|job| job.running.load(Ordering::Acquire))
        {
            return Err("녹음과 진행 중인 작업을 마친 뒤 업데이트해 주세요.".into());
        }
        self.shutting_down.store(true, Ordering::Release);
        Ok(())
    }

    pub fn resume(&self) {
        self.shutting_down.store(false, Ordering::Release);
    }

    fn mark_interrupted(&self, job: &Running) -> bool {
        if job.handle.is_finished()
            || self.records.lock().unwrap().iter().any(|r| {
                r.id == job.id && matches!(r.status, JobStatus::Cancelled | JobStatus::Cancelling)
            })
        {
            return false;
        }
        self.interrupted_jobs.lock().unwrap().insert(job.id.clone());
        true
    }
    pub(super) fn start_job(
        self: &Arc<Self>,
        spec: JobSpec,
        action: impl FnOnce(Arc<Self>, Arc<AtomicBool>, PathBuf) -> Result<Option<String>, String>
            + Send
            + 'static,
    ) -> Result<Progress, String> {
        let JobSpec {
            kind,
            meeting_id,
            source,
            required_fingerprint,
            importing: importing_core,
            include_summary,
            template,
            requested_id,
            engine,
            speaker_enabled,
            apple_variant,
        } = spec;
        let source = match source {
            Some(core) => {
                let meeting = meeting_id.as_deref().ok_or("회의를 찾을 수 없습니다.")?;
                let fingerprint = Self::source_fingerprint(&core, meeting, kind)?;
                if required_fingerprint
                    .as_ref()
                    .is_some_and(|required| required != &fingerprint)
                {
                    return Err("회의 내용이 변경되어 이전 작업을 다시 실행하지 않았습니다.".into());
                }
                Some((core, fingerprint))
            }
            None => None,
        };
        let mut running = self.running.lock().unwrap();
        if let Some((core, _)) = &source {
            self.synchronize_recording(core)?;
        }

        self.job_ready()?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("앱을 종료하고 있습니다.".into());
        }
        let supported = if kind.starts_with("summary") {
            crate::summary::availability(self).0
        } else if matches!(kind, "diarize" | "speaker-model") {
            crate::diarization::availability(self).0
        } else if let Some(engine) = engine {
            self.engine_status(
                engine,
                apple_variant.or_else(|| self.apple_speech_variant()),
            )
            .0
        } else {
            self.supported_for(self.speech_engine())
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
        let id = requested_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        if Uuid::parse_str(&id).is_err()
            || self
                .records
                .lock()
                .unwrap()
                .iter()
                .any(|record| record.id == id)
        {
            return Err("작업 ID가 올바르지 않거나 이미 사용되었습니다.".into());
        }
        let inherited = self
            .records
            .lock()
            .unwrap()
            .iter()
            .find(|record| record.retry_successor_id.as_deref() == Some(&id))
            .map(|record| {
                (
                    record.speech_engine,
                    record.speaker_enabled,
                    record.apple_speech_variant,
                )
            });
        let engine = engine.or_else(|| inherited.map(|choice| choice.0));
        let speaker_enabled = speaker_enabled.or_else(|| inherited.map(|choice| choice.1));
        let apple_variant = apple_variant.or_else(|| inherited.and_then(|choice| choice.2));
        let created_at = now();
        let record = JobRecord {
            glossary_snapshot_required: kind == "transcribe",
            speech_engine: engine.unwrap_or_else(|| self.speech_engine()),
            speaker_enabled: speaker_enabled.unwrap_or_else(|| self.speaker_enabled()),
            apple_speech_variant: if engine.unwrap_or_else(|| self.speech_engine())
                == SpeechEngine::AppleSpeech
            {
                apple_variant.or_else(|| self.apple_speech_variant())
            } else {
                None
            },
            schema_version: 1,
            id: id.clone(),
            kind: kind.into(),
            meeting_id: meeting_id.clone(),
            source_fingerprint: source.as_ref().map(|(_, fingerprint)| fingerprint.clone()),
            include_summary,
            summary_template_ref: template,
            acknowledged: false,
            retry_successor_id: None,
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
            || !self.live_allows(kind, job.record.meeting_id.as_deref())
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
        let preparation = model_job(&job.record.kind);
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
                if preparation {
                    this.acknowledge_superseded(&job_id);
                }
            })
            .map_err(message)?;
        Ok(Running {
            id,
            cancel,
            handle,
            import_existing: job.import_existing,
        })
    }

    // A completed preparation answers earlier failed attempts of the same kind:
    // their notices are hidden and their outcomes stay.
    fn acknowledge_superseded(&self, completed: &str) {
        let superseded: Vec<JobRecord> = {
            let records = self.records.lock().unwrap();
            let Some(done) = records
                .iter()
                .find(|record| record.id == completed && record.status == JobStatus::Completed)
            else {
                return;
            };
            records
                .iter()
                .filter(|record| {
                    record.kind == done.kind
                        && !record.acknowledged
                        && matches!(
                            record.status,
                            JobStatus::Failed | JobStatus::Cancelled | JobStatus::Interrupted
                        )
                })
                .cloned()
                .collect()
        };
        for mut record in superseded {
            record.acknowledged = true;
            record.updated_at = now().max(record.updated_at);
            let _ = self.save_job(record);
        }
    }

    fn launch_checked(self: &Arc<Self>, job: Pending) -> Result<Running, String> {
        let mut record = job.record.clone();
        self.launch(job).map_err(|error| {
            record.status = JobStatus::Failed;
            record.message = error.clone();
            record.updated_at = now().max(record.updated_at);
            let _ = self.save_job_or_keep(record);
            error
        })
    }
    pub fn prepare_model(self: &Arc<Self>) -> Result<Progress, String> {
        self.prepare_meeting_models(false)
    }
    pub fn require_meeting_models(&self) -> Result<(), String> {
        meeting_models_ready(&self.snapshot())
    }
    pub fn prepare_meeting_models(
        self: &Arc<Self>,
        include_summary: bool,
    ) -> Result<Progress, String> {
        self.prepare_meeting_models_with_id(include_summary, None, None, None, None)
    }

    fn prepare_meeting_models_with_id(
        self: &Arc<Self>,
        include_summary: bool,
        requested_id: Option<String>,
        required_engine: Option<SpeechEngine>,
        required_speakers: Option<bool>,
        required_variant: Option<AppleVariant>,
    ) -> Result<Progress, String> {
        if include_summary {
            crate::release_capabilities::require_local_summary()?;
        }
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("앱을 종료하고 있습니다.".into());
        }
        let engine = required_engine.unwrap_or_else(|| self.speech_engine());
        let speaker_enabled = required_speakers.unwrap_or_else(|| self.speaker_enabled());
        let apple_variant = required_variant.or_else(|| self.apple_speech_variant());
        let progress = self.snapshot();
        if !self.engine_status(engine, apple_variant).0
            || (include_summary && !progress.summary_supported)
        {
            return Err("이 환경에서는 음성 인식과 회의 요약을 아직 사용할 수 없습니다.".into());
        }
        let required = self.remaining_speech_bytes(engine)?
            + if speaker_enabled && crate::diarization::availability(self).0 {
                self.remaining_model_bytes(crate::diarization::SPEAKER_MODEL)?
            } else {
                0
            }
            + if include_summary {
                self.remaining_model_bytes(crate::summary::SUMMARY_MODEL)?
            } else {
                0
            };
        crate::storage_space::check(&self.root, required)?;
        self.start_job(
            JobSpec {
                kind: "setup",
                include_summary,
                requested_id,
                engine: Some(engine),
                speaker_enabled: Some(speaker_enabled),
                apple_variant,
                ..JobSpec::default()
            },
            move |this, cancel, path| {
                let progress = this.snapshot();
                let total = Self::speech_bytes(engine)
                    + if speaker_enabled && crate::diarization::availability(&this).0 {
                        crate::diarization::model().bytes()
                    } else {
                        0
                    }
                    + if include_summary {
                        progress.summary_bytes
                    } else {
                        0
                    };
                let mut first = if engine == SpeechEngine::AppleSpeech {
                    this.update(|p| {
                        p.phase = Some("preparing".into());
                        p.message = "macOS 한국어 언어 자산을 준비하고 있습니다.".into();
                    });
                    let result = crate::apple_speech::prepare(
                        &this,
                        &path,
                        &cancel,
                        apple_variant.ok_or("macOS 전사 방식을 다시 선택해 주세요.")?,
                    );
                    *this.apple_speech.lock().unwrap() =
                        crate::apple_speech::probe(&this.resources, apple_variant);
                    let status = result.map_err(|error| this.report(error))?;
                    *this.apple_speech.lock().unwrap() = status;
                    0
                } else {
                    let mut done = 0;
                    for engine in engine.requirements() {
                        done +=
                            this.prepare_files(ModelSpec::speech(engine), &cancel, done, total)?;
                    }
                    done
                };
                if speaker_enabled && crate::diarization::availability(&this).0 {
                    first += this.prepare_files(
                        crate::diarization::SPEAKER_MODEL,
                        &cancel,
                        first,
                        total,
                    )?;
                }
                if include_summary {
                    this.prepare_files(crate::summary::SUMMARY_MODEL, &cancel, first, total)?;
                }
                Ok(None)
            },
        )
    }
    pub(super) fn prepare_named_model(
        self: &Arc<Self>,
        spec: ModelSpec,
        kind: &'static str,
    ) -> Result<Progress, String> {
        self.prepare_named_model_with_id(spec, kind, None)
    }

    fn prepare_named_model_with_id(
        self: &Arc<Self>,
        spec: ModelSpec,
        kind: &'static str,
        requested_id: Option<String>,
    ) -> Result<Progress, String> {
        if kind == "summary-model" {
            crate::release_capabilities::require_local_summary()?;
        }
        crate::storage_space::check(&self.root, self.remaining_model_bytes(spec)?)?;
        self.start_job(
            JobSpec {
                kind,
                requested_id,
                ..JobSpec::default()
            },
            move |this, cancel, _path| {
                this.prepare_files(spec, &cancel, 0, spec.model().bytes())?;
                Ok(None)
            },
        )
    }
    fn remaining_model_bytes(&self, spec: ModelSpec) -> Result<u64, String> {
        let model = spec.model();
        let target = self.root.join(spec.relative_dir());
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
        spec: ModelSpec,
        cancel: &AtomicBool,
        offset: u64,
        total: u64,
    ) -> Result<u64, String> {
        let model = spec.model();
        let target = self.model_dir_of(spec)?;
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
                    p.model_stage = Some(spec.stage.id().into());
                    p.message = format!("{} 확인 중", spec.stage.title());
                    p.bytes = offset + done;
                    p.total_bytes = total;
                });
                let destination = target.join(&item.name);
                let matches = matches_file(&destination, item, cancel);
                cancelled(cancel)?;
                if !matches {
                    self.update(|p| p.message = format!("{} 다운로드 중", spec.stage.title()));
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
        file.write_all(spec.manifest.as_bytes())
            .map_err(crate::storage_space::write_error)?;
        file.sync_all().map_err(crate::storage_space::write_error)?;
        drop(file);
        fs::rename(marker, target.join("verified.json")).map_err(message)?;
        // Bytes set aside by an earlier attempt are not needed once every file is verified.
        if let Ok(entries) = fs::read_dir(&target) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let set_aside = name.starts_with("verified.invalid-")
                    || model.files.iter().any(|file| {
                        name.starts_with(&format!("{}.invalid-", file.name))
                            || name.starts_with(&format!("{}.partial-", file.name))
                    });
                if set_aside && entry.file_type().is_ok_and(|kind| kind.is_file()) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        Ok(model.bytes())
    }
    pub fn recording_start<T>(
        &self,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let mut running = self.running.lock().unwrap();
        self.worker_ready()?;
        if self.live.lock().unwrap().as_ref().is_some_and(|job| {
            self.recording.load(Ordering::Acquire) && job.running.load(Ordering::Acquire)
        }) {
            return Err("이미 녹음 중입니다.".into());
        }
        self.cancel_live()?;
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
                let interrupted = self.mark_interrupted(&job);
                job.cancel.store(true, Ordering::Release);
                job.handle
                    .join()
                    .map_err(|_| "처리 작업의 종료를 확인하지 못했습니다.")?;
                if interrupted {
                    self.requeue.lock().unwrap().push(job.id.clone());
                }
            }
        }
        self.worker_ready()?;
        let result = action();
        if result.is_ok() {
            self.recording.store(true, Ordering::Release);
        }
        result
    }

    pub fn recording_change<T>(
        &self,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let _running = self.running.lock().unwrap();
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("앱을 종료하고 있습니다.".into());
        }
        action()
    }

    pub fn acknowledge(&self, id: &str) -> Result<Progress, String> {
        let _retry = self.retry_gate.lock().unwrap();
        let _running = self.running.lock().unwrap();
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("앱을 종료하고 있습니다.".into());
        }
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
            JobStatus::Completed
                | JobStatus::Failed
                | JobStatus::Interrupted
                | JobStatus::NeedsReview
                | JobStatus::Cancelled
        ) {
            return Err("진행 중인 작업의 알림은 닫을 수 없습니다.".into());
        }
        record.acknowledged = true;
        record.updated_at = now().max(record.updated_at);
        self.save_job(record)?;
        Ok(self.snapshot())
    }

    pub fn retry(self: &Arc<Self>, core: CoreHandle, id: &str) -> Result<Progress, String> {
        self.retry_with(&core, id, |record, successor_id| {
            self.schedule_retry(core.clone(), record, successor_id)
        })
    }

    fn retry_with(
        &self,
        core: &CoreHandle,
        id: &str,
        schedule: impl FnOnce(&JobRecord, &str) -> Result<(), String>,
    ) -> Result<Progress, String> {
        let _retry = self.retry_gate.lock().unwrap();
        self.job_ready()?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("앱을 종료하고 있습니다.".into());
        }
        let record = self
            .records
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.id == id)
            .cloned()
            .ok_or("작업을 찾을 수 없습니다.")?;
        if let Some(reason) = retry_block_reason(&record) {
            return Err(reason.into());
        }
        if let Some(meeting_id) = &record.meeting_id {
            if record.source_fingerprint.as_ref()
                != Some(&Self::source_fingerprint(&core, meeting_id, &record.kind)?)
            {
                return Err("회의 내용이 변경되어 이전 작업을 다시 실행하지 않았습니다. 현재 회의에서 작업을 시작해 주세요.".into());
            }
        }
        let successor_id = Uuid::new_v4().to_string();
        let mut handed_off = record.clone();
        handed_off.status = JobStatus::Cancelled;
        handed_off.acknowledged = true;
        handed_off.retry_successor_id = Some(successor_id.clone());
        handed_off.phase = Some("retryPending".into());
        handed_off.message = "재시도를 준비하고 있습니다.".into();
        handed_off.updated_at = now().max(handed_off.updated_at);
        self.save_job(handed_off.clone())?;
        let scheduled = schedule(&record, &successor_id);
        let known_successor = self
            .records
            .lock()
            .unwrap()
            .iter()
            .any(|item| item.id == successor_id);
        let exists = if known_successor {
            Ok(true)
        } else {
            self.journal.retry_successor_exists(&handed_off)
        }
        .map_err(|error| {
            *self.journal_error.lock().unwrap() = Some(error.clone());
            error
        })?;
        if !exists {
            self.save_job_or_keep(record)?;
            return Err(scheduled.err().unwrap_or_else(|| {
                "재시도 작업이 만들어지지 않았습니다. 현재 상태를 확인해 주세요.".into()
            }));
        }
        handed_off.phase = None;
        handed_off.message = "새 작업으로 다시 시도했습니다.".into();
        self.save_job_or_keep(handed_off)?;
        scheduled?;
        self.update(|_| ());
        Ok(self.snapshot())
    }
    fn schedule_retry(
        self: &Arc<Self>,
        core: CoreHandle,
        record: &JobRecord,
        successor_id: &str,
    ) -> Result<(), String> {
        match record.kind.as_str() {
            "setup" => self.prepare_meeting_models_with_id(
                record.include_summary,
                Some(successor_id.into()),
                Some(record.speech_engine),
                Some(record.speaker_enabled),
                record.apple_speech_variant,
            )?,
            "model" => self.prepare_meeting_models_with_id(
                false,
                Some(successor_id.into()),
                Some(record.speech_engine),
                Some(record.speaker_enabled),
                record.apple_speech_variant,
            )?,
            "summary-model" => self.prepare_named_model_with_id(
                crate::summary::SUMMARY_MODEL,
                "summary-model",
                Some(successor_id.into()),
            )?,
            "speaker-model" => self.prepare_named_model_with_id(
                crate::diarization::SPEAKER_MODEL,
                "speaker-model",
                Some(successor_id.into()),
            )?,
            "transcribe" => self.transcribe_bound(
                core,
                record.meeting_id.clone().ok_or("회의가 없습니다.")?,
                record.source_fingerprint.clone(),
                Some(successor_id.into()),
                None,
                Some(record.speech_engine),
                Some(record.speaker_enabled),
                record.apple_speech_variant,
            )?,
            "summary" => self.summarize_bound_with_template(
                core,
                record.meeting_id.clone().ok_or("회의가 없습니다.")?,
                record.source_fingerprint.clone(),
                record.summary_template_ref.clone(),
                Some(successor_id.into()),
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
                    Some(successor_id.into()),
                )?
            }
            _ => return Err("지원하지 않는 작업입니다.".into()),
        };
        Ok(())
    }
    pub fn import(self: &Arc<Self>, core: CoreHandle, source: PathBuf) -> Result<Progress, String> {
        self.import_target(core, source, None, None)
    }

    // A file dropped on the window: its cache copy is removed when the job is dropped.
    pub(super) fn import_dropped(
        self: &Arc<Self>,
        core: CoreHandle,
        upload: crate::drop_import::Finished,
    ) -> Result<Progress, String> {
        let cleanup = crate::drop_import::RemoveOnDrop(upload.directory);
        if let Some(target) = &upload.target {
            core.validate_draft_attachment(target.clone())
                .map_err(message)?;
        }
        self.import_target(core, upload.path, upload.target, Some(cleanup))
    }

    pub fn import_into_draft(
        self: &Arc<Self>,
        core: CoreHandle,
        source: PathBuf,
        target: nero_core::DraftAttachmentTarget,
    ) -> Result<Progress, String> {
        core.validate_draft_attachment(target.clone())
            .map_err(message)?;
        self.import_target(core, source, Some(target), None)
    }

    fn import_target(
        self: &Arc<Self>,
        core: CoreHandle,
        source: PathBuf,
        target: Option<nero_core::DraftAttachmentTarget>,
        cleanup: Option<crate::drop_import::RemoveOnDrop>,
    ) -> Result<Progress, String> {
        self.start_job(
            JobSpec {
                kind: "import",
                meeting_id: target.as_ref().map(|target| target.meeting_id.clone()),
                importing: target.is_none().then(|| core.clone()),
                ..JobSpec::default()
            },
            move |this, cancel, path| {
                let _cleanup = cleanup;
                if let Some(target) = &target {
                    core.validate_draft_attachment(target.clone())
                        .map_err(message)?;
                }
                let imported = crate::speech_audio::stage_import(&source, &path, &cancel)?;
                let copied = imported.path;
                // Core keeps its own copy of the original; an import is never retried.
                let _staged = tempfile::TempPath::try_from_path(&copied).ok();
                this.update(|p| p.message = "음성을 준비하고 있습니다.".into());
                let prepared =
                    crate::speech_audio::AnalysisAudio::prepare(&this, &copied, &path, &cancel)
                        .map_err(|error| this.report(error))?;
                cancelled(&cancel)?;
                let title = imported.title;
                let meeting = this.commit_transcript_result(&cancel, || {
                    let attaching = target.is_some();
                    let result = match target {
                        Some(target) => {
                            core.import_audio_into_draft(target, &copied, prepared.path())
                        }
                        None => core.import_audio(&title, &copied, prepared.path()),
                    };
                    let rejection = if attaching {
                        // A concurrent edit or a recording that took the draft also changed
                        // the target; an invalid input is not a refused result.
                        match &result {
                            Err(
                                nero_core::CoreError::EditConflict
                                | nero_core::CoreError::RecordingBusy,
                            ) => Some(crate::failure::FailureKind::SourceChanged),
                            Err(error) => crate::failure::core_failure_kind(error)
                                .filter(|kind| *kind != crate::failure::FailureKind::Output),
                            Ok(_) => None,
                        }
                        .and_then(|kind| Some((rejection_phase(kind)?, kind)))
                    } else {
                        None
                    };
                    if let Some((phase, kind)) = rejection {
                        this.update(|progress| {
                            progress.phase = Some(phase.into());
                            progress.failure_kind = Some(kind);
                        });
                    }
                    result.map_err(message)
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
        self.transcribe_bound(core, meeting_id, None, None, None, None, None, None)
    }
    fn transcribe_bound(
        self: &Arc<Self>,
        core: CoreHandle,
        meeting_id: String,
        required_fingerprint: Option<String>,
        requested_id: Option<String>,
        required_session: Option<String>,
        required_engine: Option<SpeechEngine>,
        required_speakers: Option<bool>,
        required_variant: Option<AppleVariant>,
    ) -> Result<Progress, String> {
        let session_id = core
            .snapshot()
            .map_err(message)?
            .meetings
            .iter()
            .find(|m| m.id == meeting_id)
            .and_then(|m| m.audio.as_ref())
            .ok_or("녹음 원본을 찾을 수 없습니다.")?
            .session_id
            .clone();
        if required_session
            .as_ref()
            .is_some_and(|required| required != &session_id)
        {
            return Err("녹음 원본이 변경되어 이전 녹음 처리를 시작하지 않았습니다.".into());
        }
        let audio = core.audio_path(&meeting_id).map_err(message)?;
        let expected = core
            .transcript(&meeting_id)
            .map_err(message)?
            .map(|d| d.version);
        let session_choice = if required_session.is_some() {
            crate::speech_choice::load_session(&self.root, &session_id)?
        } else {
            None
        };
        let apple_variant = required_variant
            .or_else(|| {
                session_choice
                    .as_ref()
                    .and_then(|choice| choice.apple_speech_variant)
            })
            .or_else(|| self.apple_speech_variant());
        let speaker_enabled = required_speakers
            .or_else(|| session_choice.as_ref().map(|choice| choice.speaker_enabled))
            .unwrap_or_else(|| self.speaker_enabled());
        let engine = required_engine
            .or_else(|| session_choice.as_ref().map(|choice| choice.engine))
            .or_else(|| {
                self.live
                    .lock()
                    .unwrap()
                    .as_ref()
                    .filter(|job| job.session_id == session_id)
                    .map(|job| job.engine)
            })
            .unwrap_or_else(|| self.speech_engine());
        if self.engine_status(engine, apple_variant) != (true, true) {
            return Err("먼저 음성 인식 모델을 준비해 주세요.".into());
        }
        let job_id = requested_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let previous_job = self
            .records
            .lock()
            .unwrap()
            .iter()
            .find(|record| record.retry_successor_id.as_deref() == Some(&job_id))
            .map(|record| (record.id.clone(), record.glossary_snapshot_required));
        let inherited = if let Some((previous, required)) = previous_job {
            let previous_path = self.root.join("processing").join(previous);
            Some(crate::glossary_context::retry_context(
                &previous_path,
                &meeting_id,
                required,
            )?)
        } else if required_session.is_some() {
            Some(crate::glossary_context::capture_context(
                &self.root,
                &session_id,
                &meeting_id,
                session_choice.as_ref(),
            )?)
        } else {
            None
        };
        let job_path = directory(&self.root, &format!("processing/{job_id}"))?;
        let glossary = crate::glossary_context::freeze_job(
            &self.root,
            &job_path,
            &meeting_id,
            inherited.as_ref(),
        )
        .inspect_err(|_| discard_unrecorded(&job_path))?;
        self.start_job(
            JobSpec {
                kind: "transcribe",
                meeting_id: Some(meeting_id.clone()),
                source: Some(core.clone()),
                required_fingerprint,
                requested_id: Some(job_id),
                engine: Some(engine),
                speaker_enabled: Some(speaker_enabled),
                apple_variant,
                ..JobSpec::default()
            },
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
                if core.snapshot().map_err(message)?.active_session().is_some() {
                    return Err("녹음을 마친 뒤 음성 인식을 시작해 주세요.".into());
                }
                this.update(|p| {
                    p.phase = Some("preparing".into());
                    p.message = "모델을 확인하고 있습니다.".into();
                });
                // Apple Speech has no Nero model files; its job folder stands in.
                let directory = if engine == SpeechEngine::AppleSpeech {
                    path.clone()
                } else {
                    this.verified_model_dir(ModelSpec::speech(engine), &cancel)
                        .map_err(|error| error.message)?
                };
                this.update(|p| {
                    p.phase = Some("recognizing".into());
                    p.message = if speaker_enabled {
                        "마지막 구간과 화자를 정리하고 있습니다."
                    } else {
                        "마지막 구간을 정리하고 있습니다."
                    }
                    .into();
                });
                let _finalization = this.wait_live_asr(&meeting_id, &session_id, &cancel)?;
                // Qwen keeps the live audio and speaker stream but never commits the
                // Nemotron preview text; its final transcript is recognized below.
                let live_engine = engine.live_asr();
                let live_model = Self::model_for(live_engine);
                let live = match this.take_live(&meeting_id, &session_id) {
                    Some(live) => Some(live),
                    None => this.recover_live(&meeting_id, &session_id, &audio, &cancel)?,
                }
                .filter(|live| {
                    serde_json::from_slice::<serde_json::Value>(&live.raw).is_ok_and(|value| {
                        engine != SpeechEngine::AppleSpeech
                            && value["glossaryContext"] == glossary.evidence(live_engine)
                            && value["model"] == live_model.repository
                            && value["modelRevision"] == live_model.revision
                    })
                });
                let prepared = match &live {
                    Some(live) => {
                        crate::speech_audio::AnalysisAudio::from_live(&audio, &live.audio, &cancel)
                    }
                    None => {
                        crate::speech_audio::AnalysisAudio::prepare(&this, &audio, &path, &cancel)
                    }
                }
                .map_err(|error| this.report(error))?;
                this.update(|p| {
                    p.phase = Some("recognizing".into());
                    p.message = "텍스트로 변환 중".into();
                });
                let raw = match &live {
                    Some(live) if engine == live_engine => Ok(live.raw.clone()),
                    _ => match engine {
                        SpeechEngine::QwenKo => {
                            this.update(|p| {
                                p.message =
                                    "녹음 전체를 한국어 모델로 다시 전사하고 있습니다.".into();
                            });
                            crate::qwen_asr::recognize(
                                &this.resources,
                                prepared.path(),
                                &path,
                                &cancel,
                                &directory,
                            )
                        }
                        SpeechEngine::AppleSpeech => crate::apple_speech::recognize(
                            &this,
                            prepared.path(),
                            &path,
                            &cancel,
                            apple_variant.ok_or("macOS 전사 방식을 다시 선택해 주세요.")?,
                        ),
                        SpeechEngine::Nemotron => crate::asr::recognize(
                            &this,
                            prepared.path(),
                            &path,
                            &cancel,
                            &directory,
                        ),
                        SpeechEngine::WhisperTurbo | SpeechEngine::WhisperLargeV3 => {
                            crate::whisper_asr::recognize(
                                &this.resources,
                                prepared.path(),
                                &path,
                                &cancel,
                                &directory,
                                engine,
                                &glossary.prompt(),
                            )
                        }
                    },
                }
                .map_err(|error| {
                    if engine == SpeechEngine::AppleSpeech
                        && this.apple_speech_variant() == apple_variant
                    {
                        *this.apple_speech.lock().unwrap() =
                            crate::apple_speech::probe(&this.resources, apple_variant);
                    }
                    this.report(error)
                })?;
                let mut raw_value: serde_json::Value =
                    serde_json::from_slice(&raw).map_err(message)?;
                raw_value["glossaryContext"] = glossary.evidence(engine);
                let raw = serde_json::to_vec(&raw_value).map_err(message)?;
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path.join("recognition.json"))
                    .map_err(message)?;
                output
                    .write_all(&raw)
                    .map_err(crate::storage_space::write_error)?;
                output
                    .sync_all()
                    .map_err(crate::storage_space::write_error)?;
                prepared
                    .verify(&audio, &cancel)
                    .map_err(|error| error.message)?;
                cancelled(&cancel)?;
                let source = this.checkpoint_transcript_following(
                    &cancel,
                    || match engine {
                        SpeechEngine::AppleSpeech => {
                            core.commit_apple_transcript(&meeting_id, expected, raw)
                        }
                        SpeechEngine::Nemotron => {
                            core.commit_nemotron_transcript(&meeting_id, expected, raw)
                        }
                        SpeechEngine::WhisperTurbo | SpeechEngine::WhisperLargeV3 => {
                            core.commit_whisper_transcript(&meeting_id, expected, raw)
                        }
                        SpeechEngine::QwenKo => {
                            core.commit_qwen_transcript(&meeting_id, expected, raw)
                        }
                    },
                    |progress, source| {
                        if speaker_enabled {
                            this.bind_speaker_continuation(progress, &core, &meeting_id, source)
                        } else {
                            Ok(())
                        }
                    },
                )?;
                // Once the baseline is saved, no retry reads the live analysis WAV again.
                let _live_audio = live
                    .as_ref()
                    .and_then(|live| tempfile::TempPath::try_from_path(&live.audio).ok());
                this.update(|p| p.live_transcript = None);
                this.wait_live(&meeting_id, &session_id, &cancel)?;
                crate::terminology::correct(
                    &this,
                    &core,
                    &meeting_id,
                    source,
                    terminology,
                    &path,
                    &cancel,
                )?;
                cancelled(&cancel)?;
                if !speaker_enabled {
                    return Ok(Some(meeting_id));
                }
                // Recognition is already durable. A speaker-stage retry must never rerun ASR.
                let source = core
                    .transcript(&meeting_id)
                    .map_err(message)?
                    .ok_or("음성 기록을 찾을 수 없습니다.")?;
                this.verify_speaker_continuation(&core, &meeting_id, &source)?;
                if let Some(live) = live
                    .as_ref()
                    .filter(|live| live.speaker.join("receipt.json").is_file())
                {
                    let draft = crate::diarization::completed_live(
                        &this,
                        source,
                        prepared.path(),
                        &live.speaker,
                        &live.backend,
                        &cancel,
                    )
                    .map_err(|e| e.message)?;
                    prepared.verify(&audio, &cancel).map_err(|e| e.message)?;
                    this.commit_transcript_result(&cancel, || {
                        core.apply_diarization(&meeting_id, draft).map_err(message)
                    })?;
                } else {
                    crate::diarization::analyze_recording(
                        &this,
                        &core,
                        &meeting_id,
                        source,
                        &audio,
                        Some(&prepared),
                        &path,
                        &cancel,
                    )
                    .map_err(|error| this.report(error))?;
                }
                Ok(Some(meeting_id))
            },
        )
        .inspect_err(|_| discard_unrecorded(&job_path))
    }

    fn bind_speaker_continuation(
        &self,
        progress: &mut Progress,
        core: &CoreHandle,
        meeting_id: &str,
        source: &nero_core::Transcript,
    ) -> Result<(), String> {
        let snapshot = core.snapshot().map_err(message)?;
        let meeting = snapshot
            .meetings
            .iter()
            .find(|meeting| meeting.id == meeting_id && meeting.deleted_at.is_none())
            .ok_or("회의를 찾을 수 없습니다.")?;
        let input = serde_json::json!({"audio":meeting.audio,"transcript":source,"summary":null,"notes":null});
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&input).map_err(message)?)
        );
        let mut record = self
            .records
            .lock()
            .unwrap()
            .iter()
            .find(|record| Some(&record.id) == progress.id.as_ref())
            .cloned()
            .ok_or("음성 처리 작업을 찾을 수 없습니다.")?;
        if !matches!(record.kind.as_str(), "transcribe" | "diarize")
            || record.status != JobStatus::Running
            || record.meeting_id.as_deref() != Some(meeting_id)
        {
            return Err("음성 처리 작업이 변경되었습니다.".into());
        }
        record.kind = "diarize".into();
        record.source_fingerprint = Some(fingerprint);
        record.phase = Some("postprocessing".into());
        record.updated_at = now().max(record.updated_at);
        self.save_job_or_keep(record)?;
        progress.kind = "diarize".into();
        progress.committing = false;
        progress.summary_committing = false;
        Ok(())
    }

    fn verify_speaker_continuation(
        &self,
        core: &CoreHandle,
        meeting_id: &str,
        source: &nero_core::Transcript,
    ) -> Result<(), String> {
        let fingerprint = Self::source_fingerprint(core, meeting_id, "diarize")?;
        let progress = self.progress.lock().unwrap();
        let records = self.records.lock().unwrap();
        let record = records
            .iter()
            .find(|record| Some(&record.id) == progress.id.as_ref())
            .ok_or("음성 처리 작업을 찾을 수 없습니다.")?;
        if record.kind != "diarize"
            || record.meeting_id.as_deref() != Some(meeting_id)
            || record.source_fingerprint.as_deref() != Some(&fingerprint)
            || core
                .transcript(meeting_id)
                .map_err(message)?
                .as_ref()
                .map(|current| &current.version)
                != Some(&source.version)
        {
            return Err(
                "음성 기록이 변경되어 화자 구분을 시작하지 않았습니다. 현재 기록을 확인해 주세요."
                    .into(),
            );
        }
        Ok(())
    }

    pub(super) fn checkpoint_terminology(
        &self,
        core: &CoreHandle,
        meeting_id: &str,
        cancel: &AtomicBool,
        commit: impl FnOnce() -> nero_core::Result<Option<nero_core::Transcript>>,
    ) -> Result<Option<nero_core::Transcript>, String> {
        self.checkpoint_transcript_following(cancel, commit, |progress, result| {
            if let Some(source) = result {
                if self
                    .records
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|record| Some(&record.id) == progress.id.as_ref())
                    .is_some_and(|record| record.speaker_enabled)
                {
                    self.bind_speaker_continuation(progress, core, meeting_id, source)?;
                }
            }
            Ok(())
        })
    }

    #[cfg(test)]
    fn continue_with_speakers(
        &self,
        core: &CoreHandle,
        meeting_id: &str,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        let source = core
            .transcript(meeting_id)
            .map_err(message)?
            .ok_or("음성 기록을 찾을 수 없습니다.")?;
        let mut progress = self.progress.lock().unwrap();
        cancelled(cancel)?;
        self.job_ready()?;
        self.bind_speaker_continuation(&mut progress, core, meeting_id, &source)?;
        progress.phase = Some("preparing".into());
        progress.message = "화자 구분 준비 중".into();
        progress.failure_kind = None;
        progress.revision = self
            .revision
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        Ok(())
    }

    pub fn transcribe_recording(
        self: &Arc<Self>,
        core: CoreHandle,
        snapshot: &Snapshot,
    ) -> Result<Option<Progress>, String> {
        let Some(meeting_id) = self.recording_transcription.lock().unwrap().claim(snapshot) else {
            return Ok(None);
        };
        self.update(|p| {
            if let Some(preview) = p.live_transcript.as_mut().filter(|preview| {
                preview["meetingId"] == meeting_id
                    && snapshot
                        .session
                        .as_ref()
                        .is_some_and(|session| preview["sessionId"] == session.id)
            }) {
                preview["finishing"] = serde_json::json!(true);
                preview["message"] = serde_json::json!("마지막 구간을 정리하고 있습니다.");
            }
        });
        self.transcribe_bound(
            core,
            meeting_id,
            None,
            None,
            snapshot.session.as_ref().map(|session| session.id.clone()),
            None,
            None,
            None,
        )
        .map(Some)
    }
}

#[cfg(all(test, windows))]
mod speech_choice_tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Arc<Processing>) {
        let directory = tempfile::tempdir().unwrap();
        let resources = directory.path().join("resources");
        for path in [
            resources.join("native/diarization/cpu/bin/nemo-speech.exe"),
            crate::whisper_asr::executable(&resources),
            resources.join("native/whisper/runtime.json"),
            resources.join("native/whisper/bin/nero-vulkan-probe.exe"),
            resources.join("native/whisper/vulkan/bin/whisper-cli.exe"),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"synthetic unexecuted runtime").unwrap();
        }
        let root = directory.path().join("library");
        fs::create_dir(&root).unwrap();
        let process = Processing::new(root, resources);
        (directory, process)
    }
    #[test]
    fn choices_use_real_manifest_sizes_and_selection_does_not_download_or_reprocess() {
        let (_temporary, process) = fixture();
        let before = process.snapshot();
        assert_eq!(before.speech_engine, Some(SpeechEngine::Nemotron));
        assert_eq!(before.speech_choices.len(), 3);
        let after = process
            .set_speech_engine(SpeechEngine::WhisperTurbo)
            .unwrap();
        assert_eq!(after.speech_engine, Some(SpeechEngine::WhisperTurbo));
        assert!(!after.model_ready);
        assert_eq!(after.model_bytes, 574041195);
        assert!(after.jobs.is_empty());
        assert!(!process.root.join("meetings").exists());
        assert_eq!(
            crate::speech_choice::load(&process.root).unwrap().engine,
            SpeechEngine::WhisperTurbo
        );
        assert!(!process
            .model_dir()
            .unwrap()
            .join("ggml-large-v3-turbo-q5_0.bin")
            .exists());
    }
    #[test]
    fn running_jobs_keep_the_captured_engine_and_reject_preference_changes() {
        let (_temporary, process) = fixture();
        let (started, begin) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let job = process
            .start_job(
                JobSpec {
                    kind: "transcribe",
                    engine: Some(SpeechEngine::WhisperTurbo),
                    ..JobSpec::default()
                },
                move |_, _, _| {
                    started.send(()).unwrap();
                    wait.recv_timeout(Duration::from_secs(5)).unwrap();
                    Ok(None)
                },
            )
            .unwrap();
        begin.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(process
            .set_speech_engine(SpeechEngine::WhisperTurbo)
            .is_err());
        assert_eq!(process.speech_engine(), SpeechEngine::Nemotron);
        assert_eq!(
            process
                .records
                .lock()
                .unwrap()
                .iter()
                .find(|record| Some(&record.id) == job.id.as_ref())
                .unwrap()
                .speech_engine,
            SpeechEngine::WhisperTurbo
        );
        release.send(()).unwrap();
    }
    #[test]
    fn corrupt_preference_has_no_confirmed_selection_until_the_user_reselects() {
        let (_temporary, process) = fixture();
        fs::create_dir_all(process.root.join("preferences")).unwrap();
        fs::write(process.root.join("preferences/speech.json"), b"invalid").unwrap();
        let reopened = Processing::new(process.root.clone(), process.resources.clone());
        assert_eq!(reopened.snapshot().speech_engine, None);
        assert!(!reopened.snapshot().supported);
        let fixed = reopened.set_speech_engine(SpeechEngine::Nemotron).unwrap();
        assert_eq!(fixed.speech_engine, Some(SpeechEngine::Nemotron));
        assert!(fixed.supported);
    }
}

#[cfg(test)]
#[path = "processing_job_tests.rs"]
mod job_tests;

#[cfg(test)]
mod tests {
    use super::*;

    mod recording_ownership_tests {
        use super::*;
        use crate::lifecycle::{ExitKind, Lifecycle};
        use crate::live_speech::waiting_fixture;
        use std::sync::mpsc;

        #[test]
        fn current_live_job_survives_a_delayed_prior_registration() {
            let (_temporary, process) = setup();
            let (captured, ready) = mpsc::channel();
            let (register, allowed) = mpsc::channel();
            let old_process = process.clone();
            let old = thread::spawn(move || {
                old_process
                    .recording_start(|| {
                        captured.send(()).unwrap();
                        allowed.recv().unwrap();
                        drop(waiting_fixture(&old_process, "old-meeting", "old-session"));
                        Ok(())
                    })
                    .unwrap();
            });
            ready.recv().unwrap();
            assert!(process.running.try_lock().is_err());
            let next_process = process.clone();
            let next = thread::spawn(move || {
                next_process
                    .recording_change(|| {
                        assert_eq!(
                            next_process
                                .live
                                .lock()
                                .unwrap()
                                .as_ref()
                                .unwrap()
                                .meeting_id,
                            "old-meeting",
                        );
                        next_process.recording.store(false, Ordering::Release);
                        Ok(())
                    })
                    .unwrap();
                next_process
                    .recording_start(|| {
                        drop(waiting_fixture(&next_process, "new-meeting", "new-session"));
                        Ok(())
                    })
                    .unwrap();
            });
            register.send(()).unwrap();
            old.join().unwrap();
            next.join().unwrap();
            let (meeting_id, still_running) = {
                let live = process.live.lock().unwrap();
                let job = live.as_ref().unwrap();
                (job.meeting_id.clone(), job.running.load(Ordering::Acquire))
            };
            process.cancel_live().unwrap();
            assert!(still_running);
            assert_eq!(meeting_id, "new-meeting");
        }

        #[tokio::test]
        async fn dropped_start_waiter_keeps_live_initialization_and_exit_ownership() {
            let (_temporary, process) = setup();
            let lifecycle = Lifecycle::default();
            let transition = lifecycle.begin_transition().unwrap();
            let (captured, ready) = tokio::sync::oneshot::channel();
            let (register, allowed) = mpsc::channel();
            let (finished, done) = tokio::sync::oneshot::channel();
            let starting = process.clone();
            let work = tokio::task::spawn_blocking(move || {
                starting
                    .recording_start(|| {
                        captured.send(()).unwrap();
                        allowed.recv().unwrap();
                        drop(waiting_fixture(&starting, "meeting", "session"));
                        Ok(())
                    })
                    .unwrap();
                drop(transition);
                finished.send(()).unwrap();
            });
            ready.await.unwrap();
            let waiter = tokio::spawn(async move {
                work.await.unwrap();
            });
            waiter.abort();
            let _ = waiter.await;
            assert!(process.running.try_lock().is_err());
            assert!(lifecycle.begin_exit(ExitKind::Quit).is_err());
            assert!(lifecycle.begin_exit(ExitKind::Update).is_err());
            register.send(()).unwrap();
            done.await.unwrap();
            let registered = process
                .live
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .meeting_id
                .clone();
            process.cancel_live().unwrap();
            assert_eq!(registered, "meeting");
            assert!(lifecycle.begin_exit(ExitKind::Update).is_ok());
        }

        #[test]
        fn worker_failure_does_not_prevent_capture_state_changes() {
            let (_temporary, process) = setup();
            process.cleanup_failed();
            assert_eq!(process.recording_change(|| Ok(42)).unwrap(), 42);
            process.shutting_down.store(true, Ordering::Release);
            assert!(process.recording_change(|| Ok(())).is_err());
        }
    }

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

    #[test]
    fn update_fences_new_capture_and_refuses_to_cancel_running_work() {
        let (_temporary, process) = setup();
        process.quiesce_for_update().unwrap();
        assert!(process.recording_start(|| Ok(())).is_err());
        process.resume();
        process.recording_start(|| Ok(())).unwrap();
        assert!(process.quiesce_for_update().is_err());
        assert!(!process.shutting_down.load(Ordering::Acquire));
        process.recording.store(false, Ordering::Release);
        let cancelled = Arc::new(AtomicBool::new(false));
        let child_flag = cancelled.clone();
        let handle = thread::spawn(move || {
            while !child_flag.load(Ordering::Acquire) {
                thread::yield_now();
            }
        });
        *process.running.lock().unwrap() = Some(Running {
            id: "update-test".into(),
            cancel: cancelled.clone(),
            handle,
            import_existing: None,
        });
        assert!(process.quiesce_for_update().is_err());
        assert!(!cancelled.load(Ordering::Acquire));
        assert!(!process.shutting_down.load(Ordering::Acquire));
        cancelled.store(true, Ordering::Release);
        process
            .running
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .handle
            .join()
            .unwrap();
        process.quiesce_for_update().unwrap();
        process.resume();
        assert!(!process.shutting_down.load(Ordering::Acquire));
    }

    fn setup() -> (tempfile::TempDir, Arc<Processing>) {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let process = Processing::new(root.clone(), root.clone());
        (temporary, process)
    }

    mod speech_switch_tests {
        use super::*;

        fn touch(path: &Path) {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"synthetic").unwrap();
        }
        // Executables exist and the platform is supported; no model is executed.
        fn qwen_device() -> (tempfile::TempDir, Arc<Processing>) {
            let (temporary, process) = setup();
            *process.platform_supported_override.lock().unwrap() = Some(true);
            touch(&process.worker("AudioPrepare"));
            touch(&crate::qwen_asr::executable(&process.resources));
            touch(&process.resources.join("native/crispasr/runtime.json"));
            touch(&crate::native_speech::executable(&process, true));
            (temporary, process)
        }
        // A sparse file of the pinned size plus the verified marker; hashes are not read.
        fn ready(process: &Processing, engine: SpeechEngine) {
            let directory = process.model_dir_for(engine).unwrap();
            for file in Processing::model_for(engine).files {
                fs::File::create(directory.join(&file.name))
                    .unwrap()
                    .set_len(file.bytes)
                    .unwrap();
            }
            fs::write(directory.join("verified.json"), engine.manifest()).unwrap();
        }

        #[test]
        fn a_waiting_switch_keeps_the_current_engine_until_its_models_are_ready() {
            let (_temporary, process) = qwen_device();
            if !process.supported_for(SpeechEngine::QwenKo) {
                return; // Qwen runs only on Apple Silicon.
            }
            ready(&process, SpeechEngine::Nemotron);
            process
                .set_speech_selection(SpeechEngine::Nemotron, false, None)
                .unwrap();
            let pending = crate::speech_switch::Pending::new(SpeechEngine::QwenKo, false, None);
            crate::speech_switch::save_pending(&process.root, &pending).unwrap();
            *process.speech_switch.lock().unwrap() = Some(pending);
            process.reconcile_speech_switch(&None);
            assert_eq!(process.speech_engine(), SpeechEngine::Nemotron);
            let waiting = process.snapshot();
            assert!(waiting.model_ready);
            assert_eq!(
                waiting.speech_switch.as_ref().map(|view| view.target),
                Some(SpeechEngine::QwenKo)
            );
            ready(&process, SpeechEngine::QwenKo);
            process.recording.store(true, Ordering::Release);
            process.reconcile_speech_switch(&None);
            assert_eq!(process.speech_engine(), SpeechEngine::Nemotron);
            process.recording.store(false, Ordering::Release);
            process.reconcile_speech_switch(&None);
            assert_eq!(process.speech_engine(), SpeechEngine::QwenKo);
            assert_eq!(process.snapshot().speech_switch, None);
            assert_eq!(
                crate::speech_switch::load_pending(&process.root).unwrap(),
                None
            );
            assert_eq!(
                crate::speech_choice::load(&process.root).unwrap().engine,
                SpeechEngine::QwenKo
            );
        }

        #[test]
        fn a_ready_target_switches_now_and_cancelling_keeps_the_active_engine() {
            let (_temporary, process) = qwen_device();
            if !process.supported_for(SpeechEngine::QwenKo) {
                return;
            }
            ready(&process, SpeechEngine::Nemotron);
            ready(&process, SpeechEngine::QwenKo);
            process
                .set_speech_selection(SpeechEngine::Nemotron, false, None)
                .unwrap();
            let pending =
                crate::speech_switch::Pending::new(SpeechEngine::WhisperTurbo, false, None);
            crate::speech_switch::save_pending(&process.root, &pending).unwrap();
            *process.speech_switch.lock().unwrap() = Some(pending);
            process.cancel_speech_switch().unwrap();
            assert_eq!(process.speech_engine(), SpeechEngine::Nemotron);
            assert_eq!(
                crate::speech_switch::load_pending(&process.root).unwrap(),
                None
            );
            let switched = process.request_speech_engine(SpeechEngine::QwenKo).unwrap();
            assert_eq!(switched.speech_engine, Some(SpeechEngine::QwenKo));
            assert_eq!(switched.speech_switch, None);
        }

        #[test]
        fn first_setup_only_records_the_choice_and_broken_offer_files_block_nothing() {
            let (_temporary, process) = qwen_device();
            if !process.supported_for(SpeechEngine::QwenKo) {
                return;
            }
            // Nothing works yet: the choice is recorded and setup downloads it explicitly.
            let chosen = process.request_speech_engine(SpeechEngine::QwenKo).unwrap();
            assert_eq!(chosen.speech_engine, Some(SpeechEngine::QwenKo));
            assert_eq!(chosen.speech_switch, None);
            assert!(process.pending.lock().unwrap().is_empty());
            assert!(process.running.lock().unwrap().is_none());
            ready(&process, SpeechEngine::Nemotron);
            process
                .set_speech_selection(SpeechEngine::Nemotron, false, None)
                .unwrap();
            let offers = process.root.join("preferences/speech-offers.json");
            fs::write(&offers, b"not json").unwrap();
            fs::write(
                process.root.join("preferences/speech-switch.json"),
                b"not json",
            )
            .unwrap();
            let reopened = Processing::new(process.root.clone(), process.resources.clone());
            *reopened.platform_supported_override.lock().unwrap() = Some(true);
            let progress = reopened.snapshot();
            assert!(progress.supported);
            assert!(progress.model_ready);
            assert_eq!(progress.speech_offer, None);
            assert_eq!(progress.speech_switch, None);
            reopened.respond_speech_offer(false).unwrap();
            assert_eq!(fs::read(&offers).unwrap(), b"not json");
        }

        #[test]
        fn a_failed_choice_keeps_the_offer_and_a_failed_switch_waits_for_the_user() {
            let (_temporary, process) = qwen_device();
            if !process.supported_for(SpeechEngine::QwenKo) {
                return;
            }
            ready(&process, SpeechEngine::Nemotron);
            ready(&process, SpeechEngine::WhisperTurbo);
            process
                .set_speech_selection(SpeechEngine::Nemotron, false, None)
                .unwrap();
            let process = Processing::new(process.root.clone(), process.resources.clone());
            *process.platform_supported_override.lock().unwrap() = Some(true);
            process.recording.store(true, Ordering::Release);
            assert!(process
                .request_speech_engine(SpeechEngine::WhisperTurbo)
                .is_err());
            process.recording.store(false, Ordering::Release);
            assert!(process.snapshot().speech_offer.is_some());
            ready(&process, SpeechEngine::QwenKo);
            let pending = crate::speech_switch::Pending::new(SpeechEngine::QwenKo, false, None);
            crate::speech_switch::save_pending(&process.root, &pending).unwrap();
            *process.speech_switch.lock().unwrap() = Some(pending);
            let speech = process.root.join("preferences/speech.json");
            fs::remove_file(&speech).unwrap();
            fs::create_dir(&speech).unwrap();
            process.reconcile_speech_switch(&None);
            let failed = process.snapshot().speech_switch.unwrap();
            assert!(failed.error.is_some());
            let revision = process.snapshot().revision;
            process.reconcile_speech_switch(&None);
            assert_eq!(
                process.snapshot().revision,
                revision,
                "no retry on every poll"
            );
            process.cancel_speech_switch().unwrap();
            assert_eq!(process.snapshot().speech_switch, None);
            assert_eq!(process.speech_engine(), SpeechEngine::Nemotron);
        }

        #[test]
        fn existing_users_see_the_offer_once_and_a_decline_is_remembered() {
            let (_temporary, process) = qwen_device();
            if !process.supported_for(SpeechEngine::QwenKo) {
                return;
            }
            assert_eq!(process.snapshot().speech_offer, None); // nothing set up yet
            ready(&process, SpeechEngine::Nemotron);
            process
                .set_speech_selection(SpeechEngine::Nemotron, false, None)
                .unwrap();
            // A setup finished in this run is not an existing user yet.
            assert_eq!(process.snapshot().speech_offer, None);
            let process = Processing::new(process.root.clone(), process.resources.clone());
            *process.platform_supported_override.lock().unwrap() = Some(true);
            let offer = process.snapshot().speech_offer.unwrap();
            assert_eq!(offer.engine, SpeechEngine::QwenKo);
            let manifest = Processing::model_for(SpeechEngine::QwenKo);
            assert_eq!(
                offer.download_bytes,
                manifest.files.iter().map(|file| file.bytes).sum::<u64>()
            );
            // A switch that cannot start keeps the offer to accept again.
            *process.platform_supported_override.lock().unwrap() = Some(false);
            assert!(process.respond_speech_offer(true).is_err());
            *process.platform_supported_override.lock().unwrap() = Some(true);
            assert!(process.snapshot().speech_offer.is_some());
            process.respond_speech_offer(false).unwrap();
            assert_eq!(process.snapshot().speech_offer, None);
            assert_eq!(process.speech_engine(), SpeechEngine::Nemotron);
            let reopened = Processing::new(process.root.clone(), process.resources.clone());
            *reopened.platform_supported_override.lock().unwrap() = Some(true);
            assert_eq!(reopened.snapshot().speech_offer, None);
        }

        #[test]
        fn a_refused_transcription_leaves_no_job_folder() {
            let (_temporary, process) = qwen_device();
            if !process.supported_for(SpeechEngine::Nemotron) {
                return;
            }
            ready(&process, SpeechEngine::Nemotron);
            let core = CoreHandle::open(&process.root).unwrap();
            let input = process.root.join("synthetic.wav");
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut wav = hound::WavWriter::create(&input, spec).unwrap();
            for _ in 0..1600 {
                wav.write_sample(0_i16).unwrap();
            }
            wav.finalize().unwrap();
            let meeting = core.import_audio("Synthetic", &input, &input).unwrap();
            let jobs = || {
                fs::read_dir(process.root.join("processing"))
                    .map(|entries| entries.count())
                    .unwrap_or(0)
            };
            let before = jobs();
            process.shutting_down.store(true, Ordering::Release);
            assert!(process.transcribe(core.clone(), meeting.id).is_err());
            assert_eq!(jobs(), before);
        }
    }

    #[test]
    fn summary_fingerprint_tracks_roster_without_changing_speech_job_identity() {
        let (_temporary, process) = setup();
        let core = CoreHandle::open(&process.root).unwrap();
        let meeting = core
            .create_meeting("Synthetic meeting", "Original notes")
            .unwrap();
        let kinds = ["summary", "transcribe", "diarize"];
        let before =
            kinds.map(|kind| Processing::source_fingerprint(&core, &meeting.id, kind).unwrap());
        let attendee = nero_core::MeetingAttendee {
            id: uuid::Uuid::new_v4().to_string(),
            person_id: None,
            name: Some("Meeting reference".into()),
        };
        core.replace_meeting_attendees(
            &meeting.id,
            &nero_core::agent_control::meeting_edit_version(&meeting).unwrap(),
            vec![attendee],
        )
        .unwrap();
        let after =
            kinds.map(|kind| Processing::source_fingerprint(&core, &meeting.id, kind).unwrap());
        assert_ne!(before[0], after[0]);
        assert_eq!(before[1..], after[1..]);
        let catalog = core.people_catalog().unwrap();
        core.replace_people_catalog(
            &catalog.edit_version,
            vec![nero_core::Person {
                id: uuid::Uuid::new_v4().to_string(),
                name: "Catalog reference".into(),
            }],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            after[0],
            Processing::source_fingerprint(&core, &meeting.id, "summary").unwrap()
        );
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

    fn edited_baseline(process: &Processing) -> (CoreHandle, String, nero_core::Transcript) {
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
    fn delayed_old_completion_does_not_clear_a_new_capture_or_preview() {
        let (_temporary, process) = setup();
        let (core, meeting, _) = edited_baseline(&process);
        let (release, delayed) = std::sync::mpsc::channel();
        let old = completed_recording("replaced-session", &meeting);
        let background = process.clone();
        let callback = thread::spawn(move || {
            delayed.recv().unwrap();
            background.transcribe_recording(core, &old)
        });
        process.recording.store(true, Ordering::Release);
        process.update(|p| {
            p.live_transcript = Some(serde_json::json!({
                "meetingId":meeting,"sessionId":"new-session","text":"current","words":[]
            }))
        });
        release.send(()).unwrap();
        assert!(callback.join().unwrap().is_err());
        assert!(process.recording.load(Ordering::Acquire));
        assert_eq!(
            process
                .progress
                .lock()
                .unwrap()
                .live_transcript
                .as_ref()
                .unwrap()["sessionId"],
            "new-session"
        );
        assert!(process.records.lock().unwrap().is_empty());
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
            .meeting_idle("meeting-a", || -> nero_core::Result<()> {
                panic!("a related running job must hold restore ownership")
            })
            .is_err());
        assert!(!cancel.load(Ordering::Acquire));
        assert!(process.meeting_idle("meeting-b", || Ok(())).is_ok());
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
        assert!(process.meeting_idle("meeting-a", || Ok(())).is_ok());
        process.cleanup_failed();
        assert!(process
            .meeting_idle("meeting-a", || -> nero_core::Result<()> {
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
            .meeting_removal(&core, &[meeting.id], || -> nero_core::Result<()> {
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
    fn soft_trash_commits_before_join_and_blocks_late_native_result_writes() {
        let (_temp, process) = setup();
        let core = CoreHandle::open(&process.root).unwrap();
        let meeting = core.create_meeting("Meeting", "Preserved notes").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let worker_core = core.clone();
        let worker_id = meeting.id.clone();
        let (done, finished) = std::sync::mpsc::channel();
        let handle = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                thread::yield_now();
            }
            let stored = worker_core.list_meetings().unwrap();
            assert!(stored
                .iter()
                .find(|value| value.id == worker_id)
                .unwrap()
                .deleted_at
                .is_some());
            assert!(matches!(
                worker_core.commit_nemotron_transcript(&worker_id, None, b"synthetic".to_vec()),
                Err(nero_core::CoreError::MeetingNotFound)
            ));
            assert!(matches!(
                worker_core.commit_whisper_transcript(&worker_id, None, b"synthetic".to_vec()),
                Err(nero_core::CoreError::MeetingNotFound)
            ));
            // Joining must not retain the Core actor or a global library write lock.
            worker_core.create_folder("Unrelated").unwrap();
            done.send(()).unwrap();
        });
        *process.running.lock().unwrap() = Some(Running {
            id: "job".into(),
            cancel,
            handle,
            import_existing: None,
        });
        process.update(|progress| progress.meeting_id = Some(meeting.id.clone()));
        process
            .meeting_trash(&core, &[meeting.id.clone()], || {
                assert!(process.running.try_lock().is_err());
                core.trash_meetings(vec![meeting.id.clone()])
            })
            .unwrap();
        finished.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(process.running.lock().unwrap().is_none());
        assert!(!process
            .root
            .join("meetings")
            .join(&meeting.id)
            .join("transcripts")
            .exists());
        let stored = core.list_meetings().unwrap().remove(0);
        assert!(stored.notes == "Preserved notes", "notes changed");
        assert!(stored.deleted_at.is_some());
    }

    #[test]
    fn soft_trash_folder_cas_rejects_a_concurrent_external_write_without_cancelling() {
        let (_temp, process) = setup();
        let core = CoreHandle::open(&process.root).unwrap();
        let meeting = core.create_meeting("Meeting", "Preserved notes").unwrap();
        let folder = core.create_folder("Folder").unwrap();
        core.move_meetings(vec![meeting.id.clone()], Some(folder.id.clone()))
            .unwrap();
        let request = nero_core::FolderDeleteRequest {
            folder: folder.clone(),
            meetings: vec![nero_core::FolderMeetingState {
                id: meeting.id.clone(),
                deleted_at: None,
            }],
            trash_meetings: true,
        };
        core.validate_folder_delete(request.clone()).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let (release, released) = std::sync::mpsc::channel();
        let handle = thread::spawn(move || {
            let _ = released.recv_timeout(Duration::from_secs(2));
        });
        *process.running.lock().unwrap() = Some(Running {
            id: "job".into(),
            cancel: cancel.clone(),
            handle,
            import_existing: None,
        });
        process.update(|progress| progress.meeting_id = Some(meeting.id.clone()));
        let (write, writing) = std::sync::mpsc::channel();
        let (written, after_write) = std::sync::mpsc::channel();
        let external_core = core.clone();
        let folder_id = folder.id.clone();
        let writer = thread::spawn(move || {
            writing.recv_timeout(Duration::from_secs(2)).unwrap();
            external_core.rename_folder(&folder_id, "Renamed").unwrap();
            written.send(()).unwrap();
        });
        let result = process.meeting_trash(&core, &[meeting.id.clone()], || {
            write.send(()).unwrap();
            after_write.recv_timeout(Duration::from_secs(2)).unwrap();
            core.delete_folder_checked(request)
        });
        writer.join().unwrap();
        assert!(result.is_err());
        assert!(!cancel.load(Ordering::Acquire));
        assert!(process.running.lock().unwrap().is_some());
        assert!(core.list_meetings().unwrap()[0].deleted_at.is_none());
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
    }

    #[test]
    fn soft_trash_storage_errors_cancel_only_when_the_trash_state_was_committed() {
        for committed in [false, true] {
            let (_temp, process) = setup();
            let core = CoreHandle::open(&process.root).unwrap();
            let meeting = core.create_meeting("Meeting", "Preserved notes").unwrap();
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let (release, released) = std::sync::mpsc::channel();
            let handle = thread::spawn(move || {
                while !flag.load(Ordering::Acquire) {
                    if released.try_recv().is_ok() {
                        break;
                    }
                    thread::yield_now();
                }
            });
            *process.running.lock().unwrap() = Some(Running {
                id: "job".into(),
                cancel: cancel.clone(),
                handle,
                import_existing: None,
            });
            process.update(|progress| progress.meeting_id = Some(meeting.id.clone()));
            let result: nero_core::Result<()> =
                process.meeting_trash(&core, &[meeting.id.clone()], || {
                    if committed {
                        core.trash_meetings(vec![meeting.id.clone()])?;
                    }
                    Err(nero_core::CoreError::Storage(
                        "synthetic storage failure".into(),
                    ))
                });
            assert!(matches!(result, Err(nero_core::CoreError::Storage(_))));
            assert_eq!(cancel.load(Ordering::Acquire), committed);
            assert_eq!(
                core.list_meetings().unwrap()[0].deleted_at.is_some(),
                committed
            );
            if !committed {
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
            }
        }
    }

    #[test]
    fn soft_trash_reports_cleanup_failure_without_rolling_back_the_saved_trash() {
        let (_temp, process) = setup();
        let core = CoreHandle::open(&process.root).unwrap();
        let meeting = core.create_meeting("Meeting", "Preserved notes").unwrap();
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
            id: "job".into(),
            cancel,
            handle,
            import_existing: None,
        });
        process.update(|progress| progress.meeting_id = Some(meeting.id.clone()));
        assert!(process
            .meeting_trash(&core, &[meeting.id.clone()], || core
                .trash_meetings(vec![meeting.id.clone()]))
            .is_err());
        assert!(core.list_meetings().unwrap()[0].deleted_at.is_some());
        assert!(process.worker_ready().is_err());
    }

    #[test]
    fn stale_removal_after_draft_flush_keeps_running_and_queued_jobs_unchanged() {
        use nero_core::agent_control::{meeting_edit_version, Mutation};

        let (_temp, process) = setup();
        let core = CoreHandle::open(&process.root).unwrap();
        let meeting = core.create_meeting("Meeting", "Initial notes").unwrap();
        let request = Mutation::parse(
            "meetings_trash",
            serde_json::json!({"expectedVersions":{meeting.id.clone():meeting_edit_version(&meeting).unwrap()}}),
        )
        .unwrap();
        core.agent_validate(request.clone()).unwrap();
        core.update_meeting(&meeting.id, &meeting.title, "Flushed draft")
            .unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let (release, released) = std::sync::mpsc::channel();
        let handle = thread::spawn(move || {
            let _ = released.recv_timeout(Duration::from_secs(2));
        });
        *process.running.lock().unwrap() = Some(Running {
            id: "running-job".into(),
            cancel: cancel.clone(),
            handle,
            import_existing: None,
        });
        process.update(|progress| {
            progress.meeting_id = Some(meeting.id.clone());
            progress.kind = "transcribe".into();
            progress.status = "running".into();
            progress.message = "Processing".into();
        });
        let created_at = now();
        let queued = JobRecord {
            glossary_snapshot_required: false,
            speech_engine: crate::speech_choice::SpeechEngine::default(),
            speaker_enabled: true,
            apple_speech_variant: None,
            schema_version: 1,
            id: Uuid::new_v4().to_string(),
            kind: "summary".into(),
            meeting_id: Some(meeting.id.clone()),
            source_fingerprint: None,
            include_summary: false,
            summary_template_ref: None,
            acknowledged: false,
            retry_successor_id: None,
            failure_kind: None,
            status: JobStatus::Queued,
            phase: None,
            message: "Queued".into(),
            created_at,
            updated_at: created_at,
        };
        process.save_job(queued.clone()).unwrap();
        process.pending.lock().unwrap().push_back(Pending {
            record: queued.clone(),
            action: Box::new(|_, _, _| Ok(None)),
            import_existing: None,
            validate: None,
        });
        let before_progress = process.progress.lock().unwrap().clone();
        let before_records = process.journal.records().unwrap();
        let result = process.meeting_trash(&core, &[meeting.id.clone()], || {
            assert!(process.running.try_lock().is_err());
            core.agent_mutate(request)
        });
        assert!(matches!(result, Err(nero_core::CoreError::EditConflict)));
        assert!(!cancel.load(Ordering::Acquire));
        assert_eq!(
            process.running.lock().unwrap().as_ref().unwrap().id,
            "running-job"
        );
        let pending = process.pending.lock().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].record, queued);
        drop(pending);
        assert!(*process.progress.lock().unwrap() == before_progress);
        assert_eq!(process.journal.records().unwrap(), before_records);
        let current = core.list_meetings().unwrap().remove(0);
        assert_eq!(current.notes, "Flushed draft");
        assert!(current.deleted_at.is_none());

        let _ = release.send(());
        process
            .running
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .handle
            .join()
            .unwrap();
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
        let unreadable_id = Uuid::new_v4().to_string();
        let orphan = job(Some(&Uuid::new_v4().to_string()));
        let active = job(Some(&existing.id));
        let unknown = job(None);
        let invalid = job(Some("invalid"));
        let unreadable = job(Some(&unreadable_id));
        drop(core);
        let folder = process.root.join("meetings").join(&unreadable_id);
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("meeting.json"), b"{").unwrap();
        let core = CoreHandle::open(&process.root).unwrap();
        process.cleanup_removed_meeting_jobs(&core);
        assert!(!orphan.exists());
        assert!(active.exists());
        assert!(unknown.exists());
        assert!(invalid.exists());
        assert!(unreadable.exists());
    }
    #[cfg(unix)]
    #[test]
    fn startup_cleanup_failure_is_visible_and_does_not_take_the_processing_gate() {
        let (temp, process) = setup();
        let core = CoreHandle::open(&process.root).unwrap();
        std::os::unix::fs::symlink(temp.path(), process.root.join("processing")).unwrap();
        process.cleanup_removed_meeting_jobs(&core);
        let status = process.startup_issues.lock().unwrap()[0].clone();
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
        let status = process.startup_issues.lock().unwrap()[0].clone();
        assert_eq!(status.kind, "cleanup");
        assert_eq!(status.status, "failed");
        assert!(!status.message.contains(&meeting.id));
        assert!(process.root.join("meetings").join(&meeting.id).exists());
    }
    #[test]
    fn a_late_startup_issue_waits_until_the_main_window_has_asked() {
        let (_temp, process) = setup();
        process.record_transcript_upgrade(Err(nero_core::CoreError::Storage("late".into())));
        process.notify_late_startup_issues(|_| panic!("the window has not asked yet"));
        process.notify_startup_issues(|_| true);
        process.record_transcript_upgrade(Ok(nero_core::TranscriptUpgradeReport {
            upgraded: 0,
            unchanged: 0,
            unavailable: 1,
        }));
        let delivered = std::cell::Cell::new(0);
        process.notify_late_startup_issues(|issues| {
            delivered.set(issues.len());
            true
        });
        assert_eq!(delivered.get(), 1);
        process.record_transcript_upgrade(Err(nero_core::CoreError::RecordingBusy));
        assert!(process.startup_issues.lock().unwrap().is_empty());
    }
    #[test]
    fn successful_source_upgrade_does_not_add_a_startup_notice() {
        let (_temp, process) = setup();
        process.record_transcript_upgrade(Ok(nero_core::TranscriptUpgradeReport {
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
        process.record_transcript_upgrade(Ok(nero_core::TranscriptUpgradeReport {
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
        process.record_transcript_upgrade(Err(nero_core::CoreError::Storage(
            "private/source/path".into(),
        )));
        process.notify_startup_issues(|issues| {
            assert_eq!(issues[0].kind, "cleanup");
            assert_eq!(issues[1].kind, "transcript-upgrade");
            assert!(!issues[1].message.contains("private/source/path"));
            false
        });
        assert_eq!(process.startup_issues.lock().unwrap().len(), 2);
        process.notify_startup_issues(|_| true);
        assert!(process.startup_issues.lock().unwrap().is_empty());
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
    fn verifying_a_model_removes_bytes_set_aside_by_earlier_attempts() {
        let (_temporary, process) = setup();
        let manifest = serde_json::json!({"repository":"synthetic/model","revision":"r1",
            "files":[{"name":"model.bin","bytes":5,"sha256":format!("{:x}", Sha256::digest(b"model"))}]});
        let spec = ModelSpec {
            manifest: Box::leak(manifest.to_string().into_boxed_str()),
            label: "Synthetic",
            stage: ModelStage::Speech,
        };
        let directory = process.model_dir_of(spec).unwrap();
        fs::write(directory.join("model.bin"), b"model").unwrap();
        let set_aside = [
            "model.bin.invalid-1",
            "model.bin.partial-2",
            "verified.invalid-3.json",
        ];
        for name in set_aside {
            fs::write(directory.join(name), b"old").unwrap();
        }
        fs::write(directory.join("unrelated.invalid-4"), b"kept").unwrap();
        process
            .prepare_files(spec, &AtomicBool::new(false), 0, 5)
            .unwrap();
        assert!(process.model_ready(spec));
        for name in set_aside {
            assert!(!directory.join(name).exists(), "{name}");
        }
        assert!(directory.join("unrelated.invalid-4").exists());
    }

    #[test]
    fn model_specs_keep_the_stored_paths_and_progress_stages() {
        let (_temporary, process) = setup();
        let speaker = crate::diarization::SPEAKER_MODEL;
        let summary = crate::summary::SUMMARY_MODEL;
        for (spec, label) in [
            (speaker, "Nemotron3"),
            (summary, "Gemma4-E4B"),
            (ModelSpec::speech(SpeechEngine::Nemotron), "Nemotron35-ASR"),
            (ModelSpec::speech(SpeechEngine::QwenKo), "QwenKo"),
        ] {
            assert_eq!(
                process.model_dir_of(spec).unwrap(),
                process
                    .root
                    .join(format!("models/{label}-{}", spec.model().revision))
            );
        }
        for engine in SpeechEngine::ALL {
            assert_eq!(ModelSpec::speech(engine).stage, ModelStage::Speech);
        }
        for (stage, id, title) in [
            (ModelStage::Speech, "speech", "음성 인식 모델"),
            (speaker.stage, "speakers", "화자 구분 모델"),
            (summary.stage, "summary", "요약 모델"),
        ] {
            assert_eq!((stage.id(), stage.title()), (id, title));
        }
    }

    #[test]
    fn readiness_and_pre_use_verification_follow_one_rule_for_every_model() {
        use crate::failure::FailureKind;
        let (_temporary, process) = setup();
        let speaker = crate::diarization::SPEAKER_MODEL;
        let summary = crate::summary::SUMMARY_MODEL;
        // Sparse files of the pinned size with the marker: ready, without reading content.
        for spec in [speaker, summary] {
            let directory = process.model_dir_of(spec).unwrap();
            for file in spec.model().files {
                fs::File::create(directory.join(&file.name))
                    .unwrap()
                    .set_len(file.bytes)
                    .unwrap();
            }
            fs::write(directory.join("verified.json"), spec.manifest).unwrap();
            assert!(process.model_ready(spec));
        }
        assert!(crate::diarization::availability(&process).1);
        assert!(crate::summary::availability(&process).1);
        let marker = process.model_dir_of(speaker).unwrap().join("verified.json");
        let cancelled = process
            .verified_model_dir(speaker, &AtomicBool::new(true))
            .unwrap_err();
        assert_eq!(cancelled.kind, FailureKind::Cancelled);
        assert!(marker.exists());
        // The sparse file does not hash to the pin, so use withdraws readiness.
        let mismatch = process
            .verified_model_dir(speaker, &AtomicBool::new(false))
            .unwrap_err();
        assert_eq!(mismatch.kind, FailureKind::ModelNotReady);
        assert!(!marker.exists());
        assert!(!process.model_ready(speaker));
        assert!(!crate::diarization::availability(&process).1);
        let directory = process.root.join("synthetic-model");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("weights.bin"), b"pinned").unwrap();
        let pinned = Model {
            repository: "example/model".into(),
            revision: "example".into(),
            files: vec![ModelFile {
                name: "weights.bin".into(),
                bytes: 6,
                sha256: format!("{:x}", Sha256::digest(b"pinned")),
            }],
        };
        assert!(verify_model_files(&directory, &pinned, &AtomicBool::new(false)).is_ok());
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
    fn job_view_serializes_existing_creation_and_retry_lineage() {
        let view = JobView {
            id: "prior".into(),
            created_at: 1234,
            retry_successor_id: Some("successor".into()),
            ..Default::default()
        };
        let serialized = serde_json::to_value(view).unwrap();
        assert_eq!(serialized["createdAt"], 1234);
        assert_eq!(serialized["retrySuccessorId"], "successor");
        assert!(serialized.get("created_at").is_none());
    }

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

#[cfg(all(test, target_os = "macos"))]
#[path = "apple_speech_pipeline_tests.rs"]
mod apple_speech_pipeline_tests;
