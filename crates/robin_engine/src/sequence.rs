//! Sequence system — scripted command sequences for entity actions.
//!
//! This is the core infrastructure that drives ALL entity behavior: movement,
//! animations, combat, interactions, and cutscenes. Each entity's current
//! action is driven by a sequence of commands ([`SequenceElement`]s) grouped
//! into command levels that execute in parallel within a level and
//! sequentially across levels.
//!
//! ## Architecture
//!
//! - [`SequenceManager`] owns all active sequences and a deferred dispatch queue.
//! - [`Sequence`] groups [`SequenceElement`]s by command level.
//!   Elements at the same level run concurrently; when all finish, the next level starts.
//! - [`SequenceElement`] carries a [`Command`][crate::element::Command], state machine,
//!   priority, and a list of [`Order`]s (the sub-steps within one command).
//! - The engine calls [`SequenceManager::hourglass`] each frame, which returns
//!   [`SequenceAction`]s for the engine to dispatch to entities.
//!
//! ## Dispatch model
//!
//! We can't call into entities while the SequenceManager is borrowed,
//! so `hourglass()` returns a `Vec<SequenceAction>` that the engine processes.
//! The engine then calls back into the SequenceManager (e.g. [`SequenceManager::element_terminated`])
//! to advance the state machine.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    fmt,
};

use bitflags::bitflags;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::element::{
    ActionState, Command, EntityId, Posture, SendMessageCommand, SequenceCommand,
};
use crate::order::{Order, OrderType};

// ═══════════════════════════════════════════════════════════════════
//  IDs and references
// ═══════════════════════════════════════════════════════════════════

/// Unique identifier for a [`Sequence`].
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
pub struct SequenceId(pub u32);

/// Reference to a specific [`SequenceElement`] within a [`Sequence`].
///
/// `Ord` / `PartialOrd` compare lexicographically on
/// `(sequence_id, element_index)`.  Because `launch_sequence` stamps
/// a monotonic per-engine id and `friday_evening_cleanup` preserves
/// relative order, `SequenceId` order matches `SequenceManager`'s Vec
/// order.  So `min(refs_for_this_actor)` == "first match by linear
/// scan" — the semantic [`SequenceManager::current_element_for_actor`]
/// preserves.
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
pub struct SequenceElementRef {
    pub sequence_id: SequenceId,
    pub element_index: usize,
}

impl SequenceElementRef {
    pub fn new(sequence_id: SequenceId, element_index: usize) -> Self {
        Self {
            sequence_id,
            element_index,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Cascade flags (for state-change propagation)
// ═══════════════════════════════════════════════════════════════════

bitflags! {
    /// Controls how state changes propagate through the sequence element chain.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct CascadeFlags: u16 {
        /// Cascade to the first element at the next command level.
        const NEXT_LEVEL = 0x0001;
        /// Cascade to ALL following elements.
        const FOLLOWING  = 0x0002;
    }
}

crate::bitcode_adapters::impl_native_bitcode_flags!(CascadeFlags, u16);

// ═══════════════════════════════════════════════════════════════════
//  State & priority enums
// ═══════════════════════════════════════════════════════════════════

/// State of a sequence element.
/// Order matters — do not reorder.
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
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum SequenceState {
    Terminated,
    Done,
    InProgress,
    Todo,
    Postponed,
    Impossible,
    Interrupted,
}

/// Priority level for sequence elements.
/// Lower numeric value = higher priority.
/// `>=` comparison means "weaker than or equal".
#[derive(
    Debug,
    Default,
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
pub enum SequencePriority {
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
    #[default]
    NotYetSet,
}

impl SequencePriority {
    /// Whether this priority is `NonInterruptable` — the topmost level
    /// used by falling-pushed, rolling, landing, ladder/wall fall, and
    /// carrier-fall sequences. Animations and sequence elements
    /// carrying this priority must run to completion and must not be
    /// replaced by incoming damage or other lower-priority events.
    #[inline]
    pub fn is_non_interruptable(self) -> bool {
        self == Self::NonInterruptable
    }
}

/// Result of an actor-level instruct arbitration between the actor's
/// currently-executing sequence element and a new one being dispatched.
///
/// Returned by [`decide_priorities`] and consumed by the tick-side
/// dispatcher to decide whether to let the new element proceed, queue
/// it, or bump the current one out of the way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityDecision {
    /// New element is rejected (marked `Impossible`); current continues.
    Abandon,
    /// New element is queued behind current; current continues.
    Postpone,
    /// Current is queued behind new; new takes over.
    PostponeCurrent,
    /// Current is interrupted (state → `Interrupted`); new takes over.
    InterruptCurrent,
}

/// Arbitrate between an actor's currently-executing sequence element and
/// a new one that wants to dispatch.
///
/// This function is the single source of truth for same-actor dispatch
/// ordering. Briefly:
///
/// - `NonInterruptable`: always wins; new is postponed.
/// - `PostponeEverythingButInjuries` / `Lethal` / `Script`: only
///   damage-class priorities can displace them.
/// - `Ko` / `Ko2`: mostly reject new work; `Lethal` interrupts; `Ko2`
///   is additionally interruptable by `Ko`.
/// - `Injury`: interruptable by `Lethal`/`Injury`, otherwise postpones.
/// - `Preference`: interruptable by most things; queues behind damage.
/// - `Normal`: default case; new takes over unless it's `None`/`Wait`
///   (abandoned) or a damage class (which postpones current).
/// - `Wait`: anything other than `None` interrupts it.
/// - `None` (idle): always interrupted.
pub fn decide_priorities(current: SequencePriority, new: SequencePriority) -> PriorityDecision {
    use PriorityDecision::*;
    use SequencePriority::*;
    match current {
        NonInterruptable => Postpone,
        PostponeEverythingButInjuries => match new {
            Lethal => InterruptCurrent,
            Ko | Ko2 | Injury => PostponeCurrent,
            _ => Postpone,
        },
        Lethal => match new {
            Lethal => Abandon,
            Ko | Ko2 | Injury => PostponeCurrent,
            _ => Postpone,
        },
        Ko => match new {
            Lethal => InterruptCurrent,
            PostponeEverythingButInjuries => Postpone,
            _ => Abandon,
        },
        Ko2 => match new {
            Lethal => InterruptCurrent,
            PostponeEverythingButInjuries => Postpone,
            Ko => InterruptCurrent,
            _ => Abandon,
        },
        Injury => match new {
            Lethal | Injury => InterruptCurrent,
            _ => Postpone,
        },
        Script => match new {
            Lethal | Ko | Ko2 => InterruptCurrent,
            PostponeEverythingButInjuries | Injury => PostponeCurrent,
            _ => Postpone,
        },
        Preference => match new {
            Injury | PostponeEverythingButInjuries => PostponeCurrent,
            Lethal | Ko | Ko2 | Script | NonInterruptable | Preference | Normal => InterruptCurrent,
            None | Wait => Abandon,
            NotYetSet => InterruptCurrent, // safety fallback
        },
        Normal => match new {
            NonInterruptable | Preference | Injury | PostponeEverythingButInjuries => {
                PostponeCurrent
            }
            None | Wait => Abandon,
            _ => InterruptCurrent,
        },
        Wait => match new {
            None => Abandon,
            _ => InterruptCurrent,
        },
        None => InterruptCurrent,
        NotYetSet => InterruptCurrent, // safety fallback
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Script-level element kinds (from script Record* natives)
// ═══════════════════════════════════════════════════════════════════

/// The kind of a sequence element, derived from script `Record*` natives.
/// These represent high-level script actions that are built on top of
/// the core sequence infrastructure.
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
pub enum SequenceElementKind {
    // Camera
    ScrollCameraTo,
    JumpCameraTo,
    MoveCameraTo,
    SetZoom,
    LockCameraOn,
    ClearCameraLock,
    DisplayMap,
    // Movement
    Move,
    MoveIntoBuilding,
    EnterGame,
    LeaveGame,
    TurnTo,
    // Animation
    PlayAnim,
    PlayAnimLoop,
    PlayAnimFreeze,
    ReplaceAnim,
    RestoreAnim,
    ResetAnim,
    // Speech / dialogue
    Speak,
    SpeakPC,
    PlayDialog,
    // Timing
    Timer,
    // Seeking
    SeekActor,
    SeekActorMessage,
    SeekActorMessageWithArguments,
    StopSeek,
    // Actions / availability
    Action,
    ActionAvailable,
    CharacterAvailable,
    // Messages
    SendMessage,
    SendMessageWithArguments,
    // AI / user locks
    LockAI,
    UnlockAI,
    LockUser,
    UnlockUser,
    // Mobile elements
    StartMobileElement,
    StopMobileElement,
    ActivateMobileElement,
    DeactivateMobileElement,
    // Corpse handling
    TakeCorpse,
    LeaveCorpse,
}

// ═══════════════════════════════════════════════════════════════════
//  Movement flags
// ═══════════════════════════════════════════════════════════════════

bitflags! {
    /// Movement flags for sequence movement elements.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct MoveFlags: u32 {
        const STRAIGHT               = 0x0000_0001;
        const MAP                    = 0x0000_0002;
        const SEEK                   = 0x0000_0004;
        const NO_ANTICOLLISION       = 0x0000_0008;
        const REVERSED               = 0x0000_0010;
        const CALLED_BY_SCRIPT       = 0x0000_0020;
        const NO_TRANSITIONS         = 0x0000_0040;
        const LINE                   = 0x0000_0080;
        const STEP_BACK_IN_COMBAT    = 0x0000_0100;
        const FORCE_SWORD_MOVEMENT   = 0x0000_0200;
        const USE_POINT              = 0x0000_0400;
        const TO_JUMP                = 0x0000_0800;
        const CHARGE                 = 0x0000_1000;
        const DOOR                   = 0x0000_2000;
        const RIDER_CHARGE           = 0x0000_4000;
        const FAST                   = 0x0000_8000;
        const DIRECTIONAL_TOLERANCE  = 0x0001_0000;
        const SEEK_SHIELD            = 0x0002_0000;
        const SEEK_STOP_NPC          = 0x0004_0000;
        const SEEK_IN_BUILDINGS      = 0x0008_0000;
    }
}

crate::bitcode_adapters::impl_native_bitcode_flags!(MoveFlags, u32);

// ═══════════════════════════════════════════════════════════════════
//  Script recording session
// ═══════════════════════════════════════════════════════════════════

/// Cached origin for an actor that already has an in-flight script
/// move target (point + sector + level).  Used by
/// `RecordingSession::moving_actors`.
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RecordingMotionTarget {
    pub x: f32,
    pub y: f32,
    pub layer: u16,
    pub sector: crate::position_interface::SectorHandle,
}

/// A sequence being built up via script `Record*` calls.
///
/// Flow: `Start()` → `Record*()` → `Then()` → `Record*()` → `Thanx()`
///
/// Elements added between `Start()` and the first `Then()` get command level 1.
/// Each `Then()` bumps the level, so the next batch of `Record*` calls gets
/// a higher level (executed sequentially after the previous level completes).
/// Elements added at the *same* level execute in parallel.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RecordingSession {
    /// Current command level (starts at 1 after `Start()`, incremented by `Then()`).
    pub command_level: u16,
    /// The sequence being built.
    pub sequence: Sequence,
    /// Whether any element was added at the current command level.
    /// Used by `Then()` to only increment when something was actually recorded.
    has_elements_at_current_level: bool,
    /// Per-recording shadow of moving-actor → motion-target.  Key:
    /// actor script handle. Value: cached destination
    /// (x, y, layer, sector) recorded by the most recent
    /// `RecordEnterGame` / `RecordMove*` for that actor.  Used to
    /// suppress the second-call teleport in `RecordEnterGame` and to
    /// seed the *origin* of subsequent `RecordMove` / `RecordMoveNear`
    /// / `RecordTakeCorpse` / `RecordLeaveGame` walks.  Cleared when
    /// the session is finalised by `Thanx`.
    pub moving_actors: HashMap<i32, RecordingMotionTarget>,
}

impl Default for RecordingSession {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingSession {
    /// Start a new recording session.
    pub fn new() -> Self {
        Self {
            command_level: 1,
            sequence: Sequence::new(),
            has_elements_at_current_level: false,
            moving_actors: HashMap::new(),
        }
    }

    /// Add a sequence element at the current command level.
    /// The element's `command_level` is overwritten to match the session's current level.
    ///
    /// Priority is left at the element's default (`NotYetSet`).  Only
    /// the `*_NONINTERRUPTABLE` arms of `RecordMove` / `RecordMoveNear`
    /// raise it explicitly (to `Script` / `Preference`) via the
    /// post-record bump loop.  Callers wanting that bump should either
    /// pass a non-default priority via [`add_element_with_priority`] or
    /// walk the new tail of `sequence.elements` after this call.
    pub fn add_element(&mut self, mut element: SequenceElement) {
        element.command_level = self.command_level;
        self.sequence.append_element(element);
        self.has_elements_at_current_level = true;
    }

    /// Returns the index of the first element added at the current command
    /// level (the snapshot used by the NONINTERRUPTABLE post-record bump
    /// loop in `RecordMove` / `RecordMoveNear`).
    pub fn current_size(&self) -> usize {
        self.sequence.elements.len()
    }

    /// Stamp `priority` on every element in `[from..)` of the recorded
    /// sequence.  Walks every element added by the just-completed
    /// movement-construction call and raises its priority for
    /// NONINTERRUPTABLE styles.
    pub fn bump_priority_from(&mut self, from: usize, priority: SequencePriority) {
        for elem in self.sequence.elements[from..].iter_mut() {
            elem.priority = priority;
        }
    }

    /// Advance to the next command level (called by `Then()`).
    /// Only advances if at least one element was recorded at the current level.
    /// Returns the new command level.
    pub fn advance_level(&mut self) -> u16 {
        if self.has_elements_at_current_level {
            self.command_level += 1;
            self.has_elements_at_current_level = false;
        }
        self.command_level
    }

