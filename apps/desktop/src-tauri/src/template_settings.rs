use nero_core::templates::{
    PromptTemplate, TemplateCatalog, TemplateContent, TemplateStore, TemplateSummary,
};
use tauri::{AppHandle, Manager, WebviewWindow};

fn store(app: &AppHandle, window: &WebviewWindow) -> Result<TemplateStore, String> {
    super::require_main(window)?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|_| "템플릿 위치를 확인하지 못했습니다.")?;
    Ok(TemplateStore::new(root))
}

#[tauri::command]
pub fn list_templates(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Vec<TemplateSummary>, String> {
    store(&app, &window)?
        .list()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn read_template_catalog(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<TemplateCatalog, String> {
    store(&app, &window)?
        .catalog()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn reset_template_catalog(
    app: AppHandle,
    window: WebviewWindow,
    expected_revision: String,
) -> Result<TemplateCatalog, String> {
    store(&app, &window)?
        .reset_all(&expected_revision)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_template(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
) -> Result<PromptTemplate, String> {
    store(&app, &window)?
        .get(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn create_template(
    app: AppHandle,
    window: WebviewWindow,
    content: TemplateContent,
) -> Result<PromptTemplate, String> {
    store(&app, &window)?
        .create(content)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn update_template(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
    expected_revision: String,
    content: TemplateContent,
) -> Result<PromptTemplate, String> {
    store(&app, &window)?
        .update(&id, &expected_revision, content)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn reset_template(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
    expected_revision: String,
) -> Result<PromptTemplate, String> {
    store(&app, &window)?
        .reset(&id, &expected_revision)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_template(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
    expected_revision: String,
) -> Result<(), String> {
    store(&app, &window)?
        .delete(&id, &expected_revision)
        .map_err(|error| error.to_string())
}
