//! Short mission briefing entries shown to the player.
//!
//! Handles the data model (primary/secondary briefing lists with
//! per-entry done status).  Widget/UI management lives elsewhere.

use serde::{Deserialize, Serialize};

/// A single short briefing entry.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ShortBriefing {
    pub id: u32,
    pub done: bool,
}

/// Collection of short briefings split into primary and secondary objectives.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ShortBriefings {
    primaries: Vec<ShortBriefing>,
    secondaries: Vec<ShortBriefing>,
}

impl ShortBriefings {
    /// Build a list containing every briefing id `0..count` as a
    /// primary — used by the `DisplayAll` cheat to show every briefing
    /// the level defines, regardless of what the script has added so
    /// far.
    pub fn with_all_briefings(count: u32) -> Self {
        Self {
            primaries: (0..count)
                .map(|id| ShortBriefing { id, done: false })
                .collect(),
            secondaries: Vec::new(),
        }
    }

    /// Add a briefing if it doesn't already exist. Returns true if added.
    pub fn add(&mut self, id: u32, primary: bool) -> bool {
        if self.has(id) {
            return false;
        }
        let entry = ShortBriefing { id, done: false };
        if primary {
            self.primaries.push(entry);
        } else {
            self.secondaries.push(entry);
        }
        true
    }

    /// Mark a briefing as done by ID. Searches primaries first, then secondaries.
    pub fn mark_done(&mut self, id: u32) {
        for entry in self.primaries.iter_mut().chain(self.secondaries.iter_mut()) {
            if entry.id == id {
                entry.done = true;
                return;
            }
        }
    }

    /// Check whether a briefing with the given ID exists.
    pub fn has(&self, id: u32) -> bool {
        self.primaries.iter().any(|e| e.id == id) || self.secondaries.iter().any(|e| e.id == id)
    }

    /// Entries in authored insertion order, with each ID and completion flag
    /// borrowed together. The slice cannot change while it is being displayed.
    pub fn entries(&self, primary: bool) -> &[ShortBriefing] {
        if primary {
            &self.primaries
        } else {
            &self.secondaries
        }
    }

    /// Number of briefings of the given type.
    pub fn count(&self, primary: bool) -> usize {
        self.entries(primary).len()
    }

    /// Clear all briefings.
    pub fn clear(&mut self) {
        self.primaries.clear();
        self.secondaries.clear();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_count() {
        let mut sb = ShortBriefings::default();
        assert!(sb.add(0, true));
        assert!(sb.add(1, true));
        assert!(sb.add(10, false));
        assert_eq!(sb.count(true), 2);
        assert_eq!(sb.count(false), 1);
    }

    #[test]
    fn add_duplicate_rejected() {
        let mut sb = ShortBriefings::default();
        assert!(sb.add(5, true));
        assert!(!sb.add(5, false)); // same ID, different list — still rejected
        assert_eq!(sb.count(true), 1);
        assert_eq!(sb.count(false), 0);
    }

    #[test]
    fn mark_done_primary() {
        let mut sb = ShortBriefings::default();
        sb.add(0, true);
        sb.add(1, true);
        assert!(!sb.entries(true)[0].done);
        sb.mark_done(0);
        assert!(sb.entries(true)[0].done);
        assert!(!sb.entries(true)[1].done);
    }

    #[test]
    fn mark_done_secondary() {
        let mut sb = ShortBriefings::default();
        sb.add(10, false);
        sb.mark_done(10);
        assert!(sb.entries(false)[0].done);
    }

    #[test]
    fn has_checks_both_lists() {
        let mut sb = ShortBriefings::default();
        sb.add(1, true);
        sb.add(2, false);
        assert!(sb.has(1));
        assert!(sb.has(2));
        assert!(!sb.has(3));
    }

    #[test]
    fn entries_preserve_ids_and_order() {
        let mut sb = ShortBriefings::default();
        sb.add(42, true);
        sb.add(99, false);
        assert_eq!(sb.entries(true)[0].id, 42);
        assert_eq!(sb.entries(false)[0].id, 99);
        assert!(sb.entries(true).get(5).is_none());
    }

    #[test]
    fn clear_resets() {
        let mut sb = ShortBriefings::default();
        sb.add(1, true);
        sb.add(2, false);
        sb.clear();
        assert_eq!(sb.count(true), 0);
        assert_eq!(sb.count(false), 0);
        assert!(!sb.has(1));
    }

    #[test]
    fn with_all_briefings_populates_primaries() {
        let sb = ShortBriefings::with_all_briefings(3);
        assert_eq!(sb.count(true), 3);
        assert_eq!(sb.count(false), 0);
        assert_eq!(sb.entries(true)[0].id, 0);
        assert_eq!(sb.entries(true)[1].id, 1);
        assert_eq!(sb.entries(true)[2].id, 2);
        assert!(!sb.entries(true)[0].done);
    }

    #[test]
    fn with_all_briefings_zero_count() {
        let sb = ShortBriefings::with_all_briefings(0);
        assert_eq!(sb.count(true), 0);
        assert_eq!(sb.count(false), 0);
    }

    #[test]
    fn serde_round_trip() {
        let mut sb = ShortBriefings::default();
        sb.add(0, true);
        sb.add(1, true);
        sb.add(10, false);
        sb.mark_done(0);

        let json = serde_json::to_string(&sb).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap(),
            serde_json::json!({
                "primaries": [{"id": 0, "done": true}, {"id": 1, "done": false}],
                "secondaries": [{"id": 10, "done": false}]
            })
        );
        let restored: ShortBriefings = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.count(true), 2);
        assert_eq!(restored.count(false), 1);
        assert!(restored.entries(true)[0].done);
        assert!(!restored.entries(true)[1].done);
        assert_eq!(restored.entries(false)[0].id, 10);
    }
}
