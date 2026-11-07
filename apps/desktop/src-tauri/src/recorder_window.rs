//! The small recording panel: which snapshot it sees, when it shows or hides, and
//! the commands its window and the main window send about it. macOS draws it
//! natively in `macos_recorder`; elsewhere it is a floating webview window.
use crate::{active_session, on_core, require_local_window, require_main, DesktopState};
use nero_core::{Meeting, Snapshot};
use tauri::{Manager, WebviewWindow};
#[cfg(not(target_os = "macos"))]
use tauri::{WebviewUrl, WebviewWindowBuilder};

// The recorder panel shows only the recording's meeting and never its notes.
pub(crate) fn recorder_snapshot(snapshot: &Snapshot) -> Snapshot {
    let meeting_id = snapshot
        .session
        .as_ref()
        .map(|session| session.meeting_id.as_str());
    Snapshot {
        revision: snapshot.revision,
        meetings: snapshot
            .meetings
            .iter()
            .filter(|meeting| Some(meeting.id.as_str()) == meeting_id)
            .map(|meeting| Meeting {
                notes: String::new(),
                attendees: Vec::new(),
                ..meeting.clone()
            })
            .collect(),
        folders: Vec::new(),
        session: snapshot.session.clone(),
    }
}

pub(crate) fn request_sync(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<DesktopState>() else {
        return;
    };
    let core = state.core.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Ok(snapshot) = on_core(core, |core| core.snapshot()).await {
            sync(&app, &snapshot);
        }
    });
}

pub(crate) fn sync(app: &tauri::AppHandle, snapshot: &Snapshot) {
    #[cfg(target_os = "macos")]
    {
        let state = app.state::<DesktopState>();
        let mut recorder = state.recorder.lock().unwrap();
        recorder.update(active_session(snapshot), false);
        crate::macos_recorder::sync(app, snapshot, recorder.is_shown());
    }
    #[cfg(not(target_os = "macos"))]
    sync_floating(app, snapshot);
}

#[cfg(not(target_os = "macos"))]
fn sync_floating(app: &tauri::AppHandle, snapshot: &Snapshot) {
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
        Some(crate::recorder_policy::Presentation::Show) => {
            if let Err(message) = show_window(app) {
                app.state::<DesktopState>()
                    .recorder
                    .lock()
                    .unwrap()
                    .show_failed();
                crate::report_processing_error(
                    app,
                    snapshot
                        .session
                        .as_ref()
                        .map(|session| session.meeting_id.clone()),
                    message,
                );
            }
        }
        Some(crate::recorder_policy::Presentation::Hide) => {
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
            crate::window_position::keep_on_screen(&window);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn show_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("recorder") {
        window.set_always_on_top(true).map_err(window_error)?;
        #[cfg(target_os = "linux")]
        window
            .set_visible_on_all_workspaces(true)
            .map_err(window_error)?;
        return window.show().map_err(window_error);
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
    #[cfg(target_os = "linux")]
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
    builder.build().map(|_| ()).map_err(window_error)
}

#[cfg(not(target_os = "macos"))]
fn window_error(error: tauri::Error) -> String {
    let detail: String = error.to_string().chars().take(512).collect();
    format!("녹음 제어창을 열지 못했습니다. 앱 창에서 녹음을 조작해 주세요.\n{detail}")
}

fn dismiss(app: &tauri::AppHandle) -> Result<(), String> {
    app.state::<DesktopState>()
        .recorder
        .lock()
        .unwrap()
        .dismiss();
    #[cfg(target_os = "macos")]
    crate::macos_recorder::hide(app);
    if let Some(recorder) = app.get_webview_window("recorder") {
        recorder
            .hide()
            .map_err(|_| "제어창을 숨기지 못했습니다.".to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn show_recorder(
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
    sync(&app, &snapshot);
    Ok(())
}

#[tauri::command]
pub async fn set_auto_recorder(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    enabled: bool,
) -> Result<(), String> {
    require_main(&window)?;
    state.recorder.lock().unwrap().set_automatic(enabled);
    let snapshot = on_core(state.core.clone(), |core| core.snapshot()).await?;
    sync(&app, &snapshot);
    Ok(())
}

#[tauri::command]
pub fn drag_recorder(window: WebviewWindow) -> Result<(), String> {
    if window.label() != "recorder" {
        return Err("녹음 제어창에서 실행해 주세요.".into());
    }
    window
        .start_dragging()
        .map_err(|_| "창을 이동하지 못했습니다.".into())
}

#[tauri::command]
pub fn hide_recorder(window: WebviewWindow, app: tauri::AppHandle) -> Result<(), String> {
    require_local_window(&window)?;
    dismiss(&app)
}
