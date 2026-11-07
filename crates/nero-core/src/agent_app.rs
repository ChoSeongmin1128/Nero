// Requests an external agent sends to the running app's own services: AI chat, Notion
// sources, calendar, terminology, recording, transcript edits and settings. Only the
// shape is checked here, so a CLI can refuse a malformed request before sending it;
// the app decides everything else.
use crate::agent_control::ControlError;
use crate::{store, TranscriptVersion};
use serde::Deserialize;
use serde_json::{json, Value};

pub const METHODS: [&str; 37] = [
    "ai_chats_list",
    "ai_chat_get",
    "ai_models_list",
    "ai_chat_send",
    "ai_chat_cancel",
    "ai_chat_version_select",
    "ai_answer_save",
    "ai_chats_trash",
    "ai_chats_restore",
    "notion_status",
    "meeting_sources_list",
    "meeting_source_add",
    "meeting_source_refresh",
    "meeting_source_remove",
    "calendar_status",
    "calendar_events",
    "calendars_select",
    "calendar_sync",
    "calendar_attendees",
    "meeting_from_calendar",
    "terminology_get",
    "glossary_save",
    "settings_get",
    "settings_set",
    "recording_devices",
    "recording_control",
    "meeting_create",
    "meeting_notes_append",
    "transcription_start",
    "speakers_analyze",
    "models_prepare",
    "transcript_history",
    "transcript_save",
    "transcript_restore",
    "speakers_save",
    "job_retry",
    "job_acknowledge",
];

// Codes the app answers these requests with. Any other code reaches the agent as
// request_failed, and invalid_input means the app predates the request.
pub const ERRORS: [&str; 11] = [
    "not_found",
    "meeting_not_found",
    "edit_conflict",
    "invalid_state",
    "recording_active",
    "account_unavailable",
    "notion_unavailable",
    "speech_not_ready",
    "request_rejected",
    "app_not_ready",
    "storage_error",
];

