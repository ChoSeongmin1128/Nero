mod app_tools;
pub mod cli;
mod connection;
mod dates;
pub mod mcp;
mod mutations;
mod templates;
mod updates;
mod versions;

use nero_core::{
    reader::{LibraryReader, ReadError},
    CoreError,
};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

pub use nero_core::identity::APP_IDENTIFIER;
pub(crate) struct ToolInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub read_only: bool,
    pub destructive: bool,
    pub open_world: bool,
}

// Tools served by the CLI itself or by the app's writer bridge; app feature tools live in
// app_tools::TOOLS. METHODS, MCP annotations and descriptions all come from these two tables.
const LEGACY: [ToolInfo; 33] = [
    ToolInfo { name: "app_update", read_only: false, destructive: true, open_world: true,
        description: "Manage the running Nero app's official signed update service. action=status reads local update state; check contacts the public release channel; download starts an app-owned download and returns accepted; install saves all open editors, refuses unsaved/busy work, then accepts installation and restart. Requires explicit current user intent for check/download/install. No arbitrary URLs, files, keys or installers. accepted/restarting is not completion: re-query status and the running app version after restart. Older apps require their update screen. Never automatically retry an uncertain install." },
    ToolInfo { name: "jobs_list", read_only: true, destructive: false, open_world: false,
        description: "Read minimal processing job IDs, kinds, meeting IDs, states and editVersion from running Nero, newest first. Does not start processing or load models. Pages reflect current state." },
    ToolInfo { name: "job_cancel", read_only: false, destructive: true, open_world: false,
        description: "Cancel one queued or running job only on explicit current user intent, using its jobs_list editVersion. Requires running Nero. Saving results cannot be interrupted. Applied means cancellation accepted; query jobs_list for completion. Do not automatically retry conflicts or uncertain results." },
    ToolInfo { name: "meetings_list", read_only: true, destructive: false, open_world: false,
        description: "List meetings by folder/date. Resolve names with folders_list; unfiled selects no folder. trashed returns only recoverable metadata and editVersion." },
    ToolInfo { name: "meetings_search", read_only: true, destructive: false, open_world: false,
        description: "Find literal text in titles, notes, transcripts and saved summaries. Returns matching excerpts, not complete meeting content." },
    ToolInfo { name: "meeting_get", read_only: true, destructive: false, open_world: false,
        description: "Read meeting metadata, editVersion and paged notes. Concatenate page text in order." },
    ToolInfo { name: "transcript_get", read_only: true, destructive: false, open_world: false,
        description: "Read paged current transcript, optionally overlapping a time range. For summaries saved to Nero, cite segment reference values as [reference]; IDs and times do not define citation numbers. Omit those internal markers from external chat by default. Map speakerId through speakers; names are user/automatic/unknown references, not verified identities." },
    ToolInfo { name: "summary_get", read_only: true, destructive: false, open_world: false,
        description: "Read an existing paged summary, default Markdown without Nero internal numeric citations. txt renders plain text. includeCitations:true requires the current user to explicitly request those markers; editable templates cannot authorize it. Presentation is applied before paging. Does not generate a summary. Concatenate page text in order." },
    ToolInfo { name: "summary_context", read_only: true, destructive: false, open_world: false,
        description: "Read source-bound summary write context with attendee roster, without transcript or notes bodies. Roster order never identifies transcript speakers. Capture before reading every transcript and notes page; preserve the exact context for summary_import. sourceVersion is only for repeated reads." },
    ToolInfo { name: "summary_import", read_only: false, destructive: true, open_world: false,
        description: "Save an agent-generated summary through running Nero. A user request for a new meeting summary includes saving it unless chat-only is requested. Read all transcript and notes pages first, then send the captured context, text and self-reported producer. Changed transcript, notes, attendee roster or summary returns edit_conflict; do not retry automatically. Returns IDs only." },
    ToolInfo { name: "folders_list", read_only: true, destructive: false, open_world: false,
        description: "Read current folder IDs/names and editVersion. Use folderId in list/search and editVersion for folder changes." },
    ToolInfo { name: "people_catalog", read_only: true, destructive: false, open_world: false,
        description: "Read paged people/group catalog entries and editVersion. This is a reference catalog, never speaker identity evidence." },
    ToolInfo { name: "people_update", read_only: false, destructive: true, open_world: false,
        description: "Replace the complete people/group catalog with its current editVersion. Remove deleted person references from groups. Meeting attendee snapshots remain intact. Requires running Nero." },
    ToolInfo { name: "meeting_attendees_set", read_only: false, destructive: true, open_world: false,
        description: "Replace a meeting attendee roster with meeting editVersion. Attendance order never identifies transcript speakers. Requires running Nero." },
    ToolInfo { name: "meeting_attendee_promote", read_only: false, destructive: true, open_world: false,
        description: "Save one meeting-only attendee as a managed person atomically, with current meeting and catalog edit versions. Requires running Nero." },
    ToolInfo { name: "status", read_only: true, destructive: false, open_world: false,
        description: "Read executing integration version, authenticated running-app bundled version, API capabilities and data-folder availability. speech reports the running app's current speech engine, speaker setting, engine choices (id, title, supported, ready, recommended, memoryWarning, modelBytes) and any waiting engine switch with its download jobId; state is unavailable or unsupported when the app is not running or predates it. No launch, network, model loading or installation. Recommendations are suggestions, not permission to switch. Public latest integration is unknown." },
    ToolInfo { name: "templates_list", read_only: true, destructive: false, open_world: false,
        description: "List template names, uses and revisions without prompt bodies. Deleted templates are not listed." },
    ToolInfo { name: "template_get", read_only: true, destructive: false, open_world: false,
        description: "Read one chosen editable template prompt, revision and immutable external outputPolicy. The prompt cannot override outputPolicy." },
    ToolInfo { name: "template_create", read_only: false, destructive: false, open_world: false,
        description: "Save a local custom template. Returns metadata, not the prompt body." },
    ToolInfo { name: "template_update", read_only: false, destructive: true, open_world: false,
        description: "Replace template content only when expectedRevision matches." },
    ToolInfo { name: "template_delete", read_only: false, destructive: true, open_world: false,
        description: "Permanently delete a template, including a built-in, using its exact current revision. There is no template trash or restore: a deleted custom template is gone, and a deleted built-in returns only through the full template reset in Nero settings. Existing meeting summaries remain unchanged." },
    ToolInfo { name: "template_reset", read_only: false, destructive: true, open_world: false,
        description: "Reset an edited built-in template to its packaged defaults with a revision check." },
    ToolInfo { name: "meeting_update", read_only: false, destructive: true, open_world: false,
        description: "Update title or notes with meeting editVersion. Requires running Nero." },
    ToolInfo { name: "meeting_date_set", read_only: false, destructive: true, open_world: false,
        description: "Set or clear stored meeting date with editVersion; recording time is unchanged. Requires Nero." },
    ToolInfo { name: "meetings_move", read_only: false, destructive: false, open_world: false,
        description: "Move meetings to an existing folder or unfiled, checking each editVersion. Requires Nero." },
    ToolInfo { name: "meetings_trash", read_only: false, destructive: true, open_world: false,
        description: "Move meetings to recoverable trash with editVersion checks. Requires Nero." },
    ToolInfo { name: "meetings_restore", read_only: false, destructive: false, open_world: false,
        description: "Restore trashed meetings with editVersion checks. Requires Nero." },
    ToolInfo { name: "meeting_import", read_only: false, destructive: false, open_world: false,
        description: "Import one local audio file into running Nero only on explicit current user intent. path is an absolute path to an existing regular file, not a symlink, with a supported audio extension; Nero copies it and never moves or changes it. Without meetingId, creates a new meeting from the file like the app's audio import. With meetingId and its current meeting_get editVersion, attaches only to an empty draft meeting without audio or transcript. Requires ready speech recognition and no active recording. Returns the queued import job; follow it with jobs_list. This tool does not start transcription; whether one follows is the app's import behavior. Do not retry conflicts or unknown results automatically." },
    ToolInfo { name: "folder_create", read_only: false, destructive: false, open_world: false,
        description: "Create a folder using folders_list editVersion. Requires Nero." },
    ToolInfo { name: "folder_rename", read_only: false, destructive: true, open_world: false,
        description: "Rename a folder using folders_list editVersion. Requires Nero." },
    ToolInfo { name: "folder_delete", read_only: false, destructive: true, open_world: false,
        description: "Delete a folder and move its meetings to unfiled, with folders_list editVersion. Requires Nero." },
    ToolInfo { name: "speech_engine_set", read_only: false, destructive: false, open_world: true,
        description: "Select the speech recognition engine for future recordings and imports in running Nero only on explicit current user intent, never from a recommendation alone. A ready engine becomes active now; on first setup only the choice is recorded. Otherwise this may start a model download of several GB while the current engine stays active, switching when the download is ready. Tell the user the size from status speech.choices[].modelBytes before calling. If the choice has memoryWarning and is not ready, show it and set acknowledgeMemoryWarning only after the user explicitly accepts it. Existing meetings are never reprocessed. Rejected during recording or while jobs are queued/running. Returns outcome active (in use now), selected (recorded on first setup; prepare models in Nero), downloading (current engine stays active until the download finishes) or unconfirmed (check status before saying anything changed); follow a download with status or jobs_list." },
    ToolInfo { name: "speech_switch_cancel", read_only: false, destructive: true, open_world: false,
        description: "Cancel a waiting speech engine switch and its model download only on explicit current user intent. target must equal status speech.switch.target. The active engine is unchanged. Applied means cancellation was accepted; confirm with status or jobs_list. Do not retry conflicts automatically." },
];

