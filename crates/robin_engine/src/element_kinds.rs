//! Pure sim-data enums, constants, and helpers extracted from `element.rs`
//! so sim modules can reference them without pulling in the full Entity /
//! Sprite / renderer coupling that the robin_rs side carries.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════
//  Element type classification
// ═══════════════════════════════════════════════════════════════════

/// The concrete type of an entity.
///
/// A Rust enum gives exhaustive pattern matching, replacing what would
/// otherwise be a bitmask of entity types checked with masks.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum ElementKind {
    // Actors
    ActorPc,
    ActorSoldier,
    ActorCivilian,

    // Visual effects
    Fx,
    Target,

    // Objects
    ObjectOther,
    ObjectBonus,
    ObjectScroll,
    ObjectProjectile,
    ObjectNet,
}

impl ElementKind {
    pub fn is_actor(self) -> bool {
        matches!(
            self,
            Self::ActorPc | Self::ActorSoldier | Self::ActorCivilian
        )
    }
    pub fn is_fx(self) -> bool {
        matches!(self, Self::Fx | Self::Target)
    }
    pub fn is_object(self) -> bool {
        matches!(
            self,
            Self::ObjectOther
                | Self::ObjectBonus
                | Self::ObjectScroll
                | Self::ObjectProjectile
                | Self::ObjectNet
        )
    }
    pub fn is_human(self) -> bool {
        matches!(
            self,
            Self::ActorPc | Self::ActorSoldier | Self::ActorCivilian
        )
    }
    pub fn is_pc(self) -> bool {
        matches!(self, Self::ActorPc)
    }
    pub fn is_npc(self) -> bool {
        matches!(self, Self::ActorSoldier | Self::ActorCivilian)
    }
    pub fn is_soldier(self) -> bool {
        matches!(self, Self::ActorSoldier)
    }
    pub fn is_civilian(self) -> bool {
        matches!(self, Self::ActorCivilian)
    }
    pub fn is_fx_base(self) -> bool {
        matches!(self, Self::Fx)
    }
    pub fn is_fx_target(self) -> bool {
        matches!(self, Self::Target)
    }
    pub fn is_bonus(self) -> bool {
        matches!(self, Self::ObjectBonus)
    }
    pub fn is_projectile(self) -> bool {
        matches!(self, Self::ObjectProjectile | Self::ObjectNet)
    }

    /// Whether the sprite's bounding box should participate in the
    /// building-mask occlusion pass.
    pub fn has_valid_box_for_masking(self) -> bool {
        match self {
            Self::ActorPc
            | Self::ActorSoldier
            | Self::ActorCivilian
            | Self::ObjectOther
            | Self::ObjectBonus
            | Self::ObjectScroll
            | Self::ObjectProjectile
            | Self::ObjectNet => true,
            Self::Fx | Self::Target => false,
        }
    }
}

/// Entity condition.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum Condition {
    #[default]
    Ready,
    Waiting,
    Dead,
}

/// Mouse focus / cursor type.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum Focus {
    #[default]
    None,
    Select,
    View,
    Use,
    Bow,
    Sword,
    Hit,
    Apple,
    Stone,
    Lever,
    Heal,
    HealPortrait,
    Shield,
    ShieldPortrait,
    Purse,
    Strangle,
    Interact,
}

/// Outline colour indices.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
#[repr(u8)]
pub enum OutlineColorName {
    #[default]
    Default = 0,
    Target = 1,
    Hidden = 2,
    Striking = 3,
    Parrying = 4,
}

impl OutlineColorName {
    pub const COUNT: usize = 5;
}

/// Predefined RGB565 outline colours.
pub mod outline_colors {
    use robin_util::color::rgb565 as c;

    pub fn pc_default() -> u16 {
        c(165, 255, 82)
    }
    pub fn pc_target() -> u16 {
        c(165, 255, 82)
    }
    pub fn pc_hidden() -> u16 {
        c(165, 255, 82)
    }
    pub fn npc_evil_default() -> u16 {
        c(255, 0, 0)
    }
    pub fn npc_evil_target() -> u16 {
        c(255, 0, 0)
    }
    pub fn npc_evil_hidden() -> u16 {
        c(255, 0, 0)
    }
    pub fn npc_evil_striking() -> u16 {
        c(255, 255, 0)
    }
    pub fn npc_evil_parrying() -> u16 {
        c(0, 0x78, 0xFF)
    }
    pub fn npc_vip_default() -> u16 {
        c(115, 40, 203)
    }
    pub fn npc_vip_target() -> u16 {
        c(115, 40, 203)
    }
    pub fn npc_vip_hidden() -> u16 {
        c(115, 40, 203)
    }
    pub fn npc_good_default() -> u16 {
        c(0, 190, 255)
    }
    pub fn npc_good_target() -> u16 {
        c(0, 190, 255)
    }
    pub fn npc_good_hidden() -> u16 {
        c(0, 190, 255)
    }
    pub fn object_hidden() -> u16 {
        c(255, 247, 90)
    }
    pub fn object_target() -> u16 {
        c(255, 247, 90)
    }
    pub fn target_target() -> u16 {
        c(255, 0, 0)
    }
}

