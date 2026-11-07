use crate::QueryError;
use nero_core::agent_control::{self, AppUpdateAction};
use serde_json::{json, Map, Value};
use std::path::Path;

pub(crate) fn dispatch(root: &Path, params: Value) -> Result<Value, QueryError> {
    AppUpdateAction::parse(params.clone()).map_err(|_| QueryError::invalid())?;
    if !root.exists() {
        return Err(QueryError {
            code: "app_not_running",
            message: std::borrow::Cow::Borrowed("Nero 앱을 실행한 뒤 다시 시도해 주세요."),
        });
    }
    agent_control::request(root, "app_update", params).map_err(|error| {
        let message = match error.code {
            "app_update_unsupported" => "실행 중인 Nero는 에이전트 업데이트를 지원하지 않습니다. 앱의 업데이트 화면을 사용해 주세요.",
            "app_update_check_failed" => "공개 업데이트를 확인하지 못했습니다. 네트워크 연결과 Nero의 업데이트 화면을 확인해 주세요.",
            "app_update_failed" => "업데이트 요청을 완료하지 못했습니다. update status와 Nero의 업데이트 화면을 확인해 주세요.",
            "app_not_ready" => "Nero의 편집 저장 또는 유휴 상태를 확인하지 못했습니다. 앱에서 저장 상태와 녹음/처리 작업을 확인해 주세요.",
            "mutation_result_unknown" => "업데이트 응답을 확인하지 못했습니다. 같은 요청을 반복하지 말고 update status와 실행 중인 앱 버전을 먼저 확인해 주세요.",
            _ => crate::transport_message(error.code),
        };
        QueryError { code: error.code, message: message.into() }
    })
}

pub(crate) fn schema() -> Map<String, Value> {
    json!({"type":"object","properties":{"action":{"type":"string","enum":["status","check","download","install"]}},"required":["action"],"additionalProperties":false}).as_object().unwrap().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_app_and_invalid_update_input_never_initialize_a_library() {
        let directory = tempfile::tempdir().unwrap();
        let absent = directory.path().join("missing");
        for action in ["status", "check", "download", "install"] {
            assert_eq!(
                dispatch(&absent, json!({"action":action}))
                    .unwrap_err()
                    .code,
                "app_not_running"
            );
            assert!(!absent.exists());
        }
        for params in [
            json!({}),
            json!({"action":"install","path":"untrusted"}),
            json!({"action":"purge"}),
        ] {
            assert_eq!(dispatch(&absent, params).unwrap_err().code, "invalid_input");
            assert!(!absent.exists());
        }
    }
}
