use super::*;

// ---------------------------------------------------------------------------
// Opaque entity handle types
// ---------------------------------------------------------------------------

// These `u32` aliases are a transitional layer.  The entity system now has
// `EntityId` (a typed newtype).  New code should prefer `EntityId`; the
// aliases remain so existing code compiles without a mass rewrite.

/// Opaque handle to an NPC actor.
pub type NpcHandle = u32;
/// Opaque handle to a human actor (NPC or PC).
pub type HumanHandle = u32;

/// Opaque handle to a generic element.
pub type ElementHandle = u32;
/// Opaque handle to an object element.
pub type ObjectHandle = u32;
/// Opaque handle to a door.
pub type DoorHandle = u32;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum CharlySeekerTarget {
    SelfNpc,
    Npc(NpcHandle),
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum AiStateChangeSource {
    SelfActor,
    Null,
    Human(HumanHandle),
}

impl AiStateChangeSource {
    pub fn from_optional_human(handle: HumanHandle) -> Self {
        if handle == 0 {
            Self::Null
        } else {
            Self::Human(handle)
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum EnterSwordfightRequest {
    RaiseSword,
    Engage(HumanHandle),
    /// Direct `RHElementActorHuman::EnterSwordFight` call made by
    /// `ReconsiderSwordfight` while rebalancing an existing melee.
    Rebalance(HumanHandle),
    /// Direct `RHElementActorHuman::EnterSwordFight` call made by the
    /// already-swordfighting `EVENT_GOTHIT` arm. This synchronously updates
    /// the relationship and, when needed, authors the reciprocal command on
    /// the attacker rather than on the AI receiving the event.
    Direct(HumanHandle),
}

pub use crate::position_interface::SectorHandle;

// NpcHandle is still a `u32` alias; convert it to an `EntityId` only at
// call sites where the concrete entity type is known.

// ---------------------------------------------------------------------------
// AI lock flags
// ---------------------------------------------------------------------------

bitflags! {
    /// Bitfield controlling when an NPC's AI is locked (ignores stimuli).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct AiLockFlags: u8 {
        const BEGGAR = 0x01;
        const BUSY   = 0x02;
        const FREEZE = 0x04;
    }
}

// ---------------------------------------------------------------------------
// GoTo flags
// ---------------------------------------------------------------------------

bitflags! {
    /// Flags controlling how an NPC moves to a destination.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct GotoFlags: u16 {
        const RUN              = 0x0001;
        const BACK             = 0x0002;
        const STRAIGHT         = 0x0004;
        const STRAFE           = 0x0008;
        const ASK_OBSTACLE     = 0x0010;
        const SPECIAL_ACTION   = 0x0020;
        const USE_NORM         = 0x0040;
        const NEAR             = 0x0080;
        const GROUP_MOVE       = 0x0100;
        const FIND_ACCESSIBLE  = 0x0200;
        const DONT_STOP        = 0x0400;
        const SWORD            = 0x0800;
        const CHARGE           = 0x1000;
        const NO_HALT          = 0x2000;
        const RIDER_CHARGE     = 0x4000;
        const RIDER_CHARGE_HIT = 0x8000;

        /// Flags forbidden for civilian NPCs.
        const FORBIDDEN_CIVILIANS = Self::BACK.bits()
            | Self::SWORD.bits()
            | Self::CHARGE.bits()
            | Self::RIDER_CHARGE.bits()
            | Self::RIDER_CHARGE_HIT.bits();
    }
}

// ---------------------------------------------------------------------------
// Duty flags
// ---------------------------------------------------------------------------

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct DutyFlags: u16 {
        const KEEP_EMOTICON             = 0x0001;
        const BECAUSE_COULDNT_REACHPOINT = 0x0002;
    }
}

// ---------------------------------------------------------------------------
// Alert flags
// ---------------------------------------------------------------------------

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct AlertFlags: u16 {
        const INSTANT_MUSIC_CHANGE = 0x0001;
        const ONLY_MUSIC           = 0x0002;
    }
}

// ---------------------------------------------------------------------------
// Speech flags
// ---------------------------------------------------------------------------

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct SpeechFlags: u16 {
        const HOUSE             = 0x0001;
        const EMERGENCY         = 0x0002;
        const SCRIPT            = 0x0004;
        const ALWAYS            = 0x0008;
        const MYTALK_1          = 0x0100;
        const MYTALK_2          = 0x0200;
        const MYTALK_3          = 0x0400;
        const CYCLE_3_VARIANTS  = 0x0800;
        const MYTALK_0          = 0x1000;
    }
}

// ---------------------------------------------------------------------------
// Remark-target flags
// ---------------------------------------------------------------------------

bitflags! {
    /// Who should hear/see a remark.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct RemarkTargetFlags: u16 {
        const THIS_TYPE      = 0x0001;
        const CIVILIANS      = 0x0002;
        const VILLAINS       = 0x0004;
        const THIS_GUY       = 0x0008;
        const CIV_RESP_VILL  = 0x1000;
        const ALL_NPC        = Self::CIVILIANS.bits() | Self::VILLAINS.bits();
    }
}

// ---------------------------------------------------------------------------
// Attention value constants
// ---------------------------------------------------------------------------

pub const MAX_ATT_VALUE: i32 = 100;
pub const THREE_QUARTERS_MAX_ATT_VALUE: i32 = 75;
pub const HALF_MAX_ATT_VALUE: i32 = 50;
pub const QUARTER_MAX_ATT_VALUE: i32 = 25;

// ---------------------------------------------------------------------------
