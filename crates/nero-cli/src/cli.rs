use crate::{dates, default_data_dir, dispatch, QueryError};
use grove_core::reader::MAX_RESPONSE_BYTES;
use jiff::tz::TimeZone;
use serde_json::{json, Map, Value};
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;

pub const HELP: &str = "Nero 로컬 CLI

사용법: nero [--data-dir PATH] [--json] <명령>

  meetings list [--trashed] [--folder-id ID | --unfiled] [--from MS|DATE] [--to MS|DATE] [--on DATE] [--timezone IANA] [--offset N] [--limit N]
  meetings search QUERY [--folder-id ID | --unfiled] [--from MS|DATE] [--to MS|DATE] [--on DATE] [--timezone IANA] [--offset N] [--limit N]
  meetings get ID [--offset N] [--limit N]
  transcript get ID [--start SECONDS] [--end SECONDS] [--offset N] [--limit N]
  summary get ID [--format txt|md] [--include-citations] [--offset N] [--limit N]
  summary context ID
  summary import ID --file JSON_PATH
  folders list [--offset N] [--limit N]
  meetings update ID --expected-version HASH --file JSON_PATH
  meetings date ID --expected-version HASH [--date YYYY-MM-DD | --clear]
  meetings move --file JSON_PATH
  meetings trash --file JSON_PATH
  meetings restore --file JSON_PATH
  folders create NAME --expected-version HASH
  folders rename ID NAME --expected-version HASH
  folders delete ID --expected-version HASH
  templates list
  templates get ID
  templates create --file JSON_PATH
  templates update ID --expected-revision HASH --file JSON_PATH
  templates delete ID --expected-revision HASH
  templates reset ID --expected-revision HASH
  jobs list [--meeting-id ID] [--offset N] [--limit N]
  jobs cancel ID --expected-version HASH
  doctor
  mcp

--json              성공 결과를 JSON으로, 오류를 stderr의 JSON으로 출력
--data-dir PATH     앱 데이터 폴더 지정. 조회는 폴더나 파일을 생성하지 않음
--help, -h          도움말
--version, -V       버전
--expected-source-version HASH  같은 조회 첫 페이지의 sourceVersion
--max-bytes N        JSON payload 예산. 1 이상 1048576 이하
--timezone IANA      목록 표시/날짜 해석 시간대. 생략한 목록 표시는 UTC

회의/폴더 목록과 본문은 기본 50개, 최대 200개씩 조회합니다.
meetings get의 메모와 summary get의 본문은 개행을 보존한 최대 4096바이트 텍스트 조각으로 나눕니다.
offset/limit은 조각 수이며 페이지의 본문을 순서대로 연결하면 선택한 형식의 전체 본문이 됩니다.
요약은 기본 Markdown이며 내부 인용 번호를 제외합니다. --format txt는 일반 텍스트로 변환합니다.
--include-citations는 현재 사용자가 내부 인용 번호를 명시적으로 요청한 경우에만 사용합니다.
날짜 필터는 Unix epoch 밀리초 또는 YYYY-MM-DD이며 from 포함, to 미포함입니다.
문자열 날짜와 --on은 명시 --timezone이 필요합니다. --on과 --from/--to는 함께 쓸 수 없습니다.
날짜 경계가 없는 날이나 모호한 자정은 거부하며 epoch 경계로 명시할 수 있습니다.
다음 페이지는 pagination.nextOffset과 첫 sourceVersion을 사용합니다. 예산에 따라 limit보다 적게 반환할 수 있습니다.
음성 기록 start/end는 초 단위이며 지정 구간과 겹치는 발언을 조회합니다.
doctor는 저장소가 없는 경우에도 설치 상태를 확인합니다.
mcp는 stdin/stdout으로 MCP를 제공하며 stdout에 진단 문구를 쓰지 않습니다.
템플릿 JSON은 name/description/prompt 세 문자열입니다. --file -는 stdin을 읽습니다.
사용자 템플릿은 삭제하며 기본 템플릿은 삭제 대신 reset을 사용합니다.
회의 수정은 실행 중인 Nero에 요청합니다. 파일은 update의 title/notes 또는
일괄 작업의 expectedVersions와 move의 folderId(null=미분류)를 포함합니다.
회의/폴더 수정본은 조회 응답의 editVersion이며 sourceVersion과 다릅니다.
작업 조회/취소는 실행 중인 Nero에 요청하며 모델을 실행하지 않습니다.
작업 취소는 jobs list의 editVersion을 사용하며 취소 요청 수락 후 상태를 다시 조회합니다.
요약 import 파일은 summary context의 context, 생성한 text, producer를 포함합니다.
원문/메모/기존 요약이 바뀌면 저장하지 않습니다. 요약 저장은 실행 중인 Nero에 요청합니다.
";

