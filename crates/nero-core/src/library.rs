use crate::store;
use crate::{CoreError, Meeting, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Folder {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Placement {
    pub folder_id: Option<String>,
    pub deleted_at: Option<u64>,
}

/// One atomic document owns classification and trash state for every meeting.
/// The same fields in meeting snapshots are projections, never a second writer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Library {
    pub schema_version: u32,
    pub folders: BTreeMap<String, Folder>,
    pub placements: BTreeMap<String, Placement>,
    pub pending_purges: BTreeSet<String>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            schema_version: 1,
            folders: BTreeMap::new(),
            placements: BTreeMap::new(),
            pending_purges: BTreeSet::new(),
        }
    }
}

impl Library {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(CoreError::Storage(
                "지원하지 않는 회의 목록 형식입니다.".into(),
            ));
        }
        let mut names = BTreeSet::new();
        for (id, folder) in &self.folders {
            store::validate_id(id)?;
            if id != &folder.id
                || folder.name != folder_name(&folder.name)?
                || !names.insert(folder.name.to_lowercase())
            {
                return Err(CoreError::Storage("폴더 정보가 올바르지 않습니다.".into()));
            }
        }
        for (id, placement) in &self.placements {
            store::validate_id(id)?;
            if let Some(folder_id) = &placement.folder_id {
                store::validate_id(folder_id)?;
                if !self.folders.contains_key(folder_id) {
                    return Err(CoreError::Storage("회의의 폴더를 찾을 수 없습니다.".into()));
                }
            }
            if self.pending_purges.contains(id) {
                return Err(CoreError::Storage("회의 삭제 정보가 충돌합니다.".into()));
            }
        }
        for id in &self.pending_purges {
            store::validate_id(id)?;
        }
        Ok(())
    }

    pub fn project(&self, meeting: &mut Meeting) {
        let placement = self.placements.get(&meeting.id);
        meeting.folder_id = placement.and_then(|entry| entry.folder_id.clone());
        meeting.deleted_at = placement.and_then(|entry| entry.deleted_at);
    }

    pub fn folders(&self) -> Vec<Folder> {
        let mut folders: Vec<_> = self.folders.values().cloned().collect();
        folders.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        folders
    }

    pub fn check_name(&self, name: &str, except_id: Option<&str>) -> Result<String> {
        let name = folder_name(name)?;
        if self.folders.values().any(|folder| {
            Some(folder.id.as_str()) != except_id
                && folder.name.to_lowercase() == name.to_lowercase()
        }) {
            return Err(CoreError::InvalidInput(
                "같은 이름의 폴더가 있습니다.".into(),
            ));
        }
        Ok(name)
    }
}

fn folder_name(value: &str) -> Result<String> {
    let name = value.trim();
    if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
        return Err(CoreError::InvalidInput(
            "폴더 이름은 1자에서 120자 사이로 입력해 주세요.".into(),
        ));
    }
    Ok(name.into())
}
