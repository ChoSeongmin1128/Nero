use crate::engine::Engine;
use crate::{CoreError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordingGuard {
    AnyRecording,
    // Refused only when a meeting it changes is the one recording.
    SameMeeting,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operation {
    StartRecording,
    ImportAudio,
    AttachToDraft,
    TrashMeetings,
    DiscardRecording,
    CompactRecording,
    ReopenAfterFailedShutdown,
    CommitTranscript,
    EditTranscript,
    CommitTerminology,
    EditSpeakers,
    ApplySpeakerAnalysis,
    RestoreTranscript,
    UpgradeTranscriptSource,
    CommitSummary,
    // An imported summary or a note appended from an answer, checked against the
    // context it was written from.
    SummaryContext,
}

pub(crate) const NEW_DRAFT_ONLY: &str = "음성이 없는 새 회의에서 다시 시도해 주세요.";

impl Operation {
    pub(crate) fn recording_guard(self) -> RecordingGuard {
        match self {
            Self::StartRecording
            | Self::ImportAudio
            | Self::CompactRecording
            | Self::ReopenAfterFailedShutdown
            | Self::RestoreTranscript
            | Self::UpgradeTranscriptSource
            | Self::SummaryContext => RecordingGuard::AnyRecording,
            Self::AttachToDraft
            | Self::TrashMeetings
            | Self::DiscardRecording
            | Self::ApplySpeakerAnalysis => RecordingGuard::SameMeeting,
            Self::CommitTranscript
            | Self::EditTranscript
            | Self::CommitTerminology
            | Self::EditSpeakers
            | Self::CommitSummary => RecordingGuard::None,
        }
    }

    fn refusal(self) -> CoreError {
        let message = match self {
            Self::AttachToDraft => NEW_DRAFT_ONLY,
            Self::TrashMeetings => "녹음을 종료한 뒤 삭제해 주세요.",
            Self::DiscardRecording => "녹음을 종료한 뒤 버려 주세요.",
            Self::CompactRecording => "녹음을 마친 뒤 녹음 파일을 줄여 주세요.",
            Self::ReopenAfterFailedShutdown => "녹음 중에는 종료 준비를 취소할 수 없습니다.",
            _ => return CoreError::RecordingBusy,
        };
        CoreError::InvalidState(message.into())
    }
}

impl Engine {
    // `meetings` are the meetings the operation changes; only a same-meeting guard reads them.
    pub(crate) fn refuse_while_recording<'a>(
        &self,
        operation: Operation,
        meetings: impl IntoIterator<Item = &'a str>,
    ) -> Result<()> {
        let Some(capture) = &self.capture else {
            return Ok(());
        };
        let refused = match operation.recording_guard() {
            RecordingGuard::AnyRecording => true,
            RecordingGuard::SameMeeting => meetings
                .into_iter()
                .any(|id| id == capture.manifest.meeting_id),
            RecordingGuard::None => false,
        };
        if refused {
            Err(operation.refusal())
        } else {
            Ok(())
        }
    }
}
