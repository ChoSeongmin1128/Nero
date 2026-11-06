use super::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SERVICE: &str = nero_core::environment::CURRENT.notion_credentials_service;
const CHUNK_BYTES: usize = 1024;
const MAX_BYTES: usize = 512 * 1024;
pub(super) trait Vault {
    fn single_record(&self) -> bool {
        false
    }
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>>;
    fn write(&self, name: &str, value: &[u8]) -> Result<()>;
    fn delete(&self, name: &str) -> Result<()>;
    fn legacy(&self, name: &str) -> Result<Option<Vec<u8>>> {
        self.read(name)
    }
}
pub(super) struct SystemVault;
impl Vault for SystemVault {
    fn single_record(&self) -> bool {
        cfg!(target_os = "macos")
    }
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        crate::secret_store::read(SERVICE, name, crate::secret_store::Access::Silent)
            .map_err(|_| Error::Credentials)
    }
    fn write(&self, name: &str, value: &[u8]) -> Result<()> {
        crate::secret_store::write(SERVICE, name, value, crate::secret_store::Access::Silent)
            .map_err(|_| Error::Credentials)
    }
    fn delete(&self, name: &str) -> Result<()> {
        crate::secret_store::delete(SERVICE, name, crate::secret_store::Access::Silent)
            .map_err(|_| Error::Credentials)
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
fn single_name(account: &str) -> String {
    format!("{account}-single-v2")
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
    if store.single_record() {
        if let Some(bytes) = store.read(&single_name(account))? {
            if bytes.is_empty() || bytes.len() > MAX_BYTES {
                return Err(Error::Credentials);
            }
            return Ok(bytes);
        }
        let bytes = read_chunked(store, account)?;
        store.write(&single_name(account), &bytes)?;
        return Ok(bytes);
    }
    read_chunked(store, account)
}
fn read_chunked(store: &dyn Vault, account: &str) -> Result<Vec<u8>> {
    for _ in 0..3 {
        let Some(index) = read_index(store, account)? else {
            let value = store.legacy(account)?.ok_or(Error::Credentials)?;
            return if value.len() <= MAX_BYTES {
                Ok(value)
            } else {
                Err(Error::Credentials)
            };
        };
        let mut bytes = Vec::with_capacity(index.bytes);
        let mut valid = true;
        for number in 0..index.chunks {
            let Some(chunk) = store.read(&chunk_name(account, &index, number))? else {
                valid = false;
                break;
            };
            if chunk.len() > CHUNK_BYTES
                || (number + 1 < index.chunks && chunk.len() != CHUNK_BYTES)
            {
                valid = false;
                break;
            }
            bytes.extend(chunk);
        }
        let result = if valid
            && bytes.len() == index.bytes
            && format!("{:x}", Sha256::digest(&bytes)) == index.sha256
        {
            Ok(bytes)
        } else {
            Err(Error::Credentials)
        };
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
    if store.single_record() {
        return store.write(&single_name(account), bytes);
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
    store.delete(account)?;
    if store.single_record() {
        store.delete(&single_name(account))?;
    }
    Ok(())
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
    fn denied_chunk_read_does_not_reopen_the_index() {
        struct Denied<'a> {
            inner: &'a Memory,
            reads: Mutex<Vec<String>>,
        }
        impl Vault for Denied<'_> {
            fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
                self.reads.lock().unwrap().push(name.to_owned());
                if !name.ends_with("-chunks-v1") {
                    return Err(Error::Credentials);
                }
                self.inner.read(name)
            }
            fn write(&self, _: &str, _: &[u8]) -> Result<()> {
                unreachable!()
            }
            fn delete(&self, _: &str) -> Result<()> {
                unreachable!()
            }
        }
        let inner = Memory::default();
        write(&inner, "notion-context", b"saved").unwrap();
        let store = Denied {
            inner: &inner,
            reads: Mutex::new(Vec::new()),
        };
        assert!(read(&store, "notion-context").is_err());
        let reads = store.reads.lock().unwrap();
        assert_eq!(reads.len(), 2);
        assert!(reads[0].ends_with("-chunks-v1"));
        assert_eq!(read(&inner, "notion-context").unwrap(), b"saved");
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
    #[derive(Default)]
    struct SingleMemory {
        values: Mutex<BTreeMap<String, Vec<u8>>>,
        reads: Mutex<Vec<String>>,
        writes: Mutex<Vec<String>>,
        deny_read: Mutex<Option<String>>,
        deny_write: Mutex<bool>,
        deny_delete: Mutex<Option<String>>,
    }
    impl Vault for SingleMemory {
        fn single_record(&self) -> bool {
            true
        }
        fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
            self.reads.lock().unwrap().push(name.to_owned());
            if self.deny_read.lock().unwrap().as_deref() == Some(name) {
                return Err(Error::Credentials);
            }
            Ok(self.values.lock().unwrap().get(name).cloned())
        }
        fn write(&self, name: &str, value: &[u8]) -> Result<()> {
            self.writes.lock().unwrap().push(name.to_owned());
            if *self.deny_write.lock().unwrap() {
                return Err(Error::Credentials);
            }
            self.values
                .lock()
                .unwrap()
                .insert(name.to_owned(), value.to_vec());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<()> {
            if self.deny_delete.lock().unwrap().as_deref() == Some(name) {
                return Err(Error::Credentials);
            }
            self.values.lock().unwrap().remove(name);
            Ok(())
        }
    }

    #[test]
    fn single_large_record_requires_one_write_and_one_read() {
        let store = SingleMemory::default();
        let value = vec![b'x'; CHUNK_BYTES * 8];
        write(&store, "notion", &value).unwrap();
        assert_eq!(*store.writes.lock().unwrap(), vec![single_name("notion")]);
        assert_eq!(read(&store, "notion").unwrap(), value);
        assert_eq!(*store.reads.lock().unwrap(), vec![single_name("notion")]);
        assert_eq!(store.values.lock().unwrap().len(), 1);
    }

    #[test]
    fn single_migration_preserves_chunked_and_legacy_entries() {
        let old = Memory::default();
        let value = vec![b'x'; CHUNK_BYTES * 3];
        write(&old, "notion", &value).unwrap();
        let chunked = old.values.lock().unwrap().clone();
        let store = SingleMemory::default();
        *store.values.lock().unwrap() = chunked.clone();
        assert_eq!(read(&store, "notion").unwrap(), value);
        let values = store.values.lock().unwrap();
        for (name, bytes) in chunked {
            assert_eq!(values.get(&name), Some(&bytes));
        }
        assert_eq!(values.get(&single_name("notion")), Some(&value));
        drop(values);

        let legacy = SingleMemory::default();
        legacy
            .values
            .lock()
            .unwrap()
            .insert("notion".into(), b"legacy".to_vec());
        assert_eq!(read(&legacy, "notion").unwrap(), b"legacy");
        let values = legacy.values.lock().unwrap();
        assert_eq!(values.get("notion").unwrap(), b"legacy");
        assert_eq!(values.get(&single_name("notion")).unwrap(), b"legacy");
    }

    #[test]
    fn denied_single_read_never_queries_index_or_legacy() {
        let store = SingleMemory::default();
        *store.deny_read.lock().unwrap() = Some(single_name("notion"));
        assert!(read(&store, "notion").is_err());
        assert_eq!(*store.reads.lock().unwrap(), vec![single_name("notion")]);
        assert!(store.writes.lock().unwrap().is_empty());
    }

    #[test]
    fn failed_single_write_preserves_legacy_and_existing_value() {
        let store = SingleMemory::default();
        store
            .values
            .lock()
            .unwrap()
            .insert("notion".into(), b"legacy".to_vec());
        *store.deny_write.lock().unwrap() = true;
        assert!(read(&store, "notion").is_err());
        assert_eq!(
            store.values.lock().unwrap().get("notion").unwrap(),
            b"legacy"
        );
        assert!(!store
            .values
            .lock()
            .unwrap()
            .contains_key(&single_name("notion")));
        store
            .values
            .lock()
            .unwrap()
            .insert(single_name("notion"), b"existing".to_vec());
        assert!(write(&store, "notion", b"replacement").is_err());
        assert_eq!(
            store
                .values
                .lock()
                .unwrap()
                .get(&single_name("notion"))
                .unwrap(),
            b"existing"
        );
    }

    #[test]
    fn explicit_delete_removes_single_and_legacy_and_cannot_restore_them() {
        let old = Memory::default();
        write(&old, "notion", &vec![b'x'; CHUNK_BYTES * 2]).unwrap();
        let store = SingleMemory::default();
        *store.values.lock().unwrap() = old.values.lock().unwrap().clone();
        store
            .values
            .lock()
            .unwrap()
            .insert("notion".into(), b"legacy".to_vec());
        read(&store, "notion").unwrap();
        delete(&store, "notion").unwrap();
        assert!(store.values.lock().unwrap().is_empty());
        assert!(read(&store, "notion").is_err());
        assert!(store.values.lock().unwrap().is_empty());
    }

    #[test]
    fn failed_delete_preserves_single_across_partial_legacy_cleanup() {
        let old = Memory::default();
        write(&old, "notion", &vec![b'x'; CHUNK_BYTES * 2]).unwrap();
        let index = read_index(&old, "notion").unwrap().unwrap();
        let targets = [
            chunk_name("notion", &index, 0),
            chunk_name("notion", &index, 1),
            index_name("notion"),
            "notion".to_owned(),
            single_name("notion"),
        ];
        for target in targets {
            let store = SingleMemory::default();
            *store.values.lock().unwrap() = old.values.lock().unwrap().clone();
            store
                .values
                .lock()
                .unwrap()
                .insert("notion".into(), b"legacy".to_vec());
            store
                .values
                .lock()
                .unwrap()
                .insert(single_name("notion"), b"current".to_vec());
            *store.deny_delete.lock().unwrap() = Some(target);
            assert!(delete(&store, "notion").is_err());
            assert_eq!(read(&store, "notion").unwrap(), b"current");
            *store.deny_delete.lock().unwrap() = None;
            delete(&store, "notion").unwrap();
            assert!(store.values.lock().unwrap().is_empty());
            assert!(read(&store, "notion").is_err());
        }
    }

    #[test]
    fn failed_legacy_index_read_preserves_single() {
        let store = SingleMemory::default();
        store
            .values
            .lock()
            .unwrap()
            .insert(single_name("notion"), b"current".to_vec());
        *store.deny_read.lock().unwrap() = Some(index_name("notion"));
        assert!(delete(&store, "notion").is_err());
        assert_eq!(read(&store, "notion").unwrap(), b"current");
    }
}
