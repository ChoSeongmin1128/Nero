use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::Url;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const SCOPES: &str = "https://www.googleapis.com/auth/calendar.calendarlist.readonly https://www.googleapis.com/auth/calendar.events.readonly";

#[derive(Clone)]
pub struct Config {
    pub client_id: String,
    pub client_secret: Option<String>,
}
impl Config {
    pub fn bundled() -> Option<Self> {
        Self::from_values(
            option_env!("NERO_GOOGLE_CLIENT_ID"),
            option_env!("NERO_GOOGLE_CLIENT_SECRET"),
        )
    }
    fn from_values(client_id: Option<&str>, client_secret: Option<&str>) -> Option<Self> {
        let client_id = client_id?.trim();
        let client_secret = client_secret?.trim();
        if !valid_client_id(client_id) || client_secret.is_empty() {
            return None;
        }
        Some(Self {
            client_id: client_id.into(),
            client_secret: Some(client_secret.into()),
        })
    }
}
fn valid_client_id(value: &str) -> bool {
    value.len() <= 256
        && value.ends_with(".apps.googleusercontent.com")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectRequest {
    pub attempt_id: String,
    pub authorization_url: String,
}

pub struct Pending {
    pub id: String,
    listener: TcpListener,
    state: String,
    path: String,
    pub verifier: String,
    pub redirect_uri: String,
    pub cancelled: Arc<AtomicBool>,
    started: Instant,
}
impl Pending {
    pub fn begin(config: &Config) -> Result<(Self, ConnectRequest), String> {
        if !valid_client_id(&config.client_id) {
            return Err("Google 캘린더 연결 정보가 설정되지 않았습니다.".into());
        }
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| "브라우저 연결을 받을 로컬 주소를 열지 못했습니다.")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "브라우저 연결을 준비하지 못했습니다.")?;
        let port = listener
            .local_addr()
            .map_err(|_| "브라우저 연결 주소를 확인하지 못했습니다.")?
            .port();
        let id = Uuid::new_v4().to_string();
        let state = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let entropy = (0..4)
            .map(|_| Uuid::new_v4().simple().to_string())
            .collect::<String>();
        let verifier = URL_SAFE_NO_PAD.encode(Sha256::digest(entropy.as_bytes()));
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let path = format!("/oauth2/callback/{id}");
        let redirect_uri = format!("http://127.0.0.1:{port}{path}");
        let mut url = Url::parse("https://accounts.google.com/o/oauth2/v2/auth").unwrap();
        url.query_pairs_mut().extend_pairs([
            ("client_id", config.client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", SCOPES),
            ("state", state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("access_type", "offline"),
            ("prompt", "consent select_account"),
        ]);
        let request = ConnectRequest {
            attempt_id: id.clone(),
            authorization_url: url.into(),
        };
        Ok((
            Self {
                id,
                listener,
                state,
                path,
                verifier,
                redirect_uri,
                cancelled: Arc::new(AtomicBool::new(false)),
                started: Instant::now(),
            },
            request,
        ))
    }
    pub async fn receive(&self) -> Result<Authorization, String> {
        let listener = self
            .listener
            .try_clone()
            .map_err(|_| "브라우저 연결을 준비하지 못했습니다.".to_string())?;
        let state = self.state.clone();
        let path = self.path.clone();
        let host = Url::parse(&self.redirect_uri)
            .map_err(|_| "브라우저 연결 주소를 확인하지 못했습니다.".to_string())?
            .authority()
            .to_owned();
        let cancelled = self.cancelled.clone();
        let deadline = self.started + Duration::from_secs(180);
        tokio::task::spawn_blocking(move || {
            receive_blocking(listener, &host, &path, &state, &cancelled, deadline)
        })
        .await
        .map_err(|_| "브라우저 연결을 확인하지 못했습니다.".to_string())?
    }
}

fn read_headers(
    stream: &mut TcpStream,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Option<String> {
    let mut buffer = [0u8; 8192];
    let mut count = 0;
    while count < buffer.len() {
        if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
            return None;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero()
            || stream
                .set_read_timeout(Some(remaining.min(Duration::from_millis(100))))
                .is_err()
        {
            return None;
        }
        match stream.read(&mut buffer[count..]) {
            Ok(0) => return None,
            Ok(read) => count += read,
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
        if buffer[..count].ends_with(b"\r\n\r\n") {
            if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
                return None;
            }
            return std::str::from_utf8(&buffer[..count])
                .ok()
                .map(str::to_owned);
        }
    }
    None
}

fn receive_blocking(
    listener: TcpListener,
    host: &str,
    path: &str,
    state: &str,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<Authorization, String> {
    while Instant::now() < deadline {
        if cancelled.load(Ordering::Acquire) {
            return Err("Google 연결을 취소했습니다.".into());
        }
        match listener.accept() {
            Ok((mut stream, peer)) => {
                if !peer.ip().is_loopback() {
                    continue;
                }
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
                let connection_deadline = deadline.min(Instant::now() + Duration::from_secs(5));
                let Some(request) = read_headers(&mut stream, cancelled, connection_deadline)
                else {
                    continue;
                };
                if cancelled.load(Ordering::Acquire) {
                    return Err("Google 연결을 취소했습니다.".into());
                }
                if Instant::now() >= deadline {
                    break;
                }
                match callback(&request, host, path, state) {
                    Callback::Ignore => super::callback_page::CallbackPage::reject(
                        stream,
                        "연결 요청을 확인하지 못했습니다. Nero에서 다시 연결해 주세요.",
                    ),
                    Callback::Code(code) => {
                        return Ok(Authorization {
                            code,
                            page: super::callback_page::CallbackPage::begin(stream),
                        });
                    }
                    Callback::Denied => {
                        super::callback_page::CallbackPage::reject(
                            stream,
                            "Google 연결을 취소했습니다. Nero에서 다시 연결할 수 있습니다.",
                        );
                        return Err(
                            "Google 연결이 허용되지 않았습니다. 다시 연결할 수 있습니다.".into(),
                        );
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return Err("브라우저 연결을 받지 못했습니다. 다시 연결해 주세요.".into()),
        }
    }
    Err("Google 연결 시간이 지났습니다. 다시 연결해 주세요.".into())
}

pub struct Authorization {
    pub code: String,
    pub page: super::callback_page::CallbackPage,
}

enum Callback {
    Ignore,
    Code(String),
    Denied,
}
fn callback(request: &str, host: &str, path: &str, state: &str) -> Callback {
    if !request.ends_with("\r\n\r\n") {
        return Callback::Ignore;
    }
    let mut lines = request.split("\r\n");
    let words = lines
        .next()
        .unwrap_or_default()
        .split(' ')
        .collect::<Vec<_>>();
    if words.len() != 3 || words[0] != "GET" || words[2] != "HTTP/1.1" || !words[1].starts_with('/')
    {
        return Callback::Ignore;
    }
    let hosts = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect::<Vec<_>>();
    if hosts != [host] {
        return Callback::Ignore;
    }
    let Ok(url) = Url::parse(&format!("http://{host}{}", words[1])) else {
        return Callback::Ignore;
    };
    if url.path() != path || url.fragment().is_some() {
        return Callback::Ignore;
    }
    let mut values = std::collections::BTreeMap::new();
    for (key, value) in url.query_pairs() {
        if values
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Callback::Ignore;
        }
    }
    if values.get("state").map(String::as_str) != Some(state) {
        return Callback::Ignore;
    }
    if values.contains_key("error") {
        return Callback::Denied;
    }
    match values.remove("code") {
        Some(code)
            if !code.is_empty() && code.len() <= 4096 && !code.chars().any(char::is_control) =>
        {
            Callback::Code(code)
        }
        _ => Callback::Ignore,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_configuration_requires_both_registered_client_values() {
        let id = "synthetic.apps.googleusercontent.com";
        assert!(Config::from_values(Some(id), None).is_none());
        assert!(Config::from_values(Some(id), Some(" ")).is_none());
        assert!(Config::from_values(None, Some("synthetic")).is_none());
        assert!(Config::from_values(Some(id), Some("synthetic")).is_some());
    }
    #[test]
    fn callback_requires_exact_host_path_state_and_unique_parameters() {
        let request =
            |query: &str| format!("GET /callback?{query} HTTP/1.1\r\nHost: 127.0.0.1:9000\r\n\r\n");
        assert!(
            matches!(callback(&request("state=match&code=one"), "127.0.0.1:9000", "/callback", "match"), Callback::Code(value) if value == "one")
        );
        for query in [
            "state=wrong&code=one",
            "state=match&state=match&code=one",
            "state=match&code=one&code=two",
            "code=one",
        ] {
            assert!(matches!(
                callback(&request(query), "127.0.0.1:9000", "/callback", "match"),
                Callback::Ignore
            ));
        }
        assert!(matches!(
            callback(
                &request("state=match&code=one"),
                "127.0.0.1:9999",
                "/callback",
                "match"
            ),
            Callback::Ignore
        ));
        assert!(matches!(
            callback(
                &request("state=match&error=access_denied"),
                "127.0.0.1:9000",
                "/callback",
                "match"
            ),
            Callback::Denied
        ));
    }
    #[test]
    fn pkce_matches_the_rfc7636_known_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
    fn sockets() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let sender = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let receiver = listener.accept().unwrap().0;
        (sender, receiver)
    }
    #[test]
    fn slow_bytes_cannot_extend_the_absolute_header_deadline() {
        let (mut sender, mut receiver) = sockets();
        let stopped = Arc::new(AtomicBool::new(false));
        let writer_stopped = stopped.clone();
        let writer = std::thread::spawn(move || {
            while !writer_stopped.load(Ordering::Acquire) {
                if sender.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let started = Instant::now();
        assert!(read_headers(
            &mut receiver,
            &AtomicBool::new(false),
            started + Duration::from_millis(120)
        )
        .is_none());
        stopped.store(true, Ordering::Release);
        drop(receiver);
        writer.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn cancelling_an_incomplete_header_stops_the_blocking_read() {
        let (_sender, mut receiver) = sockets();
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        let cancellation = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            signal.store(true, Ordering::Release);
        });
        let started = Instant::now();
        assert!(
            read_headers(&mut receiver, &cancelled, started + Duration::from_secs(5)).is_none()
        );
        cancellation.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn expired_callback_worker_does_not_wait_for_a_connection() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        assert!(receive_blocking(
            listener,
            "127.0.0.1:1",
            "/callback",
            "synthetic",
            &AtomicBool::new(false),
            Instant::now() - Duration::from_secs(1)
        )
        .is_err());
    }
}
