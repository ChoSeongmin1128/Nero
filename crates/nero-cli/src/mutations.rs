use crate::{id_schema, QueryError};
use nero_core::agent_control;
use serde_json::{json, Map, Value};
use std::path::Path;

pub(crate) fn dispatch(root: &Path, method: &str, params: Value) -> Result<Value, QueryError> {
    agent_control::request(root, method, params).map_err(|error| {
        let message = match error.code {
            "invalid_input" => "수정 요청의 ID/입력값/수정본을 확인해 주세요.",
            "edit_conflict" if method == "job_cancel" => "작업 상태가 변경되었습니다. jobs_list로 현재 상태를 확인하세요.",
            "edit_conflict" if method == "speech_switch_cancel" => "대기 중인 전환 대상이 바뀌었습니다. status의 speech.switch를 다시 확인하세요.",
            "invalid_state" if method == "meeting_import" => "음성과 음성 기록이 없는 빈 새 회의에만 가져올 수 있습니다. meetingId 없이 새 회의로 가져오거나 대상 회의를 다시 확인해 주세요.",
            "edit_conflict" if method == "summary_import" => "원문, 메모 또는 기존 요약이 변경되어 저장하지 않았습니다. 현재 내용을 확인해 요약을 다시 판단하세요. 같은 요청을 자동 반복하지 마세요.",
            "edit_conflict" => "회의나 폴더가 변경되었습니다. 최신 editVersion을 조회한 뒤 다시 요청하세요.",
            "meeting_not_found" => "회의를 찾을 수 없습니다.",
            "invalid_state" => "현재 회의 상태에서는 변경할 수 없습니다. 녹음/휴지통/처리 상태를 Nero에서 확인해 주세요.",
            "app_not_ready" => "Nero가 변경 요청을 처리할 수 없습니다. 저장이나 종료가 끝난 뒤 다시 시도해 주세요.",
            "storage_error" => "변경 내용을 저장하지 못했습니다. Nero에서 저장 상태를 확인해 주세요.",
            "request_too_large" => "변경 요청이 1MiB 한도를 넘습니다. 본문이나 대상 개수를 줄여 주세요.",
            "mutation_failed" => "변경 요청을 완료하지 못했습니다. Nero에서 저장 상태를 확인해 주세요.",
            "invalid_response" | "mutation_result_unknown" => "Nero의 응답을 확인할 수 없습니다. 최신 상태를 조회해 변경 여부를 먼저 확인하세요.",
            "app_feature_unsupported" => "실행 중인 Nero는 이 요청을 지원하지 않습니다. Nero를 업데이트한 뒤 다시 시도해 주세요.",
            "audio_file_unavailable" => "가져올 음성 파일을 찾을 수 없거나 일반 파일이 아닙니다. 링크가 아닌 실제 파일의 절대 경로를 지정해 주세요.",
            "unsupported_audio_format" => "지원하지 않는 음성 파일 형식입니다. WAV, M4A, MP3, AAC, AIFF, CAF, FLAC, OGG 파일을 지정해 주세요.",
            "recording_active" => "녹음 중에는 처리할 수 없습니다. 녹음을 마친 뒤 다시 시도해 주세요.",
            "speech_not_ready" => "음성 인식이 준비되지 않았습니다. Nero에서 음성 인식을 먼저 준비해 주세요.",
            "speech_engine_unknown" => "실행 중인 Nero가 알지 못하는 엔진입니다. status의 speech.choices에서 id를 확인하세요.",
            "speech_engine_unsupported" => "이 기기에서 사용할 수 없는 엔진입니다. status의 speech.choices에서 supported를 확인하세요.",
            "memory_warning_unacknowledged" => "이 엔진에는 메모리 경고가 있습니다. status의 memoryWarning을 사용자에게 알리고 명시적으로 확인받은 경우에만 acknowledgeMemoryWarning:true로 다시 요청하세요.",
            "speech_switch_not_found" => "대기 중인 음성 인식 전환이 없습니다. status의 speech.switch를 확인하세요.",
            "processing_busy" => "녹음, 처리 또는 다운로드 작업이 대기/실행 중입니다. jobs_list로 확인하고 작업이 끝난 뒤 다시 시도하세요.",
            "processing_rejected" => "Nero가 요청을 처리하지 못했습니다. jobs_list와 status로 현재 상태를 확인하세요. 같은 요청을 자동 반복하지 마세요.",
            _ => "Nero와 연결할 수 없습니다. 앱을 실행한 뒤 다시 시도해 주세요.",
        };
        QueryError { code:error.code,message }
    })
}

