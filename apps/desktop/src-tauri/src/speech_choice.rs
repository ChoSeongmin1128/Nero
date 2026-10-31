use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpeechEngine {
    #[default]
    Nemotron,
    WhisperTurbo,
}

impl SpeechEngine {
    pub(super) const ALL: [Self; 2] = [Self::Nemotron, Self::WhisperTurbo];
    pub(super) fn manifest(self) -> &'static str {
        match self {
            Self::Nemotron => crate::asr::MANIFEST,
            Self::WhisperTurbo => crate::whisper_asr::MANIFEST,
        }
    }
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Nemotron => crate::asr::LABEL,
            Self::WhisperTurbo => "WhisperTurbo",
        }
    }
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Nemotron => "Nemotron 3.5 · 기본",
            Self::WhisperTurbo => "Whisper large-v3-turbo · 구간 처리",
        }
    }
    pub(super) fn description(self) -> &'static str {
        match self {
            Self::Nemotron => "기본 선택. 녹음 중 이어서 처리해 종료 후 대기를 줄입니다. Windows CPU 경로의 메모리 사용을 줄였습니다.",
            Self::WhisperTurbo => if cfg!(windows) {
                "한국어를 포함한 언어별 결과가 Nemotron과 다를 수 있습니다. Windows에서는 CPU로 구간별 처리하며, 더 많은 메모리와 처리 시간이 필요할 수 있습니다. 녹음 종료 후에도 추가로 기다릴 수 있습니다."
            } else {
                "한국어를 포함한 언어별 결과가 Nemotron과 다를 수 있습니다. 음성을 모아 구간별로 처리하며, 처리 장치와 회의 길이에 따라 녹음 종료 후 추가로 기다릴 수 있습니다."
            },
        }
    }
}

#[derive(Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpeechChoice {
    pub id: SpeechEngine,
    pub label: &'static str,
    pub description: &'static str,
    pub model_bytes: u64,
    pub ready: bool,
    pub supported: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Preference {
    schema_version: u32,
    engine: SpeechEngine,
}

pub(super) fn load(root: &Path) -> Result<SpeechEngine, String> {
    if let Ok(meta) = std::fs::symlink_metadata(root.join("preferences")) {
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(invalid());
        }
    }
    let file = root.join("preferences/speech.json");
    if !file.exists() && !file.is_symlink() {
        return Ok(SpeechEngine::default());
    }
    let mut raw = Vec::new();
    crate::processing::plain_file(&file)?
        .take(4097)
        .read_to_end(&mut raw)
        .map_err(|_| invalid())?;
    if raw.len() > 4096 {
        return Err(invalid());
    }
    let preference: Preference = serde_json::from_slice(&raw).map_err(|_| invalid())?;
    if preference.schema_version != 1 {
        return Err(invalid());
    }
    Ok(preference.engine)
}

pub(super) fn save(root: &Path, engine: SpeechEngine) -> Result<(), String> {
    let directory = crate::processing::directory(root, "preferences")?;
    let destination = directory.join("speech.json");
    if destination.exists() || destination.is_symlink() {
        crate::processing::plain_file(&destination)?;
    }
    let mut file = tempfile::NamedTempFile::new_in(&directory).map_err(|_| invalid())?;
    serde_json::to_writer(
        &mut file,
        &Preference {
            schema_version: 1,
            engine,
        },
    )
    .map_err(|_| invalid())?;
    file.write_all(b"\n")
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| invalid())?;
    file.persist(&destination).map_err(|_| invalid())?;
    Ok(())
}

fn invalid() -> String {
    "음성 인식 모델 설정을 읽거나 저장하지 못했습니다. 설정에서 모델을 다시 선택해 주세요.".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_preference_does_not_create_files_and_explicit_choices_persist() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("absent");
        assert_eq!(load(&root).unwrap(), SpeechEngine::Nemotron);
        assert!(!root.exists());
        std::fs::create_dir(&root).unwrap();
        save(&root, SpeechEngine::WhisperTurbo).unwrap();
        assert_eq!(load(&root).unwrap(), SpeechEngine::WhisperTurbo);
        save(&root, SpeechEngine::Nemotron).unwrap();
        assert_eq!(load(&root).unwrap(), SpeechEngine::Nemotron);
    }
    #[test]
    fn invalid_preferences_are_reported_and_not_replaced_by_a_read() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("preferences")).unwrap();
        let path = root.join("preferences/speech.json");
        let raw = br#"{"schemaVersion":1,"engine":"unknown"}"#;
        std::fs::write(&path, raw).unwrap();
        assert!(load(root).is_err());
        assert_eq!(std::fs::read(path).unwrap(), raw);
    }
}
