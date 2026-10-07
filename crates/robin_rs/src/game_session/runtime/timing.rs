//! Local multiplayer timing and exactly-once hash publication state.
//!
//! This owner does not participate in deterministic engine state or the wire
//! format. Both mission drivers use the same sampling/publication transitions.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct HostFrameSchedule {
    frame: u32,
    deadline_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StateHashSample {
    pub(super) frame: u32,
    pub(super) hash: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(in crate::game_session) struct MultiplayerTiming {
    schedule: Option<HostFrameSchedule>,
    last_sample_frame: Option<u32>,
    pending_hash: Option<StateHashSample>,
    last_clock_ahead_log_ms: u32,
    last_sleep_correction_log_ms: u32,
}

impl MultiplayerTiming {
    pub(in crate::game_session) fn schedule_frame(&self) -> Option<u32> {
        self.schedule.map(|sample| sample.frame)
    }

    pub(in crate::game_session) fn deadline_ms(&self, local_frame: u32) -> Option<i64> {
        let sample = self.schedule?;
        Some(
            sample.deadline_ms
                + (i64::from(local_frame) - i64::from(sample.frame))
                    * i64::from(robin_engine::engine::FRAME_TIME_MS),
        )
    }

    /// Extrapolate at most one normal publication interval. A host pause or
    /// delayed packet must not let a peer simulate an unbounded future.
    pub(in crate::game_session) fn prediction_limit_reached(&self, local_frame: u32) -> bool {
        self.schedule.is_some_and(|sample| {
            local_frame
                >= sample
                    .frame
                    .saturating_add(crate::multiplayer::STATE_HASH_INTERVAL)
        })
    }

    /// Waiting for a fresh clock still needs a normal UI/network poll cadence.
    pub(in crate::game_session) fn pacing_deadline_ms(
        &self,
        local_frame: u32,
        now_ms: u32,
    ) -> Option<i64> {
        if self.prediction_limit_reached(local_frame) {
            Some(i64::from(now_ms) + i64::from(robin_engine::engine::FRAME_TIME_MS))
        } else {
            self.deadline_ms(local_frame)
        }
    }

    /// Reading a shared story consumes wall time, not simulation time. Keep
    /// the last host frame watermark, but don't accumulate catch-up debt while
    /// both peers wait for the acknowledgements.
    pub(in crate::game_session) fn hold_story_clock(&mut self, local_frame: u32, now_ms: u32) {
        if let Some(sample) = &mut self.schedule {
            sample.deadline_ms = i64::from(now_ms)
                + i64::from(robin_engine::engine::FRAME_TIME_MS)
                + (i64::from(sample.frame) - i64::from(local_frame))
                    * i64::from(robin_engine::engine::FRAME_TIME_MS);
        }
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
            deadline_ms: i64::from(now_ms) + i64::from(delay_ms),
        });
        true
    }

    /// BeginSim is also the first authoritative clock anchor. A periodic hash
    /// may have been published before this peer connected, and an early story
    /// barrier can prevent the host from reaching the next hash boundary.
    pub(in crate::game_session) fn accept_start_schedule(
        &mut self,
        frame: u32,
        start_epoch_ms: u64,
        now_epoch_ms: u64,
        now_ms: u32,
    ) {
        if self.schedule.is_some() {
            return;
        }
        let deadline_ms = (i128::from(now_ms) + i128::from(start_epoch_ms)
            - i128::from(now_epoch_ms))
        .clamp(0, i128::from(u32::MAX)) as u32;
        self.schedule = Some(HostFrameSchedule {
            frame,
            deadline_ms: i64::from(deadline_ms),
        });
    }

    pub(super) fn clear_schedule(&mut self) {
        self.schedule = None;
    }

    pub(in crate::game_session) fn sample_hash(
        &mut self,
        frame: u32,
        compute: impl FnOnce() -> u64,
    ) {
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

    pub(in crate::game_session) fn clock_ahead_log_due(&mut self, now_ms: u32) -> bool {
        log_due(&mut self.last_clock_ahead_log_ms, now_ms)
    }

    pub(in crate::game_session) fn sleep_correction_log_due(&mut self, now_ms: u32) -> bool {
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
    fn begin_supplies_a_clock_without_a_periodic_hash() {
        let mut timing = MultiplayerTiming::default();
        timing.accept_start_schedule(0, 1_150, 1_000, 500);
        assert_eq!(timing.deadline_ms(0), Some(650));
        assert_eq!(timing.deadline_ms(1), Some(690));
        timing.accept_start_schedule(0, 1_150, 1_100, 700);
        assert_eq!(timing.deadline_ms(0), Some(650));
        assert!(timing.accept_schedule(25, 10, 1_650));
        assert_eq!(timing.deadline_ms(25), Some(1_660));
        timing.clear_schedule();
        timing.accept_start_schedule(7, 2_000, 2_100, 1_700);
        assert_eq!(timing.deadline_ms(7), Some(1_600));
    }

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
    fn delayed_host_clock_cannot_drive_unbounded_prediction_after_a_pause() {
        let mut timing = MultiplayerTiming::default();
        timing.accept_schedule(26, 23, 1_000);
        // No fresh clock arrives while the host is reading a scroll. Even
        // sixty seconds later, the client may not run thousands of ticks.
        let now = 61_000;
        let mut frame = 26;
        while !timing.prediction_limit_reached(frame) && timing.deadline_ms(frame).unwrap() <= now {
            frame += 1;
        }
        assert_eq!(frame, 51);
        assert_eq!(timing.pacing_deadline_ms(frame, now as u32), Some(now + 40));
        // Host resumes behind the predicted peer; the new sample remains
        // authoritative and keeps the peer held until its frame is due.
        assert!(timing.accept_schedule(40, 20, now as u32));
        assert!(!timing.prediction_limit_reached(frame));
        assert_eq!(timing.deadline_ms(frame), Some(now + 460));
    }

    #[test]
    fn shared_scroll_wait_does_not_become_fast_forward_debt() {
        let mut timing = MultiplayerTiming::default();
        timing.accept_schedule(26, 0, 1_000);
        for now in [1_200, 20_000, 61_000] {
            timing.hold_story_clock(30, now);
            assert_eq!(timing.deadline_ms(30), Some(i64::from(now) + 40));
            assert_eq!(timing.deadline_ms(31), Some(i64::from(now) + 80));
        }
        assert_eq!(timing.schedule_frame(), Some(26));
        assert!(timing.accept_schedule(31, 20, 61_020));
        assert_eq!(timing.deadline_ms(31), Some(61_040));
        timing.clear_schedule();
        assert!(!timing.prediction_limit_reached(0));
        assert_eq!(timing.pacing_deadline_ms(0, 61_100), None);
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
