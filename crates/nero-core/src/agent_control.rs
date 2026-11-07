use crate::agent_app::{self, AppRequest};
use crate::engine::Engine;
use crate::{store, CoreError, CoreHandle, ExternalSummaryProducer, Result, SummaryContext};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const PROTOCOL: u32 = 1;
const MAX_FRAME: usize = 1024 * 1024;
const DESCRIPTOR: &str = "agent-control.json";
// Requests answered at once; more wait for a free slot.
const MAX_CONNECTIONS: usize = 8;
const RESPONSE_TOO_LARGE: &str =
    "Nero의 응답이 연결의 전송 한도(1MiB)를 넘었습니다. 조회 범위를 줄여 다시 요청해 주세요.";
const UNKNOWN_RESULT: &str =
    "수정 결과를 확인하지 못했습니다. 같은 요청을 반복하기 전에 현재 상태를 조회해 주세요.";

#[derive(Debug, Clone, Serialize)]
pub struct ControlError {
    pub code: &'static str,
    pub message: String,
}

impl ControlError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn unavailable() -> Self {
        Self::new("app_not_running", "Nero 앱을 실행한 뒤 다시 시도해 주세요.")
    }
    pub fn invalid() -> Self {
        Self::new("invalid_input", "수정 요청의 형식이 올바르지 않습니다.")
    }
    fn unknown_result() -> Self {
        Self::new("mutation_result_unknown", UNKNOWN_RESULT)
    }
    pub fn core(error: CoreError) -> Self {
        let code = match &error {
            CoreError::EditConflict => "edit_conflict",
            CoreError::MeetingNotFound => "meeting_not_found",
            CoreError::InvalidInput(_) => "invalid_input",
            CoreError::RecordingBusy | CoreError::StaleSession | CoreError::InvalidState(_) => {
                "invalid_state"
            }
            CoreError::Unavailable | CoreError::StoreBusy => "app_not_ready",
            _ => "storage_error",
        };
        let message = match error {
            CoreError::RecordingBusy | CoreError::StaleSession => {
                "녹음을 마친 뒤 다시 시도해 주세요."
            }
            _ => core_code_message(code).map_or("", |(_, message)| message),
        };
        Self::new(code, message)
    }
}

// Messages for the codes a core error maps to, shared by the app's reply and the
// CLI client that reads it.
fn core_code_message(code: &str) -> Option<(&'static str, &'static str)> {
    Some(match code {
        "edit_conflict" => (
            "edit_conflict",
            "다른 변경이 저장되었습니다. 최신 정보를 읽고 다시 시도해 주세요.",
        ),
        "meeting_not_found" => ("meeting_not_found", "회의를 찾을 수 없습니다."),
        "invalid_input" => (
            "invalid_input",
            "수정 요청의 값이나 대상이 올바르지 않습니다.",
        ),
        "invalid_state" => (
            "invalid_state",
            "회의 또는 처리 상태가 바뀌어 수정하지 못했습니다. 현재 상태를 확인해 주세요.",
        ),
        "app_not_ready" => (
            "app_not_ready",
            "Nero 앱에서 저장 상태를 확인한 뒤 다시 시도해 주세요.",
        ),
        "storage_error" => (
            "storage_error",
            "변경을 저장하지 못했습니다. Nero 앱에서 저장 상태를 확인해 주세요.",
        ),
        _ => return None,
    })
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "method", content = "params", deny_unknown_fields)]
pub enum Mutation {
    #[serde(rename = "summary_import")]
    SummaryImport {
        #[serde(rename = "meetingId")]
        meeting_id: String,
        context: SummaryContext,
        text: String,
        producer: ExternalSummaryProducer,
    },
    #[serde(rename = "people_update")]
    PeopleUpdate {
        #[serde(rename = "expectedVersion")]
        expected_version: String,
        people: Vec<crate::Person>,
        groups: Vec<crate::PersonGroup>,
    },
    #[serde(rename = "meeting_attendees_set")]
    MeetingAttendeesSet {
        #[serde(rename = "meetingId")]
        meeting_id: String,
        #[serde(rename = "expectedVersion")]
        expected_version: String,
        attendees: Vec<crate::MeetingAttendee>,
    },
    #[serde(rename = "meeting_attendee_promote")]
    MeetingAttendeePromote {
        #[serde(rename = "meetingId")]
        meeting_id: String,
        #[serde(rename = "expectedVersion")]
        expected_version: String,
        #[serde(rename = "expectedCatalogVersion")]
        expected_catalog_version: String,
        #[serde(rename = "attendeeId")]
        attendee_id: String,
        name: String,
    },
    #[serde(rename = "meeting_update")]
    MeetingUpdate {
        #[serde(rename = "meetingId")]
        meeting_id: String,
        #[serde(rename = "expectedVersion")]
        expected_version: String,
        title: Option<String>,
        notes: Option<String>,
    },
    #[serde(rename = "meeting_date_set")]
    MeetingDateSet {
        #[serde(rename = "meetingId")]
        meeting_id: String,
        #[serde(rename = "expectedVersion")]
        expected_version: String,
        #[serde(rename = "meetingDate")]
        meeting_date: Option<String>,
    },
    #[serde(rename = "meetings_move")]
    MeetingsMove {
        #[serde(rename = "expectedVersions")]
        expected_versions: BTreeMap<String, String>,
        #[serde(rename = "folderId")]
        folder_id: Option<String>,
    },
    #[serde(rename = "meetings_trash")]
    MeetingsTrash {
        #[serde(rename = "expectedVersions")]
        expected_versions: BTreeMap<String, String>,
    },
    #[serde(rename = "meetings_restore")]
    MeetingsRestore {
        #[serde(rename = "expectedVersions")]
        expected_versions: BTreeMap<String, String>,
    },
    #[serde(rename = "folder_create")]
    FolderCreate {
        name: String,
        #[serde(rename = "expectedVersion")]
        expected_version: String,
    },
    #[serde(rename = "folder_rename")]
    FolderRename {
        #[serde(rename = "folderId")]
        folder_id: String,
        name: String,
        #[serde(rename = "expectedVersion")]
        expected_version: String,
    },
    #[serde(rename = "folder_delete")]
    FolderDelete {
        #[serde(rename = "folderId")]
        folder_id: String,
        #[serde(rename = "expectedVersion")]
        expected_version: String,
    },
}

pub const METHODS: [&str; 12] = [
    "summary_import",
    "meeting_update",
    "people_update",
    "meeting_attendees_set",
    "meeting_attendee_promote",
    "meeting_date_set",
    "meetings_move",
    "meetings_trash",
    "meetings_restore",
    "folder_create",
    "folder_rename",
    "folder_delete",
];

pub const JOB_METHODS: [&str; 2] = ["jobs_list", "job_cancel"];

// Requests served by the running app's processing service rather than the core writer.
// `speech_status` is an internal read used by `status`, not a separate agent tool.
pub const PROCESSING_METHODS: [&str; 4] = [
    "meeting_import",
    "speech_status",
    "speech_engine_set",
    "speech_switch_cancel",
];

const MAX_IMPORT_PATH: usize = 4096;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AppUpdateAction {
    Status,
    Check,
    Download,
    Install,
}

impl AppUpdateAction {
    pub fn parse(params: Value) -> std::result::Result<Self, ControlError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Params {
            action: AppUpdateAction,
        }
        serde_json::from_value::<Params>(params)
            .map(|params| params.action)
            .map_err(|_| ControlError::invalid())
    }
}

