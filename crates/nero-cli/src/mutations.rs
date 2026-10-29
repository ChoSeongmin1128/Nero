use crate::{id_schema, QueryError};
use nero_core::agent_control;
use serde_json::{json, Map, Value};
use std::path::Path;

pub(crate) fn dispatch(root: &Path, method: &str, params: Value) -> Result<Value, QueryError> {
    agent_control::request(root, method, params).map_err(|error| {
        let message = match error.code {
            "invalid_input" => "수정 요청의 ID/입력값/수정본을 확인해 주세요.",
            "edit_conflict" if method == "job_cancel" => "작업 상태가 변경되었습니다. jobs_list로 현재 상태를 확인하세요.",
            "edit_conflict" if method == "summary_import" => "원문, 메모 또는 기존 요약이 변경되어 저장하지 않았습니다. 현재 내용을 확인해 요약을 다시 판단하세요. 같은 요청을 자동 반복하지 마세요.",
            "edit_conflict" => "회의나 폴더가 변경되었습니다. 최신 editVersion을 조회한 뒤 다시 요청하세요.",
            "meeting_not_found" => "회의를 찾을 수 없습니다.",
            "invalid_state" => "현재 회의 상태에서는 변경할 수 없습니다. 녹음/휴지통/처리 상태를 Nero에서 확인해 주세요.",
            "app_not_ready" => "Nero가 변경 요청을 처리할 수 없습니다. 저장이나 종료가 끝난 뒤 다시 시도해 주세요.",
            "storage_error" => "변경 내용을 저장하지 못했습니다. Nero에서 저장 상태를 확인해 주세요.",
            "request_too_large" => "변경 요청이 1MiB 한도를 넘습니다. 본문이나 대상 개수를 줄여 주세요.",
            "mutation_failed" => "변경 요청을 완료하지 못했습니다. Nero에서 저장 상태를 확인해 주세요.",
            "invalid_response" | "mutation_result_unknown" => "Nero의 응답을 확인할 수 없습니다. 최신 상태를 조회해 변경 여부를 먼저 확인하세요.",
            _ => "Nero와 연결할 수 없습니다. 앱을 실행한 뒤 다시 시도해 주세요.",
        };
        QueryError { code:error.code,message }
    })
}

pub(crate) fn schema(method: &str) -> Map<String, Value> {
    let mut fields = Map::new();
    let mut required = Vec::new();
    let version = json!({"type":"string","pattern":"^sha256:[0-9a-f]{64}$"});
    if method == "jobs_list" {
        fields.insert("meetingId".into(), id_schema());
        fields.insert(
            "offset".into(),
            json!({"type":"integer","minimum":0,"default":0}),
        );
        fields.insert(
            "limit".into(),
            json!({"type":"integer","minimum":1,"maximum":200,"default":50}),
        );
    } else if method == "job_cancel" {
        fields.insert("jobId".into(), id_schema());
        fields.insert("expectedVersion".into(), version.clone());
        required.extend(["jobId", "expectedVersion"]);
    } else if method == "summary_import" {
        fields.insert("meetingId".into(), id_schema());
        let mut nullable_id = id_schema();
        nullable_id["type"] = json!(["string", "null"]);
        fields.insert("context".into(), json!({
            "type":"object",
            "properties":{
                "transcriptVersion":{"type":"object","properties":{"id":id_schema(),"revision":{"type":"integer","minimum":0}},"required":["id","revision"],"additionalProperties":false},
                "transcriptSha256":{"type":"string","pattern":"^[0-9a-f]{64}$"},
                "notesSha256":{"type":"string","pattern":"^[0-9a-f]{64}$"},
                "summaryId":nullable_id
            },
            "required":["transcriptVersion","transcriptSha256","notesSha256","summaryId"],
            "additionalProperties":false,
            "description":"Exact context captured by summary_context before reading source pages. Never replace with sourceVersion or editVersion."
        }));
        fields.insert("text".into(), json!({"type":"string","minLength":1,"description":"Complete generated summary. Citations use [reference] from transcript_get segment reference values."}));
        fields.insert("producer".into(), json!({
            "type":"object",
            "properties":{
                "name":{"type":"string","minLength":1,"maxLength":128},
                "model":{"type":"string","minLength":1,"maxLength":256},
                "modelRevision":{"type":"string","minLength":1,"maxLength":256},
                "promptVersion":{"type":"string","minLength":1,"maxLength":256},
                "template":{"type":"string","minLength":1,"maxLength":256}
            },
            "required":["name"],"additionalProperties":false,
            "description":"Agent-reported provenance, not Nero-verified model identity. Omit unknown optional fields. UTF-8 byte limits match field maxima."
        }));
        required.extend(["meetingId", "context", "text", "producer"]);
    } else if matches!(method, "meeting_update" | "meeting_date_set") {
        fields.insert("meetingId".into(), id_schema());
        fields.insert("expectedVersion".into(), version.clone());
        required.extend(["meetingId", "expectedVersion"]);
        if method == "meeting_update" {
            fields.insert("title".into(), json!({"type":"string"}));
            fields.insert("notes".into(), json!({"type":"string"}));
        } else {
            fields.insert("meetingDate".into(), json!({"type":["string","null"],"pattern":"^[0-9]{4}-[0-9]{2}-[0-9]{2}$","description":"Stored meeting date, not recording time. null clears it."}));
            required.push("meetingDate");
        }
    } else if matches!(
        method,
        "meetings_move" | "meetings_trash" | "meetings_restore"
    ) {
        fields.insert("expectedVersions".into(), json!({"type":"object","minProperties":1,"maxProperties":200,"propertyNames":{"pattern":"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"},"additionalProperties":version,"description":"Each target meeting ID maps to its current editVersion."}));
        required.push("expectedVersions");
        if method == "meetings_move" {
            fields.insert("folderId".into(), json!({"type":["string","null"],"pattern":"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$","description":"Existing destination folder ID; null means unfiled."}));
            required.push("folderId");
        }
    } else {
        fields.insert("expectedVersion".into(), version);
        required.push("expectedVersion");
        if method != "folder_create" {
            fields.insert("folderId".into(), id_schema());
            required.push("folderId");
        }
        if method != "folder_delete" {
            fields.insert("name".into(), json!({"type":"string","minLength":1}));
            required.push("name");
        }
    }
    let mut schema = json!({"type":"object","properties":fields,"required":required,"additionalProperties":false});
    if method == "meeting_update" {
        schema["anyOf"] = json!([{"required":["title"]},{"required":["notes"]}]);
    }
    schema.as_object().unwrap().clone()
}
