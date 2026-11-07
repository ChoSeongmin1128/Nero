use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

// How long a loopback listener waits for the browser once sign-in opens.
pub const CALLBACK_WINDOW: Duration = Duration::from_secs(180);

// One HTTP request head of at most 8 KiB, read until `deadline` or `cancelled()`.
pub fn read_request(
    stream: &mut TcpStream,
    deadline: Instant,
    cancelled: impl Fn() -> bool,
) -> Option<String> {
    let mut buffer = [0u8; 8192];
    let mut count = 0;
    while count < buffer.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if cancelled()
            || remaining.is_zero()
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
            if cancelled() || Instant::now() >= deadline {
                return None;
            }
            return std::str::from_utf8(&buffer[..count])
                .ok()
                .map(str::to_owned);
        }
    }
    None
}

// The query of a complete `GET <path> HTTP/1.1` request that names exactly `host`,
// with no fragment and no repeated parameter. Callers check state, errors and code.
pub fn query(request: &str, host: &str, path: &str) -> Option<BTreeMap<String, String>> {
    if !request.ends_with("\r\n\r\n") {
        return None;
    }
    let mut lines = request.split("\r\n");
    let words = lines.next()?.split(' ').collect::<Vec<_>>();
    if words.len() != 3 || words[0] != "GET" || words[2] != "HTTP/1.1" || !words[1].starts_with('/')
    {
        return None;
    }
    let hosts = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect::<Vec<_>>();
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
    Some(values)
}

// An authorization code as accepted from a callback: 1 to 4096 characters, no controls.
pub fn valid_code(code: &str) -> bool {
    !code.is_empty() && code.len() <= 4096 && !code.chars().any(char::is_control)
}

pub struct CallbackPage {
    stream: Option<TcpStream>,
    nonce: String,
}

impl CallbackPage {
    pub fn begin(stream: TcpStream) -> Self {
        Self::begin_for(
            stream,
            "Google Calendar",
            "Nero에서 캘린더를 확인해 주세요.",
        )
    }

    pub fn begin_for(stream: TcpStream, provider: &str, success_message: &str) -> Self {
        Self::start(stream, "200 OK", provider, success_message)
    }

    pub fn reject(stream: TcpStream, message: &str) {
        Self::start(stream, "400 Bad Request", "Nero", "").finish(Err(message));
    }

    fn start(mut stream: TcpStream, status: &str, provider: &str, success_message: &str) -> Self {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
        let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nTransfer-Encoding: chunked\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; style-src 'nonce-{nonce}'; script-src 'nonce-{nonce}'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'\r\nConnection: close\r\n\r\n");
        let provider: String = provider
            .chars()
            .filter(|value| !value.is_control())
            .take(80)
            .collect();
        let success_message: String = success_message.chars().take(512).collect();
        let config = script_json(
            serde_json::json!({"provider": provider, "successMessage": success_message}),
        );
        let body = include_str!("oauth-callback.html")
            .replace(
                "{{APPEARANCE}}",
                include_str!("../../../../design-system/appearance.css"),
            )
            .replace(
                "{{TYPOGRAPHY}}",
                include_str!("../../../../design-system/typography.css"),
            )
            .replace(
                "{{CONTROLS}}",
                include_str!("../../../../design-system/components.css"),
            )
            .replace("{{NONCE}}", &nonce)
            .replace("{{PROVIDER}}", &html_text(&provider))
            .replace("{{CONFIG}}", &config);
        chunk(&mut stream, &body);
        Self {
            stream: Some(stream),
            nonce,
        }
    }

    pub fn finish(&mut self, result: Result<(), &str>) {
        let Some(mut stream) = self.stream.take() else {
            return;
        };
        let message: String = result.err().unwrap_or_default().chars().take(512).collect();
        let payload = script_json(serde_json::json!({"ok": result.is_ok(), "message": message}));
        chunk(
            &mut stream,
            &format!(
                "<script nonce=\"{}\">finishConnection({payload});</script></body></html>",
                self.nonce
            ),
        );
        let _ = stream.write_all(b"0\r\n\r\n");
        let _ = stream.flush();
    }
}

fn script_json(value: serde_json::Value) -> String {
    value
        .to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}
fn html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

impl Drop for CallbackPage {
    fn drop(&mut self) {
        self.finish(Err("연결이 중단되었습니다. Nero에서 다시 연결해 주세요."));
    }
}