    /// Finalize the recording and return the built sequence.
    /// Returns `None` if no elements were recorded.
    pub fn finalize(self) -> Option<Sequence> {
        if self.sequence.is_empty() {
            None
        } else {
            Some(self.sequence)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Generic element field system
// ═══════════════════════════════════════════════════════════════════

/// Field identifiers for generic sequence elements.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum Field {
    Direction,
    Event,
    RetainedMovementGoal,
    Timer,
    Message,
    MessageArgument,
    MessageExtendedArgument,
    BowTargetGuy,
    BowTargetPoint,
    CameraPoint,
    CameraZoomLevel,
    CameraSpeed,
    ActionId,
    ActionAvailable,
    CharacterAvailable,
    ConcussionLevel,
    SpeakId,
    SpeakFlags,
    SpeakVariant,
    DialogId,
    DialogSource,
    PopupTextId,
    AnimationId,
    MapDisplay,
    JumplineSource,
    JumplineDestination,
    SwordfightPrepared,
    Amount,
    ShieldDangerPoint,
    ShieldDangerPointLayer,
    ShieldProtected,
    RollPoint,
    PurseTarget,
    NetTarget,
    WaspNestTarget,
    Opponent,
    Gate,
    Door,
    OldAnimation,
    NewAnimation,
    Freeze,
    Scroll,
    ScrollReader,
    ScrollOwner,
    /// Rust extension: 3D landing target for a ground-thrown stone noise
    /// distraction. It has no original-game field ordinal.
    NoiseDistractionTarget,
}

impl Field {
    /// Discriminant used by the original game's field enumeration. Rust's
    /// `RetainedMovementGoal` is an engine cache with no original-game property
    /// entry, so callers must handle it separately instead of shifting every
    /// subsequent field ordinal.
    #[doc(hidden)]
    pub(crate) fn original_ordinal(self) -> Option<u32> {
        use Field::*;
        Some(match self {
            Direction => 0,
            Event => 1,
            RetainedMovementGoal => return None,
            Timer => 2,
            Message => 3,
            MessageArgument => 4,
            MessageExtendedArgument => 5,
            BowTargetGuy => 6,
            BowTargetPoint => 7,
            CameraPoint => 8,
            CameraZoomLevel => 9,
            CameraSpeed => 10,
            ActionId => 11,
            ActionAvailable => 12,
            CharacterAvailable => 13,
            ConcussionLevel => 14,
            SpeakId => 15,
            SpeakFlags => 16,
            SpeakVariant => 17,
            DialogId => 18,
            DialogSource => 19,
            PopupTextId => 20,
            AnimationId => 21,
            MapDisplay => 22,
            JumplineSource => 23,
            JumplineDestination => 24,
            SwordfightPrepared => 34,
            Amount => 25,
            ShieldDangerPoint => 26,
            ShieldDangerPointLayer => 27,
            ShieldProtected => 28,
            RollPoint => 29,
            PurseTarget => 30,
            NetTarget => 31,
            WaspNestTarget => 32,
            Opponent => 33,
            Gate => 35,
            Door => 36,
            OldAnimation => 37,
            NewAnimation => 38,
            Freeze => 39,
            Scroll => 40,
            ScrollReader => 41,
            ScrollOwner => 42,
            NoiseDistractionTarget => return None,
        })
    }
}

/// Polymorphic value stored in a generic sequence element's property map.
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum FieldValue {
    Bool(bool),
    Integer(u32),
    Float(f32),
    GeoPoint2D {
        x: f32,
        y: f32,
    },
    Point3D {
        x: f32,
        y: f32,
        z: f32,
    },
    Element(EntityId),
    /// A legacy property whose key is present even though its pointer is null.
    OptionalElement(Option<EntityId>),
    Animation(OrderType),
    /// Jump-line id: indexes `FastFindGrid::level::jump_lines`.
    /// All call sites (commands::apply_table_swordfight, engine::jump::is_jumpable,
    /// movement::emit_line_goal) pass a jump-line index through this field,
    /// not a motion-grid line index.
    LineId(crate::jump_line::JumpLineIndex),
    /// A legacy line property whose key is present with a nullable pointer.
    OptionalLineId(Option<crate::jump_line::JumpLineIndex>),
    /// Opaque door ID.
    DoorId(crate::gate::DoorIndex),
    /// A legacy gate property whose key is present with a nullable pointer.
    OptionalDoorId(Option<crate::gate::DoorIndex>),
}

/// A violated sequence construction invariant.
///
/// These errors are exposed through checked construction methods. The legacy
/// convenience methods panic with the same error instead of silently dropping
/// an invalid order or changing an invalid insertion into an append.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceInvariantError {
    InvalidOrderAction,
    OrderInsertionOutOfBounds { index: usize, len: usize },
    NonContiguousCommandLevel { previous: u16, next: u16 },
    LegacyCommandRequiresGenericData { command: Command },
    MissingLegacyCommandField { command: Command, field: Field },
    InvalidLegacyCommandFieldType { command: Command, field: Field },
    NestedPostSeekSequence,
}

impl fmt::Display for SequenceInvariantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOrderAction => write!(formatter, "order action is Invalid"),
            Self::OrderInsertionOutOfBounds { index, len } => write!(
                formatter,
                "order insertion index {index} is out of bounds for length {len}"
            ),
            Self::NonContiguousCommandLevel { previous, next } => write!(
                formatter,
                "command level must stay at {previous} or advance to {}; got {next}",
                previous.saturating_add(1)
            ),
            Self::LegacyCommandRequiresGenericData { command } => {
                write!(
                    formatter,
                    "legacy command {command:?} requires generic data"
                )
            }
            Self::MissingLegacyCommandField { command, field } => write!(
                formatter,
                "legacy command {command:?} is missing required field {field:?}"
            ),
            Self::InvalidLegacyCommandFieldType { command, field } => write!(
                formatter,
                "legacy command {command:?} has the wrong value type for field {field:?}"
            ),
            Self::NestedPostSeekSequence => write!(
                formatter,
                "post-seek sequences cannot themselves contain post-seek sequences"
            ),
        }
    }
}

impl std::error::Error for SequenceInvariantError {}

// ═══════════════════════════════════════════════════════════════════
//  Element subtype data
// ═══════════════════════════════════════════════════════════════════

/// Element subtypes — variants for simple, movement, generic, damage,
/// and interaction elements.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum SequenceElementData<P: robin_util::state_hash::StateHash = Option<PostSeekSequence>> {
    /// Base type with no extra data.
    Simple,

    /// Movement-specific data.
    Movement {
        /// Projected map-space destination. The original game stores and compares it against
        /// the map position.
        destination: crate::coordinates::MapPoint,
        layer: u16,
        /// Sector handle (`None` = no sector constraint).
        sector: Option<crate::position_interface::SectorHandle>,
        /// Gate reference for door passing.
        gate_id: Option<crate::gate::DoorIndex>,
        /// Jump-line reference for line-targeted movement
        /// (`MoveFlags::LINE`).  Indexes
        /// `FastFindGrid::level::jump_lines`.
        line_id: Option<crate::jump_line::JumpLineIndex>,
        /// Target element for seek/assert.
        element: Option<EntityId>,
        flags: MoveFlags,
        tolerance: f32,
        direction: i16,
        action: OrderType,
        speed_factor: f32,
        /// Post-seek sequence: launched by the actor when the SEEK
        /// command completes (target lost/reached or self-seek
        /// collapsed).  When the SEEK is dispatched, the actor copies
        /// this onto its `ActorData::post_seek_sequence` and clears it
        /// here.
        ///
        /// **Ownership invariant for `Clone`:** the auto-derived
        /// `Clone` on `SequenceElement` deep-clones this continuation,
        /// which is fine for `Engine`-level rollback snapshots (each
        /// clone is an independent timeline) but is semantically wrong
        /// for "duplicate this element within the same engine" — both
        /// copies would launch the same post-seek chain.  Today no
        /// caller does that (the duplicate-element use site has been
        /// replaced by `macro_store::QaReplayCommand`, which records
        /// semantic player commands instead of cloning elements); if a
        /// future caller needs ownership-transfer semantics, replace
        /// the `clone()` call with a hand-written
        /// `create_copy(&mut self)` that `mem::take`s this field.
        /// Root elements use `Option<PostSeekSequence>` here. Elements inside
        /// a `PostSeekSequence` instantiate this generic with `()`, making the
        /// representation structurally non-recursive.
        post_seek_sequence: P,
    },

    /// Generic property-bag element.
    Generic {
        properties: HashMap<Field, FieldValue>,
    },

    /// Damage element.
    ///
    /// Carries all the data needed by the victim's instruction handler to
    /// apply and animate the damage.
    Damage {
        /// Origin of the damage (attacker entity).
        origin: Option<EntityId>,
        /// Projectile whose deferred impact this element represents.
        ///
        /// Original-game arrow damage keeps the arrow reference until the
        /// sequence-manager phase so the victim's final facing can be read
        /// after damage translation. Runtime projectiles remain as
        /// tombstones long enough for this reference to stay valid.
        #[serde(deserialize_with = "Option::deserialize")]
        projectile: Option<EntityId>,
        /// Raw damage value (for generic/arrow/stone).
        damage: u16,
        /// Concussion value (for generic/hit).
        concussion: u16,
        /// Sword strike type (for sword damage).
        sword_strike: Option<crate::weapons::SwordStrike>,
        /// Attacker's weapon profile index (for sword damage).
        /// Used to look up the `HtHWeaponProfile` in `ProfileManager`.
        sword_profile_idx: Option<u32>,
        /// Whether this was a harder hit.
        is_harder_hit: bool,
    },

    /// Interaction element.
    Interaction {
        /// The entity to interact with.
        antagonist: Option<EntityId>,
    },
}

impl<P: robin_util::state_hash::StateHash> SequenceElementData<P> {
    fn try_map_post_seek<Q: robin_util::state_hash::StateHash, E>(
        self,
        map: impl FnOnce(P) -> Result<Q, E>,
    ) -> Result<SequenceElementData<Q>, E> {
        Ok(match self {
            Self::Simple => SequenceElementData::Simple,
            Self::Movement {
                destination,
                layer,
                sector,
                gate_id,
                line_id,
                element,
                flags,
                tolerance,
                direction,
                action,
                speed_factor,
                post_seek_sequence,
            } => SequenceElementData::Movement {
                destination,
                layer,
                sector,
                gate_id,
                line_id,
                element,
                flags,
                tolerance,
                direction,
                action,
                speed_factor,
                post_seek_sequence: map(post_seek_sequence)?,
            },
            Self::Generic { properties } => SequenceElementData::Generic { properties },
            Self::Damage {
                origin,
                projectile,
                damage,
                concussion,
                sword_strike,
                sword_profile_idx,
                is_harder_hit,
            } => SequenceElementData::Damage {
                origin,
                projectile,
                damage,
                concussion,
                sword_strike,
                sword_profile_idx,
                is_harder_hit,
            },
            Self::Interaction { antagonist } => SequenceElementData::Interaction { antagonist },
        })
    }
}

impl SequenceElementData {
    pub fn is_movement(&self) -> bool {
        matches!(self, Self::Movement { .. })
    }

    pub fn is_generic(&self) -> bool {
        matches!(self, Self::Generic { .. })
    }

    /// Create a new sword damage element.
    pub fn new_sword_damage(
        origin: EntityId,
        sword_strike: crate::weapons::SwordStrike,
        sword_profile_idx: u32,
    ) -> Self {
        Self::Damage {
            origin: Some(origin),
            projectile: None,
            damage: 0,
            concussion: 0,
            sword_strike: Some(sword_strike),
            sword_profile_idx: Some(sword_profile_idx),
            is_harder_hit: false,
        }
    }

    /// Create a new generic damage element (concussion + wounding).
    pub fn new_damage(origin: Option<EntityId>, damage: u16, concussion: u16) -> Self {
        Self::Damage {
            origin,
            projectile: None,
            damage,
            concussion,
            sword_strike: None,
            sword_profile_idx: None,
            is_harder_hit: false,
        }
    }

    /// Create a new generic element with an empty property map.
    pub fn new_generic() -> Self {
        Self::Generic {
            properties: HashMap::new(),
        }
    }