// The running app's speech engine state for `status`. A stopped or older app is a
// state, not a status failure, so the read stays available without the app.
pub(crate) fn speech_status(root: &Path) -> Value {
    match agent_control::request(root, "speech_status", json!({})) {
        Ok(Value::Object(mut speech)) => {
            speech.insert("state".into(), json!("running"));
            Value::Object(speech)
        }
        Ok(_) => json!({"state":"unavailable","reason":"invalid_response"}),
        Err(error) if error.code == "app_feature_unsupported" => {
            json!({"state":"unsupported","reason":error.code})
        }
        Err(error) => json!({"state":"unavailable","reason":error.code}),
    }
}

fn engine_schema(description: &str) -> Value {
    json!({"type":"string","pattern":"^[A-Za-z0-9]{1,64}$","description":description})
}

pub(crate) fn schema(method: &str) -> Map<String, Value> {
    let mut fields = Map::new();
    let mut required = Vec::new();
    let version = json!({"type":"string","pattern":"^sha256:[0-9a-f]{64}$"});
    if method == "meeting_import" {
        fields.insert("path".into(), json!({"type":"string","minLength":2,"maxLength":4096,"description":format!(
            "Absolute path of an existing regular local audio file, not a symlink and without . or .. segments. Extensions: {}. Nero copies the file and never moves or changes it.",
            nero_core::AUDIO_IMPORT_EXTENSIONS.join(", ")
        )}));
        let mut meeting = id_schema();
        meeting["description"] = json!("Optional empty draft meeting without audio or transcript. Omit to create a new meeting from the file.");
        fields.insert("meetingId".into(), meeting);
        let mut expected = version.clone();
        expected["description"] =
            json!("Current meeting_get editVersion of meetingId. Required with meetingId.");
        fields.insert("expectedVersion".into(), expected);
        required.push("path");
    } else if method == "speech_engine_set" {
        fields.insert(
            "engine".into(),
            engine_schema("Engine id from status speech.choices[].id."),
        );
        fields.insert("acknowledgeMemoryWarning".into(), json!({"type":"boolean","default":false,"description":"true only after the user explicitly acknowledged that choice's current memoryWarning. Required when the chosen engine is supported, not ready and has memoryWarning."}));
        required.push("engine");
    } else if method == "speech_switch_cancel" {
        fields.insert(
            "target".into(),
            engine_schema("status speech.switch.target of the waiting switch to cancel."),
        );
        required.push("target");
    } else if method == "jobs_list" {
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
                "attendeesSha256":{"type":"string","pattern":"^[0-9a-f]{64}$"},
                "summaryId":nullable_id
            },
            "required":["transcriptVersion","transcriptSha256","notesSha256","attendeesSha256","summaryId"],
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
    } else if method == "people_update" {
        fields.insert("expectedVersion".into(), version.clone());
        fields.insert("people".into(), json!({"type":"array","items":{"type":"object","properties":{"id":id_schema(),"name":{"type":"string","minLength":1,"maxLength":120}},"required":["id","name"],"additionalProperties":false}}));
        fields.insert("groups".into(), json!({"type":"array","items":{"type":"object","properties":{"id":id_schema(),"name":{"type":"string","minLength":1,"maxLength":120},"personIds":{"type":"array","items":id_schema(),"uniqueItems":true}},"required":["id","name","personIds"],"additionalProperties":false}}));
        required.extend(["expectedVersion", "people", "groups"]);
    } else if matches!(method, "meeting_attendees_set" | "meeting_attendee_promote") {
        fields.insert("meetingId".into(), id_schema());
        fields.insert("expectedVersion".into(), version.clone());
        required.extend(["meetingId", "expectedVersion"]);
        if method == "meeting_attendees_set" {
            fields.insert("attendees".into(), json!({"type":"array","items":{"type":"object","properties":{"id":id_schema(),"personId":id_schema(),"name":{"type":"string","minLength":1,"maxLength":120}},"required":["id"],"additionalProperties":false},"description":"Stable attendee IDs and optional managed personId/name snapshot. Omit both personId and name for an unnamed temporary attendee. Attendance order never identifies transcript speakers."}));
            required.push("attendees");
        } else {
            fields.insert("attendeeId".into(), id_schema());
            fields.insert("expectedCatalogVersion".into(), version.clone());
            fields.insert(
                "name".into(),
                json!({"type":"string","minLength":1,"maxLength":120}),
            );
            required.extend(["attendeeId", "expectedCatalogVersion", "name"]);
        }
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
    if method == "meeting_import" {
        schema["dependentRequired"] =
            json!({"meetingId":["expectedVersion"],"expectedVersion":["meetingId"]});
    }
    schema.as_object().unwrap().clone()
}
