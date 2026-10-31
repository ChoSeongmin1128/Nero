mod policy;

use policy::ReleaseIdentity;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Mutex},
    time::Duration,
};
use tauri::{Emitter, Manager, WebviewWindow};
use tauri_plugin_updater::{Update, UpdaterExt};

pub const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEVBNzRBQ0ZEREVGOUNFQzAKUldUQXp2bmUvYXgwNnV6Wm96TDMxaDJvTlhoU2loZnZETmxhMGZ0dHQxOFh3a3NzdTFRUE9ROUgK";
const MAX_PACKAGE: u64 = 512 * 1024 * 1024;
const MAX_METADATA: u64 = 256 * 1024;

/// Release tooling calls the same verifier before producing a public feed.
/// This entry point opens no library, window, model or network connection.
pub fn verify_release_file(package: &Path, signature: &Path, wire: &str) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let identity = ReleaseIdentity::parse_wire(wire)?;
    let bytes = read_limited(package, MAX_PACKAGE)?;
    let signature = String::from_utf8(read_limited(signature, 16 * 1024)?)
        .map_err(|_| "서명 형식이 올바르지 않습니다.")?;
    policy::verify_package(&bytes, PUBLIC_KEY, &signature, &identity)?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    phase: String,
    current_version: String,
    // Strings preserve the exact integer across the JavaScript bridge.
    current_build: String,
    available_version: Option<String>,
    available_build: Option<String>,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    download_ready: bool,
    message: String,
    recovery_message: Option<String>,
    recovery_required: bool,
    previous_app_path: Option<String>,
    recovery_receipt_saved: bool,
}

