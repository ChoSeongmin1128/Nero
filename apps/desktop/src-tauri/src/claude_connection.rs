use crate::claude_runner::{self, Runner};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::Notify;
pub const ACCOUNT_ID: &str = "claude-code";
struct Login {
    id: String,
    cancel: Arc<AtomicBool>,
    result: Mutex<Option<Result<(), String>>>,
    done: Notify,
}
struct Refresh {
    cancel: AtomicBool,
    completed: AtomicBool,
    done: Notify,
}
struct RefreshGuard(Arc<Refresh>);
impl Drop for RefreshGuard {
    fn drop(&mut self) {
        self.0.completed.store(true, Ordering::Release);
        self.0.done.notify_waiters();
    }
}
#[derive(Default)]
struct RefreshState {
    stopping: bool,
    active: Option<Arc<Refresh>>,
}
fn lock_operation<'a>(
    lock: &'a Mutex<()>,
    cancel: &AtomicBool,
) -> Result<std::sync::MutexGuard<'a, ()>, String> {
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("취소했습니다.".into());
        }
        match lock.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err("Claude 연결을 준비하지 못했습니다.".into())
            }
        }
    }
}
pub struct ClaudeConnection {
    root: PathBuf,
    cached: Mutex<Option<Value>>,
    models: Mutex<Vec<claude_runner::SupportedModel>>,
    model_cache: claude_runner::ModelCache,
    model_operation: Mutex<()>,
    refresh: Mutex<RefreshState>,
    login: Mutex<Option<Arc<Login>>>,
}
impl ClaudeConnection {
    pub fn new(root: PathBuf) -> Self {
        let model_cache = claude_runner::ModelCache::new(&root);
        let models = model_cache.read();
        Self {
            root: root.join("ai-runtime/claude-code"),
            cached: Mutex::new(None),
            models: Mutex::new(models),
            model_cache,
            model_operation: Mutex::new(()),
            refresh: Mutex::new(RefreshState::default()),
            login: Mutex::new(None),
        }
    }
    fn resolve(&self) -> Result<Runner, String> {
        claude_runner::discover_prepared(&self.root).or_else(|_| claude_runner::discover())
    }
    fn row(ready: bool) -> Value {
        json!({"id":ACCOUNT_ID,"provider":"claude","state":if ready {"ready"} else {"signed_out"},"configured":true,"disconnectable":false})
    }
    pub fn snapshot(&self) -> Value {
        let runtime_state = claude_runner::installation_state(&self.root);
        if self
            .login
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|run| run.result.lock().unwrap().is_none())
        {
            return json!({"id":ACCOUNT_ID,"provider":"claude","state":"connecting","runtimeState":runtime_state,"configured":true,"disconnectable":false});
        }
        let mut row = self.cached.lock().unwrap().clone().unwrap_or_else(|| {
            json!({"id":ACCOUNT_ID,"provider":"claude","state":"unknown","configured":true,"disconnectable":false})
        });
        row["runtimeState"] = json!(runtime_state);
        if runtime_state == "missing" && row["state"] == "ready" {
            row["state"] = json!("unknown");
        }
        row
    }

    pub fn begin(self: &Arc<Self>) -> Result<String, String> {
        let refresh_state = self.refresh.lock().unwrap();
        if refresh_state.stopping {
            return Err("Claude 연결을 종료하고 있습니다.".into());
        }
        let mut pending = self.login.lock().unwrap();
        if pending
            .as_ref()
            .is_some_and(|run| run.result.lock().unwrap().is_none())
        {
            return Err("Claude 로그인을 진행 중입니다.".into());
        }
        let run = Arc::new(Login {
            id: uuid::Uuid::new_v4().to_string(),
            cancel: Arc::new(AtomicBool::new(false)),
            result: Mutex::new(None),
            done: Notify::new(),
        });
        *pending = Some(run.clone());
        drop(refresh_state);
        let this = self.clone();
        let id = run.id.clone();
        tauri::async_runtime::spawn(async move {
            let result = async {
                let runner = match this.resolve() {
                    Ok(runner) => runner,
                    Err(_) => claude_runner::prepare(&this.root, run.cancel.clone()).await?,
                };
                let cancel = run.cancel.clone();
                let connection = this.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let _operation = lock_operation(&connection.model_operation, &cancel)?;
                    connection.invalidate_models()?;
                    if cancel.load(Ordering::Acquire) {
                        return Err("취소했습니다.".into());
                    }
                    if !runner.auth_status(&cancel)? {
                        runner.login(&cancel)?;
                    }
                    Ok(())
                })
                .await
                .map_err(|_| "Claude 로그인을 완료하지 못했습니다.".to_string())?
            }
            .await;
            let accepted = if run.cancel.load(Ordering::Acquire) {
                Err("취소했습니다.".into())
            } else {
                result
            };
            if accepted.is_ok() {
                *this.cached.lock().unwrap() = Some(Self::row(true));
            }
            *run.result.lock().unwrap() = Some(accepted);
            run.done.notify_waiters();
        });
        Ok(id)
    }
    pub fn owns(&self, id: &str) -> bool {
        self.login
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|run| run.id == id)
    }
    pub async fn finish(&self, id: &str) -> Result<(), String> {
        let run = self
            .login
            .lock()
            .unwrap()
            .clone()
            .filter(|run| run.id == id)
            .ok_or("로그인 요청을 찾을 수 없습니다.")?;
        Self::wait_for(&run).await
    }
    async fn wait_for(run: &Login) -> Result<(), String> {
        loop {
            let notified = run.done.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(result) = run.result.lock().unwrap().clone() {
                return result;
            }
            notified.await;
        }
    }
    pub async fn cancel(&self, id: &str) -> Result<(), String> {
        let run = self
            .login
            .lock()
            .unwrap()
            .clone()
            .filter(|run| run.id == id)
            .ok_or("로그인 요청을 찾을 수 없습니다.")?;
        run.cancel.store(true, Ordering::Release);
        tokio::time::timeout(Duration::from_secs(5), Self::wait_for(&run))
            .await
            .map_err(|_| "로그인 취소를 정리 중입니다. 잠시 후 다시 시도해 주세요.".to_string())?
            .ok();
        Ok(())
    }
    async fn wait_refresh(run: &Refresh) {
        loop {
            let notified = run.done.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if run.completed.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
    pub async fn drain(&self) -> Result<(), String> {
        let refresh = {
            let mut state = self.refresh.lock().unwrap();
            state.stopping = true;
            let active = state.active.clone();
            if let Some(run) = &active {
                run.cancel.store(true, Ordering::Release);
            }
            active
        };
        let login = self.login.lock().unwrap().clone();
        if let Some(run) = &login {
            run.cancel.store(true, Ordering::Release);
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            if let Some(run) = login {
                let _ = Self::wait_for(&run).await;
            }
            if let Some(run) = refresh {
                Self::wait_refresh(&run).await;
            }
        })
        .await
        .map_err(|_| "Claude 연결을 정리 중입니다. 잠시 후 다시 시도해 주세요.".to_string())?;
        Ok(())
    }
    fn invalidate_models(&self) -> Result<(), String> {
        self.model_cache.write(&[], None)?;
        self.models.lock().unwrap().clear();
        Ok(())
    }
    pub fn models(&self) -> Vec<claude_runner::SupportedModel> {
        self.models.lock().unwrap().clone()
    }
    pub fn refresh_models(&self) -> Result<Vec<claude_runner::SupportedModel>, String> {
        let run = Arc::new(Refresh {
            cancel: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            done: Notify::new(),
        });
        {
            let mut state = self.refresh.lock().unwrap();
            if state.stopping
                || state
                    .active
                    .as_ref()
                    .is_some_and(|active| !active.completed.load(Ordering::Acquire))
            {
                return Err("Claude 모델 목록을 이미 갱신 중이거나 종료하고 있습니다.".into());
            }
            state.active = Some(run.clone());
        }
        let _guard = RefreshGuard(run.clone());
        let _operation = lock_operation(&self.model_operation, &run.cancel)?;
        let runner =
            claude_runner::discover_prepared_cancelled(&self.root, &run.cancel).or_else(|_| {
                if run.cancel.load(Ordering::Acquire) {
                    Err("취소했습니다.".into())
                } else {
                    claude_runner::discover()
                }
            })?;
        let identity = runner.identity(&run.cancel)?;
        let models = runner.supported_models(&run.cancel)?;
        if runner.identity(&run.cancel)? != identity {
            return Err("Claude 실행 파일이 변경되어 모델 목록을 저장하지 못했습니다.".into());
        }
        if run.cancel.load(Ordering::Acquire) {
            return Err("취소했습니다.".into());
        }
        self.model_cache.write(&models, Some(&identity))?;
        *self.models.lock().unwrap() = models.clone();
        Ok(models)
    }
    pub fn reply(
        &self,
        model: &str,
        prompt: &str,
        cancel: Arc<AtomicBool>,
        delta: impl FnMut(&str),
    ) -> Result<claude_runner::Reply, String> {
        if !self
            .models
            .lock()
            .unwrap()
            .iter()
            .any(|entry| entry.value == model)
        {
            return Err(
                "이 Claude 계정에서 선택한 모델을 사용할 수 없습니다. 연결을 다시 확인해 주세요."
                    .into(),
            );
        }
        let result = self.resolve()?.reply(model, prompt, cancel, delta);
        if result.is_ok() {
            *self.cached.lock().unwrap() = Some(Self::row(true));
        } else {
            *self.cached.lock().unwrap() = None;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(connection: &ClaudeConnection) -> Arc<Login> {
        let run = Arc::new(Login {
            id: "fixture".into(),
            cancel: Arc::new(AtomicBool::new(false)),
            result: Mutex::new(None),
            done: Notify::new(),
        });
        *connection.login.lock().unwrap() = Some(run.clone());
        run
    }

    #[test]
    fn operation_wait_can_be_cancelled_while_refresh_holds_lock() {
        let connection = Arc::new(ClaudeConnection::new(PathBuf::from("unused")));
        let held = connection.model_operation.lock().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_connection = connection.clone();
        let worker_cancel = cancel.clone();
        let worker = std::thread::spawn(move || {
            lock_operation(&worker_connection.model_operation, &worker_cancel).is_err()
        });
        cancel.store(true, Ordering::Release);
        assert!(worker.join().unwrap());
        drop(held);
    }

    #[test]
    fn connection_invalidation_persists_without_account_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let cache = claude_runner::ModelCache::new(&root);
        let model = claude_runner::SupportedModel {
            value: "sonnet".into(),
            display_name: "Sonnet".into(),
            resolved_model: None,
        };
        cache.write(&[model], Some(&"a".repeat(64))).unwrap();
        let connection = ClaudeConnection::new(root.clone());
        assert_eq!(connection.models().len(), 1);
        connection.invalidate_models().unwrap();
        assert!(connection.models().is_empty());
        assert!(ClaudeConnection::new(root).models().is_empty());
    }

    #[tokio::test]
    async fn drain_cancels_and_waits_for_active_refresh() {
        let connection = Arc::new(ClaudeConnection::new(PathBuf::from("unused")));
        let run = Arc::new(Refresh {
            cancel: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            done: Notify::new(),
        });
        connection.refresh.lock().unwrap().active = Some(run.clone());
        let pending = connection.clone();
        let task = tokio::spawn(async move { pending.drain().await });
        tokio::task::yield_now().await;
        assert!(run.cancel.load(Ordering::Acquire));
        assert!(!task.is_finished());
        drop(RefreshGuard(run));
        assert!(task.await.unwrap().is_ok());
        assert!(connection.refresh_models().is_err());
    }

    #[test]
    fn model_cache_is_passive_and_rejects_unadvertised_selection() {
        let connection = ClaudeConnection::new(PathBuf::from("unused"));
        assert!(connection.models().is_empty());
        let result = connection.reply(
            "claude-unknown",
            "prompt",
            Arc::new(AtomicBool::new(false)),
            |_| {},
        );
        assert!(result.is_err());
        assert!(connection.models().is_empty());
    }

    #[test]
    fn passive_snapshot_does_not_resolve_or_run_claude() {
        let connection = ClaudeConnection::new(PathBuf::from("unused"));
        for _ in 0..10 {
            assert_eq!(connection.snapshot()["state"], "unknown");
        }
    }

    #[tokio::test]
    async fn cancel_waits_for_login_to_stop_before_acknowledging() {
        let connection = Arc::new(ClaudeConnection::new(PathBuf::from("unused")));
        let run = attempt(&connection);
        let waiting = connection.clone();
        let task = tokio::spawn(async move { waiting.cancel("fixture").await });
        tokio::task::yield_now().await;
        assert!(run.cancel.load(Ordering::Acquire));
        assert!(!task.is_finished());
        *connection.cached.lock().unwrap() = Some(ClaudeConnection::row(false));
        *run.result.lock().unwrap() = Some(Err("취소했습니다.".into()));
        run.done.notify_waiters();
        assert!(task.await.unwrap().is_ok());
        assert_eq!(connection.snapshot()["state"], "signed_out");
    }

    #[tokio::test]
    async fn finish_observes_completion_before_and_during_wait() {
        let connection = Arc::new(ClaudeConnection::new(PathBuf::from("unused")));
        let first = attempt(&connection);
        *first.result.lock().unwrap() = Some(Ok(()));
        first.done.notify_waiters();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), connection.finish("fixture"))
                .await
                .unwrap()
                .is_ok()
        );

        let second = attempt(&connection);
        let waiting = connection.clone();
        let task = tokio::spawn(async move { waiting.finish("fixture").await });
        tokio::task::yield_now().await;
        *second.result.lock().unwrap() = Some(Err("fixture failure".into()));
        second.done.notify_waiters();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err(),
            "fixture failure"
        );
    }
}
