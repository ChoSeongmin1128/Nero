#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent_connection;
#[cfg(target_os = "macos")]
mod app_menu;
mod asr;
mod diarization;
mod downloads;
mod export;
mod job_journal;
mod memory;
mod microphone_permission;
mod processing;
mod recorder_geometry;
mod recorder_policy;
mod speaker_worker;
mod speech_audio;
mod storage_space;
mod summary;
mod summary_failure;
mod terminology;
use grove_core::{
    CoreHandle, Device, Meeting, Snapshot, SummaryCitations, SummaryView, Transcript,
    TranscriptHistory, TranscriptSpeaker, TranscriptVersion,
};
use processing::{Processing, Progress};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;

struct DesktopState {
    core: CoreHandle,
    quitting: Arc<AtomicBool>,
    polling: Arc<AtomicBool>,
    processing: Arc<Processing>,
    recorder: Mutex<recorder_policy::RecorderPolicy>,
}

fn require_main(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("본창에서 실행해 주세요.".into())
    }
}

#[tauri::command]
fn agent_connection_info(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> Result<agent_connection::ConnectionInfo, String> {
    require_main(&window)?;
    let resources = app
        .path()
        .resource_dir()
        .map_err(|error| error.to_string())?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    Ok(agent_connection::connection_info(&resources, &root))
}

fn require_local_window(window: &WebviewWindow) -> Result<(), String> {
    if matches!(window.label(), "main" | "recorder") {
        Ok(())
    } else {
        Err("허용되지 않은 창입니다.".into())
    }
}

async fn on_core<T: Send + 'static>(
    core: CoreHandle,
    action: impl FnOnce(CoreHandle) -> Result<T, grove_core::CoreError> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || action(core))
        .await
        .map_err(|_| "작업을 완료하지 못했습니다. 다시 시도해 주세요.".to_string())?
        .map_err(|error| error.to_string())
}

fn publish(app: &tauri::AppHandle, snapshot: &Snapshot) {
    let _ = app.emit("grove://snapshot", snapshot);
    sync_recorder(app, snapshot);
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    request_recorder_sync(app);
}

fn request_recorder_sync(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<DesktopState>() else {
        return;
    };
    let core = state.core.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Ok(snapshot) = on_core(core, |core| core.snapshot()).await {
            sync_recorder(&app, &snapshot);
        }
    });
}

fn snapshot_changed(previous: Option<&Snapshot>, snapshot: &Snapshot) -> bool {
    previous.map_or(true, |value| {
        value.meetings != snapshot.meetings
            || value.folders != snapshot.folders
            || value.session != snapshot.session
    })
}

fn show_recorder_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("recorder") {
        return window.show().map_err(recorder_window_error);
    }
    let mut builder =
        WebviewWindowBuilder::new(app, "recorder", WebviewUrl::App("recorder.html".into()))
            .title("Nero 녹음 제어")
            .inner_size(236.0, 44.0)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false);
    if let Some(monitor) = app
        .get_webview_window("main")
        .and_then(|main| main.current_monitor().ok().flatten())
    {
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
        let x = (f64::from(area.position.x) + f64::from(area.size.width) / 2.0) / scale - 118.0;
        let y = f64::from(area.position.y) / scale + 16.0;
        builder = builder.position(x, y);
    }
    builder.build().map(|_| ()).map_err(recorder_window_error)
}

fn recorder_window_error(error: tauri::Error) -> String {
    let detail: String = error.to_string().chars().take(512).collect();
    format!("녹음 제어창을 열지 못했습니다. 앱 창에서 녹음을 조작해 주세요.\n{detail}")
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    #[test]
    fn snapshot_events_follow_content_instead_of_read_revision() {
        let initial = Snapshot {
            revision: 1,
            meetings: vec![],
            folders: vec![],
            session: None,
        };
        let mut next = initial.clone();
        next.revision = 100;
        assert!(snapshot_changed(None, &initial));
        assert!(!snapshot_changed(Some(&initial), &next));
        next.session = Some(serde_json::from_value(serde_json::json!({
            "id": "capture", "meetingId": "meeting", "deviceId": "mic", "deviceName": "Microphone",
            "state": "recording", "elapsedMs": 1000, "savedFrames": 16000,
            "sampleRate": 16000, "channels": 1, "error": null
        })).unwrap());
        assert!(snapshot_changed(Some(&initial), &next));
        let mut paused = next.clone();
        paused.session.as_mut().unwrap().state = grove_core::SessionState::Paused;
        assert!(snapshot_changed(Some(&next), &paused));
        let mut elapsed = paused.clone();
        elapsed.session.as_mut().unwrap().elapsed_ms += 1;
        assert!(snapshot_changed(Some(&paused), &elapsed));
        assert!(snapshot_changed(Some(&paused), &initial));
        let mut next = initial.clone();
        next.folders.push(grove_core::Folder {
            id: "folder".into(),
            name: "Project".into(),
        });
        assert!(snapshot_changed(Some(&initial), &next));
    }
}

