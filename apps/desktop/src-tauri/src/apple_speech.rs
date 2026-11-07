use crate::processing::failure::{Failure, FailureKind};
use crate::processing::{cancelled, plain_file, Processing};
use crate::speech_choice::AppleVariant;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

pub(super) const MANIFEST: &str =
    r#"{"repository":"apple/SpeechTranscriber","revision":"system-managed-ko-KR","files":[]}"#;
const LIMIT: u64 = 16 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Availability {
    pub supported: bool,
    pub ready: bool,
    pub locale: String,
    pub asset_status: String,
    pub message: String,
    pub variant: Option<AppleVariant>,
    pub speech_available: bool,
    pub speech_locale_supported: bool,
    pub dictation_locale_supported: bool,
}
impl Default for Availability {
    fn default() -> Self {
        Self {
            variant: None,
            speech_available: false,
            speech_locale_supported: false,
            dictation_locale_supported: false,
            supported: false,
            ready: false,
            locale: "ko-KR".into(),
            asset_status: "unsupported".into(),
            message: "이 환경에서는 macOS 기본 전사를 사용할 수 없습니다.".into(),
        }
    }
}
fn executable(resources: &Path) -> PathBuf {
    let bundled = resources.join("native/apple-speech/nero-apple-speech");
    #[cfg(all(target_os = "macos", debug_assertions, not(test)))]
    if !bundled.is_file() {
        if let Some(path) = option_env!("NERO_APPLE_SPEECH_HELPER") {
            return path.into();
        }
    }
    bundled
}
fn availability(raw: &[u8]) -> Result<Availability, String> {
    let value: Availability =
        serde_json::from_slice(raw).map_err(|_| "macOS 기본 전사 상태를 확인하지 못했습니다.")?;
    if value.locale != "ko-KR"
        || !matches!(
            value.asset_status.as_str(),
            "unsupported" | "supported" | "downloading" | "installed" | "unknown"
        )
        || value.ready != (value.supported && value.asset_status == "installed")
        || value.supported && value.variant.is_none()
        || value.message.len() > 1024
    {
        return Err("macOS 기본 전사 상태를 확인하지 못했습니다.".into());
    }
    Ok(value)
}
pub(super) fn probe(resources: &Path, variant: Option<AppleVariant>) -> Availability {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) || !executable(resources).is_file()
    {
        return Availability::default();
    }
    let mut command = Command::new(executable(resources));
    command
        .arg("status")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(variant) = variant {
        command.arg(variant.argument());
    }
    let Ok(mut child) = crate::process_tree::spawn(&mut command) else {
        return unavailable();
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let mut raw = Vec::new();
                if child
                    .stdout
                    .take()
                    .is_some_and(|stdout| stdout.take(8193).read_to_end(&mut raw).is_ok())
                    && raw.len() <= 8192
                {
                    return availability(&raw).unwrap_or_else(|_| unavailable());
                }
                return unavailable();
            }
            Ok(Some(_)) | Err(_) => return unavailable(),
            Ok(None) if started.elapsed() >= Duration::from_secs(15) => return unavailable(),
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}
fn unavailable() -> Availability {
    Availability {
        asset_status: "unknown".into(),
        message: "macOS 기본 전사 상태를 확인하지 못했습니다. 다시 선택해 주세요.".into(),
        ..Availability::default()
    }
}
fn read(path: &Path) -> Result<Vec<u8>, String> {
    let file = plain_file(path)?;
    if file
        .metadata()
        .map_err(|_| "전사 결과를 읽지 못했습니다.")?
        .len()
        > LIMIT
    {
        return Err("전사 결과가 너무 큽니다.".into());
    }
    let mut raw = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut raw)
        .map_err(|_| "전사 결과를 읽지 못했습니다.")?;
    if raw.len() as u64 > LIMIT {
        return Err("전사 결과가 너무 큽니다.".into());
    }
    Ok(raw)
}
fn execute(
    process: &Processing,
    mode: &str,
    audio: Option<&Path>,
    output: &Path,
    cancel: &AtomicBool,
    variant: AppleVariant,
) -> Result<(), Failure> {
    cancelled(cancel).map_err(|message| Failure::new(FailureKind::Cancelled, message))?;
    let mut command = Command::new(executable(&process.resources));
    command.arg(mode).arg(variant.argument());
    if let Some(audio) = audio {
        command.arg(audio);
    }
    command
        .arg(output)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = crate::process_tree::spawn(&mut command).map_err(|_| {
        Failure::new(
            FailureKind::EngineUnavailable,
            "macOS 기본 전사를 시작하지 못했습니다.",
        )
    })?;
    loop {
        if cancelled(cancel).is_err() {
            if let Some(mut input) = child.stdin.take() {
                let _ = input.write_all(b"c");
            }
            for _ in 0..50 {
                if child.try_wait().ok().flatten().is_some() {
                    return Err(Failure::new(
                        FailureKind::Cancelled,
                        "음성 처리를 취소했습니다.",
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if child.kill().is_err() || child.wait().is_err() {
                process.cleanup_failed();
                return Err(Failure::new(
                    FailureKind::Cleanup,
                    "음성 처리 종료를 확인하지 못했습니다. 앱을 다시 열어 주세요.",
                ));
            }
            return Err(Failure::new(
                FailureKind::Cancelled,
                "음성 처리를 취소했습니다.",
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err(Failure::new(FailureKind::EngineUnavailable, "macOS 기본 전사를 완료하지 못했습니다. 언어 자산을 다시 준비해 주세요. 원음과 기존 기록은 유지됩니다.")),
            Err(_) => return Err(Failure::new(FailureKind::EngineUnavailable, "macOS 기본 전사 상태를 확인하지 못했습니다.")),
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}
pub(super) fn prepare(
    process: &Processing,
    path: &Path,
    cancel: &AtomicBool,
    variant: AppleVariant,
) -> Result<Availability, Failure> {
    let output = path.join("apple-preparation.json");
    execute(process, "prepare", None, &output, cancel, variant)?;
    availability(&read(&output).map_err(|m| Failure::new(FailureKind::Output, m))?)
        .map_err(|m| Failure::new(FailureKind::Output, m))
}
pub(super) fn recognize(
    process: &Processing,
    audio: &Path,
    path: &Path,
    cancel: &AtomicBool,
    variant: AppleVariant,
) -> Result<Vec<u8>, Failure> {
    let frames = crate::speech_audio::audio_frames(audio, cancel)?;
    let input_sha = crate::speech_audio::digest(audio)?;
    let helper_sha = crate::speech_audio::digest(&executable(&process.resources))?;
    let native_path = path.join("apple-native.json");
    execute(
        process,
        "transcribe",
        Some(audio),
        &native_path,
        cancel,
        variant,
    )?;
    let raw = read(&native_path).map_err(|m| Failure::new(FailureKind::Output, m))?;
    let native: Value = serde_json::from_slice(&raw).map_err(|_| {
        Failure::new(
            FailureKind::Output,
            "macOS 기본 전사 결과를 읽지 못했습니다.",
        )
    })?;
    if native["variant"] != variant.argument()
        || native["inputFrames"].as_u64() != Some(frames)
        || native["consumedInputFrames"].as_u64() != Some(frames)
        || native["finalized"] != true
        || crate::speech_audio::digest(audio)? != input_sha
        || crate::speech_audio::digest(&executable(&process.resources))? != helper_sha
    {
        return Err(Failure::new(
            FailureKind::SourceChanged,
            "처리 중 음성 원본 또는 실행기가 변경됐습니다. 기존 기록은 유지됩니다.",
        ));
    }
    let native_json = String::from_utf8(raw)
        .map_err(|_| Failure::new(FailureKind::Output, "전사 결과의 문자를 읽지 못했습니다."))?;
    let native_sha = crate::speech_audio::digest(&native_path)?;
    let envelope = json!({"schemaVersion":1,"model":match variant { AppleVariant::SpeechTranscriber => "apple/SpeechTranscriber", AppleVariant::DictationTranscriber => "apple/DictationTranscriber" },
        "modelRevision":format!("{}:ko-KR",native["systemVersion"].as_str().ok_or_else(|| Failure::new(FailureKind::Output,"macOS 버전 정보를 읽지 못했습니다."))?),
        "runtime":"apple-speech-analyzer-v1","processingPolicy":"whole-recording-native-results-v1",
        "completionEvidence":"speech-analyzer-finalized-file-exit0-v1","completed":true,
        "inputAudioFrames":frames,"audioSampleRate":16000,"inputAudioSha256":input_sha,
        "helperSha256":helper_sha,"nativeResultSha256":native_sha,"nativeResultJson":native_json});
    serde_json::to_vec(&envelope)
        .map_err(|_| Failure::new(FailureKind::Output, "전사 결과를 저장하지 못했습니다."))
}
