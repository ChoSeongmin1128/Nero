use super::context::{Binding, Citation};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_RECORD: usize = 8 * 1024 * 1024;
fn invalid() -> String {
    "저장된 AI 대화를 확인하지 못했습니다.".into()
}
pub fn id(value: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value) {
        Ok(())
    } else {
        Err("대화 정보를 확인해 주세요.".into())
    }
}
fn plain(path: &Path, directory: bool) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink()
                || (directory && !meta.is_dir())
                || (!directory && !meta.is_file())
            {
                return Err(invalid());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err(invalid());
                }
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(invalid()),
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Message {
    #[serde(default)]
    pub request_fingerprint: Option<String>,
    #[serde(default)]
    pub sequence: u64,
    pub id: String,
    pub request_id: String,
    pub role: String,
    pub text: String,
    pub status: String,
    #[serde(default)]
    pub citations: Vec<Citation>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub binding: Option<Binding>,
    #[serde(default)]
    pub saved_summary_id: Option<String>,
    #[serde(default)]
    pub note_saved: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Conversation {
    pub format_version: u32,
    pub id: Option<String>,
    pub meeting_id: String,
    pub provider: Option<String>,
    pub model_id: Option<String>,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub active_request_id: Option<String>,
}
impl Conversation {
    pub fn empty(meeting_id: &str) -> Self {
        Self {
            format_version: 1,
            id: None,
            meeting_id: meeting_id.into(),
            provider: None,
            model_id: None,
            messages: vec![],
            active_request_id: None,
        }
    }
}
pub struct Store {
    root: PathBuf,
    recovered: Mutex<HashMap<String, Conversation>>,
}
impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            recovered: Mutex::new(HashMap::new()),
        }
    }
    fn directory(&self, meeting_id: &str) -> Result<Option<PathBuf>, String> {
        id(meeting_id)?;
        if !plain(&self.root, true)? {
            return Err(invalid());
        }
        let meetings = self.root.join("meetings");
        if !plain(&meetings, true)? {
            return Ok(None);
        }
        let directory = meetings.join(meeting_id);
        if !plain(&directory, true)? {
            return Ok(None);
        }
        Ok(Some(directory))
    }
    pub fn read(&self, meeting_id: &str) -> Result<Conversation, String> {
        let Some(directory) = self.directory(meeting_id)? else {
            self.recovered.lock().unwrap().remove(meeting_id);
            return Ok(Conversation::empty(meeting_id));
        };
        if !plain(&directory.join("meeting.json"), false)? {
            self.recovered.lock().unwrap().remove(meeting_id);
            return Ok(Conversation::empty(meeting_id));
        }
        if let Some(value) = self.recovered.lock().unwrap().get(meeting_id) {
            return Ok(value.clone());
        }
        let path = directory.join("ai-conversation.json");
        if !plain(&path, false)? {
            return Ok(Conversation::empty(meeting_id));
        }
        let mut raw = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| invalid())?
            .take(MAX_RECORD as u64 + 1)
            .read_to_end(&mut raw)
            .map_err(|_| invalid())?;
        if raw.len() > MAX_RECORD {
            return Err(invalid());
        }
        let value: Conversation = serde_json::from_slice(&raw).map_err(|_| invalid())?;
        if value.format_version != 1
            || value.meeting_id != meeting_id
            || value.id.as_deref().is_none_or(|value| id(value).is_err())
            || value.messages.len() > 400
            || value.messages.iter().any(|message| {
                id(&message.id).is_err()
                    || id(&message.request_id).is_err()
                    || !matches!(message.role.as_str(), "user" | "assistant")
                    || !matches!(
                        message.status.as_str(),
                        "completed" | "streaming" | "failed" | "cancelled" | "interrupted"
                    )
            })
        {
            return Err(invalid());
        }
        Ok(value)
    }
    pub fn write(&self, value: &Conversation) -> Result<(), String> {
        id(&value.meeting_id)?;
        id(value.id.as_deref().ok_or_else(invalid)?)?;
        let raw = serde_json::to_vec(value).map_err(|_| invalid())?;
        if raw.len() > MAX_RECORD || value.messages.len() > 400 {
            return Err(
                "이 회의의 대화 저장 한도에 도달했습니다. 기존 답변은 계속 확인할 수 있습니다."
                    .into(),
            );
        }
        let directory = self.directory(&value.meeting_id)?.ok_or_else(invalid)?;
        if !plain(&directory.join("meeting.json"), false)? {
            return Err(invalid());
        }
        let path = directory.join("ai-conversation.json");
        plain(&path, false)?;
        let mut file = tempfile::NamedTempFile::new_in(&directory).map_err(|_| invalid())?;
        file.write_all(&raw)
            .and_then(|_| file.as_file().sync_all())
            .map_err(|_| invalid())?;
        if self.directory(&value.meeting_id)?.as_ref() != Some(&directory)
            || !plain(&directory.join("meeting.json"), false)?
        {
            return Err(invalid());
        }
        plain(&path, false)?;
        file.persist(&path).map_err(|_| invalid())?;
        #[cfg(unix)]
        std::fs::File::open(&directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| invalid())?;
        self.recovered.lock().unwrap().remove(&value.meeting_id);
        Ok(())
    }
    pub fn write_receipt(&self, value: &Conversation) {
        self.recovered
            .lock()
            .unwrap()
            .insert(value.meeting_id.clone(), value.clone());
        let _ = self.write(value);
    }
    pub fn purge(&self, meeting_id: &str) -> Result<(), String> {
        id(meeting_id)?;
        if let Some(directory) = self.directory(meeting_id)? {
            let path = directory.join("ai-conversation.json");
            if plain(&path, false)? {
                std::fs::remove_file(&path).map_err(|_| invalid())?;
            }
            #[cfg(unix)]
            std::fs::File::open(&directory)
                .and_then(|file| file.sync_all())
                .map_err(|_| invalid())?;
        }
        self.recovered.lock().unwrap().remove(meeting_id);
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn meeting(root: &Path, id: &str) -> PathBuf {
        let directory = root.join("meetings").join(id);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("meeting.json"), b"{}").unwrap();
        directory
    }
    fn conversation(id: &str) -> Conversation {
        let mut value = Conversation::empty(id);
        value.id = Some(uuid::Uuid::new_v4().to_string());
        value
    }
    #[test]
    fn reads_and_writes_never_create_meeting_directories() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().into());
        let id = uuid::Uuid::new_v4().to_string();
        assert!(store.read(&id).unwrap().id.is_none());
        assert!(store.write(&conversation(&id)).is_err());
        assert!(!root.path().join("meetings").exists());
        let directory = meeting(root.path(), &id);
        let value = conversation(&id);
        store.write(&value).unwrap();
        std::fs::rename(&directory, root.path().join("deleted-meeting")).unwrap();
        assert!(store.write(&value).is_err());
        store.write_receipt(&value);
        assert!(store.read(&id).unwrap().id.is_none());
        assert!(!directory.exists());
        store.purge(&id).unwrap();
    }
    #[test]
    fn failed_overwrite_preserves_existing_conversation() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().into());
        let id = uuid::Uuid::new_v4().to_string();
        let directory = meeting(root.path(), &id);
        let mut value = conversation(&id);
        store.write(&value).unwrap();
        let path = directory.join("ai-conversation.json");
        let before = std::fs::read(&path).unwrap();
        value.model_id = Some("x".repeat(MAX_RECORD));
        assert!(store.write(&value).is_err());
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
    #[cfg(unix)]
    #[test]
    fn linked_meeting_directories_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        std::fs::create_dir(root.path().join("meetings")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("meetings").join(&id)).unwrap();
        let store = Store::new(root.path().into());
        assert!(store.read(&id).is_err());
        assert!(store.write(&conversation(&id)).is_err());
        assert!(store.purge(&id).is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }
    #[test]
    fn confirmed_receipt_is_retained_until_a_later_successful_flush() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().into());
        let id = uuid::Uuid::new_v4().to_string();
        let directory = meeting(root.path(), &id);
        let mut value = conversation(&id);
        store.write(&value).unwrap();
        let path = directory.join("ai-conversation.json");
        std::fs::rename(&path, directory.join("preserved.json")).unwrap();
        std::fs::create_dir(&path).unwrap();
        value.model_id = Some("confirmed receipt".into());
        store.write_receipt(&value);
        assert_eq!(store.read(&id).unwrap().model_id, value.model_id);
        std::fs::rename(&path, directory.join("blocked-directory")).unwrap();
        std::fs::rename(directory.join("preserved.json"), &path).unwrap();
        store.write(&store.read(&id).unwrap()).unwrap();
        assert_eq!(
            Store::new(root.path().into()).read(&id).unwrap().model_id,
            value.model_id
        );
    }
    #[test]
    fn purge_is_scoped_and_idempotent_after_core_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().into());
        let first = uuid::Uuid::new_v4().to_string();
        let second = uuid::Uuid::new_v4().to_string();
        for id in [&first, &second] {
            meeting(root.path(), id);
            store.write(&conversation(id)).unwrap();
        }
        store.purge(&first).unwrap();
        store.purge(&first).unwrap();
        assert!(store.read(&first).unwrap().id.is_none());
        assert!(store.read(&second).unwrap().id.is_some());
        std::fs::rename(
            root.path().join("meetings").join(&first),
            root.path().join("removed"),
        )
        .unwrap();
        store.purge(&first).unwrap();
        assert!(!root.path().join("meetings").join(&first).exists());
    }
}
