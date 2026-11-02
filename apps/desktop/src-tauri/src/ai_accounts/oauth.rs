use super::{claude, oidc, Provider};
use crate::oauth_callback::CallbackPage;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

pub(super) struct Pending {
    pub id: String,
    pub provider: Provider,
    pub host_id: String,
    pub selected_id: Option<String>,
    pub expected_revision: Option<u64>,
    pub account_id: String,
    pub client_id: Option<String>,
    pub verifier: String,
    pub nonce: String,
    pub redirect_uri: String,
    pub cancelled: Arc<AtomicBool>,
    pub completing: AtomicBool,
    listener: TcpListener,
    pub state: String,
    started: Instant,
    page: Arc<Mutex<Option<CallbackPage>>>,
}
pub struct SignInAttempt {
    pub id: String,
    pub authorization_url: String,
}
pub(super) struct Authorization {
    pub code: String,
    pub client_id: String,
}

pub(super) fn random_value() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "안전한 연결을 준비하지 못했습니다.".to_string())?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
impl Pending {
    pub fn begin(
        host_id: &str,
        selected_id: Option<String>,
        revision: Option<u64>,
        client_id: Option<String>,
        id_hint: Option<&str>,
        email: Option<&str>,
    ) -> Result<(Self, SignInAttempt), String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|_| "브라우저 연결을 준비하지 못했습니다.")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "브라우저 연결을 준비하지 못했습니다.")?;
        let port = listener
            .local_addr()
            .map_err(|_| "브라우저 연결을 준비하지 못했습니다.")?
            .port();
        let id = uuid::Uuid::new_v4().to_string();
        let state = random_value()?;
        let nonce = random_value()?;
        let verifier = random_value()?;
        let redirect_uri = format!("http://127.0.0.1:{port}/auth/callback");
        let mut url =
            reqwest::Url::parse("https://auth.openai.com/api/accounts/authorize").unwrap();
        url.query_pairs_mut().extend_pairs([
            (
                "client_id",
                client_id.as_deref().unwrap_or("dynamic_agent_client"),
            ),
            ("ext_agent_host_id", host_id),
            ("response_type", "code"),
            ("redirect_uri", redirect_uri.as_str()),
            ("scope", oidc::SCOPES),
            ("resource", oidc::RESOURCE),
            ("state", state.as_str()),
            ("nonce", nonce.as_str()),
            ("code_challenge_method", "S256"),
            (
                "code_challenge",
                URL_SAFE_NO_PAD
                    .encode(Sha256::digest(verifier.as_bytes()))
                    .as_str(),
            ),
        ]);
        if client_id.is_none() {
            url.query_pairs_mut().append_pair("agent_name_hint", "Nero");
        } else {
            if let Some(hint) = id_hint {
                url.query_pairs_mut().append_pair("id_token_hint", hint);
            }
            if let Some(email) = email {
                url.query_pairs_mut().append_pair("login_hint", email);
            }
        }
        let account_id = selected_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Ok((
            Self {
                id: id.clone(),
                provider: Provider::ChatGpt,
                host_id: host_id.into(),
                selected_id,
                expected_revision: revision,
                account_id,
                client_id,
                verifier,
                nonce,
                redirect_uri,
                cancelled: Arc::new(AtomicBool::new(false)),
                completing: AtomicBool::new(false),
                listener,
                state,
                started: Instant::now(),
                page: Arc::new(Mutex::new(None)),
            },
            SignInAttempt {
                id,
                authorization_url: url.into(),
            },
        ))
    }
    pub fn begin_claude(
        host_id: &str,
        selected_id: Option<String>,
        revision: Option<u64>,
    ) -> Result<(Self, SignInAttempt), String> {
        let listener = TcpListener::bind(("127.0.0.1", 53692))
            .or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
            .map_err(|_| "브라우저 연결을 준비하지 못했습니다.")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "브라우저 연결을 준비하지 못했습니다.")?;
        let port = listener
            .local_addr()
            .map_err(|_| "브라우저 연결을 준비하지 못했습니다.")?
            .port();
        let id = uuid::Uuid::new_v4().to_string();
        let state = random_value()?;
        let nonce = random_value()?;
        let verifier = random_value()?;
        let redirect_uri = format!("http://localhost:{port}/callback");
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut url = reqwest::Url::parse(claude::AUTHORIZE).unwrap();
        url.query_pairs_mut().extend_pairs([
            ("code", "true"),
            ("client_id", claude::CLIENT_ID),
            ("response_type", "code"),
            ("redirect_uri", redirect_uri.as_str()),
            ("scope", claude::SCOPES),
            ("state", state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ]);
        let account_id = selected_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Ok((
            Self {
                id: id.clone(),
                provider: Provider::Claude,
                host_id: host_id.into(),
                selected_id,
                expected_revision: revision,
                account_id,
                client_id: Some(claude::CLIENT_ID.into()),
                verifier,
                nonce,
                redirect_uri,
                cancelled: Arc::new(AtomicBool::new(false)),
                completing: AtomicBool::new(false),
                listener,
                state,
                started: Instant::now(),
                page: Arc::new(Mutex::new(None)),
            },
            SignInAttempt {
                id,
                authorization_url: url.into(),
            },
        ))
    }
    pub async fn receive(&self, external: Arc<AtomicBool>) -> Result<Authorization, String> {
        let listener = self
            .listener
            .try_clone()
            .map_err(|_| "브라우저 연결을 확인하지 못했습니다.")?;
        let state = self.state.clone();
        let path = reqwest::Url::parse(&self.redirect_uri)
            .unwrap()
            .path()
            .to_string();
        let client_id = self.client_id.clone();
        let own = self.cancelled.clone();
        let host = reqwest::Url::parse(&self.redirect_uri)
            .unwrap()
            .authority()
            .to_string();
        let deadline = self.started + Duration::from_secs(180);
        let page = self.page.clone();
        let provider = match self.provider {
            Provider::ChatGpt => "ChatGPT",
            Provider::Claude => "Claude",
        };
        tokio::task::spawn_blocking(move || {
            receive(
                listener,
                &host,
                &path,
                &state,
                client_id.as_deref(),
                &own,
                &external,
                deadline,
                page,
                provider,
            )
        })
        .await
        .map_err(|_| "브라우저 연결을 확인하지 못했습니다.".to_string())?
    }
    pub fn finish_page(&self, result: Result<(), &str>) {
        if let Ok(mut page) = self.page.lock() {
            if let Some(mut page) = page.take() {
                page.finish(result);
            }
        }
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.finish_page(Err("연결이 중단되었습니다. Nero에서 다시 연결해 주세요."));
    }
}

