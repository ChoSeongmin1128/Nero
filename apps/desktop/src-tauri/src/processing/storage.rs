use super::{JobStatus, Processing};
use nero_core::CoreHandle;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;
use uuid::Uuid;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsage {
    pub meetings_bytes: u64,
    pub processing_bytes: u64,
    pub models_bytes: u64,
    pub reducible_bytes: u64,
    pub reducible_meetings: usize,
    // Recordings a job still reads or may retry from; they are left as they are.
    pub held_meetings: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Compacted {
    pub freed_bytes: u64,
    pub meetings: usize,
    pub failed: usize,
    // A recording started, so the pass stopped; the rest keep their original files.
    pub stopped: bool,
    pub usage: StorageUsage,
}

// Releases a meeting's claim even if its conversion panics, so its jobs can start again.
struct Claim<'a> {
    compacting: &'a std::sync::Mutex<std::collections::HashSet<String>>,
    id: &'a str,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        self.compacting.lock().unwrap().remove(self.id);
    }
}

// Bytes of the regular files under `path`; links and unreadable entries count as none.
fn tree_bytes(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => tree_bytes(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |meta| meta.len()),
            _ => 0,
        })
        .sum()
}

fn remove_file(path: &Path) -> u64 {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && fs::remove_file(path).is_ok() => meta.len(),
        _ => 0,
    }
}

impl Processing {
    // Compacting replaces the playback file that a job reads and that a retry of it
    // is bound to, so a recording with unsettled work is not touched.
    fn holds_recording(&self, meeting_id: &str) -> bool {
        let open = |record: &super::JobRecord| {
            record.meeting_id.as_deref() == Some(meeting_id)
                && record.status != JobStatus::Completed
                && !record.acknowledged
        };
        self.records.lock().unwrap().iter().any(open)
            || self.unsaved.lock().unwrap().values().any(open)
    }

    pub fn storage_usage(&self, core: &CoreHandle) -> nero_core::Result<StorageUsage> {
        let mut usage = StorageUsage {
            meetings_bytes: tree_bytes(&self.root.join("meetings")),
            processing_bytes: tree_bytes(&self.root.join("processing")),
            models_bytes: tree_bytes(&self.root.join("models")),
            ..Default::default()
        };
        for (id, bytes) in core.reducible_recordings()? {
            if self.holds_recording(&id) {
                usage.held_meetings += 1;
            } else {
                usage.reducible_bytes += bytes;
                usage.reducible_meetings += 1;
            }
        }
        Ok(usage)
    }

    pub fn compact_recordings(&self, core: &CoreHandle) -> nero_core::Result<Compacted> {
        if core.snapshot()?.active_session().is_some() {
            return Err(nero_core::CoreError::InvalidState(
                "녹음을 마친 뒤 녹음 파일을 줄여 주세요.".into(),
            ));
        }
        let mut result = Compacted::default();
        self.compaction_cancel.store(false, Ordering::Release);
        for (id, _) in core.reducible_recordings()? {
            if self.compaction_cancel.load(Ordering::Acquire) {
                result.stopped = true;
                break;
            }
            // Claimed under the gate new jobs take, so none starts reading the file while
            // it is replaced; the conversion itself runs after the gate is released.
            let claimed =
                self.meeting_idle(&id, || {
                    Ok(!self.holds_recording(&id)
                        && self.compacting.lock().unwrap().insert(id.clone()))
                });
            let compacted = match claimed {
                Ok(true) => {
                    let _claim = Claim {
                        compacting: &self.compacting,
                        id: &id,
                    };
                    core.compact_recording(&id, &self.compaction_cancel)
                        .map(Some)
                }
                Ok(false) => Ok(None),
                Err(error) => Err(error),
            };
            match compacted {
                Ok(None) => {}
                Ok(Some(freed)) => {
                    result.freed_bytes += freed;
                    result.meetings += 1;
                }
                Err(_) if self.compaction_cancel.load(Ordering::Acquire) => {
                    result.stopped = true;
                    break;
                }
                Err(_) => result.failed += 1,
            }
        }
        result.usage = self.storage_usage(core)?;
        Ok(result)
    }