    /// Create a new movement element with default values.
    pub fn new_movement(action: OrderType) -> Self {
        Self::Movement {
            destination: crate::coordinates::MapPoint::default(),
            layer: 0,
            sector: None,
            gate_id: None,
            line_id: None,
            element: None,
            flags: MoveFlags::empty(),
            tolerance: 0.0,
            direction: 0,
            action,
            speed_factor: 1.0,
            post_seek_sequence: None,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  SequenceElement
// ═══════════════════════════════════════════════════════════════════

/// A single element in a sequence — one command to execute.
///
/// Subtype data lives in [`SequenceElementData`] enum variants instead
/// of a polymorphic hierarchy.
///
/// ## State machine
///
/// ```text
/// Todo ──→ InProgress ──→ Terminated
///  │            │              ↑
///  │            └──→ Postponed ┘
///  │
///  └──→ Interrupted
///  └──→ Impossible
/// ```
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SequenceElement<P: robin_util::state_hash::StateHash = Option<PostSeekSequence>> {
    /// Unique ID.
    pub id: u32,

    /// EngineInner command this element represents.
    pub command: Command,

    /// Command level for parallel/sequential grouping.
    /// Elements at the same level within a sequence run concurrently.
    pub command_level: u16,

    /// The entity that owns/executes this element. `None` means the engine handles it.
    pub owner: Option<EntityId>,

    /// Current state.
    pub state: SequenceState,

    /// Interruption priority.
    pub priority: SequencePriority,

    /// Suppress player-input selection side effects for actions authored by
    /// mission scripts or replayed quick-action sequences. This flag is
    /// behavioral, not save-only
    /// provenance, because bow equip/unequip reads it while executing.
    pub script_driven: bool,

    /// Fixed-point cutting/concussion multiplier resolved from a combat
    /// gesture. This is perfect for every original/script/AI element. It is
    /// stored on the sequence element so a save or rollback in the middle of
    /// an animation cannot lose the input result.
    pub gesture_quality: crate::player_command::GestureQuality,

    /// Posture the actor should have after transition orders complete.
    pub posture_after_transition: Posture,

    /// Action state after transition orders complete.
    pub action_state_after_transition: ActionState,

    /// Number of remaining launch-time transition orders at the front of the
    /// queue. `generate_transition` stamps this before command/path orders are
    /// appended; standard sequence-order teardown decrements it.
    pub num_transition_orders: usize,

    /// Goal cached from a selected movement while this replacement waits for
    /// pathfinding. Used to reproduce Original's select-new-before-condoling-
    /// old movement handoff.
    pub retained_movement_goal: Option<crate::coordinates::MapPoint>,

    /// Replay-only authoritative gate-search result retained until a point
    /// Seek reaches its cross-sector expansion boundary.
    #[serde(deserialize_with = "Option::deserialize")]
    pub recorded_gate_path: Option<crate::gate::RecordedGatePath>,

    /// Selects who owns gate-path resolution for this point Seek.
    ///
    /// Live commands leave this as [`PointSeekRouteProvenance::Live`] and may
    /// query Rust's gate graph when the Seek is finally instructed. Original
    /// parity replay marks DropAle seeks as `OriginalReplay`: if dispatch-time
    /// source/goal identity is cross-sector, a success or failure must already
    /// have crossed the frame boundary in `ExternalFacts`. This is the delayed
    /// the original game's cross-sector route search, not command-admission work.
    pub point_seek_route_provenance: PointSeekRouteProvenance,

    /// The sub-steps (movement waypoints, animation frames, etc.) for this element.
    pub orders: VecDeque<Order>,

    /// Subtype-specific data.
    pub data: SequenceElementData<P>,

    /// Index of a postponed element (within the same sequence) that should be
    /// restarted when this element finishes.
    ///
    /// Used for *intra-sequence* postponement (e.g. `PASS_DOOR` postponing a
    /// subsequent `MOVE` within the same launched sequence).
    pub postponed_element_index: Option<usize>,

    /// Cross-sequence postpone successor — the sequence element waiting
    /// for this one to terminate (lives on the *blocking* element and
    /// points at the *waiting* one).  When this element terminates or
    /// is interrupted, the successor is released (registered for
    /// dispatch or cascaded).
    ///
    /// The existing `postponed_element_index` handles the *intra-
    /// sequence* case (e.g. `PASS_DOOR` postponing a later `MOVE` in the
    /// same launched sequence).  `cross_postponed` handles the case
    /// where instruction arbitration postpones a new element launched
    /// via a *different* sequence (e.g. a user-click sword strike issued
    /// while another sword strike sequence is mid-walk).
    pub cross_postponed: Option<(SequenceId, usize)>,

    /// Stopping a sequence element clears its next-element link once the
    /// recursive stop leaves that successor `RHSEQ_INTERRUPTED`
    /// after the recursive stop. Runtime-authored elements derive
    /// their successor from append order rather than from a stored pointer,
    /// so record the severing explicitly. Loaded v48 elements clear
    /// `legacy_v48.next` instead, exactly as the Original save does.
    pub next_link_severed: bool,

    /// Original-only authoritative members retained during v48 adoption.
    ///
    /// TODO(legacy-sequence-runtime): route `next`, `mummy`, linked-seek,
    /// deleted/script-driven and arrow fields through the
    /// corresponding runtime paths. Keeping the exact values here prevents a
    /// successful load from silently discarding state while those behaviors
    /// are being implemented.
    pub(crate) legacy_v48: Option<LegacyV48SequenceElementState>,
}

impl<P: robin_util::state_hash::StateHash> SequenceElement<P> {
    fn try_map_post_seek<Q: robin_util::state_hash::StateHash, E>(
        self,
        map: impl FnOnce(P) -> Result<Q, E>,
    ) -> Result<SequenceElement<Q>, E> {
        let Self {
            id,
            command,
            command_level,
            owner,
            state,
            priority,
            script_driven,
            gesture_quality,
            posture_after_transition,
            action_state_after_transition,
            num_transition_orders,
            retained_movement_goal,
            recorded_gate_path,
            point_seek_route_provenance,
            orders,
            data,
            postponed_element_index,
            cross_postponed,
            next_link_severed,
            legacy_v48,
        } = self;
        Ok(SequenceElement {
            id,
            command,
            command_level,
            owner,
            state,
            priority,
            script_driven,
            gesture_quality,
            posture_after_transition,
            action_state_after_transition,
            num_transition_orders,
            retained_movement_goal,
            recorded_gate_path,
            point_seek_route_provenance,
            orders,
            data: data.try_map_post_seek(map)?,
            postponed_element_index,
            cross_postponed,
            next_link_severed,
            legacy_v48,
        })
    }
}

impl SequenceElement<()> {
    /// Generic property access for elements in a flat post-seek sequence.
    pub fn get_property(&self, field: Field) -> Option<&FieldValue> {
        match &self.data {
            SequenceElementData::Generic { properties } => properties.get(&field),
            _ => None,
        }
    }
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct LegacyV48OrderState {
    pub legacy_id: u32,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct LegacyV48SequenceElementState {
    pub deleted: bool,
    pub script_driven: bool,
    /// Exact constructor storage from an old v48 save when
    /// `posture_after_transition` was not yet semantically live.
    ///
    /// Original-game sequence-element initialization left both transition-result
    /// enums uninitialized. Actor instruction handling overwrites them before any live
    /// transition use, but queued and completed elements can serialize the
    /// dormant bytes. Valid enum values remain represented by the typed field
    /// above; this sidecar is only `Some` for a proven-dormant invalid word.
    pub raw_dormant_posture_after_transition: Option<i32>,
    /// Dormant invalid counterpart of `action_state_after_transition`; see
    /// [`Self::raw_dormant_posture_after_transition`].
    pub raw_dormant_action_state_after_transition: Option<i32>,
    pub next: Option<SequenceElementRef>,
    pub postponed: Option<SequenceElementRef>,
    pub mummy: Option<SequenceId>,
    /// `None` means this is not a movement element; `Some(None)` is a
    /// movement element with a serialized null linked-seek pointer.
    pub linked_seek: Option<Option<SequenceElementRef>>,
    pub damage_arrow: Option<EntityId>,
    pub raw_sword_strike: Option<i32>,
    /// Exact movement-action storage when the raw word is
    /// not a serialized animation and the original game cannot consume it in this command/
    /// state. Old constructors left this field uninitialized for position-only
    /// movement elements.
    pub raw_dormant_movement_action: Option<i32>,
    pub order_state: Vec<LegacyV48OrderState>,
    pub generic_raw_unions: Vec<(Field, [u8; 12])>,
}

impl SequenceElement {
    /// Create a new element with the given command level, command, and owner.
    /// `id` is a placeholder — `SequenceManager::launch_sequence` stamps the
    /// real per-engine deterministic id on every element at launch time.
    ///
    /// Elements/sequences are dropped directly via `BTreeMap::retain` in
    /// `friday_evening_cleanup`, so there is no window where a "deleted"
    /// element is still pointed at by live references — no separate
    /// "deleted" flag is needed.
    pub fn new(command_level: u16, command: Command, owner: Option<EntityId>) -> Self {
        Self {
            id: 0,
            command,
            command_level,
            owner,
            state: SequenceState::Todo,
            priority: SequencePriority::NotYetSet,
            script_driven: false,
            gesture_quality: crate::player_command::GestureQuality::PERFECT,
            posture_after_transition: Posture::default(),
            action_state_after_transition: ActionState::default(),
            num_transition_orders: 0,
            retained_movement_goal: None,
            recorded_gate_path: None,
            point_seek_route_provenance: PointSeekRouteProvenance::Live,
            orders: VecDeque::new(),
            data: SequenceElementData::Simple,
            postponed_element_index: None,
            cross_postponed: None,
            next_link_severed: false,
            legacy_v48: None,
        }
    }

    /// Create a new movement element.
    pub fn new_movement(
        command_level: u16,
        command: Command,
        owner: Option<EntityId>,
        action: OrderType,
    ) -> Self {
        let mut elem = Self::new(command_level, command, owner);
        elem.data = SequenceElementData::new_movement(action);
        elem
    }

    /// Create a new generic element.
    pub fn new_generic(command_level: u16, command: Command, owner: Option<EntityId>) -> Self {
        let mut elem = Self::new(command_level, command, owner);
        elem.data = SequenceElementData::new_generic();
        elem
    }

    /// Create a payload-bearing message command in the legacy storage shape.
    ///
    /// The field bag remains the serialized representation during the staged
    /// migration, but callers supply one typed payload and
    /// [`Self::sequence_command`] performs checked conversion when it is read.
    ///
    /// The original game records all three fields, including
    /// explicit zero arguments for the no-arguments native.
    pub fn new_send_message(
        command_level: u16,
        owner: Option<EntityId>,
        payload: SendMessageCommand,
    ) -> Self {
        let mut element = Self::new_generic(command_level, Command::SendMessage, owner);
        element.set_property(Field::Message, FieldValue::Integer(payload.message as u32));
        element.set_property(
            Field::MessageArgument,
            FieldValue::Integer(payload.argument as u32),
        );
        element.set_property(
            Field::MessageExtendedArgument,
            FieldValue::Integer(payload.extended_argument as u32),
        );
        element
    }

    /// Create a new generic-damage element (concussion + wounding).
    pub fn new_damage(
        command_level: u16,
        command: Command,
        owner: Option<EntityId>,
        origin: Option<EntityId>,
        damage: u16,
        concussion: u16,
    ) -> Self {
        let mut elem = Self::new(command_level, command, owner);
        elem.data = SequenceElementData::new_damage(origin, damage, concussion);
        elem
    }

    /// Create a new interaction element.
    pub fn new_interaction(
        command_level: u16,
        command: Command,
        owner: Option<EntityId>,
        antagonist: Option<EntityId>,
    ) -> Self {
        let mut elem = Self::new(command_level, command, owner);
        elem.data = SequenceElementData::Interaction { antagonist };
        elem
    }

    /// Set a property on a generic element. Panics if not generic.
    ///
    /// "First set wins": duplicate sets are rejected via a debug
    /// assertion so any future call site that needs to mutate an
    /// existing entry is forced to use [`Self::update_property`] instead
    /// of silently relying on `HashMap::insert`'s replace semantics.
    pub fn set_property(&mut self, field: Field, value: FieldValue) {
        match &mut self.data {
            SequenceElementData::Generic { properties } => {
                debug_assert!(
                    !properties.contains_key(&field),
                    "set_property: field {:?} already present — use update_property to mutate",
                    field
                );
                properties.insert(field, value);
            }
            _ => panic!("set_property called on non-generic element"),
        }
    }

    /// Drop a property from a generic element, if present.
    ///
    /// Only engine caches are ever removed — [`Field::RetainedMovementGoal`]
    /// is the sole such field. Authored command properties are written once
    /// and read for the element's whole life.
    pub fn remove_property(&mut self, field: Field) {
        if let SequenceElementData::Generic { properties } = &mut self.data {
            properties.remove(&field);
        }
    }

    /// Get a property from a generic element. Returns `None` if not found or not generic.
    pub fn get_property(&self, field: Field) -> Option<&FieldValue> {
        match &self.data {
            SequenceElementData::Generic { properties } => properties.get(&field),
            _ => None,
        }
    }

    /// Convert this element's legacy command + subtype data into the typed
    /// command representation.
    ///
    /// Message conversion is intentionally strict: original-game initialization
    /// always write all three integer fields, so absence or a mismatched field
    /// type is corrupt state, not a request for a zero default.
    pub fn sequence_command(&self) -> Result<SequenceCommand, SequenceInvariantError> {
        SequenceCommand::try_from(self)
    }

    /// Set the speed factor on a movement element. Panics if not a movement element.
    pub fn set_speed_factor(&mut self, factor: f32) {
        match &mut self.data {
            SequenceElementData::Movement { speed_factor, .. } => *speed_factor = factor,
            _ => panic!("set_speed_factor called on non-movement element"),
        }
    }

    /// Get the speed factor. Returns 1.0 for non-movement elements.
    pub fn speed_factor(&self) -> f32 {
        match &self.data {
            SequenceElementData::Movement { speed_factor, .. } => *speed_factor,
            _ => 1.0,
        }
    }

    /// Get the current order (first in the queue).
    pub fn current_order(&self) -> Option<&Order> {
        self.orders.front()
    }

    /// Get the next order (second in the queue).
    pub fn next_order(&self) -> Option<&Order> {
        self.orders.get(1)
    }

    /// Add an order at the back of the queue.
    ///
    /// Panics on an invalid action. Use [`Self::try_push_order`] at an input
    /// boundary that needs to report corrupt data without panicking.
    pub fn push_order(&mut self, order: Order) {
        self.try_push_order(order)
            .unwrap_or_else(|error| panic!("push_order: {error}"));
    }

    /// Checked form of [`Self::push_order`].
    pub fn try_push_order(&mut self, order: Order) -> Result<(), SequenceInvariantError> {
        if order.order_type == OrderType::Invalid {
            return Err(SequenceInvariantError::InvalidOrderAction);
        }
        self.orders.push_back(order);
        Ok(())
    }

    /// Insert an order at a specific index.
    ///
    /// Panics for invalid actions or out-of-range indices. Use
    /// [`Self::try_insert_order`] at an input boundary that needs to report the
    /// invariant error.
    pub fn insert_order(&mut self, index: usize, order: Order) {
        self.try_insert_order(index, order)
            .unwrap_or_else(|error| panic!("insert_order: {error}"));
    }

    /// Checked form of [`Self::insert_order`].
    pub fn try_insert_order(
        &mut self,
        index: usize,
        order: Order,
    ) -> Result<(), SequenceInvariantError> {
        if order.order_type == OrderType::Invalid {
            return Err(SequenceInvariantError::InvalidOrderAction);
        }
        if index > self.orders.len() {
            return Err(SequenceInvariantError::OrderInsertionOutOfBounds {
                index,
                len: self.orders.len(),
            });
        }
        // VecDeque doesn't have insert, so we convert
        let mut temp: Vec<Order> = self.orders.drain(..).collect();
        temp.insert(index, order);
        self.orders = temp.into();
        Ok(())
    }

    /// Remove and return the first order, advancing to the next.
    /// Returns the new current order, or `None` if the list is now empty.
    pub fn proceed(&mut self) -> Option<&Order> {
        self.pop_current_order()?;
        self.orders.front()
    }

    /// Remove the active order and maintain the remaining leading-transition
    /// span. Rust pathfinding may complete after one or more launch transitions
    /// have already played, so this count describes the current queue rather
    /// than the queue originally stamped by `generate_transition`.
    pub fn pop_current_order(&mut self) -> Option<Order> {
        let popped = self.orders.pop_front()?;
        self.num_transition_orders = self.num_transition_orders.saturating_sub(1);
        Some(popped)
    }

    /// Mark all currently queued orders as launch-time transitions.
    pub fn initialize_transition_orders(&mut self) {
        self.num_transition_orders = self.orders.len();
    }

    /// Set the movement action on this element. For non-movement
    /// elements this is a no-op. Callers that want to propagate through
    /// the linked chain should use [`SequenceManager::set_action_recursive`].
    pub fn set_action(&mut self, new_action: OrderType) {
        if let SequenceElementData::Movement { action, .. } = &mut self.data {
            *action = new_action;
        }
    }

    #[cfg(test)]
    pub(crate) fn movement_flags_for_test(&self) -> Option<MoveFlags> {
        match &self.data {
            SequenceElementData::Movement { flags, .. } => Some(*flags),
            _ => None,
        }
    }

    /// Insert a posture/action-state-transition order (with movement)
    /// at the front of this movement element's order list. Any prefix
    /// of orders whose action matches `animation_to_replace` is eaten
    /// to make room for `distance_transition` worth of heading; the
    /// leftover of the partially-consumed order becomes a new order
    /// carrying `animation_transition`.
    ///
    /// The starting map position (`point_start`) is used as the
    /// destination of the inserted order before being walked forward
    /// along the consumed orders' headings.
    pub fn insert_transition_start(
        &mut self,
        animation_transition: OrderType,
        animation_to_replace: OrderType,
        distance_transition: f32,
        point_start: crate::coordinates::MapPoint,
        next_order_id: &mut u32,
    ) -> bool {
        let mut distance_remaining = if distance_transition == 0.0 {
            0.01
        } else {
            distance_transition
        };
        let mut inserted = false;

        let mut point = point_start;
        let mut order_idx = 0usize;
        while order_idx < self.orders.len() {
            let order_action = self.orders[order_idx].order_type;
            if order_action == animation_to_replace {
                let dest_x = self.orders[order_idx].target_x;
                let dest_y = self.orders[order_idx].target_y;
                let vx = dest_x - point.x;
                let vy = dest_y - point.y;
                let norm = (vx * vx + vy * vy).sqrt();
                if norm >= distance_remaining {
                    // Build the inserted order with its destination
                    // `distance_remaining` along the heading.
                    let (insert_x, insert_y) = if norm != 0.0 {
                        let scale = distance_remaining / norm;
                        (point.x + vx * scale, point.y + vy * scale)
                    } else {
                        (point.x, point.y)
                    };
                    let mut new_order = crate::order::Order::new(
                        animation_transition,
                        insert_x,
                        insert_y,
                        crate::order::alloc_order_id(next_order_id),
                    );
                    new_order.compute_direction = true;
                    self.insert_order(order_idx, new_order);
                    return true;
                } else {
                    // Not enough room: consume the whole order,
                    // relabel it, and keep searching.
                    distance_remaining -= norm;
                    self.orders[order_idx].order_type = animation_transition;
                    inserted = true;
                }
            }

            // If this order carries a real destination, advance the
            // running point so later iterations measure distance from
            // the correct heading origin.
            let dx = self.orders[order_idx].target_x;
            let dy = self.orders[order_idx].target_y;
            if !(dx == 0.0 && dy == 0.0) {
                point = crate::coordinates::MapPoint { x: dx, y: dy };
            }
            order_idx += 1;
        }
        inserted
    }

    /// Insert a transition order at the *end* of this movement
    /// element's order list. Walks backward through the order list
    /// looking for an order whose action is `animation_to_replace`;
    /// when found, relabels it to `animation_transition` and inserts a
    /// new `animation_to_replace` order in front of it, shifted back
    /// along the heading by `distance_transition + element tolerance`.
    ///
    /// The `aspect_ratio` parameter controls the directional-tolerance
    /// vector norm (used when `MoveFlags::DIRECTIONAL_TOLERANCE` is
    /// set).
    pub fn insert_transition_end(
        &mut self,
        animation_transition: OrderType,
        animation_to_replace: OrderType,
        distance_transition: f32,
        point_start: crate::coordinates::MapPoint,
        aspect_ratio: f32,
        next_order_id: &mut u32,
    ) {
        if self.orders.is_empty() {
            return;
        }
        let (directional_tolerance, tolerance, flags, antagonist) = match &self.data {
            SequenceElementData::Movement {
                flags,
                tolerance,
                element,
                ..
            } => (
                flags.contains(MoveFlags::DIRECTIONAL_TOLERANCE),
                *tolerance,
                *flags,
                *element,
            ),
            _ => {
                debug_assert!(
                    false,
                    "insert_transition_end called on non-movement element"
                );
                return;
            }
        };

        let mut distance_remaining = if distance_transition == 0.0 {
            0.01
        } else {
            distance_transition
        };
        distance_remaining += tolerance;

        let norm = |vx: f32, vy: f32| -> f32 {
            if directional_tolerance && aspect_ratio != 1.0 {
                // Aspect-ratio norm divides the Y component by the
                // aspect ratio before computing the hypotenuse:
                // `sqrt(mX² + (mY/aspect_ratio)²)`.  With
                // `ASPECT_RATIO ≈ 0.5736`, this stretches the Y axis
                // ~1.7434×, biasing the gap measurement toward giving
                // Y-direction motion more room.
                let sy = vy / aspect_ratio;
                (vx * vx + sy * sy).sqrt()
            } else {
                (vx * vx + vy * vy).sqrt()
            }
        };

        let len = self.orders.len();
        for i in (0..len).rev() {
            if self.orders[i].order_type != animation_to_replace {
                continue;
            }
            // Relabel this order to the transition.
            self.orders[i].order_type = animation_transition;
            // End-transition insertion relabels
            // the first order in place and deliberately leaves its existing
            // tolerance untouched. Only
            // a newly inserted second order starts with zero tolerance.
            let point_x = self.orders[i].target_x;
            let point_y = self.orders[i].target_y;

            // Walk backward to find an order carrying a location.
            // `break_after_insufficient` distinguishes "no prior order
            // had a point, fall through to start-point" from "prior
            // order had a point but not enough room, continue outer
            // loop to next candidate".
            let mut break_after_insufficient = false;
            for j in (0..i).rev() {
                let dx = self.orders[j].target_x;
                let dy = self.orders[j].target_y;
                if dx == 0.0 && dy == 0.0 {
                    continue;
                }
                let vx = dx - point_x;
                let vy = dy - point_y;
                let d = norm(vx, vy);
                if d * 1.01 >= distance_remaining {
                    let (ix, iy) = if d != 0.0 {
                        let s = distance_remaining / d;
                        (point_x + vx * s, point_y + vy * s)
                    } else {
                        (point_x, point_y)
                    };
                    let mut new_order = crate::order::Order::new(
                        animation_to_replace,
                        ix,
                        iy,
                        crate::order::alloc_order_id(next_order_id),
                    );
                    new_order.compute_direction = true;
                    new_order.tolerance = 0.0;
                    // The spliced movement order inherits the element's target
                    // element, not just the relabelled transition it precedes.
                    // Once it is the live order the target's radius widens the
                    // blocked-count arrival slack, so dropping it here strands
                    // the walk short of its waypoint for extra frames.
                    if (!flags.contains(MoveFlags::SEEK) || !flags.contains(MoveFlags::USE_POINT))
                        && let Some(a) = antagonist
                    {
                        new_order.target_actor = Some(a.index());
                        new_order.antagonist = Some(a);
                    }
                    self.insert_order(i, new_order);
                    return;
                } else {
                    distance_remaining -= d;
                    break_after_insufficient = true;
                    break;
                }
            }

            if !break_after_insufficient {
                // Fall through to start-point.
                let vx = point_start.x - point_x;
                let vy = point_start.y - point_y;
                let d = norm(vx, vy);
                if d >= distance_remaining {
                    let (ix, iy) = if d != 0.0 {
                        let s = distance_remaining / d;
                        (point_x + vx * s, point_y + vy * s)
                    } else {
                        (point_x, point_y)
                    };
                    let mut new_order = crate::order::Order::new(
                        animation_to_replace,
                        ix,
                        iy,
                        crate::order::alloc_order_id(next_order_id),
                    );
                    new_order.compute_direction = true;
                    new_order.tolerance = 0.0;
                    if (!flags.contains(MoveFlags::SEEK) || !flags.contains(MoveFlags::USE_POINT))
                        && let Some(a) = antagonist
                    {
                        new_order.target_actor = Some(a.index());
                        new_order.antagonist = Some(a);
                    }
                    self.insert_order(i, new_order);
                }
                return;
            }
        }
    }

    /// Clean up consecutive duplicate orders (same action + same
    /// destination).
    pub fn cleanup_duplicate_orders(&mut self) {
        if self.orders.len() <= 1 {
            return;
        }
        let mut i = 1;
        while i < self.orders.len() {
            let prev = &self.orders[i - 1];
            let cur = &self.orders[i];
            if prev.order_type == cur.order_type
                && prev.target_x == cur.target_x
                && prev.target_y == cur.target_y
            {
                self.orders.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Whether this command is executed immediately (synchronously) rather
    /// than being deferred to the hourglass queue.
    pub fn executed_immediately(&self) -> bool {
        let command = self
            .sequence_command()
            .unwrap_or_else(|error| panic!("executed_immediately: {error}"));
        match command {
            SequenceCommand::SendMessage(_) => true,
            SequenceCommand::Legacy(command) => matches!(
                command,
                // Commands dispatched to owner immediately
                Command::Teleport
                | Command::LockAi
                | Command::UnlockAi
                | Command::ReplaceAnim
                | Command::RestoreAnim
                | Command::Speak
                | Command::StartMobile
                | Command::StopMobile
                | Command::ActivateMobile
                | Command::DeactivateMobile
                | Command::Unblip
                // Commands dispatched to engine immediately
                | Command::LockUser
                | Command::UnlockUser
                | Command::CameraJumpTo
                | Command::Timer
                | Command::ActionAvailable
                | Command::CharacterAvailable
                    | Command::OpenScroll
            ),
        }
    }
}

/// Authority for a point Seek's delayed gate-search result.
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
pub enum PointSeekRouteProvenance {
    /// Live Rust gameplay may resolve the route from the current gate graph.
    #[default]
    Live,
    /// Original replay owns the exact success/failure outcome.
    OriginalReplay,
}

impl TryFrom<&SequenceElement> for SequenceCommand {
    type Error = SequenceInvariantError;

    fn try_from(element: &SequenceElement) -> Result<Self, Self::Error> {
        if element.command != Command::SendMessage {
            return Ok(Self::Legacy(element.command));
        }

        let SequenceElementData::Generic { properties } = &element.data else {
            return Err(SequenceInvariantError::LegacyCommandRequiresGenericData {
                command: element.command,
            });
        };

        let integer = |field| match properties.get(&field) {
            Some(FieldValue::Integer(value)) => Ok(*value as i32),
            Some(_) => Err(SequenceInvariantError::InvalidLegacyCommandFieldType {
                command: element.command,
                field,
            }),
            None => Err(SequenceInvariantError::MissingLegacyCommandField {
                command: element.command,
                field,
            }),
        };

        Ok(Self::SendMessage(SendMessageCommand::new(
            integer(Field::Message)?,
            integer(Field::MessageArgument)?,
            integer(Field::MessageExtendedArgument)?,
        )))
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Sequence
// ═══════════════════════════════════════════════════════════════════

/// A sequence of commands grouped by command level.
///
/// Elements at the same command level execute in parallel. When all
/// elements at a level finish, the next level starts.
///
/// ## Command level example
///
/// ```text
/// Level 1: [Move to door] [Wait timer]    ← these run in parallel
/// Level 2: [Pass door]                     ← waits for level 1 to finish
/// Level 3: [Move to goal]                  ← waits for level 2 to finish
/// ```
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct Sequence<P: robin_util::state_hash::StateHash = Option<PostSeekSequence>> {
    /// Unique ID.
    pub id: SequenceId,

    /// All elements in this sequence, ordered by command level.
    pub elements: Vec<SequenceElement<P>>,

    /// Index of the next element to start.
    cursor: usize,

    /// The command level currently being executed.
    current_command_level: u16,

    /// Number of elements from the current level still running.
    running_elements: u16,

    /// Number of elements currently in InProgress state.
    elements_in_progress: u16,

    /// Whether `launch()` has been called.
    started: bool,
}

/// A continuation attached to a root movement element.
///
/// Its elements instantiate the post-seek slot with `()`, so they cannot
/// attach another continuation. This matches every live construction path in
/// the Original while keeping native bitcode's coder graph finite.
pub type PostSeekSequence = Sequence<()>;

impl Sequence {
    /// Convert a newly built or legacy-decoded sequence into a one-level
    /// continuation. Nested continuations are rejected explicitly.
    pub fn try_into_post_seek(self) -> Result<PostSeekSequence, SequenceInvariantError> {
        let Self {
            id,
            elements,
            cursor,
            current_command_level,
            running_elements,
            elements_in_progress,
            started,
        } = self;
        let elements = elements
            .into_iter()
            .map(|element| {
                element.try_map_post_seek(|post_seek| {
                    if post_seek.is_some() {
                        Err(SequenceInvariantError::NestedPostSeekSequence)
                    } else {
                        Ok(())
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PostSeekSequence {
            id,
            elements,
            cursor,
            current_command_level,
            running_elements,
            elements_in_progress,
            started,
        })
    }

    /// Infallible convenience for gameplay-authored continuations. A nested
    /// continuation here is a construction bug, not recoverable input.
    pub fn into_post_seek(self) -> PostSeekSequence {
        self.try_into_post_seek()
            .unwrap_or_else(|error| panic!("invalid gameplay post-seek sequence: {error}"))
    }
}

impl PostSeekSequence {
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&SequenceElement<()>> {
        self.elements.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut SequenceElement<()>> {
        self.elements.get_mut(index)
    }

    pub fn last(&self) -> Option<&SequenceElement<()>> {
        self.elements.last()
    }

    /// Promote a detached continuation to an ordinary sequence immediately
    /// before it is launched.
    pub fn into_sequence(self) -> Sequence {
        let Self {
            id,
            elements,
            cursor,
            current_command_level,
            running_elements,
            elements_in_progress,
            started,
        } = self;
        let elements = elements
            .into_iter()
            .map(|element| {
                element
                    .try_map_post_seek(|()| {
                        Ok::<_, std::convert::Infallible>(None::<PostSeekSequence>)
                    })
                    .expect("infallible post-seek promotion")
            })
            .collect();
        Sequence {
            id,
            elements,
            cursor,
            current_command_level,
            running_elements,
            elements_in_progress,
            started,
        }
    }
}

impl Sequence {
    /// Stable-boundary counters used by the Original parity recorder.
    #[doc(hidden)]
    pub(crate) fn parity_counters(&self) -> (usize, u16, u16, u16, bool) {
        (
            self.cursor,
            self.current_command_level,
            self.running_elements,
            self.elements_in_progress,
            self.started,
        )
    }
    /// Create a new empty sequence. `id` is a placeholder —
    /// `SequenceManager::launch_sequence` stamps the real per-engine
    /// deterministic id at launch time.
    pub fn new() -> Self {
        Self {
            id: SequenceId(0),
            elements: Vec::new(),
            cursor: 0,
            current_command_level: 0,
            running_elements: 0,
            elements_in_progress: 0,
            started: false,
        }
    }

    /// Construct one fully preflighted Original v48 sequence without running
    /// launch-time state transitions or allocating new identities.
    pub(crate) fn restore_v48_state(
        id: SequenceId,
        elements: Vec<SequenceElement>,
        cursor: usize,
        current_command_level: u16,
        running_elements: u16,
        elements_in_progress: u16,
        started: bool,
    ) -> Self {
        Self {
            id,
            elements,
            cursor,
            current_command_level,
            running_elements,
            elements_in_progress,
            started,
        }
    }

    /// Build a single-element `ReceiveDamage` sequence.
    ///
    /// Used by every cheat damage path (`NUKE`, `COMA`, `SANPETRUS`,
    /// `MISTERSANDMAN`) and by `InflictPain`.
    pub fn single_damage(actor: EntityId, hp: u16, concussion: u16) -> Self {
        let mut seq = Self::new();
        seq.append_element(SequenceElement::new_damage(
            1,
            Command::ReceiveDamage,
            Some(actor),
            None,
            hp,
            concussion,
        ));
        seq
    }

    /// Append a sequence element, panicking if its command level is not
    /// contiguous. Use [`Self::try_append_element`] when importing untrusted
    /// legacy data.
    pub fn append_element(&mut self, element: SequenceElement) {
        self.try_append_element(element)
            .unwrap_or_else(|error| panic!("append_element: {error}"));
    }

    /// Checked form of [`Self::append_element`].
    pub fn try_append_element(
        &mut self,
        element: SequenceElement,
    ) -> Result<(), SequenceInvariantError> {
        if let Some(last) = self.elements.last() {
            let level_is_contiguous = element.command_level == last.command_level
                || last.command_level.checked_add(1) == Some(element.command_level);
            if !level_is_contiguous {
                return Err(SequenceInvariantError::NonContiguousCommandLevel {
                    previous: last.command_level,
                    next: element.command_level,
                });
            }
        }
        self.elements.push(element);
        Ok(())
    }

    /// Number of elements.
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Whether the sequence has no elements.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Get an element by index.
    pub fn get(&self, index: usize) -> Option<&SequenceElement> {
        self.elements.get(index)
    }

    /// Get a mutable element by index.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut SequenceElement> {
        self.elements.get_mut(index)
    }

    /// Get the last element.
    pub fn last(&self) -> Option<&SequenceElement> {
        self.elements.last()
    }

    /// Start the sequence. Returns false if already at the end.
    pub fn launch(&mut self) -> bool {
        if !self.started {
            self.started = true;
        } else {
            // re-launching is a bug
            debug_assert!(false, "sequence launched twice");
        }

        if self.cursor >= self.elements.len() {
            return false;
        }
        // `next_elements_go` is called by the manager after launch
        true
    }

    /// Advance the cursor past all elements at the current command level,
    /// collecting element indices that need to be started.
    ///
    /// Returns a list of element indices that should be dispatched.
    ///
    /// The interrupted-state guard and wait-priority test are
    /// re-read per iteration by the caller, not snapshotted here — an earlier
    /// sibling's inline execution can change both.
    pub fn next_elements_go(&mut self) -> Vec<usize> {
        debug_assert_eq!(self.running_elements, 0);

        let list_size = self.elements.len();
        if self.cursor >= list_size {
            return Vec::new();
        }

        // Get the command level at the cursor
        self.current_command_level = self.elements[self.cursor].command_level;

        let start_index = self.cursor;

        // Advance cursor past all elements at this command level
        while self.cursor < list_size
            && self.elements[self.cursor].command_level == self.current_command_level
        {
            self.cursor += 1;
            self.running_elements += 1;
        }

        // The next element (if any) must have command_level == current + 1
        debug_assert!(
            self.cursor >= list_size
                || self.elements[self.cursor].command_level == self.current_command_level + 1
        );

        let end_index = self.cursor;

        (start_index..end_index).collect()
    }

    /// Called when an element at the current level finishes.
    /// When all elements at the current level are done, returns `true`
    /// to signal that the next level should be started.
    pub fn element_ready(&mut self) -> bool {
        assert!(
            self.running_elements > 0,
            "Ready called with no running elements"
        );
        self.running_elements -= 1;
        self.running_elements == 0
    }

    /// Increment the in-progress counter.
    pub fn increase_elements_in_progress(&mut self) {
        self.elements_in_progress += 1;
    }

    /// Decrement the in-progress counter.
    pub fn decrease_elements_in_progress(&mut self) {
        assert!(
            self.elements_in_progress > 0,
            "decrease_elements_in_progress underflow"
        );
        self.elements_in_progress -= 1;
    }

    /// Whether this sequence should be cleaned up.
    pub fn is_to_be_deleted(&self) -> bool {
        if self.elements.is_empty() {
            debug_assert!(false, "empty sequence in manager");
            return true;
        }

        // If any elements are still in progress, keep it alive
        if self.elements_in_progress > 0 {
            return false;
        }

        // Check if any elements are still pending
        for elem in self.elements.iter().rev() {
            match elem.state {
                SequenceState::InProgress => {
                    debug_assert!(false, "InProgress element but elements_in_progress == 0");
                    return false;
                }
                SequenceState::Todo | SequenceState::Postponed => {
                    return false;
                }
                _ => {}
            }
        }

        true
    }

    /// Check if an entity owns any active element in this sequence.
    pub fn has_owner(&self, entity: EntityId) -> bool {
        self.elements.iter().any(|elem| {
            matches!(elem.state, SequenceState::Todo | SequenceState::InProgress)
                && elem.owner == Some(entity)
        })
    }
}

impl Default for Sequence {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════
//  State change logic
// ═══════════════════════════════════════════════════════════════════

/// Process-local provenance captured at the terminal state-change boundary for
/// the opt-in movement-goal ownership diagnostic. This deliberately lives
/// outside serialized manager state and is consumed by the engine when it
/// dispatches the corresponding condolence card.
#[derive(Debug, Clone)]
pub(crate) struct GoalOwnerTerminalProvenance {
    pub site: &'static str,
    pub selected: Option<(SequenceId, usize)>,
    pub translating: Option<(EntityId, SequenceElementRef)>,
}

thread_local! {
    static GOAL_OWNER_TERMINAL_PROVENANCE:
        std::cell::RefCell<BTreeMap<(SequenceId, u16), GoalOwnerTerminalProvenance>> =
        const { std::cell::RefCell::new(BTreeMap::new()) };
}

fn goal_owner_debug_matches(owner: EntityId) -> bool {
    static FILTER: std::sync::OnceLock<Option<(crate::entity_id::EntityIdKind, u32)>> =
        std::sync::OnceLock::new();
    let Some((kind, index)) = FILTER.get_or_init(|| {
        std::env::var_os("PARITY_DEBUG_GOAL_OWNER_HANDOFF")?;
        let filter = std::env::var("PARITY_DEBUG_GOAL_OWNER").unwrap_or_else(|_| {
            panic!("PARITY_DEBUG_GOAL_OWNER_HANDOFF requires PARITY_DEBUG_GOAL_OWNER=pc|soldier|civilian:INDEX")
        });
        let (kind, index) = filter.split_once(':').unwrap_or_else(|| {
            panic!("PARITY_DEBUG_GOAL_OWNER must look like pc|soldier|civilian:INDEX")
        });
        let kind = match kind {
            "pc" => crate::entity_id::EntityIdKind::Pc,
            "soldier" => crate::entity_id::EntityIdKind::Soldier,
            "civilian" => crate::entity_id::EntityIdKind::Civilian,
            _ => panic!("PARITY_DEBUG_GOAL_OWNER has unsupported kind {kind:?}"),
        };
        let index = index.parse::<u32>().unwrap_or_else(|error| {
            panic!("invalid PARITY_DEBUG_GOAL_OWNER={filter:?}: {error}")
        });
        Some((kind, index))
    }) else {
        return false;
    };
    owner.kind() == *kind && owner.index() == *index
}

pub(crate) fn take_goal_owner_terminal_provenance(
    owner: EntityId,
    seq_id: SequenceId,
    elem_idx: u16,
) -> Option<GoalOwnerTerminalProvenance> {
    if !goal_owner_debug_matches(owner) {
        return None;
    }
    GOAL_OWNER_TERMINAL_PROVENANCE.with(|records| records.borrow_mut().remove(&(seq_id, elem_idx)))
}

/// Result of a state change on a sequence element.
/// The caller (SequenceManager) must process these effects.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct StateChangeEffects {
    /// Loaded movement element's exact `mpsqeLinkedSeekSequenceElement`.
    /// The original game interrupts this target with cascading successors before the
    /// movement element's base-class Interrupted handling.
    pub interrupt_linked_seek: Option<SequenceElementRef>,
    /// Elements whose state should also be changed (cascade).
    pub cascade: Vec<(usize, SequenceState, CascadeFlags)>,
    /// Whether `Sequence::element_ready()` should be called.
    pub signal_ready: bool,
    /// Whether to start a postponed element.
    /// `(blocker_index, postponed_index)` released by Original's
    /// postponed-element startup. Retaining the blocker lets the
    /// manager clear its pointer at the exact restart boundary, after the
    /// registration call (and, for Terminated, after Condolation + Ready).
    pub start_postponed: Option<(usize, usize)>,
    /// Cross-sequence postponed successor to resume.  Set when an
    /// element with a non-empty `cross_postponed` link terminates or is
    /// interrupted — the sequence manager takes this (seq_id, elem_idx)
    /// pair and registers it back on the `elements_to_go` queue.
    pub resume_cross_postponed: Option<(SequenceId, usize)>,
    /// Cross-sequence postponed link whose installation belongs after this
    /// element's synchronous removal-notification callback. Actor priority
    /// arbitration can interrupt an existing postponed successor while an
    /// incoming element is still being instructed; the original game does not expose
    /// that incoming element through the blocker's postponed pointer until
    /// the interrupted successor's callback returns.
    pub install_cross_postponed_after_card: Option<(SequenceId, usize, SequenceId, usize)>,
    /// Owner entity to notify when the element is removed.
    pub notify_owner: Option<EntityId>,
    /// Full condolation record (owner + command + terminal state) —
    /// used by `SequenceManager::process_effects` to populate the
    /// engine-drained `pending_condolations` queue.
    pub condolation: Option<PendingCondolation>,
    /// Whether elements_in_progress should be incremented.
    pub increment_in_progress: bool,
    /// Whether elements_in_progress should be decremented.
    pub decrement_in_progress: bool,
    /// Which element transitioned *into* `InProgress` plus its owner,
    /// if any.  Used by `SequenceManager::process_effects` to maintain
    /// `actor_in_progress`. Carried explicitly because some call paths
    /// (e.g. `stop_element` recursion) mutate a different element than
    /// the caller passed in.
    pub entered_in_progress: Option<(usize, EntityId)>,
    /// Mirror of `entered_in_progress` for `InProgress → *` exits.
    pub left_in_progress: Option<(usize, EntityId)>,
    /// Element state transition for the actor-live index.  Live here
    /// means Todo / InProgress / Postponed: any element that should
    /// prevent the engine from synthesizing an idle Wait for the owner.
    pub actor_live_transition: Option<(usize, EntityId, SequenceState, SequenceState)>,
}

impl Sequence {
    /// Change the state of element at `elem_idx`, returning effects that
    /// the caller must process. This is the core state machine.
    pub fn set_element_state(
        &mut self,
        elem_idx: usize,
        new_state: SequenceState,
        flags: CascadeFlags,
    ) -> StateChangeEffects {
        let mut effects = StateChangeEffects {
            interrupt_linked_seek: None,
            cascade: Vec::new(),
            signal_ready: false,
            start_postponed: None,
            resume_cross_postponed: None,
            install_cross_postponed_after_card: None,
            notify_owner: None,
            condolation: None,
            increment_in_progress: false,
            decrement_in_progress: false,
            entered_in_progress: None,
            left_in_progress: None,
            actor_live_transition: None,
        };

        let old_state = self.elements[elem_idx].state;
        if old_state == new_state {
            return effects;
        }

        // The most important line: actually change the state
        self.elements[elem_idx].state = new_state;

        if let Some(owner) = self.elements[elem_idx].owner {
            effects.actor_live_transition = Some((elem_idx, owner, old_state, new_state));
        }

        // Track in-progress count and — for `SequenceManager`'s actor
        // → refs map — which specific element's state changed plus its
        // owner.  (The `elem_idx` passed in here is what actually moved;
        // outer callers can have a different "driving" elem_idx when
        // the cascade lands on a sibling.)
        if new_state == SequenceState::InProgress {
            effects.increment_in_progress = true;
            effects.entered_in_progress = self.elements[elem_idx].owner.map(|o| (elem_idx, o));
        } else if old_state == SequenceState::InProgress {
            effects.decrement_in_progress = true;
            effects.left_in_progress = self.elements[elem_idx].owner.map(|o| (elem_idx, o));
        }

        match new_state {
            SequenceState::InProgress => {
                debug_assert!(
                    old_state == SequenceState::Todo || old_state == SequenceState::Postponed,
                    "InProgress from {:?}",
                    old_state
                );
            }

            SequenceState::Impossible => {
                // Start postponed element if any
                if let Some(postponed_idx) = self.elements[elem_idx].postponed_element_index {
                    effects.start_postponed = Some((elem_idx, postponed_idx));
                }
                // Release cross-sequence postponed successor, if any.
                if let Some(cross) = self.elements[elem_idx].cross_postponed.take() {
                    effects.resume_cross_postponed = Some(cross);
                }
                // Clear orders
                self.elements[elem_idx].orders.clear();
                // Notify owner
                effects.notify_owner = self.elements[elem_idx].owner;
                if let Some(owner) = self.elements[elem_idx].owner {
                    effects.condolation = Some(PendingCondolation {
                        owner,
                        command: self.elements[elem_idx].command,
                        terminal_state: new_state,
                        seq_id: self.id,
                        elem_idx: elem_idx as u16,
                        was_selected: false,
                        from_halt: false,
                        postponed_successor_pending: false,
                        cancel_path_request_owner: None,
                    });
                }
                // Cascade
                self.compute_cascade(elem_idx, new_state, flags, &mut effects.cascade);
            }

            SequenceState::Interrupted => {
                let mut cancel_path_request_owner = None;
                if self.elements[elem_idx].data.is_movement() {
                    // Optional path-request cancellation runs
                    // before the base-class Interrupted transition. The
                    // engine consumes this marker before dispatching the
                    // resulting condolence card.
                    if self.elements[elem_idx].command == Command::MoveWaiting {
                        self.elements[elem_idx].command = Command::Move;
                        cancel_path_request_owner =
                            Some(self.elements[elem_idx].owner.unwrap_or_else(|| {
                                panic!(
                                    "MoveWaiting element {:?}/{elem_idx} has no actor owner",
                                    self.id
                                )
                            }));
                    }
                    effects.interrupt_linked_seek = self.elements[elem_idx]
                        .legacy_v48
                        .as_ref()
                        .and_then(|legacy| legacy.linked_seek)
                        .flatten();
                }
                // The original game's transition to interrupted deliberately does not
                // start postponed elements. Instruction arbitration
                // transfers the postponed pointer to the replacement before
                // interrupting the old element; a generic interruption must
                // neither start nor detach either representation here.
                // Clear orders
                self.elements[elem_idx].orders.clear();
                // Notify owner
                effects.notify_owner = self.elements[elem_idx].owner;
                if let Some(owner) = self.elements[elem_idx].owner {
                    effects.condolation = Some(PendingCondolation {
                        owner,
                        command: self.elements[elem_idx].command,
                        terminal_state: new_state,
                        seq_id: self.id,
                        elem_idx: elem_idx as u16,
                        was_selected: false,
                        from_halt: false,
                        postponed_successor_pending: false,
                        cancel_path_request_owner,
                    });
                }
                // Cascade
                self.compute_cascade(elem_idx, new_state, flags, &mut effects.cascade);
            }

            SequenceState::Terminated => {
                match old_state {
                    SequenceState::Todo | SequenceState::InProgress | SequenceState::Postponed => {
                        // Notify owner
                        effects.notify_owner = self.elements[elem_idx].owner;
                        if let Some(owner) = self.elements[elem_idx].owner {
                            effects.condolation = Some(PendingCondolation {
                                owner,
                                command: self.elements[elem_idx].command,
                                terminal_state: new_state,
                                seq_id: self.id,
                                elem_idx: elem_idx as u16,
                                was_selected: false,
                                from_halt: false,
                                postponed_successor_pending: false,
                                cancel_path_request_owner: None,
                            });
                        }
                        // Tell the sequence this element is done
                        effects.signal_ready = true;
                        // Start postponed if any
                        if let Some(postponed_idx) = self.elements[elem_idx].postponed_element_index
                        {
                            effects.start_postponed = Some((elem_idx, postponed_idx));
                        }
                        // Release cross-sequence postponed successor, if any.
                        if let Some(cross) = self.elements[elem_idx].cross_postponed.take() {
                            effects.resume_cross_postponed = Some(cross);
                        }
                    }
                    _ => {
                        // Assign the new state before dispatching its effects.
                        // Its assertion is compiled out in the shipping build,
                        // leaving an already-interrupted/impossible element
                        // Terminated without repeating owner/sequence effects.
                        // Loaded games can legitimately resume at this
                        // release-build edge, so retain the state transition
                        // and make the diagnostic non-fatal.
                        tracing::warn!(
                            sequence_id = self.id.0,
                            element_index = elem_idx,
                            ?old_state,
                            "sequence element terminated from a shipping-only state"
                        );
                    }
                }
            }

            SequenceState::Postponed => {
                // Demote `MoveOk` back to `Move` on movement elements.
                // The path-cancel half is handled by the engine-side
                // `stop_owner_active_mechanics`, but the command
                // demotion belongs on the state transition itself.
                if self.elements[elem_idx].data.is_movement()
                    && self.elements[elem_idx].command == Command::MoveOk
                {
                    self.elements[elem_idx].command = Command::Move;
                }
            }

            SequenceState::Done | SequenceState::Todo => {
                // Not typically set externally
            }
        }

        effects
    }

    /// Compute cascade targets for interrupted/impossible state propagation.
    fn compute_cascade(
        &self,
        elem_idx: usize,
        new_state: SequenceState,
        flags: CascadeFlags,
        cascade: &mut Vec<(usize, SequenceState, CascadeFlags)>,
    ) {
        let command_level = self.elements[elem_idx].command_level;

        if flags.contains(CascadeFlags::FOLLOWING) {
            if let Some(next) = self.following_element_index(elem_idx) {
                cascade.push((next, new_state, CascadeFlags::FOLLOWING));
            }
        } else if flags.contains(CascadeFlags::NEXT_LEVEL) {
            // Find the first linked follower with a different command level.
            let mut visited = HashSet::new();
            let mut next = self.following_element_index(elem_idx);
            while let Some(next_idx) = next {
                assert!(
                    visited.insert(next_idx),
                    "loaded v48 sequence {:?} has a cycle in its following chain at {next_idx}",
                    self.id
                );
                if self.elements[next_idx].command_level != command_level {
                    cascade.push((next_idx, new_state, CascadeFlags::FOLLOWING));
                    break;
                }
                next = self.following_element_index(next_idx);
            }
        }
    }

    /// Interpret the stored following link, without traversal policy.
    /// Runtime-authored sequences wire this pointer in append order. Loaded
    /// v48 elements retain its exact serialized target, including null and
    /// non-adjacent or cross-sequence links. Target validation, owner filtering,
    /// and the runtime severed-link mirror belong to the named queries below.
    fn raw_following_ref(&self, elem_idx: usize) -> Option<SequenceElementRef> {
        let element = self.elements.get(elem_idx)?;
        if let Some(legacy) = &element.legacy_v48 {
            legacy.next
        } else {
            self.elements
                .get(elem_idx + 1)
                .map(|_| SequenceElementRef::new(self.id, elem_idx + 1))
        }
    }

    /// Following edge visible to live queries after Stop severs a link.
    /// Runtime-authored elements remain physically adjacent after Halt; seeing
    /// through that edge could suppress the selected actor's condolence callback.
    fn unsevered_following_ref(&self, elem_idx: usize) -> Option<SequenceElementRef> {
        let element = self.elements.get(elem_idx)?;
        if element.next_link_severed {
            return None;
        }
        self.raw_following_ref(elem_idx)
    }

    /// Cascades operate on local indices and must reject cross-sequence or
    /// dangling imported edges rather than treating them as the end of a chain.
    fn following_element_index(&self, elem_idx: usize) -> Option<usize> {
        let next = self.unsevered_following_ref(elem_idx)?;
        // TODO(legacy-sequence-runtime): promote cascade effects from
        // element indices to SequenceElementRef if a real save ever
        // contains a following pointer outside its mummy sequence.
        assert_eq!(
            next.sequence_id, self.id,
            "loaded v48 following pointer crosses sequences: {:?}/{elem_idx} -> {:?}/{}",
            self.id, next.sequence_id, next.element_index
        );
        assert!(
            next.element_index < self.elements.len(),
            "loaded v48 following pointer targets missing element: {:?}/{elem_idx} -> {}",
            self.id,
            next.element_index
        );
        Some(next.element_index)
    }

    /// Stop an element (and possibly its postponed chain) up to a given priority.
    ///
    /// Returns the state-change effects produced. Multiple effects are
    /// possible because the implementation has two recursive calls: one
    /// inside the priority-too-strong branch (recurse on `next`) and a
    /// second **unconditional** recursion on the postponed element
    /// after the if/else. Both recursions can produce their own
    /// `StateChangeEffects`, and the manager must process each in turn —
    /// hence the `Vec` return.
    ///
    /// `resolver` is invoked lazily when a reached element's priority is
    /// still `NotYetSet`. Build one via
    /// [`crate::engine::EngineInner::priority_resolver`].
    pub fn stop_element(
        &mut self,
        elem_idx: usize,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) -> Vec<StateChangeEffects> {
        self.stop_element_with_cross_targets(elem_idx, stop_priority, resolver)
            .0
    }

    /// Stop one Original linked graph and also return cross-sequence
    /// postponed edges encountered at nodes actually visited by `Stop`.
    fn stop_element_with_cross_targets(
        &mut self,
        elem_idx: usize,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) -> (Vec<StateChangeEffects>, Vec<(SequenceId, usize)>) {
        let mut cross_targets = Vec::new();
        let effects = self.stop_element_with_debug_depth(
            elem_idx,
            stop_priority,
            resolver,
            0,
            &mut cross_targets,
        );
        (effects, cross_targets)
    }

    fn stop_element_with_debug_depth(
        &mut self,
        elem_idx: usize,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
        depth: usize,
        cross_targets: &mut Vec<(SequenceId, usize)>,
    ) -> Vec<StateChangeEffects> {
        {
            let elem = &self.elements[elem_idx];
            tracing::trace!(
                target: "parity_stop",
                depth,
                elem_idx,
                command = ?elem.command,
                state = ?elem.state,
                priority = ?elem.priority,
                postponed = ?elem.postponed_element_index,
                "stop_element enter"
            );
        }
        let mut all_effects: Vec<StateChangeEffects> = Vec::new();

        // The original game handles this node's postponed reference unconditionally at
        // the end of Stop. Same-sequence postponed edges recurse below;
        // report split-storage edges to SequenceManager's owner worklist.
        if let Some(cross) = self.elements[elem_idx].cross_postponed
            && !cross_targets.contains(&cross)
        {
            cross_targets.push(cross);
        }

        // Determine priority if not yet set: ask the owning actor's
        // priority resolver and promote `None` to `Normal` so the stop
        // actually succeeds on commands like WAIT / FREEZE.
        if self.elements[elem_idx].priority == SequencePriority::NotYetSet {
            tracing::trace!(
                target: "parity_stop",
                depth,
                elem_idx,
                "stop_element before priority resolver"
            );
            let mut resolved = resolver(&self.elements[elem_idx]);
            tracing::trace!(
                target: "parity_stop",
                depth,
                elem_idx,
                ?resolved,
                "stop_element after priority resolver"
            );
            if resolved == SequencePriority::None {
                resolved = SequencePriority::Normal;
            }
            self.elements[elem_idx].priority = resolved;
        }

        // Is the priority weak enough to be stopped? (>= means weaker or equal)
        if self.elements[elem_idx].priority >= stop_priority {
            if self.elements[elem_idx].state == SequenceState::InProgress
                && self.elements[elem_idx].data.is_movement()
            {
                // Movements in progress are kept (for transition) but their
                // successor is interrupted
                if let Some(next_idx) = self.following_element_index(elem_idx) {
                    tracing::trace!(
                        target: "parity_stop",
                        depth,
                        from = elem_idx,
                        to = next_idx,
                        "stop_element before interrupt movement successor"
                    );
                    all_effects.push(self.set_element_state(
                        next_idx,
                        SequenceState::Interrupted,
                        CascadeFlags::NEXT_LEVEL,
                    ));
                    tracing::trace!(
                        target: "parity_stop",
                        depth,
                        from = elem_idx,
                        to = next_idx,
                        "stop_element after interrupt movement successor"
                    );
                }
            } else {
                tracing::trace!(
                    target: "parity_stop",
                    depth,
                    elem_idx,
                    "stop_element before interrupt self"
                );
                all_effects.push(self.set_element_state(
                    elem_idx,
                    SequenceState::Interrupted,
                    CascadeFlags::NEXT_LEVEL,
                ));
                tracing::trace!(
                    target: "parity_stop",
                    depth,
                    elem_idx,
                    "stop_element after interrupt self"
                );
            }
        } else {
            // Can't stop this element, but try the next one.
            if let Some(next_idx) = self.following_element_index(elem_idx) {
                tracing::trace!(
                    target: "parity_stop",
                    depth,
                    from = elem_idx,
                    to = next_idx,
                    "stop_element before next"
                );
                let sub = self.stop_element_with_debug_depth(
                    next_idx,
                    stop_priority,
                    resolver,
                    depth + 1,
                    cross_targets,
                );
                all_effects.extend(sub);
                tracing::trace!(
                    target: "parity_stop",
                    depth,
                    from = elem_idx,
                    to = next_idx,
                    "stop_element after next"
                );
                if self.elements[next_idx].state == SequenceState::Interrupted {
                    // the explicit next-element reference is cleared
                    // after recursively stopping the successor. The edge is gone for
                    // every later reader, not just for cascades: a movement
                    // element that survives this Stop must afterwards see
                    // the next order is neither movement nor jumping
                    // and grow the
                    // end transition when fast-movement conversion re-runs path postprocessing.
                    if let Some(legacy) = self.elements[elem_idx].legacy_v48.as_mut() {
                        legacy.next = None;
                    }
                    self.elements[elem_idx].next_link_severed = true;
                }
            }
        }

        // Unconditional postponed-element handling — runs after the
        // if/else above. Without this, a postponed sibling attached to
        // an Interrupted parent stays alive indefinitely.
        if let Some(postponed_idx) = self.elements[elem_idx].postponed_element_index {
            tracing::trace!(
                target: "parity_stop",
                depth,
                from = elem_idx,
                to = postponed_idx,
                "stop_element before postponed"
            );
            let sub = self.stop_element_with_debug_depth(
                postponed_idx,
                stop_priority,
                resolver,
                depth + 1,
                cross_targets,
            );
            all_effects.extend(sub);
            tracing::trace!(
                target: "parity_stop",
                depth,
                from = elem_idx,
                to = postponed_idx,
                "stop_element after postponed"
            );
            // Null the postponed link when the recursive stop left it
            // INTERRUPTED so a subsequent `start_postponed` cascade
            // doesn't try to wake an already-interrupted element.
            if self.elements[postponed_idx].state == SequenceState::Interrupted {
                self.elements[elem_idx].postponed_element_index = None;
            }
        }

        tracing::trace!(target: "parity_stop", depth, elem_idx, "stop_element exit");
        all_effects
    }
}

// ═══════════════════════════════════════════════════════════════════
//  SequenceAction — dispatch events returned by hourglass
// ═══════════════════════════════════════════════════════════════════

/// An action the engine needs to perform on behalf of the sequence system.
/// Returned by [`SequenceManager::hourglass`].
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum SequenceAction {
    /// Dispatch this element to its owner entity's instruction handler.
    /// The entity will translate the command into orders.
    InstructOwner {
        owner: EntityId,
        sequence_id: SequenceId,
        element_index: usize,
    },
    /// Dispatch this element to the engine.
    /// Used for elements with no owner (camera, locks, etc.).
    EngineCommand {
        sequence_id: SequenceId,
        element_index: usize,
    },
    /// Execute immediately on the owner (synchronous, single-frame command).
    ExecuteImmediateOwner {
        owner: EntityId,
        sequence_id: SequenceId,
        element_index: usize,
    },
    /// Execute immediately on the engine (synchronous, single-frame command).
    ExecuteImmediateEngine {
        sequence_id: SequenceId,
        element_index: usize,
    },
}

/// One slot of the manager's synchronous-registration buffer.
///
/// The original game walks the elements of one command level **one at a time**,
/// and each step
/// either executes a waiting-priority element immediately or
/// may run an immediate command **inline** during registration.
/// Whatever that inline execution does therefore happens before the loop even
/// looks at the next sibling — in particular actor stopping followed by
/// stopping not-yet-launched sequence elements
/// cannot see siblings that have not been registered yet.
///
/// Rust cannot execute those commands inside the manager, so a still-pending
/// loop iteration is parked in the same ordered buffer as the action it must
/// follow.  Draining the buffer resumes the loop at exactly the point the
/// original-game evaluation would have returned to.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum PendingSyncEntry {
    /// An action the engine must dispatch.
    Action(SequenceAction),
    /// A successor-registration iteration that has not run yet.
    Register {
        sequence_id: SequenceId,
        element_index: usize,
    },
}

impl PendingSyncEntry {
    fn as_action(&self) -> Option<&SequenceAction> {
        match self {
            PendingSyncEntry::Action(action) => Some(action),
            PendingSyncEntry::Register { .. } => None,
        }
    }

    fn is_register(&self) -> bool {
        matches!(self, PendingSyncEntry::Register { .. })
    }
}

// ═══════════════════════════════════════════════════════════════════
//  SequenceManager
// ═══════════════════════════════════════════════════════════════════

/// Manages all active sequences and dispatches their elements.
///
/// Central coordinator:
/// - Owns all active sequences
/// - Maintains a deferred "to go" queue processed each frame
/// - Handles launching, termination, and cleanup
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
#[serde(transparent)]
struct OrderedSequences(IndexMap<SequenceId, Sequence>);

impl std::ops::Deref for OrderedSequences {
    type Target = IndexMap<SequenceId, Sequence>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for OrderedSequences {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<IndexMap<SequenceId, Sequence>> for OrderedSequences {
    fn from(sequences: IndexMap<SequenceId, Sequence>) -> Self {
        Self(sequences)
    }
}

impl IntoIterator for OrderedSequences {
    type Item = (SequenceId, Sequence);
    type IntoIter = indexmap::map::IntoIter<SequenceId, Sequence>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a OrderedSequences {
    type Item = (&'a SequenceId, &'a Sequence);
    type IntoIter = indexmap::map::Iter<'a, SequenceId, Sequence>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a> IntoIterator for &'a mut OrderedSequences {
    type Item = (&'a SequenceId, &'a mut Sequence);
    type IntoIter = indexmap::map::IterMut<'a, SequenceId, Sequence>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}

impl crate::bitcode_adapters::NativeBitcode for OrderedSequences {
    type Wire = Vec<(SequenceId, Sequence)>;

    fn to_wire(&self) -> Self::Wire {
        self.0
            .iter()
            .map(|(&id, sequence)| (id, sequence.clone()))
            .collect()
    }

    fn from_wire(wire: Self::Wire) -> Self {
        let mut sequences = IndexMap::with_capacity(wire.len());
        for (id, sequence) in wire {
            assert!(
                !sequences.contains_key(&id),
                "native bitcode snapshot contains duplicate sequence id {id:?}"
            );
            sequences.insert(id, sequence);
        }
        Self(sequences)
    }
}

crate::bitcode_adapters::impl_native_bitcode!(OrderedSequences);

#[derive(Debug, Clone, Copy)]
struct ActorStopSummary {
    weakest_priority: SequencePriority,
    /// No live element owned by the actor has a same-sequence next or
    /// postponed successor. Cross-sequence successors are owner-checked by
    /// `stop_owner_current_from_root`.
    cross_only: bool,
}

#[derive(Debug, Clone, Copy)]
struct PostponeTailSummary {
    tail: SequenceElementRef,
    hops: usize,
    weakest_priority: SequencePriority,
    /// Every node in this cross chain has no same-sequence successor that a
    /// Stop would additionally traverse.
    cross_only: bool,
}

#[derive(Debug, Clone, Copy)]
struct StopNoopSummary {
    tail: SequenceElementRef,
}

#[derive(Debug, Clone, robin_state_hash_derive::StateHash, bitcode::Encode, bitcode::Decode)]
pub struct SequenceManager {
    /// All active sequences, keyed by `SequenceId` in original-game manager
    /// insertion order. `IndexMap` preserves that scan order while retaining
    /// efficient ID lookup, and cleanup does not change any stored ID.
    /// Every `SequenceId` stored elsewhere (in
    /// `elements_to_go`, `actor_in_progress`, `cross_postponed`,
    /// `post_seek_sequence`, etc.) stays valid across cleanup.
    ///
    /// Fresh sequences normally have monotonic IDs, but loaded Original
    /// managers can legitimately contain non-monotonic IDs in launch order.
    /// Several first-match scans depend on preserving that order exactly.
    sequences: OrderedSequences,

    /// Actor → every `SequenceElementRef` whose element is currently
    /// live (`Todo`, `InProgress`, or `Postponed`) and owned by that
    /// actor.
    ///
    /// Lets engine paths answer "does this actor already have work?"
    /// without scanning every active sequence. It is derived from
    /// `sequences` and serialized with the manager so snapshots remain
    /// self-contained.
    // EntityId is a tagged enum; populated indexes need reversible JSON keys.
    actor_live: BTreeMap<EntityId, BTreeSet<SequenceElementRef>>,

    /// Weakest priority among each actor's live elements.
    ///
    /// This is a derived acceleration index for actor stopping. The original game walks
    /// the selected element's postponed chain even when every element is too
    /// strong for the requested stop. Large swordfight crowds can append one
    /// strong postponed element between successive `Stop(PREFERENCE)` calls,
    /// making that pointer walk triangular. If this index proves that *all*
    /// of an actor's live work is stronger than the stop, the selected graph
    /// is necessarily effect-free and can be left untouched.
    ///
    /// Snapshots omit the index; it is rebuilt lazily from `actor_live`.
    #[bitcode(skip)]
    #[state_hash(skip)]
    actor_stop_summaries: BTreeMap<EntityId, ActorStopSummary>,

    /// Per-owner tails of cross-sequence postponed chains for a prospective
    /// waiter's priority. Equal-priority swordfight instructions repeatedly
    /// append to the same chain; caching the proven `Postpone` prefix turns
    /// that operation from a triangular root walk into amortized O(1).
    /// Every cross-link topology rewrite explicitly invalidates the affected
    /// owner before installing a replacement entry.
    #[bitcode(skip)]
    #[state_hash(skip)]
    postpone_tail_cache:
        BTreeMap<EntityId, BTreeMap<(SequenceElementRef, SequencePriority), PostponeTailSummary>>,

    /// Selected Stop graphs already proven to produce no terminal
    /// transitions for one stop priority. This is repaired by an exact Stop
    /// traversal, so unrelated same-owner work cannot force every later call
    /// to repeat that traversal.
    #[bitcode(skip)]
    #[state_hash(skip)]
    stop_noop_cache:
        BTreeMap<EntityId, BTreeMap<(SequenceElementRef, SequencePriority), StopNoopSummary>>,

    /// Actor → every `SequenceElementRef` whose element is currently
    /// `InProgress` and owned by that actor.
    ///
    /// Typically one entry per actor, but a `set_element_state`
    /// cascade can briefly land two elements in `InProgress` for the
    /// same actor before the earlier one terminates — so we track the
    /// whole set and
    /// [`current_element_for_actor`](Self::current_element_for_actor)
    /// returns [`BTreeSet::first`], which lexicographically matches the
    /// old "iterate sequences in vec order, first match wins" semantic
    /// (see `SequenceElementRef` docs for why `min` == "first by scan").
    ///
    /// Replaces an O(N_seq × N_elem) nested scan that was the single
    /// hottest per-tick function in a rollback-enabled debug profile
    /// (~5–15% depending on checker mode).
    actor_in_progress: BTreeMap<EntityId, BTreeSet<SequenceElementRef>>,

    /// Temporary actor selection installed by instruction handling while priority
    /// arbitration's outgoing state-change callbacks run.
    ///
    /// The original game assigns the selected element to the incoming
    /// element before interrupting/postponing the old one. The old element's
    /// synchronous removal-notification callback must therefore observe and
    /// arbitrate against the incoming element even though it has not reached
    /// `InProgress` yet. Entries only exist inside that callback boundary and
    /// are empty at stable frame/save boundaries.
    actor_instructing: BTreeMap<EntityId, Vec<(SequenceElementRef, bool)>>,

    /// Actor selection held across the accepted element's command
    /// translation.
    ///
    /// The original game keeps the selected element pointing at the
    /// accepted element for the whole of `Translate`, and only drops it
    /// afterwards when translation produced no orders. Commands whose
    /// translation bodies terminate or interrupt the element outright —
    /// EnterSwordfight onto an actor already holding its sword, a parry
    /// that repeats one already running, AssertPosition — therefore reach
    /// removal notification while still selected, which is what performs
    /// the actor-base movement-goal cleanup. Set for the duration of one
    /// command dispatch; empty at stable frame/save boundaries.
    actor_translating: Option<(EntityId, SequenceElementRef)>,

    /// Deferred queue of elements to start. Processed in `hourglass()`.
    /// Each entry is `(sequence id, element index within that sequence)`.
    /// Serialized so mid-frame snapshots (rollback / replay) preserve
    /// the deferred-dispatch queue.
    elements_to_go: VecDeque<(SequenceId, usize)>,

    /// Ordered synchronous-dispatch buffer for WAIT-priority elements and
    /// the [`SequenceElement::executed_immediately`] command groups
    /// (Teleport, LockAi, UnlockAi, ReplaceAnim, RestoreAnim, Speak,
    /// StartMobile, StopMobile, ActivateMobile, DeactivateMobile,
    /// Unblip, LockUser, UnlockUser, CameraJumpTo, Timer,
    /// ActionAvailable, CharacterAvailable, OpenScroll, SendMessage).
    ///
    /// `executed_immediately()` is a pure predicate, and
    /// `register_element_to_go` plus `register_wait_element_to_go` queue
    /// their `SequenceAction`s for engine-side dispatch onto this buffer.
    /// Inside `hourglass`, the
    /// buffer is drained alongside `elements_to_go` as a single ordered
    /// stream of actions. The engine action loop calls
    /// [`take_pending_synchronous_actions`](Self::take_pending_synchronous_actions)
    /// after each callback so re-entrant WAIT successors run before older
    /// siblings. External entry-point wrappers can continue to drain only
    /// the immediate subset through
    /// [`take_pending_immediate_actions`](Self::take_pending_immediate_actions).
    ///
    /// The buffer also carries not-yet-run
    /// next-sequence-element iterations
    /// ([`PendingSyncEntry::Register`]); see that type for why.
    pending_synchronous_actions: VecDeque<PendingSyncEntry>,

    /// Pending removal notifications. Populated whenever
    /// a sequence element transitions to Terminated / Interrupted /
    /// Impossible; drained by the engine after `hourglass` so
    /// per-entity cleanup (wasp-victim reset, carrier cleanup, etc.)
    /// fires in a single pass.
    pending_condolations: Vec<PendingCondolationDispatch>,

    /// Per-engine sequence-id counter. Replaces the previous global
    /// atomic so id allocation is part of the rollback snapshot —
    /// otherwise live and replayed engines would advance the counter
    /// at different rates and never reconcile.
    next_sequence_id: u32,
    /// Per-engine sequence-element id counter. Same rationale as
    /// `next_sequence_id` — every element gets stamped at launch so
    /// rollback can reproduce the ids exactly.
    next_element_id: u32,

    /// Set to `true` while an AI-initiated `Halt()` is tearing down the
    /// owning NPC's sequence via `stop_owner(Preference)`. Condolations
    /// queued during that window are tagged with `from_halt=true` so
    /// downstream removal-notification handlers can suppress the
    /// `Think(EVENT_DONE)` / `Think(EVENT_IMPOSSIBLE)` /
    /// `Think(EVENT_COULDNT_REACHPOINT)` dispatch on the interrupted
    /// sequence.
    halt_pending: bool,
}

impl serde::Serialize for SequenceManager {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PersistedSequenceManager::capture(self).serialize(serializer)
    }
}
impl<'de> serde::Deserialize<'de> for SequenceManager {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(PersistedSequenceManager::deserialize(deserializer)?.into_runtime())
    }
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedSequenceManager {
    sequences: OrderedSequences,
    #[serde(with = "serde_json_any_key::any_key_map_sized")]
    actor_live: BTreeMap<EntityId, BTreeSet<SequenceElementRef>>,
    #[serde(with = "serde_json_any_key::any_key_map_sized")]
    actor_in_progress: BTreeMap<EntityId, BTreeSet<SequenceElementRef>>,
    #[serde(with = "serde_json_any_key::any_key_map_sized")]
    actor_instructing: BTreeMap<EntityId, Vec<(SequenceElementRef, bool)>>,
    #[serde(deserialize_with = "Option::deserialize")]
    actor_translating: Option<(EntityId, SequenceElementRef)>,

    elements_to_go: VecDeque<(SequenceId, usize)>,

    pending_synchronous_actions: VecDeque<PendingSyncEntry>,

    pending_condolations: Vec<PendingCondolationDispatch>,

    next_sequence_id: u32,

    next_element_id: u32,

    halt_pending: bool,
}

impl PersistedSequenceManager {
    pub(crate) fn capture(value: &SequenceManager) -> Self {
        let SequenceManager {
            sequences: _,
            actor_live: _,
            actor_stop_summaries: _,
            postpone_tail_cache: _,
            stop_noop_cache: _,
            actor_in_progress: _,
            actor_instructing: _,
            actor_translating: _,
            elements_to_go: _,
            pending_synchronous_actions: _,
            pending_condolations: _,
            next_sequence_id: _,
            next_element_id: _,
            halt_pending: _,
        } = value;
        Self {
            sequences: value.sequences.clone(),
            actor_live: value.actor_live.clone(),
            actor_in_progress: value.actor_in_progress.clone(),
            actor_instructing: value.actor_instructing.clone(),
            actor_translating: value.actor_translating,
            elements_to_go: value.elements_to_go.clone(),
            pending_synchronous_actions: value.pending_synchronous_actions.clone(),
            pending_condolations: value.pending_condolations.clone(),
            next_sequence_id: value.next_sequence_id,
            next_element_id: value.next_element_id,
            halt_pending: value.halt_pending,
        }
    }

    pub(crate) fn into_runtime(self) -> SequenceManager {
        SequenceManager {
            sequences: self.sequences,
            actor_live: self.actor_live,
            actor_stop_summaries: BTreeMap::new(),
            postpone_tail_cache: BTreeMap::new(),
            stop_noop_cache: BTreeMap::new(),
            actor_in_progress: self.actor_in_progress,
            actor_instructing: self.actor_instructing,
            actor_translating: self.actor_translating,
            elements_to_go: self.elements_to_go,
            pending_synchronous_actions: self.pending_synchronous_actions,
            pending_condolations: self.pending_condolations,
            next_sequence_id: self.next_sequence_id,
            next_element_id: self.next_element_id,
            halt_pending: self.halt_pending,
        }
    }
}

/// Fully converted state accepted by the atomic v48 manager restore.
#[derive(Debug)]
pub(crate) struct SequenceManagerV48State {
    pub sequences: Vec<Sequence>,
    pub elements_to_go: VecDeque<(SequenceId, usize)>,
    pub next_sequence_id: u32,
    pub next_element_id: u32,
}

/// Pending entity cleanup emitted by the sequence manager when an
/// element finishes.  Drained by the engine after each `hourglass`.
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PendingCondolation {
    pub owner: EntityId,
    pub command: Command,
    pub terminal_state: SequenceState,
    /// Identifier of the element whose state change generated this
    /// condolation.  Used by `EngineInner::send_condolation_card` to
    /// scrub `actor.order_queue` / clear `active_ai_anim` of any
    /// entries tagged with this `(SequenceId, elem_idx)` — orders are
    /// owned by the sequence element and die with it.
    pub seq_id: SequenceId,
    pub elem_idx: u16,
    /// Whether this element was the actor's selected sequence element
    /// at the synchronous state-change-to-removal-notification boundary.
    /// Captured before terminal elements leave the in-progress index.
    pub was_selected: bool,
    /// `true` if this condolation was queued while the owning NPC's
    /// `inside_halt_method` flag was set — i.e. the sequence was torn
    /// down by an AI-initiated `Halt()` call.  The NPC's condolation
    /// handler uses this to skip the `Think(EVENT_DONE)` /
    /// `Think(EVENT_IMPOSSIBLE)` / `Think(EVENT_COULDNT_REACHPOINT)`
    /// dispatches for these.
    pub from_halt: bool,
    /// The state change detached a cross-sequence postponed successor,
    /// but postponed-element startup occurs after this notification in the
    /// original game. Such a successor prevents the element from being the
    /// last real action while removal notification is running.
    pub postponed_successor_pending: bool,
    /// Movement interruption ran path-request cancellation for a
    /// `MOVE_WAITING` element. The engine removes this owner's pending and
    /// failed requests immediately before this card's callback. This can
    /// differ from `owner` when a movement interrupts its linked Seek first.
    #[serde(deserialize_with = "Option::deserialize")]
    pub cancel_path_request_owner: Option<EntityId>,
}

/// A removal notification plus the portion of the state change that the original
/// performs only after removal notification returns. Keeping the
/// continuation beside the card preserves the depth-first order across
/// Rust's borrow-safe dispatch boundary.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PendingCondolationDispatch {
    pub card: PendingCondolation,
    effects_after_card: StateChangeEffects,
}

impl PendingCondolationDispatch {
    pub fn cross_postponed_successor(&self) -> Option<(SequenceId, usize)> {
        self.effects_after_card.resume_cross_postponed
    }
}

impl Default for SequenceManager {
    fn default() -> Self {
        Self::new()
    }
}

mod callbacks;
mod dispatch;
mod movement;
mod registry;
mod selection;

// ═══════════════════════════════════════════════════════════════════
//  Per-element make_* helpers (free functions for test reuse)
// ═══════════════════════════════════════════════════════════════════

/// Apply fast-movement conversion to a single element in-place. Returns with no effect
/// for non-movement elements.
pub fn make_fast_element(elem: &mut SequenceElement) {
    use crate::order::OrderType;

    let rewrite_orders = elem.state != SequenceState::Todo;
    let SequenceElementData::Movement { flags, action, .. } = &mut elem.data else {
        return;
    };
    *flags |= MoveFlags::FAST;
    *action = match *action {
        OrderType::WalkingUpright | OrderType::WalkingCrouched => OrderType::RunningUpright,
        OrderType::WalkingWithSword => OrderType::RunningWithSword,
        OrderType::WalkingWithShield => OrderType::RunningUpright,
        other => other,
    };
    if rewrite_orders {
        for order in elem.orders.iter_mut() {
            order.order_type = match order.order_type {
                OrderType::WalkingUpright | OrderType::WalkingCrouched => OrderType::RunningUpright,
                OrderType::WalkingWithSword => OrderType::RunningWithSword,
                OrderType::WalkingWithShield => OrderType::RunningUpright,
                OrderType::TransitionWaitingUprightWalkingUpright
                | OrderType::TransitionWaitingCrouchedWalkingCrouched => OrderType::RunningUpright,
                OrderType::TransitionWalkingUprightWaitingUpright
                | OrderType::TransitionWalkingCrouchedWaitingCrouched => OrderType::RunningUpright,
                other => other,
            };
        }
    }
}

/// Apply slow-movement conversion to a single element in-place.
pub fn make_slow_element(elem: &mut SequenceElement) {
    use crate::order::OrderType;

    let rewrite_orders = elem.state != SequenceState::Todo;
    let SequenceElementData::Movement { flags, action, .. } = &mut elem.data else {
        return;
    };
    *flags &= !MoveFlags::FAST;
    *action = match *action {
        // Walking variants stay as-is.
        OrderType::WalkingUpright | OrderType::WalkingCrouched => *action,
        OrderType::RunningUpright => OrderType::WalkingUpright,
        OrderType::RunningWithSword => OrderType::WalkingWithSword,
        other => other,
    };
    if rewrite_orders {
        for order in elem.orders.iter_mut() {
            order.order_type = match order.order_type {
                OrderType::RunningUpright => OrderType::WalkingUpright,
                OrderType::RunningWithSword => OrderType::WalkingWithSword,
                OrderType::TransitionWaitingUprightRunningUpright
                | OrderType::TransitionWalkingCrouchedRunningUpright => OrderType::WalkingUpright,
                OrderType::TransitionRunningUprightWaitingUpright => OrderType::WalkingUpright,
                other => other,
            };
        }
    }
}

/// Apply upright-posture conversion to a single element in-place. Cancels a pending
/// `CrouchDown` command by demoting it to `Null`.
pub fn make_upright_element(elem: &mut SequenceElement) {
    use crate::order::OrderType;

    // Cancel pending crouch-down.
    if elem.command == Command::CrouchDown {
        elem.command = Command::Null;
    }

    let rewrite_orders = elem.state != SequenceState::Todo;
    let SequenceElementData::Movement { action, .. } = &mut elem.data else {
        return;
    };
    *action = match *action {
        OrderType::WalkingUpright | OrderType::RunningUpright => *action,
        OrderType::WalkingCrouched => OrderType::WalkingUpright,
        other => other,
    };
    if rewrite_orders {
        for order in elem.orders.iter_mut() {
            order.order_type = match order.order_type {
                OrderType::WalkingCrouched => OrderType::WalkingUpright,
                OrderType::TransitionWaitingCrouchedWalkingCrouched
                | OrderType::TransitionWalkingUprightWalkingCrouched
                | OrderType::TransitionRunningUprightWalkingCrouched => OrderType::WalkingUpright,
                OrderType::TransitionWalkingCrouchedWaitingCrouched => OrderType::WalkingUpright,
                other => other,
            };
        }
    }
}

/// Apply crouched-posture conversion to a single element in-place.
pub fn make_crouched_element(elem: &mut SequenceElement) {
    use crate::order::OrderType;

    let rewrite_orders = elem.state != SequenceState::Todo;
    let SequenceElementData::Movement { flags, action, .. } = &mut elem.data else {
        return;
    };
    *flags &= !MoveFlags::FAST;
    *action = match *action {
        OrderType::WalkingCrouched => *action,
        OrderType::WalkingUpright | OrderType::RunningUpright => OrderType::WalkingCrouched,
        other => other,
    };
    if rewrite_orders {
        for order in elem.orders.iter_mut() {
            order.order_type = match order.order_type {
                OrderType::WalkingUpright | OrderType::RunningUpright => OrderType::WalkingCrouched,
                OrderType::TransitionWaitingUprightWalkingUpright
                | OrderType::TransitionRunningUprightWalkingUpright
                | OrderType::TransitionWalkingCrouchedWalkingUpright => OrderType::WalkingCrouched,
                OrderType::TransitionWalkingUprightWaitingUpright
                | OrderType::TransitionRunningUprightWaitingUpright => OrderType::WalkingCrouched,
                other => other,
            };
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
#[path = "sequence/tests.rs"]
mod tests;

#[cfg(test)]
mod native_bitcode_tests {
    use super::{OrderedSequences, Sequence, SequenceElement, SequenceId};
    use crate::bitcode_adapters::NativeBitcode;
    use crate::element::Command;
    use crate::player_command::GestureQuality;

    #[test]
    #[should_panic(expected = "native bitcode snapshot contains duplicate sequence id")]
    fn ordered_sequences_reject_duplicate_ids() {
        let id = SequenceId(7);
        let wire = vec![(id, Sequence::new()), (id, Sequence::new())];

        let _ = OrderedSequences::from_wire(wire);
    }

    #[test]
    fn gesture_quality_survives_saves_and_changes_sequence_hash() {
        let mut reduced = SequenceElement::new(1, Command::SwordstrikeThrustA, None);
        reduced.gesture_quality = GestureQuality::GOOD;

        let wire = bitcode::encode(&reduced);
        let decoded: SequenceElement = bitcode::decode(&wire).expect("decode sequence element");
        assert_eq!(decoded.gesture_quality, GestureQuality::GOOD);

        let mut pre_gesture = serde_json::to_value(&reduced).expect("serialize sequence element");
        pre_gesture
            .as_object_mut()
            .expect("sequence element object")
            .remove("gesture_quality");
        assert!(
            serde_json::from_value::<SequenceElement>(pre_gesture).is_err(),
            "a native sequence element without gesture quality must not enter the current schema"
        );

        let perfect = SequenceElement::new(1, Command::SwordstrikeThrustA, None);
        assert_ne!(
            robin_util::state_hash::compute(&perfect),
            robin_util::state_hash::compute(&reduced),
        );
    }
}