#[derive(Debug, PartialEq)]
enum Action {
    Help,
    Version,
    Mcp,
    Request {
        method: &'static str,
        params: Value,
    },
    FileRequest {
        method: &'static str,
        params: Value,
        path: PathBuf,
        content: FileContent,
    },
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum FileContent {
    Template,
    MeetingUpdate,
    MeetingMove,
    MeetingSet,
    SummaryImport,
}

#[derive(Debug)]
struct Options {
    root: Option<PathBuf>,
    json: bool,
    action: Action,
}

fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Options, QueryError> {
    let mut root = None;
    let mut json = false;
    let mut arguments = Vec::new();
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        if argument == "--data-dir" {
            if root.is_some() {
                return Err(QueryError::invalid());
            }
            let value = args
                .next()
                .filter(|value| !value.is_empty())
                .ok_or_else(QueryError::invalid)?;
            root = Some(PathBuf::from(value));
        } else if let Some(value) = argument
            .to_str()
            .and_then(|value| value.strip_prefix("--data-dir="))
        {
            if root.is_some() || value.is_empty() {
                return Err(QueryError::invalid());
            }
            root = Some(PathBuf::from(value));
        } else if argument == "--json" {
            if json {
                return Err(QueryError::invalid());
            }
            json = true;
        } else {
            arguments.push(argument.into_string().map_err(|_| QueryError::invalid())?);
        }
    }
    let strings: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let action = match strings.as_slice() {
        [] | ["--help" | "-h" | "help"] => Action::Help,
        ["--version" | "-V"] => Action::Version,
        ["mcp"] => Action::Mcp,
        ["doctor" | "status"] => Action::Request {
            method: "status",
            params: json!({}),
        },
        ["meetings" | "transcript" | "summary" | "folders" | "templates" | "jobs", ..]
            if strings
                .iter()
                .any(|value| matches!(*value, "--help" | "-h")) =>
        {
            Action::Help
        }
        ["jobs", command, remaining @ ..] => parse_jobs(command, remaining)?,
        ["templates", command, remaining @ ..] => parse_templates(command, remaining)?,
        ["summary", "import", remaining @ ..] => parse_summary_import(remaining)?,
        [group @ ("meetings" | "folders"), command @ ("update" | "date" | "move" | "trash" | "restore" | "create" | "rename"
        | "delete"), remaining @ ..] => parse_mutation(group, command, remaining)?,
        [group, command, remaining @ ..] => parse_query(group, command, remaining)?,
        _ => return Err(QueryError::invalid()),
    };
    Ok(Options { root, json, action })
}

fn parse_jobs(command: &str, remaining: &[&str]) -> Result<Action, QueryError> {
    let method = match command {
        "list" => "jobs_list",
        "cancel" => "job_cancel",
        _ => return Err(QueryError::invalid()),
    };
    let mut args = remaining.iter().copied();
    let mut params = Map::new();
    if command == "cancel" {
        let id = args
            .next()
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(QueryError::invalid)?;
        params.insert("jobId".into(), json!(id));
    }
    while let Some(argument) = args.next() {
        let (flag, inline) = argument
            .split_once('=')
            .map_or((argument, None), |(flag, value)| (flag, Some(value)));
        let value = inline
            .or_else(|| args.next())
            .filter(|value| !value.is_empty())
            .ok_or_else(QueryError::invalid)?;
        let (key, value) = match (command, flag) {
            ("list", "--meeting-id") => ("meetingId", json!(value)),
            ("list", "--offset" | "--limit") => (
                if flag == "--offset" {
                    "offset"
                } else {
                    "limit"
                },
                json!(value.parse::<usize>().map_err(|_| QueryError::invalid())?),
            ),
            ("cancel", "--expected-version") => ("expectedVersion", json!(value)),
            _ => return Err(QueryError::invalid()),
        };
        if params.insert(key.into(), value).is_some() {
            return Err(QueryError::invalid());
        }
    }
    let params = Value::Object(params);
    grove_core::agent_control::JobRequest::parse(method, params.clone())
        .map_err(|_| QueryError::invalid())?;
    Ok(Action::Request { method, params })
}

