pub mod cli;
mod connection;
pub mod mcp;

use grove_core::{
    reader::{LibraryReader, ReadError},
    CoreError,
};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

pub const APP_IDENTIFIER: &str = "io.github.choseongmin1128.grove.desktop";
pub const METHODS: [&str; 7] = [
    "meetings_list",
    "meetings_search",
    "meeting_get",
    "transcript_get",
    "summary_get",
    "folders_list",
    "status",
];

#[derive(Debug)]
pub struct QueryError {
    pub code: &'static str,
    pub message: &'static str,
}

impl QueryError {
    pub fn invalid() -> Self {
        Self {
            code: "invalid_input",
            message: "입력값이 올바르지 않습니다. 도움말의 형식과 조회 범위를 확인해 주세요.",
        }
    }

    pub fn as_json(&self) -> Value {
        json!({"error": {"code": self.code, "message": self.message}})
    }
}

pub fn dispatch(root: &Path, method: &str, params: Value) -> Result<Value, QueryError> {
    if !METHODS.contains(&method) {
        return Err(QueryError {
            code: "unsupported_method",
            message: "지원하지 않는 명령입니다. Nero CLI는 읽기 전용입니다.",
        });
    }
    if method != "status" && !root.exists() {
        return Err(QueryError {
            code: "library_not_found",
            message: "Nero 데이터 폴더가 없습니다. 앱에서 저장한 뒤 다시 시도하거나 --data-dir을 지정해 주세요.",
        });
    }
    let reader = LibraryReader::open(root).map_err(query_error)?;
    reader.query(method, params).map_err(query_error)
}

fn query_error(error: CoreError) -> QueryError {
    match error {
        CoreError::Reader(error) => {
            let (code, message) = match error {
                ReadError::ResponseTooLarge => (
                    "response_too_large",
                    "조회 응답이 1MiB를 넘습니다. limit 또는 음성 구간을 줄여 다시 조회해 주세요.",
                ),
                ReadError::SnapshotChanged => (
                    "library_changed",
                    "조회 중 저장 정보가 변경되었습니다. 잠시 후 다시 조회해 주세요.",
                ),
                ReadError::InvalidDocument => (
                    "invalid_document",
                    "Nero 저장 정보가 손상되었거나 필드/연결이 누락되었거나 올바르지 않습니다. 앱에서 해당 기록을 확인해 주세요.",
                ),
                ReadError::UnsupportedFormat => (
                    "unsupported_format",
                    "지원하지 않는 저장 형식입니다. Nero 앱과 CLI를 업데이트해 주세요.",
                ),
                ReadError::DocumentTooLarge => (
                    "document_too_large",
                    "저장 정보가 읽기 한도 16MiB를 넘습니다. 앱에서 해당 기록을 확인해 주세요.",
                ),
                ReadError::UnsafePath => (
                    "unsafe_path",
                    "데이터 경로에 연결된 파일/폴더 또는 일반 폴더가 아닌 항목이 있습니다. 실제 Nero 데이터 폴더를 지정해 주세요.",
                ),
            };
            QueryError { code, message }
        }
        CoreError::InvalidInput(_) => QueryError::invalid(),
        CoreError::MeetingNotFound => QueryError {
            code: "meeting_not_found",
            message: "회의를 찾을 수 없습니다.",
        },
        _ => QueryError {
            code: "library_unavailable",
            message: "Nero 저장 정보를 읽을 수 없습니다. 데이터 폴더와 접근 권한을 확인하고 잠시 후 다시 조회해 주세요.",
        },
    }
}

pub fn default_data_dir() -> Result<PathBuf, QueryError> {
    data_dir_for(|name| std::env::var_os(name), std::env::consts::OS).ok_or(QueryError {
        code: "data_dir_unavailable",
        message: "Nero 데이터 경로를 확인할 수 없습니다. --data-dir을 지정해 주세요.",
    })
}

