#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent_connection;
mod agent_mutations;
#[cfg(target_os = "macos")]
mod app_menu;
mod app_updater;
mod asr;
mod calendar;
mod calendar_desktop;
mod diarization;
mod downloads;
mod export;
mod identity_migration;
mod job_journal;
mod lifecycle;
mod live_asr;
mod live_speech;
mod live_speech_worker;
#[cfg(target_os = "macos")]
mod macos_recorder;
mod memory;
mod microphone_permission;
mod people_desktop;
mod processing;
mod process_tree;
mod recorder_geometry;
mod recorder_policy;
mod speaker_worker;
mod speech_audio;
mod speech_library;
mod speech_choice;
mod whisper_asr;
mod storage_space;
mod summary;
mod summary_checkpoint;
mod summary_evidence;
mod summary_failure;
mod summary_template;
mod template_settings;
mod terminology;
use nero_core::{
    CoreHandle, Device, DocumentFormat, DocumentKind, ExportedDocument, Meeting, Snapshot,
    SummaryCitations, SummaryView, Transcript, TranscriptHistory, TranscriptSpeaker,
    TranscriptVersion,
};
use processing::{Processing, Progress};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tauri::{Emitter, Manager, WebviewWindow};
#[cfg(not(target_os = "macos"))]
use tauri::{WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;

struct DesktopState {
    core: CoreHandle,
    agent_mutations: Arc<agent_mutations::AgentMutations>,
    quitting: Arc<AtomicBool>,
    polling: Arc<AtomicBool>,
    processing: Arc<Processing>,
    lifecycle: lifecycle::Lifecycle,
    recorder: Mutex<recorder_policy::RecorderPolicy>,
}

fn require_main(window: &WebviewWindow) -> Result<(), String> {
    if window
        .app_handle()
        .try_state::<DesktopState>()
        .is_some_and(|state| state.lifecycle.is_closing())
    {
        return Err("앱을 종료하거나 업데이트하고 있습니다. 잠시 기다려 주세요.".into());
    }
    if window.label() == "main" {
        Ok(())
    } else {
        Err("본창에서 실행해 주세요.".into())
    }
}

#[tauri::command]
async fn agent_write_ready(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    request_id: String,
    error: Option<String>,
) -> Result<(), String> {
    require_main(&window)?;
    let agent = state.agent_mutations.clone();
    tauri::async_runtime::spawn_blocking(move || agent.ready(&request_id, error))
        .await
        .map_err(|_| "수정 준비를 완료하지 못했습니다.".to_string())?
}

#[tauri::command]
async fn agent_connection_info(
    app: tauri::AppHandle,
    window: WebviewWindow,
    force_host_refresh: Option<bool>,
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
    let connections = connection_directory(&app)?;
    let home = app
        .path()
        .home_dir()
        .map_err(|_| "사용자 경로를 확인하지 못했습니다.".to_string())?;
    let config = app
        .path()
        .config_dir()
        .map_err(|_| "설정 경로를 확인하지 못했습니다.".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let observations =
            agent_connection::observe_connection_state(&resources, &root, &connections);
        agent_connection::connection_info_with_hosts(
            &resources,
            &root,
            &connections,
            &home,
            &config,
            force_host_refresh.unwrap_or(false),
        )
        .with_observation_errors(observations)
    })
    .await
    .map_err(|_| "연결 상태를 확인하지 못했습니다. 다시 시도해 주세요.".to_string())
}

fn connection_directory(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .config_dir()
        .map(|path| path.join("Nero").join("connections"))
        .map_err(|_| "연결 설정 경로를 확인하지 못했습니다.".to_string())
}