pub struct ControlReply {
    result: Value,
    after_response: Option<Box<dyn FnOnce() + Send>>,
}
impl ControlReply {
    pub fn new(result: Value) -> Self {
        Self {
            result,
            after_response: None,
        }
    }
    pub fn after_response(result: Value, action: impl FnOnce() + Send + 'static) -> Self {
        Self {
            result,
            after_response: Some(Box::new(action)),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "method", content = "params", deny_unknown_fields)]
pub enum JobRequest {
    #[serde(rename = "jobs_list")]
    List {
        #[serde(rename = "meetingId")]
        meeting_id: Option<String>,
        #[serde(default)]
        offset: usize,
        #[serde(default = "job_page_limit")]
        limit: usize,
    },
    #[serde(rename = "job_cancel")]
    Cancel {
        #[serde(rename = "jobId")]
        job_id: String,
        #[serde(rename = "expectedVersion")]
        expected_version: String,
    },
}

fn job_page_limit() -> usize {
    50
}

impl JobRequest {
    pub fn parse(method: &str, params: Value) -> std::result::Result<Self, ControlError> {
        let request: Self = serde_json::from_value(json!({"method":method,"params":params}))
            .map_err(|_| ControlError::invalid())?;
        match &request {
            Self::List {
                meeting_id, limit, ..
            } if *limit == 0
                || *limit > 200
                || meeting_id
                    .as_ref()
                    .is_some_and(|id| store::validate_id(id).is_err()) =>
            {
                return Err(ControlError::invalid())
            }
            Self::Cancel {
                job_id,
                expected_version,
            } if store::validate_id(job_id).is_err() || !valid_version(expected_version) => {
                return Err(ControlError::invalid())
            }
            _ => (),
        }
        Ok(request)
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "method", content = "params", deny_unknown_fields)]
pub enum ProcessingRequest {
    #[serde(rename = "meeting_import")]
    MeetingImport {
        path: String,
        #[serde(rename = "meetingId")]
        meeting_id: Option<String>,
        #[serde(rename = "expectedVersion")]
        expected_version: Option<String>,
    },
    #[serde(rename = "speech_status")]
    SpeechStatus {},
    #[serde(rename = "speech_engine_set")]
    SpeechEngineSet {
        engine: String,
        #[serde(rename = "acknowledgeMemoryWarning", default)]
        acknowledge_memory_warning: bool,
    },
    #[serde(rename = "speech_switch_cancel")]
    SpeechSwitchCancel { target: String },
}

impl ProcessingRequest {
    // Checks only the request shape. The app checks the file, models and its own state.
    pub fn parse(method: &str, params: Value) -> std::result::Result<Self, ControlError> {
        if !PROCESSING_METHODS.contains(&method) || !params.is_object() {
            return Err(ControlError::invalid());
        }
        let request: Self = serde_json::from_value(json!({"method":method,"params":params}))
            .map_err(|_| ControlError::invalid())?;
        let valid = match &request {
            Self::MeetingImport {
                path,
                meeting_id,
                expected_version,
            } => {
                valid_import_path(path)
                    && match (meeting_id, expected_version) {
                        (None, None) => true,
                        (Some(id), Some(version)) => {
                            store::validate_id(id).is_ok() && valid_version(version)
                        }
                        _ => false,
                    }
            }
            Self::SpeechStatus {} => true,
            Self::SpeechEngineSet { engine, .. } | Self::SpeechSwitchCancel { target: engine } => {
                valid_engine_id(engine)
            }
        };
        if valid {
            Ok(request)
        } else {
            Err(ControlError::invalid())
        }
    }
}

// An absolute, normalized path whose extension the library can retain. Whether it is an
// existing regular file is decided by the app that reads it.
pub fn valid_import_path(path: &str) -> bool {
    let candidate = Path::new(path);
    !path.is_empty()
        && path.len() <= MAX_IMPORT_PATH
        && !path.contains('\0')
        && candidate.is_absolute()
        && candidate
            .components()
            .all(|part| !matches!(part, Component::CurDir | Component::ParentDir))
        && candidate
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| {
                crate::AUDIO_IMPORT_EXTENSIONS
                    .iter()
                    .any(|supported| extension.eq_ignore_ascii_case(supported))
            })
}

fn valid_engine_id(engine: &str) -> bool {
    (1..=64).contains(&engine.len()) && engine.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

// Read by CLIs of other versions: a field added later must not make an older CLI
// report a running app as unreachable.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppMetadata {
    pub version: String,
    pub build: String,
    pub integration_version: Option<String>,
}

enum ControlRequest {
    Metadata,
    AppUpdate(AppUpdateAction),
    Mutation(Mutation),
    Job(JobRequest),
    Processing(ProcessingRequest),
    App(AppRequest),
}
impl ControlRequest {
    fn parse(method: &str, params: Value) -> std::result::Result<Self, ControlError> {
        if method == "app_metadata" {
            if params.as_object().is_some_and(|fields| fields.is_empty()) {
                Ok(Self::Metadata)
            } else {
                Err(ControlError::invalid())
            }
        } else if method == "app_update" {
            AppUpdateAction::parse(params).map(Self::AppUpdate)
        } else if JOB_METHODS.contains(&method) {
            JobRequest::parse(method, params).map(Self::Job)
        } else if PROCESSING_METHODS.contains(&method) {
            ProcessingRequest::parse(method, params).map(Self::Processing)
        } else if agent_app::METHODS.contains(&method) {
            AppRequest::parse(method, params).map(Self::App)
        } else {
            Mutation::parse(method, params).map(Self::Mutation)
        }
    }
}

use crate::edit_version::valid as valid_version;

impl Mutation {
    pub fn parse(method: &str, params: Value) -> std::result::Result<Self, ControlError> {
        if !METHODS.contains(&method)
            || !params.is_object()
            || (method == "meeting_date_set" && params.get("meetingDate").is_none())
            || (method == "meetings_move" && params.get("folderId").is_none())
            || (method == "summary_import"
                && params
                    .get("context")
                    .and_then(|value| value.get("summaryId"))
                    .is_none())
        {
            return Err(ControlError::invalid());
        }
        let mutation: Self = serde_json::from_value(json!({"method":method,"params":params}))
            .map_err(|_| ControlError::invalid())?;
        let versions = mutation.versions();
        let folder = matches!(
            &mutation,
            Self::FolderCreate { .. }
                | Self::FolderRename { .. }
                | Self::FolderDelete { .. }
                | Self::PeopleUpdate { .. }
        );
        if (!folder && !matches!(&mutation, Self::SummaryImport { .. }) && versions.is_empty())
            || versions.len() > 200
            || versions
                .iter()
                .any(|(id, version)| store::validate_id(id).is_err() || !valid_version(version))
        {
            return Err(ControlError::invalid());
        }
        match &mutation {
            Self::MeetingUpdate { title, notes, .. } if title.is_none() && notes.is_none() => {
                return Err(ControlError::invalid())
            }
            Self::MeetingsMove {
                folder_id: Some(id),
                ..
            } if store::validate_id(id).is_err() => return Err(ControlError::invalid()),
            Self::PeopleUpdate {
                expected_version, ..
            }
            | Self::FolderCreate {
                expected_version, ..
            }
            | Self::FolderRename {
                expected_version, ..
            }
            | Self::FolderDelete {
                expected_version, ..
            } if !valid_version(expected_version) => return Err(ControlError::invalid()),
            Self::FolderRename { folder_id, .. } | Self::FolderDelete { folder_id, .. }
                if store::validate_id(folder_id).is_err() =>
            {
                return Err(ControlError::invalid())
            }
            _ => (),
        }
        match &mutation {
            Self::SummaryImport {
                meeting_id,
                context,
                text,
                producer,
            } => {
                store::validate_id(meeting_id).map_err(|_| ControlError::invalid())?;
                context.validate().map_err(|_| ControlError::invalid())?;
                producer.validate().map_err(|_| ControlError::invalid())?;
                if text.trim().is_empty() || text.len() > 2 * 1024 * 1024 || text.contains('\0') {
                    return Err(ControlError::invalid());
                }
            }
            Self::PeopleUpdate { people, groups, .. } => crate::people::People {
                people: people.clone(),
                groups: groups.clone(),
            }
            .validate()
            .map_err(|_| ControlError::invalid())?,
            Self::MeetingAttendeesSet { attendees, .. } => {
                crate::people::validate_attendees(attendees).map_err(|_| ControlError::invalid())?
            }
            Self::MeetingAttendeePromote {
                attendee_id,
                expected_catalog_version,
                name,
                ..
            } => {
                store::validate_id(attendee_id).map_err(|_| ControlError::invalid())?;
                if !valid_version(expected_catalog_version) || name.trim().is_empty() {
                    return Err(ControlError::invalid());
                }
            }
            Self::MeetingDateSet {
                meeting_date: Some(date),
                ..
            } => store::validate_meeting_date(date).map_err(|_| ControlError::invalid())?,
            Self::MeetingUpdate { title, notes, .. } => store::validate_text(
                title.as_deref().unwrap_or(""),
                notes.as_deref().unwrap_or(""),
            )
            .map_err(|_| ControlError::invalid())?,
            _ => (),
        }
        Ok(mutation)
    }

    fn versions(&self) -> Vec<(&str, &str)> {
        match self {
            Self::SummaryImport { .. } => Vec::new(),
            Self::MeetingAttendeesSet {
                meeting_id,
                expected_version,
                ..
            }
            | Self::MeetingAttendeePromote {
                meeting_id,
                expected_version,
                ..
            }
            | Self::MeetingUpdate {
                meeting_id,
                expected_version,
                ..
            }
            | Self::MeetingDateSet {
                meeting_id,
                expected_version,
                ..
            } => vec![(meeting_id, expected_version)],
            Self::MeetingsMove {
                expected_versions, ..
            }
            | Self::MeetingsTrash { expected_versions }
            | Self::MeetingsRestore { expected_versions } => expected_versions
                .iter()
                .map(|(id, version)| (id.as_str(), version.as_str()))
                .collect(),
            Self::FolderCreate { .. }
            | Self::FolderRename { .. }
            | Self::FolderDelete { .. }
            | Self::PeopleUpdate { .. } => Vec::new(),
        }
    }

