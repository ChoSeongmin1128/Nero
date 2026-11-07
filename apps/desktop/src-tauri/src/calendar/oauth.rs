use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::Url;
use serde::Serialize;
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::io::Write;
use std::net::TcpListener;
#[cfg(test)]
use std::net::TcpStream;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const SCOPES: &str = "https://www.googleapis.com/auth/calendar.calendarlist.readonly https://www.googleapis.com/auth/calendar.events.readonly";
pub const MEET_SCOPE: &str = "https://www.googleapis.com/auth/meetings.space.readonly";

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
    pub include_meet: bool,
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
        Self::begin_with_meet(config, false)
    }
    pub fn begin_with_meet(
        config: &Config,
        include_meet: bool,
    ) -> Result<(Self, ConnectRequest), String> {
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
        let scopes = if include_meet {
            format!("{SCOPES} {MEET_SCOPE}")
        } else {
            SCOPES.into()
        };
        url.query_pairs_mut().extend_pairs([
            ("client_id", config.client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", scopes.as_str()),
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
                include_meet,
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
        let deadline = self.started + crate::oauth_callback::CALLBACK_WINDOW;
        tokio::task::spawn_blocking(move || {
            receive_blocking(listener, &host, &path, &state, &cancelled, deadline)
        })
        .await
        .map_err(|_| "브라우저 연결을 확인하지 못했습니다.".to_string())?
    }
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
                let Some(request) =
                    crate::oauth_callback::read_request(&mut stream, connection_deadline, || {
                        cancelled.load(Ordering::Acquire)
                    })
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
    let Some(mut values) = crate::oauth_callback::query(request, host, path) else {
        return Callback::Ignore;
    };
    if values.get("state").map(String::as_str) != Some(state) {
        return Callback::Ignore;
    }
    if values.contains_key("error") {
        return Callback::Denied;
    }
    match values.remove("code") {
        Some(code) if crate::oauth_callback::valid_code(&code) => Callback::Code(code),
        _ => Callback::Ignore,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn meet_scope_is_requested_only_for_explicit_optional_reconnection() {
        let config = Config {
            client_id: "synthetic.apps.googleusercontent.com".into(),
            client_secret: Some("synthetic".into()),
        };
        let (normal, normal_request) = Pending::begin(&config).unwrap();
        let (meet, meet_request) = Pending::begin_with_meet(&config, true).unwrap();
        let scopes = |request: ConnectRequest| {
            Url::parse(&request.authorization_url)
                .unwrap()
                .query_pairs()
                .find(|(key, _)| key == "scope")
                .unwrap()
                .1
                .into_owned()
        };
        assert_eq!(scopes(normal_request), SCOPES);
        assert!(!normal.include_meet);
        assert!(meet.include_meet);
        assert_eq!(scopes(meet_request), format!("{SCOPES} {MEET_SCOPE}"));
    }
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
        assert!(crate::oauth_callback::read_request(
            &mut receiver,
            started + Duration::from_millis(120),
            || false
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
        assert!(crate::oauth_callback::read_request(
            &mut receiver,
            started + Duration::from_secs(5),
            || cancelled.load(Ordering::Acquire)
        )
        .is_none());
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