/// Character posture.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum Posture {
    #[default]
    Undefined,
    Upright,
    Unused,
    Lying,
    OnLadder,
    OnWall,
    Siesta,
    Carried,
    Sitting,
    Flying,
    Crouched,
    CarryingCorpse,
    Dead,
    DeadBack,
    HelpingToClimb,
    CarryingOnShoulders,
    OnShoulders,
    StuckUnderNet,
    Tied,
    LeaningOut,
    SimulatingBeggar,
    Spy,
    Tree,
    AnonymousArcher,
    Leisure,
}

impl Posture {
    pub fn is_dead(self) -> bool {
        matches!(self, Self::Dead | Self::DeadBack)
    }
    pub fn is_lying(self) -> bool {
        matches!(
            self,
            Self::Lying | Self::DeadBack | Self::Dead | Self::Tied | Self::StuckUnderNet
        )
    }
    pub fn is_hidden(self) -> bool {
        matches!(self, Self::Spy | Self::Tree | Self::AnonymousArcher)
    }

    pub fn triggers_enemy_near(self) -> bool {
        matches!(
            self,
            Self::Upright
                | Self::Crouched
                | Self::CarryingCorpse
                | Self::HelpingToClimb
                | Self::CarryingOnShoulders
        )
    }
    pub fn is_hurtable_by_arrow(self) -> bool {
        !matches!(self, Self::Spy | Self::Tree)
    }
    /// Guard on posture transitions: a dead corpse can only transition
    /// to `Carried` (pickup); all other non-`Carried` writes on a
    /// `Dead` / `DeadBack` sprite are silently dropped to prevent
    /// "return of the undead".  Any transition from a non-dead posture
    /// is allowed.
    pub fn allows_transition_to(self, new: Posture) -> bool {
        new == Posture::Carried || !self.is_dead()
    }
}

/// Belt height offset for upright humans.
pub const HUMAN_ELEVATION_BELT_UPRIGHT: f32 = 25.0;
/// Belt height offset for mounted soldiers.
pub const RIDER_ELEVATION_BELT_UPRIGHT: f32 = 30.0;

/// Crawling offset X per 16-sector direction.
pub const CRAWLING_OFFSETS_X: [f32; 16] = [
    0.0, 6.1, 11.3, 14.8, 16.0, 14.8, 11.3, 6.1, 0.0, -6.1, -11.3, -14.8, -16.0, -14.8, -11.3, -6.1,
];
/// Crawling offset Y per 16-sector direction.
pub const CRAWLING_OFFSETS_Y: [f32; 16] = [
    -9.18, -8.49, -6.48, -3.5, 0.0, 3.5, 6.48, 8.49, 9.18, 8.49, 6.48, 3.5, 0.0, -3.5, -6.48, -8.49,
];

/// Convert a 16-sector compass direction to a unit 2D vector on the
/// isometric ground plane.
#[inline]
pub fn direction_vector_16(sector: i16) -> (f32, f32) {
    // Original `SBGeoVector2D::SetSector0to15` indexes two literal tables.
    // Computing sin/cos from an f32 angle is not equivalent: rounding the
    // angle first changes several entries by multiple ULPs, which then leaks
    // into trajectory and movement arithmetic.
    const X: [f32; 16] = [
        0.0,
        0.382_683_432_365_09,
        0.707_106_781_186_54,
        0.923_879_532_511_28,
        1.0,
        0.923_879_532_511_28,
        0.707_106_781_186_54,
        0.382_683_432_365_09,
        0.0,
        -0.382_683_432_365_09,
        -0.707_106_781_186_54,
        -0.923_879_532_511_28,
        -1.0,
        -0.923_879_532_511_28,
        -0.707_106_781_186_54,
        -0.382_683_432_365_09,
    ];
    const Y: [f32; 16] = [
        -1.0,
        -0.923_879_532_511_28,
        -0.707_106_781_186_54,
        -0.382_683_432_365_09,
        0.0,
        0.382_683_432_365_09,
        0.707_106_781_186_54,
        0.923_879_532_511_28,
        1.0,
        0.923_879_532_511_28,
        0.707_106_781_186_54,
        0.382_683_432_365_09,
        0.0,
        -0.382_683_432_365_09,
        -0.707_106_781_186_54,
        -0.923_879_532_511_28,
    ];
    let index = sector.rem_euclid(16) as usize;
    (X[index], Y[index])
}

/// Phase of the Listen hero ability.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum ListenPhase {
    #[default]
    Inactive,
    EnterTransition,
    CountingDown,
    ExitTransition,
}

/// Phase of a beggar civilian's `ReceivePurse` animation chain.  The
/// chain queues three orders
/// (`ReceivingPurse` → `WaitingWithPurse` → `TransitionWaitingWithPurseWaitingUpright`)
/// and plays them back-to-back.  The phase is tracked explicitly so the
/// ability system can fire `EngineInner::reveal_scrolls` at the end of
/// `Waiting`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum ReceivePursePhase {
    #[default]
    Inactive,
    Receiving,
    Waiting,
    Transition,
}

