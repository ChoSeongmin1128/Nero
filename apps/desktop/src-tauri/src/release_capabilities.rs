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

    #[test]
    fn unavailable_local_summary_refuses_generation_and_download_before_work_starts() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let core = nero_core::CoreHandle::open(&root).unwrap();
        let process = crate::processing::Processing::new(root.clone(), root.join("resources"));
        for result in [
            process.prepare_summary(),
            process.prepare_meeting_models(true),
            process.summarize_selected(core, "missing-meeting".into(), None),
        ] {
            assert_eq!(result.err().unwrap(), LOCAL_SUMMARY_MESSAGE);
        }
        let status = process.snapshot();
        assert!(!status.local_summary_enabled);
        assert_eq!(status.local_summary_message, LOCAL_SUMMARY_MESSAGE);
        assert!(status.jobs.is_empty());
        assert!(!root.join("models").exists());
        assert!(!root.join("templates.json").exists());
    }
}
