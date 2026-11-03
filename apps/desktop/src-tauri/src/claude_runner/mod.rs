//! Claude Code subprocess transport. Credentials remain owned by Claude Code.
mod installation;
mod model_cache;
pub use model_cache::ModelCache;
mod prepare;
#[cfg(windows)]
mod windows_signature;
pub use installation::installation_state;
pub use prepare::{discover_prepared, prepare};
use serde_json::Value;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

const MAX_OUTPUT: usize = 4 * 1024 * 1024;
const MAX_INPUT: usize = 4 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(600);
const ERROR: &str = "Claude Code 실행을 완료하지 못했습니다.";

#[derive(Clone, Debug)]
pub struct Runner {
    binary: PathBuf,
}

/// Finds the native installer location, not a shell alias or a project-local binary.
pub fn discover() -> Result<Runner, String> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .ok_or("Claude Code 설치 위치를 찾지 못했습니다.")?;
    let path = PathBuf::from(home)
        .join(".local")
        .join("bin")
        .join(if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        });
    let binary = path
        .canonicalize()
        .map_err(|_| "Claude Code를 찾지 못했습니다. 공식 설치가 필요합니다.")?;
    validate(&binary)?;
    Ok(Runner { binary })
}

#[cfg(target_os = "macos")]
fn validate(binary: &Path) -> Result<(), String> {
    let result = Command::new("/usr/bin/codesign").args(["--verify", "--strict", "-R", "=anchor apple generic and identifier \"com.anthropic.claude-code\" and certificate leaf[subject.OU] = \"Q6L2SF6YDW\""]).arg(binary).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map_err(|_| ERROR)?;
    if !result.success() {
        return Err("Claude Code 실행 파일의 서명을 확인하지 못했습니다.".into());
    }
    Ok(())
}
#[cfg(windows)]
fn validate(binary: &Path) -> Result<(), String> {
    windows_signature::validate(binary)
}
#[cfg(not(any(target_os = "macos", windows)))]
fn validate(_binary: &Path) -> Result<(), String> {
    Err("이 운영체제에서는 Claude Code 실행 파일의 서명 검증을 아직 지원하지 않습니다.".into())
}

fn command(binary: &Path) -> Command {
    let mut command = Command::new(binary);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    // Keep HOME and OS facilities required for the official login, but never inherit
    // provider selection, credentials, SDK, or custom endpoint overrides.
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy().to_ascii_uppercase();
        if name.starts_with("ANTHROPIC_")
            || name.starts_with("CLAUDE_")
            || name.starts_with("CLAUDECODE")
            || name.starts_with("AWS_")
            || name.starts_with("GOOGLE_")
            || name.starts_with("VERTEX_")
            || name.starts_with("AZURE_")
            || name.starts_with("CLOUD_ML_")
            || name == "NODE_OPTIONS"
            || name == "NODE_PATH"
        {
            command.env_remove(key);
        }
    }
    command.env("DISABLE_AUTOUPDATER", "1");
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

