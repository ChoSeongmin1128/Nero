//! Explicit, isolated hardware acceptance. No default test opens the user's library.
use super::*;
use crate::speech_audio::{stage_import, AnalysisAudio};

fn configured(name: &str) -> PathBuf {
    let path =
        PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is required")));
    assert!(path.is_absolute(), "{name} must be absolute");
    path
}

#[test]
#[ignore = "requires explicit synthetic input, existing model/resources and a new isolated output directory"]
fn native_audio_import_whisper_execution_and_persisted_reader() {
    let source = configured("NERO_WHISPER_ACCEPTANCE_SOURCE")
        .canonicalize()
        .unwrap();
    let resources = configured("NERO_WHISPER_ACCEPTANCE_RESOURCES");
    let model = configured("NERO_WHISPER_ACCEPTANCE_MODEL_DIR");
    let output = configured("NERO_WHISPER_ACCEPTANCE_OUTPUT");
    let backend = std::env::var("NERO_WHISPER_ACCEPTANCE_BACKEND")
        .expect("NERO_WHISPER_ACCEPTANCE_BACKEND is required");
    assert!(matches!(backend.as_str(), "metal" | "vulkan" | "cpu"));
    // Deliberately require a new caller-selected directory: no overwrite, library
    // adoption, shared model duplication, or automatic cleanup of native evidence.
    std::fs::create_dir(&output).unwrap();
    let work = output.join("음성 가져오기");
    std::fs::create_dir(&work).unwrap();
    let library = output.join("isolated-library");
    let process = crate::processing::Processing::new(library.clone(), resources.clone());
    let cancel = AtomicBool::new(false);
    let original_sha = file_hash(&source).unwrap();
    let imported = stage_import(&source, &work, &cancel).unwrap();
    let prepared = AnalysisAudio::prepare(&process, &imported.path, &work, &cancel).unwrap();
    let core = nero_core::CoreHandle::open(library.clone()).unwrap();
    let meeting = core
        .import_audio(
            "Synthetic codec/GPU acceptance",
            &imported.path,
            prepared.path(),
        )
        .unwrap();
    let raw = recognize(&resources, prepared.path(), &work, &cancel, &model).unwrap();
    prepared.verify(&imported.path, &cancel).unwrap();
    let value: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(value["completed"], true);
    assert_eq!(
        value["inputAudioFrames"],
        prepared.evidence()["inputAudioFrames"]
    );
    let windows = value["windows"].as_array().unwrap();
    assert!(!windows.is_empty());
    for window in windows {
        assert_eq!(window["actualBackend"], backend);
        assert_eq!(window["nativeExitCode"], 0);
        if backend == "vulkan" {
            assert_eq!(value["schemaVersion"], 2);
            assert_eq!(
                window["execution"]["executableSha256"]
                    .as_str()
                    .unwrap()
                    .len(),
                64
            );
        }
    }
    let transcript = core
        .commit_whisper_transcript(&meeting.id, None, raw)
        .unwrap();
    assert!(!transcript.segments.is_empty());
    assert_eq!(
        core.transcript(&meeting.id).unwrap().unwrap().version,
        transcript.version
    );
    let reader = nero_core::reader::LibraryReader::open(&library).unwrap();
    let read = reader
        .query("transcript_get", json!({"meetingId":meeting.id}))
        .unwrap();
    assert_eq!(read["version"], json!(transcript.version));
    assert_eq!(file_hash(&source).unwrap(), original_sha);
    let model_pin: Value = serde_json::from_str(MANIFEST).unwrap();
    let model_path = model.join(model_pin["files"][0]["name"].as_str().unwrap());
    assert_eq!(
        file_hash(&model_path).unwrap(),
        model_pin["files"][0]["sha256"].as_str().unwrap()
    );
    save(
        &output.join("acceptance.json"),
        &json!({
            "sourceSha256":original_sha,"analysis":prepared.evidence(),
            "runtimeTarget":value["target"],"schemaVersion":value["schemaVersion"],
            "windows":windows,"wordCount":value["words"].as_array().unwrap().len(),
            "transcriptId":transcript.version.id,"storedReaderVerified":true,
            "originalUnchanged":true,"modelUnchanged":true,
            "userLibraryOpened":false,"newModelDownloaded":false,
            "screenControlled":false
        }),
    )
    .unwrap();
}