fn parse_query(group: &str, command: &str, remaining: &[&str]) -> Result<Action, QueryError> {
    let method = match (group, command) {
        ("meetings", "list") => "meetings_list",
        ("meetings", "search") => "meetings_search",
        ("meetings", "get") => "meeting_get",
        ("transcript", "get") => "transcript_get",
        ("summary", "get") => "summary_get",
        ("summary", "context") => "summary_context",
        ("folders", "list") => "folders_list",
        _ => return Err(QueryError::invalid()),
    };
    let mut params = Map::new();
    let mut args = remaining.iter().copied();
    if matches!(
        method,
        "meeting_get" | "transcript_get" | "summary_get" | "summary_context" | "meetings_search"
    ) {
        let value = args
            .next()
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(QueryError::invalid)?;
        params.insert(
            if method == "meetings_search" {
                "query"
            } else {
                "meetingId"
            }
            .into(),
            Value::String(value.into()),
        );
    }
    while let Some(argument) = args.next() {
        let (flag, inline) = argument
            .split_once('=')
            .map_or((argument, None), |(flag, value)| (flag, Some(value)));
        if flag == "--include-citations" {
            if method != "summary_get"
                || inline.is_some()
                || params
                    .insert("includeCitations".into(), Value::Bool(true))
                    .is_some()
            {
                return Err(QueryError::invalid());
            }
            continue;
        }
        if flag == "--trashed" {
            if method != "meetings_list"
                || inline.is_some()
                || params.insert("trashed".into(), Value::Bool(true)).is_some()
            {
                return Err(QueryError::invalid());
            }
            continue;
        }
        if flag == "--unfiled" {
            if !matches!(method, "meetings_list" | "meetings_search")
                || inline.is_some()
                || params.insert("unfiled".into(), Value::Bool(true)).is_some()
            {
                return Err(QueryError::invalid());
            }
            continue;
        }
        let field = match flag {
            "--offset" if method != "summary_context" => "offset",
            "--limit" if method != "summary_context" => "limit",
            "--max-bytes" => "maxBytes",
            "--format" if method == "summary_get" => "format",
            "--expected-source-version" => "expectedSourceVersion",
            "--folder-id" if matches!(method, "meetings_list" | "meetings_search") => "folderId",
            "--from" if matches!(method, "meetings_list" | "meetings_search") => "from",
            "--to" if matches!(method, "meetings_list" | "meetings_search") => "to",
            "--on" if matches!(method, "meetings_list" | "meetings_search") => "on",
            "--timezone" if matches!(method, "meetings_list" | "meetings_search") => "timezone",
            "--start" if method == "transcript_get" => "start",
            "--end" if method == "transcript_get" => "end",
            _ => return Err(QueryError::invalid()),
        };
        if params.contains_key(field) {
            return Err(QueryError::invalid());
        }
        let value = inline
            .or_else(|| args.next())
            .ok_or_else(QueryError::invalid)?;
        let value = match field {
            "folderId" | "on" | "timezone" | "expectedSourceVersion" => Value::String(value.into()),
            "format" if matches!(value, "txt" | "md") => Value::String(value.into()),
            "format" => return Err(QueryError::invalid()),
            "from" | "to" => match value.parse::<u64>() {
                Ok(number) => json!(number),
                Err(_) => Value::String(value.into()),
            },
            "start" | "end" => {
                let number = value.parse::<f64>().map_err(|_| QueryError::invalid())?;
                if !number.is_finite() || number < 0.0 {
                    return Err(QueryError::invalid());
                }
                json!(number)
            }
            _ => {
                let number = value.parse::<u64>().map_err(|_| QueryError::invalid())?;
                if (field == "limit" && !(1..=200).contains(&number))
                    || (field == "maxBytes" && !(1..=MAX_RESPONSE_BYTES as u64).contains(&number))
                {
                    return Err(QueryError::invalid());
                }
                json!(number)
            }
        };
        params.insert(field.into(), value);
    }
    if params.get("unfiled") == Some(&Value::Bool(true)) && params.contains_key("folderId") {
        return Err(QueryError::invalid());
    }
    let normalized = dates::normalize(method, Value::Object(params.clone()))?;
    if normalized
        .get("from")
        .and_then(Value::as_u64)
        .zip(normalized.get("to").and_then(Value::as_u64))
        .is_some_and(|(from, to)| from >= to)
    {
        return Err(QueryError::invalid());
    }
    if params
        .get("start")
        .and_then(Value::as_f64)
        .zip(params.get("end").and_then(Value::as_f64))
        .is_some_and(|(start, end)| start >= end)
    {
        return Err(QueryError::invalid());
    }
    Ok(Action::Request {
        method,
        params: Value::Object(params),
    })
}

fn parse_templates(command: &str, remaining: &[&str]) -> Result<Action, QueryError> {
    let method = match command {
        "list" => "templates_list",
        "get" => "template_get",
        "create" => "template_create",
        "update" => "template_update",
        "delete" => "template_delete",
        "reset" => "template_reset",
        _ => return Err(QueryError::invalid()),
    };
    let mut params = Map::new();
    let mut args = remaining.iter().copied();
    if !matches!(command, "list" | "create") {
        let id = args
            .next()
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(QueryError::invalid)?;
        params.insert("templateId".into(), json!(id));
    }
    let mut file = None;
    while let Some(argument) = args.next() {
        let (flag, inline) = argument
            .split_once('=')
            .map_or((argument, None), |(flag, value)| (flag, Some(value)));
        let value = inline
            .or_else(|| args.next())
            .filter(|value| !value.is_empty())
            .ok_or_else(QueryError::invalid)?;
        match flag {
            "--file" if matches!(command, "create" | "update") && file.is_none() => {
                file = Some(PathBuf::from(value))
            }
            "--expected-revision"
                if matches!(command, "update" | "delete" | "reset")
                    && !params.contains_key("expectedRevision") =>
            {
                params.insert("expectedRevision".into(), json!(value));
            }
            _ => return Err(QueryError::invalid()),
        }
    }
    if matches!(command, "update" | "delete" | "reset") && !params.contains_key("expectedRevision")
    {
        return Err(QueryError::invalid());
    }
    if matches!(command, "create" | "update") {
        Ok(Action::FileRequest {
            method,
            params: Value::Object(params),
            path: file.ok_or_else(QueryError::invalid)?,
            content: FileContent::Template,
        })
    } else {
        Ok(Action::Request {
            method,
            params: Value::Object(params),
        })
    }
}

