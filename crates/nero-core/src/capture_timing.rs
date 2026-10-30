use super::Signals;
use crate::store::CaptureTiming;
use cpal::InputStreamTimestamp;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Default)]
pub(super) struct TimingSignals {
    callback_count: AtomicU64,
    input_frames: AtomicU64,
    first_callback_after_begin_ns: AtomicU64,
    timestamp_gap_count: AtomicU64,
    timestamp_overlap_count: AtomicU64,
    timestamp_reversal_count: AtomicU64,
    max_timestamp_gap_ns: AtomicU64,
    max_timestamp_overlap_ns: AtomicU64,
    max_arrival_interval_ns: AtomicU64,
    max_host_interval_ns: AtomicU64,
    pub max_pending_frames: AtomicU64,
    pub max_writer_batch_ns: AtomicU64,
    pub max_checkpoint_ns: AtomicU64,
}

impl TimingSignals {
    pub fn snapshot(&self, queued_frames: u64, saved_frames: u64) -> Option<CaptureTiming> {
        let callback_count = self.callback_count.load(Ordering::Acquire);
        (callback_count != 0).then(|| CaptureTiming {
            callback_count,
            input_frames: self.input_frames.load(Ordering::Relaxed),
            first_callback_after_begin_ns: self
                .first_callback_after_begin_ns
                .load(Ordering::Relaxed),
            timestamp_gap_count: self.timestamp_gap_count.load(Ordering::Relaxed),
            timestamp_overlap_count: self.timestamp_overlap_count.load(Ordering::Relaxed),
            timestamp_reversal_count: self.timestamp_reversal_count.load(Ordering::Relaxed),
            max_timestamp_gap_ns: self.max_timestamp_gap_ns.load(Ordering::Relaxed),
            max_timestamp_overlap_ns: self.max_timestamp_overlap_ns.load(Ordering::Relaxed),
            max_arrival_interval_ns: self.max_arrival_interval_ns.load(Ordering::Relaxed),
            max_host_interval_ns: self.max_host_interval_ns.load(Ordering::Relaxed),
            max_pending_frames: self.max_pending_frames.load(Ordering::Relaxed),
            queued_frames,
            saved_frames,
            max_writer_batch_ns: self.max_writer_batch_ns.load(Ordering::Relaxed),
            max_checkpoint_ns: self.max_checkpoint_ns.load(Ordering::Relaxed),
        })
    }
}

pub(super) fn elapsed_ns(started: Instant) -> u64 {
    started.elapsed().as_nanos().min(u64::MAX as u128) as u64
}

pub(super) struct CallbackTiming {
    sample_rate: u32,
    previous: Option<(u64, InputStreamTimestamp, u64, u64)>,
}

impl CallbackTiming {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            previous: None,
        }
    }

    pub fn observe(&mut self, signals: &Signals, timestamp: InputStreamTimestamp, frames: u64) {
        let epoch = signals.epoch.load(Ordering::Acquire);
        let arrival_ns = elapsed_ns(signals.clock);
        let timing = &signals.timing;
        if let Some((previous_epoch, previous, previous_arrival_ns, previous_frames)) =
            self.previous
        {
            if previous_epoch == epoch {
                timing.max_arrival_interval_ns.fetch_max(
                    arrival_ns.saturating_sub(previous_arrival_ns),
                    Ordering::Relaxed,
                );
                if let Some(interval) = timestamp.callback.checked_duration_since(previous.callback)
                {
                    timing.max_host_interval_ns.fetch_max(
                        interval.as_nanos().min(u64::MAX as u128) as u64,
                        Ordering::Relaxed,
                    );
                }
                // Backend clock discontinuity does not by itself prove missing audio samples.
                if let Some(interval) = timestamp.capture.checked_duration_since(previous.capture) {
                    let expected_ns =
                        u128::from(previous_frames) * 1_000_000_000 / u128::from(self.sample_rate);
                    let actual_ns = interval.as_nanos();
                    // Sub-frame clock rounding is not evidence of a missing input interval.
                    let frame_ns = 1_000_000_000_u128.div_ceil(u128::from(self.sample_rate));
                    if actual_ns > expected_ns + frame_ns {
                        timing.timestamp_gap_count.fetch_add(1, Ordering::Relaxed);
                        timing.max_timestamp_gap_ns.fetch_max(
                            (actual_ns - expected_ns).min(u64::MAX as u128) as u64,
                            Ordering::Relaxed,
                        );
                    } else if expected_ns > actual_ns + frame_ns {
                        timing
                            .timestamp_overlap_count
                            .fetch_add(1, Ordering::Relaxed);
                        timing.max_timestamp_overlap_ns.fetch_max(
                            (expected_ns - actual_ns).min(u64::MAX as u128) as u64,
                            Ordering::Relaxed,
                        );
                    }
                } else {
                    timing
                        .timestamp_reversal_count
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        self.previous = Some((epoch, timestamp, arrival_ns, frames));
        timing.input_frames.fetch_add(frames, Ordering::Relaxed);
        if timing.callback_count.load(Ordering::Relaxed) == 0 {
            timing.first_callback_after_begin_ns.store(
                arrival_ns.saturating_sub(signals.begin_ns.load(Ordering::Relaxed)),
                Ordering::Relaxed,
            );
        }
        timing.callback_count.fetch_add(1, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cpal::StreamInstant;

    fn stamp(ns: u64) -> InputStreamTimestamp {
        InputStreamTimestamp {
            callback: StreamInstant::from_nanos(ns),
            capture: StreamInstant::from_nanos(ns),
        }
    }

    #[test]
    fn first_callback_rounding_and_resume_do_not_invent_gaps() {
        let signals = Signals::new(4);
        let mut timing = CallbackTiming::new(48_000);
        timing.observe(&signals, stamp(1_000_000_000), 512);
        timing.observe(&signals, stamp(1_010_666_667), 256);
        timing.observe(&signals, stamp(1_016_000_000), 512);
        signals.epoch.fetch_add(1, Ordering::Release);
        timing.observe(&signals, stamp(30_000_000_000), 512);
        let result = signals.timing.snapshot(1792, 1792).unwrap();
        assert_eq!(result.callback_count, 4);
        assert_eq!(result.input_frames, 1792);
        assert_eq!(result.timestamp_gap_count, 0);
        assert_eq!(result.timestamp_overlap_count, 0);
        assert_eq!(result.timestamp_reversal_count, 0);
        assert_eq!(result.max_host_interval_ns, 10_666_667);
    }

    #[test]
    fn capture_clock_gaps_overlaps_and_reversals_remain_distinct() {
        let signals = Signals::new(4);
        let mut timing = CallbackTiming::new(1000);
        timing.observe(&signals, stamp(1_000_000_000), 10);
        timing.observe(&signals, stamp(1_020_000_000), 10);
        timing.observe(&signals, stamp(1_025_000_000), 10);
        timing.observe(&signals, stamp(1_000_000_000), 10);
        let result = signals.timing.snapshot(40, 30).unwrap();
        assert_eq!(result.timestamp_gap_count, 1);
        assert_eq!(result.max_timestamp_gap_ns, 10_000_000);
        assert_eq!(result.timestamp_overlap_count, 1);
        assert_eq!(result.max_timestamp_overlap_ns, 5_000_000);
        assert_eq!(result.timestamp_reversal_count, 1);
        assert_eq!(result.queued_frames, 40);
        assert_eq!(result.saved_frames, 30);
    }
}
