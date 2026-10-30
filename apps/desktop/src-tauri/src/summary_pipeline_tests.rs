#![cfg(unix)]

use super::tests::{numbered_source, scripted_server, ScriptedServer};
use super::*;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn source() -> Transcript {
    let mut source = numbered_source(48);
    for (index, segment) in source.segments.iter_mut().enumerate() {
        segment.start = index as f64 * 225.0;
        segment.end = (index + 1) as f64 * 225.0;
    }
    source
}

fn prepare(path: &Path) -> (Arc<Processing>, PathBuf) {
    let root = path.join("data");
    fs::create_dir(&root).unwrap();
    let process = Processing::new(root, path.join("resources"));
    let binary = executable(&process, false);
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    fs::write(binary, b"scripted runtime").unwrap();
    let cache = path.join("checkpoints");
    fs::create_dir(&cache).unwrap();
    (process, cache)
}

fn data(user: &str) -> Option<Value> {
    user.lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line).ok())
}

fn numbers(lines: &Value) -> Vec<usize> {
    lines
        .as_array()
        .unwrap()
        .iter()
        .map(|line| {
            line.as_str()
                .unwrap()
                .strip_prefix('[')
                .unwrap()
                .split_once(']')
                .unwrap()
                .0
                .parse()
                .unwrap()
        })
        .collect()
}

fn request_data(request: &Value) -> Value {
    data(request["messages"][1]["content"].as_str().unwrap()).unwrap()
}

fn tokenize(user: &str) -> usize {
    let count = if let Some(value) = data(user).filter(|value| value.get("transcript").is_some()) {
        value["transcript"].as_array().unwrap().len()
    } else if user.contains("<transcript>") {
        user.lines()
            .filter(|line| {
                line.starts_with('[')
                    && line
                        .split_once(']')
                        .is_some_and(|(number, _)| number[1..].parse::<usize>().is_ok())
            })
            .count()
    } else {
        0
    };
    if count > 4 {
        7000
    } else {
        1000
    }
}

fn completed(content: Value) -> Value {
    json!({
        "choices": [{"finish_reason": "stop", "message": {"content": content.to_string()}}],
        "usage": {"prompt_tokens": 1000, "completion_tokens": 100}
    })
}

fn respond(request: &Value) -> Value {
    let value = request_data(request);
    let content = if value.get("transcript").is_some() {
        let facts: Vec<_> = numbers(&value["transcript"])
            .into_iter()
            .map(|number| {
                json!({"kind":"report", "text":format!("합성 사실 {number}"), "sources":[number]})
            })
            .collect();
        json!({"topics":[{"title":"공통 안건", "facts":facts}]})
    } else if let Some(topics) = value.get("topics").and_then(Value::as_array) {
        json!({"agendas":[{"title":"합성 안건", "members": topics.iter().map(|topic| topic["id"].clone()).collect::<Vec<_>>()}]})
    } else if value.get("title").is_some() {
        Value::Object(
            value["facts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| {
                    (
                        item["id"].as_u64().unwrap().to_string(),
                        if item.get("fact").is_some() {
                            item["fact"]["text"].clone()
                        } else {
                            json!({"heading":"핵심 발견","order":1,"text":item["text"]})
                        },
                    )
                })
                .collect(),
        )
    } else if value["facts"]
        .as_array()
        .unwrap()
        .first()
        .is_some_and(|fact| fact.get("id").is_some())
    {
        json!({"items":value["facts"].as_array().unwrap().iter().map(|item| {
            let mut fact = item["fact"].clone();
            fact["inputs"] = json!([item["id"].clone()]);
            fact
        }).collect::<Vec<_>>()})
    } else {
        panic!("unknown synthetic pipeline request")
    };
    completed(content)
}

fn requests(fixture: &ScriptedServer) -> Vec<Value> {
    fixture.requests.lock().unwrap().clone()
}