fn chunk(stream: &mut TcpStream, body: &str) {
    let _ = write!(stream, "{:x}\r\n{body}\r\n", body.len());
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    fn sockets() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let browser = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        (browser, listener.accept().unwrap().0)
    }

    fn response(result: Option<Result<(), &str>>) -> String {
        let (mut browser, server) = sockets();
        let mut page = CallbackPage::begin(server);
        if let Some(result) = result {
            page.finish(result);
        }
        drop(page);
        let mut bytes = String::new();
        browser.read_to_string(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn success_is_only_sent_after_completion_and_has_a_close_fallback() {
        let (mut browser, server) = sockets();
        browser
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut page = CallbackPage::begin(server);
        let mut initial = String::new();
        let _ = browser.read_to_string(&mut initial);
        assert!(initial.contains("Google Calendar 연결 중"));
        assert!(!initial.contains("finishConnection({"));
        page.finish(Ok(()));
        browser
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut completed = String::new();
        browser.read_to_string(&mut completed).unwrap();
        assert!(completed.contains("\"ok\":true"));
        assert!(initial.contains("if (result.ok) setTimeout(() => window.close()"));
        assert!(initial.contains("이 탭을 닫아도 됩니다"));
        assert!(completed.ends_with("0\r\n\r\n"));
    }

    #[test]
    fn errors_and_abandoned_connections_never_report_success_or_inject_markup() {
        let failed = response(Some(Err("</script><script>unsafe()</script>")));
        assert!(failed.contains("\"ok\":false"));
        assert!(!failed.contains("<script>unsafe()"));
        assert!(failed.contains("\\u003c/script\\u003e"));
        let abandoned = response(None);
        assert!(abandoned.contains("연결이 중단되었습니다"));
        assert!(abandoned.contains("\"ok\":false"));
        assert!(!abandoned.contains("\"ok\":true"));
    }

    #[test]
    fn callback_is_self_contained_uncached_and_does_not_keep_authorization_in_history() {
        let page = response(Some(Ok(())));
        assert!(page.contains("Cache-Control: no-store"));
        assert!(page.contains("Referrer-Policy: no-referrer"));
        assert!(page.contains("frame-ancestors 'none'"));
        assert!(page.contains("history.replaceState(null, '', location.pathname)"));
        assert!(!page.contains("{{NONCE}}"));
        assert!(!page.contains("<script src="));
        assert!(!page.contains("code="));
        assert!(!page.contains("client_secret"));
    }
    #[test]
    fn provider_and_success_message_cannot_inject_markup_or_script() {
        let (mut browser, server) = sockets();
        let mut page = CallbackPage::begin_for(
            server,
            "Claude</title><script>unsafe()</script>",
            "</script><script>unsafe()</script>\u{2028}",
        );
        page.finish(Ok(()));
        drop(page);
        let mut output = String::new();
        browser.read_to_string(&mut output).unwrap();
        assert!(output.contains("Claude&lt;/title&gt;&lt;script&gt;unsafe()&lt;/script&gt;"));
        assert!(output.contains("\\u003c/script\\u003e"));
        assert!(output.contains("\\u2028"));
        assert!(!output.contains("<script>unsafe()"));
        assert!(!output.contains("{{PROVIDER}}"));
        assert!(!output.contains("{{CONFIG}}"));
    }

    #[test]
    fn rejection_uses_the_same_page_and_finish_is_idempotent() {
        let (mut browser, server) = sockets();
        CallbackPage::reject(server, "승인되지 않았습니다.");
        let mut rejected = String::new();
        browser.read_to_string(&mut rejected).unwrap();
        assert!(rejected.starts_with("HTTP/1.1 400 Bad Request"));
        assert!(rejected.contains("\"ok\":false"));
        assert!(!rejected.contains("\"ok\":true"));
        let (mut browser, server) = sockets();
        let mut page = CallbackPage::begin_for(server, "ChatGPT", "Nero로 돌아가 주세요.");
        page.finish(Err("저장하지 못했습니다."));
        page.finish(Ok(()));
        drop(page);
        let mut output = String::new();
        browser.read_to_string(&mut output).unwrap();
        assert!(output.contains("\"ok\":false"));
        assert!(!output.contains("\"ok\":true"));
    }
}
