// Which work may run while a meeting is recorded. The queue asks before a job starts,
// a job asks again before it reads its meeting, and a recording start asks whether the
// running job may stay. See docs/task-management.md.
use crate::job_journal::JobKind;
use crate::speech_choice::SpeechEngine;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Session<'a> {
    Idle,
    // The live worker of `meeting`'s recording is finishing after the recording ended.
    Finishing(&'a str),
    // Its meeting is not known while it starts. `live`: a live worker (a legacy preview or
    // the recording-time MOSS processor) reads it and keeps the inference slot.
    Recording {
        meeting: Option<&'a str>,
        live: bool,
    },
}

impl<'a> Session<'a> {
    // A recording as a job sees it before reading its meeting; the queue has already
    // kept the job back while a live worker held the slot.
    pub(crate) fn recording(meeting: Option<&'a str>) -> Self {
        meeting.map_or(Self::Idle, |meeting| Self::Recording {
            meeting: Some(meeting),
            live: false,
        })
    }
}

// Recognition of a finished meeting, or its speaker analysis, that keeps running while
// another meeting records. `engine` decides only for a transcription.
pub(crate) fn runs_beside_recording(kind: JobKind, engine: Option<SpeechEngine>) -> bool {
    crate::job_control::SUPPORTED
        && match kind {
            JobKind::Transcribe => engine.is_some_and(SpeechEngine::runs_beside_recording),
            JobKind::Diarize => true,
            _ => false,
        }
}

// A live worker keeps the slot: nothing else starts, though a download already running
// keeps going.
pub(crate) fn admits(
    kind: JobKind,
    engine: Option<SpeechEngine>,
    meeting: Option<&str>,
    session: Session<'_>,
) -> bool {
    match session {
        Session::Idle => true,
        Session::Finishing(live) => kind == JobKind::Transcribe && meeting == Some(live),
        Session::Recording { live: true, .. } => false,
        Session::Recording {
            meeting: recording,
            live: false,
        } => {
            kind.prepares_model()
                || runs_beside_recording(kind, engine)
                    && recording.is_none_or(|recording| meeting != Some(recording))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_beside_a_recording_follows_kind_engine_and_session() {
        let beside = crate::job_control::SUPPORTED;
        let other = Session::Recording {
            meeting: Some("recording"),
            live: false,
        };
        let live = Session::Recording {
            meeting: Some("recording"),
            live: true,
        };
        let cases = [
            (JobKind::Transcribe, Some(SpeechEngine::Moss), beside),
            (JobKind::Transcribe, Some(SpeechEngine::QwenKo), beside),
            (JobKind::Transcribe, Some(SpeechEngine::AppleSpeech), false),
            (JobKind::Transcribe, Some(SpeechEngine::Nemotron), false),
            (JobKind::Transcribe, Some(SpeechEngine::WhisperTurbo), false),
            (
                JobKind::Transcribe,
                Some(SpeechEngine::WhisperLargeV3),
                false,
            ),
            (JobKind::Transcribe, None, false),
            (JobKind::Diarize, None, beside),
            (JobKind::Diarize, Some(SpeechEngine::Nemotron), beside),
            (JobKind::Summary, Some(SpeechEngine::Moss), false),
            (JobKind::Import, Some(SpeechEngine::Moss), false),
            (JobKind::Setup, None, true),
            (JobKind::Model, None, true),
            (JobKind::SpeakerModel, None, true),
            (JobKind::SummaryModel, None, true),
        ];
        for (kind, engine, expected) in cases {
            assert!(admits(kind, engine, Some("meeting"), Session::Idle));
            assert_eq!(
                admits(kind, engine, Some("meeting"), other),
                expected,
                "{kind:?} {engine:?}"
            );
            // A recording whose meeting is not known yet counts as another meeting's.
            assert_eq!(
                admits(
                    kind,
                    engine,
                    Some("meeting"),
                    Session::Recording {
                        meeting: None,
                        live: false
                    }
                ),
                expected
            );
            assert!(!admits(kind, engine, Some("meeting"), live), "{kind:?}");
            assert_eq!(
                admits(kind, engine, Some("recording"), other),
                kind.prepares_model(),
                "{kind:?} {engine:?}"
            );
        }
    }

    #[test]
    fn a_finishing_live_worker_admits_only_its_own_meetings_transcription() {
        let finishing = Session::Finishing("recorded");
        let moss = Some(SpeechEngine::Moss);
        assert!(admits(
            JobKind::Transcribe,
            moss,
            Some("recorded"),
            finishing
        ));
        assert!(!admits(JobKind::Transcribe, moss, Some("other"), finishing));
        assert!(!admits(JobKind::Diarize, moss, Some("recorded"), finishing));
        assert!(!admits(JobKind::Setup, None, None, finishing));
    }

    #[test]
    fn a_job_checking_its_meeting_sees_only_the_recording() {
        assert_eq!(Session::recording(None), Session::Idle);
        assert_eq!(
            Session::recording(Some("recording")),
            Session::Recording {
                meeting: Some("recording"),
                live: false
            }
        );
    }
}
