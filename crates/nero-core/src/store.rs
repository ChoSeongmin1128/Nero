use crate::library::Library;
use crate::{AudioArtifact, CoreError, Meeting, MeetingStatus, Result};
use fs2::FileExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const SCHEMA_VERSION: u32 = 1;
const MAX_JSON_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Encoding {
    F32,
    F64,
    I16,
    I32,
}

impl Encoding {
    pub fn bytes(self) -> usize {
        match self {
            Self::I16 => 2,
            Self::F32 | Self::I32 => 4,
            Self::F64 => 8,
        }
    }

    pub(crate) fn sample(self, bytes: &[u8]) -> f32 {
        match self {
            Self::F32 => f32::from_le_bytes(bytes.try_into().unwrap()),
            Self::F64 => f64::from_le_bytes(bytes.try_into().unwrap()) as f32,
            Self::I16 => i16::from_le_bytes(bytes.try_into().unwrap()) as f32 / 32768.0,
            Self::I32 => i32::from_le_bytes(bytes.try_into().unwrap()) as f32 / 2147483648.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CapturePhase {
    Prepared,
    Recording,
    Paused,
    Completed,
    Interrupted,
}

// Bound callback diagnostics while retaining overflow counts and observation ranges.
pub(crate) const MAX_CAPTURE_QUALITY_EVENTS: usize = 64;
pub(crate) const CAPTURE_QUALITY_EVENT_KINDS: usize = 8;
pub(crate) const CAPTURE_QUALITY_INDIVIDUAL_EVENTS: usize =
    MAX_CAPTURE_QUALITY_EVENTS - CAPTURE_QUALITY_EVENT_KINDS;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[repr(u8)]
pub enum CaptureQualityEventKind {
    Xrun,
    RealtimeDenied,
    RouteChanged,
    QueueOverflow,
    DeviceFailure,
    WriterFailure,
    FormatChanged,
    CaptureInterrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureQualityEvent {
    pub kind: CaptureQualityEventKind,
    pub count: u64,
    pub first_observed_at_ms: u64,
    pub last_observed_at_ms: u64,
    pub first_saved_frame: u64,
    pub last_saved_frame: u64,
    pub first_queued_frame: u64,
    pub last_queued_frame: u64,
    pub coalesced: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureTiming {
    pub callback_count: u64,
    pub input_frames: u64,
    pub first_callback_after_begin_ns: u64,
    pub timestamp_gap_count: u64,
    pub timestamp_overlap_count: u64,
    pub timestamp_reversal_count: u64,
    pub max_timestamp_gap_ns: u64,
    pub max_timestamp_overlap_ns: u64,
    pub max_arrival_interval_ns: u64,
    pub max_host_interval_ns: u64,
    pub max_pending_frames: u64,
    pub queued_frames: u64,
    pub saved_frames: u64,
    pub max_writer_batch_ns: u64,
    pub max_checkpoint_ns: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureQuality {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<CaptureTiming>,
    pub xrun_count: u64,
    pub realtime_denied_count: u64,
    pub route_changed_count: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<CaptureQualityEvent>,
}

impl CaptureQuality {
    pub fn warning(&self) -> Option<String> {
        let mut warnings: Vec<String> = Vec::new();
        if self.xrun_count != 0 {
            warnings.push("오디오 입력 지연 신호가 기록되었습니다.".into());
        }
        if self.route_changed_count != 0 {
            warnings.push(
                "녹음 중 오디오 입력 경로가 변경되었습니다. 사용할 마이크를 확인해 주세요.".into(),
            );
        }
        (!warnings.is_empty()).then(|| warnings.join(" "))
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.events.len() > MAX_CAPTURE_QUALITY_EVENTS
            || self.events.iter().any(|event| {
                event.count == 0
                    || (!event.coalesced && event.count != 1)
                    || event.first_observed_at_ms > event.last_observed_at_ms
                    || event.first_saved_frame > event.last_saved_frame
                    || event.first_queued_frame > event.last_queued_frame
                    || event.first_saved_frame > event.first_queued_frame
                    || event.last_saved_frame > event.last_queued_frame
            })
        {
            return Err(CoreError::Storage(
                "녹음 진단 정보의 형식이 올바르지 않습니다.".into(),
            ));
        }
        Ok(())
    }
}

pub(crate) fn deserialize_optional_capture_quality<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<CaptureQuality>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Diagnostic damage is independent of capture/audio integrity. Decode only
    // this optional subtree tolerantly; the enclosing document and its required
    // metadata still use their ordinary strict deserialization and validation.
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<CaptureQuality>(value)
        .ok()
        .filter(|quality| quality.validate().is_ok()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Manifest {
    pub schema_version: u32,
    pub meeting_id: String,
    pub session_id: String,
    pub device_id: String,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub encoding: Encoding,
    pub started_at: u64,
    pub phase: CapturePhase,
    pub saved_frames: u64,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_capture_quality"
    )]
    pub quality: Option<CaptureQuality>,
}

impl Manifest {
    pub fn new(
        meeting_id: &str,
        device_id: &str,
        device_name: &str,
        sample_rate: u32,
        channels: u16,
        encoding: Encoding,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            meeting_id: meeting_id.into(),
            session_id: Uuid::new_v4().to_string(),
            device_id: device_id.into(),
            device_name: device_name.into(),
            sample_rate,
            channels,
            encoding,
            started_at: now(),
            phase: CapturePhase::Prepared,
            saved_frames: 0,
            error: None,
            warning: None,
            quality: None,
        }
    }

    pub fn frame_bytes(&self) -> usize {
        self.channels as usize * self.encoding.bytes()
    }

    pub fn relative_dir(&self) -> PathBuf {
        Path::new("meetings")
            .join(&self.meeting_id)
            .join("sessions")
            .join(&self.session_id)
    }

    pub fn validate(&self) -> Result<()> {
        validate_id(&self.meeting_id)?;
        validate_id(&self.session_id)?;
        if self.schema_version != SCHEMA_VERSION
            || self.sample_rate == 0
            || self.sample_rate > 768_000
            || self.channels == 0
            || self.channels > 64
        {
            return Err(CoreError::Storage(
                "녹음 정보의 형식이 올바르지 않습니다.".into(),
            ));
        }
        if let Some(quality) = &self.quality {
            quality.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoredMeeting {
    pub schema_version: u32,
    pub meeting: Meeting,
    pub capture_id: Option<String>,
    #[serde(default)]
    pub generated_title: bool,
}

pub(crate) fn hydrate_capture_metadata(meeting: &mut Meeting, manifest: &Manifest) {
    if meeting.recorded_at.is_none() && manifest.device_id != "import" {
        meeting.recorded_at = Some(manifest.started_at);
    }
    meeting.warning = match &manifest.quality {
        Some(quality) => quality.warning(),
        None => manifest.warning.clone(),
    };
    meeting.capture_quality = manifest.quality.clone();
}

pub(crate) struct Store {
    pub root: PathBuf,
    _lock: File,
}

impl Store {
    pub fn open(root: PathBuf) -> Result<Self> {
        ensure_directory(&root)?;
        let root = root.canonicalize()?;
        let lock_path = checked_path(&root, Path::new("store.lock"))?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        no_follow(&mut options);
        let lock = options.open(lock_path)?;
        validate_file(&lock)?;
        lock.try_lock_exclusive()
            .map_err(|_| CoreError::StoreBusy)?;
        ensure_directory(&root.join("meetings"))?;
        Ok(Self { root, _lock: lock })
    }

    pub fn load(&self) -> Result<BTreeMap<String, StoredMeeting>> {
        self.load_with_library(&self.load_library()?)
    }

    pub fn load_library(&self) -> Result<Library> {
        let path = checked_path(&self.root, Path::new("library.json"))?;
        let library = if path.exists() {
            read_json(&path)?
        } else {
            Library::default()
        };
        library.validate()?;
        Ok(library)
    }

    pub fn save_library(&self, library: &Library) -> Result<()> {
        library.validate()?;
        write_json(&self.root, Path::new("library.json"), library)
    }

    pub fn load_with_library(&self, library: &Library) -> Result<BTreeMap<String, StoredMeeting>> {
        let meetings_path = checked_path(&self.root, Path::new("meetings"))?;
        let mut meetings = BTreeMap::new();
        let mut seen = BTreeSet::new();
        for entry in fs::read_dir(meetings_path)? {
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            validate_id(&id)?;
            if library.pending_purges.contains(&id) {
                continue;
            }
            if entry.file_type()?.is_symlink() || !entry.file_type()?.is_dir() {
                return Err(CoreError::Storage(
                    "회의 폴더의 형식이 올바르지 않습니다.".into(),
                ));
            }
            let relative = Path::new("meetings").join(&id).join("meeting.json");
            let path = checked_path(&self.root, &relative)?;
            if !path.exists() {
                let sessions = checked_path(
                    &self.root,
                    &Path::new("meetings").join(&id).join("sessions"),
                )?;
                if sessions.exists() {
                    return Err(CoreError::Storage(
                        "회의 정보가 없는 녹음 폴더가 있습니다.".into(),
                    ));
                }
                continue;
            }
            let mut stored: StoredMeeting = read_json(&path)?;
            if stored.schema_version != SCHEMA_VERSION
                || stored.meeting.id != id
                || !seen.insert(stored.meeting.id.clone())
            {
                return Err(CoreError::Storage(
                    "중복되거나 올바르지 않은 회의 정보가 있습니다.".into(),
                ));
            }
            validate_text(&stored.meeting.title, &stored.meeting.notes)?;
            if let Some(date) = &stored.meeting.meeting_date {
                validate_meeting_date(date)?;
            }
            if let Some(capture_id) = &stored.capture_id {
                validate_id(capture_id)?;
            }
            if let Some(audio) = &stored.meeting.audio {
                validate_artifact(&id, audio)?;
                checked_path(&self.root, Path::new(&audio.relative_path))?;
            }
            library.project(&mut stored.meeting);
            meetings.insert(id, stored);
        }
        Ok(meetings)
    }

    pub fn create(&self, title: String, notes: String) -> Result<StoredMeeting> {
        self.create_with_calendar(title, notes, None, false)
    }

    pub fn create_with_calendar(
        &self,
        title: String,
        notes: String,
        calendar_reference: Option<crate::CalendarReference>,
        generated_title: bool,
    ) -> Result<StoredMeeting> {
        validate_text(&title, &notes)?;
        let timestamp = now();
        let meeting_date = calendar_reference
            .as_ref()
            .and_then(|event| event.start_date.clone());
        if let Some(date) = &meeting_date {
            validate_meeting_date(date)?;
        }
        let meeting = Meeting {
            id: Uuid::new_v4().to_string(),
            title,
            notes,
            calendar_reference,
            created_at: timestamp,
            updated_at: timestamp,
            meeting_date,
            recorded_at: None,
            status: MeetingStatus::Draft,
            audio: None,
            error: None,
            warning: None,
            capture_quality: None,
            transcript_id: None,
            folder_id: None,
            deleted_at: None,
        };
        let relative = Path::new("meetings").join(&meeting.id);
        let directory = checked_path(&self.root, &relative)?;
        fs::create_dir(&directory)?;
        let stored = StoredMeeting {
            schema_version: SCHEMA_VERSION,
            meeting,
            capture_id: None,
            generated_title,
        };
        self.save(&stored)?;
        Ok(stored)
    }

    pub fn save(&self, stored: &StoredMeeting) -> Result<()> {
        validate_id(&stored.meeting.id)?;
        let relative = Path::new("meetings")
            .join(&stored.meeting.id)
            .join("meeting.json");
        let mut base = stored.clone();
        base.meeting.folder_id = None;
        base.meeting.deleted_at = None;
        write_json(&self.root, &relative, &base)
    }

    pub fn meeting_matches(&self, stored: &StoredMeeting) -> Result<bool> {
        validate_id(&stored.meeting.id)?;
        let relative = Path::new("meetings")
            .join(&stored.meeting.id)
            .join("meeting.json");
        let saved: StoredMeeting = read_json(&checked_path(&self.root, &relative)?)?;
        let mut expected = stored.clone();
        expected.meeting.folder_id = None;
        expected.meeting.deleted_at = None;
        Ok(saved == expected)
    }

    pub fn validate_purge(&self, ids: &BTreeSet<String>) -> Result<()> {
        for id in ids {
            validate_id(id)?;
            let path = checked_path(&self.root, &Path::new("meetings").join(id))?;
            if path.exists() {
                validate_removal_tree(&path)?;
            }
        }
        Ok(())
    }

    pub fn cleanup_purges(&self, library: &Library) -> Result<()> {
        self.validate_purge(&library.pending_purges)?;
        for id in &library.pending_purges {
            let path = checked_path(&self.root, &Path::new("meetings").join(id))?;
            if path.exists() {
                // remove_dir_all does not follow links. Validation also refuses linked
                // or special entries, and the root never includes model directories.
                fs::remove_dir_all(&path)?;
                sync_parent(&path)?;
            }
        }
        Ok(())
    }

    pub fn prepare_capture(&self, manifest: &Manifest) -> Result<File> {
        manifest.validate()?;
        let meeting_dir = Path::new("meetings").join(&manifest.meeting_id);
        let sessions_dir = checked_path(&self.root, &meeting_dir.join("sessions"))?;
        ensure_directory(&sessions_dir)?;
        let directory = checked_path(&self.root, &manifest.relative_dir())?;
        fs::create_dir(&directory)?;
        write_manifest(&self.root, manifest)?;
        create_new(&checked_path(
            &self.root,
            &manifest.relative_dir().join("audio.pcm"),
        )?)
    }

    pub fn recover(&self, meetings: &mut BTreeMap<String, StoredMeeting>) -> Result<()> {
        for stored in meetings.values_mut() {
            let previous = stored.meeting.clone();
            let hydration = match self.recover_meeting(stored) {
                Ok(hydration) => hydration,
                Err(error) => {
                    stored.meeting.status = MeetingStatus::Interrupted;
                    if self.audio_path(&stored.meeting).is_err() {
                        stored.meeting.audio = None;
                    }
                    stored.meeting.error = Some(format!(
                        "녹음 정보를 확인할 수 없어 자동 복구하지 못했습니다: {error}"
                    ));
                    None
                }
            };
            if stored.meeting != previous {
                if stored.meeting.status != previous.status
                    || stored.meeting.audio != previous.audio
                    || stored.meeting.error != previous.error
                {
                    stored.meeting.updated_at = now();
                }
                self.save(stored)?;
            }
            // Reading a completed capture must not rewrite its meeting document merely
            // to expose diagnostics or metadata added by a newer application build.
            if let Some(manifest) = hydration {
                hydrate_capture_metadata(&mut stored.meeting, &manifest);
            }
        }
        Ok(())
    }

    fn recover_meeting(&self, stored: &mut StoredMeeting) -> Result<Option<Manifest>> {
        let Some(session_id) = stored.capture_id.clone() else {
            if matches!(
                stored.meeting.status,
                MeetingStatus::Recording | MeetingStatus::Paused
            ) {
                return Err(CoreError::Storage(
                    "녹음 정보가 누락된 회의가 있습니다.".into(),
                ));
            }
            return Ok(None);
        };
        let relative = Path::new("meetings")
            .join(&stored.meeting.id)
            .join("sessions")
            .join(&session_id)
            .join("manifest.json");
        let path = checked_path(&self.root, &relative)?;
        let mut manifest: Manifest = read_json(&path)?;
        manifest.validate()?;
        if manifest.meeting_id != stored.meeting.id || manifest.session_id != session_id {
            return Err(CoreError::Storage(
                "회의와 녹음 정보가 일치하지 않습니다.".into(),
            ));
        }
        let unfinished = matches!(
            manifest.phase,
            CapturePhase::Prepared | CapturePhase::Recording | CapturePhase::Paused
        ) || matches!(
            stored.meeting.status,
            MeetingStatus::Recording | MeetingStatus::Paused
        );
        let interrupted = stored.meeting.status == MeetingStatus::Interrupted
            || manifest.phase == CapturePhase::Interrupted;
        let playback_needs_repair = interrupted && self.audio_path(&stored.meeting).is_err();
        if !unfinished && !playback_needs_repair {
            return Ok(Some(manifest));
        }
        hydrate_capture_metadata(&mut stored.meeting, &manifest);
        manifest.warning = stored.meeting.warning.clone();
        let raw = checked_path(&self.root, &manifest.relative_dir().join("audio.pcm"))
            .and_then(|path| open_read(&path));
        let raw = match raw {
            Ok(raw) => raw,
            Err(error) => {
                stored.meeting.status = MeetingStatus::Interrupted;
                stored.meeting.audio = None;
                stored.meeting.error = Some(format!(
                    "원본 녹음을 읽을 수 없어 복구하지 못했습니다: {error}"
                ));
                return Ok(None);
            }
        };
        manifest.phase = CapturePhase::Interrupted;
        manifest.error = Some(if raw.metadata()?.len() < manifest.frame_bytes() as u64 {
            "저장된 오디오가 없어 녹음을 복구하지 못했습니다.".into()
        } else {
            "녹음이 중단되었습니다. 저장된 구간을 복구했습니다.".into()
        });
        drop(raw);
        let outcome = finalize_audio(&self.root, &mut manifest);
        stored.meeting.status = MeetingStatus::Interrupted;
        stored.meeting.error = manifest.error.clone();
        match outcome {
            Ok(audio) => stored.meeting.audio = audio,
            Err(error) => {
                stored.meeting.audio = None;
                stored.meeting.error = Some(format!("녹음이 중단되었습니다. 원본은 보관되지만 재생 파일을 만들지 못했습니다: {error}"));
            }
        }
        Ok(None)
    }

    pub fn audio_path(&self, meeting: &Meeting) -> Result<PathBuf> {
        let artifact = meeting
            .audio
            .as_ref()
            .ok_or_else(|| CoreError::InvalidState("재생할 녹음이 없습니다.".into()))?;
        if matches!(
            meeting.status,
            MeetingStatus::Recording | MeetingStatus::Paused
        ) {
            return Err(CoreError::InvalidState(
                "녹음을 종료한 뒤 재생할 수 있습니다.".into(),
            ));
        }
        validate_artifact(&meeting.id, artifact)?;
        let path = checked_path(&self.root, Path::new(&artifact.relative_path))?;
        validate_playback(
            &path,
            artifact.frames,
            artifact.sample_rate,
            artifact.channels,
        )?;
        Ok(path)
    }
}

pub(crate) fn validate_id(id: &str) -> Result<()> {
    if Uuid::parse_str(id)
        .map(|value| value.to_string())
        .ok()
        .as_deref()
        != Some(id)
    {
        return Err(CoreError::InvalidInput(
            "회의 또는 녹음 ID가 올바르지 않습니다.".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_text(title: &str, notes: &str) -> Result<()> {
    if title.len() > 16 * 1024
        || notes.len() > 2 * 1024 * 1024
        || title.contains('\0')
        || notes.contains('\0')
    {
        return Err(CoreError::InvalidInput(
            "제목 또는 메모가 너무 길거나 올바르지 않습니다.".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_meeting_date(date: &str) -> Result<()> {
    let bytes = date.as_bytes();
    let invalid =
        || CoreError::InvalidInput("회의 날짜를 YYYY-MM-DD 형식으로 입력해 주세요.".into());
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        return Err(invalid());
    }
    let year: u16 = date[..4].parse().map_err(|_| invalid())?;
    let month: u8 = date[5..7].parse().map_err(|_| invalid())?;
    let day: u8 = date[8..].parse().map_err(|_| invalid())?;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => return Err(invalid()),
    };
    if year == 0 || day == 0 || day > days {
        return Err(invalid());
    }
    Ok(())
}

fn validate_artifact(meeting_id: &str, artifact: &AudioArtifact) -> Result<()> {
    validate_id(meeting_id)?;
    validate_id(&artifact.session_id)?;
    let expected = format!(
        "meetings/{meeting_id}/sessions/{}/audio.wav",
        artifact.session_id
    );
    if artifact.relative_path != expected || artifact.sample_rate == 0 || artifact.channels == 0 {
        return Err(CoreError::Storage(
            "녹음 파일 경로가 올바르지 않습니다.".into(),
        ));
    }
    Ok(())
}

pub(crate) fn write_manifest(root: &Path, manifest: &Manifest) -> Result<()> {
    manifest.validate()?;
    write_json(
        root,
        &manifest.relative_dir().join("manifest.json"),
        manifest,
    )
}

pub(crate) fn finalize_audio(
    root: &Path,
    manifest: &mut Manifest,
) -> Result<Option<AudioArtifact>> {
    manifest.validate()?;
    let raw = checked_path(root, &manifest.relative_dir().join("audio.pcm"))?;
    let file = open_read(&raw)?;
    let length = file.metadata()?.len();
    let frames = length / manifest.frame_bytes() as u64;
    manifest.saved_frames = frames;
    if frames == 0 {
        write_manifest(root, manifest)?;
        return Ok(None);
    }
    if length % manifest.frame_bytes() as u64 != 0 {
        manifest.phase = CapturePhase::Interrupted;
        manifest.error = Some("마지막 녹음 구간이 불완전하여 저장된 구간만 복구했습니다.".into());
    }
    let sample_count = frames
        .checked_mul(u64::from(manifest.channels))
        .ok_or_else(|| CoreError::Storage("녹음 길이를 읽을 수 없습니다.".into()))?;
    if sample_count > (u32::MAX as u64 - 1024) / 4 {
        write_manifest(root, manifest)?;
        return Err(CoreError::Storage(
            "WAV 파일 크기 한도를 넘었습니다. 원본 녹음은 보관되어 있습니다.".into(),
        ));
    }
    let wav_relative = manifest.relative_dir().join("audio.wav");
    let wav_path = checked_path(root, &wav_relative)?;
    if wav_path.exists()
        && validate_playback(&wav_path, frames, manifest.sample_rate, manifest.channels).is_err()
    {
        if !fs::symlink_metadata(&wav_path)?.is_file() {
            return Err(CoreError::Storage(
                "재생 파일 경로에 다른 항목이 있습니다.".into(),
            ));
        }
        let backup_relative = manifest
            .relative_dir()
            .join(format!("audio.invalid.{}.wav", Uuid::new_v4()));
        let backup = checked_path(root, &backup_relative)?;
        if backup.exists() {
            return Err(CoreError::Storage(
                "재생 파일의 보존 경로가 이미 존재합니다.".into(),
            ));
        }
        checked_path(root, &wav_relative)?;
        fs::rename(&wav_path, &backup)?;
        sync_parent(&backup)?;
    }
    if !wav_path.exists() {
        let temporary = manifest
            .relative_dir()
            .join(format!("audio.{}.wav.tmp", Uuid::new_v4()));
        let temporary_path = checked_path(root, &temporary)?;
        let output = create_new(&temporary_path)?;
        let durable = output.try_clone()?;
        let mut writer = hound::WavWriter::new(
            BufWriter::new(output),
            hound::WavSpec {
                channels: manifest.channels,
                sample_rate: manifest.sample_rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .map_err(wav_error)?;
        let mut input = BufReader::new(file);
        let mut sample = [0_u8; 8];
        for _ in 0..sample_count {
            input.read_exact(&mut sample[..manifest.encoding.bytes()])?;
            writer
                .write_sample(
                    manifest
                        .encoding
                        .sample(&sample[..manifest.encoding.bytes()]),
                )
                .map_err(wav_error)?;
        }
        writer.finalize().map_err(wav_error)?;
        durable.sync_all()?;
        checked_path(root, &wav_relative)?;
        if wav_path.exists() {
            return Err(CoreError::Storage("재생 파일이 이미 존재합니다.".into()));
        }
        fs::rename(&temporary_path, &wav_path)?;
        sync_parent(&wav_path)?;
    }
    write_manifest(root, manifest)?;
    Ok(Some(AudioArtifact {
        session_id: manifest.session_id.clone(),
        relative_path: wav_relative.to_string_lossy().replace('\\', "/"),
        frames,
        sample_rate: manifest.sample_rate,
        channels: manifest.channels,
    }))
}

fn validate_playback(path: &Path, frames: u64, sample_rate: u32, channels: u16) -> Result<()> {
    let file = open_read(path)?;
    let file_length = file.metadata()?.len();
    let reader = hound::WavReader::new(BufReader::new(file)).map_err(wav_error)?;
    let spec = reader.spec();
    let samples = u64::from(reader.len());
    if spec.channels != channels
        || spec.sample_rate != sample_rate
        || spec.bits_per_sample != 32
        || spec.sample_format != hound::SampleFormat::Float
        || u64::from(reader.duration()) != frames
        || samples != frames.saturating_mul(u64::from(channels))
    {
        return Err(CoreError::Storage(
            "녹음 파일과 저장 정보가 일치하지 않습니다.".into(),
        ));
    }
    let mut input = reader.into_inner();
    let data_start = input.stream_position()?;
    if file_length < data_start.saturating_add(samples.saturating_mul(4)) {
        return Err(CoreError::Storage(
            "녹음 재생 파일의 일부가 누락되었습니다.".into(),
        ));
    }
    Ok(())
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file = open_read(path)?;
    if file.metadata()?.len() > MAX_JSON_BYTES {
        return Err(CoreError::Storage("저장 정보가 너무 큽니다.".into()));
    }
    serde_json::from_reader(BufReader::new(file))
        .map_err(|_| CoreError::Storage("저장 정보의 형식이 올바르지 않습니다.".into()))
}

#[cfg(test)]
mod json_precision_tests {
    use super::*;

    #[test]
    fn persisted_audio_coordinates_keep_their_exact_bits_across_repeated_reads() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let coordinates = vec![
            0.0,
            0.1,
            f64::from_bits(15.2_f64.to_bits() - 1),
            51.248178375505404,
            3600.123456789,
        ];
        let expected: Vec<_> = coordinates.iter().map(|value| value.to_bits()).collect();
        let mut current = coordinates;
        for _ in 0..5 {
            write_json(&root, Path::new("coordinates.json"), &current).unwrap();
            current = read_json(&root.join("coordinates.json")).unwrap();
            assert_eq!(
                current
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
}

#[cfg(test)]
thread_local! {
    static FAIL_JSON_SYNC_FOR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn fail_next_json_sync(path: PathBuf) {
    FAIL_JSON_SYNC_FOR.with(|failure| *failure.borrow_mut() = Some(path));
}

pub(crate) fn write_json(root: &Path, relative: &Path, value: &impl Serialize) -> Result<()> {
    let destination = checked_path(root, relative)?;
    let parent = relative
        .parent()
        .ok_or_else(|| CoreError::Storage("저장 경로가 올바르지 않습니다.".into()))?;
    let temporary = parent.join(format!(".{}.json.tmp", Uuid::new_v4()));
    let temporary_path = checked_path(root, &temporary)?;
    let mut file = create_new(&temporary_path)?;
    serde_json::to_writer(&mut file, value)
        .map_err(|_| CoreError::Storage("저장 정보를 만들 수 없습니다.".into()))?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    checked_path(root, relative)?;
    fs::rename(&temporary_path, &destination)?;
    #[cfg(test)]
    if FAIL_JSON_SYNC_FOR.with(|failure| {
        failure
            .borrow_mut()
            .take_if(|path| path == &destination)
            .is_some()
    }) {
        return Err(CoreError::Storage(
            "Synthetic post-replacement sync failure".into(),
        ));
    }
    sync_parent(&destination)
}

pub(crate) fn checked_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(CoreError::Storage("저장 경로가 올바르지 않습니다.".into()));
    }
    let mut path = root.to_path_buf();
    validate_existing(&path)?;
    for part in relative.components() {
        path.push(part.as_os_str());
        validate_existing(&path)?;
    }
    Ok(path)
}

fn validate_existing(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
                return Err(CoreError::Storage(
                    "연결된 파일이나 폴더는 사용할 수 없습니다.".into(),
                ));
            }
            if metadata.is_file() {
                validate_links(&metadata)?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn validate_removal_tree(path: &Path) -> Result<()> {
    validate_existing(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            validate_removal_tree(&entry?.path())?;
        }
    } else if !metadata.is_file() {
        return Err(CoreError::Storage(
            "일반 녹음 파일이 아닌 항목은 삭제할 수 없습니다.".into(),
        ));
    }
    Ok(())
}

pub(crate) fn ensure_directory(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(CoreError::Storage("절대 저장 경로가 필요합니다.".into()));
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        // A canonical Windows path starts with a verbatim prefix (e.g. \\?\C:).
        // Inspect it only after RootDir makes it a complete absolute path.
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        validate_existing(&current)?;
        if !current.exists() {
            fs::create_dir(&current)?;
        }
        if !fs::metadata(&current)?.is_dir() {
            return Err(CoreError::Storage("저장 폴더를 열 수 없습니다.".into()));
        }
    }
    Ok(())
}

pub(crate) fn create_new(path: &Path) -> Result<File> {
    validate_existing(path)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    no_follow(&mut options);
    let file = options.open(path)?;
    validate_file(&file)?;
    Ok(file)
}

pub(crate) fn open_read(path: &Path) -> Result<File> {
    validate_existing(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    no_follow(&mut options);
    let file = options.open(path)?;
    validate_file(&file)?;
    Ok(file)
}

fn validate_file(file: &File) -> Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || is_reparse_point(&metadata) {
        return Err(CoreError::Storage("일반 파일이 아닙니다.".into()));
    }
    validate_links(&metadata)
}

#[cfg(unix)]
fn validate_links(metadata: &fs::Metadata) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    if metadata.nlink() != 1 {
        return Err(CoreError::Storage(
            "연결된 파일은 사용할 수 없습니다.".into(),
        ));
    }
    Ok(())
}
#[cfg(not(unix))]
fn validate_links(_: &fs::Metadata) -> Result<()> {
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
fn is_reparse_point(_: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn no_follow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
}
#[cfg(windows)]
fn no_follow(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    options.custom_flags(0x0020_0000);
}
#[cfg(not(any(unix, windows)))]
fn no_follow(_: &mut OpenOptions) {}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}
#[cfg(not(unix))]
fn sync_parent(_: &Path) -> Result<()> {
    Ok(())
}

fn wav_error(error: hound::Error) -> CoreError {
    CoreError::Storage(error.to_string())
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}
