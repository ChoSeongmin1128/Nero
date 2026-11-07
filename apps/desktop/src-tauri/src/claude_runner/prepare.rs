//! App-owned Claude Code runtime. No installer scripts or credential migration.
use super::{validate, Runner};
use fs2::FileExt;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

const BASE: &str = "https://downloads.claude.ai/claude-code-releases";
const MAX_BINARY: u64 = 512 * 1024 * 1024;
const MAX_META: usize = 128 * 1024;
const FAILURE: &str = "Claude Code 실행 파일을 준비하지 못했습니다.";

#[derive(Serialize, Deserialize)]
struct Receipt {
    version: String,
    platform: String,
    checksum: String,
    size: u64,
}
fn platform() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("macos", "x86_64") => Ok("darwin-x64"),
        ("windows", "aarch64") => Ok("win32-arm64"),
        ("windows", "x86_64") => Ok("win32-x64"),
        _ => Err("이 기기에서는 Claude Code를 지원하지 않습니다.".into()),
    }
}
fn name() -> &'static str {
    if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    }
}
fn hash_file(path: &Path, max: u64, cancel: Option<&AtomicBool>) -> Result<(String, u64), String> {
    let mut input = File::open(path).map_err(|_| FAILURE)?;
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut buf = [0u8; 65536];
    loop {
        if cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
            return Err("취소했습니다.".into());
        }
        let n = input.read(&mut buf).map_err(|_| FAILURE)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > max {
            return Err(FAILURE.into());
        }
        hash.update(&buf[..n]);
    }
    Ok((format!("{:x}", hash.finalize()), count))
}
fn verified(
    dir: &Path,
    version: &Version,
    target: &str,
    cancel: Option<&AtomicBool>,
) -> Result<Runner, String> {
    let receipt_path = dir.join("receipt.json");
    if fs::symlink_metadata(dir)
        .map_err(|_| FAILURE)?
        .file_type()
        .is_symlink()
        || fs::metadata(&receipt_path).map_err(|_| FAILURE)?.len() > MAX_META as u64
    {
        return Err(FAILURE.into());
    }
    let receipt: Receipt = serde_json::from_reader(File::open(receipt_path).map_err(|_| FAILURE)?)
        .map_err(|_| FAILURE)?;
    if receipt.version != version.to_string()
        || receipt.platform != target
        || receipt.size == 0
        || receipt.size > MAX_BINARY
    {
        return Err(FAILURE.into());
    }
    let binary = dir.join(name());
    if !fs::symlink_metadata(&binary)
        .map_err(|_| FAILURE)?
        .file_type()
        .is_file()
    {
        return Err(FAILURE.into());
    }
    let (digest, size) = hash_file(&binary, MAX_BINARY, cancel)?;
    if digest != receipt.checksum || size != receipt.size {
        return Err(FAILURE.into());
    }
    validate(&binary)?;
    Ok(Runner {
        binary,
        checksum: Some(digest),
    })
}
/// Return the newest intact, OS-signed app-owned runtime without network access.
pub fn discover_prepared(root: &Path) -> Result<Runner, String> {
    discover_prepared_with_cancel(root, None)
}
pub(super) fn discover_prepared_with_cancel(
    root: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<Runner, String> {
    let target = platform()?;
    let mut candidates = Vec::new();
    for item in fs::read_dir(root).map_err(|_| FAILURE)? {
        let item = item.map_err(|_| FAILURE)?;
        if let Some(name) = item.file_name().to_str() {
            if let Ok(version) = Version::parse(name) {
                if version.to_string() == name {
                    candidates.push((version, item.path()));
                }
            }
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    for (version, path) in candidates {
        if cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
            return Err("취소했습니다.".into());
        }
        if let Ok(runner) = verified(&path, &version, target, cancel) {
            return Ok(runner);
        }
    }
    Err("준비된 Claude Code 실행 파일을 찾지 못했습니다.".into())
}
async fn stopped(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
async fn get(
    client: &reqwest::Client,
    url: &str,
    cancel: &AtomicBool,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let request = async {
        let mut response = client
            .get(url)
            .send()
            .await
            .map_err(|_| FAILURE)?
            .error_for_status()
            .map_err(|_| FAILURE)?;
        let mut out = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| FAILURE)? {
            if out.len() + chunk.len() > limit {
                return Err(FAILURE.to_string());
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    };
    tokio::select! { _ = stopped(cancel) => Err("취소했습니다.".into()), result = request => result }
}
/// Downloads an official native binary into an app-owned version directory.
/// On an update check failure an already verified runtime is returned.
pub async fn prepare(root: &Path, cancel: Arc<AtomicBool>) -> Result<Runner, String> {
    let root = root.to_path_buf();
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || runtime.block_on(prepare_work(&root, cancel)))
        .await
        .map_err(|_| FAILURE.to_string())?
}
async fn prepare_work(root: &Path, cancel: Arc<AtomicBool>) -> Result<Runner, String> {
    fs::create_dir_all(root).map_err(|_| FAILURE)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(".prepare.lock"))
        .map_err(|_| FAILURE)?;
    lock.try_lock_exclusive()
        .map_err(|_| "Claude Code를 준비 중입니다.")?;
    let cached = discover_prepared_with_cancel(root, Some(&cancel)).ok();
    if cancel.load(Ordering::Acquire) {
        return Err("취소했습니다.".into());
    }
    let result = prepare_inner(root, &cancel).await;
    if cancel.load(Ordering::Acquire) {
        return Err("취소했습니다.".into());
    }
    result.or_else(|error| cached.ok_or(error))
}
// Keep one suspect directory for inspection and never remove unrecognized files.
fn quarantine_corrupt(root: &Path, original: &Path) -> Result<(), String> {
    let quarantine = root.join(".claude-corrupt");
    if quarantine.exists() {
        return Err(
            "기존 Claude Code 실행 파일을 확인하지 못했습니다. 보관된 실행 파일을 확인해 주세요."
                .into(),
        );
    }
    let metadata = fs::symlink_metadata(original).map_err(|_| FAILURE)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(FAILURE.into());
    }
    fs::rename(original, quarantine).map_err(|_| FAILURE.into())
}
fn remove_owned_runtime(dir: &Path) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|_| FAILURE)?;
    let mut owned = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| FAILURE)?;
        let filename = entry.file_name();
        if (filename != name() && filename != "receipt.json")
            || !entry.file_type().map_err(|_| FAILURE)?.is_file()
        {
            return Ok(());
        }
        owned.push(entry.path());
    }
    for path in owned {
        fs::remove_file(path).map_err(|_| FAILURE)?;
    }
    fs::remove_dir(dir).map_err(|_| FAILURE.into())
}
async fn prepare_inner(root: &Path, cancel: &AtomicBool) -> Result<Runner, String> {
    let target = platform()?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|_| FAILURE)?;
    let version_data = get(&client, &format!("{BASE}/stable"), cancel, 128).await?;
    let version_text = std::str::from_utf8(&version_data)
        .map_err(|_| FAILURE)?
        .trim();
    let version = Version::parse(version_text).map_err(|_| FAILURE)?;
    if version.to_string() != version_text || !version.pre.is_empty() || !version.build.is_empty() {
        return Err(FAILURE.into());
    }
    let final_dir = root.join(version_text);
    if let Ok(runner) = verified(&final_dir, &version, target, Some(cancel)) {
        return Ok(runner);
    }
    if cancel.load(Ordering::Acquire) {
        return Err("취소했습니다.".into());
    }
    if final_dir.exists() {
        quarantine_corrupt(root, &final_dir)?;
    }
    let metadata = get(
        &client,
        &format!("{BASE}/{version_text}/manifest.json"),
        cancel,
        MAX_META,
    )
    .await?;
    let manifest: serde_json::Value = serde_json::from_slice(&metadata).map_err(|_| FAILURE)?;
    if manifest["version"].as_str() != Some(version_text) {
        return Err(FAILURE.into());
    }
    let entry = &manifest["platforms"][target];
    let checksum = entry["checksum"].as_str().ok_or(FAILURE)?;
    let size = entry["size"].as_u64().ok_or(FAILURE)?;
    if checksum.len() != 64
        || !checksum
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || size == 0
        || size > MAX_BINARY
        || entry["binary"]
            .as_str()
            .is_some_and(|value| value != name())
    {
        return Err(FAILURE.into());
    }
    let temp = tempfile::Builder::new()
        .prefix(".claude-stage-")
        .tempdir_in(root)
        .map_err(|_| FAILURE)?;
    let binary = temp.path().join(name());
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&binary)
        .map_err(|_| FAILURE)?;
    let url = format!("{BASE}/{version_text}/{target}/{}", name());
    let download = async {
        let mut response = client
            .get(url)
            .send()
            .await
            .map_err(|_| FAILURE)?
            .error_for_status()
            .map_err(|_| FAILURE)?;
        let mut hash = Sha256::new();
        let mut count = 0u64;
        while let Some(chunk) = response.chunk().await.map_err(|_| FAILURE)? {
            count += chunk.len() as u64;
            if count > size || count > MAX_BINARY {
                return Err(FAILURE.to_string());
            }
            file.write_all(&chunk).map_err(|_| FAILURE)?;
            hash.update(&chunk);
        }
        if count != size || format!("{:x}", hash.finalize()) != checksum {
            return Err(FAILURE.into());
        }
        Ok(())
    };
    tokio::select! { _ = stopped(cancel) => return Err("취소했습니다.".into()), result = download => result? }
    file.sync_all().map_err(|_| FAILURE)?;
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).map_err(|_| FAILURE)?;
    }
    validate(&binary)?;
    let receipt = Receipt {
        version: version_text.into(),
        platform: target.into(),
        checksum: checksum.into(),
        size,
    };
    let mut f = File::create(temp.path().join("receipt.json")).map_err(|_| FAILURE)?;
    serde_json::to_writer(&mut f, &receipt).map_err(|_| FAILURE)?;
    f.sync_all().map_err(|_| FAILURE)?;
    if cancel.load(Ordering::Acquire) {
        return Err("취소했습니다.".into());
    }
    fs::rename(temp.path(), &final_dir).map_err(|_| FAILURE)?;
    let runner = verified(&final_dir, &version, target, Some(cancel))?;
    // Retain the current release and at most one previous, verified release.
    let mut others = Vec::<(Version, PathBuf)>::new();
    for item in fs::read_dir(root).map_err(|_| FAILURE)? {
        let item = item.map_err(|_| FAILURE)?;
        if let Some(name) = item.file_name().to_str() {
            if let Ok(v) = Version::parse(name) {
                if v.to_string() == name
                    && v != version
                    && verified(&item.path(), &v, target, Some(cancel)).is_ok()
                {
                    others.push((v, item.path()));
                }
            }
        }
    }
    others.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, old) in others.into_iter().skip(1) {
        if cancel.load(Ordering::Acquire) {
            return Err("취소했습니다.".into());
        }
        remove_owned_runtime(&old)
            .map_err(|_| "이전 Claude Code 실행 파일을 정리하지 못했습니다.")?;
    }
    Ok(runner)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_rejects_wrong_length_or_hash_before_signature_check() {
        let temp = tempfile::tempdir().unwrap();
        let version = Version::parse("2.1.300").unwrap();
        let target = platform().unwrap();
        fs::write(temp.path().join(name()), b"fixture").unwrap();
        let receipt = Receipt {
            version: version.to_string(),
            platform: target.into(),
            checksum: "0".repeat(64),
            size: 7,
        };
        serde_json::to_writer(
            File::create(temp.path().join("receipt.json")).unwrap(),
            &receipt,
        )
        .unwrap();
        assert!(verified(temp.path(), &version, target, None).is_err());
        assert!(hash_file(&temp.path().join(name()), 6, None).is_err());
    }
    #[test]
    fn corrupt_directory_is_preserved_and_quarantine_is_bounded() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("2.1.300");
        fs::create_dir(&original).unwrap();
        fs::write(original.join("unknown-user-file"), b"keep").unwrap();
        quarantine_corrupt(root.path(), &original).unwrap();
        assert_eq!(
            fs::read(root.path().join(".claude-corrupt/unknown-user-file")).unwrap(),
            b"keep"
        );
        fs::create_dir(&original).unwrap();
        assert!(quarantine_corrupt(root.path(), &original).is_err());
        assert!(original.exists());
    }
    #[test]
    fn runtime_cleanup_preserves_unrecognized_contents() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("runtime");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join(name()), b"binary").unwrap();
        fs::write(dir.join("receipt.json"), b"receipt").unwrap();
        fs::write(dir.join("extra"), b"keep").unwrap();
        remove_owned_runtime(&dir).unwrap();
        assert_eq!(fs::read(dir.join("extra")).unwrap(), b"keep");
        assert!(dir.join(name()).exists());
        fs::remove_file(dir.join("extra")).unwrap();
        remove_owned_runtime(&dir).unwrap();
        assert!(!dir.exists());
    }
    #[cfg(unix)]
    #[test]
    fn runtime_cleanup_preserves_symlinks() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("runtime");
        fs::create_dir(&dir).unwrap();
        symlink(root.path(), dir.join(name())).unwrap();
        remove_owned_runtime(&dir).unwrap();
        assert!(fs::symlink_metadata(dir.join(name()))
            .unwrap()
            .file_type()
            .is_symlink());
    }
    #[test]
    fn hashing_honors_cancellation() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("binary");
        fs::write(&path, b"fixture").unwrap();
        let cancel = AtomicBool::new(true);
        assert!(hash_file(&path, MAX_BINARY, Some(&cancel)).is_err());
    }
}
