use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const MEETING_ID: &str = "11111111-1111-4111-8111-111111111111";
const PRIVATE_MARKER: &str = "SYNTHETIC_PRIVATE_BODY";
const FOLDER_ID: &str = "33333333-3333-4333-8333-333333333333";

fn temporary_root() -> tempfile::TempDir {
    tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()
}

fn fixture() -> tempfile::TempDir {
    let root = temporary_root();
    let meeting = root.path().join("meetings").join(MEETING_ID);
    fs::create_dir_all(&meeting).unwrap();
    fs::write(meeting.join("meeting.json"), json!({
        "schemaVersion":1,"captureId":null,
        "meeting":{
            "id":MEETING_ID,"title":"Synthetic meeting","notes":format!("{PRIVATE_MARKER}\nsecond line\nthird line"),
            "createdAt":1000,"updatedAt":2000,"recordedAt":1000,"status":"draft",
            "audio":null,"error":null,"transcriptId":null
        }
    }).to_string()).unwrap();
    root
}

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nero"));
    command.arg("--data-dir").arg(root);
    command
}

#[test]
fn help_version_and_command_errors_do_not_create_a_missing_root() {
    let temporary = temporary_root();
    let missing = temporary.path().join("does-not-exist");
    for flag in ["--help", "--version"] {
        let output = command(&missing).arg(flag).output().unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert!(!output.stdout.is_empty());
    }
    let output = command(&missing)
        .args(["meetings", "delete", MEETING_ID])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let output = command(&missing)
        .args(["--json", "doctor"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["rootExists"],
        false
    );
    assert!(!missing.exists());
}

#[test]
fn json_reads_and_filters_share_the_read_only_dispatcher() {
    let root = fixture();
    let before = fs::read(
        root.path()
            .join("meetings")
            .join(MEETING_ID)
            .join("meeting.json"),
    )
    .unwrap();
    let output = command(root.path())
        .args([
            "--json", "meetings", "get", MEETING_ID, "--offset", "1", "--limit", "1",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["meeting"]["notes"], "second line\n");
    assert_eq!(result["pagination"]["hasMore"], true);
    assert_eq!(result["pagination"]["unit"], "textChunks");
    assert_eq!(result["apiVersion"], 1);
    for arguments in [
        vec![
            "--json", "meetings", "list", "--from", "2000", "--to", "1000",
        ],
        vec!["--json", "meetings", "list", "--limit", "201"],
        vec!["--json", "meetings", "get", "../escape"],
        vec!["--json", "summary", "generate", MEETING_ID],
    ] {
        let output = command(root.path()).args(arguments).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert!(error["error"]["code"].is_string());
        assert!(!String::from_utf8_lossy(&output.stderr).contains(PRIVATE_MARKER));
    }
    assert_eq!(
        before,
        fs::read(
            root.path()
                .join("meetings")
                .join(MEETING_ID)
                .join("meeting.json")
        )
        .unwrap()
    );
    assert!(!root.path().join("store.lock").exists());
    assert!(!root.path().join("library.json").exists());
}

#[test]
fn stored_document_errors_are_actionable_and_do_not_leak_content_or_paths() {
    let root = fixture();
    let path = root
        .path()
        .join("meetings")
        .join(MEETING_ID)
        .join("meeting.json");
    let original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut unsupported = original.clone();
    unsupported["schemaVersion"] = json!(99);
    let mut missing_schema = original.clone();
    missing_schema
        .as_object_mut()
        .unwrap()
        .remove("schemaVersion");
    let mut invalid_field = original.clone();
    invalid_field["meeting"]["title"] = json!(42);
    for (content, code) in [
        (
            format!("{{broken {PRIVATE_MARKER}").into_bytes(),
            "invalid_document",
        ),
        (
            serde_json::to_vec(&unsupported).unwrap(),
            "unsupported_format",
        ),
        (
            serde_json::to_vec(&missing_schema).unwrap(),
            "invalid_document",
        ),
        (
            serde_json::to_vec(&invalid_field).unwrap(),
            "invalid_document",
        ),
    ] {
        fs::write(&path, &content).unwrap();
        let output = command(root.path())
            .args(["--json", "meetings", "get", MEETING_ID])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let error: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["error"]["code"], code);
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        assert!(!diagnostics.contains(PRIVATE_MARKER));
        assert!(!diagnostics.contains(&root.path().to_string_lossy().to_string()));
        assert_eq!(fs::read(&path).unwrap(), content);
    }
    fs::File::create(&path)
        .unwrap()
        .set_len(16 * 1024 * 1024 + 1)
        .unwrap();
    let output = command(root.path())
        .args(["--json", "meetings", "get", MEETING_ID])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "document_too_large");
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("16MiB"));
}