/// Action state for actors.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum ActionState {
    #[default]
    Waiting,
    Bored,
    Moving,
    MovingFast,
    AimingWithBow,
    AimingWithBowUp,
    AimingWithBowDown,
    WaitingSword,
    MovingSword,
    MovingFastSword,
    ParryingSword,
    ParryingSwordLow,
    HoldingShield,
    ParryingShield,
    MovingShield,
    Menacing,
    Sleeping,
    Listening,
}

impl ActionState {
    pub fn is_moving(self) -> bool {
        matches!(self, Self::Moving | Self::MovingFast)
    }
    pub fn is_bow(self) -> bool {
        matches!(
            self,
            Self::AimingWithBow | Self::AimingWithBowUp | Self::AimingWithBowDown
        )
    }
    pub fn is_sword(self) -> bool {
        matches!(
            self,
            Self::WaitingSword
                | Self::MovingSword
                | Self::MovingFastSword
                | Self::ParryingSword
                | Self::ParryingSwordLow
        )
    }
    pub fn is_shield(self) -> bool {
        matches!(
            self,
            Self::HoldingShield | Self::ParryingShield | Self::MovingShield
        )
    }

    /// Normalise the action state onto the "moving" variant matching the
    /// actor's current weapon set, with optional speed-tier ratchets.
    ///
    /// Used to snap an actor onto its moving variant (e.g. at Seek
    /// launch) with the correct weapon arm.  Seek continuation is
    /// managed through the sequence manager, so a per-tick "re-stamp"
    /// is not needed.
    ///
    /// States outside the covered switch (e.g. `Flying`-adjacent /
    /// undefined combinations) pass through unchanged.
    pub fn set_moving(self, force_slow: bool, force_fast: bool) -> Self {
        match self {
            // Unarmed / bow / ambient waiting states and MOVING collapse
            // to MOVING (or MOVING_FAST under force_fast).
            Self::Waiting
            | Self::Bored
            | Self::AimingWithBow
            | Self::AimingWithBowUp
            | Self::AimingWithBowDown
            | Self::Sleeping
            | Self::Listening
            | Self::Moving => {
                if force_fast {
                    Self::MovingFast
                } else {
                    Self::Moving
                }
            }
            // Already running: stay fast unless force_slow ratchets down.
            Self::MovingFast => {
                if force_slow {
                    Self::Moving
                } else {
                    Self::MovingFast
                }
            }
            // Sword stances (including parries and menacing) collapse to
            // MOVING_SWORD (or MOVING_FAST_SWORD under force_fast).
            Self::WaitingSword
            | Self::MovingSword
            | Self::ParryingSword
            | Self::ParryingSwordLow
            | Self::Menacing => {
                if force_fast {
                    Self::MovingFastSword
                } else {
                    Self::MovingSword
                }
            }
            Self::MovingFastSword => {
                if force_slow {
                    Self::MovingSword
                } else {
                    Self::MovingFastSword
                }
            }
            // Shield stances all collapse to MOVING_SHIELD — there is no
            // force_slow / force_fast branch here (no MovingFastShield
            // state exists).
            Self::HoldingShield | Self::ParryingShield | Self::MovingShield => Self::MovingShield,
        }
    }
}

/// Motion state (animation progress).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum MotionState {
    #[default]
    Done,
    Start,
    InProgress,
    Terminated,
    Aborted,
    Error,
}

/// Motion method for movement.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum MotionMethod {
    #[default]
    None,
    Walk,
    Run,
    Fast,
    WalkBackwards,
    TillLastFrame,
    Drunken,
    WalkWithoutAntiCollision,
    RunWithoutAntiCollision,
}

/// Faction / camp allegiance.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum Camp {
    Royalists,
    Lacklandists,
    #[default]
    Error,
}

impl Camp {
    pub fn enemy(self) -> Self {
        match self {
            Self::Royalists => Self::Lacklandists,
            Self::Lacklandists => Self::Royalists,
            Self::Error => Self::Error,
        }
    }
    pub fn index(self) -> Option<usize> {
        match self {
            Self::Royalists => Some(0),
            Self::Lacklandists => Some(1),
            Self::Error => None,
        }
    }
}

/// Bow targeting result.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum BowTarget {
    Valid,
    Invalid,
    OutOfRange,
}

/// Sequence-element priority level.
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
)]
pub enum Priority {
    NonInterruptable,
    PostponeEverythingButInjuries,
    Lethal,
    Ko,
    Ko2,
    Injury,
    Script,
    Preference,
    Normal,
    Wait,
    None,
    NotYetSet,
}

/// Priority decision when two sequence elements compete.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum PriorityDecision {
    Abandon,
    Postpone,
    PostponeCurrent,
    InterruptCurrent,
}

/// Object type discriminant.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum ObjectType {
    #[default]
    None,
    VirtualJumper,
    VirtualListen,
    Ale,
    Apple,
    Arrow,
    Stone,
    Purse,
    Coin,
    Net,
    Wasp,
    WaspNest,
    Scroll,
    Cape,
    BonusAmulet,
    BonusAle,
    BonusApple,
    BonusArrow,
    BonusBlazon,
    BonusLambLeg,
    BonusNet,
    BonusPlants,
    BonusPurse,
    BonusRansom,
    BonusStone,
    BonusWaspNest,
    BonusAmpulla,
    BonusCoronationSpoon,
    BonusRichardsCrown,
    BonusRoyalSeal,
    BonusRoyalSceptre,
    BonusDomesdayBook,
    BonusSwordOfTheState,
}

