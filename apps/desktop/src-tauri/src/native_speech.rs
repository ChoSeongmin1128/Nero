//! The pinned NeMo speech package shared by recognition, speaker analysis and audio
//! preparation: its file locations, backend names and the bounded child runner.
use crate::processing::failure::FailureKind;
use crate::processing::{cancelled, plain_file, Processing, Worker};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::thread;
use std::time::Duration;

const LOG_LIMIT: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Backend {
    Cpu,
    Metal,
    Vulkan,
}

impl Backend {
    pub(crate) fn for_gpu(gpu: bool) -> Self {
        if !gpu {
            Self::Cpu
        } else if cfg!(target_os = "macos") {
            Self::Metal
        } else {
            Self::Vulkan
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Metal => "metal",
            Self::Vulkan => "vulkan",
        }
    }

    // The native C ABI device index: -1 selects the CPU.
    pub(crate) fn gpu(self) -> i32 {
        if matches!(self, Self::Cpu) {
            -1
        } else {
            0
        }
    }
}

pub(crate) fn executable(process: &Processing, gpu: bool) -> PathBuf {
    process
        .resources
        .join("native/diarization")
        .join(if gpu || cfg!(target_os = "macos") {
            "gpu"
        } else {
            "cpu"
        })
        .join("bin")
        .join(if cfg!(windows) {
            "nemo-speech.exe"
        } else {
            "nemo-speech"
        })
}

pub(crate) fn library(process: &Processing, gpu: bool) -> PathBuf {
    let root = executable(process, gpu)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    if cfg!(windows) {
        root.join("bin/nemo_speech_asr_c.dll")
    } else {
        root.join("lib/libnemo_speech_asr_c.dylib")
    }
}

#[derive(Deserialize)]
struct WorkerError {
    error: ErrorBody,
}

#[derive(Deserialize)]
struct ErrorBody {
    message: String,
    r#type: String,
    command: String,
    exit_code: i32,
    #[serde(default, rename = "failureKind")]
    failure_kind: Option<FailureKind>,
}

fn classify(
    log: &str,
    gpu: bool,
    exit_code: Option<i32>,
    input: Option<&Path>,
    operation: &str,
) -> FailureKind {
    // A missing Windows loader dependency is different from inference/allocation failure.
    if gpu && exit_code == Some(0xc0000135u32 as i32) {
        return FailureKind::BackendUnavailable;
    }
    for line in log.lines().rev() {
        let Ok(error) = serde_json::from_str::<WorkerError>(line) else {
            continue;
        };
        let error = error.error;
        if error.command != operation || Some(error.exit_code) != exit_code {
            continue;
        }
        if operation == "diarize" && error.r#type == "nero_speaker_worker" {
            if let Some(kind) = error.failure_kind {
                return match kind {
                    FailureKind::BackendUnavailable if !gpu => FailureKind::EngineUnavailable,
                    FailureKind::BackendUnavailable
                    | FailureKind::Allocation
                    | FailureKind::ModelNotReady
                    | FailureKind::Input
                    | FailureKind::Output
                    | FailureKind::Protocol
                    | FailureKind::Storage
                    | FailureKind::SourceChanged => kind,
                    _ => FailureKind::EngineUnavailable,
                };
            }
        }
        if gpu
            && (error.message == "use_gpu=true but no matching GPU device found (gpu_device_idx=0)"
                || error.message
                    == "--backend requested Metal, but this build has no Metal backend"
                || error.message
                    == "--backend requested Vulkan, but this build has no Vulkan backend")
        {
            return FailureKind::BackendUnavailable;
        }
        if error.r#type == "missing_model" {
            return FailureKind::ModelNotReady;
        }
        if error.message == "std::bad_alloc"
            || input.is_some_and(|input| {
                error.message == format!("{}: std::bad_alloc", input.to_string_lossy())
            })
        {
            return FailureKind::Allocation;
        }
        if error.r#type == "runtime_error" && input.is_some_and(|input| {
            let prefix = format!("{}: ", input.to_string_lossy());
            let message = error.message.strip_prefix(&prefix);
            let message = message.map(|message| message.strip_prefix(&prefix).unwrap_or(message));
            message.is_some_and(|message| message == "unsupported WAV encoding; convert with: ffmpeg -i INPUT -ac 1 -ar 16000 -c:a pcm_s16le output.wav")
        }) {
            return FailureKind::Input;
        }
        if error.message.starts_with("cannot write ")
            || error.message.starts_with("cannot finish writing ")
        {
            return FailureKind::Storage;
        }
    }
    FailureKind::EngineUnavailable
}

