//! Edit versions are SHA-256 fingerprints of the JSON a writer would replace. Readers
//! return them and writers require them back, so a change made in between is refused.
use crate::library::Library;
use crate::{CoreError, Meeting, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub(crate) fn fingerprint(value: &impl Serialize) -> Result<String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| CoreError::InvalidState("수정 버전을 확인하지 못했습니다.".into()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

// The only form `fingerprint` produces: "sha256:" and 64 lowercase hex digits.
pub(crate) fn valid(version: &str) -> bool {
    version.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

pub fn meeting_edit_version(meeting: &Meeting) -> Result<String> {
    // These fields can be hydrated from a capture manifest without editing the
    // meeting document. They must not invalidate a reader's metadata edit token.
    // Keep source/audio identities, status and all editable fields protected.
    let mut protected = meeting.clone();
    protected.warning = None;
    protected.capture_quality = None;
    protected.recorded_at = None;
    fingerprint(&protected)
}

pub(crate) fn library_edit_version(library: &Library) -> Result<String> {
    fingerprint(library)
}
