use super::{
    http::{Cancellation, Http, Request},
    now_ms, Error, Result, MCP_ENDPOINT,
};
use crate::oauth_callback::CallbackPage;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    net::TcpListener,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Metadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub authorization_response_iss_parameter_supported: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Credential {
    pub metadata: Metadata,
    pub resource: String,
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<u64>,
    pub workspace_id: Option<String>,
    pub workspace_name: Option<String>,
}
pub(super) struct Discovery {
    pub metadata: Metadata,
    pub resource: String,
}
pub(super) struct Pending {
    pub cancelled: Cancellation,
    pub discovery: Discovery,
    pub client_id: String,
    pub verifier: String,
    pub redirect_uri: String,
    pub authorization_url: String,
    callback: Mutex<Option<tokio::sync::oneshot::Receiver<Result<String>>>>,
    page: Arc<Mutex<Option<CallbackPage>>>,
}

fn safe_text(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}
pub(super) fn notion_endpoint(value: &str) -> Result<Url> {
    if value.len() > 8192 {
        return Err(Error::Protocol);
    }
    let url = Url::parse(value).map_err(|_| Error::Protocol)?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some(
                "mcp.notion.com"
                    | "api.notion.com"
                    | "www.notion.so"
                    | "notion.so"
                    | "www.notion.com"
                    | "notion.com"
            )
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err(Error::Protocol);
    }
    Ok(url)
}
fn json_request(url: String, value: Option<Value>) -> Request {
    Request {
        method: if value.is_some() { "POST" } else { "GET" },
        url,
        headers: BTreeMap::from([
            ("accept".into(), "application/json".into()),
            ("content-type".into(), "application/json".into()),
        ]),
        body: value
            .map(|value| value.to_string().into_bytes())
            .unwrap_or_default(),
        rpc_id: None,
    }
}
async fn send_json(
    http: &dyn Http,
    request: Request,
    cancellation: &Cancellation,
) -> Result<Value> {
    let response = http.send(request, cancellation).await?;
    if !(200..300).contains(&response.status) {
        return Err(Error::Connection);
    }
    serde_json::from_slice(&response.body).map_err(|_| Error::Protocol)
}

pub(super) async fn discover(http: &dyn Http, cancellation: &Cancellation) -> Result<Discovery> {
    let mut resource = None;
    for endpoint in [
        "https://mcp.notion.com/.well-known/oauth-protected-resource/mcp",
        "https://mcp.notion.com/.well-known/oauth-protected-resource",
    ] {
        let response = http
            .send(json_request(endpoint.into(), None), cancellation)
            .await?;
        if matches!(response.status, 404 | 405) {
            continue;
        }
        if response.status != 200 {
            return Err(Error::Connection);
        }
        resource =
            Some(serde_json::from_slice::<Value>(&response.body).map_err(|_| Error::Protocol)?);
        break;
    }
    let resource = resource.ok_or(Error::Protocol)?;
    let canonical = resource["resource"].as_str().unwrap_or(MCP_ENDPOINT);
    if !matches!(
        canonical,
        "https://mcp.notion.com/mcp" | "https://mcp.notion.com"
    ) {
        return Err(Error::Protocol);
    }
    let issuer = resource["authorization_servers"]
        .as_array()
        .and_then(|servers| servers.first())
        .and_then(Value::as_str)
        .ok_or(Error::Protocol)?;
    let issuer_url = notion_endpoint(issuer)?;
    let mut metadata_url = issuer_url.clone();
    metadata_url.set_path(&format!(
        "/.well-known/oauth-authorization-server{}",
        issuer_url.path().trim_end_matches('/')
    ));
    let mut response = http
        .send(json_request(metadata_url.to_string(), None), cancellation)
        .await?;
    if matches!(response.status, 404 | 405) {
        let mut oidc = issuer_url.clone();
        oidc.set_path(&format!(
            "{}/.well-known/openid-configuration",
            issuer_url.path().trim_end_matches('/')
        ));
        response = http
            .send(json_request(oidc.to_string(), None), cancellation)
            .await?;
    }
    if response.status != 200 {
        return Err(Error::Connection);
    }
    let metadata: Metadata = serde_json::from_slice(&response.body).map_err(|_| Error::Protocol)?;
    if metadata.issuer != issuer {
        return Err(Error::Protocol);
    }
    notion_endpoint(&metadata.authorization_endpoint)?;
    notion_endpoint(&metadata.token_endpoint)?;
    notion_endpoint(
        metadata
            .registration_endpoint
            .as_deref()
            .ok_or(Error::Protocol)?,
    )?;
    Ok(Discovery {
        metadata,
        resource: canonical.into(),
    })
}

pub(super) fn authorization_url(
    discovery: &Discovery,
    client_id: &str,
    redirect_uri: &str,
    verifier: &str,
    state: &str,
) -> Result<String> {
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = notion_endpoint(&discovery.metadata.authorization_endpoint)?;
    url.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("resource", &discovery.resource),
        ("prompt", "consent"),
    ]);
    Ok(url.to_string())
}

