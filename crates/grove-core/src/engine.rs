use crate::capture::{self, Capture};
use crate::library::{Folder, Library};
use crate::store::{self, CapturePhase, Manifest, Store, StoredMeeting};
use crate::{
    CoreError, Device, Meeting, MeetingStatus, Result, SessionSnapshot, SessionState, Snapshot,
};
use cpal::traits::DeviceTrait;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub(crate) struct Engine {
    pub(crate) store: Store,
    pub(crate) meetings: BTreeMap<String, StoredMeeting>,
    library: Library,
    pub(crate) capture: Option<Capture>,
    last_session: Option<SessionSnapshot>,
    revision: u64,
    pub(crate) quitting: bool,
}

impl Engine {
    pub fn open(root: PathBuf) -> Result<Self> {
        let store = Store::open(root)?;
        let library = store.load_library()?;
        let mut meetings = store.load_with_library(&library)?;
        store.recover(&mut meetings)?;
        Ok(Self {
            store,
            meetings,
            library,
            capture: None,
            last_session: None,
            revision: 0,
            quitting: false,
        })
    }

    pub fn meetings(&self) -> Vec<Meeting> {
        let mut meetings: Vec<_> = self
            .meetings
            .values()
            .map(|stored| stored.meeting.clone())
            .collect();
        if let Some(capture) = &self.capture {
            if let Some(meeting) = meetings
                .iter_mut()
                .find(|meeting| meeting.id == capture.manifest.meeting_id)
            {
                meeting.warning = capture.signals.warning();
                meeting.capture_quality = capture.signals.quality();
            }
        }
        meetings.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        meetings
    }

    pub fn create_meeting(&mut self, title: String, notes: String) -> Result<Meeting> {
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        let title = if title == "새 회의" {
            let titles: std::collections::BTreeSet<_> = self
                .meetings
                .values()
                .map(|stored| stored.meeting.title.as_str())
                .collect();
            std::iter::once(title.clone())
                .chain((1..).map(|index: u64| format!("새 회의 ({index})")))
                .find(|candidate| !titles.contains(candidate.as_str()))
                .unwrap()
        } else {
            title
        };
        let stored = self.store.create(title, notes)?;
        let meeting = stored.meeting.clone();
        self.meetings.insert(meeting.id.clone(), stored);
        Ok(meeting)
    }

    pub(crate) fn require_active_meeting(&self, id: &str) -> Result<()> {
        store::validate_id(id)?;
        self.meetings
            .get(id)
            .filter(|stored| stored.meeting.deleted_at.is_none())
            .ok_or(CoreError::MeetingNotFound)?;
        Ok(())
    }

