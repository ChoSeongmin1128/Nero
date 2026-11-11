// Engine switches that wait for their models, and the one-time switch offer.
//
// Both live in their own preference files, so `preferences/speech.json` keeps holding only
// the active choice and changes only at the moment of the switch.
use crate::speech_choice::{AppleVariant, SpeechEngine};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

// The offer shown to existing users once: Qwen3-ASR-ko for the final transcript.
pub(super) const MOSS_OFFER: &str = "moss-independent-chunks-2026-11";
const PENDING_FILE: &str = "speech-switch.json";
const OFFERS_FILE: &str = "speech-offers.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Pending {
    schema_version: u32,
    pub(super) target: SpeechEngine,
    pub(super) speaker_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) apple_speech_variant: Option<AppleVariant>,
}

impl Pending {
    pub(super) fn new(
        target: SpeechEngine,
        speaker_enabled: bool,
        apple_speech_variant: Option<AppleVariant>,
    ) -> Self {
        Self {
            schema_version: 1,
            target,
            speaker_enabled,
            apple_speech_variant,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Decision {
    Accepted,
    Declined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OfferRecord {
    decision: Decision,
    decided_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Offers {
    #[serde(default = "one")]
    schema_version: u32,
    #[serde(default)]
    decisions: BTreeMap<String, OfferRecord>,
    #[serde(skip)]
    suppressed: bool,
}

fn one() -> u32 {
    1
}

impl Default for Offers {
    fn default() -> Self {
        Self {
            schema_version: 1,
            decisions: BTreeMap::new(),
            suppressed: false,
        }
    }
}

impl Offers {
    pub(super) fn decided(&self, offer: &str) -> bool {
        self.suppressed || self.decisions.contains_key(offer)
    }
    // An unreadable record: offer nothing, and never overwrite it.
    pub(super) fn suppressed() -> Self {
        Self {
            suppressed: true,
            ..Self::default()
        }
    }
}

// What the screens need to know about a waiting switch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchView {
    pub target: SpeechEngine,
    // A download for the target is queued or running.
    pub preparing: bool,
    pub error: Option<String>,
}

// A one-time offer to move an existing setup to a newly recommended engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferView {
    pub id: &'static str,
    pub engine: SpeechEngine,
    pub title: &'static str,
    pub description: &'static str,
    pub download_bytes: u64,
    pub memory_warning: Option<String>,
}

fn invalid() -> String {
    "음성 인식 모델 전환 상태를 읽거나 저장하지 못했습니다. 설정에서 모델을 다시 선택해 주세요."
        .into()
}

fn read_small(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(invalid()),
        Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => return Err(invalid()),
        Ok(_) => (),
    }
    let mut raw = Vec::new();
    crate::processing::plain_file(path)?
        .take(65537)
        .read_to_end(&mut raw)
        .map_err(|_| invalid())?;
    if raw.len() > 65536 {
        return Err(invalid());
    }
    Ok(Some(raw))
}

// Reads never create directories or files.
fn preferences(root: &Path) -> Result<Option<std::path::PathBuf>, String> {
    let directory = root.join("preferences");
    match std::fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(invalid()),
        Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => Err(invalid()),
        Ok(_) => Ok(Some(directory)),
    }
}

pub(super) fn load_pending(root: &Path) -> Result<Option<Pending>, String> {
    let Some(directory) = preferences(root)? else {
        return Ok(None);
    };
    read_small(&directory.join(PENDING_FILE))?
        .map(|raw| {
            let pending: Pending = serde_json::from_slice(&raw).map_err(|_| invalid())?;
            if pending.schema_version != 1
                || (pending.target == SpeechEngine::AppleSpeech
                    && pending.apple_speech_variant.is_none())
            {
                return Err(invalid());
            }
            Ok(pending)
        })
        .transpose()
}

pub(super) fn load_offers(root: &Path) -> Result<Offers, String> {
    let Some(directory) = preferences(root)? else {
        return Ok(Offers::default());
    };
    let Some(raw) = read_small(&directory.join(OFFERS_FILE))? else {
        return Ok(Offers::default());
    };
    let offers: Offers = serde_json::from_slice(&raw).map_err(|_| invalid())?;
    if offers.schema_version != 1 {
        return Err(invalid());
    }
    Ok(offers)
}

fn write(root: &Path, name: &str, value: &impl Serialize) -> Result<(), String> {
    let directory = crate::processing::directory(root, "preferences")?;
    let destination = directory.join(name);
    if destination.exists() || destination.is_symlink() {
        crate::processing::plain_file(&destination)?;
    }
    let mut file = tempfile::NamedTempFile::new_in(&directory).map_err(|_| invalid())?;
    serde_json::to_writer(&mut file, value).map_err(|_| invalid())?;
    file.write_all(b"\n")
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| invalid())?;
    file.persist(&destination).map_err(|_| invalid())?;
    Ok(())
}