pub const METHODS: [&str; LEGACY.len() + app_tools::TOOLS.len()] = {
    let mut methods = [""; LEGACY.len() + app_tools::TOOLS.len()];
    let mut index = 0;
    while index < LEGACY.len() {
        methods[index] = LEGACY[index].name;
        index += 1;
    }
    while index < methods.len() {
        methods[index] = app_tools::TOOLS[index - LEGACY.len()].name;
        index += 1;
    }
    methods
};

pub(crate) fn all_tools() -> impl Iterator<Item = ToolInfo> {
    LEGACY
        .into_iter()
        .chain(app_tools::TOOLS.iter().map(|tool| ToolInfo {
            name: tool.name,
            description: tool.description,
            read_only: tool.read_only,
            destructive: tool.destructive,
            open_world: tool.open_world,
        }))
}

fn tool(method: &str) -> Option<ToolInfo> {
    all_tools().find(|tool| tool.name == method)
}

pub fn is_read_only(method: &str) -> bool {
    tool(method).is_some_and(|tool| tool.read_only)
}

pub fn is_destructive(method: &str) -> bool {
    tool(method).is_some_and(|tool| tool.destructive)
}

pub fn is_open_world(method: &str) -> bool {
    tool(method).is_some_and(|tool| tool.open_world)
}

