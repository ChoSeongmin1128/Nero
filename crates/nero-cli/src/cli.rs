use crate::{default_data_dir, dispatch, QueryError};
use serde_json::{json, Map, Value};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;

pub const HELP: &str = "Nero 읽기 전용 CLI

사용법: nero [--data-dir PATH] [--json] <명령>

  meetings list [--folder-id ID | --unfiled] [--from MS] [--to MS] [--offset N] [--limit N]
  meetings search QUERY [--folder-id ID | --unfiled] [--from MS] [--to MS] [--offset N] [--limit N]
  meetings get ID [--offset N] [--limit N]
  transcript get ID [--start SECONDS] [--end SECONDS] [--offset N] [--limit N]
  summary get ID [--offset N] [--limit N]
  folders list [--offset N] [--limit N]
  doctor
  mcp

--json              성공 결과를 JSON으로, 오류를 stderr의 JSON으로 출력
--data-dir PATH     앱 데이터 폴더 지정. 폴더나 파일을 생성하지 않음
--help, -h          도움말
--version, -V       버전

목록과 본문은 기본 50개, 최대 200개씩 조회합니다.
meetings get의 메모와 summary get의 본문은 개행을 보존한 최대 4096바이트 텍스트 조각으로 나눕니다.
offset/limit은 조각 수이며 페이지의 본문을 순서대로 연결하면 원문이 됩니다.
날짜 필터는 Unix epoch 밀리초이며 from 포함, to 미포함입니다.
음성 기록 start/end는 초 단위이며 지정 구간과 겹치는 발언을 조회합니다.
doctor는 저장소가 없는 경우에도 설치 상태를 확인합니다.
mcp는 stdin/stdout으로 MCP를 제공하며 stdout에 진단 문구를 쓰지 않습니다.
";

#[derive(Debug, PartialEq)]
enum Action {
    Help,
    Version,
    Mcp,
    Query { method: &'static str, params: Value },
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
        ["doctor" | "status"] => Action::Query {
            method: "status",
            params: json!({}),
        },
        ["meetings" | "transcript" | "summary" | "folders", ..]
            if strings
                .iter()
                .any(|value| matches!(*value, "--help" | "-h")) =>
        {
            Action::Help
        }
        [group, command, remaining @ ..] => parse_query(group, command, remaining)?,
        _ => return Err(QueryError::invalid()),
    };
    Ok(Options { root, json, action })
}

fn parse_query(group: &str, command: &str, remaining: &[&str]) -> Result<Action, QueryError> {
    let method = match (group, command) {
        ("meetings", "list") => "meetings_list",
        ("meetings", "search") => "meetings_search",
        ("meetings", "get") => "meeting_get",
        ("transcript", "get") => "transcript_get",
        ("summary", "get") => "summary_get",
        ("folders", "list") => "folders_list",
        _ => return Err(QueryError::invalid()),
    };
    let mut params = Map::new();
    let mut args = remaining.iter().copied();
    if matches!(
        method,
        "meeting_get" | "transcript_get" | "summary_get" | "meetings_search"
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
            "--offset" => "offset",
            "--limit" => "limit",
            "--folder-id" if matches!(method, "meetings_list" | "meetings_search") => "folderId",
            "--from" if matches!(method, "meetings_list" | "meetings_search") => "from",
            "--to" if matches!(method, "meetings_list" | "meetings_search") => "to",
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
            "folderId" => Value::String(value.into()),
            "start" | "end" => {
                let number = value.parse::<f64>().map_err(|_| QueryError::invalid())?;
                if !number.is_finite() || number < 0.0 {
                    return Err(QueryError::invalid());
                }
                json!(number)
            }
            _ => {
                let number = value.parse::<u64>().map_err(|_| QueryError::invalid())?;
                if field == "limit" && !(1..=200).contains(&number) {
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
    if params
        .get("from")
        .and_then(Value::as_u64)
        .zip(params.get("to").and_then(Value::as_u64))
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
    Ok(Action::Query {
        method,
        params: Value::Object(params),
    })
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
        Action::Query { method, params } => match dispatch(&root, method, params) {
            Ok(value) => {
                let output = if options.json {
                    format!("{}\n", value)
                } else {
                    render(method, &value)
                };
                write_stdout(&output)
            }
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

fn render(method: &str, value: &Value) -> String {
    let mut output = String::new();
    match method {
        "meetings_list" | "meetings_search" => {
            output.push_str("ID\t제목\t상태\n");
            if let Some(meetings) = value["meetings"].as_array() {
                for meeting in meetings {
                    output.push_str(&format!(
                        "{}\t{}\t{}\n",
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
                if value["summary"]["stale"] == true {
                    output.push_str("음성 기록 수정 전의 요약입니다.\n\n");
                }
                output.push_str(&body(&value["summary"]["text"]));
                output.push('\n');
            }
        }
        "status" => {
            output.push_str("읽기 전용: 예\n");
            output.push_str(if value["rootExists"] == true {
                "데이터 폴더: 있음\n"
            } else {
                "데이터 폴더: 없음\n"
            });
        }
        _ => {}
    }
    if let Some(page) = value["pagination"].as_object() {
        output.push_str(&format!(
            "\n조회: {}부터, 최대 {}, 전체 {}\n",
            page.get("offset").unwrap_or(&Value::Null),
            page.get("limit").unwrap_or(&Value::Null),
            page.get("total").unwrap_or(&Value::Null)
        ));
        if page.get("hasMore") == Some(&Value::Bool(true)) {
            output.push_str("다음 내용은 --offset을 늘려 조회해 주세요.\n");
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
            Action::Query {
                method: "summary_get",
                params: json!({"meetingId":"id","offset":50,"limit":20})
            }
        );
    }

    #[test]
    fn unfiled_filter_is_explicit_and_excludes_a_folder_id() {
        assert_eq!(
            args(&["meetings", "list", "--unfiled"]).unwrap().action,
            Action::Query {
                method: "meetings_list",
                params: json!({"unfiled":true})
            }
        );
        assert_eq!(
            args(&["meetings", "search", "term", "--unfiled"])
                .unwrap()
                .action,
            Action::Query {
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
    fn help_and_version_need_no_library() {
        assert_eq!(args(&["--help"]).unwrap().action, Action::Help);
        assert_eq!(args(&["--version"]).unwrap().action, Action::Version);
        assert_eq!(
            args(&["transcript", "get", "--help"]).unwrap().action,
            Action::Help
        );
    }
}
