//! Ranked board definitions published by the leaderboard service.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    BoardSimulationPolicyV1, OfficialContentEditionV1, OpaqueId, Validate, ValidationError,
};

/// The two ranked dimensions. Ransom remains a verifier-derived stat, not a
/// leaderboard metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardMetricV1 {
    OriginalScore,
    FastestSuccess,
}

/// Content a replay viewer needs to play back a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewerContentRequirementV2 {
    /// The public Demo datadir served with the web runtime.
    BundledDemo,
    /// A user-owned Full installation.
    UserLocalRetail,
}

/// Exact simulation tick duration, as a reduced fraction of microseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickDurationV1 {
    pub numerator_micros: u64,
    pub denominator: u64,
}

impl Validate for TickDurationV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::nonzero("tick_duration.numerator_micros", &self.numerator_micros)?;
        crate::validation::nonzero("tick_duration.denominator", &self.denominator)?;
        let (mut left, mut right) = (self.numerator_micros, self.denominator);
        while right != 0 {
            (left, right) = (right, left % right);
        }
        if left != 1 {
            return Err(ValidationError::ClaimMismatch {
                field: "tick_duration.canonical_fraction",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardMissionV2 {
    pub mission_id: String,
    pub display_name: String,
}

impl Validate for BoardMissionV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::text("board_mission.mission_id", &self.mission_id, 256)?;
        crate::validation::text("board_mission.display_name", &self.display_name, 100)
    }
}

/// One ranked board: a simulation policy over a set of missions of one edition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardV2 {
    pub board_id: OpaqueId,
    pub display_name: String,
    pub edition: OfficialContentEditionV1,
    pub preset_id: String,
    pub preset_name: String,
    pub difficulty_id: String,
    pub difficulty_name: String,
    pub simulation_policy: BoardSimulationPolicyV1,
    pub allow_state_load: bool,
    pub metrics: Vec<BoardMetricV1>,
    pub viewer_content_requirement: ViewerContentRequirementV2,
    pub missions: Vec<BoardMissionV2>,
}

impl BoardV2 {
    pub fn mission(&self, mission_id: &str) -> Option<&BoardMissionV2> {
        self.missions
            .iter()
            .find(|mission| mission.mission_id == mission_id)
    }
}

impl Validate for BoardV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::text("board.display_name", &self.display_name, 100)?;
        for (field, value) in [
            ("board.preset_id", &self.preset_id),
            ("board.preset_name", &self.preset_name),
            ("board.difficulty_id", &self.difficulty_id),
            ("board.difficulty_name", &self.difficulty_name),
        ] {
            crate::validation::text(field, value, 100)?;
        }
        self.simulation_policy.validate()?;
        if self.metrics.is_empty() || !crate::validation::strictly_sorted(&self.metrics) {
            return Err(ValidationError::InvalidMetrics {
                field: "board.metrics",
            });
        }
        if self.missions.is_empty() || self.missions.len() > 4_096 {
            return Err(ValidationError::CountOutOfRange {
                field: "board.missions",
            });
        }
        let mut seen = BTreeSet::new();
        for mission in &self.missions {
            mission.validate()?;
            if !seen.insert(mission.mission_id.as_str()) {
                return Err(ValidationError::Duplicate {
                    field: "board.missions",
                    value: mission.mission_id.clone(),
                });
            }
        }
        Ok(())
    }
}

/// Complete, versioned board catalog used by clients to pick a board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardMetadataV2 {
    pub schema_version: u32,
    pub tick_duration: TickDurationV1,
    pub boards: Vec<BoardV2>,
}

impl LeaderboardMetadataV2 {
    pub fn board(&self, board_id: &OpaqueId) -> Option<&BoardV2> {
        self.boards.iter().find(|board| &board.board_id == board_id)
    }
}

impl Validate for LeaderboardMetadataV2 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema_exact(
            "LeaderboardMetadataV2",
            crate::SCHEMA_VERSION_V2,
            self.schema_version,
        )?;
        self.tick_duration.validate()?;
        if self.boards.len() > 1_024 {
            return Err(ValidationError::CountOutOfRange {
                field: "leaderboard_metadata.boards",
            });
        }
        for board in &self.boards {
            board.validate()?;
        }
        if !self
            .boards
            .windows(2)
            .all(|pair| pair[0].board_id < pair[1].board_id)
        {
            return Err(ValidationError::NotCanonicalOrder {
                field: "leaderboard_metadata.boards",
            });
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{RankedSimulationDifficultyV1, RankedSimulationPolicyV1};

    pub(crate) fn board() -> BoardV2 {
        BoardV2 {
            board_id: OpaqueId::new("demo-standard-normal").unwrap(),
            display_name: "Demo / Standard / Normal".into(),
            edition: OfficialContentEditionV1::Demo,
            preset_id: "standard".into(),
            preset_name: "Standard".into(),
            difficulty_id: "normal".into(),
            difficulty_name: "Normal".into(),
            simulation_policy: BoardSimulationPolicyV1::Fixed {
                policy: RankedSimulationPolicyV1::standard(RankedSimulationDifficultyV1::Medium),
            },
            allow_state_load: true,
            metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
            viewer_content_requirement: ViewerContentRequirementV2::BundledDemo,
            missions: vec![BoardMissionV2 {
                mission_id: "Dem_Lei_MP".into(),
                display_name: "Leicester".into(),
            }],
        }
    }

    #[test]
    fn metadata_requires_sorted_unique_boards_and_missions() {
        let mut metadata = LeaderboardMetadataV2 {
            schema_version: crate::SCHEMA_VERSION_V2,
            tick_duration: TickDurationV1 {
                numerator_micros: 50_000,
                denominator: 1,
            },
            boards: vec![board()],
        };
        assert!(metadata.validate().is_ok());
        assert!(metadata.board(&board().board_id).is_some());
        metadata.boards.push(board());
        assert!(metadata.validate().is_err());
        let mut duplicate_mission = board();
        duplicate_mission
            .missions
            .push(duplicate_mission.missions[0].clone());
        assert!(duplicate_mission.validate().is_err());
    }
}
