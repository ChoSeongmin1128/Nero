use crate::processing::{Processing, Progress, Refusal};
use crate::publish;
use crate::speech_choice::SpeechEngine;
use nero_core::agent_control::{
    AppUpdateAction, ControlError, ControlReply, Handlers, JobRequest, Mutation, ProcessingRequest,
    Server,
};
use nero_core::{CoreError, CoreHandle, DraftAttachmentTarget};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

#[derive(PartialEq)]
enum Phase {
    Waiting,
    Ready,
    Rejected,
    Finished,
}
struct Pending {
    state: Mutex<(Phase, bool)>,
    changed: Condvar,
}
#[derive(Default)]
struct Gate {
    pending: Mutex<BTreeMap<String, Arc<Pending>>>,
    closing: AtomicBool,
}

impl Gate {
    fn begin(&self) -> Result<(String, Arc<Pending>), ControlError> {
        if self.closing.load(Ordering::Acquire) {
            return Err(ControlError::new("app_not_ready", "Nero가 종료 중입니다."));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let pending = Arc::new(Pending {
            state: Mutex::new((Phase::Waiting, false)),
            changed: Condvar::new(),
        });
        self.pending
            .lock()
            .unwrap()
            .insert(id.clone(), pending.clone());
        Ok((id, pending))
    }
    fn wait_ready(
        &self,
        id: &str,
        pending: &Arc<Pending>,
        timeout: Duration,
    ) -> Result<(), ControlError> {
        let deadline = Instant::now() + timeout;
        let mut state = pending.state.lock().unwrap();
        while state.0 == Phase::Waiting && !self.closing.load(Ordering::Acquire) {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            state = pending.changed.wait_timeout(state, left).unwrap().0;
        }
        if state.0 != Phase::Ready || self.closing.load(Ordering::Acquire) {
            state.0 = Phase::Finished;
            pending.changed.notify_all();
            drop(state);
            self.pending.lock().unwrap().remove(id);
            return Err(ControlError::new("app_not_ready", "Nero의 편집 저장을 확인하지 못했습니다. 앱에서 저장 상태를 확인한 뒤 다시 시도해 주세요."));
        }
        Ok(())
    }
    fn finish(&self, id: &str, pending: &Arc<Pending>) {
        pending.state.lock().unwrap().0 = Phase::Finished;
        pending.changed.notify_all();
        self.pending.lock().unwrap().remove(id);
    }
    fn ready(&self, id: &str, error: Option<String>) -> Result<(), String> {
        if id.parse::<uuid::Uuid>().is_err() {
            return Err("수정 요청이 올바르지 않습니다.".into());
        }
        let pending = self
            .pending
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or("수정 요청이 만료되었습니다.")?;
        let mut state = pending.state.lock().unwrap();
        if state.0 != Phase::Waiting || state.1 || self.closing.load(Ordering::Acquire) {
            return Err("수정 요청이 만료되었습니다.".into());
        }
        state.1 = true;
        state.0 = if error.is_some() {
            Phase::Rejected
        } else {
            Phase::Ready
        };
        pending.changed.notify_all();
        if error.is_some() {
            return Ok(());
        }
        while state.0 != Phase::Finished && !self.closing.load(Ordering::Acquire) {
            state = pending.changed.wait(state).unwrap();
        }
        Ok(())
    }
    fn close(&self) {
        self.closing.store(true, Ordering::Release);
        let pending = self
            .pending
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for item in pending {
            item.state.lock().unwrap().0 = Phase::Finished;
            item.changed.notify_all();
        }
    }
}

struct Lease {
    gate: Arc<Gate>,
    id: String,
    pending: Arc<Pending>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.gate.finish(&self.id, &self.pending);
    }
}

