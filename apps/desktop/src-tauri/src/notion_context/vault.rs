use super::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SERVICE: &str = "io.github.choseongmin1128.nero.desktop.notion-context";
const CHUNK_BYTES: usize = 1024;
const MAX_BYTES: usize = 512 * 1024;
pub(super) trait Vault {
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>>;
    fn write(&self, name: &str, value: &[u8]) -> Result<()>;
    fn delete(&self, name: &str) -> Result<()>;
    fn legacy(&self, name: &str) -> Result<Option<Vec<u8>>> {
        self.read(name)
    }
}
pub(super) struct SystemVault;
impl Vault for SystemVault {
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            match keyring::Entry::new(SERVICE, name)
                .map_err(|_| Error::Credentials)?
                .get_secret()
            {
                Ok(value) => Ok(Some(value)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(_) => Err(Error::Credentials),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = name;
            Err(Error::Credentials)
        }
    }
    fn write(&self, name: &str, value: &[u8]) -> Result<()> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            keyring::Entry::new(SERVICE, name)
                .map_err(|_| Error::Credentials)?
                .set_secret(value)
                .map_err(|_| Error::Credentials)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (name, value);
            Err(Error::Credentials)
        }
    }
    fn delete(&self, name: &str) -> Result<()> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            match keyring::Entry::new(SERVICE, name)
                .map_err(|_| Error::Credentials)?
                .delete_credential()
            {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(_) => Err(Error::Credentials),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = name;
            Err(Error::Credentials)
        }
    }
    fn legacy(&self, name: &str) -> Result<Option<Vec<u8>>> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            match keyring::Entry::new(SERVICE, name)
                .map_err(|_| Error::Credentials)?
                .get_password()
            {
                Ok(value) => Ok(Some(value.into_bytes())),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(_) => Err(Error::Credentials),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = name;
            Err(Error::Credentials)
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    generation: uuid::Uuid,
    chunks: usize,
    bytes: usize,
    sha256: String,
}
fn index_name(account: &str) -> String {
    format!("{account}-chunks-v1")
}
fn chunk_name(account: &str, index: &Index, number: usize) -> String {
    format!("{account}-{}-{number}", index.generation)
}
fn read_index(store: &dyn Vault, account: &str) -> Result<Option<Index>> {
    let Some(bytes) = store.read(&index_name(account))? else {
        return Ok(None);
    };
    if bytes.len() > CHUNK_BYTES {
        return Err(Error::Credentials);
    }
    let index: Index = serde_json::from_slice(&bytes).map_err(|_| Error::Credentials)?;
    if index.bytes == 0
        || index.bytes > MAX_BYTES
        || index.chunks != index.bytes.div_ceil(CHUNK_BYTES)
        || index.sha256.len() != 64
    {
        return Err(Error::Credentials);
    }
    Ok(Some(index))
}
pub(super) fn read(store: &dyn Vault, account: &str) -> Result<Vec<u8>> {
    for _ in 0..3 {
        let Some(index) = read_index(store, account)? else {
            let value = store.legacy(account)?.ok_or(Error::Credentials)?;
            return if value.len() <= MAX_BYTES {
                Ok(value)
            } else {
                Err(Error::Credentials)
            };
        };
        let result = (|| {
            let mut bytes = Vec::with_capacity(index.bytes);
            for number in 0..index.chunks {
                let chunk = store
                    .read(&chunk_name(account, &index, number))?
                    .ok_or(Error::Credentials)?;
                if chunk.len() > CHUNK_BYTES
                    || (number + 1 < index.chunks && chunk.len() != CHUNK_BYTES)
                {
                    return Err(Error::Credentials);
                }
                bytes.extend(chunk);
            }
            if bytes.len() != index.bytes || format!("{:x}", Sha256::digest(&bytes)) != index.sha256
            {
                return Err(Error::Credentials);
            }
            Ok(bytes)
        })();
        if result.is_ok() {
            return result;
        }
        if read_index(store, account)?
            .as_ref()
            .map(|value| value.generation)
            == Some(index.generation)
        {
            return result;
        }
    }
    Err(Error::Credentials)
}
pub(super) fn write(store: &dyn Vault, account: &str, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(Error::Credentials);
    }
    let old = read_index(store, account)?;
    let next = Index {
        generation: uuid::Uuid::new_v4(),
        chunks: bytes.len().div_ceil(CHUNK_BYTES),
        bytes: bytes.len(),
        sha256: format!("{:x}", Sha256::digest(bytes)),
    };
    let result = (|| {
        for (number, value) in bytes.chunks(CHUNK_BYTES).enumerate() {
            let name = chunk_name(account, &next, number);
            store.write(&name, value)?;
            if store.read(&name)?.as_deref() != Some(value) {
                return Err(Error::Credentials);
            }
        }
        let encoded = serde_json::to_vec(&next).map_err(|_| Error::Credentials)?;
        store.write(&index_name(account), &encoded)?;
        if store.read(&index_name(account))?.as_deref() != Some(encoded.as_slice()) {
            return Err(Error::Credentials);
        }
        Ok(())
    })();
    if result.is_ok() {
        if let Some(old) = old {
            for number in 0..old.chunks {
                let _ = store.delete(&chunk_name(account, &old, number));
            }
        }
        let _ = store.delete(account);
    } else if read_index(store, account).ok().is_some_and(|index| {
        index
            .as_ref()
            .is_none_or(|index| index.generation != next.generation)
    }) {
        for number in 0..next.chunks {
            let _ = store.delete(&chunk_name(account, &next, number));
        }
    }
    result
}
pub(super) fn delete(store: &dyn Vault, account: &str) -> Result<()> {
    if let Some(index) = read_index(store, account)? {
        for number in 0..index.chunks {
            store.delete(&chunk_name(account, &index, number))?;
        }
        store.delete(&index_name(account))?;
    }
    store.delete(account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, sync::Mutex};
    #[derive(Default)]
    struct Memory {
        values: Mutex<BTreeMap<String, Vec<u8>>>,
        writes_left: Mutex<Option<usize>>,
    }
    impl Vault for Memory {
        fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
            Ok(self.values.lock().unwrap().get(name).cloned())
        }
        fn write(&self, name: &str, value: &[u8]) -> Result<()> {
            assert!(value.len() <= CHUNK_BYTES);
            if let Some(left) = self.writes_left.lock().unwrap().as_mut() {
                if *left == 0 {
                    return Err(Error::Credentials);
                }
                *left -= 1;
            }
            self.values
                .lock()
                .unwrap()
                .insert(name.into(), value.into());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<()> {
            self.values.lock().unwrap().remove(name);
            Ok(())
        }
    }
    #[test]
    fn credential_chunks_fit_windows_and_publish_the_whole_rotated_object() {
        let store = Memory::default();
        let first = serde_json::json!({"access":"old-access".repeat(1000),"refresh":"old-refresh"})
            .to_string();
        write(&store, "notion-context", first.as_bytes()).unwrap();
        let next = serde_json::json!({"access":"new-access".repeat(1000),"refresh":"new-refresh"})
            .to_string();
        write(&store, "notion-context", next.as_bytes()).unwrap();
        assert_eq!(read(&store, "notion-context").unwrap(), next.as_bytes());
        assert!(store
            .values
            .lock()
            .unwrap()
            .values()
            .all(|value| value.len() <= CHUNK_BYTES));
        delete(&store, "notion-context").unwrap();
        assert!(store.values.lock().unwrap().is_empty());
    }
    #[test]
    fn interrupted_generation_preserves_the_committed_grant_and_own_legacy_entry() {
        let store = Memory::default();
        write(&store, "notion-context", b"original").unwrap();
        *store.writes_left.lock().unwrap() = Some(1);
        assert!(write(&store, "notion-context", &vec![b'x'; 4096]).is_err());
        assert_eq!(read(&store, "notion-context").unwrap(), b"original");
        let legacy = Memory::default();
        legacy
            .values
            .lock()
            .unwrap()
            .insert("notion-context".into(), b"legacy-nero-only".to_vec());
        assert_eq!(
            read(&legacy, "notion-context").unwrap(),
            b"legacy-nero-only"
        );
        write(&legacy, "notion-context", b"replacement").unwrap();
        assert!(legacy.read("notion-context").unwrap().is_none());
    }
}
