use std::io::Write;
use std::net::TcpStream;
use std::time::Duration;

pub struct CallbackPage {
    stream: Option<TcpStream>,
    nonce: String,
}

impl CallbackPage {
    pub fn begin(stream: TcpStream) -> Self {
        Self::start(stream, "200 OK")
    }

    pub fn reject(stream: TcpStream, message: &str) {
        Self::start(stream, "400 Bad Request").finish(Err(message));
    }

    fn start(mut stream: TcpStream, status: &str) -> Self {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
        let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nTransfer-Encoding: chunked\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; style-src 'nonce-{nonce}'; script-src 'nonce-{nonce}'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'\r\nConnection: close\r\n\r\n");
        let body = include_str!("callback-page.html")
            .replace(
                "{{APPEARANCE}}",
                include_str!("../../../../../design-system/appearance.css"),
            )
            .replace(
                "{{TYPOGRAPHY}}",
                include_str!("../../../../../design-system/typography.css"),
            )
            .replace(
                "{{CONTROLS}}",
                include_str!("../../../../../design-system/components.css"),
            )
            .replace("{{NONCE}}", &nonce);
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
        let payload = serde_json::json!({"ok": result.is_ok(), "message": message})
            .to_string()
            .replace('<', "\\u003c")
            .replace('>', "\\u003e")
            .replace('&', "\\u0026");
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
}
