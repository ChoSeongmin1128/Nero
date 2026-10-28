use crate::QueryError;
use grove_core::templates::{
    TemplateContent, TemplateError, TemplateStore, MAX_TEMPLATE_PROMPT_BYTES,
};
use serde_json::{json, Map, Value};
use std::path::Path;

pub(crate) fn dispatch(root: &Path, method: &str, params: Value) -> Result<Value, QueryError> {
    let mut params = params.as_object().ok_or_else(QueryError::invalid)?.clone();
    let store = TemplateStore::new(root);
    if method == "templates_list" {
        let deleted = params
            .remove("deleted")
            .map(|value| value.as_bool().ok_or_else(QueryError::invalid))
            .transpose()?
            .unwrap_or(false);
        if !params.is_empty() {
            return Err(QueryError::invalid());
        }
        let items = if deleted {
            store.list_deleted()
        } else {
            store.list()
        }
        .map_err(error)?;
        return Ok(json!({"apiVersion":1,"templates":items}));
    }
    let id = if method == "template_create" {
        None
    } else {
        Some(take_string(&mut params, "templateId")?)
    };
    let revision = if matches!(method, "template_get" | "template_create") {
        None
    } else {
        Some(take_string(&mut params, "expectedRevision")?)
    };
    if method == "template_get" {
        if !params.is_empty() {
            return Err(QueryError::invalid());
        }
        return Ok(
            json!({"apiVersion":1,"template":store.get(id.as_deref().unwrap()).map_err(error)?}),
        );
    }
    let summary = match method {
        "template_create" | "template_update" => {
            let content: TemplateContent =
                serde_json::from_value(Value::Object(params)).map_err(|_| QueryError::invalid())?;
            let saved = if method == "template_create" {
                store.create(content)
            } else {
                store.update(
                    id.as_deref().unwrap(),
                    revision.as_deref().unwrap(),
                    content,
                )
            }
            .map_err(error)?;
            saved.summary()
        }
        "template_delete" | "template_reset" | "template_restore" => {
            if !params.is_empty() {
                return Err(QueryError::invalid());
            }
            let id = id.as_deref().unwrap();
            let revision = revision.as_deref().unwrap();
            match method {
                "template_delete" => store.delete(id, revision),
                "template_reset" => store.reset(id, revision).map(|value| value.summary()),
                _ => store.restore(id, revision).map(|value| value.summary()),
            }
            .map_err(error)?
        }
        _ => return Err(QueryError::invalid()),
    };
    Ok(json!({"apiVersion":1,"template":summary}))
}

fn take_string(params: &mut Map<String, Value>, name: &str) -> Result<String, QueryError> {
    params
        .remove(name)
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|value| !value.is_empty())
        .ok_or_else(QueryError::invalid)
}

fn error(error: TemplateError) -> QueryError {
    let (code, message) = match error {
        TemplateError::InvalidInput => (
            "invalid_input",
            "템플릿 이름/설명/프롬프트와 ID/수정본 형식을 확인해 주세요.",
        ),
        TemplateError::NotFound => (
            "template_not_found",
            "템플릿을 찾을 수 없습니다. templates list 또는 삭제 목록을 확인해 주세요.",
        ),
        TemplateError::Conflict => (
            "template_revision_conflict",
            "템플릿이 변경되었습니다. 최신 수정본을 조회한 뒤 다시 저장해 주세요.",
        ),
        TemplateError::BuiltinProtected => (
            "template_builtin_protected",
            "기본 템플릿은 삭제 대신 기본값으로 되돌리기를 사용하세요.",
        ),
        TemplateError::Busy => (
            "template_busy",
            "다른 템플릿 변경을 처리 중입니다. 잠시 후 다시 시도해 주세요.",
        ),
        TemplateError::InvalidDocument => (
            "invalid_template_document",
            "저장된 템플릿 정보가 손상되었습니다. Nero에서 확인해 주세요.",
        ),
        TemplateError::UnsupportedFormat => (
            "unsupported_format",
            "지원하지 않는 템플릿 저장 형식입니다. Nero와 CLI를 업데이트해 주세요.",
        ),
        TemplateError::UnsafePath => (
            "unsafe_path",
            "템플릿 경로에 연결된 파일이나 올바르지 않은 항목이 있습니다.",
        ),
        TemplateError::Storage => (
            "template_unavailable",
            "템플릿을 읽거나 저장하지 못했습니다. 경로와 접근 권한을 확인해 주세요.",
        ),
    };
    QueryError { code, message }
}

pub(crate) fn schema(method: &str) -> Map<String, Value> {
    let mut fields = Map::new();
    let mut required = Vec::new();
    if method == "templates_list" {
        fields.insert(
            "deleted".into(),
            json!({"type":"boolean","description":"Only deleted custom templates; default false."}),
        );
    } else {
        if method != "template_create" {
            fields.insert("templateId".into(),json!({"type":"string","minLength":1,"description":"Built-in template ID or custom UUID returned by templates_list."}));
            required.push("templateId");
        }
        if !matches!(method, "template_get" | "template_create") {
            fields.insert("expectedRevision".into(),json!({"type":"string","pattern":"^sha256:[0-9a-f]{64}$","description":"Current revision from template_get or deleted templates_list."}));
            required.push("expectedRevision");
        }
        if matches!(method, "template_create" | "template_update") {
            fields.insert(
                "name".into(),
                json!({"type":"string","minLength":1,"maxLength":256}),
            );
            fields.insert(
                "description".into(),
                json!({"type":"string","maxLength":4096}),
            );
            fields.insert("prompt".into(),json!({"type":"string","minLength":1,"maxLength":MAX_TEMPLATE_PROMPT_BYTES,"description":"Template text, maximum 64KiB UTF-8; not a meeting transcript."}));
            required.extend(["name", "description", "prompt"]);
        }
    }
    json!({"type":"object","properties":fields,"required":required,"additionalProperties":false})
        .as_object()
        .unwrap()
        .clone()
}
