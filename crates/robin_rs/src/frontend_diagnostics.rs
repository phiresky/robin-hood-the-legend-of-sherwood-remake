//! Host-local diagnostics. Draw consumers can inspect observations, but cannot
//! advance the sampling ring or consume output queued by command handlers.
use serde::{Deserialize, Serialize};

const FRAME_SAMPLES: usize = 16;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontendDiagnostics {
    info_displayed: bool,
    frame_samples: [u32; FRAME_SAMPLES],
    #[serde(deserialize_with = "deserialize_sample_cursor")]
    sample_cursor: usize,
    last_tick_ms: u32,
    max_pending_sounds: usize,
    native_refresh_present_cost_us: u64,
    pending_console_output: Vec<String>,
}

fn deserialize_sample_cursor<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<usize, D::Error> {
    let cursor = usize::deserialize(deserializer)?;
    if cursor >= FRAME_SAMPLES {
        return Err(serde::de::Error::custom(
            "frame sample cursor is outside the diagnostics ring",
        ));
    }
    Ok(cursor)
}

impl FrontendDiagnostics {
    pub fn info_displayed(&self) -> bool {
        self.info_displayed
    }
    pub fn set_info_displayed(&mut self, displayed: bool) {
        self.info_displayed = displayed;
    }
    pub fn toggle_info_displayed(&mut self) {
        self.info_displayed = !self.info_displayed;
    }

    /// Call only at the live presentation boundary, never for screenshot draws.
    pub fn record_frame(&mut self, now: u32, pending_sounds: usize) {
        let frame_ms = if self.last_tick_ms == 0 {
            robin_engine::engine::FRAME_TIME_MS
        } else {
            now.saturating_sub(self.last_tick_ms).max(1)
        };
        self.last_tick_ms = now;
        self.frame_samples[self.sample_cursor] = frame_ms;
        self.sample_cursor = (self.sample_cursor + 1) % FRAME_SAMPLES;
        self.max_pending_sounds = self.max_pending_sounds.max(pending_sounds);
    }

    pub fn average_frame_ms(&self) -> u32 {
        (self
            .frame_samples
            .iter()
            .map(|&v| u64::from(v))
            .sum::<u64>()
            / FRAME_SAMPLES as u64)
            .max(1) as u32
    }
    pub fn max_pending_sounds(&self) -> usize {
        self.max_pending_sounds
    }
    pub fn native_refresh_present_cost_us(&self) -> u64 {
        self.native_refresh_present_cost_us
    }
    pub fn observe_present_cost(&mut self, micros: u64) {
        self.native_refresh_present_cost_us = micros;
    }
    pub fn queue_console_output(&mut self, line: String) {
        self.pending_console_output.push(line);
    }
    pub fn take_console_output(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_console_output)
    }
    pub fn clear_console_output(&mut self) {
        self.pending_console_output.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_wraps_and_reading_does_not_advance_it() {
        let mut diagnostics = FrontendDiagnostics::default();
        diagnostics.record_frame(100, 3);
        let snapshot = diagnostics.clone();
        for _ in 0..100 {
            let _ = diagnostics.average_frame_ms();
        }
        assert_eq!(diagnostics, snapshot);
        for frame in 1..=FRAME_SAMPLES {
            diagnostics.record_frame(100 + frame as u32 * 16, 1);
        }
        assert_eq!(diagnostics.average_frame_ms(), 16);
        assert_eq!(diagnostics.max_pending_sounds(), 3);
    }

    #[test]
    fn deferred_output_is_ordered_and_consumed_once() {
        let mut diagnostics = FrontendDiagnostics::default();
        diagnostics.queue_console_output("first".into());
        diagnostics.queue_console_output("second".into());
        assert_eq!(diagnostics.take_console_output(), ["first", "second"]);
        assert!(diagnostics.take_console_output().is_empty());
    }

    #[test]
    fn diagnostic_snapshot_cannot_invalidate_the_sampling_ring() {
        let mut diagnostics = FrontendDiagnostics::default();
        diagnostics.record_frame(100, 1);
        let mut snapshot = serde_json::to_value(&diagnostics).unwrap();
        let restored: FrontendDiagnostics = serde_json::from_value(snapshot.clone()).unwrap();
        assert_eq!(restored, diagnostics);
        snapshot["sample_cursor"] = serde_json::json!(FRAME_SAMPLES);
        assert!(serde_json::from_value::<FrontendDiagnostics>(snapshot).is_err());
    }
}