impl Pending {
    pub async fn begin(http: &dyn Http, id: String, cancelled: Cancellation) -> Result<Self> {
        let discovery = discover(http, &cancelled).await?;
        let listener =
            TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(|_| Error::Callback)?;
        listener
            .set_nonblocking(true)
            .map_err(|_| Error::Callback)?;
        let redirect_uri = format!(
            "http://127.0.0.1:{}/notion/callback/{id}",
            listener.local_addr().map_err(|_| Error::Callback)?.port()
        );
        let registration = send_json(http, json_request(discovery.metadata.registration_endpoint.clone().ok_or(Error::Protocol)?, Some(json!({"client_name":"Nero", "redirect_uris":[redirect_uri], "grant_types":["authorization_code","refresh_token"], "response_types":["code"], "token_endpoint_auth_method":"none"}))), &cancelled).await?;
        let client_id = registration["client_id"]
            .as_str()
            .filter(|value| safe_text(value, 2048))
            .ok_or(Error::Protocol)?
            .to_owned();
        if registration["token_endpoint_auth_method"]
            .as_str()
            .is_some_and(|method| method != "none")
        {
            return Err(Error::Protocol);
        }
        let verifier = URL_SAFE_NO_PAD.encode(Sha256::digest(
            format!(
                "{}{}{}",
                uuid::Uuid::new_v4(),
                uuid::Uuid::new_v4(),
                uuid::Uuid::new_v4()
            )
            .as_bytes(),
        ));
        let state = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let authorization_url =
            authorization_url(&discovery, &client_id, &redirect_uri, &verifier, &state)?;
        let redirect = Url::parse(&redirect_uri).map_err(|_| Error::Callback)?;
        let host = redirect.authority().to_owned();
        let path = redirect.path().to_owned();
        let cancellation = cancelled.clone();
        let metadata = discovery.metadata.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let page = Arc::new(Mutex::new(None));
        let callback_page = page.clone();
        tokio::task::spawn_blocking(move || {
            let result = receive_callback(
                listener,
                &host,
                &path,
                &state,
                &metadata,
                &cancellation,
                &callback_page,
            );
            let _ = sender.send(result);
        });
        Ok(Self {
            cancelled,
            discovery,
            client_id,
            verifier,
            redirect_uri,
            authorization_url,
            callback: Mutex::new(Some(receiver)),
            page,
        })
    }
    pub async fn receive(&self) -> Result<String> {
        let receiver = self
            .callback
            .lock()
            .unwrap()
            .take()
            .ok_or(Error::InvalidRequest)?;
        receiver.await.map_err(|_| Error::Callback)?
    }
    pub fn finish_page(&self, result: std::result::Result<(), &str>) {
        if let Some(mut page) = self.page.lock().unwrap().take() {
            page.finish(result);
        }
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancelled.cancel();
        self.finish_page(Err(
            "Notion 연결이 완료되지 않았습니다. Nero에서 다시 연결해 주세요.",
        ));
    }
}

fn receive_callback(
    listener: TcpListener,
    host: &str,
    path: &str,
    state: &str,
    metadata: &Metadata,
    cancellation: &Cancellation,
    page: &Arc<Mutex<Option<CallbackPage>>>,
) -> Result<String> {
    // The same window the other sign-ins give the browser; before, it waited forever.
    let deadline = Instant::now() + crate::oauth_callback::CALLBACK_WINDOW;
    while Instant::now() < deadline {
        cancellation.check()?;
        match listener.accept() {
            Ok((mut stream, peer)) if peer.ip().is_loopback() => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                let connection_deadline = deadline.min(Instant::now() + Duration::from_secs(5));
                let Some(request) =
                    crate::oauth_callback::read_request(&mut stream, connection_deadline, || {
                        cancellation.check().is_err()
                    })
                else {
                    continue;
                };
                match parse_callback(&request, host, path, state, metadata) {
                    Ok(code) => {
                        let mut page = page.lock().unwrap();
                        if let Err(error) = cancellation.check() {
                            CallbackPage::reject(stream, &error.to_string());
                            return Err(error);
                        }
                        *page = Some(CallbackPage::begin_for(
                            stream,
                            "Notion",
                            "Notion 연결을 완료했습니다. Nero로 돌아가 주세요.",
                        ));
                        return Ok(code);
                    }
                    Err(error) => {
                        CallbackPage::reject(stream, &error.to_string());
                        if error == Error::Cancelled {
                            return Err(error);
                        }
                    }
                }
            }
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(_) => return Err(Error::Callback),
        }
    }
    Err(Error::Timeout)
}

