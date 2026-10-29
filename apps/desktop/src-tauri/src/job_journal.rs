use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const SCHEMA_VERSION: u32 = 1;
const MAX_RECORD_BYTES: u64 = 16 * 1024;
const KINDS: &[&str] = &[
    "setup",
    "model",
    "summary-model",
    "speaker-model",
    "transcribe",
    "summary",
    "diarize",
    "import",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum JobStatus {
    Queued,
    Running,
    Completed,
    Cancelled,
    Cancelling,
    Failed,
    Interrupted,
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct JobRecord {
    pub schema_version: u32,
    pub id: String,
    pub kind: String,
    pub meeting_id: Option<String>,
    pub source_fingerprint: Option<String>,
    pub include_summary: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_template_ref: Option<crate::summary_template::TemplateRef>,
    #[serde(default)]
    pub acknowledged: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_successor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<crate::summary_failure::FailureKind>,
    pub status: JobStatus,
    pub phase: Option<String>,
    pub message: String,
    pub created_at: u64,
    pub updated_at: u64,
}

impl JobRecord {
    fn validate(&self, expected_id: &str) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION
            || self.id != expected_id
            || !valid_id(&self.id)
            || self.meeting_id.as_ref().is_some_and(|id| !valid_id(id))
            || self
                .retry_successor_id
                .as_ref()
                .is_some_and(|id| !valid_id(id) || id == &self.id)
            || (self.phase.as_deref() == Some("retryPending") && self.retry_successor_id.is_none())
            || !KINDS.contains(&self.kind.as_str())
            || self.source_fingerprint.as_ref().is_some_and(|value| {
                value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            || self
                .phase
                .as_ref()
                .is_some_and(|phase| phase.is_empty() || phase.len() > 64 || phase.contains('\0'))
            || self.summary_template_ref.as_ref().is_some_and(|value| {
                value.id.is_empty()
                    || value.id.len() > 128
                    || value.revision.len() > 128
                    || value.id.contains('\0')
                    || value.revision.contains('\0')
            })
            || self.message.len() > 4096
            || self.message.contains('\0')
            || self.updated_at < self.created_at
        {
            return Err(invalid());
        }
        Ok(())
    }
}

pub(super) struct Journal {
    root: PathBuf,
    gate: Mutex<()>,
}

#[derive(Debug)]
pub(super) struct RecoveryReport {
    pub records: Vec<JobRecord>,
    pub issues: Vec<String>,
}

impl Journal {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            gate: Mutex::new(()),
        }
    }

    pub fn write(&self, record: &JobRecord) -> Result<(), String> {
        let _guard = self.gate.lock().map_err(|_| invalid())?;
        self.write_locked(record)
    }

    fn write_locked(&self, record: &JobRecord) -> Result<(), String> {
        record.validate(&record.id)?;
        if !checked_directory(&self.root)? {
            return Err(invalid());
        }
        let processing = ensure_child(&self.root, "processing")?;
        let job = ensure_child(&processing, &record.id)?;
        let states = ensure_child(&job, "job-state")?;
        let generation = match latest_generation(&states)? {
            Some((generation, path)) => {
                // An incomplete latest generation is an uncertain write, not permission to replay.
                read_record(&path, &record.id)?;
                generation.checked_add(1).ok_or_else(invalid)?
            }
            None => 1,
        };
        let bytes = serde_json::to_vec(record).map_err(|_| invalid())?;
        if bytes.len() as u64 + 1 > MAX_RECORD_BYTES {
            return Err(invalid());
        }
        let path = states.join(format!("{generation:020}.json"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        no_follow(&mut options);
        let mut file = options.open(&path).map_err(|_| invalid())?;
        validate_file(&file)?;
        file.write_all(&bytes).map_err(|_| invalid())?;
        file.write_all(b"\n").map_err(|_| invalid())?;
        file.sync_all().map_err(|_| invalid())?;
        sync_directory(&states)
    }

    #[cfg(test)]
    pub fn records(&self) -> Result<Vec<JobRecord>, String> {
        let _guard = self.gate.lock().map_err(|_| invalid())?;
        self.records_locked()
    }

    #[cfg(test)]
    fn records_locked(&self) -> Result<Vec<JobRecord>, String> {
        let mut records = Vec::new();
        for (id, path) in self.job_directories()? {
            if let Some(record) = read_job(&path, &id)? {
                records.push(record);
            }
        }
        sort_records(&mut records);
        Ok(records)
    }

    fn job_directories(&self) -> Result<Vec<(String, PathBuf)>, String> {
        if !checked_directory(&self.root)? {
            return Ok(Vec::new());
        }
        let processing = self.root.join("processing");
        if !checked_directory(&processing)? {
            return Ok(Vec::new());
        }
        let mut jobs = Vec::new();
        for entry in fs::read_dir(&processing).map_err(|_| invalid())? {
            let entry = entry.map_err(|_| invalid())?;
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !valid_id(&id) {
                continue;
            }
            jobs.push((id, entry.path()));
        }
        Ok(jobs)
    }

    #[cfg(test)]
    pub fn recover(&self) -> Result<Vec<JobRecord>, String> {
        let _guard = self.gate.lock().map_err(|_| invalid())?;
        let mut records = self.records_locked()?;
        for record in &mut records {
            if self.recover_retry(record)? || mark_recovered(record) {
                self.write_locked(record)?;
            }
        }
        Ok(records)
    }

    pub fn retry_successor_exists(&self, record: &JobRecord) -> Result<bool, String> {
        let id = record.retry_successor_id.as_ref().ok_or_else(invalid)?;
        if !valid_id(id) || id == &record.id {
            return Err(invalid());
        }
        Ok(read_job(&self.root.join("processing").join(id), id)?.is_some())
    }

    fn recover_retry(&self, record: &mut JobRecord) -> Result<bool, String> {
        if record.phase.as_deref() != Some("retryPending") {
            return Ok(false);
        }
        match self.retry_successor_exists(record) {
            Ok(true) => {
                record.status = JobStatus::Cancelled;
                record.acknowledged = true;
                record.message = "새 작업으로 다시 시도했습니다.".into();
            }
            Ok(false) => {
                record.status = JobStatus::Cancelled;
                record.acknowledged = false;
                record.retry_successor_id = None;
                record.message =
                    "재시도를 시작하기 전에 앱이 종료되었습니다. 다시 시도할 수 있습니다.".into();
            }
            Err(_) => {
                record.status = JobStatus::NeedsReview;
                record.acknowledged = false;
                record.message =
                    "재시도 작업의 상태를 확인하지 못했습니다. 저장된 결과를 확인해 주세요.".into();
            }
        }
        record.phase = None;
        record.updated_at = now().max(record.updated_at);
        Ok(true)
    }

    pub fn recover_available(&self) -> Result<RecoveryReport, String> {
        let _guard = self.gate.lock().map_err(|_| invalid())?;
        let mut report = RecoveryReport {
            records: Vec::new(),
            issues: Vec::new(),
        };
        for (id, path) in self.job_directories()? {
            let recovered: Result<Option<JobRecord>, String> = (|| {
                let Some(mut record) = read_job(&path, &id)? else {
                    return Ok(None);
                };
                if self.recover_retry(&mut record)? || mark_recovered(&mut record) {
                    self.write_locked(&record)?;
                }
                Ok(Some(record))
            })();
            match recovered {
                Ok(Some(record)) => report.records.push(record),
                Ok(None) => (),
                Err(_) => report.issues.push(format!(
                    "작업 {id}의 상태를 복구하지 못했습니다. 다른 작업은 계속 사용할 수 있습니다."
                )),
            }
        }
        sort_records(&mut report.records);
        Ok(report)
    }
}

fn read_job(path: &Path, id: &str) -> Result<Option<JobRecord>, String> {
    if !checked_directory(path)? {
        return Ok(None);
    }
    let states = path.join("job-state");
    if !checked_directory(&states)? {
        return Ok(None);
    }
    latest_generation(&states)?
        .map(|(_, path)| read_record(&path, id))
        .transpose()
}

fn mark_recovered(record: &mut JobRecord) -> bool {
    if record.status == JobStatus::Cancelling {
        record.status = JobStatus::Cancelled;
        record.message = "취소한 작업은 다시 실행하지 않았습니다.".into();
    } else if matches!(record.status, JobStatus::Queued | JobStatus::Running) {
        record.status = if record.phase.as_deref() == Some("saving") {
            JobStatus::NeedsReview
        } else {
            JobStatus::Interrupted
        };
        record.message = if record.status == JobStatus::NeedsReview {
            "저장 중 앱이 종료되었습니다. 결과를 확인해 주세요."
        } else {
            "앱이 종료되어 작업이 중단되었습니다."
        }
        .into();
    } else {
        return false;
    }
    record.updated_at = now().max(record.updated_at);
    true
}

fn sort_records(records: &mut [JobRecord]) {
    records.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn valid_id(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}

fn invalid() -> String {
    "작업 상태를 안전하게 읽거나 저장하지 못했습니다. 기존 파일은 유지했습니다.".into()
}

fn checked_directory(path: &Path) -> Result<bool, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(invalid());
    }
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && !is_reparse_point(&metadata) =>
        {
            Ok(true)
        }
        Ok(_) => Err(invalid()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(invalid()),
    }
}

fn ensure_child(parent: &Path, name: &str) -> Result<PathBuf, String> {
    if !checked_directory(parent)? {
        return Err(invalid());
    }
    let path = parent.join(name);
    if !checked_directory(&path)? {
        match fs::create_dir(&path) {
            Ok(()) => sync_directory(parent)?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(_) => return Err(invalid()),
        }
        if !checked_directory(&path)? {
            return Err(invalid());
        }
    }
    Ok(path)
}

fn latest_generation(states: &Path) -> Result<Option<(u64, PathBuf)>, String> {
    let mut latest = None;
    for entry in fs::read_dir(states).map_err(|_| invalid())? {
        let entry = entry.map_err(|_| invalid())?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(invalid)?;
        if !name.ends_with(".json") {
            continue;
        }
        let stem = name.strip_suffix(".json").ok_or_else(invalid)?;
        let number = stem.parse::<u64>().map_err(|_| invalid())?;
        if stem.len() != 20 || number == 0 || format!("{number:020}") != stem {
            return Err(invalid());
        }
        if latest.as_ref().is_none_or(|(current, _)| number > *current) {
            latest = Some((number, entry.path()));
        }
    }
    Ok(latest)
}

fn read_record(path: &Path, id: &str) -> Result<JobRecord, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| invalid())?;
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
        return Err(invalid());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    no_follow(&mut options);
    let file = options.open(path).map_err(|_| invalid())?;
    validate_file(&file)?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(invalid());
    }
    let record: JobRecord = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    record.validate(id)?;
    Ok(record)
}

