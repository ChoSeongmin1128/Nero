mod context;
mod provider;
mod store;
use crate::ai_accounts::{AiAccounts, Provider};
use crate::notion_context::NotionContextService;
use context::Binding;
pub use context::SourceOrigin;
use nero_core::CoreHandle;
pub use provider::Models;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use store::Message;
pub use store::{Conversation, ConversationSummary};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SendRequest {
    pub request_id: String,
    pub conversation_id: String,
    #[serde(default)]
    pub meeting_ids: Vec<String>,
    pub account_id: String,
    pub model_id: String,
    pub message: String,
    #[serde(default)]
    pub source_ids: Vec<String>,
    #[serde(default)]
    pub source_origins: Option<std::collections::BTreeMap<String, context::SourceOrigin>>,
    pub template_id: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEvent {
    pub request_id: String,
    pub conversation_id: String,
    pub meeting_id: Option<String>,
    pub message_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub text: Option<String>,
    pub error: Option<String>,
    pub citations: Vec<context::Citation>,
    pub sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<context::Attachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub save_targets: Option<Vec<context::SaveTarget>>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Started {
    pub conversation_id: String,
    pub request_id: String,
}
#[derive(Clone)]
struct Partial {
    text: String,
    sequence: u64,
}
#[derive(Clone)]
struct Active {
    request_fingerprint: String,
    checkpoint: Arc<Mutex<(Instant, usize)>>,
    conversation_id: String,
    message_id: String,
    account_id: String,
    cancel: Arc<AtomicBool>,
    partial: Arc<Mutex<Partial>>,
    emit: Arc<dyn Fn(ChatEvent) + Send + Sync>,
}
pub struct AiChat {
    root: PathBuf,
    core: CoreHandle,
    pub accounts: Arc<AiAccounts>,
    pub claude: Arc<crate::claude_connection::ClaudeConnection>,
    pub notion: Option<Arc<NotionContextService>>,
    store: Mutex<store::Store>,
    running: Mutex<HashMap<String, Active>>,
}
fn provider_id(provider: Provider) -> &'static str {
    match provider {
        Provider::ChatGpt => "chatgpt",
        Provider::Claude => "claude",
    }
}
fn request_fingerprint(request: &SendRequest) -> Result<String, String> {
    let bytes = serde_json::to_vec(request).map_err(|_| "대화 요청을 확인하지 못했습니다.")?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
fn existing_request(
    store: &store::Store,
    running: &HashMap<String, Active>,
    request: &SendRequest,
    fingerprint: &str,
) -> Result<Option<Started>, String> {
    if let Some(active) = running.get(&request.request_id) {
        if active.conversation_id != request.conversation_id
            || active.request_fingerprint != fingerprint
        {
            return Err("같은 요청의 대화 또는 자료가 변경되었습니다.".into());
        }
        return Ok(Some(Started {
            conversation_id: active.conversation_id.clone(),
            request_id: request.request_id.clone(),
        }));
    }
    let conversation = store.read(&request.conversation_id)?;
    if let Some(message) = conversation
        .messages
        .iter()
        .find(|message| message.role == "user" && message.request_id == request.request_id)
    {
        if message.request_fingerprint.as_deref() != Some(fingerprint) {
            return Err("같은 요청의 대화 또는 자료가 변경되었습니다.".into());
        }
        return Ok(Some(Started {
            conversation_id: conversation.id.ok_or("대화를 확인하지 못했습니다.")?,
            request_id: request.request_id.clone(),
        }));
    }
    Ok(None)
}
impl AiChat {
    pub fn new(
        root: PathBuf,
        core: CoreHandle,
        accounts: Arc<AiAccounts>,
        notion: Option<Arc<NotionContextService>>,
    ) -> Self {
        Self {
            store: Mutex::new(store::Store::new(root.clone())),
            claude: Arc::new(crate::claude_connection::ClaudeConnection::new(
                root.clone(),
            )),
            root,
            core,
            accounts,
            notion,
            running: Mutex::new(HashMap::new()),
        }
    }
    pub fn is_busy(&self) -> bool {
        !self.running.lock().unwrap().is_empty()
    }
    pub fn cancel_all(&self) {
        let ids = {
            let running = self.running.lock().unwrap();
            for active in running.values() {
                active.cancel.store(true, Ordering::Release);
            }
            running.keys().cloned().collect::<Vec<_>>()
        };
        for id in ids {
            let _ = self.checkpoint(&id, true);
        }
    }
    pub fn drain(&self, timeout: Duration) -> Result<(), String> {
        self.cancel_all();
        let ids = self
            .running
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for id in ids {
            self.checkpoint(&id, true)?;
        }
        let deadline = Instant::now() + timeout;
        while self.is_busy() {
            if Instant::now() >= deadline {
                return Err("AI 답변을 정리하는 중입니다. 잠시 후 다시 시도해 주세요.".into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Ok(())
    }
    fn checkpoint(&self, request_id: &str, force: bool) -> Result<(), String> {
        let store = self.store.lock().unwrap();
        let running = self.running.lock().unwrap();
        let Some(active) = running.get(request_id) else {
            return Ok(());
        };
        let partial = active.partial.lock().unwrap().clone();
        let text = partial.text;
        let mut checkpoint = active.checkpoint.lock().unwrap();
        if text.len() == checkpoint.1
            || (!force
                && checkpoint.0.elapsed() < Duration::from_secs(2)
                && text.len().saturating_sub(checkpoint.1) < 64 * 1024)
        {
            return Ok(());
        }
        let mut conversation = store.read(&active.conversation_id)?;
        let message = conversation
            .messages
            .iter_mut()
            .find(|message| message.id == active.message_id)
            .ok_or("답변을 찾을 수 없습니다.")?;
        message.text = text;
        message.sequence = partial.sequence;
        let length = message.text.len();
        store.write(&conversation)?;
        *checkpoint = (Instant::now(), length);
        Ok(())
    }
    pub fn with_meeting_removal<T>(
        &self,
        ids: &[String],
        _permanent: bool,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        for id in ids {
            store::id(id)?;
        }
        // Preserve the previous layout before Core is permitted to purge its meeting folder.
        let store = self.store.lock().unwrap();
        store.import_legacy()?;
        action()
    }
    pub fn list(&self) -> Result<Vec<ConversationSummary>, String> {
        let store = self.store.lock().unwrap();
        store.import_legacy()?;
        let mut summaries = store.list()?;
        let running = self.running.lock().unwrap();
        for summary in &mut summaries {
            summary.active_request_id = running
                .iter()
                .find(|(_, active)| active.conversation_id == summary.id)
                .map(|(id, _)| id.clone());
        }
        Ok(summaries)
    }
    pub fn create(&self) -> Result<Conversation, String> {
        let store = self.store.lock().unwrap();
        store.import_legacy()?;
        store.create()
    }
    pub fn delete(&self, conversation_id: &str) -> Result<(), String> {
        store::id(conversation_id)?;
        let mut events = Vec::new();
        let result = (|| {
            let store = self.store.lock().unwrap();
            let mut running = self.running.lock().unwrap();
            let targets = running
                .iter()
                .filter(|(_, active)| active.conversation_id == conversation_id)
                .map(|(id, active)| (id.clone(), active.clone()))
                .collect::<Vec<_>>();
            for (request_id, active) in targets {
                active.cancel.store(true, Ordering::Release);
                let mut conversation = store.read(conversation_id)?;
                let message = conversation
                    .messages
                    .iter_mut()
                    .find(|message| message.id == active.message_id)
                    .ok_or("답변을 찾을 수 없습니다.")?;
                let partial = active.partial.lock().unwrap().clone();
                message.text = partial.text;
                message.sequence = partial.sequence.saturating_add(1);
                message.status = "cancelled".into();
                message.error = Some("대화를 삭제했습니다.".into());
                let event = ChatEvent {
                    request_id: request_id.clone(),
                    conversation_id: conversation_id.into(),
                    meeting_id: None,
                    message_id: message.id.clone(),
                    kind: message.status.clone(),
                    text: Some(message.text.clone()),
                    error: message.error.clone(),
                    citations: message.citations.clone(),
                    sequence: message.sequence,
                    attachments: Some(message.attachments.clone()),
                    save_targets: Some(message.save_targets.clone()),
                };
                conversation.active_request_id = None;
                conversation.updated_at = store::now();
                store.write(&conversation)?;
                running.remove(&request_id);
                events.push((active.emit, event));
            }
            store.remove(conversation_id)
        })();
        for (emit, event) in events {
            emit(event);
        }
        result
    }
    pub fn cancel_account(&self, id: &str) {
        for active in self
            .running
            .lock()
            .unwrap()
            .values()
            .filter(|active| active.account_id == id)
        {
            active.cancel.store(true, Ordering::Release);
        }
    }
    pub fn cancel(&self, request: &str) -> Result<(), String> {
        store::id(request)?;
        if let Some(active) = self.running.lock().unwrap().get(request) {
            active.cancel.store(true, Ordering::Release);
        }
        Ok(())
    }
    pub fn read(&self, conversation_id: &str) -> Result<Conversation, String> {
        let store = self.store.lock().unwrap();
        store.import_legacy()?;
        let mut conversation = store.read(conversation_id)?;
        conversation.restore_sources();
        let running = self.running.lock().unwrap();
        conversation.active_request_id = None;
        for message in &mut conversation.messages {
            if message.status == "streaming" {
                if let Some(active) = running.get(&message.request_id).filter(|active| {
                    active.conversation_id == conversation_id && active.message_id == message.id
                }) {
                    let partial = active.partial.lock().unwrap();
                    message.text = partial.text.clone();
                    message.sequence = partial.sequence;
                    conversation.active_request_id = Some(message.request_id.clone());
                } else {
                    message.status = "interrupted".into();
                    message.error = Some("답변이 중단되었습니다. 다시 시도해 주세요.".into());
                }
            }
        }
        Ok(conversation)
    }
    fn require_meeting(&self, id: &str) -> Result<(), String> {
        store::id(id)?;
        if self
            .core
            .list_meetings()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|meeting| meeting.id == id && meeting.deleted_at.is_none())
        {
            Ok(())
        } else {
            Err("회의를 찾을 수 없습니다.".into())
        }
    }
    pub async fn models(&self, id: &str) -> Result<Models, String> {
        if id == crate::claude_connection::ACCOUNT_ID {
            let models = self
                .claude
                .models()
                .into_iter()
                .map(|model| {
                    let label = match model.resolved_model.as_deref() {
                        Some(resolved) if resolved != model.display_name => {
                            format!("{} ({})", model.display_name, resolved)
                        }
                        _ => model.display_name,
                    };
                    provider::ChatModel {
                        id: model.value,
                        label,
                        max_output_tokens: None,
                    }
                })
                .collect::<Vec<_>>();
            let default_model_id = models
                .iter()
                .find(|m| m.id == "sonnet")
                .or_else(|| models.first())
                .map(|m| m.id.clone());
            return Ok(Models {
                models,
                default_model_id,
            });
        }
        let session = self.accounts.subscription_session(id).await?;
        if session.provider != Provider::ChatGpt {
            return Err("Claude 계정 연결을 확인해 주세요.".into());
        }
        provider::models(&provider::ProviderAccess {
            provider: provider_id(session.provider).into(),
            access_token: session.access_token().into(),
        })
        .await
    }
    pub fn start(
        self: &Arc<Self>,
        request: SendRequest,
        emit: Arc<dyn Fn(ChatEvent) + Send + Sync>,
    ) -> Result<Started, String> {
        store::id(&request.request_id)?;
        store::id(&request.conversation_id)?;
        for id in &request.meeting_ids {
            store::id(id)?;
        }
        if request.message.trim().is_empty()
            || request.message.len() > 32 * 1024
            || request.source_ids.len() > 16
            || request.meeting_ids.len() > 16
            || request.model_id.is_empty()
            || request.model_id.len() > 128
        {
            return Err("질문과 선택한 자료를 확인해 주세요.".into());
        }
        let fingerprint = request_fingerprint(&request)?;
        {
            let store = self.store.lock().unwrap();
            let running = self.running.lock().unwrap();
            if let Some(started) = existing_request(&store, &running, &request, &fingerprint)? {
                return Ok(started);
            }
        }
        for id in &request.meeting_ids {
            self.require_meeting(id)?;
        }
        let selected_provider = if request.account_id == crate::claude_connection::ACCOUNT_ID {
            Provider::Claude
        } else {
            self.accounts
                .snapshot()?
                .accounts
                .into_iter()
                .find(|account| {
                    account.id == request.account_id
                        && account.provider == Provider::ChatGpt
                        && (account.can_invoke || account.state == "unknown")
                })
                .ok_or("먼저 AI 계정에 로그인해 주세요.")?
                .provider
        };
        let documents = if request.source_ids.is_empty() {
            vec![]
        } else {
            self.notion
                .as_ref()
                .ok_or("Notion 연결을 준비하지 못했습니다.")?
                .context_bundle(&request.source_ids)
                .map_err(|e| e.to_string())?
                .sources
                .into_iter()
                .map(|source| {
                    serde_json::to_value(source)
                        .map_err(|_| "자료를 확인하지 못했습니다.".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        if documents.iter().any(|source| source["truncated"] == true) {
            return Err("전체 내용을 읽지 못한 자료가 있습니다. 자료를 다시 확인해 주세요.".into());
        }
        let mut prepared = context::prepare(
            &self.core,
            &self.root,
            &request.meeting_ids,
            documents,
            request.template_id.as_deref(),
        )?;
        if let Some(origins) = &request.source_origins {
            let notion = self
                .notion
                .as_ref()
                .ok_or("Notion 연결을 준비하지 못했습니다.")?;
            context::apply_source_origins(
                &mut prepared.binding.source_attachments,
                &request.meeting_ids,
                &request.source_ids,
                origins,
                |id| {
                    notion
                        .read_sources(vec![id.to_owned()])
                        .map_err(|e| e.to_string())?
                        .sources
                        .into_iter()
                        .next()
                        .map(|source| source.page_id)
                        .ok_or_else(|| "자료를 찾을 수 없습니다.".into())
                },
            )?;
        }
        let store = self.store.lock().unwrap();
        store::id(&request.conversation_id)?;
        for id in &request.meeting_ids {
            self.require_meeting(id)?;
        }
        let mut running = self.running.lock().unwrap();
        if let Some(started) = existing_request(&store, &running, &request, &fingerprint)? {
            return Ok(started);
        }
        let mut conversation = store.read(&request.conversation_id)?;
        if running
            .values()
            .any(|active| active.conversation_id == request.conversation_id)
        {
            return Err("이 대화의 답변을 기다리거나 중지해 주세요.".into());
        }
        if running.len() >= 4 {
            return Err("진행 중인 답변이 끝난 뒤 다시 시도해 주세요.".into());
        }
        if conversation.id.as_deref() != Some(request.conversation_id.as_str()) {
            return Err("대화가 변경되었습니다. 다시 열어 주세요.".into());
        }
        for message in &mut conversation.messages {
            if message.status == "streaming" {
                message.status = "interrupted".into();
            }
        }
        let conversation_id = conversation
            .id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        let message_id = uuid::Uuid::new_v4().to_string();
        let mut history = conversation
            .messages
            .iter()
            .filter(|message| message.status == "completed")
            .map(|message| {
                let text = nero_core::presentation::format_summary(
                    &message.text,
                    nero_core::DocumentFormat::Md,
                    false,
                    false,
                    message.binding.as_ref().and_then(|binding| {
                        binding
                            .citations
                            .iter()
                            .map(|citation| citation.reference)
                            .max()
                    }),
                );
                json!({"role":message.role,"content":text})
            })
            .collect::<Vec<Value>>();
        history.push(json!({"role":"user","content":format!("{}\n\n참고 자료(JSON 데이터):\n{}",request.message,prepared.input)}));
        if serde_json::to_vec(&history)
            .map_err(|_| "대화를 준비하지 못했습니다.")?
            .len()
            > 2 * 1024 * 1024
        {
            return Err("대화와 자료가 너무 많습니다. 선택한 자료를 줄여 주세요.".into());
        }
        if conversation.messages.is_empty() {
            conversation.title = request.message.trim().chars().take(60).collect();
        }
        conversation.updated_at = store::now();
        conversation.provider = Some(provider_id(selected_provider).into());
        conversation.model_id = Some(request.model_id.clone());
        conversation.active_request_id = Some(request.request_id.clone());
        conversation.messages.push(Message {
            request_fingerprint: Some(fingerprint.clone()),
            sequence: 0,
            id: uuid::Uuid::new_v4().to_string(),
            request_id: request.request_id.clone(),
            role: "user".into(),
            text: request.message.clone(),
            status: "completed".into(),
            citations: vec![],
            error: None,
            provider: None,
            model_id: None,
            actual_model_id: None,
            binding: None,
            saved_summary_id: None,
            note_saved: false,
            attachments: prepared.binding.attachments(),
            save_targets: prepared.binding.save_targets(),
            summary_receipts: Default::default(),
            note_receipts: Default::default(),
        });
        conversation.messages.push(Message {
            request_fingerprint: Some(fingerprint.clone()),
            sequence: 0,
            id: message_id.clone(),
            request_id: request.request_id.clone(),
            role: "assistant".into(),
            text: String::new(),
            status: "streaming".into(),
            citations: vec![],
            error: None,
            provider: Some(provider_id(selected_provider).into()),
            model_id: Some(request.model_id.clone()),
            actual_model_id: None,
            binding: Some(prepared.binding.clone()),
            saved_summary_id: None,
            note_saved: false,
            attachments: prepared.binding.attachments(),
            save_targets: prepared.binding.save_targets(),
            summary_receipts: Default::default(),
            note_receipts: Default::default(),
        });
        store.write(&conversation)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let partial = Arc::new(Mutex::new(Partial {
            text: String::new(),
            sequence: 0,
        }));
        running.insert(
            request.request_id.clone(),
            Active {
                request_fingerprint: fingerprint,
                checkpoint: Arc::new(Mutex::new((Instant::now(), 0))),
                conversation_id: conversation_id.clone(),
                message_id: message_id.clone(),
                account_id: request.account_id.clone(),
                cancel: cancel.clone(),
                partial: partial.clone(),
                emit: emit.clone(),
            },
        );
        drop(running);
        drop(store);
        let started = Started {
            conversation_id: conversation_id.clone(),
            request_id: request.request_id.clone(),
        };
        let this = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            tauri::async_runtime::block_on(async move {
                let mut sequence = 0;
                let mut persistence_error = false;
                let mut actual_model_id = None;
                let result = async {
                    let mut deliver = |delta: &str| {
                        if cancel.load(Ordering::Acquire) {
                            return;
                        }
                        sequence += 1;
                        {
                            let mut partial = partial.lock().unwrap();
                            partial.text.push_str(delta);
                            partial.sequence = sequence;
                        }
                        if this.checkpoint(&request.request_id, false).is_err() {
                            persistence_error = true;
                            cancel.store(true, Ordering::Release);
                        }
                        if cancel.load(Ordering::Acquire) {
                            return;
                        }
                        emit(ChatEvent {
                            request_id: request.request_id.clone(),
                            conversation_id: conversation_id.clone(),
                            meeting_id: None,
                            message_id: message_id.clone(),
                            kind: "delta".into(),
                            text: Some(delta.into()),
                            error: None,
                            citations: vec![],
                            sequence,
                            attachments: None,
                            save_targets: None,
                        });
                    };
                    if selected_provider == Provider::Claude {
                        let prompt = format!(
                            "{}\n\n대화(JSON 데이터):\n{}",
                            prepared.instructions,
                            serde_json::to_string(&history)
                                .map_err(|_| "대화를 준비하지 못했습니다.")?
                        );
                        let reply = this.claude.reply(
                            &request.model_id,
                            &prompt,
                            cancel.clone(),
                            &mut deliver,
                        )?;
                        actual_model_id = reply.actual_model_id;
                        return Ok(reply.text);
                    }
                    let session = this
                        .accounts
                        .subscription_session_with_cancel(&request.account_id, cancel.clone())
                        .await?;
                    if session.provider != selected_provider {
                        return Err("선택한 AI 계정이 변경되었습니다.".into());
                    }
                    let access = provider::ProviderAccess {
                        provider: provider_id(session.provider).into(),
                        access_token: session.access_token().into(),
                    };
                    let models = provider::models_with_cancel(&access, &cancel).await?;
                    let selected = models
                        .models
                        .iter()
                        .find(|model| model.id == request.model_id)
                        .ok_or("이 AI 계정에서 선택한 모델을 사용할 수 없습니다.")?;
                    provider::reply(
                        &access,
                        selected,
                        &prepared.instructions,
                        history,
                        cancel.clone(),
                        &mut deliver,
                    )
                    .await
                }
                .await;
                let event = this.finish(
                    &request.request_id,
                    if persistence_error {
                        Err("부분 답변을 저장하지 못했습니다. 다시 열어 확인해 주세요.".into())
                    } else {
                        result
                    },
                    persistence_error,
                    sequence + 1,
                    actual_model_id,
                );
                if let Ok(Some(event)) = event {
                    emit(event);
                } else if event.is_err() {
                    emit(ChatEvent {
                        request_id: request.request_id.clone(),
                        conversation_id,
                        meeting_id: None,
                        message_id,
                        kind: "failed".into(),
                        text: Some(partial.lock().unwrap().text.clone()),
                        error: Some(
                            "AI 대화를 저장하지 못했습니다. 다시 열어 확인해 주세요.".into(),
                        ),
                        citations: vec![],
                        sequence: sequence + 1,
                        attachments: Some(prepared.binding.attachments()),
                        save_targets: Some(prepared.binding.save_targets()),
                    });
                }
            })
        });
        Ok(started)
    }
    fn finish(
        &self,
        request_id: &str,
        result: Result<String, String>,
        persistence_error: bool,
        sequence: u64,
        actual_model_id: Option<String>,
    ) -> Result<Option<ChatEvent>, String> {
        let store = self.store.lock().unwrap();
        let mut running = self.running.lock().unwrap();
        let Some(active) = running.remove(request_id) else {
            return Ok(None);
        };
        let cancelled = active.cancel.load(Ordering::Acquire) && !persistence_error;
        let mut conversation = store.read(&active.conversation_id)?;
        let message = conversation
            .messages
            .iter_mut()
            .find(|message| message.id == active.message_id)
            .ok_or("답변을 찾을 수 없습니다.")?;
        let result = if cancelled {
            Err("취소했습니다.".into())
        } else {
            result
        };
        match result.and_then(|text| {
            context::cited(
                &text,
                &message
                    .binding
                    .as_ref()
                    .ok_or("답변 자료를 확인하지 못했습니다.")?
                    .citations,
            )
            .map(|citations| (text, citations))
        }) {
            Ok((text, citations)) => {
                message.text = text;
                message.actual_model_id = actual_model_id;
                message.citations = citations;
                message.status = "completed".into();
                message.error = None;
            }
            Err(error) => {
                {
                    let partial = active.partial.lock().unwrap();
                    message.text = partial.text.clone();
                    message.sequence = partial.sequence;
                }
                message.status = if cancelled { "cancelled" } else { "failed" }.into();
                message.error = Some(error);
            }
        }
        message.sequence = sequence;
        conversation.active_request_id = None;
        conversation.updated_at = store::now();
        let event = ChatEvent {
            request_id: request_id.into(),
            conversation_id: active.conversation_id.clone(),
            meeting_id: None,
            message_id: message.id.clone(),
            kind: message.status.clone(),
            text: Some(message.text.clone()),
            error: message.error.clone(),
            citations: message.citations.clone(),
            sequence,
            attachments: Some(message.attachments.clone()),
            save_targets: Some(message.save_targets.clone()),
        };
        store.write(&conversation)?;
        Ok(Some(event))
    }
    pub fn save_summary(
        &self,
        conversation_id: &str,
        message_id: &str,
        meeting_id: &str,
    ) -> Result<Value, String> {
        self.apply_answer(conversation_id, message_id, meeting_id, true)
    }
    pub fn answer_document(
        &self,
        conversation_id: &str,
        message_id: &str,
        format: nero_core::DocumentFormat,
    ) -> Result<String, String> {
        store::id(conversation_id)?;
        store::id(message_id)?;
        let store = self.store.lock().unwrap();
        let conversation = store.read(conversation_id)?;
        if conversation.id.as_deref() != Some(conversation_id) {
            return Err("대화를 다시 확인해 주세요.".into());
        }
        let message = conversation
            .messages
            .iter()
            .find(|message| {
                message.id == message_id
                    && message.role == "assistant"
                    && message.status == "completed"
            })
            .ok_or("완성된 답변을 선택해 주세요.")?;
        let text = context::summary_text(&message.text, &message.citations);
        let count = message
            .binding
            .as_ref()
            .and_then(|binding| {
                binding
                    .citations
                    .iter()
                    .map(|citation| citation.reference)
                    .max()
            })
            .unwrap_or(0);
        Ok(nero_core::presentation::format_summary(
            &text,
            format,
            false,
            false,
            Some(count),
        ))
    }
    pub fn append_note(
        &self,
        conversation_id: &str,
        message_id: &str,
        meeting_id: &str,
    ) -> Result<Value, String> {
        self.apply_answer(conversation_id, message_id, meeting_id, false)
    }
    fn apply_answer(
        &self,
        conversation_id: &str,
        message_id: &str,
        meeting_id: &str,
        summary: bool,
    ) -> Result<Value, String> {
        store::id(conversation_id)?;
        store::id(message_id)?;
        self.require_meeting(meeting_id)?;
        let store = self.store.lock().unwrap();
        let mut conversation = store.read(conversation_id)?;
        let message = conversation
            .messages
            .iter_mut()
            .find(|m| m.id == message_id && m.role == "assistant" && m.status == "completed")
            .ok_or("완성된 답변을 선택해 주세요.")?;
        let binding = message
            .binding
            .clone()
            .ok_or("답변 자료를 확인하지 못했습니다.")?;
        let target = binding
            .targets()
            .into_iter()
            .find(|target| target.meeting_id == meeting_id)
            .ok_or("이 답변에 첨부된 회의를 선택해 주세요.")?;
        if summary {
            if let Some(id) = message.summary_receipts.get(meeting_id) {
                return Ok(json!({"summaryId":id}));
            }
        } else if message.note_receipts.contains(meeting_id) {
            return Ok(json!({"applied":true}));
        }
        let text = context::text_for_target(&message.text, &binding.citations, meeting_id);
        let result = if summary {
            let context = target
                .summary_context
                .ok_or("음성 기록을 포함한 회의를 선택해 주세요.")?;
            let saved = self
                .core
                .import_summary_checked(
                    meeting_id,
                    &target.meeting_version,
                    context,
                    text,
                    context::producer(
                        message.provider.as_deref().unwrap_or("AI"),
                        message.model_id.as_deref().unwrap_or("unknown"),
                        binding.template,
                    ),
                )
                .map_err(|e| e.to_string())?;
            message
                .summary_receipts
                .insert(meeting_id.into(), saved.id.clone());
            json!({"summaryId":saved.id})
        } else {
            let text = nero_core::presentation::format_summary(
                &text,
                nero_core::DocumentFormat::Md,
                false,
                false,
                Some(
                    binding
                        .citations
                        .iter()
                        .filter(|c| c.meeting_id.as_deref() == Some(meeting_id))
                        .count(),
                ),
            );
            self.core
                .append_meeting_note(
                    meeting_id,
                    &target.meeting_version,
                    target.summary_context,
                    &text,
                )
                .map_err(|e| e.to_string())?;
            message.note_receipts.insert(meeting_id.into());
            json!({"applied":true})
        };
        store.write_receipt(&conversation);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn service() -> (tempfile::TempDir, Arc<AiChat>) {
        let root = tempfile::tempdir().unwrap();
        let core = CoreHandle::open(root.path().canonicalize().unwrap()).unwrap();
        let accounts =
            Arc::new(AiAccounts::new(root.path().join("accounts"), root.path().into()).unwrap());
        let chat = Arc::new(AiChat::new(root.path().into(), core, accounts, None));
        (root, chat)
    }
    fn message(id: &str, request: &str, binding: Binding, status: &str) -> Message {
        Message {
            request_fingerprint: Some("fingerprint".into()),
            sequence: 0,
            id: id.into(),
            request_id: request.into(),
            role: "assistant".into(),
            text: String::new(),
            status: status.into(),
            citations: vec![],
            error: None,
            provider: Some("chatgpt".into()),
            model_id: Some("synthetic".into()),
            actual_model_id: None,
            attachments: binding.attachments(),
            save_targets: binding.save_targets(),
            binding: Some(binding),
            saved_summary_id: None,
            note_saved: false,
            summary_receipts: Default::default(),
            note_receipts: Default::default(),
        }
    }
    fn seed(chat: &AiChat, meetings: &[String], status: &str) -> (String, String, String) {
        let prepared = context::prepare(&chat.core, &chat.root, meetings, vec![], None).unwrap();
        let mut conversation = chat.create().unwrap();
        let id = conversation.id.clone().unwrap();
        let request = uuid::Uuid::new_v4().to_string();
        let answer = uuid::Uuid::new_v4().to_string();
        conversation
            .messages
            .push(message(&answer, &request, prepared.binding, status));
        conversation.active_request_id = if status == "streaming" {
            Some(request.clone())
        } else {
            None
        };
        chat.store.lock().unwrap().write(&conversation).unwrap();
        if status == "streaming" {
            chat.running.lock().unwrap().insert(
                request.clone(),
                Active {
                    request_fingerprint: "fingerprint".into(),
                    checkpoint: Arc::new(Mutex::new((Instant::now(), 0))),
                    conversation_id: id.clone(),
                    message_id: answer.clone(),
                    account_id: uuid::Uuid::new_v4().to_string(),
                    cancel: Arc::new(AtomicBool::new(false)),
                    partial: Arc::new(Mutex::new(Partial {
                        text: "부분 답변".into(),
                        sequence: 1,
                    })),
                    emit: Arc::new(|_| {}),
                },
            );
        }
        (id, request, answer)
    }
    #[test]
    fn independent_conversation_has_no_meeting_and_remains_after_restart() {
        let (root, chat) = service();
        let conversation = chat.create().unwrap();
        let id = conversation.id.clone().unwrap();
        assert!(chat.core.list_meetings().unwrap().is_empty());
        assert_eq!(chat.list().unwrap()[0].id, id);
        assert_eq!(chat.read(&id).unwrap().id, Some(id.clone()));
        assert_eq!(
            store::Store::new(root.path().into()).read(&id).unwrap().id,
            Some(id)
        );
    }
    #[test]
    fn attached_meeting_removal_preserves_chat_snapshot_and_active_stream() {
        let (_root, chat) = service();
        let meeting = chat.core.create_meeting("첨부 회의", "원래 메모").unwrap();
        let (id, request, answer) = seed(&chat, &[meeting.id.clone()], "streaming");
        chat.with_meeting_removal(&[meeting.id.clone()], false, || {
            chat.core
                .trash_meetings(vec![meeting.id.clone()])
                .map_err(|e| e.to_string())
        })
        .unwrap();
        chat.with_meeting_removal(&[meeting.id.clone()], true, || {
            chat.core
                .purge_meetings(vec![meeting.id.clone()])
                .map_err(|e| e.to_string())
        })
        .unwrap();
        assert!(chat.is_busy());
        let event = chat
            .finish(
                &request,
                Ok("답변 완료".into()),
                false,
                2,
                Some("claude-sonnet-5-5".into()),
            )
            .unwrap()
            .unwrap();
        assert_eq!(event.kind, "completed");
        assert_eq!(
            event.attachments.as_ref().unwrap()[0].meeting_id.as_deref(),
            Some(meeting.id.as_str())
        );
        assert!(!event.save_targets.as_ref().unwrap()[0].can_save_summary);
        let conversation = chat.read(&id).unwrap();
        assert_eq!(conversation.messages[0].id, answer);
        assert_eq!(conversation.messages[0].text, "답변 완료");
        assert_eq!(
            conversation.messages[0].model_id.as_deref(),
            Some("synthetic")
        );
        assert_eq!(
            conversation.messages[0].actual_model_id.as_deref(),
            Some("claude-sonnet-5-5")
        );
        assert_eq!(
            conversation.messages[0]
                .binding
                .as_ref()
                .unwrap()
                .input_snapshot["meetings"][0]["notes"],
            "원래 메모"
        );
        assert!(context::prepare(&chat.core, &chat.root, &[meeting.id], vec![], None).is_err());
    }
    #[test]
    fn deleting_a_conversation_cancels_only_its_worker_and_no_late_write_recreates_it() {
        let (_root, chat) = service();
        let (first, request, _) = seed(&chat, &[], "streaming");
        let (second, second_request, _) = seed(&chat, &[], "streaming");
        let cancel = chat
            .running
            .lock()
            .unwrap()
            .get(&request)
            .unwrap()
            .cancel
            .clone();
        chat.delete(&first).unwrap();
        assert!(cancel.load(Ordering::Acquire));
        assert!(chat
            .finish(&request, Ok("늦은 답변".into()), false, 2, None)
            .unwrap()
            .is_none());
        assert!(chat.read(&first).is_err());
        assert_eq!(
            chat.read(&second).unwrap().active_request_id,
            Some(second_request)
        );
    }
    #[test]
    fn cancellation_wins_over_completion_waiting_for_the_store() {
        let (_root, chat) = service();
        let (id, request, _) = seed(&chat, &[], "streaming");
        let held = chat.store.lock().unwrap();
        let worker = chat.clone();
        let request_copy = request.clone();
        let done = std::thread::spawn(move || {
            worker.finish(&request_copy, Ok("완료 경쟁".into()), false, 2, None)
        });
        chat.cancel(&request).unwrap();
        drop(held);
        assert_eq!(done.join().unwrap().unwrap().unwrap().kind, "cancelled");
        assert_eq!(chat.read(&id).unwrap().messages[0].text, "부분 답변");
        assert!(!chat.is_busy());
    }
    #[test]
    fn checkpoint_and_cancel_keep_partial_text_and_monotonic_sequence() {
        let (_root, chat) = service();
        let (id, request, _) = seed(&chat, &[], "streaming");
        chat.checkpoint(&request, true).unwrap();
        let stored = chat.store.lock().unwrap().read(&id).unwrap();
        assert_eq!(stored.messages[0].text, "부분 답변");
        assert_eq!(stored.messages[0].sequence, 1);
        chat.cancel(&request).unwrap();
        let event = chat
            .finish(&request, Err("취소".into()), false, 2, None)
            .unwrap()
            .unwrap();
        assert_eq!(event.kind, "cancelled");
        assert_eq!(event.sequence, 2);
    }
    #[test]
    fn stale_target_edits_are_not_overwritten_and_other_targets_remain_independent() {
        let (_root, chat) = service();
        let first = chat.core.create_meeting("첫 회의", "첫 메모").unwrap();
        let second = chat.core.create_meeting("다음 회의", "다음 메모").unwrap();
        let (id, _, answer) = seed(&chat, &[first.id.clone(), second.id.clone()], "completed");
        {
            let store = chat.store.lock().unwrap();
            let mut value = store.read(&id).unwrap();
            value.messages[0].text = "저장할 답변".into();
            store.write(&value).unwrap();
        }
        chat.core
            .update_meeting(&first.id, "바뀐 제목", "사용자 메모")
            .unwrap();
        assert!(chat.append_note(&id, &answer, &first.id).is_err());
        assert_eq!(
            chat.core
                .list_meetings()
                .unwrap()
                .iter()
                .find(|m| m.id == first.id)
                .unwrap()
                .notes,
            "사용자 메모"
        );
        chat.append_note(&id, &answer, &second.id).unwrap();
        chat.append_note(&id, &answer, &second.id).unwrap();
        assert_eq!(
            chat.core
                .list_meetings()
                .unwrap()
                .iter()
                .find(|m| m.id == second.id)
                .unwrap()
                .notes,
            "다음 메모\n\n저장할 답변"
        );
        let unrelated = chat.core.create_meeting("관련 없음", "").unwrap();
        assert!(chat.append_note(&id, &answer, &unrelated.id).is_err());
    }
    #[test]
    fn copying_free_chat_requires_no_meeting_and_never_mutates_the_conversation() {
        let (root, chat) = service();
        let (id, _, answer) = seed(&chat, &[], "completed");
        {
            let store = chat.store.lock().unwrap();
            let mut value = store.read(&id).unwrap();
            value.messages[0].text = "## 결정\n\n- 확인합니다.".into();
            store.write(&value).unwrap();
        }
        let path = root
            .path()
            .join("ai-conversations")
            .join(format!("{id}.json"));
        let before = std::fs::read(&path).unwrap();
        assert!(chat
            .answer_document(&id, &answer, nero_core::DocumentFormat::Md)
            .unwrap()
            .contains("## 결정"));
        assert!(!chat
            .answer_document(&id, &answer, nero_core::DocumentFormat::Txt)
            .unwrap()
            .contains("##"));
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
    #[test]
    fn failed_final_save_always_releases_the_active_request() {
        let (root, chat) = service();
        let (id, request, _) = seed(&chat, &[], "streaming");
        let path = root
            .path()
            .join("ai-conversations")
            .join(format!("{id}.json"));
        std::fs::rename(&path, root.path().join("preserved.json")).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(chat
            .finish(&request, Ok("완료".into()), false, 2, None)
            .is_err());
        assert!(!chat.is_busy());
    }
    #[test]
    fn changed_sources_and_conversations_do_not_reuse_an_active_request_id() {
        let (_root, chat) = service();
        let (id, request, _) = seed(&chat, &[], "streaming");
        let second = chat.create().unwrap().id.unwrap();
        let make = |conversation_id: String| SendRequest {
            request_id: request.clone(),
            conversation_id,
            meeting_ids: vec![],
            account_id: uuid::Uuid::new_v4().to_string(),
            model_id: "m".into(),
            message: "질문".into(),
            source_ids: vec![],
            source_origins: None,
            template_id: None,
        };
        let store = chat.store.lock().unwrap();
        let running = chat.running.lock().unwrap();
        let request = make(second);
        assert!(existing_request(&store, &running, &request, "fingerprint").is_err());
        let request = make(id);
        assert!(existing_request(&store, &running, &request, "changed").is_err());
    }
}