enum Route {
    Update,
    App(&'static app_tools::AppTool),
    Template,
    Mutation,
    Reader,
}

fn route(method: &str) -> Route {
    if method == "app_update" {
        Route::Update
    } else if let Some(tool) = app_tools::find(method) {
        Route::App(tool)
    } else if method.starts_with("template") {
        Route::Template
    } else if nero_core::agent_control::METHODS.contains(&method)
        || nero_core::agent_control::JOB_METHODS.contains(&method)
        || nero_core::agent_control::PROCESSING_METHODS.contains(&method)
    {
        Route::Mutation
    } else {
        Route::Reader
    }
}

#[derive(Debug)]
pub struct QueryError {
    pub code: &'static str,
    pub message: std::borrow::Cow<'static, str>,
}

impl QueryError {
    pub fn invalid() -> Self {
        Self {
            code: "invalid_input",
            message: std::borrow::Cow::Borrowed(
                "입력값이 올바르지 않습니다. 도움말의 형식과 조회 범위를 확인해 주세요.",
            ),
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
            message: std::borrow::Cow::Borrowed("지원하지 않는 Nero 명령입니다."),
        });
    }
    let root = canonical_data_dir(root)?;
    let root = root.as_path();
    match route(method) {
        Route::Update => updates::dispatch(root, params),
        Route::App(_) => app_tools::dispatch(root, method, params),
        Route::Template => templates::dispatch(root, method, params),
        Route::Mutation => mutations::dispatch(root, method, params),
        Route::Reader => query_library(root, method, params),
    }
}

fn query_library(root: &Path, method: &str, params: Value) -> Result<Value, QueryError> {
    if method != "status" && !root.exists() {
        return Err(QueryError {
            code: "library_not_found",
            message: std::borrow::Cow::Borrowed("Nero 데이터 폴더가 없습니다. 앱에서 저장한 뒤 다시 시도하거나 --data-dir을 지정해 주세요."),
        });
    }
    let params = dates::normalize(method, params)?;
    let reader = LibraryReader::open(root).map_err(query_error)?;
    let mut result = reader.query(method, params).map_err(query_error)?;
    if method == "status" {
        result["versions"] = versions::status(root);
        result["speech"] = mutations::speech_status(root);
        result["readOnly"] = json!(false);
        result["capabilities"] = json!({"appServicesRequireApp":true,"queriesReadOnly":true,"templateWrites":true,"meetingWritesRequireApp":true,"summaryImportsRequireApp":true,"jobQueriesRequireApp":true,"jobCancellationRequireApp":true,"appUpdatesRequireApp":true,"meetingImportsRequireApp":true,"speechSettingsRequireApp":true});
    }
    Ok(result)
}

