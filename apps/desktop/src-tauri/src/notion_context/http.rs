use super::{Error, Result};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

pub(super) const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub(super) type ResponseFuture<'a> = Pin<Box<dyn Future<Output = Result<Response>> + Send + 'a>>;

#[derive(Clone)]
pub(super) struct Cancellation {
    stopped: Arc<AtomicBool>,
    deadline: Instant,
}
impl Cancellation {
    pub fn new(duration: Duration) -> Self {
        Self {
            stopped: Arc::new(AtomicBool::new(false)),
            deadline: Instant::now() + duration,
        }
    }
    pub fn cancel(&self) {
        self.stopped.store(true, Ordering::Release);
    }
    pub fn check(&self) -> Result<()> {
        if self.stopped.load(Ordering::Acquire) {
            Err(Error::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(Error::Timeout)
        } else {
            Ok(())
        }
    }
    pub async fn run<T>(&self, future: impl Future<Output = Result<T>>) -> Result<T> {
        self.check()?;
        tokio::pin!(future);
        loop {
            tokio::select! {
                value = &mut future => return value,
                _ = tokio::time::sleep(Duration::from_millis(25)) => self.check()?,
            }
        }
    }
}

pub(super) struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub rpc_id: Option<u64>,
}
pub(super) struct Response {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}
pub(super) trait Http: Send + Sync {
    fn send<'a>(&'a self, request: Request, cancellation: &'a Cancellation) -> ResponseFuture<'a>;
}

pub(super) struct ReqwestHttp(reqwest::Client);

// The 30-second request limit is a timeout, which also tells the server to cancel.
fn transport_error(error: reqwest::Error) -> Error {
    if error.is_timeout() {
        Error::Timeout
    } else {
        Error::Network
    }
}
impl ReqwestHttp {
    pub fn new() -> Result<Self> {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("Nero-Notion/", env!("CARGO_PKG_VERSION")))
            .build()
            .map(Self)
            .map_err(|_| Error::Network)
    }
}
impl Http for ReqwestHttp {
    fn send<'a>(&'a self, request: Request, cancellation: &'a Cancellation) -> ResponseFuture<'a> {
        Box::pin(async move {
            cancellation
                .run(async {
                    let method = reqwest::Method::from_bytes(request.method.as_bytes())
                        .map_err(|_| Error::Protocol)?;
                    let mut headers = HeaderMap::new();
                    for (key, value) in request.headers {
                        headers.insert(
                            HeaderName::from_bytes(key.as_bytes()).map_err(|_| Error::Protocol)?,
                            HeaderValue::from_str(&value).map_err(|_| Error::Protocol)?,
                        );
                    }
                    let mut response = self
                        .0
                        .request(method, &request.url)
                        .headers(headers)
                        .body(request.body)
                        .send()
                        .await
                        .map_err(transport_error)?;
                    if response
                        .content_length()
                        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
                    {
                        return Err(Error::Budget);
                    }
                    let status = response.status().as_u16();
                    let headers = response
                        .headers()
                        .iter()
                        .filter_map(|(key, value)| {
                            value
                                .to_str()
                                .ok()
                                .map(|value| (key.as_str().to_ascii_lowercase(), value.to_owned()))
                        })
                        .collect::<BTreeMap<_, _>>();
                    let sse = headers
                        .get("content-type")
                        .is_some_and(|value| value.starts_with("text/event-stream"));
                    let mut body = vec![];
                    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
                        cancellation.check()?;
                        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                            return Err(Error::Budget);
                        }
                        body.extend_from_slice(&chunk);
                        if sse
                            && request
                                .rpc_id
                                .is_some_and(|id| super::mcp::sse_has_response(&body, id))
                        {
                            break;
                        }
                    }
                    Ok(Response {
                        status,
                        headers,
                        body,
                    })
                })
                .await
        })
    }
}
