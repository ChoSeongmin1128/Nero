use nero_core::templates::TemplateStore;
use serde_json::{json, Value};
use std::path::Path;

pub const MEETING_SUMMARY: &str = "meeting-summary";
const MEETING_SUMMARY_NAME: &str = "회의 요약 및 저장";
const MEETING_SUMMARY_REQUEST: &str = "첨부한 회의를 요약해 주세요.";
pub const WRITING_RULES: &str = include_str!("../../../agent/meeting-summary/writing-rules.md");

pub fn catalog(root: &Path) -> Result<Value, String> {
    let templates = TemplateStore::new(root.to_path_buf())
        .list()
        .map_err(|error| error.to_string())?;
    let default = templates
        .iter()
        .find(|item| item.id == "meeting")
        .or(templates.first());
    Ok(json!({
        "skills": [{"id": MEETING_SUMMARY, "name": MEETING_SUMMARY_NAME, "description": "첨부한 회의를 요약해 저장합니다.", "aliases": ["회의요약", "meeting-summary"], "requiresMeeting": true}],
        "defaultTemplateId": default.map(|item| &item.id),
        "templates": templates.iter().map(|item| json!({"id":item.id,"name":item.name,"description":item.description,"revision":item.revision,"isDefault":item.built_in})).collect::<Vec<_>>()
    }))
}

// The question keeps only what the user typed. A skill sent without typed text asks
// the model with the skill's own request.
pub fn model_question<'a>(
    skill: Option<&str>,
    typed: &'a str,
) -> Option<std::borrow::Cow<'a, str>> {
    if !typed.trim().is_empty() {
        return Some(std::borrow::Cow::Borrowed(typed));
    }
    (skill == Some(MEETING_SUMMARY)).then_some(std::borrow::Cow::Borrowed(MEETING_SUMMARY_REQUEST))
}
pub fn name(skill: Option<&str>) -> Option<&'static str> {
    (skill == Some(MEETING_SUMMARY)).then_some(MEETING_SUMMARY_NAME)
}

pub fn validate(
    skill: Option<&str>,
    target: Option<&str>,
    meetings: &[String],
    template: Option<&str>,
) -> Result<(), String> {
    match (skill, target) {
        (None, None) => Ok(()),
        (Some(MEETING_SUMMARY), Some(target))
            if meetings.len() == 1
                && meetings[0] == target
                && template.is_some_and(|id| !id.trim().is_empty()) =>
        {
            Ok(())
        }
        (Some(MEETING_SUMMARY), _) if template.is_none_or(|id| id.trim().is_empty()) => {
            Err("요약 템플릿을 선택해 주세요.".into())
        }
        (Some(MEETING_SUMMARY), _) => Err("요약할 회의 한 개를 첨부해 주세요.".into()),
        (Some(_), _) => Err("사용할 작업을 다시 선택해 주세요.".into()),
        (None, Some(_)) => Err("요약 작업을 선택해 주세요.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_without_typed_text_asks_with_the_skill_request() {
        assert_eq!(model_question(None, "  ").as_deref(), None);
        assert_eq!(
            model_question(Some(MEETING_SUMMARY), " ").as_deref(),
            Some(MEETING_SUMMARY_REQUEST)
        );
        assert_eq!(
            model_question(Some(MEETING_SUMMARY), "결정만").as_deref(),
            Some("결정만")
        );
        assert_eq!(name(Some(MEETING_SUMMARY)), Some(MEETING_SUMMARY_NAME));
        assert_eq!(name(None), None);
    }

    #[test]
    fn summary_requires_one_explicit_matching_meeting() {
        let meetings = vec!["a".into()];
        assert!(validate(Some(MEETING_SUMMARY), Some("a"), &meetings, None).is_err());
        assert!(validate(None, None, &meetings, Some("meeting")).is_ok());
        assert!(validate(Some(MEETING_SUMMARY), Some("a"), &meetings, Some("meeting")).is_ok());
        assert!(validate(Some(MEETING_SUMMARY), None, &meetings, Some("meeting")).is_err());
        assert!(validate(Some(MEETING_SUMMARY), Some("b"), &meetings, Some("meeting")).is_err());
        assert!(validate(
            Some(MEETING_SUMMARY),
            Some("a"),
            &["a".into(), "b".into()],
            Some("meeting")
        )
        .is_err());
        assert!(validate(Some("unknown"), Some("a"), &meetings, Some("meeting")).is_err());
        assert!(validate(None, Some("a"), &meetings, Some("meeting")).is_err());
    }

    #[test]
    fn catalog_does_not_initialize_a_library_or_load_prompt_bodies() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap().join("library");
        let result = catalog(&root).unwrap();
        assert!(!root.exists());
        assert_eq!(result["skills"][0]["id"], MEETING_SUMMARY);
        assert!(result["templates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item.get("prompt").is_none()));
    }
}