fn data_dir_for(
    env: impl Fn(&str) -> Option<std::ffi::OsString>,
    platform: &str,
) -> Option<PathBuf> {
    let home = || {
        env("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let base = match platform {
        "macos" => home()?.join("Library/Application Support"),
        "windows" => PathBuf::from(env("APPDATA").filter(|value| !value.is_empty())?),
        _ => env("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|value| value.is_absolute())
            .or_else(|| home().map(|value| value.join(".local/share")))?,
    };
    Some(base.join(APP_IDENTIFIER))
}

pub fn input_schema(method: &str) -> Map<String, Value> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    if matches!(method, "meeting_get" | "transcript_get" | "summary_get") {
        properties.insert("meetingId".into(), id_schema());
        required.push("meetingId");
    }
    if matches!(method, "meetings_list" | "meetings_search") {
        properties.insert("folderId".into(), id_schema());
        for field in ["from", "to"] {
            properties.insert(field.into(), json!({"type":"integer","minimum":0,"description":"Unix epoch milliseconds. from is inclusive; to is exclusive."}));
        }
    }
    if method == "meetings_search" {
        properties.insert("query".into(), json!({"type":"string","minLength":1,"maxLength":4096,"description":"Nonempty literal text, at most 4096 UTF-8 bytes, to find in titles, notes, saved transcripts and summaries."}));
        required.push("query");
    }
    if method == "transcript_get" {
        for field in ["start", "end"] {
            properties.insert(field.into(), json!({"type":"number","minimum":0,"description":"Seconds. start is inclusive; end is exclusive."}));
        }
    }
    if method != "status" {
        let (offset_description, limit_description) = if matches!(
            method,
            "meeting_get" | "summary_get"
        ) {
            (
                "Offset in textChunks. Each chunk is at most 4096 UTF-8 bytes and retains original newlines.",
                "Maximum textChunks to return. Concatenate page text in order to recover the original body.",
            )
        } else {
            (
                "Number of items to skip.",
                "Maximum number of items to return.",
            )
        };
        properties.insert(
            "offset".into(),
            json!({"type":"integer","minimum":0,"default":0,"description":offset_description}),
        );
        properties.insert(
            "limit".into(),
            json!({"type":"integer","minimum":1,"maximum":200,"default":50,"description":limit_description}),
        );
    }
    json!({
        "type":"object", "properties":properties, "required":required,
        "additionalProperties":false
    })
    .as_object()
    .unwrap()
    .clone()
}

fn id_schema() -> Value {
    json!({"type":"string","pattern":"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn platform_paths_use_the_native_identifier_without_legacy_fallback() {
        let env = |key: &str| match key {
            "HOME" => Some(OsString::from("/home/example")),
            "APPDATA" => Some(OsString::from("C:/Users/example/AppData/Roaming")),
            "XDG_DATA_HOME" => Some(OsString::from("/custom/data")),
            _ => None,
        };
        assert_eq!(
            data_dir_for(env, "macos").unwrap(),
            PathBuf::from("/home/example/Library/Application Support").join(APP_IDENTIFIER)
        );
        assert_eq!(
            data_dir_for(env, "windows").unwrap(),
            PathBuf::from("C:/Users/example/AppData/Roaming").join(APP_IDENTIFIER)
        );
        assert_eq!(
            data_dir_for(env, "linux").unwrap(),
            PathBuf::from("/custom/data").join(APP_IDENTIFIER)
        );
        assert!(data_dir_for(|_| None, "macos").is_none());
    }

    #[test]
    fn unsupported_writes_do_not_open_or_create_a_library() {
        let temporary = tempfile::tempdir().unwrap();
        let missing = temporary.path().join("missing");
        assert_eq!(
            dispatch(&missing, "meeting_delete", json!({}))
                .unwrap_err()
                .code,
            "unsupported_method"
        );
        assert!(!missing.exists());
    }

    #[test]
    fn reader_failures_have_distinct_static_recovery_messages() {
        for (failure, code, hint) in [
            (ReadError::ResponseTooLarge, "response_too_large", "limit"),
            (ReadError::SnapshotChanged, "library_changed", "다시 조회"),
            (ReadError::InvalidDocument, "invalid_document", "손상"),
            (
                ReadError::UnsupportedFormat,
                "unsupported_format",
                "업데이트",
            ),
            (ReadError::DocumentTooLarge, "document_too_large", "16MiB"),
            (
                ReadError::UnsafePath,
                "unsafe_path",
                "실제 Nero 데이터 폴더",
            ),
        ] {
            let error = query_error(failure.into());
            assert_eq!(error.code, code);
            assert!(error.message.contains(hint));
        }
        for failure in [
            CoreError::Storage("/private/synthetic-secret: SYNTHETIC_PRIVATE_BODY".into()),
            CoreError::InvalidInput("SYNTHETIC_PRIVATE_BODY".into()),
        ] {
            let error = query_error(failure).as_json().to_string();
            assert!(!error.contains("synthetic-secret"));
            assert!(!error.contains("SYNTHETIC_PRIVATE_BODY"));
        }
    }
}