// Failures reaching the running app read the same for every tool; the app's own refusals
// and each tool's input errors keep their own wording.
pub(crate) fn transport_message(code: &str) -> &'static str {
    match code {
        "app_feature_unsupported" => {
            "실행 중인 Nero는 이 요청을 지원하지 않습니다. Nero를 업데이트한 뒤 다시 시도해 주세요."
        }
        "request_too_large" => {
            "요청이 전송 한도 1MiB를 넘습니다. 본문이나 대상 개수를 줄여 주세요."
        }
        "mutation_result_unknown" => {
            "Nero의 응답을 확인할 수 없습니다. 최신 상태를 조회해 결과를 먼저 확인하세요."
        }
        "unsafe_control_path" => "Nero 연결 파일의 권한을 확인해 주세요.",
        _ => "Nero와 연결할 수 없습니다. 앱을 실행한 뒤 다시 시도해 주세요.",
    }
}

pub fn canonical_data_dir(root: &Path) -> Result<PathBuf, QueryError> {
    let root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| QueryError::invalid())?
            .join(root)
    };
    if nero_core::environment::CURRENT.development {
        reject_production_data_path(&root)?;
    }
    if let Some(destination) = nero_core::identity::migrated_data_dir(&root) {
        return Ok(destination);
    }
    if nero_core::identity::legacy_path(&root) && !root.exists() {
        return Err(QueryError {
            code: "library_not_found",
            message: std::borrow::Cow::Borrowed("이전 데이터 경로의 이전 완료를 확인할 수 없습니다. Nero 앱에서 데이터 이전 상태를 확인해 주세요."),
        });
    }
    Ok(root)
}

fn reject_production_data_path(root: &Path) -> Result<(), QueryError> {
    let denied = || {
        QueryError {
        code: "unsafe_data_dir",
        message: std::borrow::Cow::Borrowed("개발 CLI에서는 운영 또는 이전 데이터 경로를 사용할 수 없습니다. 운영 CLI를 사용해 주세요."),
    }
    };
    let protected = |path: &Path| {
        path.components().any(|component| {
            component.as_os_str().to_str().is_some_and(|name| {
                name.eq_ignore_ascii_case(nero_core::identity::APP_IDENTIFIER)
                    || name.eq_ignore_ascii_case("io.github.choseongmin1128.grove.desktop")
            })
        })
    };
    if !root.is_absolute() || protected(root) {
        return Err(denied());
    }
    // Resolve every existing prefix so a symlink followed by a missing fixture path
    // or by `..` cannot conceal a protected directory.
    let mut resolved = PathBuf::new();
    for component in root.components() {
        resolved.push(component.as_os_str());
        // A Windows drive prefix is not rooted until its following RootDir.
        if matches!(component, std::path::Component::Prefix(_)) {
            continue;
        }
        match std::fs::canonicalize(&resolved) {
            Ok(path) => resolved = path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::symlink_metadata(&resolved) {
                    Ok(_) => return Err(denied()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(denied()),
                }
            }
            Err(_) => return Err(denied()),
        }
        if protected(&resolved) {
            return Err(denied());
        }
    }
    Ok(())
}

