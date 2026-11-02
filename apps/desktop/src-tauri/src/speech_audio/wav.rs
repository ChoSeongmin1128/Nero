use super::{check_cancel, failure};
use crate::processing::plain_file;
use crate::summary_failure::{Failure, FailureKind};
#[cfg(test)]
use hound::WavWriter;
use hound::{SampleFormat, WavReader, WavSpec};
use std::io::BufReader;
use std::path::Path;
use std::sync::atomic::AtomicBool;

pub(super) fn prepare(
    source: &Path,
    destination: &Path,
    cancel: &AtomicBool,
) -> Result<u64, Failure> {
    check_cancel(cancel)?;
    let mut reader = WavReader::new(BufReader::new(
        plain_file(source).map_err(|_| failure(FailureKind::Input))?,
    ))
    .map_err(|_| {
        Failure::new(
            FailureKind::Input,
            "이 음성 파일을 읽지 못했습니다. WAV 형식을 확인해 주세요.",
        )
    })?;
    let spec = reader.spec();
    if spec.channels == 0
        || spec.sample_rate == 0
        || spec.sample_rate > 768_000
        || reader.len() % u32::from(spec.channels) != 0
    {
        return Err(failure(FailureKind::Input));
    }
    let frames = u64::from(reader.duration());
    match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Float, 32) => convert(
            reader.samples::<f32>().map(|sample| {
                sample
                    .map(f64::from)
                    .map_err(|_| failure(FailureKind::Input))
            }),
            spec,
            frames,
            destination,
            cancel,
        ),
        (SampleFormat::Int, 8 | 16 | 24 | 32) => {
            let scale = (1_u64 << (spec.bits_per_sample - 1)) as f64;
            convert(
                reader.samples::<i32>().map(|sample| {
                    sample
                        .map(|sample| f64::from(sample) / scale)
                        .map_err(|_| failure(FailureKind::Input))
                }),
                spec,
                frames,
                destination,
                cancel,
            )
        }
        _ => Err(failure(FailureKind::Input)),
    }
}

fn convert(
    samples: impl Iterator<Item = Result<f64, Failure>>,
    spec: WavSpec,
    frames: u64,
    destination: &Path,
    cancel: &AtomicBool,
) -> Result<u64, Failure> {
    let expected = nero_core::audio_normalize::output_frames(frames, spec.sample_rate)
        .map_err(|_| failure(FailureKind::Input))?;
    if expected == 0 || u128::from(expected) * 2 + 36 > u128::from(u32::MAX) {
        return Err(failure(FailureKind::Input));
    }
    let mut output = super::output::PcmOutput::new(
        destination,
        spec.sample_rate,
        usize::from(spec.channels),
        cancel,
    )?;
    output.push_samples(samples)?;
    output.finish(Some(frames))
}

