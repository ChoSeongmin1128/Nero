use crate::{
    ai_accounts::{AiAccounts, Provider},
    ai_chat::{self, AiChat},
    notion_context::{self, NotionContextService},
    require_main, DesktopState,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{atomic::AtomicBool, Arc};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

pub struct AiDesktop {
    pub chat: Option<Arc<AiChat>>,
    pub accounts: Option<Arc<AiAccounts>>,
    pub notion: Option<Arc<NotionContextService>>,
    account_error: Option<String>,
    notion_error: Option<String>,
}
impl AiDesktop {
    pub(crate) fn chat(&self) -> Result<Arc<AiChat>, String> {
        self.chat.clone().ok_or_else(|| {
            self.account_error
                .clone()
                .unwrap_or("AI 대화를 준비하지 못했습니다.".into())
        })
    }
    pub(crate) fn accounts(&self) -> Result<Arc<AiAccounts>, String> {
        self.accounts.clone().ok_or_else(|| {
            self.account_error
                .clone()
                .unwrap_or("AI 연결을 준비하지 못했습니다.".into())
        })
    }
    pub(crate) fn notion(&self) -> Result<Arc<NotionContextService>, String> {
        self.notion.clone().ok_or_else(|| {
            self.notion_error
                .clone()
                .unwrap_or("Notion 연결을 준비하지 못했습니다.".into())
        })
    }
}
pub fn install(app: &AppHandle, root: PathBuf, resources: PathBuf, core: nero_core::CoreHandle) {
    if let Ok(contexts) =
        crate::meeting_context_desktop::MeetingContexts::new(root.clone(), core.clone())
    {
        app.manage(Arc::new(contexts));
    }
    let accounts = AiAccounts::new(root.join("ai-accounts"), resources);
    let notion = NotionContextService::new(root.clone());
    let account_error = accounts.as_ref().err().cloned();
    let notion_error = notion.as_ref().err().map(ToString::to_string);
    let accounts = accounts.ok().map(Arc::new);
    let notion = notion.ok().map(Arc::new);
    let chat = accounts
        .as_ref()
        .map(|accounts| Arc::new(AiChat::new(root, core, accounts.clone(), notion.clone())));
    app.manage(AiDesktop {
        chat,
        accounts,
        notion,
        account_error,
        notion_error,
    });
}
pub struct AiShutdown {
    _claude: Option<crate::claude_connection::ShutdownGuard>,
    _accounts: Option<crate::ai_accounts::ShutdownGuard>,
    _notion: Option<crate::notion_context::NotionShutdownGuard>,
}
pub async fn prepare_shutdown(app: &AppHandle) -> Result<AiShutdown, String> {
    let (chat, accounts, notion) = app
        .try_state::<AiDesktop>()
        .map(|state| {
            (
                state.chat.clone(),
                state.accounts.clone(),
                state.notion.clone(),
            )
        })
        .unwrap_or_default();
    let claude = match &chat {
        Some(chat) => Some(chat.claude.prepare_shutdown().await?),
        None => None,
    };
    if let Some(chat) = chat {
        tauri::async_runtime::spawn_blocking(move || chat.drain(std::time::Duration::from_secs(5)))
            .await
            .map_err(|_| "AI 대화를 저장하지 못했습니다.".to_string())??;
    }
    let accounts = match accounts {
        Some(accounts) => Some(accounts.prepare_shutdown().await?),
        None => None,
    };
    let notion = match notion {
        Some(notion) => Some(notion.prepare_shutdown().await.map_err(|e| e.to_string())?),
        None => None,
    };
    Ok(AiShutdown {
        _claude: claude,
        _accounts: accounts,
        _notion: notion,
    })
}
pub fn meeting_removal<T>(
    app: &AppHandle,
    ids: &[String],
    permanent: bool,
    action: impl FnOnce() -> nero_core::Result<T>,
) -> nero_core::Result<T> {
    let chat = app
        .try_state::<AiDesktop>()
        .and_then(|state| state.chat.clone());
    crate::meeting_context_desktop::removal(app, ids, permanent, || match chat {
        Some(chat) => chat
            .with_meeting_removal(ids, permanent, || action().map_err(|e| e.to_string()))
            .map_err(nero_core::CoreError::InvalidState),
        None => action(),
    })
}
fn require_open(app: &AppHandle) -> Result<(), String> {
    if app.state::<DesktopState>().lifecycle.is_closing() {
        return Err("Nero를 종료하거나 업데이트하고 있습니다.".into());
    }
    Ok(())
}
pub(crate) fn account_view(
    accounts: &AiAccounts,
    claude: &crate::claude_connection::ClaudeConnection,
) -> Result<Value, String> {
    let snapshot = accounts.snapshot()?;
    let mut values = vec![];
    for account in snapshot.accounts {
        if account.provider == Provider::Claude {
            continue;
        }
        let state = if account.can_invoke {
            "ready"
        } else if account.state == "connecting" {
            "connecting"
        } else if account.state == "plan_disabled" {
            "plan_disabled"
        } else if account.state == "unknown" {
            "unknown"
        } else {
            "signed_out"
        };
        values.push(json!({"id":account.id,"provider":account.provider,"state":state,"label":account.label,"configured":account.configured,"detail":account.reason}));
    }
    for provider in &snapshot.providers {
        if provider.id == Provider::Claude {
            continue;
        }
        if !values
            .iter()
            .any(|account| account["provider"] == serde_json::to_value(provider.id).unwrap())
        {
            values.push(json!({"id":"","provider":provider.id,"state":if !provider.configured{"unavailable"}else if snapshot.checked{"signed_out"}else{"unknown"},"label":provider.name,"configured":provider.configured,"detail":provider.reason}));
        }
    }
    values.push(claude.snapshot());
    Ok(
        json!({"accounts":values,"providers":snapshot.providers,"activeAccountId":snapshot.active_account_id,"checked":snapshot.checked}),
    )
}
async fn account_snapshot(state: &AiDesktop) -> Result<Value, String> {
    let accounts = state.accounts()?;
    let claude = state.chat()?.claude.clone();
    tauri::async_runtime::spawn_blocking(move || account_view(&accounts, &claude))
        .await
        .map_err(|_| "AI 상태를 확인하지 못했습니다.".to_string())?
}
async fn open(url: String) -> Result<(), String> {
    let url = reqwest::Url::parse(&url).map_err(|_| "로그인 주소를 확인하지 못했습니다.")?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some(
                "auth.openai.com"
                    | "claude.ai"
                    | "platform.claude.com"
                    | "mcp.notion.com"
                    | "www.notion.so"
                    | "notion.so"
                    | "api.notion.com"
            )
        )
    {
        return Err("로그인 주소를 확인하지 못했습니다.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        crate::external_open::open_url(&url)
            .map_err(|_| "기본 브라우저를 열지 못했습니다.".to_string())
    })
    .await
    .map_err(|_| "기본 브라우저를 열지 못했습니다.".to_string())?
}
#[tauri::command]
pub async fn ai_chat_accounts(
    window: WebviewWindow,
    state: State<'_, AiDesktop>,
) -> Result<Value, String> {
    require_main(&window)?;
    account_snapshot(&state).await
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    attempt_id: String,
    restored: bool,
}
#[tauri::command]
pub async fn ai_chat_connect(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AiDesktop>,
    provider: Provider,
    account_id: Option<String>,
) -> Result<Attempt, String> {
    require_main(&window)?;
    require_open(&app)?;
    if provider == Provider::Claude {
        return Ok(Attempt {
            attempt_id: state.chat()?.claude.begin()?,
            restored: false,
        });
    }
    let accounts = state.accounts()?;
    let restore = accounts.clone();
    let restored =
        tauri::async_runtime::spawn_blocking(move || restore.resolve_accounts_interactive())
            .await
            .map_err(|_| "AI 연결을 확인하지 못했습니다.".to_string())?;
    match restored {
        Ok(snapshot)
            if snapshot.accounts.iter().any(|account| {
                account.provider == provider
                    && account.can_invoke
                    && account_id.as_ref().is_none_or(|id| &account.id == id)
            }) =>
        {
            return Ok(Attempt {
                attempt_id: String::new(),
                restored: true,
            });
        }
        Err(error) if !accounts.legacy_recovery_required() => return Err(error),
        _ => {}
    }
    let signing = accounts.clone();
    let attempt =
        tauri::async_runtime::spawn_blocking(move || signing.begin_sign_in(provider, account_id))
            .await
            .map_err(|_| "AI 로그인을 시작하지 못했습니다.".to_string())??;
    if let Err(error) = open(attempt.authorization_url).await {
        let _ = accounts.cancel_sign_in(&attempt.id);
        return Err(error);
    }
    Ok(Attempt {
        attempt_id: attempt.id,
        restored: false,
    })
}
#[tauri::command]
pub async fn ai_chat_finish_connect(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AiDesktop>,
    attempt_id: String,
) -> Result<Value, String> {
    require_main(&window)?;
    require_open(&app)?;
    let claude = state.chat()?.claude.clone();
    if claude.owns(&attempt_id) {
        claude.finish(&attempt_id).await?;
        return account_snapshot(&state).await;
    }
    let accounts = state.accounts()?;
    accounts
        .complete_sign_in(&attempt_id, Arc::new(AtomicBool::new(false)))
        .await?;
    account_snapshot(&state).await
}
#[tauri::command]
pub async fn ai_chat_cancel_connect(
    window: WebviewWindow,
    state: State<'_, AiDesktop>,
    attempt_id: String,
) -> Result<Value, String> {
    require_main(&window)?;
    let claude = state.chat()?.claude.clone();
    if claude.owns(&attempt_id) {
        claude.cancel(&attempt_id).await?;
        return account_snapshot(&state).await;
    }
    let accounts = state.accounts()?;
    accounts.cancel_sign_in(&attempt_id)?;
    account_snapshot(&state).await
}
#[tauri::command]
pub async fn ai_chat_disconnect(
    window: WebviewWindow,
    state: State<'_, AiDesktop>,
    account_id: String,
) -> Result<Value, String> {
    require_main(&window)?;
    if let Some(chat) = &state.chat {
        chat.cancel_account(&account_id);
    }
    let accounts = state.accounts()?;
    accounts.sign_out(&account_id).await?;
    account_snapshot(&state).await
}
#[tauri::command]
pub fn ai_chat_composer_catalog(window: WebviewWindow, app: AppHandle) -> Result<Value, String> {
    require_main(&window)?;
    require_open(&app)?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|_| "템플릿 위치를 확인하지 못했습니다.".to_string())?;
    ai_chat::composer_catalog(&root)
}
#[tauri::command]
pub async fn ai_chat_models(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AiDesktop>,
    account_id: String,
    refresh: Option<bool>,
) -> Result<ai_chat::Models, String> {
    require_main(&window)?;
    require_open(&app)?;
    if refresh.unwrap_or(false) {
        state.chat()?.models_refresh(&account_id).await
    } else {
        state.chat()?.models(&account_id).await
    }
}
#[tauri::command]
pub async fn ai_chat_list(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AiDesktop>,
    trash: Option<bool>,
) -> Result<Vec<ai_chat::ConversationSummary>, String> {
    require_main(&window)?;
    let transition = app.state::<DesktopState>().lifecycle.begin_transition()?;
    let chat = state.chat()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _transition = transition;
        chat.list(trash.unwrap_or(false))
    })
    .await
    .map_err(|_| "대화 목록을 불러오지 못했습니다.".to_string())?
}
#[tauri::command]
pub async fn ai_chat_create(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AiDesktop>,
) -> Result<ai_chat::Conversation, String> {
    require_main(&window)?;
    let transition = app.state::<DesktopState>().lifecycle.begin_transition()?;
    let chat = state.chat()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _transition = transition;
        chat.create()
    })
    .await
    .map_err(|_| "새 대화를 만들지 못했습니다.".to_string())?
}
#[tauri::command]
pub async fn ai_chat_delete(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AiDesktop>,
    conversation_id: String,
) -> Result<(), String> {
    require_main(&window)?;
    let transition = app.state::<DesktopState>().lifecycle.begin_transition()?;
    let chat = state.chat()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _transition = transition;
        chat.delete(&conversation_id)
    })
    .await
    .map_err(|_| "대화를 삭제하지 못했습니다.".to_string())?
}
#[derive(Clone, Copy)]
enum TrashAction {
    Trash,
    Restore,
    Purge,
}
async fn trash_action(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AiDesktop>,
    conversation_ids: Vec<String>,
    action: TrashAction,
) -> Result<Vec<String>, String> {
    require_main(&window)?;
    if conversation_ids.is_empty() || conversation_ids.len() > 10_000 {
        return Err("정리할 대화를 선택해 주세요.".into());
    }
    let transition = app.state::<DesktopState>().lifecycle.begin_transition()?;
    let chat = state.chat()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _transition = transition;
        match action {
            TrashAction::Trash => chat.trash(&conversation_ids, true),
            TrashAction::Restore => chat.restore(&conversation_ids).map(|()| conversation_ids),
            TrashAction::Purge => chat.purge(&conversation_ids).map(|()| conversation_ids),
        }
    })
    .await
    .map_err(|_| "대화를 정리하지 못했습니다.".to_string())?
}
#[tauri::command]
pub async fn ai_chat_trash(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AiDesktop>,
    conversation_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    trash_action(window, app, state, conversation_ids, TrashAction::Trash).await
}
#[tauri::command]
pub async fn ai_chat_restore(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AiDesktop>,
    conversation_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    trash_action(window, app, state, conversation_ids, TrashAction::Restore).await
}
#[tauri::command]
pub async fn ai_chat_purge(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AiDesktop>,
    conversation_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    trash_action(window, app, state, conversation_ids, TrashAction::Purge).await
}
#[tauri::command]
pub async fn ai_chat_select_version(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AiDesktop>,
    conversation_id: String,
    message_id: String,
) -> Result<ai_chat::Conversation, String> {
    require_main(&window)?;
    let transition = app.state::<DesktopState>().lifecycle.begin_transition()?;
    let chat = state.chat()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _transition = transition;
        chat.select_version(&conversation_id, &message_id)
    })
    .await
    .map_err(|_| "질문 버전을 바꾸지 못했습니다.".to_string())?
}
#[tauri::command]
pub async fn ai_chat_read(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AiDesktop>,
    conversation_id: String,
) -> Result<ai_chat::Conversation, String> {
    require_main(&window)?;
    let transition = app.state::<DesktopState>().lifecycle.begin_transition()?;
    let chat = state.chat()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _transition = transition;
        chat.read(&conversation_id)
    })
    .await
    .map_err(|_| "대화를 불러오지 못했습니다.".to_string())?
}
#[tauri::command]
pub async fn ai_chat_send(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AiDesktop>,
    request_id: String,
    conversation_id: String,
    meeting_ids: Vec<String>,
    account_id: String,
    model_id: String,
    effort: Option<String>,
    message: String,
    source_ids: Vec<String>,
    source_origins: Option<std::collections::BTreeMap<String, ai_chat::SourceOrigin>>,
    template_id: Option<String>,
    skill_id: Option<String>,
    summary_target_id: Option<String>,
    edit_of: Option<String>,
    parent_id: Option<String>,
) -> Result<ai_chat::Started, String> {
    require_main(&window)?;
    let transition = app.state::<DesktopState>().lifecycle.begin_transition()?;
    let chat = state.chat()?;
    let request = ai_chat::SendRequest {
        request_id,
        conversation_id,
        meeting_ids,
        account_id,
        model_id,
        effort,
        message,
        source_ids,
        source_origins,
        template_id,
        skill_id,
        summary_target_id,
        edit_of,
        parent_id,
    };
    tauri::async_runtime::spawn_blocking(move || {
        let _transition = transition;
        chat.start(request, chat_events(&app))
    })
    .await
    .map_err(|_| "대화를 시작하지 못했습니다.".to_string())?
}
#[tauri::command]
pub fn ai_chat_cancel(
    window: WebviewWindow,
    state: State<'_, AiDesktop>,
    request_id: String,
) -> Result<(), String> {
    require_main(&window)?;
    state.chat()?.cancel(&request_id)
}
// Saving an answer to a meeting, for the answer buttons and agent requests. It blocks.
pub(crate) fn save_answer(
    app: &AppHandle,
    conversation_id: &str,
    message_id: &str,
    meeting_id: &str,
    summary: bool,
) -> Result<Value, String> {
    let state = app.state::<DesktopState>();
    let _transition = state.lifecycle.begin_transition()?;
    let chat = app.state::<AiDesktop>().chat()?;
    let result = if summary {
        chat.save_summary(conversation_id, message_id, meeting_id)
    } else {
        chat.append_note(conversation_id, message_id, meeting_id)
    }?;
    if let Ok(snapshot) = state.core.snapshot() {
        crate::publish(app, &snapshot);
    }
    Ok(result)
}
async fn apply_answer(
    app: AppHandle,
    conversation_id: String,
    message_id: String,
    meeting_id: String,
    summary: bool,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        save_answer(&app, &conversation_id, &message_id, &meeting_id, summary)
    })
    .await
    .map_err(|_| "답변을 저장하지 못했습니다.".to_string())?
}
// The chat events the AI panel listens to, for requests from the panel or an agent.
pub(crate) fn chat_events(app: &AppHandle) -> Arc<dyn Fn(ai_chat::ChatEvent) + Send + Sync> {
    let app = app.clone();
    Arc::new(move |event| {
        let _ = app.emit_to("main", "nero://ai-chat", event);
    })
}
#[tauri::command]
pub async fn ai_chat_save_summary(
    window: WebviewWindow,
    app: AppHandle,
    conversation_id: String,
    message_id: String,
    meeting_id: String,
) -> Result<Value, String> {
    require_main(&window)?;
    apply_answer(app, conversation_id, message_id, meeting_id, true).await
}
#[tauri::command]
pub async fn ai_chat_append_note(
    window: WebviewWindow,
    app: AppHandle,
    conversation_id: String,
    message_id: String,
    meeting_id: String,
) -> Result<Value, String> {
    require_main(&window)?;
    apply_answer(app, conversation_id, message_id, meeting_id, false).await
}
#[tauri::command]
pub async fn ai_chat_open_source(window: WebviewWindow, url: String) -> Result<(), String> {
    require_main(&window)?;
    let url = reqwest::Url::parse(&url).map_err(|_| "자료 주소를 확인해 주세요.")?;
    crate::notion_context::normalize_page_id(url.as_str())
        .map_err(|_| "자료 주소를 확인해 주세요.")?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::external_open::open_url(&url).map_err(|_| "자료를 열지 못했습니다.".to_string())
    })
    .await
    .map_err(|_| "자료를 열지 못했습니다.".to_string())?
}
#[tauri::command]
pub fn notion_connection_snapshot(
    window: WebviewWindow,
    state: State<'_, AiDesktop>,
) -> Result<notion_context::NotionConnectionSnapshot, String> {
    require_main(&window)?;
    Ok(state.notion()?.snapshot())
}
#[tauri::command]
pub async fn begin_notion_connect(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AiDesktop>,
) -> Result<Value, String> {
    require_main(&window)?;
    require_open(&app)?;
    let service = state.notion()?;
    let request = service.begin_connect().await.map_err(|e| e.to_string())?;
    if let Err(error) = open(request.authorization_url).await {
        service.cancel_connect(Some(&request.attempt_id));
        return Err(error);
    }
    Ok(json!({"attemptId":request.attempt_id}))
}
#[tauri::command]
pub async fn finish_notion_connect(
    window: WebviewWindow,
    state: State<'_, AiDesktop>,
    attempt_id: String,
) -> Result<notion_context::NotionConnectionSnapshot, String> {
    require_main(&window)?;
    state
        .notion()?
        .finish_connect(&attempt_id)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn cancel_notion_connect(
    window: WebviewWindow,
    state: State<'_, AiDesktop>,
    attempt_id: Option<String>,
) -> Result<notion_context::NotionConnectionSnapshot, String> {
    require_main(&window)?;
    let service = state.notion()?;
    service.cancel_connect(attempt_id.as_deref());
    Ok(service.snapshot())
}
#[tauri::command]
pub async fn disconnect_notion_context(
    window: WebviewWindow,
    state: State<'_, AiDesktop>,
) -> Result<notion_context::NotionConnectionSnapshot, String> {
    require_main(&window)?;
    state
        .notion()?
        .disconnect()
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn fetch_notion_sources(
    window: WebviewWindow,
    state: State<'_, AiDesktop>,
    request_id: String,
    links: Vec<String>,
) -> Result<notion_context::NotionContextBundle, String> {
    require_main(&window)?;
    state
        .notion()?
        .fetch_sources(request_id, links)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn cancel_notion_fetch(
    window: WebviewWindow,
    state: State<'_, AiDesktop>,
    request_id: String,
) -> Result<(), String> {
    require_main(&window)?;
    state.notion()?.cancel_fetch(&request_id);
    Ok(())
}
#[tauri::command]
pub fn read_notion_sources(
    window: WebviewWindow,
    state: State<'_, AiDesktop>,
    source_ids: Vec<String>,
) -> Result<notion_context::NotionContextBundle, String> {
    require_main(&window)?;
    state
        .notion()?
        .read_sources(source_ids)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ai_chat_answer_document(
    window: WebviewWindow,
    state: State<'_, AiDesktop>,
    conversation_id: String,
    message_id: String,
    format: nero_core::DocumentFormat,
) -> Result<Value, String> {
    require_main(&window)?;
    let chat = state.chat()?;
    let text = tauri::async_runtime::spawn_blocking(move || {
        chat.answer_document(&conversation_id, &message_id, format)
    })
    .await
    .map_err(|_| "답변을 복사하지 못했습니다.".to_string())??;
    Ok(json!({"text":text,"format":format}))
}
