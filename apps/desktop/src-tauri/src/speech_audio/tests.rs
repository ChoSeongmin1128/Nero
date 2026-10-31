use super::*;
use hound::{SampleFormat, WavSpec, WavWriter};
use std::fs;

fn wav(path: &Path, rate: u32, float: bool, count: u32) {
    let mut writer = WavWriter::create(
        path,
        WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: if float { 32 } else { 16 },
            sample_format: if float {
                SampleFormat::Float
            } else {
                SampleFormat::Int
            },
        },
    )
    .unwrap();
    for _ in 0..count {
        if float {
            writer.write_sample(0.0_f32).unwrap()
        } else {
            writer.write_sample(0_i16).unwrap()
        }
    }
    writer.finalize().unwrap();
}

#[cfg(target_os = "macos")]
fn converter(root: &Path) -> std::sync::Arc<Processing> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir(root.join("native")).unwrap();
    fs::write(
        root.join("native/AudioPrepare"),
        b"#!/bin/sh\ncp \"$1.converted\" \"$2\"\n",
    )
    .unwrap();
    fs::set_permissions(
        root.join("native/AudioPrepare"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    Processing::new(root.join("library"), root.to_path_buf())
}

#[test]
fn normalized_input_does_not_require_a_platform_converter() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let process = Processing::new(root.join("library"), root.join("absent-resources"));
    let source = root.join("normalized.wav");
    wav(&source, 16000, false, 160);
    let audio = AnalysisAudio::prepare(&process, &source, &root, &AtomicBool::new(false)).unwrap();
    assert_eq!(audio.path(), source);
    assert_eq!(audio.frames, 160);
    assert!(!root.join("prepare.log").exists());
}

#[test]
fn float16k_import_is_converted_to_the_storage_and_model_pcm16_contract() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let process = Processing::new(root.join("library"), root.join("absent-resources"));
    let source = root.join("float.wav");
    wav(&source, 16000, true, 160);
    let work = root.join("job");
    fs::create_dir(&work).unwrap();
    let audio = AnalysisAudio::prepare(&process, &source, &work, &AtomicBool::new(false)).unwrap();
    assert_ne!(audio.path(), source);
    let core = nero_core::CoreHandle::open(root.join("store")).unwrap();
    core.import_audio("Synthetic float", &source, audio.path())
        .unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn missing_converter_and_log_creation_failures_keep_their_categories() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = root.join("recorded.m4a");
    wav(&source, 48000, true, 480);
    let process = Processing::new(root.join("library"), root.join("absent-resources"));
    let cancel = AtomicBool::new(false);
    assert_eq!(
        AnalysisAudio::prepare(&process, &source, &root, &cancel)
            .err()
            .unwrap()
            .kind,
        FailureKind::EngineUnavailable
    );
    fs::rename(root.join("prepare.log"), root.join("first-prepare.log")).unwrap();
    fs::create_dir(root.join("prepare.log")).unwrap();
    assert_eq!(
        AnalysisAudio::prepare(&process, &source, &root, &cancel)
            .err()
            .unwrap()
            .kind,
        FailureKind::Storage
    );
}

#[test]
fn recorded_float_audio_is_prepared_once_and_bound_to_its_source() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let process = Processing::new(root.join("library"), root.join("absent-resources"));
    let source = root.join("recorded.wav");
    wav(&source, 48000, true, 480);
    let original = fs::read(&source).unwrap();
    let work = root.join("job");
    fs::create_dir(&work).unwrap();
    let cancel = AtomicBool::new(false);
    assert_eq!(
        audio_frames(&source, &cancel).unwrap_err().kind,
        FailureKind::Input
    );
    let audio = AnalysisAudio::prepare(&process, &source, &work, &cancel).unwrap();
    assert_eq!(audio.frames, 160);
    assert_eq!(audio.path(), work.join("analysis.wav"));
    assert_eq!(fs::read(&source).unwrap(), original);
    audio.verify(&source, &cancel).unwrap();
    assert_eq!(
        audio
            .verify(&root.join("other.wav"), &cancel)
            .unwrap_err()
            .kind,
        FailureKind::SourceChanged
    );
    fs::write(audio.path(), b"changed").unwrap();
    assert_eq!(
        audio.verify(&source, &cancel).unwrap_err().kind,
        FailureKind::SourceChanged
    );
    assert_eq!(
        audio
            .verify(&source, &AtomicBool::new(true))
            .unwrap_err()
            .kind,
        FailureKind::Cancelled
    );
}

#[cfg(target_os = "macos")]
#[test]
fn source_changes_and_non_normalized_converter_output_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let process = converter(&root);
    let source = root.join("recorded.m4a");
    wav(&source, 48000, true, 480);
    wav(&root.join("recorded.m4a.converted"), 48000, true, 480);
    let work = root.join("first");
    fs::create_dir(&work).unwrap();
    let cancel = AtomicBool::new(false);
    assert_eq!(
        AnalysisAudio::prepare(&process, &source, &work, &cancel)
            .err()
            .unwrap()
            .kind,
        FailureKind::Input
    );
    wav(&root.join("recorded.m4a.converted"), 16000, false, 160);
    let work = root.join("second");
    fs::create_dir(&work).unwrap();
    let audio = AnalysisAudio::prepare(&process, &source, &work, &cancel).unwrap();
    wav(&source, 48000, true, 960);
    assert_eq!(
        audio.verify(&source, &cancel).unwrap_err().kind,
        FailureKind::SourceChanged
    );
}