pub struct AgentMutations {
    gate: Arc<Gate>,
    server: Mutex<Option<Server>>,
}
impl AgentMutations {
    pub fn start(
        app: tauri::AppHandle,
        root: &Path,
        core: CoreHandle,
        processing: Arc<Processing>,
    ) -> Arc<Self> {
        let state = Arc::new(Self {
            gate: Arc::new(Gate::default()),
            server: Mutex::new(None),
        });
        let weak = Arc::downgrade(&state);
        let worker_core = core.clone();
        let job_state = weak.clone();
        let job_processing = processing.clone();
        let metadata_app = app.clone();
        let update_app = app.clone();
        let update_state = weak.clone();
        let processing_app = app.clone();
        let processing_state = weak.clone();
        let processing_core = core.clone();
        let processing_service = processing.clone();
        let app_handle = app.clone();
        let app_state = weak.clone();
        let handlers = Handlers::new(move |request| {
            let state = weak.upgrade().ok_or_else(ControlError::unavailable)?;
            state.execute(&app, &worker_core, &processing, request)
        })
        .jobs(move |request| {
            let state = job_state.upgrade().ok_or_else(ControlError::unavailable)?;
            if state.gate.closing.load(Ordering::Acquire) {
                return Err(ControlError::new("app_not_ready", "Nero가 종료 중입니다."));
            }
            match request {
                JobRequest::List {
                    meeting_id,
                    offset,
                    limit,
                } => job_processing.agent_jobs(meeting_id.as_deref(), offset, limit),
                JobRequest::Cancel {
                    job_id,
                    expected_version,
                } => job_processing.agent_cancel(&job_id, &expected_version),
            }
        })
        .metadata(move || {
            Ok(metadata_app
                .state::<crate::app_updater::AppUpdates>()
                .agent_metadata(metadata_app.path().resource_dir().ok().as_deref()))
        })
        .updates(move |action| {
            update_state
                .upgrade()
                .ok_or_else(ControlError::unavailable)?
                .execute_update(&update_app, action)
        })
        .processing(move |request| {
            processing_state
                .upgrade()
                .ok_or_else(ControlError::unavailable)?
                .execute_processing(
                    &processing_app,
                    &processing_core,
                    &processing_service,
                    request,
                )
        })
        .app(move |request| {
            let state = app_state.upgrade().ok_or_else(ControlError::unavailable)?;
            if state.gate.closing.load(Ordering::Acquire) {
                return Err(ControlError::new("app_not_ready", "Nero가 종료 중입니다."));
            }
            let save_edits = || state.save_edits(&app_handle, "meetingMutation");
            crate::agent_app::AgentApp {
                app: &app_handle,
                save_edits: &save_edits,
            }
            .handle(request)
        });
        let server = Server::serve(root, core, handlers);
        match server {
            Ok(server) => *state.server.lock().unwrap() = Some(server),
            Err(_) => eprintln!("Nero 에이전트 수정 연결을 열지 못했습니다."),
        }
        state
    }
    fn execute(
        &self,
        app: &tauri::AppHandle,
        core: &CoreHandle,
        processing: &Processing,
        request: Mutation,
    ) -> Result<Value, ControlError> {
        core.agent_validate(request.clone())
            .map_err(ControlError::core)?;
        let _lease = self.save_edits(app, "meetingMutation")?;
        let result = if let Some(ids) = request.trash_ids() {
            crate::trash_coordinated(app, processing, core, &ids, || {
                if self.gate.closing.load(Ordering::Acquire) {
                    return Err(nero_core::CoreError::Unavailable);
                }
                core.agent_mutate(request)
            })
        } else if self.gate.closing.load(Ordering::Acquire) {
            Err(nero_core::CoreError::Unavailable)
        } else if let Mutation::SummaryImport { meeting_id, .. } = &request {
            let meeting_id = meeting_id.clone();
            processing.meeting_idle(&meeting_id, || core.agent_mutate(request))
        } else {
            core.agent_mutate(request)
        };
        if let Ok(snapshot) = core.snapshot() {
            publish(app, &snapshot);
        }
        result.map_err(ControlError::core)
    }
    fn execute_processing(
        &self,
        app: &tauri::AppHandle,
        core: &CoreHandle,
        processing: &Arc<Processing>,
        request: ProcessingRequest,
    ) -> Result<Value, ControlError> {
        if self.gate.closing.load(Ordering::Acquire) {
            return Err(ControlError::new("app_not_ready", "Nero가 종료 중입니다."));
        }
        let port = AppProcessing { core, processing };
        // Attaching to a draft saves open editors first, like other meeting edits.
        handle_processing(&port, request, || self.save_edits(app, "meetingMutation"))
    }
    fn save_edits(&self, app: &tauri::AppHandle, kind: &str) -> Result<Lease, ControlError> {
        let (id, pending) = self.gate.begin()?;
        let lease = Lease {
            gate: self.gate.clone(),
            id: id.clone(),
            pending: pending.clone(),
        };
        if app
            .emit_to(
                "main",
                "nero://agent-write-requested",
                serde_json::json!({"requestId":id,"kind":kind}),
            )
            .is_err()
        {
            return Err(ControlError::new(
                "app_not_ready",
                "Nero 편집 화면에 연결하지 못했습니다.",
            ));
        }
        self.gate
            .wait_ready(&lease.id, &pending, Duration::from_secs(10))?;
        Ok(lease)
    }
    fn execute_update(
        &self,
        app: &tauri::AppHandle,
        action: AppUpdateAction,
    ) -> Result<ControlReply, ControlError> {
        if action == AppUpdateAction::Status {
            return Ok(ControlReply::new(
                serde_json::json!({"apiVersion":1,"action":action,"status":crate::app_updater::status(app)}),
            ));
        }
        if self.gate.closing.load(Ordering::Acquire) {
            return Err(ControlError::new("app_not_ready", "Nero가 종료 중입니다."));
        }
        match action {
            AppUpdateAction::Status => unreachable!(),
            AppUpdateAction::Check => {
                let status = tauri::async_runtime::block_on(crate::app_updater::check(app))
                    .map_err(|error| ControlError::new("app_update_check_failed", error))?;
                Ok(ControlReply::new(
                    serde_json::json!({"apiVersion":1,"action":action,"status":status}),
                ))
            }
            AppUpdateAction::Download => {
                let status = crate::app_updater::start_download(app.clone())
                    .map_err(|error| ControlError::new("app_update_failed", error))?;
                Ok(ControlReply::new(
                    serde_json::json!({"apiVersion":1,"action":action,"accepted":true,"status":status}),
                ))
            }
            AppUpdateAction::Install => {
                let lease = self.save_edits(app, "appUpdate")?;
                let plan = tauri::async_runtime::block_on(crate::app_updater::prepare_install(
                    app.clone(),
                ))
                .map_err(|error| ControlError::new("app_update_failed", error))?;
                let accepted = plan.accepted();
                let result = serde_json::json!({"apiVersion":1,"action":action,"accepted":accepted,"restarting":accepted,"status":plan.status.clone()});
                Ok(ControlReply::after_response(result, move || {
                    plan.start();
                    drop(lease);
                }))
            }
        }
    }
    pub fn ready(&self, id: &str, error: Option<String>) -> Result<(), String> {
        self.gate.ready(id, error)
    }
    pub fn stop(&self) {
        self.gate.close();
        self.server.lock().unwrap().take();
    }
}
impl Drop for AgentMutations {
    fn drop(&mut self) {
        self.gate.close();
        self.server.get_mut().unwrap().take();
    }
}