pub(super) fn parse_callback(
    request: &str,
    host: &str,
    path: &str,
    state: &str,
    metadata: &Metadata,
) -> Result<String> {
    let mut values = crate::oauth_callback::query(request, host, path).ok_or(Error::Callback)?;
    if values.get("state").map(String::as_str) != Some(state) {
        return Err(Error::Callback);
    }
    match values.get("iss") {
        Some(value) if value != &metadata.issuer => return Err(Error::Callback),
        None if metadata.authorization_response_iss_parameter_supported => {
            return Err(Error::Callback)
        }
        _ => (),
    }
    if values.contains_key("error") {
        return Err(Error::Cancelled);
    }
    values
        .remove("code")
        .filter(|value| safe_text(value, 4096))
        .ok_or(Error::Callback)
}

async fn token_request(
    http: &dyn Http,
    metadata: &Metadata,
    fields: Vec<(&str, String)>,
    cancellation: &Cancellation,
) -> Result<Value> {
    notion_endpoint(&metadata.token_endpoint)?;
    let mut form = Url::parse("https://mcp.notion.com/").map_err(|_| Error::Protocol)?;
    form.query_pairs_mut().extend_pairs(fields);
    let response = http
        .send(
            Request {
                method: "POST",
                url: metadata.token_endpoint.clone(),
                headers: BTreeMap::from([
                    (
                        "content-type".into(),
                        "application/x-www-form-urlencoded".into(),
                    ),
                    ("accept".into(), "application/json".into()),
                ]),
                body: form.query().unwrap_or_default().as_bytes().to_vec(),
                rpc_id: None,
            },
            cancellation,
        )
        .await?;
    // A gateway error page is not JSON; it is a temporary failure, not a broken link.
    if !(200..300).contains(&response.status) {
        let value: Value = serde_json::from_slice(&response.body).unwrap_or_default();
        return Err(
            if matches!(
                value["error"].as_str(),
                Some("invalid_grant" | "invalid_client")
            ) {
                Error::Reconnect
            } else {
                Error::Connection
            },
        );
    }
    serde_json::from_slice(&response.body).map_err(|_| Error::Protocol)
}
pub(super) async fn exchange(http: &dyn Http, pending: &Pending, code: &str) -> Result<Credential> {
    let value = token_request(
        http,
        &pending.discovery.metadata,
        vec![
            ("grant_type", "authorization_code".into()),
            ("code", code.into()),
            ("client_id", pending.client_id.clone()),
            ("redirect_uri", pending.redirect_uri.clone()),
            ("code_verifier", pending.verifier.clone()),
            ("resource", pending.discovery.resource.clone()),
        ],
        &pending.cancelled,
    )
    .await?;
    credential_from_token(
        value,
        Credential {
            metadata: pending.discovery.metadata.clone(),
            resource: pending.discovery.resource.clone(),
            client_id: pending.client_id.clone(),
            access_token: String::new(),
            refresh_token: None,
            expires_at: None,
            workspace_id: None,
            workspace_name: None,
        },
    )
}
pub(super) async fn refresh(
    http: &dyn Http,
    credential: Credential,
    cancellation: &Cancellation,
) -> Result<Credential> {
    let value = token_request(
        http,
        &credential.metadata,
        vec![
            ("grant_type", "refresh_token".into()),
            ("client_id", credential.client_id.clone()),
            (
                "refresh_token",
                credential.refresh_token.clone().ok_or(Error::Reconnect)?,
            ),
            ("resource", credential.resource.clone()),
        ],
        cancellation,
    )
    .await?;
    credential_from_token(value, credential)
}
pub(super) fn credential_from_token(value: Value, mut previous: Credential) -> Result<Credential> {
    let access = value["access_token"]
        .as_str()
        .filter(|value| safe_text(value, 32 * 1024))
        .ok_or(Error::Protocol)?;
    if value["token_type"]
        .as_str()
        .is_some_and(|value| !value.eq_ignore_ascii_case("bearer"))
    {
        return Err(Error::Protocol);
    }
    previous.access_token = access.into();
    if let Some(token) = value.get("refresh_token") {
        previous.refresh_token = Some(
            token
                .as_str()
                .filter(|value| safe_text(value, 32 * 1024))
                .ok_or(Error::Protocol)?
                .into(),
        );
    }
    previous.expires_at = match value.get("expires_in") {
        None => None,
        Some(value) => Some(
            now_ms()
                .checked_add(
                    value
                        .as_u64()
                        .filter(|value| *value > 0)
                        .ok_or(Error::Protocol)?
                        .checked_mul(1000)
                        .ok_or(Error::Protocol)?,
                )
                .ok_or(Error::Protocol)?,
        ),
    };
    if let Some(workspace) = value["workspace_id"].as_str() {
        previous.workspace_id = Some(workspace.into());
    }
    if let Some(name) = value["workspace_name"].as_str() {
        previous.workspace_name = Some(name.into());
    }
    Ok(previous)
}
