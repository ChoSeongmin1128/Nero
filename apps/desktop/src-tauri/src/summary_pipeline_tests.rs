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
                        item["fact"]["text"].clone(),
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
