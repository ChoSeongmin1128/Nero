use fs2::FileExt;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_CHECKPOINT_BYTES: u64 = 16 * 1024 * 1024;

pub(super) struct Checkpoints {
    directory: PathBuf,
    identity: String,
}

impl Checkpoints {
    pub(super) fn open(directory: &Path, identity: &Value) -> Result<Self, String> {
        plain_directory(directory)?;
        let identity = digest(identity)?;
        let path = directory.join(&identity);
        match fs::create_dir(&path) {
            Ok(()) => sync_directory(directory)?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.to_string()),
        }
        plain_directory(&path)?;
        Ok(Self {
            directory: path,
            identity,
        })
    }

    pub(super) fn load(&self, key: &Value) -> Result<Option<Value>, String> {
        plain_directory(&self.directory)?;
        let request = digest(key)?;
        let path = self.directory.join(format!("{request}.json"));
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("요약 작업 저장 경로에 일반 파일이 아닌 항목이 있습니다.".into());
        }
        if metadata.len() > MAX_CHECKPOINT_BYTES {
            return Ok(None);
        }
        let file = crate::processing::plain_file(&path)?;
        let mut bytes = Vec::new();
        file.take(MAX_CHECKPOINT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_CHECKPOINT_BYTES {
            return Ok(None);
        }
        let Ok(envelope) = serde_json::from_slice::<Value>(&bytes) else {
            return Ok(None);
        };
        let Some(response) = envelope.get("response") else {
            return Ok(None);
        };
        if envelope.get("schemaVersion").and_then(Value::as_u64) != Some(1)
            || envelope.get("identity").and_then(Value::as_str) != Some(self.identity.as_str())
            || envelope.get("request").and_then(Value::as_str) != Some(request.as_str())
            || !complete(response)
            || envelope.get("responseSha256").and_then(Value::as_str)
                != Some(digest(response)?.as_str())
        {
            return Ok(None);
        }
        Ok(Some(response.clone()))
    }

    pub(super) fn save(&self, key: &Value, response: &Value) -> Result<(), String> {
        if !complete(response) {
            return Err("완료되지 않은 요약 응답은 저장할 수 없습니다.".into());
        }
        plain_directory(&self.directory)?;
        let _lock = self.writer_lock()?;
        if self.load(key)?.is_some() {
            return Ok(());
        }
        let request = digest(key)?;
        let envelope = json!({
            "schemaVersion": 1,
            "identity": self.identity,
            "request": request,
            "responseSha256": digest(response)?,
            "response": response,
        });
        let bytes = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_CHECKPOINT_BYTES {
            return Err("요약 작업 응답이 저장 한도를 넘었습니다.".into());
        }
        let path = self.directory.join(format!("{request}.json"));
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err("요약 작업 저장 경로에 일반 파일이 아닌 항목이 있습니다.".into());
                }
                let preserved = self
                    .directory
                    .join(format!("invalid-{request}-{}.json", uuid::Uuid::new_v4()));
                fs::rename(&path, preserved).map_err(|error| error.to_string())?;
                sync_directory(&self.directory)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.to_string()),
        }
        let mut pending = tempfile::Builder::new()
            .prefix(".pending-")
            .tempfile_in(&self.directory)
            .map_err(|error| error.to_string())?;
        pending
            .write_all(&bytes)
            .and_then(|()| pending.as_file().sync_all())
            .map_err(|error| error.to_string())?;
        match pending.persist_noclobber(&path) {
            Ok(_) => sync_directory(&self.directory),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                if self.load(key)?.is_some() {
                    Ok(())
                } else {
                    Err("요약 작업 결과를 안전하게 저장하지 못했습니다.".into())
                }
            }
            Err(error) => Err(error.error.to_string()),
        }
    }

    fn writer_lock(&self) -> Result<File, String> {
        let path = self.directory.join(".writer-lock");
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err("요약 작업 잠금 경로가 올바르지 않습니다.".into())
            }
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.to_string()),
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|error| error.to_string())?;
        file.lock_exclusive().map_err(|error| error.to_string())?;
        Ok(file)
    }
}

fn complete(response: &Value) -> bool {
    let Some(choices) = response.get("choices").and_then(Value::as_array) else {
        return false;
    };
    choices.len() == 1
        && choices[0].get("finish_reason").and_then(Value::as_str) == Some("stop")
        && choices[0]
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .is_some_and(|content| !content.trim().is_empty() && !content.contains('\0'))
}

