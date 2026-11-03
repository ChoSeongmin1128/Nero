use crate::{on_core, require_main, DesktopState};
use tauri::WebviewWindow;

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
