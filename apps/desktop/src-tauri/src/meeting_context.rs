use crate::notion_context::{normalize_page_id, NotionSource, MAX_PAGES};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

const MAX_RECORD: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextReference {
    pub kind: ContextKind,
    pub page_id: String,
    pub source_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    Notion,
}

impl ContextReference {
    pub fn notion(source: &NotionSource) -> Self {
        Self {
            kind: ContextKind::Notion,
            page_id: source.page_id.clone(),
            source_id: source.source_id.clone(),
        }
    }
    fn validate(&self) -> Result<(), String> {
        match self.kind {
            ContextKind::Notion => {
                if normalize_page_id(&self.page_id).ok().as_deref() != Some(&self.page_id) {
                    return Err(invalid());
                }
                let expected_prefix = format!("notion:{}:", self.page_id);
                let hash = self
                    .source_id
                    .strip_prefix(&expected_prefix)
                    .ok_or_else(invalid)?;
                if hash.len() != 64
                    || !hash
                        .bytes()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeetingContext {
    pub revision: u64,
    pub sources: Vec<ContextReference>,
}
impl Default for MeetingContext {
    fn default() -> Self {
        Self {
            revision: 0,
            sources: Vec::new(),
        }
    }
}

fn invalid() -> String {
    "저장된 회의 자료를 확인하지 못했습니다.".into()
}
fn conflict() -> String {
    "회의 자료가 변경되었습니다. 다시 불러온 뒤 시도해 주세요.".into()
}
fn validate_id(id: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id) {
        Ok(())
    } else {
        Err(invalid())
    }
}

fn safe(path: &Path) -> Result<(), String> {
    crate::path_safety::safe_ancestors(path).map_err(|_| invalid())
}

pub struct MeetingContextStore {
    directory: PathBuf,
    gate: Mutex<()>,
}
impl MeetingContextStore {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        if !root.is_absolute()
            || root
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(invalid());
        }
        let directory = root.join("meeting-context");
        safe(&directory)?;
        Ok(Self {
            directory,
            gate: Mutex::new(()),
        })
    }
    fn path(&self, meeting_id: &str) -> Result<PathBuf, String> {
        validate_id(meeting_id)?;
        let path = self.directory.join(format!("{meeting_id}.json"));
        safe(&path)?;
        Ok(path)
    }
    fn lock(&self) -> Result<fs::File, String> {
        safe(&self.directory)?;
        fs::create_dir_all(&self.directory).map_err(|_| invalid())?;
        let path = self.directory.join("references.lock");
        safe(&path)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .map_err(|_| invalid())?;
        file.lock_exclusive().map_err(|_| invalid())?;
        Ok(file)
    }
    fn load(&self, path: &Path) -> Result<MeetingContext, String> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(value) => value,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(MeetingContext::default())
            }
            Err(_) => return Err(invalid()),
        };
        if !metadata.is_file() || metadata.len() > MAX_RECORD as u64 {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        fs::File::open(path)
            .map_err(|_| invalid())?
            .take(MAX_RECORD as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid())?;
        if bytes.len() > MAX_RECORD {
            return Err(invalid());
        }
        let value: MeetingContext = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if value.sources.len() > MAX_PAGES {
            return Err(invalid());
        }
        let mut pages = BTreeSet::new();
        for source in &value.sources {
            source.validate()?;
            if !pages.insert((&source.kind, &source.page_id)) {
                return Err(invalid());
            }
        }
        Ok(value)
    }
    fn save(&self, path: &Path, value: &MeetingContext) -> Result<(), String> {
        safe(path)?;
        fs::create_dir_all(&self.directory).map_err(|_| invalid())?;
        safe(path)?;
        let mut file = tempfile::NamedTempFile::new_in(&self.directory).map_err(|_| invalid())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| invalid())?;
        }
        serde_json::to_writer(&mut file, value).map_err(|_| invalid())?;
        file.flush()
            .and_then(|_| file.as_file().sync_all())
            .map_err(|_| invalid())?;
        safe(path)?;
        file.persist(path).map_err(|_| invalid())?;
        #[cfg(unix)]
        fs::File::open(&self.directory)
            .and_then(|f| f.sync_all())
            .map_err(|_| invalid())?;
        Ok(())
    }
    pub fn list(&self, meeting_id: &str) -> Result<MeetingContext, String> {
        let _guard = self.gate.lock().map_err(|_| invalid())?;
        self.load(&self.path(meeting_id)?)
    }
    fn update(
        &self,
        meeting_id: &str,
        expected_revision: u64,
        change: impl FnOnce(&mut MeetingContext) -> Result<(), String>,
    ) -> Result<MeetingContext, String> {
        let _guard = self.gate.lock().map_err(|_| invalid())?;
        let path = self.path(meeting_id)?;
        let _file_lock = self.lock()?;
        let mut value = self.load(&path)?;
        if value.revision != expected_revision {
            return Err(conflict());
        }
        change(&mut value)?;
        value.revision = value.revision.checked_add(1).ok_or_else(invalid)?;
        self.save(&path, &value)?;
        Ok(value)
    }
    pub fn add(
        &self,
        meeting_id: &str,
        expected_revision: u64,
        source: &NotionSource,
    ) -> Result<MeetingContext, String> {
        let reference = ContextReference::notion(source);
        reference.validate()?;
        self.update(meeting_id, expected_revision, |value| {
            if value
                .sources
                .iter()
                .any(|s| s.kind == reference.kind && s.page_id == reference.page_id)
            {
                return Err("이미 추가된 회의 자료입니다.".into());
            }
            if value.sources.len() >= MAX_PAGES {
                return Err("회의 자료는 최대 8개까지 추가할 수 있습니다.".into());
            }
            value.sources.push(reference);
            Ok(())
        })
    }
    pub fn replace(
        &self,
        meeting_id: &str,
        expected_revision: u64,
        source: &NotionSource,
    ) -> Result<MeetingContext, String> {
        let reference = ContextReference::notion(source);
        reference.validate()?;
        self.update(meeting_id, expected_revision, |value| {
            let existing = value
                .sources
                .iter_mut()
                .find(|s| s.kind == reference.kind && s.page_id == reference.page_id)
                .ok_or_else(|| "회의 자료를 찾을 수 없습니다.".to_string())?;
            *existing = reference;
            Ok(())
        })
    }
    pub fn remove(
        &self,
        meeting_id: &str,
        expected_revision: u64,
        page_id: &str,
    ) -> Result<MeetingContext, String> {
        if normalize_page_id(page_id).ok().as_deref() != Some(page_id) {
            return Err(invalid());
        }
        self.update(meeting_id, expected_revision, |value| {
            let index = value
                .sources
                .iter()
                .position(|s| s.kind == ContextKind::Notion && s.page_id == page_id)
                .ok_or_else(|| "회의 자료를 찾을 수 없습니다.".to_string())?;
            value.sources.remove(index);
            Ok(())
        })
    }
    pub fn delete(&self, meeting_id: &str) -> Result<(), String> {
        let _guard = self.gate.lock().map_err(|_| invalid())?;
        let path = self.path(meeting_id)?;
        if !self.directory.try_exists().map_err(|_| invalid())? {
            return Ok(());
        }
        let _file_lock = self.lock()?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(invalid()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(page_id: &str, hash: char) -> NotionSource {
        NotionSource {
            source_id: format!("notion:{page_id}:{}", hash.to_string().repeat(64)),
            page_id: page_id.into(),
            url: format!("https://notion.so/{page_id}"),
            title: None,
            last_edited_at: None,
            fetched_at: 0,
            sha256: String::new(),
            body: "private document body".into(),
            truncated: false,
            unknown_block_count: None,
        }
    }

    #[test]
    fn references_are_revisioned_and_do_not_duplicate_document_bodies() {
        let temp = tempfile::tempdir().unwrap();
        let store = MeetingContextStore::new(temp.path().canonicalize().unwrap()).unwrap();
        let meeting = "a0000000-0000-4000-8000-000000000001";
        let page = "b0000000-0000-4000-8000-000000000001";
        let original = source(page, 'a');
        let refreshed = source(page, 'b');
        assert_eq!(store.list(meeting).unwrap(), MeetingContext::default());
        assert!(!temp.path().join("meeting-context").exists());
        let added = store.add(meeting, 0, &original).unwrap();
        assert_eq!(added.revision, 1);
        assert!(store.add(meeting, 1, &refreshed).is_err());
        let second_store = MeetingContextStore::new(temp.path().canonicalize().unwrap()).unwrap();
        assert!(second_store.replace(meeting, 0, &refreshed).is_err());
        let updated = store.replace(meeting, 1, &refreshed).unwrap();
        assert_eq!(updated.sources[0].source_id, refreshed.source_id);
        let record = fs::read_to_string(
            temp.path()
                .join("meeting-context")
                .join(format!("{meeting}.json")),
        )
        .unwrap();
        assert!(!record.contains(&original.body));
        assert_eq!(store.remove(meeting, 2, page).unwrap().revision, 3);
        assert!(store.add(meeting, 0, &original).is_err());
        assert_eq!(store.list(meeting).unwrap().revision, 3);
        store.delete(meeting).unwrap();
        assert_eq!(store.list(meeting).unwrap(), MeetingContext::default());
    }

    #[test]
    fn invalid_ids_and_tampered_records_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let store = MeetingContextStore::new(temp.path().canonicalize().unwrap()).unwrap();
        assert!(store.list("../other").is_err());
        let meeting = "a0000000-0000-4000-8000-000000000001";
        let page = "b0000000-0000-4000-8000-000000000001";
        let original = source(page, 'a');
        store.add(meeting, 0, &original).unwrap();
        let path = temp
            .path()
            .join("meeting-context")
            .join(format!("{meeting}.json"));
        let mut value = store.list(meeting).unwrap();
        value.sources.push(value.sources[0].clone());
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(store.list(meeting).is_err());
    }
}
