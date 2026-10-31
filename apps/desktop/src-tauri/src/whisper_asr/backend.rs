//! Backend selection and native process lifecycle; stdout never establishes acceleration.
use super::runtime::Runtime;
use super::*;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Backend {
    Cpu,
    Metal,
    Vulkan,
}
impl Backend {
    fn log_extension(self) -> &'static str {
        match self {
            Self::Cpu => "cpu.log",
            Self::Metal => "metal.log",
            Self::Vulkan => "vulkan.log",
        }
    }
}
struct Attempt {
    state: BackendState,
    stderr: PathBuf,
}
pub(super) struct Run {
    pub(super) native: PathBuf,
    pub(super) backend: &'static str,
    pub(super) evidence: &'static str,
    pub(super) execution: Option<Value>,
}

pub(super) struct Session {
    runtime: Runtime,
    // Freeze the selected physical device (or confirmed unavailability) per job.
    selection: Option<super::probe::Selection>,
}
impl Session {
    pub(super) fn new(cli: PathBuf) -> Result<Self, String> {
        Ok(Self {
            runtime: Runtime::open(cli)?,
            selection: None,
        })
    }
    pub(super) fn verify(&self) -> Result<(), String> {
        self.runtime.verify()
    }
    pub(super) fn run(
        &mut self,
        model: &Path,
        audio: &Path,
        prefix: &Path,
        work: &Path,
        cancel: &AtomicBool,
        inherited_tree: bool,
    ) -> Result<Run, String> {
        self.verify()?;
        if let Some(probe) = &self.runtime.probe {
            if self.selection.is_none() {
                self.selection = Some(super::probe::select(probe, work, cancel, inherited_tree)?);
            }
            let selection = self.selection.as_ref().ok_or_else(failure)?;
            let (cli, backend, evidence) = match selection.physical_device {
                Some(_) => (
                    self.runtime.vulkan.as_ref().ok_or_else(failure)?,
                    Backend::Vulkan,
                    "vulkan-backend-initialized",
                ),
                None => (&self.runtime.cpu, Backend::Cpu, "confirmed-gpu-unavailable"),
            };
            let attempt = run_attempt(
                cli,
                model,
                audio,
                prefix,
                work,
                cancel,
                backend,
                selection.physical_device,
                inherited_tree,
            )?;
            // A capable probe followed by silent CPU initialization is an error,
            // never a second CPU attempt. This also preserves OOM and driver failures.
            if backend == Backend::Vulkan && attempt.state != BackendState::Initialized {
                return Err(failure());
            }
            self.verify()?;
            return Ok(Run {
                native: prefix.with_extension("json"),
                backend: if backend == Backend::Vulkan {
                    "vulkan"
                } else {
                    "cpu"
                },
                evidence,
                execution: Some(
                    self.runtime
                        .evidence(cli, &attempt.stderr, &selection.raw)?,
                ),
            });
        }
        let state = run_attempt(
            &self.runtime.cpu,
            model,
            audio,
            prefix,
            work,
            cancel,
            Backend::Metal,
            None,
            inherited_tree,
        )?
        .state;
        if state == BackendState::Unavailable {
            let cpu_prefix = prefix.with_file_name(format!(
                "{}-cpu",
                prefix.file_name().ok_or_else(failure)?.to_string_lossy()
            ));
            run_attempt(
                &self.runtime.cpu,
                model,
                audio,
                &cpu_prefix,
                work,
                cancel,
                Backend::Cpu,
                None,
                inherited_tree,
            )?;
            Ok(Run {
                native: cpu_prefix.with_extension("json"),
                backend: "cpu",
                evidence: "confirmed-gpu-unavailable",
                execution: None,
            })
        } else {
            Ok(Run {
                native: prefix.with_extension("json"),
                backend: "metal",
                evidence: "metal-backend-initialized",
                execution: None,
            })
        }
    }
}