    pub fn trash_ids(&self) -> Option<Vec<String>> {
        match self {
            Self::MeetingsTrash { expected_versions } => {
                Some(expected_versions.keys().cloned().collect())
            }
            _ => None,
        }
    }
}

pub(crate) use crate::edit_version::library_edit_version;
pub use crate::edit_version::meeting_edit_version;

pub(crate) fn validate(engine: &Engine, mutation: &Mutation) -> Result<()> {
    if engine.quitting {
        return Err(CoreError::Unavailable);
    }
    match mutation {
        Mutation::PeopleUpdate {
            expected_version, ..
        } => {
            if &engine.people_catalog()?.edit_version != expected_version {
                return Err(CoreError::EditConflict);
            }
        }
        Mutation::SummaryImport {
            meeting_id,
            context,
            ..
        } => {
            engine.validate_summary_context(meeting_id, context)?;
        }
        Mutation::FolderCreate {
            expected_version, ..
        }
        | Mutation::FolderRename {
            expected_version, ..
        }
        | Mutation::FolderDelete {
            expected_version, ..
        } => {
            if &engine.agent_library_version()? != expected_version {
                return Err(CoreError::EditConflict);
            }
        }
        _ => {
            for (id, expected) in mutation.versions() {
                engine.require_meeting_version(id, expected, false)?;
            }
        }
    }
    Ok(())
}

fn targets(engine: &Engine, ids: &[String]) -> Result<Value> {
    let versions = ids
        .iter()
        .map(|id| {
            let meeting = &engine
                .meetings
                .get(id)
                .ok_or(CoreError::MeetingNotFound)?
                .meeting;
            Ok(json!({"id":id,"editVersion":meeting_edit_version(meeting)?}))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({"applied":true,"meetings":versions}))
}

pub(crate) fn apply(engine: &mut Engine, mutation: Mutation) -> Result<Value> {
    validate(engine, &mutation)?;
    match mutation {
        Mutation::SummaryImport {
            meeting_id,
            context,
            text,
            producer,
        } => {
            let summary = engine.import_summary(&meeting_id, context, text, producer)?;
            Ok(json!({"applied":true,"meetingId":meeting_id,"summaryId":summary.id}))
        }
        Mutation::PeopleUpdate {
            expected_version,
            people,
            groups,
        } => {
            let catalog = engine.replace_people_catalog(&expected_version, people, groups)?;
            Ok(json!({"applied":true,"editVersion":catalog.edit_version}))
        }
        Mutation::MeetingAttendeesSet {
            meeting_id,
            expected_version,
            attendees,
        } => {
            let meeting =
                engine.replace_meeting_attendees(&meeting_id, &expected_version, attendees)?;
            Ok(
                json!({"applied":true,"meetingId":meeting.id,"editVersion":meeting_edit_version(&meeting)?}),
            )
        }
        Mutation::MeetingAttendeePromote {
            meeting_id,
            expected_version,
            expected_catalog_version,
            attendee_id,
            name,
        } => {
            let result = engine.promote_meeting_attendee(
                &meeting_id,
                &expected_version,
                &expected_catalog_version,
                &attendee_id,
                name,
            )?;
            Ok(
                json!({"applied":true,"meetingId":result.meeting.id,"editVersion":meeting_edit_version(&result.meeting)?,"catalogEditVersion":result.catalog.edit_version,"personId":result.person.id}),
            )
        }
        Mutation::MeetingUpdate {
            meeting_id,
            title,
            notes,
            ..
        } => {
            let current = engine
                .meetings
                .get(&meeting_id)
                .ok_or(CoreError::MeetingNotFound)?
                .meeting
                .clone();
            let meeting = engine.update_meeting(
                &meeting_id,
                title.unwrap_or(current.title),
                notes.unwrap_or(current.notes),
            )?;
            Ok(
                json!({"applied":true,"meetingId":meeting.id,"editVersion":meeting_edit_version(&meeting)?}),
            )
        }
        Mutation::MeetingDateSet {
            meeting_id,
            meeting_date,
            ..
        } => {
            let meeting = engine.set_meeting_date(&meeting_id, meeting_date)?;
            Ok(
                json!({"applied":true,"meetingId":meeting.id,"editVersion":meeting_edit_version(&meeting)?}),
            )
        }
        Mutation::MeetingsMove {
            expected_versions,
            folder_id,
        } => {
            let ids = expected_versions.into_keys().collect::<Vec<_>>();
            engine.move_meetings(&ids, folder_id)?;
            targets(engine, &ids)
        }
        Mutation::MeetingsTrash { expected_versions } => {
            let ids = expected_versions.into_keys().collect::<Vec<_>>();
            engine.trash_meetings(&ids)?;
            targets(engine, &ids)
        }
        Mutation::MeetingsRestore { expected_versions } => {
            let ids = expected_versions.into_keys().collect::<Vec<_>>();
            engine.restore_meetings(&ids)?;
            targets(engine, &ids)
        }
        Mutation::FolderCreate { name, .. } => {
            let folder = engine.create_folder(name)?;
            Ok(
                json!({"applied":true,"folder":folder,"editVersion":engine.agent_library_version()?}),
            )
        }
        Mutation::FolderRename {
            folder_id, name, ..
        } => {
            let folder = engine.rename_folder(&folder_id, name)?;
            Ok(
                json!({"applied":true,"folder":folder,"editVersion":engine.agent_library_version()?}),
            )
        }
        Mutation::FolderDelete { folder_id, .. } => {
            engine.delete_folder(&folder_id)?;
            Ok(
                json!({"applied":true,"folderId":folder_id,"editVersion":engine.agent_library_version()?}),
            )
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Descriptor {
    protocol: u32,
    address: SocketAddrV4,
    generation: String,
    secret: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    protocol: u32,
    generation: String,
    secret: String,
    method: String,
    params: Value,
}

fn descriptor_path(root: &Path) -> std::result::Result<PathBuf, ControlError> {
    crate::reader::LibraryReader::open(root).map_err(ControlError::core)?;
    store::checked_path(root, Path::new(DESCRIPTOR)).map_err(ControlError::core)
}

fn read_descriptor(root: &Path) -> std::result::Result<Descriptor, ControlError> {
    let path = descriptor_path(root)?;
    let file = store::open_read(&path).map_err(|_| ControlError::unavailable())?;
    let metadata = file.metadata().map_err(|_| ControlError::unavailable())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ControlError::new(
                "unsafe_control_path",
                "Nero 수정 연결 파일의 권한을 확인해 주세요.",
            ));
        }
    }
    if metadata.len() > 4096 {
        return Err(ControlError::unavailable());
    }
    let mut bytes = Vec::new();
    file.take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| ControlError::unavailable())?;
    if bytes.len() > 4096 {
        return Err(ControlError::unavailable());
    }
    let value: Descriptor =
        serde_json::from_slice(&bytes).map_err(|_| ControlError::unavailable())?;
    if value.protocol != PROTOCOL {
        return Err(ControlError::new(
            "protocol_mismatch",
            "실행 중인 Nero와 CLI의 연결 방식이 다릅니다. Nero 앱과 CLI를 최신 버전으로 업데이트해 주세요.",
        ));
    }
    if *value.address.ip() != Ipv4Addr::LOCALHOST
        || value.address.port() == 0
        || value.secret.len() != 64
        || !value.secret.bytes().all(|byte| byte.is_ascii_hexdigit())
        || value.generation.parse::<uuid::Uuid>().is_err()
    {
        return Err(ControlError::unavailable());
    }
    Ok(value)
}

fn write_frame(stream: &mut TcpStream, value: &impl Serialize) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err(std::io::Error::other("Invalid frame"));
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)?;
    stream.flush()
}
fn read_frame(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut header = [0; 4];
    stream.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(std::io::Error::other("Invalid frame"));
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    Ok(bytes)
}
// A client that stopped waiting (its read timeout passed or it exited) has closed its
// end. Its request may have queued behind a slow one and must not take effect now.
fn abandoned(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return true;
    }
    let mut probe = [0; 1];
    let closed = match stream.peek(&mut probe) {
        Ok(0) => true,
        Ok(_) => false,
        Err(error) => error.kind() != std::io::ErrorKind::WouldBlock,
    };
    stream.set_nonblocking(false).is_err() || closed
}
fn matches_secret(expected: &str, value: &str) -> bool {
    expected.len() == value.len()
        && expected
            .as_bytes()
            .iter()
            .zip(value.bytes())
            .fold(0, |result, (a, b)| result | (a ^ b))
            == 0
}

