use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

const MAX_BODY: usize = 4 * 1024 * 1024;
const MAX_FRAME: usize = 512 * 1024;
const INCOMPLETE: &str = "AI가 답변을 완성하지 못했습니다. 다시 시도해 주세요.";
const REPLY_TIMEOUT: Duration = Duration::from_secs(600);
const TIMED_OUT: &str = "AI 응답 시간이 초과되었습니다. 다시 시도해 주세요.";
const MAX_CLAUDE_RETRIES: usize = 2;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatModel {
    pub id: String,
    pub label: String,
    #[serde(skip)]
    max_output_tokens: Option<u64>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Models {
    pub models: Vec<ChatModel>,
    pub default_model_id: Option<String>,
}

pub struct ProviderAccess {
    pub provider: String,
    pub access_token: String,
}
impl Drop for ProviderAccess {
    fn drop(&mut self) {
        self.access_token.clear();
    }
}
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Nero/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .timeout(REPLY_TIMEOUT)
        .build()
        .map_err(|_| "AI 연결을 준비하지 못했습니다.".into())
}
fn authorize(request: reqwest::RequestBuilder, access: &ProviderAccess) -> reqwest::RequestBuilder {
    let request = request.bearer_auth(&access.access_token);
    if access.provider == "claude" {
        request
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "oauth-2025-04-20")
    } else {
        request
    }
}
fn http_error(status: reqwest::StatusCode) -> String {
    match status.as_u16() {
        401 => "AI 계정에 다시 로그인해 주세요.",
        403 => "이 계정에서 요청한 AI 연결을 허용하지 않았습니다.",
        429 => "AI 서비스가 요청을 제한했습니다. 잠시 후 다시 시도해 주세요.",
        413 => "선택한 자료가 너무 많습니다. 자료를 줄여 다시 시도해 주세요.",
        500..=599 => "AI 서비스가 응답하지 않습니다. 잠시 후 다시 시도해 주세요.",
        _ => "AI가 요청을 처리하지 못했습니다. 다시 시도해 주세요.",
    }
    .into()
}
fn safe_code(value: Option<&str>) -> Option<&str> {
    value.filter(|v| {
        !v.is_empty()
            && v.len() <= 128
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    })
}
fn failure_message(status: reqwest::StatusCode, code: Option<&str>) -> String {
    match code {
        Some("context_length_exceeded" | "input_too_long" | "request_too_large") => {
            "선택한 자료와 대화가 모델의 입력 한도를 넘었습니다. 자료를 줄여 주세요.".into()
        }
        Some("max_tokens" | "max_output_tokens" | "model_context_window_exceeded") => {
            "답변이 출력 한도에 도달했습니다. 질문을 나누어 다시 시도해 주세요.".into()
        }
        Some("invalid_api_key" | "authentication_error" | "token_expired") => {
            "AI 계정에 다시 로그인해 주세요.".into()
        }
        Some("rate_limit_error" | "rate_limit_exceeded" | "insufficient_quota") => {
            "AI 서비스가 요청을 제한했습니다. 잠시 후 다시 시도해 주세요.".into()
        }
        Some("permission_error" | "permission_denied") => {
            "이 계정에서 요청한 AI 연결을 허용하지 않았습니다.".into()
        }
        Some("invalid_request_error" | "invalid_request") => {
            "AI 서비스가 요청 형식을 거절했습니다. 연결을 다시 확인해 주세요.".into()
        }
        _ => http_error(status),
    }
}
async fn http_failure(mut response: reqwest::Response, provider: &str) -> String {
    let status = response.status();
    #[cfg(test)]
    let diagnostic_headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            let key = name.as_str();
            if key == "retry-after"
                || key == "x-should-retry"
                || key.starts_with("anthropic-ratelimit-")
            {
                value
                    .to_str()
                    .ok()
                    .filter(|v| v.len() <= 256)
                    .map(|v| (key.to_string(), Value::String(v.into())))
            } else {
                None
            }
        })
        .collect::<serde_json::Map<String, Value>>();
    let request_id = safe_code(
        response
            .headers()
            .get("request-id")
            .or_else(|| response.headers().get("x-request-id"))
            .and_then(|v| v.to_str().ok()),
    )
    .unwrap_or("unknown")
    .to_owned();
    let body = tokio::time::timeout(Duration::from_secs(2), async {
        let mut body = Vec::new();
        while let Some(bytes) = response.chunk().await.ok()? {
            if body.len() + bytes.len() > 16384 {
                return None;
            }
            body.extend(bytes);
        }
        serde_json::from_slice::<Value>(&body).ok()
    })
    .await
    .ok()
    .flatten()
    .unwrap_or(Value::Null);
    let code = safe_code(
        body["error"]["code"]
            .as_str()
            .or_else(|| body["error"]["type"].as_str()),
    );
    #[cfg(test)]
    if let Ok(path) = std::env::var("NERO_AI_PROBE_DIAGNOSTIC") {
        let message = body["error"]["message"]
            .as_str()
            .filter(|message| !message.contains("sk-ant-") && !message.contains("eyJ"))
            .map(|message| message.chars().take(2048).collect::<String>());
        let diagnostic = json!({"provider":provider,"status":status.as_u16(),"code":code,"message":message,"requestId":request_id,"headers":diagnostic_headers,"detailCode":safe_code(body["error"]["details"]["error_code"].as_str()),"syntheticInput":true});
        if let Ok(bytes) = serde_json::to_vec_pretty(&diagnostic) {
            let _ = std::fs::write(path, bytes);
        }
    }
    eprintln!(
        "Nero AI provider failure: provider={provider} status={} code={} request_id={request_id}",
        status.as_u16(),
        code.unwrap_or("unknown")
    );
    failure_message(status, code)
}
fn stream_failure(provider: &str, event: &Value) -> String {
    let code = safe_code(
        event["error"]["code"]
            .as_str()
            .or_else(|| event["error"]["type"].as_str())
            .or_else(|| event["response"]["incomplete_details"]["reason"].as_str())
            .or_else(|| event["response"]["error"]["code"].as_str()),
    );
    eprintln!(
        "Nero AI stream failure: provider={provider} code={}",
        code.unwrap_or("unknown")
    );
    code.map(|code| failure_message(reqwest::StatusCode::BAD_GATEWAY, Some(code)))
        .unwrap_or_else(|| INCOMPLETE.into())
}
fn model_rows(value: &Value) -> Result<Vec<ChatModel>, String> {
    let rows = value
        .get("models")
        .or_else(|| value.get("data"))
        .and_then(Value::as_array)
        .ok_or("AI 모델 목록을 확인하지 못했습니다.")?;
    let mut models = Vec::new();
    for row in rows {
        if row
            .get("visibility")
            .and_then(Value::as_str)
            .is_some_and(|v| v != "list")
            || row.get("lifecycle").and_then(Value::as_str) == Some("retired")
        {
            continue;
        }
        let Some(id) = row
            .get("slug")
            .or_else(|| row.get("id"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
            continue;
        }
        if models.iter().any(|model: &ChatModel| model.id == id) {
            continue;
        }
        let label = row
            .get("display_name")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty() && v.len() <= 256 && !v.chars().any(char::is_control))
            .unwrap_or(id);
        models.push(ChatModel {
            id: id.into(),
            label: label.into(),
            max_output_tokens: row["max_tokens"].as_u64().filter(|v| *v > 0),
        });
    }
    Ok(models)
}
// Claude subscription OAuth documents inference, not account model-catalog discovery.
// Model identifiers and limits: https://platform.claude.com/docs/en/models/overview (2026-11-02).
fn claude_models() -> Models {
    let models = [
        ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
        ("claude-opus-5-5", "Claude Opus 5.5"),
        ("claude-haiku-5-5", "Claude Haiku 5.5"),
    ]
    .into_iter()
    .map(|(id, label)| ChatModel {
        id: id.into(),
        label: label.into(),
        max_output_tokens: Some(128000),
    })
    .collect::<Vec<_>>();
    Models {
        default_model_id: Some(models[0].id.clone()),
        models,
    }
}
pub async fn models(access: &ProviderAccess) -> Result<Models, String> {
    if access.provider == "claude" {
        return Ok(claude_models());
    }
    if access.provider != "chatgpt" {
        return Err("AI 계정을 확인해 주세요.".into());
    }
    let mut response = authorize(
        client()?
            .get("https://api.openai.com/v1/models")
            .timeout(Duration::from_secs(30)),
        access,
    )
    .send()
    .await
    .map_err(|_| "AI 모델 목록을 불러오지 못했습니다.")?;
    if !response.status().is_success() {
        return Err(http_failure(response, &access.provider).await);
    }
    let mut body = Vec::new();
    while let Some(bytes) = response
        .chunk()
        .await
        .map_err(|_| "AI 모델 목록을 불러오지 못했습니다.")?
    {
        if body.len() + bytes.len() > MAX_BODY {
            return Err("AI 모델 목록이 너무 큽니다.".into());
        }
        body.extend(bytes);
    }
    let value = serde_json::from_slice(&body).map_err(|_| "AI 모델 목록을 확인하지 못했습니다.")?;
    let models = model_rows(&value)?;
    let default_model_id = models.first().map(|model| model.id.clone());
    Ok(Models {
        models,
        default_model_id,
    })
}
pub async fn models_with_cancel(
    access: &ProviderAccess,
    cancel: &AtomicBool,
) -> Result<Models, String> {
    tokio::select! {
        result = models(access) => result,
        _ = wait_cancel(cancel) => Err("취소했습니다.".into()),
    }
}

