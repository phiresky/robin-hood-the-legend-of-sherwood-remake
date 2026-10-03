//! Deterministic cooperative session rules shared by local and network players.
use serde::{Deserialize, Serialize};

pub const MAX_PLAYERS: usize = 5;

/// Stable character codes shared by lobby rules and campaign construction.
pub const TEAM_CHARACTERS: &[(u8, &str, &str)] = &[
    (b'R', "Robin Hood", "Robin des bois"),
    (b'J', "Little John", "Petit Jean"),
    (b'T', "Friar Tuck", "Frere Tuck"),
    (b'S', "Stuteley", "Stutely"),
    (b'W', "Will Scarlet", "Will Ecarlate"),
    (b'M', "Marian", "Lady Marianne"),
    (b'A', "Mustached Merry", "Paysan A"),
    (b'B', "Healing Merry", "Paysan B"),
    (b'C', "Strong Merry", "Paysan C"),
];

pub fn team_character_name(code: u8) -> &'static str {
    TEAM_CHARACTERS
        .iter()
        .find(|entry| entry.0 == code)
        .expect("validated team character")
        .1
}

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
    /// Preserve story progression and the recruited roster between missions.
    #[serde(default)]
    pub campaign: bool,
    pub players: u8,
    pub control: CharacterControl,
    /// Exact party in display order. Trailing zeroes are empty slots;
    /// all zeroes retains automatic mission-party construction.
    #[serde(default)]
    pub team: [u8; MAX_PLAYERS],
    /// Roster index to copy for each missing player slot.
    pub duplicate_choices: [u8; MAX_PLAYERS],
    pub assignments: [u8; MAX_PLAYERS],
    /// Additional enemy health per duplicate, in percent; zero disables scaling.
    pub enemy_health_per_duplicate: u16,
}

impl Default for CoopRules {
    fn default() -> Self {
        Self {
            campaign: false,
            players: 1,
            control: CharacterControl::Shared,
            team: [0; MAX_PLAYERS],
            duplicate_choices: [0; MAX_PLAYERS],
            assignments: [0, 1, 2, 3, 4],
            enemy_health_per_duplicate: 25,
        }
    }
}

impl CoopRules {
    pub fn team_len(&self) -> usize {
        self.team.iter().take_while(|&&code| code != 0).count()
    }

    pub fn team_string(&self) -> Option<String> {
        (self.team_len() > 0).then(|| {
            self.team[..self.team_len()]
                .iter()
                .map(|&c| char::from(c))
                .collect()
        })
    }

    pub fn duplicate_count(&self) -> usize {
        self.team[..self.team_len()]
            .iter()
            .enumerate()
            .filter(|(i, code)| self.team[..*i].contains(code))
            .count()
    }

    pub fn validate(self) -> Result<(), String> {
        if !(1..=MAX_PLAYERS as u8).contains(&self.players) {
            return Err("co-op requires one to five players".into());
        }
        let count = self.team_len();
        if self.campaign && count > 0 {
            return Err("campaign co-op uses the persistent campaign roster".into());
        }
        if self.team[count..].iter().any(|&code| code != 0)
            || self.team[..count]
                .iter()
                .any(|code| !TEAM_CHARACTERS.iter().any(|entry| entry.0 == *code))
        {
            return Err(
                "invalid co-op team: use one to five characters with trailing empty slots".into(),
            );
        }
        if count > 0 && self.control != CharacterControl::Shared && count < self.players as usize {
            return Err(
                "exclusive or assigned control requires at least one character per player".into(),
            );
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
    fn exact_team_allows_repeats_and_is_independent_of_shared_player_count() {
        let rules = CoopRules {
            players: 5,
            team: [b'R', b'R', 0, 0, 0],
            ..Default::default()
        };
        assert!(rules.validate().is_ok());
        assert_eq!(rules.team_string().as_deref(), Some("RR"));
        assert_eq!(rules.duplicate_count(), 1);
        for control in [CharacterControl::Exclusive, CharacterControl::Assigned] {
            assert!(CoopRules { control, ..rules }.validate().is_err());
        }
        let full = CoopRules {
            team: [b'R', b'R', b'T', b'T', b'M'],
            ..rules
        };
        assert!(full.validate().is_ok());
        assert_eq!(full.duplicate_count(), 2);
        assert_eq!(
            bitcode::decode::<CoopRules>(&bitcode::encode(&full)).unwrap(),
            full
        );
        assert_eq!(
            serde_json::from_str::<CoopRules>(&serde_json::to_string(&full).unwrap()).unwrap(),
            full
        );
        assert!(
            CoopRules {
                team: [b'R', 0, b'T', 0, 0],
                ..rules
            }
            .validate()
            .is_err()
        );
        assert!(
            CoopRules {
                team: [b'?', 0, 0, 0, 0],
                ..rules
            }
            .validate()
            .is_err()
        );
    }

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