fn digest(value: &Value) -> Result<String, String> {
    let bytes = serde_json::to_vec(&canonical(value)).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(values) => {
            let ordered: BTreeMap<_, _> = values.iter().collect();
            Value::Object(
                ordered
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonical(value)))
                    .collect::<Map<_, _>>(),
            )
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
        _ => value.clone(),
    }
}

fn plain_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("요약 작업 폴더에 연결된 경로나 다른 항목이 있습니다.".into());
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| error.to_string())?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> Value {
        json!({"source": "revision-a", "model": "pinned-model", "promptVersion": "v1"})
    }

    fn key() -> Value {
        json!({"stage": "evidence", "user": "[1] 합성 자료", "limit": 2048})
    }

    fn response(text: &str) -> Value {
        json!({"choices": [{"finish_reason": "stop", "message": {"content": text}}], "usage": {"completion_tokens": 10}})
    }

    #[test]
    fn reopening_reuses_only_the_same_source_and_request() {
        let temporary = tempfile::tempdir().unwrap();
        let checkpoints = Checkpoints::open(temporary.path(), &identity()).unwrap();
        let original = response("완료된 자료 [1]");
        checkpoints.save(&key(), &original).unwrap();
        let reopened = Checkpoints::open(temporary.path(), &identity()).unwrap();
        assert_eq!(reopened.load(&key()).unwrap(), Some(original));
        let changed = json!({"stage": "evidence", "user": "[1] 다른 자료", "limit": 2048});
        assert_eq!(reopened.load(&changed).unwrap(), None);
        let changed_source = Checkpoints::open(
            temporary.path(),
            &json!({"source": "revision-b", "model": "pinned-model", "promptVersion": "v1"}),
        )
        .unwrap();
        assert_eq!(changed_source.load(&key()).unwrap(), None);
        let changed_budget = json!({"stage": "evidence", "user": "[1] 합성 자료", "limit": 4096});
        assert_eq!(reopened.load(&changed_budget).unwrap(), None);
    }

    #[test]
    fn completed_checkpoint_is_immutable() {
        let temporary = tempfile::tempdir().unwrap();
        let checkpoints = Checkpoints::open(temporary.path(), &identity()).unwrap();
        let original = response("첫 완결 결과 [1]");
        checkpoints.save(&key(), &original).unwrap();
        let path = checkpoints
            .directory
            .join(format!("{}.json", digest(&key()).unwrap()));
        let bytes = fs::read(&path).unwrap();
        checkpoints
            .save(&key(), &response("후속 결과 [1]"))
            .unwrap();
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert_eq!(checkpoints.load(&key()).unwrap(), Some(original));
    }

    #[test]
    fn incomplete_and_empty_outputs_cannot_be_saved() {
        let temporary = tempfile::tempdir().unwrap();
        let checkpoints = Checkpoints::open(temporary.path(), &identity()).unwrap();
        for invalid in [
            json!({"choices": [{"finish_reason": "length", "message": {"content": "부분"}}]}),
            json!({"choices": [{"message": {"content": "종료 정보 없음"}}]}),
            response("  \n"),
            response("본문\0"),
            json!({"choices": []}),
        ] {
            assert!(checkpoints.save(&key(), &invalid).is_err());
            assert_eq!(checkpoints.load(&key()).unwrap(), None);
        }
    }

    #[test]
    fn corrupted_checkpoint_is_not_reused_and_is_preserved_on_replacement() {
        let temporary = tempfile::tempdir().unwrap();
        let checkpoints = Checkpoints::open(temporary.path(), &identity()).unwrap();
        let path = checkpoints
            .directory
            .join(format!("{}.json", digest(&key()).unwrap()));
        fs::write(&path, b"{unfinished").unwrap();
        assert_eq!(checkpoints.load(&key()).unwrap(), None);
        let original = response("검증된 새 결과 [1]");
        checkpoints.save(&key(), &original).unwrap();
        assert_eq!(checkpoints.load(&key()).unwrap(), Some(original));
        let preserved: Vec<_> = fs::read_dir(&checkpoints.directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("invalid-")
            })
            .collect();
        assert_eq!(preserved.len(), 1);
        assert_eq!(fs::read(&preserved[0]).unwrap(), b"{unfinished");
    }

    #[test]
    fn response_tampering_is_detected_by_the_receipt() {
        let temporary = tempfile::tempdir().unwrap();
        let checkpoints = Checkpoints::open(temporary.path(), &identity()).unwrap();
        checkpoints
            .save(&key(), &response("원래 결과 [1]"))
            .unwrap();
        let path = checkpoints
            .directory
            .join(format!("{}.json", digest(&key()).unwrap()));
        let bytes = fs::read(&path).unwrap();
        let mut envelope: Value = serde_json::from_slice(&bytes).unwrap();
        envelope["response"] = response("다른 내용 [1]");
        fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        assert_eq!(checkpoints.load(&key()).unwrap(), None);
        envelope = serde_json::from_slice(&bytes).unwrap();
        envelope["identity"] = json!("다른 원문");
        fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        assert_eq!(checkpoints.load(&key()).unwrap(), None);
    }

    #[test]
    fn abandoned_temporary_files_are_never_published_as_success() {
        let temporary = tempfile::tempdir().unwrap();
        let checkpoints = Checkpoints::open(temporary.path(), &identity()).unwrap();
        fs::write(
            checkpoints.directory.join(".pending-interrupted"),
            b"partial",
        )
        .unwrap();
        assert_eq!(checkpoints.load(&key()).unwrap(), None);
        let expected = response("완료 [1]");
        checkpoints.save(&key(), &expected).unwrap();
        assert_eq!(checkpoints.load(&key()).unwrap(), Some(expected));
    }

    #[test]
    fn equivalent_object_key_order_has_the_same_identity() {
        let first: Value = serde_json::from_str(r#"{"b":{"z":2,"a":1},"a":0}"#).unwrap();
        let second: Value = serde_json::from_str(r#"{"a":0,"b":{"a":1,"z":2}}"#).unwrap();
        assert_eq!(digest(&first).unwrap(), digest(&second).unwrap());
    }

    #[test]
    fn concurrent_saves_publish_one_complete_immutable_response() {
        let temporary = tempfile::tempdir().unwrap();
        let checkpoints =
            std::sync::Arc::new(Checkpoints::open(temporary.path(), &identity()).unwrap());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let workers: Vec<_> = (0..4)
            .map(|number| {
                let checkpoints = checkpoints.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    checkpoints.save(&key(), &response(&format!("완결 결과 {number} [1]")))
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }
        let saved = checkpoints.load(&key()).unwrap().unwrap();
        assert!(complete(&saved));
        let published: Vec<_> = fs::read_dir(&checkpoints.directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect();
        assert_eq!(published.len(), 1);
    }

    #[test]
    fn request_paths_remain_hashed_data() {
        let temporary = tempfile::tempdir().unwrap();
        let checkpoints = Checkpoints::open(temporary.path(), &identity()).unwrap();
        let request = json!({"stage": "../../outside", "user": "/absolute/path"});
        checkpoints.save(&request, &response("완료 [1]")).unwrap();
        assert!(checkpoints.load(&request).unwrap().is_some());
        assert!(checkpoints
            .directory
            .join(format!("{}.json", digest(&request).unwrap()))
            .is_file());
        assert!(!temporary.path().join("outside").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_paths_are_rejected_without_modifying_the_target() {
        use std::os::unix::fs::symlink;
        let temporary = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let alias = temporary.path().join("alias");
        symlink(external.path(), &alias).unwrap();
        assert!(Checkpoints::open(&alias, &identity()).is_err());
        let checkpoints = Checkpoints::open(temporary.path(), &identity()).unwrap();
        let target = external.path().join("private.json");
        fs::write(&target, b"unchanged").unwrap();
        symlink(
            &target,
            checkpoints
                .directory
                .join(format!("{}.json", digest(&key()).unwrap())),
        )
        .unwrap();
        assert!(checkpoints.load(&key()).is_err());
        assert!(checkpoints.save(&key(), &response("완료 [1]")).is_err());
        assert_eq!(fs::read(target).unwrap(), b"unchanged");
    }

    #[test]
    fn oversize_files_are_not_loaded() {
        let temporary = tempfile::tempdir().unwrap();
        let checkpoints = Checkpoints::open(temporary.path(), &identity()).unwrap();
        let file = File::create(
            checkpoints
                .directory
                .join(format!("{}.json", digest(&key()).unwrap())),
        )
        .unwrap();
        file.set_len(MAX_CHECKPOINT_BYTES + 1).unwrap();
        assert_eq!(checkpoints.load(&key()).unwrap(), None);
    }
}
