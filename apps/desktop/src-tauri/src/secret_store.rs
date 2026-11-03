#[derive(Clone, Copy)]
pub enum Access {
    Silent,
    Interactive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InteractionRequired,
    Unavailable,
}

#[cfg(any(target_os = "macos", test))]
mod interaction {
    use super::{Access, Error};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    };

    pub trait Policy {
        fn get(&self) -> Result<bool, Error>;
        fn set(&self, allowed: bool) -> Result<(), Error>;
    }

    pub struct Gate<P> {
        pub(super) policy: P,
        lock: Mutex<()>,
        failed: AtomicBool,
    }

    impl<P: Policy> Gate<P> {
        pub const fn new(policy: P) -> Self {
            Self {
                policy,
                lock: Mutex::new(()),
                failed: AtomicBool::new(false),
            }
        }

        pub fn run<T>(
            &self,
            access: Access,
            operation: impl FnOnce() -> Result<T, Error>,
        ) -> Result<T, Error> {
            let _lock = self.lock.lock().unwrap_or_else(|error| error.into_inner());
            if self.failed.load(Ordering::Acquire) {
                return Err(Error::Unavailable);
            }
            if matches!(access, Access::Interactive) {
                return operation();
            }
            let previous = self.policy.get()?;
            // This process-local UI policy is temporary; it does not change OS settings or ACLs.
            // https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_keychain/lib/Globals.cpp
            let mut restore = Restore {
                gate: self,
                previous,
                active: true,
            };
            if let Err(error) = self.policy.set(false) {
                restore.finish()?;
                return Err(error);
            }
            let result = operation();
            restore.finish()?;
            result
        }
    }

    struct Restore<'a, P: Policy> {
        gate: &'a Gate<P>,
        previous: bool,
        active: bool,
    }
    impl<P: Policy> Restore<'_, P> {
        fn finish(&mut self) -> Result<(), Error> {
            self.active = false;
            if self.gate.policy.set(self.previous).is_err() {
                self.gate.failed.store(true, Ordering::Release);
                return Err(Error::Unavailable);
            }
            Ok(())
        }
    }
    impl<P: Policy> Drop for Restore<'_, P> {
        fn drop(&mut self) {
            if self.active {
                let _ = self.finish();
            }
        }
    }
}

fn allowed_for(profile: &nero_core::environment::Profile, service: &str, account: &str) -> bool {
    !account.is_empty()
        && [
            profile.ai_credentials_service,
            profile.notion_credentials_service,
            profile.calendar_credentials_service,
        ]
        .contains(&service)
}

fn read_allowed_for(
    profile: &nero_core::environment::Profile,
    service: &str,
    account: &str,
) -> bool {
    allowed_for(profile, service, account)
        || (!account.is_empty() && profile.legacy_calendar_service == Some(service))
}

pub fn read(service: &str, account: &str, access: Access) -> Result<Option<Vec<u8>>, Error> {
    if !read_allowed_for(&nero_core::environment::CURRENT, service, account) {
        return Err(Error::Unavailable);
    }
    platform::read(service, account, access)
}

pub fn write(service: &str, account: &str, value: &[u8], access: Access) -> Result<(), Error> {
    if !allowed_for(&nero_core::environment::CURRENT, service, account) {
        return Err(Error::Unavailable);
    }
    platform::write(service, account, value, access)
}

