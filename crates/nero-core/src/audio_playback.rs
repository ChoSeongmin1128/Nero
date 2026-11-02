use crate::audio_normalize::{output_frames, Normalizer, BLOCK_FRAMES, SAMPLE_RATE};
use crate::store::{Encoding, Manifest};
use crate::{AudioArtifact, AudioPlayback, CoreError, Result};
use hound::{SampleFormat, WavSpec, WavWriter};
use std::io::{Read, Seek, Write};

const NATIVE_SAMPLE_LIMIT: u64 = (u32::MAX as u64 - 1024) / 4;
const PCM16_FRAME_LIMIT: u64 = (u32::MAX as u64 - 36) / 2;
const SPEECH_POLICY: &str = "native-pcm-speech-playback-v1";

pub(crate) struct Playback {
    pub frames: u64,
    pub spec: WavSpec,
}

fn invalid() -> CoreError {
    CoreError::Storage("녹음 파일과 재생 정보가 일치하지 않습니다.".into())
}

impl Playback {
    pub fn for_capture(frames: u64, rate: u32, channels: u16) -> Result<Self> {
        Self::with_native_limit(frames, rate, channels, NATIVE_SAMPLE_LIMIT)
    }

    pub(super) fn with_native_limit(
        frames: u64,
        rate: u32,
        channels: u16,
        native_limit: u64,
    ) -> Result<Self> {
        let samples = frames
            .checked_mul(u64::from(channels))
            .ok_or_else(invalid)?;
        if samples <= native_limit {
            return Ok(Self {
                frames,
                spec: WavSpec {
                    channels,
                    sample_rate: rate,
                    bits_per_sample: 32,
                    sample_format: SampleFormat::Float,
                },
            });
        }
        let frames = output_frames(frames, rate).map_err(|_| invalid())?;
        if frames == 0 || frames > PCM16_FRAME_LIMIT {
            return Err(CoreError::Storage(
                "재생 WAV 파일 크기 한도를 넘었습니다. 원본 녹음은 보관되어 있습니다.".into(),
            ));
        }
        Ok(Self {
            frames,
            spec: WavSpec {
                channels: 1,
                sample_rate: SAMPLE_RATE,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            },
        })
    }

    pub fn from_artifact(artifact: &AudioArtifact) -> Result<Self> {
        match &artifact.playback {
            None => Self::with_native_limit(
                artifact.frames,
                artifact.sample_rate,
                artifact.channels,
                u64::MAX,
            ),
            Some(playback) => {
                let expected = Self::with_native_limit(
                    artifact.frames,
                    artifact.sample_rate,
                    artifact.channels,
                    0,
                )?;
                if expected.metadata().as_ref() != Some(playback) {
                    return Err(invalid());
                }
                Ok(expected)
            }
        }
    }

    pub fn metadata(&self) -> Option<AudioPlayback> {
        (self.spec.sample_format == SampleFormat::Int).then(|| AudioPlayback {
            policy: SPEECH_POLICY.into(),
            frames: self.frames,
            sample_rate: self.spec.sample_rate,
            channels: self.spec.channels,
        })
    }

    pub fn write(
        &self,
        input: &mut impl Read,
        writer: &mut WavWriter<impl Write + Seek>,
        manifest: &Manifest,
    ) -> Result<()> {
        if self.metadata().is_none() {
            let count = manifest.saved_frames * u64::from(manifest.channels);
            let mut sample = [0_u8; 8];
            for _ in 0..count {
                input.read_exact(&mut sample[..manifest.encoding.bytes()])?;
                writer
                    .write_sample(
                        manifest
                            .encoding
                            .sample(&sample[..manifest.encoding.bytes()]),
                    )
                    .map_err(|error| CoreError::Storage(error.to_string()))?;
            }
            return Ok(());
        }
        let mut normalizer = Normalizer::new(manifest.sample_rate).map_err(|_| invalid())?;
        let mut remaining = manifest.saved_frames;
        let mut written = 0_u64;
        while remaining != 0 {
            let count = remaining.min(BLOCK_FRAMES as u64) as usize;
            let mono = read_native_mono(input, manifest.encoding, manifest.channels, count)?;
            let output = normalizer.push(&mono, false).map_err(|_| invalid())?;
            written += output.len() as u64;
            for sample in output {
                writer
                    .write_sample(sample)
                    .map_err(|error| CoreError::Storage(error.to_string()))?;
            }
            remaining -= count as u64;
        }
        let tail = normalizer.push(&[], true).map_err(|_| invalid())?;
        written += tail.len() as u64;
        for sample in tail {
            writer
                .write_sample(sample)
                .map_err(|error| CoreError::Storage(error.to_string()))?;
        }
        if written != self.frames {
            return Err(invalid());
        }
        Ok(())
    }
}

