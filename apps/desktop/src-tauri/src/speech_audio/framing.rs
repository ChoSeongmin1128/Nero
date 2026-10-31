use super::{check_cancel, failure};
use crate::processing::plain_file;
use crate::summary_failure::{Failure, FailureKind};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::AtomicBool;

// ADTS duration in Symphonia is a bitrate estimate. Count complete framed packets
// instead, also rejecting a truncated header which its streaming reader treats as EOF.
pub(super) fn adts_frames(source: &Path, cancel: &AtomicBool) -> Result<u64, Failure> {
    check_cancel(cancel)?;
    let invalid = || failure(FailureKind::Input);
    let mut file = plain_file(source).map_err(|_| invalid())?;
    let length = file.metadata().map_err(|_| invalid())?.len();
    let mut start = [0; 10];
    file.read_exact(&mut start).map_err(|_| invalid())?;
    let mut offset = 0_u64;
    if &start[..3] == b"ID3" {
        if start[6..10].iter().any(|byte| byte & 0x80 != 0) {
            return Err(invalid());
        }
        let size = start[6..10]
            .iter()
            .fold(0_u64, |size, byte| (size << 7) | u64::from(*byte));
        offset = 10
            + size
            + if start[3] == 4 && start[5] & 0x10 != 0 {
                10
            } else {
                0
            };
    }
    let mut frames = 0_u64;
    while offset < length {
        check_cancel(cancel)?;
        file.seek(SeekFrom::Start(offset)).map_err(|_| invalid())?;
        let mut header = [0; 7];
        file.read_exact(&mut header).map_err(|_| invalid())?;
        if header[0] != 0xff || header[1] & 0xf6 != 0xf0 || header[6] & 3 != 0 {
            return Err(invalid());
        }
        let packet = ((u64::from(header[3]) & 3) << 11)
            | (u64::from(header[4]) << 3)
            | (u64::from(header[5]) >> 5);
        let header_size = if header[1] & 1 == 0 { 9 } else { 7 };
        if packet <= header_size || packet > length - offset {
            return Err(invalid());
        }
        offset += packet;
        frames = frames.checked_add(1024).ok_or_else(invalid)?;
    }
    if offset != length || frames == 0 {
        return Err(invalid());
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adts_counts_complete_packets_and_rejects_short_headers_and_payloads() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("audio.aac");
        // AAC-LC, 44.1kHz, stereo, 11-byte frame, one raw data block.
        let packet = [0xff, 0xf1, 0x50, 0x80, 1, 0x7f, 0xfc, 1, 2, 3, 4];
        let mut raw = packet.repeat(3);
        std::fs::write(&path, &raw).unwrap();
        let cancel = AtomicBool::new(false);
        assert_eq!(adts_frames(&path, &cancel).unwrap(), 3072);
        raw.extend([0xff, 0xf1]);
        std::fs::write(&path, &raw).unwrap();
        assert!(adts_frames(&path, &cancel).is_err());
        std::fs::write(&path, &packet[..10]).unwrap();
        assert!(adts_frames(&path, &cancel).is_err());
        assert_eq!(
            adts_frames(&path, &AtomicBool::new(true)).unwrap_err().kind,
            FailureKind::Cancelled
        );
    }
}
