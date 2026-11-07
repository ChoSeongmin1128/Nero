use super::{
    http::{Cancellation, Http, Request, Response},
    normalize_page_id,
    oauth::Credential,
    Error, NotionSource, Result, MCP_ENDPOINT,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

const PROTOCOL: &str = "2025-11-25";
const SUPPORTED: &[&str] = &[PROTOCOL, "2025-06-18"];
const MAX_EVENTS: usize = 128;

pub(super) struct Session<'a> {
    http: &'a dyn Http,
    token: &'a str,
    version: String,
    session_id: Option<String>,
    next_id: u64,
}
impl<'a> Session<'a> {
    pub async fn initialize(
        http: &'a dyn Http,
        credential: &'a Credential,
        cancellation: &Cancellation,
    ) -> Result<Self> {
        let mut session = Self {
            http,
            token: &credential.access_token,
            version: PROTOCOL.into(),
            session_id: None,
            next_id: 1,
        };
        let result = session.call("initialize", json!({"protocolVersion":PROTOCOL,"capabilities":{},"clientInfo":{"name":"nero-notion-context","version":env!("CARGO_PKG_VERSION")}}), cancellation).await?;
        let version = result["protocolVersion"]
            .as_str()
            .filter(|version| SUPPORTED.contains(version))
            .ok_or(Error::Protocol)?;
        if !result["capabilities"]["tools"].is_object() {
            return Err(Error::ToolUnavailable);
        }
        session.version = version.into();
        session
            .notify("notifications/initialized", None, cancellation)
            .await?;
        let mut cursor = None::<String>;
        let mut seen = BTreeSet::new();
        for _ in 0..4 {
            let result = session
                .call(
                    "tools/list",
                    cursor
                        .as_ref()
                        .map(|cursor| json!({"cursor":cursor}))
                        .unwrap_or_else(|| json!({})),
                    cancellation,
                )
                .await?;
            let tools = result["tools"].as_array().ok_or(Error::Protocol)?;
            if tools.iter().any(|tool| tool["name"] == "notion-fetch") {
                return Ok(session);
            }
            cursor = result["nextCursor"]
                .as_str()
                .filter(|cursor| !cursor.is_empty() && cursor.len() <= 8192)
                .map(str::to_owned);
            if cursor
                .as_ref()
                .is_none_or(|cursor| !seen.insert(cursor.clone()))
            {
                break;
            }
        }
        Err(Error::ToolUnavailable)
    }
    fn request(&self, body: Value, id: Option<u64>) -> Request {
        let mut headers = BTreeMap::from([
            ("authorization".into(), format!("Bearer {}", self.token)),
            (
                "accept".into(),
                "application/json, text/event-stream".into(),
            ),
            ("content-type".into(), "application/json".into()),
            ("mcp-protocol-version".into(), self.version.clone()),
        ]);
        if let Some(session_id) = &self.session_id {
            headers.insert("mcp-session-id".into(), session_id.clone());
        }
        Request {
            method: "POST",
            url: MCP_ENDPOINT.into(),
            headers,
            body: body.to_string().into_bytes(),
            rpc_id: id,
        }
    }
    async fn call(
        &mut self,
        method: &str,
        params: Value,
        cancellation: &Cancellation,
    ) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = self.request(
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            Some(id),
        );
        let result = self.http.send(request, cancellation).await;
        let response = match result {
            Ok(response) => response,
            Err(error @ (Error::Cancelled | Error::Timeout)) => {
                let short = Cancellation::new(Duration::from_millis(500));
                let _ = self
                    .notify(
                        "notifications/cancelled",
                        Some(json!({"requestId":id,"reason":"Client cancelled"})),
                        &short,
                    )
                    .await;
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        if response.status == 401 {
            return Err(Error::Reconnect);
        }
        if response.status == 403 {
            return Err(Error::DocumentUnavailable);
        }
        if !(200..300).contains(&response.status) {
            return Err(Error::Connection);
        }
        if let Some(session_id) = response.headers.get("mcp-session-id") {
            if session_id.is_empty()
                || session_id.len() > 1024
                || !session_id.bytes().all(|value| value.is_ascii_graphic())
            {
                return Err(Error::Protocol);
            }
            self.session_id = Some(session_id.clone());
        }
        rpc_result(&response, id)
    }
    async fn notify(
        &self,
        method: &str,
        params: Option<Value>,
        cancellation: &Cancellation,
    ) -> Result<()> {
        let mut body = json!({"jsonrpc":"2.0","method":method});
        if let Some(params) = params {
            body["params"] = params;
        }
        let response = self
            .http
            .send(self.request(body, None), cancellation)
            .await?;
        if response.status == 401 {
            return Err(Error::Reconnect);
        }
        if !matches!(response.status, 200 | 202 | 204) {
            return Err(Error::Protocol);
        }
        Ok(())
    }
    pub async fn fetch(
        &mut self,
        page_id: &str,
        fetched_at: u64,
        cancellation: &Cancellation,
    ) -> Result<NotionSource> {
        if normalize_page_id(page_id)? != page_id {
            return Err(Error::InvalidLink);
        }
        let value = self
            .call(
                "tools/call",
                json!({"name":"notion-fetch","arguments":{"id":page_id}}),
                cancellation,
            )
            .await?;
        parse_source(value, page_id, fetched_at)
    }
}

fn sse_messages(bytes: &[u8]) -> Result<Vec<Value>> {
    let complete = bytes
        .windows(2)
        .rposition(|value| value == b"\n\n")
        .map(|end| end + 2)
        .into_iter()
        .chain(
            bytes
                .windows(4)
                .rposition(|value| value == b"\r\n\r\n")
                .map(|end| end + 4),
        )
        .max()
        .unwrap_or(0);
    let text = std::str::from_utf8(&bytes[..complete])
        .map_err(|_| Error::Protocol)?
        .replace("\r\n", "\n");
    let mut messages = vec![];
    for event in text.split("\n\n").take(MAX_EVENTS + 1) {
        if messages.len() >= MAX_EVENTS {
            return Err(Error::Budget);
        }
        let data = event
            .lines()
            .filter_map(|line| {
                line.strip_prefix("data:")
                    .map(|value| value.strip_prefix(' ').unwrap_or(value))
            })
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            continue;
        }
        match serde_json::from_str(&data) {
            Ok(value) => messages.push(value),
            Err(_) => return Err(Error::Protocol),
        }
    }
    Ok(messages)
}
pub(super) fn sse_has_response(bytes: &[u8], id: u64) -> bool {
    sse_messages(bytes).is_ok_and(|messages| {
        messages.iter().any(|value| {
            value["id"].as_u64() == Some(id)
                && (value.get("result").is_some() || value.get("error").is_some())
        })
    })
}
fn rpc_result(response: &Response, id: u64) -> Result<Value> {
    if response.body.len() > super::http::MAX_RESPONSE_BYTES {
        return Err(Error::Budget);
    }
    let content_type = response
        .headers
        .get("content-type")
        .ok_or(Error::Protocol)?;
    let values = if content_type.starts_with("text/event-stream") {
        sse_messages(&response.body)?
    } else if content_type.starts_with("application/json") {
        vec![serde_json::from_slice(&response.body).map_err(|_| Error::Protocol)?]
    } else {
        return Err(Error::Protocol);
    };
    let mut result = None;
    for value in values {
        if value["jsonrpc"] != "2.0" {
            return Err(Error::Protocol);
        }
        if value.get("method").is_some() {
            if value.get("id").is_some() {
                return Err(Error::ServerRequest);
            }
            continue;
        }
        if value["id"].as_u64() != Some(id) {
            return Err(Error::Protocol);
        }
        if let Some(error) = value.get("error") {
            // Only a missing method means the page tool is unsupported; other
            // JSON-RPC errors are the server failing this request.
            return Err(match error["code"].as_i64() {
                Some(-32601) => Error::ToolUnavailable,
                Some(-32602) => Error::InvalidRequest,
                _ => Error::Connection,
            });
        }
        if result
            .replace(value.get("result").cloned().ok_or(Error::Protocol)?)
            .is_some()
        {
            return Err(Error::Protocol);
        }
    }
    result.ok_or(Error::Protocol)
}