pub fn delete(service: &str, account: &str, access: Access) -> Result<(), Error> {
    if !allowed_for(&nero_core::environment::CURRENT, service, account) {
        return Err(Error::Unavailable);
    }
    platform::delete(service, account, access)
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{Access, Error};

    fn entry(service: &str, account: &str) -> Result<keyring::Entry, Error> {
        keyring::Entry::new(service, account).map_err(|_| Error::Unavailable)
    }

    pub fn read(service: &str, account: &str, _: Access) -> Result<Option<Vec<u8>>, Error> {
        match entry(service, account)?.get_secret() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(Error::Unavailable),
        }
    }

    pub fn write(service: &str, account: &str, value: &[u8], _: Access) -> Result<(), Error> {
        entry(service, account)?
            .set_secret(value)
            .map_err(|_| Error::Unavailable)
    }

    pub fn delete(service: &str, account: &str, _: Access) -> Result<(), Error> {
        match entry(service, account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(Error::Unavailable),
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::interaction::{Gate, Policy};
    use super::{Access, Error};
    use core_foundation::array::CFArray;
    use core_foundation::base::{CFType, CFTypeRef, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::data::CFData;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::{CFString, CFStringRef};
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use security_framework::os::macos::keychain::{SecKeychain, SecPreferencesDomain};
    use security_framework_sys::item::*;
    use security_framework_sys::keychain::{
        SecKeychainGetUserInteractionAllowed, SecKeychainSetUserInteractionAllowed,
    };
    use security_framework_sys::keychain_item::{
        SecItemAdd, SecItemCopyMatching, SecItemDelete, SecItemUpdate,
    };
    use std::ptr;

    #[link(name = "LocalAuthentication", kind = "framework")]
    extern "C" {}

    const NOT_FOUND: i32 = -25300;

    struct SystemPolicy;
    impl Policy for SystemPolicy {
        fn get(&self) -> Result<bool, Error> {
            let mut value = 0;
            if unsafe { SecKeychainGetUserInteractionAllowed(&mut value) } == 0 {
                Ok(value != 0)
            } else {
                Err(Error::Unavailable)
            }
        }
        fn set(&self, allowed: bool) -> Result<(), Error> {
            if unsafe { SecKeychainSetUserInteractionAllowed(u8::from(allowed)) } == 0 {
                Ok(())
            } else {
                Err(Error::Unavailable)
            }
        }
    }
    static GATE: Gate<SystemPolicy> = Gate::new(SystemPolicy);

    fn status_error(status: i32) -> Error {
        match status {
            -25308 | -128 | -25293 => Error::InteractionRequired,
            _ => Error::Unavailable,
        }
    }

    unsafe fn string_type(value: CFStringRef) -> CFType {
        CFString::wrap_under_get_rule(value).as_CFType()
    }

    fn context(access: Access) -> Result<Retained<AnyObject>, Error> {
        let class = AnyClass::get(c"LAContext").ok_or(Error::Unavailable)?;
        let context: Option<Retained<AnyObject>> = unsafe { msg_send![class, new] };
        let context = context.ok_or(Error::Unavailable)?;
        if matches!(access, Access::Silent) {
            unsafe {
                let _: () = msg_send![&*context, setInteractionNotAllowed: Bool::YES];
            }
        }
        Ok(context)
    }

    // The user-domain default is the same keychain selected by keyring's AppleNative backend.
    fn keychain() -> Result<SecKeychain, Error> {
        SecKeychain::default_for_domain(SecPreferencesDomain::User)
            .map_err(|error| status_error(error.code()))
    }

    enum Operation {
        Read,
        Update,
        Add,
        Delete,
    }

    fn make_query(
        service: &str,
        account: &str,
        access: Access,
        operation: Operation,
        value: Option<&[u8]>,
    ) -> Result<CFDictionary<CFType, CFType>, Error> {
        let keychain = keychain()?;
        let context = context(access)?;
        // CF dictionaries retain their values, including the Objective-C authentication context.
        let context_value = unsafe {
            CFType::wrap_under_get_rule((&*context as *const AnyObject).cast::<std::ffi::c_void>())
        };
        let mut pairs = unsafe {
            vec![
                (
                    string_type(kSecClass),
                    string_type(kSecClassGenericPassword),
                ),
                (
                    string_type(kSecAttrService),
                    CFString::new(service).as_CFType(),
                ),
                (
                    string_type(kSecAttrAccount),
                    CFString::new(account).as_CFType(),
                ),
                (string_type(kSecUseAuthenticationContext), context_value),
            ]
        };
        unsafe {
            match operation {
                Operation::Add => {
                    pairs.push((string_type(kSecUseKeychain), keychain.as_CFType()));
                    pairs.push((
                        string_type(kSecValueData),
                        CFData::from_buffer(value.ok_or(Error::Unavailable)?).as_CFType(),
                    ));
                }
                _ => {
                    pairs.push((
                        string_type(kSecMatchSearchList),
                        CFArray::from_CFTypes(&[keychain]).as_CFType(),
                    ));
                    if matches!(operation, Operation::Read) {
                        pairs.push((
                            string_type(kSecReturnData),
                            CFBoolean::true_value().as_CFType(),
                        ));
                    }
                }
            }
        }
        Ok(CFDictionary::from_CFType_pairs(&pairs))
    }

    pub fn read(service: &str, account: &str, access: Access) -> Result<Option<Vec<u8>>, Error> {
        GATE.run(access, || read_inner(service, account, access))
    }
    fn read_inner(service: &str, account: &str, access: Access) -> Result<Option<Vec<u8>>, Error> {
        let query = make_query(service, account, access, Operation::Read, None)?;
        let mut result: CFTypeRef = ptr::null();
        let status = unsafe { SecItemCopyMatching(query.as_concrete_TypeRef(), &mut result) };
        if status == NOT_FOUND {
            return Ok(None);
        }
        if status != 0 {
            return Err(status_error(status));
        }
        if result.is_null() {
            return Err(Error::Unavailable);
        }
        let result = unsafe { CFType::wrap_under_create_rule(result) };
        let data = result.downcast::<CFData>().ok_or(Error::Unavailable)?;
        Ok(Some(data.bytes().to_vec()))
    }

    pub fn write(service: &str, account: &str, value: &[u8], access: Access) -> Result<(), Error> {
        GATE.run(access, || write_inner(service, account, value, access))
    }
    fn write_inner(
        service: &str,
        account: &str,
        value: &[u8],
        access: Access,
    ) -> Result<(), Error> {
        let query = make_query(service, account, access, Operation::Update, None)?;
        let attrs = unsafe {
            CFDictionary::from_CFType_pairs(&[(
                string_type(kSecValueData),
                CFData::from_buffer(value).as_CFType(),
            )])
        };
        let status =
            unsafe { SecItemUpdate(query.as_concrete_TypeRef(), attrs.as_concrete_TypeRef()) };
        if status == 0 {
            return Ok(());
        }
        if status != NOT_FOUND {
            return Err(status_error(status));
        }
        let add = make_query(service, account, access, Operation::Add, Some(value))?;
        let status = unsafe { SecItemAdd(add.as_concrete_TypeRef(), ptr::null_mut()) };
        if status == 0 {
            Ok(())
        } else {
            Err(status_error(status))
        }
    }

    pub fn delete(service: &str, account: &str, access: Access) -> Result<(), Error> {
        GATE.run(access, || delete_inner(service, account, access))
    }
    fn delete_inner(service: &str, account: &str, access: Access) -> Result<(), Error> {
        let query = make_query(service, account, access, Operation::Delete, None)?;
        let status = unsafe { SecItemDelete(query.as_concrete_TypeRef()) };
        if status == 0 || status == NOT_FOUND {
            Ok(())
        } else {
            Err(status_error(status))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn interaction_errors_are_distinguished() {
            for status in [-25308, -128, -25293] {
                assert_eq!(status_error(status), Error::InteractionRequired);
            }
            assert_eq!(status_error(-1), Error::Unavailable);
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::{Access, Error};
    pub fn read(_: &str, _: &str, _: Access) -> Result<Option<Vec<u8>>, Error> {
        Err(Error::Unavailable)
    }
    pub fn write(_: &str, _: &str, _: &[u8], _: Access) -> Result<(), Error> {
        Err(Error::Unavailable)
    }
    pub fn delete(_: &str, _: &str, _: Access) -> Result<(), Error> {
        Err(Error::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::{allowed_for, read_allowed_for};
    use nero_core::environment::{DEVELOPMENT, PRODUCTION};

    #[test]
    fn services_are_isolated_and_only_production_reads_grove() {
        let dev = DEVELOPMENT;
        let prod = PRODUCTION;
        for service in [
            dev.ai_credentials_service,
            dev.notion_credentials_service,
            dev.calendar_credentials_service,
        ] {
            assert!(allowed_for(&dev, service, "token"));
            assert!(!read_allowed_for(&prod, service, "token"));
            assert!(!allowed_for(&dev, service, ""));
        }
        for service in [
            prod.ai_credentials_service,
            prod.notion_credentials_service,
            prod.calendar_credentials_service,
        ] {
            assert!(allowed_for(&prod, service, "token"));
            assert!(!read_allowed_for(&dev, service, "token"));
        }
        let grove = prod.legacy_calendar_service.unwrap();
        assert!(read_allowed_for(&prod, grove, "token"));
        assert!(!allowed_for(&prod, grove, "token"));
        assert!(!read_allowed_for(&dev, grove, "token"));
        assert!(!read_allowed_for(&dev, "Claude Code-credentials", "token"));
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::{
        interaction::{Gate, Policy},
        Access, Error,
    };
    use std::sync::{mpsc, Arc, Mutex};

    struct State {
        allowed: bool,
        fail_get: bool,
        fail_set_at: Option<usize>,
        fail_restore: bool,
        sets: usize,
    }
    struct Fake(Mutex<State>);
    impl Fake {
        fn new(allowed: bool) -> Self {
            Self(Mutex::new(State {
                allowed,
                fail_get: false,
                fail_set_at: None,
                fail_restore: false,
                sets: 0,
            }))
        }
    }
    impl Policy for Fake {
        fn get(&self) -> Result<bool, Error> {
            let state = self.0.lock().unwrap();
            if state.fail_get {
                Err(Error::Unavailable)
            } else {
                Ok(state.allowed)
            }
        }
        fn set(&self, allowed: bool) -> Result<(), Error> {
            let mut state = self.0.lock().unwrap();
            state.sets += 1;
            state.allowed = allowed;
            if state.fail_set_at == Some(state.sets) || (state.fail_restore && state.sets == 2) {
                return Err(Error::Unavailable);
            }
            Ok(())
        }
    }

    #[test]
    fn restores_prior_value_after_success_and_operation_error() {
        for previous in [false, true] {
            let gate = Gate::new(Fake::new(previous));
            assert_eq!(gate.run(Access::Silent, || Ok(7)), Ok(7));
            assert_eq!(
                gate.run::<()>(Access::Silent, || Err(Error::InteractionRequired)),
                Err(Error::InteractionRequired)
            );
            assert_eq!(gate.policy.get(), Ok(previous));
            assert_eq!(
                gate.run(Access::Interactive, || gate.policy.get()),
                Ok(previous)
            );
        }
    }

    #[test]
    fn get_and_disable_fail_before_operation() {
        for fail_get in [true, false] {
            let gate = Gate::new(Fake::new(true));
            {
                let mut state = gate.policy.0.lock().unwrap();
                state.fail_get = fail_get;
                if !fail_get {
                    state.fail_set_at = Some(1);
                }
            }
            assert_eq!(
                gate.run::<()>(Access::Silent, || panic!("operation must not run")),
                Err(Error::Unavailable)
            );
            assert!(gate.policy.0.lock().unwrap().allowed);
        }
        let gate = Gate::new(Fake::new(true));
        {
            let mut state = gate.policy.0.lock().unwrap();
            state.fail_set_at = Some(1);
            state.fail_restore = true;
        }
        assert_eq!(
            gate.run::<()>(Access::Silent, || panic!("operation must not run")),
            Err(Error::Unavailable)
        );
        assert_eq!(
            gate.run::<()>(Access::Interactive, || panic!(
                "failed gate must stay closed"
            )),
            Err(Error::Unavailable)
        );
    }

    #[test]
    fn failed_restore_blocks_later_calls() {
        let gate = Gate::new(Fake::new(true));
        gate.policy.0.lock().unwrap().fail_set_at = Some(2);
        assert_eq!(gate.run(Access::Silent, || Ok(1)), Err(Error::Unavailable));
        assert_eq!(
            gate.run::<()>(Access::Interactive, || panic!(
                "failed gate must stay closed"
            )),
            Err(Error::Unavailable)
        );
    }

    #[test]
    fn panic_restores_and_unlocks_gate() {
        let gate = Gate::new(Fake::new(true));
        let result = std::panic::catch_unwind(|| {
            let _: Result<(), Error> = gate.run(Access::Silent, || panic!("fixture"));
        });
        assert!(result.is_err());
        assert_eq!(gate.policy.get(), Ok(true));
        assert_eq!(gate.run(Access::Interactive, || Ok(1)), Ok(1));

        let failed = Gate::new(Fake::new(true));
        failed.policy.0.lock().unwrap().fail_set_at = Some(2);
        assert!(std::panic::catch_unwind(|| {
            let _: Result<(), Error> = failed.run(Access::Silent, || panic!("fixture"));
        })
        .is_err());
        assert_eq!(
            failed.run::<()>(Access::Interactive, || panic!(
                "failed gate must stay closed"
            )),
            Err(Error::Unavailable)
        );
    }

    #[test]
    fn silent_and_interactive_are_serialized() {
        let gate = Arc::new(Gate::new(Fake::new(true)));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let silent = gate.clone();
        let first = std::thread::spawn(move || {
            silent.run(Access::Silent, || {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
        });
        entered_rx.recv().unwrap();
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let (completed_tx, completed_rx) = mpsc::channel();
        let interactive = gate.clone();
        let second = std::thread::spawn(move || {
            attempt_tx.send(()).unwrap();
            interactive.run(Access::Interactive, || {
                completed_tx.send(()).unwrap();
                interactive.policy.get()
            })
        });
        attempt_rx.recv().unwrap();
        assert_eq!(
            completed_rx.recv_timeout(std::time::Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        release_tx.send(()).unwrap();
        completed_rx.recv().unwrap();
        assert_eq!(first.join().unwrap(), Ok(()));
        assert_eq!(second.join().unwrap(), Ok(true));
    }
}
