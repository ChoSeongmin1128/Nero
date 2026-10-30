use crate::library::Library;
use crate::presentation::{format_summary, DocumentFormat};
use crate::store::{self, StoredMeeting};
use crate::{CoreError, Meeting, Result, Summary, SummaryContext, Transcript};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const API_VERSION: u32 = 1;
const MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_TEXT_CHUNK_BYTES: usize = 4096;
const MAX_ATTEMPTS: usize = 3;

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("조회 항목 하나 또는 응답 정보가 maxBytes 한도를 넘습니다. 한도를 늘리거나 limit 또는 음성 구간을 줄여 다시 조회해 주세요.")]
    ResponseTooLarge,
    #[error("조회 중 저장 정보가 변경되었습니다. 잠시 후 다시 조회해 주세요.")]
    SnapshotChanged,
    #[error("요청한 sourceVersion과 현재 저장 정보가 다릅니다. 이전 페이지와 합치지 말고 처음부터 다시 조회해 주세요.")]
    SourceVersionMismatch,
    #[error("저장 정보의 JSON이 손상되었거나 필드/연결이 누락되었거나 올바르지 않습니다.")]
    InvalidDocument,
    #[error("지원하지 않는 저장 형식입니다. Nero 앱과 CLI 업데이트가 필요합니다.")]
    UnsupportedFormat,
    #[error("저장 정보가 읽기 한도 16MiB를 넘습니다.")]
    DocumentTooLarge,
    #[error("저장소 경로는 연결되지 않은 일반 폴더여야 합니다.")]
    UnsafePath,
}

pub struct LibraryReader {
    root: PathBuf,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    meeting_id: Option<String>,
    query: Option<String>,
    folder_id: Option<String>,
    unfiled: Option<bool>,
    trashed: Option<bool>,
    from: Option<u64>,
    to: Option<u64>,
    offset: Option<usize>,
    limit: Option<usize>,
    start: Option<f64>,
    end: Option<f64>,
    expected_source_version: Option<String>,
    max_bytes: Option<usize>,
    format: Option<DocumentFormat>,
    include_citations: Option<bool>,
}

impl Params {
    fn parse(method: &str, value: Value) -> Result<Self> {
        let allowed: &[&str] = match method {
            "meetings_list" => &[
                "folderId", "unfiled", "trashed", "from", "to", "offset", "limit",
            ],
            "meetings_search" => &[
                "query", "folderId", "unfiled", "from", "to", "offset", "limit",
            ],
            "meeting_get" => &["meetingId", "offset", "limit"],
            "summary_get" => &["meetingId", "offset", "limit", "format", "includeCitations"],
            "summary_context" => &["meetingId"],
            "transcript_get" => &["meetingId", "start", "end", "offset", "limit"],
            "folders_list" | "people_catalog" => &["offset", "limit"],
            "status" => &[],
            _ => {
                return Err(CoreError::InvalidInput(
                    "지원하지 않는 조회 명령입니다.".into(),
                ))
            }
        };
        let object = value.as_object().ok_or_else(|| {
            CoreError::InvalidInput("조회 매개변수는 JSON 객체여야 합니다.".into())
        })?;
        if object.keys().any(|key| {
            !allowed.contains(&key.as_str())
                && !(method != "status"
                    && matches!(key.as_str(), "expectedSourceVersion" | "maxBytes"))
        }) {
            return Err(CoreError::InvalidInput(
                "이 조회에서 지원하지 않는 매개변수가 있습니다.".into(),
            ));
        }
        let params: Self = serde_json::from_value(value).map_err(|_| {
            CoreError::InvalidInput("조회 매개변수의 형식이 올바르지 않습니다.".into())
        })?;
        if params.limit.is_some_and(|limit| limit == 0 || limit > 200) {
            return Err(CoreError::InvalidInput(
                "limit은 1 이상 200 이하여야 합니다.".into(),
            ));
        }
        if params
            .max_bytes
            .is_some_and(|bytes| bytes == 0 || bytes > MAX_RESPONSE_BYTES)
        {
            return Err(CoreError::InvalidInput(
                "maxBytes는 1 이상 1048576 이하여야 합니다.".into(),
            ));
        }
        if params
            .expected_source_version
            .as_ref()
            .is_some_and(|version| {
                version.strip_prefix("sha256:").is_none_or(|digest| {
                    digest.len() != 64
                        || !digest
                            .bytes()
                            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
                })
            })
        {
            return Err(CoreError::InvalidInput(
                "expectedSourceVersion은 조회 응답의 sourceVersion 문자열이어야 합니다.".into(),
            ));
        }
        if params
            .from
            .zip(params.to)
            .is_some_and(|(from, to)| from >= to)
        {
            return Err(CoreError::InvalidInput(
                "from은 to보다 작아야 합니다.".into(),
            ));
        }
        for value in [params.start, params.end].into_iter().flatten() {
            if !value.is_finite() || value < 0.0 {
                return Err(CoreError::InvalidInput(
                    "음성 구간은 0 이상의 유한한 초 단위 값이어야 합니다.".into(),
                ));
            }
        }
        if params
            .start
            .zip(params.end)
            .is_some_and(|(start, end)| start >= end)
        {
            return Err(CoreError::InvalidInput(
                "start는 end보다 작아야 합니다.".into(),
            ));
        }
        if params.unfiled == Some(true) && params.folder_id.is_some() {
            return Err(CoreError::InvalidInput(
                "미분류와 특정 폴더를 동시에 선택할 수 없습니다.".into(),
            ));
        }
        if let Some(id) = &params.folder_id {
            store::validate_id(id)?;
        }
        if matches!(
            method,
            "meeting_get" | "transcript_get" | "summary_get" | "summary_context"
        ) {
            store::validate_id(
                params
                    .meeting_id
                    .as_deref()
                    .ok_or_else(|| CoreError::InvalidInput("meetingId가 필요합니다.".into()))?,
            )?;
        }
        if method == "meetings_search" {
            let query = params.query.as_deref().unwrap_or_default();
            if query.trim().is_empty() || query.len() > 4096 || query.contains('\0') {
                return Err(CoreError::InvalidInput(
                    "검색어는 비어 있지 않은 4096바이트 이하 문자열이어야 합니다.".into(),
                ));
            }
        }
        Ok(params)
    }

    fn page(&self, total: usize, unit: &str) -> Value {
        self.page_with_count(
            total,
            unit,
            total
                .saturating_sub(self.offset.unwrap_or(0))
                .min(self.limit.unwrap_or(50)),
        )
    }

    fn page_with_count(&self, total: usize, unit: &str, returned: usize) -> Value {
        let offset = self.offset.unwrap_or(0);
        let limit = self.limit.unwrap_or(50);
        let next = offset.saturating_add(returned);
        json!({"offset": offset, "limit": limit, "total": total,
            "returned": returned, "nextOffset": (next < total).then_some(next),
            "hasMore": next < total, "unit": unit})
    }

    fn select<T>(&self, values: impl IntoIterator<Item = T>) -> Vec<T> {
        values
            .into_iter()
            .skip(self.offset.unwrap_or(0))
            .take(self.limit.unwrap_or(50))
            .collect()
    }
}

impl LibraryReader {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let root = if root.is_absolute() {
            root.to_path_buf()
        } else {
            std::env::current_dir()?.join(root)
        };
        validate_root(&root)?;
        let root = if root.exists() {
            root.canonicalize()?
        } else {
            root
        };
        Ok(Self { root })
    }

    pub fn query(&self, method: &str, params: Value) -> Result<Value> {
        let params = Params::parse(method, params)?;
        validate_root(&self.root)?;
        if method == "status" {
            return Ok(json!({"apiVersion": API_VERSION, "readOnly": true,
                "rootExists": self.root.is_dir()}));
        }
        if !self.root.is_dir() {
            return Err(CoreError::Storage(
                "Nero 저장소가 없습니다. 앱에서 회의 기록을 먼저 저장해 주세요.".into(),
            ));
        }
        for _ in 0..MAX_ATTEMPTS {
            let mut transaction = Transaction::new(&self.root);
            let result = transaction.query(method, &params);
            if !transaction.verify()? {
                continue;
            }
            let mut result = result?;
            let version = if method == "summary_get" {
                let identity = serde_json::to_vec(&(
                    transaction.version(),
                    params.format.unwrap_or_default(),
                    params.include_citations.unwrap_or(false),
                ))
                .map_err(|error| CoreError::Storage(error.to_string()))?;
                format!("sha256:{:x}", Sha256::digest(identity))
            } else {
                transaction.version()
            };
            if params
                .expected_source_version
                .as_ref()
                .is_some_and(|expected| expected != &version)
            {
                return Err(ReadError::SourceVersionMismatch.into());
            }
            result["apiVersion"] = json!(API_VERSION);
            result["sourceVersion"] = json!(version);
            return fit_response(result, method, &params);
        }
        Err(ReadError::SnapshotChanged.into())
    }
}

fn response_bytes(value: &Value) -> Result<usize> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|_| CoreError::Storage("조회 응답을 만들 수 없습니다.".into()))
}

