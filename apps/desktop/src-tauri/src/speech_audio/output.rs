use super::{check_cancel, failure, normalize::Normalizer, SAMPLE_RATE};
use crate::summary_failure::{Failure, FailureKind};
use hound::{SampleFormat, WavSpec, WavWriter};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use tempfile::{NamedTempFile, TempPath};

pub(super) struct PcmOutput<'a> {
    destination: PathBuf,
    writer: WavWriter<BufWriter<File>>,
    sync: File,
    normalizer: Normalizer,
    channels: usize,
    input_frames: u64,
    output_frames: u64,
    cancel: &'a AtomicBool,
    // Close both handles before removing the temporary file on Windows.
    temporary: TempPath,
}

impl<'a> PcmOutput<'a> {
    pub(super) fn new(
        destination: &Path,
        rate: u32,
        channels: usize,
        cancel: &'a AtomicBool,
    ) -> Result<Self, Failure> {
        check_cancel(cancel)?;
        if channels == 0 || channels > u16::MAX as usize {
            return Err(failure(FailureKind::Input));
        }
        if destination
            .try_exists()
            .map_err(|_| failure(FailureKind::Storage))?
        {
            return Err(failure(FailureKind::Storage));
        }
        let normalizer = Normalizer::new(rate).map_err(|_| failure(FailureKind::Input))?;
        let temporary = NamedTempFile::new_in(
            destination
                .parent()
                .ok_or_else(|| failure(FailureKind::Storage))?,
        )
        .map_err(|_| failure(FailureKind::Storage))?;
        let (file, temporary) = temporary.into_parts();
        let sync = file
            .try_clone()
            .map_err(|_| failure(FailureKind::Storage))?;
        let writer = WavWriter::new(
            BufWriter::new(file),
            WavSpec {
                channels: 1,
                sample_rate: SAMPLE_RATE,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            },
        )
        .map_err(|_| failure(FailureKind::Storage))?;
        Ok(Self {
            destination: destination.to_owned(),
            temporary,
            writer,
            sync,
            normalizer,
            channels,
            input_frames: 0,
            output_frames: 0,
            cancel,
        })
    }

    pub(super) fn push(&mut self, samples: &[f64]) -> Result<(), Failure> {
        self.push_samples(samples.iter().copied().map(Ok))
    }

    pub(super) fn push_samples(
        &mut self,
        samples: impl IntoIterator<Item = Result<f64, Failure>>,
    ) -> Result<(), Failure> {
        check_cancel(self.cancel)?;
        let mut mono = Vec::with_capacity(super::normalize::BLOCK_FRAMES);
        let (mut sum, mut channel) = (0.0, 0);
        for sample in samples {
            let sample = sample?;
            if !sample.is_finite() {
                return Err(failure(FailureKind::Input));
            }
            sum += sample;
            channel += 1;
            if channel != self.channels {
                continue;
            }
            let sample = sum / self.channels as f64;
            if !sample.is_finite() {
                return Err(failure(FailureKind::Input));
            }
            mono.push(sample);
            self.input_frames = self
                .input_frames
                .checked_add(1)
                .ok_or_else(|| failure(FailureKind::Input))?;
            (sum, channel) = (0.0, 0);
            if mono.len() == super::normalize::BLOCK_FRAMES {
                self.normalize(&mono, false)?;
                mono.clear();
            }
        }
        if channel != 0 {
            return Err(failure(FailureKind::Input));
        }
        self.normalize(&mono, false)
    }

    fn normalize(&mut self, mono: &[f64], finish: bool) -> Result<(), Failure> {
        check_cancel(self.cancel)?;
        let output = self
            .normalizer
            .push(mono, finish)
            .map_err(|_| failure(FailureKind::Input))?;
        self.output_frames = self
            .output_frames
            .checked_add(output.len() as u64)
            .ok_or_else(|| failure(FailureKind::Input))?;
        // RIFF's data and container sizes must both fit in their 32-bit fields.
        if self.output_frames > (u64::from(u32::MAX) - 36) / 2 {
            return Err(failure(FailureKind::Input));
        }
        for sample in output {
            self.writer
                .write_sample(sample)
                .map_err(|_| failure(FailureKind::Storage))?;
        }
        Ok(())
    }

    pub(super) fn finish(mut self, expected_input_frames: Option<u64>) -> Result<u64, Failure> {
        if self.input_frames == 0
            || expected_input_frames.is_some_and(|expected| expected != self.input_frames)
        {
            return Err(failure(FailureKind::Input));
        }
        self.normalize(&[], true)?;
        if self.output_frames == 0 {
            return Err(failure(FailureKind::Input));
        }
        let Self {
            destination,
            temporary,
            writer,
            sync,
            output_frames,
            cancel,
            ..
        } = self;
        let finalized = writer
            .finalize()
            .and_then(|_| sync.sync_all().map_err(Into::into));
        drop(sync);
        finalized.map_err(|_| failure(FailureKind::Storage))?;
        check_cancel(cancel)?;
        temporary
            .persist_noclobber(destination)
            .map_err(|_| failure(FailureKind::Storage))?;
        Ok(output_frames)
    }
}
