use crate::calendar::{self, CalendarService, CalendarSnapshot, Occurrence};
use crate::{microphone_permission, DesktopState};
use grove_core::{SessionState, Snapshot};
use std::path::PathBuf;
use std::sync::{atomic::Ordering, Arc};
use std::time::Duration;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const PREFIX: &str = "calendar-";
const SETTINGS_EVENT: &str = "nero://calendar-settings";

pub struct CalendarDesktop {
    service: Option<Arc<CalendarService>>,
    startup_error: Option<String>,
}
impl CalendarDesktop {
    fn service(&self) -> Result<Arc<CalendarService>, String> {
        self.service.clone().ok_or_else(|| {
            self.startup_error
                .clone()
                .unwrap_or_else(|| "캘린더를 준비하지 못했습니다.".into())
        })
    }
    fn snapshot(&self) -> CalendarSnapshot {
        self.service
            .as_ref()
            .map(|service| service.snapshot())
            .unwrap_or_else(|| CalendarSnapshot {
                revision: 0,
                configured: false,
                connected: false,
                connecting: false,
                restoring: false,
                account_label: None,
                calendars: vec![],
                selected_calendars: vec![],
                last_synced_at: None,
                error: self.startup_error.clone(),
            })
    }
}

/// Calendar corruption is isolated here; the meeting library still opens normally.
pub fn install(app: &tauri::AppHandle, root: PathBuf) {
    let (service, startup_error) = match CalendarService::new(root) {
        Ok(service) => (Some(Arc::new(service)), None),
        Err(error) => (None, Some(error)),
    };
    app.manage(CalendarDesktop {
        service: service.clone(),
        startup_error,
    });
    let Some(service) = service else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut previous_revision = None;
        loop {
            let state = app.state::<DesktopState>();
            if !state.polling.load(Ordering::Acquire) {
                break;
            }
            let core = state.core.clone();
            drop(state);
            if let Ok(snapshot) = crate::on_core(core, |core| core.snapshot()).await {
                let _ = service.tick(calendar::now_ms(), recording_busy(&snapshot));
                sync_windows(&app, &service);
            }
            let snapshot = service.snapshot();
            if previous_revision != Some(snapshot.revision) {
                previous_revision = Some(snapshot.revision);
                let _ = app.emit_to("main", SETTINGS_EVENT, snapshot);
            }
            if service.should_sync(calendar::now_ms()) {
                let service = service.clone();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = service.sync_now().await;
                    publish(&app);
                });
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

fn publish(app: &tauri::AppHandle) {
    let _ = app.emit_to(
        "main",
        SETTINGS_EVENT,
        app.state::<CalendarDesktop>().snapshot(),
    );
}
fn recording_busy(snapshot: &Snapshot) -> bool {
    snapshot.session.as_ref().is_some_and(|session| {
        matches!(
            session.state,
            SessionState::Recording | SessionState::Paused
        )
    })
}
fn label(key: &str) -> Option<String> {
    let digest = key.strip_prefix("google:")?;
    (digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| format!("{PREFIX}{digest}"))
}
fn key(window: &WebviewWindow) -> Result<String, String> {
    let digest = window
        .label()
        .strip_prefix(PREFIX)
        .ok_or("알림 창에서 실행해 주세요.")?;
    let key = format!("google:{digest}");
    if label(&key).as_deref() != Some(window.label()) {
        return Err("알림을 확인하지 못했습니다.".into());
    }
    Ok(key)
}
fn sync_windows(app: &tauri::AppHandle, service: &CalendarService) {
    let monitor = app
        .get_webview_window("main")
        .and_then(|main| main.current_monitor().ok().flatten());
    let reminders = service.reminders();
    let presented = presented_reminder(&reminders);
    let wanted = presented.and_then(|event| label(&event.key));
    for (name, window) in app.webview_windows() {
        if name.starts_with(PREFIX) && wanted.as_deref() != Some(name.as_str()) {
            let _ = window.destroy();
        }
    }
    if let (Some(event), Some(name)) = (presented, wanted) {
        if let Some(window) = app.get_webview_window(&name) {
            let _ = window.emit("nero://calendar-reminder", event);
            crate::recover_recorder_position(&window);
            return;
        }
        let mut builder =
            WebviewWindowBuilder::new(app, &name, WebviewUrl::App("calendar.html".into()))
                .title("Nero 일정 알림")
                .inner_size(440.0, 40.0)
                .resizable(false)
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .visible(false)
                .focused(false)
                .focusable(false)
                .accept_first_mouse(true);
        if let Some(monitor) = &monitor {
            let area = monitor.work_area();
            let scale = monitor.scale_factor();
            let x = (f64::from(area.position.x) + f64::from(area.size.width)) / scale - 456.0;
            let y = f64::from(area.position.y) / scale + 16.0;
            builder = builder.position(x, y);
        }
        if let Ok(window) = builder.build() {
            crate::recover_recorder_position(&window);
        }
    }
}

fn presented_reminder(reminders: &[Occurrence]) -> Option<&Occurrence> {
    reminders.iter().find(|event| label(&event.key).is_some())
}

pub fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) -> bool {
    if !window.label().starts_with(PREFIX) {
        return false;
    }
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        if let Some(webview) = window.app_handle().get_webview_window(window.label()) {
            if let (Ok(key), Ok(service)) = (
                key(&webview),
                window.app_handle().state::<CalendarDesktop>().service(),
            ) {
                if service.dismiss(&key).is_ok() {
                    sync_windows(window.app_handle(), &service);
                    publish(window.app_handle());
                }
            }
        }
    }
    true
}

