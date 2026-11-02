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
                !fs::read_dir(root.join("models")).unwrap().any(|entry| entry
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