pub fn request(
    root: &Path,
    method: &str,
    params: Value,
) -> std::result::Result<Value, ControlError> {
    ControlRequest::parse(method, params.clone())?;
    let descriptor = read_descriptor(root)?;
    let envelope = Envelope {
        protocol: PROTOCOL,
        generation: descriptor.generation,
        secret: descriptor.secret,
        method: method.into(),
        params,
    };
    if serde_json::to_vec(&envelope)
        .map_err(|_| ControlError::invalid())?
        .len()
        > MAX_FRAME
    {
        return Err(ControlError::new(
            "request_too_large",
            "수정 요청이 전송 한도를 넘습니다. 메모의 분량을 줄여 다시 시도해 주세요.",
        ));
    }
    let mut stream = TcpStream::connect_timeout(&descriptor.address.into(), Duration::from_secs(2))
        .map_err(|_| ControlError::unavailable())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(
            if matches!(method, "app_metadata" | "speech_status") {
                2
            } else {
                45
            },
        )))
        .map_err(|_| ControlError::unavailable())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| ControlError::unavailable())?;
    write_frame(&mut stream, &envelope).map_err(|_| ControlError::unknown_result())?;
    let bytes = read_frame(&mut stream).map_err(|_| ControlError::unknown_result())?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| ControlError::unknown_result())?;
    if let Some(error) = value.get("error") {
        if agent_app::METHODS.contains(&method) {
            return Err(app_error(error));
        }
        let received = error["code"].as_str().unwrap_or("");
        let (code, message) = match received {
            "invalid_input" if method == "app_update" => (
                "app_update_unsupported",
                "실행 중인 Nero는 에이전트 업데이트를 지원하지 않습니다. 앱의 업데이트 화면을 사용해 주세요.",
            ),
            "app_update_unsupported" => (
                "app_update_unsupported",
                "실행 중인 Nero는 에이전트 업데이트를 지원하지 않습니다. 앱의 업데이트 화면을 사용해 주세요.",
            ),
            "app_update_check_failed" => (
                "app_update_check_failed",
                "공개 업데이트를 확인하지 못했습니다. 네트워크 연결과 Nero의 업데이트 화면을 확인해 주세요.",
            ),
            "app_update_failed" => (
                "app_update_failed",
                "업데이트 요청을 완료하지 못했습니다. Nero의 업데이트 상태를 확인해 주세요.",
            ),
            // The CLI validates these requests before sending, so an app rejecting their
            // shape predates them.
            "invalid_input" | "app_feature_unsupported"
                if PROCESSING_METHODS.contains(&method) =>
            {
                (
                    "app_feature_unsupported",
                    "실행 중인 Nero는 이 요청을 지원하지 않습니다. Nero를 업데이트한 뒤 다시 시도해 주세요.",
                )
            }
            "audio_file_unavailable" => (
                "audio_file_unavailable",
                "가져올 음성 파일을 찾을 수 없거나 일반 파일이 아닙니다.",
            ),
            "unsupported_audio_format" => (
                "unsupported_audio_format",
                "지원하지 않는 음성 파일 형식입니다.",
            ),
            "recording_active" => (
                "recording_active",
                "녹음 중에는 처리할 수 없습니다. 녹음을 마친 뒤 다시 시도해 주세요.",
            ),
            "speech_not_ready" => (
                "speech_not_ready",
                "음성 인식이 준비되지 않았습니다. Nero에서 음성 인식을 먼저 준비해 주세요.",
            ),
            "speech_engine_unknown" => (
                "speech_engine_unknown",
                "실행 중인 Nero가 알지 못하는 음성 인식 엔진입니다.",
            ),
            "speech_engine_unsupported" => (
                "speech_engine_unsupported",
                "이 기기에서 사용할 수 없는 음성 인식 엔진입니다.",
            ),
            "memory_warning_unacknowledged" => (
                "memory_warning_unacknowledged",
                "선택한 음성 인식 엔진의 메모리 경고에 대한 사용자 확인이 필요합니다.",
            ),
            "speech_switch_not_found" => (
                "speech_switch_not_found",
                "대기 중인 음성 인식 전환이 없습니다.",
            ),
            "processing_busy" => (
                "processing_busy",
                "처리 작업이 대기 또는 실행 중입니다. 작업이 끝난 뒤 다시 시도해 주세요.",
            ),
            "processing_rejected" => (
                "processing_rejected",
                "Nero가 요청을 처리하지 못했습니다. 현재 처리 상태를 확인해 주세요.",
            ),
            "mutation_result_unknown" => ("mutation_result_unknown", UNKNOWN_RESULT),
            "response_too_large" => ("response_too_large", RESPONSE_TOO_LARGE),
            _ => core_code_message(received)
                .unwrap_or(("mutation_failed", "수정을 완료하지 못했습니다.")),
        };
        return Err(ControlError::new(code, message));
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(ControlError::unknown_result)
}

// The CLI checks an app request before sending it, so an app rejecting its shape predates
// it. Other codes come from the app's own services and keep the app's explanation.
fn app_error(error: &Value) -> ControlError {
    let code = error["code"].as_str().unwrap_or("");
    if matches!(code, "invalid_input" | "app_feature_unsupported") {
        return ControlError::new(
            "app_feature_unsupported",
            "실행 중인 Nero는 이 요청을 지원하지 않습니다. Nero를 업데이트한 뒤 다시 시도해 주세요.",
        );
    }
    let message = error["message"]
        .as_str()
        .filter(|message| !message.trim().is_empty() && message.len() <= 1024)
        .unwrap_or("요청을 완료하지 못했습니다. Nero에서 현재 상태를 확인해 주세요.");
    match agent_app::ERRORS.iter().find(|known| **known == code) {
        Some(known) => ControlError::new(known, message),
        None => ControlError::new("request_failed", message),
    }
}

struct TemporaryDescriptor(PathBuf);
impl Drop for TemporaryDescriptor {
    fn drop(&mut self) {
        if std::fs::symlink_metadata(&self.0)
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

fn remove_own_descriptor(root: &Path, descriptor: &Descriptor) {
    if let Ok(current) = read_descriptor(root) {
        if current.generation == descriptor.generation
            && matches_secret(&current.secret, &descriptor.secret)
        {
            if let Ok(path) = descriptor_path(root) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

// Answers one connection. A panicking handler answers an unknown result instead of
// ending the server, which no one would restart until the app does.
fn serve_connection(mut stream: TcpStream, expected: &Descriptor, handlers: &Handlers) {
    if stream.set_nonblocking(false).is_err()
        || stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .is_err()
        || stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .is_err()
    {
        return;
    }
    let frame = read_frame(&mut stream).ok();
    if frame.is_some() && abandoned(&stream) {
        return;
    }
    let result = frame
        .and_then(|bytes| serde_json::from_slice::<Envelope>(&bytes).ok())
        .filter(|value| {
            value.protocol == PROTOCOL
                && value.generation == expected.generation
                && matches_secret(&expected.secret, &value.secret)
        })
        .ok_or_else(|| ControlError::new("app_not_ready", "Nero 수정 연결을 다시 확인해 주세요."))
        .and_then(|value| ControlRequest::parse(&value.method, value.params))
        .and_then(|request| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handlers.answer(request)))
                .unwrap_or_else(|_| Err(ControlError::unknown_result()))
        });
    let (mut response, after_response) = match result {
        Ok(reply) => (json!({"result":reply.result}), reply.after_response),
        Err(error) => (json!({"error":error}), None),
    };
    // A reply over the frame limit is reported instead of closing without an answer.
    if serde_json::to_vec(&response).map_or(true, |bytes| bytes.len() > MAX_FRAME) {
        response = json!({"error":ControlError::new("response_too_large", RESPONSE_TOO_LARGE)});
    }
    let _ = write_frame(&mut stream, &response);
    // The app owns an accepted update even if its client disconnects.
    if let Some(action) = after_response {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(action));
    }
}

pub struct Server {
    stopping: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    root: PathBuf,
    descriptor: Descriptor,
    _writer: CoreHandle,
}

type Handler<T, R> = Box<dyn Fn(T) -> std::result::Result<R, ControlError> + Send + Sync>;

// What the running app answers for each request family. A family left at its default
// answers like an app that predates it.
pub struct Handlers {
    mutation: Handler<Mutation, Value>,
    jobs: Handler<JobRequest, Value>,
    metadata: Box<dyn Fn() -> std::result::Result<AppMetadata, ControlError> + Send + Sync>,
    updates: Handler<AppUpdateAction, ControlReply>,
    processing: Handler<ProcessingRequest, Value>,
    app: Handler<AppRequest, Value>,
}
impl Handlers {
    pub fn new(
        mutation: impl Fn(Mutation) -> std::result::Result<Value, ControlError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            mutation: Box::new(mutation),
            jobs: Box::new(|_| Err(ControlError::invalid())),
            metadata: Box::new(|| Err(ControlError::unavailable())),
            updates: Box::new(|_| {
                Err(ControlError::new(
                    "app_update_unsupported",
                    "이 앱은 에이전트 업데이트를 지원하지 않습니다.",
                ))
            }),
            processing: Box::new(|_| Err(ControlError::invalid())),
            app: Box::new(|_| Err(ControlError::invalid())),
        }
    }
    pub fn jobs(
        mut self,
        jobs: impl Fn(JobRequest) -> std::result::Result<Value, ControlError> + Send + Sync + 'static,
    ) -> Self {
        self.jobs = Box::new(jobs);
        self
    }
    pub fn metadata(
        mut self,
        metadata: impl Fn() -> std::result::Result<AppMetadata, ControlError> + Send + Sync + 'static,
    ) -> Self {
        self.metadata = Box::new(metadata);
        self
    }
    pub fn updates(
        mut self,
        updates: impl Fn(AppUpdateAction) -> std::result::Result<ControlReply, ControlError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.updates = Box::new(updates);
        self
    }
    pub fn processing(
        mut self,
        processing: impl Fn(ProcessingRequest) -> std::result::Result<Value, ControlError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.processing = Box::new(processing);
        self
    }
    pub fn app(
        mut self,
        app: impl Fn(AppRequest) -> std::result::Result<Value, ControlError> + Send + Sync + 'static,
    ) -> Self {
        self.app = Box::new(app);
        self
    }
    fn answer(&self, request: ControlRequest) -> std::result::Result<ControlReply, ControlError> {
        match request {
            ControlRequest::Metadata => (self.metadata)()
                .and_then(|value| serde_json::to_value(value).map_err(|_| ControlError::invalid()))
                .map(ControlReply::new),
            ControlRequest::AppUpdate(action) => (self.updates)(action),
            ControlRequest::Mutation(request) => (self.mutation)(request).map(ControlReply::new),
            ControlRequest::Job(request) => (self.jobs)(request).map(ControlReply::new),
            ControlRequest::Processing(request) => {
                (self.processing)(request).map(ControlReply::new)
            }
            ControlRequest::App(request) => (self.app)(request).map(ControlReply::new),
        }
    }
}

