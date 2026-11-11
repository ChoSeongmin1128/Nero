use super::*;

#[test]
#[ignore = "requires the packaged native MOSS worker and explicit shared model/synthetic audio paths"]
fn native_moss_runs_through_the_app_without_copying_models() {
    let shared = PathBuf::from(
        std::env::var_os("NERO_MOSS_TEST_MODEL_ROOT").expect("explicit shared model root"),
    );
    let audio = PathBuf::from(std::env::var_os("NERO_MOSS_TEST_AUDIO").expect("synthetic WAV"));
    let resources =
        PathBuf::from(std::env::var_os("NERO_MOSS_TEST_RESOURCES").expect("packaged resources"));
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    fs::create_dir(root.join("preferences")).unwrap();
    fs::write(
        root.join("preferences/model-source.json"),
        serde_json::json!({"schemaVersion":1,"root":shared}).to_string(),
    )
    .unwrap();
    let speakers = std::env::var("NERO_MOSS_TEST_SPEAKERS").is_ok_and(|v| v == "1");
    crate::speech_choice::save(&root, SpeechEngine::Moss, speakers, None).unwrap();
    let core = CoreHandle::open(&root).unwrap();
    let meeting = core
        .import_audio("Synthetic MOSS acceptance", &audio, &audio)
        .unwrap();
    let process = Processing::new(root.clone(), resources);
    assert!(process.snapshot().model_ready);
    let choices = process.snapshot().speech_choices;
    assert_eq!(
        choices.iter().map(|c| c.id).collect::<Vec<_>>(),
        [
            SpeechEngine::Moss,
            SpeechEngine::QwenKo,
            SpeechEngine::AppleSpeech
        ]
    );
    assert_eq!(
        choices.iter().find(|c| c.recommended).unwrap().id,
        SpeechEngine::Moss
    );
    let progress = process
        .transcribe(core.clone(), meeting.id.clone())
        .unwrap();
    let id = progress.id.unwrap();
    let began = std::time::Instant::now();
    let job = loop {
        process.drive(&core).unwrap();
        let progress = process.snapshot();
        let job = progress
            .jobs
            .iter()
            .find(|job| job.id == id)
            .unwrap()
            .clone();
        if !matches!(job.status.as_str(), "queued" | "running" | "cancelling") {
            break job;
        }
        assert!(
            began.elapsed() < Duration::from_secs(900),
            "native MOSS timed out"
        );
        thread::sleep(Duration::from_millis(100));
    };
    assert!(
        matches!(job.status.as_str(), "completed" | "needsReview"),
        "{}",
        job.message
    );
    let transcript = core.transcript(&meeting.id).unwrap().unwrap();
    let report = transcript
        .source
        .as_ref()
        .unwrap()
        .recognition_report
        .as_ref()
        .unwrap();

    assert!(!transcript.segments.is_empty());
    if speakers {
        assert!(transcript.speaker_analysis.is_some());
    }
    assert!(!root
        .join(ModelSpec::speech(SpeechEngine::Moss).relative_dir())
        .join("model-00000-of-00001.safetensors")
        .exists());
    if let Some(path) = std::env::var_os("NERO_MOSS_TEST_REPORT") {
        fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"jobStatus":job.status,"coverage":report,"utteranceCount":transcript.segments.len(),"modelWeightsCopied":false,"appPipeline":true,"speakerAnalysis":transcript.speaker_analysis.is_some()})).unwrap()).unwrap();
    }
    if let Some(path) = std::env::var_os("NERO_MOSS_TEST_REPORT") {
        let parent = PathBuf::from(path).parent().unwrap().to_owned();
        fs::copy(
            root.join("processing")
                .join(&id)
                .join("moss-checkpoint.json"),
            parent.join("native-checkpoint.json"),
        )
        .unwrap();
    }
    assert!(
        report.text_complete(),
        "incomplete native result: {report:?}"
    );
    process.shutdown().unwrap();
    drop(core);
}