fn evidence_requests(fixture: &ScriptedServer) -> Vec<Vec<usize>> {
    requests(fixture)
        .iter()
        .filter_map(|request| request_data(request).get("transcript").map(numbers))
        .collect()
}

#[test]
fn three_hour_fixture_covers_every_source_range_and_assembles_cited_sections() {
    let temporary = tempfile::tempdir().unwrap();
    let fixture = scripted_server(temporary.path(), tokenize, respond);
    let (process, cache) = prepare(temporary.path());
    let source = source();
    let before = serde_json::to_vec(&source).unwrap();
    let (text, receipt) = runtime()
        .block_on(pipeline::run(
            &fixture.server,
            &source,
            "",
            &cache,
            &process,
            &AtomicBool::new(false),
        ))
        .unwrap();
    assert_eq!(source.segments.last().unwrap().end, 10800.0);
    assert_eq!(serde_json::to_vec(&source).unwrap(), before);
    assert_eq!(receipt["strategy"], "evidence-agenda-sections");
    let covered: Vec<_> = receipt["ranges"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|range| range[0].as_u64().unwrap()..=range[1].as_u64().unwrap())
        .collect();
    assert_eq!(covered, (1..=48).collect::<Vec<_>>());
    let delivered: Vec<_> = evidence_requests(&fixture).into_iter().flatten().collect();
    assert_eq!(delivered, (1..=48).collect::<Vec<_>>());
    for number in 1..=48 {
        assert!(text.contains(&format!("합성 사실 {number} [{number}]")));
    }
    assert!(validate_references(&text, 48).is_ok());
    assert!(requests(&fixture)
        .iter()
        .all(|request| request["response_format"]["type"] == "json_schema"));
    assert!(receipt["events"]
        .as_array()
        .unwrap()
        .iter()
        .all(|event| event["detail"]["finishReason"] == "stop"));
}