impl ObjectType {
    /// True if the object master is registered as a "variant" master,
    /// causing object/projectile refresh to apply the ambiance
    /// (fog/night) sprite variant.  All other object types (bonuses,
    /// scrolls, virtual markers) render in Day variant only.
    pub fn has_ambiance_variant(&self) -> bool {
        matches!(
            self,
            Self::Arrow
                | Self::Stone
                | Self::Ale
                | Self::Apple
                | Self::Purse
                | Self::WaspNest
                | Self::Cape
                | Self::Net
                | Self::Coin
                | Self::Wasp
        )
    }
}

/// Bonus item type for creation.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum BonusItemType {
    Arrow,
    Stone,
    Apple,
    Ale,
    Lamb,
    Plant,
    Net,
    WaspNest,
    Purse,
    Ransom,
    Amulet,
    Blazon,
    Ampulla,
    CoronationSpoon,
    RichardsCrown,
    RoyalSeal,
    RoyalSceptre,
    DomesdayBook,
    SwordOfTheState,
}

impl BonusItemType {
    /// Decode the raw level-data ordinal into a `BonusItemType`.
    /// Returns `None` for unknown ordinals.
    pub fn from_u16(value: u16) -> Option<Self> {
        Some(match value {
            0 => Self::Arrow,
            1 => Self::Stone,
            2 => Self::Apple,
            3 => Self::Ale,
            4 => Self::Lamb,
            5 => Self::Plant,
            6 => Self::Net,
            7 => Self::WaspNest,
            8 => Self::Purse,
            9 => Self::Ransom,
            10 => Self::Amulet,
            11 => Self::Blazon,
            12 => Self::Ampulla,
            13 => Self::CoronationSpoon,
            14 => Self::RichardsCrown,
            15 => Self::RoyalSeal,
            16 => Self::RoyalSceptre,
            17 => Self::DomesdayBook,
            18 => Self::SwordOfTheState,
            _ => return None,
        })
    }
}

impl ObjectType {
    /// Whether this object type represents a bonus (pickup) item.
    pub fn is_bonus(self) -> bool {
        matches!(
            self,
            Self::BonusAmulet
                | Self::BonusAle
                | Self::BonusApple
                | Self::BonusArrow
                | Self::BonusBlazon
                | Self::BonusLambLeg
                | Self::BonusNet
                | Self::BonusPlants
                | Self::BonusPurse
                | Self::BonusRansom
                | Self::BonusStone
                | Self::BonusWaspNest
                | Self::BonusAmpulla
                | Self::BonusCoronationSpoon
                | Self::BonusRichardsCrown
                | Self::BonusRoyalSeal
                | Self::BonusRoyalSceptre
                | Self::BonusDomesdayBook
                | Self::BonusSwordOfTheState
        )
    }

    /// Whether this object type represents a "unique" (non-stacking)
    /// item.
    ///
    /// The generic stacking bonuses (`BonusAmulet..=BonusWaspNest`) are
    /// the only *non*-unique items — everything else (projectiles,
    /// scrolls, nets, relics) is unique.  `MouseFocus` uses this to
    /// suppress the numeric `GET_YES_N` cursor suffix in favour of the
    /// plain `GET_YES` hand icon.
    pub fn is_unique(self) -> bool {
        !matches!(
            self,
            Self::BonusAmulet
                | Self::BonusAle
                | Self::BonusApple
                | Self::BonusArrow
                | Self::BonusBlazon
                | Self::BonusLambLeg
                | Self::BonusNet
                | Self::BonusPlants
                | Self::BonusPurse
                | Self::BonusRansom
                | Self::BonusStone
                | Self::BonusWaspNest
        )
    }
}

/// Rendering properties for FX elements.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum RenderingProperties {
    #[default]
    Blocky,
    NeedShadow,
}

/// Detectable entity type for NPC AI vision.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum DetectableType {
    Enemy,
    Body,
    Object,
    Friend,
    MissedFriend,
    Beggar,
    #[default]
    None,
}

impl DetectableType {
    pub const COUNT: usize = 6;
}

/// Alert level for NPCs.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum Alert {
    #[default]
    Green,
    Yellow,
    Red,
}

/// Detection level.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum Detection {
    #[default]
    None,
    Unrecognized,
    Recognized,
    Killed,
}

/// NPC attitude towards the player.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum Attitude {
    Friendly,
    #[default]
    Neutral,
    Suspicious,
    Nervous,
    Hostile,
}

/// View cone patterns for NPC detection.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum ViewCone {
    #[default]
    CommandosLike,
    Patrol,
    QuickSearch,
    GetOverview,
    QuickOverview,
    SlowOverview,
    GattlingOverview,
    LookDown,
    LookTo,
    LookToOrCommandosLikeDependingOnIq,
    LookForward,
    Focus,
    GattlingFocus,
    Idle,
    Slow,
    LongRange,
    Sniper,
    SceneOfTheCrime,
    Valium,
}

/// Curiosity trigger types.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum Curiosity {
    Shot,
    Dynamite,
    Siesta,
    Steps,
    Cards,
    Watch,
    Whistle,
}

