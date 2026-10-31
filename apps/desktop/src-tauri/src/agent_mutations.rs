use crate::{processing::Processing, publish};
use nero_core::agent_control::{ControlError, JobRequest, Mutation, Server};
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

struct Lease<'a> {
    gate: &'a Gate,
    id: String,
    pending: Arc<Pending>,
}
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        self.gate.finish(&self.id, &self.pending);
    }
}

pub struct AgentMutations {
    gate: Gate,
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
            gate: Gate::default(),
            server: Mutex::new(None),
        });
        let weak = Arc::downgrade(&state);
        let worker_core = core.clone();
        let job_state = weak.clone();
        let job_processing = processing.clone();
        let metadata_app = app.clone();
        let server = Server::start_with_metadata(
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
        let (id, pending) = self.gate.begin()?;
        let lease = Lease {
            gate: &self.gate,
            id: id.clone(),
            pending: pending.clone(),
        };
        if app
            .emit_to(
                "main",
                "nero://agent-write-requested",
                serde_json::json!({"requestId":id}),
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
        let result = if let Some(ids) = request.trash_ids() {
            let preflight = request.clone();
            processing.meeting_removal_with_preflight(
                core,
                &ids,
                || {
                    if self.gate.closing.load(Ordering::Acquire) {
                        return Err(nero_core::CoreError::Unavailable);
                    }
                    core.agent_validate(preflight)
                },
                || {
                    if self.gate.closing.load(Ordering::Acquire) {
                        return Err(nero_core::CoreError::Unavailable);
                    }
                    core.agent_mutate(request)
                },
            )
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
