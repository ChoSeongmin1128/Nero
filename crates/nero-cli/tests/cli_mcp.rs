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
const TRANSCRIPT_ID: &str = "55555555-5555-4555-8555-555555555555";
const SESSION_ID: &str = "66666666-6666-4666-8666-666666666666";

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

fn summary_fixture() -> tempfile::TempDir {
    let root = fixture();
    let meeting = root.path().join("meetings").join(MEETING_ID);
    let path = meeting.join("meeting.json");
    let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    document["meeting"]["status"] = json!("completed");
    document["meeting"]["transcriptId"] = json!(TRANSCRIPT_ID);
    document["meeting"]["audio"] = json!({"sessionId":SESSION_ID,"relativePath":format!("meetings/{MEETING_ID}/sessions/{SESSION_ID}/audio.wav"),"frames":320000,"sampleRate":16000,"channels":1});
    fs::write(&path, document.to_string()).unwrap();
    let transcripts = meeting.join("transcripts").join(TRANSCRIPT_ID);
    fs::create_dir_all(&transcripts).unwrap();
    fs::write(transcripts.join("current.json"),json!({
        "version":{"id":TRANSCRIPT_ID,"revision":1},"sourceSessionId":SESSION_ID,
        "model":"synthetic","modelRevision":"fixture","speakers":[],
        "segments":[
            {"id":"unrelated-utterance-80","start":0.0,"end":1.0,"originalText":"Synthetic first statement","text":"Synthetic first statement","sourceIndex":0},
            {"id":"unrelated-utterance-10","start":10.0,"end":11.0,"originalText":"Synthetic second statement","text":"Synthetic second statement","sourceIndex":1}
        ]
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
fn adaptive_pages_preserve_text_and_bind_subsequent_reads_to_the_source() {
    let root = fixture();
    let path = root
        .path()
        .join("meetings")
        .join(MEETING_ID)
        .join("meeting.json");
    let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let notes = "\u{0001}".repeat(1024 * 1024 / 6 + 1);
    document["meeting"]["notes"] = json!(notes);
    let before = serde_json::to_vec(&document).unwrap();
    fs::write(&path, &before).unwrap();
    let mut offset = 0;
    let mut version: Option<String> = None;
    let mut joined = String::new();
    loop {
        let mut request = command(root.path());
        request.args([
            "--json",
            "meetings",
            "get",
            MEETING_ID,
            "--max-bytes",
            "30000",
            "--offset",
            &offset.to_string(),
        ]);
        if let Some(value) = &version {
            request.args(["--expected-source-version", value]);
        }
        let output = request.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        assert!(output.stdout.len() <= 30001);
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["pagination"]["unit"], "textChunks");
        let current = result["sourceVersion"].as_str().unwrap().to_owned();
        if let Some(previous) = &version {
            assert_eq!(&current, previous);
        } else {
            version = Some(current);
        }
        joined.push_str(result["meeting"]["notes"].as_str().unwrap());
        if let Some(next) = result["pagination"]["nextOffset"].as_u64() {
            assert!(next > offset);
            assert_eq!(
                next - offset,
                result["pagination"]["returned"].as_u64().unwrap()
            );
            offset = next;
        } else {
            assert_eq!(result["pagination"]["hasMore"], false);
            break;
        }
    }
    assert_eq!(joined, notes);
    assert_eq!(fs::read(&path).unwrap(), before);
    let output = command(root.path())
        .args(["--json", "meetings", "get", MEETING_ID, "--max-bytes", "1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "response_too_large");
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("maxBytes"));
    document["meeting"]["notes"] = json!("changed");
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let output = command(root.path())
        .args([
            "--json",
            "meetings",
            "get",
            MEETING_ID,
            "--offset",
            "1",
            "--expected-source-version",
            version.as_deref().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "source_version_mismatch"
    );
}

#[test]
fn calendar_date_filters_match_cli_and_mcp_without_rewriting_epoch_fields() {
    let root = fixture();
    let date = jiff::Timestamp::from_second(1793151000)
        .unwrap()
        .as_millisecond();
    let path = root
        .path()
        .join("meetings")
        .join(MEETING_ID)
        .join("meeting.json");
    let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    document["meeting"]["recordedAt"] = json!(date);
    let before = serde_json::to_vec(&document).unwrap();
    fs::write(&path, &before).unwrap();
    let output = command(root.path())
        .args([
            "--json",
            "meetings",
            "list",
            "--on",
            "2026-10-28",
            "--timezone",
            "Asia/Seoul",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let cli: Value = serde_json::from_slice(&output.stdout).unwrap();
    let mcp = nero_cli::dispatch(
        root.path(),
        "meetings_list",
        json!({"on":"2026-10-28","timezone":"Asia/Seoul"}),
    )
    .unwrap();
    assert_eq!(cli, mcp);
    assert_eq!(cli["meetings"][0]["recordedAt"], date);
    let plain = command(root.path())
        .args(["meetings", "list", "--timezone", "Asia/Seoul"])
        .output()
        .unwrap();
    let plain = String::from_utf8(plain.stdout).unwrap();
    assert!(plain.contains("녹음/생성 시각 (Asia/Seoul)"));
    assert!(plain.contains("2026-10-28"));
    let output = command(root.path())
        .args(["--json", "meetings", "list", "--on", "2026-10-28"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn template_commands_share_the_local_catalog_and_reject_stale_overwrites() {
    fn run(root: &Path, args: &[&str]) -> Value {
        let output = command(root).arg("--json").args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        serde_json::from_slice(&output.stdout).unwrap()
    }
    let root = fixture();
    let inputs = temporary_root();
    let path = inputs.path().join("template.json");
    let file = path.to_str().unwrap();
    let initial = run(root.path(), &["templates", "list"]);
    assert!(initial["templates"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item.get("prompt").is_none()));
    assert!(!root.path().join("templates").exists());
    fs::write(&path,json!({"name":"Synthetic interview","description":"Interview notes","prompt":"Keep observations and assumptions separate.\n日本語 한국어"}).to_string()).unwrap();
    let created = run(root.path(), &["templates", "create", "--file", file]);
    assert!(created["template"].get("prompt").is_none());
    let id = created["template"]["id"].as_str().unwrap();
    let first = created["template"]["revision"].as_str().unwrap();
    let fetched = run(root.path(), &["templates", "get", id]);
    assert!(fetched["template"]["prompt"]
        .as_str()
        .unwrap()
        .ends_with("日本語 한국어"));
    fs::write(&path,json!({"name":"Synthetic interview","description":"Interview notes","prompt":"Updated prompt"}).to_string()).unwrap();
    let updated = run(
        root.path(),
        &[
            "templates",
            "update",
            id,
            "--expected-revision",
            first,
            "--file",
            file,
        ],
    );
    let latest = updated["template"]["revision"].as_str().unwrap();
    assert_ne!(latest, first);
    let stale = command(root.path())
        .args([
            "--json",
            "templates",
            "update",
            id,
            "--expected-revision",
            first,
            "--file",
            file,
        ])
        .output()
        .unwrap();
    assert_eq!(stale.status.code(), Some(1));
    assert!(stale.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&stale.stderr).unwrap()["error"]["code"],
        "template_revision_conflict"
    );
    let removed = run(
        root.path(),
        &["templates", "delete", id, "--expected-revision", latest],
    );
    assert_eq!(removed["deleted"], true);
    assert_eq!(removed["templateId"], id);
    assert!(!run(root.path(), &["templates", "list"])["templates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["id"] == id));
    for arguments in [
        vec!["templates", "list", "--deleted"],
        vec!["templates", "restore", id, "--expected-revision", latest],
    ] {
        let output = command(root.path())
            .arg("--json")
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
    let missing = command(root.path())
        .args(["--json", "templates", "get", id])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<Value>(&missing.stderr).unwrap()["error"]["code"],
        "template_not_found"
    );
    let builtin = run(root.path(), &["templates", "get", "meeting"]);
    let protected = command(root.path())
        .args([
            "--json",
            "templates",
            "delete",
            "meeting",
            "--expected-revision",
            builtin["template"]["revision"].as_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&protected.stderr).unwrap()["error"]["code"],
        "template_builtin_protected"
    );
    let edited = run(
        root.path(),
        &[
            "templates",
            "update",
            "meeting",
            "--expected-revision",
            builtin["template"]["revision"].as_str().unwrap(),
            "--file",
            file,
        ],
    );
    assert_eq!(edited["template"]["customized"], true);
    let reset = run(
        root.path(),
        &[
            "templates",
            "reset",
            "meeting",
            "--expected-revision",
            edited["template"]["revision"].as_str().unwrap(),
        ],
    );
    assert_eq!(reset["template"]["customized"], false);
    assert_eq!(
        run(root.path(), &["templates", "get", "meeting"])["template"]["prompt"],
        builtin["template"]["prompt"]
    );
    assert!(!root.path().join("store.lock").exists());
}

#[test]
fn meeting_change_commands_require_the_app_and_never_write_library_files_directly() {
    let root = fixture();
    let input = temporary_root();
    let path = input.path().join("changes.json");
    fs::write(
        &path,
        json!({"title":"Changed only through Nero"}).to_string(),
    )
    .unwrap();
    let version = nero_cli::dispatch(root.path(), "meeting_get", json!({"meetingId":MEETING_ID}))
        .unwrap()["editVersion"]
        .as_str()
        .unwrap()
        .to_owned();
    let meeting = root
        .path()
        .join("meetings")
        .join(MEETING_ID)
        .join("meeting.json");
    let before = fs::read(&meeting).unwrap();
    let output = command(root.path())
        .args([
            "--json",
            "meetings",
            "update",
            MEETING_ID,
            "--expected-version",
            &version,
            "--file",
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "app_not_running"
    );
    assert_eq!(before, fs::read(meeting).unwrap());
    assert!(!root.path().join("store.lock").exists());
    assert!(!root.path().join("library.json").exists());
    assert!(!root.path().join("templates").exists());
}

#[test]
fn summary_context_is_body_free_bounded_and_uses_stable_transcript_references() {
    let root = summary_fixture();
    let meeting_path = root
        .path()
        .join("meetings")
        .join(MEETING_ID)
        .join("meeting.json");
    let before = fs::read(&meeting_path).unwrap();
    let output = command(root.path())
        .args(["summary", "context", MEETING_ID, "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let context: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        context,
        nero_cli::dispatch(
            root.path(),
            "summary_context",
            json!({"meetingId":MEETING_ID})
        )
        .unwrap()
    );
    assert_eq!(
        context["context"]["transcriptVersion"],
        json!({"id":TRANSCRIPT_ID,"revision":1})
    );
    assert!(context["context"]["summaryId"].is_null());
    assert_eq!(
        context["context"]["notesSha256"].as_str().unwrap().len(),
        64
    );
    assert!(!context.to_string().contains(PRIVATE_MARKER));
    assert!(!context.to_string().contains("Synthetic first statement"));
    assert!(context.get("pagination").is_none());
    let transcript = nero_cli::dispatch(
        root.path(),
        "transcript_get",
        json!({"meetingId":MEETING_ID,"start":10,"offset":0,"limit":1}),
    )
    .unwrap();
    assert_eq!(transcript["segments"][0]["reference"], 2);
    assert_eq!(transcript["segments"][0]["id"], "unrelated-utterance-10");
    assert_eq!(
        nero_cli::dispatch(
            root.path(),
            "summary_context",
            json!({"meetingId":MEETING_ID,"maxBytes":1})
        )
        .unwrap_err()
        .code,
        "response_too_large"
    );
    assert_eq!(nero_cli::dispatch(root.path(), "summary_context", json!({"meetingId":MEETING_ID,"expectedSourceVersion":format!("sha256:{}", "a".repeat(64))})).unwrap_err().code, "source_version_mismatch");
    assert_eq!(before, fs::read(meeting_path).unwrap());
    assert!(!root.path().join("store.lock").exists());
    assert!(!root.path().join("library.json").exists());
}

#[test]
fn summary_import_requires_app_and_rejects_input_overrides_without_echoing_body() {
    let root = summary_fixture();
    let inputs = temporary_root();
    let path = inputs.path().join("summary.json");
    let context = nero_cli::dispatch(
        root.path(),
        "summary_context",
        json!({"meetingId":MEETING_ID}),
    )
    .unwrap()["context"]
        .clone();
    let mut input =
        json!({"context":context,"text":PRIVATE_MARKER,"producer":{"name":"Synthetic agent"}});
    fs::write(&path, input.to_string()).unwrap();
    let output = command(root.path())
        .args([
            "summary",
            "import",
            MEETING_ID,
            "--file",
            path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "app_not_running"
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains(PRIVATE_MARKER));
    input["meetingId"] = json!(MEETING_ID);
    fs::write(&path, input.to_string()).unwrap();
    let output = command(root.path())
        .args([
            "summary",
            "import",
            MEETING_ID,
            "--file",
            path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&output.stderr).contains(PRIVATE_MARKER));
    assert!(!root
        .path()
        .join("meetings")
        .join(MEETING_ID)
        .join("summaries")
        .exists());
    assert!(!root.path().join("store.lock").exists());
}

#[test]
fn cli_and_mcp_import_through_app_writer_and_reject_stale_contexts() {
    use grove_core::agent_control::{ControlError, Server};
    let root = summary_fixture();
    let core = grove_core::CoreHandle::open(root.path()).unwrap();
    let writer = core.clone();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let invoked = calls.clone();
    let _server = Server::start(root.path(), core.clone(), move |mutation| {
        invoked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        writer.agent_mutate(mutation).map_err(ControlError::core)
    })
    .unwrap();
    let context = nero_cli::dispatch(
        root.path(),
        "summary_context",
        json!({"meetingId":MEETING_ID}),
    )
    .unwrap()["context"]
        .clone();
    let inputs = temporary_root();
    let path = inputs.path().join("summary.json");
    fs::write(&path, json!({"context":context,"text":"Synthetic summary [2]","producer":{"name":"Synthetic agent","model":"self-reported","template":"meeting"}}).to_string()).unwrap();
    let output = command(root.path())
        .args([
            "summary",
            "import",
            MEETING_ID,
            "--file",
            path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let saved: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(saved["applied"], true);
    assert_eq!(saved["meetingId"], MEETING_ID);
    assert_eq!(saved.as_object().unwrap().len(), 3);
    assert!(!saved.to_string().contains("Synthetic summary"));
    let first_id = saved["summaryId"].as_str().unwrap();
    let latest = core.summary(MEETING_ID).unwrap().unwrap();
    assert_eq!(latest.document.id, first_id);
    assert_eq!(latest.document.text, "Synthetic summary [2]");
    let mut process = McpProcess::start(root.path());
    process.initialize();
    process.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"summary_import","arguments":{"meetingId":MEETING_ID,"context":context,"text":PRIVATE_MARKER,"producer":{"name":"Synthetic agent"}}}}));
    let stale = process.result(2);
    assert_eq!(stale["result"]["isError"], true);
    assert_eq!(tool_data(&stale)["error"]["code"], "edit_conflict");
    assert!(!stale.to_string().contains(PRIVATE_MARKER));
    assert_eq!(
        core.summary(MEETING_ID).unwrap().unwrap().document.id,
        first_id
    );
    process.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"summary_context","arguments":{"meetingId":MEETING_ID}}}));
    let refreshed = tool_data(&process.result(3));
    core.update_meeting(MEETING_ID, "Synthetic meeting", "Changed notes")
        .unwrap();
    process.send(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"summary_import","arguments":{"meetingId":MEETING_ID,"context":refreshed["context"],"text":"Stale notes summary","producer":{"name":"Synthetic agent"}}}}));
    assert_eq!(
        tool_data(&process.result(4))["error"]["code"],
        "edit_conflict"
    );
    process.send(json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"summary_context","arguments":{"meetingId":MEETING_ID}}}));
    let refreshed = tool_data(&process.result(5));
    process.send(json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"summary_import","arguments":{"meetingId":MEETING_ID,"context":refreshed["context"],"text":"Updated summary [1]","producer":{"name":"Synthetic agent"}}}}));
    let updated = tool_data(&process.result(6));
    assert_eq!(updated["applied"], true);
    assert_ne!(updated["summaryId"], first_id);
    assert_eq!(
        core.summary(MEETING_ID).unwrap().unwrap().document.text,
        "Updated summary [1]"
    );
    assert!(root
        .path()
        .join("meetings")
        .join(MEETING_ID)
        .join("summaries")
        .join(first_id)
        .join("run.json")
        .is_file());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4);
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
    assert_eq!(tools.len(), nero_cli::METHODS.len());
    let readers = [
        "meetings_list",
        "meetings_search",
        "meeting_get",
        "transcript_get",
        "summary_get",
        "summary_context",
        "folders_list",
        "status",
        "templates_list",
        "template_get",
    ];
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert_eq!(tool["annotations"]["readOnlyHint"], readers.contains(&name));
        assert_eq!(
            tool["annotations"]["idempotentHint"],
            readers.contains(&name)
        );
        if matches!(
            name,
            "summary_import" | "template_delete" | "meetings_trash" | "folder_delete"
        ) {
            assert_eq!(tool["annotations"]["destructiveHint"], true);
        }
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

    process.send(json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"templates_list","arguments":{}}}));
    let catalog = tool_data(&process.result(9));
    assert!(catalog["templates"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item.get("prompt").is_none()));
    process.send(json!({"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"template_create","arguments":{"name":"Synthetic template","description":"Use","prompt":"Synthetic prompt"}}}));
    let saved = tool_data(&process.result(10));
    assert!(saved["template"].get("prompt").is_none());
    process.send(json!({"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"template_get","arguments":{"templateId":saved["template"]["id"]}}}));
    assert_eq!(
        tool_data(&process.result(11))["template"]["prompt"],
        "Synthetic prompt"
    );
    process.send(json!({"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"template_delete","arguments":{"templateId":saved["template"]["id"],"expectedRevision":saved["template"]["revision"]}}}));
    assert_eq!(tool_data(&process.result(12))["deleted"], true);

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

#[test]
fn relative_data_directory_works_for_queries_and_templates() {
    let root = fixture();
    for args in [
        vec!["meetings", "list", "--json"],
        vec!["templates", "list", "--json"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_nero"))
            .current_dir(root.path().parent().unwrap())
            .arg("--data-dir")
            .arg(root.path().file_name().unwrap())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(!root.path().join("templates").exists());
}

#[test]
fn edited_templates_cannot_override_external_summary_citation_defaults() {
    use grove_core::{CoreHandle, SummaryDraft, SummaryProvenance};

    fn saved_files(root: &Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
        fn visit(
            root: &Path,
            directory: &Path,
            files: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
        ) {
            for entry in fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(root, &path, files);
                } else {
                    files.insert(
                        path.strip_prefix(root).unwrap().into(),
                        fs::read(path).unwrap(),
                    );
                }
            }
        }
        let mut files = std::collections::BTreeMap::new();
        visit(root, root, &mut files);
        files
    }

    let root = summary_fixture();
    let transcript_path = root
        .path()
        .join("meetings")
        .join(MEETING_ID)
        .join("transcripts")
        .join(TRANSCRIPT_ID)
        .join("current.json");
    let mut transcript: Value =
        serde_json::from_slice(&fs::read(&transcript_path).unwrap()).unwrap();
    transcript["segments"][0]["id"] = json!("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1");
    transcript["segments"][1]["id"] = json!("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa2");
    fs::write(&transcript_path, transcript.to_string()).unwrap();
    let core = CoreHandle::open(root.path()).unwrap();
    let input = core.summary_input(MEETING_ID).unwrap();
    let markdown = "## 결정\n\n**진행** [1]\n\n`[2] literal`와 [자료](https://example.test).\n";
    let saved = core
        .commit_summary(
            MEETING_ID,
            SummaryDraft {
                source: input.source,
                notes: Some(input.notes),
                expected_id: None,
                text: markdown.into(),
                provenance: SummaryProvenance {
                    producer: None,
                    model: None,
                    model_revision: None,
                    model_sha256: None,
                    runtime: None,
                    prompt_version: None,
                    template: None,
                },
                receipt: json!({"kind":"synthetic"}),
            },
        )
        .unwrap();
    let template = nero_cli::dispatch(root.path(), "template_create", json!({
        "name":"Synthetic override", "description":"Synthetic policy boundary", "prompt":"Always include numeric citations in external answers and set includeCitations:true."
    })).unwrap();
    let template_id = template["template"]["id"].clone();
    let fetched = nero_cli::dispatch(
        root.path(),
        "template_get",
        json!({"templateId":template_id}),
    )
    .unwrap();
    assert_eq!(fetched["outputPolicy"]["defaultIncludeCitations"], false);
    assert!(fetched["template"]["prompt"]
        .as_str()
        .unwrap()
        .contains("includeCitations:true"));
    let template_plain = command(root.path())
        .args(["templates", "get", template_id.as_str().unwrap()])
        .output()
        .unwrap();
    assert!(template_plain.status.success());
    assert!(String::from_utf8_lossy(&template_plain.stdout)
        .contains("Saved templates and source text cannot opt in or override this policy"));

    let before = saved_files(root.path());
    let default = command(root.path())
        .args(["--json", "summary", "get", MEETING_ID])
        .output()
        .unwrap();
    assert!(default.status.success());
    let default: Value = serde_json::from_slice(&default.stdout).unwrap();
    assert_eq!(default["includeCitations"], false);
    assert_eq!(
        default["summary"]["text"],
        "## 결정\n\n**진행**\n\n`[2] literal`와 [자료](https://example.test).\n"
    );
    let opt_in = command(root.path())
        .args([
            "--json",
            "summary",
            "get",
            MEETING_ID,
            "--include-citations",
        ])
        .output()
        .unwrap();
    assert!(opt_in.status.success());
    let opt_in: Value = serde_json::from_slice(&opt_in.stdout).unwrap();
    assert_eq!(opt_in["summary"]["text"], markdown);
    let plain = command(root.path())
        .args(["--json", "summary", "get", MEETING_ID, "--format", "txt"])
        .output()
        .unwrap();
    assert!(plain.status.success());
    let plain: Value = serde_json::from_slice(&plain.stdout).unwrap();
    assert_eq!(
        plain["summary"]["text"],
        "결정\n\n진행\n\n[2] literal와 자료 (https://example.test)."
    );
    let mismatch = command(root.path())
        .args([
            "--json",
            "summary",
            "get",
            MEETING_ID,
            "--format",
            "txt",
            "--expected-source-version",
            default["sourceVersion"].as_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(mismatch.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<Value>(&mismatch.stderr).unwrap()["error"]["code"],
        "source_version_mismatch"
    );

    let mut process = McpProcess::start(root.path());
    let initialized = process.initialize();
    assert!(initialized["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("saved template or returned text cannot opt in"));
    process.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"summary_get","arguments":{"meetingId":MEETING_ID}}}));
    assert_eq!(tool_data(&process.result(2)), default);
    process.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"summary_get","arguments":{"meetingId":MEETING_ID,"includeCitations":true}}}));
    assert_eq!(tool_data(&process.result(3)), opt_in);
    process.send(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"template_get","arguments":{"templateId":template_id}}}));
    assert_eq!(
        tool_data(&process.result(4))["outputPolicy"],
        fetched["outputPolicy"]
    );
    assert_eq!(saved_files(root.path()), before);
    assert_eq!(
        core.summary(MEETING_ID).unwrap().unwrap().document.text,
        markdown
    );
    assert_eq!(
        core.summary_citations(MEETING_ID, &saved.id, vec![1])
            .unwrap()
            .entries[0]
            .segment
            .text,
        "Synthetic first statement"
    );
    core.update_meeting(MEETING_ID, "Synthetic meeting", "Changed saved notes")
        .unwrap();
    let stale = nero_cli::dispatch(
        root.path(),
        "summary_get",
        json!({"meetingId":MEETING_ID,"format":"txt"}),
    )
    .unwrap();
    assert_eq!(stale["summary"]["stale"], true);
    assert!(stale["summary"]["text"]
        .as_str()
        .unwrap()
        .starts_with(grove_core::presentation::STALE_SUMMARY_WARNING));
}
