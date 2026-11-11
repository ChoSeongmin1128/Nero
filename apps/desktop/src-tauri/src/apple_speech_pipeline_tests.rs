use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn apple_file_processing_preserves_text_and_skips_all_speaker_work_when_disabled() {
    for speakers in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let base = temporary.path().canonicalize().unwrap();
        let root = base.join("library");
        let resources = base.join("resources");
        fs::create_dir(&root).unwrap();
        fs::create_dir_all(resources.join("native/apple-speech")).unwrap();
        fs::write(
            resources.join("native/AudioPrepare"),
            b"unused for WAV input",
        )
        .unwrap();
        let availability = serde_json::json!({"supported":true,"ready":true,"locale":"ko-KR","assetStatus":"installed",
            "message":"Synthetic installed asset","variant":"speechTranscriber","speechAvailable":true,
            "speechLocaleSupported":true,"dictationLocaleSupported":true});
        let native = serde_json::json!({"schemaVersion":1,"variant":"speechTranscriber","locale":"ko-KR","systemVersion":"Synthetic macOS",
            "inputFrames":16000,"consumedInputFrames":16000,"sampleRate":16000.0,"finalized":true,
            "finalizationTimeSeconds":1.0,"segments":[{"start":0.1,"end":0.8,"text":"Synthetic speech"}]});
        let helper = resources.join("native/apple-speech/nero-apple-speech");
        fs::write(&helper, format!("#!/bin/sh\ncase \"$1\" in\nstatus) printf '%s' '{}' ;;\ntranscribe) printf '%s' '{}' > \"$4\" ;;\n*) exit 23 ;;\nesac\n", availability, native)).unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        crate::speech_choice::save(
            &root,
            SpeechEngine::AppleSpeech,
            speakers,
            Some(AppleVariant::SpeechTranscriber),
        )
        .unwrap();
        let process = Processing::new(root.clone(), resources);
        *process.platform_supported_override.lock().unwrap() = Some(true);
        let core = CoreHandle::open(&root).unwrap();
        let input = base.join("synthetic.wav");
        let mut wav = hound::WavWriter::create(
            &input,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..16000 {
            wav.write_sample(0_i16).unwrap();
        }
        wav.finalize().unwrap();
        let meeting = core
            .import_audio("Synthetic meeting", &input, &input)
            .unwrap();
        process.require_meeting_models().unwrap();
        let job = process
            .transcribe(core.clone(), meeting.id.clone())
            .unwrap();
        let running = process.running.lock().unwrap().take().unwrap();
        running.handle.join().unwrap();
        let source = core.transcript(&meeting.id).unwrap().unwrap();
        assert_eq!(source.model, "apple/SpeechTranscriber");
        assert_eq!(source.segments[0].text, "Synthetic speech");
        assert_eq!(source.segments[0].start, 0.1);
        assert_eq!(source.segments[0].end, 0.8);
        assert!(source.speaker_analysis.is_none());
        let saved = process
            .records
            .lock()
            .unwrap()
            .iter()
            .find(|record| Some(&record.id) == job.id.as_ref())
            .cloned()
            .unwrap();
        assert_eq!(saved.speaker_enabled, speakers);
        assert_eq!(
            saved.apple_speech_variant,
            Some(AppleVariant::SpeechTranscriber)
        );
        if speakers {
            assert_eq!(saved.kind, "diarize");
            assert_eq!(saved.status, JobStatus::Failed);
        } else {
            assert_eq!(saved.kind, "transcribe");
            assert_eq!(saved.status, JobStatus::Completed);
            assert!(
                !root.join("models").exists() || !fs::read_dir(root.join("models")).unwrap().any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("Nemotron3-"))
            );
            assert!(!root
                .join("processing")
                .join(saved.id)
                .join("speaker")
                .exists());
        }
        assert!(core.transcript_history(&meeting.id).is_ok());
        drop(core);
    }
}

#[test]
fn platform_override_is_instance_scoped_and_defaults_to_actual_gate() {
    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path().canonicalize().unwrap();
    let first = Processing::new(base.join("first"), base.join("resources"));
    let second = Processing::new(base.join("second"), base.join("resources"));
    assert_eq!(first.platform_supported(), platform_supported());
    assert_eq!(second.platform_supported(), platform_supported());

    *first.platform_supported_override.lock().unwrap() = Some(true);
    *second.platform_supported_override.lock().unwrap() = Some(false);
    assert!(first.platform_supported());
    assert!(!second.platform_supported());
    assert!(!second.supported_for(SpeechEngine::AppleSpeech));

    *first.platform_supported_override.lock().unwrap() = None;
    assert_eq!(first.platform_supported(), platform_supported());
    assert!(!second.platform_supported());
}