fn run_attempt(
    cli: &Path,
    model: &Path,
    audio: &Path,
    prefix: &Path,
    work: &Path,
    cancel: &AtomicBool,
    backend: Backend,
    physical_device: Option<u32>,
    inherited_tree: bool,
) -> Result<Attempt, String> {
    cancelled(cancel)?;
    let log_path = prefix.with_extension(backend.log_extension());
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&log_path)
        .map_err(|e| e.to_string())?;
    // Transcribed speech is untrusted text, never runtime/backend evidence.
    // Keep it separate even when it contains diagnostic-looking words.
    let stdout = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(prefix.with_extension("stdout.log"))
        .map_err(|e| e.to_string())?;
    let mut command = Command::new(cli);
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get().min(4).max(1));
    command
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(audio)
        .arg("-l")
        .arg("ko")
        .args(["-ojf", "-bs", "5", "-bo", "5", "-t"])
        .arg(threads.to_string())
        .arg("-p")
        .arg("1")
        .arg("-of")
        .arg(prefix)
        .current_dir(work)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(log));
    if backend == Backend::Cpu {
        command.arg("-ng");
    }
    // Do not inherit host overrides that can load other backends or change device selection.
    for name in [
        "GGML_BACKEND_PATH",
        "GGML_VK_VISIBLE_DEVICES",
        "GGML_VK_FORCE_MAX_ALLOCATION_SIZE",
        "GGML_VK_DISABLE_F16",
        "WHISPER_ARG_DEVICE",
        "VK_INSTANCE_LAYERS",
        "VK_LAYER_PATH",
        "VK_ADD_LAYER_PATH",
        "VK_ICD_FILENAMES",
        "VK_DRIVER_FILES",
        "VK_ADD_DRIVER_FILES",
    ] {
        command.env_remove(name);
    }
    if let Some(index) = physical_device {
        command
            .env("GGML_VK_VISIBLE_DEVICES", index.to_string())
            .args(["-dev", "0"]);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00004000);
    }
    let mut child = if inherited_tree {
        crate::process_tree::spawn_inherited(&mut command)
    } else {
        crate::process_tree::spawn(&mut command)
    }?;
    let began = Instant::now();
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Whisper 처리를 취소했습니다.".into());
        }
        if began.elapsed() > Duration::from_secs(1800) {
            return Err(
                "Whisper 구간 처리 시간이 너무 오래 걸렸습니다. 원본은 보관했습니다.".into(),
            );
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err("Whisper 구간 처리를 완료하지 못했습니다. 원본은 보관했습니다.".into());
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let log = read(&log_path, 4 * 1024 * 1024)?;
    let log = String::from_utf8_lossy(&log);
    let state = if backend == Backend::Metal {
        // No exit-0/compiled-Metal shortcut: the pinned classifier requires
        // successful device allocation and rejects silent CPU initialization.
        classify_metal(&log)?
    } else if backend == Backend::Vulkan {
        classify_vulkan(&log)?
    } else {
        if native_failure(&log)
            || !log
                .lines()
                .any(|line| line.trim() == "whisper_init_with_params_no_state: use gpu    = 0")
        {
            return Err(failure());
        }
        BackendState::ExplicitCpu
    };
    Ok(Attempt {
        state,
        stderr: log_path,
    })
}

fn native_failure(log: &str) -> bool {
    log.lines().any(|line| {
        let Some((origin, message)) = line.split_once(':') else {
            return false;
        };
        if origin == "error" {
            return true;
        }
        if !(origin.starts_with("whisper_") || origin.starts_with("ggml_"))
            || !origin
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return false;
        }
        let message = message.trim_start().to_ascii_lowercase();
        message.starts_with("failed")
            || message.starts_with("error:")
            || message.contains(" failed ")
            || message.contains("out of memory")
            || message.contains("out of device memory")
            || message.contains("erroroutof")
            || message.starts_with("vk::")
    })
}

#[derive(Debug, PartialEq, Eq)]
enum BackendState {
    Initialized,
    Unavailable,
    ExplicitCpu,
}

fn classify_metal(log: &str) -> Result<BackendState, String> {
    if native_failure(log) {
        return Err(
            "Whisper의 Metal 초기화 또는 메모리 확보에 실패했습니다. 원본은 보관했습니다.".into(),
        );
    }
    if log
        .lines()
        .any(|line| line.trim() == "whisper_backend_init_gpu: no GPU found")
    {
        return Ok(BackendState::Unavailable);
    }
    let model = log
        .lines()
        .any(|line| line.starts_with("whisper_model_load:") && line.contains("MTL0 total size ="));
    let backend = log
        .lines()
        .any(|line| line.trim() == "whisper_backend_init_gpu: using MTL0 backend");
    let device = log
        .lines()
        .any(|line| line.starts_with("ggml_metal_init: found device: Apple "));
    let graph = log
        .lines()
        .any(|line| line.trim() == "ggml_metal_init: use graph optimize = true");
    if model && backend && device && graph {
        Ok(BackendState::Initialized)
    } else {
        Err("Whisper의 실제 Metal 실행 여부를 확인하지 못했습니다. 원본은 보관했습니다.".into())
    }
}