// What agent processing requests need from the running app. Requests use the same
// `Processing` entry points as the app's own screens, which also check their conditions.
trait ProcessingPort {
    fn progress(&self) -> Progress;
    fn import_preflight(&self) -> Result<(), Refusal>;
    fn import(
        &self,
        source: PathBuf,
        target: Option<DraftAttachmentTarget>,
    ) -> Result<String, Refusal>;
    fn job(&self, id: &str) -> Value;
    fn speech_engine_preflight(
        &self,
        engine: SpeechEngine,
        acknowledged: bool,
    ) -> Result<(), Refusal>;
    fn request_speech_engine(
        &self,
        engine: SpeechEngine,
        acknowledged: bool,
    ) -> Result<Progress, Refusal>;
    fn cancel_speech_switch(&self) -> Result<Progress, String>;
}

struct AppProcessing<'a> {
    core: &'a CoreHandle,
    processing: &'a Arc<Processing>,
}

impl ProcessingPort for AppProcessing<'_> {
    fn progress(&self) -> Progress {
        self.processing.snapshot()
    }
    fn import_preflight(&self) -> Result<(), Refusal> {
        self.processing.import_preflight(self.core, None)
    }
    fn import(
        &self,
        source: PathBuf,
        target: Option<DraftAttachmentTarget>,
    ) -> Result<String, Refusal> {
        self.processing.import(self.core.clone(), source, target)
    }
    fn job(&self, id: &str) -> Value {
        self.processing.agent_job(id).unwrap_or(Value::Null)
    }
    fn speech_engine_preflight(
        &self,
        engine: SpeechEngine,
        acknowledged: bool,
    ) -> Result<(), Refusal> {
        self.processing
            .speech_engine_preflight(self.core, engine, acknowledged)
    }
    fn request_speech_engine(
        &self,
        engine: SpeechEngine,
        acknowledged: bool,
    ) -> Result<Progress, Refusal> {
        self.processing
            .request_speech_engine(self.core, engine, acknowledged)
    }
    fn cancel_speech_switch(&self) -> Result<Progress, String> {
        self.processing.cancel_speech_switch()
    }
}