fn response_prefix(value: &Value, method: &str, params: &Params, count: usize) -> Value {
    let mut prefix = value.clone();
    match method {
        "people_catalog" => prefix["entries"].as_array_mut().unwrap().truncate(count),
        "folders_list" => prefix["folders"].as_array_mut().unwrap().truncate(count),
        "meetings_list" | "meetings_search" => {
            prefix["meetings"].as_array_mut().unwrap().truncate(count)
        }
        "transcript_get" => prefix["segments"].as_array_mut().unwrap().truncate(count),
        "meeting_get" => {
            prefix["meeting"]["notes"] =
                json!(text_chunks(value["meeting"]["notes"].as_str().unwrap())
                    .into_iter()
                    .take(count)
                    .collect::<String>());
        }
        "summary_get" => {
            prefix["summary"]["text"] =
                json!(text_chunks(value["summary"]["text"].as_str().unwrap())
                    .into_iter()
                    .take(count)
                    .collect::<String>());
        }
        _ => unreachable!(),
    }
    prefix["pagination"] = params.page_with_count(
        value["pagination"]["total"].as_u64().unwrap() as usize,
        value["pagination"]["unit"].as_str().unwrap(),
        count,
    );
    prefix
}

fn fit_response(value: Value, method: &str, params: &Params) -> Result<Value> {
    let budget = params.max_bytes.unwrap_or(MAX_RESPONSE_BYTES);
    if response_bytes(&value)? <= budget {
        return Ok(value);
    }
    if method == "summary_context" {
        return Err(ReadError::ResponseTooLarge.into());
    }
    let returned = value["pagination"]["returned"].as_u64().unwrap() as usize;
    let mut minimum = 1;
    let mut maximum = returned.saturating_sub(1);
    let mut accepted = None;
    while minimum <= maximum {
        let count = minimum + (maximum - minimum) / 2;
        let candidate = response_prefix(&value, method, params, count);
        if response_bytes(&candidate)? <= budget {
            accepted = Some(count);
            minimum = count + 1;
        } else {
            maximum = count - 1;
        }
    }
    accepted
        .map(|count| response_prefix(&value, method, params, count))
        .ok_or_else(|| ReadError::ResponseTooLarge.into())
}

#[derive(Clone, PartialEq, Eq)]
enum Observation {
    File(Option<[u8; 32]>),
    Directory(Vec<String>),
    DirectoryPresence(bool),
}

struct Transaction<'a> {
    root: &'a Path,
    observations: BTreeMap<PathBuf, Observation>,
}

impl<'a> Transaction<'a> {
    fn new(root: &'a Path) -> Self {
        Self {
            root,
            observations: BTreeMap::new(),
        }
    }

    fn document<T: DeserializeOwned>(
        &mut self,
        relative: &Path,
        schema: bool,
    ) -> Result<Option<T>> {
        let content = self.content(relative)?;
        let Some(content) = content else {
            return Ok(None);
        };
        let value: Value =
            serde_json::from_slice(&content).map_err(|_| ReadError::InvalidDocument)?;
        if schema {
            match value.get("schemaVersion").and_then(Value::as_u64) {
                Some(1) => (),
                Some(_) => return Err(ReadError::UnsupportedFormat.into()),
                None => return Err(ReadError::InvalidDocument.into()),
            }
        }
        serde_json::from_value(value)
            .map(Some)
            .map_err(|_| ReadError::InvalidDocument.into())
    }

    fn content(&mut self, relative: &Path) -> Result<Option<Vec<u8>>> {
        let content = read_content(self.root, relative)?;
        self.observations.insert(
            relative.to_path_buf(),
            Observation::File(content.as_ref().map(|bytes| Sha256::digest(bytes).into())),
        );
        Ok(content)
    }

    fn directory(&mut self, relative: &Path) -> Result<Vec<String>> {
        let names = read_directory(self.root, relative)?;
        self.observations.insert(
            relative.to_path_buf(),
            Observation::Directory(names.clone()),
        );
        Ok(names)
    }

    fn directory_presence(&mut self, relative: &Path) -> Result<bool> {
        let present = read_directory_presence(self.root, relative)?;
        self.observations.insert(
            relative.to_path_buf(),
            Observation::DirectoryPresence(present),
        );
        Ok(present)
    }