/// NPC custom value slots.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
#[repr(u8)]
pub enum NpcCustomValue {
    Value1 = 0,
    Value2,
    Value3,
    Value4,
    Value5,
    Value6,
    Value7,
    Value8,
    Value9,
    Value10,
}

impl NpcCustomValue {
    pub const COUNT: usize = 10;
}

/// Quick action type for PCs.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum QuickAction {
    #[default]
    None,
    GoDown,
    GoUp,
    Interact,
}

/// Shooting mode for PCs.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum ShootType {
    #[default]
    Default,
    Roll,
    Ambush,
    Sniper,
    Gattling,
}

/// Work icon displayed in Sherwood.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum WorkIcon {
    Arrows,
    Purses,
    Stones,
    Apples,
    Beer,
    Legs,
    Plants,
    Nets,
    Wasps,
    BowTraining,
    SwordTraining,
    Regeneration,
    #[default]
    None,
}

/// Noise type for sound detection.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum NoiseType {
    Plouf,
    Bonk,
    Zonk,
    TapTapTap,
    ArfArf,
    Tirili,
    PutPut,
    Aaargh,
    Heeelp,
    Pling,
    Pfiiit,
    Logs,
    Drawbridge,
    ZingZing,
    Off,
}

/// Surface material.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum GameMaterial {
    #[default]
    Ground,
    Wood,
    Stone,
    Grass,
    Leaves,
    Water,
    Bush,
    Ice,
    Hole,
    /// Original `RHMATERIAL_NUMBER_OF_MATERIALS` sentinel stored on ordinary
    /// non-material projectile trajectory points.
    NumberOfMaterials,
    LightShadow,
}

impl GameMaterial {
    pub fn try_from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Ground),
            1 => Some(Self::Wood),
            2 => Some(Self::Stone),
            3 => Some(Self::Grass),
            4 => Some(Self::Leaves),
            5 => Some(Self::Water),
            6 => Some(Self::Bush),
            7 => Some(Self::Ice),
            8 => Some(Self::Hole),
            9 => Some(Self::NumberOfMaterials),
            10 => Some(Self::LightShadow),
            _ => None,
        }
    }

    pub fn from_u32(value: u32) -> Self {
        Self::try_from_u32(value)
            .unwrap_or_else(|| panic!("invalid live RHmaterial value: {value}"))
    }

    pub const fn as_u32(self) -> u32 {
        match self {
            Self::Ground => 0,
            Self::Wood => 1,
            Self::Stone => 2,
            Self::Grass => 3,
            Self::Leaves => 4,
            Self::Water => 5,
            Self::Bush => 6,
            Self::Ice => 7,
            Self::Hole => 8,
            Self::NumberOfMaterials => 9,
            Self::LightShadow => 10,
        }
    }

    /// Beam-me material clamp: any value `>= 9` (the walking-material
    /// count) substitutes the grid's default material.  `LightShadow`
    /// (10) also triggers the fallback because it lives past the
    /// sentinel.  Used by mission-stream readers that historically
    /// trusted the file to contain only walking materials.
    pub fn from_u32_with_default(value: u32, default: GameMaterial) -> Self {
        match value {
            0 => Self::Ground,
            1 => Self::Wood,
            2 => Self::Stone,
            3 => Self::Grass,
            4 => Self::Leaves,
            5 => Self::Water,
            6 => Self::Bush,
            7 => Self::Ice,
            8 => Self::Hole,
            _ => default,
        }
    }
}

/// Target type for AI.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum TargetType {
    Pc,
    Npc,
    Scarecrow,
}

