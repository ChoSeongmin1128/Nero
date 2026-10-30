use super::*;
use std::env;

fn required_path(name: &str) -> PathBuf {
    let value = env::var_os(name).unwrap_or_else(|| panic!("{name} must be explicitly provided"));
    let path = PathBuf::from(value);
    assert!(path.is_absolute(), "{name} must be an absolute path");
    assert!(
        !path.components().any(|component| matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )),
        "{name} cannot contain relative traversal"
    );
    path
}

fn write_path(name: &str) -> PathBuf {
    let path = required_path(name);
    assert!(
        path.components().any(|component| {
            matches!(component, std::path::Component::Normal(name) if name == ".work")
        }),
        "{name} must be an isolated directory under .work"
    );
    assert!(
        !path.components().any(|component| {
            matches!(component, std::path::Component::Normal(name) if name == "io.github.choseongmin1128.nero.desktop")
        }),
        "{name} cannot write to the active application library"
    );
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => assert!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "{name} contains a linked or non-directory path"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => panic!("cannot inspect {name}: {error}"),
        }
    }
    path
}

fn read_fixture(path: &Path) -> Vec<u8> {
    let file = plain_file(path).unwrap_or_else(|error| panic!("cannot read fixture: {error}"));
    let mut bytes = Vec::new();
    file.take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .expect("fixture read failed");
    assert!(bytes.len() <= 32 * 1024 * 1024, "fixture is too large");
    bytes
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
#[ignore = "explicit local model evaluation"]
fn local_summary_fixture() {
    let source_path = required_path("NERO_SUMMARY_SOURCE");
    let notes_path = env::var_os("NERO_SUMMARY_NOTES").map(PathBuf::from);
    let weights = required_path("NERO_SUMMARY_WEIGHTS");
    let resources = required_path("NERO_SUMMARY_RESOURCES");
    let output = write_path("NERO_SUMMARY_OUTPUT");
    let cache_base = write_path("NERO_SUMMARY_CACHE");
    assert!(
        fs::symlink_metadata(&output)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "NERO_SUMMARY_OUTPUT must be a new directory; existing evidence is preserved"
    );
    fs::create_dir_all(&output).expect("cannot create isolated output directory");
    fs::create_dir_all(&cache_base).expect("cannot create isolated cache directory");
    let data = directory(&output, "data").expect("cannot create isolated data root");
    let job = directory(&output, "job").expect("cannot create isolated job directory");
    let source_bytes = read_fixture(&source_path);
    let notes_bytes = notes_path.as_deref().map(read_fixture).unwrap_or_default();
    let source: Transcript =
        serde_json::from_slice(&source_bytes).expect("source must be a Transcript JSON document");
    let notes = String::from_utf8(notes_bytes.clone()).expect("notes must be UTF-8");
    assert!(
        !source.segments.is_empty(),
        "source must contain transcript segments"
    );
    let baseline = match env::var("NERO_SUMMARY_BASELINE").as_deref() {
        Err(env::VarError::NotPresent) | Ok("0") => false,
        Ok("1") => true,
        _ => panic!("NERO_SUMMARY_BASELINE must be 0 or 1"),
    };
    let weights_before = fs::symlink_metadata(&weights).expect("existing weights are required");
    assert!(
        weights_before.is_file() && !weights_before.file_type().is_symlink(),
        "weights must be an existing ordinary model file"
    );
    let spec = model();
    let cancel = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let watcher = env::var_os("NERO_SUMMARY_STOP").map(|_| {
        let stop = write_path("NERO_SUMMARY_STOP");
        let cancel = Arc::clone(&cancel);
        let finished = Arc::clone(&finished);
        std::thread::spawn(move || {
            while !finished.load(Ordering::Acquire) {
                if stop.is_file() {
                    cancel.store(true, Ordering::Release);
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        })
    });
    let validation_started = Instant::now();
    assert!(
        matches_file(&weights, &spec.files[0], &cancel),
        "existing read-only model does not match the pinned manifest; no model was downloaded"
    );
    let model_validation_ms = validation_started.elapsed().as_millis();
    let process = Processing::new(data, resources);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("cannot prepare local runtime");
    let pipeline_started = Instant::now();
    let outcome = runtime.block_on(async {
        let result = if baseline {
            recover_summary(&process, &job, &cancel, |gpu, folder| {
                let process = &process;
                let source = &source;
                let notes = &notes;
                let weights = &weights;
                let cancel = &cancel;
                async move {
                    let mut server = launch(process, weights, gpu, &folder, cancel).await?;
                    let result =
                        summarize_legacy_with_notes(&server, source, notes, process, cancel).await;
                    server.stop(process)?;
                    result.map_err(|error| server.diagnose(error))
                }
            })
            .await
        } else {
            run_summary_with_notes(
                &process,
                &source,
                &notes,
                &weights,
                &job,
                &cache_base,
                &cancel,
            )
            .await
        };
        match result {
            Ok((text, receipt)) => {
                validate_references(&text, source.segments.len())
                    .map_err(|error| Failure::new(FailureKind::Output, error))?;
                Ok((text, receipt))
            }
            Err(error) => Err(error),
        }
    });
    finished.store(true, Ordering::Release);
    if let Some(watcher) = watcher {
        watcher
            .join()
            .expect("evaluation cancellation watcher failed");
    }
    let pipeline_elapsed_ms = pipeline_started.elapsed().as_millis();
    let source_after = read_fixture(&source_path);
    let notes_after = notes_path.as_deref().map(read_fixture).unwrap_or_default();
    let weights_after =
        fs::symlink_metadata(&weights).expect("weights disappeared during evaluation");
    let source_unchanged = source_bytes == source_after;
    let notes_unchanged = notes_bytes == notes_after;
    let model_metadata_unchanged = weights_before.len() == weights_after.len()
        && weights_before.modified().ok() == weights_after.modified().ok()
        && weights_after.is_file()
        && !weights_after.file_type().is_symlink();
    let failure = outcome
        .as_ref()
        .err()
        .map(|error| serde_json::to_value(error).expect("failure serialization failed"));
    let mut evaluation = json!({
        "schemaVersion": 1,
        "baseline": baseline,
        "model": spec.repository,
        "modelRevision": spec.revision,
        "modelSha256": spec.files[0].sha256,
        "runtime": "llama.cpp b11734-f3fb2fcdb",
        "sourceVersion": source.version,
        "sourceSha256Before": hash(&source_bytes),
        "sourceSha256After": hash(&source_after),
        "notesSha256Before": hash(&notes_bytes),
        "notesSha256After": hash(&notes_after),
        "sourceUnchanged": source_unchanged,
        "notesUnchanged": notes_unchanged,
        "modelMetadataUnchanged": model_metadata_unchanged,
        "segments": source.segments.len(),
        "sourceEndSeconds": source.segments.iter().map(|segment| segment.end).fold(0.0_f64, f64::max),
        "modelValidationMs": model_validation_ms,
        "pipelineElapsedMs": pipeline_elapsed_ms,
        "status": if outcome.is_ok() { "completed" } else { "failed" },
        "failure": failure,
        "coreCommitPerformed": false,
        "installedAppChanged": false,
        "scope": "isolated product summary pipeline; no recognition, diarization, active-library writes or model copies"
    });
    if let Ok((text, receipt)) = &outcome {
        fs::write(output.join("summary.md"), text).expect("cannot save completed summary");
        fs::write(
            output.join("receipt.json"),
            serde_json::to_vec_pretty(receipt).expect("receipt serialization failed"),
        )
        .expect("cannot save receipt");
        evaluation["summaryBytes"] = json!(text.len());
        evaluation["summarySha256"] = json!(hash(text.as_bytes()));
    }
    fs::write(
        output.join("evaluation.json"),
        serde_json::to_vec_pretty(&evaluation).expect("evaluation serialization failed"),
    )
    .expect("cannot save evaluation");
    assert!(source_unchanged, "source fixture changed during evaluation");
    assert!(notes_unchanged, "notes fixture changed during evaluation");
    assert!(
        model_metadata_unchanged,
        "model metadata changed during read-only evaluation"
    );
    if let Err(error) = outcome {
        panic!(
            "local summary failed: kind={:?}, message={}, detail={:?}; see evaluation.json",
            error.kind, error.message, error.detail
        );
    }
}

#[test]
#[ignore = "explicit local attendance A/B evaluation"]
fn local_summary_attendance_comparison() {
    let source_path = required_path("NERO_SUMMARY_SOURCE");
    let attendees_path = required_path("NERO_SUMMARY_ATTENDEES");
    let fixture_manifest_path = required_path("NERO_SUMMARY_FIXTURE_MANIFEST");
    let notes_path = env::var_os("NERO_SUMMARY_NOTES").map(PathBuf::from);
    let weights = required_path("NERO_SUMMARY_WEIGHTS");
    let resources = required_path("NERO_SUMMARY_RESOURCES");
    let output = write_path("NERO_SUMMARY_OUTPUT");
    let source_bytes = read_fixture(&source_path);
    let attendees_bytes = read_fixture(&attendees_path);
    let manifest_bytes = read_fixture(&fixture_manifest_path);
    let notes_bytes = notes_path.as_deref().map(read_fixture).unwrap_or_default();
    let source: Transcript =
        serde_json::from_slice(&source_bytes).expect("source must be actual Transcript JSON");
    let attendees: Vec<nero_core::MeetingAttendee> =
        serde_json::from_slice(&attendees_bytes).expect("attendees must be a roster array");
    let manifest: Value =
        serde_json::from_slice(&manifest_bytes).expect("scope manifest must be JSON");
    let notes = String::from_utf8(notes_bytes.clone()).expect("notes must be UTF-8");
    assert!(
        !source.segments.is_empty() && !attendees.is_empty(),
        "source and roster must be explicitly provided"
    );
    assert_eq!(
        manifest["fixtureSha256"],
        hash(&source_bytes),
        "source must match the explicitly derived fixture manifest"
    );
    let attendees_sha256 =
        nero_core::people::attendees_digest(&attendees).expect("invalid attendance fixture");
    let source_end = source
        .segments
        .iter()
        .map(|segment| segment.end)
        .fold(0.0_f64, f64::max);
    assert!(source_end.is_finite() && source_end>0.0 && source_end<=240.0,"provide the explicit approximately three-minute source excerpt; the harness never truncates or changes speaker metadata");
    assert!(
        fs::symlink_metadata(&output)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "output must be new; earlier evidence is preserved"
    );
    let weights_before = fs::symlink_metadata(&weights).expect("existing weights required");
    assert!(weights_before.is_file() && !weights_before.file_type().is_symlink());
    let spec = model();
    let cancel = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let watcher = env::var_os("NERO_SUMMARY_STOP").map(|_| {
        let stop = write_path("NERO_SUMMARY_STOP");
        let (cancel, finished) = (cancel.clone(), finished.clone());
        std::thread::spawn(move || {
            while !finished.load(Ordering::Acquire) {
                if stop.is_file() {
                    cancel.store(true, Ordering::Release);
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        })
    });
    assert!(
        matches_file(&weights, &spec.files[0], &cancel),
        "existing model must match the pinned manifest; no download is performed"
    );
    fs::create_dir_all(&output).expect("cannot create isolated output");
    let data = directory(&output, "data").unwrap();
    let job = directory(&output, "job").unwrap();
    let cache = directory(&output, "cache").unwrap();
    let process = Processing::new(data, resources);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let started = Instant::now();
    let result=runtime.block_on(recover_local(&process,&job,&cancel,["요약 비교 모델을 불러오고 있습니다.","요약 비교 모델을 다시 준비하고 있습니다."],|gpu,folder| {
        let (process,source,notes,attendees,cache,weights,cancel)=(&process,&source,&notes,&attendees,&cache,&weights,&cancel);
        async move {
            let mut server=launch(process,weights,gpu,&folder,cancel).await?;
            let outcome=async {
                let mut variants=Vec::new();
                for (name,roster) in [("baseline",&[][..]),("roster",attendees.as_slice())] {
                    check_cancel(cancel)?;
                    let destination=directory(&folder,name).map_err(storage)?;
                    let (text,receipt)=pipeline::run_with_template(&server,source,notes,roster,cache,process,cancel,None).await?;
                    validate_references(&text,source.segments.len()).map_err(|error|Failure::new(FailureKind::Output,error))?;
                    fs::write(destination.join("summary.md"),&text).map_err(storage)?;
                    fs::write(destination.join("receipt.json"),serde_json::to_vec_pretty(&receipt).map_err(storage)?).map_err(storage)?;
                    let readable=nero_core::presentation::format_summary(&text,nero_core::DocumentFormat::Txt,false,false,Some(source.segments.len()));
                    let mentions:Vec<_>=attendees.iter().filter_map(|attendee|attendee.name.as_ref()).map(|name|json!({
                        "name":name,
                        "literalTranscriptMention":source.segments.iter().any(|segment|segment.text.contains(name)),
                        "userNamedSpeakerReference":source.speakers.iter().any(|speaker|speaker.name==*name && speaker.name_origin==Some(SpeakerNameOrigin::User)),
                        "summaryLines":readable.lines().filter(|line|line.contains(name)).collect::<Vec<_>>()
                    })).collect();
                    variants.push(json!({"variant":name,"directory":destination,"summarySha256":hash(text.as_bytes()),"notesSha256":receipt["notesSha256"],"attendeesSha256":receipt["attendeesSha256"],"strategy":receipt["strategy"],"nameMentions":mentions,"numericLines":readable.lines().filter(|line|line.chars().any(|ch|ch.is_ascii_digit())).collect::<Vec<_>>() }));
                }
                Ok((variants,json!({"sameServer":true,"pipelineVersion":pipeline::VERSION})))
            }.await;
            server.stop(process)?;
            outcome.map_err(|error|server.diagnose(error))
        }
    }));
    finished.store(true, Ordering::Release);
    if let Some(watcher) = watcher {
        watcher.join().unwrap();
    }
    let weights_after = fs::symlink_metadata(&weights).unwrap();
    let source_unchanged = read_fixture(&source_path) == source_bytes;
    let attendees_unchanged = read_fixture(&attendees_path) == attendees_bytes;
    let notes_unchanged =
        notes_path.as_deref().map(read_fixture).unwrap_or_default() == notes_bytes;
    let manifest_unchanged = read_fixture(&fixture_manifest_path) == manifest_bytes;
    let model_metadata_unchanged = weights_before.len() == weights_after.len()
        && weights_before.modified().ok() == weights_after.modified().ok()
        && weights_after.is_file()
        && !weights_after.file_type().is_symlink();
    let evaluation = json!({
        "schemaVersion":1,"status":if result.is_ok(){"completed"}else{"failed"},
        "scope":"explicit prefix excerpt and whole-meeting attendance reference; no full-recording coverage or speaker-quality claim",
        "fixtureManifest":manifest,"sourceSha256":hash(&source_bytes),"sourceEndSeconds":source_end,"segments":source.segments.len(),"attendeesSha256":attendees_sha256,
        "sourceUnchanged":source_unchanged,"attendeesUnchanged":attendees_unchanged,"notesUnchanged":notes_unchanged,"fixtureManifestUnchanged":manifest_unchanged,"modelMetadataUnchanged":model_metadata_unchanged,
        "model":spec.repository,"modelRevision":spec.revision,"modelSha256":spec.files[0].sha256,"pipelineVersion":pipeline::VERSION,"elapsedMs":started.elapsed().as_millis(),
        "variants":result.as_ref().ok().map(|(variants,_)|variants),"runtimeReceipt":result.as_ref().ok().map(|(_,receipt)|receipt),"failure":result.as_ref().err(),
        "coreCommitPerformed":false,"activeLibraryOpened":false,"modelCopies":false,"audioCopies":false,
        "review":{"meaningAndConditions":"unreviewed","amountsAndUnits":"unreviewed","assigneeAndNameSources":"unreviewed","note":"Name mentions and numeric lines are review aids, not correctness or identity judgments. Compare against the exact source excerpt; the whole-meeting roster does not imply everyone spoke here."}
    });
    fs::write(
        output.join("evaluation.json"),
        serde_json::to_vec_pretty(&evaluation).unwrap(),
    )
    .unwrap();
    assert!(
        source_unchanged
            && attendees_unchanged
            && notes_unchanged
            && manifest_unchanged
            && model_metadata_unchanged,
        "read-only inputs changed; see evaluation.json"
    );
    if let Err(error) = result {
        panic!(
            "attendance comparison failed: kind={:?}; see evaluation.json",
            error.kind
        );
    }
}
