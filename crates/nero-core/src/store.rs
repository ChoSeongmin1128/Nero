use crate::audio_playback::Playback;
use crate::disk_space;
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
pub(crate) const MAX_JSON_BYTES: u64 = 16 * 1024 * 1024;

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
        sessions_dir(&self.meeting_id).join(&self.session_id)
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

#[derive(Default)]
pub(crate) struct LoadedMeetings {
    pub meetings: BTreeMap<String, StoredMeeting>,
    // Meeting folders kept on disk but left out of the library because they could
    // not be read. Nothing may treat these IDs as deleted.
    pub unreadable: BTreeSet<String>,
}

// The library layout under the store root, shared by the writer and the reader.
pub(crate) fn meeting_document(id: &str) -> PathBuf {
    Path::new("meetings").join(id).join("meeting.json")
}
pub(crate) fn sessions_dir(id: &str) -> PathBuf {
    Path::new("meetings").join(id).join("sessions")
}
pub(crate) fn transcripts_dir(id: &str) -> PathBuf {
    Path::new("meetings").join(id).join("transcripts")
}
pub(crate) fn summaries_dir(id: &str) -> PathBuf {
    Path::new("meetings").join(id).join("summaries")
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

enum Recovered {
    // A finished capture: its diagnostics are shown without rewriting the meeting.
    Hydrate(Manifest),
    Settled,
    NeedsPlayback,
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

    #[cfg(test)]
    pub fn load(&self) -> Result<BTreeMap<String, StoredMeeting>> {
        Ok(self.load_meetings(&self.load_library()?)?.meetings)
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

    // One unreadable meeting must not keep the whole library closed. Entries whose
    // names are not meeting IDs (Finder's .DS_Store, for example) are not ours.
    pub fn load_meetings(&self, library: &Library) -> Result<LoadedMeetings> {
        let meetings_path = checked_path(&self.root, Path::new("meetings"))?;
        let mut loaded = LoadedMeetings::default();
        for entry in fs::read_dir(meetings_path)? {
            let entry = entry?;
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if validate_id(&id).is_err() || library.pending_purges.contains(&id) {
                continue;
            }
            match self.load_meeting(&entry, &id) {
                Ok(Some(mut stored)) => {
                    library.project(&mut stored.meeting);
                    loaded.meetings.insert(id, stored);
                }
                Ok(None) => {}
                Err(_) => {
                    loaded.unreadable.insert(id);
                }
            }
        }
        Ok(loaded)
    }

    fn load_meeting(&self, entry: &fs::DirEntry, id: &str) -> Result<Option<StoredMeeting>> {
        if entry.file_type()?.is_symlink() || !entry.file_type()?.is_dir() {
            return Err(CoreError::Storage(
                "회의 폴더의 형식이 올바르지 않습니다.".into(),
            ));
        }
        let relative = meeting_document(id);
        let path = checked_path(&self.root, &relative)?;
        if !path.exists() {
            let sessions = checked_path(&self.root, &sessions_dir(id))?;
            if sessions.exists() {
                return Err(CoreError::Storage(
                    "회의 정보가 없는 녹음 폴더가 있습니다.".into(),
                ));
            }
            return Ok(None);
        }
        let stored: StoredMeeting = read_json(&path)?;
        if stored.schema_version != SCHEMA_VERSION || stored.meeting.id != id {
            return Err(CoreError::Storage(
                "올바르지 않은 회의 정보가 있습니다.".into(),
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
            validate_artifact(id, audio)?;
            checked_path(&self.root, Path::new(&audio.relative_path))?;
        }
        Ok(Some(stored))
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
            attendees: Vec::new(),
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
        let relative = meeting_document(&stored.meeting.id);
        let mut base = stored.clone();
        Library::unproject(&mut base.meeting);
        write_json(&self.root, &relative, &base)
    }

    pub fn meeting_matches(&self, stored: &StoredMeeting) -> Result<bool> {
        validate_id(&stored.meeting.id)?;
        let relative = meeting_document(&stored.meeting.id);
        let saved: StoredMeeting = read_json(&checked_path(&self.root, &relative)?)?;
        let mut expected = stored.clone();
        Library::unproject(&mut expected.meeting);
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

    // Each pending meeting is checked and removed on its own: one folder that cannot
    // be removed (a link inside, a locked file) must not keep every later purge from
    // freeing space. The first failure is still reported.
    pub fn cleanup_purges(&self, library: &Library) -> Result<()> {
        let mut first_error = None;
        for id in &library.pending_purges {
            let removed = (|| {
                self.validate_purge(&BTreeSet::from([id.clone()]))?;
                let path = checked_path(&self.root, &Path::new("meetings").join(id))?;
                if path.exists() {
                    // remove_dir_all does not follow links. Validation also refuses linked
                    // or special entries, and the root never includes model directories.
                    fs::remove_dir_all(&path)?;
                    sync_parent(&path)?;
                }
                Ok(())
            })();
            if let Err(error) = removed {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    // True only when the session's PCM holds less than one frame. Anything that cannot
    // be read counts as holding audio, so it is never replaced.
    pub fn capture_is_empty(&self, meeting_id: &str, session_id: &str) -> bool {
        let directory = sessions_dir(meeting_id).join(session_id);
        let manifest = checked_path(&self.root, &directory.join("manifest.json"))
            .and_then(|path| read_json::<Manifest>(&path))
            .ok()
            .filter(|manifest| manifest.validate().is_ok());
        let Some(manifest) = manifest else {
            return false;
        };
        checked_path(&self.root, &directory.join("audio.pcm"))
            .and_then(|path| open_read(&path))
            .and_then(|file| Ok(file.metadata()?.len()))
            .is_ok_and(|length| length < manifest.frame_bytes() as u64)
    }

    pub fn prepare_capture(&self, manifest: &Manifest) -> Result<File> {
        manifest.validate()?;
        disk_space::ensure(
            &checked_path(&self.root, Path::new("meetings"))?,
            disk_space::recording_bytes(manifest),
        )?;
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

    /// Removes a capture session that never became the meeting's audio, such as a
    /// failed import. Its source copy and PCM are this app's own files.
    pub(crate) fn remove_session(&self, manifest: &Manifest) -> Result<()> {
        let directory = checked_path(&self.root, &manifest.relative_dir())?;
        if directory.exists() {
            validate_removal_tree(&directory)?;
            fs::remove_dir_all(&directory)?;
            sync_parent(&directory)?;
        }
        Ok(())
    }

    /// Settles every meeting's state from its capture and returns the meetings whose
    /// playback file still has to be written; `repair_playback` writes it.
    pub fn recover(&self, meetings: &mut BTreeMap<String, StoredMeeting>) -> Result<Vec<String>> {
        let mut playback = Vec::new();
        for stored in meetings.values_mut() {
            let previous = stored.meeting.clone();
            let hydration = match self.recover_meeting(stored) {
                Ok(Recovered::Hydrate(manifest)) => Some(manifest),
                Ok(Recovered::Settled) => None,
                Ok(Recovered::NeedsPlayback) => {
                    playback.push(stored.meeting.id.clone());
                    None
                }
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
                // A full disk must not keep the library closed. The recovered state
                // stays in memory and the next launch derives and saves it again.
                let _ = self.save(stored);
            }
            // Reading a completed capture must not rewrite its meeting document merely
            // to expose diagnostics or metadata added by a newer application build.
            if let Some(manifest) = hydration {
                hydrate_capture_metadata(&mut stored.meeting, &manifest);
            }
        }
        Ok(playback)
    }

    fn recover_meeting(&self, stored: &mut StoredMeeting) -> Result<Recovered> {
        let Some(session_id) = stored.capture_id.clone() else {
            if matches!(
                stored.meeting.status,
                MeetingStatus::Recording | MeetingStatus::Paused
            ) {
                return Err(CoreError::Storage(
                    "녹음 정보가 누락된 회의가 있습니다.".into(),
                ));
            }
            return Ok(Recovered::Settled);
        };
        let relative = sessions_dir(&stored.meeting.id)
            .join(&session_id)
            .join("manifest.json");
        let path = checked_path(&self.root, &relative)?;
        let manifest: Manifest = read_json(&path)?;
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
            return Ok(Recovered::Hydrate(manifest));
        }
        hydrate_capture_metadata(&mut stored.meeting, &manifest);
        // The state settles now; writing the playback file reads the whole PCM, so it
        // is left to `repair_playback`.
        match self.recovered_raw(&manifest) {
            Ok((_, message)) => {
                stored.meeting.status = MeetingStatus::Interrupted;
                stored.meeting.error = Some(message);
                Ok(Recovered::NeedsPlayback)
            }
            Err(error) => {
                stored.meeting.status = MeetingStatus::Interrupted;
                stored.meeting.audio = None;
                stored.meeting.error = Some(format!(
                    "원본 녹음을 읽을 수 없어 복구하지 못했습니다: {error}"
                ));
                Ok(Recovered::Settled)
            }
        }
    }

    // The interrupted session's PCM length and the message for what it holds.
    fn recovered_raw(&self, manifest: &Manifest) -> Result<(u64, String)> {
        let raw = checked_path(&self.root, &manifest.relative_dir().join("audio.pcm"))
            .and_then(|path| open_read(&path))?;
        let raw_bytes = raw.metadata()?.len();
        let message = if raw_bytes < manifest.frame_bytes() as u64 {
            "저장된 오디오가 없어 녹음을 복구하지 못했습니다."
        } else {
            "녹음이 중단되었습니다. 저장된 구간을 복구했습니다."
        };
        Ok((raw_bytes, message.into()))
    }

    /// Writes the playback file of an interrupted recording that `recover` returned.
    pub(crate) fn repair_playback(&self, stored: &mut StoredMeeting) -> Result<()> {
        let session_id = stored
            .capture_id
            .clone()
            .ok_or_else(|| CoreError::Storage("녹음 정보가 누락된 회의가 있습니다.".into()))?;
        let path = checked_path(
            &self.root,
            &sessions_dir(&stored.meeting.id)
                .join(&session_id)
                .join("manifest.json"),
        )?;
        let mut manifest: Manifest = read_json(&path)?;
        manifest.validate()?;
        manifest.warning = stored.meeting.warning.clone();
        let (raw_bytes, message) = self.recovered_raw(&manifest)?;
        manifest.phase = CapturePhase::Interrupted;
        manifest.error = Some(message);
        let playback = stored.meeting.audio.as_ref().and_then(|artifact| {
            (artifact.session_id == manifest.session_id
                && artifact.frames == raw_bytes / manifest.frame_bytes() as u64
                && artifact.sample_rate == manifest.sample_rate
                && artifact.channels == manifest.channels)
                .then(|| Playback::from_artifact(artifact).ok())
                .flatten()
        });
        let outcome = finalize_audio_with_playback(&self.root, &mut manifest, playback);
        stored.meeting.status = MeetingStatus::Interrupted;
        stored.meeting.error = manifest.error.clone();
        match outcome {
            Ok(audio) => stored.meeting.audio = audio,
            Err(error) => {
                stored.meeting.audio = None;
                stored.meeting.error = Some(format!("녹음이 중단되었습니다. 원본은 보관되지만 재생 파일을 만들지 못했습니다. 앱을 다시 열면 다시 만듭니다: {error}"));
            }
        }
        Ok(())
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
        validate_playback(&path, &Playback::from_artifact(artifact)?)?;
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

pub(crate) fn validate_artifact(meeting_id: &str, artifact: &AudioArtifact) -> Result<()> {
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
    Playback::from_artifact(artifact)?;
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
    finalize_audio_with_playback(root, manifest, None)
}

fn finalize_audio_with_playback(
    root: &Path,
    manifest: &mut Manifest,
    playback: Option<Playback>,
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
    let playback = match playback {
        Some(playback) => playback,
        None => Playback::for_capture(frames, manifest.sample_rate, manifest.channels)?,
    };
    let wav_relative = manifest.relative_dir().join("audio.wav");
    let wav_path = checked_path(root, &wav_relative)?;
    if wav_path.exists() && validate_playback(&wav_path, &playback).is_err() {
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
        remove_playback_temporaries(root, manifest)?;
        disk_space::ensure(
            &checked_path(root, &manifest.relative_dir())?,
            disk_space::playback_bytes(&playback),
        )?;
        let temporary = manifest
            .relative_dir()
            .join(format!("audio.{}.wav.tmp", Uuid::new_v4()));
        let temporary_path = checked_path(root, &temporary)?;
        let output = create_new(&temporary_path)?;
        let written = (|| -> Result<()> {
            let durable = output.try_clone()?;
            let mut writer =
                hound::WavWriter::new(BufWriter::new(output), playback.spec).map_err(wav_error)?;
            let mut input = BufReader::new(file);
            playback.write(&mut input, &mut writer, manifest)?;
            writer.finalize().map_err(wav_error)?;
            durable.sync_all()?;
            checked_path(root, &wav_relative)?;
            if wav_path.exists() {
                return Err(CoreError::Storage("재생 파일이 이미 존재합니다.".into()));
            }
            fs::rename(&temporary_path, &wav_path)?;
            Ok(())
        })();
        if let Err(error) = written {
            let _ = fs::remove_file(&temporary_path);
            return Err(error);
        }
        sync_parent(&wav_path)?;
    }
    write_manifest(root, manifest)?;
    Ok(Some(AudioArtifact {
        session_id: manifest.session_id.clone(),
        relative_path: wav_relative.to_string_lossy().replace('\\', "/"),
        frames,
        sample_rate: manifest.sample_rate,
        channels: manifest.channels,
        playback: playback.metadata(),
    }))
}

// A playback WAV left half-written by an earlier crash or full disk is never
// reused; each attempt writes a new uniquely named file.
fn remove_playback_temporaries(root: &Path, manifest: &Manifest) -> Result<()> {
    let directory = checked_path(root, &manifest.relative_dir())?;
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(id) = name
            .strip_prefix("audio.")
            .and_then(|rest| rest.strip_suffix(".wav.tmp"))
        else {
            continue;
        };
        if Uuid::parse_str(id).is_err() || !entry.file_type()?.is_file() {
            continue;
        }
        let path = checked_path(root, &manifest.relative_dir().join(&name))?;
        fs::remove_file(&path)?;
    }
    Ok(())
}

fn validate_playback(path: &Path, playback: &Playback) -> Result<()> {
    let file = open_read(path)?;
    let file_length = file.metadata()?.len();
    let reader = hound::WavReader::new(BufReader::new(file)).map_err(wav_error)?;
    let spec = reader.spec();
    let samples = u64::from(reader.len());
    if spec != playback.spec
        || u64::from(reader.duration()) != playback.frames
        || samples != playback.frames.saturating_mul(u64::from(spec.channels))
    {
        return Err(CoreError::Storage(
            "녹음 파일과 저장 정보가 일치하지 않습니다.".into(),
        ));
    }
    let mut input = reader.into_inner();
    let data_start = input.stream_position()?;
    if file_length
        < data_start.saturating_add(samples.saturating_mul(u64::from(spec.bits_per_sample / 8)))
    {
        return Err(CoreError::Storage(
            "녹음 재생 파일의 일부가 누락되었습니다.".into(),
        ));
    }
    Ok(())
}

/// A field or value this build does not know comes from a newer Nero: an update to
/// make, not damage. Writers stay strict so an older build never drops it on save.
pub(crate) fn written_by_newer_version(error: &serde_json::Error) -> bool {
    let message = error.to_string();
    message.contains("unknown field") || message.contains("unknown variant")
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = read_limited(open_read(path)?, MAX_JSON_BYTES)?
        .ok_or_else(|| CoreError::Storage("저장 정보가 너무 큽니다.".into()))?;
    serde_json::from_slice(&bytes)
        .map_err(|_| CoreError::Storage("저장 정보의 형식이 올바르지 않습니다.".into()))
}

// The file's bytes, or None when it holds more than `limit`. Checked before and
// while reading, so a file that grows meanwhile is refused too.
pub(crate) fn read_limited(file: File, limit: u64) -> Result<Option<Vec<u8>>> {
    if file.metadata()?.len() > limit {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= limit).then_some(bytes))
}

// A new file that must not exist yet, written and synced before returning.
pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = create_new(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

// Every existing component of `path` is a real directory; missing ones are allowed.
pub(crate) fn plain_ancestors(path: &Path) -> std::io::Result<bool> {
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part.as_os_str());
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if is_plain_directory(&metadata) => {}
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

pub(crate) fn is_plain_directory(metadata: &fs::Metadata) -> bool {
    metadata.is_dir() && !metadata.file_type().is_symlink() && !is_reparse_point(metadata)
}

// A regular file that is not a link, reparse point or one of several hard links.
pub(crate) fn is_plain_file(metadata: &fs::Metadata) -> bool {
    metadata.is_file()
        && !metadata.file_type().is_symlink()
        && !is_reparse_point(metadata)
        && single_link(metadata)
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
mod publication_size_tests {
    use super::*;

    #[test]
    fn publication_size_rejects_oversized_library_without_replacing_the_readable_catalog() {
        let temporary = tempfile::tempdir().unwrap();
        let store = Store::open(temporary.path().canonicalize().unwrap().join("library")).unwrap();
        let original = Library::default();
        store.save_library(&original).unwrap();
        let path = store.root.join("library.json");
        let before = fs::read(&path).unwrap();
        let roster: Vec<_> = (1..=1000)
            .map(|id| crate::MeetingAttendee {
                id: Uuid::from_u128(id).to_string(),
                person_id: None,
                name: Some("가".repeat(120)),
            })
            .collect();
        let mut large = original.clone();
        for id in 100_000..100_041 {
            large
                .meeting_attendees
                .insert(Uuid::from_u128(id).to_string(), roster.clone());
        }
        large.validate().unwrap();
        assert!(serde_json::to_vec(&large).unwrap().len() as u64 >= MAX_JSON_BYTES);
        assert!(matches!(
            store.save_library(&large),
            Err(CoreError::InvalidInput(_))
        ));
        assert_eq!(fs::read(path).unwrap(), before);
        assert_eq!(store.load_library().unwrap(), original);
    }

    #[test]
    fn publication_size_includes_the_final_newline_at_the_read_limit() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let relative = Path::new("boundary.json");
        let value = "x".repeat(MAX_JSON_BYTES as usize - 3);
        write_json(&root, relative, &value).unwrap();
        assert_eq!(
            fs::metadata(root.join(relative)).unwrap().len(),
            MAX_JSON_BYTES
        );
        assert_eq!(read_json::<String>(&root.join(relative)).unwrap(), value);
        let oversized = format!("{value}x");
        assert!(matches!(
            write_json(&root, relative, &oversized),
            Err(CoreError::InvalidInput(_))
        ));
        assert_eq!(read_json::<String>(&root.join(relative)).unwrap(), value);
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

pub(crate) fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|_| CoreError::Storage("저장 정보를 만들 수 없습니다.".into()))?;
    if bytes.len() as u64 >= MAX_JSON_BYTES {
        return Err(CoreError::InvalidInput(
            "저장 정보가 읽기 한도 16MiB를 넘습니다.".into(),
        ));
    }
    bytes.push(b'\n');
    Ok(bytes)
}

pub(crate) fn write_json(root: &Path, relative: &Path, value: &impl Serialize) -> Result<()> {
    let destination = checked_path(root, relative)?;
    let bytes = json_bytes(value)?;
    let parent = relative
        .parent()
        .ok_or_else(|| CoreError::Storage("저장 경로가 올바르지 않습니다.".into()))?;
    let temporary = parent.join(format!(".{}.json.tmp", Uuid::new_v4()));
    let temporary_path = checked_path(root, &temporary)?;
    let mut file = create_new(&temporary_path)?;
    file.write_all(&bytes)?;
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
            if metadata.is_file() && !single_link(&metadata) {
                return Err(CoreError::Storage(
                    "연결된 파일은 사용할 수 없습니다.".into(),
                ));
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
    if !single_link(&metadata) {
        return Err(CoreError::Storage(
            "연결된 파일은 사용할 수 없습니다.".into(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn single_link(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() == 1
}
#[cfg(not(unix))]
fn single_link(_: &fs::Metadata) -> bool {
    true
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
pub(crate) fn no_follow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
}
#[cfg(windows)]
pub(crate) fn no_follow(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    options.custom_flags(0x0020_0000);
}
#[cfg(not(any(unix, windows)))]
pub(crate) fn no_follow(_: &mut OpenOptions) {}

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

#[cfg(test)]
#[path = "audio_playback_tests.rs"]
mod playback_tests;

#[cfg(all(test, unix))]
mod purge_tests {
    use super::*;

    #[test]
    fn one_meeting_that_cannot_be_removed_does_not_block_the_others() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("store");
        let store = Store::open(root.clone()).unwrap();
        let blocked = store.create("Blocked".into(), String::new()).unwrap();
        let freed = store.create("Freed".into(), String::new()).unwrap();
        std::os::unix::fs::symlink(
            temp.path(),
            root.join("meetings")
                .join(&blocked.meeting.id)
                .join("linked"),
        )
        .unwrap();
        let mut library = Library::default();
        library.pending_purges.insert(blocked.meeting.id.clone());
        library.pending_purges.insert(freed.meeting.id.clone());
        assert!(store.cleanup_purges(&library).is_err());
        assert!(!root.join("meetings").join(&freed.meeting.id).exists());
        assert!(root.join("meetings").join(&blocked.meeting.id).exists());
    }
}
