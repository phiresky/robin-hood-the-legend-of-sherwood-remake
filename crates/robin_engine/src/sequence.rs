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
//  Cascade flags (for SetState propagation)
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
    /// `AppendMoveToSequence` call and raises its priority for
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
}

impl Field {
    /// Discriminant used by the Original `RHfield` enum. Rust's
    /// `RetainedMovementGoal` is an engine cache with no Original property
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
        /// Projected map-space destination. C++ `RHSequenceElementMovement`
        /// stores this as `mptDestination` and compares it against
        /// `GetPositionMap()`.
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
    /// Carries all the data needed by the victim's `Instruct` handler to
    /// apply and animate the damage.
    Damage {
        /// Origin of the damage (attacker entity).
        origin: Option<EntityId>,
        /// Projectile whose deferred impact this element represents.
        ///
        /// Original arrow damage keeps the `RHElementArrow*` until the
        /// sequence-manager phase so the victim's final facing can be read
        /// after damage translation. Runtime projectiles remain as
        /// tombstones long enough for this reference to stay valid.
        #[serde(default)]
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
    /// mission scripts or replayed quick-action sequences. Mirrors
    /// `RHSequenceElement::mbScriptDriven`; it is behavior, not save-only
    /// provenance, because bow equip/unequip reads it while executing.
    pub script_driven: bool,

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
    #[serde(default)]
    pub recorded_gate_path: Option<crate::gate::RecordedGatePath>,

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
    /// where the Instruct arbitration postpones a new element launched
    /// via a *different* sequence (e.g. a user-click sword strike issued
    /// while another sword strike sequence is mid-walk).
    pub cross_postponed: Option<(SequenceId, usize)>,

    /// `RHSequenceElement::Stop` nulls `mpsqeNextSequenceElement` once the
    /// recursive stop leaves that successor `RHSEQ_INTERRUPTED`
    /// (`RHsequenceelement.cpp:547-556`). Runtime-authored elements derive
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
    /// are being ported.
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
            posture_after_transition,
            action_state_after_transition,
            num_transition_orders,
            retained_movement_goal,
            recorded_gate_path,
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
            posture_after_transition,
            action_state_after_transition,
            num_transition_orders,
            retained_movement_goal,
            recorded_gate_path,
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
    /// Original's sequence-element constructor left both transition-result
    /// enums uninitialized. Actor `Instruct` overwrites them before any live
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
    /// Exact `RHSequenceElementMovement::maction` storage when the raw word is
    /// not an `RHanimation` and Original cannot consume it in this command/
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
            posture_after_transition: Posture::default(),
            action_state_after_transition: ActionState::default(),
            num_transition_orders: 0,
            retained_movement_goal: None,
            recorded_gate_path: None,
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
    /// Original provenance: `original-code/RHScript.cpp:6836-6843` and
    /// `original-code/RHScript.cpp:6890-6897` record all three fields, including
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
    /// Message conversion is intentionally strict: the original constructors
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
            // Original RHSequenceElementMovement::InsertTransitionEnd relabels
            // pOrder1 in place and deliberately leaves its existing
            // fTolerance untouched (the assignment is commented out).  Only
            // a newly inserted pOrder2 starts with RHOrder's zero tolerance.
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
    /// The `RHSEQ_INTERRUPTED` guard and the `GetPriority() ==
    /// RHPRIORITY_WAIT` test of `original-code/RHsequence.cpp:277-286` are
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

/// Process-local provenance captured at the terminal `SetState` boundary for
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

fn goal_owner_debug_enabled() -> bool {
    std::env::var_os("PARITY_DEBUG_GOAL_OWNER_HANDOFF").is_some()
}

fn goal_owner_debug_matches(owner: EntityId) -> bool {
    if !goal_owner_debug_enabled() {
        return false;
    }
    let filter = std::env::var("PARITY_DEBUG_GOAL_OWNER").unwrap_or_else(|_| {
        panic!(
            "PARITY_DEBUG_GOAL_OWNER_HANDOFF requires PARITY_DEBUG_GOAL_OWNER=pc|soldier|civilian:INDEX"
        )
    });
    let (kind, index) = filter.split_once(':').unwrap_or_else(|| {
        panic!("PARITY_DEBUG_GOAL_OWNER must look like pc|soldier|civilian:INDEX")
    });
    let index = index
        .parse::<u32>()
        .unwrap_or_else(|error| panic!("invalid PARITY_DEBUG_GOAL_OWNER={filter:?}: {error}"));
    match (kind, owner) {
        ("pc", EntityId::Pc(_))
        | ("soldier", EntityId::Soldier(_))
        | ("civilian", EntityId::Civilian(_)) => owner.index() == index,
        ("pc" | "soldier" | "civilian", _) => false,
        _ => panic!("PARITY_DEBUG_GOAL_OWNER has unsupported kind {kind:?}"),
    }
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
    /// Original interrupts this target with `CASCADE_FOLLOWING` before the
    /// movement element's base-class Interrupted handling.
    pub interrupt_linked_seek: Option<SequenceElementRef>,
    /// Elements whose state should also be changed (cascade).
    pub cascade: Vec<(usize, SequenceState, CascadeFlags)>,
    /// Whether `Sequence::element_ready()` should be called.
    pub signal_ready: bool,
    /// Whether to start a postponed element.
    /// `(blocker_index, postponed_index)` released by Original's
    /// `StartPostponedSequenceElement`.  Retaining the blocker lets the
    /// manager clear its pointer at the exact restart boundary, after the
    /// registration call (and, for Terminated, after Condolation + Ready).
    pub start_postponed: Option<(usize, usize)>,
    /// Cross-sequence postponed successor to resume.  Set when an
    /// element with a non-empty `cross_postponed` link terminates or is
    /// interrupted — the sequence manager takes this (seq_id, elem_idx)
    /// pair and registers it back on the `elements_to_go` queue.
    pub resume_cross_postponed: Option<(SequenceId, usize)>,
    /// Cross-sequence postponed link whose installation belongs after this
    /// element's synchronous `SendCondolationCard` callback. Actor priority
    /// arbitration can interrupt an existing postponed successor while an
    /// incoming element is still inside `Instruct`; Original does not expose
    /// that incoming element through the blocker's postponed pointer until
    /// the interrupted successor's callback returns.
    pub install_cross_postponed_after_card: Option<(SequenceId, usize, SequenceId, usize)>,
    /// Owner entity to notify via `SendCondolationCard`.
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
                    // RHSequenceElementMovement::MaybeCancelPathRequest runs
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
                // Original SetState(RHSEQ_INTERRUPTED) deliberately does not
                // call StartPostponedSequenceElement. Instruct arbitration
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
                        // Original assigns the new state before this switch.
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

    /// Resolve Original's `mpsqeNextSequenceElement` inside one sequence.
    /// Runtime-authored sequences wire this pointer in append order. Loaded
    /// v48 elements retain its exact serialized target, including null and
    /// non-adjacent links.
    fn following_element_index(&self, elem_idx: usize) -> Option<usize> {
        let element = self.elements.get(elem_idx)?;
        if element.next_link_severed {
            return None;
        }
        if let Some(legacy) = &element.legacy_v48 {
            let next = legacy.next?;
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
        } else {
            self.elements.get(elem_idx + 1).map(|_| elem_idx + 1)
        }
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

        // Original handles this node's postponed pointer unconditionally at
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
                    // `mpsqeNextSequenceElement = NULL`
                    // (`RHsequenceelement.cpp:552-555`). The edge is gone for
                    // every later reader, not just for cascades: a movement
                    // element that survives this Stop must afterwards see
                    // `IsNextMovementOrJump() == false`
                    // (`RHSequenceElementMovement.cpp:1251-1257`) and grow the
                    // end transition when `MakeFast` re-runs PostProcessPath.
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
    /// Dispatch this element to its owner entity via `Instruct()`.
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
/// `RHSequence::NextSequenceElementsGo` (`original-code/RHsequence.cpp:272-288`)
/// walks the elements of one command level **one at a time**, and each step
/// either calls `Go()` (RHPRIORITY_WAIT) or
/// `RHSequenceManager::RegisterSequenceElementToGo`
/// (`original-code/RHsequencemanager.cpp:967-978`), whose
/// `RHSequenceElement::ExecutedImmediately()` branch
/// (`original-code/RHsequenceelement.cpp:916-958`) runs the command **inline**.
/// Whatever that inline execution does therefore happens before the loop even
/// looks at the next sibling — in particular `RHElementActor::Stop` ->
/// `StopNotYetLaunchedSequenceElements` (`RHsequencemanager.cpp:1031-1054`)
/// cannot see siblings that have not been registered yet.
///
/// Rust cannot execute those commands inside the manager, so a still-pending
/// loop iteration is parked in the same ordered buffer as the action it must
/// follow.  Draining the buffer resumes the loop at exactly the point the
/// original call stack would have returned to.
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
    /// A `NextSequenceElementsGo` iteration that has not run yet.
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
        Self(wire.into_iter().collect())
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

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SequenceManager {
    /// All active sequences, keyed by `SequenceId` in Original manager
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
    actor_live: BTreeMap<EntityId, BTreeSet<SequenceElementRef>>,

    /// Weakest priority among each actor's live elements.
    ///
    /// This is a derived acceleration index for `Actor::Stop`. Original walks
    /// the selected element's postponed chain even when every element is too
    /// strong for the requested stop. Large swordfight crowds can append one
    /// strong postponed element between successive `Stop(PREFERENCE)` calls,
    /// making that pointer walk triangular. If this index proves that *all*
    /// of an actor's live work is stronger than the stop, the selected graph
    /// is necessarily effect-free and can be left untouched.
    ///
    /// Snapshots omit the index; it is rebuilt lazily from `actor_live`.
    #[serde(skip)]
    #[bitcode(skip)]
    #[state_hash(skip)]
    actor_stop_summaries: BTreeMap<EntityId, ActorStopSummary>,

    /// Per-owner tails of cross-sequence postponed chains for a prospective
    /// waiter's priority. Equal-priority swordfight instructions repeatedly
    /// append to the same chain; caching the proven `Postpone` prefix turns
    /// that operation from a triangular root walk into amortized O(1).
    /// Every cross-link topology rewrite explicitly invalidates the affected
    /// owner before installing a replacement entry.
    #[serde(skip)]
    #[bitcode(skip)]
    #[state_hash(skip)]
    postpone_tail_cache:
        BTreeMap<EntityId, BTreeMap<(SequenceElementRef, SequencePriority), PostponeTailSummary>>,

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

    /// Temporary actor selection installed by `Instruct` while priority
    /// arbitration's outgoing `SetState` callbacks run.
    ///
    /// Original assigns `RHElementActor::mpSequenceElement` to the incoming
    /// element before interrupting/postponing the old one. The old element's
    /// synchronous `SendCondolationCard` callback must therefore observe and
    /// arbitrate against the incoming element even though it has not reached
    /// `InProgress` yet. Entries only exist inside that callback boundary and
    /// are empty at stable frame/save boundaries.
    #[serde(default)]
    actor_instructing: BTreeMap<EntityId, Vec<(SequenceElementRef, bool)>>,

    /// Actor selection held across the accepted element's command
    /// translation.
    ///
    /// Original keeps `RHElementActor::mpSequenceElement` pointing at the
    /// accepted element for the whole of `Translate`, and only drops it
    /// afterwards when translation produced no orders. Commands whose
    /// translation bodies terminate or interrupt the element outright —
    /// EnterSwordfight onto an actor already holding its sword, a parry
    /// that repeats one already running, AssertPosition — therefore reach
    /// `SendCondolationCard` while still selected, which is what performs
    /// the actor-base movement-goal cleanup. Set for the duration of one
    /// command dispatch; empty at stable frame/save boundaries.
    #[serde(default)]
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
    /// `RHSequence::NextSequenceElementsGo` iterations
    /// ([`PendingSyncEntry::Register`]); see that type for why.
    pending_synchronous_actions: VecDeque<PendingSyncEntry>,

    /// Pending `SendCondolationCard` notifications.  Populated whenever
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
    /// downstream `SendCondolationCard` handlers can suppress the
    /// `Think(EVENT_DONE)` / `Think(EVENT_IMPOSSIBLE)` /
    /// `Think(EVENT_COULDNT_REACHPOINT)` dispatch on the interrupted
    /// sequence.
    halt_pending: bool,
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
    /// Whether this element was the actor's selected `mpSequenceElement`
    /// at the synchronous `SetState -> SendCondolationCard` boundary.
    /// Captured before terminal elements leave the in-progress index.
    #[serde(default)]
    pub was_selected: bool,
    /// `true` if this condolation was queued while the owning NPC's
    /// `inside_halt_method` flag was set — i.e. the sequence was torn
    /// down by an AI-initiated `Halt()` call.  The NPC's condolation
    /// handler uses this to skip the `Think(EVENT_DONE)` /
    /// `Think(EVENT_IMPOSSIBLE)` / `Think(EVENT_COULDNT_REACHPOINT)`
    /// dispatches for these.
    pub from_halt: bool,
    /// The state change detached a cross-sequence postponed successor,
    /// but the original `StartPostponedSequenceElement` point is after
    /// this card.  Such a successor makes `IsLastRealAction` false while
    /// `SendCondolationCard` is running.
    pub postponed_successor_pending: bool,
    /// Movement override ran `MaybeCancelPathRequest` for a
    /// `MOVE_WAITING` element. The engine removes this owner's pending and
    /// failed requests immediately before this card's callback. This can
    /// differ from `owner` when a movement interrupts its linked Seek first.
    #[serde(default)]
    pub cancel_path_request_owner: Option<EntityId>,
}

/// A condolence card plus the portion of `SetState` that the original
/// performs only after `SendCondolationCard` returns.  Keeping the
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

impl SequenceManager {
    /// Ordered queue and selected-owner views used by the schema-13 parity
    /// snapshot. References remain in native IDs here; the engine facade maps
    /// them to manager insertion ordinals before exposing the snapshot.
    #[doc(hidden)]
    pub(crate) fn parity_runtime_refs(
        &self,
    ) -> (
        Vec<(SequenceId, usize)>,
        Vec<(EntityId, SequenceElementRef)>,
    ) {
        if !self.pending_synchronous_actions.is_empty()
            || !self.pending_condolations.is_empty()
            || !self.actor_instructing.is_empty()
            || self.actor_translating.is_some()
            || self.halt_pending
        {
            panic!("parity sequence capture reached a non-quiescent dispatch boundary");
        }
        let actor_current = self
            .actor_in_progress
            .iter()
            .filter_map(|(owner, refs)| refs.first().copied().map(|element| (*owner, element)))
            .collect();
        (self.elements_to_go.iter().copied().collect(), actor_current)
    }
    fn is_actor_live_state(state: SequenceState) -> bool {
        matches!(
            state,
            SequenceState::Todo | SequenceState::InProgress | SequenceState::Postponed
        )
    }

    fn insert_actor_live_ref(&mut self, owner: EntityId, elem_ref: SequenceElementRef) {
        let (priority, cross_only) = self
            .get_sequence(elem_ref.sequence_id)
            .unwrap_or_else(|| {
                panic!(
                    "cannot index missing live sequence {:?}",
                    elem_ref.sequence_id
                )
            })
            .elements
            .get(elem_ref.element_index)
            .map(|element| {
                (
                    element.priority,
                    self.get_sequence(elem_ref.sequence_id)
                        .and_then(|sequence| {
                            sequence.following_element_index(elem_ref.element_index)
                        })
                        .is_none()
                        && element.postponed_element_index.is_none(),
                )
            })
            .unwrap_or_else(|| {
                panic!(
                    "cannot index missing live element {:?}/{}",
                    elem_ref.sequence_id, elem_ref.element_index
                )
            });
        let already_live = self.actor_live.contains_key(&owner);
        self.actor_live.entry(owner).or_default().insert(elem_ref);
        if !already_live {
            self.actor_stop_summaries.insert(
                owner,
                ActorStopSummary {
                    weakest_priority: priority,
                    cross_only,
                },
            );
        } else if let Some(summary) = self.actor_stop_summaries.get_mut(&owner) {
            summary.weakest_priority = summary.weakest_priority.max(priority);
            summary.cross_only &= cross_only;
        }
    }

    fn remove_actor_live_ref(&mut self, owner: EntityId, elem_ref: SequenceElementRef) {
        let removed_priority = self
            .get_element(elem_ref.sequence_id, elem_ref.element_index)
            .map(|element| element.priority);
        if let Some(set) = self.actor_live.get_mut(&owner) {
            set.remove(&elem_ref);
            if set.is_empty() {
                self.actor_live.remove(&owner);
            }
        }
        // Recomputing here can turn a linear stop cascade into quadratic
        // work. Invalidate only when the removed element may have supplied
        // the ceiling; the next Stop query rebuilds it once if needed.
        if removed_priority.is_some_and(|priority| {
            self.actor_stop_summaries
                .get(&owner)
                .is_some_and(|summary| summary.weakest_priority == priority)
        }) {
            self.actor_stop_summaries.remove(&owner);
        }
    }

    fn actor_stop_summary(&mut self, owner: EntityId) -> Option<ActorStopSummary> {
        if let Some(summary) = self.actor_stop_summaries.get(&owner) {
            return Some(*summary);
        }
        let summary = self.actor_live.get(&owner).and_then(|refs| {
            refs.iter().try_fold(
                ActorStopSummary {
                    weakest_priority: SequencePriority::NonInterruptable,
                    cross_only: true,
                },
                |mut summary, element_ref| {
                    let sequence =
                        self.get_sequence(element_ref.sequence_id)
                            .unwrap_or_else(|| {
                                panic!(
                                    "actor_live contains stale sequence ref {:?}",
                                    element_ref.sequence_id
                                )
                            });
                    let element = sequence
                        .elements
                        .get(element_ref.element_index)
                        .unwrap_or_else(|| {
                            panic!(
                                "actor_live contains stale element ref {:?}/{}",
                                element_ref.sequence_id, element_ref.element_index
                            )
                        });
                    summary.weakest_priority = summary.weakest_priority.max(element.priority);
                    summary.cross_only &= sequence
                        .following_element_index(element_ref.element_index)
                        .is_none()
                        && element.postponed_element_index.is_none();
                    Some(summary)
                },
            )
        });
        if let Some(summary) = summary {
            self.actor_stop_summaries.insert(owner, summary);
        }
        summary
    }

    pub fn new() -> Self {
        Self {
            sequences: IndexMap::new().into(),
            actor_live: BTreeMap::new(),
            actor_stop_summaries: BTreeMap::new(),
            postpone_tail_cache: BTreeMap::new(),
            actor_in_progress: BTreeMap::new(),
            actor_instructing: BTreeMap::new(),
            actor_translating: None,
            elements_to_go: VecDeque::new(),
            pending_synchronous_actions: VecDeque::new(),
            pending_condolations: Vec::new(),
            next_sequence_id: 1,
            next_element_id: 1,
            halt_pending: false,
        }
    }

    /// Atomically replace manager-owned state after every v48 identity and
    /// reference has been validated.
    pub(crate) fn restore_v48_state(&mut self, state: SequenceManagerV48State) {
        let mut restored = Self {
            sequences: state
                .sequences
                .into_iter()
                .map(|sequence| (sequence.id, sequence))
                .collect::<IndexMap<_, _>>()
                .into(),
            actor_live: BTreeMap::new(),
            actor_stop_summaries: BTreeMap::new(),
            postpone_tail_cache: BTreeMap::new(),
            actor_in_progress: BTreeMap::new(),
            actor_instructing: BTreeMap::new(),
            actor_translating: None,
            elements_to_go: state.elements_to_go,
            pending_synchronous_actions: VecDeque::new(),
            pending_condolations: Vec::new(),
            next_sequence_id: state.next_sequence_id,
            next_element_id: state.next_element_id,
            halt_pending: false,
        };
        restored.rebuild_indices();
        *self = restored;
    }

    /// Rebuild the actor element indexes from `sequences`.  This is
    /// still useful after older save loads and defensive repair paths.
    /// `sequences` itself is serialized, and `BTreeMap` preserves ids
    /// across cleanup, so no index-shift rebuild is needed on the
    /// cleanup path.
    pub fn rebuild_indices(&mut self) {
        self.actor_live.clear();
        self.actor_stop_summaries.clear();
        self.postpone_tail_cache.clear();
        self.actor_in_progress.clear();
        self.actor_instructing.clear();
        self.actor_translating = None;
        for (seq_id, seq) in &self.sequences {
            for (elem_idx, elem) in seq.elements.iter().enumerate() {
                let Some(owner) = elem.owner else {
                    continue;
                };
                let elem_ref = SequenceElementRef::new(*seq_id, elem_idx);
                if Self::is_actor_live_state(elem.state) {
                    self.actor_live.entry(owner).or_default().insert(elem_ref);
                    let cross_only = seq.following_element_index(elem_idx).is_none()
                        && elem.postponed_element_index.is_none();
                    self.actor_stop_summaries
                        .entry(owner)
                        .and_modify(|summary| {
                            summary.weakest_priority = summary.weakest_priority.max(elem.priority);
                            summary.cross_only &= cross_only;
                        })
                        .or_insert(ActorStopSummary {
                            weakest_priority: elem.priority,
                            cross_only,
                        });
                }
                if elem.state == SequenceState::InProgress {
                    self.actor_in_progress
                        .entry(owner)
                        .or_default()
                        .insert(elem_ref);
                }
            }
        }
    }

    /// Toggle the halt-pending marker. While `true`, any
    /// [`PendingCondolation`] queued via `process_effects` will be
    /// tagged with `from_halt=true`. Callers bracket a
    /// `stop_owner(Preference)` invocation with
    /// `set_halt_pending(true) … set_halt_pending(false)` so handlers
    /// can detect the AI-initiated `Halt()` window.
    pub fn set_halt_pending(&mut self, v: bool) {
        self.halt_pending = v;
    }

    /// Drain all pending SendCondolationCard notifications accumulated
    /// since the last call.  EngineInner calls this after each `hourglass`
    /// and dispatches to per-entity cleanup handlers.
    pub fn drain_pending_condolations(&mut self) -> Vec<PendingCondolationDispatch> {
        std::mem::take(&mut self.pending_condolations)
    }

    /// Whether Original's synchronous NPC condolence callback already owns
    /// this owner's `EVENT_COULDNT_REACHPOINT` delivery.
    ///
    /// `RHSequenceElement::SetState(RHSEQ_IMPOSSIBLE)` calls
    /// `SendCondolationCard` before it returns (`RHsequenceelement.cpp:405-420`),
    /// and the NPC override dispatches this event for a final Move/MoveOk
    /// action (`RHelementactornpc.cpp:6538-6568`). Rust suspends that callback
    /// in `pending_condolations`, so an engine EndThink surface must not
    /// overtake it with a second completion event.
    pub fn has_pending_couldnt_reachpoint_condolation(&self, owner: EntityId) -> bool {
        self.pending_condolations.iter().any(|pending| {
            let card = pending.card;
            card.owner == owner
                && !card.from_halt
                && !card.postponed_successor_pending
                && card.terminal_state == SequenceState::Impossible
                && matches!(
                    card.command,
                    Command::PassDoor | Command::Move | Command::MoveOk | Command::SitDown
                )
                && self.is_last_real_action(card.seq_id, usize::from(card.elem_idx))
        })
    }

    /// Restore a backlog detached around an owner-local synchronous boundary.
    /// The detached cards predate anything still queued, so they retain their
    /// original position at the front of the global FIFO.
    pub fn restore_pending_condolations(&mut self, mut pending: Vec<PendingCondolationDispatch>) {
        pending.append(&mut self.pending_condolations);
        self.pending_condolations = pending;
    }

    /// Drain only the pending condolations whose `owner` matches `owner`.
    /// Used by the per-NPC synchronous drain pass that runs right after
    /// each [`EngineInner::dispatch_filtered_stimulus`] — so a sequence
    /// that a handler's side effects just preempted fires its
    /// `Think(EVENT_DONE)` within the same call stack as the outer
    /// `Think` (re-entrant Think timing).  Condolations belonging to
    /// other entities remain queued for the end-of-tick global drain.
    pub fn drain_pending_condolations_for_owner(
        &mut self,
        owner: EntityId,
    ) -> Vec<PendingCondolationDispatch> {
        let mut matching = Vec::new();
        self.pending_condolations.retain(|c| {
            if c.card.owner == owner {
                matching.push(c.clone());
                false
            } else {
                true
            }
        });
        matching
    }

    /// Resume the part of `RHSequenceElement::SetState` that follows
    /// `SendCondolationCard`: cascade, `Ready`, and postponed-element
    /// activation.  The engine calls this only after the card's recursive
    /// `Think` and its same-frame side effects have reached a fixed point.
    pub fn finish_pending_condolation(&mut self, pending: PendingCondolationDispatch) {
        let was_halt_pending = self.halt_pending;
        self.halt_pending |= pending.card.from_halt;
        self.process_effects_after_condolation(pending.card.seq_id, pending.effects_after_card);
        self.halt_pending = was_halt_pending;
    }

    /// Suspend a cross-postponed replacement at the same callback boundary as
    /// an interrupted predecessor. This is the continuation of the outer
    /// Actor::Instruct priority arbitration, not a successor released by the
    /// interrupted element itself.
    pub fn install_cross_postponed_after_condolation(
        &mut self,
        interrupted: (SequenceId, usize),
        blocker: (SequenceId, usize),
        waiter: (SequenceId, usize),
    ) {
        let pending = self
            .pending_condolations
            .iter_mut()
            .rev()
            .find(|pending| {
                pending.card.seq_id == interrupted.0
                    && usize::from(pending.card.elem_idx) == interrupted.1
            })
            .unwrap_or_else(|| {
                panic!(
                    "interrupted postponed element {:?}/{} produced no condolence continuation",
                    interrupted.0, interrupted.1
                )
            });
        assert!(
            pending
                .effects_after_card
                .install_cross_postponed_after_card
                .is_none(),
            "interrupted postponed element {:?}/{} already has a deferred cross install",
            interrupted.0,
            interrupted.1
        );
        pending
            .effects_after_card
            .install_cross_postponed_after_card = Some((blocker.0, blocker.1, waiter.0, waiter.1));
    }

    /// Number of active sequences.
    pub fn sequence_count(&self) -> usize {
        self.sequences.len()
    }

    // ─── Lookup ─────────────────────────────────────────────────

    /// Get a sequence by ID. O(log N).
    pub fn get_sequence(&self, id: SequenceId) -> Option<&Sequence> {
        self.sequences.get(&id)
    }

    /// Get a mutable sequence by ID. O(log N).
    pub fn get_sequence_mut(&mut self, id: SequenceId) -> Option<&mut Sequence> {
        self.sequences.get_mut(&id)
    }

    /// Reassign a live element at an owner `Instruct` boundary.
    ///
    /// Original PC-on-shoulders movement changes the movement element's owner
    /// from rider to carrier only when the rider receives `Instruct`. Keep the
    /// derived actor indexes consistent with that pointer mutation.
    pub(crate) fn reassign_element_owner(
        &mut self,
        sequence_id: SequenceId,
        element_index: usize,
        new_owner: EntityId,
    ) {
        let element_ref = SequenceElementRef::new(sequence_id, element_index);
        let (old_owner, state) = self
            .get_element(sequence_id, element_index)
            .map(|element| (element.owner, element.state))
            .unwrap_or_else(|| {
                panic!("cannot reassign missing sequence element {sequence_id:?}/{element_index}")
            });
        let Some(old_owner) = old_owner else {
            panic!("cannot reassign ownerless sequence element {sequence_id:?}/{element_index}")
        };
        if old_owner == new_owner {
            return;
        }

        if Self::is_actor_live_state(state) {
            self.remove_actor_live_ref(old_owner, element_ref);
        }
        if state == SequenceState::InProgress {
            if let Some(set) = self.actor_in_progress.get_mut(&old_owner) {
                set.remove(&element_ref);
                if set.is_empty() {
                    self.actor_in_progress.remove(&old_owner);
                }
            }
        }

        self.get_element_mut(sequence_id, element_index)
            .expect("element disappeared during owner reassignment")
            .owner = Some(new_owner);

        if Self::is_actor_live_state(state) {
            self.insert_actor_live_ref(new_owner, element_ref);
        }
        if state == SequenceState::InProgress {
            self.actor_in_progress
                .entry(new_owner)
                .or_default()
                .insert(element_ref);
        }
    }

    fn index_sequence_actor_refs(&mut self, seq_id: SequenceId) {
        let refs: Vec<(EntityId, SequenceElementRef, SequenceState)> = {
            let Some(seq) = self.sequences.get(&seq_id) else {
                return;
            };
            seq.elements
                .iter()
                .enumerate()
                .filter_map(|(elem_idx, elem)| {
                    elem.owner
                        .map(|owner| (owner, SequenceElementRef::new(seq_id, elem_idx), elem.state))
                })
                .collect()
        };

        for (owner, elem_ref, state) in refs {
            if Self::is_actor_live_state(state) {
                self.insert_actor_live_ref(owner, elem_ref);
            }
            if state == SequenceState::InProgress {
                self.actor_in_progress
                    .entry(owner)
                    .or_default()
                    .insert(elem_ref);
            }
        }
    }

    /// Read-only iterator over every sequence currently owned by the
    /// manager. Used by engine-layer helpers that need to locate an
    /// actor's currently-executing element across all sequences — we
    /// don't keep a back-pointer on each actor.
    pub fn sequences_iter(&self) -> impl Iterator<Item = &Sequence> + '_ {
        self.sequences.values()
    }

    /// Install an authoritative replay route on the one pending point Seek
    /// created by DropAle. Original computes this route only when the
    /// postponed Seek is instructed, potentially many frames after the input
    /// command was recorded.
    pub(crate) fn inject_recorded_drop_ale_route(
        &mut self,
        actor: EntityId,
        destination: crate::coordinates::MapPoint,
        goal_sector: crate::position_interface::SectorHandle,
        goal_layer: u16,
        recorded_gate_path: crate::gate::RecordedGatePath,
    ) -> bool {
        let mut candidates = 0_usize;
        for sequence in self.sequences.values_mut() {
            for element in &mut sequence.elements {
                let SequenceElementData::Movement {
                    destination: element_destination,
                    layer,
                    sector,
                    element: target,
                    flags,
                    post_seek_sequence,
                    ..
                } = &mut element.data
                else {
                    continue;
                };
                let is_drop_ale = post_seek_sequence.as_ref().is_some_and(|post_seek| {
                    post_seek
                        .elements
                        .first()
                        .is_some_and(|post_element| post_element.command == Command::DropAle)
                });
                if element.owner != Some(actor)
                    || element.command != Command::Seek
                    || !Self::is_actor_live_state(element.state)
                    || target.is_some()
                    || !flags.contains(MoveFlags::SEEK)
                    || !is_drop_ale
                    || element_destination.x.to_bits() != destination.x.to_bits()
                    || element_destination.y.to_bits() != destination.y.to_bits()
                {
                    continue;
                }
                candidates += 1;
                assert!(
                    element.recorded_gate_path.is_none(),
                    "pending DropAle point Seek already has a recorded gate route"
                );
                *sector = Some(goal_sector);
                *layer = goal_layer;
                element.recorded_gate_path = Some(recorded_gate_path.clone());
            }
        }
        assert!(
            candidates <= 1,
            "recorded DropAle route matched {candidates} pending point Seeks for {actor:?}"
        );
        candidates == 1
    }

    pub(crate) fn has_pending_drop_ale_route_candidate(
        &self,
        actor: EntityId,
        destination: crate::coordinates::MapPoint,
    ) -> bool {
        let candidates = self
            .sequences
            .values()
            .flat_map(|sequence| &sequence.elements)
            .filter_map(|element| {
                let SequenceElementData::Movement {
                    destination: element_destination,
                    element: target,
                    flags,
                    post_seek_sequence,
                    ..
                } = &element.data
                else {
                    return None;
                };
                let is_drop_ale = post_seek_sequence.as_ref().is_some_and(|post_seek| {
                    post_seek
                        .elements
                        .first()
                        .is_some_and(|post_element| post_element.command == Command::DropAle)
                });
                if element.owner != Some(actor)
                    || element.command != Command::Seek
                    || !Self::is_actor_live_state(element.state)
                    || target.is_some()
                    || !flags.contains(MoveFlags::SEEK)
                    || !is_drop_ale
                    || element_destination.x.to_bits() != destination.x.to_bits()
                    || element_destination.y.to_bits() != destination.y.to_bits()
                {
                    return None;
                }
                Some(element.recorded_gate_path.is_some())
            })
            .collect::<Vec<_>>();
        assert!(
            candidates.len() <= 1,
            "recorded DropAle route matched {} pending point Seeks for {actor:?}",
            candidates.len()
        );
        if let Some(already_recorded) = candidates.first() {
            assert!(
                !*already_recorded,
                "pending DropAle point Seek already has a recorded gate route"
            );
            true
        } else {
            false
        }
    }

    pub(crate) fn is_registered_to_go(&self, seq_id: SequenceId, elem_idx: usize) -> bool {
        self.elements_to_go.contains(&(seq_id, elem_idx))
            && self
                .get_element(seq_id, elem_idx)
                .is_some_and(|element| element.state != SequenceState::Interrupted)
    }

    /// Snapshot the deferred manager FIFO without changing registration.
    /// Synchronous engine boundaries use this to identify only the elements
    /// authored by a nested statement while leaving older and foreign-owner
    /// work in place.
    pub(crate) fn deferred_elements_to_go(&self) -> Vec<(SequenceId, usize)> {
        self.elements_to_go.iter().copied().collect()
    }

    #[cfg(test)]
    pub(crate) fn v48_elements_to_go(&self) -> Vec<(SequenceId, usize)> {
        self.deferred_elements_to_go()
    }

    /// Get a reference to a specific element within a sequence.
    pub fn get_element(&self, seq_id: SequenceId, elem_idx: usize) -> Option<&SequenceElement> {
        self.get_sequence(seq_id)?.get(elem_idx)
    }

    /// Get a mutable reference to a specific element.
    pub fn get_element_mut(
        &mut self,
        seq_id: SequenceId,
        elem_idx: usize,
    ) -> Option<&mut SequenceElement> {
        self.get_sequence_mut(seq_id)?.get_mut(elem_idx)
    }

    /// Drop queue-time movement-goal snapshots held by live work for an
    /// actor whose outgoing movement genuinely exhausted. Those snapshots
    /// only bridge an interrupted replacement handoff; they must not revive
    /// a goal cleared by ordinary movement completion.
    pub(crate) fn clear_retained_movement_goals_for_actor(&mut self, actor: EntityId) {
        let live = self.actor_live.get(&actor).cloned().unwrap_or_default();
        for element_ref in live {
            let element = self
                .get_element_mut(element_ref.sequence_id, element_ref.element_index)
                .unwrap_or_else(|| {
                    panic!(
                        "actor_live contains stale element ref {:?}/{}",
                        element_ref.sequence_id, element_ref.element_index
                    )
                });
            // Replacement movements use the typed cache, while deferred
            // FaceTo stores the same snapshot as a Generic property until
            // the manager instructs its Turn. Both are Rust mirrors of
            // the one Original PositionGoalMap value owned and cleared by
            // Actor::SendCondolationCard (`RHelementactor.cpp:6698-6700`).
            element.retained_movement_goal = None;
            element.remove_property(Field::RetainedMovementGoal);
        }
    }

    // ─── Launch ─────────────────────────────────────────────────

    /// Launch a fully-built sequence. Returns its ID.
    pub fn launch_sequence(&mut self, mut sequence: Sequence) -> SequenceId {
        assert!(!sequence.is_empty(), "cannot launch an empty sequence");

        // Stamp a deterministic per-engine id over whatever the
        // `Sequence::new()` placeholder allocated. Counter advances
        // here so replay sees identical ids. Same treatment for each
        // element id — the global atomic in `SequenceElement::new`
        // was process-wide and broke rollback.
        sequence.id = SequenceId(self.next_sequence_id);
        self.next_sequence_id = self.next_sequence_id.wrapping_add(1);
        for element in sequence.elements.iter_mut() {
            element.id = self.next_element_id;
            self.next_element_id = self.next_element_id.wrapping_add(1);
        }
        let id = sequence.id;
        tracing::trace!(
            sequence_id = id.0,
            elements = ?sequence
                .elements
                .iter()
                .map(|element| (
                    element.owner,
                    element.command,
                    element.command_level,
                    element.state,
                    element.priority,
                    &element.data,
                ))
                .collect::<Vec<_>>(),
            "launching sequence"
        );
        sequence.launch();

        // Start the first batch of elements
        let to_go = sequence.next_elements_go();

        self.sequences.insert(id, sequence);
        self.index_sequence_actor_refs(id);

        // Register elements for dispatch, one loop iteration at a time.
        self.register_level_elements_to_go(id, to_go);

        id
    }

    /// Launch a single sequence element by wrapping it in a new sequence.
    pub fn launch_element(&mut self, mut element: SequenceElement) -> SequenceId {
        element.command_level = 1;
        let mut seq = Sequence::new();
        seq.append_element(element);
        self.launch_sequence(seq)
    }

    /// Interrupt one freshly launched actor Wait before its synchronous
    /// `Go()` action reaches `Instruct`.
    ///
    /// This is deliberately identity-based rather than an owner/command scan:
    /// EnterBeggar's DONE callback creates one Wait, postpones it behind the
    /// still-selected noninterruptible transition, then selected-PC
    /// `SelectAction(Beggar)` immediately stops that exact postponed element.
    /// Rust replays the callback after retiring the transition, so its split
    /// representation must remove the queued instruction before it can select
    /// the Wait. Preserve the launch (and therefore sequence/element ID
    /// consumption) while touching no older queued work for the same owner.
    pub(crate) fn interrupt_just_registered_wait_before_instruct(
        &mut self,
        owner: EntityId,
        sequence_id: SequenceId,
    ) {
        let element = self
            .get_element(sequence_id, 0)
            .unwrap_or_else(|| panic!("fresh Wait {sequence_id:?}/0 disappeared before Stop"));
        assert_eq!(
            element.owner,
            Some(owner),
            "fresh Wait {sequence_id:?}/0 changed owner before Stop"
        );
        assert_eq!(
            element.command,
            Command::Wait,
            "selected beggar callback may discard only its fresh Wait"
        );
        assert_eq!(
            element.priority,
            SequencePriority::Wait,
            "selected beggar callback Wait lost RHPRIORITY_WAIT"
        );
        assert_eq!(
            element.state,
            SequenceState::Todo,
            "selected beggar callback Wait must be stopped before Instruct"
        );
        assert!(
            element.orders.is_empty(),
            "selected beggar callback Wait translated before its Stop"
        );

        let target = (sequence_id, 0);
        let queued_actions = self
            .pending_synchronous_actions
            .iter()
            .filter(|entry| {
                matches!(
                    entry,
                    PendingSyncEntry::Action(SequenceAction::InstructOwner {
                        owner: queued_owner,
                        sequence_id: queued_sequence,
                        element_index: 0,
                    }) if *queued_owner == owner && *queued_sequence == sequence_id
                )
            })
            .count();
        assert_eq!(
            queued_actions, 1,
            "fresh Wait {sequence_id:?}/0 must have exactly one queued Go action"
        );
        self.pending_synchronous_actions.retain(|entry| {
            !matches!(
                entry,
                PendingSyncEntry::Action(SequenceAction::InstructOwner {
                    owner: queued_owner,
                    sequence_id: queued_sequence,
                    element_index: 0,
                }) if *queued_owner == owner && *queued_sequence == sequence_id
            )
        });
        assert!(
            !self.elements_to_go.contains(&target),
            "fresh RHPRIORITY_WAIT element unexpectedly entered the deferred manager FIFO"
        );
        assert!(
            self.terminate_sequence(sequence_id),
            "fresh Wait {sequence_id:?} disappeared before interruption"
        );
    }

    /// Launch a one-shot generic sequence carrying a single pre-built
    /// `Order` for `actor`, and immediately mark its element as
    /// `InProgress` so consumers (animation driver, AI peek-current)
    /// see it this frame rather than waiting for the next
    /// `hourglass` dispatch.  Used by `BeginSwordfight` /
    /// `QuitSwordfight` / `process_pending_ai_orders` to build a
    /// generic element, push the order onto its `orders` queue, then
    /// launch with priority resolution firing synchronously.  Keeping
    /// every in-flight `Order` attached to an `InProgress` element
    /// means cancellation (via `set_element_state`) naturally discards
    /// the orders along with the element.
    ///
    /// Suffixed `_unchecked` because this path bypasses the Instruct
    /// equivalent (posture/action-state stamp + priority arbitration
    /// against the actor's current element).  Every caller except
    /// `EngineInner::launch_single_order_sequence_stamped` should go
    /// through that wrapper; the `_unchecked` form is kept only for
    /// the stamped wrapper's internals.  A grep for this name should
    /// turn up exactly one caller.
    pub(crate) fn launch_single_order_sequence_unchecked(
        &mut self,
        actor: EntityId,
        command: Command,
    ) -> SequenceId {
        // Launch the empty element.  The caller (always
        // `EngineInner::launch_single_order_sequence_stamped`) is
        // responsible for running the Instruct-equivalent (posture
        // stamp + `generate_transition` + arbitration) and THEN
        // appending the pre-baked single order.  Ordering matters:
        // `generate_transition` (exit + posture + enter) populates the
        // order queue BEFORE `Translate` pushes the command's own
        // order, so those transitions play first.
        let elem = SequenceElement::new_generic(1, command, Some(actor));
        self.launch_element(elem)
    }

    /// Push an `Order` onto the given element.  Panics if the handle is
    /// stale — callers must hold a live `(seq_id, elem_idx)` for an
    /// element they just launched or are currently dispatching, so a
    /// `None` here means a bug upstream, not a recoverable race.
    pub fn push_order_on(&mut self, seq_id: SequenceId, elem_idx: usize, order: Order) {
        match self.get_element_mut(seq_id, elem_idx) {
            Some(elem) => elem.push_order(order),
            None => panic!(
                "push_order_on: no element at ({:?}, {}) — handle is stale",
                seq_id, elem_idx
            ),
        }
    }

    /// Drop every queued `Order` on the given element, keeping the element
    /// itself live.  Panics on a stale handle for the same reason
    /// [`push_order_on`](Self::push_order_on) does.
    pub fn clear_orders_on(&mut self, seq_id: SequenceId, elem_idx: usize) {
        match self.get_element_mut(seq_id, elem_idx) {
            Some(elem) => elem.orders.clear(),
            None => panic!(
                "clear_orders_on: no element at ({:?}, {}) — handle is stale",
                seq_id, elem_idx
            ),
        }
    }

    /// Find the actor's in-progress sequence element.  O(log k) via
    /// [`actor_in_progress`](Self::actor_in_progress), where k is the
    /// number of simultaneously-`InProgress` elements owned by this
    /// actor (typically 1; briefly 2 during cascades).  When an idle
    /// `Wait` overlaps a real command, the real command is the actor's
    /// current element; otherwise old idle waits could starve combat
    /// elements that should be the actor's current sequence element.
    pub fn current_element_for_actor<I: Into<EntityId>>(
        &self,
        actor: I,
    ) -> Option<(SequenceId, usize)> {
        let actor = actor.into();
        if let Some((elem_ref, false)) = self
            .actor_instructing
            .get(&actor)
            .and_then(|stack| stack.last())
        {
            return Some((elem_ref.sequence_id, elem_ref.element_index));
        }
        if let Some((owner, elem_ref)) = self.actor_translating
            && owner == actor
        {
            return Some((elem_ref.sequence_id, elem_ref.element_index));
        }
        let set = self.actor_in_progress.get(&actor)?;
        let mut refs = set.iter();
        let first = *refs.next()?;
        if refs.next().is_none() {
            return Some((first.sequence_id, first.element_index));
        }

        for elem_ref in set {
            let Some(elem) = self.get_element(elem_ref.sequence_id, elem_ref.element_index) else {
                debug_assert!(false, "actor_in_progress contains stale element ref");
                continue;
            };
            if elem.command != Command::Wait {
                return Some((elem_ref.sequence_id, elem_ref.element_index));
            }
        }
        Some((first.sequence_id, first.element_index))
    }

    /// Whether the actor's Original-equivalent `mpSequenceElement` currently
    /// names a movement element.
    pub fn actor_has_selected_movement<I: Into<EntityId>>(&self, actor: I) -> bool {
        self.current_element_for_actor(actor)
            .and_then(|(sequence_id, element_index)| self.get_element(sequence_id, element_index))
            .is_some_and(|element| element.data.is_movement())
    }

    /// Furthest currently-live movement destination for an actor.  Shift-held
    /// planning uses this as the hypothetical origin when no queued move is
    /// already ahead of it.
    pub fn actor_planned_movement_destination(
        &self,
        actor: impl Into<EntityId>,
    ) -> Option<crate::coordinates::MapPoint> {
        let actor = actor.into();
        let live = self.actor_live.get(&actor)?;
        live.iter().rev().find_map(|element_ref| {
            let element = self.get_element(element_ref.sequence_id, element_ref.element_index)?;
            match &element.data {
                SequenceElementData::Movement { destination, .. } => Some(*destination),
                _ => None,
            }
        })
    }

    /// Select the accepted element for the duration of its command
    /// translation, or release it again.
    ///
    /// Releasing before a terminal `SetState` reproduces Original's
    /// post-`Translate` `mpSequenceElement = 0` for an accepted element whose
    /// translation produced no orders: that card must not claim the actor's
    /// movement goal, while a card raised from inside the translation body
    /// must.
    pub(crate) fn set_translating_element(
        &mut self,
        selection: Option<(EntityId, SequenceElementRef)>,
    ) {
        self.actor_translating = selection;
    }

    /// Read-only exposure for the opt-in goal/condolence ownership trace.
    pub(crate) fn goal_owner_debug_translating(&self) -> Option<(EntityId, SequenceElementRef)> {
        self.actor_translating
    }

    /// Release the translation selection when its own element is the one that
    /// just detached the actor's `mpSequenceElement`.
    ///
    /// `RHElementActor::SendCondolationCard` writes `mpSequenceElement = NULL`
    /// whenever the terminal element is the selected one
    /// (`RHelementactor.cpp:6696-6701`). A command body can reach that state
    /// from inside its own `Translate` — `RHPathFinder::AddPathRequest` calls
    /// `Stop()` on the actor whose Move is being translated
    /// (`RHpathfinder.cpp:464`) — and everything the same `Translate` does
    /// afterwards, including the `Wait()` it launches next, must observe the
    /// cleared pointer. Rust holds the translation identity until after the
    /// deferred condolence dispatch, so drop it here instead.
    pub(crate) fn clear_translating_element_if_selected(
        &mut self,
        actor: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        if self.actor_translating == Some((actor, SequenceElementRef::new(seq_id, elem_idx))) {
            self.actor_translating = None;
        }
    }

    /// Select an incoming element while the outgoing element's synchronous
    /// interruption callback runs.
    ///
    /// Original stores this selection in one raw `mpSequenceElement` pointer
    /// (`RHelementactor.cpp:1451-1456`). A recursively accepted `Instruct`
    /// overwrites that pointer permanently; returning from the recursive call
    /// does not restore its caller's selection. Keep the stack only to pair
    /// Rust callback scopes, and mark the parent superseded whenever a nested
    /// selection is installed.
    pub(crate) fn begin_instruct_callback(
        &mut self,
        owner: EntityId,
        sequence_id: SequenceId,
        element_index: usize,
    ) {
        let stack = self.actor_instructing.entry(owner).or_default();
        if let Some((_, superseded)) = stack.last_mut() {
            *superseded = true;
        }
        stack.push((SequenceElementRef::new(sequence_id, element_index), false));
    }

    /// Close a matching [`Self::begin_instruct_callback`] boundary, returning
    /// whether recursive work left this element selected. This is Original's
    /// post-priority callback pointer check (`RHelementactor.cpp:1473-1479`).
    pub(crate) fn end_instruct_callback(
        &mut self,
        owner: EntityId,
        sequence_id: SequenceId,
        element_index: usize,
    ) -> bool {
        let expected = SequenceElementRef::new(sequence_id, element_index);
        let stack = self
            .actor_instructing
            .get_mut(&owner)
            .unwrap_or_else(|| panic!("missing Instruct callback selection for {owner:?}"));
        let (selected, superseded) = stack
            .pop()
            .expect("Instruct callback selection stack is empty");
        assert_eq!(
            selected, expected,
            "Instruct callback selection closed out of order"
        );
        if stack.is_empty() {
            self.actor_instructing.remove(&owner);
        }
        !superseded
    }

    /// Find the first in-progress element owned by `actor` that
    /// satisfies `predicate`, using the same actor index as
    /// [`current_element_for_actor`](Self::current_element_for_actor).
    /// Lets callers check the actor's parallel in-progress elements
    /// without scanning every sequence in the manager.
    pub fn in_progress_element_for_actor_matching(
        &self,
        actor: impl Into<EntityId>,
        mut predicate: impl FnMut(&SequenceElement) -> bool,
    ) -> Option<(SequenceId, usize)> {
        let actor = actor.into();
        let set = self.actor_in_progress.get(&actor)?;
        for elem_ref in set {
            let Some(elem) = self.get_element(elem_ref.sequence_id, elem_ref.element_index) else {
                debug_assert!(false, "actor_in_progress contains stale element ref");
                continue;
            };
            if predicate(elem) {
                return Some((elem_ref.sequence_id, elem_ref.element_index));
            }
        }
        None
    }

    /// Returns true when `actor` owns a not-yet-terminal sequence element
    /// whose command matches `predicate`.
    pub fn has_live_element_for_actor_matching(
        &self,
        actor: impl Into<EntityId>,
        mut predicate: impl FnMut(Command) -> bool,
    ) -> bool {
        self.live_element_for_actor_matching(actor, |elem| predicate(elem.command))
            .is_some()
    }

    pub fn live_element_for_actor_matching(
        &self,
        actor: impl Into<EntityId>,
        mut predicate: impl FnMut(&SequenceElement) -> bool,
    ) -> Option<(SequenceId, usize)> {
        let actor = actor.into();
        let set = self.actor_live.get(&actor)?;
        for elem_ref in set {
            let Some(elem) = self.get_element(elem_ref.sequence_id, elem_ref.element_index) else {
                debug_assert!(false, "actor_live contains stale element ref");
                continue;
            };
            if predicate(elem) {
                return Some((elem_ref.sequence_id, elem_ref.element_index));
            }
        }
        None
    }

    /// Returns true when `actor` owns a Todo or InProgress element
    /// whose command matches `predicate`.  Unlike
    /// [`Self::has_live_element_for_actor_matching`], this deliberately
    /// ignores `Postponed` elements: `EvaluateSwordfight` gates on the
    /// actor's current animation, so a queued/postponed wait-priority
    /// smalltalk element must not suppress fresh smalltalk forever.
    pub fn has_unpostponed_element_for_actor_matching(
        &self,
        actor: impl Into<EntityId>,
        mut predicate: impl FnMut(Command) -> bool,
    ) -> bool {
        let actor = actor.into();
        let Some(set) = self.actor_live.get(&actor) else {
            return false;
        };
        set.iter().any(|elem_ref| {
            let Some(elem) = self.get_element(elem_ref.sequence_id, elem_ref.element_index) else {
                debug_assert!(false, "actor_live contains stale element ref");
                return false;
            };
            matches!(elem.state, SequenceState::Todo | SequenceState::InProgress)
                && predicate(elem.command)
        })
    }

    /// Returns true when `actor` owns a Todo or InProgress element
    /// whose full element data matches `predicate`.
    pub fn has_unpostponed_element_for_actor_matching_element(
        &self,
        actor: impl Into<EntityId>,
        mut predicate: impl FnMut(&SequenceElement) -> bool,
    ) -> bool {
        self.live_element_for_actor_matching(actor, |elem| {
            matches!(elem.state, SequenceState::Todo | SequenceState::InProgress) && predicate(elem)
        })
        .is_some()
    }

    /// Peek the actor's current in-progress order — the `Order` at the
    /// front of the owning `SequenceElement`'s `orders` queue.
    pub fn current_order_for_actor<I: Into<EntityId>>(
        &self,
        actor: I,
    ) -> Option<(SequenceId, usize, &Order)> {
        let (seq_id, elem_idx) = self.current_element_for_actor(actor)?;
        let order = self.get_element(seq_id, elem_idx)?.current_order()?;
        Some((seq_id, elem_idx, order))
    }

    // ─── Element dispatch registration ──────────────────────────

    /// Run `RHSequence::NextSequenceElementsGo`'s per-element registration
    /// loop (`original-code/RHsequence.cpp:272-288`).
    ///
    /// The loop body is *not* a batch: `RHPRIORITY_WAIT` elements call `Go()`
    /// and `RHSequenceManager::RegisterSequenceElementToGo` executes the
    /// `ExecutedImmediately()` command group inline
    /// (`original-code/RHsequencemanager.cpp:967-978`,
    /// `RHsequenceelement.cpp:916-958`), all before the next sibling is even
    /// looked at.  A composite such as `[LockAi(a), Turn(b), Turn(a)]` relies
    /// on that: `LockAi` -> `ScriptLockAI` -> `RHElementActor::Stop` ->
    /// `StopNotYetLaunchedSequenceElements` only walks
    /// `mlistSequenceElementsToGo`, which does not yet contain `Turn(a)`.
    ///
    /// Rust dispatches those commands from the engine, so registration stops
    /// as soon as one element owes synchronous work and the remaining loop
    /// iterations are parked directly behind that work as
    /// [`PendingSyncEntry::Register`] entries.  Elements whose registration
    /// only appends to `elements_to_go` have no observable effect on their
    /// siblings, so those iterations still run straight through.
    fn register_level_elements_to_go(&mut self, seq_id: SequenceId, to_go: Vec<usize>) {
        let mut remaining = to_go.into_iter();
        while let Some(elem_idx) = remaining.next() {
            let before = self.pending_synchronous_actions.len();
            self.register_one_element_to_go(seq_id, elem_idx);
            if self.pending_synchronous_actions.len() > before {
                for deferred in remaining {
                    self.pending_synchronous_actions
                        .push_back(PendingSyncEntry::Register {
                            sequence_id: seq_id,
                            element_index: deferred,
                        });
                }
                return;
            }
        }
    }

    /// One iteration of that loop: the `RHSEQ_INTERRUPTED` guard plus the
    /// `GetPriority() == RHPRIORITY_WAIT` split of
    /// `original-code/RHsequence.cpp:277-286`, both read at iteration time.
    fn register_one_element_to_go(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let Some(element) = self.get_element(seq_id, elem_idx) else {
            tracing::trace!(
                ?seq_id,
                elem_idx,
                "next_elements_go: element disappeared before its registration ran"
            );
            return;
        };
        if element.state == SequenceState::Interrupted {
            return;
        }
        if element.priority == SequencePriority::Wait {
            self.register_wait_element_to_go(seq_id, elem_idx);
        } else {
            self.register_element_to_go(seq_id, elem_idx);
        }
    }

    /// Resume the parked `NextSequenceElementsGo` iterations that sit at the
    /// front of the synchronous buffer, stopping again as soon as one of them
    /// owes inline work.
    fn settle_leading_registrations(&mut self) {
        while self
            .pending_synchronous_actions
            .front()
            .is_some_and(PendingSyncEntry::is_register)
        {
            self.perform_registration_at(0);
        }
    }

    /// Run the parked loop iteration at `index`, splicing whatever it
    /// registers into exactly that position so ordering is preserved.
    fn perform_registration_at(&mut self, index: usize) {
        let entry = self
            .pending_synchronous_actions
            .remove(index)
            .expect("perform_registration_at index out of range");
        let PendingSyncEntry::Register {
            sequence_id,
            element_index,
        } = entry
        else {
            panic!("perform_registration_at called on a dispatchable action: {entry:?}");
        };
        let tail = self.pending_synchronous_actions.split_off(index);
        self.register_one_element_to_go(sequence_id, element_index);
        self.pending_synchronous_actions.extend(tail);
    }

    /// Register an element for deferred dispatch.
    ///
    /// If the element's command is in the `executed_immediately()`
    /// group, the element is *not* queued — instead, the corresponding
    /// `SequenceAction` is pushed onto
    /// [`pending_synchronous_actions`](Self::pending_synchronous_actions)
    /// for synchronous engine-side dispatch.  Non-immediate elements
    /// land on `elements_to_go` for the next `hourglass` pass.
    ///
    /// Engine-side wrappers around external entry points
    /// (`launch_sequence`, `launch_element`, `element_terminated`,
    /// `element_impossible`, `element_in_progress`,
    /// `element_interrupted`, `terminate_sequence`, `stop_owner`,
    /// `stop_pending_elements*`, `cancel_pending_move_commands`)
    /// drain pending immediate actions after each call so the
    /// immediate side effect fires this same frame: registration =
    /// dispatch.  The hourglass-internal cascade callsites in
    /// [`Self::process_effects`] need no extra drain — `hourglass`
    /// itself folds the queue into the action stream it returns.
    ///
    /// Terminal-state elements are silently skipped — only `Todo` /
    /// `Postponed` elements actually dispatch.  This situation arises
    /// legitimately when a preemption cascade lands an element into
    /// Terminated before [`Sequence::next_elements_go`] iterates over
    /// it: that iterator only filters `Interrupted`, not Terminated /
    /// Impossible.
    fn register_element_to_go(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let Some(seq) = self.sequences.get(&seq_id) else {
            return;
        };
        let Some(elem) = seq.elements.get(elem_idx) else {
            return;
        };

        if matches!(
            elem.state,
            SequenceState::Terminated | SequenceState::Impossible | SequenceState::Interrupted
        ) {
            tracing::trace!(
                ?seq_id,
                elem_idx,
                state = ?elem.state,
                command = ?elem.command,
                owner = ?elem.owner,
                "register_element_to_go: skipping terminal-state element"
            );
            return;
        }

        if elem.executed_immediately() {
            // `executed_immediately()` is a pure predicate; the matching
            // `SequenceAction` is queued here for the engine-side
            // dispatcher to drain inline.
            if let Some(action) = Self::immediate_action_for(seq_id, elem_idx, elem) {
                self.pending_synchronous_actions
                    .push_back(PendingSyncEntry::Action(action));
            } else {
                tracing::error!(
                    ?seq_id,
                    elem_idx,
                    command = ?elem.command,
                    owner = ?elem.owner,
                    "register_element_to_go: executed_immediately() = true but no \
                     immediate-action mapping — terminating element"
                );
                // Fall through to `elements_to_go` so the hourglass
                // diagnostic arm logs and terminates.  The element is
                // deliberately never put on `pending_synchronous_actions`
                // because we have no action to fire.
                self.elements_to_go.push_back((seq_id, elem_idx));
            }
            return;
        }

        self.elements_to_go.push_back((seq_id, elem_idx));
    }

    /// Emit the `Go()` action for a WAIT-priority element at registration
    /// time instead of placing it behind the next manager hourglass.
    ///
    /// Original provenance: `RHSequence::NextSequenceElementsGo` calls
    /// `RHSequenceElement::Go()` directly for `RHPRIORITY_WAIT`
    /// (`original-code/RHsequence.cpp:272-288`).  Other priorities call
    /// `RHSequenceManager::RegisterSequenceElementToGo`, whose non-immediate
    /// path appends to the manager FIFO
    /// (`original-code/RHsequencemanager.cpp:951-970`).
    fn register_wait_element_to_go(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let seq = self
            .sequences
            .get(&seq_id)
            .unwrap_or_else(|| panic!("register_wait_element_to_go: missing sequence {seq_id:?}"));
        let elem = seq.elements.get(elem_idx).unwrap_or_else(|| {
            panic!("register_wait_element_to_go: missing element ({seq_id:?}, {elem_idx})")
        });

        if !matches!(elem.state, SequenceState::Todo | SequenceState::Postponed) {
            tracing::trace!(
                ?seq_id,
                elem_idx,
                state = ?elem.state,
                command = ?elem.command,
                owner = ?elem.owner,
                "register_wait_element_to_go: Go is a no-op for non-live element"
            );
            return;
        }

        // `RHSequenceElement::Go()` bypasses `ExecutedImmediately()` and
        // routes solely by owner presence (`RHsequenceelement.cpp:440-456`).
        let action = if let Some(owner) = elem.owner {
            SequenceAction::InstructOwner {
                owner,
                sequence_id: seq_id,
                element_index: elem_idx,
            }
        } else {
            SequenceAction::EngineCommand {
                sequence_id: seq_id,
                element_index: elem_idx,
            }
        };
        self.pending_synchronous_actions
            .push_back(PendingSyncEntry::Action(action));
    }

    /// Build the `SequenceAction` for an immediate-dispatch element.
    ///
    /// 3-way switch routed by command group, not by owner-presence:
    /// owner-only commands always dispatch to the owner, engine-only
    /// commands always dispatch to the engine regardless of owner,
    /// and `SendMessage` picks owner if non-null else engine.
    ///
    /// Returns `None` for owner-only commands launched without an
    /// owner — the caller logs and terminates the element so we don't
    /// silently drop the side effect.
    fn immediate_action_for(
        seq_id: SequenceId,
        elem_idx: usize,
        elem: &SequenceElement,
    ) -> Option<SequenceAction> {
        match elem.command {
            // Owner-only group: must dispatch to owner.
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
            | Command::Unblip => Some(SequenceAction::ExecuteImmediateOwner {
                owner: elem.owner?,
                sequence_id: seq_id,
                element_index: elem_idx,
            }),
            // Engine-only group: dispatch to engine regardless of owner.
            Command::LockUser
            | Command::UnlockUser
            | Command::CameraJumpTo
            | Command::Timer
            | Command::ActionAvailable
            | Command::CharacterAvailable
            | Command::OpenScroll => Some(SequenceAction::ExecuteImmediateEngine {
                sequence_id: seq_id,
                element_index: elem_idx,
            }),
            // SendMessage: owner if present, else engine.
            Command::SendMessage => Some(match elem.owner {
                Some(owner) => SequenceAction::ExecuteImmediateOwner {
                    owner,
                    sequence_id: seq_id,
                    element_index: elem_idx,
                },
                None => SequenceAction::ExecuteImmediateEngine {
                    sequence_id: seq_id,
                    element_index: elem_idx,
                },
            }),
            _ => None,
        }
    }

    /// Drain pending immediate-dispatch actions accumulated
    /// since the last call.  Engine-side wrappers around external entry
    /// points call this after invoking `launch_sequence`,
    /// `launch_element`, `element_terminated`, `element_impossible`,
    /// `element_in_progress`, `element_interrupted`,
    /// `terminate_sequence`, `stop_owner`, `stop_pending_elements*`,
    /// or `cancel_pending_move_commands` so any immediate command that
    /// was registered fires this same frame: registration = dispatch.
    ///
    /// `hourglass` already folds this queue into its returned action
    /// stream, so callers inside the hourglass dispatch loop need not
    /// drain separately.
    pub fn take_pending_immediate_actions(&mut self) -> Vec<SequenceAction> {
        let mut immediate = Vec::new();
        let mut retained = VecDeque::new();
        loop {
            self.settle_leading_registrations();
            let Some(entry) = self.pending_synchronous_actions.pop_front() else {
                break;
            };
            match entry {
                PendingSyncEntry::Action(
                    action @ (SequenceAction::ExecuteImmediateOwner { .. }
                    | SequenceAction::ExecuteImmediateEngine { .. }),
                ) => immediate.push(action),
                other => retained.push_back(other),
            }
        }
        self.pending_synchronous_actions = retained;
        immediate
    }

    /// Pop the next synchronous action without disturbing the remainder.
    /// Script-native sequence launch uses this to stop exactly at a re-entrant
    /// SendMessage callback, then continue in order before the outer VM resumes.
    pub fn pop_pending_immediate_action(&mut self) -> Option<SequenceAction> {
        self.settle_leading_registrations();
        match self.pending_synchronous_actions.pop_front()? {
            PendingSyncEntry::Action(action) => Some(action),
            entry @ PendingSyncEntry::Register { .. } => {
                panic!("settle_leading_registrations left a parked registration: {entry:?}")
            }
        }
    }

    /// Remove the first action that Original registration executes inline,
    /// while leaving ordinary manager-Hourglass work queued in order.
    ///
    /// This is used at callbacks that occur outside `SequenceManager::Hourglass`
    /// (notably director completion during Draw and anonymous-timer expiry
    /// after the manager phase). `ExecutedImmediately()` commands and
    /// RHPRIORITY_WAIT successors run inline; other priorities stay queued.
    ///
    /// Parked `NextSequenceElementsGo` iterations are never left behind: the
    /// original executes the whole loop on this same stack, so a registration
    /// still queued after the inline scan is run here and the scan repeats.
    pub fn pop_pending_registration_inline_action(&mut self) -> Option<SequenceAction> {
        loop {
            self.settle_leading_registrations();
            let parked = self
                .pending_synchronous_actions
                .iter()
                .position(PendingSyncEntry::is_register);
            let scan_limit = parked.unwrap_or(self.pending_synchronous_actions.len());
            let inline = self
                .pending_synchronous_actions
                .iter()
                .take(scan_limit)
                .position(|entry| match entry.as_action() {
                    Some(
                        SequenceAction::ExecuteImmediateOwner { .. }
                        | SequenceAction::ExecuteImmediateEngine { .. },
                    ) => true,
                    Some(
                        SequenceAction::InstructOwner {
                            sequence_id,
                            element_index,
                            ..
                        }
                        | SequenceAction::EngineCommand {
                            sequence_id,
                            element_index,
                        },
                    ) => self
                        .get_element(*sequence_id, *element_index)
                        .is_some_and(|element| element.priority == SequencePriority::Wait),
                    None => false,
                });
            if let Some(index) = inline {
                return match self.pending_synchronous_actions.remove(index)? {
                    PendingSyncEntry::Action(action) => Some(action),
                    entry @ PendingSyncEntry::Register { .. } => {
                        panic!("inline scan selected a parked registration: {entry:?}")
                    }
                };
            }
            let Some(index) = parked else {
                return None;
            };
            self.perform_registration_at(index);
        }
    }

    pub fn next_pending_immediate_action(&mut self) -> Option<&SequenceAction> {
        self.settle_leading_registrations();
        self.pending_synchronous_actions
            .front()
            .and_then(PendingSyncEntry::as_action)
    }

    /// Drain the complete ordered stream emitted synchronously by sequence
    /// registration: direct WAIT `Go()` actions interleaved with
    /// `ExecutedImmediately()` actions.
    ///
    /// The engine action loop uses this after every callback.  If that
    /// callback completes an element and `Ready()` advances to a WAIT
    /// successor, the successor is inserted at the front of the remaining
    /// work before an older sibling action runs, matching the original
    /// re-entrant call stack.
    /// Parked [`PendingSyncEntry::Register`] iterations travel with the
    /// continuation: the original's `NextSequenceElementsGo` loop is a stack
    /// frame below the callback, so it resumes only once the callback returns.
    pub fn take_pending_synchronous_actions(&mut self) -> Vec<PendingSyncEntry> {
        self.pending_synchronous_actions.drain(..).collect()
    }

    /// Drain only the settled head of the synchronous buffer, leaving any
    /// parked `NextSequenceElementsGo` iteration queued.
    ///
    /// The manager-hourglass action loop uses this: it must dispatch the
    /// action that a registration produced before the loop's next iteration
    /// registers the following sibling.
    pub fn take_settled_synchronous_actions(&mut self) -> Vec<SequenceAction> {
        let mut actions = Vec::new();
        self.settle_leading_registrations();
        while let Some(PendingSyncEntry::Action(_)) = self.pending_synchronous_actions.front() {
            let Some(PendingSyncEntry::Action(action)) =
                self.pending_synchronous_actions.pop_front()
            else {
                unreachable!("front was just observed to be an action");
            };
            actions.push(action);
        }
        actions
    }

    /// Restore a parent callback's detached synchronous continuation after a
    /// nested callback has fully returned. Any actions still produced by the
    /// child stay in front, matching the Original's recursive call stack.
    pub fn restore_pending_synchronous_actions(&mut self, continuation: Vec<PendingSyncEntry>) {
        self.pending_synchronous_actions.extend(continuation);
    }

    /// Append owner/engine instruction actions to the deferred manager FIFO.
    ///
    /// This is used when a synchronous `Go()` registration was created while
    /// an older instruction for the same owner was already waiting in
    /// `elements_to_go`. Keeping the newer action in the synchronous queue
    /// would let it jump ahead of that older registration; appending its
    /// element identity here preserves registration order without moving any
    /// unrelated manager entries.
    pub fn append_actions_to_deferred_fifo(&mut self, actions: Vec<SequenceAction>) {
        for action in actions {
            let target = match action {
                SequenceAction::InstructOwner {
                    sequence_id,
                    element_index,
                    ..
                }
                | SequenceAction::EngineCommand {
                    sequence_id,
                    element_index,
                } => (sequence_id, element_index),
                immediate => panic!(
                    "cannot defer immediate sequence action behind manager FIFO: {immediate:?}"
                ),
            };
            self.elements_to_go.push_back(target);
        }
    }

    /// `true` iff there is at least one immediate-dispatch action awaiting
    /// drain, ignoring direct WAIT `Go()` actions in the same stream.
    pub fn has_pending_immediate_actions(&self) -> bool {
        self.pending_synchronous_actions.iter().any(|entry| {
            matches!(
                entry.as_action(),
                Some(
                    SequenceAction::ExecuteImmediateOwner { .. }
                        | SequenceAction::ExecuteImmediateEngine { .. }
                )
            )
        })
    }

    // ─── Per-frame processing ───────────────────────────────────

    /// Process all pending sequence elements for this frame.
    /// Returns actions the engine must dispatch.
    ///
    /// Drains both the deferred `elements_to_go` queue and the
    /// synchronous registration buffer (populated by
    /// [`Self::register_element_to_go`] and
    /// [`Self::register_wait_element_to_go`]). Cascade callsites in
    /// [`Self::process_effects`] re-register elements during the loop —
    /// any new synchronous actions land on that buffer and are
    /// drained here this same frame.
    pub fn hourglass(&mut self) -> Vec<SequenceAction> {
        let mut actions = Vec::new();
        while let Some(action) = self.pop_next_hourglass_action() {
            actions.push(action);
        }
        actions
    }

    /// Pop exactly one action from the live manager FIFO. Original
    /// `RHSequenceManager::Hourglass` removes one element, calls `Go()`, and
    /// only then loops. Keeping the remaining elements registered makes them
    /// observable to callbacks through `SequenceElementIsAboutToBeLaunched`.
    pub(crate) fn pop_next_hourglass_action(&mut self) -> Option<SequenceAction> {
        // WAIT Go() and ExecutedImmediately() run at registration, before
        // deferred non-WAIT work reaches the manager hourglass.
        self.settle_leading_registrations();
        match self.pending_synchronous_actions.pop_front() {
            Some(PendingSyncEntry::Action(action)) => Some(action),
            Some(entry @ PendingSyncEntry::Register { .. }) => {
                panic!("settle_leading_registrations left a parked registration: {entry:?}")
            }
            None => self.pop_deferred_hourglass_action(),
        }
    }

    fn pop_deferred_hourglass_action(&mut self) -> Option<SequenceAction> {
        loop {
            let (seq_id, elem_idx) = self.elements_to_go.pop_front()?;
            // Validate the sequence still exists
            let Some(seq) = self.sequences.get(&seq_id) else {
                continue;
            };
            if elem_idx >= seq.elements.len() {
                continue;
            }

            let elem = &seq.elements[elem_idx];

            // Only process elements that are still Todo or Postponed
            match elem.state {
                SequenceState::Todo | SequenceState::Postponed => {}
                _ => continue,
            }

            // The `register_element_to_go` path routes immediate
            // commands directly to `pending_synchronous_actions`, so
            // anything coming out of `elements_to_go` should normally
            // be non-immediate. WAIT-priority elements also bypass this
            // queue via `pending_synchronous_actions`.
            if elem.executed_immediately() {
                if let Some(action) = Self::immediate_action_for(seq_id, elem_idx, elem) {
                    return Some(action);
                } else {
                    tracing::warn!(
                        ?seq_id,
                        elem_idx,
                        command = ?elem.command,
                        owner = ?elem.owner,
                        "owner-only immediate command has no owner — terminating"
                    );
                    self.element_terminated(seq_id, elem_idx);
                }
            } else if let Some(owner) = elem.owner {
                return Some(SequenceAction::InstructOwner {
                    owner,
                    sequence_id: seq_id,
                    element_index: elem_idx,
                });
            } else {
                return Some(SequenceAction::EngineCommand {
                    sequence_id: seq_id,
                    element_index: elem_idx,
                });
            }
        }
    }

    /// Drain normal-priority work registered while an engine-side Hourglass
    /// action was executing. Original appends this work to the live manager
    /// FIFO, after actions that were already waiting.
    pub fn take_pending_deferred_actions(&mut self) -> Vec<SequenceAction> {
        let mut actions = Vec::new();
        while let Some(action) = self.pop_deferred_hourglass_action() {
            actions.push(action);
        }
        actions
    }

    /// Promote one exact deferred element produced inside a synchronous
    /// native boundary. No other same-owner work is inspected or reordered.
    pub fn take_deferred_owner_action(
        &mut self,
        owner: EntityId,
        sequence_id: SequenceId,
        element_index: usize,
    ) -> Result<Option<SequenceAction>, String> {
        let element = self
            .get_element(sequence_id, element_index)
            .ok_or_else(|| format!("missing deferred element {sequence_id:?}/{element_index}"))?;
        if element.owner != Some(owner) {
            return Err(format!(
                "deferred element {sequence_id:?}/{element_index} belongs to {:?}, expected {owner:?}",
                element.owner
            ));
        }
        if !matches!(
            element.state,
            SequenceState::Todo | SequenceState::Postponed
        ) {
            if let Some(position) = self
                .elements_to_go
                .iter()
                .position(|handle| *handle == (sequence_id, element_index))
            {
                self.elements_to_go.remove(position);
            }
            return Ok(None);
        }
        if element.executed_immediately() {
            return Err(format!(
                "deferred element {sequence_id:?}/{element_index} unexpectedly executes immediately"
            ));
        }
        let position = self
            .elements_to_go
            .iter()
            .position(|handle| *handle == (sequence_id, element_index))
            .ok_or_else(|| {
                format!(
                    "live deferred element {sequence_id:?}/{element_index} is absent from elements_to_go"
                )
            })?;
        self.elements_to_go.remove(position);
        Ok(Some(SequenceAction::InstructOwner {
            owner,
            sequence_id,
            element_index,
        }))
    }

    /// Detach one exact live element from the ordinary manager FIFO without
    /// changing any of its instruction-time state.
    ///
    /// `RHElementActorHuman::Instruct` uses this shape for repeated PC bow
    /// shots: the sequence element has already been registered and reached
    /// the human, but is retained in `mShootList` before Actor::Instruct can
    /// resolve priority, stamp transition state, or translate the command.
    #[cfg(test)]
    pub(crate) fn hold_deferred_element(&mut self, sequence_id: SequenceId, element_index: usize) {
        let element = self
            .get_element(sequence_id, element_index)
            .unwrap_or_else(|| panic!("missing held element {sequence_id:?}/{element_index}"));
        assert_eq!(
            element.state,
            SequenceState::Todo,
            "held element {sequence_id:?}/{element_index} must still be Todo"
        );
        let position = self
            .elements_to_go
            .iter()
            .position(|handle| *handle == (sequence_id, element_index))
            .unwrap_or_else(|| {
                panic!("held element {sequence_id:?}/{element_index} is absent from elements_to_go")
            });
        self.elements_to_go.remove(position);
    }

    /// Remove and return this owner's deferred actions through an exact
    /// target, preserving their relative manager-FIFO order.
    ///
    /// `SetState(TERMINATED)` registers the finishing sequence's newly-ready
    /// elements before `StartPostponedSequenceElement` registers a released
    /// cross-sequence successor.  A synchronous owner boundary must therefore
    /// not pluck that successor out of the middle of `elements_to_go`: doing
    /// so lets the old sequence run after the replacement and interrupt it.
    /// Foreign-owner entries remain in place.
    pub fn take_deferred_owner_actions_through(
        &mut self,
        owner: EntityId,
        target_sequence_id: SequenceId,
        target_element_index: usize,
    ) -> Result<Vec<SequenceAction>, String> {
        let target = (target_sequence_id, target_element_index);
        let target_element = self
            .get_element(target_sequence_id, target_element_index)
            .ok_or_else(|| {
                format!("missing deferred target {target_sequence_id:?}/{target_element_index}")
            })?;
        if target_element.owner != Some(owner) {
            return Err(format!(
                "deferred target {target_sequence_id:?}/{target_element_index} belongs to {:?}, expected {owner:?}",
                target_element.owner
            ));
        }
        if !matches!(
            target_element.state,
            SequenceState::Todo | SequenceState::Postponed
        ) {
            if let Some(position) = self
                .elements_to_go
                .iter()
                .position(|handle| *handle == target)
            {
                self.elements_to_go.remove(position);
            }
            return Ok(Vec::new());
        }
        if target_element.executed_immediately() {
            return Err(format!(
                "deferred target {target_sequence_id:?}/{target_element_index} unexpectedly executes immediately"
            ));
        }
        let target_position = self
            .elements_to_go
            .iter()
            .position(|handle| *handle == target)
            .ok_or_else(|| {
                format!(
                    "live deferred target {target_sequence_id:?}/{target_element_index} is absent from elements_to_go"
                )
            })?;

        let handles: Vec<_> = self
            .elements_to_go
            .iter()
            .take(target_position + 1)
            .copied()
            .filter(|(sequence_id, element_index)| {
                self.get_element(*sequence_id, *element_index)
                    .is_some_and(|element| element.owner == Some(owner))
            })
            .collect();

        if !handles.contains(&target) {
            return Err(format!(
                "deferred target {target_sequence_id:?}/{target_element_index} does not belong to {owner:?}"
            ));
        }

        let mut actions = Vec::with_capacity(handles.len());
        for (sequence_id, element_index) in handles {
            if let Some(action) =
                self.take_deferred_owner_action(owner, sequence_id, element_index)?
            {
                actions.push(action);
            }
        }
        Ok(actions)
    }

    // ─── State change callbacks ─────────────────────────────────

    /// Called by the engine when an element has finished (terminated).
    /// Advances the sequence to the next command level if all elements at
    /// the current level are done.
    #[track_caller]
    pub fn element_terminated(&mut self, seq_id: SequenceId, elem_idx: usize) {
        tracing::trace!(
            target: "parity_terminate_caller",
            ?seq_id,
            elem_idx,
            caller = %std::panic::Location::caller(),
            "element_terminated"
        );
        let Some(seq) = self.sequences.get_mut(&seq_id) else {
            return;
        };

        let effects = seq.set_element_state(
            elem_idx,
            SequenceState::Terminated,
            CascadeFlags::NEXT_LEVEL,
        );

        self.process_effects(seq_id, effects, "element_terminated");
    }

    /// Called when an element becomes impossible.
    ///
    /// Sequence elements marked `SequencePriority::NonInterruptable`
    /// must run to completion and can't be downgraded to `Impossible`
    /// by external events. When something tries, the call is logged
    /// and treated as a no-op so the element stays `InProgress` and
    /// finishes normally.
    pub fn element_impossible(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let Some(seq) = self.sequences.get_mut(&seq_id) else {
            return;
        };

        // Priority guard: non-interruptable elements ignore "impossible"
        // downgrades from outside their natural completion path.
        let elem = seq
            .elements
            .get(elem_idx)
            .unwrap_or_else(|| panic!("missing impossible element {seq_id:?}/{elem_idx}"));
        let blocked =
            elem.state == SequenceState::InProgress && elem.priority.is_non_interruptable();
        if blocked {
            tracing::debug!(
                ?seq_id,
                elem_idx,
                "element_impossible: blocked by NonInterruptable priority — keeping element in progress"
            );
            return;
        }

        let effects = seq.set_element_state(
            elem_idx,
            SequenceState::Impossible,
            CascadeFlags::NEXT_LEVEL,
        );

        self.process_effects(seq_id, effects, "element_impossible");
    }

    /// Apply the `RHMOTION_ABORTED` result returned by an actor's own
    /// `Execute` call.
    ///
    /// This is distinct from an external attempt to invalidate an active
    /// element. `RHElementActor::Hourglass` asserts in debug builds that its
    /// retained element is not non-interruptable, but release builds still
    /// call `SetState(RHSEQ_IMPOSSIBLE)` after an intrinsic Execute abort.
    /// Preserve that release behavior for malformed/sentinel orders authored
    /// by Original itself.
    pub fn element_impossible_from_execute(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let Some(seq) = self.sequences.get_mut(&seq_id) else {
            return;
        };

        let effects = seq.set_element_state(
            elem_idx,
            SequenceState::Impossible,
            CascadeFlags::NEXT_LEVEL,
        );

        self.process_effects(seq_id, effects, "element_impossible_from_execute");
    }

    /// Set the priority of a specific sequence element.
    ///
    /// Used by the falling-pushed / rolling / ladder-wall / landing
    /// dispatch paths to mark the active element `NonInterruptable`
    /// so the termination guard in `element_impossible` refuses to
    /// cut it short.
    pub fn set_element_priority(
        &mut self,
        seq_id: SequenceId,
        elem_idx: usize,
        priority: SequencePriority,
    ) {
        if let Some(seq) = self.sequences.get_mut(&seq_id)
            && let Some(elem) = seq.elements.get_mut(elem_idx)
        {
            let old_priority = elem.priority;
            let owner = elem.owner;
            let is_live = Self::is_actor_live_state(elem.state);
            elem.priority = priority;
            if is_live
                && old_priority != priority
                && let Some(owner) = owner
            {
                self.invalidate_postpone_tail_cache_for(owner);
                if priority > old_priority {
                    if let Some(summary) = self.actor_stop_summaries.get_mut(&owner) {
                        summary.weakest_priority = summary.weakest_priority.max(priority);
                    }
                } else if self
                    .actor_stop_summaries
                    .get(&owner)
                    .is_some_and(|summary| summary.weakest_priority == old_priority)
                {
                    self.actor_stop_summaries.remove(&owner);
                }
            }
        }
    }

    /// Called when an element starts executing (enters InProgress).
    pub fn element_in_progress(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let Some(seq) = self.sequences.get_mut(&seq_id) else {
            return;
        };

        let effects = seq.set_element_state(
            elem_idx,
            SequenceState::InProgress,
            CascadeFlags::NEXT_LEVEL,
        );

        self.process_effects(seq_id, effects, "element_in_progress");
    }

    /// Called when an element is interrupted.
    pub fn element_interrupted(
        &mut self,
        seq_id: SequenceId,
        elem_idx: usize,
        flags: CascadeFlags,
    ) {
        let Some(seq) = self.sequences.get_mut(&seq_id) else {
            return;
        };

        let effects = seq.set_element_state(elem_idx, SequenceState::Interrupted, flags);

        self.process_effects(seq_id, effects, "element_interrupted");
    }

    /// Interrupt an element after its actor has already selected an incoming
    /// replacement.
    ///
    /// Original `RHElementActor::Instruct` writes
    /// `mpSequenceElement = pNewSequenceElement` before it interrupts the old
    /// element.  Consequently the old element's synchronous
    /// `SendCondolationCard` observes that it is no longer selected.  Rust's
    /// incoming element is still `Todo` at this borrow-safe boundary, so the
    /// actor-in-progress index alone would incorrectly mark the old card as
    /// selected.
    pub fn element_interrupted_after_replacement_selected(
        &mut self,
        seq_id: SequenceId,
        elem_idx: usize,
        flags: CascadeFlags,
    ) {
        let pending_before = self.pending_condolations.len();
        self.element_interrupted(seq_id, elem_idx, flags);
        let dispatch = self
            .pending_condolations
            .get_mut(pending_before)
            .expect("replacement interruption must queue its condolence card");
        assert_eq!(dispatch.card.seq_id, seq_id);
        assert_eq!(usize::from(dispatch.card.elem_idx), elem_idx);
        dispatch.card.was_selected = false;
    }

    /// Hard-interrupt every live sequence element owned by `actor`, except
    /// those in `exempt_seq` and dead-admissible cards already waiting in the
    /// FIFO.
    ///
    /// Used on death: the graceful `stop_owner` path rewrites an
    /// in-progress movement order to a `TransitionWalking*Waiting*` stop
    /// animation and lets the element keep playing — which is correct
    /// for a live halt but produces a "corpse walks a few more frames"
    /// visual for a dead actor.  Death needs to throw every surviving
    /// sequence away cleanly. Original `Human::Kill` does not purge its
    /// sequence queue, and dead `Human::Instruct` still admits the five
    /// ordinary Receive*Damage commands, Wait, and GetKilledAtBottom. Preserve
    /// those `Todo` cards so simultaneous hits execute in FIFO order after the
    /// lethal hit; the active damage sequence survives via `exempt_seq` so its
    /// dying order becomes the actor's current order.
    ///
    /// Our arbitration doesn't run on state changes, so we do the
    /// cleanup explicitly here.
    pub fn kill_owner_sequences(&mut self, actor: EntityId, exempt_seq: SequenceId) {
        let mut targets: Vec<(SequenceId, usize)> = Vec::new();
        for (seq_id, seq) in &self.sequences {
            if *seq_id == exempt_seq {
                continue;
            }
            for (elem_idx, elem) in seq.elements.iter().enumerate() {
                if elem.owner != Some(actor) {
                    continue;
                }
                let pending_command_admitted_while_dead =
                    matches!(elem.state, SequenceState::Todo | SequenceState::Postponed)
                        && matches!(
                            elem.command,
                            Command::ReceiveHitDamage
                                | Command::ReceiveSwordDamage
                                | Command::ReceiveArrowDamage
                                | Command::ReceiveDamage
                                | Command::ReceiveMobileDamage
                                | Command::Wait
                                | Command::GetKilledAtBottom
                        );
                if pending_command_admitted_while_dead {
                    continue;
                }
                if matches!(
                    elem.state,
                    SequenceState::InProgress | SequenceState::Postponed | SequenceState::Todo
                ) {
                    targets.push((*seq_id, elem_idx));
                }
            }
        }
        for (seq_id, elem_idx) in targets {
            let Some(seq) = self.sequences.get_mut(&seq_id) else {
                continue;
            };
            let effects = seq.set_element_state(
                elem_idx,
                SequenceState::Interrupted,
                CascadeFlags::NEXT_LEVEL,
            );
            self.process_effects(seq_id, effects, "kill_owner_sequences");
        }
    }

    /// Flip an element to `Postponed` via the normal state-change
    /// pipeline.  Used by the Instruct arbitration path.  The common
    /// `set_element_state` prologue still runs (so the in-progress
    /// counter decrements when the waiter was InProgress), while the
    /// `Postponed` case body itself does nothing extra — no cascade,
    /// no signal_ready, no condolation.  `CascadeFlags::empty()`
    /// reflects that, and `process_effects` keeps `actor_in_progress`
    /// / `elements_in_progress` consistent on the InProgress→Postponed
    /// transition.  The element's `cross_postponed` / `postponed_by`
    /// links are set separately by the caller before this call.
    pub fn postpone_element(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let Some(seq) = self.sequences.get_mut(&seq_id) else {
            return;
        };
        let effects =
            seq.set_element_state(elem_idx, SequenceState::Postponed, CascadeFlags::empty());
        self.process_effects(seq_id, effects, "postpone_element");

        // Original calls Postpone from inside the element's Instruct/Go
        // boundary, after SequenceManager::Hourglass has already removed
        // that element from its launch FIFO. Rust also arbitrates owned
        // launches synchronously, while their initial manager registration
        // is still queued. Consume that registration here: otherwise the
        // manager instructs the same postponed element again next frame and
        // can attach it behind itself, creating a recursive self-cycle.
        let target = (seq_id, elem_idx);
        self.elements_to_go.retain(|entry| *entry != target);
        self.pending_synchronous_actions.retain(|entry| {
            !matches!(
                entry.as_action(),
                Some(SequenceAction::InstructOwner {
                    sequence_id,
                    element_index,
                    ..
                }) if (*sequence_id, *element_index) == target
            )
        });
    }

    /// Whether the front order on the given element can be interrupted
    /// right now.
    ///
    /// Original's `RHSequenceElement::CanInterruptNow` asserts that a current
    /// order exists and then unconditionally returns true. `RHOrder::bLockAI`
    /// is serialized but is never consulted by sequence arbitration. Keep the
    /// field for save compatibility without inventing gameplay semantics for
    /// it here.
    pub fn can_interrupt_now(&self, seq_id: SequenceId, elem_idx: usize) -> bool {
        let elem = self.get_element(seq_id, elem_idx).unwrap_or_else(|| {
            panic!("can_interrupt_now called for missing sequence element {seq_id:?}/{elem_idx}")
        });
        assert!(
            elem.orders.front().is_some(),
            "can_interrupt_now requires a current order on {seq_id:?}/{elem_idx}"
        );
        true
    }

    fn invalidate_postpone_tail_cache_for(&mut self, owner: EntityId) {
        self.postpone_tail_cache.remove(&owner);
    }

    /// Return the first blocker at which priority arbitration is not the pure
    /// `Postpone` tail-call arm. If every existing successor chooses
    /// `Postpone`, this is the chain tail and the result is cached.
    pub(crate) fn postpone_append_point(
        &mut self,
        root: (SequenceId, usize),
        waiter_priority: SequencePriority,
    ) -> ((SequenceId, usize), usize, bool) {
        let root_ref = SequenceElementRef::new(root.0, root.1);
        let owner = self
            .get_element(root.0, root.1)
            .and_then(|element| element.owner)
            .unwrap_or_else(|| {
                panic!(
                    "postpone chain root {:?}/{} is missing or ownerless",
                    root.0, root.1
                )
            });
        if let Some(&summary) = self
            .postpone_tail_cache
            .get(&owner)
            .and_then(|cache| cache.get(&(root_ref, waiter_priority)))
        {
            let tail = summary.tail;
            let tail_element = self
                .get_element(tail.sequence_id, tail.element_index)
                .unwrap_or_else(|| {
                    panic!(
                        "stale postpone-tail cache references missing {:?}/{}",
                        tail.sequence_id, tail.element_index
                    )
                });
            assert_eq!(
                tail_element.owner,
                Some(owner),
                "postpone-tail cache crosses owners"
            );
            assert!(
                tail_element.cross_postponed.is_none(),
                "postpone-tail cache was not invalidated before {:?}/{} changed",
                tail.sequence_id,
                tail.element_index
            );
            return ((tail.sequence_id, tail.element_index), summary.hops, true);
        }

        let mut current = root;
        let mut hops = 0;
        let mut weakest_priority = SequencePriority::NonInterruptable;
        let mut cross_only = true;
        let mut visited = HashSet::new();
        loop {
            assert!(
                visited.insert(current),
                "cross-postponed cycle while locating append point at {:?}/{}",
                current.0,
                current.1
            );
            let element = self.get_element(current.0, current.1).unwrap_or_else(|| {
                panic!(
                    "cross-postponed chain references missing {:?}/{}",
                    current.0, current.1
                )
            });
            assert_eq!(
                element.owner,
                Some(owner),
                "cross-postponed chain crosses owners at {:?}/{}",
                current.0,
                current.1
            );
            weakest_priority = weakest_priority.max(element.priority);
            cross_only &= self
                .get_sequence(current.0)
                .and_then(|sequence| sequence.following_element_index(current.1))
                .is_none()
                && element.postponed_element_index.is_none();
            let Some(next) = element.cross_postponed else {
                self.postpone_tail_cache.entry(owner).or_default().insert(
                    (root_ref, waiter_priority),
                    PostponeTailSummary {
                        tail: SequenceElementRef::new(current.0, current.1),
                        hops,
                        weakest_priority,
                        cross_only,
                    },
                );
                return (current, hops, true);
            };
            let existing_priority = self
                .get_element(next.0, next.1)
                .unwrap_or_else(|| {
                    panic!(
                        "cross-postponed chain references missing {:?}/{}",
                        next.0, next.1
                    )
                })
                .priority;
            if decide_priorities(existing_priority, waiter_priority) != PriorityDecision::Postpone {
                return (current, hops, false);
            }
            current = next;
            hops += 1;
        }
    }

    /// Install an append discovered by [`Self::postpone_append_point`] and
    /// advance that exact root/priority cache entry to the new tail.
    pub(crate) fn install_cached_postpone_append(
        &mut self,
        root: (SequenceId, usize),
        waiter_priority: SequencePriority,
        blocker: (SequenceId, usize),
        waiter: (SequenceId, usize),
        prior_hops: usize,
    ) {
        let owner = self
            .get_element(blocker.0, blocker.1)
            .and_then(|element| element.owner)
            .expect("postpone append blocker is missing or ownerless");
        let waiter_owner = self
            .get_element(waiter.0, waiter.1)
            .and_then(|element| element.owner)
            .expect("postpone append waiter is missing or ownerless");
        assert_eq!(owner, waiter_owner, "postpone append crosses owners");
        let prior_summary = *self
            .postpone_tail_cache
            .get(&owner)
            .and_then(|cache| {
                cache.get(&(SequenceElementRef::new(root.0, root.1), waiter_priority))
            })
            .expect("cacheable postpone append lost its root summary");
        assert_eq!(
            prior_summary.tail,
            SequenceElementRef::new(blocker.0, blocker.1),
            "cacheable postpone append blocker is not the cached tail"
        );
        assert_eq!(prior_summary.hops, prior_hops);
        let waiter_cross_only = self
            .get_sequence(waiter.0)
            .and_then(|sequence| sequence.following_element_index(waiter.1))
            .is_none()
            && self
                .get_element(waiter.0, waiter.1)
                .is_some_and(|element| element.postponed_element_index.is_none());
        self.invalidate_postpone_tail_cache_for(owner);
        let blocker_element = self
            .get_element_mut(blocker.0, blocker.1)
            .expect("postpone append blocker disappeared");
        assert!(
            blocker_element.cross_postponed.is_none(),
            "postpone append point already has a successor"
        );
        blocker_element.cross_postponed = Some(waiter);
        if decide_priorities(waiter_priority, waiter_priority) == PriorityDecision::Postpone {
            self.postpone_tail_cache.entry(owner).or_default().insert(
                (SequenceElementRef::new(root.0, root.1), waiter_priority),
                PostponeTailSummary {
                    tail: SequenceElementRef::new(waiter.0, waiter.1),
                    hops: prior_hops + 1,
                    weakest_priority: prior_summary.weakest_priority.max(waiter_priority),
                    cross_only: prior_summary.cross_only && waiter_cross_only,
                },
            );
        }
    }

    fn selected_cross_chain_all_stronger(
        &self,
        root: (SequenceId, usize),
        stop_priority: SequencePriority,
    ) -> bool {
        let Some(owner) = self
            .get_element(root.0, root.1)
            .and_then(|element| element.owner)
        else {
            return false;
        };
        let root_ref = SequenceElementRef::new(root.0, root.1);
        let Some(summary) = self.postpone_tail_cache.get(&owner).and_then(|cache| {
            cache.iter().find_map(|((cached_root, _), summary)| {
                (*cached_root == root_ref).then_some(*summary)
            })
        }) else {
            return false;
        };
        let tail = self
            .get_element(summary.tail.sequence_id, summary.tail.element_index)
            .unwrap_or_else(|| {
                panic!(
                    "selected Stop cache references missing tail {:?}/{}",
                    summary.tail.sequence_id, summary.tail.element_index
                )
            });
        assert!(
            tail.cross_postponed.is_none(),
            "selected Stop cache was not invalidated before its tail changed"
        );
        summary.cross_only && summary.weakest_priority < stop_priority
    }

    pub(crate) fn set_cross_postponed_link(
        &mut self,
        blocker: (SequenceId, usize),
        successor: Option<(SequenceId, usize)>,
    ) {
        let owner = self
            .get_element(blocker.0, blocker.1)
            .and_then(|element| element.owner)
            .expect("cross-postponed blocker is missing or ownerless");
        self.invalidate_postpone_tail_cache_for(owner);
        self.get_element_mut(blocker.0, blocker.1)
            .expect("cross-postponed blocker disappeared")
            .cross_postponed = successor;
    }

    /// Transfer a cross-sequence postponed successor from `src` onto
    /// `dst`, walking `dst`'s existing postponed chain to the tail if
    /// it already has one.
    pub fn take_over_postponed(
        &mut self,
        dst_seq: SequenceId,
        dst_idx: usize,
        src_seq: SequenceId,
        src_idx: usize,
    ) {
        let Some(src_next) = self
            .get_element(src_seq, src_idx)
            .and_then(|e| e.cross_postponed)
        else {
            return;
        };
        // Walk dst's chain to the tail (first element with no
        // cross_postponed).  At most `sequences.len()` hops — the chain
        // is acyclic by construction.
        let mut cur = (dst_seq, dst_idx);
        loop {
            let Some(e) = self.get_element(cur.0, cur.1) else {
                return;
            };
            match e.cross_postponed {
                None => break,
                Some(next) => cur = next,
            }
        }
        // Install src's successor at the tail.
        self.set_cross_postponed_link(cur, Some(src_next));
        self.set_cross_postponed_link((src_seq, src_idx), None);
    }

    /// Process effects from a state change.
    fn process_effects(
        &mut self,
        seq_id: SequenceId,
        effects: StateChangeEffects,
        terminal_site: &'static str,
    ) {
        self.process_effects_with_cross_cleanup(seq_id, effects, terminal_site, true);
    }

    /// Process a state transition while leaving inbound cross-postponed links
    /// for a caller-owned batch cleanup. `RHSequenceElement::Stop` can walk a
    /// chain containing thousands of separately allocated sequences; scanning
    /// the complete manager after every node makes that linear graph
    /// quadratic in the number of retained sequences.
    fn process_effects_deferring_cross_cleanup(
        &mut self,
        seq_id: SequenceId,
        effects: StateChangeEffects,
        terminal_site: &'static str,
    ) {
        self.process_effects_with_cross_cleanup(seq_id, effects, terminal_site, false);
    }

    fn process_effects_with_cross_cleanup(
        &mut self,
        seq_id: SequenceId,
        mut effects: StateChangeEffects,
        terminal_site: &'static str,
        clear_cross_links: bool,
    ) {
        if effects.resume_cross_postponed.is_some()
            && let Some((_, owner, _, _)) = effects.actor_live_transition
        {
            // `Sequence::set_element_state` already took this source's
            // cross-postponed pointer before returning its effects.
            self.invalidate_postpone_tail_cache_for(owner);
        }
        // RHSequenceElementMovement's override interrupts its exact linked
        // Seek before delegating to the base element's Interrupted handling.
        // Process the cross-sequence target before bookkeeping or queuing the
        // movement element's own condolence card to preserve callback order.
        if let Some(linked) = effects.interrupt_linked_seek.take() {
            let cancel_before_linked_callback = effects
                .condolation
                .as_mut()
                .and_then(|card| card.cancel_path_request_owner.take());
            let linked_element = self
                .get_element(linked.sequence_id, linked.element_index)
                .unwrap_or_else(|| {
                    panic!(
                        "loaded movement linked Seek references missing element {:?}/{}",
                        linked.sequence_id, linked.element_index
                    )
                });
            assert!(
                linked_element.data.is_movement(),
                "loaded movement linked Seek target {:?}/{} is not a movement element",
                linked.sequence_id,
                linked.element_index
            );
            let mut linked_effects = self
                .sequences
                .get_mut(&linked.sequence_id)
                .expect("linked Seek sequence disappeared")
                .set_element_state(
                    linked.element_index,
                    SequenceState::Interrupted,
                    CascadeFlags::FOLLOWING,
                );
            if let Some(cancel_owner) = cancel_before_linked_callback {
                if let Some(linked_card) = linked_effects.condolation.as_mut() {
                    assert!(
                        linked_card.cancel_path_request_owner.is_none()
                            || linked_card.cancel_path_request_owner == Some(cancel_owner),
                        "linked movement cancellation crosses actors"
                    );
                    linked_card.cancel_path_request_owner = Some(cancel_owner);
                } else if let Some(card) = effects.condolation.as_mut() {
                    // An already-terminal linked target has no callback. Keep
                    // cancellation on the source movement's own card.
                    card.cancel_path_request_owner = Some(cancel_owner);
                }
            }
            self.process_effects_with_cross_cleanup(
                linked.sequence_id,
                linked_effects,
                "linked_movement_seek_interrupt",
                clear_cross_links,
            );
        }

        if let Some(card) = effects.condolation.as_mut() {
            card.was_selected = self.current_element_for_actor(card.owner)
                == Some((card.seq_id, usize::from(card.elem_idx)));
            if goal_owner_debug_matches(card.owner) {
                let provenance = GoalOwnerTerminalProvenance {
                    site: terminal_site,
                    selected: self.current_element_for_actor(card.owner),
                    translating: self.actor_translating,
                };
                GOAL_OWNER_TERMINAL_PROVENANCE.with(|records| {
                    records
                        .borrow_mut()
                        .insert((card.seq_id, card.elem_idx), provenance);
                });
            }
            tracing::trace!(
                target: "parity_owner_handoff",
                owner = ?card.owner,
                seq_id = ?card.seq_id,
                elem_idx = card.elem_idx,
                command = ?card.command,
                terminal_state = ?card.terminal_state,
                was_selected = card.was_selected,
                instructing = ?self.actor_instructing.get(&card.owner),
                in_progress = ?self.actor_in_progress.get(&card.owner),
                "condolation card capturing selection at SetState"
            );
        }

        if let Some(seq) = self.sequences.get_mut(&seq_id) {
            if effects.increment_in_progress {
                seq.increase_elements_in_progress();
            }
            if effects.decrement_in_progress {
                seq.decrease_elements_in_progress();
            }
        }

        if let Some((elem_idx, owner, old_state, new_state)) = effects.actor_live_transition {
            let elem_ref = SequenceElementRef::new(seq_id, elem_idx);
            match (
                Self::is_actor_live_state(old_state),
                Self::is_actor_live_state(new_state),
            ) {
                (false, true) => self.insert_actor_live_ref(owner, elem_ref),
                (true, false) => self.remove_actor_live_ref(owner, elem_ref),
                _ => {}
            }
            if clear_cross_links
                && matches!(
                    new_state,
                    SequenceState::Terminated
                        | SequenceState::Interrupted
                        | SequenceState::Impossible
                )
            {
                self.clear_cross_postponed_links_to((seq_id, elem_idx));
            }
        }

        // Maintain `actor_in_progress`. The (elem_idx, owner) carried
        // by `entered/left_in_progress` point at whichever element
        // actually transitioned — which can differ from any outer
        // elem_idx the caller passed in (e.g. `stop_element` recurses
        // to a sibling / postponed element).
        if let Some((elem_idx, owner)) = effects.entered_in_progress {
            self.actor_in_progress
                .entry(owner)
                .or_default()
                .insert(SequenceElementRef::new(seq_id, elem_idx));
        }
        if let Some((elem_idx, owner)) = effects.left_in_progress
            && let Some(set) = self.actor_in_progress.get_mut(&owner)
        {
            set.remove(&SequenceElementRef::new(seq_id, elem_idx));
            if set.is_empty() {
                self.actor_in_progress.remove(&owner);
            }
        }

        // `RHSequenceElement::SetState` calls SendCondolationCard before
        // cascading or calling Ready.  Suspend those trailing effects at
        // exactly that boundary; the engine resumes them after the card's
        // recursive Think has completed.  Impossible is the one exception:
        // the original calls StartPostponedSequenceElement before falling
        // through to the Interrupted/card branch.
        if let Some(mut card) = effects.condolation.take() {
            // If this sequence tear-down came from an in-flight
            // `Halt()` call, mark the card so the `SendCondolationCard`
            // handler knows to skip the Think dispatch.
            if self.halt_pending {
                card.from_halt = true;
            }

            card.postponed_successor_pending = card.terminal_state != SequenceState::Impossible
                && effects.resume_cross_postponed.is_some();

            if card.terminal_state == SequenceState::Impossible {
                self.resume_postponed_effects(
                    seq_id,
                    effects.start_postponed.take(),
                    effects.resume_cross_postponed.take(),
                );
            }

            self.pending_condolations.push(PendingCondolationDispatch {
                card,
                effects_after_card: effects,
            });
            return;
        }

        self.process_effects_after_condolation_with_cross_cleanup(
            seq_id,
            effects,
            clear_cross_links,
        );
    }

    fn process_effects_after_condolation(
        &mut self,
        seq_id: SequenceId,
        effects: StateChangeEffects,
    ) {
        self.process_effects_after_condolation_with_cross_cleanup(seq_id, effects, true);
    }

    fn process_effects_after_condolation_with_cross_cleanup(
        &mut self,
        seq_id: SequenceId,
        effects: StateChangeEffects,
        clear_cross_links: bool,
    ) {
        let install_cross_postponed_after_card = effects.install_cross_postponed_after_card;
        // Process cascading state changes
        for (cascade_elem_idx, cascade_state, cascade_flags) in effects.cascade {
            let sub_effects = {
                let Some(seq) = self.sequences.get_mut(&seq_id) else {
                    continue;
                };
                if cascade_elem_idx >= seq.elements.len() {
                    continue;
                }
                seq.set_element_state(cascade_elem_idx, cascade_state, cascade_flags)
            };
            // Recursively process sub-effects
            self.process_effects_with_cross_cleanup(
                seq_id,
                sub_effects,
                "terminal_state_cascade",
                clear_cross_links,
            );
        }

        // Signal ready (element finished) — advance to next level
        if effects.signal_ready {
            let to_go = {
                let Some(seq) = self.sequences.get_mut(&seq_id) else {
                    return;
                };
                if seq.running_elements == 0 {
                    let elements: Vec<_> = seq
                        .elements
                        .iter()
                        .enumerate()
                        .map(|(idx, elem)| {
                            (
                                idx,
                                elem.command,
                                elem.command_level,
                                elem.owner,
                                elem.state,
                                elem.priority,
                                elem.orders.len(),
                            )
                        })
                        .collect();
                    panic!(
                        "Ready called with no running elements: seq_id={seq_id:?} cursor={} current_level={} elements_in_progress={} elements={elements:?}",
                        seq.cursor, seq.current_command_level, seq.elements_in_progress
                    );
                }
                if seq.element_ready() {
                    seq.next_elements_go()
                } else {
                    Vec::new()
                }
            };
            self.register_level_elements_to_go(seq_id, to_go);
        }

        // Start postponed element if requested.  We always re-pathfind
        // on restart:
        //
        //   1. Path rebuild: every re-registered Move/Seek element gets
        //      a fresh `InstructOwner` → `try_dispatch_move_path` pass,
        //      and `build_orders_from_path` clears the old orders before
        //      rebuilding waypoints from the actor's current position.
        //   2. We never reassign an element's `command` to
        //      `Command::MoveOk` (see `engine/posture_transitions.rs:281`
        //      for the rationale — flipping to `MoveOk` breaks
        //      `element_priority::actor_branch` priority resolution).
        //      So no element is ever in a `MoveOk` state that would
        //      need a posture-aware revert; the branch is moot.
        self.resume_postponed_effects(
            seq_id,
            effects.start_postponed,
            effects.resume_cross_postponed,
        );

        if let Some((blocker_seq, blocker_idx, waiter_seq, waiter_idx)) =
            install_cross_postponed_after_card
        {
            let waiter_state = self
                .get_element(waiter_seq, waiter_idx)
                .map(|element| element.state)
                .unwrap_or_else(|| {
                    panic!(
                        "deferred cross-postponed waiter {waiter_seq:?}/{waiter_idx} disappeared"
                    )
                });
            assert_eq!(
                waiter_state,
                SequenceState::Postponed,
                "deferred cross-postponed waiter {waiter_seq:?}/{waiter_idx} is not postponed"
            );
            let blocker = self
                .get_element(blocker_seq, blocker_idx)
                .unwrap_or_else(|| {
                    panic!(
                        "deferred cross-postponed blocker {blocker_seq:?}/{blocker_idx} disappeared"
                    )
                });
            assert!(
                blocker.cross_postponed.is_none(),
                "deferred cross-postponed blocker {blocker_seq:?}/{blocker_idx} acquired another waiter"
            );
            self.set_cross_postponed_link(
                (blocker_seq, blocker_idx),
                Some((waiter_seq, waiter_idx)),
            );
        }
    }

    fn resume_postponed_effects(
        &mut self,
        seq_id: SequenceId,
        start_postponed: Option<(usize, usize)>,
        resume_cross_postponed: Option<(SequenceId, usize)>,
    ) {
        if let Some((blocker_idx, postponed_idx)) = start_postponed {
            self.register_element_to_go(seq_id, postponed_idx);
            let blocker = self
                .sequences
                .get_mut(&seq_id)
                .and_then(|sequence| sequence.elements.get_mut(blocker_idx))
                .unwrap_or_else(|| {
                    panic!("released postponed blocker {seq_id:?}/{blocker_idx} disappeared")
                });
            assert_eq!(
                blocker.postponed_element_index,
                Some(postponed_idx),
                "released postponed blocker {seq_id:?}/{blocker_idx} no longer points to {postponed_idx}"
            );
            // Original `StartPostponedSequenceElement` clears
            // `mpsqePostponedSequenceElement` immediately after registering
            // the released element.  Leaving this edge live lets a later
            // same-frame Stop recurse through the stale blocker and interrupt
            // work that is already back on the manager FIFO.
            blocker.postponed_element_index = None;
        }

        // Release the cross-sequence postponed successor. The manager's later
        // `Go()` calls `RHElementActor::Instruct` again, which snapshots the
        // actor's posture and action state as they exist at that second
        // instruction boundary. Mark the old snapshot stale so the engine
        // performs the same refresh before arbitration and translation.
        if let Some((succ_seq_id, succ_idx)) = resume_cross_postponed
            && let Some(succ_seq) = self.sequences.get_mut(&succ_seq_id)
            && let Some(succ_elem) = succ_seq.elements.get_mut(succ_idx)
            && succ_elem.state == SequenceState::Postponed
        {
            succ_elem.state = SequenceState::Todo;
            succ_elem.posture_after_transition = crate::element::Posture::Undefined;
            self.register_element_to_go(succ_seq_id, succ_idx);
        }
    }

    // ─── Termination ────────────────────────────────────────────

    /// Terminate a sequence by interrupting its first element (cascades to all).
    pub fn terminate_sequence(&mut self, seq_id: SequenceId) -> bool {
        let Some(seq) = self.sequences.get_mut(&seq_id) else {
            return false;
        };

        assert!(!seq.is_empty());
        let effects =
            seq.set_element_state(0, SequenceState::Interrupted, CascadeFlags::NEXT_LEVEL);
        self.process_effects(seq_id, effects, "terminate_sequence");
        true
    }

    // ─── Cleanup ────────────────────────────────────────────────

    /// Remove completed/interrupted sequences.
    pub fn friday_evening_cleanup(&mut self) {
        self.friday_evening_cleanup_preserving(&std::collections::BTreeSet::new());
    }

    /// Remove completed/interrupted sequences except those still addressed by
    /// an external legacy pointer emulation.
    pub fn friday_evening_cleanup_preserving(
        &mut self,
        retained_sequences: &std::collections::BTreeSet<SequenceId>,
    ) {
        // `BTreeMap::retain` preserves keys, so every `SequenceId`
        // stored elsewhere (`elements_to_go`, `actor_live`,
        // `actor_in_progress`,
        // `cross_postponed`, `post_seek_sequence`, …) stays valid. Any
        // InProgress element in a removed sequence should already be
        // gone via the normal state-transition path, but scrub
        // `actor_in_progress` defensively in case a sequence is torn
        // down without a terminal state change. `elements_to_go`
        // entries for removed ids are dropped lazily by `hourglass`'s
        // existence check.
        let sequence_count_before = self.sequences.len();
        self.sequences
            .retain(|seq_id, seq| retained_sequences.contains(seq_id) || !seq.is_to_be_deleted());
        if self.sequences.len() != sequence_count_before {
            self.postpone_tail_cache.clear();
        }

        let sequences = &self.sequences;
        self.actor_live.retain(|_, refs| {
            refs.retain(|r| sequences.contains_key(&r.sequence_id));
            !refs.is_empty()
        });
        self.actor_in_progress.retain(|_, refs| {
            refs.retain(|r| sequences.contains_key(&r.sequence_id));
            !refs.is_empty()
        });
    }

    // ─── Cancellation helpers ───────────────────────────────────

    /// Cancel not-yet-launched move commands for a specific actor.
    ///
    /// Walks `elements_to_go` and for every matching entry calls
    /// `set_element_state(Impossible)` *before* removing the element
    /// from the queue. `Impossible` cascades through the next-element
    /// / postponed-element chains and posts a `SendCondolationCard`
    /// to the owner — so successors learn the move became impossible.
    /// (A bare `retain` would drop the queue entries without running
    /// the cascade or queuing the condolation.)
    pub fn cancel_pending_move_commands(&mut self, owner: EntityId) {
        // Pass 1: collect matching `(seq_id, elem_idx)` entries. We
        // can't mutate sequences while iterating `elements_to_go` and
        // we can't mutate `elements_to_go` while iterating `sequences`,
        // so snapshot first.
        let mut targets: Vec<(SequenceId, usize)> = Vec::new();
        for &(seq_id, elem_idx) in &self.elements_to_go {
            let Some(seq) = self.sequences.get(&seq_id) else {
                continue;
            };
            if elem_idx >= seq.elements.len() {
                continue;
            }
            let elem = &seq.elements[elem_idx];
            if elem.owner != Some(owner) {
                continue;
            }
            if matches!(
                elem.command,
                Command::PassDoor | Command::Move | Command::WaitTimer | Command::AssertPosition
            ) {
                targets.push((seq_id, elem_idx));
            }
        }

        // Pass 2: mark each target Impossible (cascading next/postponed
        // chains and queuing the owner's condolation card).
        for (seq_id, elem_idx) in &targets {
            let Some(seq) = self.sequences.get_mut(seq_id) else {
                continue;
            };
            let effects = seq.set_element_state(
                *elem_idx,
                SequenceState::Impossible,
                CascadeFlags::NEXT_LEVEL,
            );
            self.process_effects(*seq_id, effects, "cancel_pending_move_commands");
        }

        // Pass 3: drop the cancelled entries from the queue.
        let target_set: std::collections::HashSet<(SequenceId, usize)> =
            targets.into_iter().collect();
        self.elements_to_go
            .retain(|entry| !target_set.contains(entry));
    }

    /// Stop all active and pending sequence elements owned by `owner` whose
    /// priority is weak enough to be pre-empted by `stop_priority`.
    ///
    /// Calls [`Sequence::stop_element`] on the actor's authoritative current
    /// element, then runs [`Self::stop_pending_elements`] for the
    /// not-yet-launched queue. `RHElementActor::Stop` does not scan every live
    /// element owned by the actor: it follows `mpSequenceElement`, whose
    /// postponed chain remains reachable even while a terminating injury's
    /// condolence callback is running. Cross-sequence postponed work is the
    /// Rust representation of that same pointer and is stopped explicitly.
    pub fn stop_owner(
        &mut self,
        owner: EntityId,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) {
        let root = self.current_element_for_actor(owner);
        self.stop_owner_from_root(owner, root, stop_priority, resolver);
    }

    /// `RHElementActor::Stop` with an explicit root instead of the actor's
    /// currently selected element.
    ///
    /// A command that stops its owner from inside its own translation runs
    /// before the incoming element has been installed as the actor's
    /// selection, yet Original has already assigned `mpSequenceElement` by
    /// then and therefore stops through the incoming element — reaching
    /// whatever that element pushed into its postponed slot.
    pub fn stop_owner_from_root(
        &mut self,
        owner: EntityId,
        root: Option<(SequenceId, usize)>,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) {
        self.stop_owner_current_from_root(owner, root, stop_priority, resolver);
        self.stop_pending_elements(owner, stop_priority, resolver);
    }

    /// Stop only the actor-selected element and its postponed graph.
    ///
    /// Original `RHElementActor::Stop` has an observable callback boundary:
    /// stopping `mpSequenceElement` synchronously invokes
    /// `SendCondolationCard`, and only after that callback returns does Actor
    /// call `StopNotYetLaunchedSequenceElements`. Engine call sites which can
    /// pump that callback use this phase separately, then call
    /// [`Self::stop_pending_elements`] after the callback has completed.
    pub fn stop_owner_current_from_root(
        &mut self,
        owner: EntityId,
        root: Option<(SequenceId, usize)>,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) {
        // `RHSequenceElement::Stop` always follows the postponed pointer,
        // even when the current node is too strong. That is observable if a
        // weaker descendant exists, but it is a pure no-op when every live
        // element owned by this actor is stronger than `stop_priority`.
        // Checking the actor-wide ceiling is deliberately conservative: it
        // can decline this fast path because of unrelated weak work, but can
        // never hide a stoppable node in the selected graph. Terminal nodes
        // are removed from postponed links at their state-transition cleanup.
        let root_is_live = root.is_some_and(|(seq_id, elem_idx)| {
            self.get_element(seq_id, elem_idx).is_some_and(|element| {
                element.owner == Some(owner)
                    && Self::is_actor_live_state(element.state)
                    && !(element.command == Command::Wait
                        && element.priority == SequencePriority::Wait)
            })
        });
        let selected_chain_is_all_stronger =
            root.is_some_and(|root| self.selected_cross_chain_all_stronger(root, stop_priority));
        if root_is_live
            && (selected_chain_is_all_stronger
                || self.actor_stop_summary(owner).is_some_and(|summary| {
                    summary.cross_only && summary.weakest_priority < stop_priority
                }))
        {
            tracing::trace!(
                target: "parity_stop",
                ?owner,
                ?stop_priority,
                ?root,
                "manager stop_owner all live work too strong"
            );
            return;
        }

        // Original `RHElementActor::Stop` starts from exactly
        // `mpSequenceElement`. Scanning every InProgress/Postponed element is
        // observably different after loading: stale non-selected branches can
        // form a large shared next/postponed graph, so recursively stopping
        // each node as a fresh root repeats that graph exponentially.
        let mut targets = VecDeque::new();
        if let Some(current) = root
            && self.get_element(current.0, current.1).is_some_and(|elem| {
                !(elem.command == Command::Wait && elem.priority == SequencePriority::Wait)
            })
        {
            tracing::trace!(
                target: "parity_stop",
                ?owner,
                ?stop_priority,
                ?current,
                "manager stop_owner current"
            );
            targets.push_back(current);
        }
        if targets.is_empty() {
            tracing::trace!(
                target: "parity_stop",
                ?owner,
                ?stop_priority,
                "manager stop_owner no current target"
            );
        }
        let mut visited = HashSet::new();
        let mut touched_sequences = HashSet::new();
        while let Some((seq_id, elem_idx)) = targets.pop_front() {
            if !visited.insert((seq_id, elem_idx)) {
                continue;
            }
            let target_owner = self
                .get_element(seq_id, elem_idx)
                .unwrap_or_else(|| {
                    panic!("Stop postponed graph references missing {seq_id:?}/{elem_idx}")
                })
                .owner;
            touched_sequences.insert(seq_id);
            assert_eq!(
                target_owner,
                Some(owner),
                "Stop postponed graph crosses owners at {seq_id:?}/{elem_idx}"
            );
            tracing::trace!(
                target: "parity_stop",
                ?owner,
                ?seq_id,
                elem_idx,
                "manager before stop_element"
            );
            let (effects_vec, cross_targets) = self
                .sequences
                .get_mut(&seq_id)
                .expect("validated Stop target sequence disappeared")
                .stop_element_with_cross_targets(elem_idx, stop_priority, resolver);
            for cross in cross_targets {
                if !visited.contains(&cross) {
                    targets.push_back(cross);
                }
            }
            tracing::trace!(
                target: "parity_stop",
                ?owner,
                ?seq_id,
                elem_idx,
                effects = effects_vec.len(),
                "manager after stop_element"
            );
            for (effect_index, effects) in effects_vec.into_iter().enumerate() {
                tracing::trace!(
                    target: "parity_stop",
                    ?owner,
                    ?seq_id,
                    elem_idx,
                    effect_index,
                    "manager before process_effects"
                );
                self.process_effects_deferring_cross_cleanup(seq_id, effects, "stop_owner");
                tracing::trace!(
                    target: "parity_stop",
                    ?owner,
                    ?seq_id,
                    elem_idx,
                    effect_index,
                    "manager after process_effects"
                );
            }
        }

        // Original clears postponed pointers while this recursive Stop graph
        // unwinds. Restrict Rust's split-storage cleanup to the sequences the
        // graph actually visited: EnterSwordfight can invoke Stop thousands of
        // times in one frame, so even one retained-manager scan per invocation
        // is quadratic in replay history.
        self.clear_terminal_cross_postponed_links_in(&touched_sequences);
    }

    /// Drop blocker links whose postponed target was stopped either directly
    /// or by a following-element cascade.
    ///
    /// Original `RHSequenceElement::Stop` recursively stops its postponed
    /// element and nulls the pointer when that target becomes interrupted.
    /// Rust can reach the same target through `CASCADE_FOLLOWING`, in which
    /// case it is absent from the direct `stopped` list above. Retaining that
    /// link past Friday cleanup would leave a dangling sequence reference.
    fn clear_terminal_cross_postponed_links(&mut self) {
        let dead_targets: std::collections::HashSet<(SequenceId, usize)> =
            self.sequences
                .iter()
                .flat_map(|(sequence_id, sequence)| {
                    sequence.elements.iter().enumerate().filter_map(
                        move |(element_index, element)| {
                            matches!(
                                element.state,
                                SequenceState::Terminated
                                    | SequenceState::Interrupted
                                    | SequenceState::Impossible
                            )
                            .then_some((*sequence_id, element_index))
                        },
                    )
                })
                .collect();
        let mut changed_owners = BTreeSet::new();
        for sequence in self.sequences.values_mut() {
            for element in &mut sequence.elements {
                if element
                    .cross_postponed
                    .is_some_and(|target| dead_targets.contains(&target))
                {
                    if let Some(owner) = element.owner {
                        changed_owners.insert(owner);
                    }
                    element.cross_postponed = None;
                }
            }
        }
        for owner in changed_owners {
            self.invalidate_postpone_tail_cache_for(owner);
        }
    }

    /// Clear dead cross-postponed successors only from sequences visited by a
    /// bounded Stop graph. This is the local equivalent of Original nulling a
    /// postponed pointer while that recursive call unwinds.
    fn clear_terminal_cross_postponed_links_in(&mut self, source_sequences: &HashSet<SequenceId>) {
        let mut candidate_links = Vec::new();
        for sequence_id in source_sequences {
            let Some(sequence) = self.sequences.get(sequence_id) else {
                continue;
            };
            for (element_index, element) in sequence.elements.iter().enumerate() {
                if let Some(target) = element.cross_postponed {
                    candidate_links.push((*sequence_id, element_index, target));
                }
            }
        }

        let dead_sources = candidate_links
            .into_iter()
            .filter_map(|(sequence_id, element_index, target)| {
                self.get_element(target.0, target.1)
                    .is_some_and(|target_element| {
                        matches!(
                            target_element.state,
                            SequenceState::Terminated
                                | SequenceState::Interrupted
                                | SequenceState::Impossible
                        )
                    })
                    .then_some((sequence_id, element_index))
            })
            .collect::<Vec<_>>();

        for (sequence_id, element_index) in dead_sources {
            self.set_cross_postponed_link((sequence_id, element_index), None);
        }
    }

    fn clear_cross_postponed_links_to(&mut self, target: (SequenceId, usize)) {
        let mut changed_owners = BTreeSet::new();
        for sequence in self.sequences.values_mut() {
            for element in &mut sequence.elements {
                if element.cross_postponed == Some(target) {
                    if let Some(owner) = element.owner {
                        changed_owners.insert(owner);
                    }
                    element.cross_postponed = None;
                }
            }
        }
        for owner in changed_owners {
            self.invalidate_postpone_tail_cache_for(owner);
        }
    }

    /// Stop not-yet-launched elements for a specific actor up to a priority.
    pub fn stop_pending_elements(
        &mut self,
        owner: EntityId,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) {
        // Work from the same roots as Original's
        // StopNotYetLaunchedSequenceElements: entries currently registered in
        // the manager's to-go list for this owner. A root can be too strong to
        // stop while still owning a postponed pointer; RHSequenceElement::Stop
        // follows that pointer unconditionally, so retain the cross-sequence
        // targets returned by Rust's split-storage representation.
        let roots = self.pending_elements_for_owner(owner);
        self.stop_pending_roots(owner, roots, stop_priority, resolver);
        self.compact_terminal_elements_to_go();
    }

    /// Snapshot the entries which Original's
    /// `StopNotYetLaunchedSequenceElements` will visit. The C++ loop captures
    /// the list size on entry, so callback-appended work is deliberately not
    /// part of this result.
    pub fn pending_elements_for_owner(&self, owner: EntityId) -> Vec<(SequenceId, usize)> {
        self.elements_to_go
            .iter()
            .copied()
            .filter(|(seq_id, elem_idx)| {
                self.get_element(*seq_id, *elem_idx).is_some_and(|element| {
                    element.owner == Some(owner) && element.state != SequenceState::Interrupted
                })
            })
            .collect()
    }

    /// Physically remove terminal tombstones after a callback-separated
    /// pending Stop scan. State-aware registration queries hide each stopped
    /// entry immediately; compacting once after the snapshot finishes keeps
    /// the stable manager queue identical to Original without an O(queue)
    /// retain after every root.
    pub(crate) fn compact_terminal_elements_to_go(&mut self) {
        self.elements_to_go.retain(|(seq_id, elem_idx)| {
            self.sequences.get(seq_id).is_none_or(|sequence| {
                sequence
                    .elements
                    .get(*elem_idx)
                    .is_none_or(|element| element.state != SequenceState::Interrupted)
            })
        });
    }

    /// Stop one root from a previously captured pending-list snapshot.
    /// Callers that model `Actor::Stop` can close the resulting synchronous
    /// condolence stack before visiting the next captured root.
    pub fn stop_pending_element_from_root(
        &mut self,
        owner: EntityId,
        root: (SequenceId, usize),
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) {
        self.stop_pending_roots(owner, [root], stop_priority, resolver);
    }

    fn stop_pending_roots(
        &mut self,
        owner: EntityId,
        roots: impl IntoIterator<Item = (SequenceId, usize)>,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) {
        let mut targets = roots.into_iter().collect::<VecDeque<_>>();
        let mut visited = HashSet::new();
        let mut touched_sequences = HashSet::new();

        while let Some((seq_id, elem_idx)) = targets.pop_front() {
            if !visited.insert((seq_id, elem_idx)) {
                continue;
            }
            let Some(seq) = self.sequences.get(&seq_id) else {
                continue;
            };
            if elem_idx >= seq.elements.len() {
                continue;
            }
            touched_sequences.insert(seq_id);
            assert_eq!(
                seq.elements[elem_idx].owner,
                Some(owner),
                "pending postponed graph crosses owners at {seq_id:?}/{elem_idx}"
            );

            let (effects_vec, cross_targets) = self
                .sequences
                .get_mut(&seq_id)
                .expect("validated pending sequence disappeared")
                .stop_element_with_cross_targets(elem_idx, stop_priority, resolver);
            for target in cross_targets {
                if !visited.contains(&target) {
                    targets.push_back(target);
                }
            }
            for effects in effects_vec {
                self.process_effects_deferring_cross_cleanup(
                    seq_id,
                    effects,
                    "stop_pending_elements",
                );
            }
        }

        // This API is called once per captured pending root so the engine can
        // dispatch that root's synchronous condolence before advancing the
        // snapshot. A global retained-sequence cleanup here makes the whole
        // scan quadratic. Every cross edge followed by this bounded Stop has
        // its source in a touched sequence, so restrict cleanup to that set.
        self.clear_terminal_cross_postponed_links_in(&touched_sequences);
    }

    /// Stop queued elements for `owner` whose command matches `command`.
    /// Counterpart to [`Self::stop_pending_elements`] with a command
    /// filter — used by the right-click `Bow` arm to drain the PC's
    /// queued `Command::ShootBow` elements without cancelling other
    /// in-flight work.
    ///
    /// This covers both not-yet-launched `elements_to_go` entries and
    /// cross-postponed elements.  C++ stores repeated PC bow clicks in
    /// `mlpsequenceShootList`; in Rust those clicks may be represented
    /// as `SequenceState::Postponed`, so clearing the queue must see
    /// both forms.
    ///
    /// Returns the number of pending elements that were stopped + removed.
    pub fn stop_pending_elements_matching(
        &mut self,
        owner: EntityId,
        command: Command,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) -> usize {
        let has_matching_live_element = self.actor_live.get(&owner).is_some_and(|refs| {
            refs.iter().any(|element_ref| {
                self.get_element(element_ref.sequence_id, element_ref.element_index)
                    .is_some_and(|element| {
                        element.command == command && element.state != SequenceState::InProgress
                    })
            })
        });
        if !has_matching_live_element {
            return 0;
        }

        let mut to_remove = Vec::new();
        let mut terminal_effects_processed = false;

        for i in 0..self.elements_to_go.len() {
            let (seq_id, elem_idx) = self.elements_to_go[i];
            let Some(seq) = self.sequences.get(&seq_id) else {
                continue;
            };
            if elem_idx >= seq.elements.len() {
                continue;
            }
            let elem = &seq.elements[elem_idx];
            if elem.owner != Some(owner) || elem.command != command {
                continue;
            }
            if elem.state == SequenceState::InProgress {
                continue;
            }

            let effects_vec = self
                .sequences
                .get_mut(&seq_id)
                .expect("validated pending sequence disappeared")
                .stop_element(elem_idx, stop_priority, resolver);
            terminal_effects_processed |= !effects_vec.is_empty();
            for effects in effects_vec {
                self.process_effects_deferring_cross_cleanup(
                    seq_id,
                    effects,
                    "stop_pending_elements_matching",
                );
            }

            if let Some(seq) = self.sequences.get(&seq_id)
                && seq.elements[elem_idx].state == SequenceState::Interrupted
            {
                to_remove.push(i);
            }
        }

        let count = to_remove.len();
        for &idx in to_remove.iter().rev() {
            self.elements_to_go.remove(idx);
        }

        // `actor_live` contains every Todo/InProgress/Postponed element for
        // this owner. Use it to avoid a complete retained-sequence scan for
        // the overwhelmingly common no-match case (for example each queued
        // EnterSwordfight checking for an old ShootBow). Restore manager
        // insertion order before dispatch so loaded non-monotonic sequence IDs
        // preserve Original's first-to-last traversal.
        let mut postponed_targets = self
            .actor_live
            .get(&owner)
            .into_iter()
            .flat_map(|refs| refs.iter())
            .filter_map(|element_ref| {
                self.get_element(element_ref.sequence_id, element_ref.element_index)
                    .is_some_and(|element| {
                        element.command == command && element.state == SequenceState::Postponed
                    })
                    .then_some((element_ref.sequence_id, element_ref.element_index))
            })
            .collect::<Vec<_>>();
        postponed_targets.sort_by_key(|(sequence_id, element_index)| {
            (
                self.sequences
                    .get_index_of(sequence_id)
                    .unwrap_or_else(|| panic!("actor-live sequence {sequence_id:?} disappeared")),
                *element_index,
            )
        });

        let mut stopped_count = count;
        for (seq_id, elem_idx) in postponed_targets {
            let effects_vec = self
                .sequences
                .get_mut(&seq_id)
                .expect("collected postponed sequence disappeared")
                .stop_element(elem_idx, stop_priority, resolver);
            terminal_effects_processed |= !effects_vec.is_empty();
            for effects in effects_vec {
                self.process_effects_deferring_cross_cleanup(
                    seq_id,
                    effects,
                    "stop_postponed_elements_matching",
                );
            }

            if let Some(seq) = self.sequences.get(&seq_id)
                && seq.elements[elem_idx].state == SequenceState::Interrupted
            {
                stopped_count += 1;
            }
        }

        if terminal_effects_processed {
            // As in `stop_owner_current_from_root`, clear inbound links once
            // for the completed batch instead of rescanning the complete
            // manager for every matching pending or postponed element.
            self.clear_terminal_cross_postponed_links();
        }

        stopped_count
    }

    /// Returns `true` if `owner` has a queued element with this command.
    /// Includes both not-yet-launched `elements_to_go` entries and
    /// cross-postponed elements.
    pub fn queued_element_exists(&self, owner: EntityId, command: Command) -> bool {
        for &(seq_id, elem_idx) in &self.elements_to_go {
            let Some(seq) = self.sequences.get(&seq_id) else {
                continue;
            };
            let Some(elem) = seq.elements.get(elem_idx) else {
                continue;
            };
            if elem.owner == Some(owner)
                && elem.command == command
                && elem.state != SequenceState::InProgress
            {
                return true;
            }
        }
        self.sequences.values().any(|seq| {
            seq.elements.iter().any(|elem| {
                elem.owner == Some(owner)
                    && elem.command == command
                    && elem.state == SequenceState::Postponed
            })
        })
    }

    /// Check if there's a pending element with this command for this owner.
    pub fn element_is_about_to_be_launched(&self, owner: EntityId, command: Command) -> bool {
        for &(seq_id, elem_idx) in &self.elements_to_go {
            let Some(seq) = self.sequences.get(&seq_id) else {
                continue;
            };
            if elem_idx >= seq.elements.len() {
                continue;
            }
            let elem = &seq.elements[elem_idx];
            if elem.owner == Some(owner) && (command == Command::Null || elem.command == command) {
                return true;
            }
        }
        false
    }

    /// Check the two pending-command forms used by Original's actor AI:
    /// an element registered to launch, or an element postponed directly
    /// behind the actor's current element.
    ///
    /// This deliberately does not scan every postponed element owned by the
    /// actor. Original asks `GetSequenceElement()->GetPostponedSequenceElement()`
    /// here, so only the current element's immediate successor qualifies.
    pub fn element_is_about_to_be_launched_or_postponed_by_current(
        &self,
        owner: EntityId,
        command: Command,
    ) -> bool {
        if self.element_is_about_to_be_launched(owner, command) {
            return true;
        }

        let Some((seq_id, elem_idx)) = self.current_element_for_actor(owner) else {
            return false;
        };
        let Some(current) = self.get_element(seq_id, elem_idx) else {
            debug_assert!(false, "current actor element is missing from its sequence");
            return false;
        };

        let intra_sequence_matches = current
            .postponed_element_index
            .and_then(|postponed_idx| self.get_element(seq_id, postponed_idx))
            .is_some_and(|postponed| postponed.command == command);
        let cross_sequence_matches = current
            .cross_postponed
            .and_then(|(postponed_seq, postponed_idx)| {
                self.get_element(postponed_seq, postponed_idx)
            })
            .is_some_and(|postponed| postponed.command == command);

        intra_sequence_matches || cross_sequence_matches
    }

    /// Apply MakeFast to all active/pending movement elements owned by
    /// `entity` in its selected element's following/postponed chain. Sets the
    /// FAST flag, upgrades the element's action from walking to running, and
    /// rewrites queued walking / start-walking / stop-walking orders.
    pub fn make_fast(&mut self, entity: EntityId) {
        self.rewrite_selected_actor_chain(entity, make_fast_element);
    }

    /// Set `action` on the movement element at `(seq_id, elem_idx)` and recurse
    /// through its same-owner following/postponed graph. Callers use this to
    /// force a door-authored movement chain onto one animation.
    pub fn set_action_recursive(&mut self, seq_id: SequenceId, elem_idx: usize, action: OrderType) {
        let Some(root) = self.get_element(seq_id, elem_idx) else {
            return;
        };
        let owner = root.owner;
        let mut visited = HashSet::new();
        let mut pending = vec![(seq_id, elem_idx)];
        while let Some((sid, idx)) = pending.pop() {
            if !visited.insert((sid, idx)) {
                continue;
            }
            let Some(element) = self.get_element(sid, idx) else {
                continue;
            };
            if element.owner != owner {
                continue;
            }
            let following = if let Some(legacy) = &element.legacy_v48 {
                legacy
                    .next
                    .map(|next| (next.sequence_id, next.element_index))
            } else {
                self.sequences
                    .get(&sid)
                    .and_then(|sequence| sequence.elements.get(idx + 1))
                    .map(|_| (sid, idx + 1))
            };
            let postponed = element
                .cross_postponed
                .or_else(|| element.postponed_element_index.map(|next| (sid, next)));

            self.get_element_mut(sid, idx)
                .expect("SetActionRecursive graph element disappeared")
                .set_action(action);
            if let Some(following) = following {
                pending.push(following);
            }
            if let Some(postponed) = postponed {
                pending.push(postponed);
            }
        }
    }

    /// Apply MakeSlow to all active/pending movement elements owned by
    /// `entity`. Clears the FAST flag, downgrades running animations to
    /// walking, and rewrites queued transition orders accordingly.
    ///
    /// Symmetric counterpart to [`Self::make_fast`].
    pub fn make_slow(&mut self, entity: EntityId) {
        self.rewrite_selected_actor_chain(entity, make_slow_element);
    }

    /// Apply MakeUpright to all active/pending elements owned by
    /// `entity`. Rewrites crouched-movement orders to upright variants
    /// and cancels pending `CrouchDown` sequence elements (their
    /// command is demoted to `Null`).
    pub fn make_upright(&mut self, entity: EntityId) {
        self.rewrite_selected_actor_chain(entity, make_upright_element);
    }

    /// Apply MakeCrouched to all active/pending elements owned by
    /// `entity`. Clears the FAST flag, downgrades running/walking
    /// upright orders to crouched, and rewrites posture-transition
    /// orders accordingly.
    pub fn make_crouched(&mut self, entity: EntityId) {
        self.rewrite_selected_actor_chain(entity, make_crouched_element);
    }

    /// Reproduce `mpSequenceElement->Make*()`: start at the actor's selected
    /// element and recurse only through same-owner `mpsqeNextSequenceElement`
    /// and `mpsqePostponedSequenceElement` links. An unrelated queued sequence
    /// owned by the same actor is not part of that graph and must not change.
    fn rewrite_selected_actor_chain(
        &mut self,
        entity: EntityId,
        rewrite: fn(&mut SequenceElement),
    ) {
        let Some(root) = self.current_element_for_actor(entity) else {
            return;
        };
        let mut pending = vec![root];
        let mut visited = HashSet::new();

        while let Some((seq_id, elem_idx)) = pending.pop() {
            if !visited.insert((seq_id, elem_idx)) {
                continue;
            }

            let Some(element) = self.get_element(seq_id, elem_idx) else {
                continue;
            };
            if element.owner != Some(entity) {
                continue;
            }

            // Loaded v48 elements retain the exact serialized following
            // pointer. Runtime-authored sequences use their append order,
            // which is how RHSequence wires that pointer on insertion.
            let following = if let Some(legacy) = &element.legacy_v48 {
                legacy
                    .next
                    .map(|next| (next.sequence_id, next.element_index))
            } else {
                self.sequences
                    .get(&seq_id)
                    .and_then(|sequence| sequence.elements.get(elem_idx + 1))
                    .map(|_| (seq_id, elem_idx + 1))
            };
            let postponed = element
                .cross_postponed
                .or_else(|| element.postponed_element_index.map(|idx| (seq_id, idx)));

            rewrite(
                self.get_element_mut(seq_id, elem_idx)
                    .expect("selected Make* chain element disappeared"),
            );

            if let Some(next) = following {
                pending.push(next);
            }
            if let Some(postponed) = postponed {
                pending.push(postponed);
            }
        }
    }

    /// Find the next movement/jump element owned by the same entity, in
    /// either this element's own sequence (following cursor) or in the
    /// attached `post_seek_sequence` if any.
    ///
    /// Returns `true` if the next element (owned by the same entity) is
    /// itself a movement element; `false` if there is no such element
    /// or the owner differs.
    pub fn is_next_movement(&self, seq_id: SequenceId, elem_idx: usize) -> bool {
        self.next_element_in_chain(seq_id, elem_idx)
            .and_then(|(s, i)| self.get_element(s, i))
            .map(|next| next.data.is_movement())
            .unwrap_or(false)
    }

    /// As [`Self::is_next_movement`], but also accepts `Command::JumpCmd`.
    pub fn is_next_movement_or_jump(&self, seq_id: SequenceId, elem_idx: usize) -> bool {
        self.next_element_in_chain(seq_id, elem_idx)
            .and_then(|(s, i)| self.get_element(s, i))
            .map(|next| next.data.is_movement() || next.command == Command::JumpCmd)
            .unwrap_or(false)
    }

    /// Stop the currently-executing movement order for `entity` and
    /// cancel any in-flight path request. Returns `true` if at least
    /// one element was rewritten or had its path cancelled.
    ///
    /// `owner_pos` is the owner's current map position (used to shorten
    /// the movement destination to ~10 units ahead); `cancel_path` is
    /// invoked when any element in the `MoveWaiting` state needs its
    /// pending path request dropped.
    ///
    /// `stop_priority` gates the rewrite: it only runs when the
    /// element's priority is `>= stop_priority` (weaker or equal).
    /// `resolver` lazily promotes `NotYetSet` priorities (mirroring
    /// `Sequence::stop_element`).
    pub fn stop_movement_for_owner(
        &mut self,
        entity: EntityId,
        owner_pos: crate::coordinates::MapPoint,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
        next_order_id: &mut u32,
        cancel_path: &mut dyn FnMut(EntityId),
    ) -> bool {
        self.stop_movement_for_owner_from_root(
            entity,
            None,
            owner_pos,
            stop_priority,
            resolver,
            next_order_id,
            cancel_path,
        )
    }

    /// Run the movement-specific virtual `StopMovement` phase for exactly one
    /// selected element. This is the narrow counterpart to
    /// [`Self::stop_movement_for_owner`]: call sites modeling
    /// `mpSequenceElement->Stop(...)` must not rewrite unrelated in-progress
    /// movements owned by the same actor.
    pub fn stop_movement_from_root(
        &mut self,
        entity: EntityId,
        root: (SequenceId, usize),
        owner_pos: crate::coordinates::MapPoint,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
        next_order_id: &mut u32,
        cancel_path: &mut dyn FnMut(EntityId),
    ) -> bool {
        self.stop_movement_for_owner_from_root(
            entity,
            Some(root),
            owner_pos,
            stop_priority,
            resolver,
            next_order_id,
            cancel_path,
        )
    }

    fn stop_movement_for_owner_from_root(
        &mut self,
        entity: EntityId,
        root: Option<(SequenceId, usize)>,
        owner_pos: crate::coordinates::MapPoint,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
        next_order_id: &mut u32,
        cancel_path: &mut dyn FnMut(EntityId),
    ) -> bool {
        let mut changed = false;
        let mut to_interrupt: Vec<(SequenceId, usize)> = Vec::new();
        let refs: Vec<SequenceElementRef> = if let Some((sequence_id, element_index)) = root {
            vec![SequenceElementRef {
                sequence_id,
                element_index,
            }]
        } else {
            let Some(refs) = self.actor_in_progress.get(&entity) else {
                return false;
            };
            refs.iter().copied().collect()
        };
        for elem_ref in refs {
            let Some(seq) = self.sequences.get_mut(&elem_ref.sequence_id) else {
                debug_assert!(false, "actor_in_progress contains stale sequence ref");
                continue;
            };
            let seq_id = seq.id;
            let elem_idx = elem_ref.element_index;
            let Some(elem) = seq.elements.get_mut(elem_idx) else {
                debug_assert!(false, "actor_in_progress contains stale element ref");
                continue;
            };
            if elem.owner != Some(entity)
                || elem.state != SequenceState::InProgress
                || !elem.data.is_movement()
            {
                continue;
            }
            // Without this priority gate, a weaker `Preference`-
            // priority stop would still rewrite the order of a
            // stronger `Script`-priority movement, causing a visual
            // stutter even though `SequenceManager::stop_owner` will
            // then refuse to actually interrupt the element.
            if elem.priority == SequencePriority::NotYetSet {
                let mut resolved = resolver(elem);
                if resolved == SequencePriority::None {
                    resolved = SequencePriority::Normal;
                }
                elem.priority = resolved;
            }
            if elem.priority < stop_priority {
                continue;
            }
            // Clear SEEK bit; rewrite first order's animation to the
            // matching waiting-transition variant.
            if let SequenceElementData::Movement { flags, .. } = &mut elem.data {
                *flags &= !MoveFlags::SEEK;
            }
            let Some(first) = elem.orders.front_mut() else {
                continue;
            };
            let new_action = match first.order_type {
                crate::order::OrderType::WalkingUpright => {
                    Some(crate::order::OrderType::TransitionWalkingUprightWaitingUpright)
                }
                crate::order::OrderType::RunningUpright => {
                    Some(crate::order::OrderType::TransitionRunningUprightWaitingUpright)
                }
                crate::order::OrderType::WalkingCrouched => {
                    Some(crate::order::OrderType::TransitionWalkingCrouchedWaitingCrouched)
                }
                _ => None,
            };
            let Some(action) = new_action else {
                // Default case: no matching transition — the whole
                // element must be interrupted.  Path cancellation
                // fires on the `Interrupted` transition, so we
                // schedule the state change and run the cascade +
                // path cancellation together below.
                to_interrupt.push((seq_id, elem_idx));
                continue;
            };
            first.order_type = action;
            // Bumping the order id forces the actor-tick consumer
            // (`last_order_id != order.unique_id`) to retrigger
            // `new_order`, which the sprite pipeline uses to reset
            // `MotionState::Start` + `initialize_action_done` so the
            // rewritten Transition*Waiting* animation plays from the
            // first frame.
            first.reseed_id(crate::order::alloc_order_id(next_order_id));
            changed = true;
            // Trim trailing orders and shorten the movement element's
            // destination to ~10 units along the current heading.
            //
            // Original `StopMovement` calls `SetDestination`, whose inline
            // setter changes only `mptDestination`; it deliberately does not
            // rewrite `pOrder->pointDestination2D`.  The transition order
            // therefore reinitializes the sprite against its old path goal,
            // while the element retains the shortened logical destination.
            elem.orders.truncate(1);
            let first = elem.orders.front().expect("truncate kept 1 order");
            let vx = first.target_x - owner_pos.x;
            let vy = first.target_y - owner_pos.y;
            let norm = (vx * vx + vy * vy).sqrt();
            if norm > 10.0 {
                let scale = 10.0 / norm;
                let new_x = owner_pos.x + vx * scale;
                let new_y = owner_pos.y + vy * scale;
                if let SequenceElementData::Movement { destination, .. } = &mut elem.data {
                    destination.x = new_x;
                    destination.y = new_y;
                }
            }
        }
        // Only fire path cancellation for elements that actually
        // transitioned to INTERRUPTED.  A successful rewrite leaves
        // the element in INPROGRESS and keeps the path request alive.
        for (seq_id, elem_idx) in to_interrupt {
            let effects = {
                let Some(seq) = self.sequences.get_mut(&seq_id) else {
                    continue;
                };
                if seq.elements[elem_idx].command == Command::MoveWaiting {
                    seq.elements[elem_idx].command = Command::Move;
                    cancel_path(entity);
                }
                seq.set_element_state(
                    elem_idx,
                    SequenceState::Interrupted,
                    CascadeFlags::NEXT_LEVEL,
                )
            };
            self.process_effects(seq_id, effects, "interrupt_move_towards");
            changed = true;
        }
        changed
    }

    /// Resolve "next element in chain" for
    /// [`Self::is_next_movement`]/`is_next_movement_or_jump`. Follows the
    /// exact loaded v48 following pointer when present; runtime-authored
    /// sequences use append order. The next owner must match.
    ///
    /// Post-seek sequences are stored as separate `Sequence`s registered with
    /// the manager; they are not an implicit following edge. A loaded
    /// `mpsqeNextSequenceElement`, however, is authoritative even when null or
    /// non-adjacent.
    fn next_element_in_chain(
        &self,
        seq_id: SequenceId,
        elem_idx: usize,
    ) -> Option<(SequenceId, usize)> {
        let this = self.get_element(seq_id, elem_idx)?;
        if this.next_link_severed {
            return None;
        }
        let (next_seq, next_idx) = if let Some(legacy) = &this.legacy_v48 {
            let next = legacy.next?;
            (next.sequence_id, next.element_index)
        } else {
            (seq_id, elem_idx + 1)
        };
        let next = self.get_element(next_seq, next_idx)?;
        if this.owner == next.owner {
            Some((next_seq, next_idx))
        } else {
            None
        }
    }

    /// Returns `true` when no further "real" sequence element follows
    /// this one — i.e. the sequence is effectively done after this
    /// element finishes.  `Wait` and `AssertPosition` are skipped
    /// (treated as non-actions).
    pub fn is_last_real_action(&self, seq_id: SequenceId, elem_idx: usize) -> bool {
        let mut cur = (seq_id, elem_idx);
        loop {
            // Original recursively calls IsLastRealAction for every skipped
            // Wait/AssertPosition, and each invocation checks that node's
            // postponed pointer before following `next` again.
            let Some(this) = self.get_element(cur.0, cur.1) else {
                return true;
            };
            if this.postponed_element_index.is_some() || this.cross_postponed.is_some() {
                return false;
            }
            // Stop severs `mpsqeNextSequenceElement` after recursively
            // interrupting that successor (`RHsequenceelement.cpp:549-555`).
            // Runtime-authored sequences remain physically adjacent in Rust,
            // so honor the explicit null-pointer mirror before walking to the
            // next vector element. Otherwise IsLastRealAction can see through
            // a Halt-severed edge into dead AssertPosition/Move elements and
            // suppress the selected element's condolence stimulus.
            if this.next_link_severed {
                return true;
            }
            // RHElementActorNPC::IsLastRealAction follows the raw
            // GetFollowingElement pointer without requiring the next element
            // to have the same owner. This differs deliberately from the
            // movement-chain queries above: a manager-owned Timer or an
            // action for another actor still suppresses this actor's
            // condolence callback when it follows in the same sequence.
            //
            // Original: original-code/RHelementactornpc.cpp:5746-5767.
            let (next_seq, next_idx) = if let Some(legacy) = &this.legacy_v48 {
                let Some(next) = legacy.next else {
                    return true;
                };
                (next.sequence_id, next.element_index)
            } else {
                (cur.0, cur.1 + 1)
            };
            let Some(next_elem) = self.get_element(next_seq, next_idx) else {
                return true;
            };
            match next_elem.command {
                Command::Wait | Command::AssertPosition => {
                    cur = (next_seq, next_idx);
                    continue;
                }
                _ => return false,
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Per-element make_* helpers (free functions for test reuse)
// ═══════════════════════════════════════════════════════════════════

/// Apply MakeFast to a single element in-place. Returns with no effect
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

/// Apply MakeSlow to a single element in-place.
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

/// Apply MakeUpright to a single element in-place. Cancels a pending
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

/// Apply MakeCrouched to a single element in-place.
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