fn metadata_text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value[*key].as_str().map(str::to_owned))
}
pub(super) fn parse_source(result: Value, page_id: &str, fetched_at: u64) -> Result<NotionSource> {
    if result["isError"] == true {
        return Err(Error::DocumentUnavailable);
    }
    let blocks = result["content"].as_array().ok_or(Error::Protocol)?;
    if blocks.len() > 64 {
        return Err(Error::Budget);
    }
    let text = blocks
        .iter()
        .filter(|block| block["type"] == "text")
        .map(|block| block["text"].as_str().ok_or(Error::Protocol))
        .collect::<Result<Vec<_>>>()?
        .join("\n");
    if text.is_empty() && result.get("structuredContent").is_none() {
        return Err(Error::DocumentUnavailable);
    }
    let decoded = result
        .get("structuredContent")
        .cloned()
        .or_else(|| serde_json::from_str::<Value>(&text).ok());
    let value = decoded.as_ref().map(|value| {
        value
            .get("page")
            .filter(|page| page.is_object())
            .unwrap_or(value)
    });
    if let Some(returned) =
        value.and_then(|value| value["id"].as_str().or(value["page_id"].as_str()))
    {
        if normalize_page_id(returned)? != page_id {
            return Err(Error::Protocol);
        }
    }
    if value.is_some_and(|value| {
        matches!(
            value["metadata"]["type"]
                .as_str()
                .or(value["type"].as_str()),
            Some("connector" | "database" | "data_source" | "view" | "user")
        )
    }) {
        return Err(Error::DocumentUnavailable);
    }
    let body = match value {
        Some(value) => {
            metadata_text(value, &["text", "markdown", "body", "content"]).ok_or(Error::Protocol)?
        }
        None => text,
    };
    let url = value
        .and_then(|value| value["url"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("https://www.notion.so/{}", page_id.replace('-', "")));
    if normalize_page_id(&url)? != page_id {
        return Err(Error::Protocol);
    }
    let title = value.and_then(|value| metadata_text(value, &["title", "name"]));
    let last_edited_at = value.and_then(|value| {
        metadata_text(value, &["page_last_edited_at", "last_edited_time"]).or_else(|| {
            metadata_text(
                &value["metadata"],
                &["page_last_edited_at", "last_edited_time"],
            )
        })
    });
    if last_edited_at
        .as_ref()
        .is_some_and(|value| value.parse::<jiff::Timestamp>().is_err())
    {
        return Err(Error::Protocol);
    }
    let truncated = value.is_some_and(|value| {
        value["truncated"] == true
            || value["unknown_block_ids"]
                .as_array()
                .is_some_and(|ids| !ids.is_empty())
    });
    let unknown_block_count = value.and_then(|value| value["unknown_block_count"].as_u64());
    NotionSource::new(
        page_id.into(),
        url,
        title,
        last_edited_at,
        fetched_at,
        body,
        truncated,
        unknown_block_count,
    )
}