    fn verify(&self) -> Result<bool> {
        for (relative, before) in &self.observations {
            let after = match before {
                Observation::File(_) => Observation::File(
                    read_content(self.root, relative)?
                        .as_ref()
                        .map(|bytes| Sha256::digest(bytes).into()),
                ),
                Observation::Directory(_) => match read_directory(self.root, relative) {
                    Ok(names) => Observation::Directory(names),
                    Err(_) => return Ok(false),
                },
                Observation::DirectoryPresence(_) => {
                    Observation::DirectoryPresence(read_directory_presence(self.root, relative)?)
                }
            };
            if &after != before {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn version(&self) -> String {
        let mut hash = Sha256::new();
        for (path, observation) in &self.observations {
            hash.update(path.to_string_lossy().as_bytes());
            hash.update([0]);
            match observation {
                Observation::File(Some(digest)) => {
                    hash.update([1]);
                    hash.update(digest);
                }
                Observation::File(None) => hash.update([2]),
                Observation::Directory(names) => {
                    hash.update([3]);
                    for name in names {
                        hash.update(name.as_bytes());
                        hash.update([0]);
                    }
                }
                Observation::DirectoryPresence(present) => hash.update([4, u8::from(*present)]),
            }
        }
        format!("sha256:{:x}", hash.finalize())
    }

    fn library(&mut self) -> Result<Library> {
        let library: Library = self
            .document(Path::new("library.json"), true)?
            .unwrap_or_default();
        library.validate().map_err(|_| ReadError::InvalidDocument)?;
        Ok(library)
    }

    fn meeting(
        &mut self,
        id: &str,
        library: &Library,
        capture_details: bool,
    ) -> Result<Option<Meeting>> {
        self.meeting_in_view(id, library, false, capture_details)
    }

    fn meeting_in_view(
        &mut self,
        id: &str,
        library: &Library,
        trashed: bool,
        capture_details: bool,
    ) -> Result<Option<Meeting>> {
        store::validate_id(id)?;
        if library.pending_purges.contains(id)
            || library
                .placements
                .get(id)
                .is_some_and(|placement| placement.deleted_at.is_some())
                != trashed
        {
            return Ok(None);
        }
        let relative = Path::new("meetings").join(id).join("meeting.json");
        let Some(mut stored): Option<StoredMeeting> = self.document(&relative, true)? else {
            if self.directory_presence(&Path::new("meetings").join(id).join("sessions"))? {
                return Err(ReadError::InvalidDocument.into());
            }
            return Ok(None);
        };
        if stored.meeting.id != id {
            return Err(ReadError::InvalidDocument.into());
        }
        store::validate_text(&stored.meeting.title, &stored.meeting.notes)
            .map_err(|_| ReadError::InvalidDocument)?;
        if let Some(id) = &stored.meeting.transcript_id {
            store::validate_id(id).map_err(|_| ReadError::InvalidDocument)?;
        }
        if let Some(audio) = &stored.meeting.audio {
            store::validate_id(&audio.session_id).map_err(|_| ReadError::InvalidDocument)?;
        }
        if capture_details {
            if let Some(capture_id) = &stored.capture_id {
                store::validate_id(capture_id).map_err(|_| ReadError::InvalidDocument)?;
                let relative = Path::new("meetings")
                    .join(id)
                    .join("sessions")
                    .join(capture_id)
                    .join("manifest.json");
                let manifest: store::Manifest = self
                    .document(&relative, true)?
                    .ok_or(ReadError::InvalidDocument)?;
                manifest
                    .validate()
                    .map_err(|_| ReadError::InvalidDocument)?;
                if manifest.meeting_id != id || &manifest.session_id != capture_id {
                    return Err(ReadError::InvalidDocument.into());
                }
                store::hydrate_capture_metadata(&mut stored.meeting, &manifest);
            }
            if let Some(quality) = &stored.meeting.capture_quality {
                quality.validate().map_err(|_| ReadError::InvalidDocument)?;
            }
        }
        library.project(&mut stored.meeting);
        Ok((stored.meeting.deleted_at.is_some() == trashed).then_some(stored.meeting))
    }

    fn transcript(&mut self, meeting: &Meeting) -> Result<Option<Transcript>> {
        let Some(id) = &meeting.transcript_id else {
            return Ok(None);
        };
        let relative = Path::new("meetings")
            .join(&meeting.id)
            .join("transcripts")
            .join(id)
            .join("current.json");
        let document: Transcript = self
            .document(&relative, false)?
            .ok_or(ReadError::InvalidDocument)?;
        if &document.version.id != id
            || meeting
                .audio
                .as_ref()
                .map(|audio| audio.session_id.as_str())
                != Some(document.source_session_id.as_str())
        {
            return Err(ReadError::InvalidDocument.into());
        }
        let mut ids = BTreeSet::new();
        let mut speakers = BTreeSet::new();
        for speaker in &document.speakers {
            if speaker.id.is_empty() || !speakers.insert(&speaker.id) {
                return Err(ReadError::InvalidDocument.into());
            }
        }
        for segment in &document.segments {
            if segment.id.is_empty()
                || !ids.insert(&segment.id)
                || !segment.start.is_finite()
                || !segment.end.is_finite()
                || segment.start < 0.0
                || segment.end < segment.start
                || segment
                    .speaker_id
                    .as_ref()
                    .is_some_and(|id| !speakers.contains(id))
            {
                return Err(ReadError::InvalidDocument.into());
            }
        }
        Ok(Some(document))
    }

    fn summary(
        &mut self,
        meeting: &Meeting,
        transcript: Option<&Transcript>,
    ) -> Result<Option<(Value, Option<usize>)>> {
        let relative = Path::new("meetings")
            .join(&meeting.id)
            .join("summaries/current.json");
        let Some(summary): Option<Summary> = self.document(&relative, false)? else {
            return Ok(None);
        };
        store::validate_id(&summary.id).map_err(|_| ReadError::InvalidDocument)?;
        store::validate_id(&summary.source_version.id).map_err(|_| ReadError::InvalidDocument)?;
        if summary.source_sha256.len() != 64
            || !summary.source_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || summary.attendees_sha256.as_ref().is_some_and(|digest| {
                digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit())
            })
            || summary.notes_sha256.as_ref().is_some_and(|digest| {
                digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit())
            })
        {
            return Err(ReadError::InvalidDocument.into());
        }
        let stale =
            crate::summary::is_stale(&summary, transcript, &meeting.notes, &meeting.attendees)?;
        let reference_count =
            crate::summary::summary_reference_count(&meeting.id, &summary, |relative| {
                self.content(relative)
            });
        Ok(Some((
            json!({"id":summary.id,"text":summary.text,"createdAt":summary.created_at,
            "sourceVersion":summary.source_version,"stale":stale}),
            reference_count,
        )))
    }

    fn query(&mut self, method: &str, params: &Params) -> Result<Value> {
        let library = self.library()?;
        if method == "people_catalog" {
            let catalog = library.people.catalog()?;
            let entries: Vec<_> = catalog
                .people
                .iter()
                .map(|person| json!({"kind":"person","person":person}))
                .chain(
                    catalog
                        .groups
                        .iter()
                        .map(|group| json!({"kind":"group","group":group})),
                )
                .collect();
            return Ok(
                json!({"editVersion":catalog.edit_version,"pagination":params.page(entries.len(),"entries"),"entries":params.select(entries)}),
            );
        }
        if method == "folders_list" {
            let folders = library.folders();
            return Ok(
                json!({"editVersion":crate::agent_control::library_edit_version(&library)?,"pagination":params.page(folders.len(), "folders"),"folders":params.select(folders)}),
            );
        }
        if matches!(method, "meetings_list" | "meetings_search") {
            return self.list(method, params, &library);
        }
        let id = params.meeting_id.as_deref().unwrap();
        let meeting = self
            .meeting(id, &library, method == "meeting_get")?
            .ok_or(CoreError::MeetingNotFound)?;
        let transcript = self.transcript(&meeting)?;
        match method {
            "meeting_get" => {
                let mut value = meeting_value(&meeting, transcript.as_ref());
                if let Some(quality) = &meeting.capture_quality {
                    quality.validate().map_err(|_| ReadError::InvalidDocument)?;
                    value["captureQuality"] = json!(quality);
                    value["captureQualityMeaning"] = json!({
                        "framePosition":"writerProgressAtObservation",
                        "audioSampleRate":meeting.audio.as_ref().map(|audio| audio.sample_rate),
                        "hardwareEventTimeKnown":false,
                        "affectedAudioRangeKnown":false,
                        "audioLossVerified":false,
                        "gapRecoveryVerified":false,
                        "coalescedRangeIsContinuous":false
                    });
                }
                if let Some(error) = &meeting.error {
                    value["error"] = json!(error);
                }
                let (notes, page) = page_text_chunks(&meeting.notes, params);
                value["notes"] = json!(notes);
                Ok(
                    json!({"editVersion":crate::agent_control::meeting_edit_version(&meeting)?,"meeting":value,"pagination":page}),
                )
            }
            "transcript_get" => {
                let segments = transcript
                    .as_ref()
                    .map(|doc| {
                        doc.segments
                            .iter()
                            .enumerate()
                            .filter(|(_, segment)| {
                                params.start.is_none_or(|start| segment.end > start)
                                    && params.end.is_none_or(|end| segment.start < end)
                            })
                            .map(|(index, segment)| {
                                json!({"reference":index + 1,"id":segment.id,"start":segment.start,"end":segment.end,
                                "speakerId":segment.speaker_id,"text":segment.text})
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                Ok(
                    json!({"meetingId":id,"version":transcript.as_ref().map(|doc| &doc.version),
                    "speakers":transcript.as_ref().map(|doc| doc.speakers.clone()).unwrap_or_default(),
                    "pagination":params.page(segments.len(), "segments"),"segments":params.select(segments)}),
                )
            }
            "summary_get" => {
                let mut summary = self.summary(&meeting, transcript.as_ref())?;
                let formatted = summary.as_ref().map(|(summary, reference_count)| {
                    format_summary(
                        summary["text"].as_str().unwrap(),
                        params.format.unwrap_or_default(),
                        params.include_citations.unwrap_or(false),
                        summary["stale"] == true,
                        *reference_count,
                    )
                });
                let (text, page) =
                    page_text_chunks(formatted.as_deref().unwrap_or_default(), params);
                if let Some((summary, _)) = &mut summary {
                    summary["text"] = json!(text);
                }
                Ok(
                    json!({"meetingId":id,"summary":summary.map(|(summary, _)| summary),"pagination":page,
                    "format":params.format.unwrap_or_default(),"includeCitations":params.include_citations.unwrap_or(false)}),
                )
            }
            "summary_context" => {
                let source = transcript.as_ref().ok_or_else(|| {
                    CoreError::InvalidState("요약할 음성 기록이 없습니다.".into())
                })?;
                let summary = self.summary(&meeting, Some(source))?;
                let summary_id = summary
                    .as_ref()
                    .and_then(|(value, _)| value["id"].as_str())
                    .map(str::to_owned);
                let context =
                    SummaryContext::new(source, &meeting.notes, &meeting.attendees, summary_id)?;
                Ok(
                    json!({"meetingId":id,"context":context,"attendees":meeting.attendees,"attendeesMeaning":"Attendance only; order does not identify transcript speakers or verify voice identity. Never infer speaker names from this roster."}),
                )
            }
            _ => unreachable!(),
        }
    }

    fn list(&mut self, method: &str, params: &Params, library: &Library) -> Result<Value> {
        if params
            .folder_id
            .as_ref()
            .is_some_and(|id| !library.folders.contains_key(id))
        {
            return Err(CoreError::InvalidInput("지정한 폴더가 없습니다.".into()));
        }
        let names = self.directory(Path::new("meetings"))?;
        let query = params
            .query
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        let mut meetings = Vec::new();
        for id in names {
            let Some(meeting) =
                self.meeting_in_view(&id, library, params.trashed == Some(true), false)?
            else {
                continue;
            };
            let time = meeting.recorded_at.unwrap_or(meeting.created_at);
            if params
                .folder_id
                .as_ref()
                .is_some_and(|folder| meeting.folder_id.as_ref() != Some(folder))
                || (params.unfiled == Some(true) && meeting.folder_id.is_some())
                || params.from.is_some_and(|from| time < from)
                || params.to.is_some_and(|to| time >= to)
            {
                continue;
            }
            let trashed = params.trashed == Some(true);
            let transcript = if trashed {
                None
            } else {
                self.transcript(&meeting)?
            };
            let mut value = meeting_value(&meeting, transcript.as_ref());
            value["editVersion"] = json!(crate::agent_control::meeting_edit_version(&meeting)?);
            if trashed {
                value["deletedAt"] = json!(meeting.deleted_at);
                value.as_object_mut().unwrap().remove("transcriptVersion");
            }
            if method == "meetings_search" {
                let summary = self.summary(&meeting, transcript.as_ref())?;
                let mut matches = Vec::new();
                let mut matches_truncated = false;
                for (field, text) in [
                    ("title", meeting.title.as_str()),
                    ("notes", meeting.notes.as_str()),
                ] {
                    if let Some(text) = excerpt(text, &query) {
                        matches.push(json!({"field":field,"text":text}));
                    }
                }
                if let Some(doc) = &transcript {
                    for segment in &doc.segments {
                        if let Some(text) = excerpt(&segment.text, &query) {
                            if matches.len() == 20 {
                                matches_truncated = true;
                                break;
                            }
                            matches.push(json!({"field":"transcript","text":text,"segmentId":segment.id,"start":segment.start,"end":segment.end}));
                        }
                    }
                }
                if let Some((summary, reference_count)) = summary {
                    let external_text = format_summary(
                        summary["text"].as_str().unwrap_or_default(),
                        DocumentFormat::Md,
                        false,
                        false,
                        reference_count,
                    );
                    if let Some(text) = excerpt(&external_text, &query) {
                        if matches.len() == 20 {
                            matches_truncated = true;
                        } else {
                            matches.push(json!({"field":"summary","text":text,"stale":summary["stale"],"sourceVersion":summary["sourceVersion"]}));
                        }
                    }
                }
                if matches.is_empty() {
                    continue;
                }
                value["matches"] = json!(matches);
                value["matchesTruncated"] = json!(matches_truncated);
            }
            meetings.push(value);
        }
        let timestamp = |meeting: &Value| {
            meeting["recordedAt"]
                .as_u64()
                .or_else(|| meeting["createdAt"].as_u64())
        };
        meetings.sort_by(|a, b| {
            timestamp(b)
                .cmp(&timestamp(a))
                .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
        });
        Ok(
            json!({"pagination":params.page(meetings.len(),"meetings"),"meetings":params.select(meetings)}),
        )
    }
}

fn meeting_value(meeting: &Meeting, transcript: Option<&Transcript>) -> Value {
    json!({"id":meeting.id,"title":meeting.title,"createdAt":meeting.created_at,"updatedAt":meeting.updated_at,
        "meetingDate":meeting.meeting_date,"recordedAt":meeting.recorded_at,"status":meeting.status,"folderId":meeting.folder_id,
        "attendees":meeting.attendees,"attendeesMeaning":"Attendance only; order does not identify transcript speakers or verify voice identity.",
        "transcriptVersion":transcript.map(|doc| &doc.version)})
}

fn text_chunks(text: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    for mut line in text.split_inclusive('\n') {
        while !line.is_empty() {
            let mut end = line.len().min(MAX_TEXT_CHUNK_BYTES);
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            chunks.push(&line[..end]);
            line = &line[end..];
        }
    }
    chunks
}

fn page_text_chunks(text: &str, params: &Params) -> (String, Value) {
    let chunks = text_chunks(text);
    let page = params.page(chunks.len(), "textChunks");
    (params.select(chunks).concat(), page)
}

fn excerpt(text: &str, query: &str) -> Option<String> {
    text.to_lowercase().find(query)?;
    let chars: Vec<_> = text.chars().collect();
    let mut folded_bytes = 0;
    let offset = text.to_lowercase().find(query)?;
    let start = chars
        .iter()
        .position(|c| {
            folded_bytes += c.to_lowercase().collect::<String>().len();
            folded_bytes > offset
        })
        .unwrap_or(0)
        .saturating_sub(40);
    let end = start.saturating_add(160).min(chars.len());
    Some(format!(
        "{}{}{}",
        if start > 0 { "..." } else { "" },
        chars[start..end].iter().collect::<String>(),
        if end < chars.len() { "..." } else { "" }
    ))
}

fn validate_root(root: &Path) -> Result<()> {
    if !root.is_absolute()
        || root
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(CoreError::InvalidInput(
            "저장소 경로에는 상위 경로나 상대 경로 요소를 사용할 수 없습니다.".into(),
        ));
    }
    let mut current = PathBuf::new();
    for part in root.components() {
        current.push(part.as_os_str());
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                #[cfg(windows)]
                let reparse = {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let reparse = false;
                if metadata.is_symlink() || reparse || !metadata.is_dir() {
                    return Err(ReadError::UnsafePath.into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn read_content(root: &Path, relative: &Path) -> Result<Option<Vec<u8>>> {
    let path = store::checked_path(root, relative)?;
    let file = match store::open_read(&path) {
        Ok(file) => file,
        Err(_)
            if fs::symlink_metadata(&path)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(None)
        }
        Err(error) => return Err(error),
    };
    if file.metadata()?.len() > MAX_DOCUMENT_BYTES {
        return Err(ReadError::DocumentTooLarge.into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_DOCUMENT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(ReadError::DocumentTooLarge.into());
    }
    Ok(Some(bytes))
}

fn read_directory(root: &Path, relative: &Path) -> Result<Vec<String>> {
    let path = store::checked_path(root, relative)?;
    let mut names = Vec::new();
    for entry in fs::read_dir(&path)
        .map_err(|_| CoreError::Storage("회의 목록 폴더가 없거나 읽을 수 없습니다.".into()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || entry.file_type()?.is_symlink() {
            return Err(ReadError::UnsafePath.into());
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| ReadError::InvalidDocument)?;
        store::validate_id(&name).map_err(|_| ReadError::InvalidDocument)?;
        store::checked_path(root, &relative.join(&name))?;
        names.push(name);
    }
    names.sort();
    Ok(names)
}

fn read_directory_presence(root: &Path, relative: &Path) -> Result<bool> {
    let path = store::checked_path(root, relative)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(ReadError::UnsafePath.into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Folder, Placement};
    use crate::{
        AudioArtifact, CoreHandle, MeetingStatus, SummaryProvenance, TranscriptSegment,
        TranscriptSpeaker, TranscriptVersion,
    };
    use tempfile::TempDir;
    use uuid::Uuid;

    struct Fixture {
        _temporary: TempDir,
        root: PathBuf,
        id: String,
        folder: String,
        transcript: Transcript,
    }

    impl Fixture {
        fn new() -> Self {
            let temporary = tempfile::tempdir().unwrap();
            let root = temporary.path().canonicalize().unwrap();
            let store = store::Store::open(root.clone()).unwrap();
            let mut meeting = store
                .create(
                    "Synthetic meeting".into(),
                    "note one\nnote two\nnote three".into(),
                )
                .unwrap();
            let id = meeting.meeting.id.clone();
            let session_id = Uuid::new_v4().to_string();
            let transcript_id = Uuid::new_v4().to_string();
            meeting.meeting.created_at = 1000;
            meeting.meeting.updated_at = 2000;
            meeting.meeting.recorded_at = Some(1500);
            meeting.meeting.status = MeetingStatus::Completed;
            meeting.meeting.transcript_id = Some(transcript_id.clone());
            meeting.meeting.audio = Some(AudioArtifact {
                session_id: session_id.clone(),
                relative_path: format!("meetings/{id}/sessions/{session_id}/audio.wav"),
                frames: 160000,
                sample_rate: 16000,
                channels: 1,
            });
            store.save(&meeting).unwrap();
            let folder = Uuid::new_v4().to_string();
            let mut library = Library::default();
            library.folders.insert(
                folder.clone(),
                Folder {
                    id: folder.clone(),
                    name: "Synthetic folder".into(),
                },
            );
            library.placements.insert(
                id.clone(),
                Placement {
                    folder_id: Some(folder.clone()),
                    deleted_at: None,
                },
            );
            store.save_library(&library).unwrap();
            let transcript = Transcript {
                version: TranscriptVersion {
                    id: transcript_id,
                    revision: 1,
                },
                source_session_id: session_id,
                model: "fixture".into(),
                model_revision: "1".into(),
                source: None,
                speaker_analysis: None,
                speakers: vec![TranscriptSpeaker {
                    id: "speaker-a".into(),
                    name: "Speaker A".into(),
                    name_origin: None,
                }],
                segments: (0..3)
                    .map(|index| TranscriptSegment {
                        id: format!("utterance-{index}"),
                        start: index as f64 * 2.0,
                        end: index as f64 * 2.0 + 1.0,
                        original_text: format!("original {index}"),
                        text: format!("corrected {index}"),
                        speaker_id: Some("speaker-a".into()),
                        source_index: Some(index),
                    })
                    .collect(),
            };
            let fixture = Self {
                _temporary: temporary,
                root,
                id,
                folder,
                transcript,
            };
            fixture.save_transcript(&fixture.transcript);
            fixture.save_summary(&fixture.transcript);
            fixture
        }

        fn reader(&self) -> LibraryReader {
            LibraryReader::open(&self.root).unwrap()
        }

        fn save_transcript(&self, transcript: &Transcript) {
            let directory = Path::new("meetings")
                .join(&self.id)
                .join("transcripts")
                .join(&transcript.version.id);
            fs::create_dir_all(self.root.join(&directory)).unwrap();
            store::write_json(&self.root, &directory.join("current.json"), transcript).unwrap();
        }

        fn save_summary(&self, source: &Transcript) {
            let directory = Path::new("meetings").join(&self.id).join("summaries");
            fs::create_dir_all(self.root.join(&directory)).unwrap();
            let summary = Summary {
                id: Uuid::new_v4().to_string(),
                text: "summary one\nsummary two\nsummary three".into(),
                created_at: 2100,
                source_version: source.version.clone(),
                source_sha256: crate::summary::transcript_digest(source).unwrap(),
                notes_sha256: None,
                attendees_sha256: None,
                provenance: SummaryProvenance {
                    producer: None,
                    model: Some("fixture".into()),
                    model_revision: Some("1".into()),
                    model_sha256: Some("0".repeat(64)),
                    runtime: Some("fixture".into()),
                    prompt_version: Some("1".into()),
                    template: Some("fixture".into()),
                },
            };
            store::write_json(&self.root, &directory.join("current.json"), &summary).unwrap();
        }

        fn set_deleted(&self) {
            let mut library: Library = store::read_json(&self.root.join("library.json")).unwrap();
            library.placements.get_mut(&self.id).unwrap().deleted_at = Some(3000);
            store::write_json(&self.root, Path::new("library.json"), &library).unwrap();
        }
    }

    fn tree_hash(root: &Path) -> String {
        fn collect(root: &Path, at: &Path, values: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(at).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    collect(root, &path, values);
                } else {
                    values.insert(
                        path.strip_prefix(root).unwrap().to_path_buf(),
                        if cfg!(windows)
                            && path.file_name().is_some_and(|name| name == "store.lock")
                        {
                            assert_eq!(fs::metadata(&path).unwrap().len(), 0);
                            Vec::new()
                        } else {
                            fs::read(path).unwrap()
                        },
                    );
                }
            }
        }
        let mut values = BTreeMap::new();
        collect(root, root, &mut values);
        let mut digest = Sha256::new();
        for (path, bytes) in values {
            digest.update(path.to_string_lossy().as_bytes());
            digest.update(bytes);
        }
        format!("{:x}", digest.finalize())
    }

    #[test]
    fn edit_versions_match_writer_and_trash_listing_reads_metadata_only() {
        let fixture = Fixture::new();
        let reader = fixture.reader();
        let store = store::Store::open(fixture.root.clone()).unwrap();
        let stored = store.load().unwrap().remove(&fixture.id).unwrap();
        let mut library = store.load_library().unwrap();
        let mut meeting = stored.meeting.clone();
        library.project(&mut meeting);
        let detail = reader
            .query("meeting_get", json!({"meetingId":fixture.id}))
            .unwrap();
        assert_eq!(
            detail["editVersion"],
            crate::agent_control::meeting_edit_version(&meeting).unwrap()
        );
        let folders = reader.query("folders_list", json!({})).unwrap();
        assert_eq!(
            folders["editVersion"],
            crate::agent_control::library_edit_version(&library).unwrap()
        );
        library.placements.get_mut(&fixture.id).unwrap().deleted_at = Some(4000);
        store.save_library(&library).unwrap();
        library.project(&mut meeting);
        let transcript_path = fixture
            .root
            .join("meetings")
            .join(&fixture.id)
            .join("transcripts")
            .join(&fixture.transcript.version.id)
            .join("current.json");
        fs::write(transcript_path, b"invalid transcript").unwrap();
        assert_eq!(
            reader.query("meetings_list", json!({})).unwrap()["meetings"],
            json!([])
        );
        assert!(reader
            .query("meeting_get", json!({"meetingId":fixture.id}))
            .is_err());
        let trash = reader
            .query("meetings_list", json!({"trashed":true}))
            .unwrap();
        assert_eq!(trash["meetings"].as_array().unwrap().len(), 1);
        assert_eq!(trash["meetings"][0]["deletedAt"], 4000);
        assert_eq!(
            trash["meetings"][0]["editVersion"],
            crate::agent_control::meeting_edit_version(&meeting).unwrap()
        );
        for key in ["notes", "text", "audio", "transcriptVersion"] {
            assert!(trash["meetings"][0].get(key).is_none());
        }
        assert!(reader
            .query("meetings_search", json!({"query":"note", "trashed":true}))
            .is_err());
    }

    #[test]
    fn queries_do_not_write_or_require_the_audio_or_a_store_lock() {
        let fixture = Fixture::new();
        let owner = CoreHandle::open(&fixture.root).unwrap();
        assert!(matches!(
            CoreHandle::open(&fixture.root),
            Err(CoreError::StoreBusy)
        ));
        let before = tree_hash(&fixture.root);
        let reader = fixture.reader();
        for method in ["meetings_list", "folders_list", "status"] {
            reader.query(method, json!({})).unwrap();
        }
        for method in [
            "meeting_get",
            "transcript_get",
            "summary_get",
            "summary_context",
        ] {
            reader
                .query(method, json!({"meetingId":fixture.id}))
                .unwrap();
        }
        reader
            .query("meetings_search", json!({"query":"corrected"}))
            .unwrap();
        assert_eq!(before, tree_hash(&fixture.root));
        drop(owner);
    }

    #[test]
    fn a_missing_root_is_diagnostic_without_initializing_it() {
        let temporary = tempfile::tempdir().unwrap();
        let missing = temporary.path().canonicalize().unwrap().join("missing");
        let reader = LibraryReader::open(&missing).unwrap();
        assert_eq!(
            reader.query("status", json!({})).unwrap()["rootExists"],
            false
        );
        assert!(reader
            .query("meetings_list", json!({}))
            .unwrap_err()
            .to_string()
            .contains("저장소가 없습니다"));
        assert!(!missing.exists());
    }

    #[test]
    fn corrected_segments_speakers_time_ranges_and_pages_are_exposed() {
        let fixture = Fixture::new();
        let result = fixture
            .reader()
            .query(
                "transcript_get",
                json!({"meetingId":fixture.id,"start":1.0,"end":5.0,"limit":1}),
            )
            .unwrap();
        assert_eq!(result["pagination"]["total"], 2);
        assert_eq!(result["pagination"]["hasMore"], true);
        assert_eq!(result["segments"][0]["id"], "utterance-1");
        assert_eq!(result["segments"][0]["reference"], 2);
        assert_eq!(result["segments"][0]["text"], "corrected 1");
        assert_eq!(result["segments"][0]["speakerId"], "speaker-a");
        assert_eq!(result["speakers"][0]["name"], "Speaker A");
        assert!(result["segments"][0].get("speakerName").is_none());
        assert!(result["segments"][0].get("originalText").is_none());
        assert_eq!(result["version"]["revision"], 1);
        assert!(result["sourceVersion"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        let next = fixture
            .reader()
            .query(
                "transcript_get",
                json!({"meetingId":fixture.id,"offset":2,"limit":1}),
            )
            .unwrap();
        assert_eq!(next["segments"][0]["id"], "utterance-2");
        assert_eq!(next["segments"][0]["reference"], 3);
        assert_eq!(next["pagination"]["hasMore"], false);
    }

    #[test]
    fn summary_context_is_compact_and_binds_the_current_transcript_notes_and_summary() {
        let fixture = Fixture::new();
        let reader = fixture.reader();
        let params = json!({"meetingId":fixture.id});
        let before = tree_hash(&fixture.root);
        let result = reader.query("summary_context", params.clone()).unwrap();
        assert_eq!(result["meetingId"], fixture.id);
        assert!(result.get("pagination").is_none());
        assert!(result.get("text").is_none());
        let context: SummaryContext = serde_json::from_value(result["context"].clone()).unwrap();
        assert_eq!(context.transcript_version, fixture.transcript.version);
        assert_eq!(
            context.transcript_sha256,
            crate::summary::transcript_digest(&fixture.transcript).unwrap()
        );
        assert_eq!(
            context.notes_sha256,
            crate::summary::notes_digest("note one\nnote two\nnote three")
        );
        assert!(context.summary_id.is_some());
        assert_eq!(before, tree_hash(&fixture.root));
        assert!(matches!(
            reader.query(
                "summary_context",
                json!({"meetingId":fixture.id,"maxBytes":1})
            ),
            Err(CoreError::Reader(ReadError::ResponseTooLarge))
        ));
        assert!(reader
            .query(
                "summary_context",
                json!({"meetingId":fixture.id,"offset":1})
            )
            .is_err());
        let mut changed = fixture.transcript.clone();
        changed.segments[0].text = "a newer corrected transcript".into();
        fixture.save_transcript(&changed);
        let latest = reader.query("summary_context", params).unwrap();
        assert_ne!(
            result["context"]["transcriptSha256"],
            latest["context"]["transcriptSha256"]
        );
        assert_eq!(
            result["context"]["summaryId"],
            latest["context"]["summaryId"]
        );
        assert!(matches!(
            reader.query(
                "summary_context",
                json!({"meetingId":fixture.id,"expectedSourceVersion":result["sourceVersion"]})
            ),
            Err(CoreError::Reader(ReadError::SourceVersionMismatch))
        ));
    }

    #[test]
    fn legacy_summary_without_frozen_archive_is_read_without_writes_or_citation_guessing() {
        let fixture = Fixture::new();
        let relative = Path::new("meetings")
            .join(&fixture.id)
            .join("summaries/current.json");
        let mut summary: Summary = store::read_json(&fixture.root.join(&relative)).unwrap();
        summary.text = "legacy 결정 [1].".into();
        store::write_json(&fixture.root, &relative, &summary).unwrap();
        let before = tree_hash(&fixture.root);
        let reader = fixture.reader();
        let result = reader
            .query("summary_get", json!({"meetingId":fixture.id}))
            .unwrap();
        assert_eq!(result["summary"]["text"], summary.text);
        let search = reader
            .query("meetings_search", json!({"query":"legacy"}))
            .unwrap();
        assert!(search["meetings"][0]["matches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hit| hit["field"] == "summary" && hit["text"].as_str().unwrap().contains("[1]")));
        assert_eq!(before, tree_hash(&fixture.root));
    }

    #[test]
    fn summary_paging_binds_formatted_representation_and_preserves_saved_sources() {
        let fixture = Fixture::new();
        let mut frozen_source = fixture.transcript.clone();
        for segment in &mut frozen_source.segments {
            segment.id = Uuid::new_v4().to_string();
        }
        fixture.save_transcript(&frozen_source);
        let relative = Path::new("meetings")
            .join(&fixture.id)
            .join("summaries/current.json");
        let mut summary: Summary = store::read_json(&fixture.root.join(&relative)).unwrap();
        summary.source_sha256 = crate::summary::transcript_digest(&frozen_source).unwrap();
        summary.text = format!(
            "## 결정\n\n{} [1]\n\n```text\n[2] **literal**\n```\n",
            "**항목** ".repeat(900)
        );
        store::write_json(&fixture.root, &relative, &summary).unwrap();
        let frozen = Path::new("meetings")
            .join(&fixture.id)
            .join("summaries")
            .join(&summary.id)
            .join("run.json");
        fs::create_dir_all(fixture.root.join(frozen.parent().unwrap())).unwrap();
        store::write_json(
            &fixture.root,
            &frozen,
            &json!({"document":summary,"source":frozen_source,"receipt":{}}),
        )
        .unwrap();
        let reader = fixture.reader();
        let before = tree_hash(&fixture.root);
        let first = reader
            .query("summary_get", json!({"meetingId":fixture.id,"limit":1}))
            .unwrap();
        assert_eq!(first["format"], "md");
        assert_eq!(first["includeCitations"], false);
        let version = first["sourceVersion"].clone();
        for overrides in [json!({"format":"txt"}), json!({"includeCitations":true})] {
            let mut params =
                json!({"meetingId":fixture.id,"expectedSourceVersion":version,"offset":1});
            params
                .as_object_mut()
                .unwrap()
                .extend(overrides.as_object().unwrap().clone());
            assert!(matches!(
                reader.query("summary_get", params),
                Err(CoreError::Reader(ReadError::SourceVersionMismatch))
            ));
        }
        for (format, citations) in [("md", false), ("txt", false), ("md", true)] {
            let mut restored = String::new();
            let mut offset = 0;
            let mut version: Option<Value> = None;
            loop {
                let mut params = json!({"meetingId":fixture.id,"format":format,"includeCitations":citations,"offset":offset,"limit":1});
                if let Some(version) = &version {
                    params["expectedSourceVersion"] = version.clone();
                }
                let page = reader.query("summary_get", params).unwrap();
                version = Some(page["sourceVersion"].clone());
                restored.push_str(page["summary"]["text"].as_str().unwrap());
                match page["pagination"]["nextOffset"].as_u64() {
                    Some(next) => offset = next,
                    None => break,
                }
            }
            let selected = if format == "md" {
                DocumentFormat::Md
            } else {
                DocumentFormat::Txt
            };
            assert_eq!(
                restored,
                format_summary(
                    &summary.text,
                    selected,
                    citations,
                    false,
                    Some(fixture.transcript.segments.len())
                )
            );
        }
        assert_eq!(before, tree_hash(&fixture.root));
        let mut changed = fixture.transcript.clone();
        changed.version.revision += 1;
        fixture.save_transcript(&changed);
        assert!(matches!(
            reader.query(
                "summary_get",
                json!({"meetingId":fixture.id,"expectedSourceVersion":version})
            ),
            Err(CoreError::Reader(ReadError::SourceVersionMismatch))
        ));
        let stale = reader
            .query(
                "summary_get",
                json!({"meetingId":fixture.id,"format":"txt"}),
            )
            .unwrap();
        assert_eq!(stale["summary"]["stale"], true);
        assert!(stale["summary"]["text"]
            .as_str()
            .unwrap()
            .starts_with(crate::presentation::STALE_SUMMARY_WARNING));
        assert!(!stale["summary"]["text"].as_str().unwrap().contains("[1]"));
        let mut archive: Value = store::read_json(&fixture.root.join(&frozen)).unwrap();
        archive["source"]["segments"][0]["text"] = json!("손상된 frozen source");
        store::write_json(&fixture.root, &frozen, &archive).unwrap();
        assert!(matches!(reader.query("summary_get", json!({"meetingId":fixture.id,"format":"txt","expectedSourceVersion":stale["sourceVersion"]})), Err(CoreError::Reader(ReadError::SourceVersionMismatch))));
        let before = tree_hash(&fixture.root);
        let preserved = reader
            .query("summary_get", json!({"meetingId":fixture.id}))
            .unwrap();
        assert!(preserved["summary"]["text"]
            .as_str()
            .unwrap()
            .contains("[1]"));
        assert_eq!(before, tree_hash(&fixture.root));
    }

    #[test]
    fn notes_and_summary_have_line_pages_and_stale_source_versions() {
        let fixture = Fixture::new();
        let reader = fixture.reader();
        let params = json!({"meetingId":fixture.id,"offset":1,"limit":1});
        assert_eq!(
            reader.query("meeting_get", params.clone()).unwrap()["meeting"]["notes"],
            "note two\n"
        );
        let summary = reader.query("summary_get", params.clone()).unwrap();
        assert_eq!(summary["summary"]["text"], "summary two\n");
        assert_eq!(summary["pagination"]["unit"], "textChunks");
        assert_eq!(summary["summary"]["stale"], false);
        let mut changed = fixture.transcript.clone();
        changed.version.revision += 1;
        changed.segments[0].text = "latest correction".into();
        fixture.save_transcript(&changed);
        let result = reader.query("summary_get", params).unwrap();
        assert_eq!(result["summary"]["stale"], true);
        assert_eq!(result["summary"]["sourceVersion"]["revision"], 1);
        let matches = reader
            .query("meetings_search", json!({"query":"latest correction"}))
            .unwrap();
        assert_eq!(
            matches["meetings"][0]["matches"][0]["segmentId"],
            "utterance-0"
        );
        assert!(reader
            .query("meetings_search", json!({"query":"original 0"}))
            .unwrap()["meetings"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn read_only_summary_freshness_includes_the_notes_snapshot() {
        let fixture = Fixture::new();
        let summary_path = Path::new("meetings")
            .join(&fixture.id)
            .join("summaries/current.json");
        let mut summary: Summary = store::read_json(&fixture.root.join(&summary_path)).unwrap();
        summary.notes_sha256 = Some(crate::summary::notes_digest(
            "note one\nnote two\nnote three",
        ));
        store::write_json(&fixture.root, &summary_path, &summary).unwrap();
        let before = fs::read(fixture.root.join(&summary_path)).unwrap();
        let reader = fixture.reader();
        let params = json!({"meetingId":fixture.id});
        assert_eq!(
            reader.query("summary_get", params.clone()).unwrap()["summary"]["stale"],
            false
        );
        let meeting_path = Path::new("meetings").join(&fixture.id).join("meeting.json");
        let mut meeting: store::StoredMeeting =
            store::read_json(&fixture.root.join(&meeting_path)).unwrap();
        meeting.meeting.notes.push_str("\nupdated context");
        store::write_json(&fixture.root, &meeting_path, &meeting).unwrap();
        assert_eq!(
            reader.query("summary_get", params).unwrap()["summary"]["stale"],
            true
        );
        assert_eq!(fs::read(fixture.root.join(summary_path)).unwrap(), before);
    }

    #[test]
    fn list_filters_and_trash_apply_to_explicit_reads_too() {
        let fixture = Fixture::new();
        let reader = fixture.reader();
        let listed = reader
            .query(
                "meetings_list",
                json!({"folderId":fixture.folder,"from":1500,"to":1501}),
            )
            .unwrap();
        assert_eq!(listed["pagination"]["total"], 1);
        assert!(
            reader.query("meetings_list", json!({"from":1501})).unwrap()["meetings"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        fixture.set_deleted();
        assert!(
            reader.query("meetings_list", json!({})).unwrap()["meetings"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        for method in ["meeting_get", "transcript_get", "summary_get"] {
            assert!(matches!(
                reader.query(method, json!({"meetingId":fixture.id})),
                Err(CoreError::MeetingNotFound)
            ));
        }
    }

    #[test]
    fn folder_queries_observe_library_changes_without_reopening_or_writing() {
        let fixture = Fixture::new();
        let reader = fixture.reader();
        let first = reader.query("folders_list", json!({})).unwrap();
        assert_eq!(first["folders"][0]["name"], "Synthetic folder");
        assert_eq!(
            reader
                .query("meetings_list", json!({"unfiled":true}))
                .unwrap()["pagination"]["total"],
            0
        );
        let mut library: Library = store::read_json(&fixture.root.join("library.json")).unwrap();
        library.folders.get_mut(&fixture.folder).unwrap().name = "Renamed folder".into();
        library.placements.get_mut(&fixture.id).unwrap().folder_id = None;
        store::write_json(&fixture.root, Path::new("library.json"), &library).unwrap();
        let before = tree_hash(&fixture.root);
        let renamed = reader.query("folders_list", json!({})).unwrap();
        assert_eq!(renamed["folders"][0]["name"], "Renamed folder");
        assert_ne!(renamed["sourceVersion"], first["sourceVersion"]);
        let unfiled = reader
            .query("meetings_list", json!({"unfiled":true}))
            .unwrap();
        assert_eq!(unfiled["pagination"]["total"], 1);
        assert!(unfiled["meetings"][0]["folderId"].is_null());
        assert_eq!(
            reader
                .query("meetings_list", json!({"folderId":fixture.folder}))
                .unwrap()["pagination"]["total"],
            0
        );
        assert_eq!(
            reader
                .query(
                    "meetings_search",
                    json!({"query":"corrected","unfiled":true})
                )
                .unwrap()["pagination"]["total"],
            1
        );
        assert!(reader
            .query(
                "meetings_list",
                json!({"folderId":fixture.folder,"unfiled":true})
            )
            .is_err());
        assert!(reader
            .query("folders_list", json!({"unfiled":true}))
            .is_err());
        assert_eq!(tree_hash(&fixture.root), before);

        let mut added = library.folders[&fixture.folder].clone();
        added.id = uuid::Uuid::new_v4().to_string();
        added.name = "Added folder".into();
        let added_id = added.id.clone();
        library.folders.insert(added_id.clone(), added);
        store::write_json(&fixture.root, Path::new("library.json"), &library).unwrap();
        let after_add = tree_hash(&fixture.root);
        let folders = reader.query("folders_list", json!({})).unwrap();
        assert_eq!(folders["pagination"]["total"], 2);
        assert!(folders["folders"]
            .as_array()
            .unwrap()
            .iter()
            .any(|folder| folder["id"] == added_id && folder["name"] == "Added folder"));
        assert_ne!(folders["sourceVersion"], renamed["sourceVersion"]);
        assert_eq!(tree_hash(&fixture.root), after_add);
    }

    #[test]
    fn missing_corrupt_unsupported_and_oversized_documents_are_not_empty_successes() {
        let fixture = Fixture::new();
        let reader = fixture.reader();
        let meeting = fixture
            .root
            .join("meetings")
            .join(&fixture.id)
            .join("meeting.json");
        let original = fs::read(&meeting).unwrap();
        fs::write(&meeting, b"{broken").unwrap();
        assert!(matches!(
            reader.query("meetings_list", json!({})),
            Err(CoreError::Reader(ReadError::InvalidDocument))
        ));
        let mut value: Value = serde_json::from_slice(&original).unwrap();
        value["schemaVersion"] = json!(99);
        fs::write(&meeting, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            reader.query("meetings_list", json!({})),
            Err(CoreError::Reader(ReadError::UnsupportedFormat))
        ));
        fs::write(&meeting, vec![b' '; MAX_DOCUMENT_BYTES as usize + 1]).unwrap();
        assert!(matches!(
            reader.query("meetings_list", json!({})),
            Err(CoreError::Reader(ReadError::DocumentTooLarge))
        ));
        fs::write(&meeting, original).unwrap();
        let current = fixture
            .root
            .join("meetings")
            .join(&fixture.id)
            .join("transcripts")
            .join(&fixture.transcript.version.id)
            .join("current.json");
        fs::rename(&current, current.with_extension("held")).unwrap();
        assert!(matches!(
            reader.query("transcript_get", json!({"meetingId":fixture.id})),
            Err(CoreError::Reader(ReadError::InvalidDocument))
        ));
    }

    #[test]
    fn observed_file_and_directory_changes_invalidate_a_snapshot() {
        let fixture = Fixture::new();
        let mut transaction = Transaction::new(&fixture.root);
        let before = transaction
            .query(
                "meeting_get",
                &Params::parse("meeting_get", json!({"meetingId":fixture.id})).unwrap(),
            )
            .unwrap();
        assert_eq!(before["meeting"]["transcriptVersion"]["revision"], 1);
        assert!(transaction.verify().unwrap());
        let mut changed = fixture.transcript.clone();
        changed.version.revision += 1;
        fixture.save_transcript(&changed);
        assert!(!transaction.verify().unwrap());
        let mut transaction = Transaction::new(&fixture.root);
        transaction.directory(Path::new("meetings")).unwrap();
        fs::create_dir(
            fixture
                .root
                .join("meetings")
                .join(Uuid::new_v4().to_string()),
        )
        .unwrap();
        assert!(!transaction.verify().unwrap());
    }

    #[test]
    fn incomplete_creation_is_skipped_and_later_publication_invalidates_the_snapshot() {
        for publish_metadata in [false, true] {
            let fixture = Fixture::new();
            let id = Uuid::new_v4().to_string();
            let relative = Path::new("meetings").join(&id);
            fs::create_dir(fixture.root.join(&relative)).unwrap();
            let mut transaction = Transaction::new(&fixture.root);
            let params = Params::parse("meetings_list", json!({})).unwrap();
            let result = transaction.query("meetings_list", &params).unwrap();
            assert_eq!(result["pagination"]["total"], 1);
            assert!(matches!(
                transaction.observations.get(&relative.join("meeting.json")),
                Some(Observation::File(None))
            ));
            assert!(matches!(
                transaction.observations.get(&relative.join("sessions")),
                Some(Observation::DirectoryPresence(false))
            ));
            assert!(transaction.verify().unwrap());
            if publish_metadata {
                let existing = Path::new("meetings").join(&fixture.id).join("meeting.json");
                let mut document: Value = store::read_json(&fixture.root.join(existing)).unwrap();
                document["meeting"]["id"] = json!(id);
                document["meeting"]["transcriptId"] = Value::Null;
                document["meeting"]["audio"] = Value::Null;
                store::write_json(&fixture.root, &relative.join("meeting.json"), &document)
                    .unwrap();
                assert_eq!(
                    fixture.reader().query("meetings_list", json!({})).unwrap()["pagination"]
                        ["total"],
                    2
                );
            } else {
                fs::create_dir(fixture.root.join(relative.join("sessions"))).unwrap();
                assert!(matches!(
                    fixture.reader().query("meetings_list", json!({})),
                    Err(CoreError::Reader(ReadError::InvalidDocument))
                ));
            }
            assert!(!transaction.verify().unwrap());
        }
    }

    #[test]
    fn meeting_lists_use_recording_or_creation_time_with_a_stable_id_tiebreaker() {
        let fixture = Fixture::new();
        let store = store::Store::open(fixture.root.clone()).unwrap();
        let relative = Path::new("meetings").join(&fixture.id).join("meeting.json");
        let mut edited: StoredMeeting = store::read_json(&fixture.root.join(relative)).unwrap();
        edited.meeting.updated_at = u64::MAX;
        store.save(&edited).unwrap();
        let mut expected = Vec::new();
        for _ in 0..2 {
            let mut created = store
                .create("Synthetic newer draft".into(), String::new())
                .unwrap();
            created.meeting.created_at = 1600;
            created.meeting.updated_at = 1;
            store.save(&created).unwrap();
            expected.push(created.meeting.id);
        }
        expected.sort();
        let filtered = fixture
            .reader()
            .query("meetings_list", json!({"from":1600,"to":1601}))
            .unwrap();
        assert_eq!(filtered["pagination"]["total"], 2);
        expected.push(fixture.id.clone());
        let listed = fixture.reader().query("meetings_list", json!({})).unwrap();
        let actual: Vec<_> = listed["meetings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|meeting| meeting["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn input_and_response_limits_are_explicit() {
        let fixture = Fixture::new();
        let reader = fixture.reader();
        for params in [
            json!({"limit":201}),
            json!({"limit":0}),
            json!({"maxBytes":0}),
            json!({"maxBytes":MAX_RESPONSE_BYTES + 1}),
            json!({"expectedSourceVersion":"sha256:short"}),
            json!({"expectedSourceVersion":format!("sha256:{}", "G".repeat(64))}),
            json!({"from":5,"to":4}),
            json!({"unexpected":true}),
        ] {
            assert!(matches!(
                reader.query("meetings_list", params),
                Err(CoreError::InvalidInput(_))
            ));
        }
        assert!(reader
            .query("meeting_get", json!({"meetingId":"../../elsewhere"}))
            .is_err());
        assert!(reader
            .query("meetings_search", json!({"query":" "}))
            .is_err());
        let path = fixture
            .root
            .join("meetings")
            .join(&fixture.id)
            .join("meeting.json");
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["meeting"]["notes"] = json!("\u{0001}".repeat(MAX_RESPONSE_BYTES / 6 + 1));
        store::write_json(
            &fixture.root,
            path.strip_prefix(&fixture.root).unwrap(),
            &value,
        )
        .unwrap();
        let page = reader
            .query("meeting_get", json!({"meetingId":fixture.id}))
            .unwrap();
        assert!(response_bytes(&page).unwrap() <= MAX_RESPONSE_BYTES);
        assert!(page["pagination"]["returned"].as_u64().unwrap() < 50);
        assert_eq!(page["pagination"]["hasMore"], true);
        assert!(reader
            .query("meeting_get", json!({"meetingId":fixture.id,"limit":1}))
            .is_ok());
        assert!(matches!(
            reader.query("meeting_get", json!({"meetingId":fixture.id,"maxBytes":1})),
            Err(CoreError::Reader(ReadError::ResponseTooLarge))
        ));
        for params in [
            json!({"maxBytes":512}),
            json!({"expectedSourceVersion":format!("sha256:{}", "0".repeat(64))}),
        ] {
            assert!(matches!(
                reader.query("status", params),
                Err(CoreError::InvalidInput(_))
            ));
        }
    }

    #[test]
    fn byte_bounded_pages_preserve_ids_ranges_versions_and_read_only_storage() {
        let fixture = Fixture::new();
        let mut transcript = fixture.transcript.clone();
        transcript.segments = (0..9)
            .map(|index| {
                let mut segment = fixture.transcript.segments[0].clone();
                segment.id = format!("bounded-{index}");
                segment.start = index as f64 * 2.0;
                segment.end = segment.start + 1.0;
                segment.source_index = Some(index);
                segment.text = format!("발언 {index}: {}", "한글과 English 기록 ".repeat(100));
                segment
            })
            .collect();
        fixture.save_transcript(&transcript);
        let reader = fixture.reader();
        let one = reader
            .query("transcript_get", json!({"meetingId":fixture.id,"limit":1}))
            .unwrap();
        let budget = response_bytes(&one).unwrap() + 32;
        let version = one["sourceVersion"].clone();
        let before = tree_hash(&fixture.root);
        for (start, end, expected) in [(None, None, 0..9), (Some(1), Some(5), 1..3)] {
            let mut offset = 0;
            let mut ids = Vec::new();
            loop {
                let mut params = json!({"meetingId":fixture.id,"offset":offset,"limit":200,
                    "maxBytes":budget,"expectedSourceVersion":version});
                if let Some(start) = start {
                    params["start"] = json!(start);
                }
                if let Some(end) = end {
                    params["end"] = json!(end);
                }
                let page = reader.query("transcript_get", params).unwrap();
                let returned = page["segments"].as_array().unwrap();
                assert_eq!(returned.len(), 1);
                assert_eq!(page["pagination"]["returned"], returned.len());
                assert_eq!(page["pagination"]["limit"], 200);
                assert_eq!(page["pagination"]["total"], expected.len());
                assert_eq!(page["sourceVersion"], version);
                assert!(response_bytes(&page).unwrap() <= budget);
                ids.extend(
                    returned
                        .iter()
                        .map(|segment| segment["id"].as_str().unwrap().to_owned()),
                );
                match page["pagination"]["nextOffset"].as_u64() {
                    Some(next) => {
                        assert_eq!(page["pagination"]["hasMore"], true);
                        assert_eq!(next as usize, offset + returned.len());
                        offset = next as usize;
                    }
                    None => {
                        assert_eq!(page["pagination"]["hasMore"], false);
                        break;
                    }
                }
            }
            assert_eq!(
                ids,
                expected
                    .map(|index| format!("bounded-{index}"))
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(tree_hash(&fixture.root), before);
    }

    #[test]
    fn lists_searches_and_folders_follow_the_actual_bounded_page_length() {
        let fixture = Fixture::new();
        let store = store::Store::open(fixture.root.clone()).unwrap();
        for index in 0..4 {
            store
                .create(format!("Synthetic meeting {index}"), String::new())
                .unwrap();
        }
        let mut library: Library = store::read_json(&fixture.root.join("library.json")).unwrap();
        for index in 0..4 {
            let mut folder = library.folders[&fixture.folder].clone();
            folder.id = Uuid::new_v4().to_string();
            folder.name = format!(
                "Synthetic folder {index}: {}",
                "한글 ".repeat(10).trim_end()
            );
            library.folders.insert(folder.id.clone(), folder);
        }
        store::write_json(&fixture.root, Path::new("library.json"), &library).unwrap();
        let reader = fixture.reader();
        let before = tree_hash(&fixture.root);
        for (method, arguments, key) in [
            ("meetings_list", json!({}), "meetings"),
            ("meetings_search", json!({"query":"Synthetic"}), "meetings"),
            ("folders_list", json!({}), "folders"),
        ] {
            let full = reader.query(method, arguments.clone()).unwrap();
            let expected = full[key].as_array().unwrap();
            let budget = (0..expected.len())
                .map(|offset| {
                    let mut params = arguments.clone();
                    params["offset"] = json!(offset);
                    params["limit"] = json!(1);
                    response_bytes(&reader.query(method, params).unwrap()).unwrap()
                })
                .max()
                .unwrap()
                + 32;
            let mut restored = Vec::new();
            let mut offset = 0;
            loop {
                let mut params = arguments.clone();
                params["offset"] = json!(offset);
                params["limit"] = json!(200);
                params["maxBytes"] = json!(budget);
                params["expectedSourceVersion"] = full["sourceVersion"].clone();
                let page = reader.query(method, params).unwrap();
                let returned = page[key].as_array().unwrap();
                assert!(!returned.is_empty());
                assert_eq!(page["pagination"]["returned"], returned.len());
                assert_eq!(page["pagination"]["total"], expected.len());
                assert_eq!(page["sourceVersion"], full["sourceVersion"]);
                assert!(response_bytes(&page).unwrap() <= budget);
                restored.extend(returned.iter().cloned());
                match page["pagination"]["nextOffset"].as_u64() {
                    Some(next) => {
                        assert_eq!(next as usize, offset + returned.len());
                        offset = next as usize;
                    }
                    None => break,
                }
            }
            assert_eq!(&restored, expected);
        }
        assert_eq!(tree_hash(&fixture.root), before);
    }

    #[test]
    fn continuation_rejects_changed_sources_before_returning_a_new_page() {
        let fixture = Fixture::new();
        let reader = fixture.reader();
        let first = reader
            .query("transcript_get", json!({"meetingId":fixture.id,"limit":1}))
            .unwrap();
        let expected = first["sourceVersion"].clone();
        let second = reader
            .query(
                "transcript_get",
                json!({"meetingId":fixture.id,"offset":1,"limit":1,
                "expectedSourceVersion":expected}),
            )
            .unwrap();
        assert_eq!(second["sourceVersion"], expected);
        let mut changed = fixture.transcript.clone();
        changed.version.revision += 1;
        changed.segments[1].text = "새 수정 내용".into();
        fixture.save_transcript(&changed);
        let before = tree_hash(&fixture.root);
        assert!(matches!(
            reader.query(
                "transcript_get",
                json!({"meetingId":fixture.id,"offset":1,"limit":1,
                "expectedSourceVersion":expected,"maxBytes":1})
            ),
            Err(CoreError::Reader(ReadError::SourceVersionMismatch))
        ));
        assert_eq!(tree_hash(&fixture.root), before);
        let latest = reader
            .query(
                "transcript_get",
                json!({"meetingId":fixture.id,"offset":1,"limit":1}),
            )
            .unwrap();
        assert_eq!(latest["segments"][0]["text"], "새 수정 내용");
        assert_ne!(latest["sourceVersion"], expected);

        let folders = reader.query("folders_list", json!({})).unwrap();
        let mut library: Library = store::read_json(&fixture.root.join("library.json")).unwrap();
        library.folders.get_mut(&fixture.folder).unwrap().name = "Changed name".into();
        store::write_json(&fixture.root, Path::new("library.json"), &library).unwrap();
        assert!(matches!(
            reader.query(
                "folders_list",
                json!({"expectedSourceVersion":folders["sourceVersion"]})
            ),
            Err(CoreError::Reader(ReadError::SourceVersionMismatch))
        ));
    }

    #[test]
    fn a_single_oversized_segment_or_empty_page_envelope_is_not_an_empty_success() {
        let fixture = Fixture::new();
        let mut transcript = fixture.transcript.clone();
        transcript.segments.truncate(1);
        transcript.segments[0].text = "\u{0001}".repeat(MAX_RESPONSE_BYTES / 6 + 1);
        fixture.save_transcript(&transcript);
        for params in [
            json!({"meetingId":fixture.id}),
            json!({"meetingId":fixture.id,"offset":1,"maxBytes":1}),
        ] {
            assert!(matches!(
                fixture.reader().query("transcript_get", params),
                Err(CoreError::Reader(ReadError::ResponseTooLarge))
            ));
        }
    }

    #[test]
    fn byte_bounded_text_pages_reconstruct_unicode_and_newlines() {
        let fixture = Fixture::new();
        let text = format!(
            "{}\r\n\n{}\n끝",
            "가나다 English ".repeat(1600),
            "second ".repeat(900)
        );
        for (relative, method, field) in [
            (
                Path::new("meetings").join(&fixture.id).join("meeting.json"),
                "meeting_get",
                "notes",
            ),
            (
                Path::new("meetings")
                    .join(&fixture.id)
                    .join("summaries/current.json"),
                "summary_get",
                "text",
            ),
        ] {
            let mut document: Value = store::read_json(&fixture.root.join(&relative)).unwrap();
            if method == "meeting_get" {
                document["meeting"][field] = json!(text);
            } else {
                document[field] = json!(text);
            }
            store::write_json(&fixture.root, &relative, &document).unwrap();
            let reader = fixture.reader();
            let first = reader
                .query(method, json!({"meetingId":fixture.id,"limit":1}))
                .unwrap();
            let budget = response_bytes(&first).unwrap() + 32;
            let version = first["sourceVersion"].clone();
            let before = tree_hash(&fixture.root);
            let mut restored = String::new();
            let mut offset = 0;
            loop {
                let page = reader
                    .query(
                        method,
                        json!({"meetingId":fixture.id,"offset":offset,"limit":200,
                    "maxBytes":budget,"expectedSourceVersion":version}),
                    )
                    .unwrap();
                let body = if method == "meeting_get" {
                    &page["meeting"][field]
                } else {
                    &page["summary"][field]
                };
                restored.push_str(body.as_str().unwrap());
                assert_eq!(page["sourceVersion"], version);
                assert!(response_bytes(&page).unwrap() <= budget);
                match page["pagination"]["nextOffset"].as_u64() {
                    Some(next) => {
                        assert_eq!(
                            next as usize,
                            offset + page["pagination"]["returned"].as_u64().unwrap() as usize
                        );
                        offset = next as usize;
                    }
                    None => break,
                }
            }
            assert_eq!(restored, text);
            assert_eq!(tree_hash(&fixture.root), before);
        }
    }

    #[test]
    fn text_chunks_preserve_unicode_and_all_newlines() {
        let boundary = format!("{}한글\r\n\n끝\n", "a".repeat(MAX_TEXT_CHUNK_BYTES - 1));
        for text in ["", "\n", "line\n", "line\r\n\n끝\n", boundary.as_str()] {
            let mut restored = String::new();
            let mut offset = 0;
            loop {
                let params = Params {
                    offset: Some(offset),
                    limit: Some(1),
                    ..Params::default()
                };
                let (chunk, page) = page_text_chunks(text, &params);
                assert!(chunk.len() <= MAX_TEXT_CHUNK_BYTES);
                assert_eq!(page["unit"], "textChunks");
                restored.push_str(&chunk);
                if page["hasMore"] == false {
                    break;
                }
                offset += 1;
            }
            assert_eq!(restored, text);
        }
    }

    #[test]
    fn long_single_line_notes_and_summaries_can_be_read_in_full() {
        let fixture = Fixture::new();
        let text = format!("{}\n끝\n", "가나다라".repeat(MAX_RESPONSE_BYTES / 12 + 1));
        assert!(text.lines().next().unwrap().len() > MAX_RESPONSE_BYTES);
        for (relative, method, field) in [
            (
                Path::new("meetings").join(&fixture.id).join("meeting.json"),
                "meeting_get",
                "notes",
            ),
            (
                Path::new("meetings")
                    .join(&fixture.id)
                    .join("summaries/current.json"),
                "summary_get",
                "text",
            ),
        ] {
            let mut document: Value = store::read_json(&fixture.root.join(&relative)).unwrap();
            if method == "meeting_get" {
                document["meeting"][field] = json!(text);
            } else {
                document[field] = json!(text);
            }
            store::write_json(&fixture.root, &relative, &document).unwrap();
            let mut restored = String::new();
            let mut offset = 0;
            loop {
                let result = fixture
                    .reader()
                    .query(
                        method,
                        json!({"meetingId":fixture.id,"offset":offset,"limit":200}),
                    )
                    .unwrap();
                assert_eq!(result["pagination"]["unit"], "textChunks");
                let body = if method == "meeting_get" {
                    &result["meeting"][field]
                } else {
                    &result["summary"][field]
                };
                restored.push_str(body.as_str().unwrap());
                if result["pagination"]["hasMore"] == false {
                    break;
                }
                offset += 200;
            }
            assert!(offset > 0);
            assert_eq!(restored, text);
        }
    }

    #[cfg(unix)]
    #[test]
    fn linked_roots_documents_and_directories_are_rejected() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let temporary = tempfile::tempdir().unwrap();
        let alias = temporary.path().canonicalize().unwrap().join("alias");
        symlink(&fixture.root, &alias).unwrap();
        assert!(LibraryReader::open(alias).is_err());
        let path = fixture
            .root
            .join("meetings")
            .join(&fixture.id)
            .join("meeting.json");
        let held = temporary.path().join("held.json");
        fs::rename(&path, &held).unwrap();
        symlink(&held, &path).unwrap();
        assert!(fixture
            .reader()
            .query("meeting_get", json!({"meetingId":fixture.id}))
            .is_err());
        let fixture = Fixture::new();
        symlink(
            temporary.path(),
            fixture
                .root
                .join("meetings")
                .join(Uuid::new_v4().to_string()),
        )
        .unwrap();
        assert!(fixture.reader().query("meetings_list", json!({})).is_err());
    }
}
