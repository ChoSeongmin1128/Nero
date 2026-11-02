use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SERVICE: &str = "io.github.choseongmin1128.nero.desktop.ai-accounts";
const INDEX: &str = "credential-state-v1";
const CHUNK_BYTES: usize = 1024;
const MAX_BYTES: usize = 512 * 1024;
const UNAVAILABLE: &str = "AI 계정 정보를 안전하게 저장하거나 읽지 못했습니다.";

pub(super) trait Credentials: Send + Sync {
    fn read(&self, account: &str) -> Result<Option<Vec<u8>>, String>;
    fn write(&self, account: &str, value: &[u8]) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}

pub(super) struct SystemCredentials;

impl Credentials for SystemCredentials {
    fn read(&self, account: &str) -> Result<Option<Vec<u8>>, String> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let entry = keyring::Entry::new(SERVICE, account).map_err(|_| UNAVAILABLE)?;
            match entry.get_secret() {
                Ok(value) => Ok(Some(value)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(_) => Err(UNAVAILABLE.into()),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = account;
            Err(UNAVAILABLE.into())
        }
    }
    fn write(&self, account: &str, value: &[u8]) -> Result<(), String> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            keyring::Entry::new(SERVICE, account)
                .map_err(|_| UNAVAILABLE)?
                .set_secret(value)
                .map_err(|_| UNAVAILABLE.into())
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (account, value);
            Err(UNAVAILABLE.into())
        }
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let entry = keyring::Entry::new(SERVICE, account).map_err(|_| UNAVAILABLE)?;
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(_) => Err(UNAVAILABLE.into()),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = account;
            Err(UNAVAILABLE.into())
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    generation: uuid::Uuid,
    chunks: usize,
    bytes: usize,
    sha256: String,
}

fn key(index: &Index, number: usize) -> String {
    format!("state-{}-{number}", index.generation)
}

fn index(store: &dyn Credentials) -> Result<Option<Index>, String> {
    let Some(bytes) = store.read(INDEX)? else {
        return Ok(None);
    };
    if bytes.len() > CHUNK_BYTES {
        return Err(UNAVAILABLE.into());
    }
    let value: Index = serde_json::from_slice(&bytes).map_err(|_| UNAVAILABLE)?;
    if value.bytes > MAX_BYTES
        || value.chunks != value.bytes.div_ceil(CHUNK_BYTES)
        || value.chunks == 0
    {
        return Err(UNAVAILABLE.into());
    }
    Ok(Some(value))
}

pub(super) fn read<T: for<'de> Deserialize<'de> + Default>(
    store: &dyn Credentials,
) -> Result<T, String> {
    let Some(index) = index(store)? else {
        return Ok(T::default());
    };
    let mut bytes = Vec::with_capacity(index.bytes);
    for number in 0..index.chunks {
        let chunk = store.read(&key(&index, number))?.ok_or(UNAVAILABLE)?;
        if chunk.len() > CHUNK_BYTES || (number + 1 < index.chunks && chunk.len() != CHUNK_BYTES) {
            return Err(UNAVAILABLE.into());
        }
        bytes.extend(chunk);
    }
    if bytes.len() != index.bytes || format!("{:x}", Sha256::digest(&bytes)) != index.sha256 {
        return Err(UNAVAILABLE.into());
    }
    serde_json::from_slice(&bytes).map_err(|_| UNAVAILABLE.into())
}

pub(super) fn write<T: Serialize>(store: &dyn Credentials, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|_| UNAVAILABLE)?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(UNAVAILABLE.into());
    }
    let old = index(store)?;
    let next = Index {
        generation: uuid::Uuid::new_v4(),
        chunks: bytes.len().div_ceil(CHUNK_BYTES),
        bytes: bytes.len(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    };
    let result = (|| {
        for (number, chunk) in bytes.chunks(CHUNK_BYTES).enumerate() {
            let name = key(&next, number);
            store.write(&name, chunk)?;
            if store.read(&name)?.as_deref() != Some(chunk) {
                return Err(UNAVAILABLE.into());
            }
        }
        let encoded = serde_json::to_vec(&next).map_err(|_| UNAVAILABLE)?;
        store.write(INDEX, &encoded)?;
        if store.read(INDEX)?.as_deref() != Some(encoded.as_slice()) {
            return Err(UNAVAILABLE.into());
        }
        Ok(())
    })();
    if result.is_ok() {
        if let Some(old) = old {
            for number in 0..old.chunks {
                let _ = store.delete(&key(&old, number));
            }
        }
    } else {
        // Do not remove chunks if the index may already point at the committed generation.
        if index(store)
            .ok()
            .flatten()
            .is_some_and(|current| current.generation != next.generation)
        {
            for number in 0..next.chunks {
                let _ = store.delete(&key(&next, number));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    #[derive(Default)]
    pub(super) struct Memory {
        values: Mutex<BTreeMap<String, Vec<u8>>>,
        fail: Mutex<bool>,
    }
    impl Credentials for Memory {
        fn read(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.values.lock().unwrap().get(name).cloned())
        }
        fn write(&self, name: &str, value: &[u8]) -> Result<(), String> {
            if *self.fail.lock().unwrap() {
                return Err("fixture denied".into());
            }
            assert!(value.len() <= CHUNK_BYTES);
            self.values
                .lock()
                .unwrap()
                .insert(name.into(), value.to_vec());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<(), String> {
            self.values.lock().unwrap().remove(name);
            Ok(())
        }
    }
    #[test]
    fn secret_chunks_fit_windows_store_and_commit_rotating_set_together() {
        let store = Memory::default();
        let original = serde_json::json!({"access":"a".repeat(7000),"refresh":"old"});
        write(&store, &original).unwrap();
        assert_eq!(read::<serde_json::Value>(&store).unwrap(), original);
        let next = serde_json::json!({"access":"b".repeat(7000),"refresh":"replacement"});
        write(&store, &next).unwrap();
        assert_eq!(read::<serde_json::Value>(&store).unwrap(), next);
        assert!(store
            .values
            .lock()
            .unwrap()
            .values()
            .all(|value| !value.windows(3).any(|slice| slice == b"old")));
    }
    #[test]
    fn failed_replacement_preserves_the_previously_committed_set() {
        let store = Memory::default();
        let original = serde_json::json!({"refresh":"original"});
        write(&store, &original).unwrap();
        *store.fail.lock().unwrap() = true;
        assert!(write(&store, &serde_json::json!({"refresh":"replacement"})).is_err());
        assert_eq!(read::<serde_json::Value>(&store).unwrap(), original);
    }
}
