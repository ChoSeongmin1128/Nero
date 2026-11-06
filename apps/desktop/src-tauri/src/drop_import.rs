// Audio files dropped on the main window. Native file drop stays off: on macOS and
// Windows it would capture every drag, including moving meetings into folders. The
// webview streams the dropped file in bounded chunks into an app cache copy, and the
// ordinary import job copies it into the meeting before the cache copy is removed.
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// Each IPC chunk is bounded so neither side holds a whole recording in memory.
pub(super) const MAX_CHUNK: usize = 8 * 1024 * 1024;
const MAX_FILE: u64 = 16 * 1024 * 1024 * 1024;

struct Upload {
    directory: PathBuf,
    path: PathBuf,
    file: File,
    expected: u64,
    written: u64,
    target: Option<nero_core::DraftAttachmentTarget>,
}

pub(super) struct Finished {
    pub(super) path: PathBuf,
    pub(super) directory: PathBuf,
    pub(super) target: Option<nero_core::DraftAttachmentTarget>,
}

pub(super) struct DropImports {
    root: PathBuf,
    uploads: Mutex<HashMap<String, Upload>>,
}

fn failure() -> String {
    "끌어 놓은 음성 파일을 받지 못했습니다. 음성 가져오기로 다시 시도해 주세요.".into()
}

// Only a plain file name with an importable extension; never a path.
fn file_name(name: &str) -> Result<&str, String> {
    let extension = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    if name.is_empty()
        || name.len() > 255
        || name.contains(['/', '\\', ':', '\0'])
        || matches!(name, "." | "..")
        || !extension.is_some_and(|extension| {
            crate::speech_audio::import_extensions().contains(&extension.as_str())
        })
    {
        return Err("가져올 수 있는 음성 파일이 아닙니다.".into());
    }
    Ok(name)
}

impl DropImports {
    // Earlier sessions' leftovers are this app's own cache copies, never user files.
    pub(super) fn new(root: PathBuf) -> Self {
        if root.is_dir() && !root.is_symlink() {
            let _ = fs::remove_dir_all(&root);
        }
        Self {
            root,
            uploads: Mutex::new(HashMap::new()),
        }
    }

    pub(super) fn begin(
        &self,
        name: &str,
        size: u64,
        target: Option<nero_core::DraftAttachmentTarget>,
    ) -> Result<String, String> {
        let name = file_name(name)?;
        // One drop at a time: anything still open was abandoned by a reload or a failure.
        for (_, upload) in self.uploads.lock().unwrap().drain() {
            let _ = fs::remove_dir_all(upload.directory);
        }
        if size == 0 || size > MAX_FILE {
            return Err("가져올 수 있는 크기의 음성 파일이 아닙니다.".into());
        }
        let token = uuid::Uuid::new_v4().to_string();
        let directory = self.root.join(&token);
        fs::create_dir_all(&directory).map_err(|_| failure())?;
        let path = directory.join(name);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| failure())?;
        self.uploads.lock().unwrap().insert(
            token.clone(),
            Upload {
                directory,
                path,
                file,
                expected: size,
                written: 0,
                target,
            },
        );
        Ok(token)
    }

    pub(super) fn chunk(&self, token: &str, bytes: &[u8]) -> Result<(), String> {
        let mut uploads = self.uploads.lock().unwrap();
        let upload = uploads.get_mut(token).ok_or_else(failure)?;
        let written = upload.written + bytes.len() as u64;
        if bytes.is_empty() || bytes.len() > MAX_CHUNK || written > upload.expected {
            let upload = uploads.remove(token).unwrap();
            let _ = fs::remove_dir_all(upload.directory);
            return Err(failure());
        }
        if upload.file.write_all(bytes).is_err() {
            let upload = uploads.remove(token).unwrap();
            let _ = fs::remove_dir_all(upload.directory);
            return Err(
                "끌어 놓은 음성 파일을 저장하지 못했습니다. 저장 공간을 확인해 주세요.".into(),
            );
        }
        upload.written = written;
        Ok(())
    }

    pub(super) fn finish(&self, token: &str) -> Result<Finished, String> {
        let upload = self
            .uploads
            .lock()
            .unwrap()
            .remove(token)
            .ok_or_else(failure)?;
        if upload.written != upload.expected || upload.file.sync_all().is_err() {
            let _ = fs::remove_dir_all(&upload.directory);
            return Err(failure());
        }
        Ok(Finished {
            path: upload.path,
            directory: upload.directory,
            target: upload.target,
        })
    }

    pub(super) fn cancel(&self, token: &str) {
        if let Some(upload) = self.uploads.lock().unwrap().remove(token) {
            let _ = fs::remove_dir_all(upload.directory);
        }
    }
}

// Removes a finished upload's cache copy when the import job is dropped, whether it
// completed, failed or was cancelled before it ran.
pub(super) struct RemoveOnDrop(pub(super) PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploads_accept_only_plain_audio_names_and_exact_sizes() {
        let temporary = tempfile::tempdir().unwrap();
        let drops = DropImports::new(temporary.path().join("dropped-audio"));
        for name in ["../a.m4a", "a/b.m4a", "notes.txt", "", "a.m4a\0"] {
            assert!(drops.begin(name, 3, None).is_err(), "{name}");
        }
        assert!(drops.begin("회의.m4a", 0, None).is_err());
        let token = drops.begin("회의.m4a", 3, None).unwrap();
        drops.chunk(&token, b"ab").unwrap();
        assert!(drops.finish(&token).is_err(), "short upload");
        let token = drops.begin("회의.m4a", 3, None).unwrap();
        assert!(drops.chunk(&token, b"abcd").is_err(), "overflow");
        assert!(drops.finish(&token).is_err(), "removed after overflow");
        let token = drops.begin("회의.wav", 3, None).unwrap();
        drops.chunk(&token, b"abc").unwrap();
        let finished = drops.finish(&token).unwrap();
        assert_eq!(fs::read(&finished.path).unwrap(), b"abc");
        assert_eq!(finished.path.file_name().unwrap(), "회의.wav");
        drop(RemoveOnDrop(finished.directory.clone()));
        assert!(!finished.directory.exists());
    }

    #[test]
    fn cancelled_and_stale_uploads_leave_nothing_behind() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("dropped-audio");
        let drops = DropImports::new(root.clone());
        let token = drops.begin("회의.mp3", 10, None).unwrap();
        drops.chunk(&token, b"abc").unwrap();
        drops.cancel(&token);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        drops.begin("남은.mp3", 10, None).unwrap();
        drop(drops);
        DropImports::new(root.clone());
        assert!(!root.exists());
    }
}
