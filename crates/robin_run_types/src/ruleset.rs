//! Typed ranked simulation policy and the content-addressed rules configuration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{CanonicalValue, Validate, ValidationError};

/// Exact engine policy implemented by a current ranked rules configuration.
///
/// This is deliberately typed rather than encoded in
/// [`RulesConfigIdentityV1::rules`]. A service may still publish additional
/// ranking predicates in that map, but neither a client nor a verifier is
/// allowed to interpret an arbitrary string as the policy which seals the
/// deterministic engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankedSimulationPresetV1 {
    Standard,
    OriginalParity,
    Custom,
}

impl RankedSimulationPresetV1 {
    /// Stable ruleset facet ID corresponding to this policy.
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

/// Retail difficulty selected by an immutable ranked simulation policy.
/// Custom and Legendary use the explicit custom policy; they cannot silently
/// enter the Standard/Original board families.
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
    /// Existing public board IDs call the shipped Medium preset `normal`.
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

    /// Exact serde spelling used by the engine's `DifficultyLevel` wire type.
    pub const fn sim_config_wire_name(self) -> &'static str {
        match self {
            Self::Easy => "Easy",
            Self::Medium => "Medium",
            Self::Hard => "Hard",
            Self::Legendary => "Legendary",
            Self::Custom => "Custom",
        }
    }
}

pub const RANKED_SIMULATION_POLICY_VERSION_V1: u32 = 1;

/// Typed, versioned engine policy carried inside the content-addressed rules
/// configuration. The complete canonical `SimConfig` remains adjacent to it;
/// the verifier must prove that map is the one fixed configuration generated
/// by this preset/difficulty pair.
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

    pub fn matches_ruleset_labels(
        self,
        preset_id: &str,
        preset_name: &str,
        difficulty_id: &str,
        difficulty_name: &str,
    ) -> bool {
        preset_id == self.preset.preset_id()
            && preset_name == self.preset.preset_name()
            && difficulty_id == self.difficulty.difficulty_id()
            && difficulty_name == self.difficulty.difficulty_name()
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

/// Identity of the full deterministic game configuration and board rules.
///
/// The engine serializes `SimConfig` into `sim_config`; the service publishes
/// eligibility/admission facts in `rules`.  Keeping both exact documents in
/// one content-addressed identity prevents current application defaults from
/// silently redefining an existing board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulesConfigIdentityV1 {
    pub schema_version: u32,
    pub replay_schema_version: u32,
    pub ranked_simulation_policy: RankedSimulationPolicyV1,
    pub sim_config: BTreeMap<String, CanonicalValue>,
    pub rules: BTreeMap<String, CanonicalValue>,
}

impl Validate for RulesConfigIdentityV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("RulesConfigIdentityV1", self.schema_version)?;
        if self.replay_schema_version == 0 {
            return Err(ValidationError::Zero {
                field: "rules_config.replay_schema_version",
            });
        }
        if self.sim_config.is_empty() {
            return Err(ValidationError::Empty {
                field: "rules_config.sim_config",
            });
        }
        if self.rules.is_empty() {
            return Err(ValidationError::Empty {
                field: "rules_config.rules",
            });
        }
        self.ranked_simulation_policy.validate()?;
        let difficulty_matches = match self.ranked_simulation_policy.difficulty {
            RankedSimulationDifficultyV1::Custom => matches!(self.sim_config.get("difficulty"),
                Some(CanonicalValue::Object(value)) if value.len() == 1 && value.contains_key("Custom")),
            difficulty => {
                self.sim_config.get("difficulty")
                    == Some(&CanonicalValue::String(
                        difficulty.sim_config_wire_name().into(),
                    ))
            }
        };
        if !difficulty_matches {
            return Err(ValidationError::ClaimMismatch {
                field: "rules_config.ranked_simulation_policy.difficulty",
            });
        }
        for (key, value) in self.sim_config.iter().chain(&self.rules) {
            crate::validation::text("rules_config.key", key, 256)?;
            value.validate_depth(64)?;
        }
        Ok(())
    }
}
