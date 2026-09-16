//! Opt-in CPU presentation and camera sampling diagnostics. These timestamps
//! describe submission to the compositor, not GPU execution or physical scanout.
use std::{cell::RefCell, sync::OnceLock};

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("ROBIN_GAMEPLAY_PROFILE")
            .ok()
            .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
    })
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct CameraSample {
    sampled_at_us: u64,
    x: f32,
    y: f32,
    zoom: f32,
    fixed_tick: bool,
}
impl CameraSample {
    pub(crate) fn capture(
        sampled_at_us: u64,
        viewport: &crate::host::ViewportState,
        fixed_tick: bool,
    ) -> Self {
        Self {
            sampled_at_us,
            x: viewport.view_position.x,
            y: viewport.view_position.y,
            zoom: viewport.zoom_factor,
            fixed_tick,
        }
    }
}

#[derive(Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct SwapchainTiming {
    completed_at_us: u64,
    acquire_us: u64,
    submit_us: u64,
    swap_us: u64,
}
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Timings {
    swapchain: SwapchainTiming,
    previous: Option<(CameraSample, u64)>,
    intervals: Vec<u64>,
    sample_ages: Vec<u64>,
    cadence_errors: Vec<u64>,
    estimated_dropped_frames: u64,
    dropped_frame_events: u64,
}
thread_local! { static TIMINGS: RefCell<Timings> = RefCell::new(Timings::default()); }

pub(crate) fn reset() {
    if enabled() {
        TIMINGS.with(|s| *s.borrow_mut() = Timings::default());
    }
}
pub(crate) fn swapchain(completed_at_us: u64, acquire_us: u64, submit_us: u64, swap_us: u64) {
    if enabled() {
        TIMINGS.with(|s| {
            s.borrow_mut().swapchain = SwapchainTiming {
                completed_at_us,
                acquire_us,
                submit_us,
                swap_us,
            }
        });
    }
}
pub(crate) fn camera(sample: CameraSample, presented: bool) {
    if !enabled() || !presented {
        return;
    }
    TIMINGS.with(|s| s.borrow_mut().record(sample));
}
impl Timings {
    fn record(&mut self, sample: CameraSample) {
        let swap = self.swapchain;
        let age = swap.completed_at_us.saturating_sub(sample.sampled_at_us);
        if let Some((previous, completed_at_us)) = self.previous {
            let interval = swap.completed_at_us.saturating_sub(completed_at_us);
            let sample_interval = sample.sampled_at_us.saturating_sub(previous.sampled_at_us);
            let cadence_error = interval.abs_diff(sample_interval);
            let camera_step = (sample.x - previous.x).hypot(sample.y - previous.y) * sample.zoom;
            let dropped = estimated_dropped_frames(interval);
            self.estimated_dropped_frames += dropped;
            if dropped > 0 {
                self.dropped_frame_events += 1;
                tracing::info!(target: "presentation_perf",
                    target_fps = 60, estimated_dropped_frames = dropped,
                    interval_us = interval, fixed_tick = sample.fixed_tick,
                    acquire_us = swap.acquire_us, submit_us = swap.submit_us, swap_us = swap.swap_us,
                    sample_age_us = age, camera_step_px = camera_step,
                    "dropped frames (estimated from CPU presentation interval)");
            }
            self.intervals.push(interval);
            self.sample_ages.push(age);
            self.cadence_errors.push(cadence_error);
            tracing::debug!(target: "presentation_perf", fixed_tick = sample.fixed_tick,
                presented_at_us = swap.completed_at_us, sampled_at_us = sample.sampled_at_us,
                interval_us = interval, sample_interval_us = sample_interval,
                sample_age_us = age, cadence_error_us = cadence_error,
                acquire_us = swap.acquire_us, submit_us = swap.submit_us, swap_us = swap.swap_us,
                camera_x = sample.x, camera_y = sample.y, zoom = sample.zoom,
                camera_step_px = camera_step, "camera presentation sample");
            if self.intervals.len() == 120 {
                let elapsed: u64 = self.intervals.iter().sum();
                let (interval_p50_us, interval_p95_us, interval_max_us) =
                    quantiles(&mut self.intervals);
                let (_, sample_age_p95_us, sample_age_max_us) = quantiles(&mut self.sample_ages);
                let (_, cadence_error_p95_us, cadence_error_max_us) =
                    quantiles(&mut self.cadence_errors);
                tracing::info!(target: "presentation_perf", frames = 120,
                    target_fps = 60,
                    estimated_dropped_frames = self.estimated_dropped_frames,
                    dropped_frame_events = self.dropped_frame_events,
                    fps = 120_000_000.0 / elapsed.max(1) as f64,
                    interval_p50_us, interval_p95_us, interval_max_us,
                    sample_age_p95_us, sample_age_max_us, cadence_error_p95_us, cadence_error_max_us,
                    "camera presentation timing (CPU, not scanout)");
                self.estimated_dropped_frames = 0;
                self.dropped_frame_events = 0;
                self.intervals.clear();
                self.sample_ages.clear();
                self.cadence_errors.clear();
            }
        }
        self.previous = Some((sample, swap.completed_at_us));
    }
}
// Round to the nearest 60 Hz slot: tolerate sub-half-frame scheduling jitter.
// This is a target-budget estimate, not a display refresh or scanout measurement.
fn estimated_dropped_frames(interval_us: u64) -> u64 {
    ((u128::from(interval_us) * 60 + 500_000) / 1_000_000).saturating_sub(1) as u64
}

fn quantiles(values: &mut [u64]) -> (u64, u64, u64) {
    values.sort_unstable();
    let rank = |percent: usize| values[(values.len() * percent).div_ceil(100) - 1];
    (rank(50), rank(95), values[values.len() - 1])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropped_frame_estimates_tolerate_jitter_and_count_long_stalls() {
        for interval in [0, 8_333, 16_667, 20_835, 24_999] {
            assert_eq!(estimated_dropped_frames(interval), 0);
        }
        assert_eq!(estimated_dropped_frames(25_000), 1);
        assert_eq!(estimated_dropped_frames(33_333), 1);
        assert_eq!(estimated_dropped_frames(50_000), 2);
        assert_eq!(estimated_dropped_frames(543_027), 32);
    }

    #[test]
    fn steady_presents_can_hide_irregular_camera_sampling() {
        let mut timings = Timings::default();
        for (sampled_at_us, completed_at_us) in [(0, 16_000), (8_000, 32_000), (32_000, 48_000)] {
            timings.swapchain.completed_at_us = completed_at_us;
            timings.record(CameraSample {
                sampled_at_us,
                x: sampled_at_us as f32 / 1000.0,
                y: 0.0,
                zoom: 1.0,
                fixed_tick: false,
            });
        }
        assert_eq!(timings.intervals, [16_000, 16_000]);
        assert_eq!(timings.sample_ages, [24_000, 16_000]);
        assert_eq!(timings.cadence_errors, [8_000, 8_000]);
    }
    #[test]
    fn reporting_keeps_the_interval_across_window_boundaries() {
        let mut timings = Timings::default();
        for i in 0..=121 {
            timings.swapchain.completed_at_us = i * 16_000 + 1_000;
            timings.record(CameraSample {
                sampled_at_us: i * 16_000,
                x: 0.0,
                y: 0.0,
                zoom: 1.0,
                fixed_tick: true,
            });
        }
        assert_eq!(timings.intervals, [16_000]);
        assert_eq!(timings.sample_ages, [1_000]);
        assert_eq!(timings.cadence_errors, [0]);
        assert_eq!(quantiles(&mut [9, 2, 1, 4, 3]), (3, 9, 9));
    }
}
