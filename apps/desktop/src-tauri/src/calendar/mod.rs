mod attendees;
mod callback_page;
mod google;
mod oauth;
mod schedule;
mod tokens;

pub use attendees::GoogleAttendeeFetch;
pub use google::CalendarChoice;
pub use oauth::ConnectRequest;
use oauth::{Config, Pending};
use schedule::{Notice, Scheduler};
pub use schedule::{Occurrence, ReminderChange};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokens::{StoredToken, TokenStore};

const SYNC_INTERVAL_MS: u64 = 60_000;
const RETRY_INTERVAL_MS: u64 = 30_000;
const FRESH_MS: u64 = 90_000;
const MAX_STATE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarSnapshot {
    pub revision: u64,
    pub configured: bool,
    pub connected: bool,
    pub meet_connected: bool,
    pub connecting: bool,
    pub restoring: bool,
    pub keychain_blocked: bool,
    pub account_label: Option<String>,
    pub calendars: Vec<CalendarChoice>,
    pub selected_calendars: Vec<String>,
    pub last_synced_at: Option<u64>,
    pub error: Option<String>,
}

const KEYCHAIN_BLOCKED: &str =
    "저장된 Google 연결에 접근할 수 없습니다. 다시 시도해 접근을 허용하거나 다시 연결해 주세요.";

#[derive(Debug, Clone)]
enum Blocked {
    // The keychain refused background access; an explicit retry may ask for it.
    Keychain,
    // Google rejected the credential; only a new connection helps.
    Revoked(String),
}

