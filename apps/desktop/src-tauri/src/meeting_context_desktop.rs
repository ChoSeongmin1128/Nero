use crate::{
    ai_chat_desktop::AiDesktop,
    meeting_context::{MeetingContext, MeetingContextStore},
    notion_context::NotionSource,
    require_main, DesktopState,
};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, WebviewWindow};

pub struct MeetingContexts {
    store: MeetingContextStore,
    gate: Mutex<()>,
    core: nero_core::CoreHandle,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    source_id: String,
    page_id: String,
    title: Option<String>,
    url: String,
    unavailable: bool,
}
impl From<NotionSource> for SourceView {
    fn from(source: NotionSource) -> Self {
        Self {
            source_id: source.source_id,
            page_id: source.page_id,
            title: source.title,
            url: source.url,
            unavailable: false,
        }
    }
}
#[derive(Serialize)]
pub struct SourcesView {
    revision: u64,
    sources: Vec<SourceView>,
}
impl MeetingContexts {
    pub fn new(root: std::path::PathBuf, core: nero_core::CoreHandle) -> Result<Self, String> {
        Ok(Self {
            store: MeetingContextStore::new(root)?,
            gate: Mutex::new(()),
            core,
        })
    }
    fn require_meeting(&self, id: &str) -> Result<(), String> {
        if self
            .core
            .list_meetings()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|m| m.id == id && m.deleted_at.is_none())
        {
            Ok(())
        } else {
            Err("회의를 찾을 수 없습니다.".into())
        }
    }
    fn view(
        &self,
        context: MeetingContext,
        notion: &crate::notion_context::NotionContextService,
    ) -> Result<SourcesView, String> {
        Ok(hydrate(context, |id| {
            notion
                .read_sources(vec![id.to_owned()])
                .ok()?
                .sources
                .into_iter()
                .next()
        }))
    }
}
fn hydrate(
    context: MeetingContext,
    mut load: impl FnMut(&str) -> Option<NotionSource>,
) -> SourcesView {
    let sources = context
        .sources
        .into_iter()
        .map(|reference| {
            load(&reference.source_id)
                .map(SourceView::from)
                .unwrap_or_else(|| SourceView {
                    source_id: reference.source_id,
                    url: format!(
                        "https://www.notion.so/{}",
                        reference.page_id.replace('-', "")
                    ),
                    page_id: reference.page_id,
                    title: None,
                    unavailable: true,
                })
        })
        .collect();
    SourcesView {
        revision: context.revision,
        sources,
    }
}
fn service(app: &AppHandle) -> Result<Arc<MeetingContexts>, String> {
    app.try_state::<Arc<MeetingContexts>>()
        .map(|state| state.inner().clone())
        .ok_or("회의 자료를 준비하지 못했습니다.".into())
}
pub(crate) fn with_meeting<T>(
    app: &AppHandle,
    meeting_id: &str,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let state = service(app)?;
    let _guard = state
        .gate
        .lock()
        .map_err(|_| "회의 자료를 확인하지 못했습니다.")?;
    state.require_meeting(meeting_id)?;
    action()
}
pub fn removal<T>(
    app: &AppHandle,
    ids: &[String],
    permanent: bool,
    action: impl FnOnce() -> nero_core::Result<T>,
) -> nero_core::Result<T> {
    let Some(state) = app.try_state::<Arc<MeetingContexts>>() else {
        return action();
    };
    let _guard = state.gate.lock().map_err(|_| {
        nero_core::CoreError::InvalidState("회의 자료를 확인하지 못했습니다.".into())
    })?;
    let result = action();
    if permanent {
        // The meetings are already gone, so leftover context files of one meeting
        // must neither stop the others' cleanup nor report the removal as failed.
        let Ok(remaining) = state.core.list_meetings() else {
            return result;
        };
        let root = &app.state::<DesktopState>().processing.root;
        for id in ids
            .iter()
            .filter(|id| !remaining.iter().any(|m| &m.id == *id))
        {
            let cleanup = [
                state.store.delete(id),
                crate::domain_glossary::delete(root, id),
                crate::glossary_context::delete_meeting(root, id),
            ];
            if cleanup.iter().any(Result::is_err) {
                eprintln!("Nero: 삭제한 회의의 자료 일부를 정리하지 못했습니다.");
            }
        }
    }
    result
}
pub(crate) enum SourceChange {
    Add(String),
    Refresh(String),
    Remove(String),
}
// The context panel and agent requests share these; they block, so screens run them off
// the async runtime.
pub(crate) fn sources(app: &AppHandle, meeting_id: &str) -> Result<SourcesView, String> {
    let _lease = app.state::<DesktopState>().lifecycle.begin_transition()?;
    let store = service(app)?;
    let notion = app.state::<AiDesktop>().notion()?;
    let _guard = store
        .gate
        .lock()
        .map_err(|_| "회의 자료를 확인하지 못했습니다.")?;
    store.require_meeting(meeting_id)?;
    store.view(store.store.list(meeting_id)?, &notion)
}
pub(crate) fn change_sources(
    app: &AppHandle,
    meeting_id: &str,
    expected_revision: u64,
    change: SourceChange,
) -> Result<SourcesView, String> {
    let _lease = app.state::<DesktopState>().lifecycle.begin_transition()?;
    let store = service(app)?;
    let notion = app.state::<AiDesktop>().notion()?;
    let _guard = store
        .gate
        .lock()
        .map_err(|_| "회의 자료를 확인하지 못했습니다.")?;
    store.require_meeting(meeting_id)?;
    let read = |source_id: String| -> Result<NotionSource, String> {
        notion
            .read_sources(vec![source_id])
            .map_err(|e| e.to_string())?
            .sources
            .pop()
            .ok_or("자료를 찾을 수 없습니다.".into())
    };
    let context = match change {
        SourceChange::Add(source_id) => {
            store
                .store
                .add(meeting_id, expected_revision, &read(source_id)?)?
        }
        SourceChange::Refresh(source_id) => {
            store
                .store
                .replace(meeting_id, expected_revision, &read(source_id)?)?
        }
        SourceChange::Remove(page_id) => {
            store
                .store
                .remove(meeting_id, expected_revision, &page_id)?
        }
    };
    store.view(context, &notion)
}
impl SourcesView {
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
    pub(crate) fn url_of(&self, page_id: &str) -> Option<&str> {
        self.sources
            .iter()
            .find(|source| source.page_id == page_id)
            .map(|source| source.url.as_str())
    }
}
async fn blocking<T: Send + 'static>(
    action: impl FnOnce() -> Result<T, String> + Send + 'static,
    failure: &str,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(action)
        .await
        .map_err(|_| failure.to_string())?
}
#[tauri::command]
pub async fn meeting_context_sources(
    window: WebviewWindow,
    app: AppHandle,
    meeting_id: String,
) -> Result<SourcesView, String> {
    require_main(&window)?;
    blocking(
        move || sources(&app, &meeting_id),
        "회의 자료를 불러오지 못했습니다.",
    )
    .await
}
#[tauri::command]
pub async fn meeting_context_add(
    window: WebviewWindow,
    app: AppHandle,
    meeting_id: String,
    expected_revision: u64,
    source_id: String,
) -> Result<SourcesView, String> {
    require_main(&window)?;
    blocking(
        move || {
            change_sources(
                &app,
                &meeting_id,
                expected_revision,
                SourceChange::Add(source_id),
            )
        },
        "회의 자료를 저장하지 못했습니다.",
    )
    .await
}
#[tauri::command]
pub async fn meeting_context_remove(
    window: WebviewWindow,
    app: AppHandle,
    meeting_id: String,
    expected_revision: u64,
    page_id: String,
) -> Result<SourcesView, String> {
    require_main(&window)?;
    blocking(
        move || {
            change_sources(
                &app,
                &meeting_id,
                expected_revision,
                SourceChange::Remove(page_id),
            )
        },
        "회의 자료를 저장하지 못했습니다.",
    )
    .await
}
#[tauri::command]
pub async fn meeting_context_refresh(
    window: WebviewWindow,
    app: AppHandle,
    meeting_id: String,
    expected_revision: u64,
    source_id: String,
) -> Result<SourcesView, String> {
    require_main(&window)?;
    blocking(
        move || {
            change_sources(
                &app,
                &meeting_id,
                expected_revision,
                SourceChange::Refresh(source_id),
            )
        },
        "회의 자료를 저장하지 못했습니다.",
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting_context::ContextReference;
    use std::collections::HashMap;

    fn source(page_id: &str, body: &str) -> NotionSource {
        NotionSource {
            source_id: format!("notion:{page_id}:{}", "a".repeat(64)),
            page_id: page_id.into(),
            url: format!("https://www.notion.so/{page_id}"),
            title: Some("자료".into()),
            last_edited_at: None,
            fetched_at: 0,
            sha256: String::new(),
            body: body.into(),
            truncated: false,
            unknown_block_count: None,
        }
    }

    #[test]
    fn metadata_is_hydrated_individually_and_missing_reference_can_be_removed() {
        let temp = tempfile::tempdir().unwrap();
        let store = MeetingContextStore::new(temp.path().canonicalize().unwrap()).unwrap();
        let meeting = "a0000000-0000-4000-8000-000000000001";
        let body = "가".repeat(70_000);
        let sources: Vec<_> = (1..=3)
            .map(|n| source(&format!("b0000000-0000-4000-8000-{n:012}"), &body))
            .collect();
        assert!(
            sources
                .iter()
                .map(|source| source.body.len())
                .sum::<usize>()
                > 512 * 1024
        );
        let mut context = MeetingContext::default();
        for source in &sources {
            context = store.add(meeting, context.revision, source).unwrap();
        }
        let cached: HashMap<_, _> = sources
            .iter()
            .map(|s| (s.source_id.clone(), s.clone()))
            .collect();
        let view = hydrate(context.clone(), |id| cached.get(id).cloned());
        assert_eq!(view.sources.len(), 3);
        assert!(view.sources.iter().all(|source| !source.unavailable));
        let absent = &sources[1].source_id;
        let view = hydrate(context, |id| {
            if id == absent {
                None
            } else {
                cached.get(id).cloned()
            }
        });
        assert!(view.sources[1].unavailable);
        let after = store
            .remove(meeting, view.revision, &view.sources[1].page_id)
            .unwrap();
        assert_eq!(after.sources.len(), 2);
        assert!(!after
            .sources
            .contains(&ContextReference::notion(&sources[1])));
    }
}
