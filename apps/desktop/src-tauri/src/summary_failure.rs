use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FailureKind {
    Cancelled,
    Allocation,
    BackendUnavailable,
    EngineUnavailable,
    Protocol,
    Input,
    InputLimit,
    ModelNotReady,
    MeetingUnavailable,
    RecordingActive,
    SourceChanged,
    Output,
    OutputLimit,
    Storage,
    Cleanup,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailureDetail {
    pub stage: String,
    pub finish_reason: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_limit: Option<u32>,
    pub generated_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Failure {
    pub kind: FailureKind,
    pub message: String,
    pub detail: Option<FailureDetail>,
}

impl Failure {
    pub fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            detail: None,
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Failure {}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::new(FailureKind::Output, message)
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self::new(FailureKind::Output, message)
    }
}

fn number_between(line: &str, prefix: &str, suffix: &str) -> bool {
    let Some(number) = line
        .strip_prefix(prefix)
        .and_then(|value| value.strip_suffix(suffix))
    else {
        return false;
    };
    let number = number.trim();
    !number.is_empty()
        && number
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
        && number
            .parse::<f64>()
            .is_ok_and(|value| value.is_finite() && value >= 0.0)
}

fn buffer_allocation(line: &str, prefix: &str) -> bool {
    let Some((backend, bytes)) = line
        .strip_prefix(prefix)
        .and_then(|value| value.split_once(" buffer of size "))
    else {
        return false;
    };
    !backend.is_empty()
        && backend
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        && !bytes.is_empty()
        && bytes.bytes().all(|byte| byte.is_ascii_digit())
}

fn vulkan_result(line: &str) -> Option<&str> {
    let (result, location) = line
        .strip_prefix("ggml_vulkan: error ")?
        .split_once(" at ")?;
    let (file, line) = location.rsplit_once(':')?;
    if !file.ends_with("ggml-vulkan.cpp") || line.parse::<u32>().ok()? == 0 {
        return None;
    }
    Some(result)
}

fn typed_server_allocation(line: &str) -> bool {
    let Some((function, exception)) = line
        .strip_prefix("srv ")
        .and_then(|value| value.trim_start().split_once(": got exception: "))
    else {
        return false;
    };
    !function.is_empty()
        && function
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && exception == "std::bad_alloc"
}

fn allocation(line: &str) -> bool {
    matches!(
        line,
        "GGML status: error (failed to allocate memory)"
            | "ggml_metal_host_malloc: error: vm_allocate failed"
    ) || number_between(line, "ggml_malloc: failed to allocate ", " MB")
        || number_between(line, "ggml_calloc: failed to allocate ", " MB")
        || number_between(
            line,
            "ggml_backend_metal_buffer_type_alloc_buffer: failed to allocate Metal buffer of ",
            " bytes (out of memory)",
        )
        || number_between(
            line,
            "ggml_metal_buffer_init: error: failed to allocate buffer, size = ",
            " MiB",
        )
        || number_between(
            line,
            "ggml_backend_cpu_buffer_type_alloc_buffer: failed to allocate buffer of size ",
            "",
        )
        || buffer_allocation(line, "ggml_gallocr_reserve_n_impl: failed to allocate ")
        || buffer_allocation(
            line,
            "ggml_backend_buft_alloc_buffer_n_default: failed to allocate ",
        )
        || matches!(
            vulkan_result(line),
            Some("ErrorOutOfHostMemory" | "ErrorOutOfDeviceMemory")
        )
        || typed_server_allocation(line)
}

fn backend_unavailable(line: &str) -> bool {
    matches!(
        line,
        "ggml_vulkan: Error: Vulkan 1.2 required."
            | "ggml_metal_device_init: error: failed to create library"
            | "ggml_metal_device_init: error: failed to create command queue"
            | "ggml_metal_library_init_from_source: failed to create Metal library from source"
    ) || matches!(
        vulkan_result(line),
        Some("ErrorInitializationFailed" | "ErrorIncompatibleDriver")
    )
}

pub fn classify_worker_failure(log: &str, gpu: bool) -> FailureKind {
    // Match pinned b11734 runtime diagnostics, never free-form memory warnings.
    if log.lines().map(str::trim).any(allocation) {
        FailureKind::Allocation
    } else if gpu && log.lines().map(str::trim).any(backend_unavailable) {
        FailureKind::BackendUnavailable
    } else {
        FailureKind::EngineUnavailable
    }
}

pub fn recovery(kind: FailureKind, gpu: bool, used: bool) -> Option<bool> {
    if used {
        return None;
    }
    match kind {
        FailureKind::Allocation | FailureKind::EngineUnavailable => Some(gpu),
        FailureKind::BackendUnavailable if gpu => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_runtime_allocation_diagnostics_are_distinct_from_pressure() {
        for diagnostic in [
            "ggml_malloc: failed to allocate   4.00 MB",
            "ggml_calloc: failed to allocate   8.00 MB",
            "ggml_gallocr_reserve_n_impl: failed to allocate Metal buffer of size 4096",
            "ggml_backend_buft_alloc_buffer_n_default: failed to allocate CPU buffer of size 8192",
            "ggml_backend_metal_buffer_type_alloc_buffer: failed to allocate Metal buffer of 4096 bytes (out of memory)",
            "ggml_metal_buffer_init: error: failed to allocate buffer, size =    32.00 MiB",
            "ggml_metal_host_malloc: error: vm_allocate failed",
            "ggml_backend_cpu_buffer_type_alloc_buffer: failed to allocate buffer of size 4096",
            "GGML status: error (failed to allocate memory)",
            "ggml_vulkan: error ErrorOutOfDeviceMemory at ggml/src/ggml-vulkan/ggml-vulkan.cpp:362",
            "ggml_vulkan: error ErrorOutOfHostMemory at D:\\runtime\\ggml-vulkan.cpp:362",
            "srv  update_slots: got exception: std::bad_alloc",
        ] {
            assert_eq!(classify_worker_failure(diagnostic, true), FailureKind::Allocation, "{diagnostic}");
        }
    }

    #[test]
    fn model_text_os_pressure_and_unattributed_death_do_not_prove_allocation_failure() {
        for diagnostic in [
            "out of memory",
            "low memory",
            "memory warning",
            "SIGKILL",
            "terminated by signal 9",
            "std::bad_alloc",
            "VK_ERROR_OUT_OF_DEVICE_MEMORY",
            "Assistant: the result is out of memory",
            "model output: ggml_malloc: failed to allocate 4.00 MB",
            "ggml_vulkan: error ErrorOutOfDeviceMemory at meeting.txt:1",
            "ggml_malloc: failed to allocate a little MB",
            "ggml_malloc: failed to allocate NaN MB",
            "ggml_malloc: failed to allocate 4.00 MB; retry please",
        ] {
            assert_eq!(
                classify_worker_failure(diagnostic, true),
                FailureKind::EngineUnavailable,
                "{diagnostic}"
            );
        }
    }

    #[test]
    fn confirmed_gpu_initialization_failures_allow_only_gpu_to_cpu_recovery() {
        for diagnostic in [
            "ggml_vulkan: Error: Vulkan 1.2 required.",
            "ggml_metal_device_init: error: failed to create library",
            "ggml_metal_device_init: error: failed to create command queue",
            "ggml_metal_library_init_from_source: failed to create Metal library from source",
            "ggml_vulkan: error ErrorIncompatibleDriver at ggml-vulkan.cpp:362",
        ] {
            assert_eq!(
                classify_worker_failure(diagnostic, true),
                FailureKind::BackendUnavailable
            );
            assert_eq!(
                classify_worker_failure(diagnostic, false),
                FailureKind::EngineUnavailable
            );
        }
        assert_eq!(
            classify_worker_failure("GPU is slow or unsupported", true),
            FailureKind::EngineUnavailable
        );
    }

    #[test]
    fn concrete_allocation_failure_is_not_hidden_by_backend_initialization_failure() {
        let log = "ggml_metal_device_init: error: failed to create library\nggml_malloc: failed to allocate 4.00 MB";
        assert_eq!(classify_worker_failure(log, true), FailureKind::Allocation);
    }

    #[test]
    fn retry_budget_is_shared_by_backend_fallback_and_same_backend_recovery() {
        for gpu in [false, true] {
            for kind in [FailureKind::Allocation, FailureKind::EngineUnavailable] {
                assert_eq!(recovery(kind, gpu, false), Some(gpu));
                assert_eq!(recovery(kind, gpu, true), None);
            }
        }
        assert_eq!(
            recovery(FailureKind::BackendUnavailable, true, false),
            Some(false)
        );
        assert_eq!(
            recovery(FailureKind::BackendUnavailable, false, false),
            None
        );
        assert_eq!(recovery(FailureKind::BackendUnavailable, true, true), None);
    }

    #[test]
    fn user_actions_invalid_results_and_persistence_failures_are_never_retried() {
        for kind in [
            FailureKind::Cancelled,
            FailureKind::Protocol,
            FailureKind::Input,
            FailureKind::InputLimit,
            FailureKind::ModelNotReady,
            FailureKind::MeetingUnavailable,
            FailureKind::RecordingActive,
            FailureKind::SourceChanged,
            FailureKind::Output,
            FailureKind::OutputLimit,
            FailureKind::Storage,
            FailureKind::Cleanup,
        ] {
            for gpu in [false, true] {
                assert_eq!(recovery(kind, gpu, false), None);
                assert_eq!(recovery(kind, gpu, true), None);
            }
        }
        assert_eq!(Failure::from("validation failed").kind, FailureKind::Output);
    }
}
