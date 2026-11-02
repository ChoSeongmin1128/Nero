use crate::{processing::Processing, publish};
use nero_core::agent_control::{
    AppUpdateAction, ControlError, ControlReply, JobRequest, Mutation, Server,
};
use nero_core::CoreHandle;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
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
        let server = Server::start_with_updates(
            root,
            core,
            move |request| {
                let state = weak.upgrade().ok_or_else(ControlError::unavailable)?;
                state.execute(&app, &worker_core, &processing, request)
            },
            move |request| {
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
            },
            move || {
                Ok(metadata_app
                    .state::<crate::app_updater::AppUpdates>()
                    .agent_metadata(metadata_app.path().resource_dir().ok().as_deref()))
            },
            move |action| {
                update_state
                    .upgrade()
                    .ok_or_else(ControlError::unavailable)?
                    .execute_update(&update_app, action)
            },
        );
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
            crate::ai_chat_desktop::meeting_removal(app, &ids, false, || {
                processing.meeting_trash(core, &ids, || {
                    if self.gate.closing.load(Ordering::Acquire) {
                        return Err(nero_core::CoreError::Unavailable);
                    }
                    core.agent_mutate(request)
                })
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