/// All game commands.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Default,
    num_enum::TryFromPrimitive,
    robin_state_hash_derive::StateHash,
)]
#[repr(i32)]
#[allow(missing_docs)]
pub enum Command {
    #[default]
    Null,
    Generic,
    AssertPosition,
    AddImpact,
    LockUser,
    UnlockUser,
    CameraGoto,
    CameraJumpTo,
    ZoomLevel,
    DisplayMap,
    UnlockDoor,
    Timer,
    PlayDialog,
    LockCameraOn,
    LockCameraStop,
    SendMessage,
    DisplayPopupText,
    FreezeAll,
    Freeze,
    PassDoor,
    Move,
    Seek,
    MoveOk,
    MoveWaiting,
    Teleport,
    ChangePosition,
    Turn,
    TurnElement,
    TurnFast,
    EquipBow,
    EquipBowUp,
    EquipBowDown,
    UnequipBow,
    LowerBow,
    RaiseBow,
    ShootBow,
    LowerBowLeanOut,
    RaiseBowLeanOut,
    ReceiveDamage,
    ReceiveSwordDamage,
    ReceiveArrowDamage,
    ReceiveStoneDamage,
    ReceiveHitDamage,
    ReceiveMobileDamage,
    ReceiveNet,
    Fall,
    Fainted,
    Recover,
    Knee,
    PrepareSwordfight,
    EnterSwordfight,
    QuitSwordfight,
    ParrySword,
    ParrySwordLow,
    StopParrySword,
    SwordstrikeSmalltalkLeft,
    SwordstrikeSmalltalkRight,
    ParrySmalltalkLeft,
    ParrySmalltalkRight,
    SwordstrikeThrustA,
    SwordstrikeThrustB,
    SwordstrikeThrustC,
    SwordstrikeThrustD,
    SwordstrikeThrustE,
    SwordstrikeThrustF,
    SwordstrikeThrustG,
    SwordstrikeThrustH,
    SwordstrikeThrustI,
    SwordstrikeTired,
    SwordstrikeDown,
    Provoke,
    StandUp,
    DrinkWhisky,
    TakeWhisky,
    GetKilledAtBottom,
    WakeUp,
    /// Dead Original ABI slot `RHCOMMAND_ROLL`.
    Roll,
    CrouchDown,
    CrouchUp,
    ActionAvailable,
    CharacterAvailable,
    SpeakHeroReachDestination,
    SpeakVipsAreForRobin,
    JumpCmd,
    Take,
    SearchCmd,
    HitCmd,
    HealCmd,
    ThrowApple,
    ThrowStone,
    ThrowPurse,
    ThrowWaspNest,
    ThrowNet,
    EnterHelpingClimb,
    LeaveHelpingClimb,
    ClimbUpOnShoulders,
    ClimbDownFromShoulders,
    EnterBeggar,
    LeaveBeggar,
    EnterListen,
    LeaveListen,
    TakeCorpse,
    DropCorpse,
    DropAmmo,
    DropAle,
    EatCmd,
    RaiseShield,
    LowerShield,
    ParryShield,
    HideBehindShield,
    UseLever,
    HandleTarget,
    HitTarget,
    Pay,
    TieCmd,
    StrangleCmd,
    TakeTarget,
    WhistleCmd,
    ReceivePurse,
    Point,
    KickLow,
    LookDown,
    SitDown,
    FlyDoor,
    Untie,
    SendDone,
    AssignPath,
    LockAi,
    UnlockAi,
    Unblip,
    ScriptHit,
    EnterAttentiveMode,
    LeaveAttentiveMode,
    LeaveAttentiveModeOfficer,
    LeanOut,
    LookLeft,
    LookRight,
    ReceiveWaspSting,
    StartMenace,
    StopMenace,
    GatherSoldiers,
    DrinkAle,
    StopSleep,
    BeggarShowFace,
    EnterLeisure,
    /// ABI placeholder for Original's misspelled
    /// `RHCOMMAND_ANNIMAL_AFFRAID`.
    ///
    /// No shipped mission creates animal actors, but omitting the dead
    /// discriminant shifts every following serialized command.
    AnimalAfraid,
    Speak,
    ActivateArrow,
    ActivateSword,
    ActivateHandle,
    ActivateLever,
    ActivateSearch,
    ActivateStone,
    ActivateApple,
    ActivateHeal,
    ActivateMoney,
    StartMobile,
    StopMobile,
    ActivateMobile,
    DeactivateMobile,
    WaitTimer,
    Wait,
    LaunchPostSeek,
    LaunchQuickAction,
    PlayAnim,
    PlayAnimLoop,
    PlayAnimFreeze,
    PlayAnimFrozen,
    Leave,
    WaitFreeLift,
    ReplaceAnim,
    RestoreAnim,
    BodyCheck,
    CrossRailroad,
    StopAll,
    OpenScroll,
    RaiseShieldInstantly,
    RefreshSeek,
    ShootBowOnce,
    /// Rust-only direct jump command. Original's serialized jump command is
    /// [`Self::JumpCmd`].
    Jump,
    /// Rust-only command for leaving the spy disguise.
    LeaveSpy,
    /// Rust-only command for leaving a tree disguise.
    LeaveTree,
}

/// Payload of the sequence `SendMessage` command.
///
/// Original provenance: `original-code/RHScript.cpp:6825-6917` constructs
/// both message entry points with the same three integer properties, while
/// `original-code/RHelementactor.cpp:2939-2966` and
/// `original-code/RHengine.cpp:5185-5197` deliver that exact triple to the
/// actor or global `ProcessMessage` callback respectively.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct SendMessageCommand {
    pub message: i32,
    pub argument: i32,
    pub extended_argument: i32,
}

impl SendMessageCommand {
    pub const fn new(message: i32, argument: i32, extended_argument: i32) -> Self {
        Self {
            message,
            argument,
            extended_argument,
        }
    }
}

/// Typed view of a sequence command.
///
/// Payload-bearing families are migrated out of the legacy generic property
/// bag one family at a time. `Legacy` deliberately retains the remaining
/// command discriminants so callers can adopt this type without a flag-day
/// conversion of every gameplay command.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub enum SequenceCommand {
    SendMessage(SendMessageCommand),
    Legacy(Command),
}

impl SequenceCommand {
    /// Return the legacy discriminant used by existing dispatch switches.
    pub const fn legacy_command(self) -> Command {
        match self {
            Self::SendMessage(_) => Command::SendMessage,
            Self::Legacy(command) => command,
        }
    }
}

impl Command {
    pub fn is_swordstrike(self) -> bool {
        matches!(
            self,
            Self::SwordstrikeThrustA
                | Self::SwordstrikeThrustB
                | Self::SwordstrikeThrustC
                | Self::SwordstrikeThrustD
                | Self::SwordstrikeThrustE
                | Self::SwordstrikeThrustF
                | Self::SwordstrikeThrustG
                | Self::SwordstrikeThrustH
                | Self::SwordstrikeThrustI
        )
    }

