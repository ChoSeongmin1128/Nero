use crate::engine::Engine;
use crate::library::Library;
use crate::{
    store, CoreError, CoreHandle, ExternalSummaryProducer, Meeting, Result, SummaryContext,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const PROTOCOL: u32 = 1;
const MAX_FRAME: usize = 1024 * 1024;
const DESCRIPTOR: &str = "agent-control.json";

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
        Self::new(
            "mutation_result_unknown",
            "수정 결과를 확인하지 못했습니다. 같은 요청을 반복하기 전에 현재 상태를 조회해 주세요.",
        )
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
            CoreError::EditConflict => {
                "다른 변경이 저장되었습니다. 최신 정보를 읽고 다시 시도해 주세요."
            }
            CoreError::MeetingNotFound => "회의를 찾을 수 없습니다.",
            CoreError::InvalidInput(_) => "수정 요청의 값이나 대상이 올바르지 않습니다.",
            CoreError::RecordingBusy | CoreError::StaleSession => {
                "녹음을 마친 뒤 다시 시도해 주세요."
            }
            CoreError::InvalidState(_) => {
                "회의 또는 처리 상태가 바뀌어 수정하지 못했습니다. 현재 상태를 확인해 주세요."
            }
            CoreError::Unavailable | CoreError::StoreBusy => {
                "Nero 앱이 수정 요청을 처리할 수 없습니다."
            }
            _ => "변경을 저장하지 못했습니다. Nero 앱에서 저장 상태를 확인해 주세요.",
        };
        Self::new(code, message)
    }
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

pub const METHODS: [&str; 9] = [
    "summary_import",
    "meeting_update",
    "meeting_date_set",
    "meetings_move",
    "meetings_trash",
    "meetings_restore",
    "folder_create",
    "folder_rename",
    "folder_delete",
];

pub const JOB_METHODS: [&str; 2] = ["jobs_list", "job_cancel"];

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

enum ControlRequest {
    Mutation(Mutation),
    Job(JobRequest),
}
impl ControlRequest {
    fn parse(method: &str, params: Value) -> std::result::Result<Self, ControlError> {
        if JOB_METHODS.contains(&method) {
            JobRequest::parse(method, params).map(Self::Job)
        } else {
            Mutation::parse(method, params).map(Self::Mutation)
        }
    }
}

fn valid_version(version: &str) -> bool {
    version
        .strip_prefix("sha256:")
        .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

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
            Self::FolderCreate { .. } | Self::FolderRename { .. } | Self::FolderDelete { .. }
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
            Self::FolderCreate {
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
            Self::MeetingUpdate {
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
            Self::FolderCreate { .. } | Self::FolderRename { .. } | Self::FolderDelete { .. } => {
                Vec::new()
            }
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

fn fingerprint(value: &impl Serialize) -> Result<String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| CoreError::InvalidState("수정 버전을 확인하지 못했습니다.".into()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

pub fn meeting_edit_version(meeting: &Meeting) -> Result<String> {
    // These fields can be hydrated from a capture manifest without editing the
    // meeting document. They must not invalidate a reader's metadata edit token.
    // Keep source/audio identities, status and all editable fields protected.
    let mut protected = meeting.clone();
    protected.warning = None;
    protected.capture_quality = None;
    protected.recorded_at = None;
    fingerprint(&protected)
}
pub(crate) fn library_edit_version(library: &Library) -> Result<String> {
    fingerprint(library)
}

pub(crate) fn validate(engine: &Engine, mutation: &Mutation) -> Result<()> {
    if engine.quitting {
        return Err(CoreError::Unavailable);
    }
    match mutation {
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
                let current = engine.meetings.get(id).ok_or(CoreError::MeetingNotFound)?;
                if meeting_edit_version(&current.meeting)? != expected {
                    return Err(CoreError::EditConflict);
                }
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
    if value.protocol != PROTOCOL
        || *value.address.ip() != Ipv4Addr::LOCALHOST
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
        .set_read_timeout(Some(Duration::from_secs(45)))
        .map_err(|_| ControlError::unavailable())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| ControlError::unavailable())?;
    write_frame(&mut stream, &envelope).map_err(|_| ControlError::unknown_result())?;
    let bytes = read_frame(&mut stream).map_err(|_| ControlError::unknown_result())?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| ControlError::unknown_result())?;
    if let Some(error) = value.get("error") {
        let (code, message) = match error["code"].as_str().unwrap_or("") {
            "edit_conflict" => (
                "edit_conflict",
                "다른 변경이 저장되었습니다. 최신 정보를 읽고 다시 시도해 주세요.",
            ),
            "invalid_input" => (
                "invalid_input",
                "수정 요청의 값이나 대상이 올바르지 않습니다.",
            ),
            "meeting_not_found" => ("meeting_not_found", "회의를 찾을 수 없습니다."),
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
            _ => ("mutation_failed", "수정을 완료하지 못했습니다."),
        };
        return Err(ControlError::new(code, message));
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(ControlError::unknown_result)
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

pub struct Server {
    stopping: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    root: PathBuf,
    descriptor: Descriptor,
    _writer: CoreHandle,
}

impl Server {
    pub fn start(
        root: &Path,
        core: CoreHandle,
        handler: impl Fn(Mutation) -> std::result::Result<Value, ControlError> + Send + 'static,
    ) -> std::result::Result<Self, ControlError> {
        Self::start_with_jobs(root, core, handler, |_| Err(ControlError::invalid()))
    }

    pub fn start_with_jobs(
        root: &Path,
        core: CoreHandle,
        handler: impl Fn(Mutation) -> std::result::Result<Value, ControlError> + Send + 'static,
        jobs: impl Fn(JobRequest) -> std::result::Result<Value, ControlError> + Send + 'static,
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
                while !stopped.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, peer)) => {
                            if !peer.ip().is_loopback() {
                                continue;
                            }
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                            let result = read_frame(&mut stream)
                                .ok()
                                .and_then(|bytes| serde_json::from_slice::<Envelope>(&bytes).ok())
                                .filter(|value| {
                                    value.protocol == PROTOCOL
                                        && value.generation == expected.generation
                                        && matches_secret(&expected.secret, &value.secret)
                                })
                                .ok_or_else(|| {
                                    ControlError::new(
                                        "app_not_ready",
                                        "Nero 수정 연결을 다시 확인해 주세요.",
                                    )
                                })
                                .and_then(|value| {
                                    ControlRequest::parse(&value.method, value.params)
                                })
                                .and_then(|request| match request {
                                    ControlRequest::Mutation(request) => handler(request),
                                    ControlRequest::Job(request) => jobs(request),
                                });
                            let response = match result {
                                Ok(value) => json!({"result":value}),
                                Err(error) => json!({"error":error}),
                            };
                            let _ = write_frame(&mut stream, &response);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(20))
                        }
                        Err(_) => break,
                    }
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
