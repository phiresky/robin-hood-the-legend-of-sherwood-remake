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

/// Non-null pointer identity in the Original's global element table.
///
/// AI state stores resolved indices in Rust's zero-based entity arena. Slot
/// zero is a real entity (and commonly Robin), so the nominal handle must
/// represent the complete `u32` range; runtime absence lives exclusively in
/// `Option<AiEntityHandle>`.
///
/// This is deliberately distinct from the Original v48 stream encoding:
/// `SerializePointerToElement` uses `54321` as its null marker and the legacy
/// reader translates that marker to `LegacyAiElementRef(None)` before runtime
/// handle resolution.
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
pub struct AiEntityHandle(u32);

impl AiEntityHandle {
    #[inline]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    #[inline]
    pub const fn get(self) -> u32 {
        self.0
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
        Some(AiEntityHandle::new(self))
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

#[derive(Serialize, Deserialize)]
struct TaggedAiEntityHandle {
    entity: u32,
}

/// Encode nullable runtime handles without reintroducing the historical
/// `0 == NULL` ambiguity. A tagged value can represent live arena slot zero;
/// `null` remains absence.
pub(crate) fn serialize_optional_ai_handle<S>(
    handle: &Option<AiEntityHandle>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match handle {
        Some(handle) => serializer.serialize_some(&TaggedAiEntityHandle {
            entity: handle.get(),
        }),
        None => serializer.serialize_none(),
    }
}

/// Decode the tagged current representation. Historical Rust JSON is not
/// accepted here: schema-version gates reject it before runtime state is
/// decoded, while Original C++ pointer sentinels are handled exclusively by
/// `legacy_save`.
pub(crate) fn deserialize_optional_ai_handle<'de, D>(
    deserializer: D,
) -> Result<Option<AiEntityHandle>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<TaggedAiEntityHandle>::deserialize(deserializer)?
        .map(|tagged| AiEntityHandle::new(tagged.entity)))
}

#[cfg(test)]
mod optional_ai_handle_serde_tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct Fixture {
        #[serde(
            serialize_with = "serialize_optional_ai_handle",
            deserialize_with = "deserialize_optional_ai_handle"
        )]
        handle: Option<AiEntityHandle>,
    }

    #[test]
    fn current_schema_rejects_legacy_bare_zero() {
        assert!(serde_json::from_str::<Fixture>(r#"{"handle":0}"#).is_err());
    }

    #[test]
    fn current_schema_rejects_legacy_bare_nonzero() {
        assert!(serde_json::from_str::<Fixture>(r#"{"handle":17}"#).is_err());
    }

    #[test]
    fn current_schema_requires_the_nullable_handle_field() {
        assert!(serde_json::from_str::<Fixture>(r#"{}"#).is_err());
    }

    #[test]
    fn tagged_slot_zero_round_trips_as_live_handle() {
        let fixture = Fixture {
            handle: Some(AiEntityHandle::new(0)),
        };
        let json = serde_json::to_string(&fixture).unwrap();
        assert_eq!(json, r#"{"handle":{"entity":0}}"#);
        assert_eq!(serde_json::from_str::<Fixture>(&json).unwrap(), fixture);
    }

    #[test]
    fn null_round_trips_without_a_fake_handle() {
        let fixture = Fixture { handle: None };
        let json = serde_json::to_string(&fixture).unwrap();
        assert_eq!(json, r#"{"handle":null}"#);
        assert_eq!(serde_json::from_str::<Fixture>(&json).unwrap(), fixture);
    }

    #[test]
    fn native_codec_preserves_slot_zero_and_absence() {
        let handles = [
            None,
            Some(AiEntityHandle::new(0)),
            Some(AiEntityHandle::new(9)),
        ];
        let encoded = bitcode::encode(&handles);
        let decoded: [Option<AiEntityHandle>; 3] = bitcode::decode(&encoded).unwrap();
        assert_eq!(decoded, handles);
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
pub enum CharlySeekerTarget {
    SelfNpc,
    Npc(AiEntityHandle),
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
    Human(AiEntityHandle),
}

impl AiStateChangeSource {
    pub fn from_optional_human(handle: impl IntoOptionalAiHandle) -> Self {
        handle
            .into_optional_ai_handle()
            .map_or(Self::Null, Self::Human)
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
    Engage(AiEntityHandle),
    /// Direct `RHElementActorHuman::EnterSwordFight` call made by
    /// `ReconsiderSwordfight` while rebalancing an existing melee.
    Rebalance(AiEntityHandle),
    /// Direct `RHElementActorHuman::EnterSwordFight` call made by the
    /// already-swordfighting `EVENT_GOTHIT` arm. This synchronously updates
    /// the relationship and, when needed, authors the reciprocal command on
    /// the attacker rather than on the AI receiving the event.
    Direct(AiEntityHandle),
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