#[test]
fn response_limits_explain_how_to_read_a_smaller_page() {
    let root = fixture();
    let path = root
        .path()
        .join("meetings")
        .join(MEETING_ID)
        .join("meeting.json");
    let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    document["meeting"]["notes"] = json!("\u{0001}".repeat(1024 * 1024 / 6 + 1));
    let before = serde_json::to_vec(&document).unwrap();
    fs::write(&path, &before).unwrap();
    let output = command(root.path())
        .args(["--json", "meetings", "get", MEETING_ID])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "response_too_large");
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("limit"));
    let output = command(root.path())
        .args(["--json", "meetings", "get", MEETING_ID, "--limit", "1"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["pagination"]["unit"], "textChunks");
    assert_eq!(result["pagination"]["hasMore"], true);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn explicit_linked_roots_are_rejected_without_following_them() {
    let root = fixture();
    let aliases = temporary_root();
    let linked = aliases.path().join("linked-root");
    std::os::unix::fs::symlink(root.path(), &linked).unwrap();
    for arguments in [
        vec!["--json", "doctor"],
        vec!["--json", "meetings", "get", MEETING_ID],
    ] {
        let output = command(&linked).args(arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let error: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["error"]["code"], "unsafe_path");
        assert!(!String::from_utf8_lossy(&output.stderr).contains(PRIVATE_MARKER));
    }
}

struct McpProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
}

