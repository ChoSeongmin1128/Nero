use super::{retry_block_reason, JobStatus, ModelSpec, Processing, Running, SpeechEngine};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;

const RECEIPT: &str = "speech-model-retirement.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub pending: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    target: SpeechEngine,
    candidates: Vec<SpeechEngine>,
}

fn failure() -> String {
    "이전 음성 인식 모델을 정리하지 못했습니다. 현재 모델은 그대로 유지됩니다. 다시 시도해 주세요."
        .into()
}

fn checked_directory(root: &Path, relative: &Path) -> Result<Option<PathBuf>, String> {
    if !root.is_absolute()
        || root
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(failure());
    }
    let root_meta = match fs::symlink_metadata(root) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(failure()),
    };
    if !root_meta.is_dir() || root_meta.file_type().is_symlink() {
        return Err(failure());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if root_meta.file_attributes() & 0x400 != 0 {
            return Err(failure());
        }
    }
    let canonical_root = root.canonicalize().map_err(|_| failure())?;
    let mut path = canonical_root.clone();
    for part in relative.components() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => (),
            Ok(_) => return Err(failure()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(failure()),
        }
        crate::path_safety::safe_ancestors(&path).map_err(|_| failure())?;
        if path.canonicalize().map_err(|_| failure())? != path {
            return Err(failure());
        }
    }
    Ok(Some(path))
}

fn receipt_path(root: &Path) -> Result<Option<PathBuf>, String> {
    Ok(checked_directory(root, Path::new("preferences"))?.map(|path| path.join(RECEIPT)))
}

fn load(root: &Path) -> Result<Option<Receipt>, String> {
    let Some(path) = receipt_path(root)? else {
        return Ok(None);
    };
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => (),
        _ => return Err(failure()),
    }
    crate::path_safety::safe_ancestors(&path).map_err(|_| failure())?;
    let mut raw = Vec::new();
    super::plain_file(&path)?
        .take(4097)
        .read_to_end(&mut raw)
        .map_err(|_| failure())?;
    if raw.len() > 4096 {
        return Err(failure());
    }
    let receipt: Receipt = serde_json::from_slice(&raw).map_err(|_| failure())?;
    if receipt.schema_version != 1
        || receipt.candidates.len() > SpeechEngine::ALL.len()
        || receipt
            .candidates
            .iter()
            .any(|engine| *engine == SpeechEngine::AppleSpeech)
        || receipt
            .candidates
            .iter()
            .enumerate()
            .any(|(index, engine)| receipt.candidates[..index].contains(engine))
    {
        return Err(failure());
    }
    Ok(Some(receipt))
}

fn save(root: &Path, receipt: &Receipt) -> Result<(), String> {
    super::directory(root, "preferences").map_err(|_| failure())?;
    let path = receipt_path(root)?.ok_or_else(failure)?;
    if path.exists() || path.is_symlink() {
        super::plain_file(&path).map_err(|_| failure())?;
        crate::path_safety::safe_ancestors(&path).map_err(|_| failure())?;
    }
    let mut output =
        tempfile::NamedTempFile::new_in(path.parent().unwrap()).map_err(|_| failure())?;
    serde_json::to_writer(&mut output, receipt).map_err(|_| failure())?;
    output
        .write_all(b"\n")
        .and_then(|_| output.as_file().sync_all())
        .map_err(|_| failure())?;
    output.persist(&path).map_err(|_| failure())?;
    Ok(())
}

fn clear(root: &Path) -> Result<(), String> {
    let Some(path) = receipt_path(root)? else {
        return Ok(());
    };
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            crate::path_safety::safe_ancestors(&path).map_err(|_| failure())?;
            fs::remove_file(path).map_err(|_| failure())
        }
        _ => Err(failure()),
    }
}

