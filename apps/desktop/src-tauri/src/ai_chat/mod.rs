mod context;
mod provider;
mod store;
use crate::ai_accounts::{AiAccounts, Provider};
use crate::notion_context::NotionContextService;
use context::Binding;
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
pub use store::Conversation;
use store::Message;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SendRequest {
    pub request_id: String,
    pub conversation_id: Option<String>,
    pub meeting_id: String,
    pub include_meeting: bool,
    pub account_id: String,
    pub model_id: String,
    pub message: String,
    #[serde(default)]
    pub source_ids: Vec<String>,
    pub template_id: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEvent {
    pub request_id: String,
    pub conversation_id: String,
    pub meeting_id: String,
    pub message_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub text: Option<String>,
    pub error: Option<String>,
    pub citations: Vec<context::Citation>,
    pub sequence: u64,
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
    meeting_id: String,
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
        if active.meeting_id != request.meeting_id || active.request_fingerprint != fingerprint {
            return Err("같은 요청의 대화 또는 자료가 변경되었습니다.".into());
        }
        return Ok(Some(Started {
            conversation_id: active.conversation_id.clone(),
            request_id: request.request_id.clone(),
        }));
    }
    let conversation = store.read(&request.meeting_id)?;
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
        let mut conversation = store.read(&active.meeting_id)?;
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
        permanent: bool,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        for id in ids {
            store::id(id)?;
        }
        let mut events = Vec::new();
        let result = (|| {
            let store = self.store.lock().unwrap();
            let mut running = self.running.lock().unwrap();
            let targets = running
                .iter()
                .filter(|(_, active)| ids.contains(&active.meeting_id))
                .map(|(request_id, active)| (request_id.clone(), active.clone()))
                .collect::<Vec<_>>();
            for (request_id, active) in targets {
                active.cancel.store(true, Ordering::Release);
                let mut conversation = store.read(&active.meeting_id)?;
                let message = conversation
                    .messages
                    .iter_mut()
                    .find(|message| message.id == active.message_id)
                    .ok_or("답변을 찾을 수 없습니다.")?;
                {
                    let partial = active.partial.lock().unwrap();
                    message.text = partial.text.clone();
                    message.sequence = partial.sequence.saturating_add(1);
                }
                message.status = "cancelled".into();
                message.error = Some("회의에서 대화를 중지했습니다.".into());
                let event = ChatEvent {
                    request_id: request_id.clone(),
                    conversation_id: active.conversation_id.clone(),
                    meeting_id: active.meeting_id.clone(),
                    message_id: message.id.clone(),
                    kind: message.status.clone(),
                    text: Some(message.text.clone()),
                    error: message.error.clone(),
                    citations: message.citations.clone(),
                    sequence: message.sequence,
                };
                conversation.active_request_id = None;
                store.write(&conversation)?;
                running.remove(&request_id);
                events.push((active.emit, event));
            }
            let result = action();
            let remaining = self
                .core
                .list_meetings()
                .map_err(|error| error.to_string())?;
            let mut cleanup = Ok(());
            if permanent {
                for id in ids
                    .iter()
                    .filter(|id| !remaining.iter().any(|meeting| &meeting.id == *id))
                {
                    if let Err(error) = store.purge(id) {
                        cleanup = Err(error);
                    }
                }
            }
            match result {
                Ok(value) => cleanup.map(|_| value),
                Err(error) => Err(error),
            }
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
    pub fn read(&self, meeting: &str) -> Result<Conversation, String> {
        let store = self.store.lock().unwrap();
        self.require_meeting(meeting)?;
        let mut conversation = store.read(meeting)?;
        let running = self.running.lock().unwrap();
        for message in &mut conversation.messages {
            if message.status == "streaming" {
                if let Some(active) = running.get(&message.request_id).filter(|active| {
                    active.meeting_id == meeting && active.message_id == message.id
                }) {
                    {
                        let partial = active.partial.lock().unwrap();
                        message.text = partial.text.clone();
                        message.sequence = partial.sequence;
                    }
                    conversation.active_request_id = Some(message.request_id.clone());
                } else {
                    message.status = "interrupted".into();
                    message.error = Some("답변이 중단되었습니다. 다시 시도해 주세요.".into());
                    conversation.active_request_id = None;
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
        let session = self.accounts.subscription_session(id).await?;
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
        self.require_meeting(&request.meeting_id)?;
        if request.message.trim().is_empty()
            || request.message.len() > 32 * 1024
            || request.source_ids.len() > 16
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
        let account = self
            .accounts
            .snapshot()?
            .accounts
            .into_iter()
            .find(|account| account.id == request.account_id && account.can_invoke)
            .ok_or("먼저 AI 계정을 연결해 주세요.")?;
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
        let prepared = context::prepare(
            &self.core,
            &self.root,
            &request.meeting_id,
            request.include_meeting,
            documents,
            request.template_id.as_deref(),
        )?;
        let store = self.store.lock().unwrap();
        self.require_meeting(&request.meeting_id)?;
        let mut running = self.running.lock().unwrap();
        if let Some(started) = existing_request(&store, &running, &request, &fingerprint)? {
            return Ok(started);
        }
        let mut conversation = store.read(&request.meeting_id)?;
        if running
            .values()
            .any(|active| active.meeting_id == request.meeting_id)
        {
            return Err("이 회의의 답변을 기다리거나 중지해 주세요.".into());
        }
        if running.len() >= 4 {
            return Err("진행 중인 답변이 끝난 뒤 다시 시도해 주세요.".into());
        }
        if conversation.id != request.conversation_id {
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
                    None,
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
        conversation.provider = Some(provider_id(account.provider).into());
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
            binding: None,
            saved_summary_id: None,
            note_saved: false,
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
            provider: Some(provider_id(account.provider).into()),
            model_id: Some(request.model_id.clone()),
            binding: Some(prepared.binding),
            saved_summary_id: None,
            note_saved: false,
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
                meeting_id: request.meeting_id.clone(),
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
        tauri::async_runtime::spawn(async move {
            let mut sequence = 0;
            let mut persistence_error = false;
            let result = async {
                let session = this
                    .accounts
                    .subscription_session_with_cancel(&request.account_id, cancel.clone())
                    .await?;
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
                    |delta| {
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
                            meeting_id: request.meeting_id.clone(),
                            message_id: message_id.clone(),
                            kind: "delta".into(),
                            text: Some(delta.into()),
                            error: None,
                            citations: vec![],
                            sequence,
                        });
                    },
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
            );
            if let Ok(Some(event)) = event {
                emit(event);
            } else if event.is_err() {
                emit(ChatEvent {
                    request_id: request.request_id.clone(),
                    conversation_id,
                    meeting_id: request.meeting_id,
                    message_id,
                    kind: "failed".into(),
                    text: Some(partial.lock().unwrap().text.clone()),
                    error: Some("AI 대화를 저장하지 못했습니다. 다시 열어 확인해 주세요.".into()),
                    citations: vec![],
                    sequence: sequence + 1,
                });
            }
        });
        Ok(started)
    }
    fn finish(
        &self,
        request_id: &str,
        result: Result<String, String>,
        persistence_error: bool,
        sequence: u64,
    ) -> Result<Option<ChatEvent>, String> {
        let store = self.store.lock().unwrap();
        let mut running = self.running.lock().unwrap();
        let Some(active) = running.remove(request_id) else {
            return Ok(None);
        };
        let cancelled = active.cancel.load(Ordering::Acquire) && !persistence_error;
        if !self
            .core
            .list_meetings()
            .map_err(|error| error.to_string())?
            .iter()
            .any(|meeting| meeting.id == active.meeting_id && meeting.deleted_at.is_none())
        {
            return Ok(None);
        }
        let mut conversation = store.read(&active.meeting_id)?;
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
        let event = ChatEvent {
            request_id: request_id.into(),
            conversation_id: active.conversation_id.clone(),
            meeting_id: active.meeting_id.clone(),
            message_id: message.id.clone(),
            kind: message.status.clone(),
            text: Some(message.text.clone()),
            error: message.error.clone(),
            citations: message.citations.clone(),
            sequence,
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
        meeting_id: &str,
        format: nero_core::DocumentFormat,
    ) -> Result<String, String> {
        store::id(conversation_id)?;
        store::id(message_id)?;
        let store = self.store.lock().unwrap();
        self.require_meeting(meeting_id)?;
        let conversation = store.read(meeting_id)?;
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
        let store = self.store.lock().unwrap();
        self.require_meeting(meeting_id)?;
        let mut conversation = store.read(meeting_id)?;
        if conversation.id.as_deref() != Some(conversation_id) {
            return Err("대화를 다시 확인해 주세요.".into());
        }
        let message = conversation
            .messages
            .iter_mut()
            .find(|message| {
                message.id == message_id
                    && message.role == "assistant"
                    && message.status == "completed"
            })
            .ok_or("완성된 답변을 선택해 주세요.")?;
        let binding: Binding = message
            .binding
            .clone()
            .ok_or("답변 자료를 확인하지 못했습니다.")?;
        if binding.meeting_id != meeting_id {
            return Err("다른 회의의 답변을 저장할 수 없습니다.".into());
        }
        if summary && message.saved_summary_id.is_some() {
            return Ok(json!({"summaryId":message.saved_summary_id}));
        }
        if !summary && message.note_saved {
            return Ok(json!({"applied":true}));
        }
        let text = context::summary_text(&message.text, &message.citations);
        let result = if summary {
            let context = binding
                .summary_context
                .ok_or("음성 기록을 포함한 답변만 회의 요약으로 저장할 수 있습니다.")?;
            let saved = self
                .core
                .import_summary_checked(
                    meeting_id,
                    &binding.meeting_version,
                    context,
                    text,
                    context::producer(
                        message.provider.as_deref().unwrap_or("AI"),
                        message.model_id.as_deref().unwrap_or("unknown"),
                        binding.template,
                    ),
                )
                .map_err(|e| e.to_string())?;
            message.saved_summary_id = Some(saved.id.clone());
            json!({"summaryId":saved.id})
        } else {
            let text = nero_core::presentation::format_summary(
                &text,
                nero_core::DocumentFormat::Md,
                false,
                false,
                None,
            );
            self.core
                .append_meeting_note(
                    meeting_id,
                    &binding.meeting_version,
                    binding.summary_context,
                    &text,
                )
                .map_err(|e| e.to_string())?;
            message.note_saved = true;
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
    fn active(meeting: &str, conversation: &str, message: &str) -> Active {
        Active {
            request_fingerprint: "fingerprint".into(),
            checkpoint: Arc::new(Mutex::new((Instant::now(), 0))),
            conversation_id: conversation.into(),
            meeting_id: meeting.into(),
            message_id: message.into(),
            account_id: "account".into(),
            cancel: Arc::new(AtomicBool::new(false)),
            partial: Arc::new(Mutex::new(Partial {
                text: "부분 답변".into(),
                sequence: 1,
            })),
            emit: Arc::new(|_| {}),
        }
    }
    fn seed(chat: &AiChat) -> (String, String, String, String) {
        let meeting = chat.core.create_meeting("Example", "").unwrap().id;
        let conversation = uuid::Uuid::new_v4().to_string();
        let request = uuid::Uuid::new_v4().to_string();
        let message = uuid::Uuid::new_v4().to_string();
        let mut document = Conversation::empty(&meeting);
        document.id = Some(conversation.clone());
        document.active_request_id = Some(request.clone());
        document.messages.push(Message {
            request_fingerprint: None,
            sequence: 0,
            id: message.clone(),
            request_id: request.clone(),
            role: "assistant".into(),
            text: String::new(),
            status: "streaming".into(),
            citations: vec![],
            error: None,
            provider: None,
            model_id: None,
            binding: None,
            saved_summary_id: None,
            note_saved: false,
        });
        chat.store.lock().unwrap().write(&document).unwrap();
        chat.running
            .lock()
            .unwrap()
            .insert(request.clone(), active(&meeting, &conversation, &message));
        (meeting, conversation, request, message)
    }
    fn capture_events(chat: &Arc<AiChat>, request: &str) -> Arc<Mutex<Vec<ChatEvent>>> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let received = events.clone();
        let weak_chat = Arc::downgrade(chat);
        chat.running.lock().unwrap().get_mut(request).unwrap().emit = Arc::new(move |event| {
            let chat = weak_chat.upgrade().unwrap();
            assert!(chat.store.try_lock().is_ok());
            assert!(chat.running.try_lock().is_ok());
            received.lock().unwrap().push(event);
        });
        events
    }
    #[test]
    fn cancellation_while_completion_waits_for_storage_remains_cancelled() {
        let (_root, chat) = service();
        let (meeting, _, request, _) = seed(&chat);
        let storage = chat.store.lock().unwrap();
        let worker_chat = chat.clone();
        let worker_request = request.clone();
        let (started, ready) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            started.send(()).unwrap();
            worker_chat.finish(&worker_request, Ok("late answer".into()), false, 2)
        });
        ready.recv().unwrap();
        chat.cancel(&request).unwrap();
        drop(storage);
        let event = worker.join().unwrap().unwrap().unwrap();
        assert_eq!(event.kind, "cancelled");
        assert_eq!(chat.read(&meeting).unwrap().messages[0].status, "cancelled");
    }
    #[test]
    fn copying_completed_answers_uses_shared_formats_without_internal_references_or_writes() {
        let (_root, chat) = service();
        let (meeting, conversation, _, message) = seed(&chat);
        assert!(chat
            .answer_document(
                &conversation,
                &message,
                &meeting,
                nero_core::DocumentFormat::Md
            )
            .is_err());
        let path = chat
            .root
            .join("meetings")
            .join(&meeting)
            .join("ai-conversation.json");
        {
            let store = chat.store.lock().unwrap();
            let mut document = store.read(&meeting).unwrap();
            let answer = &mut document.messages[0];
            answer.status = "completed".into();
            answer.text = "## 결정 사항\n\n- 일정을 확인합니다. [1]".into();
            answer.binding = Some(Binding {
                meeting_id: meeting.clone(),
                meeting_version: String::new(),
                summary_context: None,
                citations: vec![context::Citation {
                    reference: 1,
                    kind: "meeting".into(),
                    label: "00:01".into(),
                    meeting_id: Some(meeting.clone()),
                    segment_id: Some("segment".into()),
                    start: Some(1.0),
                    url: None,
                    source_id: None,
                }],
                template: None,
                input_sha256: String::new(),
            });
            store.write(&document).unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        let markdown = chat
            .answer_document(
                &conversation,
                &message,
                &meeting,
                nero_core::DocumentFormat::Md,
            )
            .unwrap();
        let text = chat
            .answer_document(
                &conversation,
                &message,
                &meeting,
                nero_core::DocumentFormat::Txt,
            )
            .unwrap();
        assert!(markdown.contains("## 결정 사항"));
        assert!(!markdown.contains("[1]"));
        assert!(text.contains("결정 사항"));
        assert!(!text.contains("##"));
        assert!(!text.contains("[1]"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    #[test]
    fn checkpoint_is_throttled_but_cancel_flushes_partial_text() {
        let (_root, chat) = service();
        let (meeting, _, request, _) = seed(&chat);
        chat.checkpoint(&request, false).unwrap();
        assert!(
            chat.store.lock().unwrap().read(&meeting).unwrap().messages[0]
                .text
                .is_empty()
        );
        chat.cancel_all();
        assert_eq!(
            chat.store.lock().unwrap().read(&meeting).unwrap().messages[0].text,
            "부분 답변"
        );
        chat.running.lock().unwrap().clear();
        let read = chat.read(&meeting).unwrap();
        assert_eq!(read.messages[0].status, "interrupted");
        assert_eq!(read.messages[0].text, "부분 답변");
    }
    #[test]
    fn final_read_failure_always_releases_running_ownership() {
        let (_root, chat) = service();
        let (meeting, _, request, _) = seed(&chat);
        let directory = chat.root.join("meetings").join(&meeting);
        std::fs::write(directory.join("ai-conversation.json"), b"broken").unwrap();
        assert!(chat
            .finish(&request, Err("failed".into()), false, 1)
            .is_err());
        assert!(!chat.is_busy());
    }
    #[test]
    fn permanent_removal_prevents_late_finish_from_recreating_conversation() {
        let (_root, chat) = service();
        let (meeting, _, request, _) = seed(&chat);
        let events = capture_events(&chat, &request);
        chat.core.trash_meetings(vec![meeting.clone()]).unwrap();
        chat.with_meeting_removal(&[meeting.clone()], true, || {
            chat.core
                .purge_meetings(vec![meeting.clone()])
                .map_err(|error| error.to_string())
        })
        .unwrap();
        assert!(chat
            .finish(&request, Ok("late answer".into()), false, 2)
            .unwrap()
            .is_none());
        assert!(chat
            .store
            .lock()
            .unwrap()
            .read(&meeting)
            .unwrap()
            .id
            .is_none());
        assert!(!chat.is_busy());
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].request_id, request);
        assert_eq!(events[0].kind, "cancelled");
        assert_eq!(events[0].text.as_deref(), Some("부분 답변"));
        assert_eq!(events[0].sequence, 2);
    }
    #[test]
    fn trash_and_restore_delivers_terminal_for_the_cached_active_request() {
        let (_root, chat) = service();
        let (meeting, conversation, request, message) = seed(&chat);
        let events = capture_events(&chat, &request);
        let cached = chat.read(&meeting).unwrap();
        assert_eq!(cached.active_request_id.as_deref(), Some(request.as_str()));
        assert_eq!(cached.messages[0].status, "streaming");
        chat.with_meeting_removal(&[meeting.clone()], false, || {
            chat.core
                .trash_meetings(vec![meeting.clone()])
                .map_err(|error| error.to_string())
        })
        .unwrap();
        chat.core.restore_meetings(vec![meeting.clone()]).unwrap();
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.request_id, cached.active_request_id.unwrap());
        assert_eq!(event.conversation_id, conversation);
        assert_eq!(event.meeting_id, meeting);
        assert_eq!(event.message_id, message);
        assert_eq!(event.kind, "cancelled");
        assert_eq!(event.text.as_deref(), Some("부분 답변"));
        assert!(event.sequence > cached.messages[0].sequence);
        let restored = chat.read(&meeting).unwrap();
        assert!(restored.active_request_id.is_none());
        assert_eq!(restored.messages[0].status, event.kind);
        assert_eq!(restored.messages[0].sequence, event.sequence);
        assert_eq!(restored.messages[0].text, event.text.as_deref().unwrap());
        assert!(chat
            .finish(&request, Ok("late answer".into()), false, 2)
            .unwrap()
            .is_none());
        assert!(!chat.is_busy());
    }
    #[test]
    fn failed_removal_retains_partial_and_the_existing_conversation() {
        let (_root, chat) = service();
        let (meeting, _, request, _) = seed(&chat);
        let events = capture_events(&chat, &request);
        let result: Result<(), String> =
            chat.with_meeting_removal(&[meeting.clone()], true, || Err("core rejected".into()));
        assert!(result.is_err());
        assert_eq!(
            chat.store.lock().unwrap().read(&meeting).unwrap().messages[0].text,
            "부분 답변"
        );
        assert!(!chat.running.lock().unwrap().contains_key(&request));
        assert!(chat
            .finish(&request, Ok("late answer".into()), false, 2)
            .unwrap()
            .is_none());
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].request_id, request);
        assert_eq!(events[0].kind, "cancelled");
        assert_eq!(events[0].text.as_deref(), Some("부분 답변"));
        assert_eq!(events[0].sequence, 2);
    }
    #[test]
    fn core_purge_without_chat_wrapper_also_removes_the_conversation() {
        let (_root, chat) = service();
        let (meeting, _, request, _) = seed(&chat);
        let path = chat
            .root
            .join("meetings")
            .join(&meeting)
            .join("ai-conversation.json");
        assert!(path.is_file());
        chat.core.trash_meetings(vec![meeting.clone()]).unwrap();
        chat.core.purge_meetings(vec![meeting.clone()]).unwrap();
        assert!(!path.exists());
        assert!(chat
            .finish(&request, Ok("late".into()), false, 2)
            .unwrap()
            .is_none());
        assert!(!path.exists());
    }
    #[test]
    fn logical_purge_with_cleanup_error_still_releases_jobs_and_removes_chat() {
        let (_root, chat) = service();
        let (meeting, _, request, _) = seed(&chat);
        chat.core.trash_meetings(vec![meeting.clone()]).unwrap();
        let directory = chat.root.join("meetings").join(&meeting);
        let preserved = chat.root.join("cleanup-failure-fixture");
        let result: Result<(), String> =
            chat.with_meeting_removal(&[meeting.clone()], true, || {
                std::fs::rename(&directory, &preserved).unwrap();
                chat.core
                    .purge_meetings(vec![meeting.clone()])
                    .map_err(|error| error.to_string())?;
                std::fs::rename(&preserved, &directory).unwrap();
                Err("physical cleanup failed".into())
            });
        assert!(result.is_err());
        assert!(!chat.is_busy());
        assert!(directory.join("meeting.json").exists());
        assert!(!directory.join("ai-conversation.json").exists());
        assert!(chat
            .finish(&request, Ok("late".into()), false, 2)
            .unwrap()
            .is_none());
        assert!(!directory.join("ai-conversation.json").exists());
        chat.core.retry_purge_cleanup().unwrap();
        assert!(!directory.exists());
    }
    #[test]
    fn request_identity_covers_sources_provider_context_and_meeting() {
        let mut request = SendRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            conversation_id: None,
            meeting_id: uuid::Uuid::new_v4().to_string(),
            include_meeting: true,
            account_id: "account".into(),
            model_id: "model".into(),
            message: "question".into(),
            source_ids: vec![],
            template_id: None,
        };
        let original = request_fingerprint(&request).unwrap();
        request.source_ids.push("source-revision".into());
        assert_ne!(original, request_fingerprint(&request).unwrap());
        request.source_ids.clear();
        request.include_meeting = false;
        assert_ne!(original, request_fingerprint(&request).unwrap());
        request.include_meeting = true;
        let store = tempfile::tempdir().unwrap();
        let storage = store::Store::new(store.path().into());
        let mut running = HashMap::new();
        let mut owner = active(
            &uuid::Uuid::new_v4().to_string(),
            &uuid::Uuid::new_v4().to_string(),
            &uuid::Uuid::new_v4().to_string(),
        );
        owner.request_fingerprint = original.clone();
        running.insert(request.request_id.clone(), owner);
        assert!(existing_request(&storage, &running, &request, &original).is_err());
    }
}
