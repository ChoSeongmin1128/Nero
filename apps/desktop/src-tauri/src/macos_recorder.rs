use crate::{DesktopState, RecordingAction};
use nero_core::{CaptureQuality, CaptureQualityEventKind, SessionState, Snapshot};
use serde::Serialize;
use std::ffi::{c_char, CStr, CString};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use tauri::{Emitter, Manager};

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PanelUpdate<'a> {
    generation: u64,
    revision: u64,
    visible: bool,
    #[serde(rename = "sessionID")]
    session_id: Option<&'a str>,
    meeting_title: &'a str,
    phase: &'static str,
    #[serde(rename = "elapsedMS")]
    elapsed_ms: u64,
    error: Option<&'a str>,
    warning: Option<&'a str>,
    preferred_display: u32,
}

extern "C" {
    fn nero_recorder_update(
        bytes: *const u8,
        length: usize,
        callback: extern "C" fn(*const c_char, *const c_char),
    );
    fn nero_recorder_hide(generation: u64);
    fn nero_recorder_complete(session: *const c_char, error: *const c_char);
}

pub fn sync(app: &tauri::AppHandle, snapshot: &Snapshot, visible: bool) {
    APP.get_or_init(|| app.clone());
    let session = snapshot.session.as_ref();
    let meeting = session.and_then(|s| snapshot.meetings.iter().find(|m| m.id == s.meeting_id));
    let warning =
        session.and_then(|s| attention_warning(s.capture_quality.as_ref(), s.warning.as_deref()));
    let update = PanelUpdate {
        generation: GENERATION.fetch_add(1, Ordering::Relaxed) + 1,
        revision: snapshot.revision,
        visible,
        session_id: session.map(|s| s.id.as_str()),
        meeting_title: meeting.map(|m| m.title.as_str()).unwrap_or("회의"),
        phase: match session.map(|s| &s.state) {
            Some(SessionState::Recording) => "recording",
            Some(SessionState::Paused) => "paused",
            _ => "idle",
        },
        elapsed_ms: session.map(|s| s.elapsed_ms).unwrap_or(0),
        error: session.and_then(|s| s.error.as_deref()),
        warning,
        preferred_display: 0,
    };
    if let Ok(bytes) = serde_json::to_vec(&update) {
        let _ = app.run_on_main_thread(move || unsafe {
            nero_recorder_update(bytes.as_ptr(), bytes.len(), action)
        });
    }
}

fn attention_warning<'a>(
    quality: Option<&CaptureQuality>,
    warning: Option<&'a str>,
) -> Option<&'a str> {
    let informative_only = quality.is_some_and(|q| {
        (q.xrun_count > 0 || q.realtime_denied_count > 0 || !q.events.is_empty())
            && q.route_changed_count == 0
            && q.events.iter().all(|e| {
                matches!(
                    e.kind,
                    CaptureQualityEventKind::Xrun | CaptureQualityEventKind::RealtimeDenied
                )
            })
    });
    (!informative_only).then_some(warning).flatten()
}

pub fn hide(app: &tauri::AppHandle) {
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    let _ = app.run_on_main_thread(move || unsafe { nero_recorder_hide(generation) });
}

extern "C" fn action(action: *const c_char, session: *const c_char) {
    if action.is_null() || session.is_null() {
        return;
    }
    let (Ok(action), Ok(session)) = (
        unsafe { CStr::from_ptr(action) }.to_str(),
        unsafe { CStr::from_ptr(session) }.to_str(),
    ) else {
        return;
    };
    let Some(app) = APP.get().cloned() else {
        return;
    };
    if action == "hide" {
        if app
            .state::<DesktopState>()
            .recorder
            .lock()
            .unwrap()
            .dismiss_session(session)
        {
            hide(&app);
        }
        return;
    }
    let requested = match action {
        "pause" => Some(RecordingAction::Pause),
        "resume" => Some(RecordingAction::Resume),
        "stop" => Some(RecordingAction::Stop),
        "open" => None,
        _ => return,
    };
    let session = session.to_owned();
    tauri::async_runtime::spawn(async move {
        let result = if let Some(action) = requested {
            crate::change_recording(app.clone(), session.clone(), action)
                .await
                .map(|_| ())
        } else {
            let core = app.state::<DesktopState>().core.clone();
            let expected = session.clone();
            match crate::on_core(core, |core| core.snapshot()).await {
                Ok(snapshot) => {
                    if let Some(current) = snapshot.session.filter(|s| s.id == expected) {
                        crate::show_main(&app);
                        app.emit_to(
                            "main",
                            "nero://open-meeting",
                            crate::OpenMeeting {
                                meeting_id: current.meeting_id,
                                error: current.error,
                            },
                        )
                        .map_err(|_| "회의를 열지 못했습니다.".to_string())
                    } else {
                        Err("녹음이 종료되었습니다.".into())
                    }
                }
                Err(error) => Err(error),
            }
        };
        let error = result.err();
        let session = CString::new(session).unwrap();
        let message = error.and_then(|s| CString::new(s.replace('\0', "")).ok());
        let _ = app.run_on_main_thread(move || unsafe {
            nero_recorder_complete(
                session.as_ptr(),
                message.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            )
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn informational_diagnostics_do_not_replace_the_recording_indicator() {
        let info = CaptureQuality {
            xrun_count: 1,
            ..Default::default()
        };
        assert_eq!(attention_warning(Some(&info), Some("input timing")), None);
        let route = CaptureQuality {
            route_changed_count: 1,
            ..info
        };
        assert_eq!(
            attention_warning(Some(&route), Some("device changed")),
            Some("device changed")
        );
        assert_eq!(
            attention_warning(None, Some("unknown warning")),
            Some("unknown warning")
        );
        assert_eq!(
            attention_warning(Some(&CaptureQuality::default()), Some("unknown warning")),
            Some("unknown warning")
        );
    }
}