pub struct AppUpdates {
    current: ReleaseIdentity,
    cache: PathBuf,
    status: Mutex<Status>,
    candidate: Mutex<Option<Update>>,
    operation: tokio::sync::Mutex<()>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PendingInstall {
    from: ReleaseIdentity,
    target: ReleaseIdentity,
}

impl PendingInstall {
    fn message(&self, current: &ReleaseIdentity) -> Option<String> {
        // A later compatible app supersedes this installation attempt. Keep the
        // journal for diagnosis; it must not become a permanent update warning.
        if *current == self.target || current.newer_than(&self.target) {
            None
        } else if *current == self.from {
            Some("이전 업데이트가 완료되지 않았습니다. 현재 앱과 회의 데이터는 유지됩니다. 다시 시도할 수 있습니다.".into())
        } else {
            Some("이전 업데이트 기록과 현재 앱 버전이 다릅니다. 현재 앱을 유지합니다.".into())
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoveryRecord {
    previous_app_path: String,
}

enum InstallFailure {
    Retryable(String),
    PreviousAppNotRestored(PathBuf),
}
impl From<tauri_plugin_updater::Error> for InstallFailure {
    fn from(error: tauri_plugin_updater::Error) -> Self {
        match error {
            tauri_plugin_updater::Error::PreviousAppNotRestored(path) => {
                Self::PreviousAppNotRestored(path)
            }
            _ => Self::Retryable(
                "업데이트 설치를 완료하지 못했습니다. 다시 시도할 수 있습니다.".into(),
            ),
        }
    }
}

enum FailedInstall {
    Retryable(String),
    RecoveryRequired {
        record: RecoveryRecord,
        receipt_saved: bool,
    },
}

fn finish_failed_install(
    mut fence: IdleFence,
    failure: InstallFailure,
    cache: &Path,
) -> FailedInstall {
    match failure {
        InstallFailure::Retryable(message) => {
            let message = fence.rollback().err().unwrap_or(message);
            FailedInstall::Retryable(message)
        }
        InstallFailure::PreviousAppNotRestored(path) => {
            let record = RecoveryRecord {
                previous_app_path: path.to_string_lossy().into_owned(),
            };
            let receipt_saved = serde_json::to_vec(&record)
                .ok()
                .is_some_and(|bytes| write_atomic(&cache.join("recovery.json"), &bytes).is_ok());
            // The install path is NOT known to contain a usable app. Keep all
            // admission closed until this process exits; Drop must not resume it.
            fence.commit();
            FailedInstall::RecoveryRequired {
                record,
                receipt_saved,
            }
        }
    }
}

struct IdleFence {
    core: nero_core::CoreHandle,
    processing: std::sync::Arc<crate::processing::Processing>,
    lease: Option<crate::lifecycle::ExitLease>,
    armed: bool,
}
impl IdleFence {
    fn prepare(
        core: nero_core::CoreHandle,
        processing: std::sync::Arc<crate::processing::Processing>,
        lease: crate::lifecycle::ExitLease,
    ) -> Result<Self, String> {
        if !lease.owns(crate::lifecycle::ExitKind::Update) {
            return Err("업데이트 설치 권한을 확인하지 못했습니다.".into());
        }
        processing.quiesce_for_update()?;
        let fence = Self {
            core,
            processing,
            lease: Some(lease),
            armed: true,
        };
        fence
            .core
            .prepare_shutdown(None)
            .map_err(|error| error.to_string())?;
        Ok(fence)
    }
    fn rollback(&mut self) -> Result<(), String> {
        if !self.armed {
            return Ok(());
        }
        self.armed = false;
        if !self
            .lease
            .as_ref()
            .is_some_and(|lease| lease.owns(crate::lifecycle::ExitKind::Update))
        {
            return Err("다른 종료 작업이 진행 중이므로 업데이트 복구를 중단했습니다.".into());
        }
        let resumed = self.core.resume_after_failed_shutdown();
        self.processing.resume();
        resumed.map_err(|_| {
            "설치를 중단했습니다. 회의 데이터를 보존했으며 앱을 다시 열어 주세요.".into()
        })
    }
    fn commit(mut self) {
        self.armed = false;
        self.lease.take().unwrap().commit();
    }
}
impl Drop for IdleFence {
    fn drop(&mut self) {
        let _ = self.rollback();
    }
}

impl AppUpdates {
    pub fn new(cache: PathBuf) -> Result<Self, String> {
        // This file is embedded in the executable, never read from a writable feed.
        let config: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json"))
            .map_err(|_| "앱 버전을 확인하지 못했습니다.")?;
        let current = ReleaseIdentity::new(
            config["version"].as_str().unwrap_or(""),
            config["bundle"]["macOS"]["bundleVersion"]
                .as_str()
                .unwrap_or(""),
        )?;
        let previous_recovery = read_limited(&cache.join("recovery.json"), MAX_METADATA)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<RecoveryRecord>(&bytes).ok());
        let pending_message = read_limited(&cache.join("pending.json"), MAX_METADATA)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<PendingInstall>(&bytes).ok())
            .and_then(|pending| pending.message(&current));
        // A newly launched app may have been manually repaired. Preserve the
        // historical warning/path without claiming the prior restore succeeded.
        let recovery_message = previous_recovery.as_ref().map(|_| "이전 업데이트에서 앱 복원에 실패했습니다. 이전 앱의 보관 위치와 설치 상태를 확인해 주세요.".to_string()).or(pending_message);
        let status = Status {
            phase: "idle".into(),
            current_version: current.version.clone(),
            current_build: current.build.to_string(),
            available_version: None,
            available_build: None,
            downloaded_bytes: 0,
            total_bytes: None,
            download_ready: false,
            message: String::new(),
            recovery_message,
            recovery_required: false,
            previous_app_path: previous_recovery
                .as_ref()
                .map(|record| record.previous_app_path.clone()),
            recovery_receipt_saved: previous_recovery.is_some(),
        };
        Ok(Self {
            current,
            cache,
            status: Mutex::new(status),
            candidate: Mutex::new(None),
            operation: tokio::sync::Mutex::new(()),
        })
    }
    pub(crate) fn agent_metadata(
        &self,
        resources: Option<&Path>,
    ) -> nero_core::agent_control::AppMetadata {
        let integration_version = resources
            .and_then(|path| read_limited(&path.join("agent/agent-build.json"), MAX_METADATA).ok())
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .filter(|value| value["version"] == self.current.version && value["apiVersion"] == 1)
            .and_then(|value| value["integrationVersion"].as_str().map(str::to_owned));
        nero_core::agent_control::AppMetadata {
            version: self.current.version.clone(),
            build: self.current.build.to_string(),
            integration_version,
        }
    }

    fn view(&self) -> Status {
        self.status.lock().unwrap().clone()
    }
    fn restore_cached(&self, app: &tauri::AppHandle) {
        if self.view().phase != "idle" {
            return;
        }
        let restored = (|| {
            let raw = read_limited(&self.cache.join("manifest.json"), MAX_METADATA)?;
            let manifest =
                serde_json::from_slice(&raw).map_err(|_| "업데이트 정보를 읽지 못했습니다.")?;
            let Some(update) = self
                .updater(app)?
                .restore_update(manifest)
                .map_err(|_| "업데이트 정보를 확인하지 못했습니다.")?
            else {
                return Ok::<_, String>(());
            };
            let bytes = read_limited(&self.cache.join("package"), MAX_PACKAGE)?;
            let identity = self.verify(&update, &bytes)?;
            {
                let mut status = self.status.lock().unwrap();
                status.available_version = Some(identity.version);
                status.available_build = Some(identity.build.to_string());
                status.downloaded_bytes = bytes.len() as u64;
                status.total_bytes = Some(bytes.len() as u64);
                status.download_ready = true;
            }
            *self.candidate.lock().unwrap() = Some(update);
            self.publish(
                app,
                "ready",
                "이전에 다운로드한 업데이트를 설치할 수 있습니다.",
            );
            Ok(())
        })();
        // An incomplete/corrupt cache is never installed. A fresh online check
        // remains available; ordinary startup does not become an error dialog.
        if restored.is_err() {
            *self.candidate.lock().unwrap() = None;
        }
    }
    fn transition(&self, phase: &str, message: &str) -> Status {
        let mut status = self.status.lock().unwrap();
        if !status.recovery_required {
            status.phase = phase.into();
            status.message = message.into();
        }
        status.clone()
    }
    fn publish(&self, app: &tauri::AppHandle, phase: &str, message: &str) -> Status {
        let status = self.transition(phase, message);
        let _ = app.emit_to("main", "nero://app-update", &status);
        status
    }
    fn latch_recovery(&self, record: RecoveryRecord, receipt_saved: bool) -> Status {
        let mut status = self.status.lock().unwrap();
        status.phase = "recoveryRequired".into();
        status.recovery_required = true;
        status.previous_app_path = Some(record.previous_app_path);
        status.recovery_receipt_saved = receipt_saved;
        status.download_ready = false;
        status.message = if receipt_saved {
            "업데이트 중 이전 앱을 원래 위치로 복원하지 못했습니다. 작업을 중지했습니다. 보관된 이전 앱의 위치를 확인하고 앱을 복구해 주세요."
        } else {
            "업데이트 중 이전 앱을 원래 위치로 복원하지 못했습니다. 복구 안내도 저장하지 못했습니다. 표시된 보관 위치를 복사한 뒤 앱을 종료해 주세요."
        }.into();
        status.recovery_message = Some(status.message.clone());
        *self.candidate.lock().unwrap() = None;
        status.clone()
    }
    fn updater(&self, app: &tauri::AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
        let current = self.current.clone();
        app.updater_builder()
            .timeout(Duration::from_secs(30))
            // Plugin's default hook destroys windows before ShellExecute can
            // report a launch failure. Our saved-state/fence protocol runs first;
            // keep the app usable if Windows refuses to launch the installer.
            .on_before_exit(|| {})
            .version_comparator(move |_, release| {
                ReleaseIdentity::parse_wire(&release.version.to_string())
                    .is_ok_and(|next| next.newer_than(&current))
            })
            .build()
            .map_err(|_| "업데이트 설정을 확인하지 못했습니다.".into())
    }
    fn verify(&self, update: &Update, bytes: &[u8]) -> Result<ReleaseIdentity, String> {
        let identity = ReleaseIdentity::parse_wire(&update.version)?;
        if !identity.newer_than(&self.current) {
            return Err("현재보다 새로운 빌드만 설치할 수 있습니다.".into());
        }
        // The raw spelling must also be canonical, not just semver-normalized.
        if update.raw_json["version"].as_str() != Some(identity.wire().as_str()) {
            return Err("업데이트 버전 형식이 올바르지 않습니다.".into());
        }
        validate_url(&update.download_url)?;
        policy::verify_package(bytes, PUBLIC_KEY, &update.signature, &identity)?;
        update
            .verify(bytes)
            .map_err(|_| "업데이트 파일의 서명을 확인하지 못했습니다.")?;
        Ok(identity)
    }
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| "업데이트 파일을 읽지 못했습니다.")?;
    if file
        .metadata()
        .map_err(|_| "업데이트 파일을 확인하지 못했습니다.")?
        .len()
        > limit
    {
        return Err("업데이트 파일이 허용 크기를 초과했습니다.".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "업데이트 파일을 읽지 못했습니다.")?;
    if bytes.len() as u64 > limit {
        return Err("업데이트 파일이 허용 크기를 초과했습니다.".into());
    }
    Ok(bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or("업데이트 저장 경로를 확인하지 못했습니다.")?;
    std::fs::create_dir_all(parent).map_err(|_| "업데이트 저장 공간을 준비하지 못했습니다.")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "업데이트 파일을 저장하지 못했습니다.")?;
    file.write_all(bytes)
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| "업데이트 파일을 저장하지 못했습니다.")?;
    file.persist(path)
        .map_err(|_| "업데이트 파일을 저장하지 못했습니다.")?;
    Ok(())
}

fn validate_url(url: &reqwest::Url) -> Result<(), String> {
    let valid = url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url
            .path()
            .starts_with("/ChoSeongmin1128/Nero/releases/download/")
        && !url.path().contains("%")
        && !url.path().contains("/../");
    if valid {
        Ok(())
    } else {
        Err("공식 Nero 업데이트 파일 경로가 아닙니다.".into())
    }
}

fn require_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("본창에서 실행해 주세요.".into())
    }
}