    /// True when the command is one of the movement-bearing commands
    /// that embeds an active walk/seek/door/jump step.
    pub fn is_part_of_movement(self) -> bool {
        matches!(
            self,
            Self::Move
                | Self::MoveOk
                | Self::Seek
                | Self::PassDoor
                | Self::Jump
                | Self::AssertPosition
        )
    }
}

bitflags! {
    /// Flags for exiting an action state during transitions.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ExitActionStateFlags: u32 {
        const MUST_BE_WAITING              = 0x0000_0001;
        const CAN_BE_MOVING                = 0x0000_0002;
        const CAN_BE_MOVING_FAST           = 0x0000_0004;
        const CAN_BE_AIMING_BOW            = 0x0000_0008;
        const CAN_BE_AIMING_BOW_UP         = 0x0000_0010;
        const CAN_BE_HOLDING_SWORD         = 0x0000_0020;
        const CAN_BE_PARRYING_SWORD        = 0x0000_0040;
        const CAN_BE_ALERTED               = 0x0000_0080;
        const CAN_BE_BORED                 = 0x0000_0100;
        const CAN_BE_HOLDING_SHIELD        = 0x0000_0200;
        const CAN_BE_PARRYING_SHIELD       = 0x0000_0400;
        const CAN_BE_MENACING              = 0x0000_0800;
        const CAN_BE_SLEEPING              = 0x0000_1000;
        const MUST_BE_LISTENING            = 0x0000_2000;
        const CAN_BE_LISTENING             = 0x0000_4000;
        const CAN_BE_HIDING_BEHIND_SHIELD  = 0x0000_8000;
        const CAN_BE_AIMING_BOW_DOWN       = 0x0001_0000;
    }

    /// Flags for entering an action state during transitions.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct EnterActionStateFlags: u32 {
        const MUST_BE_MOVING         = 0x0000_0001;
        const MUST_BE_MOVING_FAST    = 0x0000_0002;
        const MUST_BE_BORED          = 0x0000_0004;
        const MUST_BE_AIMING_BOW     = 0x0000_0008;
        const MUST_BE_AIMING_BOW_UP  = 0x0000_0010;
        const MUST_BE_HOLDING_SWORD  = 0x0000_0020;
        const MUST_BE_PARRYING_SWORD = 0x0000_0040;
        const MUST_BE_ALERTED        = 0x0000_0080;
        const MUST_BE_HOLDING_SHIELD = 0x0000_0100;
        const MUST_BE_AIMING_BOW_DOWN= 0x0000_0200;
    }

    /// Flags for posture transitions.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ChangePostureFlags: u32 {
        const MUST_BE_UPRIGHT              = 0x0000_0001;
        const MUST_BE_CROUCHED             = 0x0000_0002;
        const MUST_BE_LYING                = 0x0000_0004;
        const MUST_BE_HELPING_TO_CLIMB     = 0x0000_0008;
        const MUST_BE_ON_SHOULDERS         = 0x0000_0010;
        const MUST_BE_CARRYING_CORPSE      = 0x0000_0020;
        const CAN_BE_CROUCHED              = 0x0000_0040;
        const CAN_BE_ON_LADDER             = 0x0000_0080;
        const CAN_BE_ON_WALL               = 0x0000_0100;
        const CAN_BE_ON_SHOULDERS          = 0x0000_0200;
        const CAN_BE_HELPING_TO_CLIMB      = 0x0000_0400;
        const CAN_BE_LYING                 = 0x0000_0800;
        const CAN_BE_CARRYING_CORPSE       = 0x0000_1000;
        const CAN_BE_CARRYING_ON_SHOULDERS = 0x0000_2000;
        const CAN_BE_LEANING_OUT           = 0x0000_4000;
        const MUST_BE_SIMULATING_BEGGAR    = 0x0000_8000;
        const CAN_BE_SIMULATING_BEGGAR     = 0x0001_0000;
        const CAN_BE_LEISURING             = 0x0002_0000;
        const CAN_BE_ANONYMOUS_ARCHER      = 0x0004_0000;
    }

    /// Target action filter flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct TargetFilter: u32 {
        const ARROW  = 0x0001;
        const CUT    = 0x0002;
        const HANDLE = 0x0004;
        const MONEY  = 0x0008;
        const LEVER  = 0x0010;
        const STONE  = 0x0020;
        const APPLE  = 0x0040;
        const HEAL   = 0x0080;
        const SEARCH = 0x0100;
        const LISTEN = 0x0200;
        const TAKE   = 0x0400;
    }
}

#[cfg(test)]
mod tests {
    use super::{ActionState, Command, GameMaterial, direction_vector_16};