#[tauri::command]
async fn agent_connect_client(
    app: tauri::AppHandle,
    window: WebviewWindow,
    client: String,
) -> Result<agent_connection::ConnectionInfo, String> {
    require_main(&window)?;
    let resources = app
        .path()
        .resource_dir()
        .map_err(|_| "연결 경로를 확인하지 못했습니다.".to_string())?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|_| "연결 경로를 확인하지 못했습니다.".to_string())?;
    let connections = connection_directory(&app)?;
    let home = app
        .path()
        .home_dir()
        .map_err(|_| "사용자 경로를 확인하지 못했습니다.".to_string())?;
    let config = app
        .path()
        .config_dir()
        .map_err(|_| "설정 경로를 확인하지 못했습니다.".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let before = agent_connection::connection_info_with_hosts(
            &resources,
            &root,
            &connections,
            &home,
            &config,
            true,
        );
        let attempt = before.connection_attempt(&client);
        if attempt == agent_connection::ConnectionAttempt::Recheck {
            return Ok(before);
        }
        let action =
            agent_connection::prepare_connection(&resources, &root, &connections, &client)?;
        let installed = if attempt == agent_connection::ConnectionAttempt::Install {
            agent_connection::install_prepared_connection(&home, &connections, &client)
        } else {
            Ok(false)
        };
        if matches!(&installed, Ok(false)) {
            open_connection(&action)?;
        }
        let observations =
            agent_connection::observe_connection_state(&resources, &root, &connections);
        let info = agent_connection::connection_info_with_hosts(
            &resources,
            &root,
            &connections,
            &home,
            &config,
            false,
        )
        .with_observation_errors(observations);
        Ok(match installed {
            Ok(_) => info,
            Err(detail) => info.with_action_error(&client, detail),
        })
    })
    .await
    .map_err(|_| "연결 화면을 열지 못했습니다. 다시 시도해 주세요.".to_string())?
}

fn open_connection(action: &agent_connection::ConnectionAction) -> Result<(), String> {
    let valid = match action.kind.as_str() {
        "file" => {
            std::path::Path::new(&action.target).is_absolute()
                && std::path::Path::new(&action.target)
                    .extension()
                    .is_some_and(|value| value == "mcpb")
        }
        "url" => action
            .target
            .starts_with("codex://plugins/nero?marketplacePath="),
        _ => false,
    };
    if !valid || action.target.contains('\0') {
        return Err("연결 대상을 확인하지 못했습니다.".into());
    }
    #[cfg(target_os = "macos")]
    {
        let mut command = std::process::Command::new("/usr/bin/open");
        if action.kind == "file" {
            command.args(["-b", "com.anthropic.claudefordesktop"]);
        }
        let status = command
            .arg(&action.target)
            .output()
            .map_err(|_| "앱을 열지 못했습니다. 설치 상태를 확인해 주세요.".to_string())?;
        if status.status.success() {
            return Ok(());
        }
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "shell32")]
        extern "system" {
            fn ShellExecuteW(
                window: *mut std::ffi::c_void,
                operation: *const u16,
                file: *const u16,
                parameters: *const u16,
                directory: *const u16,
                show: i32,
            ) -> isize;
        }
        let wide = |text: &str| {
            std::ffi::OsStr::new(text)
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        let operation = wide("open");
        let target = wide(&action.target);
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        if result > 32 {
            return Ok(());
        }
    }
    Err("앱을 열지 못했습니다. 설치 상태를 확인해 주세요.".into())
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
    action: impl FnOnce(CoreHandle) -> Result<T, nero_core::CoreError> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || action(core))
        .await
        .map_err(|_| "작업을 완료하지 못했습니다. 다시 시도해 주세요.".to_string())?
        .map_err(|error| error.to_string())
}

fn publish(app: &tauri::AppHandle, snapshot: &Snapshot) {
    let _ = app.emit("nero://snapshot", snapshot);
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

#[cfg(not(target_os = "macos"))]
fn show_recorder_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("recorder") {
        window
            .set_always_on_top(true)
            .map_err(recorder_window_error)?;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        window
            .set_visible_on_all_workspaces(true)
            .map_err(recorder_window_error)?;
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
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        builder = builder.visible_on_all_workspaces(true);
    }
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

#[cfg(not(target_os = "macos"))]
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
        paused.session.as_mut().unwrap().state = nero_core::SessionState::Paused;
        assert!(snapshot_changed(Some(&next), &paused));
        let mut elapsed = paused.clone();
        elapsed.session.as_mut().unwrap().elapsed_ms += 1;
        assert!(snapshot_changed(Some(&paused), &elapsed));
        assert!(snapshot_changed(Some(&paused), &initial));
        let mut next = initial.clone();
        next.folders.push(nero_core::Folder {
            id: "folder".into(),
            name: "Project".into(),
        });
        assert!(snapshot_changed(Some(&initial), &next));
    }
}