impl Server {
    pub fn start(
        root: &Path,
        core: CoreHandle,
        handler: impl Fn(Mutation) -> std::result::Result<Value, ControlError> + Send + Sync + 'static,
    ) -> std::result::Result<Self, ControlError> {
        Self::serve(root, core, Handlers::new(handler))
    }

    pub fn start_with_jobs(
        root: &Path,
        core: CoreHandle,
        handler: impl Fn(Mutation) -> std::result::Result<Value, ControlError> + Send + Sync + 'static,
        jobs: impl Fn(JobRequest) -> std::result::Result<Value, ControlError> + Send + Sync + 'static,
    ) -> std::result::Result<Self, ControlError> {
        Self::serve(root, core, Handlers::new(handler).jobs(jobs))
    }

    pub fn start_with_metadata(
        root: &Path,
        core: CoreHandle,
        handler: impl Fn(Mutation) -> std::result::Result<Value, ControlError> + Send + Sync + 'static,
        jobs: impl Fn(JobRequest) -> std::result::Result<Value, ControlError> + Send + Sync + 'static,
        metadata: impl Fn() -> std::result::Result<AppMetadata, ControlError> + Send + Sync + 'static,
    ) -> std::result::Result<Self, ControlError> {
        Self::serve(
            root,
            core,
            Handlers::new(handler).jobs(jobs).metadata(metadata),
        )
    }

    pub fn start_with_updates(
        root: &Path,
        core: CoreHandle,
        handler: impl Fn(Mutation) -> std::result::Result<Value, ControlError> + Send + Sync + 'static,
        jobs: impl Fn(JobRequest) -> std::result::Result<Value, ControlError> + Send + Sync + 'static,
        metadata: impl Fn() -> std::result::Result<AppMetadata, ControlError> + Send + Sync + 'static,
        updates: impl Fn(AppUpdateAction) -> std::result::Result<ControlReply, ControlError>
            + Send
            + Sync
            + 'static,
    ) -> std::result::Result<Self, ControlError> {
        Self::serve(
            root,
            core,
            Handlers::new(handler)
                .jobs(jobs)
                .metadata(metadata)
                .updates(updates),
        )
    }

    pub fn start_with_processing(
        root: &Path,
        core: CoreHandle,
        handler: impl Fn(Mutation) -> std::result::Result<Value, ControlError> + Send + Sync + 'static,
        jobs: impl Fn(JobRequest) -> std::result::Result<Value, ControlError> + Send + Sync + 'static,
        metadata: impl Fn() -> std::result::Result<AppMetadata, ControlError> + Send + Sync + 'static,
        updates: impl Fn(AppUpdateAction) -> std::result::Result<ControlReply, ControlError>
            + Send
            + Sync
            + 'static,
        processing: impl Fn(ProcessingRequest) -> std::result::Result<Value, ControlError>
            + Send
            + Sync
            + 'static,
    ) -> std::result::Result<Self, ControlError> {
        Self::serve(
            root,
            core,
            Handlers::new(handler)
                .jobs(jobs)
                .metadata(metadata)
                .updates(updates)
                .processing(processing),
        )
    }