    #[test]
    fn direction_vectors_match_original_literal_table_bit_exactly() {
        let expected_x = [
            0x0000_0000,
            0x3ec3_ef15,
            0x3f35_04f3,
            0x3f6c_835e,
            0x3f80_0000,
            0x3f6c_835e,
            0x3f35_04f3,
            0x3ec3_ef15,
            0x0000_0000,
            0xbec3_ef15,
            0xbf35_04f3,
            0xbf6c_835e,
            0xbf80_0000,
            0xbf6c_835e,
            0xbf35_04f3,
            0xbec3_ef15,
        ];
        let expected_y = [
            0xbf80_0000,
            0xbf6c_835e,
            0xbf35_04f3,
            0xbec3_ef15,
            0x0000_0000,
            0x3ec3_ef15,
            0x3f35_04f3,
            0x3f6c_835e,
            0x3f80_0000,
            0x3f6c_835e,
            0x3f35_04f3,
            0x3ec3_ef15,
            0x0000_0000,
            0xbec3_ef15,
            0xbf35_04f3,
            0xbf6c_835e,
        ];

        for sector in 0..16 {
            let (x, y) = direction_vector_16(sector);
            assert_eq!(
                x.to_bits(),
                expected_x[sector as usize],
                "sector {sector} x"
            );
            assert_eq!(
                y.to_bits(),
                expected_y[sector as usize],
                "sector {sector} y"
            );
        }
        assert_eq!(direction_vector_16(-5), direction_vector_16(11));
        assert_eq!(direction_vector_16(27), direction_vector_16(11));
    }

    #[test]
    fn game_material_raw_conversion_is_exact_and_non_clamping() {
        for material in [
            GameMaterial::Ground,
            GameMaterial::Wood,
            GameMaterial::Stone,
            GameMaterial::Grass,
            GameMaterial::Leaves,
            GameMaterial::Water,
            GameMaterial::Bush,
            GameMaterial::Ice,
            GameMaterial::Hole,
            GameMaterial::NumberOfMaterials,
            GameMaterial::LightShadow,
        ] {
            assert_eq!(
                GameMaterial::try_from_u32(material.as_u32()),
                Some(material)
            );
        }
        assert_eq!(GameMaterial::try_from_u32(11), None);
        assert_eq!(GameMaterial::try_from_u32(217_559_952), None);
    }

    #[test]
    fn command_discriminants_retain_original_dead_animal_slot() {
        assert_eq!(Command::EnterLeisure as i32, 144);
        assert_eq!(Command::AnimalAfraid as i32, 145);
        assert_eq!(Command::Speak as i32, 146);
        assert_eq!(Command::WaitTimer as i32, 160);
        assert_eq!(Command::Wait as i32, 161);
        assert_eq!(Command::ShootBowOnce as i32, 178);
        assert_eq!(Command::Jump as i32, 179);
        assert_eq!(Command::LeaveSpy as i32, 180);
        assert_eq!(Command::LeaveTree as i32, 181);
    }

    /// Branch-for-branch verification of `set_moving`.
    #[test]
    fn set_moving_matches_original_switch() {
        // Arm 1: waiting / bored / bow aims / sleep / listen / MOVING.
        for s in [
            ActionState::Waiting,
            ActionState::Bored,
            ActionState::AimingWithBow,
            ActionState::AimingWithBowUp,
            ActionState::AimingWithBowDown,
            ActionState::Sleeping,
            ActionState::Listening,
            ActionState::Moving,
        ] {
            assert_eq!(s.set_moving(false, false), ActionState::Moving);
            assert_eq!(s.set_moving(false, true), ActionState::MovingFast);
            // force_slow has no effect in this arm.
            assert_eq!(s.set_moving(true, false), ActionState::Moving);
        }

        // Arm 2: MOVING_FAST — stay fast unless force_slow ratchets down.
        assert_eq!(
            ActionState::MovingFast.set_moving(false, false),
            ActionState::MovingFast
        );
        assert_eq!(
            ActionState::MovingFast.set_moving(true, false),
            ActionState::Moving
        );
        assert_eq!(
            ActionState::MovingFast.set_moving(false, true),
            ActionState::MovingFast
        );

        // Arm 3: sword waiting / moving / parries / menacing.
        for s in [
            ActionState::WaitingSword,
            ActionState::MovingSword,
            ActionState::ParryingSword,
            ActionState::ParryingSwordLow,
            ActionState::Menacing,
        ] {
            assert_eq!(s.set_moving(false, false), ActionState::MovingSword);
            assert_eq!(s.set_moving(false, true), ActionState::MovingFastSword);
            assert_eq!(s.set_moving(true, false), ActionState::MovingSword);
        }

        // Arm 4: MOVING_FAST_SWORD — stay fast unless force_slow.
        assert_eq!(
            ActionState::MovingFastSword.set_moving(false, false),
            ActionState::MovingFastSword
        );
        assert_eq!(
            ActionState::MovingFastSword.set_moving(true, false),
            ActionState::MovingSword
        );
        assert_eq!(
            ActionState::MovingFastSword.set_moving(false, true),
            ActionState::MovingFastSword
        );

        // Arm 5: shield stances all collapse to MOVING_SHIELD regardless
        // of force flags (no MovingFastShield state exists).
        for s in [
            ActionState::HoldingShield,
            ActionState::ParryingShield,
            ActionState::MovingShield,
        ] {
            assert_eq!(s.set_moving(false, false), ActionState::MovingShield);
            assert_eq!(s.set_moving(false, true), ActionState::MovingShield);
            assert_eq!(s.set_moving(true, false), ActionState::MovingShield);
        }
    }
}