    pub fn create_meeting_from_calendar(
        &mut self,
        reference: crate::CalendarReference,
    ) -> Result<Meeting> {
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        let digest = reference.occurrence_key.strip_prefix("google:");
        if !digest.is_some_and(|value| {
            value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) || reference.end_at <= reference.start_at
        {
            return Err(CoreError::InvalidInput(
                "일정 정보를 확인하지 못했습니다.".into(),
            ));
        }
        store::validate_text(&reference.title, "")?;
        if let Some(stored) = self.meetings.values().find(|stored| {
            stored
                .meeting
                .calendar_reference
                .as_ref()
                .is_some_and(|current| current.occurrence_key == reference.occurrence_key)
        }) {
            return Ok(stored.meeting.clone());
        }
        let stored = self.store.create_with_calendar(
            reference.title.clone(),
            String::new(),
            Some(reference),
        )?;
        let meeting = stored.meeting.clone();
        self.meetings.insert(meeting.id.clone(), stored);
        Ok(meeting)
    }

    pub(crate) fn agent_library_version(&self) -> Result<String> {
        crate::agent_control::library_edit_version(&self.library)
    }

    pub fn folders(&self) -> Vec<Folder> {
        self.library.folders()
    }

    pub fn retry_purge_cleanup(&mut self) -> Result<()> {
        self.store.cleanup_purges(&self.library)
    }

    fn commit_library(&mut self, library: Library) -> Result<()> {
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        if let Err(error) = self.store.save_library(&library) {
            // Directory sync can fail after atomic replacement. Reflect the
            // complete committed document rather than returning an older snapshot.
            if self.store.load_library().ok().as_ref() == Some(&library) {
                self.apply_library(library);
            }
            return Err(error);
        }
        self.apply_library(library);
        Ok(())
    }

    fn apply_library(&mut self, library: Library) {
        self.library = library;
        self.meetings
            .retain(|id, _| !self.library.pending_purges.contains(id));
        for stored in self.meetings.values_mut() {
            self.library.project(&mut stored.meeting);
        }
        if self
            .last_session
            .as_ref()
            .is_some_and(|session| self.library.pending_purges.contains(&session.meeting_id))
        {
            self.last_session = None;
        }
        self.revision = self.revision.saturating_add(1);
    }

    pub fn create_folder(&mut self, name: String) -> Result<Folder> {
        let name = self.library.check_name(&name, None)?;
        let folder = Folder {
            id: uuid::Uuid::new_v4().to_string(),
            name,
        };
        let mut library = self.library.clone();
        library.folders.insert(folder.id.clone(), folder.clone());
        self.commit_library(library)?;
        Ok(folder)
    }

    pub fn rename_folder(&mut self, id: &str, name: String) -> Result<Folder> {
        store::validate_id(id)?;
        if !self.library.folders.contains_key(id) {
            return Err(CoreError::InvalidInput("폴더를 찾을 수 없습니다.".into()));
        }
        let folder = Folder {
            id: id.into(),
            name: self.library.check_name(&name, Some(id))?,
        };
        let mut library = self.library.clone();
        library.folders.insert(id.into(), folder.clone());
        self.commit_library(library)?;
        Ok(folder)
    }

    pub fn delete_folder(&mut self, id: &str) -> Result<()> {
        store::validate_id(id)?;
        let mut library = self.library.clone();
        if library.folders.remove(id).is_none() {
            return Err(CoreError::InvalidInput("폴더를 찾을 수 없습니다.".into()));
        }
        for placement in library.placements.values_mut() {
            if placement.folder_id.as_deref() == Some(id) {
                placement.folder_id = None;
            }
        }
        self.commit_library(library)
    }

    fn library_ids(&self, ids: &[String], allow_purged: bool) -> Result<BTreeSet<String>> {
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        if ids.is_empty() {
            return Err(CoreError::InvalidInput("회의를 선택해 주세요.".into()));
        }
        let ids: BTreeSet<_> = ids.iter().cloned().collect();
        for id in &ids {
            store::validate_id(id)?;
            if !self.meetings.contains_key(id)
                && !(allow_purged && self.library.pending_purges.contains(id))
            {
                return Err(CoreError::MeetingNotFound);
            }
        }
        Ok(ids)
    }

    pub fn move_meetings(&mut self, ids: &[String], folder_id: Option<String>) -> Result<()> {
        let ids = self.library_ids(ids, false)?;
        if let Some(id) = &folder_id {
            store::validate_id(id)?;
            if !self.library.folders.contains_key(id) {
                return Err(CoreError::InvalidInput("폴더를 찾을 수 없습니다.".into()));
            }
        }
        for id in &ids {
            self.require_active_meeting(id)?;
        }
        let mut library = self.library.clone();
        for id in ids {
            library.placements.entry(id).or_default().folder_id = folder_id.clone();
        }
        self.commit_library(library)
    }

    pub fn trash_meetings(&mut self, ids: &[String]) -> Result<()> {
        let ids = self.library_ids(ids, false)?;
        if self
            .capture
            .as_ref()
            .is_some_and(|capture| ids.contains(&capture.manifest.meeting_id))
        {
            return Err(CoreError::InvalidState(
                "녹음을 종료한 뒤 삭제해 주세요.".into(),
            ));
        }
        let mut library = self.library.clone();
        let now = store::now();
        for id in ids {
            library
                .placements
                .entry(id)
                .or_default()
                .deleted_at
                .get_or_insert(now);
        }
        self.commit_library(library)
    }

    pub fn restore_meetings(&mut self, ids: &[String]) -> Result<()> {
        let ids = self.library_ids(ids, false)?;
        let mut library = self.library.clone();
        for id in ids {
            library.placements.entry(id).or_default().deleted_at = None;
        }
        self.commit_library(library)
    }

    pub fn purge_meetings(&mut self, ids: &[String]) -> Result<()> {
        let ids = self.library_ids(ids, true)?;
        for id in &ids {
            if self
                .meetings
                .get(id)
                .is_some_and(|stored| stored.meeting.deleted_at.is_none())
            {
                return Err(CoreError::InvalidState(
                    "휴지통에 있는 회의만 영구 삭제할 수 있습니다.".into(),
                ));
            }
        }
        self.store.validate_purge(&ids)?;
        let mut library = self.library.clone();
        for id in &ids {
            library.placements.remove(id);
            library.pending_purges.insert(id.clone());
        }
        self.commit_library(library)?;
        self.store.cleanup_purges(&self.library).map_err(|_| CoreError::Storage(
            "회의를 목록에서 제거했지만 일부 파일을 삭제하지 못했습니다. 앱을 다시 열면 재시도합니다.".into()
        ))
    }

    pub fn update_meeting(&mut self, id: &str, title: String, notes: String) -> Result<Meeting> {
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        self.require_active_meeting(id)?;
        store::validate_text(&title, &notes)?;
        let mut stored = self
            .meetings
            .get(id)
            .ok_or(CoreError::MeetingNotFound)?
            .clone();
        stored.meeting.title = title;
        stored.meeting.notes = notes;
        stored.meeting.updated_at = store::now();
        self.store.save(&stored)?;
        let meeting = stored.meeting.clone();
        self.meetings.insert(id.into(), stored);
        Ok(meeting)
    }

    pub fn set_meeting_date(&mut self, id: &str, meeting_date: Option<String>) -> Result<Meeting> {
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        self.require_active_meeting(id)?;
        if let Some(date) = &meeting_date {
            store::validate_meeting_date(date)?;
        }
        let mut stored = self
            .meetings
            .get(id)
            .ok_or(CoreError::MeetingNotFound)?
            .clone();
        if stored.meeting.meeting_date == meeting_date {
            return Ok(stored.meeting);
        }
        stored.meeting.meeting_date = meeting_date;
        stored.meeting.updated_at = store::now();
        self.store.save(&stored)?;
        let meeting = stored.meeting.clone();
        self.meetings.insert(id.into(), stored);
        Ok(meeting)
    }

    pub fn devices(&self) -> Result<Vec<Device>> {
        capture::devices()
    }

    fn draft(&self, id: &str) -> Result<()> {
        if self.quitting {
            return Err(CoreError::Unavailable);
        }
        self.require_active_meeting(id)?;
        if self.capture.is_some() {
            return Err(CoreError::RecordingBusy);
        }
        let stored = self.meetings.get(id).ok_or(CoreError::MeetingNotFound)?;
        if stored.meeting.status != MeetingStatus::Draft
            || stored.capture_id.is_some()
            || stored.meeting.audio.is_some()
        {
            return Err(CoreError::InvalidState(
                "기존 녹음은 덮어쓰지 않습니다. 새 회의를 만들어 주세요.".into(),
            ));
        }
        Ok(())
    }

    pub fn start(&mut self, meeting_id: &str, device_id: &str) -> Result<Snapshot> {
        self.draft(meeting_id)?;
        let (device, config, encoding) = capture::select_device(device_id)?;
        let id = device
            .id()
            .map_err(|_| CoreError::Audio("마이크를 찾을 수 없습니다.".into()))?
            .to_string();
        let name = device
            .description()
            .map_err(|_| CoreError::Audio("마이크 정보를 읽을 수 없습니다.".into()))?
            .name()
            .to_owned();
        let manifest = Manifest::new(
            meeting_id,
            &id,
            &name,
            config.sample_rate(),
            config.channels(),
            encoding,
        );
        let raw = self.store.prepare_capture(&manifest)?;
        let (mut capture, sink) = Capture::prepare(&self.store.root, manifest, raw)?;
        capture.attach_stream(&device, config, sink)?;
        if let Some(error) = capture.signals.error() {
            return Err(CoreError::Audio(error));
        }
        self.activate(capture)?;
        self.play_capture()?;
        Ok(self.snapshot())
    }

    fn play_capture(&mut self) -> Result<()> {
        let result = self
            .capture
            .as_ref()
            .ok_or(CoreError::Unavailable)?
            .play()
            .and_then(|_| {
                self.capture
                    .as_ref()
                    .and_then(|capture| capture.signals.error())
                    .map_or(Ok(()), |error| Err(CoreError::Audio(error)))
            });
        if let Err(error) = result {
            let _ = self.finish(CapturePhase::Interrupted, Some(error.to_string()));
            return Err(error);
        }
        Ok(())
    }

    fn activate(&mut self, mut capture: Capture) -> Result<()> {
        self.draft(&capture.manifest.meeting_id)?;
        let mut stored = self
            .meetings
            .get(&capture.manifest.meeting_id)
            .ok_or(CoreError::MeetingNotFound)?
            .clone();
        stored.capture_id = Some(capture.manifest.session_id.clone());
        stored.meeting.recorded_at = Some(capture.manifest.started_at);
        stored.meeting.status = MeetingStatus::Recording;
        stored.meeting.error = None;
        stored.meeting.warning = capture.signals.warning();
        stored.meeting.capture_quality = capture.signals.quality();
        stored.meeting.updated_at = store::now();
        self.store.save(&stored)?;
        capture.begin();
        self.meetings.insert(stored.meeting.id.clone(), stored);
        self.last_session = None;
        self.capture = Some(capture);
        Ok(())
    }

    fn check_session(&self, session_id: &str) -> Result<()> {
        store::validate_id(session_id)?;
        if self
            .capture
            .as_ref()
            .map(|capture| capture.manifest.session_id.as_str())
            != Some(session_id)
        {
            return Err(CoreError::StaleSession);
        }
        Ok(())
    }

    pub fn pause(&mut self, session_id: &str) -> Result<Snapshot> {
        self.check_session(session_id)?;
        let capture = self.capture.as_mut().unwrap();
        if capture.manifest.phase == CapturePhase::Paused {
            return Ok(self.snapshot());
        }
        if let Err(error) = capture.pause() {
            let message = error.to_string();
            let _ = self.finish(CapturePhase::Interrupted, Some(message));
            return Err(error);
        }
        let id = capture.manifest.meeting_id.clone();
        if let Err(error) = self.save_phase(&id, MeetingStatus::Paused) {
            let _ = self.finish(CapturePhase::Interrupted, Some(error.to_string()));
            return Err(error);
        }
        Ok(self.snapshot())
    }

    pub fn resume(&mut self, session_id: &str) -> Result<Snapshot> {
        self.check_session(session_id)?;
        let capture = self.capture.as_ref().unwrap();
        if capture.manifest.phase == CapturePhase::Recording {
            return Ok(self.snapshot());
        }
        let id = capture.manifest.meeting_id.clone();
        self.save_phase(&id, MeetingStatus::Recording)?;
        if let Err(error) = self.capture.as_mut().unwrap().resume() {
            let _ = self.finish(CapturePhase::Interrupted, Some(error.to_string()));
            return Err(error);
        }
        Ok(self.snapshot())
    }

    fn save_phase(&mut self, id: &str, status: MeetingStatus) -> Result<()> {
        let mut stored = self
            .meetings
            .get(id)
            .ok_or(CoreError::MeetingNotFound)?
            .clone();
        stored.meeting.status = status;
        stored.meeting.warning = self
            .capture
            .as_ref()
            .and_then(|capture| capture.signals.warning());
        stored.meeting.capture_quality = self
            .capture
            .as_ref()
            .and_then(|capture| capture.signals.quality());
        stored.meeting.updated_at = store::now();
        self.store.save(&stored)?;
        self.meetings.insert(id.into(), stored);
        Ok(())
    }

    pub fn stop(&mut self, session_id: &str) -> Result<Snapshot> {
        self.check_session(session_id)?;
        self.finish(CapturePhase::Completed, None)?;
        Ok(self.snapshot())
    }

    fn finish(&mut self, phase: CapturePhase, error: Option<String>) -> Result<()> {
        let capture = self.capture.take().ok_or(CoreError::StaleSession)?;
        let previous = capture_snapshot(&capture);
        let outcome = match capture.finish(phase, error) {
            Ok(outcome) => outcome,
            Err(error) => {
                let message = error.to_string();
                let mut last = previous;
                last.state = SessionState::Interrupted;
                last.error = Some(message.clone());
                if let Some(stored) = self.meetings.get_mut(&last.meeting_id) {
                    stored.meeting.status = MeetingStatus::Interrupted;
                    stored.meeting.error = Some(message);
                    stored.meeting.warning = last.warning.clone();
                    stored.meeting.capture_quality = last.capture_quality.clone();
                    stored.meeting.updated_at = store::now();
                    let _ = self.store.save(stored);
                }
                self.last_session = Some(last);
                return Err(error);
            }
        };
        let mut stored = self
            .meetings
            .get(&outcome.manifest.meeting_id)
            .ok_or(CoreError::MeetingNotFound)?
            .clone();
        stored.meeting.status = if outcome.manifest.phase == CapturePhase::Completed {
            MeetingStatus::Completed
        } else {
            MeetingStatus::Interrupted
        };
        stored.meeting.audio = outcome.audio;
        stored.meeting.error = outcome.manifest.error.clone();
        stored.meeting.warning = outcome.manifest.warning.clone();
        stored.meeting.capture_quality = outcome.manifest.quality.clone();
        stored.meeting.updated_at = store::now();
        let mut last = session_snapshot(&outcome.manifest, outcome.manifest.saved_frames);
        let saved = self.store.save(&stored);
        if let Err(error) = &saved {
            let message =
                format!("녹음 파일은 보관했지만 회의 정보를 저장하지 못했습니다: {error}");
            stored.meeting.status = MeetingStatus::Interrupted;
            stored.meeting.error = Some(message.clone());
            last.state = SessionState::Interrupted;
            last.error = Some(message);
        }
        self.meetings.insert(stored.meeting.id.clone(), stored);
        self.last_session = Some(last);
        saved
    }

    pub fn poll_capture(&mut self) {
        let error = self
            .capture
            .as_ref()
            .and_then(|capture| capture.signals.error());
        if let Some(error) = error {
            let _ = self.finish(CapturePhase::Interrupted, Some(error));
        }
    }

    pub fn snapshot(&mut self) -> Snapshot {
        self.revision = self.revision.saturating_add(1);
        Snapshot {
            revision: self.revision,
            folders: self.folders(),
            meetings: self.meetings(),
            session: self
                .capture
                .as_ref()
                .map(capture_snapshot)
                .or_else(|| self.last_session.clone()),
        }
    }

    pub fn audio_path(&self, meeting_id: &str) -> Result<PathBuf> {
        self.require_active_meeting(meeting_id)?;
        let meeting = &self
            .meetings
            .get(meeting_id)
            .ok_or(CoreError::MeetingNotFound)?
            .meeting;
        self.store.audio_path(meeting)
    }

    pub fn prepare_shutdown(&mut self, expected_session_id: Option<&str>) -> Result<Snapshot> {
        if self.quitting {
            return Ok(self.snapshot());
        }
        let active_id = self
            .capture
            .as_ref()
            .map(|capture| capture.manifest.session_id.as_str());
        if active_id != expected_session_id {
            return Err(CoreError::StaleSession);
        }
        self.quitting = true;
        if self.capture.is_some() {
            if let Err(error) = self.finish(CapturePhase::Completed, None) {
                self.quitting = false;
                return Err(error);
            }
        }
        Ok(self.snapshot())
    }

    pub fn shutdown(&mut self) {
        if self.capture.is_some() {
            let _ = self.finish(
                CapturePhase::Interrupted,
                Some("앱이 종료되어 녹음을 중단했습니다.".into()),
            );
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn capture_snapshot(capture: &Capture) -> SessionSnapshot {
    let mut snapshot = session_snapshot(&capture.manifest, capture.frames());
    snapshot.warning = capture.signals.warning();
    snapshot.capture_quality = capture.signals.quality();
    snapshot
}

fn session_snapshot(manifest: &Manifest, frames: u64) -> SessionSnapshot {
    SessionSnapshot {
        id: manifest.session_id.clone(),
        meeting_id: manifest.meeting_id.clone(),
        device_id: manifest.device_id.clone(),
        device_name: manifest.device_name.clone(),
        state: match manifest.phase {
            CapturePhase::Prepared | CapturePhase::Recording => SessionState::Recording,
            CapturePhase::Paused => SessionState::Paused,
            CapturePhase::Completed => SessionState::Completed,
            CapturePhase::Interrupted => SessionState::Interrupted,
        },
        elapsed_ms: frames.saturating_mul(1000) / u64::from(manifest.sample_rate),
        saved_frames: frames,
        sample_rate: manifest.sample_rate,
        channels: manifest.channels,
        error: manifest.error.clone(),
        warning: manifest.warning.clone(),
        capture_quality: manifest.quality.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::CallbackSink;
    use crate::store::{checked_path, Encoding};
    use crate::CoreHandle;
    use std::fs;
    use std::io::Write;
    use uuid::Uuid;

    fn engine() -> (tempfile::TempDir, Engine) {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap().join("dev");
        let engine = Engine::open(root).unwrap();
        (temporary, engine)
    }

    fn calendar_reference(letter: &str) -> crate::CalendarReference {
        crate::CalendarReference {
            occurrence_key: format!("google:{}", letter.repeat(64)),
            title: "Synthetic schedule".into(),
            start_at: 1_000_000,
            end_at: 2_000_000,
            calendar_name: "Synthetic calendar".into(),
            join_urls: vec!["https://example.test/meeting".into()],
        }
    }

    #[test]
    fn calendar_meeting_is_a_draft_and_reopening_preserves_manual_fields_and_reference() {
        let (_temporary, mut engine) = engine();
        let reference = calendar_reference("a");
        let meeting = engine
            .create_meeting_from_calendar(reference.clone())
            .unwrap();
        assert_eq!(meeting.status, MeetingStatus::Draft);
        assert_eq!(meeting.title, reference.title);
        assert_eq!(meeting.notes, "");
        assert_eq!(meeting.calendar_reference, Some(reference.clone()));
        assert!(engine.capture.is_none());
        let edited = engine
            .update_meeting(&meeting.id, "Manual title".into(), "Manual notes".into())
            .unwrap();
        let path = engine
            .store
            .root
            .join("meetings")
            .join(&meeting.id)
            .join("meeting.json");
        let before = fs::read(&path).unwrap();
        let mut changed_event = reference;
        changed_event.title = "Changed provider title".into();
        changed_event.start_at += 1;
        assert_eq!(
            engine
                .create_meeting_from_calendar(changed_event.clone())
                .unwrap(),
            edited
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(engine.meetings.len(), 1);
        let root = engine.store.root.clone();
        drop(engine);
        let mut reopened = Engine::open(root).unwrap();
        assert_eq!(
            reopened
                .create_meeting_from_calendar(changed_event)
                .unwrap(),
            edited
        );
    }

    #[test]
    fn trashed_calendar_occurrence_returns_existing_without_restore_or_duplicate() {
        let (_temporary, mut engine) = engine();
        let reference = calendar_reference("b");
        let meeting = engine
            .create_meeting_from_calendar(reference.clone())
            .unwrap();
        engine.trash_meetings(&[meeting.id.clone()]).unwrap();
        let existing = engine.create_meeting_from_calendar(reference).unwrap();
        assert_eq!(existing.id, meeting.id);
        assert!(existing.deleted_at.is_some());
        assert_eq!(engine.meetings.len(), 1);
    }

    #[test]
    fn separate_recurring_occurrences_create_separate_drafts_and_plain_meetings_have_no_reference()
    {
        let (_temporary, mut engine) = engine();
        let first = engine
            .create_meeting_from_calendar(calendar_reference("a"))
            .unwrap();
        let second = engine
            .create_meeting_from_calendar(calendar_reference("b"))
            .unwrap();
        assert_ne!(first.id, second.id);
        let plain = engine
            .create_meeting("New meeting".into(), String::new())
            .unwrap();
        assert_eq!(plain.calendar_reference, None);
        let mut legacy = serde_json::to_value(&plain).unwrap();
        legacy.as_object_mut().unwrap().remove("calendarReference");
        assert_eq!(serde_json::from_value::<Meeting>(legacy).unwrap(), plain);
    }

    #[test]
    fn concurrent_calendar_requests_share_one_meeting() {
        let temporary = tempfile::tempdir().unwrap();
        let core =
            CoreHandle::open(temporary.path().canonicalize().unwrap().join("library")).unwrap();
        let other = core.clone();
        let first = std::thread::spawn(move || {
            other
                .create_meeting_from_calendar(calendar_reference("a"))
                .unwrap()
        });
        let second = core
            .create_meeting_from_calendar(calendar_reference("a"))
            .unwrap();
        assert_eq!(first.join().unwrap().id, second.id);
        assert_eq!(core.list_meetings().unwrap().len(), 1);
    }

    #[test]
    fn calendar_recording_reopens_completed_occurrence_without_touching_audio_or_starting_device() {
        let (_temporary, mut engine) = engine();
        let reference = calendar_reference("a");
        let meeting = engine
            .create_meeting_from_calendar(reference.clone())
            .unwrap();
        let (session, mut sink) = synthetic(&mut engine, &meeting.id);
        sink.write(&[0.5_f32.to_le_bytes(); 2].concat());
        engine.stop(&session).unwrap();
        let audio = engine.audio_path(&meeting.id).unwrap();
        let before = fs::read(&audio).unwrap();
        let completed = engine.meetings[&meeting.id].meeting.clone();
        let root = engine.store.root.clone();
        drop(engine);
        let core = CoreHandle::open(root).unwrap();
        let (existing, snapshot) = core
            .create_calendar_recording(reference, "unavailable synthetic device")
            .unwrap();
        assert_eq!(existing, completed);
        assert_eq!(snapshot.meetings.len(), 1);
        assert_eq!(fs::read(audio).unwrap(), before);
        assert!(!snapshot
            .session
            .is_some_and(|session| session.state == SessionState::Recording));
    }

    fn synthetic(engine: &mut Engine, meeting_id: &str) -> (String, CallbackSink) {
        engine.draft(meeting_id).unwrap();
        let manifest = Manifest::new(meeting_id, "synthetic", "시험 입력", 1000, 1, Encoding::F32);
        let id = manifest.session_id.clone();
        let raw = engine.store.prepare_capture(&manifest).unwrap();
        let (capture, sink) = Capture::prepare(&engine.store.root, manifest, raw).unwrap();
        engine.activate(capture).unwrap();
        (id, sink)
    }

    #[test]
    fn handle_preserves_meeting_and_notes_without_accessing_audio() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap().join("dev");
        let handle = CoreHandle::open(&root).unwrap();
        let clone = handle.clone();
        let meeting = handle
            .create_meeting("긴 회의 제목", "한글 메모\n확정 전 내용")
            .unwrap();
        assert!(matches!(CoreHandle::open(&root), Err(CoreError::StoreBusy)));
        let before = handle.snapshot().unwrap();
        let changed = clone
            .update_meeting(&meeting.id, "변경한 제목", "수정한 메모")
            .unwrap();
        let after = handle.snapshot().unwrap();
        assert!(after.revision > before.revision);
        drop(handle);
        assert_eq!(clone.list_meetings().unwrap()[0], changed);
        drop(clone);
        let reopened = CoreHandle::open(&root).unwrap();
        assert_eq!(reopened.list_meetings().unwrap(), vec![changed]);
    }

    #[test]
    fn quality_warning_is_live_and_survives_stop_and_reopen() {
        let (_temporary, mut engine) = engine();
        let meeting = engine.create_meeting("회의".into(), "메모".into()).unwrap();
        let (session_id, mut sink) = synthetic(&mut engine, &meeting.id);
        engine
            .capture
            .as_ref()
            .unwrap()
            .signals
            .device_event(cpal::ErrorKind::Xrun.into());
        engine.poll_capture();
        assert!(engine.capture.is_some());
        let snapshot = engine.snapshot();
        let warning = snapshot.session.as_ref().unwrap().warning.clone();
        let quality = snapshot.session.as_ref().unwrap().capture_quality.clone();
        assert!(warning.is_some());
        assert_eq!(quality.as_ref().unwrap().xrun_count, 1);
        assert_eq!(snapshot.session.unwrap().state, SessionState::Recording);
        assert_eq!(snapshot.meetings[0].warning, warning);
        assert_eq!(snapshot.meetings[0].capture_quality, quality);
        sink.write(&[0.25_f32.to_le_bytes(); 10].concat());
        engine.pause(&session_id).unwrap();
        let saved: StoredMeeting = serde_json::from_slice(
            &fs::read(
                engine
                    .store
                    .root
                    .join("meetings")
                    .join(&meeting.id)
                    .join("meeting.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(saved.meeting.warning, warning);
        assert_eq!(saved.meeting.capture_quality, quality);
        engine.resume(&session_id).unwrap();
        sink.write(&[0.5_f32.to_le_bytes(); 10].concat());
        let snapshot = engine.stop(&session_id).unwrap();
        assert_eq!(
            snapshot.session.as_ref().unwrap().state,
            SessionState::Completed
        );
        assert_eq!(snapshot.session.unwrap().warning, warning);
        let finished = &snapshot.meetings[0];
        assert_eq!(finished.status, MeetingStatus::Completed);
        assert!(finished.error.is_none());
        assert_eq!(finished.warning, warning);
        assert_eq!(finished.capture_quality, quality);
        assert_eq!(finished.audio.as_ref().unwrap().frames, 20);
        let root = engine.store.root.clone();
        drop(sink);
        drop(engine);
        let reopened = Engine::open(root).unwrap();
        assert_eq!(reopened.meetings[&meeting.id].meeting, *finished);
    }

    #[test]
    fn play_failure_preserves_the_saved_session_notes_and_first_callback() {
        let (_temporary, mut engine) = engine();
        let meeting = engine
            .create_meeting("회의".into(), "원래 메모".into())
            .unwrap();
        engine
            .set_meeting_date(&meeting.id, Some("2026-10-20".into()))
            .unwrap();
        let (session_id, mut sink) = synthetic(&mut engine, &meeting.id);
        let path = engine
            .store
            .root
            .join("meetings")
            .join(&meeting.id)
            .join("meeting.json");
        let saved: StoredMeeting = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.capture_id.as_deref(), Some(session_id.as_str()));
        assert_eq!(saved.meeting.status, MeetingStatus::Recording);
        let first_callback = [0.25_f32.to_le_bytes(); 10].concat();
        sink.write(&first_callback);
        engine.capture.as_ref().unwrap().flush().unwrap();
        let session_path = engine
            .store
            .root
            .join(engine.capture.as_ref().unwrap().manifest.relative_dir());
        assert!(matches!(engine.play_capture(), Err(CoreError::Audio(_))));
        assert!(engine.capture.is_none());
        assert_eq!(
            fs::read(session_path.join("audio.pcm")).unwrap(),
            first_callback
        );
        let saved: StoredMeeting = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.capture_id.as_deref(), Some(session_id.as_str()));
        assert_eq!(saved.meeting.notes, "원래 메모");
        assert_eq!(saved.meeting.meeting_date.as_deref(), Some("2026-10-20"));
        assert_eq!(saved.meeting.status, MeetingStatus::Interrupted);
        assert!(saved.meeting.error.is_some());
        assert_eq!(saved.meeting.audio.as_ref().unwrap().frames, 10);
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(session_path.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest.phase, CapturePhase::Interrupted);
        assert!(manifest.error.is_some());
        let root = engine.store.root.clone();
        drop(sink);
        drop(engine);
        let reopened = Engine::open(root).unwrap();
        assert_eq!(reopened.meetings[&meeting.id].meeting, saved.meeting);
    }

    #[test]
    fn recovery_keeps_checkpoint_quality_and_reconciles_completed_meeting_warning() {
        let (_temporary, mut engine) = engine();
        let quality = store::CaptureQuality {
            xrun_count: 2,
            realtime_denied_count: 0,
            route_changed_count: 1,
            ..Default::default()
        };
        let warning = quality.warning();
        let mut session_paths = Vec::new();
        let mut ids = Vec::new();
        for phase in [CapturePhase::Recording, CapturePhase::Completed] {
            let meeting = engine.create_meeting("회의".into(), "메모".into()).unwrap();
            let mut manifest = Manifest::new(
                &meeting.id,
                "synthetic",
                "시험 입력",
                1000,
                1,
                Encoding::F32,
            );
            let mut raw = engine.store.prepare_capture(&manifest).unwrap();
            raw.write_all(&[0.25_f32.to_le_bytes(); 10].concat())
                .unwrap();
            raw.sync_all().unwrap();
            drop(raw);
            manifest.phase = phase;
            manifest.quality = Some(quality.clone());
            manifest.warning = warning.clone();
            let audio = if phase == CapturePhase::Completed {
                store::finalize_audio(&engine.store.root, &mut manifest).unwrap()
            } else {
                store::write_manifest(&engine.store.root, &manifest).unwrap();
                None
            };
            let stored = engine.meetings.get_mut(&meeting.id).unwrap();
            stored.capture_id = Some(manifest.session_id.clone());
            stored.meeting.status = if phase == CapturePhase::Completed {
                MeetingStatus::Completed
            } else {
                MeetingStatus::Recording
            };
            stored.meeting.audio = audio;
            engine.store.save(stored).unwrap();
            session_paths.push(engine.store.root.join(manifest.relative_dir()));
            ids.push(meeting.id);
        }
        let root = engine.store.root.clone();
        drop(engine);
        let reopened = Engine::open(root.clone()).unwrap();
        for (index, id) in ids.iter().enumerate() {
            let meeting = &reopened.meetings[id].meeting;
            assert_eq!(meeting.warning, warning);
            assert_eq!(
                meeting.status,
                if index == 0 {
                    MeetingStatus::Interrupted
                } else {
                    MeetingStatus::Completed
                }
            );
            assert_eq!(meeting.audio.as_ref().unwrap().frames, 10);
            let manifest: Manifest = serde_json::from_slice(
                &fs::read(session_paths[index].join("manifest.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(manifest.quality.as_ref(), Some(&quality));
            assert_eq!(manifest.warning, warning);
        }
        drop(reopened);
        let reopened = Engine::open(root).unwrap();
        for id in ids {
            assert_eq!(reopened.meetings[&id].meeting.warning, warning);
        }
    }

    #[test]
    fn completed_legacy_diagnostics_hydrate_without_rewriting_meeting_manifest_or_audio() {
        let (_temporary, mut engine) = engine();
        let meeting = engine.create_meeting("회의".into(), "메모".into()).unwrap();
        let mut manifest = Manifest::new(
            &meeting.id,
            "synthetic",
            "시험 입력",
            1000,
            1,
            Encoding::F32,
        );
        let mut raw = engine.store.prepare_capture(&manifest).unwrap();
        raw.write_all(&[0.25_f32.to_le_bytes(); 10].concat())
            .unwrap();
        raw.sync_all().unwrap();
        drop(raw);
        manifest.phase = CapturePhase::Completed;
        manifest.quality = Some(store::CaptureQuality {
            realtime_denied_count: 1,
            ..Default::default()
        });
        manifest.warning = Some("Legacy priority warning".into());
        let audio = store::finalize_audio(&engine.store.root, &mut manifest).unwrap();
        let stored = engine.meetings.get_mut(&meeting.id).unwrap();
        stored.capture_id = Some(manifest.session_id.clone());
        stored.meeting.status = MeetingStatus::Completed;
        stored.meeting.audio = audio;
        engine.store.save(stored).unwrap();
        let root = engine.store.root.clone();
        let paths = [
            root.join("meetings").join(&meeting.id).join("meeting.json"),
            root.join(manifest.relative_dir()).join("manifest.json"),
            root.join(manifest.relative_dir()).join("audio.pcm"),
            root.join(manifest.relative_dir()).join("audio.wav"),
        ];
        let before = paths.each_ref().map(|path| fs::read(path).unwrap());
        let expected_edit_version =
            crate::agent_control::meeting_edit_version(&stored.meeting).unwrap();
        drop(engine);

        let reopened = Engine::open(root.clone()).unwrap();
        let hydrated = &reopened.meetings[&meeting.id].meeting;
        assert_eq!(hydrated.recorded_at, Some(manifest.started_at));
        assert!(hydrated.warning.is_none());
        assert_eq!(hydrated.capture_quality, manifest.quality);
        assert_eq!(
            crate::agent_control::meeting_edit_version(hydrated).unwrap(),
            expected_edit_version
        );
        let reader = crate::reader::LibraryReader::open(&root).unwrap();
        let details = reader
            .query("meeting_get", serde_json::json!({"meetingId":meeting.id}))
            .unwrap();
        assert_eq!(
            details["meeting"]["captureQuality"]["realtimeDeniedCount"],
            1
        );
        assert!(details["meeting"]["captureQuality"].get("events").is_none());
        assert_eq!(
            details["meeting"]["captureQualityMeaning"]["audioLossVerified"],
            false
        );
        assert_eq!(details["editVersion"], expected_edit_version);
        let listed = reader
            .query("meetings_list", serde_json::json!({}))
            .unwrap();
        assert!(listed["meetings"][0].get("captureQuality").is_none());
        assert_eq!(listed["meetings"][0]["editVersion"], expected_edit_version);
        assert_eq!(paths.each_ref().map(|path| fs::read(path).unwrap()), before);
        drop(reopened);
        let reopened = Engine::open(root).unwrap();
        assert!(reopened.meetings[&meeting.id].meeting.warning.is_none());
        assert_eq!(paths.each_ref().map(|path| fs::read(path).unwrap()), before);
    }

    #[test]
    fn optional_capture_diagnostic_damage_preserves_completed_meetings_and_all_file_bytes() {
        use serde_json::json;
        let valid = json!({
            "xrunCount": 1,
            "realtimeDeniedCount": 0,
            "routeChangedCount": 0,
            "events": [{
                "kind": "xrun", "count": 1,
                "firstObservedAtMs": 1000, "lastObservedAtMs": 1000,
                "firstSavedFrame": 5, "lastSavedFrame": 5,
                "firstQueuedFrame": 10, "lastQueuedFrame": 10,
                "coalesced": false
            }]
        });
        let mut unknown_kind = valid.clone();
        unknown_kind["events"][0]["kind"] = json!("xurn");
        let mut missing_field = valid.clone();
        missing_field["events"][0]
            .as_object_mut()
            .unwrap()
            .remove("count");
        let mut invalid_range = valid.clone();
        invalid_range["events"][0]["firstSavedFrame"] = json!(11);
        let mut wrong_events_type = valid.clone();
        wrong_events_type["events"] = json!("invalid events");
        let mut too_many_events = valid.clone();
        too_many_events["events"] = json!(vec![valid["events"][0].clone(); 65]);
        for diagnostic in [
            unknown_kind,
            missing_field,
            invalid_range,
            wrong_events_type,
            too_many_events,
            json!("invalid quality"),
        ] {
            for target in ["meeting", "manifest", "both"] {
                let (_temporary, mut engine) = engine();
                let meeting = engine.create_meeting("회의".into(), "메모".into()).unwrap();
                let mut manifest = Manifest::new(
                    &meeting.id,
                    "synthetic",
                    "시험 입력",
                    1000,
                    1,
                    Encoding::F32,
                );
                manifest.quality = Some(serde_json::from_value(valid.clone()).unwrap());
                let mut raw = engine.store.prepare_capture(&manifest).unwrap();
                raw.write_all(&[0.25_f32.to_le_bytes(); 10].concat())
                    .unwrap();
                raw.sync_all().unwrap();
                drop(raw);
                manifest.phase = CapturePhase::Completed;
                let audio = store::finalize_audio(&engine.store.root, &mut manifest).unwrap();
                let stored = engine.meetings.get_mut(&meeting.id).unwrap();
                stored.capture_id = Some(manifest.session_id.clone());
                stored.meeting.status = MeetingStatus::Completed;
                stored.meeting.audio = audio.clone();
                stored.meeting.capture_quality = manifest.quality.clone();
                engine.store.save(stored).unwrap();
                let root = engine.store.root.clone();
                let paths = [
                    root.join("meetings").join(&meeting.id).join("meeting.json"),
                    root.join(manifest.relative_dir()).join("manifest.json"),
                    root.join(manifest.relative_dir()).join("audio.pcm"),
                    root.join(manifest.relative_dir()).join("audio.wav"),
                ];
                drop(engine);
                if matches!(target, "meeting" | "both") {
                    let mut value: serde_json::Value =
                        serde_json::from_slice(&fs::read(&paths[0]).unwrap()).unwrap();
                    value["meeting"]["captureQuality"] = diagnostic.clone();
                    fs::write(&paths[0], serde_json::to_vec(&value).unwrap()).unwrap();
                }
                if matches!(target, "manifest" | "both") {
                    let mut value: serde_json::Value =
                        serde_json::from_slice(&fs::read(&paths[1]).unwrap()).unwrap();
                    value["quality"] = diagnostic.clone();
                    fs::write(&paths[1], serde_json::to_vec(&value).unwrap()).unwrap();
                }
                let before = paths.each_ref().map(|path| fs::read(path).unwrap());
                let expected_quality =
                    (target == "meeting").then(|| manifest.quality.clone().unwrap());
                let reopened = Engine::open(root.clone()).unwrap();
                let hydrated = &reopened.meetings[&meeting.id].meeting;
                assert_eq!(
                    hydrated.status,
                    MeetingStatus::Completed,
                    "{target}: {diagnostic}"
                );
                assert!(hydrated.error.is_none(), "{target}: {diagnostic}");
                assert_eq!(hydrated.audio, audio);
                assert_eq!(hydrated.capture_quality, expected_quality);
                let reader = crate::reader::LibraryReader::open(&root).unwrap();
                let details = reader
                    .query("meeting_get", json!({"meetingId":meeting.id}))
                    .unwrap();
                assert_eq!(details["meeting"]["status"], "completed");
                assert!(details["meeting"].get("error").is_none());
                if target == "meeting" {
                    assert_eq!(details["meeting"]["captureQuality"], valid);
                } else {
                    assert!(details["meeting"].get("captureQuality").is_none());
                    assert!(details["meeting"].get("captureQualityMeaning").is_none());
                }
                let listed = reader.query("meetings_list", json!({})).unwrap();
                assert!(listed["meetings"][0].get("captureQuality").is_none());
                assert_eq!(paths.each_ref().map(|path| fs::read(path).unwrap()), before);
                drop(reopened);
                let reopened = Engine::open(root).unwrap();
                assert_eq!(
                    reopened.meetings[&meeting.id].meeting.status,
                    MeetingStatus::Completed
                );
                assert_eq!(paths.each_ref().map(|path| fs::read(path).unwrap()), before);
            }
        }
    }

    #[test]
    fn optional_diagnostic_tolerance_does_not_accept_invalid_capture_metadata() {
        use serde_json::json;
        let manifest = Manifest::new(
            &uuid::Uuid::new_v4().to_string(),
            "synthetic",
            "시험 입력",
            1000,
            1,
            Encoding::F32,
        );
        let mut value = serde_json::to_value(&manifest).unwrap();
        value["quality"] = json!("invalid optional diagnostic");
        let decoded: Manifest = serde_json::from_value(value.clone()).unwrap();
        assert!(decoded.quality.is_none());
        decoded.validate().unwrap();
        value["sampleRate"] = json!(0);
        let decoded: Manifest = serde_json::from_value(value.clone()).unwrap();
        assert!(decoded.validate().is_err());
        value["sampleRate"] = json!("invalid required metadata");
        assert!(serde_json::from_value::<Manifest>(value).is_err());
        let (_temporary, mut engine) = engine();
        let meeting = engine.create_meeting("회의".into(), "메모".into()).unwrap();
        let mut value = serde_json::to_value(&meeting).unwrap();
        value["captureQuality"] = json!("invalid optional diagnostic");
        value["notes"] = json!(false);
        assert!(serde_json::from_value::<Meeting>(value).is_err());
    }

    #[test]
    fn old_meeting_json_opens_without_inventing_recording_or_meeting_dates() {
        let (_temporary, mut engine) = engine();
        let meeting = engine
            .create_meeting("기존 회의".into(), "메모".into())
            .unwrap();
        let path = engine
            .store
            .root
            .join("meetings")
            .join(&meeting.id)
            .join("meeting.json");
        let mut stored: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        stored["meeting"]
            .as_object_mut()
            .unwrap()
            .remove("meetingDate");
        stored["meeting"]
            .as_object_mut()
            .unwrap()
            .remove("recordedAt");
        let original = serde_json::to_vec(&stored).unwrap();
        fs::write(&path, &original).unwrap();
        let root = engine.store.root.clone();
        drop(engine);
        let restored = Engine::open(root).unwrap();
        let restored = &restored.meetings[&meeting.id].meeting;
        assert_eq!(restored, &meeting);
        assert_eq!(restored.meeting_date, None);
        assert_eq!(restored.recorded_at, None);
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn meeting_date_persists_and_clears_without_changing_recording_origin() {
        let (_temporary, mut engine) = engine();
        let meeting = engine.create_meeting("회의".into(), "메모".into()).unwrap();
        engine
            .set_meeting_date(&meeting.id, Some("2024-02-29".into()))
            .unwrap();
        let mut manifest = Manifest::new(
            &meeting.id,
            "synthetic",
            "시험 입력",
            1000,
            1,
            Encoding::F32,
        );
        manifest.started_at = 1_700_000_000_123;
        let raw = engine.store.prepare_capture(&manifest).unwrap();
        let (capture, mut sink) =
            Capture::prepare(&engine.store.root, manifest.clone(), raw).unwrap();
        engine.activate(capture).unwrap();
        sink.write(&[0.25_f32.to_le_bytes(); 10].concat());
        engine.stop(&manifest.session_id).unwrap();
        let updated = engine
            .update_meeting(&meeting.id, "수정한 제목".into(), "수정한 메모".into())
            .unwrap();
        assert_eq!(updated.created_at, meeting.created_at);
        assert_eq!(updated.recorded_at, Some(manifest.started_at));
        assert_eq!(updated.meeting_date.as_deref(), Some("2024-02-29"));
        let root = engine.store.root.clone();
        drop(sink);
        drop(engine);
        let handle = CoreHandle::open(&root).unwrap();
        assert_eq!(handle.list_meetings().unwrap(), vec![updated]);
        let cleared = handle.set_meeting_date(&meeting.id, None).unwrap();
        assert_eq!(cleared.meeting_date, None);
        assert_eq!(cleared.recorded_at, Some(manifest.started_at));
        assert_eq!(cleared.created_at, meeting.created_at);
        drop(handle);
        assert_eq!(
            CoreHandle::open(root).unwrap().list_meetings().unwrap(),
            vec![cleared]
        );
    }

    #[test]
    fn invalid_meeting_dates_leave_saved_metadata_unchanged() {
        let (_temporary, mut engine) = engine();
        let meeting = engine.create_meeting("회의".into(), "메모".into()).unwrap();
        let path = engine
            .store
            .root
            .join("meetings")
            .join(&meeting.id)
            .join("meeting.json");
        let before = fs::read(&path).unwrap();
        for date in [
            "",
            "2026-1-01",
            "2026-01-1",
            "0000-01-01",
            "10000-01-01",
            "2026-00-01",
            "2026-13-01",
            "2026-01-00",
            "2026-04-31",
            "2026-02-29",
            "1900-02-29",
            "2024-02-30",
            "2026/01/01",
            "２０２６-01-01",
            "2026-01-01 ",
        ] {
            assert!(
                matches!(
                    engine.set_meeting_date(&meeting.id, Some(date.into())),
                    Err(CoreError::InvalidInput(_))
                ),
                "{date}"
            );
            assert_eq!(engine.meetings[&meeting.id].meeting, meeting);
            assert_eq!(fs::read(&path).unwrap(), before);
        }
        for date in ["0001-01-01", "2000-02-29", "2024-02-29", "9999-12-31"] {
            assert!(
                engine
                    .set_meeting_date(&meeting.id, Some(date.into()))
                    .is_ok(),
                "{date}"
            );
        }
    }

    #[test]
    fn recovery_uses_microphone_manifest_date_but_keeps_import_origin_unknown() {
        let (_temporary, mut engine) = engine();
        let mut microphone = None;
        let mut imported = None;
        for device in ["synthetic", "import"] {
            let meeting = engine
                .create_meeting("기존 녹음".into(), "메모".into())
                .unwrap();
            let mut manifest =
                Manifest::new(&meeting.id, device, "시험 입력", 1000, 1, Encoding::F32);
            manifest.started_at = 1_600_000_000_123;
            let mut raw = engine.store.prepare_capture(&manifest).unwrap();
            raw.write_all(&[0.125_f32.to_le_bytes(); 10].concat())
                .unwrap();
            raw.sync_all().unwrap();
            drop(raw);
            manifest.phase = CapturePhase::Completed;
            let audio = store::finalize_audio(&engine.store.root, &mut manifest).unwrap();
            let stored = engine.meetings.get_mut(&meeting.id).unwrap();
            stored.capture_id = Some(manifest.session_id.clone());
            stored.meeting.status = MeetingStatus::Completed;
            stored.meeting.audio = audio;
            engine.store.save(stored).unwrap();
            if device == "import" {
                imported = Some(meeting);
            } else {
                microphone = Some(meeting);
            }
        }
        let root = engine.store.root.clone();
        drop(engine);
        let restored = Engine::open(root.clone()).unwrap();
        let microphone = microphone.unwrap();
        let imported = imported.unwrap();
        let recorded = &restored.meetings[&microphone.id].meeting;
        assert_eq!(recorded.recorded_at, Some(1_600_000_000_123));
        assert_eq!(recorded.created_at, microphone.created_at);
        assert_eq!(recorded.updated_at, microphone.updated_at);
        let unknown = &restored.meetings[&imported.id].meeting;
        assert_eq!(unknown.recorded_at, None);
        assert_eq!(unknown.created_at, imported.created_at);
        drop(restored);
        let reopened = Engine::open(root).unwrap();
        assert_eq!(
            reopened.meetings[&microphone.id].meeting.recorded_at,
            Some(1_600_000_000_123)
        );
        assert_eq!(reopened.meetings[&imported.id].meeting.recorded_at, None);
    }

    #[test]
    fn default_titles_fill_available_suffixes_and_survive_reopen() {
        let (_temporary, mut engine) = engine();
        let first = engine.create_meeting("새 회의".into(), "".into()).unwrap();
        let second = engine.create_meeting("새 회의".into(), "".into()).unwrap();
        let third = engine.create_meeting("새 회의".into(), "".into()).unwrap();
        assert_eq!(first.title, "새 회의");
        assert_eq!(second.title, "새 회의 (1)");
        assert_eq!(third.title, "새 회의 (2)");
        engine
            .update_meeting(&second.id, "주간 회의".into(), "".into())
            .unwrap();
        assert_eq!(
            engine
                .create_meeting("새 회의".into(), "".into())
                .unwrap()
                .title,
            "새 회의 (1)"
        );
        let custom = engine
            .create_meeting("주간 회의".into(), "".into())
            .unwrap();
        assert_eq!(custom.title, "주간 회의");
        let root = engine.store.root.clone();
        drop(engine);
        let mut restored = Engine::open(root).unwrap();
        assert_eq!(
            restored
                .create_meeting("새 회의".into(), "".into())
                .unwrap()
                .title,
            "새 회의 (3)"
        );
        assert_eq!(
            restored
                .create_meeting("새 회의 (2)".into(), "".into())
                .unwrap()
                .title,
            "새 회의 (2)"
        );
    }

    #[test]
    fn concurrent_default_meeting_creation_reserves_titles_in_core() {
        let temporary = tempfile::tempdir().unwrap();
        let handle =
            CoreHandle::open(temporary.path().canonicalize().unwrap().join("dev")).unwrap();
        let callers: Vec<_> = (0..8)
            .map(|_| {
                let handle = handle.clone();
                std::thread::spawn(move || handle.create_meeting("새 회의", "").unwrap().title)
            })
            .collect();
        let titles: std::collections::BTreeSet<_> = callers
            .into_iter()
            .map(|caller| caller.join().unwrap())
            .collect();
        assert_eq!(titles.len(), 8);
        assert!(titles.contains("새 회의"));
        for index in 1..8 {
            assert!(titles.contains(&format!("새 회의 ({index})")));
        }
    }

    #[test]
    fn stale_session_cannot_pause_or_stop_another_recording() {
        let (_temporary, mut engine) = engine();
        let first = engine.create_meeting("첫 회의".into(), "".into()).unwrap();
        let second = engine
            .create_meeting("다음 회의".into(), "".into())
            .unwrap();
        let (first_id, mut sink) = synthetic(&mut engine, &first.id);
        assert!(matches!(
            engine.draft(&second.id),
            Err(CoreError::RecordingBusy)
        ));
        assert!(matches!(
            engine.pause(&Uuid::new_v4().to_string()),
            Err(CoreError::StaleSession)
        ));
        sink.write(&[0.25_f32.to_le_bytes(); 10].concat());
        engine.stop(&first_id).unwrap();
        drop(sink);
        let (second_id, _sink) = synthetic(&mut engine, &second.id);
        assert!(matches!(
            engine.stop(&first_id),
            Err(CoreError::StaleSession)
        ));
        assert_eq!(engine.snapshot().session.unwrap().id, second_id);
    }

    #[test]
    fn crash_recovery_uses_complete_raw_frames_and_keeps_notes() {
        let (temporary, mut engine) = engine();
        let meeting = engine
            .create_meeting("회의".into(), "직접 작성한 메모".into())
            .unwrap();
        let mut manifest = Manifest::new(
            &meeting.id,
            "synthetic",
            "시험 입력",
            1000,
            1,
            Encoding::F32,
        );
        let mut raw = engine.store.prepare_capture(&manifest).unwrap();
        raw.write_all(&[0.125_f32.to_le_bytes(); 123].concat())
            .unwrap();
        raw.write_all(&[1, 2]).unwrap();
        raw.sync_all().unwrap();
        drop(raw);
        manifest.phase = CapturePhase::Recording;
        manifest.saved_frames = 100;
        store::write_manifest(&engine.store.root, &manifest).unwrap();
        let stored = engine.meetings.get_mut(&meeting.id).unwrap();
        stored.capture_id = Some(manifest.session_id.clone());
        stored.meeting.status = MeetingStatus::Recording;
        engine.store.save(stored).unwrap();
        let root = engine.store.root.clone();
        drop(engine);
        let restored = Engine::open(root.clone()).unwrap();
        let meeting = &restored.meetings[&meeting.id].meeting;
        assert_eq!(meeting.status, MeetingStatus::Interrupted);
        assert_eq!(meeting.notes, "직접 작성한 메모");
        assert_eq!(meeting.audio.as_ref().unwrap().frames, 123);
        assert_eq!(
            hound::WavReader::open(restored.audio_path(&meeting.id).unwrap())
                .unwrap()
                .duration(),
            123
        );
        assert_eq!(
            fs::metadata(root.join(manifest.relative_dir()).join("audio.pcm"))
                .unwrap()
                .len(),
            494
        );
        drop(restored);
        drop(temporary);
    }

    #[test]
    fn invalid_capture_metadata_is_isolated_without_changing_saved_edits_or_audio() {
        for corruption in ["json", "schema", "identity"] {
            let (_temporary, mut engine, id, _source) = crate::transcript::tests::setup();
            engine
                .update_meeting(&id, "원래 제목".into(), "사용자가 작성한 메모".into())
                .unwrap();
            let initial = engine
                .commit_transcript(&id, None, crate::transcript::tests::output())
                .unwrap();
            let corrected = engine
                .edit_transcript(&id, initial.version, vec!["사용자가 수정한 문장".into()])
                .unwrap();
            let healthy = engine
                .create_meeting("다른 회의".into(), "다른 메모".into())
                .unwrap();
            let root = engine.store.root.clone();
            let capture_id = engine.meetings[&id].capture_id.as_ref().unwrap();
            let session = root
                .join("meetings")
                .join(&id)
                .join("sessions")
                .join(capture_id);
            let manifest_path = session.join("manifest.json");
            let corrupt_bytes = if corruption == "json" {
                b"{truncated".to_vec()
            } else {
                let mut manifest: serde_json::Value =
                    serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
                if corruption == "schema" {
                    manifest["schemaVersion"] = 999.into();
                } else {
                    manifest["meetingId"] = Uuid::new_v4().to_string().into();
                }
                serde_json::to_vec(&manifest).unwrap()
            };
            fs::write(&manifest_path, &corrupt_bytes).unwrap();
            let transcript = root
                .join("meetings")
                .join(&id)
                .join("transcripts")
                .join(&corrected.version.id);
            let paths = [
                session.join("source.wav"),
                session.join("audio.pcm"),
                session.join("audio.wav"),
                transcript.join("raw.json"),
                transcript.join("initial.json"),
                transcript.join("current.json"),
            ];
            let before: Vec<_> = paths.iter().map(|path| fs::read(path).unwrap()).collect();
            drop(engine);
            let restored = Engine::open(root.clone()).unwrap();
            let affected = &restored.meetings[&id].meeting;
            assert_eq!(affected.status, MeetingStatus::Interrupted);
            assert_eq!(affected.title, "원래 제목");
            assert_eq!(affected.notes, "사용자가 작성한 메모");
            assert!(affected
                .error
                .as_ref()
                .unwrap()
                .contains("자동 복구하지 못했습니다"));
            assert!(restored.audio_path(&id).is_ok());
            assert_eq!(
                serde_json::to_value(restored.transcript(&id).unwrap().unwrap()).unwrap(),
                serde_json::to_value(&corrected).unwrap()
            );
            assert_eq!(restored.meetings[&healthy.id].meeting, healthy);
            for (path, original) in paths.iter().zip(&before) {
                assert_eq!(&fs::read(path).unwrap(), original);
            }
            assert_eq!(fs::read(&manifest_path).unwrap(), corrupt_bytes);
            let affected = affected.clone();
            drop(restored);
            let reopened = Engine::open(root).unwrap();
            assert_eq!(reopened.meetings[&id].meeting, affected);
            assert_eq!(fs::read(&manifest_path).unwrap(), corrupt_bytes);
        }
    }

    #[test]
    fn recovery_without_a_complete_frame_does_not_claim_audio_was_recovered() {
        let (_temporary, mut engine) = engine();
        let meeting = engine
            .create_meeting("회의".into(), "원래 메모".into())
            .unwrap();
        let mut manifest = Manifest::new(
            &meeting.id,
            "synthetic",
            "시험 입력",
            1000,
            1,
            Encoding::F32,
        );
        let mut raw = engine.store.prepare_capture(&manifest).unwrap();
        raw.write_all(&[1, 2]).unwrap();
        raw.sync_all().unwrap();
        drop(raw);
        manifest.phase = CapturePhase::Recording;
        store::write_manifest(&engine.store.root, &manifest).unwrap();
        let stored = engine.meetings.get_mut(&meeting.id).unwrap();
        stored.capture_id = Some(manifest.session_id.clone());
        stored.meeting.status = MeetingStatus::Recording;
        engine.store.save(stored).unwrap();
        let root = engine.store.root.clone();
        drop(engine);
        let restored = Engine::open(root.clone()).unwrap();
        let affected = restored.meetings[&meeting.id].meeting.clone();
        assert_eq!(affected.status, MeetingStatus::Interrupted);
        assert!(affected.audio.is_none());
        assert_eq!(affected.notes, "원래 메모");
        assert_eq!(
            affected.error.as_deref(),
            Some("저장된 오디오가 없어 녹음을 복구하지 못했습니다.")
        );
        let raw_path = root.join(manifest.relative_dir()).join("audio.pcm");
        assert_eq!(fs::read(&raw_path).unwrap(), [1, 2]);
        drop(restored);
        let reopened = Engine::open(root).unwrap();
        assert_eq!(reopened.meetings[&meeting.id].meeting, affected);
        assert_eq!(fs::read(raw_path).unwrap(), [1, 2]);
    }

    #[test]
    fn interrupted_finalization_is_retried_after_transient_failure() {
        let (_temporary, mut engine) = engine();
        let meeting = engine
            .create_meeting("회의".into(), "직접 쓴 메모".into())
            .unwrap();
        let (id, mut sink) = synthetic(&mut engine, &meeting.id);
        let wav = engine
            .store
            .root
            .join(engine.capture.as_ref().unwrap().manifest.relative_dir())
            .join("audio.wav");
        fs::create_dir(&wav).unwrap();
        sink.write(&[0.5_f32.to_le_bytes(); 20].concat());
        engine.stop(&id).unwrap();
        let failed = &engine.meetings[&meeting.id].meeting;
        assert_eq!(failed.status, MeetingStatus::Interrupted);
        assert!(failed.audio.is_none());
        fs::remove_dir(&wav).unwrap();
        let root = engine.store.root.clone();
        drop(sink);
        drop(engine);
        let restored = Engine::open(root.clone()).unwrap();
        let repaired = &restored.meetings[&meeting.id].meeting;
        assert_eq!(repaired.status, MeetingStatus::Interrupted);
        assert_eq!(repaired.notes, "직접 쓴 메모");
        assert_eq!(repaired.audio.as_ref().unwrap().frames, 20);
        assert!(restored.audio_path(&meeting.id).unwrap().exists());
        assert_eq!(
            fs::metadata(wav.with_file_name("audio.pcm")).unwrap().len(),
            80
        );
    }

    #[test]
    fn invalid_interrupted_playback_is_preserved_and_regenerated() {
        let (_temporary, mut engine) = engine();
        let meeting = engine.create_meeting("회의".into(), "".into()).unwrap();
        let (_id, mut sink) = synthetic(&mut engine, &meeting.id);
        sink.write(&[0.5_f32.to_le_bytes(); 20].concat());
        engine.shutdown();
        let wav = engine.audio_path(&meeting.id).unwrap();
        let raw = wav.with_file_name("audio.pcm");
        let original = fs::read(&raw).unwrap();
        fs::write(&wav, b"invalid playback").unwrap();
        let root = engine.store.root.clone();
        drop(sink);
        drop(engine);
        let restored = Engine::open(root).unwrap();
        assert_eq!(
            restored.meetings[&meeting.id]
                .meeting
                .audio
                .as_ref()
                .unwrap()
                .frames,
            20
        );
        assert!(restored.audio_path(&meeting.id).unwrap().exists());
        assert_eq!(fs::read(raw).unwrap(), original);
        let backup = fs::read_dir(wav.parent().unwrap())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("audio.invalid.")
            })
            .unwrap();
        assert_eq!(fs::read(backup.path()).unwrap(), b"invalid playback");
    }

    #[test]
    fn missing_raw_is_reported_without_claiming_recovery() {
        let (_temporary, mut engine) = engine();
        let meeting = engine.create_meeting("회의".into(), "".into()).unwrap();
        let (_id, mut sink) = synthetic(&mut engine, &meeting.id);
        sink.write(&[0.5_f32.to_le_bytes(); 8].concat());
        engine.shutdown();
        let wav = engine.audio_path(&meeting.id).unwrap();
        let raw = wav.with_file_name("audio.pcm");
        fs::rename(&raw, raw.with_file_name("held.pcm")).unwrap();
        fs::rename(&wav, wav.with_file_name("held.wav")).unwrap();
        let root = engine.store.root.clone();
        drop(sink);
        drop(engine);
        let restored = Engine::open(root).unwrap();
        let meeting = &restored.meetings[&meeting.id].meeting;
        assert_eq!(meeting.status, MeetingStatus::Interrupted);
        assert!(meeting.audio.is_none());
        assert!(meeting
            .error
            .as_ref()
            .unwrap()
            .contains("원본 녹음을 읽을 수 없어"));
    }

    #[test]
    fn shutdown_finalizes_and_does_not_overwrite_recording() {
        let (_temporary, mut engine) = engine();
        let meeting = engine.create_meeting("회의".into(), "메모".into()).unwrap();
        let (_id, mut sink) = synthetic(&mut engine, &meeting.id);
        sink.write(&[0.5_f32.to_le_bytes(); 32].concat());
        let root = engine.store.root.clone();
        engine.shutdown();
        assert!(engine.capture.is_none());
        assert!(engine.audio_path(&meeting.id).unwrap().exists());
        assert!(engine.draft(&meeting.id).is_err());
        drop(sink);
        drop(engine);
        let restored = Engine::open(root).unwrap();
        let meeting = &restored.meetings[&meeting.id].meeting;
        assert_eq!(meeting.status, MeetingStatus::Interrupted);
        assert_eq!(meeting.audio.as_ref().unwrap().frames, 32);
    }

    #[test]
    fn prepare_shutdown_fences_new_start_and_checks_owner_atomically() {
        let (_temporary, mut engine) = engine();
        let first = engine.create_meeting("회의".into(), "".into()).unwrap();
        let second = engine
            .create_meeting("다음 회의".into(), "".into())
            .unwrap();
        let (session, mut sink) = synthetic(&mut engine, &first.id);
        sink.write(&[0.5_f32.to_le_bytes(); 8].concat());
        assert!(matches!(
            engine.prepare_shutdown(None),
            Err(CoreError::StaleSession)
        ));
        assert!(!engine.quitting);
        assert!(matches!(
            engine.prepare_shutdown(Some(&Uuid::new_v4().to_string())),
            Err(CoreError::StaleSession)
        ));
        let snapshot = engine.prepare_shutdown(Some(&session)).unwrap();
        assert!(engine.quitting);
        assert_eq!(snapshot.session.unwrap().state, SessionState::Completed);
        assert!(matches!(
            engine.draft(&second.id),
            Err(CoreError::Unavailable)
        ));
        assert!(engine.prepare_shutdown(None).is_ok());
    }

    #[test]
    fn last_handle_drop_finalizes_active_capture_before_reopen() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap().join("dev");
        let handle = CoreHandle::open(&root).unwrap();
        let meeting = handle.create_meeting("회의", "메모").unwrap();
        let meeting_id = meeting.id.clone();
        let (_session, mut sink) = handle
            .call(move |engine| Ok(synthetic(engine, &meeting_id)))
            .unwrap();
        sink.write(&[0.75_f32.to_le_bytes(); 10].concat());
        drop(handle);
        let restored = CoreHandle::open(&root).unwrap();
        let meeting = restored.list_meetings().unwrap().remove(0);
        assert_eq!(meeting.status, MeetingStatus::Interrupted);
        assert_eq!(meeting.audio.as_ref().unwrap().frames, 10);
        assert!(restored.audio_path(&meeting.id).unwrap().exists());
    }

    #[test]
    fn traversal_and_duplicate_identity_are_rejected() {
        let (_temporary, mut engine) = engine();
        let meeting = engine.create_meeting("회의".into(), "".into()).unwrap();
        assert!(engine
            .update_meeting("../meeting", "".into(), "".into())
            .is_err());
        assert!(checked_path(&engine.store.root, std::path::Path::new("../outside")).is_err());
        let duplicate_id = Uuid::new_v4().to_string();
        let duplicate_dir = engine.store.root.join("meetings").join(&duplicate_id);
        fs::create_dir(&duplicate_dir).unwrap();
        fs::copy(
            engine
                .store
                .root
                .join("meetings")
                .join(meeting.id)
                .join("meeting.json"),
            duplicate_dir.join("meeting.json"),
        )
        .unwrap();
        assert!(engine.store.load().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_store_and_audio_paths_are_rejected() {
        use std::os::unix::fs::symlink;
        let (_temporary, mut engine) = engine();
        let meeting = engine.create_meeting("회의".into(), "".into()).unwrap();
        let (id, mut sink) = synthetic(&mut engine, &meeting.id);
        sink.write(&[0.5_f32.to_le_bytes(); 2].concat());
        engine.stop(&id).unwrap();
        let path = engine.audio_path(&meeting.id).unwrap();
        let elsewhere = engine.store.root.join("other.wav");
        fs::rename(&path, &elsewhere).unwrap();
        symlink(&elsewhere, &path).unwrap();
        assert!(engine.audio_path(&meeting.id).is_err());
        let link = engine.store.root.with_file_name("linked-dev");
        symlink(&engine.store.root, &link).unwrap();
        assert!(CoreHandle::open(link).is_err());
    }

    #[test]
    fn library_folders_and_placements_survive_reopening_without_moving_audio() {
        let (_temporary, mut engine) = engine();
        let folder = engine.create_folder("  주간 회의  ".into()).unwrap();
        assert_eq!(folder.name, "주간 회의");
        assert!(engine.create_folder("주간 회의".into()).is_err());
        let other = engine.create_folder("Planning".into()).unwrap();
        assert!(engine.create_folder("planning".into()).is_err());
        assert!(engine.rename_folder(&other.id, "주간 회의".into()).is_err());
        let meeting = engine.create_meeting("회의".into(), "메모".into()).unwrap();
        let (session, mut sink) = synthetic(&mut engine, &meeting.id);
        sink.write(&[0.5_f32.to_le_bytes(); 2].concat());
        engine.stop(&session).unwrap();
        let audio = engine.audio_path(&meeting.id).unwrap();
        engine
            .move_meetings(&[meeting.id.clone()], Some(folder.id.clone()))
            .unwrap();
        engine
            .update_meeting(&meeting.id, "바뀐 이름".into(), "수정".into())
            .unwrap();
        assert_eq!(engine.audio_path(&meeting.id).unwrap(), audio);
        let stored: StoredMeeting = store::read_json(
            &engine
                .store
                .root
                .join("meetings")
                .join(&meeting.id)
                .join("meeting.json"),
        )
        .unwrap();
        assert_eq!(stored.meeting.folder_id, None);
        let root = engine.store.root.clone();
        drop(engine);
        let reopened = Engine::open(root).unwrap();
        assert_eq!(
            reopened
                .meetings()
                .iter()
                .find(|value| value.id == meeting.id)
                .unwrap()
                .folder_id,
            Some(folder.id)
        );
        assert_eq!(reopened.folders().len(), 2);
        assert_eq!(reopened.audio_path(&meeting.id).unwrap(), audio);
    }

    #[test]
    fn batch_validation_prevents_partial_move_trash_or_purge() {
        let (_temporary, mut engine) = engine();
        let meeting = engine
            .create_meeting("보존할 회의".into(), "".into())
            .unwrap();
        let folder = engine.create_folder("폴더".into()).unwrap();
        let unknown = Uuid::new_v4().to_string();
        let ids = [meeting.id.clone(), unknown];
        assert!(engine.move_meetings(&ids, Some(folder.id)).is_err());
        assert!(engine.trash_meetings(&ids).is_err());
        assert_eq!(engine.meetings()[0].folder_id, None);
        assert_eq!(engine.meetings()[0].deleted_at, None);
        assert!(engine.purge_meetings(&[meeting.id.clone()]).is_err());
        assert!(engine
            .store
            .root
            .join("meetings")
            .join(&meeting.id)
            .exists());
    }

    #[test]
    fn deleting_folder_unfiles_active_and_trashed_meetings_without_deleting_them() {
        let (_temporary, mut engine) = engine();
        let folder = engine.create_folder("삭제할 폴더".into()).unwrap();
        let first = engine.create_meeting("회의 1".into(), "".into()).unwrap();
        let second = engine.create_meeting("회의 2".into(), "".into()).unwrap();
        let ids = [first.id.clone(), second.id.clone()];
        engine.move_meetings(&ids, Some(folder.id.clone())).unwrap();
        engine.trash_meetings(&[second.id.clone()]).unwrap();
        engine.delete_folder(&folder.id).unwrap();
        assert_eq!(engine.meetings().len(), 2);
        assert!(engine
            .meetings()
            .iter()
            .all(|meeting| meeting.folder_id.is_none()));
        engine.restore_meetings(&[second.id]).unwrap();
        assert!(engine
            .meetings()
            .iter()
            .all(|meeting| meeting.deleted_at.is_none()));
    }

    #[test]
    fn trash_is_persistent_and_rejects_edits_and_recording_until_restore() {
        let (_temporary, mut engine) = engine();
        let meeting = engine
            .create_meeting("회의".into(), "기존 메모".into())
            .unwrap();
        engine.trash_meetings(&[meeting.id.clone()]).unwrap();
        assert!(engine.require_active_meeting(&meeting.id).is_err());
        assert!(engine
            .update_meeting(&meeting.id, "변경".into(), "".into())
            .is_err());
        assert!(engine
            .set_meeting_date(&meeting.id, Some("2026-10-21".into()))
            .is_err());
        assert!(engine.draft(&meeting.id).is_err());
        let root = engine.store.root.clone();
        drop(engine);
        let mut reopened = Engine::open(root).unwrap();
        assert!(reopened.meetings()[0].deleted_at.is_some());
        reopened.restore_meetings(&[meeting.id.clone()]).unwrap();
        assert!(reopened.require_active_meeting(&meeting.id).is_ok());
        assert_eq!(reopened.meetings()[0].notes, "기존 메모");
    }

    #[test]
    fn recording_cannot_be_trashed_or_permanently_deleted() {
        let (_temporary, mut engine) = engine();
        let first = engine
            .create_meeting("다른 회의".into(), "".into())
            .unwrap();
        let meeting = engine.create_meeting("녹음 중".into(), "".into()).unwrap();
        let (id, _) = synthetic(&mut engine, &meeting.id);
        assert!(engine
            .trash_meetings(&[first.id.clone(), meeting.id.clone()])
            .is_err());
        assert!(engine
            .meetings()
            .iter()
            .all(|meeting| meeting.deleted_at.is_none()));
        assert!(engine.purge_meetings(&[meeting.id]).is_err());
        engine.stop(&id).unwrap();
    }

    #[test]
    fn permanent_deletion_removes_only_selected_meeting_and_keeps_tombstone() {
        let (_temporary, mut engine) = engine();
        let first = engine.create_meeting("삭제".into(), "".into()).unwrap();
        let second = engine.create_meeting("보존".into(), "메모".into()).unwrap();
        let models = engine.store.root.join("models");
        fs::create_dir(&models).unwrap();
        fs::write(models.join("model.bin"), b"preserved model").unwrap();
        engine.trash_meetings(&[first.id.clone()]).unwrap();
        engine.purge_meetings(&[first.id.clone()]).unwrap();
        assert!(!engine.store.root.join("meetings").join(&first.id).exists());
        assert!(engine.store.root.join("meetings").join(&second.id).exists());
        assert_eq!(
            fs::read(models.join("model.bin")).unwrap(),
            b"preserved model"
        );
        assert!(engine.library.pending_purges.contains(&first.id));
        let root = engine.store.root.clone();
        drop(engine);
        let reopened = Engine::open(root).unwrap();
        assert_eq!(reopened.meetings().len(), 1);
        assert_eq!(reopened.meetings()[0].id, second.id);
    }

    #[test]
    fn library_write_failure_does_not_change_batch_in_memory_or_on_disk() {
        let (_temporary, mut engine) = engine();
        let meeting = engine.create_meeting("회의".into(), "보존".into()).unwrap();
        fs::create_dir(engine.store.root.join("library.json")).unwrap();
        assert!(engine.trash_meetings(&[meeting.id.clone()]).is_err());
        assert_eq!(engine.meetings()[0].deleted_at, None);
        let stored: StoredMeeting = store::read_json(
            &engine
                .store
                .root
                .join("meetings")
                .join(meeting.id)
                .join("meeting.json"),
        )
        .unwrap();
        assert_eq!(stored.meeting.notes, "보존");
        assert_eq!(stored.meeting.deleted_at, None);
    }

    #[cfg(unix)]
    #[test]
    fn purge_refuses_linked_files_and_preserves_the_external_target() {
        use std::os::unix::fs::symlink;
        let (_temporary, mut engine) = engine();
        let meeting = engine.create_meeting("회의".into(), "".into()).unwrap();
        let outside = engine.store.root.join("external.txt");
        fs::write(&outside, b"do not touch").unwrap();
        symlink(
            &outside,
            engine
                .store
                .root
                .join("meetings")
                .join(&meeting.id)
                .join("linked.txt"),
        )
        .unwrap();
        engine.trash_meetings(&[meeting.id.clone()]).unwrap();
        assert!(engine.purge_meetings(&[meeting.id.clone()]).is_err());
        assert!(!engine.library.pending_purges.contains(&meeting.id));
        assert_eq!(fs::read(&outside).unwrap(), b"do not touch");
        assert!(engine.meetings()[0].deleted_at.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn purge_cleanup_is_explicit_and_reports_failure_without_reviving_a_meeting() {
        use std::os::unix::fs::symlink;
        let (_temporary, mut engine) = engine();
        let meeting = engine.create_meeting("회의".into(), "".into()).unwrap();
        engine.trash_meetings(&[meeting.id.clone()]).unwrap();
        engine.purge_meetings(&[meeting.id.clone()]).unwrap();
        let root = engine.store.root.clone();
        let path = root.join("meetings").join(&meeting.id);
        fs::create_dir(&path).unwrap();
        fs::write(path.join("unfinished.raw"), b"pending cleanup").unwrap();
        drop(engine);
        let mut reopened = Engine::open(root.clone()).unwrap();
        assert!(reopened.meetings().is_empty());
        assert!(path.exists());
        reopened.retry_purge_cleanup().unwrap();
        assert!(!path.exists());
        let outside = root.join("external");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep.raw"), b"preserved").unwrap();
        symlink(&outside, &path).unwrap();
        assert!(reopened.retry_purge_cleanup().is_err());
        assert_eq!(fs::read(outside.join("keep.raw")).unwrap(), b"preserved");
        assert!(reopened.meetings().is_empty());
    }
}
