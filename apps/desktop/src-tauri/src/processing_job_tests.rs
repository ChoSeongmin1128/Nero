use super::*;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

fn setup() -> (tempfile::TempDir, Arc<Processing>) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let process = Processing::new(root.clone(), root.join("unused-resources"));
    (temporary, process)
}

fn record(kind: &str) -> JobRecord {
    JobRecord {
        schema_version: 1,
        id: Uuid::new_v4().to_string(),
        kind: kind.into(),
        meeting_id: None,
        source_fingerprint: None,
        include_summary: false,
        acknowledged: false,
        failure_kind: None,
        status: JobStatus::Queued,
        phase: None,
        message: "대기 중".into(),
        created_at: 1,
        updated_at: 1,
    }
}

fn pending(
    record: JobRecord,
    action: impl FnOnce(Arc<Processing>, Arc<AtomicBool>, PathBuf) -> Result<Option<String>, String>
        + Send
        + 'static,
) -> Pending {
    Pending {
        record,
        action: Box::new(action),
        import_existing: None,
        validate: None,
    }
}

fn launch(process: &Arc<Processing>, job: Pending) -> String {
    let id = job.record.id.clone();
    process.save_job(job.record.clone()).unwrap();
    let running = process.launch_checked(job).unwrap();
    *process.running.lock().unwrap() = Some(running);
    id
}

fn finish(process: &Processing) {
    let job = process.running.lock().unwrap().take();
    if let Some(job) = job {
        job.handle.join().unwrap();
    }
}

fn status(process: &Processing, id: &str) -> JobStatus {
    process
        .records
        .lock()
        .unwrap()
        .iter()
        .find(|record| record.id == id)
        .unwrap()
        .status
}

fn wait_for_cancel_then_release(
    observed: Sender<()>,
    release: Receiver<()>,
    cancel: Arc<AtomicBool>,
) -> Result<Option<String>, String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !cancel.load(Ordering::Acquire) {
        if Instant::now() >= deadline {
            return Err("취소 요청이 도착하지 않았습니다.".into());
        }
        thread::sleep(Duration::from_millis(2));
    }
    observed.send(()).map_err(message)?;
    release
        .recv_timeout(Duration::from_secs(3))
        .map_err(message)?;
    Err("처리를 취소했습니다.".into())
}

fn release_while_capture_gate_is_held(
    process: Arc<Processing>,
    release: Sender<()>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match process.running.try_lock() {
                Err(std::sync::TryLockError::WouldBlock) => {
                    release.send(()).unwrap();
                    return;
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    panic!("작업 소유권 잠금이 손상됐습니다.")
                }
                Ok(_) => (),
            }
            assert!(
                Instant::now() < deadline,
                "녹음/종료 소유권 잠금을 획득하지 않았습니다."
            );
            thread::sleep(Duration::from_millis(2));
        }
    })
}