// A bounded channel drains both pipes while the caller processes stdout.
fn run(
    mut command: Command,
    input: Vec<u8>,
    timeout: Duration,
    cancel: &AtomicBool,
    mut stdout_chunk: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    if input.len() > MAX_INPUT || cancel.load(Ordering::Acquire) {
        return Err("요청이 너무 크거나 취소되었습니다.".into());
    }
    let mut child = crate::process_tree::spawn(&mut command)?;
    let mut stdin = child.stdin.take().ok_or(ERROR)?;
    let (input_tx, input_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = stdin.write_all(&input);
        drop(stdin);
        let _ = input_tx.send(result);
    });
    let stdout = child.stdout.take().ok_or(ERROR)?;
    let stderr = child.stderr.take().ok_or(ERROR)?;
    let (tx, rx) = mpsc::sync_channel::<(bool, Vec<u8>)>(16);
    for (is_stdout, mut pipe) in [
        (true, Box::new(stdout) as Box<dyn Read + Send>),
        (false, Box::new(stderr) as Box<dyn Read + Send>),
    ] {
        let tx = tx.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send((is_stdout, buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                }
            }
        });
    }
    drop(tx);
    let deadline = Instant::now() + timeout;
    let mut total = 0usize;
    let mut exited = None;
    let mut disconnected = false;
    let mut input_done = false;
    loop {
        if cancel.load(Ordering::Acquire) || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if cancel.load(Ordering::Acquire) {
                "취소했습니다."
            } else {
                "Claude 응답 시간이 초과되었습니다."
            }
            .into());
        }
        match if disconnected {
            thread::sleep(Duration::from_millis(50));
            Err(mpsc::RecvTimeoutError::Timeout)
        } else {
            rx.recv_timeout(Duration::from_millis(50))
        } {
            Ok((is_stdout, chunk)) => {
                total += chunk.len();
                if total > MAX_OUTPUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("Claude 응답이 너무 큽니다.".into());
                }
                if is_stdout {
                    stdout_chunk(&chunk)?;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => disconnected = true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if exited.is_none() {
            exited = child.try_wait().map_err(|_| ERROR)?;
        }
        if !input_done {
            match input_rx.try_recv() {
                Ok(Ok(())) => input_done = true,
                Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => return Err(ERROR.into()),
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if disconnected {
            if let Some(status) = exited {
                if !input_done {
                    continue;
                }
                return if status.success() {
                    Ok(())
                } else {
                    Err(ERROR.into())
                };
            }
        }
    }
}

fn capture(
    command: Command,
    input: Vec<u8>,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    run(command, input, timeout, cancel, |chunk| {
        bytes.extend_from_slice(chunk);
        Ok(())
    })?;
    Ok(bytes)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn shell(script: &str) -> Command {
        let mut cmd = command(Path::new("/bin/sh"));
        cmd.args(["-c", script]);
        cmd
    }
    #[test]
    fn parses_only_matching_supported_model_metadata() {
        let output = br#"{"type":"control_response","response":{"subtype":"success","request_id":"models","response":{"account":{"email":"private"},"models":[{"value":"sonnet","displayName":"Sonnet","resolvedModel":"claude-sonnet-5-5"},{"value":"claude-opus-5-5","displayName":"Opus 5.5"},{"value":"--bad","displayName":"bad"}]}}}
"#;
        let models = parse_supported_models(output).unwrap();
        assert_eq!(models[0].value, "sonnet");
        assert_eq!(
            models[0].resolved_model.as_deref(),
            Some("claude-sonnet-5-5")
        );
        assert_eq!(models[1].value, "claude-opus-5-5");
        assert_eq!(models[1].resolved_model, None);
        assert!(parse_supported_models(
            br#"{"type":"control_response","response":{"subtype":"error","request_id":"models"}}"#
        )
        .is_err());
    }
    #[test]
    fn actual_model_is_only_recorded_when_stream_unambiguous() {
        let mut stream = ReplyStream::default();
        stream.feed(br#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-sonnet-5-5"}}}
"#, &mut |_| {}).unwrap();
        assert_eq!(stream.actual_model().as_deref(), Some("claude-sonnet-5-5"));
        stream.feed(br#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-opus-5-5"}}}
"#, &mut |_| {}).unwrap();
        assert_eq!(stream.actual_model(), None);
    }
    #[test]
    fn captures_success_and_rejects_failure() {
        let cancel = AtomicBool::new(false);
        assert_eq!(
            capture(
                shell("printf test"),
                vec![],
                Duration::from_secs(2),
                &cancel
            )
            .unwrap(),
            b"test"
        );
        assert!(capture(shell("exit 1"), vec![], Duration::from_secs(2), &cancel).is_err());
    }
    #[test]
    fn times_out_even_if_child_closes_stdout() {
        let cancel = AtomicBool::new(false);
        assert!(capture(
            shell("exec 1>&-; exec sleep 3"),
            vec![],
            Duration::from_millis(100),
            &cancel
        )
        .is_err());
    }
    #[test]
    fn rejects_excessive_output() {
        let cancel = AtomicBool::new(false);
        assert!(capture(
            shell("head -c 5000000 /dev/zero"),
            vec![],
            Duration::from_secs(5),
            &cancel
        )
        .is_err());
    }
    #[test]
    fn streams_before_exit_and_accepts_only_matching_success() {
        let cancel = AtomicBool::new(false);
        let mut stream = ReplyStream::default();
        let mut pieces = String::new();
        let start = Instant::now();
        let script = "printf '%s\\n' '{\"type\":\"stream_event\",\"event\":{\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}}'; sleep 1; printf '%s\\n' '{\"type\":\"stream_event\",\"event\":{\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}}' '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"hello\"}'";
        run(
            shell(script),
            vec![],
            Duration::from_secs(3),
            &cancel,
            |chunk| {
                stream.feed(chunk, &mut |piece| {
                    assert!(start.elapsed() < Duration::from_millis(800));
                    pieces.push_str(piece);
                })
            },
        )
        .unwrap();
        assert_eq!(stream.finish(&mut |_| {}).unwrap(), "hello");
        assert_eq!(pieces, "hello");
    }
    #[test]
    fn rejects_truncation_and_error_result() {
        for line in [
            r#"{"type":"result","subtype":"success","is_error":false,"result":"x","stop_reason":"max_tokens"}"#,
            r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":"x"}"#,
        ] {
            let mut stream = ReplyStream::default();
            let result = stream.feed(format!("{line}\n").as_bytes(), &mut |_| {});
            assert!(result.is_err() || stream.finish(&mut |_| {}).is_err());
        }
    }
    #[test]
    fn rejects_stream_without_terminal_stop_but_accepts_result_only() {
        let result =
            b"{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"x\"}\n";
        let mut streamed = ReplyStream::default();
        streamed
            .feed(
                b"{\"type\":\"stream_event\",\"event\":{\"type\":\"message_start\"}}\n",
                &mut |_| {},
            )
            .unwrap();
        streamed.feed(result, &mut |_| {}).unwrap();
        assert!(streamed.finish(&mut |_| {}).is_err());
        let mut complete = ReplyStream::default();
        complete.feed(result, &mut |_| {}).unwrap();
        assert_eq!(complete.finish(&mut |_| {}).unwrap(), "x");
    }
    #[test]
    fn rejects_success_if_child_did_not_accept_input() {
        let cancel = AtomicBool::new(false);
        assert!(capture(
            shell("exec 0<&-; printf success"),
            vec![b'x'; 1024 * 1024],
            Duration::from_secs(2),
            &cancel
        )
        .is_err());
    }
    #[test]
    fn cancellation_terminates_running_child() {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let task = thread::spawn(move || {
            capture(
                shell("exec sleep 5"),
                vec![],
                Duration::from_secs(10),
                &flag,
            )
        });
        thread::sleep(Duration::from_millis(100));
        cancel.store(true, Ordering::Release);
        assert!(task.join().unwrap().is_err());
    }
}

impl Runner {
    pub fn auth_status(&self, cancel: &AtomicBool) -> Result<bool, String> {
        let mut cmd = command(&self.binary);
        cmd.args(["auth", "status"]);
        let output = match capture(cmd, vec![], Duration::from_secs(20), cancel) {
            Ok(v) => v,
            Err(e) if e == ERROR => return Ok(false),
            Err(e) => return Err(e),
        };
        let json: Value = serde_json::from_slice(&output)
            .map_err(|_| "Claude 인증 상태를 확인하지 못했습니다.")?;
        Ok(json.get("loggedIn").and_then(Value::as_bool) == Some(true)
            && json.get("authMethod").and_then(Value::as_str) == Some("claude.ai"))
    }
    /// Claude Code opens its own browser flow. No terminal or credential transfer.
    pub fn login(&self, cancel: &AtomicBool) -> Result<(), String> {
        let mut cmd = command(&self.binary);
        cmd.args(["auth", "login"]);
        capture(cmd, vec![], Duration::from_secs(300), cancel)?;
        if !self.auth_status(cancel)? {
            return Err("Claude 구독 인증을 확인하지 못했습니다.".into());
        }
        Ok(())
    }
    pub fn identity(&self) -> Result<String, String> {
        use sha2::{Digest, Sha256};
        let mut file = std::fs::File::open(&self.binary).map_err(|_| ERROR)?;
        let mut hash = Sha256::new();
        let mut total = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let n = file.read(&mut buffer).map_err(|_| ERROR)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > 512 * 1024 * 1024 {
                return Err(ERROR.into());
            }
            hash.update(&buffer[..n]);
        }
        if total == 0 {
            return Err(ERROR.into());
        }
        Ok(format!("{:x}", hash.finalize()))
    }
    pub fn supported_models(&self, cancel: &AtomicBool) -> Result<Vec<SupportedModel>, String> {
        let work = tempfile::tempdir().map_err(|_| ERROR)?;
        let mut cmd = command(&self.binary);
        cmd.current_dir(work.path()).args([
            "--print",
            "--safe-mode",
            "--restricted",
            "--tools",
            "",
            "--disallowedTools",
            "mcp__*",
            "--strict-mcp-config",
            "--mcp-config",
            "{\"mcpServers\":{}}",
            "--settings",
            "{}",
            "--setting-sources",
            "",
            "--disable-slash-commands",
            "--no-session-persistence",
            "--no-chrome",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
        ]);
        let input = b"{\"type\":\"control_request\",\"request_id\":\"models\",\"request\":{\"subtype\":\"initialize\"}}\n".to_vec();
        let output = capture(cmd, input, Duration::from_secs(30), cancel)?;
        parse_supported_models(&output)
    }
    pub fn reply(
        &self,
        model: &str,
        prompt: &str,
        cancel: Arc<AtomicBool>,
        mut delta: impl FnMut(&str),
    ) -> Result<Reply, String> {
        if !valid_model(model) {
            return Err("Claude 모델을 확인해 주세요.".into());
        }
        if !self.auth_status(&cancel)? {
            return Err("Claude Code에서 구독 계정으로 로그인해 주세요.".into());
        }
        let work = tempfile::tempdir().map_err(|_| ERROR)?;
        let mut cmd = command(&self.binary);
        cmd.current_dir(work.path()).args([
            "--print",
            "--safe-mode",
            "--restricted",
            "--tools",
            "",
            "--disallowedTools",
            "mcp__*",
            "--strict-mcp-config",
            "--mcp-config",
            "{\"mcpServers\":{}}",
            "--settings",
            "{}",
            "--no-session-persistence",
            "--no-chrome",
            "--model",
            model,
            "--include-partial-messages",
            "--output-format",
            "stream-json",
            "--verbose",
        ]);
        let mut stream = ReplyStream::default();
        run(cmd, prompt.as_bytes().to_vec(), TIMEOUT, &cancel, |chunk| {
            stream.feed(chunk, &mut delta)
        })?;
        let text = stream.finish(&mut delta)?;
        Ok(Reply {
            text,
            actual_model_id: stream.actual_model(),
        })
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SupportedModel {
    pub value: String,
    pub display_name: String,
    pub resolved_model: Option<String>,
}

pub(crate) fn valid_model(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'[' | b']'))
}

fn parse_supported_models(output: &[u8]) -> Result<Vec<SupportedModel>, String> {
    for line in output
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
    {
        let frame: Value = serde_json::from_slice(line).map_err(|_| ERROR)?;
        if frame["type"] != "control_response" || frame["response"]["request_id"] != "models" {
            continue;
        }
        if frame["response"]["subtype"] != "success" {
            return Err(ERROR.into());
        }
        let rows = frame["response"]["response"]["models"]
            .as_array()
            .ok_or(ERROR)?;
        let mut result = Vec::new();
        for row in rows {
            let Some(value) = row["value"].as_str().filter(|v| valid_model(v)) else {
                continue;
            };
            let Some(name) = row["displayName"]
                .as_str()
                .filter(|v| !v.is_empty() && v.len() <= 256 && !v.chars().any(char::is_control))
            else {
                continue;
            };
            if result.iter().any(|m: &SupportedModel| m.value == value) {
                continue;
            }
            result.push(SupportedModel {
                value: value.into(),
                display_name: name.into(),
                resolved_model: row["resolvedModel"]
                    .as_str()
                    .filter(|v| valid_model(v))
                    .map(str::to_owned),
            });
        }
        if !result.is_empty() {
            return Ok(result);
        }
        return Err(ERROR.into());
    }
    Err(ERROR.into())
}

#[derive(Debug)]
pub struct Reply {
    pub text: String,
    pub actual_model_id: Option<String>,
}

#[derive(Default)]
struct ReplyStream {
    pending: Vec<u8>,
    streamed: String,
    result: Option<String>,
    stop: Option<String>,
    saw_partial: bool,
    actual_models: std::collections::BTreeSet<String>,
}
impl ReplyStream {
    fn feed(&mut self, chunk: &[u8], delta: &mut impl FnMut(&str)) -> Result<(), String> {
        self.pending.extend_from_slice(chunk);
        while let Some(pos) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=pos).collect();
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let v: Value = serde_json::from_slice(&line).map_err(|_| ERROR)?;
            match v.get("type").and_then(Value::as_str) {
                Some("stream_event") => {
                    self.saw_partial = true;
                    let event = &v["event"];
                    if event["type"] == "message_start" {
                        if let Some(model) = event["message"]["model"]
                            .as_str()
                            .filter(|v| valid_model(v))
                        {
                            self.actual_models.insert(model.to_owned());
                        }
                    }
                    if event["type"] == "content_block_delta"
                        && event["delta"]["type"] == "text_delta"
                    {
                        let text = event["delta"]["text"].as_str().ok_or(ERROR)?;
                        self.streamed.push_str(text);
                        delta(text);
                    }
                    if event["type"] == "message_delta" {
                        if let Some(stop) = event["delta"]["stop_reason"].as_str() {
                            self.stop = Some(stop.into());
                        }
                    }
                }
                Some("result") => {
                    if self.result.is_some() || v["is_error"] != false || v["subtype"] != "success"
                    {
                        return Err(ERROR.into());
                    }
                    if let Some(reason) = v["stop_reason"].as_str() {
                        self.stop = Some(reason.into());
                    }
                    self.result = Some(v["result"].as_str().ok_or(ERROR)?.into());
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn actual_model(&self) -> Option<String> {
        if self.actual_models.len() == 1 {
            self.actual_models.iter().next().cloned()
        } else {
            None
        }
    }
    fn finish(&mut self, delta: &mut impl FnMut(&str)) -> Result<String, String> {
        if !self.pending.is_empty() {
            self.feed(b"\n", delta)?;
        }
        if (self.saw_partial && self.stop.as_deref() != Some("end_turn"))
            || self.stop.as_deref().is_some_and(|s| s != "end_turn")
        {
            return Err("Claude 답변이 완성되지 않았습니다. 다시 시도해 주세요.".into());
        }
        let result = self.result.take().ok_or(ERROR)?;
        if self.streamed.is_empty() {
            delta(&result);
        } else if self.streamed != result {
            return Err(ERROR.into());
        }
        Ok(result)
    }
}

#[cfg(test)]
mod service_verification {
    #[tokio::test]
    #[ignore = "explicit official runtime download and code-signature verification"]
    async fn prepare_official_runtime_without_credentials() {
        let root = std::path::PathBuf::from(
            std::env::var_os("NERO_CLAUDE_PREPARE_ROOT").expect("explicit runtime destination"),
        );
        assert!(root.is_absolute());
        let runner = super::prepare(
            &root,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .await
        .unwrap();
        let mut command = super::command(&runner.binary);
        command.arg("--version");
        let output = super::capture(
            command,
            vec![],
            std::time::Duration::from_secs(20),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        let version = String::from_utf8(output).unwrap();
        assert!(version.contains("Claude Code"));
        eprintln!(
            "Official Claude Code prepared and signature verified: {}",
            version.trim()
        );
    }
}
