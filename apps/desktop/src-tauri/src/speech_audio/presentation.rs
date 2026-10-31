//! AAC packet padding is not presentation audio. Apply declared M4A timing once,
//! while still consuming and checking every decoded packet through EOF.
use super::check_cancel;
use crate::processing::plain_file;
use crate::summary_failure::{Failure, FailureKind};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use symphonia::core::meta::{Metadata, RawValue};

fn invalid() -> Failure {
    Failure::new(FailureKind::Input, "이 M4A 파일의 재생 구간을 정확히 확인하지 못했습니다. WAV로 변환한 뒤 다시 가져와 주세요. 원본과 기존 기록은 유지됩니다.")
}

#[derive(Debug, PartialEq)]
struct Gapless {
    delay: u64,
    padding: u64,
    frames: u64,
}

fn smpb(value: &str) -> Result<Gapless, Failure> {
    if value.len() > 512 {
        return Err(invalid());
    }
    let mut fields = value.split_ascii_whitespace();
    let mut number = || {
        let value = fields.next().ok_or_else(invalid)?;
        if value.len() > 16 || !value.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        u64::from_str_radix(value, 16).map_err(|_| invalid())
    };
    if number()? != 0 {
        return Err(invalid());
    }
    let result = Gapless {
        delay: number()?,
        padding: number()?,
        frames: number()?,
    };
    if result.frames == 0 {
        return Err(invalid());
    }
    Ok(result)
}

fn gapless(metadata: Metadata<'_>) -> Result<Option<Gapless>, Failure> {
    let mut result = None;
    if let Some(revision) = metadata.current() {
        for tag in &revision.media.tags {
            if !tag
                .raw
                .key
                .eq_ignore_ascii_case("com.apple.iTunes:iTunSMPB")
            {
                continue;
            }
            let RawValue::String(value) = &tag.raw.value else {
                return Err(invalid());
            };
            let value = smpb(value)?;
            if result.replace(value).is_some() {
                return Err(invalid());
            }
        }
    }
    Ok(result)
}

#[derive(Clone, Copy)]
struct Atom {
    payload: u64,
    end: u64,
}

fn child(
    file: &mut File,
    parent: Atom,
    kind: &[u8; 4],
    cancel: &AtomicBool,
) -> Result<Option<Atom>, Failure> {
    let mut offset = parent.payload;
    let mut found = None;
    while offset < parent.end {
        check_cancel(cancel)?;
        if parent.end - offset < 8 {
            return Err(invalid());
        }
        file.seek(SeekFrom::Start(offset)).map_err(|_| invalid())?;
        let mut header = [0; 8];
        file.read_exact(&mut header).map_err(|_| invalid())?;
        let size = u32::from_be_bytes(header[..4].try_into().unwrap());
        let (size, header_size) = match size {
            0 => (parent.end - offset, 8),
            1 => {
                let mut extended = [0; 8];
                file.read_exact(&mut extended).map_err(|_| invalid())?;
                (u64::from_be_bytes(extended), 16)
            }
            size => (u64::from(size), 8),
        };
        if size < header_size || size > parent.end - offset {
            return Err(invalid());
        }
        if &header[4..] == kind {
            let atom = Atom {
                payload: offset + header_size,
                end: offset + size,
            };
            if found.replace(atom).is_some() {
                return Err(invalid());
            }
        }
        offset += size;
    }
    Ok(found)
}

fn payload(file: &mut File, atom: Atom, max: usize) -> Result<Vec<u8>, Failure> {
    let size = atom.end - atom.payload;
    if size > max as u64 {
        return Err(invalid());
    }
    file.seek(SeekFrom::Start(atom.payload))
        .map_err(|_| invalid())?;
    let mut bytes = vec![0; size as usize];
    file.read_exact(&mut bytes).map_err(|_| invalid())?;
    Ok(bytes)
}

fn timescale(file: &mut File, atom: Atom) -> Result<u64, Failure> {
    let bytes = payload(file, atom, 128)?;
    let index = match bytes.first() {
        Some(0) => 12,
        Some(1) => 20,
        _ => return Err(invalid()),
    };
    let value = u32::from_be_bytes(
        bytes
            .get(index..index + 4)
            .ok_or_else(invalid)?
            .try_into()
            .unwrap(),
    );
    if value == 0 {
        return Err(invalid());
    }
    Ok(u64::from(value))
}

struct Edit {
    start: u64,
    duration: u64,
    movie_scale: u64,
}