#[tauri::command]
pub fn read_calendar_settings(
    window: WebviewWindow,
    state: tauri::State<'_, CalendarDesktop>,
) -> Result<CalendarSnapshot, String> {
    crate::require_main(&window)?;
    Ok(state.snapshot())
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectAttempt {
    attempt_id: String,
}

#[tauri::command]
pub async fn begin_calendar_connect(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, CalendarDesktop>,
) -> Result<ConnectAttempt, String> {
    crate::require_main(&window)?;
    let service = state.service()?;
    let request = service.begin_connect()?;
    publish(&app);
    let url = request.authorization_url;
    let opened = tauri::async_runtime::spawn_blocking(move || open_authorization(&url))
        .await
        .map_err(|_| "기본 브라우저를 열지 못했습니다.".to_string())
        .and_then(|result| result);
    if let Err(error) = opened {
        service.cancel_connect();
        publish(&app);
        return Err(error);
    }
    Ok(ConnectAttempt {
        attempt_id: request.attempt_id,
    })
}

#[tauri::command]
pub async fn finish_calendar_connect(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, CalendarDesktop>,
    attempt_id: String,
) -> Result<CalendarSnapshot, String> {
    crate::require_main(&window)?;
    let result = state.service()?.finish_connect(&attempt_id).await;
    publish(&app);
    if result.is_ok() {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    result
}
#[tauri::command]
pub fn cancel_calendar_connect(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, CalendarDesktop>,
    attempt_id: Option<String>,
) -> Result<CalendarSnapshot, String> {
    crate::require_main(&window)?;
    state
        .service()?
        .cancel_connect_attempt(attempt_id.as_deref());
    publish(&app);
    Ok(state.snapshot())
}
#[tauri::command]
pub async fn disconnect_calendar(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, CalendarDesktop>,
) -> Result<CalendarSnapshot, String> {
    crate::require_main(&window)?;
    let service = state.service()?;
    let result = service.disconnect().await;
    sync_windows(&app, &service);
    publish(&app);
    result
}
#[tauri::command]
pub fn select_calendars(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, CalendarDesktop>,
    ids: Vec<String>,
) -> Result<CalendarSnapshot, String> {
    crate::require_main(&window)?;
    let service = state.service()?;
    let snapshot = service.set_calendars(ids)?;
    let _ = service.tick(calendar::now_ms(), false);
    sync_windows(&app, &service);
    publish(&app);
    Ok(snapshot)
}
#[tauri::command]
pub async fn sync_calendar(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, CalendarDesktop>,
) -> Result<CalendarSnapshot, String> {
    crate::require_main(&window)?;
    let result = state.service()?.sync_now().await;
    publish(&app);
    result
}
#[tauri::command]
pub fn read_calendar_reminder(
    window: WebviewWindow,
    state: tauri::State<'_, CalendarDesktop>,
) -> Result<Option<Occurrence>, String> {
    let key = key(&window)?;
    Ok(state.service()?.reminder(&key))
}
#[tauri::command]
pub fn calendar_reminder_ready(
    window: WebviewWindow,
    state: tauri::State<'_, CalendarDesktop>,
) -> Result<(), String> {
    let key = key(&window)?;
    if state.service()?.reminder(&key).is_none() {
        return Err("일정 알림이 끝났습니다.".into());
    }
    window
        .show()
        .map_err(|_| "일정 알림을 표시하지 못했습니다.".into())
}
#[tauri::command]
pub fn dismiss_calendar_reminder(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, CalendarDesktop>,
) -> Result<(), String> {
    let key = key(&window)?;
    let service = state.service()?;
    service.dismiss(&key)?;
    sync_windows(&app, &service);
    publish(&app);
    Ok(())
}
#[tauri::command]
pub async fn start_calendar_recording(
    app: tauri::AppHandle,
    window: WebviewWindow,
    calendar: tauri::State<'_, CalendarDesktop>,
    state: tauri::State<'_, DesktopState>,
) -> Result<Snapshot, String> {
    let key = key(&window)?;
    let service = calendar.service()?;
    state.processing.require_meeting_models()?;
    let snapshot = crate::on_core(state.core.clone(), |core| core.snapshot()).await?;
    if recording_busy(&snapshot) {
        return Err("진행 중인 녹음을 마친 뒤 시작해 주세요.".into());
    }
    microphone_permission::ensure_authorized(&app).await?;
    // A click never acts on cached event text, a cancelled event, or a partial sync.
    service.sync_now().await?;
    service.tick(calendar::now_ms(), false)?;
    let ticket = service.reminder_ticket(&key, calendar::now_ms())?;
    let core = state.core.clone();
    let processing = state.processing.clone();
    let recording_calendar = service.clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || {
        processing.recording_start(|| {
            let current = core.snapshot().map_err(|error| error.to_string())?;
            if recording_busy(&current) {
                return Err("진행 중인 녹음을 마친 뒤 시작해 주세요.".into());
            }
            let selected = core
                .devices()
                .map_err(|error| error.to_string())?
                .into_iter()
                .find(|device| device.is_default)
                .map(|device| device.id)
                .unwrap_or_default();
            recording_calendar.with_current_reminder(&ticket, calendar::now_ms(), |event| {
                core.create_recording(&event.title, "", &selected)
                    .map_err(|error| error.to_string())
            })
        })
    })
    .await
    .map_err(|_| "녹음을 시작하지 못했습니다.".to_string())??;
    let _ = service.dismiss(&key);
    let _ = service.tick(calendar::now_ms(), true);
    sync_windows(&app, &service);
    crate::publish(&app, &snapshot);
    crate::show_main(&app);
    if let Some(session) = &snapshot.session {
        let _ = app.emit_to(
            "main",
            "grove://open-meeting",
            serde_json::json!({"meetingId":session.meeting_id,"error":null}),
        );
    }
    Ok(snapshot)
}

fn open_authorization(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "Google 연결 주소를 확인하지 못했습니다.")?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("accounts.google.com")
        || parsed.path() != "/o/oauth2/v2/auth"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("Google 연결 주소를 확인하지 못했습니다.".into());
    }
    #[cfg(target_os = "macos")]
    if std::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .is_ok_and(|status| status.success())
    {
        return Ok(());
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
        let wide = |value: &str| {
            std::ffi::OsStr::new(value)
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        let operation = wide("open");
        let target = wide(url);
        if unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        } > 32
        {
            return Ok(());
        }
    }
    Err("기본 브라우저를 열지 못했습니다. 다시 연결해 주세요.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reminder_labels_only_accept_generated_occurrence_ids() {
        assert!(label("google:../main").is_none());
        assert!(label("main").is_none());
        assert_eq!(
            label(&format!("google:{}", "a".repeat(64))),
            Some(format!("calendar-{}", "a".repeat(64)))
        );
    }
    #[test]
    fn only_the_first_pending_occurrence_is_presented_and_dismissal_advances_it() {
        let occurrence = |letter: &str| Occurrence {
            key: format!("google:{}", letter.repeat(64)),
            title: "Synthetic".into(),
            start_at: 900_000,
            end_at: 1_200_000,
            calendar_name: "Synthetic".into(),
            join_urls: vec![],
        };
        let first = occurrence("a");
        let second = occurrence("b");
        let third = occurrence("c");
        let pending = vec![first.clone(), second.clone(), third.clone()];
        assert_eq!(
            presented_reminder(&pending).map(|event| event.key.as_str()),
            Some(first.key.as_str())
        );
        let remaining = vec![second.clone(), third.clone()];
        assert_eq!(
            presented_reminder(&remaining).map(|event| event.key.as_str()),
            Some(second.key.as_str())
        );
        assert_eq!(
            presented_reminder(&[third.clone()]).map(|event| event.key.clone()),
            Some(third.key)
        );
        assert!(presented_reminder(&[]).is_none());
    }
}
