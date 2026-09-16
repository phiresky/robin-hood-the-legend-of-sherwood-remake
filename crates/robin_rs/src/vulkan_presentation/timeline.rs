//! Interpret only consecutive, comparable display timestamps. Stage-local clocks
//! need not match either the CPU clock or another queried stage's clock.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Serialize, Deserialize)]
struct Sample {
    id: u64,
    time: u64,
    domain: i32,
    domain_id: u64,
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Timeline {
    previous: Option<Sample>,
    intervals: u64,
    elapsed: u64,
    gaps: u64,
    misses: u64,
    max_interval: u64,
}

impl Timeline {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    fn observe(&mut self, next: Sample) -> Option<u64> {
        if next.time == 0 {
            self.reset();
            return None;
        }
        let interval = self
            .previous
            .filter(|old| {
                old.id.checked_add(1) == Some(next.id)
                    && old.time < next.time
                    && old.domain == next.domain
                    && old.domain_id == next.domain_id
            })
            .map(|old| next.time - old.time);
        if interval.is_none() {
            self.reset();
        }
        self.previous = Some(next);
        interval
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn record(
        &mut self,
        id: u64,
        time: u64,
        ready: u64,
        domain: i32,
        domain_id: u64,
        stage: &'static str,
        refresh: u64,
        refresh_interval: u64,
    ) {
        let interval = self.observe(Sample {
            id,
            time,
            domain,
            domain_id,
        });
        if time == 0 {
            // Vulkan defines zero as unavailable, not a discarded-frame signal.
            tracing::debug!(target: "presentation_perf", present_id = id, "display presentation timestamp unavailable");
            return;
        }
        let missed = interval.and_then(|dt| missed_refreshes(dt, refresh, refresh_interval));
        tracing::debug!(target: "presentation_perf", present_id = id, stage,
            displayed_at_ns = time, gpu_ready_at_ns = ready, interval_ns = ?interval, missed_refreshes = ?missed,
            time_domain = domain, time_domain_id = domain_id, "Vulkan display presentation sample");
        if let Some(interval) = interval {
            self.intervals += 1;
            self.elapsed += interval;
            self.max_interval = self.max_interval.max(interval);
            self.misses += missed.unwrap_or(0);
            // This is a measured display interval exceeding our 60 fps budget,
            // including when the driver cannot describe a fixed refresh grid.
            if interval >= 25_000_000 || missed.is_some_and(|n| n > 0) {
                self.gaps += 1;
                tracing::info!(target: "presentation_perf", present_id = id, stage, interval_ns = interval,
                    displayed_at_ns = time, time_domain = domain, time_domain_id = domain_id,
                    target_fps = 60, missed_refreshes = ?missed, refresh_ns = refresh,
                    "display presentation gap (Vulkan feedback)");
            }
            if self.intervals == 120 {
                tracing::info!(target: "presentation_perf", intervals = self.intervals,
                    fps = 1_000_000_000.0 * self.intervals as f64 / self.elapsed as f64,
                    display_gap_events = self.gaps, max_interval_ns = self.max_interval,
                    missed_refreshes = ?missed.map(|_| self.misses), stage,
                    refresh_ns = refresh, refresh_interval_ns = refresh_interval,
                    "Vulkan display presentation summary");
                let previous = self.previous;
                self.reset();
                self.previous = previous;
            }
        }
    }
}

fn missed_refreshes(interval: u64, duration: u64, refresh_interval: u64) -> Option<u64> {
    // Fixed refresh only. VRR and unknown timing have no countable missed slots.
    if duration == 0 || refresh_interval != duration {
        return None;
    }
    Some(
        ((u128::from(interval) + u128::from(duration) / 2) / u128::from(duration)).saturating_sub(1)
            as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(id: u64, time: u64) -> Sample {
        Sample {
            id,
            time,
            domain: 1,
            domain_id: 4,
        }
    }
    #[test]
    fn counts_display_refreshes_without_assuming_sixty_hertz() {
        for period in [6_944_444, 16_666_667, 20_000_000] {
            assert_eq!(missed_refreshes(period, period, period), Some(0));
            assert_eq!(missed_refreshes(3 * period + 100, period, period), Some(2));
        }
        assert_eq!(missed_refreshes(50_000_000, 0, 0), None);
        assert_eq!(missed_refreshes(50_000_000, 16_666_667, u64::MAX), None);
        assert_eq!(missed_refreshes(50_000_000, 16_666_667, 0), None);
    }
    #[test]
    fn never_bridges_missing_feedback_or_clock_changes() {
        let mut t = Timeline::default();
        assert_eq!(t.observe(sample(1, 100)), None);
        assert_eq!(t.observe(sample(2, 120)), Some(20));
        assert_eq!(t.observe(sample(4, 160)), None);
        assert_eq!(t.observe(sample(5, 0)), None);
        assert_eq!(t.observe(sample(6, 200)), None);
        assert_eq!(
            t.observe(Sample {
                domain: 2,
                ..sample(7, 220)
            }),
            None
        );
        assert_eq!(
            t.observe(Sample {
                domain: 2,
                domain_id: 5,
                ..sample(8, 240)
            }),
            None
        );
        assert_eq!(t.observe(sample(9, 20)), None);
        assert_eq!(t.observe(sample(10, 10)), None);
        assert_eq!(t.observe(sample(11, 30)), Some(20));
        t.reset();
        assert_eq!(t.observe(sample(1, 50)), None);
    }
    #[test]
    fn reports_measured_gaps_without_inventing_unknown_refresh_counts() {
        let mut t = Timeline::default();
        t.record(1, 1_000_000, 0, 1, 1, "first_pixel_out", 16_666_667, 0);
        t.record(2, 51_000_000, 0, 1, 1, "first_pixel_out", 16_666_667, 0);
        assert_eq!(t.gaps, 1);
        assert_eq!(t.misses, 0);
        assert_eq!(t.elapsed, 50_000_000);
    }
    #[test]
    fn summary_retains_boundary_interval_but_resize_resets_it() {
        let mut t = Timeline::default();
        for id in 1..=121 {
            t.record(
                id,
                id * 10_000_000,
                0,
                1,
                1,
                "first_pixel_out",
                10_000_000,
                10_000_000,
            );
        }
        assert_eq!(t.intervals, 0);
        t.record(
            122,
            1_250_000_000,
            0,
            1,
            1,
            "first_pixel_out",
            10_000_000,
            10_000_000,
        );
        assert_eq!(t.intervals, 1);
        assert_eq!(t.misses, 3);
        t.reset();
        assert!(t.previous.is_none());
        assert_eq!(t.intervals, 0);
    }
}
