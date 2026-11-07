use super::context::{Attachment, Binding, Citation, SaveTarget};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

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
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    if !plain(path, false)? {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| invalid())?
        .take(MAX_RECORD as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() > MAX_RECORD {
        return Err(invalid());
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let parent = path.parent().ok_or_else(invalid)?;
    if !plain(parent, true)? {
        return Err(invalid());
    }
    plain(path, false)?;
    let bytes = serde_json::to_vec(value).map_err(|_| invalid())?;
    if bytes.len() > MAX_RECORD {
        return Err("대화 저장 한도에 도달했습니다. 새 대화에서 이어가 주세요.".into());
    }
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|_| invalid())?;
    file.write_all(&bytes)
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| invalid())?;
    plain(path, false)?;
    file.persist(path).map_err(|_| invalid())?;
    #[cfg(unix)]
    std::fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|_| invalid())?;
    Ok(())
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Message {
    // The message this one follows. Editing a question adds a sibling under the same
    // parent; `ROOT` marks the first turn. Older files had a single line of messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<super::context::TemplateSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_target_id: Option<String>,
    #[serde(default)]
    pub binding: Option<Binding>,
    #[serde(default)]
    pub saved_summary_id: Option<String>,
    #[serde(default)]
    pub note_saved: bool,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub save_targets: Vec<SaveTarget>,
    #[serde(default)]
    pub summary_receipts: BTreeMap<String, String>,
    #[serde(default)]
    pub note_receipts: BTreeSet<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Conversation {
    pub format_version: u32,
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub meeting_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    pub provider: Option<String>,
    pub model_id: Option<String>,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub active_request_id: Option<String>,
    // The question version shown at each fork, keyed by the shared parent.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub versions: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<u64>,
}
pub const ROOT: &str = "root";
impl Conversation {
    pub fn empty(conversation_id: &str) -> Self {
        Self {
            format_version: 2,
            id: Some(conversation_id.into()),
            meeting_id: String::new(),
            title: "새 대화".into(),
            created_at: now(),
            updated_at: now(),
            provider: None,
            model_id: None,
            messages: vec![],
            active_request_id: None,
            versions: BTreeMap::new(),
            deleted_at: None,
        }
    }
    pub fn normalize_parents(&mut self) {
        for index in 0..self.messages.len() {
            if self.messages[index].parent_id.is_none() {
                let parent = if index == 0 {
                    ROOT.into()
                } else {
                    self.messages[index - 1].id.clone()
                };
                self.messages[index].parent_id = Some(parent);
            }
        }
    }
    fn parent_key(&self, index: usize) -> &str {
        match &self.messages[index].parent_id {
            Some(parent) => parent,
            None if index == 0 => ROOT,
            None => &self.messages[index - 1].id,
        }
    }
    // The shown line of messages: from the root, the selected child at each fork, or the
    // newest child when nothing was selected.
    pub fn path(&self) -> Vec<usize> {
        let mut path = Vec::new();
        let mut key = ROOT.to_string();
        loop {
            let children = (0..self.messages.len())
                .filter(|&index| self.parent_key(index) == key)
                .collect::<Vec<_>>();
            let Some(&newest) = children.last() else {
                break;
            };
            let chosen = self
                .versions
                .get(&key)
                .and_then(|id| {
                    children
                        .iter()
                        .copied()
                        .find(|&index| &self.messages[index].id == id)
                })
                .unwrap_or(newest);
            path.push(chosen);
            key = self.messages[chosen].id.clone();
        }
        path
    }
    pub fn restore_sources(&mut self) {
        let request_sources = self
            .messages
            .iter()
            .filter(|message| message.role == "assistant")
            .filter_map(|message| {
                message.binding.as_ref().map(|binding| {
                    (
                        message.request_id.clone(),
                        (binding.attachments(), binding.save_targets()),
                    )
                })
            })
            .collect::<HashMap<_, _>>();
        for message in &mut self.messages {
            if let Some(binding) = &message.binding {
                message.attachments = binding.attachments();
                message.save_targets = binding.save_targets();
            } else if message.role == "user" {
                if let Some((attachments, targets)) = request_sources.get(&message.request_id) {
                    message.attachments = attachments.clone();
                    message.save_targets = targets.clone();
                }
            }
        }
    }
    fn validate(&self, id_value: &str) -> Result<(), String> {
        let positions = self
            .messages
            .iter()
            .enumerate()
            .map(|(index, message)| (message.id.as_str(), index))
            .collect::<HashMap<_, _>>();
        let parent_known = |index: usize| {
            let parent = self.parent_key(index);
            parent == ROOT
                || positions
                    .get(parent)
                    .is_some_and(|&position| position < index)
        };
        let version_known = |key: &String, child: &String| {
            (key == ROOT || positions.contains_key(key.as_str()))
                && positions.get(child.as_str()).is_some_and(|&index| {
                    self.messages[index].role == "user" && self.parent_key(index) == key
                })
        };
        if positions.len() != self.messages.len()
            || !(0..self.messages.len()).all(parent_known)
            || !self
                .versions
                .iter()
                .all(|(key, child)| version_known(key, child))
        {
            return Err(invalid());
        }
        if self.format_version != 2
            || self.id.as_deref() != Some(id_value)
            || self.messages.len() > 400
            || self.messages.iter().any(|m| {
                id(&m.id).is_err()
                    || id(&m.request_id).is_err()
                    || !matches!(m.role.as_str(), "user" | "assistant")
                    || !matches!(
                        m.status.as_str(),
                        "completed" | "streaming" | "failed" | "cancelled" | "interrupted"
                    )
            })
        {
            return Err(invalid());
        }
        Ok(())
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub provider: Option<String>,
    pub model_id: Option<String>,
    pub message_count: usize,
    pub active_request_id: Option<String>,
    pub meeting_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<u64>,
}
// A conversation file's size and modification time, to reuse its list row.
type FileStamp = (u64, std::time::SystemTime);
pub struct Store {
    root: PathBuf,
    recovered: Mutex<HashMap<String, Conversation>>,
    legacy_imported: AtomicBool,
    // List rows of unchanged files. The list is read after every answer; a file can
    // be megabytes, so only new or changed conversations are parsed again.
    rows: Mutex<HashMap<String, (FileStamp, ConversationSummary)>>,
}
impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            recovered: Mutex::new(HashMap::new()),
            legacy_imported: AtomicBool::new(false),
            rows: Mutex::new(HashMap::new()),
        }
    }
    fn directory(&self, create: bool) -> Result<Option<PathBuf>, String> {
        if !plain(&self.root, true)? {
            return Err(invalid());
        }
        let directory = self.root.join("ai-conversations");
        if !plain(&directory, true)? {
            if !create {
                return Ok(None);
            }
            std::fs::create_dir(&directory).map_err(|_| invalid())?;
            if !plain(&directory, true)? {
                return Err(invalid());
            }
        }
        Ok(Some(directory))
    }
    pub fn read(&self, conversation_id: &str) -> Result<Conversation, String> {
        id(conversation_id)?;
        let directory = self.directory(false)?.ok_or("대화를 찾을 수 없습니다.")?;
        if let Some(value) = self.recovered.lock().unwrap().get(conversation_id) {
            return Ok(value.clone());
        }
        let mut value: Conversation =
            read_json(&directory.join(format!("{conversation_id}.json")))?;
        value.validate(conversation_id)?;
        value.normalize_parents();
        Ok(value)
    }
    pub fn list(&self, trash: bool) -> Result<Vec<ConversationSummary>, String> {
        let Some(directory) = self.directory(false)? else {
            return Ok(vec![]);
        };
        let mut result = Vec::new();
        for entry in std::fs::read_dir(directory).map_err(|_| invalid())? {
            let path = entry.map_err(|_| invalid())?.path();
            let Some(value) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().and_then(|s| s.to_str()) != Some("json") || id(value).is_err() {
                continue;
            }
            let row = self.row(value, &path)?;
            if row.deleted_at.is_some() == trash {
                result.push(row);
            }
        }
        result.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(result)
    }
    fn row(&self, value: &str, path: &Path) -> Result<ConversationSummary, String> {
        let recovered = self.recovered.lock().unwrap().contains_key(value);
        let stamp = std::fs::symlink_metadata(path)
            .ok()
            .filter(|meta| meta.is_file() && !recovered)
            .and_then(|meta| Some((meta.len(), meta.modified().ok()?)));
        if let Some((cached, row)) = self.rows.lock().unwrap().get(value) {
            if Some(*cached) == stamp {
                return Ok(row.clone());
            }
        }
        let conversation = self.read(value)?;
        let shown = conversation.path();
        let meeting_ids = shown
            .iter()
            .map(|&index| &conversation.messages[index])
            .filter(|message| message.role == "user")
            .flat_map(|message| &message.attachments)
            .filter(|attachment| attachment.kind == "meeting")
            .filter_map(|attachment| attachment.meeting_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let row = ConversationSummary {
            id: value.into(),
            title: conversation.title,
            created_at: conversation.created_at,
            updated_at: conversation.updated_at,
            provider: conversation.provider,
            model_id: conversation.model_id,
            message_count: shown.len(),
            active_request_id: conversation.active_request_id,
            meeting_ids,
            deleted_at: conversation.deleted_at,
        };
        if let Some(stamp) = stamp {
            self.rows
                .lock()
                .unwrap()
                .insert(value.into(), (stamp, row.clone()));
        }
        Ok(row)
    }
    pub fn create(&self) -> Result<Conversation, String> {
        let value = Conversation::empty(&uuid::Uuid::new_v4().to_string());
        self.directory(true)?;
        self.write_new(&value)?;
        Ok(value)
    }
    pub fn write(&self, value: &Conversation) -> Result<(), String> {
        let conversation_id = value.id.as_deref().ok_or_else(invalid)?;
        id(conversation_id)?;
        let directory = self.directory(false)?.ok_or_else(invalid)?;
        if !plain(&directory.join(format!("{conversation_id}.json")), false)? {
            return Err("대화를 찾을 수 없습니다.".into());
        }
        self.write_new(value)
    }
    fn write_new(&self, value: &Conversation) -> Result<(), String> {
        let conversation_id = value.id.as_deref().ok_or_else(invalid)?;
        id(conversation_id)?;
        value.validate(conversation_id)?;
        let directory = self.directory(false)?.ok_or_else(invalid)?;
        self.rows.lock().unwrap().remove(conversation_id);
        atomic_json(&directory.join(format!("{conversation_id}.json")), value)?;
        self.recovered.lock().unwrap().remove(conversation_id);
        Ok(())
    }
    pub fn write_receipt(&self, value: &Conversation) {
        if let Some(id) = &value.id {
            self.recovered
                .lock()
                .unwrap()
                .insert(id.clone(), value.clone());
            let _ = self.write(value);
        }
    }
    pub fn trash(&self, conversation_id: &str) -> Result<(), String> {
        let mut value = self.read(conversation_id)?;
        if value.deleted_at.is_none() {
            value.deleted_at = Some(now());
            self.write(&value)?;
        }
        Ok(())
    }
    pub fn restore(&self, conversation_id: &str) -> Result<(), String> {
        let mut value = self.read(conversation_id)?;
        if value.deleted_at.take().is_some() {
            self.write(&value)?;
        }
        Ok(())
    }
    // Only conversations already in the trash are removed for good.
    pub fn purge(&self, conversation_id: &str) -> Result<(), String> {
        if self.read(conversation_id)?.deleted_at.is_none() {
            return Err("휴지통에 있는 대화만 완전히 삭제할 수 있습니다.".into());
        }
        self.remove(conversation_id)
    }
    pub fn remove(&self, conversation_id: &str) -> Result<(), String> {
        id(conversation_id)?;
        let directory = self.directory(false)?.ok_or_else(invalid)?;
        let path = directory.join(format!("{conversation_id}.json"));
        self.rows.lock().unwrap().remove(conversation_id);
        if plain(&path, false)? {
            std::fs::remove_file(&path).map_err(|_| invalid())?;
        }
        self.recovered.lock().unwrap().remove(conversation_id);
        Ok(())
    }
    // Only the native writer's explicit global-chat entry point invokes this import.
    pub fn import_legacy(&self) -> Result<(), String> {
        if self.legacy_imported.load(Ordering::Acquire) {
            return Ok(());
        }
        let Some(directory) = self.directory(true)? else {
            return Err(invalid());
        };
        let marker = directory.join("legacy-imports.json");
        let mut imported: BTreeSet<String> = if plain(&marker, false)? {
            read_json(&marker)?
        } else {
            BTreeSet::new()
        };
        let meetings = self.root.join("meetings");
        if !plain(&meetings, true)? {
            self.legacy_imported.store(true, Ordering::Release);
            return Ok(());
        }
        for entry in std::fs::read_dir(&meetings).map_err(|_| invalid())? {
            let path = entry.map_err(|_| invalid())?.path();
            let Some(meeting_id) = path.file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            if id(meeting_id).is_err() {
                continue;
            }
            if !plain(&path, true)? {
                continue;
            }
            let legacy = path.join("ai-conversation.json");
            if !plain(&legacy, false)? {
                continue;
            }
            let mut value: Conversation = read_json(&legacy)?;
            let conversation_id = value.id.clone().ok_or_else(invalid)?;
            id(&conversation_id)?;
            if imported.contains(&conversation_id) {
                continue;
            }
            if value.format_version != 1 || value.meeting_id != meeting_id {
                return Err(invalid());
            }
            value.format_version = 2;
            value.meeting_id.clear();
            value.title = value
                .messages
                .iter()
                .find(|m| m.role == "user")
                .map(|m| m.text.chars().take(60).collect())
                .unwrap_or_else(|| "이전 대화".into());
            value.created_at = now();
            value.updated_at = value.created_at;
            value.restore_sources();
            for message in &mut value.messages {
                if message.binding.is_some() {
                    if let Some(summary_id) = message.saved_summary_id.clone() {
                        message
                            .summary_receipts
                            .insert(meeting_id.into(), summary_id);
                    }
                    if message.note_saved {
                        message.note_receipts.insert(meeting_id.into());
                    }
                }
            }
            let target = directory.join(format!("{conversation_id}.json"));
            if plain(&target, false)? {
                let existing = self.read(&conversation_id)?;
                if existing
                    .messages
                    .iter()
                    .map(|m| (&m.id, &m.request_id))
                    .collect::<Vec<_>>()
                    != value
                        .messages
                        .iter()
                        .map(|m| (&m.id, &m.request_id))
                        .collect::<Vec<_>>()
                {
                    return Err(invalid());
                }
            } else {
                self.write_new(&value)?;
            }
            imported.insert(conversation_id);
            atomic_json(&marker, &imported)?;
        }
        self.legacy_imported.store(true, Ordering::Release);
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_do_not_initialize_and_global_conversations_do_not_create_meetings() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().into());
        assert!(store.list(false).unwrap().is_empty());
        assert!(!root.path().join("ai-conversations").exists());
        let value = store.create().unwrap();
        assert!(store.read(value.id.as_deref().unwrap()).is_ok());
        assert!(!root.path().join("meetings").exists());
    }
    #[test]
    fn list_collects_meetings_used_in_user_messages() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().into());
        let mut value = store.create().unwrap();
        let meeting_a = uuid::Uuid::new_v4().to_string();
        let meeting_b = uuid::Uuid::new_v4().to_string();
        let meeting_c = uuid::Uuid::new_v4().to_string();
        let message = |role: &str, meeting: &str| {
            serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(), "requestId": uuid::Uuid::new_v4().to_string(),
            "role": role, "text": "example", "status": "completed",
            "attachments": [{"kind": "meeting", "title": "example", "meetingId": meeting}]
        })).unwrap()
        };
        value.messages = vec![
            message("user", &meeting_a),
            message("user", &meeting_b),
            message("user", &meeting_a),
            message("assistant", &meeting_c),
        ];
        store.write(&value).unwrap();
        let mut expected = vec![meeting_a, meeting_b];
        expected.sort();
        assert_eq!(store.list(false).unwrap()[0].meeting_ids, expected);
    }
    #[test]
    fn listing_reuses_rows_of_unchanged_files_and_rereads_written_ones() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().into());
        let mut value = store.create().unwrap();
        let id = value.id.clone().unwrap();
        value.title = "처음 제목".into();
        store.write(&value).unwrap();
        assert_eq!(store.list(false).unwrap()[0].title, "처음 제목");
        // Same size and time but unreadable content: an unchanged file is not parsed again.
        let path = root
            .path()
            .join("ai-conversations")
            .join(format!("{id}.json"));
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let length = std::fs::metadata(&path).unwrap().len() as usize;
        std::fs::write(&path, vec![b' '; length]).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        assert_eq!(store.list(false).unwrap()[0].title, "처음 제목");
        value.title = "바뀐 제목".into();
        store.write(&value).unwrap();
        assert_eq!(store.list(false).unwrap()[0].title, "바뀐 제목");
        store.trash(&id).unwrap();
        assert!(store.list(false).unwrap().is_empty());
        assert_eq!(store.list(true).unwrap()[0].title, "바뀐 제목");
    }
    #[test]
    fn deleted_conversation_is_not_recreated_by_late_write() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().into());
        let value = store.create().unwrap();
        let id = value.id.as_deref().unwrap();
        store.remove(id).unwrap();
        assert!(store.read(id).is_err());
        assert!(store.write(&value).is_err());
    }
    #[test]
    fn oversized_update_preserves_previous_record() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().into());
        let mut value = store.create().unwrap();
        let id = value.id.clone().unwrap();
        let before = std::fs::read(
            root.path()
                .join("ai-conversations")
                .join(format!("{id}.json")),
        )
        .unwrap();
        value.title = "x".repeat(MAX_RECORD);
        assert!(store.write(&value).is_err());
        assert_eq!(
            std::fs::read(
                root.path()
                    .join("ai-conversations")
                    .join(format!("{id}.json"))
            )
            .unwrap(),
            before
        );
    }
    #[cfg(unix)]
    #[test]
    fn linked_global_storage_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("ai-conversations")).unwrap();
        let store = Store::new(root.path().into());
        assert!(store.create().is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    #[test]
    fn importing_legacy_preserves_original_bytes_and_deleted_chats_are_not_imported_again() {
        let root = tempfile::tempdir().unwrap();
        let meeting = uuid::Uuid::new_v4().to_string();
        let conversation = uuid::Uuid::new_v4().to_string();
        let request = uuid::Uuid::new_v4().to_string();
        let message = uuid::Uuid::new_v4().to_string();
        let directory = root.path().join("meetings").join(&meeting);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("ai-conversation.json");
        let raw=serde_json::to_vec(&serde_json::json!({"formatVersion":1,"id":conversation,"meetingId":meeting,"provider":"chatgpt","modelId":"synthetic","activeRequestId":null,"messages":[{"id":message,"requestId":request,"role":"assistant","text":"보존할 이전 답변","status":"completed"}]})).unwrap();
        std::fs::write(&path, &raw).unwrap();
        let store = Store::new(root.path().into());
        store.import_legacy().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), raw);
        assert_eq!(
            store.read(&conversation).unwrap().messages[0].text,
            "보존할 이전 답변"
        );
        assert_eq!(store.list(false).unwrap().len(), 1);
        store.remove(&conversation).unwrap();
        let restarted = Store::new(root.path().into());
        restarted.import_legacy().unwrap();
        assert!(restarted.list(false).unwrap().is_empty());
        assert_eq!(std::fs::read(path).unwrap(), raw);
    }
    #[test]
    fn failed_write_receipt_stays_visible_until_flush_and_delete_discards_it() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::new(root.path().into());
        let mut value = store.create().unwrap();
        let id = value.id.clone().unwrap();
        let path = root
            .path()
            .join("ai-conversations")
            .join(format!("{id}.json"));
        let saved = root.path().join("saved.json");
        std::fs::rename(&path, &saved).unwrap();
        std::fs::create_dir(&path).unwrap();
        value.title = "확정된 저장 영수증".into();
        store.write_receipt(&value);
        assert_eq!(store.read(&id).unwrap().title, value.title);
        std::fs::rename(&path, root.path().join("blocked-directory")).unwrap();
        std::fs::rename(&saved, &path).unwrap();
        store.write(&store.read(&id).unwrap()).unwrap();
        assert_eq!(
            Store::new(root.path().into()).read(&id).unwrap().title,
            value.title
        );
        store.remove(&id).unwrap();
        assert!(store.read(&id).is_err());
    }
}

