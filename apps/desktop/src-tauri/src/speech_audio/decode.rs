use super::{check_cancel, failure, output::PcmOutput};
use crate::processing::plain_file;
use crate::summary_failure::{Failure, FailureKind};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use symphonia::core::codecs::audio::well_known::CODEC_ID_AAC;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::common::Limit;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::{FormatReader, Track};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Duration;

fn decode_failure(error: Error) -> Failure {
    let message = match error {
        Error::Unsupported(_) => "이 파일의 인코딩 방식을 지원하지 않습니다. WAV로 변환한 뒤 다시 가져와 주세요.",
        _ => "음성 파일을 끝까지 읽지 못했습니다. 파일이 손상됐는지 확인해 주세요. 원본과 기존 기록은 유지됩니다.",
    };
    Failure::new(FailureKind::Input, message)
}

fn open(source: &Path) -> Result<Box<dyn FormatReader>, Failure> {
    let file = plain_file(source).map_err(|_| failure(FailureKind::Input))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = source.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }
    symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default()
                .limit_tag_bytes(Limit::Maximum(0))
                .limit_visual_bytes(Limit::Maximum(0)),
        )
        .map_err(decode_failure)
}

fn single_track(tracks: &[Track]) -> Result<&Track, Failure> {
    if tracks.is_empty() {
        return Err(Failure::new(
            FailureKind::Input,
            "음성 트랙을 찾지 못했습니다. 음성 파일인지 확인해 주세요.",
        ));
    }
    if tracks.len() != 1 {
        return Err(Failure::new(FailureKind::Input, "오디오 트랙이 여러 개인 파일은 지원하지 않습니다. 사용할 트랙을 별도 음성 파일로 저장해 주세요."));
    }
    Ok(&tracks[0])
}

#[cfg(target_os = "macos")]
pub(super) fn validate_container(source: &Path) -> Result<(), Failure> {
    let format = open(source)?;
    single_track(format.tracks()).map(|_| ())
}

pub(super) fn prepare(
    source: &Path,
    destination: &Path,
    cancel: &AtomicBool,
) -> Result<u64, Failure> {
    check_cancel(cancel)?;
    let mut format = open(source)?;
    let container = format.format_info().short_name;
    let track = single_track(format.tracks())?;
    let track_id = track.id;
    // Unindexed MP3 duration may be estimated, even for VBR. Never trim valid audio
    // or reject a file against that estimate. Xing/LAME delay+padding is exact.
    let estimated_mp3 = container == "mp3" && track.delay.is_none() && track.padding.is_none();
    let expected_frames = if container == "aac" {
        Some(super::framing::adts_frames(source, cancel)?)
    } else if estimated_mp3 {
        None
    } else {
        track.num_frames
    };
    let parameters = track.codec_params.as_ref().and_then(|p| p.audio())
        .ok_or_else(|| Failure::new(FailureKind::Input, "이 파일의 음성 인코딩 방식을 지원하지 않습니다. WAV로 변환한 뒤 다시 가져와 주세요."))?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(parameters, &AudioDecoderOptions::default().verify(true))
        .map_err(decode_failure)?;
    let mut presentation = if container == "isomp4" && parameters.codec == CODEC_ID_AAC {
        Some(super::presentation::Presentation::m4a_aac(
            source,
            expected_frames.ok_or_else(|| failure(FailureKind::Input))?,
            parameters
                .sample_rate
                .ok_or_else(|| failure(FailureKind::Input))?,
            format.metadata(),
            cancel,
        )?)
    } else {
        None
    };
    let mut output = None;
    let mut specification = None;
    let mut samples = Vec::<f64>::new();
    loop {
        check_cancel(cancel)?;
        let Some(mut packet) = format.next_packet().map_err(decode_failure)? else {
            break;
        };
        if packet.track_id != track_id {
            continue;
        }
        if estimated_mp3 {
            packet.dur = packet.dur.saturating_add(packet.trim_end);
            packet.trim_end = Duration::ZERO;
        }
        let decoded = decoder.decode(&packet).map_err(decode_failure)?;
        let spec = decoded.spec();
        if specification
            .as_ref()
            .is_some_and(|previous| previous != spec)
        {
            return Err(Failure::new(FailureKind::Input, "도중에 음성 형식이 바뀌는 파일은 지원하지 않습니다. 단일 형식의 음성 파일로 변환해 주세요."));
        }
        if decoded.frames() == 0 {
            continue;
        }
        if output.is_none() {
            output = Some(PcmOutput::new(
                destination,
                spec.rate(),
                spec.channels().count(),
                cancel,
            )?);
            specification = Some(spec.clone());
        }
        samples.resize(decoded.samples_interleaved(), 0.0);
        decoded.copy_to_slice_interleaved(&mut samples);
        let range = match &mut presentation {
            Some(presentation) => presentation.packet(decoded.frames())?,
            None => 0..decoded.frames(),
        };
        let channels = spec.channels().count();
        output
            .as_mut()
            .unwrap()
            .push(&samples[range.start * channels..range.end * channels])?;
    }
    if decoder.finalize().verify_ok == Some(false) {
        return Err(failure(FailureKind::Input));
    }
    check_cancel(cancel)?;
    let expected_frames = match presentation {
        Some(presentation) => Some(presentation.finish()?),
        None => expected_frames,
    };
    output
        .ok_or_else(|| failure(FailureKind::Input))?
        .finish(expected_frames)
}

#[cfg(test)]
#[path = "decode_tests.rs"]
mod tests;