#[cfg(test)]
mod tests {
    use super::super::SAMPLE_RATE;
    use super::*;
    use std::fs;
    #[test]
    fn live_normalization_matches_file_conversion_across_rates_and_delivery_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        for rate in [8_000, 16_000, 44_100, 48_000, 96_000, 192_000] {
            let source = temp.path().join(format!("live-{rate}.wav"));
            let target = temp.path().join(format!("file-{rate}.wav"));
            fixture(&source, rate, 1, rate + 17, 1000.0);
            prepare(&source, &target, &AtomicBool::new(false)).unwrap();
            let samples: Vec<f64> = WavReader::open(&source)
                .unwrap()
                .samples::<f32>()
                .map(|s| f64::from(s.unwrap()))
                .collect();
            let expected: Vec<i16> = WavReader::open(&target)
                .unwrap()
                .samples::<i16>()
                .map(Result::unwrap)
                .collect();
            for delivery in [137, 2048] {
                let mut live = crate::speech_audio::normalize::Normalizer::new(rate).unwrap();
                let mut actual = Vec::new();
                for block in samples.chunks(delivery) {
                    actual.extend(live.push(block, false).unwrap());
                }
                actual.extend(live.push(&[], true).unwrap());
                assert_eq!(actual, expected, "rate {rate}, delivery {delivery}");
                assert!(live.push(&[], true).is_err());
            }
        }
        assert!(crate::speech_audio::normalize::Normalizer::new(16000)
            .unwrap()
            .push(&[f64::NAN], false)
            .is_err());
    }
    fn fixture(path: &Path, rate: u32, channels: u16, frames: u32, frequency: f64) {
        let mut writer = WavWriter::create(
            path,
            WavSpec {
                channels,
                sample_rate: rate,
                bits_per_sample: 32,
                sample_format: SampleFormat::Float,
            },
        )
        .unwrap();
        for frame in 0..frames {
            let value = (std::f64::consts::TAU * frequency * f64::from(frame) / f64::from(rate))
                .sin() as f32
                * 0.5;
            for _ in 0..channels {
                writer.write_sample(value).unwrap();
            }
        }
        writer.finalize().unwrap();
    }
    #[test]
    fn native_rate_stereo_float_is_normalized_without_modifying_source() {
        let temp = tempfile::tempdir().unwrap();
        for rate in [8_000, 16_000, 44_100, 48_000, 96_000, 192_000] {
            let source = temp.path().join(format!("{rate}.wav"));
            let target = temp.path().join(format!("{rate}-analysis.wav"));
            let frames = rate + 17;
            fixture(&source, rate, 2, frames, 1000.0);
            let original = fs::read(&source).unwrap();
            let expected = (u64::from(frames) * u64::from(SAMPLE_RATE) + u64::from(rate / 2))
                / u64::from(rate);
            assert_eq!(
                prepare(&source, &target, &AtomicBool::new(false)).unwrap(),
                expected
            );
            assert_eq!(
                super::super::native_frames(&target, &AtomicBool::new(false)).unwrap(),
                expected
            );
            assert_eq!(fs::read(&source).unwrap(), original);
        }
    }
    #[test]
    fn resampling_preserves_timing_and_suppresses_frequencies_above_output_nyquist() {
        let temp = tempfile::tempdir().unwrap();
        let mut rms = Vec::new();
        for frequency in [1000.0, 12000.0] {
            let source = temp.path().join(format!("{frequency}.wav"));
            let target = temp.path().join(format!("{frequency}-analysis.wav"));
            fixture(&source, 48000, 1, 48000, frequency);
            prepare(&source, &target, &AtomicBool::new(false)).unwrap();
            let samples: Vec<_> = WavReader::open(target)
                .unwrap()
                .into_samples::<i16>()
                .map(Result::unwrap)
                .collect();
            let interior = &samples[1000..15000];
            rms.push(
                (interior
                    .iter()
                    .map(|&sample| f64::from(sample).powi(2))
                    .sum::<f64>()
                    / interior.len() as f64)
                    .sqrt(),
            );
        }
        assert!(rms[0] > 11000.0 && rms[0] < 12000.0);
        assert!(rms[1] < rms[0] * 0.01);
        let source = temp.path().join("impulse.wav");
        let target = temp.path().join("impulse-analysis.wav");
        let mut writer = WavWriter::create(
            &source,
            WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: SampleFormat::Float,
            },
        )
        .unwrap();
        for index in 0..48000 {
            writer
                .write_sample(if index == 24000 { 1.0_f32 } else { 0.0 })
                .unwrap();
        }
        writer.finalize().unwrap();
        prepare(&source, &target, &AtomicBool::new(false)).unwrap();
        let samples: Vec<_> = WavReader::open(target)
            .unwrap()
            .into_samples::<i16>()
            .map(Result::unwrap)
            .collect();
        let peak = samples
            .iter()
            .enumerate()
            .max_by_key(|(_, sample)| i32::from(**sample).abs())
            .unwrap()
            .0;
        assert_eq!(peak, 8000);
    }
    #[test]
    fn integer_pcm_widths_and_stereo_mix_preserve_amplitude() {
        let temp = tempfile::tempdir().unwrap();
        for bits in [8, 16, 24, 32] {
            let source = temp.path().join(format!("integer-{bits}.wav"));
            let target = temp.path().join(format!("integer-{bits}-analysis.wav"));
            let mut writer = WavWriter::create(
                &source,
                WavSpec {
                    channels: 2,
                    sample_rate: 16000,
                    bits_per_sample: bits,
                    sample_format: SampleFormat::Int,
                },
            )
            .unwrap();
            let half_scale = (1_i64 << (bits - 2)) as i32;
            for _ in 0..32 {
                writer.write_sample(half_scale).unwrap();
                writer.write_sample(0_i32).unwrap();
            }
            writer.finalize().unwrap();
            assert_eq!(
                prepare(&source, &target, &AtomicBool::new(false)).unwrap(),
                32
            );
            let samples: Vec<_> = WavReader::open(target)
                .unwrap()
                .into_samples::<i16>()
                .map(Result::unwrap)
                .collect();
            assert!(samples.iter().all(|sample| *sample == 8192));
        }
    }
    #[test]
    fn short_tail_cancellation_invalid_samples_and_existing_output_are_checked() {
        let temp = tempfile::tempdir().unwrap();
        for frames in [2, 3, 7, 1023, 2048, 2049] {
            let source = temp.path().join(format!("short-{frames}.wav"));
            let target = temp.path().join(format!("short-{frames}-analysis.wav"));
            fixture(&source, 48000, 1, frames, 1000.0);
            let expected = (u64::from(frames) * 16000 + 24000) / 48000;
            assert_eq!(
                prepare(&source, &target, &AtomicBool::new(false)).unwrap(),
                expected
            );
            assert_eq!(
                prepare(&source, &target, &AtomicBool::new(false))
                    .unwrap_err()
                    .kind,
                FailureKind::Storage
            );
            assert_eq!(
                prepare(&source, &target, &AtomicBool::new(true))
                    .unwrap_err()
                    .kind,
                FailureKind::Cancelled
            );
        }
        let source = temp.path().join("invalid.wav");
        let mut writer = WavWriter::create(
            &source,
            WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 32,
                sample_format: SampleFormat::Float,
            },
        )
        .unwrap();
        writer.write_sample(f32::NAN).unwrap();
        writer.finalize().unwrap();
        assert_eq!(
            prepare(
                &source,
                &temp.path().join("invalid-output.wav"),
                &AtomicBool::new(false)
            )
            .unwrap_err()
            .kind,
            FailureKind::Input
        );
    }
}
