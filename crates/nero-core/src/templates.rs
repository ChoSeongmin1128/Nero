use crate::store;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

pub const MAX_TEMPLATE_PROMPT_BYTES: usize = 64 * 1024;
const MAX_DOCUMENT_BYTES: usize = store::MAX_JSON_BYTES as usize;
const FORMAT_VERSION: u32 = 1;
const DEFAULTS: &str = include_str!("templates/defaults.json");

pub type Result<T> = std::result::Result<T, TemplateError>;

#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("템플릿 입력값이 올바르지 않습니다.")]
    InvalidInput,
    #[error("템플릿을 찾을 수 없습니다.")]
    NotFound,
    #[error("템플릿이 변경되었습니다. 최신 내용을 확인한 뒤 다시 저장해 주세요.")]
    Conflict,
    #[error("다른 템플릿 변경을 처리 중입니다. 잠시 후 다시 시도해 주세요.")]
    Busy,
    #[error("저장된 템플릿 정보가 손상되었습니다.")]
    InvalidDocument,
    #[error("지원하지 않는 템플릿 저장 형식입니다.")]
    UnsupportedFormat,
    #[error("연결된 파일이나 올바르지 않은 템플릿 경로는 사용할 수 없습니다.")]
    UnsafePath,
    #[error("템플릿을 읽거나 저장하지 못했습니다.")]
    Storage,
}

impl From<std::io::Error> for TemplateError {
    fn from(_: std::io::Error) -> Self {
        Self::Storage
    }
}

