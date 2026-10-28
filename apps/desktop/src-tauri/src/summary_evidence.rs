use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const KINDS: [&str; 8] = [
    "context",
    "report",
    "proposal",
    "decision",
    "action",
    "condition",
    "reversal",
    "question",
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Fact {
    pub kind: String,
    pub text: String,
    pub sources: Vec<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Topic {
    pub title: String,
    pub facts: Vec<Fact>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Evidence {
    pub topics: Vec<Topic>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Agenda {
    pub title: String,
    pub members: Vec<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct AgendaPlan {
    pub agendas: Vec<Agenda>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(from = "ReconciledFactWire", into = "ReconciledFactWire")]
pub(super) struct ReconciledFact {
    pub fact: Fact,
    pub inputs: Vec<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ReconciledFactWire {
    kind: String,
    text: String,
    sources: Vec<usize>,
    inputs: Vec<usize>,
}

impl From<ReconciledFactWire> for ReconciledFact {
    fn from(value: ReconciledFactWire) -> Self {
        Self {
            fact: Fact {
                kind: value.kind,
                text: value.text,
                sources: value.sources,
            },
            inputs: value.inputs,
        }
    }
}

impl From<ReconciledFact> for ReconciledFactWire {
    fn from(value: ReconciledFact) -> Self {
        Self {
            kind: value.fact.kind,
            text: value.fact.text,
            sources: value.fact.sources,
            inputs: value.inputs,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Reconciliation {
    pub items: Vec<ReconciledFact>,
}

fn indices_schema() -> Value {
    json!({
        "type": "array",
        "items": {"type": "integer", "minimum": 1},
        "minItems": 1,
        "uniqueItems": true
    })
}

fn fact_properties() -> Value {
    json!({
        "kind": {"type": "string", "enum": KINDS},
        "text": {"type": "string", "minLength": 1},
        "sources": indices_schema()
    })
}

fn fact_schema() -> Value {
    json!({
        "type": "object",
        "properties": fact_properties(),
        "required": ["kind", "text", "sources"],
        "additionalProperties": false
    })
}

pub(super) fn evidence_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "topics": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "minLength": 1},
                        "facts": {"type": "array", "minItems": 1, "items": fact_schema()}
                    },
                    "required": ["title", "facts"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["topics"],
        "additionalProperties": false
    })
}

pub(super) fn agenda_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "agendas": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "minLength": 1},
                        "members": indices_schema()
                    },
                    "required": ["title", "members"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["agendas"],
        "additionalProperties": false
    })
}

#[cfg(test)]
pub(super) fn reconciliation_schema() -> Value {
    let mut properties = fact_properties();
    properties["inputs"] = indices_schema();
    json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "properties": properties,
                    "required": ["kind", "text", "sources", "inputs"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["items"],
        "additionalProperties": false
    })
}

fn validate_title(title: &str) -> Result<(), String> {
    if title.trim().is_empty()
        || title
            .chars()
            .any(|c| c.is_control() || matches!(c, '[' | ']' | '\u{2028}' | '\u{2029}'))
    {
        return Err("요약 안건 제목이 올바르지 않습니다.".into());
    }
    Ok(())
}

fn validate_fact(fact: &Fact) -> Result<(), String> {
    if !KINDS.contains(&fact.kind.as_str()) {
        return Err("요약 항목의 종류가 올바르지 않습니다.".into());
    }
    if fact.text.trim().is_empty() || fact.text.contains('\0') {
        return Err("요약 항목의 내용이 올바르지 않습니다.".into());
    }
    if fact.sources.is_empty()
        || fact.sources.iter().any(|source| *source == 0)
        || fact.sources.iter().collect::<BTreeSet<_>>().len() != fact.sources.len()
    {
        return Err("요약 항목의 원문 번호가 올바르지 않습니다.".into());
    }
    Ok(())
}

pub(super) fn validate_evidence(
    evidence: &Evidence,
    first: usize,
    last: usize,
) -> Result<(), String> {
    if first == 0 || first > last || evidence.topics.is_empty() {
        return Err("요약할 원문 범위 또는 추출 결과가 올바르지 않습니다.".into());
    }
    for topic in &evidence.topics {
        validate_title(&topic.title)?;
        if topic.facts.is_empty() {
            return Err("요약 안건에 확인할 내용이 없습니다.".into());
        }
        for fact in &topic.facts {
            validate_fact(fact)?;
            if fact
                .sources
                .iter()
                .any(|source| *source < first || *source > last)
            {
                return Err("요약 항목의 원문 번호가 입력 범위를 벗어났습니다.".into());
            }
        }
    }
    Ok(())
}

