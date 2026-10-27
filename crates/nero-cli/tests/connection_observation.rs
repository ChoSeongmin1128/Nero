use fs2::FileExt;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

struct Session {
    child: Child,
    input: ChildStdin,
    lines: Receiver<String>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Session {
    fn launch(root: &Path, directory: &Path, client_name: &str, token: &str, modern: bool) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_nero"))
            .arg("--data-dir")
            .arg(root)
            .arg("mcp")
            .env("NERO_CONNECTION_DIR", directory)
            .env("NERO_CONNECTION_CLIENT", "codex")
            .env("NERO_CONNECTION_TOKEN", token)
            .env("NERO_CONNECTION_GENERATION", "a".repeat(64))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (send, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        let mut session = Self {
            child,
            input,
            lines,
        };
        if modern {
            let metadata = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":client_name,"version":"test"},"io.modelcontextprotocol/clientCapabilities":{}});
            session.send(json!({"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":metadata}}));
            let discovered = session.response(1);
            assert!(
                discovered["result"]["_meta"]["io.modelcontextprotocol/serverInfo"].is_object()
            );
            assert!(discovered["result"]["supportedVersions"]
                .as_array()
                .unwrap()
                .contains(&json!("2026-07-28")));
            assert_eq!(fs::read_dir(directory).unwrap().count(), 0);
            session.send(
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta":metadata}}),
            );
            assert_eq!(
                session.response(2)["result"]["tools"]
                    .as_array()
                    .unwrap()
                    .len(),
                7
            );
            return session;
        }
        session.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":client_name,"version":"test"}}}));
        assert!(session.response(1)["result"]["serverInfo"].is_object());
        session.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        session.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
        assert_eq!(
            session.response(2)["result"]["tools"]
                .as_array()
                .unwrap()
                .len(),
            7
        );
        session
    }
    fn send(&mut self, value: Value) {
        writeln!(self.input, "{value}").unwrap();
        self.input.flush().unwrap();
    }
    fn response(&self, id: u32) -> Value {
        for _ in 0..10 {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(5))
                .expect("bounded MCP response");
            let value: Value = serde_json::from_str(&line).unwrap();
            if value["id"] == id {
                return value;
            }
        }
        panic!("MCP response missing")
    }
}

fn setup() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    &'static str,
) {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("connections/codex/sessions");
    fs::create_dir_all(&directory).unwrap();
    let root = temp.path().join("missing meeting library");
    let token = "11111111-1111-4111-8111-111111111111";
    fs::write(
        directory.parent().unwrap().join("state.json"),
        json!({"schemaVersion":1,"client":"codex","token":token,"generation":"a".repeat(64),"dataDir":root}).to_string(),
    )
    .unwrap();
    (temp, directory, root, token)
}

#[test]
fn initialized_managed_session_holds_a_receipt_lock_until_disconnect_without_opening_library() {
    let (_temp, directory, root, token) = setup();
    let session = Session::launch(&root, &directory, "codex-mcp-client", token, false);
    let path = fs::read_dir(&directory)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap()
        .path();
    let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["clientName"], "codex-mcp-client");
    let receipt = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))
        .unwrap();
    assert!(receipt.try_lock_exclusive().is_err());
    assert!(!root.exists());
    drop(session);
    assert!(receipt.try_lock_exclusive().is_ok());
    assert!(!root.exists());
}

#[test]
fn self_probe_and_invalid_registration_cannot_claim_a_connected_client() {
    let (_temp, directory, root, token) = setup();
    for (name, provided_token) in [
        ("nero-self-check", token),
        ("codex-mcp-client", "22222222-2222-4222-8222-222222222222"),
    ] {
        let _session = Session::launch(&root, &directory, name, provided_token, false);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
        assert!(!root.exists());
    }
}

#[test]
fn modern_discovery_records_only_the_following_valid_inline_tool_request() {
    let (_temp, directory, root, token) = setup();
    let _session = Session::launch(&root, &directory, "codex-mcp-client", token, true);
    let path = fs::read_dir(&directory)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap()
        .path();
    let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(value["clientName"], "codex-mcp-client");
    assert_eq!(value["generation"], "a".repeat(64));
    assert!(!root.exists());
}
