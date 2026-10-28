use crate::store;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

pub const MAX_TEMPLATE_PROMPT_BYTES: usize = 64 * 1024;
const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
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
    #[error("기본 템플릿은 삭제할 수 없습니다. 기본값으로 되돌릴 수 있습니다.")]
    BuiltinProtected,
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
    pub deleted: bool,
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
    pub deleted: bool,
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
            deleted: self.deleted,
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
    deleted: bool,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Catalog {
    version: u32,
    templates: BTreeMap<String, StoredTemplate>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            version: FORMAT_VERSION,
            templates: BTreeMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct History {
    version: u32,
    template: PromptTemplate,
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
        self.list_state(false)
    }

    pub fn list_deleted(&self) -> Result<Vec<TemplateSummary>> {
        self.list_state(true)
    }

    fn list_state(&self, deleted: bool) -> Result<Vec<TemplateSummary>> {
        let builtins = self.builtins()?;
        let catalog = self.read_catalog(&builtins)?;
        let mut values = Vec::new();
        if !deleted {
            for builtin in &builtins {
                values.push(self.effective(&catalog, &builtins, &builtin.id)?.summary());
            }
        }
        let mut custom = Vec::new();
        for id in catalog.templates.keys() {
            if builtins.iter().any(|builtin| &builtin.id == id) {
                continue;
            }
            let value = self.effective(&catalog, &builtins, id)?;
            if value.deleted == deleted {
                custom.push(value.summary());
            }
        }
        custom.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        values.extend(custom);
        Ok(values)
    }

    pub fn get(&self, id: &str) -> Result<PromptTemplate> {
        let builtins = self.builtins()?;
        validate_id(id, &builtins)?;
        let value = self.effective(&self.read_catalog(&builtins)?, &builtins, id)?;
        if value.deleted {
            return Err(TemplateError::NotFound);
        }
        Ok(value)
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
                    deleted: false,
                },
            );
            Ok((self.effective(catalog, builtins, &id)?, None))
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
            if previous.deleted {
                return Err(TemplateError::NotFound);
            }
            if previous.content() == content {
                return Ok((previous, None));
            }
            catalog.templates.insert(
                id.into(),
                StoredTemplate {
                    content: Some(content),
                    nonce: Uuid::new_v4().to_string(),
                    deleted: false,
                },
            );
            Ok((self.effective(catalog, builtins, id)?, Some(previous)))
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
                return Ok((previous, None));
            }
            catalog.templates.insert(
                id.into(),
                StoredTemplate {
                    content: None,
                    nonce: Uuid::new_v4().to_string(),
                    deleted: false,
                },
            );
            Ok((self.effective(catalog, builtins, id)?, Some(previous)))
        })
    }

    pub fn delete(&self, id: &str, expected_revision: &str) -> Result<TemplateSummary> {
        validate_revision(expected_revision)?;
        validate_id(id, &self.builtins()?)?;
        self.mutate(|catalog, builtins| {
            validate_id(id, builtins)?;
            let previous = self.effective(catalog, builtins, id)?;
            require_revision(&previous, expected_revision)?;
            if previous.built_in {
                return Err(TemplateError::BuiltinProtected);
            }
            if previous.deleted {
                return Ok((previous.summary(), None));
            }
            let stored = catalog
                .templates
                .get_mut(id)
                .ok_or(TemplateError::NotFound)?;
            stored.deleted = true;
            stored.nonce = Uuid::new_v4().to_string();
            Ok((
                self.effective(catalog, builtins, id)?.summary(),
                Some(previous),
            ))
        })
    }

    pub fn restore(&self, id: &str, expected_revision: &str) -> Result<PromptTemplate> {
        validate_revision(expected_revision)?;
        validate_id(id, &self.builtins()?)?;
        self.mutate(|catalog, builtins| {
            validate_id(id, builtins)?;
            let previous = self.effective(catalog, builtins, id)?;
            require_revision(&previous, expected_revision)?;
            if previous.built_in {
                return Err(TemplateError::InvalidInput);
            }
            if !previous.deleted {
                return Ok((previous, None));
            }
            let stored = catalog
                .templates
                .get_mut(id)
                .ok_or(TemplateError::NotFound)?;
            stored.deleted = false;
            stored.nonce = Uuid::new_v4().to_string();
            Ok((self.effective(catalog, builtins, id)?, Some(previous)))
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
            if builtin.id.is_empty()
                || builtin.id.len() > 64
                || !builtin
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                || Uuid::parse_str(&builtin.id).is_ok()
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
        for (id, value) in &catalog.templates {
            validate_id(id, builtins).map_err(|_| TemplateError::InvalidDocument)?;
            if !valid_uuid(&value.nonce)
                || value
                    .content
                    .as_ref()
                    .is_some_and(|content| validate_content(content).is_err())
                || value.content.is_none() && !builtins.iter().any(|builtin| &builtin.id == id)
                || value.deleted && builtins.iter().any(|builtin| &builtin.id == id)
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
        let stored = catalog.templates.get(id);
        let base = builtin.map(Builtin::content);
        let content = stored
            .and_then(|value| value.content.clone())
            .or_else(|| base.clone())
            .ok_or(TemplateError::NotFound)?;
        let default_revision = base.as_ref().map(|content| digest(&(id, content)));
        let deleted = stored.is_some_and(|value| value.deleted);
        let revision = digest(&(
            id,
            &content,
            &default_revision,
            deleted,
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
            deleted,
        })
    }

    fn mutate<T>(
        &self,
        action: impl FnOnce(&mut Catalog, &[Builtin]) -> Result<(T, Option<PromptTemplate>)>,
    ) -> Result<T> {
        let builtins = self.builtins()?;
        let _lock = self.lock()?;
        let original = self.read_catalog(&builtins)?;
        let mut catalog = original.clone();
        let (result, previous) = action(&mut catalog, &builtins)?;
        if catalog != original {
            let bytes = serde_json::to_vec(&catalog).map_err(|_| TemplateError::InvalidDocument)?;
            if bytes.len() + 1 > MAX_DOCUMENT_BYTES {
                return Err(TemplateError::InvalidInput);
            }
            if let Some(previous) = previous {
                self.archive(previous)?;
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

    fn archive(&self, template: PromptTemplate) -> Result<()> {
        let folder = self.root.join("templates/history");
        validate_directory(&folder)?;
        store::ensure_directory(&folder)?;
        let relative =
            Path::new("templates/history").join(format!("{}.json", &template.revision[7..]));
        let path = self.root.join(&relative);
        if let Some(bytes) = read_optional(&path)? {
            let previous: History =
                serde_json::from_slice(&bytes).map_err(|_| TemplateError::InvalidDocument)?;
            if previous.version != FORMAT_VERSION || previous.template != template {
                return Err(TemplateError::InvalidDocument);
            }
            return Ok(());
        }
        let mut file = store::create_new(&path)?;
        serde_json::to_writer(
            &mut file,
            &History {
                version: FORMAT_VERSION,
                template,
            },
        )
        .map_err(|_| TemplateError::Storage)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        #[cfg(unix)]
        File::open(&folder)?.sync_all()?;
        Ok(())
    }
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

    fn history(root: &Path, revision: &str) -> PromptTemplate {
        let bytes = fs::read(
            root.join("templates/history")
                .join(format!("{}.json", &revision[7..])),
        )
        .unwrap();
        serde_json::from_slice::<History>(&bytes).unwrap().template
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
        assert_eq!(
            store.get("meeting").unwrap().prompt,
            "사실과 결정을 구분합니다."
        );
        assert!(store.list_deleted().unwrap().is_empty());
        assert!(!root.exists());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    #[test]
    fn custom_changes_share_cas_and_delete_can_be_restored_without_losing_content() {
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
        assert_eq!(history(&root, &created.revision), created);
        let before = fs::read(root.join("templates/catalog.json")).unwrap();
        assert!(matches!(
            first.update(&created.id, &created.revision, content("오래된 편집")),
            Err(TemplateError::Conflict)
        ));
        assert_eq!(
            before,
            fs::read(root.join("templates/catalog.json")).unwrap()
        );
        let deleted = first.delete(&edited.id, &edited.revision).unwrap();
        assert!(deleted.deleted);
        assert_ne!(deleted.revision, edited.revision);
        assert!(matches!(
            second.get(&edited.id),
            Err(TemplateError::NotFound)
        ));
        assert_eq!(second.list().unwrap().len(), 2);
        assert_eq!(second.list_deleted().unwrap(), vec![deleted.clone()]);
        assert_eq!(history(&root, &edited.revision), edited);
        assert!(matches!(
            second.restore(&created.id, &edited.revision),
            Err(TemplateError::Conflict)
        ));
        let restored = second.restore(&created.id, &deleted.revision).unwrap();
        assert!(!restored.deleted);
        assert_eq!(restored.prompt, "  수정한 사용자 프롬프트\n");
        assert_ne!(restored.revision, edited.revision);
        assert!(first.list_deleted().unwrap().is_empty());
        assert!(history(&root, &deleted.revision).deleted);
        assert_eq!(first.list().unwrap().len(), 3);
        assert!(!root.join("store.lock").exists());
        assert!(!root.join("meetings").exists());
        assert!(!root.join("models").exists());
    }

    #[test]
    fn builtin_reset_preserves_the_edit_and_prevents_aba_overwrites() {
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
        assert!(matches!(
            store.delete("meeting", &edited.revision),
            Err(TemplateError::BuiltinProtected)
        ));
        let reset = store.reset("meeting", &edited.revision).unwrap();
        assert_eq!(reset.content(), original.content());
        assert!(!reset.customized);
        assert_ne!(reset.revision, original.revision);
        assert_ne!(reset.revision, edited.revision);
        assert_eq!(history(temp.path(), &edited.revision), edited);
        assert!(matches!(
            store.update("meeting", &original.revision, content("오래된 편집")),
            Err(TemplateError::Conflict)
        ));
        assert_eq!(store.reset("meeting", &reset.revision).unwrap(), reset);
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
        assert_eq!(after_upgrade.prompt, changed.prompt);
        assert_ne!(after_upgrade.revision, changed.revision);
        let reset = next.reset("meeting", &after_upgrade.revision).unwrap();
        assert_eq!(reset.prompt, "새 기본값");
        let third = fixture_with_default(temp.path(), "세 번째 기본값");
        assert_eq!(third.get("meeting").unwrap().prompt, "세 번째 기본값");
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
    fn linked_roots_catalogs_locks_and_history_are_rejected() {
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

        let store = fixture(&real);
        let initial = store.create(content("초기 입력")).unwrap();
        symlink(temp.path(), real.join("templates/history")).unwrap();
        assert!(matches!(
            store.update(&initial.id, &initial.revision, content("변경")),
            Err(TemplateError::UnsafePath)
        ));
        assert_eq!(store.get(&initial.id).unwrap(), initial);

        let hard_root = temp.path().join("hard-library");
        fs::create_dir_all(hard_root.join("templates")).unwrap();
        fs::hard_link(&external, hard_root.join("templates/catalog.json")).unwrap();
        assert!(matches!(
            fixture(&hard_root).list(),
            Err(TemplateError::UnsafePath)
        ));
    }
}
