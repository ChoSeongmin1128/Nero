use crate::processing::{ModelSpec, ModelStage};
use serde::Deserialize;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Source {
    schema_version: u32,
    root: PathBuf,
}

pub(super) fn shared(root: &Path, spec: ModelSpec) -> Result<Option<PathBuf>, String> {
    if !nero_core::environment::CURRENT.development || spec.stage == ModelStage::Summary {
        return Ok(None);
    }
    let config = root.join("preferences/model-source.json");
    if !config.exists() && !config.is_symlink() {
        return Ok(None);
    }
    let invalid =
        || "공통 모델 경로를 확인하지 못했습니다. 기존 모델은 변경하지 않았습니다.".to_string();
    crate::path_safety::safe_ancestors(&config).map_err(|_| invalid())?;
    let mut raw = Vec::new();
    crate::processing::plain_file(&config)?
        .take(4097)
        .read_to_end(&mut raw)
        .map_err(|_| invalid())?;
    if raw.len() > 4096 {
        return Err(invalid());
    }
    let source: Source = serde_json::from_slice(&raw).map_err(|_| invalid())?;
    if source.schema_version != 1
        || !source.root.is_absolute()
        || source
            .root
            .components()
            .any(|p| matches!(p, Component::ParentDir))
    {
        return Err(invalid());
    }
    let model: serde_json::Value = serde_json::from_str(spec.manifest).map_err(|_| invalid())?;
    let path = source.root.join("models").join(format!(
        "{}-{}",
        spec.label,
        model["revision"].as_str().ok_or_else(invalid)?
    ));
    if !path.exists() {
        return Ok(None);
    }
    crate::path_safety::safe_ancestors(&path).map_err(|_| invalid())?;
    if path.canonicalize().map_err(|_| invalid())? != path || !path.is_dir() {
        return Err(invalid());
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speech_choice::SpeechEngine;
    #[test]
    fn explicit_shared_models_are_read_only_and_no_library_is_opened() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let app = root.join("app");
        let shared_root = root.join("shared");
        std::fs::create_dir_all(app.join("preferences")).unwrap();
        let spec = ModelSpec::speech(SpeechEngine::Moss);
        let dir =
            shared_root
                .join("models")
                .join(format!("{}-{}", spec.label, spec.model().revision));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            app.join("preferences/model-source.json"),
            serde_json::json!({"schemaVersion":1,"root":shared_root}).to_string(),
        )
        .unwrap();
        assert_eq!(shared(&app, spec).unwrap(), Some(dir));
        assert!(!shared_root.join("library.json").exists());
        assert!(!shared_root.join("preferences").exists());
    }
}