impl From<crate::CoreError> for TemplateError {
    fn from(_: crate::CoreError) -> Self {
        Self::Storage
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateContent {
    pub name: String,
    pub description: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub revision: String,
    pub built_in: bool,
    pub customized: bool,
    pub default_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateCatalog {
    pub revision: String,
    pub templates: Vec<TemplateSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub prompt: String,
    pub revision: String,
    pub built_in: bool,
    pub customized: bool,
    pub default_revision: Option<String>,
}

impl PromptTemplate {
    pub fn summary(&self) -> TemplateSummary {
        TemplateSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            revision: self.revision.clone(),
            built_in: self.built_in,
            customized: self.customized,
            default_revision: self.default_revision.clone(),
        }
    }

    fn content(&self) -> TemplateContent {
        TemplateContent {
            name: self.name.clone(),
            description: self.description.clone(),
            prompt: self.prompt.clone(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Builtin {
    id: String,
    name: String,
    description: String,
    prompt: String,
}

impl Builtin {
    fn content(&self) -> TemplateContent {
        TemplateContent {
            name: self.name.clone(),
            description: self.description.clone(),
            prompt: self.prompt.clone(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Defaults {
    version: u32,
    templates: Vec<Builtin>,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredTemplate {
    content: Option<TemplateContent>,
    nonce: String,
    #[serde(rename = "deleted", default, skip_serializing_if = "is_false")]
    legacy_deleted: bool,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Catalog {
    version: u32,
    templates: BTreeMap<String, StoredTemplate>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    suppressed_builtins: BTreeSet<String>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            version: FORMAT_VERSION,
            templates: BTreeMap::new(),
            suppressed_builtins: BTreeSet::new(),
        }
    }
}

pub struct TemplateStore {
    root: PathBuf,
    defaults: String,
}

impl TemplateStore {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_owned(),
            defaults: DEFAULTS.to_owned(),
        }
    }

    pub fn list(&self) -> Result<Vec<TemplateSummary>> {
        Ok(self.catalog()?.templates)
    }

    pub fn catalog(&self) -> Result<TemplateCatalog> {
        let builtins = self.builtins()?;
        let catalog = self.read_catalog(&builtins)?;
        self.catalog_view(&catalog, &builtins)
    }

    fn catalog_view(&self, catalog: &Catalog, builtins: &[Builtin]) -> Result<TemplateCatalog> {
        let mut values = Vec::new();
        for builtin in builtins {
            if !catalog.suppressed_builtins.contains(&builtin.id) {
                values.push(self.effective(catalog, builtins, &builtin.id)?.summary());
            }
        }
        let mut custom = Vec::new();
        for (id, stored) in &catalog.templates {
            if stored.legacy_deleted || builtins.iter().any(|builtin| &builtin.id == id) {
                continue;
            }
            custom.push(self.effective(catalog, builtins, id)?.summary());
        }
        custom.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        values.extend(custom);
        Ok(TemplateCatalog {
            revision: digest(&("template-catalog-v1", builtins, catalog)),
            templates: values,
        })
    }

    pub fn reset_all(&self, expected_revision: &str) -> Result<TemplateCatalog> {
        validate_revision(expected_revision)?;
        self.mutate(|catalog, builtins| {
            if self.catalog_view(catalog, builtins)?.revision != expected_revision {
                return Err(TemplateError::Conflict);
            }
            // Fresh nonces invalidate editors opened before the reset, including
            // templates whose visible content already matched bundled defaults.
            catalog.templates = builtins
                .iter()
                .map(|builtin| {
                    (
                        builtin.id.clone(),
                        StoredTemplate {
                            content: None,
                            nonce: Uuid::new_v4().to_string(),
                            legacy_deleted: false,
                        },
                    )
                })
                .collect();
            catalog.suppressed_builtins.clear();
            self.catalog_view(catalog, builtins)
        })
    }

    pub fn get(&self, id: &str) -> Result<PromptTemplate> {
        let builtins = self.builtins()?;
        validate_id(id, &builtins)?;
        let catalog = self.read_catalog(&builtins)?;
        self.effective(&catalog, &builtins, id)
    }

    pub fn create(&self, content: TemplateContent) -> Result<PromptTemplate> {
        validate_content(&content)?;
        self.mutate(|catalog, builtins| {
            let id = Uuid::new_v4().to_string();
            catalog.templates.insert(
                id.clone(),
                StoredTemplate {
                    content: Some(content),
                    nonce: Uuid::new_v4().to_string(),
                    legacy_deleted: false,
                },
            );
            self.effective(catalog, builtins, &id)
        })
    }

    pub fn update(
        &self,
        id: &str,
        expected_revision: &str,
        content: TemplateContent,
    ) -> Result<PromptTemplate> {
        validate_content(&content)?;
        validate_revision(expected_revision)?;
        validate_id(id, &self.builtins()?)?;
        self.mutate(|catalog, builtins| {
            validate_id(id, builtins)?;
            let previous = self.effective(catalog, builtins, id)?;
            require_revision(&previous, expected_revision)?;
            if previous.content() == content {
                return Ok(previous);
            }
            catalog.templates.insert(
                id.into(),
                StoredTemplate {
                    content: Some(content),
                    nonce: Uuid::new_v4().to_string(),
                    legacy_deleted: false,
                },
            );
            self.effective(catalog, builtins, id)
        })
    }

    pub fn reset(&self, id: &str, expected_revision: &str) -> Result<PromptTemplate> {
        validate_revision(expected_revision)?;
        validate_id(id, &self.builtins()?)?;
        self.mutate(|catalog, builtins| {
            validate_id(id, builtins)?;
            let previous = self.effective(catalog, builtins, id)?;
            require_revision(&previous, expected_revision)?;
            if !previous.built_in {
                return Err(TemplateError::InvalidInput);
            }
            if !previous.customized {
                return Ok(previous);
            }
            catalog.templates.insert(
                id.into(),
                StoredTemplate {
                    content: None,
                    nonce: Uuid::new_v4().to_string(),
                    legacy_deleted: false,
                },
            );
            self.effective(catalog, builtins, id)
        })
    }

    pub fn delete(&self, id: &str, expected_revision: &str) -> Result<()> {
        validate_revision(expected_revision)?;
        validate_id(id, &self.builtins()?)?;
        self.mutate(|catalog, builtins| {
            validate_id(id, builtins)?;
            let previous = self.effective(catalog, builtins, id)?;
            require_revision(&previous, expected_revision)?;
            if previous.built_in {
                catalog.suppressed_builtins.insert(id.into());
                catalog.templates.remove(id);
            } else {
                catalog
                    .templates
                    .remove(id)
                    .ok_or(TemplateError::NotFound)?;
            }
            Ok(())
        })
    }

    fn builtins(&self) -> Result<Vec<Builtin>> {
        let defaults: Defaults =
            serde_json::from_str(&self.defaults).map_err(|_| TemplateError::InvalidDocument)?;
        if defaults.version != FORMAT_VERSION {
            return Err(TemplateError::UnsupportedFormat);
        }
        let mut ids = BTreeSet::new();
        if defaults.templates.is_empty() {
            return Err(TemplateError::InvalidDocument);
        }
        for builtin in &defaults.templates {
            if !valid_builtin_id(&builtin.id)
                || !ids.insert(&builtin.id)
                || validate_content(&builtin.content()).is_err()
            {
                return Err(TemplateError::InvalidDocument);
            }
        }
        Ok(defaults.templates)
    }

    fn read_catalog(&self, builtins: &[Builtin]) -> Result<Catalog> {
        self.validate_root()?;
        let folder = self.root.join("templates");
        validate_directory(&folder)?;
        let path = folder.join("catalog.json");
        let Some(bytes) = read_optional(&path)? else {
            return Ok(Catalog::default());
        };
        let catalog: Catalog =
            serde_json::from_slice(&bytes).map_err(|_| TemplateError::InvalidDocument)?;
        if catalog.version != FORMAT_VERSION {
            return Err(TemplateError::UnsupportedFormat);
        }
        if catalog
            .suppressed_builtins
            .iter()
            .any(|id| !valid_builtin_id(id))
        {
            return Err(TemplateError::InvalidDocument);
        }
        for (id, value) in &catalog.templates {
            validate_id(id, builtins).map_err(|_| TemplateError::InvalidDocument)?;
            if !valid_uuid(&value.nonce)
                || value
                    .content
                    .as_ref()
                    .is_some_and(|content| validate_content(content).is_err())
                || value.content.is_none() && !builtins.iter().any(|builtin| &builtin.id == id)
                || value.legacy_deleted && builtins.iter().any(|builtin| &builtin.id == id)
                || catalog.suppressed_builtins.contains(id)
            {
                return Err(TemplateError::InvalidDocument);
            }
        }
        Ok(catalog)
    }

    fn effective(
        &self,
        catalog: &Catalog,
        builtins: &[Builtin],
        id: &str,
    ) -> Result<PromptTemplate> {
        let builtin = builtins.iter().find(|builtin| builtin.id == id);
        if catalog.suppressed_builtins.contains(id) {
            return Err(TemplateError::NotFound);
        }
        let stored = catalog.templates.get(id);
        if stored.is_some_and(|value| value.legacy_deleted) {
            return Err(TemplateError::NotFound);
        }
        let base = builtin.map(Builtin::content);
        let content = stored
            .and_then(|value| value.content.clone())
            .or_else(|| base.clone())
            .ok_or(TemplateError::NotFound)?;
        let default_revision = base.as_ref().map(|content| digest(&(id, content)));
        let revision = digest(&(
            id,
            &content,
            &default_revision,
            false,
            stored.map(|value| value.nonce.as_str()),
        ));
        let customized = base.as_ref().is_some_and(|base| base != &content);
        Ok(PromptTemplate {
            id: id.into(),
            name: content.name,
            description: content.description,
            prompt: content.prompt,
            revision,
            built_in: builtin.is_some(),
            customized,
            default_revision,
        })
    }

    fn mutate<T>(&self, action: impl FnOnce(&mut Catalog, &[Builtin]) -> Result<T>) -> Result<T> {
        let builtins = self.builtins()?;
        let _lock = self.lock()?;
        let original = self.read_catalog(&builtins)?;
        let mut catalog = original.clone();
        let result = action(&mut catalog, &builtins)?;
        if catalog != original {
            let bytes = serde_json::to_vec(&catalog).map_err(|_| TemplateError::InvalidDocument)?;
            if bytes.len() + 1 > MAX_DOCUMENT_BYTES {
                return Err(TemplateError::InvalidInput);
            }
            self.validate_root()?;
            store::write_json(&self.root, Path::new("templates/catalog.json"), &catalog)?;
        }
        Ok(result)
    }

    fn validate_root(&self) -> Result<()> {
        if !self.root.is_absolute()
            || self
                .root
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
        {
            return Err(TemplateError::UnsafePath);
        }
        let mut current = PathBuf::new();
        for part in self.root.components() {
            current.push(part.as_os_str());
            if matches!(part, Component::Prefix(_)) {
                continue;
            }
            validate_directory(&current)?;
        }
        Ok(())
    }

    fn lock(&self) -> Result<File> {
        self.validate_root()?;
        let folder = self.root.join("templates");
        store::ensure_directory(&folder)?;
        self.validate_root()?;
        validate_directory(&folder)?;
        let path = folder.join("catalog.lock");
        validate_optional_file(&path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        no_follow(&mut options);
        let file = options.open(&path)?;
        validate_file_metadata(&file.metadata()?)?;
        file.try_lock_exclusive().map_err(|_| TemplateError::Busy)?;
        validate_optional_file(&path)?;
        Ok(file)
    }
}

fn is_false(value: &bool) -> bool {
    !value
}

fn digest(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("template revision has only serializable text");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn validate_content(content: &TemplateContent) -> Result<()> {
    if content.name.trim().is_empty()
        || content.name.len() > 256
        || content
            .name
            .chars()
            .any(|value| matches!(value, '\0' | '\n' | '\r'))
        || content.description.len() > 4096
        || content.description.contains('\0')
        || content.prompt.trim().is_empty()
        || content.prompt.len() > MAX_TEMPLATE_PROMPT_BYTES
        || content.prompt.contains('\0')
    {
        return Err(TemplateError::InvalidInput);
    }
    Ok(())
}

fn valid_uuid(value: &str) -> bool {
    Uuid::parse_str(value)
        .ok()
        .map(|uuid| uuid.to_string())
        .as_deref()
        == Some(value)
}

fn valid_builtin_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && Uuid::parse_str(id).is_err()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn validate_id(id: &str, builtins: &[Builtin]) -> Result<()> {
    if builtins.iter().any(|builtin| builtin.id == id) || valid_uuid(id) {
        Ok(())
    } else {
        Err(TemplateError::InvalidInput)
    }
}

fn validate_revision(value: &str) -> Result<()> {
    if value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) {
        Ok(())
    } else {
        Err(TemplateError::InvalidInput)
    }
}

fn require_revision(template: &PromptTemplate, expected: &str) -> Result<()> {
    if template.revision == expected {
        Ok(())
    } else {
        Err(TemplateError::Conflict)
    }
}

fn validate_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && !is_reparse(&metadata) =>
        {
            Ok(())
        }
        Ok(_) => Err(TemplateError::UnsafePath),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn validate_optional_file(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_file_metadata(&metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn validate_file_metadata(metadata: &fs::Metadata) -> Result<()> {
    if !metadata.is_file() || metadata.file_type().is_symlink() || is_reparse(metadata) {
        return Err(TemplateError::UnsafePath);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(TemplateError::UnsafePath);
        }
    }
    Ok(())
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(metadata) => {
            validate_file_metadata(&metadata)?;
            if metadata.len() > MAX_DOCUMENT_BYTES as u64 {
                return Err(TemplateError::InvalidDocument);
            }
        }
    }
    let mut file = store::open_read(path)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(TemplateError::InvalidDocument);
    }
    Ok(Some(bytes))
}

#[cfg(windows)]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
fn is_reparse(_: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn no_follow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
}
#[cfg(windows)]
fn no_follow(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    options.custom_flags(0x0020_0000);
}
#[cfg(not(any(unix, windows)))]
fn no_follow(_: &mut OpenOptions) {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn temporary() -> TempDir {
        tempfile::Builder::new()
            .prefix("nero-template-test-")
            .tempdir_in(std::env::temp_dir().canonicalize().unwrap())
            .unwrap()
    }

    fn content(prompt: &str) -> TemplateContent {
        TemplateContent {
            name: "검토용 템플릿".into(),
            description: "합성 입력 확인".into(),
            prompt: prompt.into(),
        }
    }

    fn fixture(root: &Path) -> TemplateStore {
        fixture_with_default(root, "사실과 결정을 구분합니다.")
    }

    fn fixture_with_default(root: &Path, prompt: &str) -> TemplateStore {
        TemplateStore {
            root: root.to_owned(),
            defaults: json!({"version":1,"templates":[
                {"id":"meeting","name":"회의 요약","description":"일반 회의","prompt":prompt},
                {"id":"interview","name":"인터뷰 요약","description":"발언 근거","prompt":"관찰과 해석을 구분합니다."}
            ]}).to_string(),
        }
    }

    #[test]
    fn missing_library_reads_bundled_defaults_without_creating_anything() {
        let temp = temporary();
        let root = temp.path().join("does-not-exist");
        let store = fixture(&root);
        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "meeting");
        assert!(list[0].built_in);
        assert!(!list[0].customized);
        let encoded = serde_json::to_value(&list).unwrap();
        assert!(encoded[0].get("prompt").is_none());
        assert!(encoded[0].get("deleted").is_none());
        assert_eq!(
            store.get("meeting").unwrap().prompt,
            "사실과 결정을 구분합니다."
        );
        assert!(!root.exists());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[test]
    fn custom_changes_share_cas_and_deletion_removes_the_catalog_entry() {
        let temp = temporary();
        let root = temp.path().join("library");
        let first = fixture(&root);
        let second = fixture(&root);
        let created = first
            .create(content("첫 번째 사용자 프롬프트\n끝 개행\n"))
            .unwrap();
        assert!(valid_uuid(&created.id));
        assert!(!created.built_in);
        assert_eq!(second.get(&created.id).unwrap(), created);
        let edited = second
            .update(
                &created.id,
                &created.revision,
                content("  수정한 사용자 프롬프트\n"),
            )
            .unwrap();
        assert_ne!(edited.revision, created.revision);
        let before = fs::read(root.join("templates/catalog.json")).unwrap();
        assert!(matches!(
            first.update(&created.id, &created.revision, content("오래된 편집")),
            Err(TemplateError::Conflict)
        ));
        assert!(matches!(
            first.delete(&created.id, &created.revision),
            Err(TemplateError::Conflict)
        ));
        assert_eq!(
            before,
            fs::read(root.join("templates/catalog.json")).unwrap()
        );
        first.delete(&edited.id, &edited.revision).unwrap();
        assert!(matches!(
            second.get(&edited.id),
            Err(TemplateError::NotFound)
        ));
        assert!(matches!(
            second.delete(&edited.id, &edited.revision),
            Err(TemplateError::NotFound)
        ));
        assert_eq!(second.list().unwrap().len(), 2);
        let catalog: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("templates/catalog.json")).unwrap())
                .unwrap();
        assert!(catalog["templates"].get(&edited.id).is_none());
        assert!(!root.join("store.lock").exists());
        assert!(!root.join("meetings").exists());
        assert!(!root.join("models").exists());
        assert!(!root.join("templates/history").exists());
    }

    #[test]
    fn legacy_deleted_entries_remain_unchanged_until_explicit_migration() {
        let temp = temporary();
        let store = fixture(temp.path());
        let created = store.create(content("보존할 삭제 전 원문")).unwrap();
        let path = temp.path().join("templates/catalog.json");
        let mut catalog: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        catalog["templates"][&created.id]["deleted"] = json!(true);
        let bytes = serde_json::to_vec(&catalog).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert_eq!(store.list().unwrap().len(), 2);
        assert!(matches!(
            store.get(&created.id),
            Err(TemplateError::NotFound)
        ));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        store.create(content("다른 항목")).unwrap();
        let after: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            after["templates"][&created.id],
            catalog["templates"][&created.id]
        );
    }

    #[test]
    fn builtin_reset_restores_defaults_and_prevents_aba_overwrites() {
        let temp = temporary();
        let store = fixture(temp.path());
        let original = store.get("meeting").unwrap();
        let edited = store
            .update(
                "meeting",
                &original.revision,
                content("사용자 정의 회의 형식"),
            )
            .unwrap();
        assert!(edited.built_in);
        assert!(edited.customized);
        assert_eq!(edited.default_revision, original.default_revision);
        let reset = store.reset("meeting", &edited.revision).unwrap();
        assert_eq!(reset.content(), original.content());
        assert!(!reset.customized);
        assert_ne!(reset.revision, original.revision);
        assert_ne!(reset.revision, edited.revision);
        assert!(matches!(
            store.update("meeting", &original.revision, content("오래된 편집")),
            Err(TemplateError::Conflict)
        ));
        assert_eq!(store.reset("meeting", &reset.revision).unwrap(), reset);
        assert!(!temp.path().join("templates/history").exists());
    }

    #[test]
    fn default_updates_keep_overrides_and_invalidate_old_default_revisions() {
        let temp = temporary();
        let first = fixture(temp.path());
        let original = first.get("meeting").unwrap();
        let next = fixture_with_default(temp.path(), "새 기본값");
        let new_default = next.get("meeting").unwrap();
        assert_ne!(original.revision, new_default.revision);
        assert_ne!(original.default_revision, new_default.default_revision);
        assert!(matches!(
            next.update("meeting", &original.revision, content("오래된 버전")),
            Err(TemplateError::Conflict)
        ));
        let changed = first
            .update("meeting", &original.revision, content("사용자 형식 유지"))
            .unwrap();
        let after_upgrade = next.get("meeting").unwrap();
        assert_eq!(after_upgrade.content(), changed.content());
        assert_ne!(after_upgrade.revision, changed.revision);
        let reset = next.reset("meeting", &after_upgrade.revision).unwrap();
        assert_eq!(reset.prompt, "새 기본값");
        let third = fixture_with_default(temp.path(), "세 번째 기본값");
        assert_eq!(third.get("meeting").unwrap().prompt, "세 번째 기본값");
    }

    #[test]
    fn deleted_builtin_keeps_only_its_id_and_stays_hidden_after_default_updates() {
        let temp = temporary();
        let store = fixture(temp.path());
        let original = store.get("meeting").unwrap();
        let edited = store
            .update(
                "meeting",
                &original.revision,
                content("deleted-prompt-marker"),
            )
            .unwrap();
        assert!(matches!(
            store.delete("meeting", &original.revision),
            Err(TemplateError::Conflict)
        ));
        store.delete("meeting", &edited.revision).unwrap();
        let bytes = fs::read(temp.path().join("templates/catalog.json")).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("deleted-prompt-marker"));
        let catalog: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(catalog["templates"].get("meeting").is_none());
        assert_eq!(catalog["suppressedBuiltins"], json!(["meeting"]));
        let upgraded = fixture_with_default(temp.path(), "updated bundled prompt");
        assert!(matches!(
            upgraded.get("meeting"),
            Err(TemplateError::NotFound)
        ));
        assert_eq!(upgraded.list().unwrap().len(), 1);
        // A later default set may temporarily omit the deleted ID. Keep the
        // suppression without requiring a retained prompt or a current default.
        let without_meeting = TemplateStore { root: temp.path().to_owned(), defaults: json!({"version":1,"templates":[
            {"id":"interview","name":"Interview","description":"Synthetic","prompt":"Instructions"}
        ]}).to_string() };
        assert_eq!(without_meeting.list().unwrap().len(), 1);
        assert!(matches!(
            fixture(temp.path()).get("meeting"),
            Err(TemplateError::NotFound)
        ));
        assert_eq!(
            fs::read(temp.path().join("templates/catalog.json")).unwrap(),
            bytes
        );
    }

    #[test]
    fn reset_all_uses_catalog_cas_and_invalidates_individual_editors() {
        let temp = temporary();
        let first = fixture(temp.path());
        let original = first.get("meeting").unwrap();
        let old_catalog = first.catalog().unwrap();
        let second = fixture(temp.path());
        let custom = second.create(content("custom-prompt-marker")).unwrap();
        let before = fs::read(temp.path().join("templates/catalog.json")).unwrap();
        assert!(matches!(
            first.reset_all(&old_catalog.revision),
            Err(TemplateError::Conflict)
        ));
        assert_eq!(
            fs::read(temp.path().join("templates/catalog.json")).unwrap(),
            before
        );
        assert_eq!(first.get(&custom.id).unwrap(), custom);
        let edited = first
            .update(
                "meeting",
                &original.revision,
                content("override-prompt-marker"),
            )
            .unwrap();
        first.delete("meeting", &edited.revision).unwrap();
        let latest = first.catalog().unwrap();
        let reset = first.reset_all(&latest.revision).unwrap();
        assert_eq!(reset, second.catalog().unwrap());
        assert_eq!(reset.templates.len(), 2);
        assert!(reset
            .templates
            .iter()
            .all(|item| item.built_in && !item.customized));
        assert!(matches!(
            first.get(&custom.id),
            Err(TemplateError::NotFound)
        ));
        let restored = first.get("meeting").unwrap();
        assert_eq!(restored.content(), original.content());
        assert_ne!(restored.revision, original.revision);
        assert!(matches!(
            first.update("meeting", &original.revision, content("stale editor")),
            Err(TemplateError::Conflict)
        ));
        let bytes = fs::read(temp.path().join("templates/catalog.json")).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("custom-prompt-marker"));
        assert!(!String::from_utf8_lossy(&bytes).contains("override-prompt-marker"));
        assert!(!temp.path().join("templates/history").exists());
    }

    #[test]
    fn empty_catalog_survives_reopen_and_reset_restores_only_defaults() {
        let temp = temporary();
        let store = fixture(temp.path());
        let custom = store.create(content("custom")).unwrap();
        store.delete(&custom.id, &custom.revision).unwrap();
        for item in store.list().unwrap() {
            store.delete(&item.id, &item.revision).unwrap();
        }
        assert!(fixture(temp.path()).list().unwrap().is_empty());
        fs::create_dir(temp.path().join("meetings")).unwrap();
        let unrelated = temp.path().join("meetings/existing-summary.json");
        fs::write(
            &unrelated,
            b"synthetic archived summary and prompt reference",
        )
        .unwrap();
        let existing = fs::read(&unrelated).unwrap();
        let empty = store.catalog().unwrap();
        assert!(empty.templates.is_empty());
        let defaults = store.reset_all(&empty.revision).unwrap();
        assert_eq!(defaults.templates.len(), 2);
        assert_eq!(fs::read(&unrelated).unwrap(), existing);
        assert!(matches!(
            store.get(&custom.id),
            Err(TemplateError::NotFound)
        ));
    }

    #[test]
    fn catalog_revision_covers_default_updates_without_writing_legacy_catalogs() {
        let temp = temporary();
        let first = fixture(temp.path());
        let created = first.create(content("legacy v1 content")).unwrap();
        let path = temp.path().join("templates/catalog.json");
        let before = fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&before).contains("suppressedBuiltins"));
        let original = first.catalog().unwrap();
        let upgraded = fixture_with_default(temp.path(), "updated default");
        assert_ne!(original.revision, upgraded.catalog().unwrap().revision);
        assert!(matches!(
            upgraded.reset_all(&original.revision),
            Err(TemplateError::Conflict)
        ));
        assert_eq!(upgraded.get(&created.id).unwrap(), created);
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn simultaneous_mutation_is_rejected_while_reads_remain_available() {
        let temp = temporary();
        let first = fixture(temp.path());
        let second = fixture(temp.path());
        let lock = first.lock().unwrap();
        assert!(matches!(
            second.create(content("새 형식")),
            Err(TemplateError::Busy)
        ));
        assert!(matches!(
            second.reset_all(&second.catalog().unwrap().revision),
            Err(TemplateError::Busy)
        ));
        assert_eq!(second.list().unwrap().len(), 2);
        drop(lock);
        assert!(second.create(content("새 형식")).is_ok());
    }

    #[test]
    fn malformed_or_future_catalog_is_never_overwritten() {
        let temp = temporary();
        let folder = temp.path().join("templates");
        fs::create_dir(&folder).unwrap();
        let path = folder.join("catalog.json");
        let store = fixture(temp.path());
        for text in [
            "{broken",
            r#"{"version":2,"templates":{}}"#,
            r#"{"version":1,"templates":{"../escape":{"content":null,"nonce":"bad","deleted":false}}}"#,
            r#"{"version":1,"templates":{},"suppressedBuiltins":["../escape"]}"#,
        ] {
            fs::write(&path, text).unwrap();
            assert!(store.list().is_err());
            assert!(store.create(content("교체하지 말 것")).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
        }
    }

    #[test]
    fn invalid_content_and_paths_fail_before_creating_the_library() {
        let temp = temporary();
        let root = temp.path().join("absent");
        let store = fixture(&root);
        let mut invalid = content(" ");
        assert!(matches!(
            store.create(invalid.clone()),
            Err(TemplateError::InvalidInput)
        ));
        invalid.prompt = "x".repeat(MAX_TEMPLATE_PROMPT_BYTES + 1);
        assert!(matches!(
            store.create(invalid.clone()),
            Err(TemplateError::InvalidInput)
        ));
        invalid = content("정상 프롬프트");
        invalid.name = "제목\n다음 줄".into();
        assert!(matches!(
            store.create(invalid),
            Err(TemplateError::InvalidInput)
        ));
        assert!(matches!(
            store.get("../escape"),
            Err(TemplateError::InvalidInput)
        ));
        assert!(matches!(
            store.update("meeting", "not-a-revision", content("편집")),
            Err(TemplateError::InvalidInput)
        ));
        assert!(!root.exists());
        assert!(matches!(
            fixture(Path::new("relative")).list(),
            Err(TemplateError::UnsafePath)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn linked_roots_catalogs_and_locks_are_rejected() {
        use std::os::unix::fs::symlink;
        let temp = temporary();
        let real = temp.path().join("real");
        fs::create_dir(&real).unwrap();
        let linked = temp.path().join("linked");
        symlink(&real, &linked).unwrap();
        assert!(matches!(
            fixture(&linked).list(),
            Err(TemplateError::UnsafePath)
        ));
        assert!(matches!(
            fixture(&linked.join("missing")).create(content("편집")),
            Err(TemplateError::UnsafePath)
        ));

        let root = temp.path().join("library");
        let folder = root.join("templates");
        fs::create_dir_all(&folder).unwrap();
        let external = temp.path().join("outside.json");
        fs::write(&external, r#"{"version":1,"templates":{}}"#).unwrap();
        symlink(&external, folder.join("catalog.json")).unwrap();
        assert!(matches!(
            fixture(&root).list(),
            Err(TemplateError::UnsafePath)
        ));
        assert!(fixture(&root).create(content("편집")).is_err());
        assert_eq!(
            fs::read_to_string(&external).unwrap(),
            r#"{"version":1,"templates":{}}"#
        );

        let lock_root = temp.path().join("lock-library");
        fs::create_dir_all(lock_root.join("templates")).unwrap();
        symlink(&external, lock_root.join("templates/catalog.lock")).unwrap();
        assert!(matches!(
            fixture(&lock_root).create(content("편집")),
            Err(TemplateError::UnsafePath)
        ));

        let hard_root = temp.path().join("hard-library");
        fs::create_dir_all(hard_root.join("templates")).unwrap();
        fs::hard_link(&external, hard_root.join("templates/catalog.json")).unwrap();
        assert!(matches!(
            fixture(&hard_root).list(),
            Err(TemplateError::UnsafePath)
        ));
    }
}
