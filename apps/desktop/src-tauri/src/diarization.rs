//! Optional whole-recording speaker analysis. It never treats ASR-local tags as identities.
use crate::processing::{
    cancelled, directory, invalidate_readiness, matches_file, plain_file, Model, Processing,
    Progress, Worker,
};
use crate::summary_failure::{Failure, FailureKind};
use grove_core::{CoreHandle, SpeakerAnalysisDraft, Transcript, TranscriptVersion};
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{atomic::AtomicBool, Arc};
use std::thread;
use std::time::Duration;

pub const MANIFEST: &str = include_str!("../../models/diarization.json");
const RUNTIME: &str = "nemo-speech-0.2.0@6a3ca369370782acee0dd155e34394ba60b920c4";
const RESULT_LIMIT: u64 = 16 * 1024 * 1024;

fn model() -> Model {
    serde_json::from_str(MANIFEST).expect("bundled diarization manifest")
}

fn model_dir(process: &Processing) -> Result<PathBuf, String> {
    directory(
        &process.root,
        &format!("models/Nemotron3-{}", model().revision),
    )
}

fn executable(process: &Processing, gpu: bool) -> PathBuf {
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

pub fn availability(process: &Processing) -> (bool, bool, u64) {
    let spec = model();
    let supported = (cfg!(all(target_os = "macos", target_arch = "aarch64"))
        || cfg!(all(target_os = "windows", target_arch = "x86_64")))
        && (executable(process, true).is_file() || executable(process, false).is_file());
    let ready = model_dir(process).ok().is_some_and(|directory| {
        plain_file(&directory.join("verified.json"))
            .ok()
            .is_some_and(|file| {
                let mut contents = String::new();
                file.take(65536).read_to_string(&mut contents).is_ok() && contents == MANIFEST
            })
            && spec.files.iter().all(|item| {
                fs::symlink_metadata(directory.join(&item.name)).is_ok_and(|metadata| {
                    metadata.is_file()
                        && !metadata.file_type().is_symlink()
                        && metadata.len() == item.bytes
                })
            })
    });
    (
        supported,
        ready,
        spec.files.iter().map(|item| item.bytes).sum(),
    )
}

fn failure(kind: FailureKind) -> Failure {
    Failure::new(
        kind,
        match kind {
            FailureKind::Cancelled => "화자 구분을 취소했습니다.",
            FailureKind::Allocation => "화자 구분에 필요한 메모리를 확보하지 못했습니다.",
            FailureKind::BackendUnavailable => "화자 구분 처리 장치를 사용할 수 없습니다.",
            FailureKind::ModelNotReady => "화자 구분 모델을 다시 준비해 주세요.",
            FailureKind::Input => "화자 구분에 사용할 녹음 파일을 읽지 못했습니다.",
            FailureKind::Output | FailureKind::Protocol => "화자 구분 결과를 읽지 못했습니다.",
            FailureKind::Storage => "화자 구분 결과를 저장하지 못했습니다.",
            FailureKind::Cleanup => {
                "화자 구분 실행기의 종료를 확인하지 못했습니다. 앱을 다시 열어 주세요."
            }
            FailureKind::SourceChanged => {
                "처리 중 음성 기록이 변경됐습니다. 현재 기록은 유지했습니다."
            }
            _ => "화자 구분을 완료하지 못했습니다.",
        },
    )
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), Failure> {
    cancelled(cancel).map_err(|_| failure(FailureKind::Cancelled))
}

fn record(path: &Path, value: &Value) -> Result<(), Failure> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| failure(FailureKind::Storage))?;
    serde_json::to_writer_pretty(&mut file, value).map_err(|_| failure(FailureKind::Storage))?;
    file.write_all(b"\n")
        .map_err(|_| failure(FailureKind::Storage))?;
    file.sync_all().map_err(|_| failure(FailureKind::Storage))
}

fn args(audio: &Path, model: &Path, output: &Path, gpu: bool) -> Vec<std::ffi::OsString> {
    [
        Path::new("--json"),
        Path::new("--verbose"),
        Path::new("diarize"),
        audio,
        Path::new("--model"),
        model,
        Path::new("--backend"),
        Path::new(if gpu {
            if cfg!(target_os = "macos") {
                "metal"
            } else {
                "vulkan"
            }
        } else {
            "cpu"
        }),
        Path::new("--preset"),
        Path::new("v3-offline"),
        Path::new("--no-batching"),
        Path::new("--format"),
        Path::new("json"),
        Path::new("--output"),
        output,
    ]
    .iter()
    .map(|path| path.as_os_str().to_os_string())
    .collect()
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
}