pub(crate) fn read_native_mono(
    input: &mut impl Read,
    encoding: Encoding,
    channels: u16,
    frames: usize,
) -> Result<Vec<f64>> {
    let frame_bytes = usize::from(channels) * encoding.bytes();
    let mut bytes = vec![0; frames * frame_bytes];
    input.read_exact(&mut bytes)?;
    let mut mono = Vec::with_capacity(frames);
    for frame in bytes.chunks_exact(frame_bytes) {
        let mut sum = 0.0;
        for value in frame.chunks_exact(encoding.bytes()) {
            let sample = encoding.sample(value);
            if !sample.is_finite() {
                return Err(CoreError::Audio("음성 샘플이 올바르지 않습니다.".into()));
            }
            sum += f64::from(sample);
        }
        mono.push(sum / f64::from(channels));
    }
    Ok(mono)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_riff_boundary_selects_playback_before_conversion() {
        let last = NATIVE_SAMPLE_LIMIT / 2;
        let native = Playback::for_capture(last, 96_000, 2).unwrap();
        assert!(native.metadata().is_none());
        assert_eq!(native.frames, last);
        let normalized = Playback::for_capture(last + 1, 96_000, 2).unwrap();
        assert!(normalized.metadata().is_some());
        assert_eq!(normalized.frames, output_frames(last + 1, 96_000).unwrap());
    }

    #[test]
    fn three_hour_high_rate_and_multichannel_capture_fits_standard_playback_riff() {
        for (rate, channels) in [(96_000, 2), (48_000, 4), (192_000, 64), (768_000, 64)] {
            let frames = u64::from(rate) * 3 * 60 * 60;
            let playback = Playback::for_capture(frames, rate, channels).unwrap();
            assert_eq!(playback.frames, 172_800_000);
            assert_eq!(playback.spec.sample_rate, SAMPLE_RATE);
            assert_eq!(playback.spec.channels, 1);
            assert_eq!(playback.spec.bits_per_sample, 16);
            assert_eq!(playback.spec.sample_format, SampleFormat::Int);
            assert_eq!(playback.frames * 2, 345_600_000);
            assert!(playback.frames <= PCM16_FRAME_LIMIT);
        }
        let normal = Playback::for_capture(48_000 * 3 * 60 * 60, 48_000, 2).unwrap();
        assert!(normal.metadata().is_none());
    }

    #[test]
    fn explicit_playback_geometry_and_policy_must_match_native_coordinates() {
        let native_frames = 96_000 * 94 * 60 + 17;
        let layout = Playback::for_capture(native_frames, 96_000, 2).unwrap();
        let artifact = AudioArtifact {
            session_id: uuid::Uuid::new_v4().to_string(),
            relative_path: "audio.wav".into(),
            frames: native_frames,
            sample_rate: 96_000,
            channels: 2,
            playback: layout.metadata(),
        };
        let decoded = Playback::from_artifact(&artifact).unwrap();
        assert_eq!(
            decoded.frames,
            output_frames(native_frames, 96_000).unwrap()
        );
        let mut variants = Vec::new();
        let mut changed = artifact.clone();
        changed.playback.as_mut().unwrap().frames += 1;
        variants.push(changed);
        let mut changed = artifact.clone();
        changed.playback.as_mut().unwrap().sample_rate = 48_000;
        variants.push(changed);
        let mut changed = artifact.clone();
        changed.playback.as_mut().unwrap().channels = 2;
        variants.push(changed);
        let mut changed = artifact.clone();
        changed.playback.as_mut().unwrap().policy = "unverified".into();
        variants.push(changed);
        let mut changed = artifact.clone();
        changed.frames += 96_000;
        variants.push(changed);
        for changed in variants {
            assert!(Playback::from_artifact(&changed).is_err());
        }
    }

    #[test]
    fn legacy_artifact_roundtrip_does_not_add_playback_metadata() {
        let previous = serde_json::json!({
            "sessionId":uuid::Uuid::new_v4().to_string(),
            "relativePath":"audio.wav", "frames":48017,
            "sampleRate":48000,"channels":2
        });
        let artifact: AudioArtifact = serde_json::from_value(previous.clone()).unwrap();
        assert!(artifact.playback.is_none());
        assert_eq!(serde_json::to_value(&artifact).unwrap(), previous);
        let layout = Playback::from_artifact(&artifact).unwrap();
        assert_eq!(layout.frames, 48_017);
        assert_eq!(layout.spec.sample_rate, 48_000);
        assert_eq!(layout.spec.channels, 2);
        assert_eq!(layout.spec.sample_format, SampleFormat::Float);
    }
}
