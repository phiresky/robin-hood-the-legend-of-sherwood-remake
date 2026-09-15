//! Typed ranked simulation policy and the per-board admission policy.

use serde::{Deserialize, Serialize};

use crate::{Validate, ValidationError};

/// Exact engine policy family selected by a ranked board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankedSimulationPresetV1 {
    Standard,
    OriginalParity,
    Custom,
}

impl RankedSimulationPresetV1 {
    /// Stable board facet ID corresponding to this policy.
    pub const fn preset_id(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::OriginalParity => "original",
            Self::Custom => "custom",
        }
    }

    pub const fn preset_name(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::OriginalParity => "Original",
            Self::Custom => "Custom",
        }
    }
}

/// Retail difficulty selected by a ranked simulation policy. Custom and
/// Legendary use the explicit custom policy; they cannot silently enter the
/// Standard/Original board families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankedSimulationDifficultyV1 {
    Easy,
    Medium,
    Hard,
    Legendary,
    Custom,
}

impl RankedSimulationDifficultyV1 {
    /// Public board IDs call the shipped Medium preset `normal`.
    pub const fn difficulty_id(self) -> &'static str {
        match self {
            Self::Easy => "easy",
            Self::Medium => "normal",
            Self::Hard => "hard",
            Self::Legendary => "legendary",
            Self::Custom => "custom",
        }
    }

    pub const fn difficulty_name(self) -> &'static str {
        match self {
            Self::Easy => "Easy",
            Self::Medium => "Normal",
            Self::Hard => "Hard",
            Self::Legendary => "Legendary",
            Self::Custom => "Custom",
        }
    }
}

pub const RANKED_SIMULATION_POLICY_VERSION_V1: u32 = 1;

/// Typed, versioned engine policy. The engine derives the one expected
/// `SimConfig` for fixed presets and admits a validated custom `SimConfig`
/// under the `Custom` preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankedSimulationPolicyV1 {
    pub version: u32,
    pub preset: RankedSimulationPresetV1,
    pub difficulty: RankedSimulationDifficultyV1,
}

impl RankedSimulationPolicyV1 {
    pub const fn standard(difficulty: RankedSimulationDifficultyV1) -> Self {
        Self {
            version: RANKED_SIMULATION_POLICY_VERSION_V1,
            preset: RankedSimulationPresetV1::Standard,
            difficulty,
        }
    }

    pub const fn original_parity(difficulty: RankedSimulationDifficultyV1) -> Self {
        Self {
            version: RANKED_SIMULATION_POLICY_VERSION_V1,
            preset: RankedSimulationPresetV1::OriginalParity,
            difficulty,
        }
    }
}

impl Validate for RankedSimulationPolicyV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.version != RANKED_SIMULATION_POLICY_VERSION_V1 {
            return Err(ValidationError::SchemaVersion {
                document: "RankedSimulationPolicyV1",
                expected: RANKED_SIMULATION_POLICY_VERSION_V1,
                actual: self.version,
            });
        }
        if self.preset != RankedSimulationPresetV1::Custom
            && matches!(
                self.difficulty,
                RankedSimulationDifficultyV1::Legendary | RankedSimulationDifficultyV1::Custom
            )
        {
            return Err(ValidationError::ClaimMismatch {
                field: "ranked_simulation_policy.difficulty",
            });
        }
        Ok(())
    }
}

/// Simulation settings a ranked board admits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoardSimulationPolicyV1 {
    /// Exactly the `SimConfig` of one Standard or Original preset/difficulty.
    Fixed { policy: RankedSimulationPolicyV1 },
    /// Any validated `SimConfig`, verified under a `Custom` policy whose
    /// difficulty is taken from the replayed configuration.
    AnyConfig,
}

impl Validate for BoardSimulationPolicyV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Fixed { policy } => {
                policy.validate()?;
                if policy.preset == RankedSimulationPresetV1::Custom {
                    return Err(ValidationError::ClaimMismatch {
                        field: "board_simulation_policy.fixed.preset",
                    });
                }
                Ok(())
            }
            Self::AnyConfig => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_board_policy_rejects_custom_and_unranked_difficulties() {
        assert!(
            BoardSimulationPolicyV1::Fixed {
                policy: RankedSimulationPolicyV1::standard(RankedSimulationDifficultyV1::Hard),
            }
            .validate()
            .is_ok()
        );
        assert!(
            BoardSimulationPolicyV1::Fixed {
                policy: RankedSimulationPolicyV1::standard(RankedSimulationDifficultyV1::Legendary),
            }
            .validate()
            .is_err()
        );
        assert!(
            BoardSimulationPolicyV1::Fixed {
                policy: RankedSimulationPolicyV1 {
                    version: RANKED_SIMULATION_POLICY_VERSION_V1,
                    preset: RankedSimulationPresetV1::Custom,
                    difficulty: RankedSimulationDifficultyV1::Custom,
                },
            }
            .validate()
            .is_err()
        );
        assert_eq!(
            serde_json::to_value(BoardSimulationPolicyV1::AnyConfig).unwrap(),
            serde_json::json!({ "kind": "any_config" })
        );
    }
}
