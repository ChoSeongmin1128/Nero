use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatModel {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub effort: EffortCapability,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub previous: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffortCapability {
    pub supported: Option<bool>,
    pub levels: Vec<String>,
    pub default: Option<String>,
}
impl EffortCapability {
    pub fn allows(&self, value: &str) -> bool {
        self.supported == Some(true) && self.levels.iter().any(|level| level == value)
    }
}
fn catalog_effort(row: &Value) -> EffortCapability {
    let mut supported = row["supports_reasoning"].as_bool();
    let mut levels = Vec::new();
    if supported != Some(false) {
        if let Some(rows) = row["supported_reasoning_levels"].as_array() {
            for item in rows {
                let level = item.as_str().or_else(|| item["effort"].as_str());
                if let Some(level) = level.filter(|v| {
                    matches!(
                        *v,
                        "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
                    )
                }) {
                    if !levels.iter().any(|v| v == level) {
                        levels.push(level.to_string());
                    }
                }
            }
        }
    }
    if supported.is_none() && !levels.is_empty() {
        supported = Some(true);
    }
    let default = row["default_reasoning_level"]
        .as_str()
        .filter(|v| levels.iter().any(|level| level == *v))
        .map(str::to_owned);
    if supported.is_some() || !levels.is_empty() {
        return EffortCapability {
            supported,
            levels,
            default,
        };
    }
    // OpenAI model pages, reviewed 2026-11-03:
    // https://developers.openai.com/api/docs/models/gpt-6-astra
    // https://developers.openai.com/api/docs/models/gpt-6.1-sol
    // https://developers.openai.com/api/docs/models/gpt-6-sol
    // https://developers.openai.com/api/docs/models/gpt-6-luna
    let id = row["slug"].as_str().or_else(|| row["id"].as_str());
    let (values, default): (&[&str], Option<&str>) = match id {
        Some("gpt-6-astra") => (&["low", "medium", "high", "xhigh", "max"], None),
        Some("gpt-6.1-sol") => (&["low", "medium", "high", "xhigh", "max"], Some("medium")),
        Some("gpt-6-sol" | "gpt-6-luna") => (
            &["none", "low", "medium", "high", "xhigh", "max"],
            Some("medium"),
        ),
        _ => return EffortCapability::default(),
    };
    EffortCapability {
        supported: Some(true),
        levels: values.iter().map(|v| (*v).to_owned()).collect(),
        default: default.map(str::to_owned),
    }
}
#[derive(Clone, Serialize)]
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
    request.bearer_auth(&access.access_token)
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
            if key == "retry-after" || key == "x-should-retry" {
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
            // A streaming `error` event carries its code at the top level.
            .or_else(|| event["code"].as_str())
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
            effort: catalog_effort(row),
            previous: false,
        });
    }
    Ok(models)
}
pub async fn models(access: &ProviderAccess) -> Result<Models, String> {
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
                            "완성된 답변과 수신한 내용이 다릅니다. 다시 시도해 주세요.".into()
                        );
                    }
                }
                self.response_id = Some(id.into());
                self.completed = true;
            }
            _ => {}
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
fn request_body(
    model: &ChatModel,
    instructions: &str,
    input: Vec<Value>,
    effort: Option<&str>,
) -> Value {
    let mut body = json!({"model":model.id,"instructions":instructions,"input":input,"store":false,"stream":true});
    if let Some(effort) = effort {
        body["reasoning"] = json!({"effort":effort});
    }
    body
}
pub async fn reply(
    access: &ProviderAccess,
    model: &ChatModel,
    instructions: &str,
    input: Vec<Value>,
    effort: Option<&str>,
    cancel: Arc<AtomicBool>,
    mut delta: impl FnMut(&str),
) -> Result<String, String> {
    let endpoint = match access.provider.as_str() {
        "chatgpt" => "https://api.openai.com/v1/responses",
        _ => return Err("AI 계정을 확인해 주세요.".into()),
    };
    if cancel.load(Ordering::Acquire) {
        return Err("취소했습니다.".into());
    }
    let deadline = tokio::time::Instant::now() + REPLY_TIMEOUT;
    let client = client()?;
    let request = authorize(client.post(endpoint), access)
        .header("Accept", "text/event-stream")
        .json(&request_body(model, instructions, input, effort))
        .build()
        .map_err(|_| "AI 요청을 준비하지 못했습니다.")?;
    let response = async {
        let mut response = client
            .execute(request)
            .await
            .map_err(|_| "AI 서비스에 연결하지 못했습니다.")?;
        if !response.status().is_success() {
            return Err(http_failure(response, &access.provider).await);
        }
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
    #[test]
    fn a_streaming_error_event_is_read_by_its_top_level_code() {
        let event = serde_json::json!({"type":"error","code":"rate_limit_exceeded","message":"x"});
        assert_eq!(
            stream_failure("chatgpt", &event),
            "AI 서비스가 요청을 제한했습니다. 잠시 후 다시 시도해 주세요."
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
    fn model_visibility_retirement_and_no_key_fallback() {
        let models=model_rows(&json!({"data":[{"id":"visible","display_name":"Visible"},{"id":"gone","lifecycle":"retired"},{"id":"private","visibility":"hidden"}]})).unwrap();
        assert_eq!(models.len(), 1);
        let body = request_body(
            &models[0],
            "Nero meeting assistant",
            vec![json!({"role":"user","content":"hello"})],
            None,
        );
        assert_eq!(body["instructions"], "Nero meeting assistant");
        assert!(body.get("api_key").is_none());
        assert!(body.get("tools").is_none());
    }
    #[test]
    fn effort_metadata_and_documented_fallback_are_model_specific() {
        let models = model_rows(&json!({"models":[
            {"slug":"gpt-6.1-sol"},
            {"slug":"gpt-6-astra","supports_reasoning":false,"supported_reasoning_levels":[{"effort":"high"}]},
            {"slug":"custom","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}],"default_reasoning_level":"high"},
            {"slug":"unknown"}
        ]})).unwrap();
        assert!(models[0].effort.allows("xhigh"));
        assert!(!models[0].effort.allows("none"));
        assert_eq!(models[0].effort.default.as_deref(), Some("medium"));
        assert_eq!(models[1].effort.supported, Some(false));
        assert!(models[1].effort.levels.is_empty());
        assert!(!models[1].effort.allows("high"));
        assert!(models[2].effort.allows("high"));
        assert_eq!(models[3].effort.supported, None);
        let body = request_body(&models[0], "test", vec![], Some("high"));
        assert_eq!(body["reasoning"]["effort"], "high");
        assert!(request_body(&models[0], "test", vec![], None)
            .get("reasoning")
            .is_none());
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
                ChatModel{id:model.clone(),label:model,effort:EffortCapability::default(),previous:false}
            }else{
                let catalog=models(&access).await?;
                catalog.models.into_iter().next().ok_or("이 계정에서 사용할 모델을 찾지 못했습니다.")?
            };
            let model=&selected_model;
            write("requesting_synthetic_answer",None,Some(&model.id));
            eprintln!("Nero subscription probe: requesting synthetic answer");
            let answer=reply(&access,model,"You are Nero, a meeting assistant. Reply to this connectivity test with exactly: Nero connection verified",vec![json!({"role":"user","content":"Connectivity test. No meeting or personal data is included."})],None,Arc::new(AtomicBool::new(false)),|_|{}).await?;
            if answer.trim().is_empty(){return Err("AI 답변이 비어 있습니다.".to_string());}
            write("completed",None,Some(&model.id));Ok::<(),String>(())
        }.await;
        if let Err(error) = &result {
            write("failed", Some(error), None);
        }
        result.unwrap();
    }
}
