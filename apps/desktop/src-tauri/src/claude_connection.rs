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
    browser_url: Mutex<Option<String>>,
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
pub struct ShutdownGuard {
    connection: Arc<ClaudeConnection>,
}
impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        self.connection.refresh.lock().unwrap().stopping = false;
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
            browser_url: Mutex::new(None),
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
                let browser = run.clone();
                let connection = this.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let _operation = lock_operation(&connection.model_operation, &cancel)?;
                    connection.invalidate_models()?;
                    if cancel.load(Ordering::Acquire) {
                        return Err("취소했습니다.".into());
                    }
                    if !runner.auth_status(&cancel)? {
                        runner.login(&cancel, |url| {
                            *browser.browser_url.lock().unwrap() = Some(url);
                            browser.done.notify_waiters();
                        })?;
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
    pub async fn reopen_url(&self, id: &str) -> Result<String, String> {
        let run = self
            .login
            .lock()
            .unwrap()
            .as_ref()
            .filter(|run| run.id == id)
            .cloned()
            .ok_or("진행 중인 로그인 화면이 없습니다.")?;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let notified = run.done.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if run.cancel.load(Ordering::Acquire) || run.result.lock().unwrap().is_some() {
                    return Err("진행 중인 로그인 화면이 없습니다.".into());
                }
                if let Some(url) = run.browser_url.lock().unwrap().clone() {
                    return Ok(url);
                }
                notified.await;
            }
        })
        .await
        .map_err(|_| {
            "로그인 주소를 아직 확인하지 못했습니다. 잠시 후 다시 시도해 주세요.".to_string()
        })?
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
    pub async fn prepare_shutdown(self: &Arc<Self>) -> Result<ShutdownGuard, String> {
        let refresh = {
            let mut state = self.refresh.lock().unwrap();
            if state.stopping {
                return Err("Claude 연결을 이미 정리 중입니다.".into());
            }
            state.stopping = true;
            let active = state.active.clone();
            if let Some(run) = &active {
                run.cancel.store(true, Ordering::Release);
            }
            active
        };
        let guard = ShutdownGuard {
            connection: self.clone(),
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
        Ok(guard)
    }
    fn invalidate_models(&self) -> Result<(), String> {
        self.model_cache.write(&[], None)?;
        self.models.lock().unwrap().clear();
        Ok(())
    }
    pub fn models(&self) -> Vec<claude_runner::SupportedModel> {
        self.models.lock().unwrap().clone()
    }
    pub fn check_auth(&self) -> Result<(), String> {
        if claude_runner::installation_state(&self.root) == "missing" {
            return Ok(());
        }
        self.check_auth_with(|cancel| {
            let runner =
                claude_runner::discover_prepared_cancelled(&self.root, cancel).or_else(|_| {
                    if cancel.load(Ordering::Acquire) {
                        Err("취소했습니다.".into())
                    } else {
                        claude_runner::discover()
                    }
                })?;
            runner.auth_status(cancel)
        })
    }
    fn check_auth_with(
        &self,
        check: impl FnOnce(&AtomicBool) -> Result<bool, String>,
    ) -> Result<(), String> {
        let run = Arc::new(Refresh {
            cancel: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            done: Notify::new(),
        });
        {
            let mut state = self.refresh.lock().unwrap();
            if state.stopping {
                return Err("Claude 연결을 종료하고 있습니다.".into());
            }
            if state
                .active
                .as_ref()
                .is_some_and(|active| !active.completed.load(Ordering::Acquire))
                || self
                    .login
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|login| login.result.lock().unwrap().is_none())
            {
                return Ok(());
            }
            state.active = Some(run.clone());
        }
        let _guard = RefreshGuard(run.clone());
        let _operation = lock_operation(&self.model_operation, &run.cancel)?;
        let ready = check(&run.cancel)?;
        if run.cancel.load(Ordering::Acquire) {
            return Err("취소했습니다.".into());
        }
        *self.cached.lock().unwrap() = Some(Self::row(ready));
        Ok(())
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
        self.reload_models(&runner, &identity, &run.cancel)
    }
    // Reads the model list from this exact executable and caches it under its identity.
    // The caller holds `model_operation`.
    fn reload_models(
        &self,
        runner: &claude_runner::Runner,
        identity: &str,
        cancel: &AtomicBool,
    ) -> Result<Vec<claude_runner::SupportedModel>, String> {
        let models = runner.supported_models(cancel)?;
        if runner.identity(cancel)? != identity {
            return Err("Claude 실행 파일이 변경되어 모델 목록을 저장하지 못했습니다.".into());
        }
        if cancel.load(Ordering::Acquire) {
            return Err("취소했습니다.".into());
        }
        self.model_cache.write(&models, Some(identity))?;
        *self.models.lock().unwrap() = models.clone();
        Ok(models)
    }
    pub fn reply(
        &self,
        model: &str,
        prompt: &str,
        effort: Option<&str>,
        cancel: Arc<AtomicBool>,
        delta: impl FnMut(&str),
    ) -> Result<claude_runner::Reply, String> {
        let operation = lock_operation(&self.model_operation, &cancel)?;
        let runner = self.resolve()?;
        let identity = runner.identity(&cancel)?;
        // Claude Code updates itself; a new executable reads its own model list here
        // instead of failing every request until the app restarts.
        let models = match self.model_cache.read_for_runtime(&identity) {
            Ok(models) => models,
            Err(_) => self.reload_models(&runner, &identity, &cancel)?,
        };
        if !models.iter().any(|entry| {
            entry.value == model
                && effort.map_or(true, |value| {
                    entry.supports_effort == Some(true)
                        && entry
                            .supported_effort_levels
                            .iter()
                            .any(|level| level == value)
                })
        }) {
            return Err(
                "선택한 Claude 모델을 확인하지 못했습니다. 모델 목록을 새로고침해 주세요.".into(),
            );
        }
        drop(operation);
        let result = runner.reply(model, prompt, effort, cancel, delta);
        // An expired login is known to be signed out, so the screen asks to log in
        // again; other failures say nothing about the login.
        *self.cached.lock().unwrap() = match &result {
            Ok(_) => Some(Self::row(true)),
            Err(message) if message == claude_runner::LOGIN_EXPIRED => Some(Self::row(false)),
            Err(_) => None,
        };
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
            browser_url: Mutex::new(None),
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
            supports_effort: None,
            supported_effort_levels: vec![],
        };
        cache.write(&[model], Some(&"a".repeat(64))).unwrap();
        let connection = ClaudeConnection::new(root.clone());
        assert_eq!(connection.models().len(), 1);
        connection.invalidate_models().unwrap();
        assert!(connection.models().is_empty());
        assert!(ClaudeConnection::new(root).models().is_empty());
    }

    #[tokio::test]
    async fn shutdown_guard_cancels_refresh_and_reopens_on_drop() {
        let connection = Arc::new(ClaudeConnection::new(PathBuf::from("unused")));
        let run = Arc::new(Refresh {
            cancel: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            done: Notify::new(),
        });
        connection.refresh.lock().unwrap().active = Some(run.clone());
        let pending = connection.clone();
        let task = tokio::spawn(async move { pending.prepare_shutdown().await });
        tokio::task::yield_now().await;
        assert!(run.cancel.load(Ordering::Acquire));
        assert!(!task.is_finished());
        drop(RefreshGuard(run));
        let guard = task.await.unwrap().unwrap();
        assert!(connection.refresh.lock().unwrap().stopping);
        assert!(connection.refresh_models().is_err());
        drop(guard);
        assert!(!connection.refresh.lock().unwrap().stopping);
    }

    #[tokio::test]
    async fn interrupted_shutdown_releases_guard_while_refresh_finishes() {
        let connection = Arc::new(ClaudeConnection::new(PathBuf::from("unused")));
        let run = Arc::new(Refresh {
            cancel: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            done: Notify::new(),
        });
        connection.refresh.lock().unwrap().active = Some(run.clone());
        let pending = connection.clone();
        let task = tokio::spawn(async move { pending.prepare_shutdown().await });
        tokio::task::yield_now().await;
        assert!(connection.refresh.lock().unwrap().stopping);
        task.abort();
        let _ = task.await;
        assert!(!connection.refresh.lock().unwrap().stopping);
        assert!(connection.refresh_models().is_err());
        drop(RefreshGuard(run));
    }

    #[test]
    fn model_cache_is_passive_and_rejects_unadvertised_selection() {
        let connection = ClaudeConnection::new(PathBuf::from("unused"));
        assert!(connection.models().is_empty());
        let result = connection.reply(
            "claude-unknown",
            "prompt",
            None,
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

    #[test]
    fn auth_check_preserves_state_and_models_on_failure() {
        let connection = ClaudeConnection::new(PathBuf::from("unused"));
        *connection.cached.lock().unwrap() = Some(ClaudeConnection::row(true));
        let before = connection.cached.lock().unwrap().clone();
        assert!(connection
            .check_auth_with(|_| Err("temporary failure".into()))
            .is_err());
        assert_eq!(*connection.cached.lock().unwrap(), before);
        assert!(connection.models().is_empty());
        connection.check_auth_with(|_| Ok(false)).unwrap();
        assert_eq!(
            connection.cached.lock().unwrap().as_ref().unwrap()["state"],
            "signed_out"
        );
        connection.check_auth_with(|_| Ok(true)).unwrap();
        assert_eq!(
            connection.cached.lock().unwrap().as_ref().unwrap()["state"],
            "ready"
        );
    }

    #[test]
    fn auth_check_skips_running_login_or_refresh_and_rejects_shutdown() {
        let connection = Arc::new(ClaudeConnection::new(PathBuf::from("unused")));
        let run = attempt(&connection);
        connection
            .check_auth_with(|_| panic!("login already running"))
            .unwrap();
        *run.result.lock().unwrap() = Some(Ok(()));
        let refresh = Arc::new(Refresh {
            cancel: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            done: Notify::new(),
        });
        connection.refresh.lock().unwrap().active = Some(refresh);
        connection
            .check_auth_with(|_| panic!("refresh already running"))
            .unwrap();
        connection.refresh.lock().unwrap().stopping = true;
        assert!(connection
            .check_auth_with(|_| panic!("shutdown in progress"))
            .is_err());
    }

    #[tokio::test]
    async fn shutdown_cancels_auth_check_without_publishing_its_result() {
        let connection = Arc::new(ClaudeConnection::new(PathBuf::from("unused")));
        let checking = connection.clone();
        let (entered, waiting) = tokio::sync::oneshot::channel();
        let task = tokio::task::spawn_blocking(move || {
            checking.check_auth_with(|cancel| {
                entered.send(()).unwrap();
                while !cancel.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(true)
            })
        });
        waiting.await.unwrap();
        let guard = connection.prepare_shutdown().await.unwrap();
        assert!(task.await.unwrap().is_err());
        assert!(connection.cached.lock().unwrap().is_none());
        drop(guard);
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
    #[tokio::test]
    async fn reopening_uses_the_same_live_attempt_and_does_not_complete_login() {
        let connection = Arc::new(ClaudeConnection::new(PathBuf::from("unused")));
        let run = attempt(&connection);
        let waiting = connection.clone();
        let finish = tokio::spawn(async move { waiting.finish("fixture").await });
        let reopening = connection.clone();
        let task = tokio::spawn(async move { reopening.reopen_url("fixture").await });
        tokio::task::yield_now().await;
        let url = "https://claude.ai/oauth/authorize?state=fixture&code_challenge=fixture";
        *run.browser_url.lock().unwrap() = Some(url.into());
        run.done.notify_waiters();
        assert_eq!(task.await.unwrap().unwrap(), url);
        assert_eq!(connection.reopen_url("fixture").await.unwrap(), url);
        assert!(!finish.is_finished());
        assert!(connection.reopen_url("stale").await.is_err());
        run.cancel.store(true, Ordering::Release);
        assert!(connection.reopen_url("fixture").await.is_err());
        run.cancel.store(false, Ordering::Release);
        *run.result.lock().unwrap() = Some(Ok(()));
        run.done.notify_waiters();
        assert!(finish.await.unwrap().is_ok());
        assert!(connection.reopen_url("fixture").await.is_err());
    }
}
