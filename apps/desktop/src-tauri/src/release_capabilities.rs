pub const LOCAL_SUMMARY_ENABLED: bool = false;
pub const LOCAL_SUMMARY_MESSAGE: &str = "앱 내 요약은 준비 중입니다.";

pub fn require_local_summary() -> Result<(), String> {
    if LOCAL_SUMMARY_ENABLED {
        Ok(())
    } else {
        Err(LOCAL_SUMMARY_MESSAGE.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    fn inventory(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        fn visit(root: &Path, directory: &Path, entries: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
            for entry in std::fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                if entry.file_type().unwrap().is_dir() {
                    entries.insert(relative, None);
                    visit(root, &path, entries);
                } else {
                    entries.insert(relative, Some(std::fs::read(path).unwrap()));
                }
            }
        }
        let mut entries = BTreeMap::new();
        visit(root, root, &mut entries);
        entries
    }

    #[test]
    fn unavailable_local_summary_refuses_generation_and_download_before_work_starts() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let core = nero_core::CoreHandle::open(&root).unwrap();
        let process = crate::processing::Processing::new(root.clone(), root.join("resources"));
        let initial = process.snapshot();
        assert!(initial.jobs.is_empty());
        let before = inventory(&root);
        for result in [
            process.prepare_summary(),
            process.prepare_meeting_models(true),
            process.summarize_selected(core.clone(), "missing-meeting".into(), None),
        ] {
            assert_eq!(result.err().unwrap(), LOCAL_SUMMARY_MESSAGE);
        }
        let status = process.snapshot();
        assert!(!status.local_summary_enabled);
        assert_eq!(status.local_summary_message, LOCAL_SUMMARY_MESSAGE);
        assert!(status.jobs.is_empty());
        assert_eq!(inventory(&root), before);
    }
}