fn parse_summary_import(remaining: &[&str]) -> Result<Action, QueryError> {
    let (id, remaining) = remaining.split_first().ok_or_else(QueryError::invalid)?;
    if id.starts_with("--") || id.is_empty() {
        return Err(QueryError::invalid());
    }
    let path = match remaining {
        ["--file", path] if !path.is_empty() => *path,
        [argument] => argument
            .strip_prefix("--file=")
            .filter(|path| !path.is_empty())
            .ok_or_else(QueryError::invalid)?,
        _ => return Err(QueryError::invalid()),
    };
    Ok(Action::FileRequest {
        method: "summary_import",
        params: json!({"meetingId":id}),
        path: PathBuf::from(path),
        content: FileContent::SummaryImport,
    })
}

fn parse_mutation(group: &str, command: &str, remaining: &[&str]) -> Result<Action, QueryError> {
    let method = match (group, command) {
        ("meetings", "update") => "meeting_update",
        ("meetings", "date") => "meeting_date_set",
        ("meetings", "move") => "meetings_move",
        ("meetings", "trash") => "meetings_trash",
        ("meetings", "restore") => "meetings_restore",
        ("folders", "create") => "folder_create",
        ("folders", "rename") => "folder_rename",
        ("folders", "delete") => "folder_delete",
        _ => return Err(QueryError::invalid()),
    };
    let mut args = remaining.iter().copied();
    let mut params = Map::new();
    if matches!(
        method,
        "meeting_update" | "meeting_date_set" | "folder_rename" | "folder_delete"
    ) {
        let id = args
            .next()
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(QueryError::invalid)?;
        params.insert(
            if method.starts_with("folder") {
                "folderId"
            } else {
                "meetingId"
            }
            .into(),
            json!(id),
        );
    }
    if matches!(method, "folder_create" | "folder_rename") {
        let name = args
            .next()
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(QueryError::invalid)?;
        params.insert("name".into(), json!(name));
    }
    let mut file = None;
    while let Some(argument) = args.next() {
        let (flag, inline) = argument
            .split_once('=')
            .map_or((argument, None), |(flag, value)| (flag, Some(value)));
        if flag == "--clear"
            && method == "meeting_date_set"
            && inline.is_none()
            && !params.contains_key("meetingDate")
        {
            params.insert("meetingDate".into(), Value::Null);
            continue;
        }
        let value = inline
            .or_else(|| args.next())
            .filter(|value| !value.is_empty())
            .ok_or_else(QueryError::invalid)?;
        match flag {
            "--expected-version"
                if !matches!(
                    method,
                    "meetings_move" | "meetings_trash" | "meetings_restore"
                ) && !params.contains_key("expectedVersion") =>
            {
                params.insert("expectedVersion".into(), json!(value));
            }
            "--date" if method == "meeting_date_set" && !params.contains_key("meetingDate") => {
                params.insert("meetingDate".into(), json!(value));
            }
            "--file"
                if matches!(
                    method,
                    "meeting_update" | "meetings_move" | "meetings_trash" | "meetings_restore"
                ) && file.is_none() =>
            {
                file = Some(PathBuf::from(value))
            }
            _ => return Err(QueryError::invalid()),
        }
    }
    let content = match method {
        "meeting_update" => Some(FileContent::MeetingUpdate),
        "meetings_move" => Some(FileContent::MeetingMove),
        "meetings_trash" | "meetings_restore" => Some(FileContent::MeetingSet),
        _ => None,
    };
    if let Some(content) = content {
        if method == "meeting_update" && !params.contains_key("expectedVersion") {
            return Err(QueryError::invalid());
        }
        Ok(Action::FileRequest {
            method,
            params: Value::Object(params),
            path: file.ok_or_else(QueryError::invalid)?,
            content,
        })
    } else {
        let params = Value::Object(params);
        grove_core::agent_control::Mutation::parse(method, params.clone())
            .map_err(|_| QueryError::invalid())?;
        Ok(Action::Request { method, params })
    }
}

