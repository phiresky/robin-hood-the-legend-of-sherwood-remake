//! Opt-in wall-clock profiling for the host-side gameplay frame.
//!
//! Set `ROBIN_GAMEPLAY_PROFILE=1` to collect timings.  Keeping the switch in a
//! `OnceLock` makes the ordinary disabled path a predictable branch; clocks,
//! thread-local mutation, and logging are only used after explicit opt-in.

use std::cell::RefCell;
use std::sync::OnceLock;

const LOG_INTERVAL: u32 = 120;
const PHASE_COUNT: usize = 8;

#[derive(Clone, Copy)]
pub(super) enum Phase {
    Prepare = 0,
    Simulation = 1,
    Recording = 2,
    Audio = 3,
    Render = 4,
    PostInitialize = 5,
    Pacing = 6,
    Total = 7,
}

#[derive(Default)]
struct Stats {
    frames: u32,
    count: [u32; PHASE_COUNT],
    total_us: [u128; PHASE_COUNT],
    max_us: [u128; PHASE_COUNT],
}

thread_local! {
    static STATS: RefCell<Stats> = RefCell::new(Stats::default());
}

pub(super) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("ROBIN_GAMEPLAY_PROFILE")
            .ok()
            .is_some_and(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
    })
}

pub(super) fn start(enabled: bool) -> Option<web_time::Instant> {
    enabled.then(web_time::Instant::now)
}

pub(super) fn record(phase: Phase, start: Option<web_time::Instant>) {
    let Some(start) = start else {
        return;
    };
    let elapsed_us = start.elapsed().as_micros();
    STATS.with(|cell| {
        let mut stats = cell.borrow_mut();
        let index = phase as usize;
        stats.count[index] += 1;
        stats.total_us[index] += elapsed_us;
        stats.max_us[index] = stats.max_us[index].max(elapsed_us);
        if matches!(phase, Phase::Total) {
            stats.frames += 1;
            if stats.frames >= LOG_INTERVAL {
                log_and_reset(&mut stats);
            }
        }
    });
}

fn average(stats: &Stats, phase: Phase) -> u128 {
    let index = phase as usize;
    stats.total_us[index] / u128::from(stats.count[index].max(1))
}

fn maximum(stats: &Stats, phase: Phase) -> u128 {
    stats.max_us[phase as usize]
}

fn log_and_reset(stats: &mut Stats) {
    tracing::info!(
        target: "robin_rs::game_session::frame_perf",
        frames = stats.frames,
        prepare_avg_us = average(stats, Phase::Prepare),
        prepare_max_us = maximum(stats, Phase::Prepare),
        simulation_avg_us = average(stats, Phase::Simulation),
        simulation_max_us = maximum(stats, Phase::Simulation),
        recording_avg_us = average(stats, Phase::Recording),
        audio_avg_us = average(stats, Phase::Audio),
        render_avg_us = average(stats, Phase::Render),
        render_max_us = maximum(stats, Phase::Render),
        post_initialize_avg_us = average(stats, Phase::PostInitialize),
        pacing_avg_us = average(stats, Phase::Pacing),
        pacing_max_us = maximum(stats, Phase::Pacing),
        total_avg_us = average(stats, Phase::Total),
        total_max_us = maximum(stats, Phase::Total),
        "gameplay frame phase timing"
    );
    *stats = Stats::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averages_use_each_phases_own_sample_count() {
        let mut stats = Stats::default();
        stats.count[Phase::Render as usize] = 2;
        stats.total_us[Phase::Render as usize] = 30;
        assert_eq!(average(&stats, Phase::Render), 15);
        assert_eq!(average(&stats, Phase::Audio), 0);
    }
}
