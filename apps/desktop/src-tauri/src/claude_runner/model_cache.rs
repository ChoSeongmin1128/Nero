use super::{valid_model, SupportedModel};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const LIMIT: u64 = 128 * 1024;
const ERROR: &str = "Claude 모델 목록을 저장하지 못했습니다.";
const SOURCE: &str = "signed-official-claude-code";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Cache {
    version: u8,
    source: String,
    runtime_sha256: Option<String>,
    models: Vec<SupportedModel>,
}

pub struct ModelCache {
    path: PathBuf,
}
impl ModelCache {
    pub fn new(root: &Path) -> Self {
        Self {
            path: root.join("ai-accounts/claude-models-public-v1.json"),
        }
    }
    pub fn read(&self) -> Vec<SupportedModel> {
        self.read_inner().unwrap_or_default()
    }
    fn read_inner(&self) -> Result<Vec<SupportedModel>, String> {
        crate::path_safety::safe_ancestors(&self.path).map_err(|_| ERROR)?;
        let file = fs::File::open(&self.path).map_err(|_| ERROR)?;
        if !file.metadata().map_err(|_| ERROR)?.is_file() {
            return Err(ERROR.into());
        }
        let mut bytes = Vec::new();
        file.take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ERROR)?;
        if bytes.len() as u64 > LIMIT {
            return Err(ERROR.into());
        }
        let cache: Cache = serde_json::from_slice(&bytes).map_err(|_| ERROR)?;
        if cache.version != 1
            || cache.source != SOURCE
            || !valid(&cache.models)
            || (!cache.models.is_empty()
                && !cache.runtime_sha256.as_deref().is_some_and(valid_hash))
            || cache
                .runtime_sha256
                .as_deref()
                .is_some_and(|hash| !valid_hash(hash))
        {
            return Err(ERROR.into());
        }
        Ok(cache.models)
    }
    pub fn write(&self, models: &[SupportedModel], hash: Option<&str>) -> Result<(), String> {
        if !valid(models)
            || (!models.is_empty() && !hash.is_some_and(valid_hash))
            || hash.is_some_and(|v| !valid_hash(v))
        {
            return Err(ERROR.into());
        }
        let bytes = serde_json::to_vec(&Cache {
            version: 1,
            source: SOURCE.into(),
            runtime_sha256: hash.map(str::to_owned),
            models: models.to_vec(),
        })
        .map_err(|_| ERROR)?;
        if bytes.len() as u64 > LIMIT {
            return Err(ERROR.into());
        }
        let parent = self.path.parent().ok_or(ERROR)?;
        crate::path_safety::safe_ancestors(&self.path).map_err(|_| ERROR)?;
        fs::create_dir_all(parent).map_err(|_| ERROR)?;
        crate::path_safety::safe_ancestors(&self.path).map_err(|_| ERROR)?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|_| ERROR)?;
        file.write_all(&bytes)
            .and_then(|_| file.as_file().sync_all())
            .map_err(|_| ERROR)?;
        crate::path_safety::safe_ancestors(&self.path).map_err(|_| ERROR)?;
        file.persist(&self.path).map_err(|_| ERROR)?;
        Ok(())
    }
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}
fn valid(models: &[SupportedModel]) -> bool {
    models.len() <= 256
        && models.iter().enumerate().all(|(index, model)| {
            valid_model(&model.value)
                && !model.display_name.is_empty()
                && model.display_name.len() <= 256
                && !model.display_name.chars().any(char::is_control)
                && model.resolved_model.as_deref().is_none_or(valid_model)
                && !models[..index]
                    .iter()
                    .any(|other| other.value == model.value)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_reopens_and_rejects_bad_source_without_account_data() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let cache = ModelCache::new(&root);
        let model = SupportedModel {
            value: "sonnet".into(),
            display_name: "Sonnet".into(),
            resolved_model: None,
        };
        cache.write(&[model], Some(&"a".repeat(64))).unwrap();
        assert_eq!(ModelCache::new(&root).read()[0].value, "sonnet");
        assert!(cache.write(&cache.read(), Some("bad hash")).is_err());
        assert_eq!(cache.read()[0].value, "sonnet");
        let path = dir.path().join("ai-accounts/claude-models-public-v1.json");
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("account"));
        fs::write(path, text.replace(SOURCE, "untrusted")).unwrap();
        assert!(cache.read().is_empty());
    }
}
