use super::*;
use std::time::Duration;

#[test]
#[ignore = "explicit local microphone acceptance; records only into an auto-cleaned temporary library"]
fn microphone_pause_resume_and_restart_preserve_recorded_audio() {
    let report = std::env::var_os("NERO_CAPTURE_ACCEPTANCE_REPORT").expect("explicit report path");
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let core = CoreHandle::open(&root).unwrap();
    let devices = core.devices().unwrap();
    let device = devices
        .iter()
        .find(|d| d.is_default)
        .expect("default microphone");
    let started = core
        .create_recording("Microphone acceptance", "", &device.id)
        .unwrap();
    let session = started.session.unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        core.pause(&session.id).unwrap().session.unwrap().state,
        SessionState::Paused
    );
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        core.resume(&session.id).unwrap().session.unwrap().state,
        SessionState::Recording
    );
    std::thread::sleep(Duration::from_secs(2));
    let stopped = core.stop(&session.id).unwrap();
    let meeting = stopped
        .meetings
        .iter()
        .find(|m| m.id == session.meeting_id)
        .unwrap();
    assert_eq!(meeting.status, MeetingStatus::Completed);
    let audio = meeting.audio.clone().unwrap();
    assert!(audio.frames >= u64::from(audio.sample_rate));
    assert_eq!(audio.session_id, session.id);
    let wav = root.join(&audio.relative_path);
    assert!(wav.is_file());
    let reader = hound::WavReader::open(&wav).unwrap();
    assert!(reader.duration() > 0);
    drop(core);
    let reopened = CoreHandle::open(&root).unwrap();
    let after = reopened.snapshot().unwrap();
    assert_eq!(
        after
            .meetings
            .iter()
            .find(|m| m.id == session.meeting_id)
            .unwrap()
            .audio
            .as_ref(),
        Some(&audio)
    );
    std::fs::write(
        report,
        serde_json::to_vec_pretty(&serde_json::json!({
            "capturedFrames":audio.frames,"sampleRate":audio.sample_rate,"channels":audio.channels,
            "pauseResume":true,"restartPreservedAudio":true,"temporaryLibrary":true,
            "userLibraryWritten":false,"audioRetained":false
        }))
        .unwrap(),
    )
    .unwrap();
}