#[derive(Default)]
struct Wire {
    pending: Vec<u8>,
    bytes: usize,
}
impl Wire {
    fn parse(frame: &[u8]) -> Result<Option<Value>, String> {
        let raw =
            std::str::from_utf8(frame).map_err(|_| "AI 응답의 글자를 확인하지 못했습니다.")?;
        let data = raw
            .lines()
            .filter_map(|line| {
                line.strip_prefix("data:")
                    .map(|v| v.strip_prefix(' ').unwrap_or(v))
            })
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() || data == "[DONE]" {
            return Ok(None);
        }
        serde_json::from_str(&data)
            .map(Some)
            .map_err(|_| "AI 응답을 확인하지 못했습니다.".into())
    }
    fn feed(
        &mut self,
        bytes: &[u8],
        mut event: impl FnMut(Value) -> Result<(), String>,
    ) -> Result<(), String> {
        self.bytes += bytes.len();
        if self.bytes > MAX_BODY * 4 {
            return Err("AI 응답이 너무 깁니다.".into());
        }
        self.pending.extend_from_slice(bytes);
        loop {
            let lf = self
                .pending
                .windows(2)
                .position(|v| v == b"\n\n")
                .map(|at| (at, 2));
            let crlf = self
                .pending
                .windows(4)
                .position(|v| v == b"\r\n\r\n")
                .map(|at| (at, 4));
            let Some((at, width)) = [lf, crlf].into_iter().flatten().min_by_key(|(at, _)| *at)
            else {
                break;
            };
            if at > MAX_FRAME {
                return Err("AI 응답을 확인하지 못했습니다.".into());
            }
            let rest = self.pending.split_off(at + width);
            let frame = std::mem::replace(&mut self.pending, rest);
            if let Some(value) = Self::parse(&frame[..at])? {
                event(value)?;
            }
        }
        if self.pending.len() > MAX_FRAME {
            return Err("AI 응답을 확인하지 못했습니다.".into());
        }
        Ok(())
    }
    fn tail(&mut self) -> Result<Option<Value>, String> {
        Self::parse(&std::mem::take(&mut self.pending))
    }
}
#[derive(Default)]
struct Events {
    text: String,
    completed: bool,
    response_id: Option<String>,
    stop_reason: Option<String>,
    blocks: BTreeMap<u64, String>,
}
impl Events {
    fn text(&mut self, part: &str, delta: &mut impl FnMut(&str)) -> Result<(), String> {
        if self.completed {
            return Err("완료된 AI 응답에 추가 내용이 도착했습니다.".into());
        }
        if self.text.len() + part.len() > MAX_BODY {
            return Err("AI 응답이 너무 깁니다.".into());
        }
        self.text.push_str(part);
        delta(part);
        Ok(())
    }
    fn event(
        &mut self,
        provider: &str,
        event: Value,
        delta: &mut impl FnMut(&str),
    ) -> Result<(), String> {
        let kind = event["type"].as_str().unwrap_or_default();
        if matches!(kind, "error" | "response.failed" | "response.incomplete") {
            return Err(stream_failure(provider, &event));
        }
        if provider == "chatgpt" {
            match kind {
                "response.created" => {
                    let id = event["response"]["id"]
                        .as_str()
                        .ok_or("AI 응답을 확인하지 못했습니다.")?;
                    if self.response_id.as_deref().is_some_and(|old| old != id) {
                        return Err("AI 응답이 변경되었습니다.".into());
                    }
                    self.response_id = Some(id.into());
                }
                "response.output_text.delta" => self.text(
                    event["delta"]
                        .as_str()
                        .ok_or("AI 응답을 확인하지 못했습니다.")?,
                    delta,
                )?,
                "response.completed" => {
                    let response = &event["response"];
                    let id = response["id"]
                        .as_str()
                        .ok_or("AI 응답을 확인하지 못했습니다.")?;
                    if self.response_id.as_deref().is_some_and(|old| old != id)
                        || response["status"] != "completed"
                    {
                        return Err(INCOMPLETE.into());
                    }
                    if let Some(output) = response["output"].as_array() {
                        let text = output
                            .iter()
                            .flat_map(|item| item["content"].as_array().into_iter().flatten())
                            .filter(|item| item["type"] == "output_text")
                            .filter_map(|item| item["text"].as_str())
                            .collect::<String>();
                        if text != self.text {
                            return Err(
                                "완성된 답변과 수신한 내용이 다릅니다. 다시 시도해 주세요.".into(),
                            );
                        }
                    }
                    self.response_id = Some(id.into());
                    self.completed = true;
                }
                _ => {}
            }
        } else {
            match kind {
                "message_start" => {
                    if self.response_id.is_some() || self.completed {
                        return Err("AI 응답이 변경되었습니다.".into());
                    }
                    self.response_id = Some(
                        event["message"]["id"]
                            .as_str()
                            .ok_or("AI 응답을 확인하지 못했습니다.")?
                            .into(),
                    );
                }
                "content_block_start" => {
                    if self.response_id.is_none() || self.completed {
                        return Err(INCOMPLETE.into());
                    }
                    let index = event["index"]
                        .as_u64()
                        .ok_or("AI 응답을 확인하지 못했습니다.")?;
                    let block = &event["content_block"];
                    let kind = block["type"].as_str().unwrap_or_default();
                    if !matches!(kind, "text" | "thinking" | "redacted_thinking")
                        || self.blocks.insert(index, kind.into()).is_some()
                    {
                        return Err(INCOMPLETE.into());
                    }
                    if kind == "text" {
                        self.text(block["text"].as_str().unwrap_or_default(), delta)?;
                    }
                }
                "content_block_delta" => {
                    let index = event["index"]
                        .as_u64()
                        .ok_or("AI 응답을 확인하지 못했습니다.")?;
                    if !self.blocks.contains_key(&index) {
                        return Err(INCOMPLETE.into());
                    }
                    if event["delta"]["type"] == "text_delta" {
                        if self.blocks.get(&index).map(String::as_str) != Some("text") {
                            return Err(INCOMPLETE.into());
                        }
                        self.text(
                            event["delta"]["text"]
                                .as_str()
                                .ok_or("AI 응답을 확인하지 못했습니다.")?,
                            delta,
                        )?;
                    }
                }
                "content_block_stop" => {
                    let index = event["index"]
                        .as_u64()
                        .ok_or("AI 응답을 확인하지 못했습니다.")?;
                    if self.blocks.remove(&index).is_none() {
                        return Err(INCOMPLETE.into());
                    }
                }
                "message_delta" => {
                    if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                        self.stop_reason = Some(reason.into());
                    }
                }
                "message_stop" => {
                    if self.response_id.is_none() || !self.blocks.is_empty() {
                        return Err(INCOMPLETE.into());
                    }
                    if !matches!(
                        self.stop_reason.as_deref(),
                        Some("end_turn" | "stop_sequence")
                    ) {
                        return Err(failure_message(
                            reqwest::StatusCode::BAD_GATEWAY,
                            self.stop_reason.as_deref(),
                        ));
                    }
                    self.completed = true;
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn finish(self) -> Result<String, String> {
        if !self.completed || self.text.trim().is_empty() {
            return Err(INCOMPLETE.into());
        }
        Ok(self.text)
    }
}
fn request_body(provider: &str, model: &ChatModel, instructions: &str, input: Vec<Value>) -> Value {
    if provider == "claude" {
        json!({"model":model.id,"system":instructions,"messages":input,
            "max_tokens":model.max_output_tokens.unwrap_or(8192).min(32768),"stream":true})
    } else {
        json!({"model":model.id,"instructions":instructions,"input":input,"store":false,"stream":true})
    }
}
fn clone_reply_request(
    request: &reqwest::Request,
    deadline: tokio::time::Instant,
) -> Result<reqwest::Request, String> {
    let remaining = deadline
        .checked_duration_since(tokio::time::Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(TIMED_OUT)?;
    let mut request = request
        .try_clone()
        .ok_or("AI 요청을 준비하지 못했습니다.")?;
    *request.timeout_mut() = Some(remaining);
    Ok(request)
}
fn claude_retry_delay(
    provider: &str,
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    retries: usize,
    random: f64,
) -> Option<Duration> {
    if provider != "claude"
        || status != reqwest::StatusCode::TOO_MANY_REQUESTS
        || retries >= MAX_CLAUDE_RETRIES
        || headers.get("x-should-retry").and_then(|v| v.to_str().ok()) != Some("true")
        || headers
            .keys()
            .any(|name| name.as_str().starts_with("anthropic-ratelimit-unified"))
    {
        return None;
    }
    let seconds = |name: &str, divisor: f64| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|v| v / divisor)
    };
    if headers.contains_key("retry-after-ms") || headers.contains_key("retry-after") {
        let delay = seconds("retry-after-ms", 1000.0).or_else(|| seconds("retry-after", 1.0))?;
        return (delay <= 60.0).then(|| Duration::from_secs_f64(delay));
    }
    // Anthropic SDK backoff: https://github.com/anthropics/anthropic-sdk-typescript/blob/main/src/client.ts
    Some(Duration::from_secs_f64(
        0.5 * 2_f64.powi(retries as i32) * (1.0 - random.clamp(0.0, 1.0) * 0.25),
    ))
}
fn retry_random() -> f64 {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64 / u32::MAX as f64
}
async fn wait_retry(
    delay: Duration,
    deadline: tokio::time::Instant,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        return Err("취소했습니다.".into());
    }
    if deadline.saturating_duration_since(tokio::time::Instant::now()) <= delay {
        return Err(TIMED_OUT.into());
    }
    tokio::select! {
        biased;
        _ = wait_cancel(cancel) => Err("취소했습니다.".into()),
        _ = tokio::time::sleep(delay) => {
            if cancel.load(Ordering::Acquire) {
                Err("취소했습니다.".into())
            } else if tokio::time::Instant::now() >= deadline {
                Err(TIMED_OUT.into())
            } else {
                Ok(())
            }
        }
    }
}
pub async fn reply(
    access: &ProviderAccess,
    model: &ChatModel,
    instructions: &str,
    input: Vec<Value>,
    cancel: Arc<AtomicBool>,
    mut delta: impl FnMut(&str),
) -> Result<String, String> {
    let endpoint = match access.provider.as_str() {
        "chatgpt" => "https://api.openai.com/v1/responses",
        "claude" => "https://api.anthropic.com/v1/messages?beta=true",
        _ => return Err("AI 계정을 확인해 주세요.".into()),
    };
    if cancel.load(Ordering::Acquire) {
        return Err("취소했습니다.".into());
    }
    let deadline = tokio::time::Instant::now() + REPLY_TIMEOUT;
    let client = client()?;
    let request = authorize(client.post(endpoint), access)
        .header("Accept", "text/event-stream")
        .json(&request_body(&access.provider, model, instructions, input))
        .build()
        .map_err(|_| "AI 요청을 준비하지 못했습니다.")?;
    let response = async {
        let mut retries = 0;
        let mut response = loop {
            if cancel.load(Ordering::Acquire) {
                return Err("취소했습니다.".into());
            }
            let response = client
                .execute(clone_reply_request(&request, deadline)?)
                .await
                .map_err(|_| "AI 서비스에 연결하지 못했습니다.")?;
            if response.status().is_success() {
                break response;
            }
            let Some(delay) = claude_retry_delay(
                &access.provider,
                response.status(),
                response.headers(),
                retries,
                retry_random(),
            ) else {
                return Err(http_failure(response, &access.provider).await);
            };
            retries += 1;
            drop(response);
            wait_retry(delay, deadline, &cancel).await?;
        };
        let mut wire = Wire::default();
        let mut events = Events::default();
        loop {
            let chunk = response
                .chunk()
                .await
                .map_err(|_| "AI 응답이 중단되었습니다. 다시 시도해 주세요.")?;
            let Some(chunk) = chunk else {
                break;
            };
            wire.feed(&chunk, |value| {
                events.event(&access.provider, value, &mut delta)
            })?;
            if events.completed {
                break;
            }
        }
        if cancel.load(Ordering::Acquire) {
            return Err("취소했습니다.".into());
        }
        if let Some(value) = wire.tail()? {
            events.event(&access.provider, value, &mut delta)?;
        }
        events.finish()
    };
    tokio::select! {
        biased;
        _ = wait_cancel(&cancel) => Err("취소했습니다.".into()),
        result = tokio::time::timeout_at(deadline, response) => result.unwrap_or_else(|_| Err(TIMED_OUT.into())),
    }
}
async fn wait_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn retry_headers() -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-should-retry", "true".parse().unwrap());
        headers
    }
    #[test]
    fn retries_only_explicit_transient_claude_throttles_before_the_limit() {
        let mut headers = retry_headers();
        let status = reqwest::StatusCode::TOO_MANY_REQUESTS;
        assert!(claude_retry_delay("claude", status, &headers, 0, 0.0).is_some());
        assert!(claude_retry_delay("claude", status, &headers, 1, 0.0).is_some());
        assert!(claude_retry_delay("claude", status, &headers, 2, 0.0).is_none());
        assert!(claude_retry_delay("chatgpt", status, &headers, 0, 0.0).is_none());
        for status in [401, 403, 408, 409, 500, 503] {
            assert!(claude_retry_delay(
                "claude",
                reqwest::StatusCode::from_u16(status).unwrap(),
                &headers,
                0,
                0.0,
            )
            .is_none());
        }
        headers.remove("x-should-retry");
        assert!(claude_retry_delay("claude", status, &headers, 0, 0.0).is_none());
        headers.insert("x-should-retry", "false".parse().unwrap());
        assert!(claude_retry_delay("claude", status, &headers, 0, 0.0).is_none());
        headers.insert("x-should-retry", "true".parse().unwrap());
        headers.insert(
            "anthropic-ratelimit-unified-status",
            "allowed".parse().unwrap(),
        );
        assert!(claude_retry_delay("claude", status, &headers, 0, 0.0).is_none());
    }
    #[test]
    fn retry_backoff_has_bounded_exponential_jitter() {
        let headers = retry_headers();
        let delay = |retries, random| {
            claude_retry_delay(
                "claude",
                reqwest::StatusCode::TOO_MANY_REQUESTS,
                &headers,
                retries,
                random,
            )
            .unwrap()
        };
        assert_eq!(delay(0, 0.0), Duration::from_millis(500));
        assert_eq!(delay(0, 1.0), Duration::from_millis(375));
        assert_eq!(delay(1, 0.0), Duration::from_millis(1000));
        assert_eq!(delay(1, 1.0), Duration::from_millis(750));
        for _ in 0..16 {
            let delay = delay(0, retry_random());
            assert!((Duration::from_millis(375)..=Duration::from_millis(500)).contains(&delay));
        }
    }
    #[test]
    fn retry_after_is_respected_without_shortening_long_server_delays() {
        let mut headers = retry_headers();
        let delay = |headers: &reqwest::header::HeaderMap| {
            claude_retry_delay(
                "claude",
                reqwest::StatusCode::TOO_MANY_REQUESTS,
                headers,
                0,
                0.0,
            )
        };
        headers.insert("retry-after", "9".parse().unwrap());
        assert_eq!(delay(&headers), Some(Duration::from_secs(9)));
        headers.insert("retry-after-ms", "1500".parse().unwrap());
        assert_eq!(delay(&headers), Some(Duration::from_millis(1500)));
        headers.insert("retry-after-ms", "60000".parse().unwrap());
        assert_eq!(delay(&headers), Some(Duration::from_secs(60)));
        headers.insert("retry-after-ms", "60001".parse().unwrap());
        assert!(delay(&headers).is_none());
        headers.remove("retry-after-ms");
        headers.insert("retry-after", "60.001".parse().unwrap());
        assert!(delay(&headers).is_none());
        headers.insert("retry-after", "0".parse().unwrap());
        assert_eq!(delay(&headers), Some(Duration::ZERO));
        for invalid in [
            "invalid",
            "-1",
            "NaN",
            "inf",
            "Wed, 21 Oct 2030 07:28:00 GMT",
        ] {
            headers.insert("retry-after", invalid.parse().unwrap());
            assert!(delay(&headers).is_none());
        }
    }
    #[test]
    fn cloned_attempt_preserves_the_frozen_request_and_shares_its_deadline() {
        let request = reqwest::Client::new()
            .post("https://example.invalid/v1/messages?beta=true")
            .header("Authorization", "Bearer synthetic")
            .header("anthropic-beta", "oauth-2025-04-20")
            .timeout(REPLY_TIMEOUT)
            .json(&json!({"messages":[{"role":"user","content":"frozen"}],"stream":true}))
            .build()
            .unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        let attempt = clone_reply_request(&request, deadline).unwrap();
        assert_eq!(attempt.method(), request.method());
        assert_eq!(attempt.url(), request.url());
        assert_eq!(attempt.headers(), request.headers());
        assert_eq!(
            attempt.body().unwrap().as_bytes(),
            request.body().unwrap().as_bytes()
        );
        assert!(attempt.timeout().copied().unwrap() <= Duration::from_secs(1));
        assert_eq!(request.timeout().copied(), Some(REPLY_TIMEOUT));
        assert_eq!(
            clone_reply_request(&request, tokio::time::Instant::now()).unwrap_err(),
            TIMED_OUT
        );
    }
    #[tokio::test]
    async fn retry_wait_stops_promptly_on_cancellation() {
        let cancel = Arc::new(AtomicBool::new(false));
        let trigger = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            cancel.store(true, Ordering::Release);
        };
        let wait = tokio::time::timeout(
            Duration::from_millis(500),
            wait_retry(
                Duration::from_secs(60),
                tokio::time::Instant::now() + REPLY_TIMEOUT,
                &cancel,
            ),
        );
        let (result, ()) = tokio::join!(wait, trigger);
        assert_eq!(result.unwrap().unwrap_err(), "취소했습니다.");
    }
    #[tokio::test]
    async fn retry_wait_cannot_extend_the_request_deadline() {
        let cancel = AtomicBool::new(false);
        let result = wait_retry(
            Duration::from_secs(60),
            tokio::time::Instant::now() + Duration::from_millis(10),
            &cancel,
        )
        .await;
        assert_eq!(result.unwrap_err(), TIMED_OUT);
        cancel.store(true, Ordering::Release);
        assert_eq!(
            wait_retry(
                Duration::ZERO,
                tokio::time::Instant::now() + REPLY_TIMEOUT,
                &cancel
            )
            .await
            .unwrap_err(),
            "취소했습니다."
        );
    }
    fn decode(provider: &str, bytes: &[u8]) -> Result<String, String> {
        let mut wire = Wire::default();
        let mut events = Events::default();
        let mut output = String::new();
        for byte in bytes {
            wire.feed(&[*byte], |value| {
                events.event(provider, value, &mut |part| output.push_str(part))
            })?;
        }
        if let Some(value) = wire.tail()? {
            events.event(provider, value, &mut |part| output.push_str(part))?;
        }
        let text = events.finish()?;
        assert_eq!(output, text);
        Ok(text)
    }
    #[test]
    fn split_utf8_mixed_separators_and_terminal_tail_are_preserved() {
        let wire="data: {\"type\":\"response.created\",\"response\":{\"id\":\"r\"}}\r\n\r\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"회의\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"completed\"}}";
        assert_eq!(decode("chatgpt", wire.as_bytes()).unwrap(), "회의");
    }
    #[test]
    fn done_or_truncated_response_never_completes() {
        assert!(decode("chatgpt",b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\ndata: [DONE]\n\n").is_err());
        assert!(decode("chatgpt", b"data: {\"type\":\"response.incomplete\"}\n\n").is_err());
        assert!(decode("chatgpt",b"data: {\"type\":\"response.created\",\"response\":{\"id\":\"r1\"}}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r2\",\"status\":\"completed\"}}\n\n").is_err());
    }
    #[test]
    fn claude_requires_stopped_blocks_and_terminal_end_turn() {
        let prefix="data: {\"type\":\"message_start\",\"message\":{\"id\":\"m\"}}\n\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"회의\"}}\n\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n";
        let wire=format!("{prefix}data: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}}}}\n\ndata: {{\"type\":\"message_stop\"}}");
        assert_eq!(decode("claude", wire.as_bytes()).unwrap(), "회의");
        assert!(decode("claude", wire.replace("end_turn", "max_tokens").as_bytes()).is_err());
        assert!(decode("claude", prefix.as_bytes()).is_err());
    }
    #[test]
    fn model_visibility_retirement_and_no_key_fallback() {
        let models=model_rows(&json!({"data":[{"id":"visible","display_name":"Visible","max_tokens":32000},{"id":"gone","lifecycle":"retired"},{"id":"private","visibility":"hidden"}]})).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].max_output_tokens, Some(32000));
        let body = request_body(
            "claude",
            &models[0],
            "Nero meeting assistant",
            vec![json!({"role":"user","content":"hello"})],
        );
        assert_eq!(body["system"], "Nero meeting assistant");
        assert!(body.get("api_key").is_none());
        assert!(body.get("tools").is_none());
        assert_eq!(body["max_tokens"], 32000);
    }
    #[test]
    fn completed_text_mismatch_is_rejected() {
        let mut state = Events::default();
        state
            .event(
                "chatgpt",
                json!({"type":"response.output_text.delta","delta":"first"}),
                &mut |_| {},
            )
            .unwrap();
        assert!(state.event("chatgpt",json!({"type":"response.completed","response":{"id":"r","status":"completed","output":[{"content":[{"type":"output_text","text":"other"}]}]}}),&mut |_|{}).is_err());
    }
}