fn remove_model(root: &Path, spec: ModelSpec) -> Result<(), String> {
    let relative = PathBuf::from(spec.relative_dir());
    let Some(directory) = checked_directory(root, &relative)? else {
        return Ok(());
    };
    let model = spec.model();
    let mut names = vec!["verified.json".to_string()];
    for file in &model.files {
        if Path::new(&file.name).components().count() != 1
            || !matches!(
                Path::new(&file.name).components().next(),
                Some(Component::Normal(_))
            )
        {
            return Err(failure());
        }
        names.extend([file.name.clone(), format!("{}.part", file.name)]);
    }
    for entry in fs::read_dir(&directory).map_err(|_| failure())? {
        let entry = entry.map_err(|_| failure())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if super::model_set_aside_file(&model, &name) {
            names.push(name);
        }
    }
    for name in &names {
        let path = directory.join(name);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
                crate::path_safety::safe_ancestors(&path).map_err(|_| failure())?;
            }
            _ => return Err(failure()),
        }
    }
    for name in names {
        checked_directory(root, &relative)?.ok_or_else(failure)?;
        let path = directory.join(name);
        crate::path_safety::safe_ancestors(&path).map_err(|_| failure())?;
        match fs::remove_file(path) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(failure()),
        }
    }
    Ok(())
}

pub(super) fn initial(root: &Path) -> Option<View> {
    match load(root) {
        Ok(None) => None,
        Ok(Some(_)) => Some(View {
            pending: true,
            error: None,
        }),
        Err(error) => Some(View {
            pending: true,
            error: Some(error),
        }),
    }
}

fn retired(engine: SpeechEngine) -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
        && matches!(
            engine,
            SpeechEngine::Nemotron | SpeechEngine::WhisperTurbo | SpeechEngine::WhisperLargeV3
        )
}

impl Processing {
    pub(super) fn queue_model_retirement(&self, target: SpeechEngine) -> Result<(), String> {
        if !target.offered() || !self.selectable(target) {
            return Ok(());
        }
        let schedule_failure = || {
            "이전 모델 정리 계획을 저장하지 못해 음성 인식 모델을 변경하지 않았습니다.".to_string()
        };
        let mut receipt = load(&self.root)
            .map_err(|_| schedule_failure())?
            .unwrap_or(Receipt {
                schema_version: 1,
                target,
                candidates: Vec::new(),
            });
        receipt.candidates.retain(|engine| retired(*engine));
        let required = target.requirements();
        for engine in SpeechEngine::ALL
            .into_iter()
            .filter(|engine| retired(*engine))
        {
            if !required.contains(&engine) && !receipt.candidates.contains(&engine) {
                receipt.candidates.push(engine);
            }
        }
        receipt.target = target;
        if receipt.candidates.is_empty() {
            return Ok(());
        }
        save(&self.root, &receipt).map_err(|_| schedule_failure())?;
        *self.model_cleanup.lock().unwrap() = Some(View {
            pending: true,
            error: None,
        });
        Ok(())
    }

