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
            matches!(component, std::path::Component::Normal(name) if name == "io.github.choseongmin1128.grove.desktop")
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