fn classify_vulkan(log: &str) -> Result<BackendState, String> {
    if native_failure(log) {
        return Err(failure());
    }
    // Pinned b5454 diagnostic names after GGML_VK_VISIBLE_DEVICES selects one device.
    let model = log.lines().any(|line| {
        line.starts_with("whisper_model_load:")
            && line.contains("Vulkan0")
            && line.contains("total size =")
    });
    let backend = log
        .lines()
        .any(|line| line.trim() == "whisper_backend_init_gpu: using Vulkan0 backend");
    let compute = log.lines().any(|line| {
        line.trim()
            == "ggml_backend_vk_graph_compute: nero-vulkan-graph-completed-v1 device=Vulkan0"
    });
    if model && backend && compute {
        Ok(BackendState::Initialized)
    } else {
        Err(failure())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vulkan_needs_model_and_compute_buffers_not_a_loader_or_gpu_name() {
        let valid = "whisper_model_load:      Vulkan0 total size = 100.00 MB\nwhisper_backend_init_gpu: using Vulkan0 backend\nggml_backend_vk_graph_compute: nero-vulkan-graph-completed-v1 device=Vulkan0\n";
        assert_eq!(classify_vulkan(valid).unwrap(), BackendState::Initialized);
        for insufficient in [
            "",
            "ggml_vulkan: Found 1 Vulkan devices:\n",
            "whisper_backend_init_gpu: using Vulkan0 backend\n",
            "whisper_backend_init_gpu: no GPU found\n",
        ] {
            assert!(classify_vulkan(insufficient).is_err());
        }
        for error in [
            "ggml_vulkan: out of device memory",
            "whisper_backend_init_gpu: failed to initialize Vulkan0 backend",
            "ggml_vulkan: vk::OutOfDeviceMemoryError",
            "error: failed to allocate context",
        ] {
            assert!(classify_vulkan(&(valid.to_owned() + error)).is_err());
        }
        assert!(
            classify_vulkan(&(valid.to_owned() + "build failed, error: ordinary speech\n")).is_ok()
        );
    }

    #[test]
    fn metal_requires_positive_initialization_and_never_hides_allocation_failure() {
        let valid="whisper_model_load:         MTL0 total size = 100.00 MB\nwhisper_backend_init_gpu: using MTL0 backend\nggml_metal_init: found device: Apple Example\nggml_metal_init: use graph optimize = true\n";
        assert_eq!(classify_metal(valid).unwrap(), BackendState::Initialized);
        assert_eq!(
            classify_metal("whisper_backend_init_gpu: no GPU found\n").unwrap(),
            BackendState::Unavailable
        );
        assert!(classify_metal("whisper_backend_init_gpu: using MTL0 backend\n").is_err());
        for failure in [
            "whisper_backend_init_gpu: failed to initialize MTL0 backend",
            "ggml_metal_buffer: failed to allocate Metal buffer",
            "error: failed to allocate context",
            "ggml_metal_library_init: failed",
        ] {
            assert!(classify_metal(&(valid.to_owned() + failure)).is_err());
        }
        // Even diagnostic-looking ordinary speech is not a native diagnostic.
        // run_attempt separately captures CLI stdout, and parses only stderr.
        assert_eq!(
            classify_metal(&(valid.to_owned() + "build failed, error: deployment failed\n"))
                .unwrap(),
            BackendState::Initialized
        );
        assert_eq!(
            classify_metal(&(valid.to_owned() + "main: processing 'failed-example.wav'\n"))
                .unwrap(),
            BackendState::Initialized
        );
        assert!(classify_metal(
            &(valid.to_owned() + "ggml_metal_graph_compute: command buffer failed with status 5\n")
        )
        .is_err());
    }
}