impl Blocked {
    fn message(&self) -> String {
        match self {
            Self::Keychain => KEYCHAIN_BLOCKED.into(),
            Self::Revoked(message) => message.clone(),
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct State {
    schema_version: u32,
    revision: u64,
    account: Option<String>,
    #[serde(default)]
    token_slot: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    meet_authorized: bool,
    calendars: Vec<CalendarChoice>,
    selected: Vec<String>,
    events: Vec<Occurrence>,
    notices: BTreeMap<String, Notice>,
    last_synced_at: Option<u64>,
}
#[derive(Clone)]
struct Access {
    token: String,
    expires_at: u64,
}
fn is_false(value: &bool) -> bool {
    !value
}
struct Inner {
    state: State,
    access: Option<Access>,
    scheduler: Scheduler,
    pending: Option<Pending>,
    connecting_id: Option<String>,
    authorization_url: Option<String>,
    cancel: Option<Arc<AtomicBool>>,
    auth_ready: bool,
    credential_blocked: Option<Blocked>,
    stored_token: Option<(bool, StoredToken)>,
    pending_token: Option<(bool, StoredToken, Access)>,
    needs_sync: bool,
    // The events predate a sleep or clock jump; reminders wait for a new sync.
    woke: bool,
    rejections: u64,
    last_attempt: Option<u64>,
    last_tick: Option<u64>,
    error: Option<String>,
}

pub struct CalendarService {
    directory: PathBuf,
    config: Option<Config>,
    client: reqwest::Client,
    inner: Mutex<Inner>,
    generation: AtomicU64,
    connection_epoch: AtomicU64,
    syncing: AtomicBool,
    token_gate: Mutex<()>,
    access_gate: tokio::sync::Mutex<()>,
}

struct QueryContext {
    generation: u64,
    account: String,
    selected: Vec<String>,
    token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingBrowse {
    pub calendar: CalendarSnapshot,
    pub occurrences: Vec<Occurrence>,
    pub day: String,
    pub time_zone: String,
    pub scope_revision: u64,
    #[serde(skip)]
    context: QueryContext,
}

pub(crate) fn day_bounds(day: &str, time_zone: &str) -> Result<(u64, u64), String> {
    let invalid = || "날짜와 시간대를 확인해 주세요.".to_string();
    let date: jiff::civil::Date = day.parse().map_err(|_| invalid())?;
    if date.to_string() != day {
        return Err(invalid());
    }
    let zone = jiff::tz::TimeZone::get(time_zone).map_err(|_| invalid())?;
    let start = date.to_zoned(zone.clone()).map_err(|_| invalid())?;
    let end = date
        .tomorrow()
        .map_err(|_| invalid())?
        .to_zoned(zone)
        .map_err(|_| invalid())?;
    let start = u64::try_from(start.timestamp().as_millisecond()).map_err(|_| invalid())?;
    let end = u64::try_from(end.timestamp().as_millisecond()).map_err(|_| invalid())?;
    if end <= start {
        return Err(invalid());
    }
    Ok((start, end))
}
pub struct ReminderTicket {
    key: String,
    generation: u64,
}
struct SyncGuard<'a>(&'a AtomicBool);
impl Drop for SyncGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl CalendarService {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        let directory = root.join("calendar");
        let mut state = load(&directory)?;
        if state.schema_version == 0 {
            state.schema_version = 1;
        }
        let scheduler = Scheduler {
            notices: state.notices.clone(),
            ..Scheduler::default()
        };
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Google 캘린더 연결을 준비하지 못했습니다.")?;
        Ok(Self {
            directory,
            config: Config::bundled(),
            client,
            inner: Mutex::new(Inner {
                state,
                access: None,
                scheduler,
                pending: None,
                connecting_id: None,
                authorization_url: None,
                cancel: None,
                auth_ready: false,
                credential_blocked: None,
                stored_token: None,
                pending_token: None,
                needs_sync: true,
                woke: false,
                rejections: 0,
                last_attempt: None,
                last_tick: None,
                error: None,
            }),
            generation: AtomicU64::new(0),
            connection_epoch: AtomicU64::new(0),
            syncing: AtomicBool::new(false),
            token_gate: Mutex::new(()),
            access_gate: tokio::sync::Mutex::new(()),
        })
    }
    pub fn snapshot(&self) -> CalendarSnapshot {
        let inner = self.inner.lock().unwrap();
        self.snapshot_inner(&inner)
    }
    fn snapshot_inner(&self, inner: &Inner) -> CalendarSnapshot {
        let restoring = self.config.is_some()
            && inner.state.account.is_some()
            && !inner.auth_ready
            && inner.error.is_none()
            && inner.connecting_id.is_none();
        // Once the first check at launch fails for now (offline, say), a linked account
        // stays connected with its error and retry; only a revoked or blocked
        // credential, or a link that never synced, reads as disconnected.
        let connected = inner.state.account.is_some()
            && (inner.auth_ready
                || (!restoring
                    && inner.credential_blocked.is_none()
                    && inner.state.last_synced_at.is_some()));
        CalendarSnapshot {
            revision: inner.state.revision,
            configured: self.config.is_some()
                && cfg!(any(target_os = "macos", target_os = "windows")),
            connected,
            meet_connected: inner.state.meet_authorized
                && inner.state.account.is_some()
                && inner.auth_ready,
            connecting: inner.connecting_id.is_some(),
            restoring,
            keychain_blocked: matches!(inner.credential_blocked, Some(Blocked::Keychain)),
            account_label: inner.state.account.clone(),
            calendars: inner.state.calendars.clone(),
            selected_calendars: inner.state.selected.clone(),
            last_synced_at: inner.state.last_synced_at,
            error: inner.error.clone(),
        }
    }
    pub fn begin_connect(&self) -> Result<ConnectRequest, String> {
        self.begin_connection(false)
    }
    pub fn begin_meet_connect(&self) -> Result<ConnectRequest, String> {
        if self.inner.lock().unwrap().state.account.is_none() {
            return Err("먼저 Google 캘린더를 연결해 주세요.".into());
        }
        self.begin_connection(true)
    }
    fn begin_connection(&self, include_meet: bool) -> Result<ConnectRequest, String> {
        if !cfg!(any(target_os = "macos", target_os = "windows")) {
            return Err("Google 캘린더 연결은 macOS와 Windows에서 지원합니다.".into());
        }
        let config = self
            .config
            .as_ref()
            .ok_or("이 버전에는 Google 캘린더 연결 정보가 설정되지 않았습니다.")?;
        let (pending, request) = if include_meet {
            Pending::begin_with_meet(config, true)?
        } else {
            Pending::begin(config)?
        };
        self.cancel_connect();
        let mut inner = self.inner.lock().unwrap();
        inner.cancel = Some(pending.cancelled.clone());
        inner.connecting_id = Some(pending.id.clone());
        inner.authorization_url = Some(request.authorization_url.clone());
        inner.pending = Some(pending);
        inner.error = None;
        inner.state.revision += 1;
        Ok(request)
    }
    pub fn authorization_url(&self, attempt_id: &str) -> Result<String, String> {
        let inner = self.inner.lock().unwrap();
        if inner.connecting_id.as_deref() != Some(attempt_id) {
            return Err("현재 연결 요청이 아닙니다. 다시 시도해 주세요.".into());
        }
        inner
            .authorization_url
            .clone()
            .ok_or_else(|| "Google 연결 정보를 확인 중입니다. 잠시 기다려 주세요.".into())
    }
    pub fn cancel_connect(&self) {
        self.cancel_connect_attempt(None);
    }
    pub fn cancel_connect_attempt(&self, attempt: Option<&str>) {
        let mut inner = self.inner.lock().unwrap();
        if attempt.is_some_and(|attempt| inner.connecting_id.as_deref() != Some(attempt)) {
            return;
        }
        self.connection_epoch.fetch_add(1, Ordering::AcqRel);
        self.generation.fetch_add(1, Ordering::AcqRel);
        if let Some(cancel) = inner.cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        inner.pending = None;
        inner.connecting_id = None;
        inner.authorization_url = None;
        inner.state.revision += 1;
    }
    pub async fn finish_connect(&self, attempt_id: &str) -> Result<CalendarSnapshot, String> {
        let generation = self.generation.load(Ordering::Acquire);
        let pending = {
            let mut inner = self.inner.lock().unwrap();
            if inner
                .pending
                .as_ref()
                .is_none_or(|pending| pending.id != attempt_id)
            {
                return Err("현재 연결 요청이 아닙니다.".into());
            }
            inner.pending.take().unwrap()
        };
        let result = self.complete(&pending, generation).await;
        if generation == self.generation.load(Ordering::Acquire) {
            let mut inner = self.inner.lock().unwrap();
            if inner.connecting_id.as_deref() == Some(attempt_id) {
                inner.connecting_id = None;
                inner.authorization_url = None;
                inner.cancel = None;
                if let Err(message) = &result {
                    inner.error = Some(message.clone());
                }
                inner.state.revision += 1;
            }
        }
        result?;
        Ok(self.snapshot())
    }
    async fn complete(&self, pending: &Pending, generation: u64) -> Result<(), String> {
        let mut authorization = pending.receive().await?;
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.connecting_id.as_deref() == Some(&pending.id) {
                inner.authorization_url = None;
            }
        }
        let result = self
            .complete_authorized(pending, generation, &authorization.code)
            .await;
        authorization
            .page
            .finish(result.as_ref().map(|_| ()).map_err(String::as_str));
        result
    }
    async fn complete_authorized(
        &self,
        pending: &Pending,
        generation: u64,
        code: &str,
    ) -> Result<(), String> {
        let config = self.config.as_ref().ok_or("Google 연결 정보가 없습니다.")?;
        let response = google::exchange(&self.client, config, pending, code)
            .await
            .map_err(|error| error.message())?;
        let calendars = google::calendars(&self.client, &response.access_token)
            .await
            .map_err(|error| error.message())?;
        let account = primary_calendar(&calendars)?.id.clone();
        require_requested_scope(response.scope.as_deref(), pending.include_meet)?;
        let refresh_token = response
            .refresh_token
            .filter(|token| !token.is_empty())
            .ok_or("백그라운드 연결 권한을 받지 못했습니다. 다시 연결해 주세요.")?;
        let _gate = self.token_gate.lock().unwrap();
        let mut inner = self.inner.lock().unwrap();
        if generation != self.generation.load(Ordering::Acquire) {
            return Err("현재 연결 요청이 아닙니다.".into());
        }
        let next = connected_state(&inner.state, &account, calendars, pending.include_meet)?;
        let next = commit_connection(
            inner.state.token_slot,
            next,
            |slot| {
                TokenStore
                    .save(
                        &StoredToken {
                            refresh_token,
                            account_id: account,
                            client_id: config.client_id.clone(),
                        },
                        slot,
                        crate::secret_store::Access::Interactive,
                    )
                    .map_err(|_| {
                        "Google 연결을 저장하지 못했습니다. 다시 연결해 주세요.".to_string()
                    })
            },
            |next| persist(&self.directory, next),
        )?;
        inner.scheduler = Scheduler {
            notices: next.notices.clone(),
            ..Scheduler::default()
        };
        inner.state = next;
        inner.access = Some(Access {
            token: response.access_token,
            expires_at: now_ms().saturating_add(response.expires_in.saturating_mul(1000)),
        });
        inner.auth_ready = true;
        inner.credential_blocked = None;
        inner.stored_token = None;
        inner.pending_token = None;
        inner.needs_sync = true;
        inner.last_attempt = None;
        inner.error = None;
        Ok(())
    }
    pub fn set_calendars(&self, mut ids: Vec<String>) -> Result<CalendarSnapshot, String> {
        ids.sort();
        ids.dedup();
        let mut inner = self.inner.lock().unwrap();
        if !inner.auth_ready || inner.connecting_id.is_some() {
            return Err("Google 연결을 확인한 뒤 캘린더를 선택해 주세요.".into());
        }
        if ids.iter().any(|id| {
            !inner
                .state
                .calendars
                .iter()
                .any(|calendar| &calendar.id == id)
        }) {
            return Err("현재 목록의 캘린더를 선택해 주세요.".into());
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        let mut next = inner.state.clone();
        next.selected = ids;
        next.events.clear();
        next.last_synced_at = None;
        next.revision += 1;
        persist(&self.directory, &next)?;
        inner.state = next;
        inner.needs_sync = true;
        inner.last_attempt = None;
        inner.error = None;
        drop(inner);
        Ok(self.snapshot())
    }
    fn recover_pending(
        inner: &mut Inner,
        save: impl FnOnce(&StoredToken, bool) -> Result<(), crate::secret_store::Error>,
    ) -> Result<(), String> {
        let (slot, token, access) = inner
            .pending_token
            .clone()
            .ok_or("저장할 연결 정보가 없습니다.")?;
        save(&token, slot)
            .map_err(|_| "Google 연결을 저장하지 못했습니다. 다시 시도해 주세요.".to_string())?;
        inner.pending_token = None;
        inner.stored_token = Some((slot, token));
        inner.access = (access.expires_at > now_ms().saturating_add(60_000)).then_some(access);
        Ok(())
    }
    pub async fn sync_explicit(&self) -> Result<CalendarSnapshot, String> {
        {
            let _access = self.access_gate.lock().await;
            let _gate = self.token_gate.lock().unwrap();
            let mut inner = self.inner.lock().unwrap();
            if inner.pending_token.is_some() {
                Self::recover_pending(&mut inner, |token, slot| {
                    TokenStore.save(token, slot, crate::secret_store::Access::Interactive)
                })?;
            } else if matches!(inner.credential_blocked, Some(Blocked::Keychain)) {
                let slot = inner.state.token_slot;
                let token = TokenStore
                    .load(slot, crate::secret_store::Access::Interactive)
                    .map_err(|_| {
                        "Google 연결을 확인하지 못했습니다. 다시 시도해 주세요.".to_string()
                    })?;
                inner.stored_token = Some((slot, token));
            }
            inner.credential_blocked = None;
            inner.error = None;
            inner.needs_sync = true;
            inner.last_attempt = None;
        }
        self.sync_now().await
    }
    pub async fn disconnect(&self) -> Result<CalendarSnapshot, String> {
        self.cancel_connect();
        let _gate = self.token_gate.lock().unwrap();
        let mut inner = self.inner.lock().unwrap();
        inner.state = State {
            schema_version: 1,
            revision: inner.state.revision + 1,
            ..State::default()
        };
        inner.access = None;
        inner.scheduler = Scheduler::default();
        inner.auth_ready = false;
        inner.credential_blocked = None;
        inner.stored_token = None;
        inner.pending_token = None;
        inner.needs_sync = false;
        inner.error = None;
        let saved = persist(&self.directory, &inner.state);
        let removed = TokenStore
            .delete()
            .map_err(|_| "Google 연결을 삭제하지 못했습니다.".to_string());
        if let Err(message) = saved.and(removed) {
            inner.error = Some(message.clone());
            return Err(message);
        }
        drop(inner);
        Ok(self.snapshot())
    }
    pub fn should_sync(&self, now: u64) -> bool {
        let inner = self.inner.lock().unwrap();
        self.config.is_some()
            && inner.state.account.is_some()
            && inner.connecting_id.is_none()
            && inner.credential_blocked.is_none()
            && !self.syncing.load(Ordering::Acquire)
            && inner
                .last_attempt
                .is_none_or(|last| now < last || now.saturating_sub(last) >= RETRY_INTERVAL_MS)
            && (inner.needs_sync
                || inner
                    .state
                    .last_synced_at
                    .is_none_or(|last| now < last || now.saturating_sub(last) >= SYNC_INTERVAL_MS))
    }
    pub async fn sync_now(&self) -> Result<CalendarSnapshot, String> {
        if self.syncing.swap(true, Ordering::AcqRel) {
            return Err("일정을 새로고침하고 있습니다. 잠시 후 다시 시도해 주세요.".into());
        }
        let _guard = SyncGuard(&self.syncing);
        let generation = self.generation.load(Ordering::Acquire);
        let result = self.synchronize(generation).await;
        if generation == self.generation.load(Ordering::Acquire) {
            let mut inner = self.inner.lock().unwrap();
            if let Err(message) = &result {
                inner.error = Some(message.clone());
                inner.needs_sync = true;
            }
            inner.state.revision += 1;
        }
        result?;
        Ok(self.snapshot())
    }
    fn sync_error(&self, generation: u64, error: google::Failure) -> String {
        let mut inner = self.inner.lock().unwrap();
        if generation == self.generation.load(Ordering::Acquire)
            && matches!(error, google::Failure::Reconnect)
        {
            inner.auth_ready = false;
            inner.access = None;
            inner.credential_blocked = Some(Blocked::Revoked(error.message()));
            inner.error = Some(error.message());
            inner.state.revision += 1;
        }
        error.message()
    }
    async fn query_error(&self, context: &QueryContext, error: google::Failure) -> String {
        let _access = self.access_gate.lock().await;
        let mut inner = self.inner.lock().unwrap();
        if matches!(error, google::Failure::Reconnect)
            && context.generation == self.generation.load(Ordering::Acquire)
            && inner
                .access
                .as_ref()
                .is_some_and(|access| access.token == context.token)
        {
            inner.auth_ready = false;
            inner.access = None;
            inner.credential_blocked = Some(Blocked::Revoked(error.message()));
            inner.error = Some(error.message());
            inner.state.revision += 1;
        }
        if matches!(error, google::Failure::Unauthorized)
            && context.generation == self.generation.load(Ordering::Acquire)
            && inner
                .access
                .as_ref()
                .is_some_and(|access| access.token == context.token)
        {
            inner.access = None;
            inner.rejections += 1;
            inner.needs_sync = true;
        }
        error.message()
    }
    fn rejections(&self) -> u64 {
        self.inner.lock().unwrap().rejections
    }
    // A rejected access token gets one retry, which refreshes it. A token that is rejected
    // again right after a successful refresh will not recover by itself, so the user is
    // asked to reconnect instead of seeing the same retry message forever.
    async fn with_token_retry<T, F, Fut>(&self, generation: u64, attempt: F) -> Result<T, String>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T, String>>,
    {
        let before = self.rejections();
        let result = attempt().await;
        if result.is_ok() || self.rejections() == before {
            return result;
        }
        let retried = self.rejections();
        let result = attempt().await;
        if result.is_err() && self.rejections() != retried {
            return Err(self.sync_error(generation, google::Failure::Reconnect));
        }
        result
    }
    fn credential_error(&self, generation: u64, error: crate::secret_store::Error) -> String {
        let message = KEYCHAIN_BLOCKED.to_string();
        if error == crate::secret_store::Error::InteractionRequired {
            let mut inner = self.inner.lock().unwrap();
            if generation == self.generation.load(Ordering::Acquire) {
                inner.credential_blocked = Some(Blocked::Keychain);
                inner.auth_ready = false;
                inner.stored_token = None;
            }
        }
        message
    }
    // The caller holds access_gate, so waiters share both successful loads and denial.
    fn cached_token(
        &self,
        slot: bool,
        generation: u64,
        load: impl FnOnce() -> Result<StoredToken, crate::secret_store::Error>,
    ) -> Result<StoredToken, String> {
        {
            let inner = self.inner.lock().unwrap();
            if generation != self.generation.load(Ordering::Acquire) {
                return Err("캘린더 선택이나 연결이 변경되었습니다.".into());
            }
            if let Some(blocked) = &inner.credential_blocked {
                return Err(blocked.message());
            }
            if let Some((_, token)) = inner
                .stored_token
                .as_ref()
                .filter(|(saved, _)| *saved == slot)
            {
                return Ok(token.clone());
            }
        }
        let token = load().map_err(|error| self.credential_error(generation, error))?;
        let mut inner = self.inner.lock().unwrap();
        if generation != self.generation.load(Ordering::Acquire) {
            return Err("캘린더 선택이나 연결이 변경되었습니다.".into());
        }
        inner.stored_token = Some((slot, token.clone()));
        Ok(token)
    }
    fn finish_refresh(
        &self,
        generation: u64,
        connection_epoch: u64,
        account: &str,
        token_slot: bool,
        stored: StoredToken,
        response: google::TokenResponse,
        save: impl FnOnce(&StoredToken, bool) -> Result<(), crate::secret_store::Error>,
    ) -> Result<String, String> {
        let _gate = self.token_gate.lock().unwrap();
        let mut inner = self.inner.lock().unwrap();
        if connection_epoch != self.connection_epoch.load(Ordering::Acquire)
            || inner.state.account.as_deref() != Some(account)
            || inner.state.token_slot != token_slot
            || self
                .config
                .as_ref()
                .is_none_or(|config| stored.client_id != config.client_id)
        {
            return Err("캘린더 연결이 변경되었습니다.".into());
        }
        if let Some(refresh_token) = response.refresh_token {
            let rotated = StoredToken {
                refresh_token,
                ..stored
            };
            if save(&rotated, token_slot).is_err() {
                let message = KEYCHAIN_BLOCKED.to_string();
                inner.pending_token = Some((
                    token_slot,
                    rotated,
                    Access {
                        token: response.access_token.clone(),
                        expires_at: now_ms()
                            .saturating_add(response.expires_in.saturating_mul(1000)),
                    },
                ));
                inner.credential_blocked = Some(Blocked::Keychain);
                inner.error = Some(message.clone());
                inner.state.revision += 1;
                inner.auth_ready = false;
                inner.stored_token = None;
                return Err(message);
            }
            inner.stored_token = Some((token_slot, rotated));
            inner.pending_token = None;
        }
        if generation != self.generation.load(Ordering::Acquire) {
            return Err("캘린더 선택이나 연결이 변경되었습니다.".into());
        }
        inner.access = Some(Access {
            token: response.access_token.clone(),
            expires_at: now_ms().saturating_add(response.expires_in.saturating_mul(1000)),
        });
        Ok(response.access_token)
    }
    async fn query_context(&self, generation: u64) -> Result<QueryContext, String> {
        let _access = self.access_gate.lock().await;
        let config = self
            .config
            .as_ref()
            .ok_or("이 버전에는 Google 캘린더 연결 정보가 설정되지 않았습니다.")?;
        let (account, selected, token_slot, access, connection_epoch) = {
            let inner = self.inner.lock().unwrap();
            if generation != self.generation.load(Ordering::Acquire) {
                return Err("캘린더 선택이나 연결이 변경되었습니다.".into());
            }
            if inner.connecting_id.is_some() {
                return Err("Google 연결을 확인하고 있습니다.".into());
            }
            if let Some(blocked) = &inner.credential_blocked {
                return Err(blocked.message());
            }
            (
                inner
                    .state
                    .account
                    .clone()
                    .ok_or("먼저 Google 캘린더를 연결해 주세요.")?,
                inner.state.selected.clone(),
                inner.state.token_slot,
                inner
                    .access
                    .as_ref()
                    .filter(|access| access.expires_at > now_ms().saturating_add(60_000))
                    .map(|access| access.token.clone()),
                self.connection_epoch.load(Ordering::Acquire),
            )
        };
        let token = if let Some(token) = access {
            token
        } else {
            let stored = self.cached_token(token_slot, generation, || {
                let _gate = self.token_gate.lock().unwrap();
                TokenStore.load(token_slot, crate::secret_store::Access::Silent)
            })?;
            if stored.account_id != account {
                return Err("저장된 Google 계정 연결이 다릅니다. 다시 연결해 주세요.".into());
            }
            let response = google::refresh(&self.client, config, &stored)
                .await
                .map_err(|error| self.sync_error(generation, error))?;
            self.finish_refresh(
                generation,
                connection_epoch,
                &account,
                token_slot,
                stored,
                response,
                |token, slot| TokenStore.save(token, slot, crate::secret_store::Access::Silent),
            )?
        };
        Ok(QueryContext {
            generation,
            account,
            selected,
            token,
        })
    }

    async fn query_calendars(
        &self,
        context: &QueryContext,
    ) -> Result<(Vec<CalendarChoice>, Vec<CalendarChoice>), String> {
        let (account, selected, token) = (&context.account, &context.selected, &context.token);
        let calendars = match google::calendars(&self.client, token).await {
            Ok(calendars) => calendars,
            Err(error) => return Err(self.query_error(context, error).await),
        };
        if primary_calendar(&calendars)?.id.as_str() != account.as_str() {
            return Err("Google 계정 연결이 변경되었습니다. 다시 연결해 주세요.".into());
        }
        let available = available_selection(selected, &calendars)?;
        let choices = calendars
            .iter()
            .filter(|calendar| available.contains(&calendar.id))
            .cloned()
            .collect();
        Ok((calendars, choices))
    }

    async fn synchronize(&self, generation: u64) -> Result<(), String> {
        self.with_token_retry(generation, || self.synchronize_once(generation))
            .await
    }

    async fn synchronize_once(&self, generation: u64) -> Result<(), String> {
        self.inner.lock().unwrap().last_attempt = Some(now_ms());
        let context = self.query_context(generation).await?;
        let (calendars, choices) = self.query_calendars(&context).await?;
        let (token, account) = (&context.token, &context.account);
        let events = match google::events(&self.client, token, account, &choices, now_ms()).await {
            Ok(events) => events,
            Err(error) => return Err(self.query_error(&context, error).await),
        };
        self.commit_synchronization(generation, calendars, events, now_ms())
    }

    fn commit_synchronization(
        &self,
        generation: u64,
        calendars: Vec<CalendarChoice>,
        events: Vec<Occurrence>,
        synced_at: u64,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        if generation != self.generation.load(Ordering::Acquire) {
            return Err("캘린더 선택이나 연결이 변경되었습니다.".into());
        }
        let mut next = inner.state.clone();
        let selected = available_selection(&next.selected, &calendars)?;
        if selected != next.selected {
            self.generation.fetch_add(1, Ordering::AcqRel);
            next.selected = selected;
        }
        next.calendars = calendars;
        next.events = events;
        next.last_synced_at = Some(synced_at);
        next.revision += 1;
        persist(&self.directory, &next)?;
        inner.state = next;
        inner.auth_ready = true;
        inner.needs_sync = false;
        inner.woke = false;
        inner.error = None;
        Ok(())
    }

    pub async fn browse_day(
        &self,
        day: String,
        time_zone: String,
        expected_scope: Option<u64>,
    ) -> Result<MeetingBrowse, String> {
        let generation = self.generation.load(Ordering::Acquire);
        self.with_token_retry(generation, || {
            self.browse_day_once(&day, &time_zone, expected_scope)
        })
        .await
    }

    async fn browse_day_once(
        &self,
        day: &str,
        time_zone: &str,
        expected_scope: Option<u64>,
    ) -> Result<MeetingBrowse, String> {
        let (day, time_zone) = (day.to_owned(), time_zone.to_owned());
        let (start_at, end_at) = day_bounds(&day, &time_zone)?;
        let generation = self.generation.load(Ordering::Acquire);
        if expected_scope.is_some_and(|expected| expected != generation) {
            return Err("캘린더 선택이나 연결이 변경되었습니다. 일정을 다시 불러와 주세요.".into());
        }
        let context = self.query_context(generation).await?;
        let (_, choices) = self.query_calendars(&context).await?;
        let mut occurrences = match google::events_between(
            &self.client,
            &context.token,
            &context.account,
            &choices,
            start_at,
            end_at,
        )
        .await
        {
            Ok(events) => events,
            Err(error) => return Err(self.query_error(&context, error).await),
        };
        occurrences.sort_by(|a, b| a.start_at.cmp(&b.start_at).then_with(|| a.key.cmp(&b.key)));
        self.finish_browse(context, day, time_zone, occurrences)
    }

    fn finish_browse(
        &self,
        context: QueryContext,
        day: String,
        time_zone: String,
        occurrences: Vec<Occurrence>,
    ) -> Result<MeetingBrowse, String> {
        let inner = self.inner.lock().unwrap();
        self.require_browse_context(&inner, &context)?;
        Ok(MeetingBrowse {
            calendar: self.snapshot_inner(&inner),
            occurrences,
            day,
            time_zone,
            scope_revision: context.generation,
            context,
        })
    }

    fn require_browse_context(&self, inner: &Inner, context: &QueryContext) -> Result<(), String> {
        if context.generation != self.generation.load(Ordering::Acquire)
            || !inner.auth_ready
            || inner.connecting_id.is_some()
            || inner.state.account.as_ref() != Some(&context.account)
            || inner.state.selected != context.selected
        {
            return Err("캘린더 선택이나 연결이 변경되었습니다. 일정을 다시 불러와 주세요.".into());
        }
        Ok(())
    }

    pub fn with_browsed_occurrence<T>(
        &self,
        browse: &MeetingBrowse,
        key: &str,
        action: impl FnOnce(&Occurrence) -> Result<T, String>,
    ) -> Result<T, String> {
        let inner = self.inner.lock().unwrap();
        self.require_browse_context(&inner, &browse.context)?;
        let event = browse
            .occurrences
            .iter()
            .find(|event| event.key == key)
            .ok_or("일정이 변경되었거나 삭제되었습니다. 새로고침해 주세요.")?;
        action(event)
    }
    pub fn tick(&self, now: u64, recording_busy: bool) -> Result<Vec<ReminderChange>, String> {
        let mut inner = self.inner.lock().unwrap();
        if inner
            .last_tick
            .is_some_and(|last| now < last || now.saturating_sub(last) > 10_000)
        {
            inner.needs_sync = true;
            inner.woke = true;
            inner.last_attempt = None;
        }
        inner.last_tick = Some(now);
        // A failed sync or a connection in progress leaves recent events usable, so a
        // reminder on screen stays; age, a wake or a lost credential still hide it.
        let fresh = inner.auth_ready
            && !inner.woke
            && inner
                .state
                .last_synced_at
                .is_some_and(|last| now >= last && now.saturating_sub(last) <= FRESH_MS);
        let events = inner.state.events.clone();
        let previous = inner.scheduler.clone();
        let changes = inner.scheduler.tick(&events, now, fresh, recording_busy);
        if !changes.is_empty() {
            let mut next = inner.state.clone();
            next.notices = inner.scheduler.notices.clone();
            next.revision += 1;
            if let Err(message) = persist(&self.directory, &next) {
                inner.scheduler = previous;
                return Err(message);
            }
            inner.state = next;
        }
        Ok(changes)
    }
    pub fn dismiss(&self, key: &str) -> Result<Option<ReminderChange>, String> {
        let mut inner = self.inner.lock().unwrap();
        let previous = inner.scheduler.clone();
        let change = inner.scheduler.dismiss(key);
        let mut next = inner.state.clone();
        next.notices = inner.scheduler.notices.clone();
        next.revision += 1;
        if let Err(message) = persist(&self.directory, &next) {
            inner.scheduler = previous;
            return Err(message);
        }
        inner.state = next;
        Ok(change)
    }
    pub fn reminder(&self, key: &str) -> Option<Occurrence> {
        self.inner.lock().unwrap().scheduler.visible(key)
    }
    pub fn reminder_ticket(&self, key: &str, now: u64) -> Result<ReminderTicket, String> {
        let inner = self.inner.lock().unwrap();
        current_reminder(&inner, key, now)?;
        Ok(ReminderTicket {
            key: key.into(),
            generation: self.generation.load(Ordering::Acquire),
        })
    }
    pub fn with_current_reminder<T>(
        &self,
        ticket: &ReminderTicket,
        now: u64,
        action: impl FnOnce(&Occurrence) -> Result<T, String>,
    ) -> Result<T, String> {
        let inner = self.inner.lock().unwrap();
        if ticket.generation != self.generation.load(Ordering::Acquire) {
            return Err("캘린더 선택이나 연결이 변경되었습니다. 현재 일정을 확인해 주세요.".into());
        }
        let event = current_reminder(&inner, &ticket.key, now)?;
        // Selection/disconnect/dismiss cannot pass this lock between validation
        // and the core's atomic busy-check/create/start operation.
        action(event)
    }
    pub fn reminders(&self) -> Vec<Occurrence> {
        let mut events: Vec<_> = self
            .inner
            .lock()
            .unwrap()
            .scheduler
            .visible
            .values()
            .cloned()
            .collect();
        events.sort_by(|a, b| a.start_at.cmp(&b.start_at).then_with(|| a.key.cmp(&b.key)));
        events
    }
}

fn current_reminder<'a>(inner: &'a Inner, key: &str, now: u64) -> Result<&'a Occurrence, String> {
    let fresh = inner.auth_ready
        && inner.state.account.is_some()
        && !inner.state.selected.is_empty()
        && inner.connecting_id.is_none()
        && !inner.needs_sync
        && inner.error.is_none()
        && inner
            .state
            .last_synced_at
            .is_some_and(|last| now >= last && now.saturating_sub(last) <= FRESH_MS)
        && inner
            .last_tick
            .is_some_and(|last| now >= last && now.saturating_sub(last) <= 10_000);
    if !fresh
        || inner.scheduler.visible(key).is_none()
        || inner
            .scheduler
            .notices
            .get(key)
            .is_some_and(|notice| notice.dismissed)
    {
        return Err("현재 시작할 일정 알림이 아닙니다. 일정을 다시 확인해 주세요.".into());
    }
    inner
        .state
        .events
        .iter()
        .find(|event| event.key == key && schedule::reminder_window(event, now))
        .ok_or_else(|| {
            "일정이 변경되었거나 알림 시간이 지났습니다. 현재 일정을 확인해 주세요.".into()
        })
}

