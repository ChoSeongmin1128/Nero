mod callback_page;
mod google;
mod oauth;
mod schedule;
mod tokens;

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
    pub connecting: bool,
    pub restoring: bool,
    pub account_label: Option<String>,
    pub calendars: Vec<CalendarChoice>,
    pub selected_calendars: Vec<String>,
    pub last_synced_at: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct State {
    schema_version: u32,
    revision: u64,
    account: Option<String>,
    #[serde(default)]
    token_slot: bool,
    calendars: Vec<CalendarChoice>,
    selected: Vec<String>,
    events: Vec<Occurrence>,
    notices: BTreeMap<String, Notice>,
    last_synced_at: Option<u64>,
}
struct Access {
    token: String,
    expires_at: u64,
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
    needs_sync: bool,
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
                needs_sync: true,
                last_attempt: None,
                last_tick: None,
                error: None,
            }),
            generation: AtomicU64::new(0),
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
        CalendarSnapshot {
            revision: inner.state.revision,
            configured: self.config.is_some()
                && cfg!(any(target_os = "macos", target_os = "windows")),
            connected: inner.state.account.is_some() && inner.auth_ready,
            connecting: inner.connecting_id.is_some(),
            restoring: self.config.is_some()
                && inner.state.account.is_some()
                && !inner.auth_ready
                && inner.error.is_none()
                && inner.connecting_id.is_none(),
            account_label: inner.state.account.clone(),
            calendars: inner.state.calendars.clone(),
            selected_calendars: inner.state.selected.clone(),
            last_synced_at: inner.state.last_synced_at,
            error: inner.error.clone(),
        }
    }
    pub fn begin_connect(&self) -> Result<ConnectRequest, String> {
        if !cfg!(any(target_os = "macos", target_os = "windows")) {
            return Err("Google 캘린더 연결은 macOS와 Windows에서 지원합니다.".into());
        }
        let config = self
            .config
            .as_ref()
            .ok_or("이 버전에는 Google 캘린더 연결 정보가 설정되지 않았습니다.")?;
        let (pending, request) = Pending::begin(config)?;
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
        let account = calendars
            .iter()
            .find(|calendar| calendar.primary)
            .map(|calendar| calendar.id.clone())
            .ok_or("Google 계정의 기본 캘린더를 확인하지 못했습니다.")?;
        let refresh_token = response
            .refresh_token
            .filter(|token| !token.is_empty())
            .ok_or("백그라운드 연결 권한을 받지 못했습니다. 다시 연결해 주세요.")?;
        let _gate = self.token_gate.lock().unwrap();
        let mut inner = self.inner.lock().unwrap();
        if generation != self.generation.load(Ordering::Acquire) {
            return Err("현재 연결 요청이 아닙니다.".into());
        }
        let mut next = inner.state.clone();
        if next.account.as_deref() != Some(&account) {
            next.selected.clear();
            next.notices.clear();
        }
        next.account = Some(account.clone());
        next.calendars = calendars;
        next.events.clear();
        next.last_synced_at = None;
        next.revision += 1;
        let next = commit_connection(
            inner.state.token_slot,
            next,
            |slot| {
                TokenStore.save(
                    &StoredToken {
                        refresh_token,
                        account_id: account,
                        client_id: config.client_id.clone(),
                    },
                    slot,
                )
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
        inner.needs_sync = false;
        inner.error = None;
        let saved = persist(&self.directory, &inner.state);
        let removed = TokenStore.delete();
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
            inner.error = Some(error.message());
            inner.state.revision += 1;
        }
        error.message()
    }
    async fn query_context(&self, generation: u64) -> Result<QueryContext, String> {
        let _access = self.access_gate.lock().await;
        let config = self
            .config
            .as_ref()
            .ok_or("이 버전에는 Google 캘린더 연결 정보가 설정되지 않았습니다.")?;
        let (account, selected, token_slot, access) = {
            let inner = self.inner.lock().unwrap();
            if generation != self.generation.load(Ordering::Acquire) {
                return Err("캘린더 선택이나 연결이 변경되었습니다.".into());
            }
            if inner.connecting_id.is_some() {
                return Err("Google 연결을 확인하고 있습니다.".into());
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
            )
        };
        let token = if let Some(token) = access {
            token
        } else {
            let stored = {
                let _gate = self.token_gate.lock().unwrap();
                TokenStore.load(token_slot)?
            };
            if stored.account_id != account {
                return Err("저장된 Google 계정 연결이 다릅니다. 다시 연결해 주세요.".into());
            }
            let response = google::refresh(&self.client, config, &stored)
                .await
                .map_err(|error| self.sync_error(generation, error))?;
            let _gate = self.token_gate.lock().unwrap();
            let mut inner = self.inner.lock().unwrap();
            if generation != self.generation.load(Ordering::Acquire) {
                return Err("캘린더 선택이나 연결이 변경되었습니다.".into());
            }
            if let Some(refresh_token) = response.refresh_token {
                TokenStore.save(
                    &StoredToken {
                        refresh_token,
                        ..stored
                    },
                    token_slot,
                )?;
            }
            inner.access = Some(Access {
                token: response.access_token.clone(),
                expires_at: now_ms().saturating_add(response.expires_in.saturating_mul(1000)),
            });
            response.access_token
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
        if calendars
            .iter()
            .find(|calendar| calendar.primary)
            .map(|calendar| calendar.id.as_str())
            != Some(account.as_str())
        {
            return Err("Google 계정 연결이 변경되었습니다. 다시 연결해 주세요.".into());
        }
        let choices = selected
            .iter()
            .map(|id| {
                calendars
                    .iter()
                    .find(|calendar| &calendar.id == id)
                    .cloned()
                    .ok_or(
                        "선택한 캘린더에 접근할 수 없습니다. 캘린더를 다시 선택해 주세요."
                            .to_owned(),
                    )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((calendars, choices))
    }

    async fn synchronize(&self, generation: u64) -> Result<(), String> {
        self.inner.lock().unwrap().last_attempt = Some(now_ms());
        let context = self.query_context(generation).await?;
        let (calendars, choices) = self.query_calendars(&context).await?;
        let (token, account) = (&context.token, &context.account);
        let events = match google::events(&self.client, token, account, &choices, now_ms()).await {
            Ok(events) => events,
            Err(error) => return Err(self.query_error(&context, error).await),
        };
        let mut inner = self.inner.lock().unwrap();
        if generation != self.generation.load(Ordering::Acquire) {
            return Err("캘린더 선택이나 연결이 변경되었습니다.".into());
        }
        let mut next = inner.state.clone();
        next.calendars = calendars;
        next.events = events;
        next.last_synced_at = Some(now_ms());
        next.revision += 1;
        persist(&self.directory, &next)?;
        inner.state = next;
        inner.auth_ready = true;
        inner.needs_sync = false;
        inner.error = None;
        Ok(())
    }

    pub async fn browse_day(
        &self,
        day: String,
        time_zone: String,
        expected_scope: Option<u64>,
    ) -> Result<MeetingBrowse, String> {
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
            inner.last_attempt = None;
        }
        inner.last_tick = Some(now);
        let fresh = inner.auth_ready
            && inner.connecting_id.is_none()
            && !inner.needs_sync
            && inner.error.is_none()
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
        .find(|event| {
            event.key == key
                && event.start_at > now
                && event.end_at > event.start_at
                && now >= event.start_at.saturating_sub(schedule::LEAD_MS)
        })
        .ok_or_else(|| {
            "일정이 변경되었거나 시작 시간이 지났습니다. 현재 일정을 확인해 주세요.".into()
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
    fn reminder_service(root: &Path) -> CalendarService {
        let service = CalendarService::new(root.into()).unwrap();
        {
            let mut inner = service.inner.lock().unwrap();
            inner.auth_ready = true;
            inner.needs_sync = false;
            inner.state.account = Some("synthetic account".into());
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
}
