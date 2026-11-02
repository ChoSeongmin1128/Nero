use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use std::time::Duration;

const MAX_BODY: usize = 4 * 1024 * 1024;
const MAX_FRAME: usize = 512 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatModel { pub id: String, pub label: String }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Models { pub models: Vec<ChatModel>, pub default_model_id: Option<String> }

pub struct ProviderAccess { pub provider: String, pub access_token: String }
impl Drop for ProviderAccess {
    fn drop(&mut self) { self.access_token.clear(); }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(20)).timeout(Duration::from_secs(600)).build()
        .map_err(|_| "AI 연결을 준비하지 못했습니다.".into())
}
fn http_error(status: reqwest::StatusCode) -> String {
    match status.as_u16() {
        401 => "AI 계정에 다시 로그인해 주세요.",
        403 => "이 계정에서 요청한 AI를 사용할 수 없습니다.",
        429 => "구독 사용 한도에 도달했습니다. 잠시 후 다시 시도해 주세요.",
        413 => "선택한 자료가 너무 많습니다. 자료를 줄여 다시 시도해 주세요.",
        500..=599 => "AI 서비스가 응답하지 않습니다. 잠시 후 다시 시도해 주세요.",
        _ => "AI가 요청을 처리하지 못했습니다. 다시 시도해 주세요.",
    }.into()
}
pub async fn models(access: &ProviderAccess) -> Result<Models, String> {
    if access.provider != "chatgpt" { return Err("이 AI 계정의 대화 연결은 아직 지원하지 않습니다.".into()); }
    let mut response = client()?.get("https://api.openai.com/v1/models")
        .bearer_auth(&access.access_token).send().await.map_err(|_| "AI 모델 목록을 불러오지 못했습니다.")?;
    if !response.status().is_success() { return Err(http_error(response.status())); }
    let mut body = Vec::new();
    while let Some(bytes) = response.chunk().await.map_err(|_| "AI 모델 목록을 불러오지 못했습니다.")? {
        if body.len() + bytes.len() > MAX_BODY { return Err("AI 모델 목록이 너무 큽니다.".into()); }
        body.extend(bytes);
    }
    let value: Value = serde_json::from_slice(&body).map_err(|_| "AI 모델 목록을 확인하지 못했습니다.")?;
    let rows = value.get("models").or_else(|| value.get("data")).and_then(Value::as_array).ok_or("AI 모델 목록을 확인하지 못했습니다.")?;
    let mut models = Vec::new();
    for row in rows {
        if row.get("visibility").and_then(Value::as_str).is_some_and(|v| v != "list") { continue; }
        let Some(id) = row.get("slug").or_else(|| row.get("id")).and_then(Value::as_str) else { continue; };
        if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) { continue; }
        if models.iter().any(|model: &ChatModel| model.id == id) { continue; }
        let label = row.get("display_name").and_then(Value::as_str).filter(|v| !v.is_empty() && v.len() <= 256 && !v.chars().any(char::is_control)).unwrap_or(id);
        models.push(ChatModel { id: id.into(), label: label.into() });
    }
    let default_model_id = models.first().map(|model| model.id.clone());
    Ok(Models { models, default_model_id })
}