fn commit_connection(
    current_slot: bool,
    mut next: State,
    stage_token: impl FnOnce(bool) -> Result<(), String>,
    save_state: impl FnOnce(&State) -> Result<(), String>,
) -> Result<State, String> {
    // A failed state write leaves the old slot and old state untouched. There
    // are only two known slots; a failed stage is replaced by the next attempt
    // and both are removed by explicit disconnect.
    next.token_slot = !current_slot;
    stage_token(next.token_slot)?;
    save_state(&next)?;
    Ok(next)
}

fn require_requested_scope(scope: Option<&str>, include_meet: bool) -> Result<(), String> {
    if include_meet
        && !scope.is_some_and(|scope| {
            scope
                .split_whitespace()
                .any(|scope| scope == oauth::MEET_SCOPE)
        })
    {
        return Err(
            "Google Meet 참석자 읽기 권한이 승인되지 않았습니다. 기존 캘린더 연결은 유지됩니다."
                .into(),
        );
    }
    Ok(())
}

fn connected_state(
    previous: &State,
    account: &str,
    calendars: Vec<CalendarChoice>,
    include_meet: bool,
) -> Result<State, String> {
    let primary = primary_calendar(&calendars)?;
    if primary.id != account {
        return Err("Google 계정의 기본 캘린더를 확인하지 못했습니다.".into());
    }
    if include_meet && previous.account.as_deref() != Some(account) {
        return Err("캘린더에 연결한 것과 같은 Google 계정으로 Meet을 연결해 주세요.".into());
    }
    let mut next = previous.clone();
    if next.account.as_deref() != Some(account) {
        next.selected = vec![primary.id.clone()];
        next.notices.clear();
    } else {
        next.selected = available_selection(&next.selected, &calendars)?;
    }
    next.account = Some(account.into());
    next.meet_authorized = include_meet;
    next.calendars = calendars;
    if !include_meet {
        next.events.clear();
        next.last_synced_at = None;
    }
    next.revision += 1;
    Ok(next)
}

