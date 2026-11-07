//! Typed processing failures shared by recognition, speaker analysis, audio preparation,
//! terminology and summaries. The kind reaches the UI; the message is already user-facing.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

// What a Core error means for processing. A caller decides what an unmapped error
// means, and may read a variant differently where its operation gives it another sense.
pub fn core_failure_kind(error: &nero_core::CoreError) -> Option<FailureKind> {
    use nero_core::CoreError;
    match error {
        CoreError::MeetingNotFound => Some(FailureKind::MeetingUnavailable),
        CoreError::InvalidState(_) => Some(FailureKind::SourceChanged),
        CoreError::InvalidInput(_) => Some(FailureKind::Output),
        CoreError::RecordingBusy => Some(FailureKind::RecordingActive),
        CoreError::Storage(_) => Some(FailureKind::Storage),
        CoreError::Unavailable => Some(FailureKind::EngineUnavailable),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nero_core::CoreError;

    #[test]
    fn core_errors_map_to_one_failure_meaning() {
        for (error, kind) in [
            (
                CoreError::MeetingNotFound,
                Some(FailureKind::MeetingUnavailable),
            ),
            (
                CoreError::InvalidState("changed".into()),
                Some(FailureKind::SourceChanged),
            ),
            (
                CoreError::InvalidInput("rejected".into()),
                Some(FailureKind::Output),
            ),
            (CoreError::RecordingBusy, Some(FailureKind::RecordingActive)),
            (
                CoreError::Storage("disk".into()),
                Some(FailureKind::Storage),
            ),
            (CoreError::Unavailable, Some(FailureKind::EngineUnavailable)),
            (CoreError::EditConflict, None),
            (CoreError::StaleSession, None),
            (CoreError::StoreBusy, None),
        ] {
            assert_eq!(core_failure_kind(&error), kind, "{error}");
        }
    }

    #[test]
    fn plain_messages_become_output_failures() {
        assert_eq!(Failure::from("validation failed").kind, FailureKind::Output);
        assert_eq!(
            Failure::from(String::from("validation failed")).kind,
            FailureKind::Output
        );
    }
}