fn bounded_log(path: &Path) -> String {
    let mut log = String::new();
    if let Ok(file) = plain_file(path) {
        let _ = file.take(LOG_LIMIT).read_to_string(&mut log);
    }
    log
}

// Runs one packaged child to completion. Each caller words the failure for its own stage.
pub(crate) fn run(
    process: &Processing,
    executable: &Path,
    arguments: &[std::ffi::OsString],
    log: &Path,
    cancel: &AtomicBool,
    gpu: bool,
    input: Option<&Path>,
    operation: &str,
) -> Result<(), FailureKind> {
    cancelled(cancel).map_err(|_| FailureKind::Cancelled)?;
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(log)
        .map_err(|_| FailureKind::Storage)?;
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .current_dir(log.parent().expect("worker log directory"))
        .env("HF_HUB_OFFLINE", "1")
        .env("TRANSFORMERS_OFFLINE", "1")
        .stdout(Stdio::from(
            output.try_clone().map_err(|_| FailureKind::Storage)?,
        ))
        .stderr(Stdio::from(output));
    // Only app-owned CLI arguments configure this invocation, not a parent shell's NeMo profile.
    for (name, _) in std::env::vars_os() {
        if name
            .to_str()
            .is_some_and(|name| name.starts_with("NEMO_SPEECH_"))
        {
            command.env_remove(name);
        }
    }
    let mut worker = Worker(
        command
            .spawn()
            .map_err(|_| FailureKind::EngineUnavailable)?,
    );
    loop {
        if cancelled(cancel).is_err() {
            worker.stop().map_err(|_| {
                process.cleanup_failed();
                FailureKind::Cleanup
            })?;
            return Err(FailureKind::Cancelled);
        }
        match worker.0.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(classify(
                    &bounded_log(log),
                    gpu,
                    status.code(),
                    input,
                    operation,
                ))
            }
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(_) => {
                worker.stop().map_err(|_| {
                    process.cleanup_failed();
                    FailureKind::Cleanup
                })?;
                return Err(FailureKind::EngineUnavailable);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::sync::Arc;

    fn classify_diarize(
        log: &str,
        gpu: bool,
        exit_code: Option<i32>,
        input: Option<&Path>,
    ) -> FailureKind {
        classify(log, gpu, exit_code, input, "diarize")
    }

    fn worker_error(message: &str, kind: &str) -> String {
        json!({"error": {"message": message, "type": kind, "command": "diarize", "exit_code": 1}})
            .to_string()
    }

    #[test]
    fn backend_names_match_their_serialized_request_form() {
        for backend in [Backend::Cpu, Backend::Metal, Backend::Vulkan] {
            assert_eq!(serde_json::to_value(backend).unwrap(), backend.name());
        }
        assert_eq!(Backend::for_gpu(false), Backend::Cpu);
        assert_eq!(
            Backend::for_gpu(true),
            if cfg!(target_os = "macos") {
                Backend::Metal
            } else {
                Backend::Vulkan
            }
        );
        assert_eq!(Backend::Cpu.gpu(), -1);
        assert_eq!(Backend::for_gpu(true).gpu(), 0);
    }

    #[test]
    fn cpu_fallback_requires_typed_gpu_absence_and_never_an_os_pressure_hint() {
        let missing = worker_error(
            "use_gpu=true but no matching GPU device found (gpu_device_idx=0)",
            "runtime_error",
        );
        assert_eq!(
            classify_diarize(&missing, true, Some(1), None),
            FailureKind::BackendUnavailable
        );
        assert_eq!(
            classify_diarize(&missing, false, Some(1), None),
            FailureKind::EngineUnavailable
        );
        assert_eq!(
            classify_diarize(&missing, true, Some(2), None),
            FailureKind::EngineUnavailable
        );
        assert_eq!(
            classify_diarize("memory warning", true, Some(1), None),
            FailureKind::EngineUnavailable
        );
        assert_eq!(
            classify_diarize(
                "use_gpu=true but no matching GPU device found (gpu_device_idx=0)",
                true,
                Some(1),
                None
            ),
            FailureKind::EngineUnavailable
        );
        assert_eq!(
            classify_diarize(
                &worker_error("std::bad_alloc", "runtime_error"),
                true,
                Some(1),
                None
            ),
            FailureKind::Allocation
        );
        assert_eq!(
            classify_diarize("", true, Some(0xc0000135u32 as i32), None),
            FailureKind::BackendUnavailable
        );
        for kind in [
            FailureKind::BackendUnavailable,
            FailureKind::Allocation,
            FailureKind::Protocol,
        ] {
            let error = json!({"error":{"message":"native operation failed",
                "type":"nero_speaker_worker","command":"diarize","exit_code":1,"failureKind":kind}})
            .to_string();
            assert_eq!(classify_diarize(&error, true, Some(1), None), kind);
            assert_eq!(
                classify_diarize(&error, false, Some(1), None),
                if kind == FailureKind::BackendUnavailable {
                    FailureKind::EngineUnavailable
                } else {
                    kind
                }
            );
            assert_eq!(
                classify_diarize(&error, true, Some(2), None),
                FailureKind::EngineUnavailable
            );
        }
    }

    #[test]
    fn model_and_output_storage_failures_remain_distinct() {
        assert_eq!(
            classify_diarize(
                &worker_error("missing pinned model", "missing_model"),
                true,
                Some(1),
                None
            ),
            FailureKind::ModelNotReady
        );
        assert_eq!(
            classify_diarize(
                &worker_error("cannot write private/path", "runtime_error"),
                true,
                Some(1),
                None
            ),
            FailureKind::Storage
        );
    }

    #[test]
    fn per_input_allocation_error_requires_the_invoked_path_and_typed_error_contract() {
        let input = Path::new("example/whole.wav");
        let error = worker_error("example/whole.wav: std::bad_alloc", "runtime_error");
        assert_eq!(
            classify_diarize(&error, true, Some(1), Some(input)),
            FailureKind::Allocation
        );
        for actual_input in [None, Some(Path::new("other/whole.wav"))] {
            assert_eq!(
                classify_diarize(&error, true, Some(1), actual_input),
                FailureKind::EngineUnavailable
            );
        }
        assert_eq!(
            classify_diarize(&error, true, Some(2), Some(input)),
            FailureKind::EngineUnavailable
        );
        let mut wrong_command: Value = serde_json::from_str(&error).unwrap();
        wrong_command["error"]["command"] = "transcribe".into();
        assert_eq!(
            classify_diarize(&wrong_command.to_string(), true, Some(1), Some(input)),
            FailureKind::EngineUnavailable
        );
        assert_eq!(
            classify_diarize(
                "example/whole.wav: std::bad_alloc",
                true,
                Some(1),
                Some(input)
            ),
            FailureKind::EngineUnavailable
        );
    }

    #[test]
    fn unsupported_wave_is_an_input_error_only_for_the_invoked_native_contract() {
        let input = Path::new("private/recorded.wav");
        let diagnostic = "unsupported WAV encoding; convert with: ffmpeg -i INPUT -ac 1 -ar 16000 -c:a pcm_s16le output.wav";
        for message in [
            format!("private/recorded.wav: {diagnostic}"),
            format!("private/recorded.wav: private/recorded.wav: {diagnostic}"),
        ] {
            let log = worker_error(&message, "runtime_error");
            assert_eq!(
                classify_diarize(&log, true, Some(1), Some(input)),
                FailureKind::Input
            );
            assert_eq!(
                classify_diarize(&log, true, Some(2), Some(input)),
                FailureKind::EngineUnavailable
            );
            assert_eq!(
                classify_diarize(&log, true, Some(1), Some(Path::new("other.wav"))),
                FailureKind::EngineUnavailable
            );
            assert_eq!(
                classify_diarize(
                    &worker_error(&message, "unknown"),
                    true,
                    Some(1),
                    Some(input)
                ),
                FailureKind::EngineUnavailable
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_reaps_the_single_worker_without_starting_a_cpu_retry() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let process = Processing::new(root.clone(), root);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let log = temporary.path().join("worker.log");
        let started = std::time::Instant::now();
        let worker = thread::spawn(move || {
            run(
                &process,
                Path::new("/bin/sleep"),
                &["30".into()],
                &log,
                &worker_cancel,
                false,
                None,
                "diarize",
            )
        });
        thread::sleep(Duration::from_millis(150));
        cancel.store(true, std::sync::atomic::Ordering::Release);
        assert_eq!(worker.join().unwrap().unwrap_err(), FailureKind::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