#[tauri::command]
pub async fn app_update_status(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppUpdates>,
) -> Result<Status, String> {
    require_window(&window)?;
    if let Ok(_operation) = state.operation.try_lock() {
        state.restore_cached(&app);
    }
    Ok(state.view())
}

#[tauri::command]
pub async fn app_update_check(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppUpdates>,
) -> Result<Status, String> {
    require_window(&window)?;
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "업데이트 작업이 진행 중입니다.")?;
    if app.state::<crate::DesktopState>().lifecycle.is_closing() {
        return Err("앱을 종료하거나 업데이트하고 있습니다.".into());
    }
    state.publish(&app, "checking", "새 업데이트를 확인하고 있습니다.");
    let result = async {
        let updater = state.updater(&app)?;
        let candidate = updater
            .check()
            .await
            .map_err(|_| "업데이트를 확인하지 못했습니다. 잠시 후 다시 확인해 주세요.")?;
        if let Some(update) = &candidate {
            let identity = ReleaseIdentity::parse_wire(&update.version)?;
            validate_url(&update.download_url)?;
            if update.raw_json["version"].as_str() != Some(identity.wire().as_str()) {
                return Err("업데이트 버전 형식이 올바르지 않습니다.".to_string());
            }
            let mut status = state.status.lock().unwrap();
            status.available_version = Some(identity.version);
            status.available_build = Some(identity.build.to_string());
            status.downloaded_bytes = 0;
            status.total_bytes = None;
            status.download_ready = false;
        }
        *state.candidate.lock().unwrap() = candidate;
        if state.candidate.lock().unwrap().is_some() {
            Ok(state.publish(&app, "available", "새 업데이트를 다운로드할 수 있습니다."))
        } else {
            {
                let mut status = state.status.lock().unwrap();
                status.available_version = None;
                status.available_build = None;
                status.downloaded_bytes = 0;
                status.total_bytes = None;
                status.download_ready = false;
            }
            Ok(state.publish(&app, "current", "최신 버전을 사용하고 있습니다."))
        }
    }
    .await;
    if let Err(error) = &result {
        state.publish(&app, "error", error);
    }
    result
}

