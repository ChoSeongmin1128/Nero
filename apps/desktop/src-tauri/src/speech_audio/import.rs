use super::formats::validate_import;
use crate::processing::{cancelled, directory, message, plain_file};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub(crate) struct ImportedAudio {
    pub(crate) path: PathBuf,
    pub(crate) title: String,
}

// Only stages the original file. Meeting creation and job commits belong to Processing/Core.
pub(crate) fn stage_import(
    source: &Path,
    work: &Path,
    cancel: &AtomicBool,
) -> Result<ImportedAudio, String> {
    cancelled(cancel)?;
    let source = source.canonicalize().map_err(message)?;
    validate_import(&source)?;
    let mut input = plain_file(&source)?;
    let before = input.metadata().map_err(message)?;
    let parent = directory(work, "source")?;
    let destination = parent.join(source.file_name().ok_or("음성 파일 이름이 필요합니다.")?);
    let mut output = tempfile::NamedTempFile::new_in(parent).map_err(message)?;
    let mut buffer = [0u8; 128 * 1024];
    let mut bytes = 0u64;
    loop {
        cancelled(cancel)?;
        let count = input.read(&mut buffer).map_err(message)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[..count]).map_err(message)?;
        bytes = bytes
            .checked_add(count as u64)
            .ok_or("음성 파일이 너무 큽니다.")?;
    }
    output.as_file().sync_all().map_err(message)?;
    let after = input.metadata().map_err(message)?;
    if bytes != before.len()
        || after.len() != before.len()
        || after.modified().ok() != before.modified().ok()
    {
        return Err("가져오는 동안 파일이 변경됐습니다. 다시 선택해 주세요.".into());
    }
    cancelled(cancel)?;
    output.persist_noclobber(&destination).map_err(message)?;
    Ok(ImportedAudio {
        path: destination,
        title: source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("가져온 회의")
            .chars()
            .take(200)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_original_without_overwriting_existing_job_files() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("Meeting.M4A");
        std::fs::write(&source, b"original bytes").unwrap();
        let work = root.path().join("job");
        std::fs::create_dir(&work).unwrap();
        let cancel = AtomicBool::new(false);
        let imported = stage_import(&source, &work, &cancel).unwrap();
        assert_eq!(imported.title, "Meeting");
        assert_eq!(std::fs::read(&imported.path).unwrap(), b"original bytes");
        std::fs::write(&source, b"replacement").unwrap();
        assert!(stage_import(&source, &work, &cancel).is_err());
        assert_eq!(std::fs::read(&imported.path).unwrap(), b"original bytes");
        assert_eq!(std::fs::read_dir(work.join("source")).unwrap().count(), 1);
    }

    #[test]
    fn cancelled_import_does_not_publish_a_file() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("audio.wav");
        std::fs::write(&source, b"original bytes").unwrap();
        assert!(stage_import(&source, root.path(), &AtomicBool::new(true)).is_err());
        assert!(!root.path().join("source").exists());
        assert_eq!(std::fs::read(source).unwrap(), b"original bytes");
    }
}