#[test]
fn failed_length_range_is_regenerated_while_completed_siblings_are_reused() {
    let temporary = tempfile::tempdir().unwrap();
    let failing = Arc::new(AtomicBool::new(true));
    let flag = failing.clone();
    let fixture = scripted_server(temporary.path(), tokenize, move |request| {
        let value = request_data(request);
        if value
            .get("transcript")
            .is_some_and(|lines| numbers(lines).contains(&25))
            && flag.load(Ordering::Acquire)
        {
            json!({"choices":[{"finish_reason":"length","message":{"content":"{\"topics\":"}}],"usage":{"completion_tokens":2048}})
        } else {
            respond(request)
        }
    });
    let (process, cache) = prepare(temporary.path());
    let source = source();
    let cancel = AtomicBool::new(false);
    let error = runtime()
        .block_on(pipeline::run(
            &fixture.server,
            &source,
            "",
            &cache,
            &process,
            &cancel,
        ))
        .unwrap_err();
    assert_eq!(error.kind, FailureKind::OutputLimit);
    assert_eq!(
        error.detail.unwrap().finish_reason.as_deref(),
        Some("length")
    );
    failing.store(false, Ordering::Release);
    let (text, receipt) = runtime()
        .block_on(pipeline::run(
            &fixture.server,
            &source,
            "",
            &cache,
            &process,
            &cancel,
        ))
        .unwrap();
    assert!(text.contains("합성 사실 48 [48]"));
    let delivered = evidence_requests(&fixture);
    assert_eq!(
        delivered
            .iter()
            .filter(|range| **range == vec![1, 2, 3])
            .count(),
        1
    );
    assert_eq!(
        delivered
            .iter()
            .filter(|range| **range == vec![22, 23, 24])
            .count(),
        1
    );
    assert_eq!(
        delivered
            .iter()
            .filter(|range| **range == vec![25, 26, 27])
            .count(),
        2
    );
    assert_eq!(
        receipt["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["request"]["cached"] == true)
            .count(),
        8
    );
}

#[test]
fn output_limited_agenda_is_partitioned_into_complete_accounted_sections() {
    let temporary = tempfile::tempdir().unwrap();
    let fixture = scripted_server(temporary.path(), tokenize, |request| {
        let value = request_data(request);
        if value.get("title").is_some() && value["facts"].as_array().unwrap().len() > 2 {
            json!({"choices":[{"finish_reason":"length","message":{"content":"{\"discussion\":"}}],"usage":{"completion_tokens":2048}})
        } else {
            respond(request)
        }
    });
    let (process, cache) = prepare(temporary.path());
    let source = source();
    let (text, receipt) = runtime()
        .block_on(pipeline::run(
            &fixture.server,
            &source,
            "",
            &cache,
            &process,
            &AtomicBool::new(false),
        ))
        .unwrap();
    for number in 1..=48 {
        assert!(text.contains(&format!("합성 사실 {number} [{number}]")));
    }
    let successful_writer_events: Vec<_> = receipt["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["request"]["stage"] == "write-agenda")
        .collect();
    assert!(successful_writer_events.len() > 1);
    assert!(successful_writer_events
        .iter()
        .all(|event| event["detail"]["finishReason"] == "stop"));
    assert_eq!(
        evidence_requests(&fixture)
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
        (1..=48).collect::<Vec<_>>()
    );
}

#[test]
fn changed_notes_invalidate_completed_stage_checkpoints() {
    let temporary = tempfile::tempdir().unwrap();
    let fixture = scripted_server(temporary.path(), tokenize, respond);
    let (process, cache) = prepare(temporary.path());
    let source = source();
    let cancel = AtomicBool::new(false);
    runtime()
        .block_on(pipeline::run(
            &fixture.server,
            &source,
            "첫 참고 메모",
            &cache,
            &process,
            &cancel,
        ))
        .unwrap();
    let original_requests = requests(&fixture).len();
    let (_, receipt) = runtime()
        .block_on(pipeline::run(
            &fixture.server,
            &source,
            "수정된 참고 메모",
            &cache,
            &process,
            &cancel,
        ))
        .unwrap();
    assert_eq!(requests(&fixture).len(), original_requests * 2);
    assert!(receipt["events"]
        .as_array()
        .unwrap()
        .iter()
        .all(|event| event["request"]["cached"] == false));
    assert!(requests(&fixture)[original_requests..]
        .iter()
        .all(|request| {
            request["messages"][1]["content"]
                .as_str()
                .unwrap()
                .contains("수정된 참고 메모")
        }));
}

#[test]
fn source_numbers_outside_an_extracted_range_never_become_checkpoints() {
    let temporary = tempfile::tempdir().unwrap();
    let fixture = scripted_server(temporary.path(), tokenize, |request| {
        let value = request_data(request);
        let source = numbers(&value["transcript"]).last().unwrap() + 1;
        completed(json!({"topics":[{"title":"안건","facts":[{
            "kind":"report","text":"범위 밖 근거","sources":[source]
        }]}]}))
    });
    let (process, cache) = prepare(temporary.path());
    let error = runtime()
        .block_on(pipeline::run(
            &fixture.server,
            &source(),
            "",
            &cache,
            &process,
            &AtomicBool::new(false),
        ))
        .unwrap_err();
    assert_eq!(error.kind, FailureKind::Output);
    let directories: Vec<_> = fs::read_dir(&cache)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(directories.len(), 1);
    assert!(fs::read_dir(&directories[0]).unwrap().next().is_none());
    assert!(requests(&fixture)
        .iter()
        .all(|request| { request_data(request).get("transcript").is_some() }));
}

#[test]
fn preaccepted_cancellation_starts_no_generation_or_checkpoint() {
    let temporary = tempfile::tempdir().unwrap();
    let fixture = scripted_server(temporary.path(), tokenize, |_| {
        panic!("cancelled input must not reach generation")
    });
    let (process, cache) = prepare(temporary.path());
    let error = runtime()
        .block_on(pipeline::run(
            &fixture.server,
            &source(),
            "",
            &cache,
            &process,
            &AtomicBool::new(true),
        ))
        .unwrap_err();
    assert_eq!(error.kind, FailureKind::Cancelled);
    assert!(requests(&fixture).is_empty());
    assert!(fs::read_dir(cache).unwrap().next().is_none());
}

#[test]
fn three_hour_template_pipeline_preserves_all_facts_and_invalidates_changed_prompt() {
    let temporary = tempfile::tempdir().unwrap();
    let fixture = scripted_server(temporary.path(), tokenize, respond);
    let (process, cache) = prepare(temporary.path());
    let source = source();
    let template = nero_core::templates::TemplateStore::new(process.root.canonicalize().unwrap())
        .get("interview")
        .unwrap();
    let (text, receipt) = runtime()
        .block_on(pipeline::run_with_template(
            &fixture.server,
            &source,
            "",
            &[],
            &cache,
            &process,
            &AtomicBool::new(false),
            Some(&template),
        ))
        .unwrap();
    assert_eq!(source.segments.last().unwrap().end, 10800.0);
    assert!(text.starts_with("## 핵심 발견"));
    for number in 1..=48 {
        assert!(text.contains(&format!("합성 사실 {number} [{number}]")));
    }
    assert_eq!(receipt["template"]["revision"], template.revision);
    let mut changed = template.clone();
    changed.prompt.push_str("\n후속 확인을 강조한다.");
    changed.revision = "changed-revision".into();
    let (_, new_receipt) = runtime()
        .block_on(pipeline::run_with_template(
            &fixture.server,
            &source,
            "",
            &[],
            &cache,
            &process,
            &AtomicBool::new(false),
            Some(&changed),
        ))
        .unwrap();
    assert_eq!(new_receipt["template"]["revision"], "changed-revision");
    assert!(new_receipt["events"]
        .as_array()
        .unwrap()
        .iter()
        .all(|event| event["request"]["cached"] == false));
}

fn roster() -> Vec<nero_core::MeetingAttendee> {
    vec![nero_core::MeetingAttendee {
        id: "11111111-1111-4111-8111-111111111111".into(),
        person_id: None,
        name: Some("Reference name".into()),
    }]
}

#[test]
fn attendance_references_reach_all_stages_and_invalidate_cache_without_changing_notes() {
    let temporary = tempfile::tempdir().unwrap();
    let fixture = scripted_server(temporary.path(), tokenize, |request| {
        let value = request_data(request);
        if value.get("change").is_some() {
            return completed(json!({"previous":[]}));
        }
        let mut result = respond(request);
        if value.get("transcript").is_some() {
            let mut content: Value =
                serde_json::from_str(result["choices"][0]["message"]["content"].as_str().unwrap())
                    .unwrap();
            for fact in content["topics"][0]["facts"].as_array_mut().unwrap() {
                if fact["sources"][0] == 48 {
                    fact["kind"] = json!("condition");
                }
            }
            result["choices"][0]["message"]["content"] = json!(content.to_string());
        }
        result
    });
    let (process, cache) = prepare(temporary.path());
    let source = source();
    let original = serde_json::to_vec(&source).unwrap();
    let notes = "Original notes only";
    let mut attendees = roster();
    let cancel = AtomicBool::new(false);
    let (_, receipt) = runtime()
        .block_on(pipeline::run_with_template(
            &fixture.server,
            &source,
            notes,
            &attendees,
            &cache,
            &process,
            &cancel,
            None,
        ))
        .unwrap();
    for stage in ["evidence", "link-changes", "agenda", "write-agenda"] {
        assert!(receipt["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["request"]["stage"] == stage));
    }
    let reference = serde_json::to_string(&AttendanceReference {
        attendees: &attendees,
    })
    .unwrap();
    for request in requests(&fixture) {
        let user = request["messages"][1]["content"].as_str().unwrap();
        assert_eq!(user.matches("<attendance_reference>").count(), 1);
        assert!(user.contains(&reference));
        assert!(user.contains(&json!({"meetingNotes":notes}).to_string()));
        assert!(user
            .contains("명단에만 있는 이름으로 익명 발언이나 1인칭 약속의 담당자를 채우지 마세요"));
        assert!(
            !user.contains("별도의 참석자 명단/회의 일시/장소 메타데이터는 제공되지 않았습니다")
        );
    }
    assert_eq!(
        receipt["notesSha256"],
        format!("{:x}", Sha256::digest(notes.as_bytes()))
    );
    assert_eq!(
        receipt["attendeesSha256"],
        nero_core::people::attendees_digest(&attendees).unwrap()
    );
    let first_requests = requests(&fixture).len();
    attendees[0].name = Some("Changed reference".into());
    let (_, changed) = runtime()
        .block_on(pipeline::run_with_template(
            &fixture.server,
            &source,
            notes,
            &attendees,
            &cache,
            &process,
            &cancel,
            None,
        ))
        .unwrap();
    assert_eq!(changed["notesSha256"], receipt["notesSha256"]);
    assert_ne!(changed["attendeesSha256"], receipt["attendeesSha256"]);
    assert!(changed["events"]
        .as_array()
        .unwrap()
        .iter()
        .all(|event| event["request"]["cached"] == false));
    assert!(requests(&fixture)[first_requests..]
        .iter()
        .all(|request| request["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("Changed reference")));
    assert_eq!(serde_json::to_vec(&source).unwrap(), original);
    assert_eq!(fs::read_dir(cache).unwrap().count(), 2);
}

#[test]
fn whole_summary_uses_separate_attendance_data_and_keeps_unverified_speakers_unassigned() {
    let temporary = tempfile::tempdir().unwrap();
    let fixture = scripted_server(
        temporary.path(),
        |_| 1000,
        |request| {
            let user = request["messages"][1]["content"].as_str().unwrap();
            assert!(user.contains("<attendance_reference>"));
            assert!(user.contains("화자 미지정"));
            assert!(
                user.contains("명단 순서/인원수/임시 참석자 번호를 화자 번호에 연결하지 마세요")
            );
            assert!(user.contains("숫자/금액/단위/조건/정정과 미정 상태를 보존하세요"));
            json!({"choices":[{"finish_reason":"stop","message":{"content":"담당 미정 [1]"}}],"usage":{"prompt_tokens":1000,"completion_tokens":100}})
        },
    );
    let (process, cache) = prepare(temporary.path());
    let source = numbered_source(2);
    let attendees = roster();
    let (text, receipt) = runtime()
        .block_on(pipeline::run_with_template(
            &fixture.server,
            &source,
            "Original notes",
            &attendees,
            &cache,
            &process,
            &AtomicBool::new(false),
            None,
        ))
        .unwrap();
    assert_eq!(text, "담당 미정 [1]");
    assert_eq!(receipt["strategy"], "whole");
    assert_eq!(
        receipt["attendeesSha256"],
        nero_core::people::attendees_digest(&attendees).unwrap()
    );
    assert!(!source
        .speakers
        .iter()
        .any(|speaker| speaker.name == "Reference name"));
}

#[test]
fn oversized_attendance_fails_before_generation_without_dropping_references() {
    let temporary = tempfile::tempdir().unwrap();
    let fixture = scripted_server(
        temporary.path(),
        |user| {
            if user.contains("<attendance_reference>") {
                CONTEXT
            } else {
                100
            }
        },
        |_| panic!("oversized reference must not reach generation"),
    );
    let (process, cache) = prepare(temporary.path());
    let error = runtime()
        .block_on(pipeline::run_with_template(
            &fixture.server,
            &numbered_source(2),
            "",
            &roster(),
            &cache,
            &process,
            &AtomicBool::new(false),
            None,
        ))
        .unwrap_err();
    assert_eq!(error.kind, FailureKind::InputLimit);
    assert!(requests(&fixture).is_empty());
    assert_eq!(fs::read_dir(cache).unwrap().count(), 0);
}
