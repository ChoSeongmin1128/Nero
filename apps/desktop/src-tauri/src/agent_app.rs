// Answers nero_core::agent_app requests with the operations the app's own screens use.
// Writes that read or change a meeting first save open editors through the agent write
// lease, and every change tells the open screens to refresh what they show.
use crate::ai_chat_desktop::{self, AiDesktop};
use crate::meeting_context_desktop::{self, SourceChange};
use crate::{calendar_desktop, domain_glossary, terminology, DesktopState};
use nero_core::agent_app::{AnswerTarget, AppRequest, RecordingAction};
use nero_core::agent_control::{self, ControlError};
use nero_core::{CoreError, TranscriptSpeaker};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

// Handlers never answer `invalid_input`: the CLI reads that code from an app request as
// an app that predates it.
fn rejected(message: impl Into<String>) -> ControlError {
    ControlError::new("request_rejected", message)
}
fn core_error(error: CoreError) -> ControlError {
    let message = error.to_string();
    let mapped = ControlError::core(error);
    let code = if mapped.code == "invalid_input" {
        "request_rejected"
    } else {
        mapped.code
    };
    ControlError::new(
        code,
        if message.trim().is_empty() {
            mapped.message
        } else {
            message
        },
    )
}
fn value(result: impl Serialize) -> Result<Value, ControlError> {
    serde_json::to_value(result).map_err(|_| rejected("응답을 만들지 못했습니다."))
}
// An agent reads a conversation without each answer's stored inputs: the attached meeting
// and Notion text, every citation it allowed and the template prompt. They can reach
// megabytes, past the app connection's limit, and the meeting and source tools read them.
fn conversation_view(conversation: crate::ai_chat::Conversation) -> Result<Value, ControlError> {
    let mut view = value(conversation)?;
    for message in view["messages"].as_array_mut().into_iter().flatten() {
        if let Some(fields) = message.as_object_mut() {
            fields.remove("binding");
        }
    }
    Ok(view)
}
fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(future)
}

// Open screens refresh the named area; data stays consistent through revision checks.
fn changed(app: &AppHandle, detail: Value) {
    let _ = app.emit_to("main", "nero://agent-changed", detail);
}
fn publish_snapshot(app: &AppHandle) {
    if let Ok(snapshot) = app.state::<DesktopState>().core.snapshot() {
        crate::publish(app, &snapshot);
    }
}

pub(crate) struct AgentApp<'a, L> {
    pub app: &'a AppHandle,
    // Saves the screens' open edits and holds them until the change is written.
    pub save_edits: &'a dyn Fn() -> Result<L, ControlError>,
}