fn edit(source: &Path, rate: u32, cancel: &AtomicBool) -> Result<Option<Edit>, Failure> {
    let mut file = plain_file(source).map_err(|_| invalid())?;
    let root = Atom {
        payload: 0,
        end: file.metadata().map_err(|_| invalid())?.len(),
    };
    let moov = child(&mut file, root, b"moov", cancel)?.ok_or_else(invalid)?;
    let trak = child(&mut file, moov, b"trak", cancel)?.ok_or_else(invalid)?;
    let Some(edts) = child(&mut file, trak, b"edts", cancel)? else {
        return Ok(None);
    };
    let elst = child(&mut file, edts, b"elst", cancel)?.ok_or_else(invalid)?;
    let bytes = payload(&mut file, elst, 32)?;
    if bytes.get(1..4) != Some(&[0, 0, 0]) || bytes.get(4..8) != Some(&1_u32.to_be_bytes()) {
        return Err(invalid());
    }
    let (duration, start, tail) = match (bytes.first(), bytes.len()) {
        (Some(0), 20) => (
            u64::from(u32::from_be_bytes(bytes[8..12].try_into().unwrap())),
            i64::from(i32::from_be_bytes(bytes[12..16].try_into().unwrap())),
            16,
        ),
        (Some(1), 28) => (
            u64::from_be_bytes(bytes[8..16].try_into().unwrap()),
            i64::from_be_bytes(bytes[16..24].try_into().unwrap()),
            24,
        ),
        _ => return Err(invalid()),
    };
    // Empty/multiple edits, dwell and speed changes are edits, not encoder padding.
    if start < 0 || duration == 0 || bytes[tail..] != [0, 1, 0, 0] {
        return Err(invalid());
    }
    let mvhd = child(&mut file, moov, b"mvhd", cancel)?.ok_or_else(invalid)?;
    let movie_scale = timescale(&mut file, mvhd)?;
    let mdia = child(&mut file, trak, b"mdia", cancel)?.ok_or_else(invalid)?;
    let mdhd = child(&mut file, mdia, b"mdhd", cancel)?.ok_or_else(invalid)?;
    let media_scale = timescale(&mut file, mdhd)?;
    let start = u128::from(start as u64) * u128::from(rate);
    if start % u128::from(media_scale) != 0 {
        return Err(invalid());
    }
    let start = u64::try_from(start / u128::from(media_scale)).map_err(|_| invalid())?;
    Ok(Some(Edit {
        start,
        duration,
        movie_scale,
    }))
}

pub(super) struct Presentation {
    span: Range<u64>,
    coded_frames: u64,
    consumed: u64,
}

impl Presentation {
    pub(super) fn m4a_aac(
        source: &Path,
        media_frames: u64,
        rate: u32,
        metadata: Metadata<'_>,
        cancel: &AtomicBool,
    ) -> Result<Self, Failure> {
        if media_frames == 0 || rate == 0 {
            return Err(invalid());
        }
        // AAC-LC uses 1024-sample access units; a container may give a partial
        // duration for the last unit. The decoder must still consume that unit.
        let coded_frames = media_frames.checked_add(1023).ok_or_else(invalid)? / 1024 * 1024;
        let edit = edit(source, rate, cancel)?;
        let span = if let Some(gapless) = gapless(metadata)? {
            let end = gapless
                .delay
                .checked_add(gapless.frames)
                .ok_or_else(invalid)?;
            if end.checked_add(gapless.padding) != Some(coded_frames) {
                return Err(invalid());
            }
            if let Some(edit) = edit {
                let movie_ticks = (u128::from(gapless.frames) * u128::from(edit.movie_scale))
                    .div_ceil(u128::from(rate));
                if edit.start != gapless.delay || movie_ticks != u128::from(edit.duration) {
                    return Err(invalid());
                }
            }
            gapless.delay..end
        } else if let Some(edit) = edit {
            let frames = (u128::from(edit.duration) * u128::from(rate))
                .div_ceil(u128::from(edit.movie_scale));
            let end = edit
                .start
                .checked_add(u64::try_from(frames).map_err(|_| invalid())?)
                .ok_or_else(invalid)?;
            edit.start..end.min(media_frames)
        } else {
            0..media_frames
        };
        if span.is_empty() || span.end > coded_frames {
            return Err(invalid());
        }
        Ok(Self {
            span,
            coded_frames,
            consumed: 0,
        })
    }

    pub(super) fn packet(&mut self, frames: usize) -> Result<Range<usize>, Failure> {
        let start = self.consumed;
        self.consumed = start.checked_add(frames as u64).ok_or_else(invalid)?;
        if self.consumed > self.coded_frames {
            return Err(invalid());
        }
        let from = self.span.start.clamp(start, self.consumed);
        let to = self.span.end.clamp(from, self.consumed);
        Ok((from - start) as usize..(to - start) as usize)
    }

    pub(super) fn finish(self) -> Result<u64, Failure> {
        if self.consumed != self.coded_frames {
            return Err(invalid());
        }
        Ok(self.span.end - self.span.start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn declared_gapless_interval_excludes_priming_but_still_requires_coded_eof() {
        let parsed = smpb("00000000 00000840 000002BF 000000000000B501").unwrap();
        assert_eq!(
            parsed,
            Gapless {
                delay: 2112,
                padding: 703,
                frames: 46337
            }
        );
        let mut projection = Presentation {
            span: 2112..48449,
            coded_frames: 49152,
            consumed: 0,
        };
        let mut frames = 0;
        for _ in 0..48 {
            frames += projection.packet(1024).unwrap().len();
        }
        assert_eq!(frames, 46337);
        assert_eq!(projection.finish().unwrap(), 46337);
        let projection = Presentation {
            span: 2112..48449,
            coded_frames: 49152,
            consumed: 48449,
        };
        assert!(projection.finish().is_err());
        for invalid in ["", "0 1 2", "1 1 2 3", "0 1 2 0", "0 X 2 3"] {
            assert!(smpb(invalid).is_err());
        }
    }
}