#[tauri::command]
pub async fn app_update_download(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppUpdates>,
) -> Result<Status, String> {
    require_window(&window)?;
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "업데이트 작업이 진행 중입니다.")?;
    let update = state
        .candidate
        .lock()
        .unwrap()
        .clone()
        .ok_or("먼저 업데이트를 확인해 주세요.")?;
    state.publish(
        &app,
        "downloading",
        "업데이트를 다운로드하고 있습니다. 녹음과 회의 작업은 계속할 수 있습니다.",
    );
    let result = async {
        validate_url(&update.download_url)?;
        // The official plugin verifies downloads, but its buffer is unbounded.
        // Bound the transfer here and invoke the same official verification.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(900))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 || attempt.url().scheme() != "https" {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|_| "다운로드를 준비하지 못했습니다.")?;
        let mut response = client
            .get(update.download_url.clone())
            .send()
            .await
            .and_then(|response| response.error_for_status())
            .map_err(|_| "업데이트 파일을 다운로드하지 못했습니다.")?;
        let total = response.content_length();
        if total.is_some_and(|size| size > MAX_PACKAGE) {
            return Err("업데이트 파일이 허용 크기를 초과했습니다.".to_string());
        }
        let mut bytes = Vec::new();
        let mut last_event = std::time::Instant::now();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "다운로드가 중단되었습니다. 다시 시도해 주세요.")?
        {
            if bytes.len() as u64 + chunk.len() as u64 > MAX_PACKAGE {
                return Err("업데이트 파일이 허용 크기를 초과했습니다.".to_string());
            }
            bytes.extend_from_slice(&chunk);
            {
                let mut status = state.status.lock().unwrap();
                status.downloaded_bytes = bytes.len() as u64;
                status.total_bytes = total;
            }
            if last_event.elapsed() >= Duration::from_millis(250) {
                let _ = app.emit_to("main", "nero://app-update", state.view());
                last_event = std::time::Instant::now();
            }
        }
        state.verify(&update, &bytes)?;
        write_atomic(&state.cache.join("package"), &bytes)?;
        write_atomic(
            &state.cache.join("manifest.json"),
            &serde_json::to_vec(&update.raw_json)
                .map_err(|_| "업데이트 정보를 저장하지 못했습니다.")?,
        )?;
        state.status.lock().unwrap().download_ready = true;
        Ok(state.publish(
            &app,
            "ready",
            "업데이트 준비를 마쳤습니다. 작업을 저장한 뒤 다시 시작해 설치할 수 있습니다.",
        ))
    }
    .await;
    if let Err(error) = &result {
        state.publish(&app, "error", error);
    }
    result
}