pub(super) fn validate_plan(plan: &AgendaPlan, count: usize) -> Result<(), String> {
    if count == 0 || plan.agendas.is_empty() {
        return Err("요약할 항목 또는 안건이 없습니다.".into());
    }
    let mut seen = BTreeSet::new();
    for agenda in &plan.agendas {
        validate_title(&agenda.title)?;
        if agenda.members.is_empty() {
            return Err("요약 안건에 포함된 항목이 없습니다.".into());
        }
        for member in &agenda.members {
            if *member == 0 || *member > count || !seen.insert(*member) {
                return Err("요약 안건의 항목 번호가 중복되거나 범위를 벗어났습니다.".into());
            }
        }
    }
    if seen.len() != count {
        return Err("요약 안건에서 일부 항목이 누락되었습니다.".into());
    }
    Ok(())
}

pub(super) fn validate_reconciliation(
    result: &Reconciliation,
    inputs: &[Fact],
) -> Result<(), String> {
    if inputs.is_empty() || result.items.is_empty() {
        return Err("통합할 요약 항목 또는 통합 결과가 없습니다.".into());
    }
    let mut input_sources = BTreeSet::new();
    for fact in inputs {
        validate_fact(fact)?;
        input_sources.extend(fact.sources.iter().copied());
    }
    let mut accounted = BTreeSet::new();
    let mut output_sources = BTreeSet::new();
    for item in &result.items {
        validate_fact(&item.fact)?;
        if item.inputs.is_empty() {
            return Err("통합 항목의 입력 근거가 없습니다.".into());
        }
        let mut allowed_sources = BTreeSet::new();
        for index in &item.inputs {
            if *index == 0 || *index > inputs.len() || !accounted.insert(*index) {
                return Err("통합 항목의 입력 번호가 중복되거나 범위를 벗어났습니다.".into());
            }
            allowed_sources.extend(inputs[*index - 1].sources.iter().copied());
        }
        if item
            .fact
            .sources
            .iter()
            .any(|source| !allowed_sources.contains(source))
        {
            return Err("통합 항목에 입력 근거에 없는 원문 번호가 포함되었습니다.".into());
        }
        output_sources.extend(item.fact.sources.iter().copied());
    }
    if accounted.len() != inputs.len() {
        return Err("통합 결과에서 일부 입력 항목이 누락되었습니다.".into());
    }
    if output_sources != input_sources {
        return Err("통합 결과에서 일부 원문 근거가 누락되었습니다.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(text: &str, sources: &[usize]) -> Fact {
        Fact {
            kind: "report".into(),
            text: text.into(),
            sources: sources.to_vec(),
        }
    }

    fn evidence(facts: Vec<Fact>) -> Evidence {
        Evidence {
            topics: vec![Topic {
                title: "진행 상황".into(),
                facts,
            }],
        }
    }

    fn reconciled(fact: Fact, inputs: &[usize]) -> ReconciledFact {
        ReconciledFact {
            fact,
            inputs: inputs.to_vec(),
        }
    }

    #[test]
    fn evidence_keeps_original_source_numbers_and_rejects_invented_or_duplicate_sources() {
        assert!(validate_evidence(&evidence(vec![fact("진행 보고", &[8, 10])]), 8, 12).is_ok());
        for sources in [vec![], vec![0], vec![7], vec![13], vec![8, 8]] {
            assert!(validate_evidence(&evidence(vec![fact("보고", &sources)]), 8, 12).is_err());
        }
        assert!(validate_evidence(&evidence(vec![fact("보고", &[8])]), 0, 12).is_err());
        assert!(validate_evidence(&evidence(vec![fact("보고", &[8])]), 12, 8).is_err());
    }

    #[test]
    fn evidence_rejects_empty_content_topics_and_unknown_kinds() {
        assert!(validate_evidence(&Evidence { topics: vec![] }, 1, 2).is_err());
        assert!(validate_evidence(&evidence(vec![]), 1, 2).is_err());
        for text in ["", " \n\t", "내용\0"] {
            assert!(validate_evidence(&evidence(vec![fact(text, &[1])]), 1, 2).is_err());
        }
        for kind in KINDS {
            let mut value = fact("확인 내용", &[1]);
            value.kind = kind.into();
            assert!(validate_evidence(&evidence(vec![value]), 1, 2).is_ok());
        }
        let mut value = fact("확인 내용", &[1]);
        value.kind = "assumption".into();
        assert!(validate_evidence(&evidence(vec![value]), 1, 2).is_err());
    }

    #[test]
    fn titles_cannot_add_lines_or_source_citations() {
        for title in [
            "",
            " ",
            "안건\n제목",
            "안건\r제목",
            "안건\0",
            "안건 [99]",
            "안건\u{2028}제목",
        ] {
            let mut value = evidence(vec![fact("내용", &[1])]);
            value.topics[0].title = title.into();
            assert!(validate_evidence(&value, 1, 2).is_err());
        }
    }

    #[test]
    fn agenda_accounts_for_every_fact_exactly_once() {
        let plan = AgendaPlan {
            agendas: vec![
                Agenda {
                    title: "첫 안건".into(),
                    members: vec![3, 1],
                },
                Agenda {
                    title: "다음 안건".into(),
                    members: vec![2, 4],
                },
            ],
        };
        assert!(validate_plan(&plan, 4).is_ok());
        for members in [
            vec![],
            vec![0, 1, 2, 3],
            vec![1, 2, 3, 5],
            vec![1, 2, 3],
            vec![1, 2, 3, 3],
        ] {
            let invalid = AgendaPlan {
                agendas: vec![Agenda {
                    title: "안건".into(),
                    members,
                }],
            };
            assert!(validate_plan(&invalid, 4).is_err());
        }
        let mut duplicate = plan;
        duplicate.agendas[1].members.push(1);
        assert!(validate_plan(&duplicate, 4).is_err());
        assert!(validate_plan(&AgendaPlan { agendas: vec![] }, 0).is_err());
    }

    #[test]
    fn reconciliation_can_merge_facts_without_losing_original_sources() {
        let inputs = vec![
            fact("먼저 제안함", &[1, 2]),
            fact("나중에 조건을 변경함", &[8]),
        ];
        let result = Reconciliation {
            items: vec![reconciled(fact("조건이 변경된 제안", &[1, 2, 8]), &[1, 2])],
        };
        assert!(validate_reconciliation(&result, &inputs).is_ok());
        let mut lost = result.clone();
        lost.items[0].fact.sources = vec![1, 8];
        assert!(validate_reconciliation(&lost, &inputs).is_err());
        let mut invented = result;
        invented.items[0].fact.sources.push(9);
        assert!(validate_reconciliation(&invented, &inputs).is_err());
    }

    #[test]
    fn reconciliation_sources_must_belong_to_the_claimed_input_facts() {
        let inputs = vec![fact("첫 안건", &[2]), fact("별도 안건", &[9])];
        let swapped = Reconciliation {
            items: vec![
                reconciled(fact("첫 안건", &[9]), &[1]),
                reconciled(fact("별도 안건", &[2]), &[2]),
            ],
        };
        assert!(validate_reconciliation(&swapped, &inputs).is_err());
        for indices in [vec![], vec![0, 2], vec![1, 3], vec![1], vec![1, 2, 2]] {
            let invalid = Reconciliation {
                items: vec![reconciled(fact("통합", &[2, 9]), &indices)],
            };
            assert!(validate_reconciliation(&invalid, &inputs).is_err());
        }
        assert!(validate_reconciliation(&Reconciliation { items: vec![] }, &inputs).is_err());
        assert!(validate_reconciliation(&Reconciliation { items: vec![] }, &[]).is_err());
    }

    #[test]
    fn reconciliation_rejects_duplicate_accounting_across_output_items() {
        let inputs = vec![fact("첫 내용", &[1]), fact("다음 내용", &[2])];
        let invalid = Reconciliation {
            items: vec![
                reconciled(fact("통합", &[1, 2]), &[1, 2]),
                reconciled(fact("첫 내용", &[1]), &[1]),
            ],
        };
        assert!(validate_reconciliation(&invalid, &inputs).is_err());
    }

    #[test]
    fn typed_roundtrip_uses_direct_reconciliation_fields_and_denies_unknown_fields() {
        let value = Reconciliation {
            items: vec![reconciled(fact("진행 상황", &[3]), &[1])],
        };
        let wire = serde_json::to_value(&value).unwrap();
        assert_eq!(
            wire["items"][0],
            json!({
                "kind": "report", "text": "진행 상황", "sources": [3], "inputs": [1]
            })
        );
        assert_eq!(
            serde_json::from_value::<Reconciliation>(wire.clone()).unwrap(),
            value
        );
        let mut unknown = wire;
        unknown["items"][0]["unexpected"] = json!(true);
        assert!(serde_json::from_value::<Reconciliation>(unknown).is_err());
        assert!(serde_json::from_value::<Evidence>(json!({"topics": [], "other": 1})).is_err());
        assert!(serde_json::from_value::<AgendaPlan>(
            json!({"agendas": [{"title": "안건", "members": [1], "extra": 1}]})
        )
        .is_err());
        assert!(serde_json::from_value::<Fact>(
            json!({"kind": "report", "text": "보고", "sources": [1], "extra": 1})
        )
        .is_err());
    }

    #[test]
    fn schemas_require_all_fields_and_strict_objects() {
        let evidence = evidence_schema();
        assert_eq!(evidence["additionalProperties"], false);
        assert_eq!(evidence["properties"]["topics"]["minItems"], 1);
        let fact = &evidence["properties"]["topics"]["items"]["properties"]["facts"]["items"];
        assert_eq!(fact["additionalProperties"], false);
        assert_eq!(fact["properties"]["kind"]["enum"], json!(KINDS));
        assert_eq!(fact["properties"]["sources"]["items"]["minimum"], 1);
        let agenda = agenda_schema();
        assert_eq!(
            agenda["properties"]["agendas"]["items"]["additionalProperties"],
            false
        );
        let reconciliation = reconciliation_schema();
        let item = &reconciliation["properties"]["items"]["items"];
        assert_eq!(item["additionalProperties"], false);
        assert_eq!(
            item["required"],
            json!(["kind", "text", "sources", "inputs"])
        );
    }
}
