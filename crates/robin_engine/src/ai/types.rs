use super::*;
use std::num::NonZeroU32;

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

/// Non-null pointer identity in the Original's global element table.
///
/// AI state historically stored nullable `RHElement*` values as raw table
/// indices and used index zero as `NULL`. Runtime fields pair this nominal
/// type with [`Option`], so a present reference cannot accidentally contain
/// the null encoding. Raw zero is accepted only by legacy/serde migration
/// helpers at their boundary.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(transparent)]
pub struct AiEntityHandle(NonZeroU32);

impl AiEntityHandle {
    #[inline]
    pub const fn new(raw: u32) -> Option<Self> {
        match NonZeroU32::new(raw) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }

    #[inline]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl std::fmt::Display for AiEntityHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(f)
    }
}

impl From<AiEntityHandle> for u32 {
    fn from(handle: AiEntityHandle) -> Self {
        handle.get()
    }
}

/// Accepted inputs at AI snapshot lookup boundaries. Raw handles remain
/// supported for required/list entries that have not historically been
/// nullable; optional controller references flow through without being
/// collapsed back to zero first.
pub trait IntoOptionalAiHandle {
    fn into_optional_ai_handle(self) -> Option<AiEntityHandle>;
}

impl IntoOptionalAiHandle for u32 {
    fn into_optional_ai_handle(self) -> Option<AiEntityHandle> {
        AiEntityHandle::new(self)
    }
}

impl IntoOptionalAiHandle for AiEntityHandle {
    fn into_optional_ai_handle(self) -> Option<AiEntityHandle> {
        Some(self)
    }
}

impl IntoOptionalAiHandle for Option<AiEntityHandle> {
    fn into_optional_ai_handle(self) -> Option<AiEntityHandle> {
        self
    }
}

/// Decode both current `null | non-zero` JSON and pre-migration raw handle
/// fields where zero represented `NULL`.
pub(crate) fn deserialize_optional_ai_handle<'de, D>(
    deserializer: D,
) -> Result<Option<AiEntityHandle>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<u32>::deserialize(deserializer).map(|raw| raw.and_then(AiEntityHandle::new))
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
pub enum CharlySeekerTarget {
    SelfNpc,
    Npc(NpcHandle),
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
pub enum AiStateChangeSource {
    SelfActor,
    Null,
    Human(HumanHandle),
}

impl AiStateChangeSource {
    pub fn from_optional_human(handle: impl IntoOptionalAiHandle) -> Self {
        handle
            .into_optional_ai_handle()
            .map_or(Self::Null, |handle| Self::Human(handle.get()))
    }
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

crate::bitcode_adapters::impl_native_bitcode_flags!(AiLockFlags, u8);

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

crate::bitcode_adapters::impl_native_bitcode_flags!(GotoFlags, u16);

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

crate::bitcode_adapters::impl_native_bitcode_flags!(DutyFlags, u16);

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

crate::bitcode_adapters::impl_native_bitcode_flags!(AlertFlags, u16);

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

crate::bitcode_adapters::impl_native_bitcode_flags!(SpeechFlags, u16);

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

crate::bitcode_adapters::impl_native_bitcode_flags!(RemarkTargetFlags, u16);

// ---------------------------------------------------------------------------
// Attention value constants
// ---------------------------------------------------------------------------

pub const MAX_ATT_VALUE: i32 = 100;
pub const THREE_QUARTERS_MAX_ATT_VALUE: i32 = 75;
pub const HALF_MAX_ATT_VALUE: i32 = 50;
pub const QUARTER_MAX_ATT_VALUE: i32 = 25;

// ---------------------------------------------------------------------------
