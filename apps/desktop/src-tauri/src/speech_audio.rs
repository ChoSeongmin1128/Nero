use crate::processing::{cancelled, plain_file, Processing};
use crate::summary_failure::{Failure, FailureKind};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

#[cfg(any(windows, test))]
#[path = "speech_audio_wav.rs"]
mod portable_wav;

pub const SAMPLE_RATE: u32 = 16000;

pub(super) fn supported(process: &Processing) -> bool {
    cfg!(all(target_os = "windows", target_arch = "x86_64"))
        || process.worker("AudioPrepare").is_file()
}

fn failure(kind: FailureKind) -> Failure {
    Failure::new(
        kind,
        match kind {
            FailureKind::Cancelled => "음성 처리를 취소했습니다.",
            FailureKind::SourceChanged => {
                "처리 중 녹음 파일이 변경됐습니다. 현재 기록은 유지했습니다."
            }
            FailureKind::Storage => "처리할 음성 파일을 저장하지 못했습니다.",
            FailureKind::Cleanup => {
                "음성 처리 실행기의 종료를 확인하지 못했습니다. 앱을 다시 열어 주세요."
            }
            FailureKind::EngineUnavailable => "음성 처리 중 오류가 발생했습니다.",
            _ => "처리할 녹음 파일을 준비하지 못했습니다.",
        },
    )
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), Failure> {
    cancelled(cancel).map_err(|_| failure(FailureKind::Cancelled))
}

pub(super) struct AnalysisAudio {
    source: PathBuf,
    source_sha256: String,
    path: PathBuf,
    frames: u64,
    sha256: String,
}

impl AnalysisAudio {
    pub(super) fn prepare(
        process: &Processing,
        source: &Path,
        work: &Path,
        cancel: &AtomicBool,
    ) -> Result<Self, Failure> {
        check_cancel(cancel)?;
        let source_sha256 = digest(source)?;
        let (path, frames) = match native_frames(source, cancel) {
            Ok(frames) => (source.to_path_buf(), frames),
            Err(error) if error.kind == FailureKind::Cancelled => return Err(error),
            Err(_) => {
                let path = work.join("analysis.wav");
                #[cfg(windows)]
                portable_wav::prepare(source, &path, cancel)?;
                #[cfg(not(windows))]
                {
                    let arguments = [
                        source.as_os_str().to_os_string(),
                        path.as_os_str().to_os_string(),
                    ];
                    crate::diarization::run(
                        process,
                        &process.worker("AudioPrepare"),
                        &arguments,
                        &work.join("prepare.log"),
                        cancel,
                        false,
                        Some(source),
                        "prepare-audio",
                    )
                    .map_err(|error| failure(error.kind))?;
                }
                let frames = native_frames(&path, cancel)?;
                (path, frames)
            }
        };
        let sha256 = digest(&path)?;
        let prepared = Self {
            source: source.to_path_buf(),
            source_sha256,
            path,
            frames,
            sha256,
        };
        prepared.verify(source, cancel)?;
        Ok(prepared)
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn verify(&self, source: &Path, cancel: &AtomicBool) -> Result<(), Failure> {
        check_cancel(cancel)?;
        if self.source != source
            || digest(source)? != self.source_sha256
            || digest(&self.path)? != self.sha256
        {
            return Err(failure(FailureKind::SourceChanged));
        }
        check_cancel(cancel)
    }

    pub(super) fn evidence(&self) -> Value {
        json!({"format":"mono-16khz-wav-v1", "sourceAudioSha256":self.source_sha256,
            "analysisAudioSha256":self.sha256, "inputAudioFrames":self.frames,
            "audioSampleRate":SAMPLE_RATE})
    }
}

fn native_frames(audio: &Path, cancel: &AtomicBool) -> Result<u64, Failure> {
    let mut file = plain_file(audio).map_err(|_| failure(FailureKind::Input))?;
    let mut header = [0u8; 12];
    file.read_exact(&mut header)
        .map_err(|_| failure(FailureKind::Input))?;
    if &header[..4] != b"RIFF" || &header[8..] != b"WAVE" {
        return Err(failure(FailureKind::Input));
    }
    loop {
        check_cancel(cancel)?;
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk)
            .map_err(|_| failure(FailureKind::Input))?;
        let length = u32::from_le_bytes(chunk[4..].try_into().unwrap());
        if &chunk[..4] == b"fmt " {
            if length < 16 {
                return Err(failure(FailureKind::Input));
            }
            let mut format = [0u8; 2];
            file.read_exact(&mut format)
                .map_err(|_| failure(FailureKind::Input))?;
            if u16::from_le_bytes(format) != 1 {
                return Err(failure(FailureKind::Input));
            }
            break;
        }
        file.seek(SeekFrom::Current(i64::from(length) + i64::from(length % 2)))
            .map_err(|_| failure(FailureKind::Input))?;
    }
    let reader = hound::WavReader::new(plain_file(audio).map_err(|_| failure(FailureKind::Input))?)
        .map_err(|_| failure(FailureKind::Input))?;
    if reader.spec().sample_format != hound::SampleFormat::Int
        || reader.spec().bits_per_sample != 16
    {
        return Err(failure(FailureKind::Input));
    }
    audio_frames(audio, cancel)
}