    pub fn serve(
        root: &Path,
        core: CoreHandle,
        handlers: Handlers,
    ) -> std::result::Result<Self, ControlError> {
        let path = descriptor_path(root)?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| ControlError::new("app_not_ready", "Nero 수정 연결을 열지 못했습니다."))?;
        let std::net::SocketAddr::V4(address) = listener
            .local_addr()
            .map_err(|_| ControlError::unavailable())?
        else {
            return Err(ControlError::unavailable());
        };
        listener
            .set_nonblocking(true)
            .map_err(|_| ControlError::unavailable())?;
        let descriptor = Descriptor {
            protocol: PROTOCOL,
            address,
            generation: uuid::Uuid::new_v4().to_string(),
            secret: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        };
        let relative = PathBuf::from(format!(".agent-control-{}.tmp", descriptor.generation));
        let temporary = store::checked_path(root, &relative).map_err(ControlError::core)?;
        let mut file = store::create_new(&temporary).map_err(ControlError::core)?;
        let _temporary_guard = TemporaryDescriptor(temporary.clone());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|_| ControlError::unavailable())?;
        }
        serde_json::to_writer(&mut file, &descriptor).map_err(|_| ControlError::unavailable())?;
        file.sync_all().map_err(|_| ControlError::unavailable())?;
        store::checked_path(root, Path::new(DESCRIPTOR)).map_err(ControlError::core)?;
        std::fs::rename(&temporary, path).map_err(|_| ControlError::unavailable())?;
        let stopping = Arc::new(AtomicBool::new(false));
        let stopped = stopping.clone();
        let expected = descriptor.clone();
        let worker = thread::Builder::new()
            .name("nero-agent-control".into())
            .spawn(move || {
                let handlers = Arc::new(handlers);
                let mut connections: Vec<thread::JoinHandle<()>> = Vec::new();
                while !stopped.load(Ordering::Acquire) {
                    connections.retain(|connection| !connection.is_finished());
                    if connections.len() >= MAX_CONNECTIONS {
                        thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    match listener.accept() {
                        Ok((stream, peer)) => {
                            if !peer.ip().is_loopback() {
                                continue;
                            }
                            // Each request runs on its own thread, so one waiting on the
                            // network, a model or an edit lease does not hold up the rest.
                            let (expected, handlers) = (expected.clone(), handlers.clone());
                            if let Ok(connection) = thread::Builder::new()
                                .name("nero-agent-request".into())
                                .spawn(move || serve_connection(stream, &expected, &handlers))
                            {
                                connections.push(connection);
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(20))
                        }
                        // Accept fails transiently (descriptor exhaustion, aborted
                        // connections); the server keeps listening until it is stopped.
                        Err(_) => thread::sleep(Duration::from_millis(200)),
                    }
                }
                for connection in connections {
                    let _ = connection.join();
                }
            })
            .map_err(|_| {
                remove_own_descriptor(root, &descriptor);
                ControlError::unavailable()
            })?;
        Ok(Self {
            stopping,
            worker: Some(worker),
            root: root.into(),
            descriptor,
            _writer: core,
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        remove_own_descriptor(&self.root, &self.descriptor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Meeting;
    use std::sync::atomic::AtomicUsize;

    fn temporary_directory() -> tempfile::TempDir {
        let parent = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        tempfile::tempdir_in(parent).unwrap()
    }

    fn fixture() -> (tempfile::TempDir, CoreHandle, Meeting) {
        let directory = temporary_directory();
        let core = CoreHandle::open(directory.path()).unwrap();
        let meeting = core.create_meeting("주간 회의", "원래 메모").unwrap();
        (directory, core, meeting)
    }

    fn folder_version(root: &Path) -> String {
        crate::reader::LibraryReader::open(root)
            .unwrap()
            .query("folders_list", json!({}))
            .unwrap()["editVersion"]
            .as_str()
            .unwrap()
            .into()
    }

    #[test]
    fn app_updates_reject_untrusted_inputs_and_report_unsupported_apps() {
        for params in [
            json!({}),
            json!({"action":"purge"}),
            json!({"action":"install","url":"https://untrusted.test"}),
        ] {
            assert!(AppUpdateAction::parse(params).is_err());
        }
        let (directory, core, _) = fixture();
        let _server = Server::start_with_updates(
            directory.path(),
            core,
            |_| Ok(json!({})),
            |_| Ok(json!({})),
            || Err(ControlError::unavailable()),
            |_| Err(ControlError::invalid()),
        )
        .unwrap();
        assert_eq!(
            request(directory.path(), "app_update", json!({"action":"status"}))
                .unwrap_err()
                .code,
            "app_update_unsupported"
        );
    }

    #[test]
    fn fragmented_delayed_frame_is_read_before_dispatch() {
        let (directory, core, _) = fixture();
        let _server = Server::start_with_updates(
            directory.path(),
            core,
            |_| Ok(json!({})),
            |_| Ok(json!({})),
            || Err(ControlError::unavailable()),
            |_| Err(ControlError::invalid()),
        )
        .unwrap();
        let descriptor = read_descriptor(directory.path()).unwrap();
        let mut stream = TcpStream::connect(descriptor.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let bytes = serde_json::to_vec(&Envelope {
            protocol: PROTOCOL,
            generation: descriptor.generation,
            secret: descriptor.secret,
            method: "app_update".into(),
            params: json!({"action":"status"}),
        })
        .unwrap();
        let header = (bytes.len() as u32).to_be_bytes();
        stream.write_all(&header[..2]).unwrap();
        thread::sleep(Duration::from_millis(100));
        stream.write_all(&header[2..]).unwrap();
        stream.write_all(&bytes[..bytes.len() / 2]).unwrap();
        thread::sleep(Duration::from_millis(100));
        stream.write_all(&bytes[bytes.len() / 2..]).unwrap();
        let response: Value = serde_json::from_slice(&read_frame(&mut stream).unwrap()).unwrap();
        assert_eq!(response["error"]["code"], "invalid_input");
    }

    #[test]
    fn accepted_update_response_is_flushed_before_its_owned_work_runs() {
        let (directory, core, _) = fixture();
        let (acknowledged, acknowledgement) = std::sync::mpsc::channel();
        let acknowledgement = std::sync::Mutex::new(Some(acknowledgement));
        let (completed, completion) = std::sync::mpsc::channel();
        let _server = Server::start_with_updates(
            directory.path(),
            core,
            |_| Ok(json!({})),
            |_| Ok(json!({})),
            || Err(ControlError::unavailable()),
            move |action| {
                assert_eq!(action, AppUpdateAction::Install);
                let acknowledgement = acknowledgement.lock().unwrap().take().unwrap();
                let completed = completed.clone();
                Ok(ControlReply::after_response(
                    json!({"accepted":true,"restarting":true}),
                    move || {
                        completed
                            .send(acknowledgement.recv_timeout(Duration::from_secs(2)).is_ok())
                            .unwrap();
                    },
                ))
            },
        )
        .unwrap();
        let result = request(directory.path(), "app_update", json!({"action":"install"})).unwrap();
        assert_eq!(result, json!({"accepted":true,"restarting":true}));
        acknowledged.send(()).unwrap();
        assert!(completion.recv_timeout(Duration::from_secs(3)).unwrap());
    }

    #[test]
    fn accepted_update_work_survives_a_disconnected_client() {
        let (directory, core, _) = fixture();
        let (completed, completion) = std::sync::mpsc::channel();
        let (started, start) = std::sync::mpsc::channel();
        let (disconnected, disconnect) = std::sync::mpsc::channel::<()>();
        let disconnect = std::sync::Mutex::new(disconnect);
        let _server = Server::start_with_updates(
            directory.path(),
            core,
            |_| Ok(json!({})),
            |_| Ok(json!({})),
            || Err(ControlError::unavailable()),
            move |_| {
                started.send(()).unwrap();
                disconnect
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
                let completed = completed.clone();
                Ok(ControlReply::after_response(
                    json!({"accepted":true}),
                    move || {
                        completed.send(()).unwrap();
                    },
                ))
            },
        )
        .unwrap();
        let descriptor = read_descriptor(directory.path()).unwrap();
        let mut stream = TcpStream::connect(descriptor.address).unwrap();
        write_frame(
            &mut stream,
            &Envelope {
                protocol: PROTOCOL,
                generation: descriptor.generation,
                secret: descriptor.secret,
                method: "app_update".into(),
                params: json!({"action":"install"}),
            },
        )
        .unwrap();
        start.recv_timeout(Duration::from_secs(3)).unwrap();
        drop(stream);
        disconnected.send(()).unwrap();
        completion.recv_timeout(Duration::from_secs(3)).unwrap();
    }

    #[test]
    fn a_request_its_client_abandoned_before_it_was_read_never_runs() {
        let calls = Arc::new(AtomicUsize::new(0));
        let invoked = calls.clone();
        let handlers = Handlers::new(move |_| {
            invoked.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"applied":true}))
        });
        let descriptor = Descriptor {
            protocol: PROTOCOL,
            address: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1),
            generation: uuid::Uuid::new_v4().to_string(),
            secret: "a".repeat(64),
        };
        let envelope = Envelope {
            protocol: PROTOCOL,
            generation: descriptor.generation.clone(),
            secret: descriptor.secret.clone(),
            method: "meeting_update".into(),
            params: json!({"meetingId":uuid::Uuid::new_v4().to_string(),"expectedVersion":format!("sha256:{}", "0".repeat(64)),"title":"제목"}),
        };
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        // The client wrote its request, then gave up before the server read it.
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        write_frame(&mut client, &envelope).unwrap();
        drop(client);
        let (stream, _) = listener.accept().unwrap();
        serve_connection(stream, &descriptor, &handlers);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        write_frame(&mut client, &envelope).unwrap();
        let (stream, _) = listener.accept().unwrap();
        serve_connection(stream, &descriptor, &handlers);
        let response: Value = serde_json::from_slice(&read_frame(&mut client).unwrap()).unwrap();
        assert_eq!(response["result"]["applied"], true);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_slow_request_does_not_hold_up_other_requests() {
        let (directory, core, meeting) = fixture();
        let (started, start) = std::sync::mpsc::channel();
        let (released, release) = std::sync::mpsc::channel::<()>();
        let release = std::sync::Mutex::new(release);
        let _server = Server::start_with_metadata(
            directory.path(),
            core,
            move |_| {
                started.send(()).unwrap();
                let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(5));
                Ok(json!({"applied":true}))
            },
            |_| Ok(json!({"jobs":[]})),
            || {
                Ok(AppMetadata {
                    version: "0.0.2".into(),
                    build: "7".into(),
                    integration_version: None,
                })
            },
        )
        .unwrap();
        let params = json!({"meetingId":meeting.id,"expectedVersion":meeting_edit_version(&meeting).unwrap(),"notes":"에이전트 메모"});
        let root = directory.path().to_path_buf();
        let slow = thread::spawn(move || request(&root, "meeting_update", params));
        start.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(
            request(directory.path(), "app_metadata", json!({})).unwrap()["build"],
            "7"
        );
        released.send(()).unwrap();
        assert_eq!(slow.join().unwrap().unwrap()["applied"], true);
    }

    #[test]
    fn a_panicking_handler_is_answered_and_the_server_keeps_serving() {
        let (directory, core, meeting) = fixture();
        let calls = Arc::new(AtomicUsize::new(0));
        let invoked = calls.clone();
        let _server = Server::start(directory.path(), core, move |_| {
            if invoked.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("synthetic handler failure");
            }
            Ok(json!({"applied":true}))
        })
        .unwrap();
        let params = json!({"meetingId":meeting.id,"expectedVersion":meeting_edit_version(&meeting).unwrap(),"notes":"에이전트 메모"});
        assert_eq!(
            request(directory.path(), "meeting_update", params.clone())
                .unwrap_err()
                .code,
            "mutation_result_unknown"
        );
        assert_eq!(
            request(directory.path(), "meeting_update", params).unwrap()["applied"],
            true
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_reply_over_the_frame_limit_is_reported_as_too_large() {
        let (directory, core, meeting) = fixture();
        let _server = Server::start(directory.path(), core, |_| {
            Ok(json!({"text":"x".repeat(MAX_FRAME)}))
        })
        .unwrap();
        let params = json!({"meetingId":meeting.id,"expectedVersion":meeting_edit_version(&meeting).unwrap(),"notes":"에이전트 메모"});
        assert_eq!(
            request(directory.path(), "meeting_update", params)
                .unwrap_err()
                .code,
            "response_too_large"
        );
    }

    #[test]
    fn metadata_is_authenticated_read_only_and_has_no_parameters() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let core = CoreHandle::open(&root).unwrap();
        let _server = Server::start_with_metadata(
            root.as_path(),
            core,
            |_| panic!("metadata must not mutate"),
            |_| panic!("metadata must not query or cancel jobs"),
            || {
                Ok(AppMetadata {
                    version: "0.1.0".into(),
                    build: "47".into(),
                    integration_version: Some("0.1.0-nero.33".into()),
                })
            },
        )
        .unwrap();
        assert!(request(root.as_path(), "app_metadata", json!({"refresh":true})).is_err());
        assert_eq!(
            request(root.as_path(), "app_metadata", json!({})).unwrap()["build"],
            "47"
        );
        assert!(!root.as_path().join("models").exists());
        let mut descriptor = read_descriptor(root.as_path()).unwrap();
        descriptor.secret = "wrong".into();
        let mut stream = TcpStream::connect(descriptor.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        write_frame(
            &mut stream,
            &Envelope {
                protocol: PROTOCOL,
                generation: descriptor.generation,
                secret: descriptor.secret,
                method: "app_metadata".into(),
                params: json!({}),
            },
        )
        .unwrap();
        let denied: Value = serde_json::from_slice(&read_frame(&mut stream).unwrap()).unwrap();
        assert!(denied.get("result").is_none());
        assert_eq!(denied["error"]["code"], "app_not_ready");
    }

    #[test]
    fn job_requests_use_authenticated_transport_and_reject_unknown_fields() {
        let (directory, core, _) = fixture();
        let calls = Arc::new(AtomicUsize::new(0));
        let invoked = calls.clone();
        let _server = Server::start_with_jobs(
            directory.path(),
            core,
            |_| panic!("not a mutation"),
            move |request| {
                invoked.fetch_add(1, Ordering::SeqCst);
                assert!(matches!(request, JobRequest::List { .. }));
                Ok(json!({"jobs":[]}))
            },
        )
        .unwrap();
        assert!(JobRequest::parse("jobs_list", json!({"unknown":true})).is_err());
        assert!(JobRequest::parse(
            "job_cancel",
            json!({"jobId":uuid::Uuid::new_v4().to_string()})
        )
        .is_err());
        assert_eq!(
            request(directory.path(), "jobs_list", json!({})).unwrap(),
            json!({"jobs":[]})
        );
        let descriptor = read_descriptor(directory.path()).unwrap();
        let mut stream = TcpStream::connect(descriptor.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        write_frame(
            &mut stream,
            &Envelope {
                protocol: PROTOCOL,
                generation: descriptor.generation,
                secret: "invalid".into(),
                method: "jobs_list".into(),
                params: json!({}),
            },
        )
        .unwrap();
        let response: Value = serde_json::from_slice(&read_frame(&mut stream).unwrap()).unwrap();
        assert!(response.get("error").is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn processing_requests_validate_shape_before_reaching_the_app() {
        let id = uuid::Uuid::new_v4().to_string();
        let version = format!("sha256:{}", "0".repeat(64));
        let audio = std::env::temp_dir().join("synthetic-meeting.M4A");
        let audio = audio.to_str().unwrap();
        assert_eq!(
            ProcessingRequest::parse("meeting_import", json!({"path":audio})).unwrap(),
            ProcessingRequest::MeetingImport {
                path: audio.into(),
                meeting_id: None,
                expected_version: None
            }
        );
        assert!(ProcessingRequest::parse(
            "meeting_import",
            json!({"path":audio,"meetingId":id,"expectedVersion":version})
        )
        .is_ok());
        assert_eq!(
            ProcessingRequest::parse("speech_engine_set", json!({"engine":"qwenKo"})).unwrap(),
            ProcessingRequest::SpeechEngineSet {
                engine: "qwenKo".into(),
                acknowledge_memory_warning: false
            }
        );
        assert!(ProcessingRequest::parse("speech_status", json!({})).is_ok());
        assert!(
            ProcessingRequest::parse("speech_switch_cancel", json!({"target":"qwenKo"})).is_ok()
        );
        let parent = std::env::temp_dir().join("..").join("synthetic.wav");
        for (method, params) in [
            ("meeting_import", json!({"path":"synthetic.wav"})),
            ("meeting_import", json!({"path":parent.to_str().unwrap()})),
            (
                "meeting_import",
                json!({"path":std::env::temp_dir().join("notes.txt").to_str().unwrap()}),
            ),
            (
                "meeting_import",
                json!({"path":std::env::temp_dir().join("audio").to_str().unwrap()}),
            ),
            ("meeting_import", json!({"path":format!("{audio}\0.wav")})),
            ("meeting_import", json!({"path":audio,"meetingId":id})),
            (
                "meeting_import",
                json!({"path":audio,"expectedVersion":version}),
            ),
            (
                "meeting_import",
                json!({"path":audio,"meetingId":"../escape","expectedVersion":version}),
            ),
            (
                "meeting_import",
                json!({"path":audio,"meetingId":id,"expectedVersion":"old"}),
            ),
            ("meeting_import", json!({"path":audio,"transcribe":true})),
            ("speech_status", json!({"refresh":true})),
            ("speech_status", Value::Null),
            ("speech_engine_set", json!({})),
            ("speech_engine_set", json!({"engine":"../qwen"})),
            (
                "speech_engine_set",
                json!({"engine":"qwenKo","download":false}),
            ),
            ("speech_switch_cancel", json!({})),
            ("speech_purge", json!({})),
        ] {
            assert!(
                ProcessingRequest::parse(method, params.clone()).is_err(),
                "{method} {params}"
            );
        }
    }

    #[test]
    fn processing_requests_use_authenticated_transport_and_report_older_apps() {
        let (directory, core, _) = fixture();
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let received = calls.clone();
        let server = Server::start_with_processing(
            directory.path(),
            core.clone(),
            |_| panic!("not a core mutation"),
            |_| panic!("not a job request"),
            || Err(ControlError::unavailable()),
            |_| Err(ControlError::invalid()),
            move |request| {
                received.lock().unwrap().push(request);
                Ok(json!({"applied":true}))
            },
        )
        .unwrap();
        let audio = std::env::temp_dir().join("synthetic.wav");
        let params = json!({"path":audio.to_str().unwrap()});
        assert_eq!(
            request(directory.path(), "meeting_import", params.clone()).unwrap(),
            json!({"applied":true})
        );
        assert_eq!(
            request(
                directory.path(),
                "meeting_import",
                json!({"path":"relative.wav"})
            )
            .unwrap_err()
            .code,
            "invalid_input"
        );
        assert!(matches!(
            calls.lock().unwrap().as_slice(),
            [ProcessingRequest::MeetingImport {
                meeting_id: None,
                ..
            }]
        ));
        drop(server);
        let _older = Server::start_with_updates(
            directory.path(),
            core,
            |_| panic!("not a core mutation"),
            |_| panic!("not a job request"),
            || Err(ControlError::unavailable()),
            |_| Err(ControlError::invalid()),
        )
        .unwrap();
        for (method, params) in [
            ("meeting_import", params),
            ("speech_status", json!({})),
            ("speech_engine_set", json!({"engine":"nemotron"})),
        ] {
            assert_eq!(
                request(directory.path(), method, params).unwrap_err().code,
                "app_feature_unsupported"
            );
        }
        assert!(!directory.path().join("models").exists());
    }

    #[test]
    fn metadata_compare_and_swap_preserves_newer_edits_and_partial_fields() {
        let (_directory, core, meeting) = fixture();
        let version = meeting_edit_version(&meeting).unwrap();
        let request = Mutation::parse(
            "meeting_update",
            json!({"meetingId":meeting.id,"expectedVersion":version,"title":"변경한 제목"}),
        )
        .unwrap();
        let response = core.agent_mutate(request.clone()).unwrap();
        assert_eq!(response["applied"], true);
        assert!(response.get("snapshot").is_none());
        assert!(response.get("notes").is_none());
        assert!(matches!(
            core.agent_mutate(request),
            Err(CoreError::EditConflict)
        ));
        let current = core.list_meetings().unwrap().remove(0);
        assert_eq!(current.title, "변경한 제목");
        assert_eq!(current.notes, "원래 메모");
        assert_eq!(
            response["editVersion"],
            meeting_edit_version(&current).unwrap()
        );
    }

    #[test]
    fn metadata_edit_version_ignores_hydrated_diagnostics_but_protects_edits_and_sources() {
        let (_directory, _core, meeting) = fixture();
        let version = meeting_edit_version(&meeting).unwrap();
        let mut hydrated = meeting.clone();
        hydrated.warning = Some("Synthetic diagnostic".into());
        hydrated.capture_quality = Some(crate::CaptureQuality {
            xrun_count: 1,
            ..Default::default()
        });
        hydrated.recorded_at = Some(1234);
        assert_eq!(meeting_edit_version(&hydrated).unwrap(), version);

        let mut changes = Vec::new();
        let mut changed = meeting.clone();
        changed.title.push_str(" changed");
        changes.push(changed);
        let mut changed = meeting.clone();
        changed.notes.push_str(" changed");
        changes.push(changed);
        let mut changed = meeting.clone();
        changed.meeting_date = Some("2026-10-29".into());
        changes.push(changed);
        let mut changed = meeting.clone();
        changed.folder_id = Some(uuid::Uuid::new_v4().to_string());
        changes.push(changed);
        let mut changed = meeting.clone();
        changed.deleted_at = Some(1234);
        changes.push(changed);
        let mut changed = meeting.clone();
        changed.transcript_id = Some(uuid::Uuid::new_v4().to_string());
        changes.push(changed);
        let mut changed = meeting.clone();
        changed.audio = Some(crate::AudioArtifact {
            session_id: uuid::Uuid::new_v4().to_string(),
            relative_path: "synthetic/audio.wav".into(),
            frames: 16000,
            sample_rate: 16000,
            channels: 1,
            playback: None,
        });
        changes.push(changed);
        for changed in changes {
            assert_ne!(meeting_edit_version(&changed).unwrap(), version);
        }
    }

    #[test]
    fn batch_validation_precedes_changes_and_trash_preserves_audio_files() {
        let (directory, core, one) = fixture();
        let two = core.create_meeting("다른 회의", "").unwrap();
        let sentinel = directory
            .path()
            .join("meetings")
            .join(&one.id)
            .join("original.pcm");
        std::fs::write(&sentinel, b"original audio").unwrap();
        let mut versions = BTreeMap::from([
            (one.id.clone(), meeting_edit_version(&one).unwrap()),
            (two.id.clone(), format!("sha256:{}", "0".repeat(64))),
        ]);
        let invalid =
            Mutation::parse("meetings_trash", json!({"expectedVersions":versions})).unwrap();
        assert!(matches!(
            core.agent_mutate(invalid),
            Err(CoreError::EditConflict)
        ));
        assert!(core
            .list_meetings()
            .unwrap()
            .iter()
            .all(|meeting| meeting.deleted_at.is_none()));
        versions.remove(&two.id);
        let result = core
            .agent_mutate(
                Mutation::parse("meetings_trash", json!({"expectedVersions":versions})).unwrap(),
            )
            .unwrap();
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"original audio");
        let updated = result["meetings"][0]["editVersion"].as_str().unwrap();
        core.agent_mutate(
            Mutation::parse(
                "meetings_restore",
                json!({"expectedVersions":{one.id.clone():updated}}),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(core
            .list_meetings()
            .unwrap()
            .iter()
            .all(|meeting| meeting.deleted_at.is_none()));
    }

    #[test]
    fn library_edits_use_the_reader_version_and_do_not_delete_meetings() {
        let (directory, core, meeting) = fixture();
        let initial = folder_version(directory.path());
        let created = core
            .agent_mutate(
                Mutation::parse(
                    "folder_create",
                    json!({"expectedVersion":initial,"name":"제품"}),
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(created["editVersion"], folder_version(directory.path()));
        assert!(matches!(
            core.agent_mutate(
                Mutation::parse(
                    "folder_create",
                    json!({"expectedVersion":initial,"name":"운영"})
                )
                .unwrap()
            ),
            Err(CoreError::EditConflict)
        ));
        let folder = created["folder"]["id"].as_str().unwrap();
        core.move_meetings(vec![meeting.id.clone()], Some(folder.into()))
            .unwrap();
        core.agent_mutate(
            Mutation::parse(
                "folder_delete",
                json!({"expectedVersion":folder_version(directory.path()),"folderId":folder}),
            )
            .unwrap(),
        )
        .unwrap();
        let current = core.list_meetings().unwrap();
        assert_eq!(current.len(), 1);
        assert!(current[0].folder_id.is_none());
        assert!(current[0].deleted_at.is_none());
    }

    #[test]
    fn invalid_requests_do_not_reach_the_writer() {
        let id = uuid::Uuid::new_v4().to_string();
        let version = format!("sha256:{}", "0".repeat(64));
        for (method, params) in [
            (
                "meetings_purge",
                json!({"expectedVersions":{id.clone():version}}),
            ),
            (
                "meeting_update",
                json!({"meetingId":id,"expectedVersion":version}),
            ),
            (
                "meeting_update",
                json!({"meetingId":id,"expectedVersion":version,"title":"제목","unknown":true}),
            ),
            (
                "meeting_date_set",
                json!({"meetingId":id,"expectedVersion":version}),
            ),
            (
                "meeting_date_set",
                json!({"meetingId":id,"expectedVersion":version,"meetingDate":"2026-99-01"}),
            ),
            (
                "meetings_move",
                json!({"expectedVersions":{id.clone():version}}),
            ),
            ("meetings_trash", json!({"expectedVersions":{}})),
            (
                "folder_create",
                json!({"expectedVersion":"old","name":"제품"}),
            ),
        ] {
            assert!(Mutation::parse(method, params).is_err(), "{method}");
        }
        assert!(Mutation::parse(
            "meeting_date_set",
            json!({"meetingId":id,"expectedVersion":version,"meetingDate":null})
        )
        .is_ok());
    }

    #[test]
    fn ipc_authentication_generation_and_shutdown_are_bounded() {
        let (directory, core, meeting) = fixture();
        let calls = Arc::new(AtomicUsize::new(0));
        let invoked = calls.clone();
        let writer = core.clone();
        let server = Server::start(directory.path(), core.clone(), move |mutation| {
            invoked.fetch_add(1, Ordering::SeqCst);
            writer.agent_mutate(mutation).map_err(ControlError::core)
        })
        .unwrap();
        let descriptor = read_descriptor(directory.path()).unwrap();
        let params = json!({"meetingId":meeting.id,"expectedVersion":meeting_edit_version(&meeting).unwrap(),"notes":"에이전트 메모"});
        for (secret, generation) in [
            ("invalid".to_owned(), descriptor.generation.clone()),
            (descriptor.secret.clone(), uuid::Uuid::new_v4().to_string()),
        ] {
            let mut stream = TcpStream::connect(descriptor.address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            write_frame(
                &mut stream,
                &Envelope {
                    protocol: PROTOCOL,
                    generation,
                    secret,
                    method: "meeting_update".into(),
                    params: params.clone(),
                },
            )
            .unwrap();
            let response: Value =
                serde_json::from_slice(&read_frame(&mut stream).unwrap()).unwrap();
            assert!(response.get("error").is_some());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let response = request(directory.path(), "meeting_update", params).unwrap();
        assert_eq!(response["applied"], true);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(server);
        assert!(!directory.path().join(DESCRIPTOR).exists());
        assert_eq!(request(directory.path(),"meeting_update",json!({"meetingId":meeting.id,"expectedVersion":meeting_edit_version(&meeting).unwrap(),"title":"다시"})).unwrap_err().code,"app_not_running");
    }

    #[test]
    fn shutdown_does_not_remove_a_different_descriptor_generation() {
        let (directory, core, _) = fixture();
        let server = Server::start(directory.path(), core, |_| Ok(json!({}))).unwrap();
        let mut descriptor = read_descriptor(directory.path()).unwrap();
        descriptor.generation = uuid::Uuid::new_v4().to_string();
        let path = directory.path().join(DESCRIPTOR);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        serde_json::to_writer(file, &descriptor).unwrap();
        drop(server);
        assert!(path.exists());
    }

    #[test]
    fn missing_app_and_sensitive_errors_do_not_create_or_disclose_control_data() {
        let directory = temporary_directory();
        let absent = directory.path().join("absent");
        assert!(read_descriptor(&absent).is_err());
        assert!(!absent.exists());
        let private = "secret-token-at-/private/path";
        for error in [
            CoreError::Storage(private.into()),
            CoreError::InvalidState(private.into()),
            CoreError::InvalidInput(private.into()),
        ] {
            assert!(!ControlError::core(error).message.contains(private));
        }
    }

    #[cfg(unix)]
    #[test]
    fn linked_and_public_control_files_are_rejected() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let (directory, core, _) = fixture();
        let server = Server::start(directory.path(), core, |_| Ok(json!({}))).unwrap();
        let path = directory.path().join(DESCRIPTOR);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            read_descriptor(directory.path()).err().unwrap().code,
            "unsafe_control_path"
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(server);
        let external = tempfile::NamedTempFile::new().unwrap();
        symlink(external.path(), &path).unwrap();
        assert!(read_descriptor(directory.path()).is_err());
    }
}