fn sync_recorder(app: &tauri::AppHandle, snapshot: &Snapshot) {
    let Some(main) = app.get_webview_window("main") else {
        return;
    };
    let background = !main.is_visible().unwrap_or(true) || main.is_minimized().unwrap_or(false);
    let action = app
        .state::<DesktopState>()
        .recorder
        .lock()
        .unwrap()
        .update(active_session(snapshot), background);
    match action {
        Some(recorder_policy::Presentation::Show) => {
            if let Err(message) = show_recorder_window(app) {
                app.state::<DesktopState>()
                    .recorder
                    .lock()
                    .unwrap()
                    .show_failed();
                let _ = app.emit_to(
                    "main",
                    "grove://processing-error",
                    ProcessingError {
                        meeting_id: snapshot
                            .session
                            .as_ref()
                            .map(|session| session.meeting_id.clone()),
                        message,
                    },
                );
            }
        }
        Some(recorder_policy::Presentation::Hide) => {
            if let Some(window) = app.get_webview_window("recorder") {
                let _ = window.hide();
            }
        }
        None => (),
    }
    if app
        .state::<DesktopState>()
        .recorder
        .lock()
        .unwrap()
        .can_reposition(std::time::Instant::now())
    {
        if let Some(window) = app
            .get_webview_window("recorder")
            .filter(|window| window.is_visible().unwrap_or(false))
        {
            recover_recorder_position(&window);
        }
    }
}

fn recover_recorder_position(window: &WebviewWindow) {
    use recorder_geometry::Rect;
    let (Ok(position), Ok(size), Ok(monitors)) = (
        window.outer_position(),
        window.outer_size(),
        window.available_monitors(),
    ) else {
        return;
    };
    let areas = monitors
        .iter()
        .map(|monitor| {
            let a = monitor.work_area();
            Rect {
                x: a.position.x,
                y: a.position.y,
                width: a.size.width,
                height: a.size.height,
            }
        })
        .collect::<Vec<_>>();
    let preferred = window
        .current_monitor()
        .ok()
        .flatten()
        .and_then(|current| {
            monitors
                .iter()
                .position(|m| m.position() == current.position())
        })
        .unwrap_or(0);
    if let Some((x, y)) = recorder_geometry::recover_position(
        Rect {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        },
        &areas,
        preferred,
    ) {
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    }
}

#[tauri::command]
async fn set_auto_recorder(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    enabled: bool,
) -> Result<(), String> {
    require_main(&window)?;
    state.recorder.lock().unwrap().set_automatic(enabled);
    let snapshot = on_core(state.core.clone(), |core| core.snapshot()).await?;
    sync_recorder(&app, &snapshot);
    Ok(())
}

#[tauri::command]
fn drag_recorder(window: WebviewWindow) -> Result<(), String> {
    if window.label() != "recorder" {
        return Err("녹음 제어창에서 실행해 주세요.".into());
    }
    window
        .start_dragging()
        .map_err(|_| "창을 이동하지 못했습니다.".into())
}

fn active_session(snapshot: &Snapshot) -> Option<&str> {
    snapshot
        .session
        .as_ref()
        .filter(|session| {
            matches!(
                session.state,
                grove_core::SessionState::Recording | grove_core::SessionState::Paused
            )
        })
        .map(|session| session.id.as_str())
}

