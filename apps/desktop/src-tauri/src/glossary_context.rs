use crate::{domain_glossary, speech_choice::SpeechEngine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_SNAPSHOT_BYTES: u64 = 256 * 1024;
const MAX_PROMPT_BYTES: usize = 224;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Context {
    schema_version: u32,
    meeting_id: String,
    snapshot: domain_glossary::Snapshot,
    snapshot_sha256: String,
}
fn invalid() -> String {
    "고정된 회의 용어집을 확인하지 못했습니다.".into()
}
impl Context {
    fn new(meeting_id: &str, snapshot: domain_glossary::Snapshot) -> Result<Self, String> {
        Ok(Self {
            schema_version: 1,
            meeting_id: meeting_id.into(),
            snapshot_sha256: snapshot.sha256()?,
            snapshot,
        })
    }
    fn validate(&self, meeting_id: &str) -> Result<(), String> {
        if self.schema_version != 1
            || self.meeting_id != meeting_id
            || self.snapshot.sha256()? != self.snapshot_sha256
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn prompt(&self) -> String {
        let mut result = String::new();
        for entry in &self.snapshot.entries {
            let separator = if result.is_empty() { "" } else { ", " };
            if result.len() + separator.len() + entry.term.len() > MAX_PROMPT_BYTES {
                break;
            }
            result.push_str(separator);
            result.push_str(&entry.term);
        }
        result
    }
    pub fn evidence(&self, engine: SpeechEngine) -> Value {
        let prompt = self.prompt();
        let supported = matches!(
            engine,
            SpeechEngine::WhisperTurbo | SpeechEngine::WhisperLargeV3
        );
        json!({"revision":self.snapshot.revision,"sha256":self.snapshot_sha256,"promptStatus":if !supported {"unsupported"} else if prompt.is_empty() {"empty"} else {"used"},"promptBytes":if supported {prompt.len()} else {0}})
    }
}
fn session_path(root: &Path, session: &str, meeting_id: &str) -> Result<PathBuf, String> {
    if !uuid::Uuid::parse_str(session).is_ok_and(|id| id.to_string() == session) {
        return Err(invalid());
    }
    if !uuid::Uuid::parse_str(meeting_id).is_ok_and(|id| id.to_string() == meeting_id) {
        return Err(invalid());
    }
    let path = root
        .join("preferences/capture-glossary")
        .join(meeting_id)
        .join(format!("{session}.json"));
    crate::path_safety::safe_ancestors(&path).map_err(|_| invalid())?;
    Ok(path)
}
fn read(path: &Path, meeting_id: &str) -> Result<Option<Context>, String> {
    crate::path_safety::safe_ancestors(path).map_err(|_| invalid())?;
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(invalid()),
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_SNAPSHOT_BYTES {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    crate::processing::plain_file(path)?
        .take(MAX_SNAPSHOT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() as u64 > MAX_SNAPSHOT_BYTES {
        return Err(invalid());
    }
    let context: Context = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    context.validate(meeting_id)?;
    Ok(Some(context))
}
fn write_once(path: &Path, context: &Context) -> Result<(), String> {
    let parent = path.parent().ok_or_else(invalid)?;
    fs::create_dir_all(parent).map_err(|_| invalid())?;
    crate::path_safety::safe_ancestors(path).map_err(|_| invalid())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|_| invalid())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| invalid())?;
    }
    serde_json::to_writer(&mut file, context).map_err(|_| invalid())?;
    file.flush()
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| invalid())?;
    file.persist_noclobber(path).map_err(|_| invalid())?;
    Ok(())
}
pub fn delete_meeting(root: &Path, meeting_id: &str) -> Result<(), String> {
    let probe = session_path(root, "00000000-0000-4000-8000-000000000000", meeting_id)?;
    let dir = probe.parent().ok_or_else(invalid)?;
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(invalid()),
    };
    for entry in entries {
        let entry = entry.map_err(|_| invalid())?;
        let name = entry.file_name().into_string().map_err(|_| invalid())?;
        let Some(id) = name.strip_suffix(".json") else {
            return Err(invalid());
        };
        if !uuid::Uuid::parse_str(id).is_ok_and(|parsed| parsed.to_string() == id) {
            return Err(invalid());
        }
        let path = session_path(root, id, meeting_id)?;
        read(&path, meeting_id)?.ok_or_else(invalid)?;
        fs::remove_file(path).map_err(|_| invalid())?;
    }
    fs::remove_dir(dir).map_err(|_| invalid())
}
pub fn load_session(
    root: &Path,
    session: &str,
    meeting_id: &str,
) -> Result<Option<Context>, String> {
    read(&session_path(root, session, meeting_id)?, meeting_id)
}
pub fn freeze_session(root: &Path, session: &str, meeting_id: &str) -> Result<Context, String> {
    let path = session_path(root, session, meeting_id)?;
    if let Some(existing) = read(&path, meeting_id)? {
        return Ok(existing);
    }
    let context = Context::new(
        meeting_id,
        domain_glossary::load_snapshot(root, meeting_id)?,
    )?;
    if write_once(&path, &context).is_err() {
        return read(&path, meeting_id)?.ok_or_else(invalid);
    }
    Ok(context)
}
pub fn freeze_job(
    root: &Path,
    job_path: &Path,
    meeting_id: &str,
    inherited: Option<&Context>,
) -> Result<Context, String> {
    let path = job_path.join("glossary-snapshot.json");
    if let Some(existing) = read(&path, meeting_id)? {
        return Ok(existing);
    }
    let context = match inherited {
        Some(value) => {
            value.validate(meeting_id)?;
            value.clone()
        }
        None => Context::new(
            meeting_id,
            domain_glossary::load_snapshot(root, meeting_id)?,
        )?,
    };
    if write_once(&path, &context).is_err() {
        return read(&path, meeting_id)?.ok_or_else(invalid);
    }
    Ok(context)
}
pub fn retry_context(path: &Path, meeting_id: &str, required: bool) -> Result<Context, String> {
    crate::path_safety::safe_ancestors(path).map_err(|_| invalid())?;
    let metadata = fs::symlink_metadata(path).map_err(|_| invalid())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid());
    }
    match load_job(path, meeting_id)? {
        Some(context) => Ok(context),
        None if !required => Context::new(meeting_id, domain_glossary::Snapshot::default()),
        None => Err(invalid()),
    }
}
pub fn capture_context(
    root: &Path,
    session: &str,
    meeting_id: &str,
    choice: Option<&crate::speech_choice::SessionChoice>,
) -> Result<Context, String> {
    match choice {
        Some(choice) if choice.glossary_snapshot_required => {
            load_session(root, session, meeting_id)?.ok_or_else(invalid)
        }
        Some(_) => Context::new(meeting_id, domain_glossary::Snapshot::default()),
        None => {
            if load_session(root, session, meeting_id)?.is_some() {
                return Err(invalid());
            }
            Context::new(meeting_id, domain_glossary::Snapshot::default())
        }
    }
}
pub fn load_job(path: &Path, meeting_id: &str) -> Result<Option<Context>, String> {
    read(&path.join("glossary-snapshot.json"), meeting_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frozen_session_and_job_survive_later_changes_and_reject_other_meetings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let meeting = "a0000000-0000-4000-8000-000000000001";
        let session = "a0000000-0000-4000-8000-000000000002";
        let session_context = freeze_session(&root, session, meeting).unwrap();
        let job = root.join("job");
        let frozen = freeze_job(&root, &job, meeting, Some(&session_context)).unwrap();
        assert_eq!(load_job(&job, meeting).unwrap(), Some(frozen.clone()));
        assert_eq!(
            freeze_session(&root, session, meeting).unwrap(),
            session_context
        );
        assert!(load_job(&job, "a0000000-0000-4000-8000-000000000003").is_err());
        assert_eq!(
            frozen.evidence(SpeechEngine::Nemotron)["promptStatus"],
            "unsupported"
        );
        assert_eq!(
            frozen.evidence(SpeechEngine::WhisperTurbo)["promptStatus"],
            "empty"
        );
    }
    #[test]
    fn legacy_missing_retry_is_empty_but_corrupt_snapshot_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let meeting = "a0000000-0000-4000-8000-000000000001";
        let root = dir.path().canonicalize().unwrap();
        let old_job = root.join("old");
        assert!(retry_context(&old_job, meeting, false).is_err());
        fs::create_dir(&old_job).unwrap();
        assert!(retry_context(&old_job, meeting, false)
            .unwrap()
            .prompt()
            .is_empty());
        assert!(retry_context(&old_job, meeting, true).is_err());
        let context = Context::new(meeting, domain_glossary::Snapshot::default()).unwrap();
        let mut altered = serde_json::to_value(&context).unwrap();
        altered["snapshotSha256"] = "invalid".into();
        fs::write(
            old_job.join("glossary-snapshot.json"),
            serde_json::to_vec(&altered).unwrap(),
        )
        .unwrap();
        assert!(retry_context(&old_job, meeting, false).is_err());
    }
    #[test]
    fn capture_marker_distinguishes_legacy_and_missing_new_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let meeting = "a0000000-0000-4000-8000-000000000001";
        let session = "a0000000-0000-4000-8000-000000000002";
        let legacy: crate::speech_choice::SessionChoice = serde_json::from_value(
            json!({"schemaVersion":1,"engine":"nemotron","speakerEnabled":true}),
        )
        .unwrap();
        let required: crate::speech_choice::SessionChoice = serde_json::from_value(json!({"schemaVersion":1,"engine":"nemotron","speakerEnabled":true,"glossarySnapshotRequired":true})).unwrap();
        assert!(capture_context(&root, session, meeting, Some(&legacy))
            .unwrap()
            .prompt()
            .is_empty());
        assert!(capture_context(&root, session, meeting, Some(&required)).is_err());
        let frozen = freeze_session(&root, session, meeting).unwrap();
        assert_eq!(
            capture_context(&root, session, meeting, Some(&required)).unwrap(),
            frozen
        );
        assert!(capture_context(&root, session, meeting, None).is_err());
        assert!(capture_context(&root, session, meeting, Some(&legacy))
            .unwrap()
            .prompt()
            .is_empty());
    }
    #[test]
    fn prompt_contains_only_whole_approved_terms_and_is_bounded() {
        let snapshot = domain_glossary::Snapshot {
            revision: 2,
            entries: vec![
                domain_glossary::Entry {
                    term: "가".repeat(70),
                    source_id: "source".into(),
                },
                domain_glossary::Entry {
                    term: "나".repeat(10),
                    source_id: "source".into(),
                },
            ],
        };
        let context = Context::new("meeting", snapshot).unwrap();
        assert_eq!(context.prompt(), "가".repeat(70));
        assert!(context.prompt().len() <= MAX_PROMPT_BYTES);
        assert_eq!(
            context.evidence(SpeechEngine::WhisperTurbo)["promptStatus"],
            "used"
        );
    }
}