// Handlers never answer `invalid_input`: the CLI reports that code from a processing
// request as an app that predates it.
fn recording_active() -> ControlError {
    ControlError::new("recording_active", "녹음을 마친 뒤 다시 시도해 주세요.")
}
fn rejected() -> ControlError {
    ControlError::new("processing_rejected", "Nero가 요청을 처리하지 못했습니다.")
}
fn active_job(status: &str) -> bool {
    matches!(status, "queued" | "running" | "cancelling")
}
// The codes agents read for each reason Processing refuses; `other` answers the rest.
fn refused(refusal: Refusal, other: impl FnOnce(String) -> ControlError) -> ControlError {
    match refusal {
        Refusal::Recording(_) => recording_active(),
        Refusal::NotReady(_) => {
            ControlError::new("speech_not_ready", "음성 인식을 먼저 준비해 주세요.")
        }
        Refusal::MemoryWarning => ControlError::new(
            "memory_warning_unacknowledged",
            "메모리 경고에 대한 사용자 확인이 필요합니다.",
        ),
        Refusal::Meeting(CoreError::InvalidInput(_)) => {
            ControlError::core(CoreError::MeetingNotFound)
        }
        Refusal::Meeting(error) => ControlError::core(error),
        Refusal::Other(message) => other(message),
    }
}

fn handle_processing<G>(
    port: &impl ProcessingPort,
    request: ProcessingRequest,
    save_edits: impl FnOnce() -> Result<G, ControlError>,
) -> Result<Value, ControlError> {
    match request {
        ProcessingRequest::MeetingImport {
            path,
            meeting_id,
            expected_version,
        } => {
            let target = match (meeting_id, expected_version) {
                (Some(meeting_id), Some(expected_version)) => Some(DraftAttachmentTarget {
                    meeting_id,
                    expected_version,
                }),
                (None, None) => None,
                _ => return Err(rejected()),
            };
            meeting_import(port, &path, target, save_edits)
        }
        ProcessingRequest::SpeechStatus {} => Ok(speech_view(&port.progress())),
        ProcessingRequest::SpeechEngineSet {
            engine,
            acknowledge_memory_warning,
        } => speech_engine_set(port, &engine, acknowledge_memory_warning),
        ProcessingRequest::SpeechSwitchCancel { target } => speech_switch_cancel(port, &target),
    }
}

fn import_source(path: &str) -> Result<PathBuf, ControlError> {
    let source = PathBuf::from(path);
    let unavailable = || {
        ControlError::new(
            "audio_file_unavailable",
            "가져올 음성 파일을 찾을 수 없습니다.",
        )
    };
    let metadata = std::fs::symlink_metadata(&source).map_err(|_| unavailable())?;
    if !source.is_absolute() || !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(unavailable());
    }
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    if !extension.is_some_and(|extension| {
        crate::speech_audio::import_extensions().contains(&extension.as_str())
    }) {
        return Err(ControlError::new(
            "unsupported_audio_format",
            "지원하지 않는 음성 파일입니다.",
        ));
    }
    Ok(source)
}

fn meeting_import<G>(
    port: &impl ProcessingPort,
    path: &str,
    target: Option<DraftAttachmentTarget>,
    save_edits: impl FnOnce() -> Result<G, ControlError>,
) -> Result<Value, ControlError> {
    let source = import_source(path)?;
    // The queue's own reason (the same import already queued, no space) is kept.
    let refused = |refusal| {
        refused(refusal, |message| {
            ControlError::new("processing_rejected", message)
        })
    };
    port.import_preflight().map_err(refused)?;
    // Open editors are saved before the import checks the draft's version.
    let _edits = target.as_ref().map(|_| save_edits()).transpose()?;
    let meeting_id = target.as_ref().map(|target| target.meeting_id.clone());
    let id = port.import(source, target).map_err(refused)?;
    Ok(json!({"applied":true,"meetingId":meeting_id,"job":port.job(&id)}))
}

fn unknown_engine() -> ControlError {
    ControlError::new("speech_engine_unknown", "알 수 없는 음성 인식 엔진입니다.")
}
fn speech_engine(id: &str) -> Result<SpeechEngine, ControlError> {
    serde_json::from_value(json!(id)).map_err(|_| unknown_engine())
}

// Speech settings without paths, descriptions or internal diagnostics.
fn speech_view(progress: &Progress) -> Value {
    let switch = progress.speech_switch.as_ref().map(|switch| {
        let job = progress
            .jobs
            .iter()
            .filter(|job| {
                job.kind == "setup" && job.speech_engine == switch.target && active_job(&job.status)
            })
            .max_by_key(|job| job.created_at)
            .map(|job| job.id.clone());
        json!({"target":switch.target,"preparing":switch.preparing,"jobId":job})
    });
    let choices = progress
        .speech_choices
        .iter()
        .map(|choice| {
            json!({"id":choice.id,"title":choice.label,"supported":choice.supported,"ready":choice.ready,
                "recommended":choice.recommended,"memoryWarning":choice.memory_warning,"modelBytes":choice.model_bytes})
        })
        .collect::<Vec<_>>();
    json!({"engine":progress.speech_engine,"speakerEnabled":progress.speaker_enabled,
        "supported":progress.supported,"ready":progress.model_ready,"choices":choices,"switch":switch})
}