    pub(super) fn retire_models_locked(&self, running: &Option<Running>) {
        let state = self.model_cleanup.lock().unwrap().clone();
        if state.is_none()
            || state.as_ref().is_some_and(|state| state.error.is_some())
            || !self.model_cleanup_ready.load(Ordering::Acquire)
            || !self.idle_locked(running)
        {
            return;
        }
        let result = (|| {
            let Some(mut receipt) = load(&self.root)? else {
                return Ok(false);
            };
            receipt.candidates.retain(|engine| retired(*engine));
            save(&self.root, &receipt)?;
            let current = self.speech_engine();
            if current != receipt.target {
                return Err("모델 변경이 완료되지 않아 이전 모델 정리를 보류했습니다. 현재 모델을 확인한 뒤 정리를 다시 시도해 주세요.".into());
            }
            if !self.supported_for(current) || !self.model_ready_for(current) {
                return Ok(true);
            }
            self.worker_ready()?;
            let current_requirements = current.requirements();
            let mut required = current_requirements.clone();
            if let Some(pending) = *self.speech_switch.lock().unwrap() {
                required.extend(pending.target.requirements());
            }
            for record in self.records.lock().unwrap().iter() {
                if matches!(record.kind.as_str(), "setup" | "model" | "transcribe")
                    && (matches!(
                        record.status,
                        JobStatus::Queued | JobStatus::Running | JobStatus::Cancelling
                    ) || retry_block_reason(record).is_none())
                {
                    required.extend(record.speech_engine.requirements());
                }
            }
            let candidates = receipt.candidates.clone();
            for engine in candidates {
                if current_requirements.contains(&engine) {
                    receipt.candidates.retain(|candidate| *candidate != engine);
                    save(&self.root, &receipt)?;
                    continue;
                }
                if required.contains(&engine) {
                    continue;
                }
                remove_model(&self.root, ModelSpec::speech(engine))?;
                receipt.candidates.retain(|candidate| *candidate != engine);
                save(&self.root, &receipt)?;
            }
            if receipt.candidates.is_empty() {
                clear(&self.root)?;
            }
            Ok::<bool, String>(!receipt.candidates.is_empty())
        })();
        let next = match result {
            Ok(false) => None,
            Ok(true) => Some(View {
                pending: true,
                error: None,
            }),
            Err(error) => Some(View {
                pending: true,
                error: Some(error),
            }),
        };
        if state != next {
            *self.model_cleanup.lock().unwrap() = next;
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
    }

    pub fn retry_speech_model_cleanup(&self) -> Result<super::Progress, String> {
        let _switch = self.speech_switch_gate.lock().unwrap();
        let running = self.running.lock().unwrap();
        if !self.idle_locked(&running) {
            return Err("녹음과 처리 작업을 마친 뒤 모델 정리를 다시 시도해 주세요.".into());
        }
        if let Some(mut receipt) = load(&self.root)? {
            let current = self.speech_engine();
            if receipt.target != current {
                if !self.supported_for(current) || !self.model_ready_for(current) {
                    return Err(
                        "현재 음성 인식 모델을 먼저 준비한 뒤 이전 모델 정리를 다시 시도해 주세요."
                            .into(),
                    );
                }
                receipt.target = current;
                save(&self.root, &receipt)?;
            }
        }
        *self.model_cleanup.lock().unwrap() = initial(&self.root).map(|_| View {
            pending: true,
            error: None,
        });
        self.retire_models_locked(&running);
        Ok(self.snapshot())
    }

    pub(super) fn reconcile_model_retirement(&self, running: &Option<Running>) {
        let Ok(_switch) = self.speech_switch_gate.try_lock() else {
            return;
        };
        self.retire_models_locked(running);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job_journal::JobRecord;
    use std::sync::Arc;

    fn fixture() -> (tempfile::TempDir, Arc<Processing>) {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        crate::speech_choice::save(&root, SpeechEngine::Nemotron, false, None).unwrap();
        let process = Processing::new(root.clone(), root);
        *process.platform_supported_override.lock().unwrap() = Some(true);
        *process.speaker_enabled.lock().unwrap() = false;
        process.model_cleanup_ready.store(true, Ordering::Release);
        for path in [
            process.worker("AudioPrepare"),
            process.resources.join("native/moss/MossHarness"),
            crate::native_speech::executable(&process, true),
            crate::native_speech::library(&process, true),
            crate::whisper_asr::executable(&process.resources),
            process.resources.join("native/whisper/runtime.json"),
            process
                .resources
                .join("native/whisper/bin/nero-vulkan-probe.exe"),
            process
                .resources
                .join("native/whisper/vulkan/bin/whisper-cli.exe"),
            crate::qwen_asr::executable(&process.resources),
            process.resources.join("native/crispasr/runtime.json"),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"synthetic unexecuted runtime").unwrap();
        }
        (temporary, process)
    }

    fn ready(process: &Processing, engine: SpeechEngine) -> PathBuf {
        let spec = ModelSpec::speech(engine);
        let directory = process.model_dir_of(spec).unwrap();
        for file in spec.model().files {
            fs::File::create(directory.join(file.name))
                .unwrap()
                .set_len(file.bytes)
                .unwrap();
        }
        fs::write(directory.join("verified.json"), spec.manifest).unwrap();
        directory
    }

    fn model_file(process: &Processing, engine: SpeechEngine) -> PathBuf {
        let spec = ModelSpec::speech(engine);
        process
            .root
            .join(spec.relative_dir())
            .join(&spec.model().files[0].name)
    }

    fn transcribe_record() -> JobRecord {
        JobRecord {
            glossary_snapshot_required: false,
            speech_engine: SpeechEngine::Nemotron,
            speaker_enabled: false,
            apple_speech_variant: None,
            schema_version: 1,
            id: uuid::Uuid::new_v4().to_string(),
            kind: "transcribe".into(),
            meeting_id: None,
            source_fingerprint: None,
            include_summary: false,
            summary_template_ref: None,
            acknowledged: false,
            retry_successor_id: None,
            failure_kind: None,
            status: JobStatus::Failed,
            phase: Some("recognizing".into()),
            message: "synthetic".into(),
            created_at: 1,
            updated_at: 1,
        }
    }

    fn supported(process: &Processing, engine: SpeechEngine) -> bool {
        process.supported_for(engine)
    }

    #[test]
    fn ready_explicit_switch_retires_asr_weights_and_owned_residue_only() {
        let (_temporary, process) = fixture();
        if !supported(&process, SpeechEngine::Moss) {
            return;
        }
        let old = ready(&process, SpeechEngine::Nemotron);
        ready(&process, SpeechEngine::Moss);
        let name = ModelSpec::speech(SpeechEngine::Nemotron).model().files[0]
            .name
            .clone();
        let residue = [
            format!("{name}.part"),
            format!("{name}.invalid-old"),
            format!("{name}.partial-old"),
            "verified.invalid-old.json".into(),
        ];
        for name in &residue {
            fs::write(old.join(name), b"owned old bytes").unwrap();
        }
        fs::write(old.join("unrelated.invalid-old"), b"keep").unwrap();
        let speaker = process
            .root
            .join(crate::diarization::SPEAKER_MODEL.relative_dir());
        fs::create_dir_all(&speaker).unwrap();
        fs::write(speaker.join("speaker.gguf"), b"keep").unwrap();
        let old_revision = process.root.join("models/Nemotron35-ASR-other-revision");
        fs::create_dir(&old_revision).unwrap();
        fs::write(old_revision.join(&name), b"keep").unwrap();
        let after = process.request_speech_engine(SpeechEngine::Moss).unwrap();
        assert_eq!(after.speech_engine, Some(SpeechEngine::Moss));
        assert!(after.model_ready);
        assert!(after.speech_cleanup.is_none());
        assert!(!old.join(&name).exists());
        assert!(!old.join("verified.json").exists());
        for name in residue {
            assert!(!old.join(name).exists());
        }
        assert!(old.join("unrelated.invalid-old").exists());
        assert!(speaker.join("speaker.gguf").exists());
        assert!(old_revision.join(name).exists());
        assert!(crate::native_speech::library(&process, true).exists());
        assert!(model_file(&process, SpeechEngine::Moss).exists());
        assert!(load(&process.root).unwrap().is_none());
    }

    #[test]
    fn an_unconfirmed_worker_shutdown_preserves_old_weights_and_reports_cleanup_separately() {
        let (_temporary, process) = fixture();
        if !supported(&process, SpeechEngine::Moss) {
            return;
        }
        ready(&process, SpeechEngine::Nemotron);
        ready(&process, SpeechEngine::Moss);
        process.cleanup_failed();
        let after = process.request_speech_engine(SpeechEngine::Moss).unwrap();
        assert_eq!(after.speech_engine, Some(SpeechEngine::Moss));
        assert!(after.model_ready);
        assert!(after
            .speech_cleanup
            .unwrap()
            .error
            .unwrap()
            .contains("종료를 확인하지 못했습니다"));
        assert!(model_file(&process, SpeechEngine::Nemotron).exists());
        assert!(load(&process.root).unwrap().is_some());
        let retried = process.retry_speech_model_cleanup().unwrap();
        assert!(retried.speech_cleanup.unwrap().error.is_some());
        assert!(model_file(&process, SpeechEngine::Nemotron).exists());
    }

    #[test]
    fn qwen_no_longer_needs_nemotron_preview_weights() {
        let (_temporary, process) = fixture();
        if !supported(&process, SpeechEngine::QwenKo) {
            return;
        }
        ready(&process, SpeechEngine::Nemotron);
        ready(&process, SpeechEngine::QwenKo);
        let after = process.request_speech_engine(SpeechEngine::QwenKo).unwrap();
        assert_eq!(after.speech_engine, Some(SpeechEngine::QwenKo));
        assert!(!model_file(&process, SpeechEngine::Nemotron).exists());
        assert!(after.speech_cleanup.is_none());
    }

    #[test]
    fn unready_target_preserves_previous_weights_until_explicit_preparation_finishes() {
        let (_temporary, process) = fixture();
        if !supported(&process, SpeechEngine::Moss) {
            return;
        }
        ready(&process, SpeechEngine::Nemotron);
        process.set_speech_engine(SpeechEngine::Moss).unwrap();
        assert!(model_file(&process, SpeechEngine::Nemotron).exists());
        assert!(load(&process.root).unwrap().is_some());
        process.reconcile_model_retirement(&process.running.lock().unwrap());
        assert!(model_file(&process, SpeechEngine::Nemotron).exists());
        ready(&process, SpeechEngine::Moss);
        process.reconcile_model_retirement(&process.running.lock().unwrap());
        assert!(!model_file(&process, SpeechEngine::Nemotron).exists());
    }

    #[test]
    fn retryable_asr_work_protects_weights_until_acknowledged() {
        let (_temporary, process) = fixture();
        if !supported(&process, SpeechEngine::Moss) {
            return;
        }
        ready(&process, SpeechEngine::Nemotron);
        ready(&process, SpeechEngine::Moss);
        process.records.lock().unwrap().push(transcribe_record());
        let after = process.request_speech_engine(SpeechEngine::Moss).unwrap();
        assert!(after.speech_cleanup.is_some());
        assert!(model_file(&process, SpeechEngine::Nemotron).exists());
        let revision = process.revision.load(Ordering::Acquire);
        process.reconcile_model_retirement(&process.running.lock().unwrap());
        assert_eq!(process.revision.load(Ordering::Acquire), revision);
        process.records.lock().unwrap()[0].acknowledged = true;
        process.reconcile_model_retirement(&process.running.lock().unwrap());
        assert!(!model_file(&process, SpeechEngine::Nemotron).exists());
    }

    #[test]
    fn saved_transcript_postprocessing_does_not_keep_the_old_asr_model() {
        let (_temporary, process) = fixture();
        if !supported(&process, SpeechEngine::Moss) {
            return;
        }
        ready(&process, SpeechEngine::Nemotron);
        ready(&process, SpeechEngine::Moss);
        let mut record = transcribe_record();
        record.phase = Some("postprocessing".into());
        process.records.lock().unwrap().push(record);
        process.request_speech_engine(SpeechEngine::Moss).unwrap();
        assert!(!model_file(&process, SpeechEngine::Nemotron).exists());
    }

    #[test]
    fn capture_or_queue_prevents_retirement_and_first_core_sync_is_required() {
        let (_temporary, process) = fixture();
        if !supported(&process, SpeechEngine::Moss) {
            return;
        }
        ready(&process, SpeechEngine::Nemotron);
        ready(&process, SpeechEngine::Moss);
        process.model_cleanup_ready.store(false, Ordering::Release);
        process.request_speech_engine(SpeechEngine::Moss).unwrap();
        assert!(model_file(&process, SpeechEngine::Nemotron).exists());
        process.model_cleanup_ready.store(true, Ordering::Release);
        process.recording.store(true, Ordering::Release);
        process.reconcile_model_retirement(&process.running.lock().unwrap());
        assert!(model_file(&process, SpeechEngine::Nemotron).exists());
        process.recording.store(false, Ordering::Release);
        process.reconcile_model_retirement(&process.running.lock().unwrap());
        assert!(!model_file(&process, SpeechEngine::Nemotron).exists());
    }

    #[test]
    fn uncommitted_switch_never_deletes_and_a_later_selection_keeps_the_receipt_union() {
        let (_temporary, process) = fixture();
        if !supported(&process, SpeechEngine::Moss) {
            return;
        }
        ready(&process, SpeechEngine::Nemotron);
        ready(&process, SpeechEngine::Moss);
        process.queue_model_retirement(SpeechEngine::Moss).unwrap();
        process.reconcile_model_retirement(&process.running.lock().unwrap());
        assert!(model_file(&process, SpeechEngine::Nemotron).exists());
        assert!(model_file(&process, SpeechEngine::Moss).exists());
        assert!(process
            .model_cleanup
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .error
            .is_some());
        assert!(load(&process.root).unwrap().is_some());
        process.request_speech_engine(SpeechEngine::Moss).unwrap();
        assert!(!model_file(&process, SpeechEngine::Nemotron).exists());
        assert!(load(&process.root).unwrap().is_none());
    }

    #[test]
    fn a_retired_engine_cannot_be_selected_after_switching_to_moss() {
        let (_temporary, process) = fixture();
        if !supported(&process, SpeechEngine::Moss) {
            return;
        }
        ready(&process, SpeechEngine::Nemotron);
        ready(&process, SpeechEngine::Moss);
        process.records.lock().unwrap().push(transcribe_record());
        process.request_speech_engine(SpeechEngine::Moss).unwrap();
        assert!(process
            .request_speech_engine(SpeechEngine::Nemotron)
            .is_err());
        assert_eq!(process.speech_engine(), SpeechEngine::Moss);
        assert!(model_file(&process, SpeechEngine::Nemotron).exists());
        assert!(model_file(&process, SpeechEngine::Moss).exists());
    }

    #[test]
    fn explicit_cleanup_retry_rebases_an_uncommitted_receipt_and_preserves_the_current_model() {
        let (_temporary, process) = fixture();
        if !supported(&process, SpeechEngine::Nemotron) {
            return;
        }
        ready(&process, SpeechEngine::Nemotron);
        ready(&process, SpeechEngine::Moss);
        save(
            &process.root,
            &Receipt {
                schema_version: 1,
                target: SpeechEngine::Moss,
                candidates: vec![SpeechEngine::Moss, SpeechEngine::Nemotron],
            },
        )
        .unwrap();
        *process.model_cleanup.lock().unwrap() = initial(&process.root);
        process.reconcile_model_retirement(&process.running.lock().unwrap());
        assert!(model_file(&process, SpeechEngine::Moss).exists());
        let after = process.retry_speech_model_cleanup().unwrap();
        assert_eq!(after.speech_engine, Some(SpeechEngine::Nemotron));
        assert!(model_file(&process, SpeechEngine::Nemotron).exists());
        assert!(model_file(&process, SpeechEngine::Moss).exists());
        assert!(after.speech_cleanup.is_none());
    }

    #[test]
    fn moss_transition_retires_all_three_legacy_asr_models_but_keeps_qwen() {
        let (_temporary, process) = fixture();
        if !supported(&process, SpeechEngine::Moss) {
            return;
        }
        for engine in [
            SpeechEngine::Nemotron,
            SpeechEngine::WhisperTurbo,
            SpeechEngine::WhisperLargeV3,
            SpeechEngine::Moss,
            SpeechEngine::QwenKo,
        ] {
            ready(&process, engine);
        }
        process.request_speech_engine(SpeechEngine::Moss).unwrap();
        for engine in [
            SpeechEngine::Nemotron,
            SpeechEngine::WhisperTurbo,
            SpeechEngine::WhisperLargeV3,
        ] {
            assert!(!model_file(&process, engine).exists());
        }
        for engine in [SpeechEngine::Moss, SpeechEngine::QwenKo] {
            assert!(model_file(&process, engine).exists());
        }
    }

    #[test]
    fn stale_receipts_cannot_delete_moss_or_qwen() {
        let (_temporary, process) = fixture();
        if !supported(&process, SpeechEngine::Moss) {
            return;
        }
        ready(&process, SpeechEngine::Moss);
        ready(&process, SpeechEngine::QwenKo);
        ready(&process, SpeechEngine::Nemotron);
        process.request_speech_engine(SpeechEngine::Moss).unwrap();
        save(
            &process.root,
            &Receipt {
                schema_version: 1,
                target: SpeechEngine::Moss,
                candidates: vec![
                    SpeechEngine::Moss,
                    SpeechEngine::QwenKo,
                    SpeechEngine::Nemotron,
                ],
            },
        )
        .unwrap();
        *process.model_cleanup.lock().unwrap() = initial(&process.root);
        process.reconcile_model_retirement(&process.running.lock().unwrap());
        assert!(model_file(&process, SpeechEngine::Moss).exists());
        assert!(model_file(&process, SpeechEngine::QwenKo).exists());
        assert!(!model_file(&process, SpeechEngine::Nemotron).exists());
    }

    #[cfg(unix)]
    #[test]
    fn linked_model_root_or_leaf_is_never_followed_and_failure_can_resume_after_restart() {
        let (_temporary, process) = fixture();
        if !supported(&process, SpeechEngine::Moss) {
            return;
        }
        ready(&process, SpeechEngine::Nemotron);
        ready(&process, SpeechEngine::Moss);
        let old = model_file(&process, SpeechEngine::Nemotron);
        fs::remove_file(&old).unwrap();
        let outside = process.root.join("outside-model");
        fs::write(&outside, b"outside").unwrap();
        std::os::unix::fs::symlink(&outside, &old).unwrap();
        let after = process.request_speech_engine(SpeechEngine::Moss).unwrap();
        assert_eq!(after.speech_engine, Some(SpeechEngine::Moss));
        assert!(after.speech_cleanup.unwrap().error.is_some());
        assert_eq!(fs::read(&outside).unwrap(), b"outside");
        fs::remove_file(&old).unwrap();
        fs::write(&old, b"owned old model").unwrap();
        let reopened = Processing::new(process.root.clone(), process.resources.clone());
        *reopened.platform_supported_override.lock().unwrap() = Some(true);
        reopened.model_cleanup_ready.store(true, Ordering::Release);
        reopened.reconcile_model_retirement(&reopened.running.lock().unwrap());
        assert!(!old.exists());
        assert!(load(&reopened.root).unwrap().is_none());
        assert_eq!(fs::read(&outside).unwrap(), b"outside");

        let models = process.root.join("models");
        let retained = process.root.join("retained-models");
        fs::rename(&models, &retained).unwrap();
        std::os::unix::fs::symlink(&retained, &models).unwrap();
        assert!(remove_model(&process.root, ModelSpec::speech(SpeechEngine::Moss)).is_err());
        assert!(retained
            .join(
                ModelSpec::speech(SpeechEngine::Moss)
                    .relative_dir()
                    .trim_start_matches("models/")
            )
            .exists());
    }
}
