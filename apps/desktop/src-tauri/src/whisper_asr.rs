//! Whisper entry points. PCM ownership, native projection, runtime trust and execution are separate.
mod backend;
mod buffered;
mod native;
mod probe;
mod runtime;
use crate::processing::{cancelled, plain_file};
pub(super) use buffered::Buffered;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub(super) const MANIFEST: &str = include_str!("../../../../config/models/whisper-turbo.json");
const RUNTIME: &str = include_str!("../../../../scripts/native-whisper/runtime.json");
const RATE: u64 = 16000;
const CONTEXT: u64 = 3 * RATE;
const OWNED: u64 = 120 * RATE;
const MIN_OWNED: u64 = 90 * RATE;

pub(super) fn executable(resources: &Path) -> PathBuf {
    resources.join("native/whisper/bin").join(if cfg!(windows) {
        "whisper-cli.exe"
    } else {
        "whisper-cli"
    })
}
pub(super) fn supported(resources: &Path) -> bool {
    (cfg!(all(target_os = "windows", target_arch = "x86_64"))
        || cfg!(all(target_os = "macos", target_arch = "aarch64")))
        && executable(resources).is_file()
}

fn failure() -> String {
    "Whisper의 원본 글자나 시각을 확인하지 못했습니다. 원음과 기존 기록은 유지했습니다.".into()
}

fn save(path: &Path, value: &Value) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut file, value).map_err(|e| e.to_string())?;
    file.write_all(b"\n")
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}

fn read(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    plain_file(path)?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err(failure());
    }
    Ok(bytes)
}

fn file_hash(path: &Path) -> Result<String, String> {
    crate::speech_audio::digest(path).map_err(|error| error.message)
}

pub(super) fn recognize(
    resources: &Path,
    audio: &Path,
    work: &Path,
    cancel: &AtomicBool,
    model_dir: &Path,
) -> Result<Vec<u8>, String> {
    let model: Value = serde_json::from_str(MANIFEST).map_err(|_| failure())?;
    let mut buffered = Buffered::new(
        executable(resources),
        model_dir.join(model["files"][0]["name"].as_str().ok_or_else(failure)?),
        work.to_path_buf(),
        false,
    )?;
    let mut wav = hound::WavReader::new(plain_file(audio)?).map_err(|_| failure())?;
    if wav.spec().channels != 1
        || wav.spec().sample_rate != 16000
        || wav.spec().bits_per_sample != 16
        || wav.spec().sample_format != hound::SampleFormat::Int
    {
        return Err(failure());
    }
    let frames = u64::from(wav.duration());
    let mut samples = wav.samples::<i16>();
    loop {
        let block = samples
            .by_ref()
            .take(16000)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| failure())?;
        if block.is_empty() {
            break;
        }
        buffered.push(&block, cancel)?;
    }
    let sha = buffered.pcm_digest();
    let result = buffered.finish(frames, &sha, cancel)?;
    serde_json::to_vec(&result).map_err(|_| failure())
}
