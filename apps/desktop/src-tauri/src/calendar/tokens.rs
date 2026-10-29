use serde::{Deserialize, Serialize};

const SERVICE: &str = "io.github.choseongmin1128.nero.desktop.google-calendar";
const LEGACY_SERVICE: &str = "io.github.choseongmin1128.grove.desktop.google-calendar";
const ACCOUNT: &str = "google-calendar";
const ALTERNATE_ACCOUNT: &str = "google-calendar-alternate";
const UNAVAILABLE: &str = "저장된 Google 연결을 읽지 못했습니다. 다시 연결해 주세요.";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredToken {
    pub refresh_token: String,
    pub account_id: String,
    pub client_id: String,
}

trait Credentials {
    fn read(&self, service: &str, account: &str) -> Result<Option<String>, String>;
    fn write(&self, service: &str, account: &str, value: &str) -> Result<(), String>;
    fn delete(&self, service: &str, account: &str) -> Result<(), String>;
}

fn account(slot: bool) -> &'static str {
    if slot {
        ALTERNATE_ACCOUNT
    } else {
        ACCOUNT
    }
}

fn marker(slot: bool) -> String {
    format!("{}-migration-complete", account(slot))
}

fn decode(value: &str) -> Result<StoredToken, String> {
    serde_json::from_str(value)
        .map_err(|_| "저장된 Google 연결을 확인하지 못했습니다. 다시 연결해 주세요.".into())
}

fn verified_save(store: &impl Credentials, value: &StoredToken, slot: bool) -> Result<(), String> {
    let encoded = serde_json::to_string(value).map_err(|_| UNAVAILABLE.to_string())?;
    store.write(SERVICE, account(slot), &encoded)?;
    let saved = store.read(SERVICE, account(slot))?.ok_or(UNAVAILABLE)?;
    if decode(&saved)? != *value {
        return Err(UNAVAILABLE.into());
    }
    store.write(SERVICE, &marker(slot), "complete")
}

fn load_or_migrate(store: &impl Credentials, slot: bool) -> Result<StoredToken, String> {
    if let Some(value) = store.read(SERVICE, account(slot))? {
        return decode(&value);
    }
    if store.read(SERVICE, &marker(slot))?.is_some() {
        return Err(UNAVAILABLE.into());
    }
    let encoded = store
        .read(LEGACY_SERVICE, account(slot))?
        .ok_or(UNAVAILABLE)?;
    let value = decode(&encoded)?;
    verified_save(store, &value, slot)?;
    Ok(value)
}

fn disconnect_slot(store: &impl Credentials, slot: bool) -> Result<(), String> {
    // Preserve the old credential without restoring it after an explicit disconnect.
    store.write(SERVICE, &marker(slot), "disconnected")?;
    store.delete(SERVICE, account(slot))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
struct SystemCredentials;

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl Credentials for SystemCredentials {
    fn read(&self, service: &str, account: &str) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(service, account).map_err(|_| UNAVAILABLE)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(UNAVAILABLE.into()),
        }
    }
    fn write(&self, service: &str, account: &str, value: &str) -> Result<(), String> {
        keyring::Entry::new(service, account)
            .map_err(|_| UNAVAILABLE)?
            .set_password(value)
            .map_err(|_| UNAVAILABLE.into())
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(service, account).map_err(|_| UNAVAILABLE)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(UNAVAILABLE.into()),
        }
    }
}

pub struct TokenStore;
impl TokenStore {
    pub fn load(&self, slot: bool) -> Result<StoredToken, String> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            load_or_migrate(&SystemCredentials, slot)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = slot;
            Err(UNAVAILABLE.into())
        }
    }
    pub fn save(&self, value: &StoredToken, slot: bool) -> Result<(), String> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            verified_save(&SystemCredentials, value, slot)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (value, slot);
            Err(UNAVAILABLE.into())
        }
    }
    pub fn delete(&self) -> Result<(), String> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let first = disconnect_slot(&SystemCredentials, false);
            let second = disconnect_slot(&SystemCredentials, true);
            first.and(second)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Memory {
        values: RefCell<BTreeMap<(String, String), String>>,
        reject_write: bool,
        corrupt_write: bool,
        reject_read: bool,
    }
    impl Credentials for Memory {
        fn read(&self, service: &str, account: &str) -> Result<Option<String>, String> {
            if self.reject_read {
                return Err("locked".into());
            }
            Ok(self
                .values
                .borrow()
                .get(&(service.into(), account.into()))
                .cloned())
        }
        fn write(&self, service: &str, account: &str, value: &str) -> Result<(), String> {
            if self.reject_write {
                return Err("denied".into());
            }
            self.values.borrow_mut().insert(
                (service.into(), account.into()),
                if self.corrupt_write {
                    "invalid".into()
                } else {
                    value.into()
                },
            );
            Ok(())
        }
        fn delete(&self, service: &str, account: &str) -> Result<(), String> {
            self.values
                .borrow_mut()
                .remove(&(service.into(), account.into()));
            Ok(())
        }
    }
    fn legacy(store: &Memory, slot: bool) -> StoredToken {
        let value = StoredToken {
            refresh_token: "synthetic".into(),
            account_id: "fixture".into(),
            client_id: "client".into(),
        };
        store.values.borrow_mut().insert(
            (LEGACY_SERVICE.into(), account(slot).into()),
            serde_json::to_string(&value).unwrap(),
        );
        value
    }
    #[test]
    fn both_slots_migrate_and_retain_the_old_credentials() {
        let store = Memory::default();
        for slot in [false, true] {
            let expected = legacy(&store, slot);
            assert!(load_or_migrate(&store, slot).unwrap() == expected);
            assert!(store.read(LEGACY_SERVICE, account(slot)).unwrap().is_some());
            assert!(store.read(SERVICE, &marker(slot)).unwrap().is_some());
        }
    }
    #[test]
    fn disconnect_does_not_resurrect_a_retained_credential() {
        let store = Memory::default();
        legacy(&store, false);
        load_or_migrate(&store, false).unwrap();
        disconnect_slot(&store, false).unwrap();
        assert!(load_or_migrate(&store, false).is_err());
        assert!(store.read(LEGACY_SERVICE, ACCOUNT).unwrap().is_some());
    }
    #[test]
    fn failures_never_remove_the_source_or_claim_migration_complete() {
        for store in [
            Memory {
                reject_write: true,
                ..Memory::default()
            },
            Memory {
                corrupt_write: true,
                ..Memory::default()
            },
            Memory {
                reject_read: true,
                ..Memory::default()
            },
        ] {
            legacy(&store, false);
            assert!(load_or_migrate(&store, false).is_err());
            assert!(store
                .values
                .borrow()
                .contains_key(&(LEGACY_SERVICE.into(), ACCOUNT.into())));
            assert!(!store
                .values
                .borrow()
                .contains_key(&(SERVICE.into(), marker(false))));
        }
    }
    #[test]
    fn current_credentials_take_precedence_over_retained_tokens() {
        let store = Memory::default();
        legacy(&store, false);
        let expected = StoredToken {
            refresh_token: "current".into(),
            account_id: "other".into(),
            client_id: "client".into(),
        };
        verified_save(&store, &expected, false).unwrap();
        assert!(load_or_migrate(&store, false).unwrap() == expected);
    }
}