fn read_content_file(
    path: &std::path::Path,
    mut params: Value,
    kind: FileContent,
) -> Result<Value, QueryError> {
    let error = || QueryError {
        code: "invalid_input",
        message: "요청 파일의 JSON 필드와 1MiB 이하 크기를 확인해 주세요.",
    };
    let mut bytes = Vec::new();
    let mut input: Box<dyn Read> = if path == std::path::Path::new("-") {
        Box::new(io::stdin().lock())
    } else {
        Box::new(std::fs::File::open(path).map_err(|_| error())?)
    };
    input
        .by_ref()
        .take(MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error())?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(error());
    }
    let content: Value = serde_json::from_slice(&bytes).map_err(|_| error())?;
    let content = content.as_object().ok_or_else(error)?;
    let valid = match kind {
        FileContent::Template => {
            content.len() == 3
                && ["name", "description", "prompt"]
                    .into_iter()
                    .all(|key| content.get(key).is_some_and(Value::is_string))
        }
        FileContent::MeetingUpdate => {
            !content.is_empty()
                && content
                    .keys()
                    .all(|key| matches!(key.as_str(), "title" | "notes"))
                && content.values().all(Value::is_string)
        }
        FileContent::MeetingMove => {
            content.len() == 2
                && content
                    .get("expectedVersions")
                    .is_some_and(Value::is_object)
                && content
                    .get("folderId")
                    .is_some_and(|value| value.is_null() || value.is_string())
        }
        FileContent::MeetingSet => {
            content.len() == 1
                && content
                    .get("expectedVersions")
                    .is_some_and(Value::is_object)
        }
        FileContent::SummaryImport => {
            content.len() == 3
                && content.get("context").is_some_and(Value::is_object)
                && content.get("text").is_some_and(Value::is_string)
                && content.get("producer").is_some_and(Value::is_object)
        }
    };
    if !valid {
        return Err(error());
    }
    for (key, value) in content {
        params
            .as_object_mut()
            .ok_or_else(error)?
            .insert(key.clone(), value.clone());
    }
    Ok(params)
}

fn execute_request(root: &std::path::Path, method: &str, params: Value, json: bool) -> i32 {
    let timezone = match dates::time_zone(params.get("timezone")) {
        Ok(zone) => zone,
        Err(error) => return write_error(&error, json),
    };
    match dispatch(root, method, params) {
        Ok(value) => write_stdout(&if json {
            format!("{}\n", value)
        } else {
            render(method, &value, &timezone)
        }),
        Err(error) => write_error(&error, json),
    }
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> i32 {
    let arguments: Vec<OsString> = args.into_iter().collect();
    let requested_json = arguments.iter().any(|value| value == "--json");
    let options = match parse(arguments) {
        Ok(options) => options,
        Err(error) => return write_error(&error, requested_json),
    };
    match options.action {
        Action::Help => return write_stdout(HELP),
        Action::Version => return write_stdout(&format!("nero {}\n", env!("CARGO_PKG_VERSION"))),
        _ => {}
    }
    let root = match options.root.map(Ok).unwrap_or_else(default_data_dir) {
        Ok(root) => root,
        Err(error) => return write_error(&error, options.json),
    };
    match options.action {
        Action::Mcp => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match runtime {
                Ok(runtime) => match runtime.block_on(crate::mcp::serve(root)) {
                    Ok(()) => 0,
                    Err(()) => write_error(
                        &QueryError {
                            code: "mcp_transport",
                            message: "MCP 연결을 처리할 수 없습니다.",
                        },
                        options.json,
                    ),
                },
                Err(_) => write_error(
                    &QueryError {
                        code: "mcp_runtime",
                        message: "MCP 실행 환경을 시작할 수 없습니다.",
                    },
                    options.json,
                ),
            }
        }
        Action::Request { method, params } => execute_request(&root, method, params, options.json),
        Action::FileRequest {
            method,
            params,
            path,
            content,
        } => match read_content_file(&path, params, content) {
            Ok(params) => execute_request(&root, method, params, options.json),
            Err(error) => write_error(&error, options.json),
        },
        _ => unreachable!(),
    }
}

fn write_stdout(output: &str) -> i32 {
    match io::stdout().lock().write_all(output.as_bytes()) {
        Ok(()) => 0,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => 0,
        Err(_) => 1,
    }
}

fn write_error(error: &QueryError, json: bool) -> i32 {
    let output = if json {
        error.as_json().to_string()
    } else {
        error.message.into()
    };
    let _ = writeln!(io::stderr().lock(), "{output}");
    if matches!(error.code, "invalid_input" | "unsupported_method") {
        2
    } else {
        1
    }
}

