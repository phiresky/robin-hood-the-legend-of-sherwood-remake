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

    /// Number of briefings of the given type.
    pub fn count(&self, primary: bool) -> usize {
        if primary {
            self.primaries.len()
        } else {
            self.secondaries.len()
        }
    }

    /// Get briefing ID at index within the primary or secondary list.
    pub fn get_id(&self, primary: bool, index: usize) -> Option<u32> {
        let list = if primary {
            &self.primaries
        } else {
            &self.secondaries
        };
        list.get(index).map(|e| e.id)
    }

    /// Check whether the entry at index is done.
    pub fn is_entry_done(&self, primary: bool, index: usize) -> Option<bool> {
        let list = if primary {
            &self.primaries
        } else {
            &self.secondaries
        };
        list.get(index).map(|e| e.done)
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
        assert_eq!(sb.is_entry_done(true, 0), Some(false));
        sb.mark_done(0);
        assert_eq!(sb.is_entry_done(true, 0), Some(true));
        assert_eq!(sb.is_entry_done(true, 1), Some(false));
    }

    #[test]
    fn mark_done_secondary() {
        let mut sb = ShortBriefings::default();
        sb.add(10, false);
        sb.mark_done(10);
        assert_eq!(sb.is_entry_done(false, 0), Some(true));
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
    fn get_id() {
        let mut sb = ShortBriefings::default();
        sb.add(42, true);
        sb.add(99, false);
        assert_eq!(sb.get_id(true, 0), Some(42));
        assert_eq!(sb.get_id(false, 0), Some(99));
        assert_eq!(sb.get_id(true, 5), None);
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
        assert_eq!(sb.get_id(true, 0), Some(0));
        assert_eq!(sb.get_id(true, 1), Some(1));
        assert_eq!(sb.get_id(true, 2), Some(2));
        assert_eq!(sb.is_entry_done(true, 0), Some(false));
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
        let restored: ShortBriefings = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.count(true), 2);
        assert_eq!(restored.count(false), 1);
        assert_eq!(restored.is_entry_done(true, 0), Some(true));
        assert_eq!(restored.is_entry_done(true, 1), Some(false));
        assert_eq!(restored.get_id(false, 0), Some(10));
    }
}
