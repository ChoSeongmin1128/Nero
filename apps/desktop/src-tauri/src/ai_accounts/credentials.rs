use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SERVICE: &str = nero_core::environment::CURRENT.ai_credentials_service;
const INDEX: &str = "credential-state-v1";
const SINGLE: &str = "credential-state-v2";
const CHUNK_BYTES: usize = 1024;
const MAX_BYTES: usize = 512 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Error {
    Unavailable,
    InteractionRequired,
    LegacyInteractionRequired,
    Corrupt,
}
impl Error {
    pub fn message(self) -> String {
        match self {
            Self::LegacyInteractionRequired => {
                "기존 AI 계정에 접근할 수 없습니다. 기존 계정 정보는 보존됩니다."
            }
            _ => "AI 계정 정보를 안전하게 저장하거나 읽지 못했습니다.",
        }
        .into()
    }
}
impl From<crate::secret_store::Error> for Error {
    fn from(error: crate::secret_store::Error) -> Self {
        match error {
            crate::secret_store::Error::InteractionRequired => Self::InteractionRequired,
            crate::secret_store::Error::Unavailable => Self::Unavailable,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Format {
    Chunked,
    Single,
}
pub(super) trait Credentials: Send + Sync {
    fn format(&self) -> Format {
        Format::Chunked
    }
    fn read_interactive(&self, account: &str) -> Result<Option<Vec<u8>>, Error> {
        self.read(account)
    }
    fn read(&self, account: &str) -> Result<Option<Vec<u8>>, Error>;
    fn write(&self, account: &str, value: &[u8]) -> Result<(), Error>;
    fn delete(&self, account: &str) -> Result<(), Error>;
}

pub(super) struct SystemCredentials;

impl Credentials for SystemCredentials {
    fn format(&self) -> Format {
        if cfg!(target_os = "macos") {
            Format::Single
        } else {
            Format::Chunked
        }
    }
    fn read(&self, account: &str) -> Result<Option<Vec<u8>>, Error> {
        crate::secret_store::read(SERVICE, account, crate::secret_store::Access::Silent)
            .map_err(Error::from)
    }
    fn read_interactive(&self, account: &str) -> Result<Option<Vec<u8>>, Error> {
        if self.format() != Format::Single || account != SINGLE {
            return Err(Error::Unavailable);
        }
        crate::secret_store::read(SERVICE, account, crate::secret_store::Access::Interactive)
            .map_err(Error::from)
    }
    fn write(&self, account: &str, value: &[u8]) -> Result<(), Error> {
        crate::secret_store::write(SERVICE, account, value, crate::secret_store::Access::Silent)
            .map_err(Error::from)
    }
    fn delete(&self, account: &str) -> Result<(), Error> {
        crate::secret_store::delete(SERVICE, account, crate::secret_store::Access::Silent)
            .map_err(Error::from)
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

fn index(store: &dyn Credentials) -> Result<Option<Index>, Error> {
    let Some(bytes) = store.read(INDEX)? else {
        return Ok(None);
    };
    if bytes.len() > CHUNK_BYTES {
        return Err(Error::Corrupt);
    }
    let value: Index = serde_json::from_slice(&bytes).map_err(|_| Error::Corrupt)?;
    if value.bytes > MAX_BYTES
        || value.chunks != value.bytes.div_ceil(CHUNK_BYTES)
        || value.chunks == 0
    {
        return Err(Error::Corrupt);
    }
    Ok(Some(value))
}

pub(super) fn read<T: for<'de> Deserialize<'de> + Default + Serialize>(
    store: &dyn Credentials,
) -> Result<T, Error> {
    read_with_access(store, false)
}
pub(super) fn read_interactive<T: for<'de> Deserialize<'de> + Default + Serialize>(
    store: &dyn Credentials,
) -> Result<T, Error> {
    read_with_access(store, true)
}
fn read_with_access<T: for<'de> Deserialize<'de> + Default + Serialize>(
    store: &dyn Credentials,
    interactive: bool,
) -> Result<T, Error> {
    if store.format() == Format::Chunked {
        return read_chunked(store);
    }
    let current = if interactive {
        store.read_interactive(SINGLE)?
    } else {
        store.read(SINGLE)?
    };
    if let Some(bytes) = current {
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err(Error::Corrupt);
        }
        return serde_json::from_slice(&bytes).map_err(|_| Error::Corrupt);
    }
    // A denied legacy read must not be treated as an empty account collection.
    let legacy: T = read_chunked(store).map_err(|error| match error {
        Error::InteractionRequired => Error::LegacyInteractionRequired,
        other => other,
    })?;
    // Keep legacy entries intact; publish v2 only after the entire legacy state validates.
    let bytes = serde_json::to_vec(&legacy).map_err(|_| Error::Corrupt)?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(Error::Corrupt);
    }
    store.write(SINGLE, &bytes)?;
    Ok(legacy)
}
fn read_chunked<T: for<'de> Deserialize<'de> + Default>(
    store: &dyn Credentials,
) -> Result<T, Error> {
    let Some(index) = index(store)? else {
        return Ok(T::default());
    };
    let mut bytes = Vec::with_capacity(index.bytes);
    for number in 0..index.chunks {
        let chunk = store.read(&key(&index, number))?.ok_or(Error::Corrupt)?;
        if chunk.len() > CHUNK_BYTES || (number + 1 < index.chunks && chunk.len() != CHUNK_BYTES) {
            return Err(Error::Corrupt);
        }
        bytes.extend(chunk);
    }
    if bytes.len() != index.bytes || format!("{:x}", Sha256::digest(&bytes)) != index.sha256 {
        return Err(Error::Corrupt);
    }
    serde_json::from_slice(&bytes).map_err(|_| Error::Corrupt)
}

pub(super) fn write<T: Serialize>(store: &dyn Credentials, value: &T) -> Result<(), Error> {
    let bytes = serde_json::to_vec(value).map_err(|_| Error::Corrupt)?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(Error::Corrupt);
    }
    if store.format() == Format::Single {
        return store.write(SINGLE, &bytes);
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
                return Err(Error::Corrupt);
            }
        }
        let encoded = serde_json::to_vec(&next).map_err(|_| Error::Corrupt)?;
        store.write(INDEX, &encoded)?;
        if store.read(INDEX)?.as_deref() != Some(encoded.as_slice()) {
            return Err(Error::Corrupt);
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
        fn read(&self, name: &str) -> Result<Option<Vec<u8>>, Error> {
            Ok(self.values.lock().unwrap().get(name).cloned())
        }
        fn write(&self, name: &str, value: &[u8]) -> Result<(), Error> {
            if *self.fail.lock().unwrap() {
                return Err(Error::InteractionRequired);
            }
            assert!(value.len() <= CHUNK_BYTES);
            self.values
                .lock()
                .unwrap()
                .insert(name.into(), value.to_vec());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<(), Error> {
            self.values.lock().unwrap().remove(name);
            Ok(())
        }
    }
    struct SingleMemory {
        inner: Memory,
        deny_legacy: Mutex<bool>,
        calls: Mutex<Vec<String>>,
    }
    impl SingleMemory {
        fn new() -> Self {
            Self {
                inner: Memory::default(),
                deny_legacy: Mutex::new(false),
                calls: Mutex::new(Vec::new()),
            }
        }
    }
    impl Credentials for SingleMemory {
        fn format(&self) -> Format {
            Format::Single
        }
        fn read(&self, name: &str) -> Result<Option<Vec<u8>>, Error> {
            self.calls.lock().unwrap().push(format!("silent:{name}"));
            if name != SINGLE && *self.deny_legacy.lock().unwrap() {
                return Err(Error::InteractionRequired);
            }
            self.inner.read(name)
        }
        fn read_interactive(&self, name: &str) -> Result<Option<Vec<u8>>, Error> {
            assert_eq!(name, SINGLE);
            self.calls
                .lock()
                .unwrap()
                .push(format!("interactive:{name}"));
            self.inner.read(name)
        }
        fn write(&self, name: &str, value: &[u8]) -> Result<(), Error> {
            self.calls.lock().unwrap().push(format!("write:{name}"));
            if *self.inner.fail.lock().unwrap() {
                return Err(Error::InteractionRequired);
            }
            self.inner
                .values
                .lock()
                .unwrap()
                .insert(name.into(), value.into());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<(), Error> {
            self.calls.lock().unwrap().push(format!("delete:{name}"));
            self.inner.delete(name)
        }
    }
    #[test]
    fn single_item_rotates_without_touching_legacy_chunks() {
        let store = SingleMemory::new();
        let old = serde_json::json!({"token":"old"});
        write(&store.inner, &old).unwrap();
        let legacy = store.inner.values.lock().unwrap().clone();
        assert_eq!(read::<serde_json::Value>(&store).unwrap(), old);
        for (name, value) in &legacy {
            assert_eq!(store.inner.values.lock().unwrap().get(name), Some(value));
        }
        store.calls.lock().unwrap().clear();
        let next = serde_json::json!({"token":"new".repeat(2000)});
        write(&store, &next).unwrap();
        assert_eq!(read::<serde_json::Value>(&store).unwrap(), next);
        assert_eq!(
            *store.calls.lock().unwrap(),
            vec![format!("write:{SINGLE}"), format!("silent:{SINGLE}")]
        );
        for (name, value) in &legacy {
            assert_eq!(store.inner.values.lock().unwrap().get(name), Some(value));
        }
    }
    #[test]
    fn denied_legacy_is_never_retried_interactively_or_treated_as_empty() {
        let store = SingleMemory::new();
        *store.deny_legacy.lock().unwrap() = true;
        assert_eq!(
            read_interactive::<serde_json::Value>(&store).unwrap_err(),
            Error::LegacyInteractionRequired
        );
        assert_eq!(
            *store.calls.lock().unwrap(),
            vec![format!("interactive:{SINGLE}"), format!("silent:{INDEX}")]
        );
        assert!(store.inner.values.lock().unwrap().is_empty());
    }
    #[test]
    fn corrupt_legacy_and_unavailable_legacy_are_not_interaction_denials() {
        let store = SingleMemory::new();
        store
            .inner
            .values
            .lock()
            .unwrap()
            .insert(INDEX.into(), b"not-json".to_vec());
        assert_eq!(
            read::<serde_json::Value>(&store).unwrap_err(),
            Error::Corrupt
        );
        assert!(!store.inner.values.lock().unwrap().contains_key(SINGLE));
        struct Unavailable;
        impl Credentials for Unavailable {
            fn format(&self) -> Format {
                Format::Single
            }
            fn read(&self, name: &str) -> Result<Option<Vec<u8>>, Error> {
                if name == SINGLE {
                    Ok(None)
                } else {
                    Err(Error::Unavailable)
                }
            }
            fn write(&self, _: &str, _: &[u8]) -> Result<(), Error> {
                unreachable!()
            }
            fn delete(&self, _: &str) -> Result<(), Error> {
                unreachable!()
            }
        }
        assert_eq!(
            read::<serde_json::Value>(&Unavailable).unwrap_err(),
            Error::Unavailable
        );
    }
    #[test]
    fn single_item_failed_write_keeps_old_value() {
        let store = SingleMemory::new();
        let original = serde_json::json!({"refresh":"old"});
        write(&store, &original).unwrap();
        *store.inner.fail.lock().unwrap() = true;
        assert!(write(&store, &serde_json::json!({"refresh":"new"})).is_err());
        assert_eq!(read::<serde_json::Value>(&store).unwrap(), original);
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