fn render(method: &str, value: &Value, timezone: &TimeZone) -> String {
    let mut output = String::new();
    match method {
        "jobs_list" => {
            output.push_str("ID\t종류\t회의 ID\t상태\t수정본\n");
            if let Some(jobs) = value["jobs"].as_array() {
                for job in jobs {
                    output.push_str(&format!(
                        "{}\t{}\t{}\t{}\t{}\n",
                        text(&job["id"]),
                        text(&job["kind"]),
                        text(&job["meetingId"]),
                        text(&job["status"]),
                        text(&job["editVersion"])
                    ));
                }
            }
        }
        "templates_list" => {
            output.push_str("ID\t이름\t용도\n");
            if let Some(items) = value["templates"].as_array() {
                for item in items {
                    output.push_str(&format!(
                        "{}\t{}\t{}\n",
                        text(&item["id"]),
                        text(&item["name"]),
                        text(&item["description"])
                    ));
                }
            }
        }
        "template_get" => {
            let template = &value["template"];
            output.push_str(&format!(
                "{}\nID: {}\n수정본: {}\n{}\n\n외부 출력 정책: {}\n\n프롬프트:\n{}\n",
                text(&template["name"]),
                text(&template["id"]),
                text(&template["revision"]),
                text(&template["description"]),
                body(&value["outputPolicy"]["instruction"]),
                body(&template["prompt"])
            ));
        }
        method if !crate::is_read_only(method) => {
            output.push_str(&serde_json::to_string_pretty(value).unwrap());
            output.push('\n');
        }
        "meetings_list" | "meetings_search" => {
            output.push_str(&format!(
                "녹음/생성 시각 ({})\tID\t제목\t상태\n",
                timezone.iana_name().unwrap_or("UTC")
            ));
            if let Some(meetings) = value["meetings"].as_array() {
                for meeting in meetings {
                    output.push_str(&format!(
                        "{}\t{}\t{}\t{}\n",
                        dates::timestamp(
                            meeting
                                .get("recordedAt")
                                .filter(|value| !value.is_null())
                                .unwrap_or(&meeting["createdAt"]),
                            timezone
                        ),
                        text(&meeting["id"]),
                        text(&meeting["title"]),
                        text(&meeting["status"])
                    ));
                    if let Some(matches) = meeting["matches"].as_array() {
                        for found in matches {
                            output.push_str(&format!(
                                "  {}: {}\n",
                                text(&found["field"]),
                                text(&found["text"])
                            ));
                        }
                    }
                }
            }
        }
        "folders_list" => {
            output.push_str("ID\t폴더\n");
            if let Some(folders) = value["folders"].as_array() {
                for folder in folders {
                    output.push_str(&format!(
                        "{}\t{}\n",
                        text(&folder["id"]),
                        text(&folder["name"])
                    ));
                }
            }
        }
        "meeting_get" => {
            let meeting = &value["meeting"];
            output.push_str(&format!(
                "{}\nID: {}\n상태: {}\n\n{}\n",
                text(&meeting["title"]),
                text(&meeting["id"]),
                text(&meeting["status"]),
                body(&meeting["notes"])
            ));
        }
        "transcript_get" => {
            if value["version"].is_null() {
                output.push_str("저장된 음성 기록이 없습니다.\n");
            }
            if let Some(segments) = value["segments"].as_array() {
                for segment in segments {
                    output.push_str(&format!(
                        "[{} - {}] {}\n{}\n",
                        segment["start"],
                        segment["end"],
                        value["speakers"]
                            .as_array()
                            .and_then(|speakers| speakers
                                .iter()
                                .find(|speaker| speaker["id"] == segment["speakerId"]))
                            .map(|speaker| text(&speaker["name"]))
                            .unwrap_or_default(),
                        body(&segment["text"])
                    ));
                }
            }
        }
        "summary_get" => {
            if value["summary"].is_null() {
                output.push_str("저장된 요약이 없습니다.\n");
            } else {
                output.push_str(&body(&value["summary"]["text"]));
                output.push('\n');
            }
        }
        "summary_context" => {
            output.push_str(&serde_json::to_string_pretty(value).unwrap());
            output.push('\n');
        }
        "status" => {
            output.push_str("조회: 읽기 전용\n템플릿 관리: 가능\n회의 수정: Nero 실행 필요\n");
            output.push_str(if value["rootExists"] == true {
                "데이터 폴더: 있음\n"
            } else {
                "데이터 폴더: 없음\n"
            });
        }
        _ => {}
    }
    if let Some(version) = value["editVersion"].as_str() {
        output.push_str(&format!("\n수정본: {version}\n"));
    }
    if let Some(page) = value["pagination"].as_object() {
        output.push_str(&format!(
            "\n조회: {}부터, 반환 {}, 전체 {}\n",
            page.get("offset").unwrap_or(&Value::Null),
            page.get("returned").unwrap_or(&Value::Null),
            page.get("total").unwrap_or(&Value::Null)
        ));
        if page.get("hasMore") == Some(&Value::Bool(true)) {
            if method == "jobs_list" {
                output.push_str(&format!(
                    "다음: --offset {}\n",
                    page.get("nextOffset").unwrap_or(&Value::Null)
                ));
            } else {
                output.push_str(&format!(
                    "다음: --offset {} --expected-source-version {}\n",
                    page.get("nextOffset").unwrap_or(&Value::Null),
                    text(&value["sourceVersion"])
                ));
            }
        } else {
            output.push_str("요청한 조회 범위의 끝입니다.\n");
        }
    }
    output
}

