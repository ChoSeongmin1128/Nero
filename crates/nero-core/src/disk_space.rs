use crate::audio_playback::Playback;
use crate::store::Manifest;
use crate::{CoreError, Result};
use std::path::Path;

// Left free for macOS itself. It grows swap in 1 GiB files, and a disk with no
// room for the next one can stall the whole machine.
pub(crate) const SYSTEM_RESERVE_BYTES: u64 = 1 << 30;
// meeting.json (notes up to 2 MiB) is written beside its replacement, plus the
// manifest and library.json.
const METADATA_BYTES: u64 = 16 << 20;
// Audio that can still arrive after a low-space check: the capture queue holds up
// to 2 s, checkpoints run every second and the desktop polls every 750 ms.
const HEADROOM_SECONDS: u64 = 10;

pub(crate) fn available(path: &Path) -> Result<u64> {
    #[cfg(test)]
    if let Some(bytes) = tests::forced(path) {
        return Ok(bytes);
    }
    fs2::available_space(path)
        .map_err(|_| CoreError::Storage("저장 공간을 확인하지 못했습니다.".into()))
}

pub(crate) fn wav_bytes(frames: u64, sample_rate: u32, channels: u16) -> u64 {
    match Playback::for_capture(frames, sample_rate, channels) {
        Ok(playback) => playback.wav_bytes(),
        // Past the playback limits nothing is written; keep the float32 upper bound.
        Err(_) => frames.saturating_mul(u64::from(channels)).saturating_mul(4),
    }
}

// Space a capture needs to keep recording and still produce its playback WAV.
pub(crate) fn recording_bytes(manifest: &Manifest) -> u64 {
    let headroom = u64::from(manifest.sample_rate).saturating_mul(HEADROOM_SECONDS);
    wav_bytes(
        manifest.saved_frames.saturating_add(headroom),
        manifest.sample_rate,
        manifest.channels,
    )
    .saturating_add(headroom.saturating_mul(manifest.frame_bytes() as u64))
    .saturating_add(METADATA_BYTES)
    .saturating_add(SYSTEM_RESERVE_BYTES)
}

pub(crate) fn playback_bytes(playback: &Playback) -> u64 {
    playback
        .wav_bytes()
        .saturating_add(METADATA_BYTES)
        .saturating_add(SYSTEM_RESERVE_BYTES)
}

pub(crate) fn shortfall(available: u64, required: u64) -> Option<String> {
    if available >= required {
        return None;
    }
    let missing = required - available;
    Some(if missing >= 1_000_000_000 {
        format!("{:.1} GB", (missing as f64 / 100_000_000.0).ceil() / 10.0)
    } else {
        format!("{} MB", missing.div_ceil(1_000_000))
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    static FORCED: Mutex<Vec<(PathBuf, u64)>> = Mutex::new(Vec::new());

    // Pretends the volume holding `root` has `bytes` free, for tests only.
    pub(crate) fn force(root: &Path, bytes: Option<u64>) {
        let mut forced = FORCED.lock().unwrap();
        forced.retain(|(path, _)| path != root);
        if let Some(bytes) = bytes {
            forced.push((root.to_path_buf(), bytes));
        }
    }

    pub(super) fn forced(path: &Path) -> Option<u64> {
        FORCED
            .lock()
            .unwrap()
            .iter()
            .find(|(root, _)| path.starts_with(root))
            .map(|(_, bytes)| *bytes)
    }

    #[test]
    fn shortfall_rounds_up_and_accepts_the_exact_boundary() {
        assert_eq!(shortfall(20, 20), None);
        assert_eq!(shortfall(20, 21).as_deref(), Some("1 MB"));
        assert_eq!(shortfall(0, 1_000_000_001).as_deref(), Some("1.1 GB"));
    }

    #[test]
    fn wav_estimate_matches_the_float_playback_and_keeps_the_reserve() {
        let playback = Playback::for_capture(48_000, 48_000, 2).unwrap();
        assert!(playback.wav_bytes() >= 48_000 * 2 * 4);
        assert!(playback_bytes(&playback) > SYSTEM_RESERVE_BYTES + 48_000 * 2 * 4);
    }
}
