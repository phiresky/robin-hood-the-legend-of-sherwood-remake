//! Reconnaissance reports from soldiers — tracks what they've seen
//! (noise, bodies, enemies).

use serde::{Deserialize, Serialize};

/// The type/severity of a reconnaissance report. Variants are ordered by
/// severity — [`ReconReport::update`] only upgrades, never downgrades.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ReportType {
    #[default]
    Nothing,
    Noise,
    Body,
    MissedCharly,
    DeadBody,
    Enemy,
}

/// A reconnaissance report carried by a soldier.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReconReport {
    pub seek_position: (f32, f32),
    pub report_type: ReportType,
    /// Indices of bodies the soldier has seen.
    pub seen_body_indices: Vec<u32>,
    /// Index of the "charly" (target NPC) this report refers to.
    pub charly_idx: Option<u32>,
    /// Whether the charly has actually been spotted.
    pub charly_seen: bool,
}

impl ReconReport {
    /// Clear the report back to its initial state.
    pub fn reset(&mut self) {
        self.seen_body_indices.clear();
        self.report_type = ReportType::Nothing;
        self.charly_idx = None;
        self.charly_seen = false;
    }

    /// Upgrade the report type and position. Only takes effect when
    /// `new_type` is at least as severe as the current type (monotonic upgrade).
    pub fn update(&mut self, new_type: ReportType, position: (f32, f32)) {
        if new_type >= self.report_type {
            self.report_type = new_type;
            self.seek_position = position;
        }
    }

    /// Record a body index as seen.
    pub fn add_seen_body(&mut self, idx: u32) {
        self.seen_body_indices.push(idx);
    }

    /// Check whether a specific body index has been seen.
    pub fn is_body_seen(&self, idx: u32) -> bool {
        self.seen_body_indices.contains(&idx)
    }

    /// Number of bodies seen so far.
    pub fn get_seen_body_count(&self) -> usize {
        self.seen_body_indices.len()
    }

    /// Set the charly (target NPC) for this report. Resets `charly_seen`
    /// to `false`.
    pub fn set_charly(&mut self, idx: u32) {
        self.charly_idx = Some(idx);
        self.charly_seen = false;
    }

    /// Whether the charly has been spotted.
    pub fn is_charly_seen(&self) -> bool {
        self.charly_seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_clears_state() {
        let mut r = ReconReport::default();
        r.update(ReportType::Enemy, (10.0, 20.0));
        r.add_seen_body(1);
        r.add_seen_body(2);
        r.set_charly(5);
        r.charly_seen = true;

        r.reset();

        assert_eq!(r.report_type, ReportType::Nothing);
        assert!(r.seen_body_indices.is_empty());
        assert_eq!(r.charly_idx, None);
        assert!(!r.charly_seen);
    }

    #[test]
    fn update_upgrades_report_type() {
        let mut r = ReconReport::default();
        assert_eq!(r.report_type, ReportType::Nothing);

        r.update(ReportType::Noise, (1.0, 2.0));
        assert_eq!(r.report_type, ReportType::Noise);
        assert_eq!(r.seek_position, (1.0, 2.0));

        // Same severity — should still update (>=).
        r.update(ReportType::Noise, (3.0, 4.0));
        assert_eq!(r.seek_position, (3.0, 4.0));

        // Higher severity — should upgrade.
        r.update(ReportType::Enemy, (5.0, 6.0));
        assert_eq!(r.report_type, ReportType::Enemy);
        assert_eq!(r.seek_position, (5.0, 6.0));

        // Lower severity — should NOT downgrade.
        r.update(ReportType::Noise, (7.0, 8.0));
        assert_eq!(r.report_type, ReportType::Enemy);
        assert_eq!(r.seek_position, (5.0, 6.0));
    }

    #[test]
    fn body_tracking() {
        let mut r = ReconReport::default();
        assert_eq!(r.get_seen_body_count(), 0);
        assert!(!r.is_body_seen(42));

        r.add_seen_body(42);
        r.add_seen_body(7);

        assert_eq!(r.get_seen_body_count(), 2);
        assert!(r.is_body_seen(42));
        assert!(r.is_body_seen(7));
        assert!(!r.is_body_seen(99));
    }

    #[test]
    fn set_charly_resets_seen_flag() {
        let mut r = ReconReport::default();
        r.set_charly(3);
        assert_eq!(r.charly_idx, Some(3));
        assert!(!r.is_charly_seen());

        r.charly_seen = true;
        assert!(r.is_charly_seen());

        // Setting a new charly resets the seen flag.
        r.set_charly(10);
        assert_eq!(r.charly_idx, Some(10));
        assert!(!r.is_charly_seen());
    }

    #[test]
    fn serde_roundtrip() {
        let mut r = ReconReport::default();
        r.update(ReportType::DeadBody, (100.0, 200.0));
        r.add_seen_body(1);
        r.set_charly(5);
        r.charly_seen = true;

        let json = serde_json::to_string(&r).unwrap();
        let r2: ReconReport = serde_json::from_str(&json).unwrap();

        assert_eq!(r2.report_type, ReportType::DeadBody);
        assert_eq!(r2.seek_position, (100.0, 200.0));
        assert!(r2.is_body_seen(1));
        assert_eq!(r2.charly_idx, Some(5));
        assert!(r2.charly_seen);
    }
}