const MAX_TEXT: usize = 32 * 1024;
const MAX_ITEMS: usize = 400;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AnswerTarget {
    Summary,
    Notes,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecordingAction {
    Start,
    Pause,
    Resume,
    Stop,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SegmentText {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpeakerName {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpeakerAssignment {
    pub segment_id: String,
    pub speaker_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlossaryEntry {
    pub term: String,
    pub source_id: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(
    tag = "method",
    content = "params",
    deny_unknown_fields,
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AppRequest {
    AiChatsList {
        #[serde(default)]
        trashed: bool,
        meeting_id: Option<String>,
    },
    AiChatGet {
        conversation_id: String,
    },
    AiModelsList {
        #[serde(default)]
        refresh: bool,
    },
    AiChatSend {
        conversation_id: Option<String>,
        #[serde(default)]
        message: String,
        #[serde(default)]
        meeting_ids: Vec<String>,
        #[serde(default)]
        source_ids: Vec<String>,
        account_id: String,
        model_id: String,
        effort: Option<String>,
        skill_id: Option<String>,
        template_id: Option<String>,
        edit_of: Option<String>,
    },
    AiChatCancel {
        conversation_id: String,
        request_id: String,
    },
    AiChatVersionSelect {
        conversation_id: String,
        message_id: String,
    },
    AiAnswerSave {
        conversation_id: String,
        message_id: String,
        meeting_id: String,
        target: AnswerTarget,
    },
    AiChatsTrash {
        #[serde(default)]
        conversation_ids: Vec<String>,
        older_than_days: Option<u32>,
    },
    AiChatsRestore {
        conversation_ids: Vec<String>,
    },
    NotionStatus {},
    MeetingSourcesList {
        meeting_id: String,
    },
    MeetingSourceAdd {
        meeting_id: String,
        url: String,
    },
    MeetingSourceRefresh {
        meeting_id: String,
        page_id: String,
    },
    MeetingSourceRemove {
        meeting_id: String,
        page_id: String,
    },
    CalendarStatus {},
    CalendarEvents {
        day: String,
        timezone: String,
    },
    CalendarsSelect {
        calendar_ids: Vec<String>,
    },
    CalendarSync {},
    CalendarAttendees {
        meeting_id: String,
    },
    MeetingFromCalendar {
        occurrence_key: String,
        day: String,
        timezone: String,
        scope_revision: u64,
        meeting_id: Option<String>,
        expected_version: Option<String>,
    },
    TerminologyGet {
        meeting_id: Option<String>,
    },
    GlossarySave {
        meeting_id: String,
        expected_revision: u64,
        entries: Vec<GlossaryEntry>,
    },
    SettingsGet {},
    SettingsSet {
        speaker_analysis: Option<bool>,
        terminology: Option<bool>,
    },
    RecordingDevices {},
    RecordingControl {
        action: RecordingAction,
        meeting_id: Option<String>,
        device_id: Option<String>,
        title: Option<String>,
        // pause, resume or stop only: refused when another recording is active.
        #[serde(default)]
        session_id: Option<String>,
    },
    MeetingCreate {
        #[serde(default)]
        title: String,
        #[serde(default)]
        notes: String,
    },
    MeetingNotesAppend {
        meeting_id: String,
        expected_version: String,
        text: String,
    },
    TranscriptionStart {
        meeting_id: String,
    },
    SpeakersAnalyze {
        meeting_id: String,
        expected_version: TranscriptVersion,
    },
    ModelsPrepare {},
    TranscriptHistory {
        meeting_id: String,
    },
    TranscriptSave {
        meeting_id: String,
        expected_version: TranscriptVersion,
        segments: Vec<SegmentText>,
    },
    TranscriptRestore {
        meeting_id: String,
        expected_version: TranscriptVersion,
        target: TranscriptVersion,
    },
    SpeakersSave {
        meeting_id: String,
        expected_version: TranscriptVersion,
        speakers: Vec<SpeakerName>,
        #[serde(default)]
        assignments: Vec<SpeakerAssignment>,
    },
    JobRetry {
        job_id: String,
        expected_version: String,
    },
    JobAcknowledge {
        job_id: String,
        expected_version: String,
    },
}

fn id(value: &str) -> bool {
    store::validate_id(value).is_ok()
}
fn ids(values: &[String]) -> bool {
    values.len() <= MAX_ITEMS && values.iter().all(|value| id(value))
}
fn text(value: &str, limit: usize) -> bool {
    value.len() <= limit && !value.contains('\0')
}
fn name(value: &str) -> bool {
    !value.trim().is_empty() && text(value, 256) && !value.chars().any(char::is_control)
}
fn token(value: &str) -> bool {
    (1..=256).contains(&value.len())
        && value.bytes().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b':' | b'@' | b'[' | b']')
        })
}
use crate::edit_version::valid as version;
fn transcript_version(value: &TranscriptVersion) -> bool {
    id(&value.id)
}
fn calendar_day(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}
fn zone(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'/' | b'_' | b'-' | b'+'))
}
fn notion_url(value: &str) -> bool {
    text(value, 2048)
        && ["https://www.notion.so/", "https://notion.so/"]
            .iter()
            .any(|prefix| value.starts_with(prefix))
}

impl AppRequest {
    pub fn parse(method: &str, params: Value) -> Result<Self, ControlError> {
        if !METHODS.contains(&method) || !params.is_object() {
            return Err(ControlError::invalid());
        }
        let request: Self = serde_json::from_value(json!({"method":method,"params":params}))
            .map_err(|_| ControlError::invalid())?;
        if request.valid() {
            Ok(request)
        } else {
            Err(ControlError::invalid())
        }
    }

    fn valid(&self) -> bool {
        match self {
            Self::AiChatsList { meeting_id, .. } => meeting_id.as_deref().is_none_or(id),
            Self::AiChatGet { conversation_id } => id(conversation_id),
            Self::AiModelsList { .. }
            | Self::NotionStatus {}
            | Self::CalendarStatus {}
            | Self::CalendarSync {}
            | Self::SettingsGet {}
            | Self::RecordingDevices {}
            | Self::ModelsPrepare {} => true,
            Self::AiChatSend {
                conversation_id,
                message,
                meeting_ids,
                source_ids,
                account_id,
                model_id,
                effort,
                skill_id,
                template_id,
                edit_of,
            } => {
                conversation_id.as_deref().is_none_or(id)
                    && text(message, MAX_TEXT)
                    && (!message.trim().is_empty() || skill_id.is_some())
                    && meeting_ids.len() <= 16
                    && ids(meeting_ids)
                    && source_ids.len() <= 16
                    && source_ids.iter().all(|value| token(value))
                    && token(account_id)
                    && token(model_id)
                    && effort.as_deref().is_none_or(token)
                    && skill_id.as_deref().is_none_or(token)
                    && template_id.as_deref().is_none_or(token)
                    && edit_of.as_deref().is_none_or(token)
                    && (edit_of.is_none() || conversation_id.is_some())
            }
            Self::AiChatCancel {
                conversation_id,
                request_id,
            } => id(conversation_id) && id(request_id),
            Self::AiChatVersionSelect {
                conversation_id,
                message_id,
            } => id(conversation_id) && token(message_id),
            Self::AiAnswerSave {
                conversation_id,
                message_id,
                meeting_id,
                ..
            } => id(conversation_id) && token(message_id) && id(meeting_id),
            Self::AiChatsTrash {
                conversation_ids,
                older_than_days,
            } => match older_than_days {
                Some(days) => conversation_ids.is_empty() && (1..=3650).contains(days),
                None => !conversation_ids.is_empty() && ids(conversation_ids),
            },
            Self::AiChatsRestore { conversation_ids } => {
                !conversation_ids.is_empty() && ids(conversation_ids)
            }
            Self::MeetingSourcesList { meeting_id }
            | Self::CalendarAttendees { meeting_id }
            | Self::TranscriptionStart { meeting_id }
            | Self::TranscriptHistory { meeting_id } => id(meeting_id),
            Self::SpeakersAnalyze {
                meeting_id,
                expected_version,
            } => id(meeting_id) && transcript_version(expected_version),
            Self::TerminologyGet { meeting_id } => meeting_id.as_deref().is_none_or(id),
            Self::MeetingSourceAdd { meeting_id, url } => id(meeting_id) && notion_url(url),
            Self::MeetingSourceRefresh {
                meeting_id,
                page_id,
            }
            | Self::MeetingSourceRemove {
                meeting_id,
                page_id,
            } => id(meeting_id) && token(page_id),
            Self::CalendarEvents { day, timezone } => calendar_day(day) && zone(timezone),
            Self::CalendarsSelect { calendar_ids } => {
                calendar_ids.len() <= 100
                    && calendar_ids
                        .iter()
                        .all(|value| text(value, 1024) && !value.is_empty())
            }
            Self::MeetingFromCalendar {
                occurrence_key,
                day,
                timezone,
                meeting_id,
                expected_version,
                ..
            } => {
                text(occurrence_key, 1024)
                    && !occurrence_key.is_empty()
                    && calendar_day(day)
                    && zone(timezone)
                    && match (meeting_id, expected_version) {
                        (None, None) => true,
                        (Some(meeting), Some(expected)) => id(meeting) && version(expected),
                        _ => false,
                    }
            }
            Self::GlossarySave {
                meeting_id,
                entries,
                ..
            } => {
                id(meeting_id)
                    && entries.len() <= 200
                    && entries.iter().all(|entry| {
                        name(&entry.term)
                            && entry.term.chars().count() <= 100
                            && token(&entry.source_id)
                    })
            }
            Self::SettingsSet {
                speaker_analysis,
                terminology,
            } => speaker_analysis.is_some() || terminology.is_some(),
            Self::RecordingControl {
                action,
                meeting_id,
                device_id,
                title,
                session_id,
            } => {
                session_id.as_deref().is_none_or(id)
                    && !(*action == RecordingAction::Start && session_id.is_some())
                    && meeting_id.as_deref().is_none_or(id)
                    && device_id
                        .as_deref()
                        .is_none_or(|value| text(value, 1024) && !value.is_empty())
                    && title.as_deref().is_none_or(name)
                    && (*action == RecordingAction::Start
                        || meeting_id.is_none() && device_id.is_none() && title.is_none())
                    && !(meeting_id.is_some() && title.is_some())
            }
            Self::MeetingCreate { title, notes } => {
                (title.is_empty() || name(title)) && text(notes, MAX_TEXT * 8)
            }
            Self::MeetingNotesAppend {
                meeting_id,
                expected_version,
                text: addition,
            } => {
                id(meeting_id)
                    && version(expected_version)
                    && !addition.trim().is_empty()
                    && text(addition, MAX_TEXT * 8)
            }
            Self::TranscriptSave {
                meeting_id,
                expected_version,
                segments,
            } => {
                id(meeting_id)
                    && transcript_version(expected_version)
                    && !segments.is_empty()
                    && segments.len() <= 20_000
                    && segments
                        .iter()
                        .all(|segment| id(&segment.id) && text(&segment.text, MAX_TEXT))
            }
            Self::TranscriptRestore {
                meeting_id,
                expected_version,
                target,
            } => {
                id(meeting_id) && transcript_version(expected_version) && transcript_version(target)
            }
            Self::SpeakersSave {
                meeting_id,
                expected_version,
                speakers,
                assignments,
            } => {
                id(meeting_id)
                    && transcript_version(expected_version)
                    && speakers.len() <= 64
                    && speakers
                        .iter()
                        .all(|speaker| token(&speaker.id) && name(&speaker.name))
                    && assignments.len() <= 20_000
                    && assignments.iter().all(|assignment| {
                        id(&assignment.segment_id)
                            && assignment.speaker_id.as_deref().is_none_or(token)
                    })
            }
            Self::JobRetry {
                job_id,
                expected_version,
            }
            | Self::JobAcknowledge {
                job_id,
                expected_version,
            } => id(job_id) && version(expected_version),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_method_parses_its_documented_shape_and_rejects_extra_fields() {
        let meeting = "00000000-0000-4000-8000-000000000001";
        let version = format!("sha256:{}", "a".repeat(64));
        let transcript = json!({"id":meeting,"revision":1});
        let samples = [
            ("ai_chats_list", json!({"trashed":true})),
            ("ai_chat_get", json!({"conversationId":meeting})),
            ("ai_models_list", json!({})),
            (
                "ai_chat_send",
                json!({"message":"결정만","meetingIds":[meeting],"accountId":"claude-code","modelId":"sonnet","effort":"medium"}),
            ),
            (
                "ai_chat_cancel",
                json!({"conversationId":meeting,"requestId":meeting}),
            ),
            (
                "ai_chat_version_select",
                json!({"conversationId":meeting,"messageId":"user:x"}),
            ),
            (
                "ai_answer_save",
                json!({"conversationId":meeting,"messageId":"m","meetingId":meeting,"target":"notes"}),
            ),
            ("ai_chats_trash", json!({"olderThanDays":30})),
            ("ai_chats_restore", json!({"conversationIds":[meeting]})),
            ("notion_status", json!({})),
            ("meeting_sources_list", json!({"meetingId":meeting})),
            (
                "meeting_source_add",
                json!({"meetingId":meeting,"url":"https://www.notion.so/page"}),
            ),
            (
                "meeting_source_refresh",
                json!({"meetingId":meeting,"pageId":"page"}),
            ),
            (
                "meeting_source_remove",
                json!({"meetingId":meeting,"pageId":"page"}),
            ),
            ("calendar_status", json!({})),
            (
                "calendar_events",
                json!({"day":"2026-11-05","timezone":"Asia/Seoul"}),
            ),
            ("calendars_select", json!({"calendarIds":["primary"]})),
            ("calendar_sync", json!({})),
            ("calendar_attendees", json!({"meetingId":meeting})),
            (
                "meeting_from_calendar",
                json!({"occurrenceKey":"google:abc","day":"2026-11-05","timezone":"Asia/Seoul","scopeRevision":3}),
            ),
            ("terminology_get", json!({"meetingId":meeting})),
            (
                "glossary_save",
                json!({"meetingId":meeting,"expectedRevision":2,"entries":[{"term":"Nemotron","sourceId":"notion:x:y"}]}),
            ),
            ("settings_get", json!({})),
            ("settings_set", json!({"speakerAnalysis":false})),
            ("recording_devices", json!({})),
            (
                "recording_control",
                json!({"action":"start","title":"주간 회의"}),
            ),
            ("meeting_create", json!({"title":"빈 회의"})),
            (
                "meeting_notes_append",
                json!({"meetingId":meeting,"expectedVersion":version,"text":"추가"}),
            ),
            ("transcription_start", json!({"meetingId":meeting})),
            (
                "speakers_analyze",
                json!({"meetingId":meeting,"expectedVersion":transcript}),
            ),
            ("models_prepare", json!({})),
            ("transcript_history", json!({"meetingId":meeting})),
            (
                "transcript_save",
                json!({"meetingId":meeting,"expectedVersion":transcript,"segments":[{"id":meeting,"text":"고친 문장"}]}),
            ),
            (
                "transcript_restore",
                json!({"meetingId":meeting,"expectedVersion":transcript,"target":transcript}),
            ),
            (
                "speakers_save",
                json!({"meetingId":meeting,"expectedVersion":transcript,"speakers":[{"id":"speaker-1","name":"김철수"}],"assignments":[{"segmentId":meeting,"speakerId":null}]}),
            ),
            (
                "job_retry",
                json!({"jobId":meeting,"expectedVersion":version}),
            ),
            (
                "job_acknowledge",
                json!({"jobId":meeting,"expectedVersion":version}),
            ),
        ];
        assert_eq!(samples.len(), METHODS.len());
        assert!(AppRequest::parse(
            "recording_control",
            json!({"action":"stop","sessionId":meeting})
        )
        .is_ok());
        for (method, params) in samples {
            assert!(
                AppRequest::parse(method, params.clone()).is_ok(),
                "{method}"
            );
            let mut extra = params;
            extra["unexpected"] = json!(true);
            assert!(
                AppRequest::parse(method, extra).is_err(),
                "{method} accepted an extra field"
            );
        }
    }

    #[test]
    fn requests_that_name_nothing_or_mix_targets_are_rejected() {
        let meeting = "00000000-0000-4000-8000-000000000001";
        for (method, params) in [
            (
                "ai_chat_send",
                json!({"message":"  ","accountId":"a","modelId":"m"}),
            ),
            (
                "ai_chat_send",
                json!({"message":"질문","accountId":"a","modelId":"m","editOf":"x"}),
            ),
            ("ai_chats_trash", json!({})),
            (
                "ai_chats_trash",
                json!({"conversationIds":[meeting],"olderThanDays":10}),
            ),
            ("settings_set", json!({})),
            (
                "recording_control",
                json!({"action":"stop","meetingId":meeting}),
            ),
            (
                "recording_control",
                json!({"action":"start","meetingId":meeting,"title":"x"}),
            ),
            (
                "recording_control",
                json!({"action":"start","sessionId":meeting}),
            ),
            (
                "meeting_source_add",
                json!({"meetingId":meeting,"url":"https://example.com/page"}),
            ),
            (
                "calendar_events",
                json!({"day":"2026-11-5","timezone":"Asia/Seoul"}),
            ),
            (
                "meeting_from_calendar",
                json!({"occurrenceKey":"k","day":"2026-11-05","timezone":"Asia/Seoul","scopeRevision":1,"meetingId":meeting}),
            ),
            ("settings_set", json!({"autoRecorder":true})),
            (
                "meeting_notes_append",
                json!({"meetingId":meeting,"expectedVersion":"v","text":"x"}),
            ),
            ("meetings_purge", json!({"meetingIds":[meeting]})),
        ] {
            assert!(AppRequest::parse(method, params).is_err(), "{method}");
        }
    }
}
