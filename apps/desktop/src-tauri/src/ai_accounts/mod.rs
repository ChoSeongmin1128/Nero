mod claude;
mod credentials;
mod oauth;
mod oidc;

use fs2::FileExt;
pub use oauth::SignInAttempt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SIGN_IN_REQUIRED: &str = "ChatGPT에 다시 로그인해 주세요.";
const PLAN_DISABLED: &str = "ChatGPT 구독 사용이 허용되지 않았습니다. 다시 연결해 주세요.";
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(HTTP_TIMEOUT.as_secs() * 3);

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    #[serde(rename = "chatgpt")]
    ChatGpt,
    #[serde(rename = "claude")]
    Claude,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: Provider,
    pub name: String,
    pub configured: bool,
    pub reason: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSnapshot {
    pub id: String,
    pub provider: Provider,
    pub label: String,
    pub email: Option<String>,
    pub state: String,
    pub can_invoke: bool,
    pub configured: bool,
    pub reason: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountsSnapshot {
    pub active_account_id: Option<String>,
    pub accounts: Vec<AccountSnapshot>,
    pub providers: Vec<ProviderInfo>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignOutOutcome {
    pub account: AccountSnapshot,
    pub revocation_confirmed: bool,
}

pub struct SubscriptionSession {
    pub provider: Provider,
    pub account_id: String,
    pub base_url: &'static str,
    access_token: String,
}
#[must_use]
pub struct ShutdownGuard {
    closing: Arc<AtomicBool>,
}
impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        self.closing.store(false, Ordering::Release);
    }
}
impl SubscriptionSession {
    pub fn access_token(&self) -> &str {
        &self.access_token
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SavedState {
    host_id: Option<String>,
    active_account_id: Option<String>,
    #[serde(default)]
    accounts: Vec<AccountRecord>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AccountRecord {
    provider: Provider,
    id: String,
    client_id: String,
    subject: Option<String>,
    email: Option<String>,
    name: Option<String>,
    revision: u64,
    tokens: Option<Tokens>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    scopes: Vec<String>,
    expires_at: u64,
}
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    token_type: String,
    expires_in: u64,
    #[serde(default)]
    scope: Option<String>,
}
struct HttpFailure {
    code: Option<String>,
}

#[derive(Clone)]
pub struct AiAccounts {
    directory: PathBuf,
    client: reqwest::Client,
    store: Arc<dyn credentials::Credentials>,
    operation: Arc<tokio::sync::Mutex<()>>,
    pending: Arc<Mutex<Option<Arc<oauth::Pending>>>>,
    closing: Arc<AtomicBool>,
    public_snapshot: Arc<Mutex<Option<AccountsSnapshot>>>,
}

impl AiAccounts {
    pub fn new(root: PathBuf, _resources: PathBuf) -> Result<Self, String> {
        Self::with_store(root, Arc::new(credentials::SystemCredentials))
    }
    fn with_store(
        directory: PathBuf,
        store: Arc<dyn credentials::Credentials>,
    ) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|_| "AI 연결을 준비하지 못했습니다.")?;
        Ok(Self {
            directory,
            client,
            store,
            operation: Arc::new(tokio::sync::Mutex::new(())),
            pending: Arc::new(Mutex::new(None)),
            closing: Arc::new(AtomicBool::new(false)),
            public_snapshot: Arc::new(Mutex::new(None)),
        })
    }
    pub fn snapshot(&self) -> Result<AccountsSnapshot, String> {
        let mut cached = self
            .public_snapshot
            .lock()
            .map_err(|_| "AI 연결 상태를 확인하지 못했습니다.")?;
        if let Some(snapshot) = cached.as_ref() {
            return Ok(snapshot.clone());
        }
        let state = credentials::read::<SavedState>(self.store.as_ref())?;
        let snapshot = Self::public_view(&state);
        *cached = Some(snapshot.clone());
        Ok(snapshot)
    }
    fn public_view(state: &SavedState) -> AccountsSnapshot {
        AccountsSnapshot {
            active_account_id: state.active_account_id.clone(),
            accounts: state.accounts.iter().map(AccountRecord::snapshot).collect(),
            providers: vec![
                ProviderInfo {
                    id: Provider::ChatGpt,
                    name: "ChatGPT".into(),
                    configured: true,
                    reason: None,
                },
                ProviderInfo {
                    id: Provider::Claude,
                    name: "Claude".into(),
                    configured: true,
                    reason: None,
                },
            ],
        }
    }
    pub fn begin_sign_in(
        &self,
        provider: Provider,
        account_id: Option<String>,
    ) -> Result<SignInAttempt, String> {
        self.ensure_open()?;
        self.cancel_pending()?;
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| "다른 AI 계정 작업이 진행 중입니다.")?;
        self.ensure_open()?;
        let _file = self.file_guard()?;
        let mut state = self.read()?;
        let selected = account_id
            .as_ref()
            .map(|id| {
                state
                    .accounts
                    .iter()
                    .find(|account| &account.id == id && account.provider == provider)
                    .cloned()
                    .ok_or("선택한 AI 계정을 찾지 못했습니다.")
            })
            .transpose()?;
        if state.host_id.is_none() {
            state.host_id = Some(format!("urn:uuid:{}", uuid::Uuid::new_v4()));
            self.save(&state)?;
        }
        let (pending, request) = if provider == Provider::Claude {
            oauth::Pending::begin_claude(
                state.host_id.as_deref().unwrap(),
                account_id,
                selected.as_ref().map(|account| account.revision),
            )
        } else {
            oauth::Pending::begin(
                state.host_id.as_deref().unwrap(),
                account_id,
                selected.as_ref().map(|account| account.revision),
                selected.as_ref().map(|account| account.client_id.clone()),
                selected
                    .as_ref()
                    .and_then(|account| account.tokens.as_ref())
                    .and_then(|tokens| tokens.id_token.as_deref()),
                selected
                    .as_ref()
                    .and_then(|account| account.email.as_deref()),
            )
        }?;
        let mut current = self
            .pending
            .lock()
            .map_err(|_| "AI 연결 상태를 확인하지 못했습니다.")?;
        self.ensure_open()?;
        *current = Some(Arc::new(pending));
        Ok(request)
    }
    pub async fn complete_sign_in(
        &self,
        id: &str,
        cancel: Arc<AtomicBool>,
    ) -> Result<AccountSnapshot, String> {
        self.ensure_open()?;
        let pending = self
            .pending
            .lock()
            .map_err(|_| "AI 연결 상태를 확인하지 못했습니다.")?
            .as_ref()
            .filter(|pending| pending.id == id)
            .cloned()
            .ok_or("진행 중인 로그인을 찾지 못했습니다.")?;
        if pending.completing.swap(true, Ordering::AcqRel) {
            return Err("로그인을 확인 중입니다.".into());
        }
        let result = self.finish_sign_in(&pending, &cancel).await;
        pending.finish_page(result.as_ref().map(|_| ()).map_err(String::as_str));
        let mut current = self
            .pending
            .lock()
            .map_err(|_| "AI 연결 상태를 확인하지 못했습니다.")?;
        if current.as_ref().is_some_and(|current| current.id == id) {
            *current = None;
        }
        result
    }
    async fn finish_sign_in(
        &self,
        pending: &oauth::Pending,
        cancel: &Arc<AtomicBool>,
    ) -> Result<AccountSnapshot, String> {
        if pending.provider == Provider::Claude {
            return self.finish_claude_sign_in(pending, cancel).await;
        }
        let callback = pending.receive(cancel.clone()).await?;
        let _operation = with_cancel(self.operation.lock(), &pending.cancelled, cancel).await?;
        self.ensure_open()?;
        let _file = self.file_guard()?;
        stopped(&pending.cancelled, cancel)?;
        let mut state = self.read()?;
        if state.host_id.as_deref() != Some(pending.host_id.as_str()) {
            return Err("로그인 중 기기 연결 정보가 변경되었습니다. 다시 연결해 주세요.".into());
        }
        let selected = match pending.selected_id.as_ref() {
            Some(id) => {
                let account = state
                    .accounts
                    .iter()
                    .find(|account| &account.id == id)
                    .ok_or(SIGN_IN_REQUIRED)?;
                if Some(account.revision) != pending.expected_revision
                    || account.client_id != callback.client_id
                {
                    return Err("로그인 중 계정이 변경되었습니다. 다시 연결해 주세요.".into());
                }
                account.clone()
            }
            None => {
                if state
                    .accounts
                    .iter()
                    .any(|account| account.client_id == callback.client_id)
                {
                    return Err("저장된 ChatGPT 계정을 선택해 다시 연결해 주세요.".into());
                }
                let account = AccountRecord {
                    provider: Provider::ChatGpt,
                    id: pending.account_id.clone(),
                    client_id: callback.client_id.clone(),
                    subject: None,
                    email: None,
                    name: None,
                    revision: 0,
                    tokens: None,
                };
                state.accounts.push(account.clone());
                self.save(&state)?;
                account
            }
        };
        let discovery = self.discovery(&pending.cancelled, cancel).await?;
        let form = [
            ("grant_type", "authorization_code"),
            ("client_id", callback.client_id.as_str()),
            ("code", callback.code.as_str()),
            ("code_verifier", pending.verifier.as_str()),
            ("redirect_uri", pending.redirect_uri.as_str()),
            ("resource", oidc::RESOURCE),
        ];
        let response = self
            .token_request(&discovery.token_endpoint, &form, &pending.cancelled, cancel)
            .await
            .map_err(|failure| oauth_message(&failure))?;
        let id_token = response
            .id_token
            .as_deref()
            .ok_or("ChatGPT 계정 인증이 완료되지 않았습니다.")?;
        let jwks = self
            .json_request(
                self.client.get(&discovery.jwks_uri),
                &pending.cancelled,
                cancel,
            )
            .await
            .map_err(|_| "ChatGPT 인증 키를 확인하지 못했습니다.")?;
        let identity = oidc::verify_id_token(
            id_token,
            &jwks,
            &callback.client_id,
            Some(&pending.nonce),
            &response.access_token,
            now(),
        )?;
        if selected
            .subject
            .as_deref()
            .is_some_and(|subject| subject != identity.subject)
        {
            return Err("선택한 ChatGPT 계정과 로그인한 계정이 다릅니다.".into());
        }
        let tokens = response.into_tokens(None, false)?;
        let pending_guard = self
            .pending
            .lock()
            .map_err(|_| "AI 연결 상태를 확인하지 못했습니다.")?;
        if !pending_guard
            .as_ref()
            .is_some_and(|current| current.id == pending.id)
        {
            return Err("AI 로그인을 취소했습니다.".into());
        }
        stopped(&pending.cancelled, cancel)?;
        self.ensure_open()?;
        let mut current = self.read()?;
        let record = current
            .accounts
            .iter_mut()
            .find(|account| account.id == selected.id)
            .ok_or(SIGN_IN_REQUIRED)?;
        if record.revision != selected.revision || record.client_id != selected.client_id {
            return Err("로그인 중 계정이 변경되었습니다. 다시 연결해 주세요.".into());
        }
        record.subject = Some(identity.subject);
        record.email = identity.email;
        record.name = identity.name;
        record.tokens = Some(tokens);
        record.revision += 1;
        let snapshot = record.snapshot();
        current.active_account_id = Some(record.id.clone());
        self.save(&current)?;
        Ok(snapshot)
    }
    async fn finish_claude_sign_in(
        &self,
        pending: &oauth::Pending,
        cancel: &Arc<AtomicBool>,
    ) -> Result<AccountSnapshot, String> {
        let callback = pending.receive(cancel.clone()).await?;
        let _operation = with_cancel(self.operation.lock(), &pending.cancelled, cancel).await?;
        self.ensure_open()?;
        let _file = self.file_guard()?;
        stopped(&pending.cancelled, cancel)?;
        let state = self.read()?;
        if state.host_id.as_deref() != Some(pending.host_id.as_str()) {
            return Err("로그인 중 기기 연결 정보가 변경되었습니다. 다시 연결해 주세요.".into());
        }
        let selected = match pending.selected_id.as_deref() {
            Some(id) => {
                let account = state
                    .accounts
                    .iter()
                    .find(|account| account.id == id && account.provider == Provider::Claude)
                    .ok_or("선택한 Claude 계정을 찾지 못했습니다.")?;
                if Some(account.revision) != pending.expected_revision {
                    return Err("로그인 중 계정이 변경되었습니다. 다시 연결해 주세요.".into());
                }
                Some(account.clone())
            }
            None => None,
        };
        let value=self.json_request(self.client.post(claude::TOKEN).json(&serde_json::json!({"grant_type":"authorization_code","client_id":claude::CLIENT_ID,"code":callback.code,"state":pending.state,"redirect_uri":pending.redirect_uri,"code_verifier":pending.verifier})),&pending.cancelled,cancel).await.map_err(|_|"Claude 연결을 완료하지 못했습니다. 다시 시도해 주세요.")?;
        let response: claude::Response = serde_json::from_value(value)
            .map_err(|_| "Claude 구독 인증 응답을 확인하지 못했습니다.")?;
        let identity = response.identity();
        if let (Some(expected), Some((actual, _))) = (
            selected
                .as_ref()
                .and_then(|account| account.subject.as_deref()),
            identity.as_ref(),
        ) {
            if expected != actual {
                return Err("선택한 Claude 계정과 로그인한 계정이 다릅니다.".into());
            }
        }
        let tokens = response.into_tokens(None)?;
        let pending_guard = self
            .pending
            .lock()
            .map_err(|_| "AI 연결 상태를 확인하지 못했습니다.")?;
        if !pending_guard
            .as_ref()
            .is_some_and(|current| current.id == pending.id)
        {
            return Err("AI 로그인을 취소했습니다.".into());
        }
        stopped(&pending.cancelled, cancel)?;
        self.ensure_open()?;
        let mut current = self.read()?;
        let record = if let Some(selected) = selected {
            let record = current
                .accounts
                .iter_mut()
                .find(|account| account.id == selected.id)
                .ok_or("선택한 Claude 계정을 찾지 못했습니다.")?;
            if record.provider != Provider::Claude || record.revision != selected.revision {
                return Err("로그인 중 계정이 변경되었습니다. 다시 연결해 주세요.".into());
            }
            record
        } else {
            current.accounts.push(AccountRecord {
                provider: Provider::Claude,
                id: pending.account_id.clone(),
                client_id: claude::CLIENT_ID.into(),
                subject: None,
                email: None,
                name: None,
                revision: 0,
                tokens: None,
            });
            current.accounts.last_mut().unwrap()
        };
        record.subject = identity.as_ref().map(|(subject, _)| subject.clone());
        record.email = identity.and_then(|(_, email)| email);
        record.name = None;
        record.tokens = Some(tokens);
        record.revision += 1;
        let snapshot = record.snapshot();
        current.active_account_id = Some(record.id.clone());
        self.save(&current)?;
        Ok(snapshot)
    }
    async fn refresh_claude_session(
        &self,
        state: &mut SavedState,
        position: usize,
        cancel: &Arc<AtomicBool>,
    ) -> Result<SubscriptionSession, String> {
        let account = state.accounts[position].clone();
        let old = account
            .tokens
            .as_ref()
            .ok_or("Claude에 다시 로그인해 주세요.")?;
        let refresh = old
            .refresh_token
            .as_deref()
            .ok_or("Claude에 다시 로그인해 주세요.")?;
        stopped(cancel, cancel)?;
        self.ensure_open()?;
        let transaction = Arc::new(AtomicBool::new(false));
        let response=self.json_request(self.client.post(claude::TOKEN).json(&serde_json::json!({"grant_type":"refresh_token","client_id":claude::CLIENT_ID,"refresh_token":refresh})),&transaction,&transaction).await;
        let value = match response {
            Ok(value) => value,
            Err(failure) => {
                if failure.code.as_deref().is_some_and(terminal_refresh) {
                    state.accounts[position].tokens = None;
                    state.accounts[position].revision += 1;
                    self.save(state)?;
                }
                return Err("Claude 인증을 갱신하지 못했습니다. 다시 로그인해 주세요.".into());
            }
        };
        let response: claude::Response = serde_json::from_value(value)
            .map_err(|_| "Claude 인증 갱신 응답을 확인하지 못했습니다.")?;
        if let (Some(expected), Some((actual, _))) =
            (account.subject.as_deref(), response.identity())
        {
            if expected != actual {
                state.accounts[position].tokens = None;
                state.accounts[position].revision += 1;
                self.save(state)?;
                return Err("갱신된 Claude 계정이 기존 계정과 다릅니다.".into());
            }
        }
        let replacement = match response.into_tokens(Some(old)) {
            Ok(replacement) => replacement,
            Err(error) => {
                state.accounts[position].tokens = None;
                state.accounts[position].revision += 1;
                self.save(state)?;
                return Err(error);
            }
        };
        let access_token = replacement.access_token.clone();
        let enabled = replacement.plan_enabled(Provider::Claude);
        state.accounts[position].tokens = Some(replacement);
        state.accounts[position].revision += 1;
        self.save(state)?;
        stopped(cancel, cancel)?;
        if !enabled {
            return Err(plan_disabled(Provider::Claude).into());
        }
        Ok(SubscriptionSession {
            provider: Provider::Claude,
            account_id: account.id,
            base_url: claude::BASE_URL,
            access_token,
        })
    }
    pub fn cancel_sign_in(&self, id: &str) -> Result<(), String> {
        let mut current = self
            .pending
            .lock()
            .map_err(|_| "AI 연결 상태를 확인하지 못했습니다.")?;
        if current.as_ref().is_some_and(|pending| pending.id == id) {
            if let Some(pending) = current.take() {
                pending.cancelled.store(true, Ordering::Release);
                pending.finish_page(Err("AI 로그인을 취소했습니다."));
            }
        }
        Ok(())
    }
    fn cancel_pending(&self) -> Result<(), String> {
        if let Some(pending) = self
            .pending
            .lock()
            .map_err(|_| "AI 연결 상태를 확인하지 못했습니다.")?
            .take()
        {
            pending.cancelled.store(true, Ordering::Release);
            pending.finish_page(Err("AI 로그인을 취소했습니다."));
        }
        Ok(())
    }
    pub async fn prepare_shutdown(&self) -> Result<ShutdownGuard, String> {
        self.drain_operations(SHUTDOWN_TIMEOUT).await
    }
    async fn drain_operations(&self, timeout: Duration) -> Result<ShutdownGuard, String> {
        self.closing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "AI 계정 정보를 저장하는 중입니다. 잠시 후 다시 시도해 주세요.")?;
        let guard = ShutdownGuard {
            closing: self.closing.clone(),
        };
        self.cancel_pending()?;
        // The desktop lifecycle blocks new operations before draining this lock.
        let _operation = tokio::time::timeout(timeout, self.operation.lock())
            .await
            .map_err(|_| "AI 계정 정보를 저장하는 중입니다. 잠시 후 다시 시도해 주세요.")?;
        Ok(guard)
    }
    fn ensure_open(&self) -> Result<(), String> {
        if self.closing.load(Ordering::Acquire) {
            Err("앱을 종료하거나 업데이트하고 있습니다. 잠시 기다려 주세요.".into())
        } else {
            Ok(())
        }
    }
    pub fn select_account(&self, id: &str) -> Result<AccountSnapshot, String> {
        self.ensure_open()?;
        self.cancel_pending()?;
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| "다른 AI 계정 작업이 진행 중입니다.")?;
        self.ensure_open()?;
        let _file = self.file_guard()?;
        let mut state = self.read()?;
        let selected = state
            .accounts
            .iter()
            .find(|account| account.id == id)
            .ok_or("선택한 AI 계정을 찾지 못했습니다.")?;
        let result = selected.snapshot();
        if !result.can_invoke {
            return Err(result.reason.unwrap_or_else(|| SIGN_IN_REQUIRED.into()));
        }
        state.active_account_id = Some(id.into());
        self.save(&state)?;
        Ok(result)
    }
    pub async fn sign_out(&self, id: &str) -> Result<SignOutOutcome, String> {
        self.ensure_open()?;
        self.cancel_pending()?;
        let _operation = self.operation.lock().await;
        self.ensure_open()?;
        let _file = self.file_guard()?;
        let mut state = self.read()?;
        let selected = state
            .accounts
            .iter()
            .find(|account| account.id == id)
            .cloned()
            .ok_or("선택한 AI 계정을 찾지 못했습니다.")?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let revocation_confirmed = if selected.provider == Provider::Claude {
            selected.tokens.is_none()
        } else if let Some(refresh) = selected
            .tokens
            .as_ref()
            .and_then(|tokens| tokens.refresh_token.as_deref())
        {
            if let Ok(discovery) = self.discovery(&cancelled, &cancelled).await {
                let request = self
                    .client
                    .post(discovery.revocation_endpoint)
                    .header(
                        reqwest::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .body(form_body(&[
                        ("token", refresh),
                        ("token_type_hint", "refresh_token"),
                        ("client_id", selected.client_id.as_str()),
                    ]));
                match request.send().await {
                    Ok(response) => response.status() == reqwest::StatusCode::OK,
                    Err(_) => false,
                }
            } else {
                false
            }
        } else {
            true
        };
        let record = state
            .accounts
            .iter_mut()
            .find(|account| account.id == id)
            .unwrap();
        record.tokens = None;
        record.revision += 1;
        let account = record.snapshot();
        if state.active_account_id.as_deref() == Some(id) {
            state.active_account_id = None;
        }
        self.save(&state)?;
        Ok(SignOutOutcome {
            account,
            revocation_confirmed,
        })
    }
    pub async fn subscription_session(
        &self,
        account_id: &str,
    ) -> Result<SubscriptionSession, String> {
        let cancelled = Arc::new(AtomicBool::new(false));
        self.subscription_session_with_cancel(account_id, cancelled)
            .await
    }
    pub async fn subscription_session_with_cancel(
        &self,
        account_id: &str,
        cancel: Arc<AtomicBool>,
    ) -> Result<SubscriptionSession, String> {
        self.ensure_open()?;
        stopped(&cancel, &cancel)?;
        let accounts = self.clone();
        let account_id = account_id.to_string();
        let transaction_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            accounts
                .session_transaction(&account_id, transaction_cancel)
                .await
        });
        with_cancel(task, &cancel, &cancel)
            .await?
            .map_err(|_| "ChatGPT 인증을 갱신하지 못했습니다.".to_string())?
    }
    async fn session_transaction(
        &self,
        account_id: &str,
        cancel: Arc<AtomicBool>,
    ) -> Result<SubscriptionSession, String> {
        self.ensure_open()?;
        let _operation = with_cancel(self.operation.lock(), &cancel, &cancel).await?;
        self.ensure_open()?;
        let _file = self.file_guard()?;
        let mut state = self.read()?;
        let position = state
            .accounts
            .iter()
            .position(|account| account.id == account_id)
            .ok_or(SIGN_IN_REQUIRED)?;
        let account = state.accounts[position].clone();
        let old = account.tokens.as_ref().ok_or(SIGN_IN_REQUIRED)?;
        if account.provider == Provider::ChatGpt && account.subject.is_none() {
            return Err(SIGN_IN_REQUIRED.into());
        }
        if !old.plan_enabled(account.provider) {
            return Err(plan_disabled(account.provider).into());
        }
        if account.provider == Provider::Claude && old.expires_at <= now() {
            return self
                .refresh_claude_session(&mut state, position, &cancel)
                .await;
        }
        let access = if old.expires_at > now() {
            old.access_token.clone()
        } else {
            let refresh = old.refresh_token.as_deref().ok_or(SIGN_IN_REQUIRED)?;
            let discovery = self.discovery(&cancel, &cancel).await?;
            let form = [
                ("grant_type", "refresh_token"),
                ("client_id", account.client_id.as_str()),
                ("refresh_token", refresh),
                ("resource", oidc::RESOURCE),
            ];
            let jwks = self
                .json_request(self.client.get(&discovery.jwks_uri), &cancel, &cancel)
                .await
                .map_err(|_| "ChatGPT 인증 키를 확인하지 못했습니다.")?;
            stopped(&cancel, &cancel)?;
            self.ensure_open()?;
            let transaction = Arc::new(AtomicBool::new(false));
            let response = match self
                .token_request(&discovery.token_endpoint, &form, &transaction, &transaction)
                .await
            {
                Ok(response) => response,
                Err(failure) => {
                    if failure.code.as_deref().is_some_and(terminal_refresh) {
                        state.accounts[position].tokens = None;
                        state.accounts[position].revision += 1;
                        self.save(&state)?;
                    }
                    return Err(oauth_message(&failure));
                }
            };
            if let Some(id_token) = response.id_token.as_deref() {
                let identity = match oidc::verify_id_token(
                    id_token,
                    &jwks,
                    &account.client_id,
                    None,
                    &response.access_token,
                    now(),
                ) {
                    Ok(identity) => identity,
                    Err(error) => {
                        state.accounts[position].tokens = None;
                        state.accounts[position].revision += 1;
                        self.save(&state)?;
                        return Err(error);
                    }
                };
                if Some(identity.subject.as_str()) != account.subject.as_deref() {
                    state.accounts[position].tokens = None;
                    state.accounts[position].revision += 1;
                    self.save(&state)?;
                    return Err("갱신된 ChatGPT 계정이 기존 계정과 다릅니다.".into());
                }
            }
            let replacement = match response.into_tokens(Some(old), true) {
                Ok(replacement) => replacement,
                Err(error) => {
                    state.accounts[position].tokens = None;
                    state.accounts[position].revision += 1;
                    self.save(&state)?;
                    return Err(error);
                }
            };
            if !replacement.plan_enabled(Provider::ChatGpt) {
                state.accounts[position].tokens = Some(replacement);
                state.accounts[position].revision += 1;
                self.save(&state)?;
                return Err(PLAN_DISABLED.into());
            }
            let access = replacement.access_token.clone();
            state.accounts[position].tokens = Some(replacement);
            state.accounts[position].revision += 1;
            self.save(&state)?;
            access
        };
        stopped(&cancel, &cancel)?;
        Ok(SubscriptionSession {
            provider: account.provider,
            account_id: account_id.into(),
            base_url: if account.provider == Provider::Claude {
                claude::BASE_URL
            } else {
                oidc::RESOURCE
            },
            access_token: access,
        })
    }
    async fn discovery(
        &self,
        own: &Arc<AtomicBool>,
        cancel: &Arc<AtomicBool>,
    ) -> Result<oidc::Discovery, String> {
        let value = self
            .json_request(self.client.get(oidc::DISCOVERY), own, cancel)
            .await
            .map_err(|_| "ChatGPT 연결 정보를 확인하지 못했습니다.")?;
        let discovery: oidc::Discovery = serde_json::from_value(value)
            .map_err(|_| "ChatGPT 연결 정보를 확인하지 못했습니다.")?;
        discovery.validate()?;
        Ok(discovery)
    }
    async fn token_request(
        &self,
        url: &str,
        form: &[(&str, &str)],
        own: &Arc<AtomicBool>,
        cancel: &Arc<AtomicBool>,
    ) -> Result<TokenResponse, HttpFailure> {
        let value = self
            .json_request(
                self.client
                    .post(url)
                    .header(
                        reqwest::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .body(form_body(form)),
                own,
                cancel,
            )
            .await?;
        serde_json::from_value(value).map_err(|_| HttpFailure { code: None })
    }
    async fn json_request(
        &self,
        request: reqwest::RequestBuilder,
        own: &Arc<AtomicBool>,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Value, HttpFailure> {
        let mut response = with_cancel(request.send(), own, cancel)
            .await
            .map_err(|_| HttpFailure { code: None })?
            .map_err(|_| HttpFailure { code: None })?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = with_cancel(response.chunk(), own, cancel)
            .await
            .map_err(|_| HttpFailure { code: None })?
            .map_err(|_| HttpFailure { code: None })?
        {
            if bytes.len() + chunk.len() > 1024 * 1024 {
                return Err(HttpFailure { code: None });
            }
            bytes.extend(chunk);
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| HttpFailure { code: None })?;
        if !status.is_success() {
            return Err(HttpFailure {
                code: value
                    .get("error")
                    .and_then(|error| {
                        error
                            .as_str()
                            .or_else(|| error.get("code").and_then(Value::as_str))
                    })
                    .map(String::from),
            });
        }
        Ok(value)
    }
    fn read(&self) -> Result<SavedState, String> {
        let mut cached = self
            .public_snapshot
            .lock()
            .map_err(|_| "AI 연결 상태를 확인하지 못했습니다.")?;
        let result = credentials::read::<SavedState>(self.store.as_ref());
        *cached = result.as_ref().ok().map(Self::public_view);
        result
    }
    fn save(&self, state: &SavedState) -> Result<(), String> {
        let mut cached = self
            .public_snapshot
            .lock()
            .map_err(|_| "AI 연결 상태를 확인하지 못했습니다.")?;
        *cached = None;
        credentials::write(self.store.as_ref(), state)?;
        *cached = Some(Self::public_view(state));
        Ok(())
    }
    fn file_guard(&self) -> Result<File, String> {
        fs::create_dir_all(&self.directory).map_err(|_| "AI 연결 상태를 준비하지 못했습니다.")?;
        if fs::symlink_metadata(&self.directory)
            .map_err(|_| "AI 연결 상태를 확인하지 못했습니다.")?
            .file_type()
            .is_symlink()
        {
            return Err("AI 연결 경로를 확인하지 못했습니다.".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.directory, fs::Permissions::from_mode(0o700))
                .map_err(|_| "AI 연결 경로를 보호하지 못했습니다.")?;
        }
        let path = self.directory.join("credentials.lock");
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err("AI 연결 경로를 확인하지 못했습니다.".into());
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|_| "AI 연결 상태를 준비하지 못했습니다.")?;
        file.try_lock_exclusive()
            .map_err(|_| "다른 창에서 AI 계정 작업이 진행 중입니다.")?;
        Ok(file)
    }
}
impl Tokens {
    fn plan_enabled(&self, provider: Provider) -> bool {
        match provider {
            Provider::ChatGpt => {
                self.scopes.iter().any(|scope| scope == oidc::DIRECT_SCOPE)
                    && self.scopes.iter().any(|scope| scope == "resource.invoke")
            }
            Provider::Claude => {
                self.access_token.starts_with("sk-ant-oat")
                    && self.scopes.iter().any(|scope| scope == "user:inference")
            }
        }
    }
}
impl AccountRecord {
    fn snapshot(&self) -> AccountSnapshot {
        let (state, can_invoke, reason) = match self.tokens.as_ref() {
            Some(tokens) if !tokens.plan_enabled(self.provider) => (
                "plan_disabled",
                false,
                Some(plan_disabled(self.provider).into()),
            ),
            Some(tokens)
                if (self.subject.is_some() || self.provider == Provider::Claude)
                    && (tokens.expires_at > now() || tokens.refresh_token.is_some()) =>
            {
                ("connected", true, None)
            }
            _ => (
                "signed_out",
                false,
                Some(sign_in_required(self.provider).into()),
            ),
        };
        let label = self
            .name
            .as_ref()
            .or(self.email.as_ref())
            .cloned()
            .unwrap_or_else(|| {
                if self.provider == Provider::Claude {
                    "Claude 구독".into()
                } else {
                    "ChatGPT 계정".into()
                }
            });
        AccountSnapshot {
            id: self.id.clone(),
            provider: self.provider,
            label: format!("{label} ({})", self.id.chars().take(6).collect::<String>()),
            email: self.email.clone(),
            state: state.into(),
            can_invoke,
            configured: true,
            reason,
        }
    }
}
impl TokenResponse {
    fn into_tokens(self, previous: Option<&Tokens>, refresh: bool) -> Result<Tokens, String> {
        if !self.token_type.eq_ignore_ascii_case("Bearer")
            || !valid_secret(&self.access_token)
            || self.expires_in == 0
        {
            return Err("ChatGPT 인증 응답을 확인하지 못했습니다.".into());
        }
        let refresh_token = self.refresh_token.filter(|token| valid_secret(token));
        if refresh && refresh_token.is_none() {
            return Err("ChatGPT 인증 갱신을 완료하지 못했습니다. 다시 로그인해 주세요.".into());
        }
        let scopes = match self.scope {
            Some(scope) => scope.split_whitespace().map(String::from).collect(),
            None => previous
                .map(|tokens| tokens.scopes.clone())
                .unwrap_or_default(),
        };
        if scopes.iter().any(|scope| scope == "offline_access") && refresh_token.is_none() {
            return Err("ChatGPT 인증 갱신 권한을 확인하지 못했습니다.".into());
        }
        let expires_at = now()
            .checked_add(self.expires_in)
            .ok_or("ChatGPT 인증 만료 시간을 확인하지 못했습니다.")?;
        Ok(Tokens {
            access_token: self.access_token,
            refresh_token,
            id_token: self
                .id_token
                .or_else(|| previous.and_then(|tokens| tokens.id_token.clone())),
            scopes,
            expires_at,
        })
    }
}
fn valid_secret(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96 * 1024
        && value.bytes().all(|byte| byte.is_ascii_graphic())
}
fn terminal_refresh(code: &str) -> bool {
    matches!(
        code,
        "invalid_grant"
            | "invalid_refresh_token"
            | "token_expired"
            | "refresh_token_expired"
            | "refresh_token_invalidated"
            | "refresh_token_reused"
    )
}
fn oauth_message(failure: &HttpFailure) -> String {
    if failure.code.as_deref().is_some_and(terminal_refresh) {
        SIGN_IN_REQUIRED.into()
    } else if failure.code.as_deref() == Some("invalid_client") {
        "ChatGPT 앱 등록 정보를 확인하지 못했습니다.".into()
    } else {
        "ChatGPT 연결을 완료하지 못했습니다. 잠시 후 다시 시도해 주세요.".into()
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn stopped(own: &AtomicBool, cancel: &AtomicBool) -> Result<(), String> {
    if own.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) {
        Err("AI 연결을 취소했습니다.".into())
    } else {
        Ok(())
    }
}
async fn with_cancel<T>(
    future: impl std::future::Future<Output = T>,
    own: &Arc<AtomicBool>,
    cancel: &Arc<AtomicBool>,
) -> Result<T, String> {
    tokio::pin!(future);
    loop {
        stopped(own, cancel)?;
        tokio::select! {value=&mut future=>{stopped(own,cancel)?;return Ok(value);},_=tokio::time::sleep(Duration::from_millis(25))=>{}}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::AtomicUsize;
    #[derive(Default)]
    struct Memory {
        values: Mutex<BTreeMap<String, Vec<u8>>>,
        reads: AtomicUsize,
        fail_write: AtomicBool,
    }
    impl credentials::Credentials for Memory {
        fn read(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            Ok(self.values.lock().unwrap().get(name).cloned())
        }
        fn write(&self, name: &str, value: &[u8]) -> Result<(), String> {
            if self.fail_write.load(Ordering::Relaxed) {
                return Err("synthetic credential write failure".into());
            }
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
    fn tokens() -> Tokens {
        Tokens {
            access_token: "SYNTHETIC_ACCESS_PRIVATE".into(),
            refresh_token: Some("SYNTHETIC_REFRESH_PRIVATE".into()),
            id_token: Some("SYNTHETIC_ID_PRIVATE".into()),
            scopes: vec![
                oidc::DIRECT_SCOPE.into(),
                "resource.invoke".into(),
                "offline_access".into(),
            ],
            expires_at: now() + 3600,
        }
    }
    fn record(id: &str, client_id: &str) -> AccountRecord {
        AccountRecord {
            provider: Provider::ChatGpt,
            id: id.into(),
            client_id: client_id.into(),
            subject: Some(format!("subject-{id}")),
            email: Some("same@example.invalid".into()),
            name: None,
            revision: 1,
            tokens: Some(tokens()),
        }
    }
    #[test]
    fn snapshots_never_expose_tokens_and_keep_same_email_registrations_separate() {
        let temp = tempfile::tempdir().unwrap();
        let manager =
            AiAccounts::with_store(temp.path().join("ai"), Arc::new(Memory::default())).unwrap();
        manager
            .save(&SavedState {
                host_id: Some("urn:uuid:fixture".into()),
                active_account_id: Some("first".into()),
                accounts: vec![
                    record("first", "oaiapp_first"),
                    record("second", "oaiapp_second"),
                ],
            })
            .unwrap();
        let snapshot = manager.snapshot().unwrap();
        assert_eq!(snapshot.accounts.len(), 2);
        assert_ne!(snapshot.accounts[0].label, snapshot.accounts[1].label);
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains("PRIVATE"));
        assert!(!encoded.contains("oaiapp_"));
        assert!(snapshot.providers[1].configured);
        assert!(snapshot.providers[1].reason.is_none());
        assert!(!temp.path().join("ai").exists());
    }
    #[test]
    fn declining_plan_scope_keeps_identity_but_never_enables_inference() {
        let mut account = record("first", "oaiapp_fixture");
        account.tokens.as_mut().unwrap().scopes = vec!["openid".into()];
        let snapshot = account.snapshot();
        assert!(!snapshot.can_invoke);
        assert_eq!(snapshot.state, "plan_disabled");
        assert!(account.subject.is_some());
    }
    #[test]
    fn refresh_replacement_is_required_and_scope_is_not_silently_elevated() {
        let previous = tokens();
        let response = TokenResponse {
            access_token: "new_access".into(),
            refresh_token: None,
            id_token: None,
            token_type: "Bearer".into(),
            expires_in: 3600,
            scope: None,
        };
        assert!(response.into_tokens(Some(&previous), true).is_err());
        let response = TokenResponse {
            access_token: "new_access".into(),
            refresh_token: Some("new_refresh".into()),
            id_token: None,
            token_type: "Bearer".into(),
            expires_in: 3600,
            scope: Some("openid".into()),
        };
        let next = response.into_tokens(Some(&previous), true).unwrap();
        assert!(!next.plan_enabled(Provider::ChatGpt));
        assert_eq!(next.refresh_token.as_deref(), Some("new_refresh"));
        assert_eq!(next.id_token, previous.id_token);
    }
    #[tokio::test]
    async fn cancelled_session_access_does_not_start_network_or_credential_reads() {
        let temp = tempfile::tempdir().unwrap();
        let manager =
            AiAccounts::with_store(temp.path().join("ai"), Arc::new(Memory::default())).unwrap();
        let cancel = Arc::new(AtomicBool::new(true));
        assert!(manager
            .subscription_session_with_cancel("none", cancel)
            .await
            .is_err());
        assert!(!temp.path().join("ai").exists());
    }
    #[tokio::test]
    async fn shutdown_cancels_browser_wait_and_blocks_operations_until_guard_is_dropped() {
        let temp = tempfile::tempdir().unwrap();
        let manager =
            AiAccounts::with_store(temp.path().join("ai"), Arc::new(Memory::default())).unwrap();
        let (pending, _) =
            oauth::Pending::begin("urn:uuid:fixture", None, None, None, None, None).unwrap();
        let pending = Arc::new(pending);
        *manager.pending.lock().unwrap() = Some(pending.clone());

        let guard = manager.prepare_shutdown().await.unwrap();
        assert!(pending.cancelled.load(Ordering::Acquire));
        assert!(manager.pending.lock().unwrap().is_none());
        assert!(manager.begin_sign_in(Provider::ChatGpt, None).is_err());
        assert!(manager.select_account("none").is_err());
        assert!(manager.sign_out("none").await.is_err());
        assert!(manager.subscription_session("none").await.is_err());
        assert!(manager.prepare_shutdown().await.is_err());
        assert!(manager.ensure_open().is_err());
        assert!(!temp.path().join("ai").exists());
        drop(guard);
        assert!(manager.ensure_open().is_ok());
    }
    #[tokio::test]
    async fn cancelling_an_accepted_sign_in_finishes_the_browser_with_failure() {
        use std::io::{Read, Write};
        let temp = tempfile::tempdir().unwrap();
        let manager =
            AiAccounts::with_store(temp.path().join("ai"), Arc::new(Memory::default())).unwrap();
        let (pending, attempt) =
            oauth::Pending::begin("urn:uuid:fixture", None, None, None, None, None).unwrap();
        let pending = Arc::new(pending);
        *manager.pending.lock().unwrap() = Some(pending.clone());
        let url = reqwest::Url::parse(&pending.redirect_uri).unwrap();
        let mut browser = std::net::TcpStream::connect(("127.0.0.1", url.port().unwrap())).unwrap();
        browser
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        write!(browser, "GET {}?state={}&code=CODE_PRIVATE&client_id=oaiapp_fixture HTTP/1.1\r\nHost: {}\r\n\r\n", url.path(), pending.state, url.authority()).unwrap();
        tokio::time::timeout(
            Duration::from_secs(2),
            pending.receive(Arc::new(AtomicBool::new(false))),
        )
        .await
        .unwrap()
        .unwrap();
        manager.cancel_sign_in(&attempt.id).unwrap();
        let mut output = String::new();
        browser.read_to_string(&mut output).unwrap();
        assert!(output.contains("AI 로그인을 취소했습니다"));
        assert!(output.contains("\"ok\":false"));
        assert!(!output.contains("\"ok\":true"));
        assert!(!output.contains("CODE_PRIVATE"));
        assert!(!temp.path().join("ai").exists());
    }
    #[tokio::test]
    async fn shutdown_waits_until_in_flight_credential_commit_has_finished() {
        let temp = tempfile::tempdir().unwrap();
        let manager =
            AiAccounts::with_store(temp.path().join("ai"), Arc::new(Memory::default())).unwrap();
        let operation = manager.operation.clone().lock_owned().await;
        let draining = manager.clone();
        let task = tokio::spawn(async move { draining.prepare_shutdown().await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !manager.closing.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!task.is_finished());
        let mut replacement = record("first", "oaiapp_fixture");
        replacement.tokens.as_mut().unwrap().refresh_token = Some("ROTATED_FIXTURE".into());
        manager
            .save(&SavedState {
                host_id: Some("urn:uuid:fixture".into()),
                active_account_id: Some("first".into()),
                accounts: vec![replacement],
            })
            .unwrap();
        drop(operation);
        let guard = task.await.unwrap().unwrap();
        let state = manager.read().unwrap();
        assert_eq!(
            state.accounts[0]
                .tokens
                .as_ref()
                .unwrap()
                .refresh_token
                .as_deref(),
            Some("ROTATED_FIXTURE")
        );
        assert!(manager.ensure_open().is_err());
        drop(guard);
        assert!(manager.ensure_open().is_ok());
    }
    #[tokio::test]
    async fn shutdown_timeout_preserves_in_flight_operation_and_restores_admission() {
        let temp = tempfile::tempdir().unwrap();
        let manager =
            AiAccounts::with_store(temp.path().join("ai"), Arc::new(Memory::default())).unwrap();
        let operation = manager.operation.clone().lock_owned().await;
        assert!(manager
            .drain_operations(Duration::from_millis(1))
            .await
            .is_err());
        assert!(manager.ensure_open().is_ok());
        assert!(manager.operation.try_lock().is_err());
        manager
            .save(&SavedState {
                host_id: Some("urn:uuid:fixture".into()),
                active_account_id: Some("first".into()),
                accounts: vec![record("first", "oaiapp_fixture")],
            })
            .unwrap();
        drop(operation);
        assert_eq!(manager.read().unwrap().accounts.len(), 1);
    }
    #[tokio::test]
    async fn queued_session_access_cannot_start_after_shutdown_admission_closes() {
        let temp = tempfile::tempdir().unwrap();
        let manager =
            AiAccounts::with_store(temp.path().join("ai"), Arc::new(Memory::default())).unwrap();
        let operation = manager.operation.clone().lock_owned().await;
        let queued = manager.clone();
        let session = tokio::spawn(async move { queued.subscription_session("none").await });
        tokio::task::yield_now().await;
        let draining = manager.clone();
        let shutdown = tokio::spawn(async move { draining.prepare_shutdown().await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !manager.closing.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(operation);
        let guard = shutdown.await.unwrap().unwrap();
        assert!(session.await.unwrap().is_err());
        assert!(!temp.path().join("ai").exists());
        drop(guard);
    }
    #[test]
    fn ordinary_account_selection_does_not_replace_another_accounts_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let manager =
            AiAccounts::with_store(temp.path().join("ai"), Arc::new(Memory::default())).unwrap();
        let first = record("first", "oaiapp_first");
        let second = record("second", "oaiapp_second");
        manager
            .save(&SavedState {
                host_id: Some("urn:uuid:fixture".into()),
                active_account_id: Some("first".into()),
                accounts: vec![first, second],
            })
            .unwrap();
        manager.select_account("second").unwrap();
        let stored = manager.read().unwrap();
        assert_eq!(stored.accounts[0].client_id, "oaiapp_first");
        assert_eq!(
            stored.accounts[0]
                .tokens
                .as_ref()
                .unwrap()
                .refresh_token
                .as_deref(),
            Some("SYNTHETIC_REFRESH_PRIVATE")
        );
        assert_eq!(stored.active_account_id.as_deref(), Some("second"));
    }
    #[test]
    fn claude_connection_has_no_invented_person_identity_and_cannot_use_api_key_tokens() {
        let mut account = record("claude-local", "fixture");
        account.provider = Provider::Claude;
        account.subject = None;
        account.email = None;
        account.name = None;
        let token = account.tokens.as_mut().unwrap();
        token.access_token = "sk-ant-oat01-fixture".into();
        token.scopes = vec!["user:inference".into()];
        let snapshot = account.snapshot();
        assert!(snapshot.can_invoke);
        assert!(snapshot.label.starts_with("Claude 구독"));
        assert!(snapshot.email.is_none());
        account.tokens.as_mut().unwrap().access_token = "sk-ant-api03-fixture".into();
        assert!(!account.snapshot().can_invoke);
    }
    #[test]
    fn cross_process_operation_lock_prevents_racing_rotating_credential_writes() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(Memory::default());
        let first = AiAccounts::with_store(temp.path().join("ai"), store.clone()).unwrap();
        let second = AiAccounts::with_store(temp.path().join("ai"), store).unwrap();
        let _held = first.file_guard().unwrap();
        assert!(second.file_guard().is_err());
    }

    #[test]
    fn repeated_public_snapshots_share_one_secret_read_across_clones_and_threads() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(Memory::default());
        credentials::write(
            store.as_ref(),
            &SavedState {
                host_id: Some("fixture-host".into()),
                active_account_id: Some("first".into()),
                accounts: vec![record("first", "fixture-client")],
            },
        )
        .unwrap();
        let manager = AiAccounts::with_store(temp.path().join("ai"), store.clone()).unwrap();
        let before = store.reads.load(Ordering::Relaxed);
        let first = manager.snapshot().unwrap();
        let after_first = store.reads.load(Ordering::Relaxed);
        assert!(after_first > before);
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let mut threads = Vec::new();
        for _ in 0..8 {
            let manager = manager.clone();
            let barrier = barrier.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..20 {
                    assert!(manager.snapshot().unwrap().accounts[0].can_invoke);
                }
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(store.reads.load(Ordering::Relaxed), after_first);
        let encoded = serde_json::to_string(&first).unwrap();
        assert!(!encoded.contains("PRIVATE"));
        assert!(!encoded.contains("fixture-client"));
        assert!(!temp.path().join("ai").exists());
    }
    #[test]
    fn successful_save_refreshes_public_metadata_without_an_extra_snapshot_secret_read() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(Memory::default());
        let manager = AiAccounts::with_store(temp.path().join("ai"), store.clone()).unwrap();
        assert!(manager.snapshot().unwrap().accounts.is_empty());
        let mut account = record("first", "fixture-client");
        account.tokens = None;
        manager
            .save(&SavedState {
                host_id: None,
                active_account_id: Some("first".into()),
                accounts: vec![account],
            })
            .unwrap();
        let after_save = store.reads.load(Ordering::Relaxed);
        let snapshot = manager.snapshot().unwrap();
        assert_eq!(snapshot.active_account_id.as_deref(), Some("first"));
        assert_eq!(snapshot.accounts[0].state, "signed_out");
        assert_eq!(store.reads.load(Ordering::Relaxed), after_save);
    }
    #[test]
    fn failed_credential_save_invalidates_public_metadata_instead_of_claiming_the_change() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(Memory::default());
        let manager = AiAccounts::with_store(temp.path().join("ai"), store.clone()).unwrap();
        let initial = SavedState {
            host_id: None,
            active_account_id: Some("first".into()),
            accounts: vec![record("first", "fixture-client")],
        };
        manager.save(&initial).unwrap();
        assert!(manager.snapshot().unwrap().accounts[0].can_invoke);
        store.fail_write.store(true, Ordering::Relaxed);
        assert!(manager.save(&SavedState::default()).is_err());
        store.fail_write.store(false, Ordering::Relaxed);
        let after_failure = store.reads.load(Ordering::Relaxed);
        let recovered = manager.snapshot().unwrap();
        assert!(recovered.accounts[0].can_invoke);
        assert!(store.reads.load(Ordering::Relaxed) > after_failure);
        let after_recovery = store.reads.load(Ordering::Relaxed);
        manager.snapshot().unwrap();
        assert_eq!(store.reads.load(Ordering::Relaxed), after_recovery);
    }
    #[tokio::test]
    async fn invocation_freshly_reads_external_changes_instead_of_trusting_public_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(Memory::default());
        let manager = AiAccounts::with_store(temp.path().join("ai"), store.clone()).unwrap();
        let mut saved = SavedState {
            host_id: None,
            active_account_id: Some("first".into()),
            accounts: vec![record("first", "fixture-client")],
        };
        manager.save(&saved).unwrap();
        assert!(manager.snapshot().unwrap().accounts[0].can_invoke);
        saved.accounts[0].tokens = None;
        saved.accounts[0].revision += 1;
        credentials::write(store.as_ref(), &saved).unwrap();
        let after_external_change = store.reads.load(Ordering::Relaxed);
        assert!(manager.snapshot().unwrap().accounts[0].can_invoke);
        assert_eq!(store.reads.load(Ordering::Relaxed), after_external_change);
        assert!(manager.subscription_session("first").await.is_err());
        assert!(store.reads.load(Ordering::Relaxed) > after_external_change);
        let after_invocation = store.reads.load(Ordering::Relaxed);
        assert!(!manager.snapshot().unwrap().accounts[0].can_invoke);
        assert_eq!(store.reads.load(Ordering::Relaxed), after_invocation);
    }
}

fn form_body(fields: &[(&str, &str)]) -> String {
    let mut url = reqwest::Url::parse("https://auth.openai.com").unwrap();
    url.query_pairs_mut().extend_pairs(fields.iter().copied());
    url.query().unwrap_or_default().to_owned()
}

fn sign_in_required(provider: Provider) -> &'static str {
    match provider {
        Provider::ChatGpt => SIGN_IN_REQUIRED,
        Provider::Claude => "Claude에 다시 로그인해 주세요.",
    }
}
fn plan_disabled(provider: Provider) -> &'static str {
    match provider {
        Provider::ChatGpt => PLAN_DISABLED,
        Provider::Claude => "Claude 구독 사용 권한이 없습니다. 다시 연결해 주세요.",
    }
}