fn classify(log: &str, gpu: bool, exit_code: Option<i32>, input: Option<&Path>) -> FailureKind {
    // A missing Windows loader dependency is different from inference/allocation failure.
    if gpu && exit_code == Some(0xc0000135u32 as i32) {
        return FailureKind::BackendUnavailable;
    }
    for line in log.lines().rev() {
        let Ok(error) = serde_json::from_str::<WorkerError>(line) else {
            continue;
        };
        let error = error.error;
        if error.command != "diarize" || Some(error.exit_code) != exit_code {
            continue;
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
        let _ = file.take(RESULT_LIMIT).read_to_string(&mut log);
    }
    log
}

fn run(
    process: &Processing,
    executable: &Path,
    arguments: &[std::ffi::OsString],
    log: &Path,
    cancel: &AtomicBool,
    gpu: bool,
    input: Option<&Path>,
) -> Result<(), Failure> {
    check_cancel(cancel)?;
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(log)
        .map_err(|_| failure(FailureKind::Storage))?;
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .current_dir(log.parent().expect("worker log directory"))
        .env("HF_HUB_OFFLINE", "1")
        .env("TRANSFORMERS_OFFLINE", "1")
        .stdout(Stdio::from(
            output
                .try_clone()
                .map_err(|_| failure(FailureKind::Storage))?,
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
            .map_err(|_| failure(FailureKind::EngineUnavailable))?,
    );
    loop {
        if cancelled(cancel).is_err() {
            worker.stop().map_err(|_| {
                process.cleanup_failed();
                failure(FailureKind::Cleanup)
            })?;
            return Err(failure(FailureKind::Cancelled));
        }
        match worker.0.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(failure(classify(
                    &bounded_log(log),
                    gpu,
                    status.code(),
                    input,
                )))
            }
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(_) => {
                worker.stop().map_err(|_| {
                    process.cleanup_failed();
                    failure(FailureKind::Cleanup)
                })?;
                return Err(failure(FailureKind::EngineUnavailable));
            }
        }
    }
}

fn configuration(gpu: bool) -> Value {
    json!({
        "scope": "recording", "preset": "v3-offline", "fullAttention": false,
        "backend": if gpu { if cfg!(target_os = "macos") { "metal" } else { "vulkan" } } else { "cpu" },
        // Pinned DiarSegmentationCfg is also used by long-recording compaction.
        // Preserve the actual float32 onset/offset values in the receipt.
        "postprocessingSource": "pinned-runtime-defaults",
        "onset": 0.641_f32, "offset": 0.561_f32, "padOnset": 0.229, "padOffset": 0.079,
        "minDurationOn": 0.511, "minDurationOff": 0.296, "batching": false
    })
}

