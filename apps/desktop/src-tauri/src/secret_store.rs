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

fn allowed(service: &str, account: &str) -> bool {
    !account.is_empty()
        && matches!(
            service,
            "io.github.choseongmin1128.nero.desktop.ai-accounts"
                | "io.github.choseongmin1128.nero.desktop.notion-context"
                | "io.github.choseongmin1128.nero.desktop.google-calendar"
                | "io.github.choseongmin1128.grove.desktop.google-calendar"
        )
}

pub fn read(service: &str, account: &str, access: Access) -> Result<Option<Vec<u8>>, Error> {
    if !allowed(service, account) {
        return Err(Error::Unavailable);
    }
    platform::read(service, account, access)
}

pub fn write(service: &str, account: &str, value: &[u8], access: Access) -> Result<(), Error> {
    if !allowed(service, account) {
        return Err(Error::Unavailable);
    }
    platform::write(service, account, value, access)
}

pub fn delete(service: &str, account: &str, access: Access) -> Result<(), Error> {
    if !allowed(service, account) {
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
    use security_framework_sys::keychain_item::{
        SecItemAdd, SecItemCopyMatching, SecItemDelete, SecItemUpdate,
    };
    use std::ptr;

    #[link(name = "LocalAuthentication", kind = "framework")]
    extern "C" {}

    const NOT_FOUND: i32 = -25300;

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
    use super::allowed;
    #[test]
    fn restricts_service_and_account() {
        assert!(allowed(
            "io.github.choseongmin1128.nero.desktop.ai-accounts",
            "token"
        ));
        assert!(!allowed(
            "io.github.choseongmin1128.nero.desktop.ai-accounts",
            ""
        ));
        assert!(!allowed("Claude Code-credentials", "token"));
        assert!(!allowed("", "token"));
    }
}