// Selected calendars the account still lists. A calendar that was deleted or
// unshared is dropped instead of failing every sync; if all were, the primary.
fn available_selection(
    selected: &[String],
    calendars: &[CalendarChoice],
) -> Result<Vec<String>, String> {
    let available = selected
        .iter()
        .filter(|id| calendars.iter().any(|calendar| &calendar.id == *id))
        .cloned()
        .collect::<Vec<_>>();
    if available.is_empty() && !selected.is_empty() {
        return Ok(vec![primary_calendar(calendars)?.id.clone()]);
    }
    Ok(available)
}

fn primary_calendar(calendars: &[CalendarChoice]) -> Result<&CalendarChoice, String> {
    let mut primary = calendars.iter().filter(|calendar| calendar.primary);
    match (primary.next(), primary.next()) {
        (Some(calendar), None) if !calendar.id.is_empty() => Ok(calendar),
        _ => Err("Google 계정의 기본 캘린더를 확인하지 못했습니다.".into()),
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}
fn checked_directory(directory: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(directory) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(true),
        Ok(_) => Err("캘린더 저장 폴더를 확인하지 못했습니다.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("캘린더 저장 폴더를 읽지 못했습니다.".into()),
    }
}
fn load(directory: &Path) -> Result<State, String> {
    if !checked_directory(directory)? {
        return Ok(State::default());
    }
    let path = directory.join("state.json");
    match std::fs::symlink_metadata(&path) {
        Ok(meta)
            if meta.is_file()
                && !meta.file_type().is_symlink()
                && meta.len() <= MAX_STATE_BYTES =>
        {
            ()
        }
        Ok(_) => return Err("캘린더 저장 정보를 확인하지 못했습니다.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(State::default()),
        Err(_) => return Err("캘린더 저장 정보를 읽지 못했습니다.".into()),
    }
    let bytes = std::fs::read(path).map_err(|_| "캘린더 저장 정보를 읽지 못했습니다.")?;
    let state: State =
        serde_json::from_slice(&bytes).map_err(|_| "캘린더 저장 정보가 올바르지 않습니다.")?;
    if state.schema_version != 1 {
        return Err("지원하지 않는 캘린더 저장 형식입니다.".into());
    }
    Ok(state)
}
fn persist(directory: &Path, state: &State) -> Result<(), String> {
    use std::io::Write;
    if !checked_directory(directory)? {
        std::fs::create_dir(directory).map_err(|_| "캘린더 저장 폴더를 만들지 못했습니다.")?;
    }
    checked_directory(directory)?;
    let path = directory.join("state.json");
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err("캘린더 저장 파일을 확인하지 못했습니다.".into());
        }
    }
    let bytes = serde_json::to_vec(state).map_err(|_| "캘린더 저장 정보를 만들지 못했습니다.")?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err("캘린더 저장 정보가 한도를 넘었습니다.".into());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(directory)
        .map_err(|_| "캘린더 저장을 준비하지 못했습니다.")?;
    temporary
        .write_all(&bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| "캘린더 정보를 저장하지 못했습니다.")?;
    temporary
        .persist(&path)
        .map_err(|_| "캘린더 정보를 교체하지 못했습니다.")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn calendar_choice(id: &str, primary: bool) -> CalendarChoice {
        CalendarChoice {
            id: id.into(),
            name: "Synthetic calendar".into(),
            primary,
        }
    }
    #[test]
    fn first_connection_selects_only_the_provider_identified_primary_calendar() {
        let calendars = vec![
            calendar_choice("looks-like-an-account@example.invalid", false),
            calendar_choice("opaque-primary-id", true),
            calendar_choice("team-calendar", false),
        ];
        let connected =
            connected_state(&State::default(), "opaque-primary-id", calendars, false).unwrap();
        assert_eq!(connected.account.as_deref(), Some("opaque-primary-id"));
        assert_eq!(connected.selected, vec!["opaque-primary-id"]);
    }
    #[test]
    fn same_account_reconnection_preserves_explicit_selection_including_none() {
        for selected in [
            vec![],
            vec!["team-calendar".into()],
            vec!["primary-id".into(), "team-calendar".into()],
        ] {
            for include_meet in [false, true] {
                let previous = State {
                    account: Some("primary-id".into()),
                    selected: selected.clone(),
                    ..State::default()
                };
                let next = connected_state(
                    &previous,
                    "primary-id",
                    vec![
                        calendar_choice("team-calendar", false),
                        calendar_choice("primary-id", true),
                        calendar_choice("newly-visible-calendar", false),
                    ],
                    include_meet,
                )
                .unwrap();
                assert_eq!(next.selected, selected);
            }
        }
    }
    #[test]
    fn another_account_gets_its_primary_without_inheriting_shared_selection_or_reminders() {
        let temp = tempfile::tempdir().unwrap();
        let service = reminder_service(temp.path());
        let inner = service.inner.lock().unwrap();
        let mut previous = inner.state.clone();
        previous.notices.insert(
            "old-account-notice".into(),
            Notice {
                retain_until: 2_000_000,
                dismissed: true,
            },
        );
        assert!(!previous.selected.is_empty());
        assert!(!previous.events.is_empty());
        assert!(!previous.notices.is_empty());
        let next = connected_state(
            &previous,
            "new-primary-id",
            vec![
                calendar_choice("one", false),
                calendar_choice("new-primary-id", true),
            ],
            false,
        )
        .unwrap();
        assert_eq!(next.selected, vec!["new-primary-id"]);
        assert!(next.events.is_empty());
        assert!(next.notices.is_empty());
        assert!(next.last_synced_at.is_none());
        assert!(!next.meet_authorized);
    }
    #[test]
    fn missing_ambiguous_or_mismatched_primary_never_creates_a_default_selection() {
        for calendars in [
            vec![calendar_choice("primary-id", false)],
            vec![
                calendar_choice("primary-id", true),
                calendar_choice("another-primary", true),
            ],
            vec![calendar_choice("another-primary", true)],
        ] {
            let previous = State::default();
            assert!(connected_state(&previous, "primary-id", calendars, false).is_err());
            assert!(previous.account.is_none());
            assert!(previous.selected.is_empty());
        }
    }
    #[test]
    fn explicitly_cleared_selection_stays_empty_after_restart_and_reconnect() {
        let temp = tempfile::tempdir().unwrap();
        let service = CalendarService::new(temp.path().into()).unwrap();
        {
            let mut inner = service.inner.lock().unwrap();
            inner.state = connected_state(
                &State {
                    schema_version: 1,
                    ..State::default()
                },
                "primary-id",
                vec![
                    calendar_choice("primary-id", true),
                    calendar_choice("team-calendar", false),
                ],
                false,
            )
            .unwrap();
            inner.auth_ready = true;
        }
        assert_eq!(service.snapshot().selected_calendars, vec!["primary-id"]);
        assert!(service
            .set_calendars(vec![])
            .unwrap()
            .selected_calendars
            .is_empty());
        service
            .commit_synchronization(
                service.generation.load(Ordering::Acquire),
                vec![
                    calendar_choice("newly-visible-calendar", false),
                    calendar_choice("primary-id", true),
                ],
                vec![],
                1_000_000,
            )
            .unwrap();
        assert!(service.snapshot().selected_calendars.is_empty());
        let restored = CalendarService::new(temp.path().into()).unwrap();
        assert!(restored.snapshot().selected_calendars.is_empty());
        let inner = restored.inner.lock().unwrap();
        let next = connected_state(
            &inner.state,
            "primary-id",
            inner.state.calendars.clone(),
            false,
        )
        .unwrap();
        assert!(next.selected.is_empty());
    }
    #[test]
    fn optional_meet_authorization_requires_explicit_grant_and_preserves_calendar_data() {
        let temp = tempfile::tempdir().unwrap();
        let service = reminder_service(temp.path());
        let inner = service.inner.lock().unwrap();
        let previous = &inner.state;
        let encoded = serde_json::to_vec(previous).unwrap();
        assert!(require_requested_scope(None, true).is_err());
        assert!(require_requested_scope(Some(oauth::SCOPES), true).is_err());
        assert!(
            require_requested_scope(Some(&format!("{}suffix", oauth::MEET_SCOPE)), true).is_err()
        );
        assert!(require_requested_scope(
            Some(&format!("{} {}", oauth::SCOPES, oauth::MEET_SCOPE)),
            true
        )
        .is_ok());
        assert!(require_requested_scope(None, false).is_ok());
        assert!(connected_state(
            previous,
            "another synthetic account",
            previous.calendars.clone(),
            true
        )
        .is_err());
        assert_eq!(serde_json::to_vec(previous).unwrap(), encoded);
        let next = connected_state(
            previous,
            previous.account.as_deref().unwrap(),
            previous.calendars.clone(),
            true,
        )
        .unwrap();
        assert!(next.meet_authorized);
        assert_eq!(next.selected, previous.selected);
        assert_eq!(next.events, previous.events);
        assert_eq!(next.last_synced_at, previous.last_synced_at);
        assert_eq!(
            serde_json::to_vec(&next.notices).unwrap(),
            serde_json::to_vec(&previous.notices).unwrap()
        );
        assert_eq!(next.token_slot, previous.token_slot);
    }
    #[test]
    fn empty_read_does_not_create_files_and_snapshot_does_not_fake_connection() {
        let temp = tempfile::tempdir().unwrap();
        let service = CalendarService::new(temp.path().into()).unwrap();
        assert!(!service.snapshot().connected);
        assert!(!service.snapshot().connecting);
        assert!(!temp.path().join("calendar").exists());
        assert!(!service.should_sync(now_ms()));
    }
    #[test]
    fn saved_state_omits_tokens_and_resume_requires_a_successful_sync() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("calendar");
        let state = State {
            schema_version: 1,
            account: Some("synthetic-account".into()),
            last_synced_at: Some(1000),
            ..State::default()
        };
        persist(&directory, &state).unwrap();
        let saved = std::fs::read(directory.join("state.json")).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&saved).unwrap();
        assert!(value.get("accessToken").is_none() && value.get("refreshToken").is_none());
        let mut service = CalendarService::new(temp.path().into()).unwrap();
        service.config = Some(Config {
            client_id: "synthetic.apps.googleusercontent.com".into(),
            client_secret: None,
        });
        assert!(service.snapshot().restoring && !service.snapshot().connected);
        assert!(service.tick(2000, false).unwrap().is_empty());
        assert_eq!(saved, std::fs::read(directory.join("state.json")).unwrap());
    }
    #[test]
    fn invalid_calendar_choice_does_not_cancel_a_current_operation() {
        let temp = tempfile::tempdir().unwrap();
        let service = CalendarService::new(temp.path().into()).unwrap();
        service.inner.lock().unwrap().auth_ready = true;
        let generation = service.generation.load(Ordering::Acquire);
        assert!(service.set_calendars(vec!["unknown".into()]).is_err());
        assert_eq!(service.generation.load(Ordering::Acquire), generation);
        assert!(!temp.path().join("calendar").exists());
    }
    #[test]
    fn browser_reopen_is_bound_to_the_active_attempt_and_cleared_on_cancel() {
        let temp = tempfile::tempdir().unwrap();
        let service = CalendarService::new(temp.path().into()).unwrap();
        {
            let mut inner = service.inner.lock().unwrap();
            inner.connecting_id = Some("current".into());
            inner.authorization_url = Some("https://accounts.google.com/synthetic".into());
        }
        assert!(service.authorization_url("old").is_err());
        assert!(service.authorization_url("current").is_ok());
        service.cancel_connect_attempt(Some("current"));
        assert!(service.authorization_url("current").is_err());
    }
    #[test]
    fn late_cancel_does_not_cancel_a_newer_browser_attempt() {
        let temp = tempfile::tempdir().unwrap();
        let service = CalendarService::new(temp.path().into()).unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let mut inner = service.inner.lock().unwrap();
            inner.connecting_id = Some("new-attempt".into());
            inner.cancel = Some(cancelled.clone());
        }
        service.cancel_connect_attempt(Some("old-attempt"));
        assert!(!cancelled.load(Ordering::Acquire));
        assert_eq!(service.generation.load(Ordering::Acquire), 0);
        service.cancel_connect_attempt(Some("new-attempt"));
        assert!(cancelled.load(Ordering::Acquire));
        assert!(service.inner.lock().unwrap().connecting_id.is_none());
    }
    #[test]
    fn a_stale_auth_failure_does_not_disable_a_new_connection() {
        let temp = tempfile::tempdir().unwrap();
        let service = CalendarService::new(temp.path().into()).unwrap();
        service.inner.lock().unwrap().auth_ready = true;
        service.generation.store(2, Ordering::Release);
        service.sync_error(1, google::Failure::Reconnect);
        assert!(service.inner.lock().unwrap().auth_ready);
        service.sync_error(2, google::Failure::Reconnect);
        assert!(!service.inner.lock().unwrap().auth_ready);
    }
    #[test]
    fn wake_and_clock_jump_require_a_new_sync_before_reminders() {
        let temp = tempfile::tempdir().unwrap();
        let service = CalendarService::new(temp.path().into()).unwrap();
        {
            let mut inner = service.inner.lock().unwrap();
            inner.auth_ready = true;
            inner.needs_sync = false;
            inner.state.last_synced_at = Some(1_000_000);
            inner.last_tick = Some(1_000_000);
            inner.state.events = vec![Occurrence {
                key: "one".into(),
                title: "Synthetic".into(),
                start_at: 1_300_000,
                end_at: 1_500_000,
                calendar_name: "Synthetic".into(),
                join_urls: vec![],
            }];
        }
        assert!(service.tick(1_020_000, false).unwrap().is_empty());
        assert!(service.inner.lock().unwrap().needs_sync);
        assert!(service.tick(900_000, false).unwrap().is_empty());
        assert!(service.inner.lock().unwrap().needs_sync);
    }
    #[test]
    fn a_failed_sync_keeps_a_reminder_already_on_screen() {
        let temp = tempfile::tempdir().unwrap();
        let service = CalendarService::new(temp.path().into()).unwrap();
        {
            let mut inner = service.inner.lock().unwrap();
            inner.auth_ready = true;
            inner.needs_sync = false;
            inner.state.last_synced_at = Some(1_000_000);
            inner.last_tick = Some(1_000_000);
            inner.state.events = vec![Occurrence {
                key: "one".into(),
                title: "Synthetic".into(),
                start_at: 1_200_000,
                end_at: 1_500_000,
                calendar_name: "Synthetic".into(),
                join_urls: vec![],
            }];
        }
        assert_eq!(service.tick(1_005_000, false).unwrap()[0].kind, "show");
        {
            let mut inner = service.inner.lock().unwrap();
            inner.error = Some("네트워크".into());
            inner.needs_sync = true;
        }
        assert!(service.tick(1_010_000, false).unwrap().is_empty());
    }
    #[test]
    fn corrupt_calendar_state_is_reported_without_overwriting_it() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("calendar");
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("state.json");
        std::fs::write(&path, b"synthetic corrupt state").unwrap();
        assert!(CalendarService::new(temp.path().into()).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"synthetic corrupt state");
    }
    #[test]
    fn failed_account_state_commit_preserves_the_active_grant_in_either_slot() {
        use std::cell::RefCell;
        for active in [false, true] {
            let slots = RefCell::new([None::<String>, None::<String>]);
            slots.borrow_mut()[usize::from(active)] = Some("previous synthetic grant".into());
            let previous = State {
                schema_version: 1,
                token_slot: active,
                account: Some("previous account".into()),
                ..State::default()
            };
            let mut next = previous.clone();
            next.account = Some("new account".into());
            let result = commit_connection(
                active,
                next,
                |slot| {
                    slots.borrow_mut()[usize::from(slot)] = Some("new synthetic grant".into());
                    Ok(())
                },
                |_| Err("synthetic state save failure".into()),
            );
            assert!(result.is_err());
            assert_eq!(previous.account.as_deref(), Some("previous account"));
            assert_eq!(
                slots.borrow()[usize::from(previous.token_slot)].as_deref(),
                Some("previous synthetic grant")
            );
        }
    }
    #[test]
    fn committed_account_points_to_its_staged_grant_and_keeps_the_prior_slot() {
        use std::cell::RefCell;
        let slots = RefCell::new([Some("previous synthetic grant".to_string()), None]);
        let saved = RefCell::new(None::<State>);
        let next = State {
            schema_version: 1,
            account: Some("new account".into()),
            ..State::default()
        };
        let committed = commit_connection(
            false,
            next,
            |slot| {
                slots.borrow_mut()[usize::from(slot)] = Some("new synthetic grant".into());
                Ok(())
            },
            |next| {
                *saved.borrow_mut() = Some(next.clone());
                Ok(())
            },
        )
        .unwrap();
        assert!(committed.token_slot);
        assert_eq!(
            saved.borrow().as_ref().unwrap().token_slot,
            committed.token_slot
        );
        assert_eq!(
            slots.borrow()[usize::from(committed.token_slot)].as_deref(),
            Some("new synthetic grant")
        );
        assert_eq!(
            slots.borrow()[0].as_deref(),
            Some("previous synthetic grant")
        );
    }
    #[tokio::test]
    async fn credential_denial_is_shared_and_successful_load_is_cached() {
        use std::sync::atomic::AtomicUsize;
        let temp = tempfile::tempdir().unwrap();
        let mut service = Arc::new(CalendarService::new(temp.path().to_path_buf()).unwrap());
        Arc::get_mut(&mut service).unwrap().config = Some(Config {
            client_id: "synthetic".into(),
            client_secret: None,
        });
        service.inner.lock().unwrap().state.account = Some("fixture".into());
        assert!(service.should_sync(1));
        let calls = Arc::new(AtomicUsize::new(0));
        let run = |service: Arc<CalendarService>, calls: Arc<AtomicUsize>| async move {
            let _gate = service.access_gate.lock().await;
            service.cached_token(false, 0, || {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(crate::secret_store::Error::InteractionRequired)
            })
        };
        let (first, second) = tokio::join!(
            run(service.clone(), calls.clone()),
            run(service.clone(), calls.clone())
        );
        assert!(first.is_err() && second.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!service.should_sync(1));
        let _ = service.snapshot();
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Successful explicit reconnection resets the denial state.
        service.inner.lock().unwrap().credential_blocked = None;
        let token = StoredToken {
            refresh_token: "synthetic".into(),
            account_id: "fixture".into(),
            client_id: "client".into(),
        };
        let _gate = service.access_gate.lock().await;
        for _ in 0..2 {
            assert!(service
                .cached_token(false, 0, || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(token.clone())
                })
                .is_ok());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn rotated_token_denied_save_is_preserved_until_explicit_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let service = CalendarService::new(temp.path().to_path_buf()).unwrap();
        let previous = StoredToken {
            refresh_token: "old".into(),
            account_id: "fixture".into(),
            client_id: "client".into(),
        };
        let rotated = StoredToken {
            refresh_token: "rotated".into(),
            ..previous.clone()
        };
        let mut persisted = previous.clone();
        let mut inner = service.inner.lock().unwrap();
        inner.pending_token = Some((
            false,
            rotated.clone(),
            Access {
                token: "accepted-access".into(),
                expires_at: now_ms() + 120_000,
            },
        ));
        inner.credential_blocked = Some(Blocked::Keychain);
        assert!(CalendarService::recover_pending(&mut inner, |_, _| Err(
            crate::secret_store::Error::InteractionRequired
        ))
        .is_err());
        assert!(persisted == previous);
        assert!(inner.pending_token.is_some());
        assert!(inner.credential_blocked.is_some());
        CalendarService::recover_pending(&mut inner, |token, _| {
            persisted = token.clone();
            Ok(())
        })
        .unwrap();
        assert!(persisted == rotated);
        assert!(inner.pending_token.is_none());
        assert_eq!(inner.access.as_ref().unwrap().token, "accepted-access");
        assert!(inner.access.as_ref().unwrap().expires_at > now_ms() + 60_000);
    }

    #[tokio::test]
    async fn delayed_refresh_preserves_rotation_after_selection_change_only() {
        let temp = tempfile::tempdir().unwrap();
        let mut service = CalendarService::new(temp.path().to_path_buf()).unwrap();
        service.config = Some(Config {
            client_id: "client".into(),
            client_secret: None,
        });
        service.inner.lock().unwrap().state.account = Some("fixture".into());
        let old = StoredToken {
            refresh_token: "old".into(),
            account_id: "fixture".into(),
            client_id: "client".into(),
        };
        let response = || google::TokenResponse {
            access_token: "access".into(),
            refresh_token: Some("rotated".into()),
            expires_in: 3600,
            scope: None,
            token_type: "Bearer".into(),
        };
        // A mock HTTP response is held while the selected calendars change.
        let generation = service.generation.load(Ordering::Acquire);
        let epoch = service.connection_epoch.load(Ordering::Acquire);
        let (send_response, receive_response) = tokio::sync::oneshot::channel();
        let mut saved = None;
        let request = async {
            let response = receive_response.await.unwrap();
            service.finish_refresh(
                generation,
                epoch,
                "fixture",
                false,
                old.clone(),
                response,
                |token, _| {
                    saved = Some(token.clone());
                    Ok(())
                },
            )
        };
        let change_selection = async {
            service.generation.fetch_add(1, Ordering::AcqRel);
            send_response.send(response()).ok().unwrap();
        };
        let (result, ()) = tokio::join!(request, change_selection);
        assert!(result.is_err());
        assert_eq!(saved.unwrap().refresh_token, "rotated");
        assert!(service.inner.lock().unwrap().access.is_none());

        // If the silent save is denied, the rotated token stays pending even
        // though this request's selection generation is stale.
        assert!(service
            .finish_refresh(
                generation,
                epoch,
                "fixture",
                false,
                old.clone(),
                response(),
                |_, _| Err(crate::secret_store::Error::InteractionRequired)
            )
            .is_err());
        {
            let inner = service.inner.lock().unwrap();
            assert_eq!(
                inner.pending_token.as_ref().unwrap().1.refresh_token,
                "rotated"
            );
            assert!(inner.credential_blocked.is_some());
            assert!(inner.access.is_none());
        }

        // A disconnected or replaced identity never writes the old slot.
        service.connection_epoch.fetch_add(1, Ordering::AcqRel);
        let mut wrote = false;
        assert!(service
            .finish_refresh(
                generation,
                epoch,
                "fixture",
                false,
                old,
                response(),
                |_, _| {
                    wrote = true;
                    Ok(())
                }
            )
            .is_err());
        assert!(!wrote);
    }

    #[test]
    fn expired_pending_access_does_not_prevent_rotated_token_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let service = CalendarService::new(temp.path().to_path_buf()).unwrap();
        let token = StoredToken {
            refresh_token: "rotated".into(),
            account_id: "fixture".into(),
            client_id: "client".into(),
        };
        let mut inner = service.inner.lock().unwrap();
        inner.pending_token = Some((
            false,
            token.clone(),
            Access {
                token: "expired".into(),
                expires_at: 0,
            },
        ));
        let mut saved = None;
        CalendarService::recover_pending(&mut inner, |value, _| {
            saved = Some(value.clone());
            Ok(())
        })
        .unwrap();
        assert!(saved.unwrap() == token);
        assert!(inner.pending_token.is_none());
        assert!(inner.access.is_none());
        assert!(inner.stored_token.as_ref().unwrap().1 == token);
    }

    fn reminder_service(root: &Path) -> CalendarService {
        let service = CalendarService::new(root.into()).unwrap();
        {
            let mut inner = service.inner.lock().unwrap();
            inner.auth_ready = true;
            inner.needs_sync = false;
            inner.state.account = Some("one".into());
            inner.state.calendars = vec![CalendarChoice {
                id: "one".into(),
                name: "Synthetic".into(),
                primary: true,
            }];
            inner.state.selected = vec!["one".into()];
            inner.state.last_synced_at = Some(1_000_000);
            inner.last_tick = Some(1_000_000);
            let events = vec![Occurrence {
                key: "current".into(),
                title: "Original title".into(),
                start_at: 1_120_000,
                end_at: 1_300_000,
                calendar_name: "Synthetic".into(),
                join_urls: vec![],
            }];
            inner.scheduler.tick(&events, 1_000_000, true, false);
            inner.state.events = events;
        }
        service
    }
    #[test]
    fn post_worker_validation_rejects_selection_changes_and_cancelled_events_before_creation() {
        for changed in ["selection", "cancelled", "dismissed", "connection"] {
            let temp = tempfile::tempdir().unwrap();
            let service = reminder_service(temp.path());
            let ticket = service.reminder_ticket("current", 1_000_000).unwrap();
            match changed {
                "selection" => {
                    service.set_calendars(vec![]).unwrap();
                }
                "cancelled" => service.inner.lock().unwrap().state.events.clear(),
                "dismissed" => {
                    service.dismiss("current").unwrap();
                }
                "connection" => {
                    service.cancel_connect();
                    service.inner.lock().unwrap().state.account = None;
                }
                _ => unreachable!(),
            }
            let created = AtomicBool::new(false);
            assert!(
                service
                    .with_current_reminder(&ticket, 1_000_001, |_| {
                        created.store(true, Ordering::Release);
                        Ok(())
                    })
                    .is_err(),
                "{changed}"
            );
            assert!(!created.load(Ordering::Acquire), "{changed}");
        }
    }
    #[test]
    fn post_worker_validation_uses_current_title_and_rejects_stale_or_expired_occurrences() {
        let temp = tempfile::tempdir().unwrap();
        let service = reminder_service(temp.path());
        let ticket = service.reminder_ticket("current", 1_000_000).unwrap();
        service.inner.lock().unwrap().state.events[0].title = "Updated title".into();
        assert_eq!(
            service
                .with_current_reminder(&ticket, 1_000_001, |event| Ok(event.title.clone()))
                .unwrap(),
            "Updated title"
        );
        for now in [999_999, 1_020_000, 1_120_000] {
            assert!(service
                .with_current_reminder(&ticket, now, |_| Ok(()))
                .is_err());
        }
    }

    #[test]
    fn browse_day_bounds_follow_local_midnight_and_dst_instead_of_a_fixed_24_hours() {
        for (day, zone, hours, expected) in [
            ("2026-03-08", "America/New_York", 23, "2026-03-08T05:00:00Z"),
            ("2026-11-01", "America/New_York", 25, "2026-11-01T04:00:00Z"),
            ("2026-10-29", "Asia/Seoul", 24, "2026-10-28T15:00:00Z"),
        ] {
            let (start, end) = day_bounds(day, zone).unwrap();
            assert_eq!(end - start, hours * 60 * 60 * 1000);
            assert_eq!(
                start,
                expected
                    .parse::<jiff::Timestamp>()
                    .unwrap()
                    .as_millisecond() as u64
            );
        }
        assert!(day_bounds("2026-02-30", "Asia/Seoul").is_err());
        assert!(day_bounds("2026-10-29", "invalid-zone").is_err());
        assert!(day_bounds("2011-12-30", "Pacific/Apia").is_err());
    }

    fn browse_context(service: &CalendarService) -> QueryContext {
        let inner = service.inner.lock().unwrap();
        QueryContext {
            generation: service.generation.load(Ordering::Acquire),
            account: inner.state.account.clone().unwrap(),
            selected: inner.state.selected.clone(),
            token: "synthetic-private-token".into(),
        }
    }

    #[test]
    fn browsing_finished_events_never_overwrites_the_reminder_cache_or_serializes_authentication() {
        let temp = tempfile::tempdir().unwrap();
        let service = reminder_service(temp.path());
        let before = serde_json::to_vec(&service.inner.lock().unwrap().state).unwrap();
        let finished = Occurrence {
            key: "finished".into(),
            title: "Past schedule".into(),
            start_at: 1,
            end_at: 2,
            calendar_name: "Synthetic".into(),
            join_urls: vec![],
        };
        let browse = service
            .finish_browse(
                browse_context(&service),
                "2026-10-29".into(),
                "Asia/Seoul".into(),
                vec![finished.clone()],
            )
            .unwrap();
        assert_eq!(
            service
                .with_browsed_occurrence(&browse, "finished", |event| Ok(event.clone()))
                .unwrap(),
            finished
        );
        let serialized = serde_json::to_string(&browse).unwrap();
        assert!(!serialized.contains("synthetic-private-token"));
        assert_eq!(
            serde_json::to_vec(&service.inner.lock().unwrap().state).unwrap(),
            before
        );
        assert!(service.reminder("current").is_some());
        assert!(!temp.path().join("calendar").exists());
    }

    #[test]
    fn a_failing_sync_keeps_a_previously_synced_account_connected() {
        let temp = tempfile::tempdir().unwrap();
        let service = CalendarService::new(temp.path().into()).unwrap();
        {
            let mut inner = service.inner.lock().unwrap();
            inner.state.account = Some("primary-id".into());
            inner.state.last_synced_at = Some(1);
            inner.auth_ready = false;
            inner.error = Some("네트워크".into());
        }
        assert!(service.snapshot().connected);
        service.inner.lock().unwrap().credential_blocked = Some(Blocked::Keychain);
        assert!(!service.snapshot().connected);
        let mut inner = service.inner.lock().unwrap();
        inner.credential_blocked = None;
        inner.state.last_synced_at = None;
        drop(inner);
        assert!(!service.snapshot().connected);
    }

    #[test]
    fn only_a_keychain_block_offers_the_access_retry() {
        let temp = tempfile::tempdir().unwrap();
        let service = CalendarService::new(temp.path().into()).unwrap();
        service.inner.lock().unwrap().state.account = Some("primary-id".into());
        service.inner.lock().unwrap().credential_blocked = Some(Blocked::Keychain);
        assert!(service.snapshot().keychain_blocked);
        service.inner.lock().unwrap().credential_blocked =
            Some(Blocked::Revoked("권한 해제".into()));
        assert!(!service.snapshot().keychain_blocked);
    }

    #[test]
    fn a_vanished_calendar_leaves_the_selection_instead_of_failing_sync() {
        let calendar = |id: &str, primary: bool| CalendarChoice {
            id: id.into(),
            name: id.into(),
            primary,
        };
        let listed = vec![calendar("primary-id", true), calendar("team", false)];
        let keep = |selected: &[&str]| {
            available_selection(
                &selected.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                &listed,
            )
            .unwrap()
        };
        assert_eq!(keep(&["team", "deleted"]), vec!["team"]);
        assert_eq!(keep(&["deleted"]), vec!["primary-id"]);
        assert!(keep(&[]).is_empty());
    }

    #[test]
    fn browsing_and_creation_revalidate_the_account_and_selected_calendars() {
        for changed in ["selection", "connection", "account"] {
            let temp = tempfile::tempdir().unwrap();
            let service = reminder_service(temp.path());
            let context = browse_context(&service);
            let browse = service
                .finish_browse(
                    browse_context(&service),
                    "2026-10-29".into(),
                    "Asia/Seoul".into(),
                    vec![],
                )
                .unwrap();
            match changed {
                "selection" => {
                    service.set_calendars(vec![]).unwrap();
                }
                "connection" => service.cancel_connect(),
                "account" => {
                    service.inner.lock().unwrap().state.account = Some("other account".into())
                }
                _ => unreachable!(),
            }
            assert!(
                service
                    .finish_browse(context, "2026-10-29".into(), "Asia/Seoul".into(), vec![])
                    .is_err(),
                "{changed}"
            );
            let created = AtomicBool::new(false);
            assert!(
                service
                    .with_browsed_occurrence(&browse, "current", |_| {
                        created.store(true, Ordering::Release);
                        Ok(())
                    })
                    .is_err(),
                "{changed}"
            );
            assert!(!created.load(Ordering::Acquire));
        }
    }

    #[test]
    fn a_deleted_remote_occurrence_cannot_create_from_the_old_reminder_cache() {
        let temp = tempfile::tempdir().unwrap();
        let service = reminder_service(temp.path());
        let browse = service
            .finish_browse(
                browse_context(&service),
                "2026-10-29".into(),
                "Asia/Seoul".into(),
                vec![],
            )
            .unwrap();
        assert!(service.reminder("current").is_some());
        assert!(service
            .with_browsed_occurrence(&browse, "current", |_| -> Result<(), String> {
                panic!("deleted event must not create a meeting")
            })
            .is_err());
    }

    #[test]
    fn a_late_authentication_failure_cannot_invalidate_a_refreshed_access_token() {
        let temp = tempfile::tempdir().unwrap();
        let service = reminder_service(temp.path());
        let old = browse_context(&service);
        {
            let mut inner = service.inner.lock().unwrap();
            inner.access = Some(Access {
                token: "synthetic-new-access".into(),
                expires_at: u64::MAX,
            });
        }
        let before = service.snapshot();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(service.query_error(&old, google::Failure::Reconnect));
        let after = service.snapshot();
        assert!(after.connected);
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.error, before.error);
        let mut current = browse_context(&service);
        current.token = service
            .inner
            .lock()
            .unwrap()
            .access
            .as_ref()
            .unwrap()
            .token
            .clone();
        runtime.block_on(service.query_error(&current, google::Failure::Reconnect));
        assert!(!service.snapshot().connected);
        assert!(service.inner.lock().unwrap().access.is_none());
    }

    #[test]
    fn authentication_failure_waits_for_an_in_flight_token_refresh_before_invalidating_state() {
        let temp = tempfile::tempdir().unwrap();
        let service = reminder_service(temp.path());
        let old = browse_context(&service);
        service.inner.lock().unwrap().access = Some(Access {
            token: old.token.clone(),
            expires_at: u64::MAX,
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            let refresh = async {
                let _access = service.access_gate.lock().await;
                tokio::task::yield_now().await;
                service.inner.lock().unwrap().access = Some(Access {
                    token: "synthetic-refreshed-access".into(),
                    expires_at: u64::MAX,
                });
            };
            tokio::join!(
                refresh,
                service.query_error(&old, google::Failure::Reconnect)
            );
        });
        assert!(service.snapshot().connected);
        assert!(service.snapshot().error.is_none());
    }

    #[test]
    fn a_rejected_access_token_is_dropped_for_one_refresh_without_asking_to_reconnect() {
        let temp = tempfile::tempdir().unwrap();
        let service = reminder_service(temp.path());
        let context = browse_context(&service);
        service.inner.lock().unwrap().access = Some(Access {
            token: context.token.clone(),
            expires_at: u64::MAX,
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(service.query_error(&context, google::Failure::Unauthorized));
        {
            let inner = service.inner.lock().unwrap();
            assert!(inner.access.is_none());
            assert!(inner.credential_blocked.is_none());
            assert!(inner.auth_ready);
            assert!(inner.needs_sync);
        }
        assert!(service.snapshot().connected);
        assert_eq!(service.rejections(), 1);
    }

    #[test]
    fn one_rejection_retries_while_a_repeated_rejection_asks_to_reconnect() {
        let temp = tempfile::tempdir().unwrap();
        let service = reminder_service(temp.path());
        let generation = service.generation.load(Ordering::Acquire);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let calls = std::cell::Cell::new(0);
        let reject = || service.inner.lock().unwrap().rejections += 1;
        let once = runtime.block_on(service.with_token_retry(generation, || {
            calls.set(calls.get() + 1);
            let first = calls.get() == 1;
            if first {
                reject();
            }
            async move {
                if first {
                    Err("rejected".to_string())
                } else {
                    Ok(())
                }
            }
        }));
        assert_eq!(once, Ok(()));
        assert_eq!(calls.get(), 2);
        assert!(service.snapshot().connected);
        let plain = runtime.block_on(
            service.with_token_retry(generation, || async { Err::<(), _>("other".to_string()) }),
        );
        assert_eq!(plain.unwrap_err(), "other");
        calls.set(0);
        let always = runtime.block_on(service.with_token_retry(generation, || {
            calls.set(calls.get() + 1);
            reject();
            async { Err::<(), _>("rejected".to_string()) }
        }));
        assert_eq!(calls.get(), 2);
        assert!(always.unwrap_err().contains("다시 연결"));
        assert!(!service.snapshot().connected);
    }
}
