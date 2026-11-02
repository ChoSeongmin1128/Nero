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
        .timeout(Duration::from_secs(600))
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
        429 => "AI 서비스가 사용량을 제한했습니다. 잠시 후 다시 시도해 주세요.",
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
            "AI 서비스가 사용량을 제한했습니다. 잠시 후 다시 시도해 주세요.".into()
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
    let request = authorize(client()?.post(endpoint), access)
        .header("Accept", "text/event-stream")
        .json(&request_body(&access.provider, model, instructions, input));
    let mut response = tokio::select! {
        response = request.send() => response.map_err(|_| "AI 서비스에 연결하지 못했습니다.")?,
        _ = wait_cancel(&cancel) => return Err("취소했습니다.".into()),
    };
    if !response.status().is_success() {
        return Err(http_failure(response, &access.provider).await);
    }
    let mut wire = Wire::default();
    let mut events = Events::default();
    loop {
        let chunk = tokio::select! {
            chunk = response.chunk() => chunk.map_err(|_| "AI 응답이 중단되었습니다. 다시 시도해 주세요.")?,
            _ = wait_cancel(&cancel) => return Err("취소했습니다.".into()),
        };
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
}
async fn wait_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