pub(super) fn digest(path: &Path) -> Result<String, Failure> {
    let mut input = plain_file(path).map_err(|_| failure(FailureKind::Input))?;
    let mut hash = Sha256::new();
    let mut block = [0u8; 65536];
    loop {
        let count = input
            .read(&mut block)
            .map_err(|_| failure(FailureKind::Input))?;
        if count == 0 {
            break;
        }
        hash.update(&block[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(super) fn audio_frames(audio: &Path, cancel: &AtomicBool) -> Result<u64, Failure> {
    let input = plain_file(audio).map_err(|_| failure(FailureKind::Input))?;
    let mut reader =
        hound::WavReader::new(BufReader::new(input)).map_err(|_| failure(FailureKind::Input))?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.sample_rate != SAMPLE_RATE {
        return Err(failure(FailureKind::Input));
    }
    let expected = u64::from(reader.duration());
    if expected == 0 {
        return Err(failure(FailureKind::Input));
    }
    let mut frames = 0u64;
    match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => {
            for sample in reader.samples::<f32>() {
                if !sample.map_err(|_| failure(FailureKind::Input))?.is_finite() {
                    return Err(failure(FailureKind::Input));
                }
                frames += 1;
                if frames % u64::from(SAMPLE_RATE) == 0 {
                    check_cancel(cancel)?;
                }
            }
        }
        (hound::SampleFormat::Int, 16) => {
            for sample in reader.samples::<i16>() {
                sample.map_err(|_| failure(FailureKind::Input))?;
                frames += 1;
                if frames % u64::from(SAMPLE_RATE) == 0 {
                    check_cancel(cancel)?;
                }
            }
        }
        _ => return Err(failure(FailureKind::Input)),
    }
    if frames != expected {
        return Err(failure(FailureKind::Input));
    }
    check_cancel(cancel)?;
    Ok(frames)
}

#[cfg(test)]
mod tests {
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

    #[cfg(unix)]
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
        let audio =
            AnalysisAudio::prepare(&process, &source, &root, &AtomicBool::new(false)).unwrap();
        assert_eq!(audio.path(), source);
        assert_eq!(audio.frames, 160);
        assert!(!root.join("prepare.log").exists());
    }

    #[cfg(unix)]
    #[test]
    fn float16k_import_is_converted_to_the_storage_and_model_pcm16_contract() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let process = converter(&root);
        let source = root.join("float.wav");
        wav(&source, 16000, true, 160);
        wav(&root.join("float.wav.converted"), 16000, false, 160);
        let work = root.join("job");
        fs::create_dir(&work).unwrap();
        let audio =
            AnalysisAudio::prepare(&process, &source, &work, &AtomicBool::new(false)).unwrap();
        assert_ne!(audio.path(), source);
        let core = grove_core::CoreHandle::open(root.join("store")).unwrap();
        core.import_audio("Synthetic float", &source, audio.path())
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn missing_converter_and_log_creation_failures_keep_their_categories() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = root.join("recorded.wav");
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

    #[cfg(unix)]
    #[test]
    fn recorded_float_audio_is_prepared_once_and_bound_to_its_source() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let process = converter(&root);
        let source = root.join("recorded.wav");
        wav(&source, 48000, true, 480);
        wav(&root.join("recorded.wav.converted"), 16000, false, 160);
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
        // Reuse does not invoke the converter again.
        fs::rename(
            root.join("native/AudioPrepare"),
            root.join("native/AudioPrepare-unused"),
        )
        .unwrap();
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

    #[cfg(unix)]
    #[test]
    fn source_changes_and_non_normalized_converter_output_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let process = converter(&root);
        let source = root.join("recorded.wav");
        wav(&source, 48000, true, 480);
        wav(&root.join("recorded.wav.converted"), 48000, true, 480);
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
        wav(&root.join("recorded.wav.converted"), 16000, false, 160);
        let work = root.join("second");
        fs::create_dir(&work).unwrap();
        let audio = AnalysisAudio::prepare(&process, &source, &work, &cancel).unwrap();
        wav(&source, 48000, true, 960);
        assert_eq!(
            audio.verify(&source, &cancel).unwrap_err().kind,
            FailureKind::SourceChanged
        );
    }
}
