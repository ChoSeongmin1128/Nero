use serde::{Deserialize, Serialize};

const SERVICE: &str = "io.github.choseongmin1128.grove.desktop.google-calendar";
const ACCOUNT: &str = "google-calendar";
const ALTERNATE_ACCOUNT: &str = "google-calendar-alternate";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredToken {
    pub refresh_token: String,
    pub account_id: String,
    pub client_id: String,
}

pub struct TokenStore;
impl TokenStore {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn entry(slot: bool) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, if slot { ALTERNATE_ACCOUNT } else { ACCOUNT })
            .map_err(|_| "OS 보안 저장소를 열지 못했습니다. 다시 연결해 주세요.".into())
    }
    pub fn load(&self, slot: bool) -> Result<StoredToken, String> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let value = Self::entry(slot)?
                .get_password()
                .map_err(|_| "저장된 Google 연결을 읽지 못했습니다. 다시 연결해 주세요.")?;
            serde_json::from_str(&value)
                .map_err(|_| "저장된 Google 연결을 확인하지 못했습니다. 다시 연결해 주세요.".into())
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = slot;
            Err("Google 캘린더 연결은 macOS와 Windows에서 지원합니다.".into())
        }
    }
    pub fn save(&self, value: &StoredToken, slot: bool) -> Result<(), String> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let encoded =
                serde_json::to_string(value).map_err(|_| "Google 연결을 저장하지 못했습니다.")?;
            Self::entry(slot)?
                .set_password(&encoded)
                .map_err(|_| "Google 연결을 OS 보안 저장소에 저장하지 못했습니다.".into())
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (value, slot);
            Err("Google 캘린더 연결은 macOS와 Windows에서 지원합니다.".into())
        }
    }
    pub fn delete(&self) -> Result<(), String> {
        // Both bounded slots belong to this calendar connection. A failed staged
        // connection never overwrites the active slot, and disconnect removes both.
        let first = self.delete_slot(false);
        let second = self.delete_slot(true);
        first.and(second)
    }
    fn delete_slot(&self, slot: bool) -> Result<(), String> {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        match Self::entry(slot)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("OS 보안 저장소의 Google 연결을 삭제하지 못했습니다.".into()),
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = slot;
            Ok(())
        }
    }
}