#[derive(Default)]
pub(super) struct Events { pending: Vec<u8>, text: String, completed: bool, response_id: Option<String>, bytes: usize }
impl Events {
    fn event(&mut self, frame: &[u8], delta: &mut impl FnMut(&str)) -> Result<(), String> {
        let raw = std::str::from_utf8(frame).map_err(|_| "AI 응답의 글자를 확인하지 못했습니다.")?;
        let data = raw.lines().filter_map(|line| line.strip_prefix("data:").map(|v| v.strip_prefix(' ').unwrap_or(v))).collect::<Vec<_>>().join("\n");
        if data.is_empty() || data == "[DONE]" { return Ok(()); }
        let event: Value = serde_json::from_str(&data).map_err(|_| "AI 응답을 확인하지 못했습니다.")?;
        let kind = event["type"].as_str().unwrap_or_default();
        if kind == "response.created" {
            let id = event["response"]["id"].as_str().ok_or("AI 응답을 확인하지 못했습니다.")?;
            if self.response_id.as_deref().is_some_and(|old| old != id) { return Err("AI 응답이 변경되었습니다.".into()); }
            self.response_id = Some(id.into());
        }
        if kind == "response.output_text.delta" {
            if self.completed { return Err("완료된 AI 응답에 추가 내용이 도착했습니다.".into()); }
            let value = event["delta"].as_str().ok_or("AI 응답을 확인하지 못했습니다.")?;
            if self.text.len() + value.len() > MAX_BODY { return Err("AI 응답이 너무 깁니다.".into()); }
            self.text.push_str(value); delta(value);
        }
        if matches!(kind, "error" | "response.failed" | "response.incomplete") { return Err("AI가 답변을 완성하지 못했습니다. 다시 시도해 주세요.".into()); }
        if kind == "response.completed" {
            let response = &event["response"];
            let id = response["id"].as_str().ok_or("AI 응답을 확인하지 못했습니다.")?;
            if self.response_id.as_deref().is_some_and(|old| old != id) || response["status"] != "completed" { return Err("AI가 답변을 완성하지 못했습니다.".into()); }
            self.response_id = Some(id.into()); self.completed = true;
        }
        Ok(())
    }
    pub(super) fn feed(&mut self, bytes: &[u8], delta: &mut impl FnMut(&str)) -> Result<(), String> {
        self.bytes += bytes.len();
        if self.bytes > MAX_BODY * 4 { return Err("AI 응답이 너무 깁니다.".into()); }
        self.pending.extend_from_slice(bytes);
        loop {
            let lf = self.pending.windows(2).position(|v| v == b"\n\n").map(|at| (at, 2));
            let crlf = self.pending.windows(4).position(|v| v == b"\r\n\r\n").map(|at| (at, 4));
            let Some((at, width)) = [lf, crlf].into_iter().flatten().min_by_key(|(at, _)| *at) else { break; };
            let rest = self.pending.split_off(at + width);
            let frame = std::mem::replace(&mut self.pending, rest);
            self.event(&frame[..at], delta)?;
        }
        if self.pending.len() > MAX_FRAME { return Err("AI 응답을 확인하지 못했습니다.".into()); }
        Ok(())
    }
    pub(super) fn finish(mut self, delta: &mut impl FnMut(&str)) -> Result<String, String> {
        if !self.pending.is_empty() { let tail = std::mem::take(&mut self.pending); self.event(&tail, delta)?; }
        if !self.completed || self.text.trim().is_empty() { return Err("AI가 답변을 완성하지 못했습니다. 다시 시도해 주세요.".into()); }
        Ok(self.text)
    }
}

pub async fn reply(access: &ProviderAccess, model: &str, instructions: &str, input: Vec<Value>, cancel: Arc<AtomicBool>, mut delta: impl FnMut(&str)) -> Result<String, String> {
    if access.provider != "chatgpt" { return Err("이 AI 계정의 대화 연결은 아직 지원하지 않습니다.".into()); }
    if cancel.load(Ordering::Acquire) { return Err("취소했습니다.".into()); }
    let request = client()?.post("https://api.openai.com/v1/responses").bearer_auth(&access.access_token)
        .header("Accept", "text/event-stream").json(&json!({"model":model,"instructions":instructions,"input":input,"store":false,"stream":true}));
    let mut response = tokio::select! {
        response = request.send() => response.map_err(|_| "AI 서비스에 연결하지 못했습니다.")?,
        _ = wait_cancel(&cancel) => return Err("취소했습니다.".into()),
    };
    if !response.status().is_success() { return Err(http_error(response.status())); }
    let mut events = Events::default();
    loop {
        let chunk = tokio::select! {
            chunk = response.chunk() => chunk.map_err(|_| "AI 응답이 중단되었습니다. 다시 시도해 주세요.")?,
            _ = wait_cancel(&cancel) => return Err("취소했습니다.".into()),
        };
        let Some(chunk) = chunk else { break; };
        events.feed(&chunk, &mut delta)?;
        if events.completed { break; }
    }
    if cancel.load(Ordering::Acquire) { return Err("취소했습니다.".into()); }
    events.finish(&mut delta)
}
async fn wait_cancel(cancel: &AtomicBool) { while !cancel.load(Ordering::Acquire) { tokio::time::sleep(Duration::from_millis(50)).await; } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn split_utf8_and_terminal_tail_are_preserved() {
        let wire = "data: {\"type\":\"response.created\",\"response\":{\"id\":\"r\"}}\r\n\r\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"회의\"}\r\n\r\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"completed\"}}";
        let mut events = Events::default(); let mut text = String::new();
        for byte in wire.as_bytes() { events.feed(&[*byte], &mut |part| text.push_str(part)).unwrap(); }
        assert_eq!(events.finish(&mut |_|{}).unwrap(), "회의"); assert_eq!(text, "회의");
    }
    #[test] fn done_marker_without_completed_response_is_not_success() {
        let mut events = Events::default();
        events.feed(b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\ndata: [DONE]\n\n", &mut |_|{}).unwrap();
        assert!(events.finish(&mut |_|{}).is_err());
    }
    #[test] fn incomplete_and_changed_response_are_rejected() {
        let mut events = Events::default();
        events.feed(b"data: {\"type\":\"response.created\",\"response\":{\"id\":\"r1\"}}\n\n", &mut |_|{}).unwrap();
        assert!(events.feed(b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r2\",\"status\":\"completed\"}}\n\n", &mut |_|{}).is_err());
        assert!(Events::default().feed(b"data: {\"type\":\"response.incomplete\"}\n\n", &mut |_|{}).is_err());
    }
}
