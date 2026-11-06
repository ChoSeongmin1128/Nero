use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpeechEngine {
    #[default]
    Nemotron,
    WhisperTurbo,
    WhisperLargeV3,
    AppleSpeech,
    QwenKo,
}

impl SpeechEngine {
    pub(super) const ALL: [Self; 5] = [
        Self::Nemotron,
        Self::QwenKo,
        Self::WhisperTurbo,
        Self::WhisperLargeV3,
        Self::AppleSpeech,
    ];
    // Every engine whose models a selection needs, itself first. Qwen also needs the
    // Nemotron model for its recording preview.
    pub(super) fn requirements(self) -> Vec<Self> {
        if self.live_asr() == self {
            vec![self]
        } else {
            vec![self, self.live_asr()]
        }
    }
    // The recognizer streamed during recording. Qwen previews with Nemotron and
    // produces the saved transcript after recording ends.
    pub(super) fn live_asr(self) -> Self {
        match self {
            Self::QwenKo => Self::Nemotron,
            other => other,
        }
    }
    pub(super) fn manifest(self) -> &'static str {
        match self {
            Self::Nemotron => crate::asr::MANIFEST,
            Self::WhisperTurbo => crate::whisper_asr::MANIFEST,
            Self::WhisperLargeV3 => crate::whisper_asr::LARGE_MANIFEST,
            Self::AppleSpeech => crate::apple_speech::MANIFEST,
            Self::QwenKo => crate::qwen_asr::MANIFEST,
        }
    }
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Nemotron => crate::asr::LABEL,
            Self::WhisperTurbo => "WhisperTurbo",
            Self::WhisperLargeV3 => "WhisperLargeV3",
            Self::AppleSpeech => "AppleSpeech",
            Self::QwenKo => "QwenKo",
        }
    }
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Nemotron => "Nemotron 3.5 · 기본",
            Self::WhisperTurbo => "Whisper large-v3-turbo · 구간 처리",
            Self::WhisperLargeV3 => "Whisper large-v3 · 구간 처리",
            Self::AppleSpeech => "macOS 기본 전사",
            Self::QwenKo => "Qwen3-ASR 한국어 · 녹음 후 전사",
        }
    }
    pub(super) fn description(self) -> &'static str {
        match self {
            Self::QwenKo => "녹음 중에는 Nemotron으로 미리 보여 주고, 녹음이 끝나면 한국어 모델로 다시 전사해 저장합니다. 녹음 종료 후 추가로 기다려야 하며 Nemotron 모델도 함께 준비합니다.",
            Self::AppleSpeech => "녹음 종료 후 macOS에서 한국어를 전사합니다. Nero 전용 음성 인식 모델이 필요하지 않습니다. 준비 시 시스템 언어 자산을 다운로드할 수 있습니다.",
            Self::Nemotron => "녹음과 함께 음성 기록을 준비합니다. 지원되는 GPU를 우선 사용하고 CPU로도 처리할 수 있습니다.",
            Self::WhisperLargeV3 => "고사양 Mac용 큰 모델입니다. 모델 다운로드와 구간별 처리에 더 많은 저장 공간과 메모리, 시간이 필요합니다.",
            Self::WhisperTurbo => if cfg!(windows) {
                "한국어를 포함한 언어별 결과가 Nemotron과 다를 수 있습니다. Windows에서는 CPU로 구간별 처리하며, 더 많은 메모리와 처리 시간이 필요할 수 있습니다. 녹음 종료 후에도 추가로 기다릴 수 있습니다."
            } else {
                "한국어를 포함한 언어별 결과가 Nemotron과 다를 수 있습니다. 음성을 모아 구간별로 처리하며, 처리 장치와 회의 길이에 따라 녹음 종료 후 추가로 기다릴 수 있습니다."
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppleVariant {
    SpeechTranscriber,
    DictationTranscriber,
}
impl AppleVariant {
    pub(super) fn argument(self) -> &'static str {
        match self {
            Self::SpeechTranscriber => "speechTranscriber",
            Self::DictationTranscriber => "dictationTranscriber",
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
    pub recommended: bool,
    pub recommendation_reason: Option<&'static str>,
    pub memory_warning: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Preference {
    schema_version: u32,
    pub(super) engine: SpeechEngine,
    #[serde(default = "enabled")]
    pub(super) speaker_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) apple_speech_variant: Option<AppleVariant>,
}

pub(super) fn enabled() -> bool {
    true
}

pub(super) fn load(root: &Path) -> Result<Preference, String> {
    if let Ok(meta) = std::fs::symlink_metadata(root.join("preferences")) {
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(invalid());
        }
    }
    let file = root.join("preferences/speech.json");
    if !file.exists() && !file.is_symlink() {
        return Ok(Preference {
            schema_version: 1,
            engine: SpeechEngine::default(),
            speaker_enabled: true,
            apple_speech_variant: None,
        });
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
    if preference.schema_version != 1
        || (preference.engine == SpeechEngine::AppleSpeech
            && preference.apple_speech_variant.is_none())
    {
        return Err(invalid());
    }
    Ok(preference)
}

pub(super) fn save(
    root: &Path,
    engine: SpeechEngine,
    speaker_enabled: bool,
    apple_speech_variant: Option<AppleVariant>,
) -> Result<(), String> {
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
            speaker_enabled,
            apple_speech_variant,
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
        assert_eq!(load(&root).unwrap().engine, SpeechEngine::Nemotron);
        assert!(!root.exists());
        std::fs::create_dir(&root).unwrap();
        save(&root, SpeechEngine::WhisperTurbo, true, None).unwrap();
        assert_eq!(load(&root).unwrap().engine, SpeechEngine::WhisperTurbo);
        save(&root, SpeechEngine::Nemotron, true, None).unwrap();
        assert_eq!(load(&root).unwrap().engine, SpeechEngine::Nemotron);
    }
    #[test]
    fn existing_preferences_keep_speakers_and_disabled_choice_persists() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("preferences")).unwrap();
        std::fs::write(
            root.join("preferences/speech.json"),
            br#"{"schemaVersion":1,"engine":"nemotron"}"#,
        )
        .unwrap();
        assert!(load(root).unwrap().speaker_enabled);
        save(
            root,
            SpeechEngine::AppleSpeech,
            false,
            Some(AppleVariant::DictationTranscriber),
        )
        .unwrap();
        let selected = load(root).unwrap();
        assert_eq!(selected.engine, SpeechEngine::AppleSpeech);
        assert!(!selected.speaker_enabled);
    }
    #[test]
    fn session_selection_is_immutable_and_does_not_rewrite_current_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let session = uuid::Uuid::new_v4().to_string();
        assert!(load_session(root, &session).unwrap().is_none());
        freeze_session(
            root,
            &session,
            SpeechEngine::AppleSpeech,
            false,
            Some(AppleVariant::DictationTranscriber),
        )
        .unwrap();
        save(root, SpeechEngine::Nemotron, true, None).unwrap();
        let existing = freeze_session(root, &session, SpeechEngine::Nemotron, true, None).unwrap();
        assert_eq!(existing.engine, SpeechEngine::AppleSpeech);
        assert!(!existing.speaker_enabled);
        assert!(load(root).unwrap().speaker_enabled);
        assert!(load_session(root, "../other").is_err());
    }
    #[test]
    fn build62_session_choices_are_read_and_new_sessions_record_the_glossary_requirement() {
        let root = tempfile::tempdir().unwrap();
        let session = uuid::Uuid::new_v4().to_string();
        let frozen =
            freeze_session(root.path(), &session, SpeechEngine::Nemotron, true, None).unwrap();
        assert!(frozen.glossary_snapshot_required);
        assert!(
            load_session(root.path(), &session)
                .unwrap()
                .unwrap()
                .glossary_snapshot_required
        );
        // Exactly as build62 (0.0.1 public) wrote it, before the requirement existed.
        let legacy = uuid::Uuid::new_v4().to_string();
        std::fs::write(
            session_file(root.path(), &legacy).unwrap(),
            br#"{"schemaVersion":1,"engine":"nemotron","speakerEnabled":true}"#,
        )
        .unwrap();
        assert!(
            !load_session(root.path(), &legacy)
                .unwrap()
                .unwrap()
                .glossary_snapshot_required
        );
    }
    #[cfg(unix)]
    #[test]
    fn session_choices_reject_linked_parent_directories() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("preferences")).unwrap();
        let session = uuid::Uuid::new_v4().to_string();
        assert!(load_session(root.path(), &session).is_err());
        assert!(
            freeze_session(root.path(), &session, SpeechEngine::Nemotron, false, None).is_err()
        );
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
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

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SessionChoice {
    schema_version: u32,
    #[serde(default)]
    pub(super) glossary_snapshot_required: bool,
    pub(super) engine: SpeechEngine,
    pub(super) speaker_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) apple_speech_variant: Option<AppleVariant>,
}
fn session_file(root: &Path, session: &str) -> Result<std::path::PathBuf, String> {
    uuid::Uuid::parse_str(session).map_err(|_| invalid())?;
    Ok(root
        .join("preferences/capture-speech")
        .join(format!("{session}.json")))
}
pub(super) fn load_session(root: &Path, session: &str) -> Result<Option<SessionChoice>, String> {
    let path = session_file(root, session)?;
    for directory in [
        root.join("preferences"),
        root.join("preferences/capture-speech"),
    ] {
        match std::fs::symlink_metadata(directory) {
            Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => return Err(invalid()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(invalid()),
            Ok(_) => (),
        }
    }
    if !path.exists() && !path.is_symlink() {
        return Ok(None);
    }
    let mut raw = Vec::new();
    crate::processing::plain_file(&path)?
        .take(4097)
        .read_to_end(&mut raw)
        .map_err(|_| invalid())?;
    if raw.len() > 4096 {
        return Err(invalid());
    }
    let choice: SessionChoice = serde_json::from_slice(&raw).map_err(|_| invalid())?;
    if choice.schema_version != 1
        || (choice.engine == SpeechEngine::AppleSpeech && choice.apple_speech_variant.is_none())
    {
        return Err(invalid());
    }
    Ok(Some(choice))
}
pub(super) fn freeze_session(
    root: &Path,
    session: &str,
    engine: SpeechEngine,
    speaker_enabled: bool,
    apple_speech_variant: Option<AppleVariant>,
) -> Result<SessionChoice, String> {
    if let Some(choice) = load_session(root, session)? {
        return Ok(choice);
    }
    let path = session_file(root, session)?;
    let directory = crate::processing::directory(root, "preferences/capture-speech")?;
    let mut file = tempfile::NamedTempFile::new_in(directory).map_err(|_| invalid())?;
    let choice = SessionChoice {
        schema_version: 1,
        glossary_snapshot_required: true,
        engine,
        speaker_enabled,
        apple_speech_variant,
    };
    serde_json::to_writer(&mut file, &choice).map_err(|_| invalid())?;
    file.as_file().sync_all().map_err(|_| invalid())?;
    file.persist_noclobber(path).map_err(|_| invalid())?;
    Ok(choice)
}
