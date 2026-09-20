//! Deterministic cooperative session rules shared by local and network players.
use serde::{Deserialize, Serialize};

pub const MAX_PLAYERS: usize = 5;

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum CharacterControl {
    #[default]
    Shared,
    Exclusive,
    Assigned,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct CoopRules {
    pub players: u8,
    pub control: CharacterControl,
    /// Roster index to copy for each missing player slot.
    pub duplicate_choices: [u8; MAX_PLAYERS],
    pub assignments: [u8; MAX_PLAYERS],
    /// Additional enemy health per duplicate, in percent; zero disables scaling.
    pub enemy_health_per_duplicate: u16,
}

impl Default for CoopRules {
    fn default() -> Self {
        Self {
            players: 1,
            control: CharacterControl::Shared,
            duplicate_choices: [0; MAX_PLAYERS],
            assignments: [0, 1, 2, 3, 4],
            enemy_health_per_duplicate: 25,
        }
    }
}

impl CoopRules {
    pub fn validate(self) -> Result<(), String> {
        if !(1..=MAX_PLAYERS as u8).contains(&self.players) {
            return Err("co-op requires one to five players".into());
        }
        if self.enemy_health_per_duplicate > 200 {
            return Err("co-op health scaling exceeds 200% per duplicate".into());
        }
        if self
            .duplicate_choices
            .iter()
            .any(|&choice| choice >= MAX_PLAYERS as u8)
        {
            return Err("invalid co-op character choice".into());
        }
        let mut assignments = self.assignments;
        assignments.sort_unstable();
        if assignments != [0, 1, 2, 3, 4] {
            return Err("co-op assignments must be a permutation of player slots".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cooperative_rules_validate_player_count_and_unique_assignments() {
        let mut rules = CoopRules::default();
        for players in 1..=5 {
            rules.players = players;
            assert!(rules.validate().is_ok());
        }
        rules.players = 6;
        assert!(rules.validate().is_err());
        rules.players = 2;
        rules.assignments = [0, 0, 2, 3, 4];
        assert!(rules.validate().is_err());
    }
}