#[tauri::command]
async fn read_snapshot(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
) -> Result<Snapshot, String> {
    require_local_window(&window)?;
    on_core(state.core.clone(), |core| core.snapshot()).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResults {
    ids: Vec<String>,
    unavailable: usize,
    hits: Vec<SearchHit>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchHit {
    meeting_id: String,
    kind: &'static str,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    segment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transcript_version: Option<TranscriptVersion>,
}

fn search_excerpt(text: &str, query: &str) -> String {
    // Map the folded byte offset back to the original character without storing
    // an index for every byte of a long transcript or splitting Korean text.
    let offset = text.to_lowercase().find(query).unwrap_or(0);
    let mut folded_bytes = 0;
    let mut match_index = 0;
    for (index, character) in text.chars().enumerate() {
        match_index = index;
        let next = folded_bytes + character.to_lowercase().map(char::len_utf8).sum::<usize>();
        if offset < next {
            break;
        }
        folded_bytes = next;
    }
    let start = match_index.saturating_sub(32);
    let mut excerpt: String = text.chars().skip(start).take(100).collect();
    if start > 0 {
        excerpt.insert_str(0, "...");
    }
    if text.chars().skip(start + 100).next().is_some() {
        excerpt.push_str("...");
    }
    excerpt
}

fn search_hit(meeting_id: &str, kind: &'static str, text: &str, query: &str) -> SearchHit {
    SearchHit {
        meeting_id: meeting_id.into(),
        kind,
        text: search_excerpt(text, query),
        index: None,
        segment_id: None,
        transcript_version: None,
    }
}

#[tauri::command]
async fn search_meetings(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    query: String,
    include_deleted: bool,
) -> Result<SearchResults, String> {
    require_main(&window)?;
    let query = query.trim().to_lowercase();
    if query.len() > 4096 || query.contains('\0') {
        return Err("검색어가 너무 길거나 올바르지 않습니다.".into());
    }
    let snapshot = on_core(state.core.clone(), |core| core.snapshot()).await?;
    let mut result = SearchResults {
        ids: Vec::new(),
        unavailable: 0,
        hits: Vec::new(),
    };
    for meeting in snapshot.meetings {
        if !include_deleted && meeting.deleted_at.is_some() {
            continue;
        }
        if query.is_empty() {
            result.ids.push(meeting.id);
            continue;
        }
        if let Some((kind, text)) = [
            ("title", meeting.title.as_str()),
            ("notes", meeting.notes.as_str()),
        ]
        .into_iter()
        .find(|(_, text)| text.to_lowercase().contains(&query))
        {
            result
                .hits
                .push(search_hit(&meeting.id, kind, text, &query));
            result.ids.push(meeting.id);
            continue;
        }
        let id = meeting.id.clone();
        let transcript = on_core(state.core.clone(), move |core| core.transcript(&id)).await;
        let mut unavailable = transcript.is_err();
        let matched = transcript.ok().flatten().and_then(|transcript| {
            if let Some((index, segment)) = transcript
                .segments
                .iter()
                .enumerate()
                .find(|(_, segment)| segment.text.to_lowercase().contains(&query))
            {
                let mut hit = search_hit(&meeting.id, "transcript", &segment.text, &query);
                hit.index = Some(index);
                hit.segment_id = Some(segment.id.clone());
                hit.transcript_version = Some(transcript.version.clone());
                return Some(hit);
            }
            transcript
                .speakers
                .iter()
                .find(|speaker| speaker.name.to_lowercase().contains(&query))
                .map(|speaker| {
                    let mut hit = search_hit(&meeting.id, "speaker", &speaker.name, &query);
                    if let Some((index, segment)) =
                        transcript.segments.iter().enumerate().find(|(_, segment)| {
                            segment.speaker_id.as_deref() == Some(speaker.id.as_str())
                        })
                    {
                        hit.index = Some(index);
                        hit.segment_id = Some(segment.id.clone());
                    }
                    hit.transcript_version = Some(transcript.version.clone());
                    hit
                })
        });
        if let Some(hit) = matched {
            result.ids.push(meeting.id);
            result.hits.push(hit);
        } else {
            let id = meeting.id.clone();
            let summary = on_core(state.core.clone(), move |core| core.summary(&id)).await;
            unavailable |= summary.is_err();
            if let Some(summary) = summary
                .ok()
                .flatten()
                .filter(|summary| summary.document.text.to_lowercase().contains(&query))
            {
                result.hits.push(search_hit(
                    &meeting.id,
                    "summary",
                    &summary.document.text,
                    &query,
                ));
                result.ids.push(meeting.id);
            }
        }
        result.unavailable += usize::from(unavailable);
    }
    Ok(result)
}

#[tauri::command]
async fn create_meeting(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    title: String,
    notes: String,
) -> Result<Meeting, String> {
    require_main(&window)?;
    state.processing.require_meeting_models()?;
    on_core(state.core.clone(), move |core| {
        core.create_meeting(&title, &notes)
    })
    .await
}

#[tauri::command]
async fn update_meeting(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    id: String,
    title: String,
    notes: String,
) -> Result<Meeting, String> {
    require_main(&window)?;
    on_core(state.core.clone(), move |core| {
        core.update_meeting(&id, &title, &notes)
    })
    .await
}

#[tauri::command]
async fn set_meeting_date(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    id: String,
    meeting_date: Option<String>,
) -> Result<Meeting, String> {
    require_main(&window)?;
    let meeting = on_core(state.core.clone(), move |core| {
        core.set_meeting_date(&id, meeting_date)
    })
    .await?;
    let snapshot = on_core(state.core.clone(), |core| core.snapshot()).await?;
    publish(&app, &snapshot);
    Ok(meeting)
}

async fn finish_library_change(
    app: &tauri::AppHandle,
    core: CoreHandle,
    result: Result<(), String>,
) -> Result<Snapshot, String> {
    let snapshot = on_core(core, |core| core.snapshot()).await;
    if let Ok(snapshot) = &snapshot {
        publish(app, snapshot);
    }
    result?;
    snapshot
}

#[tauri::command]
async fn create_folder(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    name: String,
) -> Result<Snapshot, String> {
    require_main(&window)?;
    let result = on_core(state.core.clone(), move |core| {
        core.create_folder(&name).map(|_| ())
    })
    .await;
    finish_library_change(&app, state.core.clone(), result).await
}

#[tauri::command]
async fn rename_folder(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    id: String,
    name: String,
) -> Result<Snapshot, String> {
    require_main(&window)?;
    let result = on_core(state.core.clone(), move |core| {
        core.rename_folder(&id, &name).map(|_| ())
    })
    .await;
    finish_library_change(&app, state.core.clone(), result).await
}

#[tauri::command]
async fn delete_folder(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    id: String,
) -> Result<Snapshot, String> {
    require_main(&window)?;
    let result = on_core(state.core.clone(), move |core| core.delete_folder(&id)).await;
    finish_library_change(&app, state.core.clone(), result).await
}

#[tauri::command]
async fn move_meetings(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    ids: Vec<String>,
    folder_id: Option<String>,
) -> Result<Snapshot, String> {
    require_main(&window)?;
    let result = on_core(state.core.clone(), move |core| {
        core.move_meetings(ids, folder_id)
    })
    .await;
    finish_library_change(&app, state.core.clone(), result).await
}

#[tauri::command]
async fn trash_meetings(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    ids: Vec<String>,
) -> Result<Snapshot, String> {
    require_main(&window)?;
    let processing = state.processing.clone();
    let result = on_core(state.core.clone(), move |core| {
        processing.meeting_removal(&core, &ids, || core.trash_meetings(ids.clone()))
    })
    .await;
    finish_library_change(&app, state.core.clone(), result).await
}

#[tauri::command]
async fn restore_meetings(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    ids: Vec<String>,
) -> Result<Snapshot, String> {
    require_main(&window)?;
    let result = on_core(state.core.clone(), move |core| core.restore_meetings(ids)).await;
    finish_library_change(&app, state.core.clone(), result).await
}

#[tauri::command]
async fn purge_meetings(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    ids: Vec<String>,
) -> Result<Snapshot, String> {
    require_main(&window)?;
    let processing = state.processing.clone();
    let result = on_core(state.core.clone(), move |core| {
        processing.meeting_removal(&core, &ids, || {
            processing.bind_legacy_jobs(&core)?;
            let result = core.purge_meetings(ids.clone());
            let remaining = core.list_meetings()?;
            let purged = ids
                .iter()
                .filter(|id| !remaining.iter().any(|meeting| &meeting.id == *id))
                .cloned()
                .collect::<Vec<_>>();
            let cleanup = processing.purge_meeting_jobs(&purged);
            result.and(cleanup)
        })
    })
    .await;
    finish_library_change(&app, state.core.clone(), result).await
}

#[tauri::command]
async fn list_devices(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<Device>, String> {
    require_main(&window)?;
    on_core(state.core.clone(), |core| core.devices()).await
}

#[tauri::command]
async fn start_recording(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
    device_id: Option<String>,
) -> Result<Snapshot, String> {
    require_main(&window)?;
    state.processing.require_meeting_models()?;
    microphone_permission::ensure_authorized(&app).await?;
    let processing = state.processing.clone();
    let core = state.core.clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || {
        processing.recording_start(|| {
            let selected = match device_id {
                Some(id) => id,
                None => core
                    .devices()
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .find(|device| device.is_default)
                    .map(|device| device.id)
                    .unwrap_or_default(),
            };
            core.start(&meeting_id, &selected)
                .map_err(|e| e.to_string())
        })
    })
    .await
    .map_err(|_| "녹음을 시작하지 못했습니다.".to_string())??;
    publish(&app, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
async fn pause_recording(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    session_id: String,
) -> Result<Snapshot, String> {
    require_local_window(&window)?;
    let snapshot = on_core(state.core.clone(), move |core| core.pause(&session_id)).await?;
    publish(&app, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
async fn resume_recording(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    session_id: String,
) -> Result<Snapshot, String> {
    require_local_window(&window)?;
    let snapshot = on_core(state.core.clone(), move |core| core.resume(&session_id)).await?;
    publish(&app, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
async fn stop_recording(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    session_id: String,
) -> Result<Snapshot, String> {
    require_local_window(&window)?;
    let snapshot = on_core(state.core.clone(), move |core| core.stop(&session_id)).await?;
    publish(&app, &snapshot);
    let processing = state.processing.clone();
    let core = state.core.clone();
    let completed = snapshot.clone();
    match tauri::async_runtime::spawn_blocking(move || {
        processing.transcribe_recording(core, &completed)
    })
    .await
    {
        Ok(Ok(Some(progress))) => {
            let _ = app.emit_to("main", "grove://processing", progress);
        }
        Ok(Ok(None)) => (),
        result => {
            let message = match result {
                Ok(Err(error)) => error,
                _ => "음성 인식을 시작하지 못했습니다.".into(),
            };
            let _ = app.emit_to(
                "main",
                "grove://processing-error",
                ProcessingError {
                    meeting_id: snapshot
                        .session
                        .as_ref()
                        .map(|session| session.meeting_id.clone()),
                    message,
                },
            );
        }
    }
    Ok(snapshot)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessingError {
    meeting_id: Option<String>,
    message: String,
}

#[tauri::command]
async fn audio_file(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
) -> Result<String, String> {
    require_main(&window)?;
    on_core(state.core.clone(), move |core| core.audio_path(&meeting_id))
        .await?
        .into_os_string()
        .into_string()
        .map_err(|_| "음성 경로를 열지 못했습니다.".into())
}

#[tauri::command]
async fn export_text(
    window: WebviewWindow,
    app: tauri::AppHandle,
    suggested_name: String,
    contents: String,
) -> Result<bool, String> {
    require_main(&window)?;
    export::validate_contents(&contents)?;
    let name = export::suggested_name(&suggested_name)?;
    let selected = tauri::async_runtime::spawn_blocking(move || {
        let dialog = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("내보내기");
        let dialog = if name.ends_with(".md") {
            dialog
                .add_filter("Markdown", &["md"])
                .add_filter("텍스트", &["txt"])
        } else {
            dialog
                .add_filter("텍스트", &["txt"])
                .add_filter("Markdown", &["md"])
        };
        dialog.set_file_name(name).blocking_save_file()
    })
    .await
    .map_err(|_| "저장 창을 열지 못했습니다.")?;
    let Some(selected) = selected else {
        return Ok(false);
    };
    let path = selected
        .into_path()
        .map_err(|_| "파일을 저장할 위치를 선택해 주세요.")?;
    tauri::async_runtime::spawn_blocking(move || export::write_text(&path, &contents))
        .await
        .map_err(|_| "파일을 저장하지 못했습니다.")??;
    Ok(true)
}

#[tauri::command]
fn hide_recorder(window: WebviewWindow, app: tauri::AppHandle) -> Result<(), String> {
    require_local_window(&window)?;
    app.state::<DesktopState>()
        .recorder
        .lock()
        .unwrap()
        .dismiss();
    if let Some(recorder) = app.get_webview_window("recorder") {
        recorder
            .hide()
            .map_err(|_| "제어창을 숨기지 못했습니다.".to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn hide_main(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<(), String> {
    require_main(&window)?;
    let snapshot = on_core(state.core.clone(), |core| core.snapshot()).await?;
    window
        .hide()
        .map_err(|_| "창을 숨기지 못했습니다.".to_string())?;
    sync_recorder(&app, &snapshot);
    Ok(())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenMeeting {
    meeting_id: String,
    error: Option<String>,
}

#[tauri::command]
async fn open_meeting(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
    error: Option<String>,
) -> Result<(), String> {
    require_local_window(&window)?;
    let id = meeting_id.clone();
    let exists = on_core(state.core.clone(), move |core| {
        core.list_meetings()
            .map(|meetings| meetings.iter().any(|meeting| meeting.id == id))
    })
    .await?;
    if !exists {
        return Err("회의를 찾지 못했습니다.".into());
    }
    show_main(&app);
    app.emit_to(
        "main",
        "grove://open-meeting",
        OpenMeeting {
            meeting_id,
            error: error.map(|text| text.chars().take(2048).collect()),
        },
    )
    .map_err(|_| "회의를 열지 못했습니다.".into())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuitRequest {
    session_id: Option<String>,
}

fn ask_quit(app: &tauri::AppHandle) {
    show_main(app);
    let app = app.clone();
    let core = app.state::<DesktopState>().core.clone();
    tauri::async_runtime::spawn(async move {
        if let Ok(snapshot) = on_core(core, |core| core.snapshot()).await {
            let session_id = active_session(&snapshot).map(str::to_string);
            let _ = app.emit_to("main", "grove://quit-requested", QuitRequest { session_id });
        }
    });
}

#[tauri::command]
async fn request_quit(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    session_id: Option<String>,
) -> Result<(), String> {
    require_main(&window)?;
    let processing = state.processing.clone();
    let stopped = tauri::async_runtime::spawn_blocking(move || processing.shutdown())
        .await
        .map_err(|_| "음성 처리를 종료하지 못했습니다.".to_string())
        .and_then(|result| result);
    if let Err(error) = stopped {
        state.processing.resume();
        return Err(error);
    }
    let result = on_core(state.core.clone(), move |core| {
        core.prepare_shutdown(session_id.as_deref())
    })
    .await;
    let snapshot = match result {
        Ok(snapshot) => snapshot,
        Err(error) => {
            state.processing.resume();
            return Err(error);
        }
    };
    publish(&app, &snapshot);
    state.quitting.store(true, Ordering::Release);
    state.polling.store(false, Ordering::Release);
    app.exit(0);
    Ok(())
}

#[tauri::command]
fn processing_status(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<Progress, String> {
    require_main(&window)?;
    state.processing.notify_startup_issues(|issues| {
        app.emit_to(
            "main",
            "grove://processing-error",
            ProcessingError {
                meeting_id: None,
                message: issues
                    .iter()
                    .map(|issue| issue.message.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            },
        )
        .is_ok()
    });
    Ok(state.processing.snapshot())
}
#[tauri::command]
fn prepare_model(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
) -> Result<Progress, String> {
    require_main(&window)?;
    state.processing.prepare_model()
}
#[tauri::command]
async fn prepare_meeting_models(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    include_summary: Option<bool>,
) -> Result<Progress, String> {
    require_main(&window)?;
    let snapshot = on_core(state.core.clone(), |core| core.snapshot()).await?;
    if active_session(&snapshot).is_some() {
        return Err("녹음을 마친 뒤 준비해 주세요.".into());
    }
    state
        .processing
        .prepare_meeting_models(include_summary.unwrap_or(false))
}
#[tauri::command]
async fn import_audio(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<Option<Progress>, String> {
    require_main(&window)?;
    state.processing.require_meeting_models()?;
    if !state.processing.snapshot().supported {
        return Err("이 기기의 음성 가져오기가 아직 준비되지 않았습니다.".into());
    }
    let snapshot = on_core(state.core.clone(), |core| core.snapshot()).await?;
    if active_session(&snapshot).is_some() {
        return Err("녹음을 마친 뒤 음성을 가져와 주세요.".into());
    }
    let source = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("음성", &["wav", "m4a", "mp3", "aac", "aiff", "aif", "caf"])
            .blocking_pick_file()
    })
    .await
    .map_err(|_| "파일 선택 창을 열지 못했습니다.")?;
    let Some(source) = source else {
        return Ok(None);
    };
    let source = source
        .into_path()
        .map_err(|_| "로컬 음성 파일을 선택해 주세요.")?;
    state
        .processing
        .import(state.core.clone(), source)
        .map(Some)
}
#[tauri::command]
async fn transcribe_audio(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
) -> Result<Progress, String> {
    require_main(&window)?;
    let processing = state.processing.clone();
    let core = state.core.clone();
    tauri::async_runtime::spawn_blocking(move || processing.transcribe(core, meeting_id))
        .await
        .map_err(|_| "음성 인식을 시작하지 못했습니다.".to_string())?
}
#[tauri::command]
fn cancel_processing(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    job_id: String,
) -> Result<(), String> {
    require_main(&window)?;
    state.processing.cancel(&job_id)
}
#[tauri::command]
fn acknowledge_processing(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    job_id: String,
) -> Result<Progress, String> {
    require_main(&window)?;
    state.processing.acknowledge(&job_id)
}
#[tauri::command]
async fn retry_processing(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    job_id: String,
) -> Result<Progress, String> {
    require_main(&window)?;
    let process = state.processing.clone();
    let core = state.core.clone();
    tauri::async_runtime::spawn_blocking(move || process.retry(core, &job_id))
        .await
        .map_err(|_| "작업을 다시 시작하지 못했습니다.".to_string())?
}
#[tauri::command]
fn prepare_summary_model(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
) -> Result<Progress, String> {
    require_main(&window)?;
    state.processing.prepare_summary()
}
#[tauri::command]
fn prepare_diarization_model(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
) -> Result<Progress, String> {
    require_main(&window)?;
    state.processing.prepare_diarization()
}
#[tauri::command]
async fn diarize_meeting(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
    expected: TranscriptVersion,
) -> Result<Progress, String> {
    require_main(&window)?;
    let processing = state.processing.clone();
    let core = state.core.clone();
    tauri::async_runtime::spawn_blocking(move || processing.diarize(core, meeting_id, expected))
        .await
        .map_err(|_| "화자 구분을 시작하지 못했습니다.".to_string())?
}
#[tauri::command]
async fn summarize_meeting(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
) -> Result<Progress, String> {
    require_main(&window)?;
    let processing = state.processing.clone();
    let core = state.core.clone();
    tauri::async_runtime::spawn_blocking(move || processing.summarize(core, meeting_id))
        .await
        .map_err(|_| "요약을 시작하지 못했습니다.".to_string())?
}
#[tauri::command]
async fn read_summary(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
) -> Result<Option<SummaryView>, String> {
    require_main(&window)?;
    on_core(state.core.clone(), move |core| core.summary(&meeting_id)).await
}

#[tauri::command]
async fn read_summary_citations(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
    summary_id: String,
    references: Vec<u32>,
) -> Result<SummaryCitations, String> {
    require_main(&window)?;
    on_core(state.core.clone(), move |core| {
        core.summary_citations(&meeting_id, &summary_id, references)
    })
    .await
}

#[tauri::command]
fn read_terminology_settings(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
) -> Result<terminology::Settings, String> {
    require_main(&window)?;
    terminology::read_settings(&state.processing)
}

#[tauri::command]
fn set_terminology_enabled(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    enabled: bool,
) -> Result<terminology::Settings, String> {
    require_main(&window)?;
    terminology::set_enabled(&state.processing, enabled)
}

#[tauri::command]
async fn read_transcript(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
) -> Result<Option<Transcript>, String> {
    require_main(&window)?;
    on_core(state.core.clone(), move |core| core.transcript(&meeting_id)).await
}
#[tauri::command]
async fn read_transcript_history(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
) -> Result<TranscriptHistory, String> {
    require_main(&window)?;
    on_core(state.core.clone(), move |core| {
        core.transcript_history(&meeting_id)
    })
    .await
}

#[tauri::command]
async fn restore_transcript(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
    expected: TranscriptVersion,
    target: TranscriptVersion,
) -> Result<Transcript, String> {
    require_main(&window)?;
    let processing = state.processing.clone();
    let core = state.core.clone();
    let document = tauri::async_runtime::spawn_blocking(move || {
        processing.transcript_restore(&meeting_id, || {
            core.restore_transcript(&meeting_id, expected, target)
        })
    })
    .await
    .map_err(|_| "이전 기록을 적용하지 못했습니다.".to_string())?
    .map_err(|error| error.to_string())?;
    let snapshot = on_core(state.core.clone(), |core| core.snapshot()).await?;
    publish(&app, &snapshot);
    Ok(document)
}

#[tauri::command]
async fn save_transcript(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
    expected: TranscriptVersion,
    texts: Vec<String>,
) -> Result<Transcript, String> {
    require_main(&window)?;
    on_core(state.core.clone(), move |core| {
        core.edit_transcript(&meeting_id, expected, texts)
    })
    .await
}

#[tauri::command]
async fn save_speakers(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
    expected: TranscriptVersion,
    speakers: Vec<TranscriptSpeaker>,
    assignments: Vec<Option<String>>,
) -> Result<Transcript, String> {
    require_main(&window)?;
    on_core(state.core.clone(), move |core| {
        core.edit_speakers(&meeting_id, expected, speakers, assignments)
    })
    .await
}

#[cfg(target_os = "macos")]
fn tray_icon(_theme: tauri::Theme) -> tauri::image::Image<'static> {
    tauri::include_image!("icons/tray-template.png")
}

#[cfg(not(target_os = "macos"))]
fn tray_icon(theme: tauri::Theme) -> tauri::image::Image<'static> {
    match theme {
        tauri::Theme::Dark => tauri::include_image!("icons/tray-on-dark.png"),
        _ => tauri::include_image!("icons/tray-on-light.png"),
    }
}

fn main() {
    // This isolated child only loads the packaged speech library. It must run
    // before single-instance registration, windows, recording or model setup.
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() == Some(std::ffi::OsStr::new("--nero-speaker-worker")) {
        let request = arguments.next();
        if request.is_none() || arguments.next().is_some() {
            eprintln!("invalid speaker worker request");
            std::process::exit(64);
        }
        match speaker_worker::entry(std::path::Path::new(&request.unwrap())) {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("{}", error.error_json());
                std::process::exit(1);
            }
        }
    }
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder.menu(app_menu::default);
    let app = builder
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main(app)
        }))
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            agent_connection_info,
            read_snapshot,
            search_meetings,
            create_meeting,
            update_meeting,
            set_meeting_date,
            create_folder,
            rename_folder,
            delete_folder,
            move_meetings,
            trash_meetings,
            restore_meetings,
            purge_meetings,
            list_devices,
            start_recording,
            pause_recording,
            resume_recording,
            stop_recording,
            audio_file,
            export_text,
            hide_recorder,
            set_auto_recorder,
            drag_recorder,
            hide_main,
            open_meeting,
            request_quit,
            processing_status,
            prepare_model,
            prepare_meeting_models,
            import_audio,
            transcribe_audio,
            cancel_processing,
            retry_processing,
            acknowledge_processing,
            read_terminology_settings,
            set_terminology_enabled,
            read_transcript,
            read_transcript_history,
            restore_transcript,
            save_transcript,
            save_speakers,
            prepare_summary_model,
            prepare_diarization_model,
            diarize_meeting,
            summarize_meeting,
            read_summary,
            read_summary_citations
        ])
        .setup(|app| {
            let root = app.path().app_data_dir()?;
            let core = CoreHandle::open(&root)?;
            let processing = Processing::new(root, app.path().resource_dir()?);
            processing.cleanup_removed_meeting_jobs(&core);
            processing.upgrade_transcript_sources(&core);
            let polling = Arc::new(AtomicBool::new(true));
            app.manage(DesktopState {
                core: core.clone(),
                quitting: Arc::new(AtomicBool::new(false)),
                polling: polling.clone(),
                recorder: Mutex::new(recorder_policy::RecorderPolicy::default()),
                processing: processing.clone(),
            });
            let menu = tauri::menu::Menu::with_items(
                app,
                &[
                    &tauri::menu::MenuItem::with_id(app, "open", "Nero 열기", true, None::<&str>)?,
                    &tauri::menu::PredefinedMenuItem::separator(app)?,
                    &tauri::menu::MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?,
                ],
            )?;
            tauri::tray::TrayIconBuilder::with_id("grove")
                .icon(tray_icon(
                    app.get_webview_window("main")
                        .and_then(|window| window.theme().ok())
                        .unwrap_or(tauri::Theme::Light),
                ))
                .icon_as_template(cfg!(target_os = "macos"))
                .tooltip("Nero")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "quit" => ask_quit(app),
                    _ => (),
                })
                .build(app)?;
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut previous: Option<Snapshot> = None;
                let mut previous_progress = None;
                while polling.load(Ordering::Acquire) {
                    let _ = processing.drive(&core);
                    if let Ok(snapshot) = core.snapshot() {
                        let changed = snapshot_changed(previous.as_ref(), &snapshot);
                        if changed {
                            publish(&handle, &snapshot);
                            previous = Some(snapshot);
                        } else {
                            sync_recorder(&handle, &snapshot);
                        }
                    }
                    let progress = processing.snapshot();
                    if previous_progress.as_ref() != Some(&progress) {
                        let _ = handle.emit_to("main", "grove://processing", &progress);
                        previous_progress = Some(progress);
                    }
                    std::thread::sleep(Duration::from_millis(750));
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "recorder" && matches!(event, tauri::WindowEvent::Moved(_)) {
                window
                    .app_handle()
                    .state::<DesktopState>()
                    .recorder
                    .lock()
                    .unwrap()
                    .moved(std::time::Instant::now());
            }
            if window.label() == "main"
                && matches!(
                    event,
                    tauri::WindowEvent::Focused(_) | tauri::WindowEvent::Resized(_)
                )
            {
                request_recorder_sync(window.app_handle());
            }
            #[cfg(target_os = "windows")]
            if let tauri::WindowEvent::ThemeChanged(theme) = event {
                if window.label() == "main" {
                    if let Some(tray) = window.app_handle().tray_by_id("grove") {
                        let _ = tray.set_icon(Some(tray_icon(*theme)));
                    }
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if window.label() == "main" {
                    let _ = window.emit("grove://close-requested", ());
                } else {
                    window
                        .app_handle()
                        .state::<DesktopState>()
                        .recorder
                        .lock()
                        .unwrap()
                        .dismiss();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("Nero를 시작하지 못했습니다.");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { ref api, .. } = event {
            if !app.state::<DesktopState>().quitting.load(Ordering::Acquire) {
                api.prevent_exit();
                ask_quit(app);
            }
        }
        if let tauri::RunEvent::Exit = event {
            app.state::<DesktopState>()
                .polling
                .store(false, Ordering::Release);
        }
    });
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn search_excerpt_bounds_unicode_context_without_byte_slicing() {
        let text = format!("{}한국어 검색{}", "앞".repeat(150), "뒤".repeat(150));
        let excerpt = search_excerpt(&text, "한국어");
        assert!(excerpt.contains("한국어 검색"));
        assert!(excerpt.starts_with("...") && excerpt.ends_with("..."));
        assert!(excerpt.chars().count() <= 106);
        let expanded = format!("{}İstanbul 모임", "앞".repeat(80));
        assert!(search_excerpt(&expanded, "i\u{307}stanbul").contains("İstanbul"));
    }

    #[test]
    fn search_hit_exposes_bounded_context_and_optional_anchor_only() {
        let hit = search_hit("meeting", "notes", "회의 안건", "안건");
        let value = serde_json::to_value(hit).unwrap();
        assert_eq!(value["meetingId"], "meeting");
        assert_eq!(value["kind"], "notes");
        assert_eq!(value["text"], "회의 안건");
        assert!(value.get("index").is_none());
        assert!(value.get("segmentId").is_none());
        assert!(value.get("transcriptVersion").is_none());
    }
}
