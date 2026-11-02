use crate::secret_store::{Access, Error};
use serde::{Deserialize, Serialize};

const SERVICE: &str = "io.github.choseongmin1128.nero.desktop.google-calendar";
const LEGACY_SERVICE: &str = "io.github.choseongmin1128.grove.desktop.google-calendar";
const ACCOUNT: &str = "google-calendar";
const ALTERNATE_ACCOUNT: &str = "google-calendar-alternate";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredToken {
    pub refresh_token: String,
    pub account_id: String,
    pub client_id: String,
}

trait Credentials {
    fn read(&self, service: &str, account: &str, access: Access) -> Result<Option<String>, Error>;
    fn write(&self, service: &str, account: &str, value: &str, access: Access)
        -> Result<(), Error>;
    fn delete(&self, service: &str, account: &str, access: Access) -> Result<(), Error>;
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

fn verified_save(
    store: &impl Credentials,
    value: &StoredToken,
    slot: bool,
    access: Access,
) -> Result<(), Error> {
    let encoded = serde_json::to_string(value).map_err(|_| Error::Unavailable)?;
    store.write(SERVICE, account(slot), &encoded, access)?;
    let saved = store
        .read(SERVICE, account(slot), Access::Silent)?
        .ok_or(Error::Unavailable)?;
    if decode(&saved).map_err(|_| Error::Unavailable)? != *value {
        return Err(Error::Unavailable);
    }
    store.write(SERVICE, &marker(slot), "complete", Access::Silent)
}

fn load_or_migrate(
    store: &impl Credentials,
    slot: bool,
    access: Access,
) -> Result<StoredToken, Error> {
    if let Some(value) = store.read(SERVICE, account(slot), access)? {
        return decode(&value).map_err(|_| Error::Unavailable);
    }
    if store
        .read(SERVICE, &marker(slot), Access::Silent)?
        .is_some()
    {
        return Err(Error::InteractionRequired);
    }
    let encoded = store
        .read(LEGACY_SERVICE, account(slot), Access::Silent)?
        .ok_or(Error::Unavailable)?;
    let value = decode(&encoded).map_err(|_| Error::Unavailable)?;
    verified_save(store, &value, slot, Access::Silent)?;
    Ok(value)
}

fn disconnect_slot(store: &impl Credentials, slot: bool) -> Result<(), Error> {
    // Preserve the old credential without restoring it after an explicit disconnect.
    store.write(SERVICE, &marker(slot), "disconnected", Access::Silent)?;
    store.delete(SERVICE, account(slot), Access::Silent)
}

struct SystemCredentials;
impl Credentials for SystemCredentials {
    fn read(&self, service: &str, account: &str, access: Access) -> Result<Option<String>, Error> {
        crate::secret_store::read(service, account, access)?
            .map(|bytes| String::from_utf8(bytes).map_err(|_| Error::Unavailable))
            .transpose()
    }
    fn write(
        &self,
        service: &str,
        account: &str,
        value: &str,
        access: Access,
    ) -> Result<(), Error> {
        crate::secret_store::write(service, account, value.as_bytes(), access)
    }
    fn delete(&self, service: &str, account: &str, access: Access) -> Result<(), Error> {
        crate::secret_store::delete(service, account, access)
    }
}

pub struct TokenStore;
impl TokenStore {
    pub fn load(&self, slot: bool, access: Access) -> Result<StoredToken, Error> {
        load_or_migrate(&SystemCredentials, slot, access)
    }
    pub fn save(&self, value: &StoredToken, slot: bool, access: Access) -> Result<(), Error> {
        verified_save(&SystemCredentials, value, slot, access)
    }
    pub fn delete(&self) -> Result<(), Error> {
        let first = disconnect_slot(&SystemCredentials, false);
        let second = disconnect_slot(&SystemCredentials, true);
        first.and(second)
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
        calls: RefCell<Vec<(String, String, String, bool)>>,
    }
    impl Credentials for Memory {
        fn read(
            &self,
            service: &str,
            account: &str,
            access: Access,
        ) -> Result<Option<String>, Error> {
            self.calls.borrow_mut().push((
                "read".into(),
                service.into(),
                account.into(),
                matches!(access, Access::Interactive),
            ));
            if self.reject_read {
                return Err(Error::InteractionRequired);
            }
            Ok(self
                .values
                .borrow()
                .get(&(service.into(), account.into()))
                .cloned())
        }
        fn write(
            &self,
            service: &str,
            account: &str,
            value: &str,
            access: Access,
        ) -> Result<(), Error> {
            self.calls.borrow_mut().push((
                "write".into(),
                service.into(),
                account.into(),
                matches!(access, Access::Interactive),
            ));
            if self.reject_write {
                return Err(Error::InteractionRequired);
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
        fn delete(&self, service: &str, account: &str, access: Access) -> Result<(), Error> {
            self.calls.borrow_mut().push((
                "delete".into(),
                service.into(),
                account.into(),
                matches!(access, Access::Interactive),
            ));
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
            assert!(load_or_migrate(&store, slot, Access::Silent).unwrap() == expected);
            assert!(store
                .read(LEGACY_SERVICE, account(slot), Access::Silent)
                .unwrap()
                .is_some());
            assert!(store
                .read(SERVICE, &marker(slot), Access::Silent)
                .unwrap()
                .is_some());
        }
    }
    #[test]
    fn disconnect_does_not_resurrect_a_retained_credential() {
        let store = Memory::default();
        legacy(&store, false);
        load_or_migrate(&store, false, Access::Silent).unwrap();
        disconnect_slot(&store, false).unwrap();
        assert!(load_or_migrate(&store, false, Access::Silent).is_err());
        assert!(store
            .read(LEGACY_SERVICE, ACCOUNT, Access::Silent)
            .unwrap()
            .is_some());
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
            assert!(load_or_migrate(&store, false, Access::Silent).is_err());
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
    fn explicit_load_and_save_limit_interaction_to_one_current_item() {
        let store = Memory::default();
        let token = legacy(&store, false);
        store.calls.borrow_mut().clear();
        assert!(load_or_migrate(&store, false, Access::Interactive).unwrap() == token);
        let calls = store.calls.borrow();
        assert_eq!(calls.iter().filter(|entry| entry.3).count(), 1);
        assert_eq!(
            calls[0],
            ("read".into(), SERVICE.into(), ACCOUNT.into(), true)
        );
        assert!(calls.iter().skip(1).all(|entry| !entry.3));
        drop(calls);

        store.calls.borrow_mut().clear();
        verified_save(&store, &token, false, Access::Interactive).unwrap();
        let calls = store.calls.borrow();
        assert_eq!(calls.iter().filter(|entry| entry.3).count(), 1);
        assert_eq!(
            calls[0],
            ("write".into(), SERVICE.into(), ACCOUNT.into(), true)
        );
        assert!(calls.iter().skip(1).all(|entry| !entry.3));
        drop(calls);

        store.calls.borrow_mut().clear();
        disconnect_slot(&store, false).unwrap();
        disconnect_slot(&store, true).unwrap();
        assert!(store.calls.borrow().iter().all(|entry| !entry.3));
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
        verified_save(&store, &expected, false, Access::Silent).unwrap();
        assert!(load_or_migrate(&store, false, Access::Silent).unwrap() == expected);
    }
}