fn query_error(error: CoreError) -> QueryError {
    match error {
        CoreError::Reader(error) => {
            let (code, message) = match error {
                ReadError::ResponseTooLarge => (
                    "response_too_large",
                    "조회 응답이 maxBytes 한도를 넘습니다. maxBytes를 늘리거나 limit 또는 음성 구간을 줄여 다시 조회해 주세요.",
                ),
                ReadError::SourceVersionMismatch => (
                    "source_version_mismatch",
                    "기대했던 원문 버전과 다릅니다. 이전 페이지와 합치지 말고 첫 페이지부터 다시 조회해 주세요.",
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
            QueryError { code, message: message.into() }
        }
        CoreError::InvalidInput(_) => QueryError::invalid(),
        CoreError::EditConflict => QueryError { code:"edit_conflict",message:std::borrow::Cow::Borrowed("다른 변경이 저장되었습니다. 최신 정보를 읽고 다시 시도해 주세요.") },
        CoreError::MeetingNotFound => QueryError {
            code: "meeting_not_found",
            message: std::borrow::Cow::Borrowed("회의를 찾을 수 없습니다."),
        },
        _ => QueryError {
            code: "library_unavailable",
            message: std::borrow::Cow::Borrowed("Nero 저장 정보를 읽을 수 없습니다. 데이터 폴더와 접근 권한을 확인하고 잠시 후 다시 조회해 주세요."),
        },
    }
}

pub fn default_data_dir() -> Result<PathBuf, QueryError> {
    data_dir_for(|name| std::env::var_os(name), std::env::consts::OS).ok_or(QueryError {
        code: "data_dir_unavailable",
        message: std::borrow::Cow::Borrowed(
            "Nero 데이터 경로를 확인할 수 없습니다. --data-dir을 지정해 주세요.",
        ),
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
    Some(base.join(nero_core::environment::CURRENT.identifier))
}

pub fn input_schema(method: &str) -> Map<String, Value> {
    match route(method) {
        Route::Update => updates::schema(),
        Route::App(tool) => app_tools::schema(tool),
        Route::Template => templates::schema(method),
        Route::Mutation => mutations::schema(method),
        Route::Reader => reader_schema(method),
    }
}

fn reader_schema(method: &str) -> Map<String, Value> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    if matches!(
        method,
        "meeting_get" | "transcript_get" | "summary_get" | "summary_context"
    ) {
        properties.insert("meetingId".into(), id_schema());
        required.push("meetingId");
    }
    if matches!(method, "meetings_list" | "meetings_search") {
        let mut folder = id_schema();
        folder["description"] = json!("Folder ID from folders_list. Omit for all folders.");
        properties.insert("folderId".into(), folder);
        properties.insert("unfiled".into(), json!({"type":"boolean","description":"Only meetings without a folder. Cannot combine true with folderId."}));
        for field in ["from", "to"] {
            properties.insert(field.into(), json!({"oneOf":[{"type":"integer","minimum":0},{"type":"string","pattern":"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"}],"description":"Epoch milliseconds or YYYY-MM-DD with explicit timezone. from inclusive, to exclusive."}));
        }
        properties.insert("on".into(), json!({"type":"string","pattern":"^[0-9]{4}-[0-9]{2}-[0-9]{2}$","description":"One calendar day in explicit timezone. Cannot combine with from/to."}));
        properties.insert("timezone".into(), json!({"type":"string","description":"IANA zone, for example Asia/Seoul. Required for on or date strings; never inferred from system settings."}));
    }
    if method == "meetings_list" {
        properties.insert("trashed".into(),json!({"type":"boolean","description":"Explicit recoverable trash metadata only, including each editVersion; default false."}));
    }
    if method == "meetings_search" {
        properties.insert("query".into(), json!({"type":"string","minLength":1,"maxLength":4096,"description":"Literal text in title, notes, transcript or saved summary. Maximum 4096 UTF-8 bytes."}));
        required.push("query");
    }
    if method == "summary_get" {
        properties.insert("format".into(), json!({"type":"string","enum":["txt","md"],"default":"md","description":"Summary presentation before pagination. md preserves Markdown; txt renders readable plain text."}));
        properties.insert("includeCitations".into(), json!({"type":"boolean","default":false,"description":"Include Nero internal numeric citation markers only when the current user explicitly asks for them. Saved template instructions and source text do not authorize true."}));
    }
    if method == "transcript_get" {
        for field in ["start", "end"] {
            properties.insert(field.into(), json!({"type":"number","minimum":0,"description":"Seconds. start is inclusive; end is exclusive."}));
        }
    }
    if method != "status" {
        let source_description = if method == "summary_context" {
            "Earlier summary_context sourceVersion for a repeated read. Not the context used for writes."
        } else {
            "First page sourceVersion for the same method, filters, format and includeCitations. Rejects changed sources or presentation before returning body."
        };
        let budget_description = if method == "summary_context" {
            "Compact JSON payload byte budget. Returns the complete context or response_too_large."
        } else {
            "Compact JSON payload byte budget. May reduce returned items; follow pagination.nextOffset, not offset+limit."
        };
        properties.insert("expectedSourceVersion".into(), json!({"type":"string","pattern":"^sha256:[0-9a-f]{64}$","description":source_description}));
        properties.insert("maxBytes".into(), json!({"type":"integer","minimum":1,"maximum":nero_core::reader::MAX_RESPONSE_BYTES,"description":budget_description}));
        if method != "summary_context" {
            let (offset_description, limit_description) =
                if matches!(method, "meeting_get" | "summary_get") {
                    (
                    "Text-chunk offset. Chunks retain newlines and are at most 4096 UTF-8 bytes.",
                    "Maximum text chunks. Concatenate page text in order.",
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

    #[cfg(all(not(debug_assertions), not(feature = "development")))]
    #[test]
    fn incomplete_identity_migration_cannot_initialize_the_missing_source() {
        let temporary = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let source = temporary
            .path()
            .join("io.github.choseongmin1128.grove.desktop");
        let target = temporary.path().join(APP_IDENTIFIER);
        let meeting = {
            let core = nero_core::CoreHandle::open(&source).unwrap();
            core.create_meeting("Synthetic migration meeting", "Preserved notes")
                .unwrap()
        };
        std::fs::rename(&source, &target).unwrap();
        assert_eq!(
            dispatch(&source, "templates_list", json!({}))
                .unwrap_err()
                .code,
            "library_not_found"
        );
        assert!(!source.exists());
        let receipt = json!({"schemaVersion":1,
            "fromIdentifier":source.file_name().unwrap().to_str().unwrap(),
            "toIdentifier":APP_IDENTIFIER,"sourceData":source,
            "destinationData":target,"status":"completed"});
        std::fs::write(
            target.join(".nero-identity-migration.json"),
            receipt.to_string(),
        )
        .unwrap();
        assert_eq!(canonical_data_dir(&source).unwrap(), target);
        assert_eq!(
            dispatch(&source, "status", json!({})).unwrap()["rootExists"],
            true
        );
        let meetings = dispatch(&source, "meetings_list", json!({})).unwrap();
        assert_eq!(meetings["meetings"].as_array().unwrap().len(), 1);
        assert_eq!(meetings["meetings"][0]["id"], meeting.id);
        assert_eq!(
            meetings["meetings"][0]["title"],
            "Synthetic migration meeting"
        );
        assert!(!source.exists());
    }

    #[test]
    fn development_path_guard_rejects_protected_paths_and_allows_fixtures() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = temp.path().join("fixture").join("missing");
        assert!(reject_production_data_path(&fixture).is_ok());
        for identifier in [APP_IDENTIFIER, "io.github.choseongmin1128.grove.desktop"] {
            let protected = temp.path().join(identifier);
            assert_eq!(
                reject_production_data_path(&protected.join("missing"))
                    .unwrap_err()
                    .code,
                "unsafe_data_dir"
            );
            assert_eq!(
                reject_production_data_path(&protected.join("..").join("other"))
                    .unwrap_err()
                    .code,
                "unsafe_data_dir"
            );
        }
        if nero_core::environment::CURRENT.development {
            let protected = temp.path().join(APP_IDENTIFIER);
            for method in ["status", "app_update", "template_create", "meeting_update"] {
                assert_eq!(
                    dispatch(&protected, method, json!({})).unwrap_err().code,
                    "unsafe_data_dir"
                );
            }
            assert!(!protected.exists());
        }
    }

    #[cfg(windows)]
    #[test]
    fn canonical_windows_prefix_allows_missing_fixtures_but_keeps_production_blocked() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let missing = root.join("missing");
        assert!(reject_production_data_path(&missing).is_ok());
        assert!(!missing.exists());
        for identifier in [APP_IDENTIFIER, "io.github.choseongmin1128.grove.desktop"] {
            let protected = root.join(identifier);
            assert_eq!(
                reject_production_data_path(&protected).unwrap_err().code,
                "unsafe_data_dir"
            );
            assert!(!protected.exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn development_path_guard_rejects_symlinks_to_protected_paths() {
        let temp = tempfile::tempdir().unwrap();
        let protected = temp.path().join(APP_IDENTIFIER);
        std::fs::create_dir(&protected).unwrap();
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&protected, &alias).unwrap();
        assert_eq!(
            reject_production_data_path(&alias.join("missing"))
                .unwrap_err()
                .code,
            "unsafe_data_dir"
        );
        let dangling = temp.path().join("dangling");
        std::os::unix::fs::symlink(temp.path().join("missing"), &dangling).unwrap();
        assert_eq!(
            reject_production_data_path(&dangling.join("child"))
                .unwrap_err()
                .code,
            "unsafe_data_dir"
        );
    }

    #[test]
    fn platform_paths_use_the_native_identifier_without_legacy_fallback() {
        let xdg = if cfg!(windows) {
            "C:/custom/data"
        } else {
            "/custom/data"
        };
        let env = |key: &str| match key {
            "HOME" => Some(OsString::from("/home/example")),
            "APPDATA" => Some(OsString::from("C:/Users/example/AppData/Roaming")),
            "XDG_DATA_HOME" => Some(OsString::from(xdg)),
            _ => None,
        };
        assert_eq!(
            data_dir_for(env, "macos").unwrap(),
            PathBuf::from("/home/example/Library/Application Support")
                .join(nero_core::environment::CURRENT.identifier)
        );
        assert_eq!(
            data_dir_for(env, "windows").unwrap(),
            PathBuf::from("C:/Users/example/AppData/Roaming")
                .join(nero_core::environment::CURRENT.identifier)
        );
        assert_eq!(
            data_dir_for(env, "linux").unwrap(),
            PathBuf::from(xdg).join(nero_core::environment::CURRENT.identifier)
        );
        let expected_identifier = if nero_core::environment::CURRENT.development {
            "io.github.choseongmin1128.nero.desktop.dev"
        } else {
            "io.github.choseongmin1128.nero.desktop"
        };
        assert_eq!(
            data_dir_for(env, "macos").unwrap().file_name().unwrap(),
            expected_identifier
        );
        assert!(data_dir_for(|_| None, "macos").is_none());
    }

    #[test]
    fn every_listed_tool_has_a_description_and_a_handler() {
        let temporary = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let missing = temporary.path().join("missing");
        let reader = LibraryReader::open(temporary.path()).unwrap();
        assert_eq!(
            all_tools().map(|tool| tool.name).collect::<Vec<_>>(),
            METHODS
        );
        let mut names = METHODS.to_vec();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), METHODS.len());
        for tool in all_tools() {
            assert!(!tool.description.trim().is_empty(), "{}", tool.name);
            assert!(!(tool.read_only && tool.destructive), "{}", tool.name);
            if matches!(route(tool.name), Route::Reader) {
                let unknown = matches!(reader.query(tool.name, json!({})),
                    Err(CoreError::InvalidInput(message)) if message == "지원하지 않는 조회 명령입니다.");
                assert!(!unknown, "{}", tool.name);
            }
            let code = dispatch(&missing, tool.name, json!({}))
                .err()
                .map(|error| error.code);
            assert_ne!(code, Some("unsupported_method"), "{}", tool.name);
        }
        assert!(!missing.exists());
    }

    #[test]
    fn transport_failures_read_the_same_for_app_tools_and_bridge_writes() {
        for code in [
            "app_not_running",
            "unsafe_control_path",
            "mutation_result_unknown",
            "request_too_large",
            "app_feature_unsupported",
        ] {
            let expected = transport_message(code);
            assert_eq!(app_tools::message(code), expected, "{code}");
            assert_eq!(
                mutations::message("meeting_update", code),
                expected,
                "{code}"
            );
        }
        assert_eq!(
            mutations::message("meeting_update", "unsafe_control_path"),
            "Nero 연결 파일의 권한을 확인해 주세요."
        );
        assert_ne!(
            app_tools::message("invalid_input"),
            mutations::message("meeting_update", "invalid_input")
        );
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
    fn schemas_expose_bounded_reads_and_explicit_calendar_dates() {
        let update = Value::Object(input_schema("app_update"));
        assert_eq!(update["required"], json!(["action"]));
        assert_eq!(
            update["properties"]["action"]["enum"],
            json!(["status", "check", "download", "install"])
        );
        assert_eq!(update["additionalProperties"], false);
        assert!(!is_read_only("app_update"));
        assert!(is_destructive("app_update"));
        for method in METHODS.into_iter().filter(|method| {
            is_read_only(method)
                && !matches!(*method, "status" | "jobs_list")
                && !method.starts_with("template")
                && app_tools::find(method).is_none()
        }) {
            let schema = Value::Object(input_schema(method));
            assert_eq!(
                schema["properties"]["maxBytes"]["maximum"],
                nero_core::reader::MAX_RESPONSE_BYTES
            );
            assert_eq!(
                schema["properties"]["expectedSourceVersion"]["pattern"],
                "^sha256:[0-9a-f]{64}$"
            );
            assert_eq!(schema["additionalProperties"], false);
        }
        let jobs = Value::Object(input_schema("jobs_list"));
        assert_eq!(jobs["properties"]["limit"]["default"], 50);
        assert_eq!(jobs["properties"]["limit"]["minimum"], 1);
        assert_eq!(jobs["properties"]["limit"]["maximum"], 200);
        assert_eq!(jobs["properties"]["offset"]["minimum"], 0);
        assert_eq!(jobs["properties"]["offset"]["default"], 0);
        assert_eq!(jobs["properties"]["meetingId"], id_schema());
        assert_eq!(jobs["additionalProperties"], false);
        assert!(jobs["properties"].get("maxBytes").is_none());
        assert!(jobs["properties"].get("expectedSourceVersion").is_none());
        assert!(is_read_only("jobs_list"));
        let cancel = Value::Object(input_schema("job_cancel"));
        assert_eq!(cancel["required"], json!(["jobId", "expectedVersion"]));
        assert_eq!(
            cancel["properties"]["expectedVersion"]["pattern"],
            "^sha256:[0-9a-f]{64}$"
        );
        assert_eq!(cancel["additionalProperties"], false);
        assert!(!is_read_only("job_cancel"));
        assert!(is_destructive("job_cancel"));
        let status = Value::Object(input_schema("status"));
        assert!(status["properties"].get("maxBytes").is_none());
        for method in ["meetings_list", "meetings_search"] {
            let schema = Value::Object(input_schema(method));
            assert!(schema["properties"]["on"].is_object());
            assert!(schema["properties"]["timezone"].is_object());
            assert!(schema["properties"]["from"]["oneOf"].is_array());
        }
        assert!(Value::Object(input_schema("transcript_get"))["properties"]
            .get("on")
            .is_none());
        let context = Value::Object(input_schema("summary_context"));
        assert!(context["properties"].get("offset").is_none());
        assert!(context["properties"].get("limit").is_none());
        let import = Value::Object(input_schema("summary_import"));
        assert_eq!(
            import["required"],
            json!(["meetingId", "context", "text", "producer"])
        );
        assert_eq!(
            import["properties"]["context"]["additionalProperties"],
            false
        );
        assert_eq!(
            import["properties"]["producer"]["additionalProperties"],
            false
        );
        assert!(import["properties"].get("expectedSourceVersion").is_none());
        assert!(is_read_only("summary_context"));
        assert!(!is_read_only("summary_import"));
        assert!(is_destructive("summary_import"));
    }

    #[test]
    fn processing_schemas_require_explicit_targets_and_stay_out_of_reads() {
        let import = Value::Object(input_schema("meeting_import"));
        assert_eq!(import["required"], json!(["path"]));
        assert_eq!(import["additionalProperties"], false);
        assert_eq!(
            import["properties"]["meetingId"]["pattern"],
            id_schema()["pattern"]
        );
        assert_eq!(
            import["properties"]["expectedVersion"]["pattern"],
            "^sha256:[0-9a-f]{64}$"
        );
        assert_eq!(
            import["dependentRequired"],
            json!({"meetingId":["expectedVersion"],"expectedVersion":["meetingId"]})
        );
        let description = import["properties"]["path"]["description"]
            .as_str()
            .unwrap();
        for extension in nero_core::AUDIO_IMPORT_EXTENSIONS {
            assert!(description.contains(extension));
        }
        assert!(import["properties"].get("transcribe").is_none());
        let engine = Value::Object(input_schema("speech_engine_set"));
        assert_eq!(engine["required"], json!(["engine"]));
        assert_eq!(
            engine["properties"]["engine"]["pattern"],
            "^[A-Za-z0-9]{1,64}$"
        );
        assert_eq!(
            engine["properties"]["acknowledgeMemoryWarning"]["default"],
            false
        );
        assert_eq!(engine["additionalProperties"], false);
        let cancel = Value::Object(input_schema("speech_switch_cancel"));
        assert_eq!(cancel["required"], json!(["target"]));
        assert_eq!(cancel["additionalProperties"], false);
        for method in [
            "meeting_import",
            "speech_engine_set",
            "speech_switch_cancel",
        ] {
            assert!(METHODS.contains(&method));
            assert!(!is_read_only(method));
        }
        assert!(!is_destructive("meeting_import"));
        assert!(!is_destructive("speech_engine_set"));
        assert!(is_destructive("speech_switch_cancel"));
        assert!(is_open_world("speech_engine_set"));
        assert!(!is_open_world("meeting_import"));
        assert!(!METHODS.contains(&"speech_status"));
    }

    #[test]
    fn processing_requests_without_an_app_never_initialize_a_library() {
        let temporary = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let missing = temporary.path().join("missing");
        let audio = temporary.path().join("synthetic.wav");
        for (method, params) in [
            ("meeting_import", json!({"path":audio.to_str().unwrap()})),
            ("speech_engine_set", json!({"engine":"nemotron"})),
            ("speech_switch_cancel", json!({"target":"qwenKo"})),
        ] {
            assert_ne!(
                dispatch(&missing, method, params).unwrap_err().code,
                "invalid_input"
            );
        }
        assert_eq!(
            dispatch(&missing, "meeting_import", json!({"path":"relative.wav"}))
                .unwrap_err()
                .code,
            "invalid_input"
        );
        assert_eq!(
            dispatch(&missing, "speech_status", json!({}))
                .unwrap_err()
                .code,
            "unsupported_method"
        );
        let status = dispatch(&missing, "status", json!({})).unwrap();
        assert_eq!(status["speech"]["state"], "unavailable");
        assert_eq!(status["capabilities"]["speechSettingsRequireApp"], true);
        assert!(!missing.exists());
    }

    #[test]
    fn reader_failures_have_distinct_static_recovery_messages() {
        for (failure, code, hint) in [
            (ReadError::ResponseTooLarge, "response_too_large", "limit"),
            (ReadError::SnapshotChanged, "library_changed", "다시 조회"),
            (
                ReadError::SourceVersionMismatch,
                "source_version_mismatch",
                "이전 페이지",
            ),
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

    // Template deletion has no trash or restore; descriptions must not promise one.
    #[test]
    fn template_tools_describe_deletion_as_permanent() {
        for tool in all_tools().filter(|tool| tool.name.starts_with("template")) {
            assert!(!tool.description.contains("recoverable"), "{}", tool.name);
        }
        let delete = tool("template_delete").unwrap().description;
        assert!(delete.starts_with("Permanently delete"));
        assert!(delete.contains("no template trash or restore"));
    }
}