fn text(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or("")
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

fn body(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Result<Options, QueryError> {
        parse(values.iter().map(OsString::from))
    }

    #[test]
    fn jobs_require_bounded_queries_and_explicit_cancel_version() {
        let id = "00000000-0000-0000-0000-000000000001";
        let version = format!("sha256:{}", "a".repeat(64));
        assert!(args(&["jobs", "list", "--meeting-id", id, "--limit", "200"]).is_ok());
        assert!(args(&["jobs", "list", "--limit", "201"]).is_err());
        assert!(args(&["jobs", "list", "--limit", "0"]).is_err());
        assert!(args(&["jobs", "cancel", id]).is_err());
        assert!(args(&["jobs", "cancel", id, "--expected-version", &version]).is_ok());
        assert!(args(&[
            "jobs",
            "cancel",
            id,
            "--expected-version",
            &version,
            "--expected-version",
            &version
        ])
        .is_err());
    }

    #[test]
    fn parses_global_options_and_bounded_body_reads() {
        let options = args(&[
            "summary",
            "get",
            "id",
            "--offset",
            "50",
            "--json",
            "--limit=20",
            "--data-dir",
            "/missing",
        ])
        .unwrap();
        assert!(options.json);
        assert_eq!(options.root, Some(PathBuf::from("/missing")));
        assert_eq!(
            options.action,
            Action::Request {
                method: "summary_get",
                params: json!({"meetingId":"id","offset":50,"limit":20})
            }
        );
    }

    #[test]
    fn unfiled_filter_is_explicit_and_excludes_a_folder_id() {
        assert_eq!(
            args(&["meetings", "list", "--unfiled"]).unwrap().action,
            Action::Request {
                method: "meetings_list",
                params: json!({"unfiled":true})
            }
        );
        assert_eq!(
            args(&["meetings", "search", "term", "--unfiled"])
                .unwrap()
                .action,
            Action::Request {
                method: "meetings_search",
                params: json!({"query":"term","unfiled":true})
            }
        );
        for values in [
            vec!["meetings", "list", "--unfiled", "--folder-id", "id"],
            vec!["meetings", "list", "--unfiled", "--unfiled"],
            vec!["meetings", "list", "--unfiled=true"],
            vec!["folders", "list", "--unfiled"],
        ] {
            assert!(args(&values).is_err());
        }
    }

    #[test]
    fn rejects_writes_unknown_duplicate_and_invalid_filters() {
        for values in [
            vec!["meetings", "delete", "id"],
            vec!["meetings", "list", "--limit", "201"],
            vec!["meetings", "list", "--limit", "0"],
            vec!["meetings", "list", "--from", "20", "--to", "10"],
            vec!["transcript", "get", "id", "--start", "NaN"],
            vec!["meetings", "list", "--unknown", "value"],
            vec!["meetings", "list", "--limit", "2", "--limit", "3"],
        ] {
            assert!(args(&values).is_err(), "{values:?}");
        }
    }

    #[test]
    fn date_and_page_arguments_are_explicit_and_method_scoped() {
        let version = format!("sha256:{}", "a".repeat(64));
        let parsed = args(&[
            "meetings",
            "list",
            "--on",
            "2026-10-28",
            "--timezone",
            "Asia/Seoul",
            "--max-bytes",
            "12000",
            "--expected-source-version",
            &version,
        ])
        .unwrap();
        assert_eq!(
            parsed.action,
            Action::Request {
                method: "meetings_list",
                params: json!({"on":"2026-10-28","timezone":"Asia/Seoul","maxBytes":12000,"expectedSourceVersion":version})
            }
        );
        for arguments in [
            vec!["meetings", "list", "--from", "2026-10-28"],
            vec![
                "meetings",
                "list",
                "--on",
                "2026-10-28",
                "--timezone",
                "Invalid/Zone",
            ],
            vec![
                "meetings",
                "list",
                "--from",
                "2026-10-29",
                "--to",
                "2026-10-28",
                "--timezone",
                "UTC",
            ],
            vec![
                "meetings",
                "list",
                "--on",
                "2026-10-28",
                "--from",
                "1",
                "--timezone",
                "UTC",
            ],
            vec!["meetings", "list", "--max-bytes", "0"],
            vec!["meetings", "list", "--max-bytes", "1048577"],
            vec!["transcript", "get", "id", "--timezone", "UTC"],
            vec!["folders", "list", "--on", "2026-10-28"],
        ] {
            assert!(args(&arguments).is_err(), "{arguments:?}");
        }
    }

    #[test]
    fn human_list_displays_the_actual_recording_time_and_next_page_contract() {
        let output = render(
            "meetings_list",
            &json!({"meetings":[{"id":"one","title":"Meeting","status":"draft","createdAt":0,"recordedAt":1000}],"sourceVersion":"sha256:test","pagination":{"offset":0,"limit":50,"returned":1,"total":2,"hasMore":true,"nextOffset":1}}),
            &TimeZone::UTC,
        );
        assert!(output.contains("녹음/생성 시각 (UTC)"));
        assert!(output.contains("1970-01-01 00:00\tone\tMeeting\tdraft"));
        assert!(output.contains("--offset 1 --expected-source-version sha256:test"));
        assert!(!output.contains("--offset 50"));
    }

    #[test]
    fn mutation_commands_require_explicit_revisions_and_file_shapes() {
        let version = format!("sha256:{}", "a".repeat(64));
        assert!(matches!(
            args(&[
                "templates",
                "update",
                "meeting",
                "--expected-revision",
                &version,
                "--file",
                "input.json"
            ])
            .unwrap()
            .action,
            Action::FileRequest {
                method: "template_update",
                content: FileContent::Template,
                ..
            }
        ));
        assert!(matches!(
            args(&[
                "meetings",
                "update",
                "11111111-1111-4111-8111-111111111111",
                "--expected-version",
                &version,
                "--file",
                "input.json"
            ])
            .unwrap()
            .action,
            Action::FileRequest {
                method: "meeting_update",
                content: FileContent::MeetingUpdate,
                ..
            }
        ));
        assert!(matches!(
            args(&["meetings", "trash", "--file", "input.json"])
                .unwrap()
                .action,
            Action::FileRequest {
                method: "meetings_trash",
                content: FileContent::MeetingSet,
                ..
            }
        ));
        for arguments in [
            vec!["templates", "update", "meeting", "--file", "input.json"],
            vec!["templates", "delete", "meeting"],
            vec!["templates", "create"],
            vec!["meetings", "update", "id", "--file", "input.json"],
            vec![
                "meetings",
                "date",
                "11111111-1111-4111-8111-111111111111",
                "--expected-version",
                &version,
                "--date",
                "2026-10-28",
                "--clear",
            ],
            vec!["folders", "create", "Folder"],
            vec!["templates", "get", "meeting", "--deleted"],
        ] {
            assert!(args(&arguments).is_err(), "{arguments:?}");
        }
    }

    #[test]
    fn summary_commands_keep_write_context_in_the_input_file() {
        assert_eq!(
            args(&["summary", "context", "id"]).unwrap().action,
            Action::Request {
                method: "summary_context",
                params: json!({"meetingId":"id"})
            }
        );
        assert_eq!(
            args(&["summary", "import", "id", "--file=input.json"])
                .unwrap()
                .action,
            Action::FileRequest {
                method: "summary_import",
                params: json!({"meetingId":"id"}),
                path: PathBuf::from("input.json"),
                content: FileContent::SummaryImport
            }
        );
        for arguments in [
            vec!["summary", "context", "id", "--offset", "0"],
            vec!["summary", "context", "id", "--limit", "1"],
            vec!["summary", "import", "id"],
            vec!["summary", "import", "id", "--file="],
            vec!["summary", "import", "id", "--file", "one", "--file", "two"],
            vec![
                "summary",
                "import",
                "id",
                "--expected-source-version",
                "hash",
                "--file",
                "input.json",
            ],
        ] {
            assert!(args(&arguments).is_err(), "{arguments:?}");
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("summary.json");
        let input = json!({"context":{},"text":"Summary","producer":{"name":"Agent"}});
        std::fs::write(&path, input.to_string()).unwrap();
        let result = read_content_file(
            &path,
            json!({"meetingId":"fixed"}),
            FileContent::SummaryImport,
        )
        .unwrap();
        assert_eq!(result["meetingId"], "fixed");
        assert_eq!(result["text"], "Summary");
        let mut overridden = input;
        overridden["meetingId"] = json!("other");
        std::fs::write(&path, overridden.to_string()).unwrap();
        assert!(read_content_file(
            &path,
            json!({"meetingId":"fixed"}),
            FileContent::SummaryImport
        )
        .is_err());
    }

    #[test]
    fn input_files_cannot_override_the_target_or_expected_revision() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("content.json");
        std::fs::write(
            &path,
            serde_json::to_vec(
                &json!({"name":"Title","description":"Use","prompt":"Text","templateId":"other"}),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(read_content_file(
            &path,
            json!({"templateId":"meeting","expectedRevision":"fixed"}),
            FileContent::Template
        )
        .is_err());
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({"notes":"Text","expectedVersion":"other"})).unwrap(),
        )
        .unwrap();
        assert!(read_content_file(
            &path,
            json!({"meetingId":"one","expectedVersion":"fixed"}),
            FileContent::MeetingUpdate
        )
        .is_err());
    }

    #[test]
    fn help_and_version_need_no_library() {
        assert_eq!(args(&["--help"]).unwrap().action, Action::Help);
        assert_eq!(args(&["--version"]).unwrap().action, Action::Version);
        assert_eq!(
            args(&["transcript", "get", "--help"]).unwrap().action,
            Action::Help
        );
    }
}