#[cfg(test)]
mod live_probe {
    use super::*;
    #[tokio::test]
    #[ignore = "requires an explicit subscription login in the default browser"]
    async fn subscription_login_and_response() {
        let provider =
            std::env::var("NERO_AI_PROBE_PROVIDER").expect("NERO_AI_PROBE_PROVIDER is required");
        let selected = match provider.as_str() {
            "claude" => crate::ai_accounts::Provider::Claude,
            "chatgpt" => crate::ai_accounts::Provider::ChatGpt,
            _ => panic!("invalid provider"),
        };
        let root = std::path::PathBuf::from(
            std::env::var("NERO_AI_PROBE_ROOT").expect("NERO_AI_PROBE_ROOT is required"),
        );
        std::fs::create_dir_all(&root).unwrap();
        let receipt = root.join(format!("{provider}-result.json"));
        let write = |phase: &str, error: Option<&str>, model: Option<&str>| {
            std::fs::write(&receipt,serde_json::to_vec_pretty(&json!({"provider":provider,"phase":phase,"error":error,"model":model,"syntheticInput":true})).unwrap()).unwrap();
        };
        let result=async {
            let accounts=crate::ai_accounts::AiAccounts::new(root.join("control"),root.clone())?;
            eprintln!("Nero subscription probe: reading Nero account state");
            let current=accounts.snapshot()?.accounts.into_iter().find(|account|account.provider==selected);
            let account=if std::env::var("NERO_AI_PROBE_REUSE").as_deref()==Ok("1") && current.as_ref().is_some_and(|account|account.can_invoke){current.unwrap()}else{
                let attempt=accounts.begin_sign_in(selected,current.as_ref().map(|account|account.id.clone()))?;
                let url=reqwest::Url::parse(&attempt.authorization_url).map_err(|_|"로그인 주소를 준비하지 못했습니다.".to_string())?;
                write("waiting_for_login",None,None);
                if crate::external_open::open_url(&url).is_err(){let _=accounts.cancel_sign_in(&attempt.id);return Err("기본 브라우저를 열지 못했습니다.".to_string());}
                accounts.complete_sign_in(&attempt.id,Arc::new(AtomicBool::new(false))).await?
            };
            write("authenticated",None,None);
            eprintln!("Nero subscription probe: authenticated");
            let session=accounts.subscription_session(&account.id).await?;
            let access=ProviderAccess{provider:provider.clone(),access_token:session.access_token().into()};
            let selected_model=if let Ok(model)=std::env::var("NERO_AI_PROBE_MODEL"){
                ChatModel{id:model.clone(),label:model,max_output_tokens:Some(128)}
            }else{
                let catalog=models(&access).await?;
                catalog.models.into_iter().next().ok_or("이 계정에서 사용할 모델을 찾지 못했습니다.")?
            };
            let model=&selected_model;
            write("requesting_synthetic_answer",None,Some(&model.id));
            eprintln!("Nero subscription probe: requesting synthetic answer");
            let answer=reply(&access,model,"You are Nero, a meeting assistant. Reply to this connectivity test with exactly: Nero connection verified",vec![json!({"role":"user","content":"Connectivity test. No meeting or personal data is included."})],Arc::new(AtomicBool::new(false)),|_|{}).await?;
            if answer.trim().is_empty(){return Err("AI 답변이 비어 있습니다.".to_string());}
            write("completed",None,Some(&model.id));Ok::<(),String>(())
        }.await;
        if let Err(error) = &result {
            write("failed", Some(error), None);
        }
        result.unwrap();
    }
}
