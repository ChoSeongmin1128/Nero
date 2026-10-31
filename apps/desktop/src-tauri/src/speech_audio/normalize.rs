use rubato::{FftFixedIn, Resampler};

pub(crate) const BLOCK_FRAMES: usize = 2048;

pub(crate) struct Normalizer {
    rate: u32,
    resampler: Option<FftFixedIn<f64>>,
    delay: usize,
    consumed: u64,
    written: u64,
    pending: Vec<f64>,
    finished: bool,
}
impl Normalizer {
    pub(crate) fn new(rate: u32) -> Result<Self, String> {
        if rate == 0 || rate > 768000 {
            return Err("Invalid capture rate".into());
        }
        let resampler = if rate == 16000 {
            None
        } else {
            Some(
                FftFixedIn::new(rate as usize, 16000, BLOCK_FRAMES, 2, 1)
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
    pub(crate) fn push(&mut self, samples: &[f64], finish: bool) -> Result<Vec<i16>, String> {
        if self.finished || samples.len() > BLOCK_FRAMES || samples.iter().any(|s| !s.is_finite()) {
            return Err("Invalid normalized input lifecycle".into());
        }
        self.consumed += samples.len() as u64;
        self.pending.extend_from_slice(samples);
        let expected =
            (u128::from(self.consumed) * 16000 + u128::from(self.rate / 2)) / u128::from(self.rate);
        let mut result = Vec::new();
        let mut flushes = 0;
        while self.pending.len() >= BLOCK_FRAMES || (finish && self.written < expected as u64) {
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
                if self.written >= expected as u64 {
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
            if !self.pending.is_empty() || self.written != expected as u64 {
                return Err("Incomplete resampling tail".into());
            }
        }
        Ok(result)
    }
}