fn engine_outcome(progress: &Progress, engine: SpeechEngine) -> Value {
    let outcome = if progress
        .speech_switch
        .as_ref()
        .is_some_and(|switch| switch.target == engine)
    {
        "downloading"
    } else if progress.speech_engine == Some(engine) && progress.model_ready {
        "active"
    } else if progress.speech_engine == Some(engine) {
        "selected"
    } else {
        "unconfirmed"
    };
    json!({"applied":true,"outcome":outcome,"speech":speech_view(progress)})
}

fn speech_engine_set(
    port: &impl ProcessingPort,
    id: &str,
    acknowledged: bool,
) -> Result<Value, ControlError> {
    let engine = speech_engine(id)?;
    let progress = port.progress();
    let choice = progress
        .speech_choices
        .iter()
        .find(|choice| choice.id == engine)
        .ok_or_else(unknown_engine)?;
    if !choice.supported {
        return Err(ControlError::new(
            "speech_engine_unsupported",
            "이 기기에서 사용할 수 없는 음성 인식 엔진입니다.",
        ));
    }
    port.speech_engine_preflight(engine, acknowledged)
        .map_err(|refusal| refused(refusal, |_| rejected()))?;
    let unchanged = progress.speech_switch.as_ref().map_or(
        progress.speech_engine == Some(engine) && progress.model_ready,
        |switch| switch.target == engine && switch.preparing,
    );
    if unchanged {
        return Ok(engine_outcome(&progress, engine));
    }
    if progress.jobs.iter().any(|job| active_job(&job.status)) {
        return Err(ControlError::new(
            "processing_busy",
            "처리 작업이 끝난 뒤 다시 시도해 주세요.",
        ));
    }
    let progress = port
        .request_speech_engine(engine, acknowledged)
        .map_err(|refusal| refused(refusal, |_| rejected()))?;
    Ok(engine_outcome(&progress, engine))
}

