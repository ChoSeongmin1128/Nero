use crate::{export, on_core, require_main, DesktopState};
use tauri::WebviewWindow;
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub(crate) async fn reveal_recording_file(
    window: WebviewWindow,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
) -> Result<(), String> {
    require_main(&window)?;
    let path = on_core(state.core.clone(), move |core| core.audio_path(&meeting_id))
        .await
        .map_err(|_| "녹음 파일을 찾거나 재생할 수 없습니다. 녹음을 확인해 주세요.".to_string())?;
    tauri::async_runtime::spawn_blocking(move || tauri_plugin_opener::reveal_item_in_dir(path))
        .await
        .map_err(|_| "녹음 파일의 위치를 열지 못했습니다.".to_string())?
        .map_err(|_| "녹음 파일의 위치를 열지 못했습니다.".to_string())
}

// The stored audio.wav keeps its stable name; only the exported copy carries the title.
#[tauri::command]
pub(crate) async fn export_recording_file(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
    meeting_id: String,
) -> Result<bool, String> {
    require_main(&window)?;
    let (source, meeting) = on_core(state.core.clone(), move |core| {
        let source = core.audio_path(&meeting_id)?;
        let meeting = core
            .list_meetings()?
            .into_iter()
            .find(|meeting| meeting.id == meeting_id)
            .ok_or(nero_core::CoreError::MeetingNotFound)?;
        Ok((source, meeting))
    })
    .await
    .map_err(|_| "녹음 파일을 찾거나 재생할 수 없습니다. 녹음을 확인해 주세요.".to_string())?;
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .ok_or("녹음 파일 형식을 확인하지 못했습니다.")?
        .to_ascii_lowercase();
    let name = export::file_name(&recording_stem(&meeting), &extension);
    let selected = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_parent(&window)
            .set_title("녹음 파일 내보내기")
            .add_filter("녹음 파일", &[extension.as_str()])
            .set_file_name(name)
            .blocking_save_file()
    })
    .await
    .map_err(|_| "저장 창을 열지 못했습니다.")?;
    let Some(selected) = selected else {
        return Ok(false);
    };
    let path = selected
        .into_path()
        .map_err(|_| "파일을 저장할 위치를 선택해 주세요.")?;
    tauri::async_runtime::spawn_blocking(move || export::copy_file(&source, &path))
        .await
        .map_err(|_| "녹음 파일을 저장하지 못했습니다.")??;
    Ok(true)
}

fn recording_stem(meeting: &nero_core::Meeting) -> String {
    let title = match meeting.title.trim() {
        "" => "제목 없는 회의",
        title => title,
    };
    match meeting.meeting_date.as_deref() {
        Some(day) if !title.contains(day) => format!("{title} {day}"),
        _ => title.to_owned(),
    }
}