#[cfg(test)]
mod legacy_attachment_tests {
    use super::*;
    #[test]
    fn imported_latest_user_keeps_the_matching_answers_meeting_attachment_for_followup() {
        let root = tempfile::tempdir().unwrap();
        let meeting = uuid::Uuid::new_v4().to_string();
        let conversation = uuid::Uuid::new_v4().to_string();
        let first = uuid::Uuid::new_v4().to_string();
        let latest = uuid::Uuid::new_v4().to_string();
        let binding = |meeting_id: &str| serde_json::json!({"meetingId":meeting_id,"meetingVersion":"saved-version","summaryContext":null,"citations":[],"template":null,"inputSha256":"saved-hash"});
        let unrelated = uuid::Uuid::new_v4().to_string();
        let raw=serde_json::to_vec(&serde_json::json!({"formatVersion":1,"id":conversation,"meetingId":meeting,"provider":"chatgpt","modelId":"synthetic","messages":[
            {"id":uuid::Uuid::new_v4().to_string(),"requestId":first,"role":"user","text":"이전 질문","status":"completed"},
            {"id":uuid::Uuid::new_v4().to_string(),"requestId":first,"role":"assistant","text":"이전 답변","status":"completed","binding":binding(&unrelated)},
            {"id":uuid::Uuid::new_v4().to_string(),"requestId":latest,"role":"user","text":"마지막 질문","status":"completed"},
            {"id":uuid::Uuid::new_v4().to_string(),"requestId":latest,"role":"assistant","text":"마지막 답변","status":"completed","binding":binding(&meeting)}
        ]})).unwrap();
        let directory = root.path().join("meetings").join(&meeting);
        std::fs::create_dir_all(&directory).unwrap();
        let original = directory.join("ai-conversation.json");
        std::fs::write(&original, &raw).unwrap();
        let store = Store::new(root.path().into());
        store.import_legacy().unwrap();
        let value = store.read(&conversation).unwrap();
        let last_user = value
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .unwrap();
        assert_eq!(last_user.attachments.len(), 1);
        assert_eq!(
            last_user.attachments[0].meeting_id.as_deref(),
            Some(meeting.as_str())
        );
        assert_ne!(
            last_user.attachments[0].meeting_id.as_deref(),
            Some(unrelated.as_str())
        );
        assert_eq!(last_user.save_targets[0].meeting_id, meeting);
        assert!(last_user.binding.is_none());
        assert_eq!(std::fs::read(original).unwrap(), raw);
    }
}
