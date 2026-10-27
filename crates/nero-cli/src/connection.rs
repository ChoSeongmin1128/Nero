use fs2::FileExt;
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct Observer {
    directory: PathBuf,
    client: String,
    token: String,
    generation: String,
    lock: Mutex<Option<Session>>,
}

struct Session {
    lock: File,
    receipt_path: PathBuf,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.lock);
        let _ = std::fs::remove_file(&self.receipt_path);
        let _ = std::fs::remove_file(self.receipt_path.with_extension("lock"));
    }
}

impl Observer {
    pub(crate) fn from_environment(root: &Path) -> Option<Self> {
        let directory = PathBuf::from(std::env::var_os("NERO_CONNECTION_DIR")?);
        let client = std::env::var("NERO_CONNECTION_CLIENT").ok()?;
        let token = std::env::var("NERO_CONNECTION_TOKEN").ok()?;
        let generation = std::env::var("NERO_CONNECTION_GENERATION").ok()?;
        Self::configured(root, directory, client, token, generation)
    }

    fn configured(
        root: &Path,
        directory: PathBuf,
        client: String,
        token: String,
        generation: String,
    ) -> Option<Self> {
        if !directory.is_absolute()
            || !matches!(client.as_str(), "claude" | "codex")
            || !valid_token(&token)
            || generation.len() != 64
            || !generation.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return None;
        }
        if std::fs::symlink_metadata(&directory)
            .ok()?
            .file_type()
            .is_symlink()
            || !directory.is_dir()
        {
            return None;
        }
        let state_path = directory.parent()?.join("state.json");
        if std::fs::symlink_metadata(&state_path)
            .ok()?
            .file_type()
            .is_symlink()
        {
            return None;
        }
        let mut bytes = Vec::new();
        File::open(state_path)
            .ok()?
            .take(4097)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > 4096 {
            return None;
        }
        let state: Value = serde_json::from_slice(&bytes).ok()?;
        if state["schemaVersion"] != 1
            || state["token"] != token
            || state["client"] != client
            || state["generation"] != generation
            || state["dataDir"].as_str()? != root.to_str()?
        {
            return None;
        }
        Some(Self {
            directory,
            client,
            token,
            generation,
            lock: Mutex::new(None),
        })
    }

    pub(crate) fn initialized(&self, name: &str) {
        if name.len() > 128 || !host_matches(&self.client, name) {
            return;
        }
        let Ok(mut held) = self.lock.lock() else {
            return;
        };
        if held.is_some() {
            return;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = self
            .directory
            .join(format!("{now:032}-{}.json", std::process::id()));
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let Ok(file) = options.open(path.with_extension("lock")) else {
            return;
        };
        if file.try_lock_exclusive().is_err() {
            return;
        }
        let value = json!({"schemaVersion":1,"client":self.client,"token":self.token,"clientName":name,"generation":self.generation,"pid":std::process::id()});
        let Ok(mut receipt) = options.open(&path) else {
            return;
        };
        if receipt.write_all(value.to_string().as_bytes()).is_err() || receipt.sync_all().is_err() {
            return;
        }
        *held = Some(Session {
            lock: file,
            receipt_path: path,
        });
    }
}

fn valid_token(token: &str) -> bool {
    token.len() == 36
        && token.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

fn host_matches(client: &str, name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    match client {
        "claude" => matches!(
            name.as_str(),
            "claude" | "claude-ai" | "claude-desktop" | "claude desktop"
        ),
        "codex" => {
            name == "codex"
                || name.starts_with("codex_")
                || name.starts_with("codex-")
                || name == "codex desktop"
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf, String) {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("connection/sessions");
        std::fs::create_dir_all(&directory).unwrap();
        let root = temp.path().join("missing library");
        let token = "11111111-1111-4111-8111-111111111111".to_string();
        std::fs::write(
            directory.parent().unwrap().join("state.json"),
            json!({"schemaVersion":1,"token":token,"client":"codex","generation":"a".repeat(64),"dataDir":root}).to_string(),
        )
        .unwrap();
        (temp, directory, root, token)
    }

    #[test]
    fn configured_observer_only_writes_for_matching_initialized_host_and_holds_session_lock() {
        let (_temp, directory, root, token) = setup();
        let observer = Observer::configured(
            &root,
            directory.clone(),
            "codex".into(),
            token,
            "a".repeat(64),
        )
        .unwrap();
        observer.initialized("nero-self-check");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
        observer.initialized("codex");
        let path = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "lock")
            })
            .unwrap()
            .path();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        assert!(file.try_lock_exclusive().is_err());
        drop(observer);
        assert!(file.try_lock_exclusive().is_ok());
        assert!(!root.exists());
    }

    #[test]
    fn rejects_missing_registration_and_mismatched_library_or_token_without_creating_anything() {
        let (_temp, directory, root, token) = setup();
        assert!(Observer::configured(
            &root,
            directory.clone(),
            "codex".into(),
            "bad".into(),
            "a".repeat(64)
        )
        .is_none());
        assert!(Observer::configured(
            &root.join("other"),
            directory.clone(),
            "codex".into(),
            token.clone(),
            "a".repeat(64)
        )
        .is_none());
        assert!(Observer::configured(
            &root,
            directory.join("missing"),
            "codex".into(),
            token,
            "a".repeat(64)
        )
        .is_none());
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
        assert!(!root.exists());
    }
}