impl McpProcess {
    fn start(root: &Path) -> Self {
        let mut child = command(root)
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin: Some(stdin),
            lines,
        }
    }

    fn send(&mut self, value: Value) {
        writeln!(self.stdin.as_mut().unwrap(), "{value}").unwrap();
    }

    fn result(&self, id: u64) -> Value {
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(10))
                .expect("MCP response timed out");
            let result: Value =
                serde_json::from_str(&line).expect("stdout must contain only MCP JSON lines");
            if result["id"] == id {
                return result;
            }
            assert!(
                result.get("method").is_some(),
                "unexpected response: {result}"
            );
        }
    }

    fn initialize(&mut self) -> Value {
        let message = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"synthetic-test","version":"1"}
        }}).to_string();
        let (first, last) = message.split_at(message.len() / 2);
        self.stdin
            .as_mut()
            .unwrap()
            .write_all(first.as_bytes())
            .unwrap();
        self.stdin
            .as_mut()
            .unwrap()
            .write_all(last.as_bytes())
            .unwrap();
        self.stdin.as_mut().unwrap().write_all(b"\r\n").unwrap();
        let initialized = self.result(1);
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        initialized
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tool_data(response: &Value) -> Value {
    let result = &response["result"];
    assert!(result.get("structuredContent").is_none());
    let content = result["content"].as_array().unwrap();
    assert_eq!(content.len(), 1, "one JSON body, without duplicate content");
    assert_eq!(content[0]["type"], "text");
    serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn cli_and_mcp_observe_folder_moves_and_renames_on_each_read() {
    let root = fixture();
    let library_path = root.path().join("library.json");
    let mut library = json!({"schemaVersion":1,"folders":{},"placements":{},"pendingPurges":[]});
    library["folders"][FOLDER_ID] = json!({"id":FOLDER_ID,"name":"Planning"});
    library["placements"][MEETING_ID] = json!({"folderId":FOLDER_ID,"deletedAt":null});
    fs::write(&library_path, library.to_string()).unwrap();
    let output = command(root.path())
        .args(["--json", "folders", "list"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let cli_folders: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        cli_folders["folders"][0],
        json!({"id":FOLDER_ID,"name":"Planning"})
    );
    let mut process = McpProcess::start(root.path());
    process.initialize();
    process.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"meetings_list","arguments":{"folderId":FOLDER_ID}}}));
    assert_eq!(
        tool_data(&process.result(2))["meetings"][0]["folderId"],
        FOLDER_ID
    );
    library["folders"][FOLDER_ID]["name"] = json!("Renamed planning");
    library["placements"][MEETING_ID]["folderId"] = Value::Null;
    fs::write(&library_path, library.to_string()).unwrap();
    let before = fs::read(&library_path).unwrap();
    process.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"folders_list","arguments":{}}}));
    let renamed = tool_data(&process.result(3));
    assert_eq!(renamed["folders"][0]["name"], "Renamed planning");
    process.send(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"meetings_list","arguments":{"unfiled":true}}}));
    let unfiled = tool_data(&process.result(4));
    assert_eq!(unfiled["pagination"]["total"], 1);
    assert!(unfiled["meetings"][0]["folderId"].is_null());
    let output = command(root.path())
        .args(["--json", "meetings", "list", "--folder-id", FOLDER_ID])
        .output()
        .unwrap();
    assert!(output.status.success());
    let filtered: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(filtered["pagination"]["total"], 0);
    assert_eq!(before, fs::read(&library_path).unwrap());

    let added_id = "44444444-4444-4444-8444-444444444444";
    library["folders"][added_id] = json!({"id":added_id,"name":"Added planning"});
    fs::write(&library_path, library.to_string()).unwrap();
    let after_add = fs::read(&library_path).unwrap();
    process.send(json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"folders_list","arguments":{}}}));
    let folders = tool_data(&process.result(5));
    assert_eq!(folders["pagination"]["total"], 2);
    assert!(folders["folders"]
        .as_array()
        .unwrap()
        .iter()
        .any(|folder| { folder["id"] == added_id && folder["name"] == "Added planning" }));
    assert_ne!(folders["sourceVersion"], renamed["sourceVersion"]);
    let output = command(root.path())
        .args(["--json", "folders", "list"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let cli_added: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(cli_added["folders"], folders["folders"]);
    assert_eq!(cli_added["sourceVersion"], folders["sourceVersion"]);
    assert_eq!(after_add, fs::read(&library_path).unwrap());
    assert!(!root.path().join("store.lock").exists());
}

#[test]
fn stdio_initialization_tools_errors_and_framing_use_the_sdk() {
    let root = fixture();
    let mut process = McpProcess::start(root.path());
    let initialized = process.initialize();
    assert_eq!(initialized["result"]["serverInfo"]["name"], "nero");
    assert!(initialized["result"]["capabilities"]["tools"].is_object());
    assert!(initialized["result"]["capabilities"]
        .get("resources")
        .is_none());

    process.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}));
    let listed = process.result(2);
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 7);
    for tool in tools {
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["annotations"]["destructiveHint"], false);
        assert_eq!(tool["annotations"]["openWorldHint"], false);
    }

    process.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"meeting_get","arguments":{"meetingId":MEETING_ID,"offset":1,"limit":1}}}));
    let called = process.result(3);
    assert_eq!(tool_data(&called)["meeting"]["notes"], "second line\n");
    assert_eq!(called["result"]["isError"], false);

    process.send(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"meetings_list","arguments":{"limit":201}}}));
    let invalid = process.result(4);
    assert!(invalid["result"]["isError"] == true || invalid.get("error").is_some());

    process.send(json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"meeting_delete","arguments":{"meetingId":MEETING_ID}}}));
    assert_eq!(process.result(5)["error"]["code"], -32601);
    process.send(
        json!({"jsonrpc":"2.0","id":6,"method":"meeting/update","params":{"text":PRIVATE_MARKER}}),
    );
    assert_eq!(process.result(6)["error"]["code"], -32601);
    process.send(json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"meeting_get","arguments":{"meetingId":"22222222-2222-4222-8222-222222222222"}}}));
    let missing = process.result(7);
    assert_eq!(missing["result"]["isError"], true);
    assert_eq!(tool_data(&missing)["error"]["code"], "meeting_not_found");

    fs::write(
        root.path()
            .join("meetings")
            .join(MEETING_ID)
            .join("meeting.json"),
        format!("{{broken {PRIVATE_MARKER}"),
    )
    .unwrap();
    process.send(json!({"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"meeting_get","arguments":{"meetingId":MEETING_ID}}}));
    let damaged = process.result(8);
    assert_eq!(damaged["result"]["isError"], true);
    assert_eq!(tool_data(&damaged)["error"]["code"], "invalid_document");
    assert!(!damaged.to_string().contains(PRIVATE_MARKER));
    assert!(!damaged
        .to_string()
        .contains(&root.path().to_string_lossy().to_string()));

    process.stdin.take();
    let _ = process.child.kill();
    let _ = process.child.wait();
    let mut errors = String::new();
    process
        .child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut errors)
        .unwrap();
    assert!(
        errors.is_empty(),
        "MCP diagnostics must not contain source content"
    );
    assert!(!root.path().join("store.lock").exists());
}