fn cancelled(own: &AtomicBool, external: &AtomicBool) -> bool {
    own.load(Ordering::Acquire) || external.load(Ordering::Acquire)
}
fn receive(
    listener: TcpListener,
    host: &str,
    path: &str,
    state: &str,
    client: Option<&str>,
    own: &AtomicBool,
    external: &AtomicBool,
    deadline: Instant,
    page: Arc<Mutex<Option<CallbackPage>>>,
    provider: &str,
) -> Result<Authorization, String> {
    while Instant::now() < deadline {
        if cancelled(own, external) {
            return Err("AI 로그인을 취소했습니다.".into());
        }
        match listener.accept() {
            Ok((mut stream, peer)) => {
                if !peer.ip().is_loopback() {
                    continue;
                }
                let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
                let local_deadline = deadline.min(Instant::now() + Duration::from_secs(5));
                let Some(request) = read_headers(&mut stream, own, external, local_deadline) else {
                    CallbackPage::reject(
                        stream,
                        "연결 요청을 확인하지 못했습니다. Nero에서 다시 연결해 주세요.",
                    );
                    continue;
                };
                let result = callback_for_path(&request, host, path, state, client);
                if cancelled(own, external) {
                    CallbackPage::reject(stream, "AI 로그인을 취소했습니다.");
                    return Err("AI 로그인을 취소했습니다.".into());
                }
                match result {
                    None => CallbackPage::reject(
                        stream,
                        "연결 요청을 확인하지 못했습니다. Nero에서 다시 연결해 주세요.",
                    ),
                    Some(Err(message)) => {
                        CallbackPage::reject(stream, &message);
                        return Err(message);
                    }
                    Some(Ok(authorization)) => {
                        let mut callback =
                            CallbackPage::begin_for(stream, provider, "Nero로 돌아가 주세요.");
                        let mut target = page
                            .lock()
                            .map_err(|_| "브라우저 연결 상태를 확인하지 못했습니다.")?;
                        if cancelled(own, external) {
                            callback.finish(Err("AI 로그인을 취소했습니다."));
                            return Err("AI 로그인을 취소했습니다.".into());
                        }
                        if target.is_some() {
                            return Err("AI 로그인을 확인 중입니다.".into());
                        }
                        *target = Some(callback);
                        return Ok(authorization);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(_) => return Err("브라우저 연결을 받지 못했습니다.".into()),
        }
    }
    Err("AI 로그인 시간이 지났습니다. 다시 시도해 주세요.".into())
}
fn read_headers(
    stream: &mut TcpStream,
    own: &AtomicBool,
    external: &AtomicBool,
    deadline: Instant,
) -> Option<String> {
    let mut bytes = [0u8; 8192];
    let mut length = 0;
    while length < bytes.len() && Instant::now() < deadline && !cancelled(own, external) {
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .ok()?;
        match stream.read(&mut bytes[length..]) {
            Ok(0) => return None,
            Ok(count) => length += count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                continue
            }
            Err(_) => return None,
        }
        if bytes[..length].ends_with(b"\r\n\r\n") {
            return std::str::from_utf8(&bytes[..length]).ok().map(String::from);
        }
    }
    None
}
fn callback_for_path(
    request: &str,
    host: &str,
    path: &str,
    state: &str,
    selected_client: Option<&str>,
) -> Option<Result<Authorization, String>> {
    if !request.ends_with("\r\n\r\n") {
        return None;
    }
    let mut lines = request.split("\r\n");
    let words: Vec<_> = lines.next()?.split(' ').collect();
    if words.len() != 3 || words[0] != "GET" || words[2] != "HTTP/1.1" || !words[1].starts_with('/')
    {
        return None;
    }
    let hosts: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect();
    if hosts != [host] {
        return None;
    }
    let url = reqwest::Url::parse(&format!("http://{host}{}", words[1])).ok()?;
    if url.path() != path || url.fragment().is_some() {
        return None;
    }
    let mut values = BTreeMap::new();
    for (name, value) in url.query_pairs() {
        if values
            .insert(name.into_owned(), value.into_owned())
            .is_some()
        {
            return None;
        }
    }
    if values.get("state").map(String::as_str) != Some(state) {
        return None;
    }
    if values.contains_key("error") {
        return Some(Err("AI 로그인이 허용되지 않았습니다.".into()));
    }
    let code = values.remove("code")?;
    if code.is_empty() || code.len() > 4096 || code.chars().any(char::is_control) {
        return None;
    }
    let issued = values.remove("client_id");
    let client_id = match (selected_client, issued) {
        (Some(expected), Some(actual)) if expected == actual => actual,
        (Some(expected), None) => expected.into(),
        (None, Some(actual)) if valid_client(&actual) => actual,
        _ => {
            return Some(Err(
                "AI 앱 등록 정보를 확인하지 못했습니다. 다시 로그인해 주세요.".into(),
            ))
        }
    };
    Some(Ok(Authorization { code, client_id }))
}
pub(super) fn valid_client(value: &str) -> bool {
    value != "dynamic_agent_client"
        && !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
fn callback(
    request: &str,
    host: &str,
    state: &str,
    client: Option<&str>,
) -> Option<Result<Authorization, String>> {
    callback_for_path(request, host, "/auth/callback", state, client)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn send_callback(pending: &Pending, state: &str) -> TcpStream {
        let url = reqwest::Url::parse(&pending.redirect_uri).unwrap();
        let mut browser = TcpStream::connect(("127.0.0.1", url.port().unwrap())).unwrap();
        browser
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        write!(browser, "GET {}?state={state}&code=CODE_PRIVATE&client_id=oaiapp_fixture HTTP/1.1\r\nHost: {}\r\n\r\n", url.path(), url.authority()).unwrap();
        browser
    }

    #[tokio::test]
    async fn accepted_callback_remains_pending_until_credential_commit_result() {
        let (pending, _) =
            Pending::begin("urn:uuid:fixture", None, None, None, None, None).unwrap();
        let mut browser = send_callback(&pending, &pending.state);
        let authorization = tokio::time::timeout(
            Duration::from_secs(2),
            pending.receive(Arc::new(AtomicBool::new(false))),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(authorization.code, "CODE_PRIVATE");
        browser
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut initial = String::new();
        let _ = browser.read_to_string(&mut initial);
        assert!(initial.contains("ChatGPT 연결 중"));
        assert!(!initial.contains("finishConnection({"));
        assert!(!initial.contains("CODE_PRIVATE"));
        assert!(!initial.contains("oaiapp_fixture"));
        pending.finish_page(Err("계정 정보를 저장하지 못했습니다."));
        browser
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut completed = String::new();
        browser.read_to_string(&mut completed).unwrap();
        assert!(completed.contains("\"ok\":false"));
        assert!(completed.contains("계정 정보를 저장하지 못했습니다"));
        assert!(!completed.contains("\"ok\":true"));
    }

    #[tokio::test]
    async fn dropping_an_accepted_callback_reports_failure_without_success() {
        let (pending, _) =
            Pending::begin("urn:uuid:fixture", None, None, None, None, None).unwrap();
        let mut browser = send_callback(&pending, &pending.state);
        tokio::time::timeout(
            Duration::from_secs(2),
            pending.receive(Arc::new(AtomicBool::new(false))),
        )
        .await
        .unwrap()
        .unwrap();
        drop(pending);
        let mut output = String::new();
        browser.read_to_string(&mut output).unwrap();
        assert!(output.contains("연결이 중단되었습니다"));
        assert!(output.contains("\"ok\":false"));
        assert!(!output.contains("\"ok\":true"));
    }

    #[tokio::test]
    async fn invalid_callback_is_rejected_without_consuming_the_valid_attempt() {
        let (pending, _) =
            Pending::begin("urn:uuid:fixture", None, None, None, None, None).unwrap();
        let pending = Arc::new(pending);
        let client = pending.clone();
        let browser = std::thread::spawn(move || {
            let mut rejected = send_callback(&client, "wrong-state");
            let mut output = String::new();
            rejected.read_to_string(&mut output).unwrap();
            (output, send_callback(&client, &client.state))
        });
        tokio::time::timeout(
            Duration::from_secs(2),
            pending.receive(Arc::new(AtomicBool::new(false))),
        )
        .await
        .unwrap()
        .unwrap();
        let (rejected, mut accepted) = browser.join().unwrap();
        assert!(rejected.starts_with("HTTP/1.1 400 Bad Request"));
        assert!(rejected.contains("\"ok\":false"));
        assert!(!rejected.contains("CODE_PRIVATE"));
        pending.finish_page(Ok(()));
        let mut output = String::new();
        accepted.read_to_string(&mut output).unwrap();
        assert!(output.contains("\"ok\":true"));
    }
    #[test]
    fn dynamic_registration_uses_nero_and_fresh_pkce_without_any_other_clients_identity() {
        let (first, request) =
            Pending::begin("urn:uuid:fixture", None, None, None, None, None).unwrap();
        let (second, second_request) =
            Pending::begin("urn:uuid:fixture", None, None, None, None, None).unwrap();
        let url = reqwest::Url::parse(&request.authorization_url).unwrap();
        let query: BTreeMap<_, _> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(query["client_id"], "dynamic_agent_client");
        assert_eq!(query["agent_name_hint"], "Nero");
        assert_eq!(query["code_challenge_method"], "S256");
        assert_eq!(
            query["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(first.verifier.as_bytes()))
        );
        assert_ne!(first.state, second.state);
        assert_ne!(request.authorization_url, second_request.authorization_url);
        assert!(first.redirect_uri.ends_with("/auth/callback"));
    }
    #[test]
    fn callback_binds_state_issued_client_and_exact_loopback_uri() {
        let request = |query: &str| {
            format!("GET /auth/callback?{query} HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n\r\n")
        };
        assert!(matches!(
            callback(
                &request("state=match&code=one&client_id=oaiapp_fixture"),
                "127.0.0.1:1234",
                "match",
                None
            ),
            Some(Ok(_))
        ));
        for query in [
            "state=wrong&code=one&client_id=oaiapp_fixture",
            "state=match&code=one&code=two&client_id=oaiapp_fixture",
            "state=match&code=one&client_id=dynamic_agent_client",
        ] {
            assert!(!matches!(
                callback(&request(query), "127.0.0.1:1234", "match", None),
                Some(Ok(_))
            ));
        }
        assert!(matches!(
            callback(
                &request("state=match&code=one&client_id=other"),
                "127.0.0.1:1234",
                "match",
                Some("oaiapp_fixture")
            ),
            Some(Err(_))
        ));
        assert!(callback(
            &request("state=match&code=one"),
            "127.0.0.1:9999",
            "match",
            Some("oaiapp_fixture")
        )
        .is_none());
        assert!(matches!(
            callback(
                &request("state=match&error=access_denied"),
                "127.0.0.1:1234",
                "match",
                None
            ),
            Some(Err(_))
        ));
    }
    #[tokio::test]
    async fn cancelling_the_browser_attempt_does_not_wait_for_the_timeout() {
        let (pending, _) =
            Pending::begin("urn:uuid:fixture", None, None, None, None, None).unwrap();
        pending.cancelled.store(true, Ordering::Release);
        assert!(pending
            .receive(Arc::new(AtomicBool::new(false)))
            .await
            .is_err());
    }
    #[test]
    fn claude_browser_flow_is_pkce_bound_and_requests_subscription_scopes_only() {
        let (pending, attempt) = Pending::begin_claude("urn:uuid:fixture", None, None).unwrap();
        let url = reqwest::Url::parse(&attempt.authorization_url).unwrap();
        let query: BTreeMap<_, _> = url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        assert_eq!(url.host_str(), Some("claude.ai"));
        assert_eq!(query["client_id"], claude::CLIENT_ID);
        assert_eq!(query["scope"], "user:profile user:inference");
        assert_ne!(pending.state, pending.verifier);
        assert_eq!(
            query["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(pending.verifier.as_bytes()))
        );
        let callback_url = reqwest::Url::parse(&pending.redirect_uri).unwrap();
        assert_eq!(callback_url.host_str(), Some("localhost"));
        assert_eq!(callback_url.path(), "/callback");
        let host = callback_url.authority();
        let request = format!(
            "GET /callback?state={}&code=fixture HTTP/1.1\r\nHost: {host}\r\n\r\n",
            pending.state
        );
        assert!(matches!(
            callback_for_path(
                &request,
                host,
                "/callback",
                &pending.state,
                Some(claude::CLIENT_ID)
            ),
            Some(Ok(_))
        ));
        assert!(callback_for_path(
            &request,
            host,
            "/auth/callback",
            &pending.state,
            Some(claude::CLIENT_ID)
        )
        .is_none());
    }
}
