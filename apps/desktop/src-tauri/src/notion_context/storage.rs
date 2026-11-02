use super::{oauth::Credential, Connection, Error, NotionSource, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub(super) trait CredentialStore: Send + Sync {
    fn load(&self, slot: bool) -> Result<Credential>;
    fn save(&self, slot: bool, credential: &Credential) -> Result<()>;
    fn delete(&self) -> Result<()>;
}
pub(super) struct SecureCredentials;
fn account(slot: bool) -> &'static str {
    if slot {
        "notion-context-alternate"
    } else {
        "notion-context"
    }
}

impl CredentialStore for SecureCredentials {
    fn load(&self, slot: bool) -> Result<Credential> {
        let value = super::vault::read(&super::vault::SystemVault, account(slot))?;
        serde_json::from_slice(&value).map_err(|_| Error::Credentials)
    }
    fn save(&self, slot: bool, credential: &Credential) -> Result<()> {
        let value = serde_json::to_vec(credential).map_err(|_| Error::Credentials)?;
        super::vault::write(&super::vault::SystemVault, account(slot), &value)
    }
    fn delete(&self) -> Result<()> {
        for slot in [false, true] {
            super::vault::delete(&super::vault::SystemVault, account(slot))?;
        }
        Ok(())
    }
}

pub(super) struct Storage {
    directory: PathBuf,
}
impl Storage {
    pub fn new(root: PathBuf) -> Result<Self> {
        if !root.is_absolute()
            || root
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
        {
            return Err(Error::Storage);
        }
        let directory = root.join("notion-context");
        ensure_safe_ancestors(&directory)?;
        Ok(Self { directory })
    }
    pub fn connection(&self) -> Result<Connection> {
        let path = self.directory.join("connection.json");
        if !path.try_exists().map_err(|_| Error::Storage)? {
            return Ok(Connection::default());
        }
        let value: Connection = read_json(&path, 32 * 1024)?;
        if value.schema_version != 1 {
            return Err(Error::Storage);
        }
        Ok(value)
    }
    pub fn save_connection(&self, connection: &Connection) -> Result<()> {
        write_json(&self.directory.join("connection.json"), connection)
    }
    pub fn save_source(&self, source: &NotionSource) -> Result<()> {
        source.validate()?;
        let path = self.source_path(&source.source_id)?;
        if path.try_exists().map_err(|_| Error::Storage)? {
            let existing: NotionSource = read_json(&path, super::MAX_PAGE_BYTES * 6 + 32 * 1024)?;
            if &existing != source {
                return Err(Error::Storage);
            }
            return Ok(());
        }
        write_json(&path, source)
    }
    pub fn source(&self, source_id: &str) -> Result<NotionSource> {
        let source: NotionSource = read_json(
            &self.source_path(source_id)?,
            super::MAX_PAGE_BYTES * 6 + 32 * 1024,
        )?;
        source.validate()?;
        if source.source_id != source_id {
            return Err(Error::Storage);
        }
        Ok(source)
    }
    fn source_path(&self, source_id: &str) -> Result<PathBuf> {
        let parts = source_id.split(':').collect::<Vec<_>>();
        if parts.len() != 3
            || parts[0] != "notion"
            || super::normalize_page_id(parts[1])? != parts[1]
            || parts[2].len() != 64
            || !parts[2]
                .bytes()
                .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
        {
            return Err(Error::InvalidLink);
        }
        Ok(self
            .directory
            .join("sources")
            .join(format!("{}-{}.json", parts[1], parts[2])))
    }
}

fn ensure_safe_ancestors(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Err(Error::Storage),
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(Error::Storage),
        }
    }
    Ok(())
}
fn read_json<T: DeserializeOwned>(path: &Path, maximum: usize) -> Result<T> {
    ensure_safe_ancestors(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| Error::Storage)?;
    if !metadata.is_file() || metadata.len() > maximum as u64 {
        return Err(Error::Storage);
    }
    let mut bytes = vec![];
    fs::File::open(path)
        .map_err(|_| Error::Storage)?
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Storage)?;
    if bytes.len() > maximum {
        return Err(Error::Budget);
    }
    serde_json::from_slice(&bytes).map_err(|_| Error::Storage)
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    ensure_safe_ancestors(path)?;
    let directory = path.parent().ok_or(Error::Storage)?;
    fs::create_dir_all(directory).map_err(|_| Error::Storage)?;
    ensure_safe_ancestors(path)?;
    let mut file = tempfile::NamedTempFile::new_in(directory).map_err(|_| Error::Storage)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| Error::Storage)?;
    }
    serde_json::to_writer(&mut file, value).map_err(|_| Error::Storage)?;
    file.flush().map_err(|_| Error::Storage)?;
    file.as_file().sync_all().map_err(|_| Error::Storage)?;
    file.persist(path).map_err(|_| Error::Storage)?;
    Ok(())
}