fn sync_recorder(app: &tauri::AppHandle, snapshot: &Snapshot) {
    #[cfg(target_os = "macos")]
    {
        let state = app.state::<DesktopState>();
        let mut recorder = state.recorder.lock().unwrap();
        recorder.update(active_session(snapshot), false);
        macos_recorder::sync(app, snapshot, recorder.is_shown());
    }
    #[cfg(not(target_os = "macos"))]
    sync_floating_recorder(app, snapshot);
}

#[cfg(not(target_os = "macos"))]
fn sync_floating_recorder(app: &tauri::AppHandle, snapshot: &Snapshot) {
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
                    "nero://processing-error",
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
async fn show_recorder(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    session_id: String,
) -> Result<(), String> {
    require_main(&window)?;
    let snapshot = on_core(state.core.clone(), |core| core.snapshot()).await?;
    let session = active_session(&snapshot)
        .filter(|id| *id == session_id)
        .ok_or("녹음이 종료되었습니다.")?;
    state.recorder.lock().unwrap().request_show(session);
    sync_recorder(&app, &snapshot);
    Ok(())
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
                nero_core::SessionState::Recording | nero_core::SessionState::Paused
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
    let transition = state.lifecycle.begin_transition()?;
    let processing = state.processing.clone();
    let core = state.core.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _transition = transition;
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
            let snapshot = core
                .start(&meeting_id, &selected)
                .map_err(|e| e.to_string())?;
            publish(&app, &snapshot);
            start_live_speech(&app, &snapshot);
            Ok(snapshot)
        })
    })
    .await
    .map_err(|_| "녹음을 시작하지 못했습니다.".to_string())?
}

#[tauri::command]
async fn pause_recording(
    app: tauri::AppHandle,
    window: WebviewWindow,
    session_id: String,
) -> Result<Snapshot, String> {
    require_local_window(&window)?;
    change_recording(app, session_id, RecordingAction::Pause).await
}

pub(crate) fn start_live_speech(app: &tauri::AppHandle, snapshot: &Snapshot) {
    let state = app.state::<DesktopState>();
    if state.processing.start_live(&state.core, snapshot).is_err() {
        let _ = app.emit_to(
            "main",
            "nero://processing-error",
            ProcessingError {
                meeting_id: snapshot.session.as_ref().map(|s| s.meeting_id.clone()),
                message: "녹음은 계속합니다. 음성 기록은 녹음을 마친 뒤 처리합니다.".into(),
            },
        );
    }
}

#[tauri::command]
async fn resume_recording(
    app: tauri::AppHandle,
    window: WebviewWindow,
    session_id: String,
) -> Result<Snapshot, String> {
    require_local_window(&window)?;
    change_recording(app, session_id, RecordingAction::Resume).await
}

#[tauri::command]
async fn stop_recording(
    app: tauri::AppHandle,
    window: WebviewWindow,
    session_id: String,
) -> Result<Snapshot, String> {
    require_local_window(&window)?;
    change_recording(app, session_id, RecordingAction::Stop).await
}

#[derive(Clone, Copy)]
enum RecordingAction {
    Pause,
    Resume,
    Stop,
}