fn validate_file(file: &File) -> Result<(), String> {
    let metadata = file.metadata().map_err(|_| invalid())?;
    if !metadata.is_file() || is_reparse_point(&metadata) || metadata.len() > MAX_RECORD_BYTES {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
fn is_reparse_point(_: &fs::Metadata) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn no_follow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(0x100).mode(0o600);
}
#[cfg(any(target_os = "linux", target_os = "android"))]
fn no_follow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(0x20000).mode(0o600);
}
#[cfg(windows)]
fn no_follow(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    options.custom_flags(0x0020_0000);
}
#[cfg(not(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "android",
    windows
)))]
fn no_follow(_: &mut OpenOptions) {}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|_| invalid())
}
#[cfg(not(unix))]
fn sync_directory(_: &Path) -> Result<(), String> {
    Ok(())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(status: JobStatus) -> JobRecord {
        JobRecord {
            schema_version: 1,
            id: Uuid::new_v4().to_string(),
            kind: "transcribe".into(),
            meeting_id: Some(Uuid::new_v4().to_string()),
            source_fingerprint: Some("a".repeat(64)),
            include_summary: false,
            summary_template_ref: None,
            acknowledged: false,
            retry_successor_id: None,
            failure_kind: None,
            status,
            phase: Some("recognizing".into()),
            message: "변환 중".into(),
            created_at: 1,
            updated_at: 2,
        }
    }

    fn states(root: &Path, record: &JobRecord) -> PathBuf {
        root.join("processing").join(&record.id).join("job-state")
    }

    #[test]
    fn retry_reservation_recovery_distinguishes_missing_existing_and_corrupt_successors() {
        for state in ["missing", "queued", "completed", "corrupt"] {
            let temporary = tempfile::tempdir().unwrap();
            let journal = Journal::new(temporary.path().into());
            let mut previous = record(JobStatus::Cancelled);
            let mut successor = record(JobStatus::Queued);
            successor.meeting_id = previous.meeting_id.clone();
            previous.acknowledged = true;
            previous.phase = Some("retryPending".into());
            previous.retry_successor_id = Some(successor.id.clone());
            journal.write(&previous).unwrap();
            if state != "missing" {
                if state == "completed" {
                    successor.status = JobStatus::Completed;
                }
                journal.write(&successor).unwrap();
                if state == "corrupt" {
                    fs::write(
                        states(temporary.path(), &successor).join("00000000000000000002.json"),
                        b"invalid",
                    )
                    .unwrap();
                }
            }
            let recovered = journal.recover_available().unwrap();
            let old = recovered
                .records
                .iter()
                .find(|value| value.id == previous.id)
                .unwrap();
            assert_eq!(
                old.acknowledged,
                matches!(state, "queued" | "completed"),
                "{state}"
            );
            assert_eq!(
                old.status,
                if state == "corrupt" {
                    JobStatus::NeedsReview
                } else {
                    JobStatus::Cancelled
                },
                "{state}"
            );
            assert_eq!(old.phase, None);
            if state == "missing" {
                assert_eq!(old.retry_successor_id, None);
            }
            let again = journal.recover_available().unwrap();
            assert_eq!(
                again.records.iter().find(|value| value.id == previous.id),
                Some(old)
            );
        }
    }

    #[test]
    fn legacy_records_without_retry_link_remain_readable_and_keep_acknowledgement() {
        let original = record(JobStatus::Cancelled);
        let mut json = serde_json::to_value(&original).unwrap();
        json.as_object_mut().unwrap().remove("retrySuccessorId");
        let restored: JobRecord = serde_json::from_value(json).unwrap();
        assert_eq!(restored.retry_successor_id, None);
        assert_eq!(restored.acknowledged, original.acknowledged);
    }

    #[test]
    fn reading_a_missing_journal_does_not_initialize_directories() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("missing");
        let journal = Journal::new(root.clone());
        assert!(journal.records().unwrap().is_empty());
        assert!(journal.recover().unwrap().is_empty());
        assert!(!root.exists());
    }

    #[test]
    fn transitions_append_and_reopen_without_replacing_history() {
        let temp = tempfile::tempdir().unwrap();
        let journal = Journal::new(temp.path().into());
        let mut record = record(JobStatus::Queued);
        journal.write(&record).unwrap();
        let first = states(temp.path(), &record).join("00000000000000000001.json");
        let before = fs::read(&first).unwrap();
        record.status = JobStatus::Running;
        record.updated_at = 3;
        journal.write(&record).unwrap();
        assert_eq!(fs::read(first).unwrap(), before);
        assert_eq!(
            Journal::new(temp.path().into()).records().unwrap(),
            vec![record]
        );
    }

    #[test]
    fn truncated_latest_generation_never_replays_an_older_state() {
        let temp = tempfile::tempdir().unwrap();
        let journal = Journal::new(temp.path().into());
        let record = record(JobStatus::Queued);
        journal.write(&record).unwrap();
        let latest = states(temp.path(), &record).join("00000000000000000002.json");
        fs::write(&latest, b"{\"status\":").unwrap();
        let before = fs::read(&latest).unwrap();
        assert!(journal.records().is_err());
        assert!(journal.recover().is_err());
        assert!(journal.write(&record).is_err());
        assert_eq!(fs::read(&latest).unwrap(), before);
        assert_eq!(
            fs::read_dir(states(temp.path(), &record)).unwrap().count(),
            2
        );
    }

    #[test]
    fn recovery_records_interruption_and_keeps_cancelled_and_terminal_jobs() {
        let temp = tempfile::tempdir().unwrap();
        let journal = Journal::new(temp.path().into());
        let queued = record(JobStatus::Queued);
        let running = record(JobStatus::Running);
        let cancelled = record(JobStatus::Cancelled);
        let completed = record(JobStatus::Completed);
        let failed = record(JobStatus::Failed);
        for record in [&queued, &running, &cancelled, &completed, &failed] {
            journal.write(record).unwrap();
        }
        let recovered = journal.recover().unwrap();
        for original in [&queued, &running] {
            assert_eq!(
                recovered
                    .iter()
                    .find(|r| r.id == original.id)
                    .unwrap()
                    .status,
                JobStatus::Interrupted
            );
        }
        for original in [&cancelled, &completed, &failed] {
            assert_eq!(
                recovered.iter().find(|r| r.id == original.id).unwrap(),
                original
            );
        }
        let before = journal.records().unwrap();
        assert_eq!(journal.recover().unwrap(), before);
    }

    #[test]
    fn interrupted_saving_needs_review_instead_of_automatic_retry() {
        let temp = tempfile::tempdir().unwrap();
        let journal = Journal::new(temp.path().into());
        let mut record = record(JobStatus::Running);
        record.phase = Some("saving".into());
        journal.write(&record).unwrap();
        let recovered = journal.recover().unwrap();
        assert_eq!(recovered[0].status, JobStatus::NeedsReview);
        assert_eq!(recovered[0].source_fingerprint, record.source_fingerprint);
        assert_eq!(recovered[0].meeting_id, record.meeting_id);
    }

    #[test]
    fn recovery_changes_only_journal_files_and_ignores_legacy_jobs() {
        let temp = tempfile::tempdir().unwrap();
        let journal = Journal::new(temp.path().into());
        let record = record(JobStatus::Running);
        let meeting = temp.path().join("meetings");
        fs::create_dir(&meeting).unwrap();
        let original = meeting.join("preserved.pcm");
        fs::write(&original, [1, 2, 3, 4]).unwrap();
        let legacy = temp
            .path()
            .join("processing")
            .join(Uuid::new_v4().to_string());
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("raw.json"), b"legacy").unwrap();
        journal.write(&record).unwrap();
        journal.recover().unwrap();
        assert_eq!(fs::read(original).unwrap(), [1, 2, 3, 4]);
        assert_eq!(fs::read(legacy.join("raw.json")).unwrap(), b"legacy");
        assert!(!legacy.join("job-state").exists());
    }

    #[test]
    fn malformed_records_are_rejected_without_creating_a_job_directory() {
        let temp = tempfile::tempdir().unwrap();
        let journal = Journal::new(temp.path().into());
        let original = record(JobStatus::Queued);
        for malformed in [
            JobRecord {
                schema_version: 2,
                ..original.clone()
            },
            JobRecord {
                id: "../other".into(),
                ..original.clone()
            },
            JobRecord {
                meeting_id: Some("../other".into()),
                ..original.clone()
            },
            JobRecord {
                kind: "shell".into(),
                ..original.clone()
            },
            JobRecord {
                source_fingerprint: Some("bad".into()),
                ..original.clone()
            },
            JobRecord {
                updated_at: 0,
                ..original.clone()
            },
        ] {
            assert!(journal.write(&malformed).is_err());
        }
        assert!(!temp.path().join("processing").exists());
    }

    #[test]
    fn a_record_must_match_its_job_directory_and_have_no_unknown_fields() {
        let temp = tempfile::tempdir().unwrap();
        let journal = Journal::new(temp.path().into());
        let record = record(JobStatus::Running);
        journal.write(&record).unwrap();
        let latest = states(temp.path(), &record).join("00000000000000000002.json");
        let mut wrong = serde_json::to_value(&record).unwrap();
        wrong["id"] = serde_json::json!(Uuid::new_v4().to_string());
        fs::write(&latest, serde_json::to_vec(&wrong).unwrap()).unwrap();
        assert!(journal.records().is_err());
        wrong["id"] = serde_json::json!(record.id);
        wrong["unexpected"] = serde_json::json!(true);
        fs::write(&latest, serde_json::to_vec(&wrong).unwrap()).unwrap();
        assert!(journal.records().is_err());
    }

    #[test]
    fn available_recovery_isolates_corruption_and_preserves_cancel_intent() {
        let temp = tempfile::tempdir().unwrap();
        let journal = Journal::new(temp.path().into());
        let queued = record(JobStatus::Queued);
        let corrupt = record(JobStatus::Running);
        let mut cancelling = record(JobStatus::Cancelling);
        cancelling.phase = Some("saving".into());
        for record in [&queued, &corrupt, &cancelling] {
            journal.write(record).unwrap();
        }
        let older = states(temp.path(), &corrupt).join("00000000000000000001.json");
        let latest = states(temp.path(), &corrupt).join("00000000000000000002.json");
        fs::write(&latest, b"{\"untrusted\":\"private broken state").unwrap();
        let older_before = fs::read(&older).unwrap();
        let latest_before = fs::read(&latest).unwrap();

        let report = journal.recover_available().unwrap();
        assert_eq!(report.records.len(), 2);
        assert_eq!(report.issues.len(), 1);
        assert!(report.issues[0].contains(corrupt.id.as_str()));
        assert!(!report.issues[0].contains("private broken state"));
        assert!(!report.issues[0].contains(temp.path().to_str().unwrap()));
        assert!(!report.records.iter().any(|record| record.id == corrupt.id));
        assert_eq!(
            report
                .records
                .iter()
                .find(|record| record.id == queued.id)
                .unwrap()
                .status,
            JobStatus::Interrupted
        );
        assert_eq!(
            report
                .records
                .iter()
                .find(|record| record.id == cancelling.id)
                .unwrap()
                .status,
            JobStatus::Cancelled
        );
        assert_eq!(fs::read(&older).unwrap(), older_before);
        assert_eq!(fs::read(&latest).unwrap(), latest_before);
        assert_eq!(
            fs::read_dir(states(temp.path(), &corrupt)).unwrap().count(),
            2
        );
        assert!(journal.records().is_err());
        assert!(journal.recover().is_err());
        assert_eq!(journal.recover_available().unwrap().records, report.records);
    }

    #[test]
    fn available_recovery_isolates_an_append_failure_without_publishing_unwritten_state() {
        let temp = tempfile::tempdir().unwrap();
        let journal = Journal::new(temp.path().into());
        let queued = record(JobStatus::Queued);
        let exhausted = record(JobStatus::Running);
        journal.write(&queued).unwrap();
        journal.write(&exhausted).unwrap();
        let last = states(temp.path(), &exhausted).join(format!("{:020}.json", u64::MAX));
        let bytes = serde_json::to_vec(&exhausted).unwrap();
        fs::write(&last, &bytes).unwrap();

        let report = journal.recover_available().unwrap();
        assert_eq!(report.records.len(), 1);
        assert_eq!(report.records[0].id, queued.id);
        assert_eq!(report.records[0].status, JobStatus::Interrupted);
        assert_eq!(report.issues.len(), 1);
        assert!(report.issues[0].contains(exhausted.id.as_str()));
        assert_eq!(fs::read(last).unwrap(), bytes);
        assert_eq!(
            fs::read_dir(states(temp.path(), &exhausted))
                .unwrap()
                .count(),
            2
        );
    }

    #[test]
    fn available_recovery_keeps_an_unsafe_processing_root_as_a_global_failure() {
        let temp = tempfile::tempdir().unwrap();
        let processing = temp.path().join("processing");
        fs::write(&processing, b"preserved").unwrap();
        let journal = Journal::new(temp.path().into());
        assert!(journal.recover_available().is_err());
        assert_eq!(fs::read(processing).unwrap(), b"preserved");
    }

    #[cfg(unix)]
    #[test]
    fn available_recovery_isolates_a_linked_job_without_following_it() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let journal = Journal::new(temp.path().into());
        let valid = record(JobStatus::Queued);
        let linked = record(JobStatus::Queued);
        journal.write(&valid).unwrap();
        let linked_job = temp.path().join("processing").join(&linked.id);
        symlink(outside.path(), &linked_job).unwrap();

        let report = journal.recover_available().unwrap();
        assert_eq!(report.records.len(), 1);
        assert_eq!(report.records[0].id, valid.id);
        assert_eq!(report.records[0].status, JobStatus::Interrupted);
        assert_eq!(report.issues.len(), 1);
        assert!(report.issues[0].contains(linked.id.as_str()));
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        assert!(fs::symlink_metadata(linked_job)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn linked_generation_files_and_directories_are_rejected() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let journal = Journal::new(temp.path().into());
        let original = record(JobStatus::Queued);
        journal.write(&original).unwrap();
        let outside_file = outside.path().join("record.json");
        fs::write(&outside_file, serde_json::to_vec(&original).unwrap()).unwrap();
        symlink(
            &outside_file,
            states(temp.path(), &original).join("00000000000000000002.json"),
        )
        .unwrap();
        let before = fs::read(&outside_file).unwrap();
        assert!(journal.records().is_err());
        assert!(journal.recover().is_err());
        assert_eq!(fs::read(&outside_file).unwrap(), before);

        let linked = record(JobStatus::Queued);
        let linked_job = temp.path().join("processing").join(&linked.id);
        fs::create_dir(&linked_job).unwrap();
        symlink(outside.path(), linked_job.join("job-state")).unwrap();
        assert!(journal.write(&linked).is_err());
        assert!(!outside.path().join("00000000000000000001.json").exists());
    }
}
