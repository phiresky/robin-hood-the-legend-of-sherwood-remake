//! Local multiplayer timing and exactly-once hash publication state.
//!
//! This owner does not participate in deterministic engine state or the wire
//! format. Both mission drivers use the same sampling/publication transitions.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct HostFrameSchedule {
    frame: u32,
    deadline_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StateHashSample {
    pub(super) frame: u32,
    pub(super) hash: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct MultiplayerTiming {
    schedule: Option<HostFrameSchedule>,
    last_sample_frame: Option<u32>,
    pending_hash: Option<StateHashSample>,
    last_clock_ahead_log_ms: u32,
    last_sleep_correction_log_ms: u32,
}

impl MultiplayerTiming {
    pub(super) fn schedule_frame(&self) -> Option<u32> {
        self.schedule.map(|sample| sample.frame)
    }

    pub(super) fn deadline_ms(&self, local_frame: u32) -> Option<i64> {
        let sample = self.schedule?;
        Some(
            i64::from(sample.deadline_ms)
                + (i64::from(local_frame) - i64::from(sample.frame))
                    * i64::from(robin_engine::engine::FRAME_TIME_MS),
        )
    }

    pub(super) fn accept_schedule(&mut self, frame: u32, delay_ms: u32, now_ms: u32) -> bool {
        if self
            .schedule_frame()
            .is_some_and(|previous| frame < previous)
        {
            return false;
        }
        self.schedule = Some(HostFrameSchedule {
            frame,
            deadline_ms: now_ms.saturating_add(delay_ms),
        });
        true
    }

    pub(super) fn sample_hash(&mut self, frame: u32, compute: impl FnOnce() -> u64) {
        if frame.is_multiple_of(crate::multiplayer::STATE_HASH_INTERVAL)
            && self.last_sample_frame != Some(frame)
        {
            self.last_sample_frame = Some(frame);
            self.pending_hash = Some(StateHashSample {
                frame,
                hash: compute(),
            });
        }
    }

    pub(super) fn take_publication(&mut self) -> Option<StateHashSample> {
        self.pending_hash.take()
    }

    pub(super) fn begin_host_frame(&mut self) {
        self.pending_hash = None;
    }

    pub(super) fn reset_for_resynchronization(&mut self) {
        self.last_sample_frame = None;
        self.pending_hash = None;
    }

    pub(super) fn clock_ahead_log_due(&mut self, now_ms: u32) -> bool {
        log_due(&mut self.last_clock_ahead_log_ms, now_ms)
    }

    pub(super) fn sleep_correction_log_due(&mut self, now_ms: u32) -> bool {
        log_due(&mut self.last_sleep_correction_log_ms, now_ms)
    }
}

fn log_due(last_ms: &mut u32, now_ms: u32) -> bool {
    if now_ms.saturating_sub(*last_ms) < 1000 {
        return false;
    }
    *last_ms = now_ms;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_schedule_cannot_replace_the_current_clock() {
        let mut timing = MultiplayerTiming::default();
        assert!(timing.accept_schedule(25, 10, 100));
        assert!(!timing.accept_schedule(24, 900, 200));
        assert_eq!(timing.deadline_ms(25), Some(110));
        assert_eq!(timing.deadline_ms(26), Some(150));
        assert_eq!(timing.deadline_ms(24), Some(70));
        assert!(timing.accept_schedule(25, 20, 100));
        assert_eq!(timing.deadline_ms(25), Some(120));
    }

    #[test]
    fn sampling_and_publication_are_once_per_boundary() {
        let mut timing = MultiplayerTiming::default();
        timing.sample_hash(1, || panic!("not a hash boundary"));
        assert_eq!(timing.take_publication(), None);
        timing.sample_hash(25, || 123);
        timing.sample_hash(25, || panic!("duplicate sample"));
        assert_eq!(
            timing.take_publication(),
            Some(StateHashSample {
                frame: 25,
                hash: 123
            })
        );
        assert_eq!(timing.take_publication(), None);
        timing.begin_host_frame();
        timing.sample_hash(25, || panic!("paused frame must not resample"));
    }

    #[test]
    fn log_throttles_are_independent_and_do_not_change_deadlines() {
        let mut timing = MultiplayerTiming::default();
        assert_eq!(timing.deadline_ms(0), None);
        timing.accept_schedule(0, 40, 0);
        assert!(!timing.clock_ahead_log_due(999));
        assert!(timing.clock_ahead_log_due(1000));
        assert!(!timing.clock_ahead_log_due(1001));
        assert!(timing.sleep_correction_log_due(1001));
        assert!(!timing.sleep_correction_log_due(1999));
        assert!(timing.clock_ahead_log_due(2000));
        assert_eq!(timing.deadline_ms(0), Some(40));
    }

    #[test]
    fn resynchronization_discards_old_hash_and_allows_resampling() {
        let mut timing = MultiplayerTiming::default();
        timing.accept_schedule(25, 10, 100);
        timing.sample_hash(25, || 1);
        timing.reset_for_resynchronization();
        assert_eq!(timing.take_publication(), None);
        assert_eq!(timing.deadline_ms(25), Some(110));
        timing.sample_hash(25, || 2);
        assert_eq!(
            timing.take_publication(),
            Some(StateHashSample { frame: 25, hash: 2 })
        );
    }

    #[test]
    fn abandoned_host_frame_cannot_publish_stale_hash() {
        let mut timing = MultiplayerTiming::default();
        timing.sample_hash(25, || 1);
        timing.begin_host_frame();
        assert_eq!(timing.take_publication(), None);
        timing.sample_hash(25, || panic!("same boundary was already sampled"));
        timing.sample_hash(50, || 2);
        assert_eq!(
            timing.take_publication(),
            Some(StateHashSample { frame: 50, hash: 2 })
        );
    }
}
