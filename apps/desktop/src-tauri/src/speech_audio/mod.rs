use crate::processing::{cancelled, plain_file, Processing};
use crate::summary_failure::{Failure, FailureKind};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

mod decode;
mod formats;
mod framing;
mod import;
pub(crate) mod normalize;
mod output;
mod platform;
mod wav;

pub(crate) use formats::import_extensions;
pub(crate) use import::stage_import;

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
    pub(super) fn from_live(
        source: &Path,
        path: &Path,
        cancel: &AtomicBool,
    ) -> Result<Self, Failure> {
        let prepared = Self {
            source: source.to_owned(),
            source_sha256: digest_cancellable(source, cancel)?,
            path: path.to_owned(),
            frames: native_frames(path, cancel)?,
            sha256: digest_cancellable(path, cancel)?,
        };
        prepared.verify(source, cancel)?;
        Ok(prepared)
    }
    pub(super) fn prepare(
        process: &Processing,
        source: &Path,
        work: &Path,
        cancel: &AtomicBool,
    ) -> Result<Self, Failure> {
        check_cancel(cancel)?;
        let source_sha256 = digest_cancellable(source, cancel)?;
        let (path, frames) = match native_frames(source, cancel) {
            Ok(frames) => (source.to_path_buf(), frames),
            Err(error) if error.kind == FailureKind::Cancelled => return Err(error),
            Err(_) => {
                let path = work.join("analysis.wav");
                platform::prepare(process, source, &path, work, cancel)?;
                let frames = native_frames(&path, cancel)?;
                (path, frames)
            }
        };
        let sha256 = digest_cancellable(&path, cancel)?;
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
            || digest_cancellable(source, cancel)? != self.source_sha256
            || digest_cancellable(&self.path, cancel)? != self.sha256
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
    digest_cancellable(path, &AtomicBool::new(false))
}

fn digest_cancellable(path: &Path, cancel: &AtomicBool) -> Result<String, Failure> {
    let mut input = plain_file(path).map_err(|_| failure(FailureKind::Input))?;
    let mut hash = Sha256::new();
    let mut block = [0u8; 65536];
    loop {
        check_cancel(cancel)?;
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
mod tests;