fn analyze(
    process: &Processing,
    source: Transcript,
    audio: &Path,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<SpeakerAnalysisDraft, Failure> {
    let spec = model();
    let directory = model_dir(process).map_err(|_| failure(FailureKind::Storage))?;
    let item = &spec.files[0];
    check_cancel(cancel)?;
    if !matches_file(&directory.join(&item.name), item, cancel) {
        check_cancel(cancel)?;
        invalidate_readiness(&directory).map_err(|_| failure(FailureKind::Storage))?;
        return Err(failure(FailureKind::ModelNotReady));
    }
    plain_file(audio).map_err(|_| failure(FailureKind::Input))?;
    let mut gpu = executable(process, true).is_file();
    let mut attempt = 0u8;
    loop {
        check_cancel(cancel)?;
        let attempt_dir = directory_for_attempt(path, attempt)?;
        let output = attempt_dir.join("diarization.json");
        let config = configuration(gpu);
        record(
            &attempt_dir.join("request.json"),
            &json!({
                "sourceVersion": source.version, "sourceSessionId": source.source_session_id,
                "model": spec.repository, "modelRevision": spec.revision,
                "modelSha256": item.sha256, "runtime": RUNTIME, "configuration": config
            }),
        )?;
        process.update(|progress| {
            progress.phase = Some("diarizing".into());
            progress.message = "화자 구분 중".into();
        });
        let result = run(
            process,
            &executable(process, gpu),
            &args(audio, &directory.join(&item.name), &output, gpu),
            &attempt_dir.join("worker.log"),
            cancel,
            gpu,
            Some(audio),
        );
        record(
            &attempt_dir.join("outcome.json"),
            &match &result {
                Ok(()) => {
                    json!({"attempt": attempt, "backend": config["backend"], "status": "completed"})
                }
                Err(error) => {
                    json!({"attempt": attempt, "backend": config["backend"], "status": "failed", "failureKind": error.kind})
                }
            },
        )?;
        match result {
            Ok(()) => {
                check_cancel(cancel)?;
                let file = plain_file(&output).map_err(|_| failure(FailureKind::Output))?;
                if file
                    .metadata()
                    .map_err(|_| failure(FailureKind::Storage))?
                    .len()
                    > RESULT_LIMIT
                {
                    return Err(failure(FailureKind::Output));
                }
                let mut raw = Vec::new();
                file.take(RESULT_LIMIT + 1)
                    .read_to_end(&mut raw)
                    .map_err(|_| failure(FailureKind::Output))?;
                if raw.len() as u64 > RESULT_LIMIT {
                    return Err(failure(FailureKind::Output));
                }
                return Ok(SpeakerAnalysisDraft {
                    source,
                    model: spec.repository,
                    model_revision: spec.revision,
                    model_sha256: item.sha256.clone(),
                    runtime: RUNTIME.into(),
                    configuration: config,
                    raw,
                });
            }
            Err(error)
                if error.kind == FailureKind::BackendUnavailable
                    && gpu
                    && attempt == 0
                    && executable(process, false).is_file() =>
            {
                check_cancel(cancel)?;
                gpu = false;
                attempt += 1;
                process.update(|progress| progress.retry_count = attempt);
            }
            Err(error) => return Err(error),
        }
    }
}

fn directory_for_attempt(path: &Path, attempt: u8) -> Result<PathBuf, Failure> {
    directory(path, &format!("diarization-{attempt}")).map_err(|_| failure(FailureKind::Storage))
}

impl Processing {
    pub fn prepare_diarization(self: &Arc<Self>) -> Result<Progress, String> {
        self.prepare_named_model(MANIFEST, "speaker-model", "Nemotron3")
    }

    pub fn diarize(
        self: &Arc<Self>,
        core: CoreHandle,
        meeting_id: String,
        expected: TranscriptVersion,
    ) -> Result<Progress, String> {
        let source = core
            .transcript(&meeting_id)
            .map_err(|error| error.to_string())?
            .ok_or("먼저 음성 기록을 만들어 주세요.")?;
        if source.version != expected {
            return Err(
                "음성 기록이 변경됐습니다. 현재 기록을 확인한 뒤 다시 시도해 주세요.".into(),
            );
        }
        let audio = core
            .audio_path(&meeting_id)
            .map_err(|error| error.to_string())?;
        if !availability(self).1 {
            return Err("먼저 화자 구분 모델을 준비해 주세요.".into());
        }
        self.start(
            "diarize",
            Some(meeting_id.clone()),
            move |this, cancel, path| {
                this.update(|progress| {
                    progress.phase = Some("preparing".into());
                    progress.message = "화자 구분 준비 중".into();
                });
                let result = (|| -> Result<(), Failure> {
                    let snapshot = core
                        .snapshot()
                        .map_err(|_| failure(FailureKind::MeetingUnavailable))?;
                    if snapshot.session.is_some_and(|session| {
                        matches!(
                            session.state,
                            grove_core::SessionState::Recording | grove_core::SessionState::Paused
                        )
                    }) {
                        return Err(Failure::new(
                            FailureKind::RecordingActive,
                            "녹음을 마친 뒤 화자를 구분해 주세요.",
                        ));
                    }
                    let draft = analyze(&this, source, &audio, &path, &cancel)?;
                    check_cancel(&cancel)?;
                    let mut commit_failure = None;
                    this.commit_transcript_result(&cancel, || {
                        core.apply_diarization(&meeting_id, draft).map_err(|error| {
                            commit_failure = Some(match error {
                                grove_core::CoreError::InvalidState(_) => {
                                    FailureKind::SourceChanged
                                }
                                grove_core::CoreError::MeetingNotFound => {
                                    FailureKind::MeetingUnavailable
                                }
                                grove_core::CoreError::InvalidInput(_) => FailureKind::Output,
                                _ => FailureKind::Storage,
                            });
                            error.to_string()
                        })
                    })
                    .map_err(|_| failure(commit_failure.unwrap_or(FailureKind::Cancelled)))?;
                    Ok(())
                })();
                if let Err(error) = result {
                    this.update(|progress| progress.failure_kind = Some(error.kind));
                    return Err(error.message);
                }
                Ok(Some(meeting_id))
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_recording_invocation_preserves_state_and_never_uses_full_attention() {
        let arguments = args(
            Path::new("whole.wav"),
            Path::new("pinned.gguf"),
            Path::new("new.json"),
            true,
        );
        let values: Vec<_> = arguments
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect();
        assert_eq!(
            values.iter().filter(|value| **value == "whole.wav").count(),
            1
        );
        assert_eq!(values[0..3], ["--json", "--verbose", "diarize"]);
        assert!(values
            .windows(2)
            .any(|pair| pair == ["--preset", "v3-offline"]));
        assert!(values.contains(&"--no-batching"));
        assert!(!values.contains(&"--offline"));
        for flag in [
            "--onset",
            "--offset",
            "--pad-onset",
            "--pad-offset",
            "--min-duration-on",
            "--min-duration-off",
        ] {
            assert!(!values.contains(&flag));
        }
        let config = configuration(true);
        assert_eq!(config["postprocessingSource"], "pinned-runtime-defaults");
        assert_eq!(config["onset"], json!(0.641_f32));
        assert_eq!(config["offset"], json!(0.561_f32));
        assert_eq!(config["padOnset"], json!(0.229));
        assert_eq!(config["padOffset"], json!(0.079));
        assert_eq!(config["minDurationOn"], json!(0.511));
        assert_eq!(config["minDurationOff"], json!(0.296));
    }

    fn worker_error(message: &str, kind: &str) -> String {
        json!({"error": {"message": message, "type": kind, "command": "diarize", "exit_code": 1}})
            .to_string()
    }

    #[test]
    fn cpu_fallback_requires_typed_gpu_absence_and_never_an_os_pressure_hint() {
        let missing = worker_error(
            "use_gpu=true but no matching GPU device found (gpu_device_idx=0)",
            "runtime_error",
        );
        assert_eq!(
            classify(&missing, true, Some(1), None),
            FailureKind::BackendUnavailable
        );
        assert_eq!(
            classify(&missing, false, Some(1), None),
            FailureKind::EngineUnavailable
        );
        assert_eq!(
            classify(&missing, true, Some(2), None),
            FailureKind::EngineUnavailable
        );
        assert_eq!(
            classify("memory warning", true, Some(1), None),
            FailureKind::EngineUnavailable
        );
        assert_eq!(
            classify(
                "use_gpu=true but no matching GPU device found (gpu_device_idx=0)",
                true,
                Some(1),
                None
            ),
            FailureKind::EngineUnavailable
        );
        assert_eq!(
            classify(
                &worker_error("std::bad_alloc", "runtime_error"),
                true,
                Some(1),
                None
            ),
            FailureKind::Allocation
        );
        assert_eq!(
            classify("", true, Some(0xc0000135u32 as i32), None),
            FailureKind::BackendUnavailable
        );
    }

    #[test]
    fn model_and_output_storage_failures_remain_distinct() {
        assert_eq!(
            classify(
                &worker_error("missing pinned model", "missing_model"),
                true,
                Some(1),
                None
            ),
            FailureKind::ModelNotReady
        );
        assert_eq!(
            classify(
                &worker_error("cannot write private/path", "runtime_error"),
                true,
                Some(1),
                None
            ),
            FailureKind::Storage
        );
        assert!(!failure(FailureKind::Storage)
            .message
            .contains("private/path"));
    }

    #[test]
    fn per_input_allocation_error_requires_the_invoked_path_and_typed_error_contract() {
        let input = Path::new("example/whole.wav");
        let error = worker_error("example/whole.wav: std::bad_alloc", "runtime_error");
        assert_eq!(
            classify(&error, true, Some(1), Some(input)),
            FailureKind::Allocation
        );
        for actual_input in [None, Some(Path::new("other/whole.wav"))] {
            assert_eq!(
                classify(&error, true, Some(1), actual_input),
                FailureKind::EngineUnavailable
            );
        }
        assert_eq!(
            classify(&error, true, Some(2), Some(input)),
            FailureKind::EngineUnavailable
        );
        let mut wrong_command: Value = serde_json::from_str(&error).unwrap();
        wrong_command["error"]["command"] = "transcribe".into();
        assert_eq!(
            classify(&wrong_command.to_string(), true, Some(1), Some(input)),
            FailureKind::EngineUnavailable
        );
        assert_eq!(
            classify(
                "example/whole.wav: std::bad_alloc",
                true,
                Some(1),
                Some(input)
            ),
            FailureKind::EngineUnavailable
        );
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_reaps_the_single_worker_without_starting_a_cpu_retry() {
        let temporary = tempfile::tempdir().unwrap();
        let process = Processing::new(temporary.path().into(), temporary.path().into());
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
            )
        });
        thread::sleep(Duration::from_millis(150));
        cancel.store(true, std::sync::atomic::Ordering::Release);
        assert_eq!(
            worker.join().unwrap().unwrap_err().kind,
            FailureKind::Cancelled
        );
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