#[tauri::command]
pub async fn app_update_install(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppUpdates>,
    desktop: tauri::State<'_, crate::DesktopState>,
) -> Result<Status, String> {
    require_window(&window)?;
    if state.view().recovery_required {
        return Ok(state.view());
    }
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "업데이트 작업이 진행 중입니다.")?;
    // Restore using the running app's trusted config. Cached metadata cannot
    // select its own public key, executable path or comparator.
    let metadata = read_limited(&state.cache.join("manifest.json"), MAX_METADATA)?;
    let manifest =
        serde_json::from_slice(&metadata).map_err(|_| "업데이트 정보를 읽지 못했습니다.")?;
    let update = state
        .updater(&app)?
        .restore_update(manifest)
        .map_err(|_| "업데이트 정보를 확인하지 못했습니다.")?
        .ok_or("현재보다 새로운 빌드만 설치할 수 있습니다.")?;
    let bytes = read_limited(&state.cache.join("package"), MAX_PACKAGE)?;
    let identity = state.verify(&update, &bytes)?;
    if desktop.quitting.load(Ordering::Acquire) {
        return Err("앱을 종료하고 있습니다.".into());
    }
    let lease = desktop
        .lifecycle
        .begin_exit(crate::lifecycle::ExitKind::Update)?;
    let processing = desktop.processing.clone();
    let core = desktop.core.clone();
    let prepared =
        tauri::async_runtime::spawn_blocking(move || IdleFence::prepare(core, processing, lease))
            .await
            .map_err(|_| "앱의 작업 상태를 확인하지 못했습니다.".to_string())
            .and_then(|result| result);
    let fence = match prepared {
        Ok(fence) => fence,
        Err(error) => {
            return Ok(state.publish(&app, "waiting", &error));
        }
    };
    state.publish(
        &app,
        "installing",
        "업데이트를 설치하고 Nero를 다시 시작합니다.",
    );
    let pending = PendingInstall {
        from: state.current.clone(),
        target: identity,
    };
    let receipt =
        serde_json::to_vec(&pending).map_err(|_| "업데이트 기록을 만들지 못했습니다.".to_string());
    let cache = state.cache.clone();
    let quitting = desktop.quitting.clone();
    let polling = desktop.polling.clone();
    let install_app = app.clone();
    let installed = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let result = (|| -> Result<(), InstallFailure> {
            write_atomic(
                &cache.join("pending.json"),
                &receipt.map_err(InstallFailure::Retryable)?,
            )
            .map_err(InstallFailure::Retryable)?;
            update.install(&bytes).map_err(InstallFailure::from)
        })();
        if let Err(error) = result {
            return match finish_failed_install(fence, error, &cache) {
                FailedInstall::Retryable(message) => Err(message),
                FailedInstall::RecoveryRequired {
                    record,
                    receipt_saved,
                } => {
                    // Publish from the owning task, not its IPC waiter: even a
                    // cancelled caller cannot lose the preserved backup path.
                    let status = install_app
                        .state::<AppUpdates>()
                        .latch_recovery(record, receipt_saved);
                    let _ = install_app.emit_to("main", "nero://app-update", &status);
                    Err(status.message)
                }
            };
        }
        // Keep successful ownership through restart in this task as well. A
        // dropped IPC future cannot reopen admission after the Mac app swap.
        quitting.store(true, Ordering::Release);
        polling.store(false, Ordering::Release);
        fence.commit();
        install_app.restart();
    })
    .await
    .map_err(|_| "업데이트 설치를 완료하지 못했습니다.".to_string())
    .and_then(|result| result);
    if let Err(error) = installed {
        if state.view().recovery_required {
            return Ok(state.view());
        }
        // Keep the verified download for retry; never touch meetings or models.
        return Ok(state.publish(&app, "error", &error));
    }
    Ok(state.view())
}