pub(super) fn save_pending(root: &Path, pending: &Pending) -> Result<(), String> {
    write(root, PENDING_FILE, pending)
}

pub(super) fn clear_pending(root: &Path) -> Result<(), String> {
    let Some(directory) = preferences(root)? else {
        return Ok(());
    };
    let path = directory.join(PENDING_FILE);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            std::fs::remove_file(path).map_err(|_| invalid())
        }
        _ => Err(invalid()),
    }
}

pub(super) fn decide(
    root: &Path,
    offers: &mut Offers,
    offer: &str,
    decision: Decision,
    now: u64,
) -> Result<(), String> {
    let mut next = offers.clone();
    next.decisions.insert(
        offer.into(),
        OfferRecord {
            decision,
            decided_at: now,
        },
    );
    write(root, OFFERS_FILE, &next)?;
    *offers = next;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_create_nothing_and_writes_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap().join("absent");
        assert_eq!(load_pending(&root).unwrap(), None);
        assert!(!load_offers(&root).unwrap().decided(MOSS_OFFER));
        clear_pending(&root).unwrap();
        assert!(!root.exists());
        std::fs::create_dir(&root).unwrap();
        let pending = Pending::new(SpeechEngine::QwenKo, true, None);
        save_pending(&root, &pending).unwrap();
        assert_eq!(load_pending(&root).unwrap(), Some(pending));
        clear_pending(&root).unwrap();
        assert_eq!(load_pending(&root).unwrap(), None);
        let mut offers = load_offers(&root).unwrap();
        decide(&root, &mut offers, MOSS_OFFER, Decision::Declined, 7).unwrap();
        assert!(load_offers(&root).unwrap().decided(MOSS_OFFER));
    }

    #[test]
    fn switch_state_never_touches_the_released_preference_file() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("preferences")).unwrap();
        let speech = root.join("preferences/speech.json");
        std::fs::write(&speech, br#"{"schemaVersion":1,"engine":"nemotron"}"#).unwrap();
        save_pending(root, &Pending::new(SpeechEngine::QwenKo, true, None)).unwrap();
        let mut offers = Offers::default();
        decide(root, &mut offers, MOSS_OFFER, Decision::Accepted, 1).unwrap();
        assert_eq!(
            std::fs::read(&speech).unwrap(),
            br#"{"schemaVersion":1,"engine":"nemotron"}"#
        );
    }

    #[test]
    fn invalid_or_linked_state_is_reported_and_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("preferences")).unwrap();
        let path = root.join("preferences").join(PENDING_FILE);
        std::fs::write(&path, b"{\"schemaVersion\":1,\"target\":\"unknown\"}").unwrap();
        assert!(load_pending(root).is_err());
        assert!(path.exists());
    }

    #[test]
    fn a_pending_apple_switch_requires_its_frozen_transcription_variant() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("preferences")).unwrap();
        let path = root.join("preferences").join(PENDING_FILE);
        let raw = br#"{"schemaVersion":1,"target":"appleSpeech","speakerEnabled":false}"#;
        std::fs::write(&path, raw).unwrap();
        assert!(load_pending(root).is_err());
        assert_eq!(std::fs::read(path).unwrap(), raw);
    }
}