fn speech_switch_cancel(port: &impl ProcessingPort, target: &str) -> Result<Value, ControlError> {
    let target = speech_engine(target)?;
    let switch = port.progress().speech_switch.ok_or_else(|| {
        ControlError::new(
            "speech_switch_not_found",
            "대기 중인 음성 인식 전환이 없습니다.",
        )
    })?;
    if switch.target != target {
        return Err(ControlError::new(
            "edit_conflict",
            "대기 중인 전환 대상이 바뀌었습니다.",
        ));
    }
    let progress = port.cancel_speech_switch().map_err(|_| rejected())?;
    Ok(json!({"applied":true,"speech":speech_view(&progress)}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_request_cannot_be_resumed_by_a_late_ack() {
        let gate = Gate::default();
        let (id, pending) = gate.begin().unwrap();
        assert!(gate.wait_ready(&id, &pending, Duration::ZERO).is_err());
        assert!(gate.ready(&id, None).is_err());
    }
    #[test]
    fn frontend_lock_waits_for_mutation_finish_and_ack_is_consumed_once() {
        let gate = Arc::new(Gate::default());
        let (id, pending) = gate.begin().unwrap();
        let ready_gate = gate.clone();
        let ready_id = id.clone();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            done_tx.send(ready_gate.ready(&ready_id, None)).unwrap();
        });
        gate.wait_ready(&id, &pending, Duration::from_secs(1))
            .unwrap();
        assert!(done_rx.try_recv().is_err());
        assert!(gate.ready(&id, None).is_err());
        gate.finish(&id, &pending);
        assert!(done_rx.recv().unwrap().is_ok());
        thread.join().unwrap();
    }
    use crate::processing::JobView;
    use crate::speech_choice::SpeechChoice;
    use crate::speech_switch::SwitchView;

    #[derive(Default)]
    struct Fake {
        progress: Mutex<Progress>,
        recording: bool,
        models_missing: bool,
        draft_error: Mutex<Option<CoreError>>,
        calls: Mutex<Vec<String>>,
        imports: Mutex<Vec<(PathBuf, Option<DraftAttachmentTarget>)>>,
        engines: Mutex<Vec<SpeechEngine>>,
    }

    impl Fake {
        fn call(&self, name: &str) {
            self.calls.lock().unwrap().push(name.into());
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Fake {
        // The conditions `Processing` checks, in its order.
        fn preflight(&self) -> Result<(), Refusal> {
            if self.recording {
                return Err(Refusal::Recording("synthetic".into()));
            }
            if self.models_missing {
                return Err(Refusal::NotReady("synthetic".into()));
            }
            Ok(())
        }
    }

    impl ProcessingPort for Fake {
        fn progress(&self) -> Progress {
            self.progress.lock().unwrap().clone()
        }
        fn import_preflight(&self) -> Result<(), Refusal> {
            self.preflight()
        }
        fn import(
            &self,
            source: PathBuf,
            target: Option<DraftAttachmentTarget>,
        ) -> Result<String, Refusal> {
            self.preflight()?;
            if target.is_some() {
                self.call("validate");
                if let Some(error) = self.draft_error.lock().unwrap().take() {
                    return Err(Refusal::Meeting(error));
                }
            }
            self.call("import");
            let id = uuid::Uuid::new_v4().to_string();
            self.progress.lock().unwrap().jobs.push(JobView {
                id: id.clone(),
                kind: "import".into(),
                meeting_id: target.as_ref().map(|target| target.meeting_id.clone()),
                status: "queued".into(),
                ..Default::default()
            });
            self.imports.lock().unwrap().push((source, target));
            Ok(id)
        }
        fn job(&self, id: &str) -> Value {
            self.progress.lock().unwrap().jobs.iter().find(|job| job.id == id).map_or(Value::Null, |job| {
                json!({"id":job.id,"kind":job.kind,"meetingId":job.meeting_id,"status":job.status,"editVersion":"sha256:synthetic"})
            })
        }
        fn speech_engine_preflight(
            &self,
            engine: SpeechEngine,
            acknowledged: bool,
        ) -> Result<(), Refusal> {
            if self.recording {
                return Err(Refusal::Recording("synthetic".into()));
            }
            let progress = self.progress.lock().unwrap();
            let warned = progress.speech_choices.iter().any(|choice| {
                choice.id == engine && choice.memory_warning.is_some() && !choice.ready
            });
            if warned && !acknowledged {
                return Err(Refusal::MemoryWarning);
            }
            Ok(())
        }
        fn request_speech_engine(
            &self,
            engine: SpeechEngine,
            acknowledged: bool,
        ) -> Result<Progress, Refusal> {
            self.speech_engine_preflight(engine, acknowledged)?;
            self.engines.lock().unwrap().push(engine);
            let mut progress = self.progress.lock().unwrap();
            let ready = progress
                .speech_choices
                .iter()
                .any(|choice| choice.id == engine && choice.ready);
            if ready {
                progress.speech_engine = Some(engine);
                progress.model_ready = true;
            } else {
                progress.speech_switch = Some(SwitchView {
                    target: engine,
                    preparing: true,
                    error: None,
                });
                progress.jobs.push(JobView {
                    id: "setup-job".into(),
                    kind: "setup".into(),
                    speech_engine: engine,
                    status: "queued".into(),
                    ..Default::default()
                });
            }
            Ok(progress.clone())
        }
        fn cancel_speech_switch(&self) -> Result<Progress, String> {
            self.call("cancel");
            let mut progress = self.progress.lock().unwrap();
            progress.speech_switch = None;
            Ok(progress.clone())
        }
    }

    fn choice(id: SpeechEngine, ready: bool, memory_warning: Option<&str>) -> SpeechChoice {
        SpeechChoice {
            id,
            label: "Synthetic engine",
            description: "Synthetic description",
            model_bytes: 3_000_000_000,
            ready,
            supported: id != SpeechEngine::AppleSpeech,
            recommended: id == SpeechEngine::QwenKo,
            recommendation_reason: Some("Synthetic reason"),
            memory_warning: memory_warning.map(str::to_owned),
        }
    }

    fn ready_fake() -> Fake {
        Fake {
            progress: Mutex::new(Progress {
                speech_engine: Some(SpeechEngine::Nemotron),
                speaker_enabled: true,
                supported: true,
                model_ready: true,
                speech_choices: vec![
                    choice(SpeechEngine::Nemotron, true, None),
                    choice(
                        SpeechEngine::QwenKo,
                        false,
                        Some("Synthetic memory warning"),
                    ),
                    choice(SpeechEngine::WhisperTurbo, false, None),
                    choice(SpeechEngine::AppleSpeech, false, None),
                ],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn run(fake: &Fake, method: &str, params: Value) -> Result<Value, ControlError> {
        let request = ProcessingRequest::parse(method, params).unwrap();
        handle_processing(fake, request, || {
            fake.call("save");
            Ok(())
        })
    }

    fn audio_file(directory: &Path, name: &str) -> String {
        let path = directory.join(name);
        std::fs::write(&path, b"synthetic").unwrap();
        path.to_str().unwrap().into()
    }

    #[test]
    fn imports_accept_only_existing_regular_audio_files() {
        let directory = tempfile::tempdir().unwrap();
        let fake = ready_fake();
        let missing = directory.path().join("missing.wav");
        assert_eq!(
            run(&fake, "meeting_import", json!({"path":missing}))
                .unwrap_err()
                .code,
            "audio_file_unavailable"
        );
        std::fs::create_dir(directory.path().join("folder.wav")).unwrap();
        let folder = directory.path().join("folder.wav");
        assert_eq!(
            run(&fake, "meeting_import", json!({"path":folder}))
                .unwrap_err()
                .code,
            "audio_file_unavailable"
        );
        #[cfg(unix)]
        {
            let target = audio_file(directory.path(), "target.wav");
            let link = directory.path().join("link.wav");
            std::os::unix::fs::symlink(target, &link).unwrap();
            assert_eq!(
                run(&fake, "meeting_import", json!({"path":link}))
                    .unwrap_err()
                    .code,
                "audio_file_unavailable"
            );
        }
        // Core accepts every retained format; the app decides what it can decode.
        let text = audio_file(directory.path(), "notes.txt");
        assert_eq!(
            import_source(&text).unwrap_err().code,
            "unsupported_audio_format"
        );
        assert!(fake.imports.lock().unwrap().is_empty());
    }

    #[test]
    fn imports_require_idle_recording_and_ready_speech_models() {
        let directory = tempfile::tempdir().unwrap();
        let audio = audio_file(directory.path(), "meeting.M4A");
        let recording = Fake {
            recording: true,
            ..ready_fake()
        };
        assert_eq!(
            run(&recording, "meeting_import", json!({"path":audio}))
                .unwrap_err()
                .code,
            "recording_active"
        );
        let unprepared = Fake {
            models_missing: true,
            ..ready_fake()
        };
        assert_eq!(
            run(&unprepared, "meeting_import", json!({"path":audio}))
                .unwrap_err()
                .code,
            "speech_not_ready"
        );
        assert!(recording.imports.lock().unwrap().is_empty());
        assert!(unprepared.imports.lock().unwrap().is_empty());
    }

    #[test]
    fn new_imports_return_their_job_without_saving_open_editors() {
        let directory = tempfile::tempdir().unwrap();
        let audio = audio_file(directory.path(), "meeting.wav");
        let fake = ready_fake();
        fake.progress.lock().unwrap().jobs.push(JobView {
            id: "older-import".into(),
            kind: "import".into(),
            status: "completed".into(),
            ..Default::default()
        });
        let result = run(&fake, "meeting_import", json!({"path":audio})).unwrap();
        assert_eq!(result["applied"], true);
        assert!(result["meetingId"].is_null());
        assert_eq!(result["job"]["kind"], "import");
        assert_eq!(result["job"]["status"], "queued");
        assert_ne!(result["job"]["id"], "older-import");
        assert_eq!(fake.calls(), ["import"]);
        let imports = fake.imports.lock().unwrap();
        assert_eq!(imports[0].0, PathBuf::from(&audio));
        assert!(imports[0].1.is_none());
    }

    #[test]
    fn draft_attachments_save_editors_then_check_the_meeting_version() {
        let directory = tempfile::tempdir().unwrap();
        let audio = audio_file(directory.path(), "meeting.flac");
        let id = uuid::Uuid::new_v4().to_string();
        let version = format!("sha256:{}", "a".repeat(64));
        let params = json!({"path":audio,"meetingId":id,"expectedVersion":version});
        let fake = ready_fake();
        let result = run(&fake, "meeting_import", params.clone()).unwrap();
        assert_eq!(fake.calls(), ["save", "validate", "import"]);
        assert_eq!(result["meetingId"], id);
        assert_eq!(result["job"]["meetingId"], id);
        assert_eq!(
            fake.imports.lock().unwrap()[0].1,
            Some(DraftAttachmentTarget {
                meeting_id: id.clone(),
                expected_version: version.clone()
            })
        );
        for (error, code) in [
            (CoreError::EditConflict, "edit_conflict"),
            (CoreError::MeetingNotFound, "meeting_not_found"),
            (CoreError::InvalidState("synthetic".into()), "invalid_state"),
            (
                CoreError::InvalidInput("synthetic".into()),
                "meeting_not_found",
            ),
        ] {
            let fake = ready_fake();
            *fake.draft_error.lock().unwrap() = Some(error);
            assert_eq!(
                run(&fake, "meeting_import", params.clone())
                    .unwrap_err()
                    .code,
                code
            );
            assert!(fake.imports.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn speech_status_exposes_choices_and_the_waiting_switch_without_internal_fields() {
        let fake = ready_fake();
        {
            let mut progress = fake.progress.lock().unwrap();
            progress.speech_switch = Some(SwitchView {
                target: SpeechEngine::QwenKo,
                preparing: true,
                error: None,
            });
            progress.jobs.push(JobView {
                id: "download".into(),
                kind: "setup".into(),
                speech_engine: SpeechEngine::QwenKo,
                status: "running".into(),
                ..Default::default()
            });
        }
        let view = run(&fake, "speech_status", json!({})).unwrap();
        assert_eq!(view["engine"], "nemotron");
        assert_eq!(view["speakerEnabled"], true);
        assert_eq!(view["ready"], true);
        assert_eq!(
            view["switch"],
            json!({"target":"qwenKo","preparing":true,"jobId":"download"})
        );
        assert_eq!(
            view["choices"][1],
            json!({"id":"qwenKo","title":"Synthetic engine","supported":true,"ready":false,
                "recommended":true,"memoryWarning":"Synthetic memory warning","modelBytes":3_000_000_000u64})
        );
        assert!(!view.to_string().contains("Synthetic description"));
        assert!(!view.to_string().contains("Synthetic reason"));
    }

    #[test]
    fn engine_selection_checks_support_memory_warning_and_idle_state_first() {
        let fake = ready_fake();
        for (params, code) in [
            (json!({"engine":"unknownEngine"}), "speech_engine_unknown"),
            (json!({"engine":"appleSpeech"}), "speech_engine_unsupported"),
            (json!({"engine":"qwenKo"}), "memory_warning_unacknowledged"),
        ] {
            assert_eq!(
                run(&fake, "speech_engine_set", params).unwrap_err().code,
                code
            );
        }
        let recording = Fake {
            recording: true,
            ..ready_fake()
        };
        assert_eq!(
            run(
                &recording,
                "speech_engine_set",
                json!({"engine":"whisperTurbo"})
            )
            .unwrap_err()
            .code,
            "recording_active"
        );
        fake.progress.lock().unwrap().jobs.push(JobView {
            id: "busy".into(),
            kind: "transcribe".into(),
            status: "running".into(),
            ..Default::default()
        });
        assert_eq!(
            run(&fake, "speech_engine_set", json!({"engine":"whisperTurbo"}))
                .unwrap_err()
                .code,
            "processing_busy"
        );
        assert!(fake.engines.lock().unwrap().is_empty());
        assert!(recording.engines.lock().unwrap().is_empty());
    }

    #[test]
    fn engine_selection_reports_downloads_and_does_not_repeat_a_waiting_switch() {
        let fake = ready_fake();
        let result = run(
            &fake,
            "speech_engine_set",
            json!({"engine":"qwenKo","acknowledgeMemoryWarning":true}),
        )
        .unwrap();
        assert_eq!(result["outcome"], "downloading");
        assert_eq!(result["speech"]["engine"], "nemotron");
        assert_eq!(result["speech"]["switch"]["jobId"], "setup-job");
        let again = run(
            &fake,
            "speech_engine_set",
            json!({"engine":"qwenKo","acknowledgeMemoryWarning":true}),
        )
        .unwrap();
        assert_eq!(again["outcome"], "downloading");
        assert_eq!(*fake.engines.lock().unwrap(), [SpeechEngine::QwenKo]);
        let active = ready_fake();
        assert_eq!(
            run(&active, "speech_engine_set", json!({"engine":"nemotron"})).unwrap()["outcome"],
            "active"
        );
        assert!(active.engines.lock().unwrap().is_empty());
    }

    #[test]
    fn switch_cancellation_requires_the_waiting_target() {
        let fake = ready_fake();
        assert_eq!(
            run(&fake, "speech_switch_cancel", json!({"target":"qwenKo"}))
                .unwrap_err()
                .code,
            "speech_switch_not_found"
        );
        fake.progress.lock().unwrap().speech_switch = Some(SwitchView {
            target: SpeechEngine::QwenKo,
            preparing: true,
            error: None,
        });
        assert_eq!(
            run(
                &fake,
                "speech_switch_cancel",
                json!({"target":"whisperTurbo"})
            )
            .unwrap_err()
            .code,
            "edit_conflict"
        );
        assert!(fake.calls().is_empty());
        let result = run(&fake, "speech_switch_cancel", json!({"target":"qwenKo"})).unwrap();
        assert_eq!(result["applied"], true);
        assert!(result["speech"]["switch"].is_null());
        assert_eq!(fake.calls(), ["cancel"]);
    }

    #[test]
    fn close_releases_frontend_without_authorizing_a_pending_write() {
        let gate = Gate::default();
        let (id, pending) = gate.begin().unwrap();
        gate.close();
        assert!(gate
            .wait_ready(&id, &pending, Duration::from_secs(1))
            .is_err());
        assert!(gate.ready(&id, None).is_err());
        assert!(gate.begin().is_err());
    }
}
