use rubato::{FftFixedIn, Resampler};

pub const BLOCK_FRAMES: usize = 2048;
pub const SAMPLE_RATE: u32 = 16000;

pub fn output_frames(frames: u64, rate: u32) -> Result<u64, String> {
    if rate == 0 || rate > 768000 {
        return Err("Invalid capture rate".into());
    }
    ((u128::from(frames) * u128::from(SAMPLE_RATE) + u128::from(rate / 2)) / u128::from(rate))
        .try_into()
        .map_err(|_| "Normalized frame count exceeded limit".into())
}

pub struct Normalizer {
    rate: u32,
    resampler: Option<FftFixedIn<f64>>,
    delay: usize,
    consumed: u64,
    written: u64,
    pending: Vec<f64>,
    finished: bool,
}
impl Normalizer {
    pub fn new(rate: u32) -> Result<Self, String> {
        if rate == 0 || rate > 768000 {
            return Err("Invalid capture rate".into());
        }
        let resampler = if rate == SAMPLE_RATE {
            None
        } else {
            Some(
                FftFixedIn::new(rate as usize, SAMPLE_RATE as usize, BLOCK_FRAMES, 2, 1)
                    .map_err(|e| e.to_string())?,
            )
        };
        let delay = resampler.as_ref().map_or(0, |r| r.output_delay());
        Ok(Self {
            rate,
            resampler,
            delay,
            consumed: 0,
            written: 0,
            pending: Vec::new(),
            finished: false,
        })
    }
    pub fn push(&mut self, samples: &[f64], finish: bool) -> Result<Vec<i16>, String> {
        if self.finished || samples.len() > BLOCK_FRAMES || samples.iter().any(|s| !s.is_finite()) {
            return Err("Invalid normalized input lifecycle".into());
        }
        self.consumed += samples.len() as u64;
        self.pending.extend_from_slice(samples);
        let expected = output_frames(self.consumed, self.rate)?;
        let mut result = Vec::new();
        let mut flushes = 0;
        while self.pending.len() >= BLOCK_FRAMES || (finish && self.written < expected) {
            let take = BLOCK_FRAMES.min(self.pending.len());
            let mut input = vec![vec![0.0; BLOCK_FRAMES]];
            input[0][..take].copy_from_slice(&self.pending[..take]);
            self.pending.drain(..take);
            let output = if let Some(resampler) = &mut self.resampler {
                resampler
                    .process(&input, None)
                    .map_err(|e| e.to_string())?
                    .remove(0)
            } else {
                input.remove(0)[..take].to_vec()
            };
            let skip = self.delay.min(output.len());
            self.delay -= skip;
            for sample in &output[skip..] {
                if self.written >= expected {
                    break;
                }
                if !sample.is_finite() {
                    return Err("Invalid resampler output".into());
                }
                result.push(
                    (sample.clamp(-1.0, 1.0) * 32768.0)
                        .round()
                        .clamp(-32768.0, 32767.0) as i16,
                );
                self.written += 1;
            }
            flushes += 1;
            if flushes > 64 {
                return Err("Resampler exceeded bounded flush".into());
            }
        }
        if finish {
            self.finished = true;
            if !self.pending.is_empty() || self.written != expected {
                return Err("Incomplete resampling tail".into());
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_audio_stays_bounded_across_delivery_boundaries_and_tail() {
        for rate in [8_000, 16_000, 44_100, 96_000, 192_000, 768_000] {
            let mut normalizer = Normalizer::new(rate).unwrap();
            let samples = [0.25; 137];
            let mut written = 0_u64;
            for _ in 0..97 {
                written += normalizer.push(&samples, false).unwrap().len() as u64;
                assert!(normalizer.pending.len() < BLOCK_FRAMES);
            }
            written += normalizer.push(&[], true).unwrap().len() as u64;
            assert_eq!(written, output_frames(137 * 97, rate).unwrap());
            assert!(normalizer.pending.is_empty());
            assert!(normalizer.push(&[], true).is_err());
        }
        let mut normalizer = Normalizer::new(SAMPLE_RATE).unwrap();
        assert!(normalizer.push(&[0.0; BLOCK_FRAMES + 1], false).is_err());
        assert!(normalizer.push(&[f64::NAN], false).is_err());
    }
}