async fn change_recording(
    app: tauri::AppHandle,
    session_id: String,
    action: RecordingAction,
) -> Result<Snapshot, String> {
    let state = app.state::<DesktopState>();
    let transition = state.lifecycle.begin_transition()?;
    let processing = state.processing.clone();
    let core = state.core.clone();
    // This blocking task owns the whole handoff, even if the IPC future is
    // dropped. There is no idle gap between capture EOF and job admission.
    tauri::async_runtime::spawn_blocking(move || {
        let _transition = transition;
        let snapshot = processing.recording_change(|| {
            match action {
                RecordingAction::Pause => core.pause(&session_id),
                RecordingAction::Resume => core.resume(&session_id),
                RecordingAction::Stop => core.stop(&session_id),
            }
            .map_err(|error| error.to_string())
        })?;
        publish(&app, &snapshot);
        if matches!(action, RecordingAction::Stop) {
            // Job admission takes the same gate, after the capture change releases it.
            match processing.transcribe_recording(core, &snapshot) {
                Ok(Some(progress)) => {
                    let _ = app.emit_to("main", "nero://processing", progress);
                }
                Ok(None) => (),
                Err(message) => {
                    let _ = app.emit_to(
                        "main",
                        "nero://processing-error",
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
        }
        Ok(snapshot)
    })
    .await
    .map_err(|_| "녹음 상태와 후속 작업을 확인하지 못했습니다.".to_string())?
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
async fn export_document(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
    kind: DocumentKind,
    format: DocumentFormat,
    include_citations: bool,
) -> Result<ExportedDocument, String> {
    require_main(&window)?;
    on_core(state.core.clone(), move |core| {
        core.export_document(&meeting_id, kind, format, include_citations)
    })
    .await
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
    dismiss_recorder(&app)
}

fn dismiss_recorder(app: &tauri::AppHandle) -> Result<(), String> {
    app.state::<DesktopState>()
        .recorder
        .lock()
        .unwrap()
        .dismiss();
    #[cfg(target_os = "macos")]
    macos_recorder::hide(app);
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
        "nero://open-meeting",
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
            let _ = app.emit_to("main", "nero://quit-requested", QuitRequest { session_id });
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
    let exit_lease = state.lifecycle.begin_exit(lifecycle::ExitKind::Quit)?;
    let processing = state.processing.clone();
    let core = state.core.clone();
    let quitting = state.quitting.clone();
    let polling = state.polling.clone();
    // Own the lease inside the side-effecting task, so dropping the IPC future
    // cannot release it while shutdown is still running in the background.
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = processing.shutdown() {
            processing.resume();
            return Err(error);
        }
        let snapshot = match core.prepare_shutdown(session_id.as_deref()) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                processing.resume();
                return Err(error.to_string());
            }
        };
        publish(&app, &snapshot);
        quitting.store(true, Ordering::Release);
        polling.store(false, Ordering::Release);
        exit_lease.commit();
        app.exit(0);
        Ok(())
    })
    .await
    .map_err(|_| "음성 처리를 종료하지 못했습니다.".to_string())?
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
            "nero://processing-error",
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
fn set_speech_engine(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    engine: speech_choice::SpeechEngine,
) -> Result<Progress,String> {
    require_main(&window)?;
    state.processing.set_speech_engine(engine)
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
async fn draft_attachment_target(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
) -> Result<nero_core::DraftAttachmentTarget, String> {
    require_main(&window)?;
    on_core(state.core.clone(), move |core| {
        core.draft_attachment_target(&meeting_id)
    })
    .await
}
#[tauri::command]
async fn import_audio(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    target: Option<nero_core::DraftAttachmentTarget>,
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
    if let Some(target) = target.clone() {
        on_core(state.core.clone(), move |core| {
            core.validate_draft_attachment(target)
        })
        .await?;
    }
    let source = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter(
                "음성",
                if cfg!(windows) {
                    &["wav"][..]
                } else {
                    &["wav", "m4a", "mp3", "aac", "aiff", "aif", "caf"][..]
                },
            )
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
    match target {
        Some(target) => state
            .processing
            .import_into_draft(state.core.clone(), source, target),
        None => state.processing.import(state.core.clone(), source),
    }
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
    template: Option<summary_template::TemplateRef>,
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
) -> Result<Progress, String> {
    require_main(&window)?;
    let processing = state.processing.clone();
    let core = state.core.clone();
    tauri::async_runtime::spawn_blocking(move || {
        processing.summarize_selected(core, meeting_id, template)
    })
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
        processing.meeting_idle(&meeting_id, || {
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

fn main() {
    // This isolated child only loads the packaged speech library. It must run
    // before single-instance registration, windows, recording or model setup.
    let mut arguments = std::env::args_os().skip(1);
    let mode = arguments.next();
    if mode.as_deref() == Some(std::ffi::OsStr::new("--nero-verify-app-update")) {
        let values: Vec<_> = arguments.collect();
        let result = if values.len() == 3 {
            app_updater::verify_release_file(
                std::path::Path::new(&values[0]),
                std::path::Path::new(&values[1]),
                &values[2].to_string_lossy(),
            )
        } else {
            Err("업데이트 검증 인수가 올바르지 않습니다.".into())
        };
        if let Err(error) = &result {
            eprintln!("{error}");
        }
        if let Ok(digest) = &result {
            println!("{digest}");
        }
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
    if mode.as_deref() == Some(std::ffi::OsStr::new("--nero-live-speech-worker")) {
        let request = arguments.next();
        if request.is_none() || arguments.next().is_some() {
            live_speech_worker::ExitPolicy::Normal.exit(64);
        }
        match live_speech_worker::entry(std::path::Path::new(&request.unwrap())) {
            Ok(policy) => policy.exit(0),
            Err(error) => {
                eprintln!("{error}");
                live_speech_worker::ExitPolicy::Retained.exit(1);
            }
        }
    }
    if mode.as_deref() == Some(std::ffi::OsStr::new("--nero-speaker-worker")) {
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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            people_desktop::people_catalog,
            people_desktop::replace_people_catalog,
            people_desktop::meeting_attendees,
            people_desktop::replace_meeting_attendees,
            people_desktop::promote_meeting_attendee,
            app_updater::app_update_status,
            app_updater::app_update_check,
            app_updater::app_update_download,
            app_updater::app_update_install,
            app_updater::app_update_recovery_quit,
            agent_write_ready,
            calendar_desktop::read_calendar_settings,
            calendar_desktop::read_calendar_meeting_occurrences,
            calendar_desktop::create_meeting_from_calendar,
            calendar_desktop::begin_calendar_connect,
            calendar_desktop::finish_calendar_connect,
            calendar_desktop::reopen_calendar_connect,
            calendar_desktop::cancel_calendar_connect,
            calendar_desktop::disconnect_calendar,
            calendar_desktop::select_calendars,
            calendar_desktop::sync_calendar,
            calendar_desktop::read_calendar_reminder,
            calendar_desktop::calendar_reminder_ready,
            calendar_desktop::dismiss_calendar_reminder,
            calendar_desktop::start_calendar_recording,
            template_settings::list_templates,
            template_settings::read_template_catalog,
            template_settings::reset_template_catalog,
            template_settings::get_template,
            template_settings::create_template,
            template_settings::update_template,
            template_settings::reset_template,
            template_settings::delete_template,
            agent_connection_info,
            agent_connect_client,
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
            export_document,
            hide_recorder,
            show_recorder,
            set_auto_recorder,
            drag_recorder,
            hide_main,
            open_meeting,
            request_quit,
            processing_status,
            set_speech_engine,
            prepare_model,
            prepare_meeting_models,
            draft_attachment_target,
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
            app.manage(app_updater::AppUpdates::new(
                app.path().app_cache_dir()?.join("app-update"),
            )?);
            identity_migration::validate_startup(&root)?;
            let core = CoreHandle::open(&root)?;
            let processing = Processing::new(root.clone(), app.path().resource_dir()?);
            processing.cleanup_removed_meeting_jobs(&core);
            processing.upgrade_transcript_sources(&core);
            let polling = Arc::new(AtomicBool::new(true));
            let agent_mutations = agent_mutations::AgentMutations::start(
                app.handle().clone(),
                &root,
                core.clone(),
                processing.clone(),
            );
            app.manage(DesktopState {
                agent_mutations,
                core: core.clone(),
                quitting: Arc::new(AtomicBool::new(false)),
                polling: polling.clone(),
                recorder: Mutex::new(recorder_policy::RecorderPolicy::default()),
                processing: processing.clone(),
                lifecycle: lifecycle::Lifecycle::default(),
            });
            calendar_desktop::install(app.handle(), root.clone());
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
                        let _ = handle.emit_to("main", "nero://processing", &progress);
                        previous_progress = Some(progress);
                    }
                    std::thread::sleep(Duration::from_millis(750));
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if calendar_desktop::handle_window_event(window, event) {
                return;
            }
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
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if window.label() == "main" {
                    #[cfg(target_os = "windows")]
                    ask_quit(window.app_handle());
                    #[cfg(not(target_os = "windows"))]
                    let _ = window.emit("nero://close-requested", ());
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
        #[cfg(target_os = "macos")]
        if matches!(&event, tauri::RunEvent::Reopen { .. }) {
            show_main(app);
        }
        if let tauri::RunEvent::ExitRequested { ref api, .. } = event {
            if !app.state::<DesktopState>().quitting.load(Ordering::Acquire) {
                api.prevent_exit();
                ask_quit(app);
            }
        }
        if let tauri::RunEvent::Exit = event {
            app.state::<DesktopState>().agent_mutations.stop();
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