#[tauri::command]
pub fn app_update_recovery_quit(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppUpdates>,
    desktop: tauri::State<'_, crate::DesktopState>,
) -> Result<(), String> {
    require_window(&window)?;
    if !state.view().recovery_required || !desktop.lifecycle.is_closing() {
        return Err("앱 복구가 필요한 상태에서만 실행할 수 있습니다.".into());
    }
    // Core/processing are already fenced. Do not reopen or re-finalize them.
    desktop.quitting.store(true, Ordering::Release);
    desktop.polling.store(false, Ordering::Release);
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prepared_fixture(
        root: &Path,
    ) -> (
        nero_core::CoreHandle,
        std::sync::Arc<crate::processing::Processing>,
        crate::lifecycle::Lifecycle,
        IdleFence,
    ) {
        let core = nero_core::CoreHandle::open(root).unwrap();
        core.create_meeting("보존", "기존 메모").unwrap();
        let processing = crate::processing::Processing::new(root.into(), root.into());
        let lifecycle = crate::lifecycle::Lifecycle::default();
        let lease = lifecycle
            .begin_exit(crate::lifecycle::ExitKind::Update)
            .unwrap();
        let fence = IdleFence::prepare(core.clone(), processing.clone(), lease).unwrap();
        (core, processing, lifecycle, fence)
    }

    #[test]
    fn previous_app_not_restored_preserves_path_and_keeps_all_admission_closed() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let backup = root.join("이전 앱.app");
        std::fs::create_dir(&backup).unwrap();
        std::fs::write(backup.join("marker"), b"preserved previous application").unwrap();
        let (core, processing, lifecycle, fence) = prepared_fixture(&root.join("library"));
        let cache = root.join("cache");
        let failure = tauri_plugin_updater::Error::PreviousAppNotRestored(backup.clone());
        let FailedInstall::RecoveryRequired {
            record,
            receipt_saved,
        } = finish_failed_install(fence, failure.into(), &cache)
        else {
            panic!("a failed restore must not resume normal work");
        };
        assert!(receipt_saved);
        assert_eq!(record.previous_app_path, backup.to_string_lossy());
        assert!(core.create_folder("차단된 변경").is_err());
        assert!(processing.recording_start(|| Ok(())).is_err());
        assert!(lifecycle
            .begin_exit(crate::lifecycle::ExitKind::Quit)
            .is_err());
        assert_eq!(
            std::fs::read(backup.join("marker")).unwrap(),
            b"preserved previous application"
        );
        let state = AppUpdates::new(cache.clone()).unwrap();
        let status = state.latch_recovery(record, receipt_saved);
        let json = serde_json::to_value(status).unwrap();
        assert_eq!(json["phase"], "recoveryRequired");
        assert_eq!(json["recoveryRequired"], true);
        assert_eq!(json["previousAppPath"], backup.to_string_lossy().as_ref());
        assert_eq!(json["downloadReady"], false);
        for phase in ["ready", "error", "current"] {
            let late = state.transition(phase, "late ordinary status");
            assert_eq!(late.phase, "recoveryRequired");
            assert!(late.recovery_required);
            assert_eq!(late.previous_app_path.as_deref(), backup.to_str());
            assert!(!late.message.contains("late ordinary status"));
        }
        let relaunched = AppUpdates::new(cache).unwrap().view();
        assert_eq!(
            relaunched.previous_app_path.as_deref(),
            Some(backup.to_str().unwrap())
        );
        assert!(relaunched.recovery_message.unwrap().contains("복원에 실패"));
        // Only the already-failed process is latched; a new manually launched
        // process keeps the historical receipt without declaring a repair.
        assert!(!relaunched.recovery_required);
    }

    #[test]
    fn recovery_receipt_storage_failure_does_not_reopen_gates_or_lose_the_live_path() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let blocked_cache = root.join("not-a-directory");
        std::fs::write(&blocked_cache, b"preserve").unwrap();
        let (core, processing, lifecycle, fence) = prepared_fixture(&root.join("library"));
        let backup = root.join("previous.app");
        let failure = tauri_plugin_updater::Error::PreviousAppNotRestored(backup.clone());
        let FailedInstall::RecoveryRequired {
            record,
            receipt_saved,
        } = finish_failed_install(fence, failure.into(), &blocked_cache)
        else {
            panic!("recovery failure must remain latched");
        };
        assert!(!receipt_saved);
        let state = AppUpdates::new(root.join("status-cache")).unwrap();
        let status = state.latch_recovery(record, receipt_saved);
        assert_eq!(status.previous_app_path.as_deref(), backup.to_str());
        assert!(status.message.contains("저장하지 못했습니다"));
        assert!(core.create_folder("차단된 변경").is_err());
        assert!(processing.recording_start(|| Ok(())).is_err());
        assert!(lifecycle.is_closing());
        assert_eq!(std::fs::read(blocked_cache).unwrap(), b"preserve");
    }

    #[test]
    fn ordinary_install_error_reopens_its_gates_and_retains_meeting_data() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let (core, processing, lifecycle, fence) = prepared_fixture(&root.join("library"));
        let failure =
            tauri_plugin_updater::Error::Io(std::io::Error::other("synthetic launch failure"));
        assert!(matches!(
            finish_failed_install(fence, failure.into(), &root.join("cache")),
            FailedInstall::Retryable(_)
        ));
        assert!(!lifecycle.is_closing());
        core.create_folder("다시 작업").unwrap();
        assert_eq!(core.list_meetings().unwrap()[0].notes, "기존 메모");
        processing.recording_start(|| Ok(())).unwrap();
    }

    #[test]
    fn installation_failure_reopens_both_admission_gates_and_preserves_data() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let core = nero_core::CoreHandle::open(&root).unwrap();
        let meeting = core.create_meeting("보존", "기존 메모").unwrap();
        let processing = crate::processing::Processing::new(root.clone(), root);
        let lifecycle = crate::lifecycle::Lifecycle::default();
        let lease = lifecycle
            .begin_exit(crate::lifecycle::ExitKind::Update)
            .unwrap();
        let fence = IdleFence::prepare(core.clone(), processing.clone(), lease).unwrap();
        assert!(core.create_folder("설치 중 변경").is_err());
        assert!(processing.recording_start(|| Ok(())).is_err());
        assert!(lifecycle
            .begin_exit(crate::lifecycle::ExitKind::Quit)
            .is_err());
        // A launcher failure/early return drops the same guard used by install.
        drop(fence);
        assert!(!lifecycle.is_closing());
        core.create_folder("다시 작업").unwrap();
        assert_eq!(core.list_meetings().unwrap()[0].id, meeting.id);
        assert_eq!(core.list_meetings().unwrap()[0].notes, "기존 메모");
        processing.recording_start(|| Ok(())).unwrap();
    }

    #[test]
    fn configuration_uses_the_verifiers_key_and_frontend_cannot_bypass_policy() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(config["plugins"]["updater"]["pubkey"], PUBLIC_KEY);
        assert_eq!(config["plugins"]["updater"]["requireSignedVersion"], true);
        let capabilities: serde_json::Value =
            serde_json::from_str(include_str!("../../capabilities/desktop.json")).unwrap();
        assert!(capabilities["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|permission| !permission.as_str().unwrap().starts_with("updater:")));
    }
    #[test]
    fn artifact_url_is_pinned_to_the_official_release_repository() {
        assert!(validate_url(
            &"https://github.com/ChoSeongmin1128/Nero/releases/download/build41/Nero.exe"
                .parse()
                .unwrap()
        )
        .is_ok());
        for url in [
            "http://github.com/ChoSeongmin1128/Nero/releases/download/x/a",
            "https://github.com/other/Nero/releases/download/x/a",
            "https://github.com.evil.test/ChoSeongmin1128/Nero/releases/download/x/a",
            "https://token@github.com/ChoSeongmin1128/Nero/releases/download/x/a",
            "https://github.com/ChoSeongmin1128/Nero/releases/download/x/a?token=secret",
        ] {
            assert!(validate_url(&url.parse().unwrap()).is_err(), "{url}");
        }
    }
    #[test]
    fn superseded_pending_does_not_clear_a_real_restore_failure_or_rewrite_journals() {
        let temp = tempfile::tempdir().unwrap();
        let state = AppUpdates::new(temp.path().into()).unwrap();
        let pending = serde_json::to_vec(&PendingInstall {
            from: ReleaseIdentity::new(
                &state.current.version,
                &(state.current.build - 2).to_string(),
            )
            .unwrap(),
            target: ReleaseIdentity::new(
                &state.current.version,
                &(state.current.build - 1).to_string(),
            )
            .unwrap(),
        })
        .unwrap();
        let recovery = serde_json::to_vec(&RecoveryRecord {
            previous_app_path: "synthetic-previous.app".into(),
        })
        .unwrap();
        write_atomic(&temp.path().join("pending.json"), &pending).unwrap();
        write_atomic(&temp.path().join("recovery.json"), &recovery).unwrap();
        let relaunched = AppUpdates::new(temp.path().into()).unwrap().view();
        assert!(relaunched.recovery_message.unwrap().contains("복원에 실패"));
        assert_eq!(
            relaunched.previous_app_path.as_deref(),
            Some("synthetic-previous.app")
        );
        assert_eq!(
            read_limited(&temp.path().join("pending.json"), MAX_METADATA).unwrap(),
            pending
        );
        assert_eq!(
            read_limited(&temp.path().join("recovery.json"), MAX_METADATA).unwrap(),
            recovery
        );
    }

    #[test]
    fn metadata_reports_only_this_apps_bundled_integration_without_initializing_paths() {
        let temp = tempfile::tempdir().unwrap();
        let state = AppUpdates::new(temp.path().join("cache")).unwrap();
        let resources = temp.path().join("resources");
        assert!(state
            .agent_metadata(Some(&resources))
            .integration_version
            .is_none());
        assert!(!resources.exists());
        let agent = resources.join("agent");
        std::fs::create_dir_all(&agent).unwrap();
        for (version, api, expected) in [
            (&state.current.version[..], 1, true),
            ("0.0.0", 1, false),
            (&state.current.version[..], 2, false),
        ] {
            std::fs::write(agent.join("agent-build.json"), serde_json::json!({"version":version,"apiVersion":api,"integrationVersion":"0.1.0-nero.33"}).to_string()).unwrap();
            let metadata = state.agent_metadata(Some(&resources));
            assert_eq!(metadata.version, state.current.version);
            assert_eq!(metadata.build, state.current.build.to_string());
            assert_eq!(metadata.integration_version.is_some(), expected);
        }
        assert!(!temp.path().join("cache").exists());
    }

    #[test]
    fn completed_or_superseded_install_does_not_warn_but_incomplete_and_downgrades_do() {
        let pending = PendingInstall {
            from: ReleaseIdentity::new("0.0.1", "43").unwrap(),
            target: ReleaseIdentity::new("0.0.1", "45").unwrap(),
        };
        assert!(pending
            .message(&pending.from)
            .unwrap()
            .contains("완료되지 않았"));
        assert!(pending.message(&pending.target).is_none());
        assert!(pending
            .message(&ReleaseIdentity::new("0.1.0", "46").unwrap())
            .is_none());
        for (version, build) in [("0.0.1", "44"), ("0.0.0", "46"), ("0.1.0", "44")] {
            assert!(pending
                .message(&ReleaseIdentity::new(version, build).unwrap())
                .is_some());
        }
    }

    #[test]
    fn cache_reads_are_bounded_and_pending_install_survives_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let initial = AppUpdates::new(tmp.path().into()).unwrap();
        let pending = PendingInstall {
            from: initial.current.clone(),
            target: ReleaseIdentity::new(
                &initial.current.version,
                &(initial.current.build + 1).to_string(),
            )
            .unwrap(),
        };
        write_atomic(
            &tmp.path().join("pending.json"),
            &serde_json::to_vec(&pending).unwrap(),
        )
        .unwrap();
        let state = AppUpdates::new(tmp.path().into()).unwrap();
        assert!(state
            .view()
            .recovery_message
            .unwrap()
            .contains("완료되지 않았"));
        let completed = PendingInstall {
            from: ReleaseIdentity::new(
                &initial.current.version,
                &(initial.current.build - 1).to_string(),
            )
            .unwrap(),
            target: initial.current.clone(),
        };
        write_atomic(
            &tmp.path().join("pending.json"),
            &serde_json::to_vec(&completed).unwrap(),
        )
        .unwrap();
        let completed_state = AppUpdates::new(tmp.path().into()).unwrap();
        assert!(completed_state.view().recovery_message.is_none());
        assert!(completed_state.view().message.is_empty());
        assert!(read_limited(&tmp.path().join("pending.json"), 2).is_err());
        write_atomic(&tmp.path().join("package"), b"old").unwrap();
        write_atomic(&tmp.path().join("package"), b"new").unwrap();
        assert_eq!(
            read_limited(&tmp.path().join("package"), 3).unwrap(),
            b"new"
        );
    }
}