impl<L> AgentApp<'_, L> {
    pub(crate) fn handle(&self, request: AppRequest) -> Result<Value, ControlError> {
        let app = self.app;
        let desktop = app
            .try_state::<DesktopState>()
            .ok_or_else(|| ControlError::new("app_not_ready", "Nero를 시작하는 중입니다."))?;
        if desktop.lifecycle.is_closing() {
            return Err(ControlError::new("app_not_ready", "Nero가 종료 중입니다."));
        }
        match request {
            AppRequest::AiChatsList {
                trashed,
                meeting_id,
            } => self.chats_list(trashed, meeting_id),
            AppRequest::AiChatGet { conversation_id } => {
                let (_lease, chat) = self.chat_lease()?;
                conversation_view(chat.read(&conversation_id).map_err(rejected)?)
            }
            AppRequest::AiModelsList { refresh } => self.models(refresh),
            AppRequest::AiChatSend {
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
                // A summary skill names the one meeting it summarizes; the chat checks the rest.
                let summary_target_id = skill_id
                    .as_ref()
                    .filter(|_| meeting_ids.len() == 1)
                    .map(|_| meeting_ids[0].clone());
                self.send(crate::ai_chat::SendRequest {
                    request_id: uuid::Uuid::new_v4().to_string(),
                    conversation_id: conversation_id.unwrap_or_default(),
                    meeting_ids,
                    account_id,
                    model_id,
                    effort,
                    message,
                    source_ids,
                    source_origins: None,
                    template_id,
                    skill_id,
                    summary_target_id,
                    edit_of,
                    parent_id: None,
                })
            }
            AppRequest::AiChatCancel {
                conversation_id,
                request_id,
            } => {
                let chat = self.chat()?;
                let conversation = chat.read(&conversation_id).map_err(rejected)?;
                if conversation.active_request_id.as_deref() != Some(request_id.as_str()) {
                    return Err(ControlError::new(
                        "edit_conflict",
                        "이 대화에서 진행 중인 답변이 아닙니다. ai_chat_get으로 확인해 주세요.",
                    ));
                }
                chat.cancel(&request_id).map_err(rejected)?;
                Ok(json!({"applied":true}))
            }
            AppRequest::AiChatVersionSelect {
                conversation_id,
                message_id,
            } => {
                let (_lease, chat) = self.chat_lease()?;
                let conversation = chat
                    .select_version(&conversation_id, &message_id)
                    .map_err(rejected)?;
                changed(
                    app,
                    json!({"area":"aiChat","conversationIds":[conversation_id]}),
                );
                conversation_view(conversation)
            }
            AppRequest::AiAnswerSave {
                conversation_id,
                message_id,
                meeting_id,
                target,
            } => {
                let _lease = (self.save_edits)()?;
                let result = ai_chat_desktop::save_answer(
                    app,
                    &conversation_id,
                    &message_id,
                    &meeting_id,
                    target == AnswerTarget::Summary,
                )
                .map_err(rejected)?;
                Ok(result)
            }
            AppRequest::AiChatsTrash {
                conversation_ids,
                older_than_days,
            } => self.chats_trash(conversation_ids, older_than_days),
            AppRequest::AiChatsRestore { conversation_ids } => {
                let (_lease, chat) = self.chat_lease()?;
                chat.restore(&conversation_ids).map_err(rejected)?;
                changed(
                    app,
                    json!({"area":"aiChat","conversationIds":conversation_ids}),
                );
                Ok(json!({"restored":conversation_ids}))
            }
            AppRequest::NotionStatus {} => value(self.notion()?.snapshot()),
            AppRequest::MeetingSourcesList { meeting_id } => {
                value(meeting_context_desktop::sources(app, &meeting_id).map_err(rejected)?)
            }
            AppRequest::MeetingSourceAdd { meeting_id, url } => {
                let current =
                    meeting_context_desktop::sources(app, &meeting_id).map_err(rejected)?;
                let source_id = self.fetch_source(url)?;
                self.change_sources(
                    &meeting_id,
                    current.revision(),
                    SourceChange::Add(source_id),
                )
            }
            AppRequest::MeetingSourceRefresh {
                meeting_id,
                page_id,
            } => {
                let current =
                    meeting_context_desktop::sources(app, &meeting_id).map_err(rejected)?;
                let url = current
                    .url_of(&page_id)
                    .ok_or_else(|| {
                        ControlError::new("not_found", "이 회의에 붙은 자료가 아닙니다.")
                    })?
                    .to_string();
                let source_id = self.fetch_source(url)?;
                self.change_sources(
                    &meeting_id,
                    current.revision(),
                    SourceChange::Refresh(source_id),
                )
            }
            AppRequest::MeetingSourceRemove {
                meeting_id,
                page_id,
            } => {
                let current =
                    meeting_context_desktop::sources(app, &meeting_id).map_err(rejected)?;
                if current.url_of(&page_id).is_none() {
                    return Err(ControlError::new(
                        "not_found",
                        "이 회의에 붙은 자료가 아닙니다.",
                    ));
                }
                self.change_sources(
                    &meeting_id,
                    current.revision(),
                    SourceChange::Remove(page_id),
                )
            }
            AppRequest::CalendarStatus {} => value(calendar_desktop::settings(app)),
            AppRequest::CalendarEvents { day, timezone } => value(
                block_on(calendar_desktop::occurrences(app, day, timezone)).map_err(rejected)?,
            ),
            AppRequest::CalendarsSelect { calendar_ids } => {
                value(calendar_desktop::select(app, calendar_ids).map_err(rejected)?)
            }
            AppRequest::CalendarSync {} => {
                value(block_on(calendar_desktop::sync(app)).map_err(rejected)?)
            }
            AppRequest::CalendarAttendees { meeting_id } => value(
                block_on(calendar_desktop::attendees(app, meeting_id, true)).map_err(rejected)?,
            ),
            AppRequest::MeetingFromCalendar {
                occurrence_key,
                day,
                timezone,
                scope_revision,
                meeting_id,
                expected_version,
            } => {
                let target =
                    meeting_id
                        .zip(expected_version)
                        .map(
                            |(meeting_id, expected_version)| nero_core::DraftAttachmentTarget {
                                meeting_id,
                                expected_version,
                            },
                        );
                let _lease = match target {
                    Some(_) => Some((self.save_edits)()?),
                    None => None,
                };
                let meeting = block_on(calendar_desktop::create_meeting(
                    app,
                    occurrence_key,
                    day,
                    timezone,
                    scope_revision,
                    target,
                ))
                .map_err(rejected)?;
                publish_snapshot(app);
                meeting_view(&meeting)
            }
            AppRequest::TerminologyGet { meeting_id } => {
                let settings = terminology::read_settings(&desktop.processing).map_err(rejected)?;
                let glossary = match meeting_id {
                    Some(id) => Some(domain_glossary::view(app, &id).map_err(rejected)?),
                    None => None,
                };
                Ok(json!({"settings":settings,"glossary":glossary}))
            }
            AppRequest::GlossarySave {
                meeting_id,
                expected_revision,
                entries,
            } => {
                let entries = entries
                    .into_iter()
                    .map(|entry| domain_glossary::Entry {
                        term: entry.term,
                        source_id: entry.source_id,
                    })
                    .collect();
                let snapshot =
                    domain_glossary::save_entries(app, &meeting_id, expected_revision, entries)
                        .map_err(rejected)?;
                changed(app, json!({"area":"terminology","meetingId":meeting_id}));
                value(snapshot)
            }
            AppRequest::SettingsGet {} => self.settings(),
            AppRequest::SettingsSet {
                speaker_analysis,
                terminology: enabled,
            } => {
                if let Some(enabled) = speaker_analysis {
                    desktop
                        .processing
                        .set_speaker_enabled(enabled)
                        .map_err(rejected)?;
                }
                if let Some(enabled) = enabled {
                    terminology::set_enabled(&desktop.processing, enabled).map_err(rejected)?;
                }
                changed(app, json!({"area":"settings"}));
                self.settings()
            }
            AppRequest::RecordingDevices {} => {
                let devices = desktop.core.devices().map_err(core_error)?;
                let snapshot = desktop.core.snapshot().map_err(core_error)?;
                Ok(json!({"devices":devices,"session":snapshot.session}))
            }
            AppRequest::RecordingControl {
                action,
                meeting_id,
                device_id,
                title,
                session_id,
            } => self.recording(action, meeting_id, device_id, title, session_id),
            AppRequest::MeetingCreate { title, notes } => {
                desktop
                    .processing
                    .require_meeting_models()
                    .map_err(not_ready)?;
                let title = if title.trim().is_empty() {
                    "새 회의".to_string()
                } else {
                    title
                };
                let meeting = desktop
                    .core
                    .create_meeting(&title, &notes)
                    .map_err(core_error)?;
                publish_snapshot(app);
                meeting_view(&meeting)
            }
            AppRequest::MeetingNotesAppend {
                meeting_id,
                expected_version,
                text,
            } => {
                let _lease = (self.save_edits)()?;
                let meeting = desktop
                    .core
                    .append_meeting_note(&meeting_id, &expected_version, None, &text)
                    .map_err(core_error)?;
                publish_snapshot(app);
                meeting_view(&meeting)
            }
            AppRequest::TranscriptionStart { meeting_id } => {
                desktop
                    .processing
                    .transcribe(desktop.core.clone(), meeting_id.clone())
                    .map_err(rejected)?;
                desktop.processing.agent_jobs(Some(&meeting_id), 0, 5)
            }
            AppRequest::SpeakersAnalyze {
                meeting_id,
                expected_version,
            } => {
                desktop
                    .processing
                    .diarize(desktop.core.clone(), meeting_id.clone(), expected_version)
                    .map_err(rejected)?;
                desktop.processing.agent_jobs(Some(&meeting_id), 0, 5)
            }
            AppRequest::ModelsPrepare {} => {
                let snapshot = desktop.core.snapshot().map_err(core_error)?;
                if crate::active_session(&snapshot).is_some() {
                    return Err(ControlError::new(
                        "recording_active",
                        "녹음을 마친 뒤 준비해 주세요.",
                    ));
                }
                desktop
                    .processing
                    .prepare_meeting_models(false)
                    .map_err(rejected)?;
                desktop.processing.agent_jobs(None, 0, 10)
            }
            AppRequest::TranscriptHistory { meeting_id } => value(
                desktop
                    .core
                    .transcript_history(&meeting_id)
                    .map_err(core_error)?,
            ),
            AppRequest::TranscriptSave {
                meeting_id,
                expected_version,
                segments,
            } => {
                let _lease = (self.save_edits)()?;
                let current = self.transcript(&meeting_id)?;
                let mut texts: Vec<String> = current
                    .segments
                    .iter()
                    .map(|segment| segment.text.clone())
                    .collect();
                for change in segments {
                    let index = current
                        .segments
                        .iter()
                        .position(|segment| segment.id == change.id)
                        .ok_or_else(|| ControlError::new("edit_conflict", "음성 기록에 없는 구간입니다. transcript_get으로 다시 확인해 주세요."))?;
                    texts[index] = change.text;
                }
                let document = desktop
                    .core
                    .edit_transcript(&meeting_id, expected_version, texts)
                    .map_err(core_error)?;
                publish_snapshot(app);
                Ok(json!({"meetingId":meeting_id,"version":document.version}))
            }
            AppRequest::TranscriptRestore {
                meeting_id,
                expected_version,
                target,
            } => {
                let _lease = (self.save_edits)()?;
                let core = desktop.core.clone();
                let document = desktop
                    .processing
                    .meeting_idle(&meeting_id, || {
                        core.restore_transcript(&meeting_id, expected_version, target)
                    })
                    .map_err(core_error)?;
                publish_snapshot(app);
                Ok(json!({"meetingId":meeting_id,"version":document.version}))
            }
            AppRequest::SpeakersSave {
                meeting_id,
                expected_version,
                speakers,
                assignments,
            } => {
                let _lease = (self.save_edits)()?;
                let current = self.transcript(&meeting_id)?;
                let mut speaker_ids: Vec<Option<String>> = current
                    .segments
                    .iter()
                    .map(|segment| segment.speaker_id.clone())
                    .collect();
                for change in assignments {
                    let index = current
                        .segments
                        .iter()
                        .position(|segment| segment.id == change.segment_id)
                        .ok_or_else(|| ControlError::new("edit_conflict", "음성 기록에 없는 구간입니다. transcript_get으로 다시 확인해 주세요."))?;
                    speaker_ids[index] = change.speaker_id;
                }
                let speakers = speakers
                    .into_iter()
                    .map(|speaker| TranscriptSpeaker {
                        id: speaker.id,
                        name: speaker.name,
                        name_origin: None,
                    })
                    .collect();
                let document = desktop
                    .core
                    .edit_speakers(&meeting_id, expected_version, speakers, speaker_ids)
                    .map_err(core_error)?;
                publish_snapshot(app);
                Ok(
                    json!({"meetingId":meeting_id,"version":document.version,"speakers":document.speakers}),
                )
            }
            AppRequest::JobRetry {
                job_id,
                expected_version,
            } => {
                desktop
                    .processing
                    .agent_check_job(&job_id, &expected_version)?;
                desktop
                    .processing
                    .retry(desktop.core.clone(), &job_id)
                    .map_err(rejected)?;
                Ok(json!({"applied":true}))
            }
            AppRequest::JobAcknowledge {
                job_id,
                expected_version,
            } => {
                desktop
                    .processing
                    .agent_check_job(&job_id, &expected_version)?;
                desktop.processing.acknowledge(&job_id).map_err(rejected)?;
                Ok(json!({"applied":true}))
            }
        }
    }

    fn chat(&self) -> Result<std::sync::Arc<crate::ai_chat::AiChat>, ControlError> {
        self.app
            .state::<AiDesktop>()
            .chat()
            .map_err(|message| ControlError::new("account_unavailable", message))
    }
    // The screen's transition rule for chat store work; see AiDesktop::chat_lease.
    fn chat_lease(
        &self,
    ) -> Result<
        (
            crate::lifecycle::TransitionLease,
            std::sync::Arc<crate::ai_chat::AiChat>,
        ),
        ControlError,
    > {
        self.app
            .state::<AiDesktop>()
            .chat_lease(self.app)
            .map_err(rejected)
    }
    fn notion(
        &self,
    ) -> Result<std::sync::Arc<crate::notion_context::NotionContextService>, ControlError> {
        self.app
            .state::<AiDesktop>()
            .notion()
            .map_err(|message| ControlError::new("notion_unavailable", message))
    }

    fn chats_list(&self, trashed: bool, meeting_id: Option<String>) -> Result<Value, ControlError> {
        let (_lease, chat) = self.chat_lease()?;
        let mut conversations = chat.list(trashed).map_err(rejected)?;
        if let Some(id) = meeting_id {
            conversations.retain(|item| item.meeting_ids.contains(&id));
        }
        Ok(json!({"conversations":conversations}))
    }

    fn chats_trash(
        &self,
        ids: Vec<String>,
        older_than_days: Option<u32>,
    ) -> Result<Value, ControlError> {
        let (_lease, chat) = self.chat_lease()?;
        let ids = match older_than_days {
            Some(days) => {
                let cutoff = now_ms().saturating_sub(u64::from(days) * DAY_MS);
                chat.list(false)
                    .map_err(rejected)?
                    .into_iter()
                    .filter(|item| {
                        item.active_request_id.is_none()
                            && item.updated_at.max(item.created_at) < cutoff
                    })
                    .map(|item| item.id)
                    .collect::<Vec<_>>()
            }
            None => ids,
        };
        if ids.is_empty() {
            return Ok(json!({"trashed":[]}));
        }
        let trashed = chat.trash(&ids, true).map_err(rejected)?;
        changed(self.app, json!({"area":"aiChat","conversationIds":trashed}));
        Ok(json!({"trashed":trashed}))
    }

    fn models(&self, refresh: bool) -> Result<Value, ControlError> {
        let state = self.app.state::<AiDesktop>();
        let accounts = state
            .accounts()
            .map_err(|message| ControlError::new("account_unavailable", message))?;
        let chat = self.chat()?;
        let view = ai_chat_desktop::account_view(&accounts, &chat.claude).map_err(rejected)?;
        let mut rows = Vec::new();
        for mut account in view["accounts"].as_array().cloned().unwrap_or_default() {
            let id = account["id"].as_str().unwrap_or("").to_string();
            let usable =
                !id.is_empty() && matches!(account["state"].as_str(), Some("ready" | "unknown"));
            if usable {
                let models = if refresh {
                    block_on(chat.models_refresh(&id))
                } else {
                    block_on(chat.models(&id))
                };
                match models {
                    Ok(models) => account["models"] = value(models)?,
                    Err(message) => account["modelsError"] = json!(message),
                }
            }
            rows.push(account);
        }
        Ok(json!({"accounts":rows}))
    }

    fn send(&self, request: crate::ai_chat::SendRequest) -> Result<Value, ControlError> {
        let mut request = request;
        let chat = self.chat()?;
        let _lease = (self.save_edits)()?;
        if request.conversation_id.is_empty() {
            request.conversation_id = chat
                .create()
                .map_err(rejected)?
                .id
                .ok_or_else(|| rejected("새 대화를 만들지 못했습니다."))?;
        }
        let conversation_id = request.conversation_id.clone();
        let started = chat
            .start(request, ai_chat_desktop::chat_events(self.app))
            .map_err(rejected)?;
        changed(
            self.app,
            json!({"area":"aiChat","conversationIds":[conversation_id]}),
        );
        value(started)
    }

    fn fetch_source(&self, url: String) -> Result<String, ControlError> {
        let request_id = format!("agent-{}", uuid::Uuid::new_v4());
        block_on(self.notion()?.fetch_sources(request_id, vec![url]))
            .map_err(|error| ControlError::new("notion_unavailable", error.to_string()))?
            .sources
            .pop()
            .map(|source| source.source_id)
            .ok_or_else(|| {
                ControlError::new("notion_unavailable", "Notion 페이지를 읽지 못했습니다.")
            })
    }

    fn change_sources(
        &self,
        meeting_id: &str,
        revision: u64,
        change: SourceChange,
    ) -> Result<Value, ControlError> {
        let view = meeting_context_desktop::change_sources(self.app, meeting_id, revision, change)
            .map_err(rejected)?;
        changed(
            self.app,
            json!({"area":"meetingSources","meetingId":meeting_id}),
        );
        value(view)
    }

    fn settings(&self) -> Result<Value, ControlError> {
        let desktop = self.app.state::<DesktopState>();
        let progress = desktop.processing.snapshot();
        let terminology = terminology::read_settings(&desktop.processing).map_err(rejected)?;
        Ok(json!({"speakerAnalysis":progress.speaker_enabled,"terminology":terminology}))
    }

    fn transcript(&self, meeting_id: &str) -> Result<nero_core::Transcript, ControlError> {
        self.app
            .state::<DesktopState>()
            .core
            .transcript(meeting_id)
            .map_err(core_error)?
            .ok_or_else(|| ControlError::new("invalid_state", "이 회의에는 음성 기록이 없습니다."))
    }

    fn recording(
        &self,
        action: RecordingAction,
        meeting_id: Option<String>,
        device_id: Option<String>,
        title: Option<String>,
        expected_session: Option<String>,
    ) -> Result<Value, ControlError> {
        let app = self.app;
        let desktop = app.state::<DesktopState>();
        let snapshot = desktop.core.snapshot().map_err(core_error)?;
        let session = crate::active_session(&snapshot).map(str::to_owned);
        // An agent naming a recording never pauses or stops one the person started since.
        if expected_session.is_some() && expected_session != session {
            return Err(ControlError::new(
                "edit_conflict",
                "진행 중인 녹음이 바뀌었습니다. recording_devices로 현재 녹음을 확인해 주세요.",
            ));
        }
        let changed = match action {
            RecordingAction::Start => {
                if session.is_some() {
                    return Err(ControlError::new("recording_active", "이미 녹음 중입니다."));
                }
                match meeting_id {
                    Some(id) => block_on(crate::begin_recording(app.clone(), id, device_id))
                        .map_err(rejected)?,
                    None => {
                        desktop
                            .processing
                            .require_meeting_models()
                            .map_err(not_ready)?;
                        // The meeting is made only after readiness and microphone
                        // permission pass, so a refused start leaves no empty meeting.
                        let title = title.unwrap_or_else(|| "새 회의".into());
                        block_on(crate::start_capture(app, device_id, move |core, device| {
                            let snapshot = core
                                .create_recording(&title, "", device)
                                .map_err(|error| error.to_string())?;
                            Ok(((), snapshot))
                        }))
                        .map(|(_, snapshot)| snapshot)
                        .map_err(rejected)?
                    }
                }
            }
            _ => {
                let session = session.ok_or_else(|| {
                    ControlError::new("invalid_state", "진행 중인 녹음이 없습니다.")
                })?;
                let change = match action {
                    RecordingAction::Pause => crate::RecordingAction::Pause,
                    RecordingAction::Resume => crate::RecordingAction::Resume,
                    _ => crate::RecordingAction::Stop,
                };
                block_on(crate::change_recording(app.clone(), session, change)).map_err(rejected)?
            }
        };
        Ok(json!({"session":changed.session}))
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}
fn meeting_view(meeting: &nero_core::Meeting) -> Result<Value, ControlError> {
    let version = agent_control::meeting_edit_version(meeting).map_err(core_error)?;
    Ok(json!({"meeting":{"id":meeting.id,"title":meeting.title},"editVersion":version}))
}
fn not_ready(message: String) -> ControlError {
    ControlError::new("speech_not_ready", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_conversation_view_leaves_out_stored_answer_inputs() {
        let id = uuid::Uuid::new_v4().to_string();
        let message = |role: &str| {
            json!({"id":uuid::Uuid::new_v4().to_string(),"requestId":uuid::Uuid::new_v4().to_string(),
                "role":role,"text":"보존할 답변","status":"completed",
                "citations":[{"reference":1,"kind":"meeting","label":"회의 00:01"}],
                "binding":{"citations":[],"inputSha256":"synthetic","templatePrompt":"프롬프트",
                    "inputSnapshot":{"meetings":[{"notes":"x".repeat(2 * 1024 * 1024)}]}}})
        };
        let conversation: crate::ai_chat::Conversation = serde_json::from_value(json!({
            "formatVersion":2,"id":id,"provider":"claude","modelId":"synthetic",
            "messages":[message("user"), message("assistant")]
        }))
        .unwrap();
        let view = conversation_view(conversation).unwrap();
        assert!(serde_json::to_vec(&view).unwrap().len() < 64 * 1024);
        for message in view["messages"].as_array().unwrap() {
            assert!(message.get("binding").is_none());
            assert_eq!(message["text"], "보존할 답변");
            assert_eq!(message["citations"][0]["reference"], 1);
        }
    }
}
