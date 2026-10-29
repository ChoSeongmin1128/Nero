use super::{check_cancel, failure, SAMPLE_RATE};
use crate::processing::plain_file;
use crate::summary_failure::{Failure, FailureKind};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use rubato::{FftFixedIn, Resampler};
use std::fs::OpenOptions;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::sync::atomic::AtomicBool;

const BLOCK_FRAMES: usize = 2048;

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

fn pcm16(sample: f64) -> Result<i16, Failure> {
    if !sample.is_finite() {
        return Err(failure(FailureKind::Input));
    }
    Ok((sample.clamp(-1.0, 1.0) * 32768.0)
        .round()
        .clamp(-32768.0, 32767.0) as i16)
}

fn convert(
    mut samples: impl Iterator<Item = Result<f64, Failure>>,
    spec: WavSpec,
    frames: u64,
    destination: &Path,
    cancel: &AtomicBool,
) -> Result<u64, Failure> {
    let expected = (u128::from(frames) * u128::from(SAMPLE_RATE)
        + u128::from(spec.sample_rate / 2))
        / u128::from(spec.sample_rate);
    if expected == 0 || expected * 2 + 36 > u128::from(u32::MAX) {
        return Err(failure(FailureKind::Input));
    }
    let expected = expected as u64;
    let mut resampler = if spec.sample_rate == SAMPLE_RATE {
        None
    } else {
        Some(
            FftFixedIn::<f64>::new(
                spec.sample_rate as usize,
                SAMPLE_RATE as usize,
                BLOCK_FRAMES,
                2,
                1,
            )
            .map_err(|_| failure(FailureKind::Input))?,
        )
    };
    let mut delay = resampler
        .as_ref()
        .map_or(0, |resampler| resampler.output_delay());
    let flush_limit = resampler.as_ref().map_or(0, |resampler| {
        ((delay + resampler.output_frames_max() + 1) as u64 * u64::from(spec.sample_rate))
            .div_ceil(u64::from(SAMPLE_RATE))
            + BLOCK_FRAMES as u64
    });
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|_| failure(FailureKind::Storage))?;
    let sync = file
        .try_clone()
        .map_err(|_| failure(FailureKind::Storage))?;
    let mut writer = WavWriter::new(
        BufWriter::new(file),
        WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        },
    )
    .map_err(|_| failure(FailureKind::Storage))?;
    let mut input = vec![vec![0.0; BLOCK_FRAMES]];
    let mut output = resampler
        .as_ref()
        .map(|resampler| resampler.output_buffer_allocate(true));
    let (mut consumed, mut written, mut padded) = (0_u64, 0_u64, 0_u64);
    while consumed < frames || written < expected {
        check_cancel(cancel)?;
        let read = (frames - consumed).min(BLOCK_FRAMES as u64) as usize;
        input[0].fill(0.0);
        for value in &mut input[0][..read] {
            let mut sum = 0.0;
            for _ in 0..spec.channels {
                let sample = samples
                    .next()
                    .ok_or_else(|| failure(FailureKind::Input))??;
                if !sample.is_finite() {
                    return Err(failure(FailureKind::Input));
                }
                sum += sample;
            }
            *value = sum / f64::from(spec.channels);
        }
        consumed += read as u64;
        let ready = if let Some(resampler) = resampler.as_mut() {
            padded += (BLOCK_FRAMES - read) as u64;
            if padded > flush_limit {
                return Err(failure(FailureKind::Output));
            }
            let output = output.as_mut().unwrap();
            let (_, produced) = resampler
                .process_into_buffer(&input, output, None)
                .map_err(|_| failure(FailureKind::Input))?;
            &output[0][..produced]
        } else {
            &input[0][..read]
        };
        let skip = delay.min(ready.len());
        delay -= skip;
        for &sample in &ready[skip..] {
            if written >= expected {
                break;
            }
            writer
                .write_sample(pcm16(sample)?)
                .map_err(|_| failure(FailureKind::Storage))?;
            written += 1;
        }
    }
    if consumed != frames || samples.next().is_some() || written != expected {
        return Err(failure(FailureKind::Input));
    }
    check_cancel(cancel)?;
    writer
        .finalize()
        .map_err(|_| failure(FailureKind::Storage))?;
    sync.sync_all().map_err(|_| failure(FailureKind::Storage))?;
    check_cancel(cancel)?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
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