#[test]
fn starting_recording_keeps_a_download_running_and_does_not_cancel_it() {
    let (_temporary, process) = setup();
    let (started, started_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let id = launch(
        &process,
        pending(record("summary-model"), move |_, cancel, _| {
            started.send(()).map_err(message)?;
            release_rx
                .recv_timeout(Duration::from_secs(3))
                .map_err(message)?;
            assert!(!cancel.load(Ordering::Acquire));
            Ok(None)
        }),
    );
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(process.recording_start(|| Ok::<_, String>(17)).unwrap(), 17);
    assert_eq!(status(&process, &id), JobStatus::Running);
    assert_eq!(process.running.lock().unwrap().as_ref().unwrap().id, id);
    assert!(!process
        .running
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .cancel
        .load(Ordering::Acquire));
    release.send(()).unwrap();
    finish(&process);
    assert_eq!(status(&process, &id), JobStatus::Completed);
}

#[test]
fn a_failed_queued_cancel_keeps_the_pending_action_and_latest_bytes() {
    let (_temporary, process) = setup();
    let record = record("transcribe");
    let id = record.id.clone();
    process.save_job(record.clone()).unwrap();
    process
        .pending
        .lock()
        .unwrap()
        .push_back(pending(record, |_, _, _| Ok(None)));
    let corrupt = process
        .root
        .join("processing")
        .join(&id)
        .join("job-state/00000000000000000002.json");
    fs::write(&corrupt, b"{\"status\":").unwrap();
    let before = fs::read(&corrupt).unwrap();
    assert!(process.cancel(&id).is_err());
    assert_eq!(process.pending.lock().unwrap().len(), 1);
    assert_eq!(process.pending.lock().unwrap()[0].record.id, id);
    assert_eq!(status(&process, &id), JobStatus::Queued);
    assert_eq!(fs::read(corrupt).unwrap(), before);
}

#[test]
fn recording_preemption_never_promotes_a_user_cancel_to_interrupted() {
    let (_temporary, process) = setup();
    let (observed, observed_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let id = launch(
        &process,
        pending(record("summary"), move |_, cancel, _| {
            wait_for_cancel_then_release(observed, release_rx, cancel)
        }),
    );
    process.cancel(&id).unwrap();
    observed_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let releaser = release_while_capture_gate_is_held(process.clone(), release);
    process.recording_start(|| Ok::<_, String>(())).unwrap();
    releaser.join().unwrap();
    assert_eq!(status(&process, &id), JobStatus::Cancelled);
    assert_eq!(process.snapshot().status, "cancelled");
    assert!(!process.interrupted_jobs.lock().unwrap().contains(&id));
}

#[test]
fn shutdown_never_promotes_a_user_cancel_to_interrupted() {
    let (_temporary, process) = setup();
    let (observed, observed_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let id = launch(
        &process,
        pending(record("transcribe"), move |_, cancel, _| {
            wait_for_cancel_then_release(observed, release_rx, cancel)
        }),
    );
    process.cancel(&id).unwrap();
    observed_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let releaser = release_while_capture_gate_is_held(process.clone(), release);
    process.shutdown().unwrap();
    releaser.join().unwrap();
    assert_eq!(status(&process, &id), JobStatus::Cancelled);
    assert_eq!(process.snapshot().status, "cancelled");
    assert!(!process.interrupted_jobs.lock().unwrap().contains(&id));
}

#[test]
fn cancelling_one_queued_job_keeps_another_and_advances_global_revision() {
    let (_temporary, process) = setup();
    let one = record("summary");
    let two = record("transcribe");
    let one_id = one.id.clone();
    let two_id = two.id.clone();
    for record in [one, two] {
        process.save_job(record.clone()).unwrap();
        process
            .pending
            .lock()
            .unwrap()
            .push_back(pending(record, |_, _, _| Ok(None)));
    }
    let before = process.snapshot().revision;
    process.cancel(&one_id).unwrap();
    let after = process.snapshot();
    assert!(after.revision > before);
    assert_eq!(status(&process, &one_id), JobStatus::Cancelled);
    assert_eq!(status(&process, &two_id), JobStatus::Queued);
    assert_eq!(process.pending.lock().unwrap().len(), 1);
    assert_eq!(process.pending.lock().unwrap()[0].record.id, two_id);
}

#[test]
fn global_revision_survives_job_reset_and_terminal_transition() {
    let (_temporary, process) = setup();
    let (release, release_rx) = mpsc::channel();
    let before = process.snapshot().revision;
    let id = launch(
        &process,
        pending(record("model"), move |_, _, _| {
            release_rx
                .recv_timeout(Duration::from_secs(3))
                .map_err(message)?;
            Ok(None)
        }),
    );
    let running = process.snapshot();
    assert_eq!(running.id.as_deref(), Some(id.as_str()));
    assert!(running.revision > before);
    release.send(()).unwrap();
    finish(&process);
    let completed = process.snapshot();
    assert_eq!(completed.status, "completed");
    assert!(completed.revision > running.revision);
    assert_eq!(status(&process, &id), JobStatus::Completed);
}

#[test]
fn restart_marks_a_saving_job_for_review_without_scheduling_it() {
    let (temporary, process) = setup();
    let preserved = process.root.join("existing-audio.pcm");
    fs::write(&preserved, [1, 2, 3, 4]).unwrap();
    let mut record = record("summary");
    record.status = JobStatus::Running;
    record.phase = Some("saving".into());
    let id = record.id.clone();
    process.save_job(record).unwrap();
    drop(process);
    let reopened = Processing::new(
        temporary.path().into(),
        temporary.path().join("unused-resources"),
    );
    assert_eq!(status(&reopened, &id), JobStatus::NeedsReview);
    assert!(reopened.pending.lock().unwrap().is_empty());
    assert!(reopened.running.lock().unwrap().is_none());
    assert!(
        !reopened
            .snapshot()
            .jobs
            .iter()
            .find(|job| job.id == id)
            .unwrap()
            .retryable
    );
    assert_eq!(fs::read(preserved).unwrap(), [1, 2, 3, 4]);
}

#[test]
fn commit_persists_saving_before_the_effect_can_run() {
    let (_temporary, process) = setup();
    let mut record = record("summary");
    record.status = JobStatus::Running;
    let id = record.id.clone();
    process.save_job(record).unwrap();
    process.update(|progress| {
        progress.id = Some(id.clone());
        progress.kind = "summary".into();
        progress.status = "running".into();
    });
    let journal = Journal::new(process.root.clone());
    process
        .commit_summary(&AtomicBool::new(false), || {
            let record = journal
                .records()
                .unwrap()
                .into_iter()
                .find(|record| record.id == id)
                .unwrap();
            assert_eq!(record.status, JobStatus::Running);
            assert_eq!(record.phase.as_deref(), Some("saving"));
            Ok(())
        })
        .unwrap();
}

fn edited_source(process: &Processing) -> (CoreHandle, String, grove_core::Transcript) {
    let core = CoreHandle::open(&process.root).unwrap();
    let input = process.root.join("synthetic-input.wav");
    let data_size = 16000u32 * 2;
    let mut wav = Vec::from(b"RIFF".as_slice());
    wav.extend_from_slice(&(36 + data_size).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&16000u32.to_le_bytes());
    wav.extend_from_slice(&32000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.resize(wav.len() + data_size as usize, 0);
    fs::write(&input, wav).unwrap();
    let meeting = core
        .import_audio("Synthetic meeting", &input, &input)
        .unwrap();
    let raw = serde_json::to_vec(&serde_json::json!({
        "model":"OpenMOSS-Team/MOSS-Transcribe-Diarize",
        "segments":[{"start":0.0,"end":1.0,"text":"원문","speaker_id":"S1"}],
        "hitTokenLimit":false,"hasUnparsedText":false,
        "processedAudioFrames":16000,"inputAudioFrames":16000,"audioDurationSeconds":1.0
    }))
    .unwrap();
    let source = core.commit_transcript(&meeting.id, None, raw).unwrap();
    (core, meeting.id, source)
}

#[test]
fn rejected_transcript_output_preserves_its_cause_and_remains_retryable() {
    let (_temporary, process) = setup();
    let (core, meeting_id, source) = edited_source(&process);
    let edited = core
        .edit_transcript(
            &meeting_id,
            source.version,
            vec!["사용자가 수정한 문장".into()],
        )
        .unwrap();
    let transcripts = process
        .root
        .join("meetings")
        .join(&meeting_id)
        .join("transcripts");
    let original = transcripts.join(&edited.version.id);
    let preserved = [
        core.audio_path(&meeting_id).unwrap(),
        original.join("raw.json"),
        original.join("initial.json"),
        original.join("current.json"),
        process
            .root
            .join("meetings")
            .join(&meeting_id)
            .join("meeting.json"),
    ];
    let before: Vec<_> = preserved
        .iter()
        .map(|path| fs::read(path).unwrap())
        .collect();
    let fingerprint = Processing::source_fingerprint(&core, &meeting_id, "transcribe").unwrap();
    let rejected = serde_json::to_vec(&serde_json::json!({
        "model":"OpenMOSS-Team/MOSS-Transcribe-Diarize",
        "segments":[{"start":0.0,"end":1.0,"text":"미완성 출력","speaker_id":"S1"}],
        "hitTokenLimit":false,"hasUnparsedText":true,
        "processedAudioFrames":16000,"inputAudioFrames":16000,"audioDurationSeconds":1.0
    }))
    .unwrap();
    let mut record = record("transcribe");
    record.meeting_id = Some(meeting_id.clone());
    record.source_fingerprint = Some(fingerprint.clone());
    let owned_core = core.clone();
    let owned_id = meeting_id.clone();
    let expected = edited.version.clone();
    let output = rejected.clone();
    let id = launch(
        &process,
        pending(record, move |this, cancel, _| {
            this.checkpoint_transcript(&cancel, || {
                owned_core.commit_whole_transcript(&owned_id, Some(expected), output)
            })?;
            panic!("rejected output must not reach postprocessing")
        }),
    );
    finish(&process);
    let progress = process.snapshot();
    assert_eq!(progress.status, "failed");
    assert_eq!(progress.phase.as_deref(), Some("outputRejected"));
    assert!(progress
        .message
        .contains("일부 발언을 끝까지 읽지 못했습니다"));
    assert_eq!(
        progress.failure_kind,
        Some(crate::summary_failure::FailureKind::Output)
    );
    assert!(
        progress
            .jobs
            .iter()
            .find(|job| job.id == id)
            .unwrap()
            .retryable
    );
    assert_eq!(
        core.transcript(&meeting_id).unwrap().unwrap().version,
        edited.version
    );
    assert_eq!(
        Processing::source_fingerprint(&core, &meeting_id, "transcribe").unwrap(),
        fingerprint
    );
    for (path, bytes) in preserved.iter().zip(before) {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    let rejected_directory = fs::read_dir(transcripts)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path != &original)
        .unwrap();
    assert_eq!(
        fs::read(rejected_directory.join("raw.json")).unwrap(),
        rejected
    );
    assert!(!rejected_directory.join("current.json").exists());
    let reopened = Processing::new(process.root.clone(), process.resources.clone());
    let job = reopened
        .snapshot()
        .jobs
        .into_iter()
        .find(|job| job.id == id)
        .unwrap();
    assert_eq!(job.status, "failed");
    assert!(job.retryable);
    assert!(job.message.contains("일부 발언을 끝까지 읽지 못했습니다"));
}

#[test]
fn transcript_cas_rejection_reports_source_change_without_replacing_the_current_edit() {
    let (_temporary, process) = setup();
    let (core, meeting_id, source) = edited_source(&process);
    let fingerprint = Processing::source_fingerprint(&core, &meeting_id, "transcribe").unwrap();
    let edited = core
        .edit_transcript(
            &meeting_id,
            source.version.clone(),
            vec!["새로 저장한 사용자 수정".into()],
        )
        .unwrap();
    let directory = process.root.join("meetings").join(&meeting_id);
    let transcript = directory.join("transcripts").join(&edited.version.id);
    let raw = fs::read(transcript.join("raw.json")).unwrap();
    let preserved = [
        core.audio_path(&meeting_id).unwrap(),
        transcript.join("current.json"),
        directory.join("meeting.json"),
    ];
    let before: Vec<_> = preserved
        .iter()
        .map(|path| fs::read(path).unwrap())
        .collect();
    let mut record = record("transcribe");
    record.meeting_id = Some(meeting_id.clone());
    record.source_fingerprint = Some(fingerprint);
    let owned_core = core.clone();
    let owned_id = meeting_id.clone();
    let id = launch(
        &process,
        pending(record, move |this, cancel, _| {
            this.checkpoint_transcript(&cancel, || {
                owned_core.commit_transcript(&owned_id, Some(source.version), raw)
            })?;
            Ok(None)
        }),
    );
    finish(&process);
    let progress = process.snapshot();
    assert_eq!(progress.status, "failed");
    assert_eq!(progress.phase.as_deref(), Some("sourceChanged"));
    assert_eq!(
        progress.failure_kind,
        Some(crate::summary_failure::FailureKind::SourceChanged)
    );
    assert!(progress
        .message
        .contains("전사가 변경되어 새 결과를 적용하지 않았습니다"));
    assert!(process
        .retry(core.clone(), &id)
        .err()
        .unwrap()
        .contains("회의 내용이 변경"));
    assert_eq!(
        core.transcript(&meeting_id).unwrap().unwrap().version,
        edited.version
    );
    for (path, bytes) in preserved.iter().zip(before) {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    let reopened = Processing::new(process.root.clone(), process.resources.clone());
    let job = reopened
        .snapshot()
        .jobs
        .into_iter()
        .find(|job| job.id == id)
        .unwrap();
    assert_eq!(job.status, "failed");
    assert_eq!(
        job.failure_kind,
        Some(crate::summary_failure::FailureKind::SourceChanged)
    );
}

#[test]
fn missing_transcript_meeting_is_a_definite_unavailable_failure() {
    let (_temporary, process) = setup();
    let core = CoreHandle::open(&process.root).unwrap();
    let meeting_id = Uuid::new_v4().to_string();
    let mut record = record("transcribe");
    record.meeting_id = Some(meeting_id.clone());
    let id = launch(
        &process,
        pending(record, move |this, cancel, _| {
            this.checkpoint_transcript(&cancel, || {
                core.commit_whole_transcript(&meeting_id, None, Vec::new())
            })?;
            Ok(None)
        }),
    );
    finish(&process);
    let progress = process.snapshot();
    assert_eq!(progress.status, "failed");
    assert_eq!(progress.phase.as_deref(), Some("meetingUnavailable"));
    assert_eq!(
        progress.failure_kind,
        Some(crate::summary_failure::FailureKind::MeetingUnavailable)
    );
    assert_eq!(progress.message, "회의를 찾을 수 없습니다.");
    let reopened = Processing::new(process.root.clone(), process.resources.clone());
    assert_eq!(status(&reopened, &id), JobStatus::Failed);
}

#[test]
fn uncertain_transcript_storage_or_actor_shutdown_still_requires_result_confirmation() {
    for error in [
        grove_core::CoreError::Storage("저장 결과 불확실".into()),
        grove_core::CoreError::Unavailable,
    ] {
        let (_temporary, process) = setup();
        let id = launch(
            &process,
            pending(record("transcribe"), move |this, cancel, _| {
                this.checkpoint_transcript::<()>(&cancel, || Err(error))?;
                Ok(None)
            }),
        );
        finish(&process);
        let progress = process.snapshot();
        assert_eq!(progress.status, "needsReview");
        assert_eq!(progress.phase.as_deref(), Some("saving"));
        assert!(progress
            .message
            .contains("저장 완료 여부를 확인하지 못했습니다"));
        assert!(
            !progress
                .jobs
                .iter()
                .find(|job| job.id == id)
                .unwrap()
                .retryable
        );
        let reopened = Processing::new(process.root.clone(), process.resources.clone());
        assert_eq!(status(&reopened, &id), JobStatus::NeedsReview);
    }
}

#[test]
fn retry_rejects_a_changed_source_and_preserves_audio_and_current_edits() {
    let (_temporary, process) = setup();
    let (core, meeting_id, source) = edited_source(&process);
    let mut record = record("transcribe");
    record.meeting_id = Some(meeting_id.clone());
    record.source_fingerprint =
        Some(Processing::source_fingerprint(&core, &meeting_id, "transcribe").unwrap());
    record.status = JobStatus::Interrupted;
    let id = record.id.clone();
    process.save_job(record).unwrap();
    let edited = core
        .edit_transcript(
            &meeting_id,
            source.version,
            vec!["사용자가 수정한 문장".into()],
        )
        .unwrap();
    let audio_path = core.audio_path(&meeting_id).unwrap();
    let audio_before = fs::read(&audio_path).unwrap();
    let edited_before = serde_json::to_vec(&edited).unwrap();
    let result = process.retry(core.clone(), &id);
    assert!(result.is_err());
    assert_eq!(status(&process, &id), JobStatus::Interrupted);
    assert!(process.pending.lock().unwrap().is_empty());
    assert!(process.running.lock().unwrap().is_none());
    assert_eq!(fs::read(audio_path).unwrap(), audio_before);
    assert_eq!(
        serde_json::to_vec(&core.transcript(&meeting_id).unwrap().unwrap()).unwrap(),
        edited_before
    );
}

#[test]
fn retry_binding_rejects_changes_between_initial_check_and_new_input_capture() {
    let (_temporary, process) = setup();
    let (core, id, source) = edited_source(&process);
    let original = Processing::source_fingerprint(&core, &id, "transcribe").unwrap();
    assert_eq!(
        Processing::source_fingerprint(&core, &id, "transcribe").unwrap(),
        original
    );
    let edited = core
        .edit_transcript(
            &id,
            source.version,
            vec!["검사 직후 사용자가 저장한 문장".into()],
        )
        .unwrap();
    let result = process.start_meeting(
        "transcribe",
        id.clone(),
        &core,
        Some(original),
        |_, _, _| panic!("stale retry must not launch"),
    );
    assert!(result.err().unwrap().contains("회의 내용이 변경"));
    assert!(process.records.lock().unwrap().is_empty());
    assert!(process.pending.lock().unwrap().is_empty());
    assert_eq!(
        core.transcript(&id).unwrap().unwrap().version,
        edited.version
    );
}

#[test]
fn importing_commit_rejects_cancel_and_reports_the_created_meeting_as_completed() {
    let (_temporary, process) = setup();
    let mut record = record("import");
    record.status = JobStatus::Running;
    let id = record.id.clone();
    let created_id = Uuid::new_v4().to_string();
    let expected = created_id.clone();
    launch(
        &process,
        pending(record, move |this, cancel, _| {
            this.commit_transcript_result(&cancel, || {
                assert!(this.cancel(&this.snapshot().id.unwrap()).is_err());
                assert_eq!(this.snapshot().phase.as_deref(), Some("saving"));
                Ok(())
            })?;
            Ok(Some(created_id))
        }),
    );
    finish(&process);
    assert_eq!(status(&process, &id), JobStatus::Completed);
    assert_eq!(
        process.snapshot().meeting_id.as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn acknowledging_a_terminal_notice_preserves_outcome_and_history_across_restart() {
    let (_temporary, process) = setup();
    let mut saved_record = record("summary");
    saved_record.status = JobStatus::Failed;
    saved_record.failure_kind = Some(crate::summary_failure::FailureKind::Storage);
    saved_record.message = "저장 실패의 내부 진단".into();
    let id = saved_record.id.clone();
    process.save_job(saved_record.clone()).unwrap();
    process.update(|p| {
        p.id = Some(id.clone());
        p.kind = "summary".into();
        p.status = "failed".into();
        p.failure_kind = saved_record.failure_kind;
        p.message = saved_record.message.clone();
    });
    let before = fs::read(
        process
            .root
            .join("processing")
            .join(&id)
            .join("job-state/00000000000000000001.json"),
    )
    .unwrap();
    let view = process.acknowledge(&id).unwrap();
    assert_eq!(view.status, "acknowledged");
    assert!(
        view.jobs
            .iter()
            .find(|job| job.id == id)
            .unwrap()
            .acknowledged
    );
    assert_eq!(status(&process, &id), JobStatus::Failed);
    assert_eq!(
        fs::read(
            process
                .root
                .join("processing")
                .join(&id)
                .join("job-state/00000000000000000001.json")
        )
        .unwrap(),
        before
    );
    let reopened = Processing::new(process.root.clone(), process.resources.clone());
    let job = reopened
        .snapshot()
        .jobs
        .into_iter()
        .find(|job| job.id == id)
        .unwrap();
    assert!(job.acknowledged);
    assert_eq!(job.failure_kind, saved_record.failure_kind);
    assert_eq!(job.message, saved_record.message);
    let queued = record("transcribe");
    let queued_id = queued.id.clone();
    reopened.save_job(queued).unwrap();
    assert!(reopened.acknowledge(&queued_id).is_err());
}

#[test]
fn a_late_cancel_cannot_reopen_the_completed_state_while_the_thread_unwinds() {
    let (_temporary, process) = setup();
    let mut record = record("model");
    record.status = JobStatus::Completed;
    let id = record.id.clone();
    process.save_job(record).unwrap();
    let (release, wait) = mpsc::channel();
    *process.running.lock().unwrap() = Some(Running {
        id: id.clone(),
        cancel: Arc::new(AtomicBool::new(false)),
        handle: thread::spawn(move || {
            wait.recv().unwrap();
        }),
        import_existing: None,
    });
    process.update(|p| {
        p.id = Some(id.clone());
        p.kind = "model".into();
        p.status = "completed".into();
    });
    assert!(process.cancel(&id).is_err());
    assert_eq!(status(&process, &id), JobStatus::Completed);
    release.send(()).unwrap();
    finish(&process);
}

#[test]
fn speaker_continuation_is_durable_retryable_and_releases_only_the_finished_save_gate() {
    let (_temporary, process) = setup();
    let (core, meeting_id, source) = edited_source(&process);
    let mut job = record("transcribe");
    job.meeting_id = Some(meeting_id.clone());
    job.status = JobStatus::Running;
    let id = job.id.clone();
    process.save_job(job).unwrap();
    process.update(|p| {
        p.id = Some(id.clone());
        p.kind = "transcribe".into();
        p.status = "running".into();
        p.meeting_id = Some(meeting_id.clone());
        p.committing = true;
        p.phase = Some("saving".into());
    });
    process
        .continue_with_speakers(&core, &meeting_id, &AtomicBool::new(false))
        .unwrap();
    let p = process.snapshot();
    assert_eq!(p.kind, "diarize");
    assert!(!p.committing);
    let stored = process
        .journal
        .records()
        .unwrap()
        .into_iter()
        .find(|r| r.id == id)
        .unwrap();
    assert_eq!(stored.kind, "diarize");
    assert_eq!(
        stored.source_fingerprint,
        Some(Processing::source_fingerprint(&core, &meeting_id, "diarize").unwrap())
    );
    assert_eq!(
        core.transcript(&meeting_id).unwrap().unwrap().version,
        source.version
    );
    process.update(|p| p.status = "failed".into());
    assert!(
        process
            .snapshot()
            .jobs
            .iter()
            .find(|r| r.id == id)
            .unwrap()
            .retryable
    );
}

#[test]
fn cancelled_recognition_cannot_publish_a_speaker_continuation() {
    let (_temporary, process) = setup();
    let (core, meeting_id, _) = edited_source(&process);
    let mut job = record("transcribe");
    job.meeting_id = Some(meeting_id.clone());
    job.status = JobStatus::Running;
    let id = job.id.clone();
    process.save_job(job).unwrap();
    process.update(|p| {
        p.id = Some(id.clone());
        p.kind = "transcribe".into();
        p.status = "running".into();
        p.meeting_id = Some(meeting_id.clone());
    });
    assert!(process
        .continue_with_speakers(&core, &meeting_id, &AtomicBool::new(true))
        .is_err());
    assert_eq!(
        process
            .journal
            .records()
            .unwrap()
            .into_iter()
            .find(|r| r.id == id)
            .unwrap()
            .kind,
        "transcribe"
    );
}