    /// Removes analysis audio that a job left behind. It is a copy converted from the
    /// meeting's playback file, and a job that needs one converts it again. Run before
    /// any job starts. A live recording's copy stays until its transcript is saved,
    /// because the first recognition after the recording reuses it.
    pub fn remove_leftover_analysis_audio(&self, core: &CoreHandle) -> u64 {
        let Ok(entries) = fs::read_dir(self.root.join("processing")) else {
            return 0;
        };
        let transcribed: BTreeSet<String> = core
            .list_meetings()
            .unwrap_or_default()
            .into_iter()
            .filter(|meeting| meeting.transcript_id.is_some())
            .map(|meeting| meeting.id)
            .collect();
        let mut freed = 0;
        // This runs beside jobs a screen or agent may start meanwhile: only folders
        // present when it began are looked at, and never one of a job still active.
        let entries: Vec<_> = entries.filter_map(Result::ok).collect();
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if Uuid::parse_str(&name).is_err()
                || !entry.file_type().is_ok_and(|kind| kind.is_dir())
            {
                continue;
            }
            let active = self.records.lock().unwrap().iter().any(|record| {
                record.id == name
                    && matches!(
                        record.status,
                        JobStatus::Queued | JobStatus::Running | JobStatus::Cancelling
                    )
            });
            if active {
                continue;
            }
            let job = entry.path();
            freed += remove_file(&job.join("speaker-input").join("analysis.wav"));
            let live = job.join("live-request.json");
            let reusable = fs::symlink_metadata(&live).is_ok()
                && !fs::read(&live)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                    .and_then(|request| request["meetingId"].as_str().map(str::to_owned))
                    .is_some_and(|meeting| transcribed.contains(&meeting));
            if !reusable {
                freed += remove_file(&job.join("analysis.wav"));
            }
        }
        freed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leftover_analysis_audio_is_removed_but_evidence_and_unconsumed_live_audio_stay() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let core = CoreHandle::open(root.join("library")).unwrap();
        let process = Processing::new(root.join("library"), root.join("resources"));
        let waiting = core.create_meeting("녹음만 있는 회의", "").unwrap();
        let processing = root.join("library").join("processing");
        let job = |files: &[(&str, &[u8])]| {
            let path = processing.join(Uuid::new_v4().to_string());
            for (name, bytes) in files {
                let file = path.join(name);
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(file, bytes).unwrap();
            }
            path
        };
        let finished = job(&[
            ("analysis.wav", b"12345"),
            ("speaker-input/analysis.wav", b"123"),
            ("recognition.json", b"{}"),
            ("job-state/1.json", b"{}"),
        ]);
        let request = serde_json::to_vec(&serde_json::json!({"meetingId": waiting.id})).unwrap();
        let live = job(&[("analysis.wav", b"1234567"), ("live-request.json", &request)]);
        let unknown_live = job(&[("analysis.wav", b"1"), ("live-request.json", b"not json")]);
        fs::create_dir_all(processing.join("not-a-job")).unwrap();
        fs::write(processing.join("not-a-job").join("analysis.wav"), b"1").unwrap();
        // A job a screen started while this ran keeps the audio it is reading.
        let running = job(&[("analysis.wav", b"12")]);
        process.records.lock().unwrap().push(super::super::JobRecord {
            glossary_snapshot_required: false,
            speech_engine: Default::default(),
            speaker_enabled: true,
            apple_speech_variant: None,
            schema_version: 1,
            id: running.file_name().unwrap().to_string_lossy().into_owned(),
            kind: "transcribe".into(),
            meeting_id: Some(waiting.id.clone()),
            source_fingerprint: None,
            include_summary: false,
            summary_template_ref: None,
            acknowledged: false,
            retry_successor_id: None,
            failure_kind: None,
            status: JobStatus::Running,
            phase: None,
            message: "처리 중".into(),
            created_at: 1,
            updated_at: 1,
        });

        assert_eq!(process.remove_leftover_analysis_audio(&core), 8);
        assert!(!finished.join("analysis.wav").exists());
        assert!(!finished.join("speaker-input").join("analysis.wav").exists());
        assert!(finished.join("recognition.json").exists());
        assert!(finished.join("job-state").join("1.json").exists());
        assert!(live.join("analysis.wav").exists());
        assert!(unknown_live.join("analysis.wav").exists());
        assert!(processing.join("not-a-job").join("analysis.wav").exists());
        assert!(running.join("analysis.wav").exists());
        assert_eq!(process.remove_leftover_analysis_audio(&core), 0);

        let usage = process.storage_usage(&core).unwrap();
        assert_eq!(usage.reducible_meetings, 0);
        assert!(usage.processing_bytes >= 8);
    }
}
