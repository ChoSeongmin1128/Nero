mod http;
mod lifecycle;
mod mcp;
mod oauth;
mod storage;
#[cfg(test)]
mod tests;
mod vault;

use http::{Cancellation, Http, ReqwestHttp};
use lifecycle::Lifecycle;
pub use lifecycle::NotionShutdownGuard;
use oauth::{Credential, Pending};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use storage::{CredentialStore, SecureCredentials, Storage};

const MCP_ENDPOINT: &str = "https://mcp.notion.com/mcp";
const MAX_PAGES: usize = 8;
const MAX_PAGE_BYTES: usize = 256 * 1024;
const MAX_CONTEXT_BYTES: usize = 512 * 1024;
const REFRESH_TIMEOUT: Duration = Duration::from_secs(30);
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidLink,
    InvalidRequest,
    Cancelled,
    Timeout,
    Budget,
    Network,
    Protocol,
    Connection,
    Reconnect,
    Credentials,
    Storage,
    Callback,
    ToolUnavailable,
    DocumentUnavailable,
    ServerRequest,
    Busy,
    Shutdown,
}
impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLink => "Notion 페이지 링크나 페이지 ID를 확인해 주세요.",
            Self::InvalidRequest => "현재 요청을 확인하지 못했습니다. 다시 시도해 주세요.",
            Self::Cancelled => "Notion 요청을 취소했습니다.", Self::Timeout => "Notion 요청 시간이 지났습니다. 다시 시도해 주세요.",
            Self::Budget => "선택한 Notion 자료가 한 번에 읽을 수 있는 범위를 넘었습니다. 문서를 나누어 선택해 주세요.",
            Self::Network => "Notion에 연결하지 못했습니다. 네트워크 상태를 확인해 주세요.",
            Self::Protocol => "Notion 연결 응답을 확인하지 못했습니다. 다시 연결해 주세요.",
            Self::Connection => "Notion에 연결하지 못했습니다. 잠시 후 다시 시도해 주세요.",
            Self::Reconnect => "Notion 연결이 만료되었거나 해제되었습니다. 다시 연결해 주세요.",
            Self::Credentials => "Nero의 Notion 연결을 보안 저장소에서 확인하지 못했습니다. 다시 연결해 주세요.",
            Self::Storage => "Nero에 저장한 Notion 자료를 확인하지 못했습니다.",
            Self::Callback => "Notion 인증 응답을 확인하지 못했습니다. 연결을 다시 시도해 주세요.",
            Self::ToolUnavailable => "연결한 Notion에서 페이지 읽기를 지원하지 않습니다.",
            Self::DocumentUnavailable => "선택한 Notion 페이지에 접근할 수 없습니다. 공유 권한과 링크를 확인해 주세요.",
            Self::ServerRequest => "Notion에서 이 자료 읽기에 지원하지 않는 추가 요청을 보냈습니다.",
            Self::Busy => "Notion 자료를 불러오고 있습니다. 완료하거나 취소한 뒤 다시 시도해 주세요.",
            Self::Shutdown => "앱이 종료를 준비하고 있습니다. 준비가 끝난 뒤 다시 시도해 주세요.",
        })
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Connection {
    schema_version: u32,
    active: bool,
    credential_slot: bool,
    workspace_id: Option<String>,
    workspace_name: Option<String>,
}
impl Default for Connection {
    fn default() -> Self {
        Self {
            schema_version: 1,
            active: false,
            credential_slot: false,
            workspace_id: None,
            workspace_name: None,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionConnectionSnapshot {
    pub connected: bool,
    pub connecting: bool,
    pub workspace_name: Option<String>,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionConnectRequest {
    pub attempt_id: String,
    pub authorization_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotionSource {
    pub source_id: String,
    pub page_id: String,
    pub url: String,
    pub title: Option<String>,
    pub last_edited_at: Option<String>,
    pub fetched_at: u64,
    pub sha256: String,
    pub body: String,
    pub truncated: bool,
    pub unknown_block_count: Option<u64>,
}
impl NotionSource {
    fn snapshot_hash(&self) -> Result<String> {
        let bytes = serde_json::to_vec(&(
            &self.page_id,
            &self.url,
            &self.title,
            &self.last_edited_at,
            self.fetched_at,
            &self.sha256,
            &self.body,
            self.truncated,
            self.unknown_block_count,
        ))
        .map_err(|_| Error::Storage)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
    fn new(
        page_id: String,
        url: String,
        title: Option<String>,
        last_edited_at: Option<String>,
        fetched_at: u64,
        body: String,
        truncated: bool,
        unknown_block_count: Option<u64>,
    ) -> Result<Self> {
        if body.len() > MAX_PAGE_BYTES {
            return Err(Error::Budget);
        }
        let sha256 = format!("{:x}", Sha256::digest(body.as_bytes()));
        let mut source = Self {
            source_id: String::new(),
            page_id,
            url,
            title,
            last_edited_at,
            fetched_at,
            sha256,
            body,
            truncated,
            unknown_block_count,
        };
        source.source_id = format!("notion:{}:{}", source.page_id, source.snapshot_hash()?);
        source.validate()?;
        Ok(source)
    }
    fn validate(&self) -> Result<()> {
        if normalize_page_id(&self.page_id)? != self.page_id
            || normalize_page_id(&self.url)? != self.page_id
            || self.body.len() > MAX_PAGE_BYTES
            || self
                .title
                .as_ref()
                .is_some_and(|value| value.len() > 2048 || value.chars().any(char::is_control))
            || self
                .last_edited_at
                .as_ref()
                .is_some_and(|value| value.parse::<jiff::Timestamp>().is_err())
            || self.sha256 != format!("{:x}", Sha256::digest(self.body.as_bytes()))
            || self.source_id != format!("notion:{}:{}", self.page_id, self.snapshot_hash()?)
        {
            return Err(Error::Storage);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotionContextBundle {
    pub kind: &'static str,
    pub sources: Vec<NotionSource>,
    pub total_bytes: usize,
}

struct Inner {
    connection: Connection,
    pending: Option<Pending>,
    connecting_id: Option<String>,
    cancellation: Option<Cancellation>,
    error: Option<String>,
    reconnect: bool,
}
struct FetchRegistration<'a> {
    id: &'a str,
    requests: &'a Mutex<BTreeMap<String, Cancellation>>,
    cancellation: Cancellation,
}
impl Drop for FetchRegistration<'_> {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.requests.lock().unwrap().remove(self.id);
    }
}
pub struct NotionContextService {
    storage: Arc<Storage>,
    credentials: Arc<dyn CredentialStore>,
    http: Arc<dyn Http>,
    inner: Arc<Mutex<Inner>>,
    generation: Arc<AtomicU64>,
    credential_gate: Arc<tokio::sync::Mutex<()>>,
    fetch_gate: tokio::sync::Mutex<()>,
    requests: Mutex<BTreeMap<String, Cancellation>>,
    lifecycle: Arc<Lifecycle>,
}
impl NotionContextService {
    pub fn new(root: PathBuf) -> Result<Self> {
        Self::with_services(
            root,
            Arc::new(SecureCredentials),
            Arc::new(ReqwestHttp::new()?),
        )
    }
    fn with_services(
        root: PathBuf,
        credentials: Arc<dyn CredentialStore>,
        http: Arc<dyn Http>,
    ) -> Result<Self> {
        let storage = Storage::new(root)?;
        let connection = storage.connection()?;
        Ok(Self {
            storage: Arc::new(storage),
            credentials,
            http,
            inner: Arc::new(Mutex::new(Inner {
                connection,
                pending: None,
                connecting_id: None,
                cancellation: None,
                error: None,
                reconnect: false,
            })),
            generation: Arc::new(AtomicU64::new(0)),
            credential_gate: Arc::new(tokio::sync::Mutex::new(())),
            fetch_gate: tokio::sync::Mutex::new(()),
            requests: Mutex::new(BTreeMap::new()),
            lifecycle: Arc::new(Lifecycle::default()),
        })
    }
    pub fn snapshot(&self) -> NotionConnectionSnapshot {
        let inner = self.inner.lock().unwrap();
        NotionConnectionSnapshot {
            connected: inner.connection.active && !inner.reconnect,
            connecting: inner.connecting_id.is_some(),
            workspace_name: inner.connection.workspace_name.clone(),
            error: inner.error.clone(),
        }
    }
    pub async fn begin_connect(&self) -> Result<NotionConnectRequest> {
        self.lifecycle.check_open()?;
        self.cancel_connect(None);
        let generation = self.generation.load(Ordering::Acquire);
        let id = uuid::Uuid::new_v4().to_string();
        let cancellation = Cancellation::new(Duration::from_secs(600));
        {
            let mut inner = self.inner.lock().unwrap();
            inner.connecting_id = Some(id.clone());
            inner.cancellation = Some(cancellation.clone());
            inner.error = None;
        }
        if let Err(error) = self.lifecycle.check_open() {
            self.cancel_connect(Some(&id));
            return Err(error);
        }
        let pending = Pending::begin(self.http.as_ref(), id.clone(), cancellation).await;
        let mut inner = self.inner.lock().unwrap();
        if self.lifecycle.check_open().is_err()
            || generation != self.generation.load(Ordering::Acquire)
            || inner.connecting_id.as_deref() != Some(&id)
        {
            return Err(Error::Cancelled);
        }
        match pending {
            Ok(pending) => {
                let request = NotionConnectRequest {
                    attempt_id: id,
                    authorization_url: pending.authorization_url.clone(),
                };
                inner.pending = Some(pending);
                Ok(request)
            }
            Err(error) => {
                inner.connecting_id = None;
                inner.cancellation = None;
                inner.error = Some(error.to_string());
                Err(error)
            }
        }
    }
    pub fn authorization_url(&self, attempt_id: &str) -> Result<String> {
        self.inner
            .lock()
            .unwrap()
            .pending
            .as_ref()
            .filter(|pending| pending.id == attempt_id)
            .map(|pending| pending.authorization_url.clone())
            .ok_or(Error::InvalidRequest)
    }
    pub fn cancel_connect(&self, attempt_id: Option<&str>) {
        let mut inner = self.inner.lock().unwrap();
        if attempt_id.is_some_and(|id| inner.connecting_id.as_deref() != Some(id)) {
            return;
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        if let Some(cancellation) = inner.cancellation.take() {
            cancellation.cancel();
        }
        inner.pending = None;
        inner.connecting_id = None;
    }
    pub async fn finish_connect(&self, attempt_id: &str) -> Result<NotionConnectionSnapshot> {
        self.lifecycle.check_open()?;
        let generation = self.generation.load(Ordering::Acquire);
        let pending = {
            let mut inner = self.inner.lock().unwrap();
            if inner.connecting_id.as_deref() != Some(attempt_id) {
                return Err(Error::InvalidRequest);
            }
            inner.pending.take().ok_or(Error::InvalidRequest)?
        };
        let result = self.complete_connect(&pending, generation).await;
        let error = result.as_ref().err().map(ToString::to_string);
        pending.finish_page(error.as_deref().map_or(Ok(()), Err));
        let mut inner = self.inner.lock().unwrap();
        if generation == self.generation.load(Ordering::Acquire)
            && inner.connecting_id.as_deref() == Some(attempt_id)
        {
            inner.connecting_id = None;
            inner.cancellation = None;
            if let Err(error) = &result {
                inner.error = Some(error.to_string());
            }
        }
        drop(inner);
        result?;
        Ok(self.snapshot())
    }
    async fn complete_connect(&self, pending: &Pending, generation: u64) -> Result<()> {
        let code = pending.receive().await?;
        let gate = pending
            .cancelled
            .run(async { Ok(self.credential_gate.clone().lock_owned().await) })
            .await?;
        pending.cancelled.check()?;
        self.lifecycle.check_open()?;
        let credential = oauth::exchange(self.http.as_ref(), pending, &code).await?;
        mcp::Session::initialize(self.http.as_ref(), &credential, &pending.cancelled).await?;
        pending.cancelled.check()?;
        let operation = self.lifecycle.enter()?;
        let inner = self.inner.clone();
        let storage = self.storage.clone();
        let credentials = self.credentials.clone();
        let current_generation = self.generation.clone();
        let cancellation = pending.cancelled.clone();
        tokio::task::spawn_blocking(move || {
            let _gate = gate;
            let _operation = operation;
            let next = {
                let inner = inner.lock().unwrap();
                if generation != current_generation.load(Ordering::Acquire) {
                    return Err(Error::Cancelled);
                }
                Connection {
                    schema_version: 1,
                    active: true,
                    credential_slot: !inner.connection.credential_slot,
                    workspace_id: credential.workspace_id.clone(),
                    workspace_name: credential.workspace_name.clone(),
                }
            };
            credentials.save(next.credential_slot, &credential)?;
            cancellation.check()?;
            let mut inner = inner.lock().unwrap();
            if generation != current_generation.load(Ordering::Acquire) {
                return Err(Error::Cancelled);
            }
            storage.save_connection(&next)?;
            inner.connection = next;
            inner.error = None;
            inner.reconnect = false;
            Ok(())
        })
        .await
        .map_err(|_| Error::Storage)?
    }
    pub async fn disconnect(&self) -> Result<NotionConnectionSnapshot> {
        self.lifecycle.check_open()?;
        self.cancel_connect(None);
        for request in self.requests.lock().unwrap().values() {
            request.cancel();
        }
        let gate = self.credential_gate.clone().lock_owned().await;
        let operation = self.lifecycle.enter()?;
        let inner = self.inner.clone();
        let storage = self.storage.clone();
        let credentials = self.credentials.clone();
        tokio::task::spawn_blocking(move || {
            let _gate = gate;
            let _operation = operation;
            let next = Connection::default();
            storage.save_connection(&next)?;
            {
                let mut inner = inner.lock().unwrap();
                inner.connection = next;
                inner.error = None;
                inner.reconnect = false;
            }
            credentials.delete()
        })
        .await
        .map_err(|_| Error::Storage)??;
        Ok(self.snapshot())
    }
    async fn credential(
        &self,
        cancellation: &Cancellation,
        force_refresh: bool,
        generation: u64,
    ) -> Result<Credential> {
        let gate = cancellation
            .run(async { Ok(self.credential_gate.clone().lock_owned().await) })
            .await?;
        cancellation.check()?;
        self.lifecycle.check_open()?;
        let slot = {
            let inner = self.inner.lock().unwrap();
            if generation != self.generation.load(Ordering::Acquire) {
                return Err(Error::Cancelled);
            }
            if !inner.connection.active || inner.reconnect {
                return Err(Error::Reconnect);
            }
            inner.connection.credential_slot
        };
        let mut credential = self.credentials.load(slot)?;
        if force_refresh
            || credential
                .expires_at
                .is_some_and(|expiry| expiry <= now_ms().saturating_add(30_000))
        {
            cancellation.check()?;
            let operation = self.lifecycle.enter()?;
            let http = self.http.clone();
            let credentials = self.credentials.clone();
            let inner = self.inner.clone();
            let task = tokio::spawn(async move {
                let _gate = gate;
                let _operation = operation;
                let transaction = Cancellation::new(REFRESH_TIMEOUT);
                let refreshed = oauth::refresh(http.as_ref(), credential, &transaction).await;
                let credential = match refreshed {
                    Ok(credential) => credential,
                    Err(error) => {
                        if error == Error::Reconnect {
                            let mut inner = inner.lock().unwrap();
                            if inner.connection.credential_slot == slot {
                                inner.reconnect = true;
                                inner.error = Some(error.to_string());
                            }
                        }
                        return Err(error);
                    }
                };
                let saved = credential.clone();
                tokio::task::spawn_blocking(move || credentials.save(slot, &saved))
                    .await
                    .map_err(|_| Error::Credentials)??;
                Ok(credential)
            });
            let refreshed = task.await.map_err(|_| Error::Credentials)?;
            cancellation.check()?;
            self.lifecycle.check_open()?;
            credential = refreshed?;
        }
        cancellation.check()?;
        self.lifecycle.check_open()?;
        Ok(credential)
    }
    pub async fn fetch_sources(
        &self,
        request_id: String,
        links: Vec<String>,
    ) -> Result<NotionContextBundle> {
        self.lifecycle.check_open()?;
        if request_id.is_empty()
            || request_id.len() > 120
            || !request_id.bytes().all(|value| value.is_ascii_graphic())
        {
            return Err(Error::InvalidRequest);
        }
        if links.is_empty() || links.len() > MAX_PAGES {
            return Err(Error::Budget);
        }
        let mut seen = BTreeSet::new();
        let pages = links
            .iter()
            .map(|link| normalize_page_id(link))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|id| seen.insert(id.clone()))
            .collect::<Vec<_>>();
        let _fetch = self.fetch_gate.try_lock().map_err(|_| Error::Busy)?;
        let cancellation = Cancellation::new(Duration::from_secs(60));
        {
            let mut requests = self.requests.lock().unwrap();
            if requests.contains_key(&request_id) {
                return Err(Error::InvalidRequest);
            }
            requests.insert(request_id.clone(), cancellation.clone());
        }
        let _registration = FetchRegistration {
            id: &request_id,
            requests: &self.requests,
            cancellation: cancellation.clone(),
        };
        let generation = self.generation.load(Ordering::Acquire);
        let result = self.fetch_pages(&pages, &cancellation, generation).await;
        if let Err(error) = &result {
            let mut inner = self.inner.lock().unwrap();
            if generation == self.generation.load(Ordering::Acquire) {
                inner.error = Some(error.to_string());
                if matches!(error, Error::Reconnect) {
                    inner.reconnect = true;
                }
            }
        }
        result
    }
    async fn fetch_pages(
        &self,
        pages: &[String],
        cancellation: &Cancellation,
        generation: u64,
    ) -> Result<NotionContextBundle> {
        for retry in 0..2 {
            let credential = self
                .credential(cancellation, retry != 0, generation)
                .await?;
            let mut session =
                match mcp::Session::initialize(self.http.as_ref(), &credential, cancellation).await
                {
                    Err(Error::Reconnect) if retry == 0 => continue,
                    value => value?,
                };
            let mut sources = vec![];
            let mut total_bytes = 0usize;
            let mut unauthorized = false;
            for page in pages {
                let source = match session.fetch(page, now_ms(), cancellation).await {
                    Err(Error::Reconnect) if retry == 0 => {
                        unauthorized = true;
                        break;
                    }
                    value => value?,
                };
                total_bytes = total_bytes.saturating_add(source.body.len());
                if total_bytes > MAX_CONTEXT_BYTES {
                    return Err(Error::Budget);
                }
                sources.push(source);
            }
            if unauthorized {
                continue;
            }
            cancellation.check()?;
            let _operation = self.lifecycle.enter()?;
            let inner = self.inner.lock().unwrap();
            if generation != self.generation.load(Ordering::Acquire) || !inner.connection.active {
                return Err(Error::Cancelled);
            }
            for source in &sources {
                self.storage.save_source(source)?;
            }
            drop(inner);
            return Ok(NotionContextBundle {
                kind: "external_reference",
                sources,
                total_bytes,
            });
        }
        Err(Error::Reconnect)
    }
    pub fn cancel_fetch(&self, request_id: &str) {
        if let Some(cancellation) = self.requests.lock().unwrap().get(request_id) {
            cancellation.cancel();
        }
    }
    pub async fn prepare_shutdown(&self) -> Result<NotionShutdownGuard> {
        self.prepare_shutdown_with_timeout(REFRESH_TIMEOUT).await
    }
    async fn prepare_shutdown_with_timeout(
        &self,
        timeout: Duration,
    ) -> Result<NotionShutdownGuard> {
        let guard = self.lifecycle.close()?;
        self.cancel_connect(None);
        for cancellation in self.requests.lock().unwrap().values() {
            cancellation.cancel();
        }
        self.lifecycle.drain(timeout).await?;
        Ok(guard)
    }
    pub fn read_sources(&self, source_ids: Vec<String>) -> Result<NotionContextBundle> {
        self.context_bundle(&source_ids)
    }
    pub fn context_bundle(&self, source_ids: &[String]) -> Result<NotionContextBundle> {
        if source_ids.len() > MAX_PAGES {
            return Err(Error::Budget);
        }
        let mut sources = vec![];
        let mut seen = BTreeSet::new();
        let mut total_bytes = 0usize;
        for source_id in source_ids {
            if !seen.insert(source_id.clone()) {
                continue;
            }
            let source = self.storage.source(&source_id)?;
            total_bytes = total_bytes.saturating_add(source.body.len());
            if total_bytes > MAX_CONTEXT_BYTES {
                return Err(Error::Budget);
            }
            sources.push(source);
        }
        Ok(NotionContextBundle {
            kind: "external_reference",
            sources,
            total_bytes,
        })
    }
}

pub fn normalize_page_id(input: &str) -> Result<String> {
    let value = input.trim();
    if value.is_empty() || value.len() > 8192 || value.chars().any(char::is_control) {
        return Err(Error::InvalidLink);
    }
    let is_url = value.to_ascii_lowercase().starts_with("https://")
        || value.to_ascii_lowercase().starts_with("http://");
    let candidate = if is_url {
        let url = Url::parse(value).map_err(|_| Error::InvalidLink)?;
        let host = url.host_str().ok_or(Error::InvalidLink)?;
        if url.scheme() != "https"
            || !(matches!(
                host,
                "notion.so"
                    | "www.notion.so"
                    | "app.notion.com"
                    | "notion.site"
                    | "www.notion.site"
            ) || host.ends_with(".notion.site")
                || host.ends_with(".notion.so"))
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(Error::InvalidLink);
        }
        url.path_segments()
            .and_then(|segments| segments.filter(|value| !value.is_empty()).next_back())
            .ok_or(Error::InvalidLink)?
            .to_owned()
    } else {
        value.to_owned()
    };
    if matches!(candidate.len(), 32 | 36) {
        if let Ok(id) = uuid::Uuid::parse_str(&candidate) {
            return Ok(id.hyphenated().to_string());
        }
    }
    if !is_url {
        return Err(Error::InvalidLink);
    }
    let raw = candidate
        .get(candidate.len().saturating_sub(32)..)
        .ok_or(Error::InvalidLink)?;
    if raw.len() != 32
        || !raw.bytes().all(|value| value.is_ascii_hexdigit())
        || (candidate.len() > 32 && !candidate[..candidate.len() - 32].ends_with('-'))
    {
        return Err(Error::InvalidLink);
    }
    uuid::Uuid::parse_str(raw)
        .map(|id| id.hyphenated().to_string())
        .map_err(|_| Error::InvalidLink)
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default()
}