#[test]
fn every_apple_speech_start_check_uses_the_platform_and_audio_prepare_gate() {
    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path().canonicalize().unwrap();
    let root = base.join("library");
    let resources = base.join("resources");
    fs::create_dir(&root).unwrap();
    fs::create_dir_all(resources.join("native/apple-speech")).unwrap();
    let availability = serde_json::json!({"supported":true,"ready":true,"locale":"ko-KR","assetStatus":"installed",
        "message":"Synthetic installed asset","variant":"speechTranscriber","speechAvailable":true,
        "speechLocaleSupported":true,"dictationLocaleSupported":true});
    let helper = resources.join("native/apple-speech/nero-apple-speech");
    fs::write(
        &helper,
        format!("#!/bin/sh\ncase \"$1\" in\nstatus) printf '%s' '{availability}' ;;\n*) exit 23 ;;\nesac\n"),
    )
    .unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    let process = Processing::new(root, resources.clone());
    let variant = Some(AppleVariant::SpeechTranscriber);
    let apple = SpeechEngine::AppleSpeech;
    // The helper alone reports a supported, ready system; the app's own gates still apply.
    *process.platform_supported_override.lock().unwrap() = Some(true);
    assert_eq!(process.engine_status(apple, variant), (false, true));
    fs::write(resources.join("native/AudioPrepare"), b"unused").unwrap();
    assert_eq!(process.engine_status(apple, variant), (true, true));
    *process.platform_supported_override.lock().unwrap() = Some(false);
    assert_eq!(process.engine_status(apple, variant), (false, true));
    let error = process
        .prepare_meeting_models_with_id(false, None, Some(apple), Some(false), variant)
        .err();
    assert_eq!(
        error.as_deref(),
        Some("이 환경에서는 음성 인식과 회의 요약을 아직 사용할 수 없습니다.")
    );
    assert!(process.records.lock().unwrap().is_empty());
}

#[test]
fn a_failed_status_check_neither_delays_startup_nor_hides_the_choice() {
    let temporary = tempfile::tempdir().unwrap();
    let base = temporary.path().canonicalize().unwrap();
    let root = base.join("library");
    let resources = base.join("resources");
    fs::create_dir(&root).unwrap();
    fs::create_dir_all(resources.join("native/apple-speech")).unwrap();
    fs::write(resources.join("native/AudioPrepare"), b"unused").unwrap();
    let helper = resources.join("native/apple-speech/nero-apple-speech");
    fs::write(&helper, "#!/bin/sh\nsleep 3\nexit 9\n").unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    let started = std::time::Instant::now();
    let process = Processing::new(root, resources);
    assert!(started.elapsed() < Duration::from_secs(2));
    *process.platform_supported_override.lock().unwrap() = Some(true);
    if !crate::apple_speech::installed(&process.resources) {
        return; // The helper runs only on Apple Silicon macOS.
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while process.apple_speech.lock().unwrap().asset_status != "unknown" {
        assert!(std::time::Instant::now() < deadline);
        thread::sleep(Duration::from_millis(50));
    }
    let apple = |process: &Processing| {
        process
            .snapshot()
            .speech_choices
            .into_iter()
            .find(|choice| choice.id == SpeechEngine::AppleSpeech)
            .unwrap()
    };
    assert!(apple(&process).supported);
    assert!(!apple(&process).ready);
    // Choosing it checks again; nothing is saved until that check succeeds.
    let availability = serde_json::json!({"supported":true,"ready":true,"locale":"ko-KR","assetStatus":"installed",
        "message":"Synthetic installed asset","variant":"speechTranscriber","speechAvailable":true,
        "speechLocaleSupported":true,"dictationLocaleSupported":true});
    fs::write(
        &helper,
        format!("#!/bin/sh\ncase \"$1\" in\nstatus) printf '%s' '{availability}' ;;\n*) exit 23 ;;\nesac\n"),
    )
    .unwrap();
    process
        .request_speech_engine(SpeechEngine::AppleSpeech)
        .unwrap();
    assert_eq!(
        process.snapshot().speech_engine,
        Some(SpeechEngine::AppleSpeech)
    );
}
