//! Orthogonal runtime semantics for human actors.
//!
//! The retail class hierarchy made four facts coincide: PCs were named
//! heroes, player-controlled, Royalist, and exposed the hero-action UI;
//! soldiers were generic troops, AI-controlled, and usually hostile. Custom
//! missions deliberately break those assumptions. These types describe the
//! independent axes without changing the legacy entity/profile variants.
//!
//! The fields backed by these enums enter serialized engine state at
//! SAVE56/NET23/REPLAY16.

use serde::{Deserialize, Serialize};

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
#[serde(rename_all = "snake_case")]
pub enum HumanArchetype {
    Hero,
    Soldier,
    Civilian,
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
#[serde(rename_all = "snake_case")]
pub enum DecisionPolicy {
    /// Fine-grained hero actions come from a player seat.
    PlayerDirected,
    /// The established malignity/soldier AI owns decisions.
    EnemyAi,
    /// The established civilian/friendly AI owns decisions.
    FriendlyAi,
    /// Mission script and actor sequences own decisions.
    Scripted,
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
#[serde(rename_all = "snake_case")]
pub enum CommandInterface {
    /// No player command surface is exposed.
    #[default]
    None,
    /// Original per-character action palette, quick actions, and abilities.
    HeroActions,
    /// High-level movement, formation, follow, patrol, and stance orders.
    TacticalOrders,
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
#[serde(rename_all = "snake_case")]
pub enum MissionRole {
    /// A hero whose survival participates in ordinary party defeat logic.
    PlayerParty,
    /// A hero waiting for mission-script rescue/activation.
    RescueTarget,
    /// A player-commandable tactical unit which is not a hero-party member.
    TacticalAlly,
    /// An autonomous battle participant, regardless of allegiance.
    #[default]
    Combatant,
    /// A civilian/background actor.
    Civilian,
    /// An actor present for display/testing but excluded from win/loss rules.
    Exhibition,
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
#[serde(rename_all = "snake_case")]
pub enum CombatStance {
    Hold,
    Defensive,
    #[default]
    Aggressive,
}

impl CombatStance {
    pub fn next(self) -> Self {
        match self {
            Self::Hold => Self::Defensive,
            Self::Defensive => Self::Aggressive,
            Self::Aggressive => Self::Hold,
        }
    }
}

/// Read-only semantic view used by gameplay systems. It is derived from the
/// actor's archetype data and attached AI brain so no duplicated controller
/// discriminator can drift out of sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HumanControlProfile {
    pub archetype: HumanArchetype,
    pub decision_policy: DecisionPolicy,
    pub command_interface: CommandInterface,
    pub mission_role: MissionRole,
    pub combat_stance: CombatStance,
}
