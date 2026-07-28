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
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
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

// ═══════════════════════════════════════════════════════════════════
//  Script recording session
// ═══════════════════════════════════════════════════════════════════

/// Cached origin for an actor that already has an in-flight script
/// move target (point + sector + level).  Used by
/// `RecordingSession::moving_actors`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct RecordingMotionTarget {
    pub x: f32,
    pub y: f32,
    pub layer: u16,
    pub sector: u16,
}

/// A sequence being built up via script `Record*` calls.
///
/// Flow: `Start()` → `Record*()` → `Then()` → `Record*()` → `Thanx()`
///
/// Elements added between `Start()` and the first `Then()` get command level 1.
/// Each `Then()` bumps the level, so the next batch of `Record*` calls gets
/// a higher level (executed sequentially after the previous level completes).
/// Elements added at the *same* level execute in parallel.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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
)]
pub enum Field {
    Direction,
    RetainedMovementGoal,
    Timer,
    Message,
    MessageArgument,
    MessageExtendedArgument,
    CameraPoint,
    CameraZoomLevel,
    CameraSpeed,
    ActionId,
    ActionAvailable,
    CharacterAvailable,
    SpeakId,
    SpeakFlags,
    SpeakVariant,
    DialogId,
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
    PurseTarget,
    NetTarget,
    WaspNestTarget,
    Opponent,
    Door,
    OldAnimation,
    NewAnimation,
    Freeze,
    Scroll,
    ScrollReader,
    ScrollOwner,
}

/// Polymorphic value stored in a generic sequence element's property map.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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
    Animation(OrderType),
    /// Jump-line id: indexes `FastFindGrid::level::jump_lines`.
    /// All call sites (commands::apply_table_swordfight, engine::jump::is_jumpable,
    /// movement::emit_line_goal) pass a jump-line index through this field,
    /// not a motion-grid line index.
    LineId(crate::jump_line::JumpLineIndex),
    /// Opaque door ID.
    DoorId(crate::gate::DoorIndex),
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
        }
    }
}

impl std::error::Error for SequenceInvariantError {}

// ═══════════════════════════════════════════════════════════════════
//  Element subtype data
// ═══════════════════════════════════════════════════════════════════

/// Element subtypes — variants for simple, movement, generic, damage,
/// and interaction elements.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub enum SequenceElementData {
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
        /// `Clone` on `SequenceElement` deep-clones this `Box<Sequence>`,
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
        post_seek_sequence: Option<Box<Sequence>>,
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
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SequenceElement {
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

    /// The sub-steps (movement waypoints, animation frames, etc.) for this element.
    pub orders: VecDeque<Order>,

    /// Subtype-specific data.
    pub data: SequenceElementData,

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
            posture_after_transition: Posture::default(),
            action_state_after_transition: ActionState::default(),
            num_transition_orders: 0,
            retained_movement_goal: None,
            orders: VecDeque::new(),
            data: SequenceElementData::Simple,
            postponed_element_index: None,
            cross_postponed: None,
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
                    if (!flags.contains(MoveFlags::SEEK) || !flags.contains(MoveFlags::USE_POINT))
                        && let Some(a) = antagonist
                    {
                        new_order.target_actor = Some(a.index());
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
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct Sequence {
    /// Unique ID.
    pub id: SequenceId,

    /// All elements in this sequence, ordered by command level.
    pub elements: Vec<SequenceElement>,

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

impl Sequence {
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
    pub fn next_elements_go(&mut self) -> Vec<(usize, bool)> {
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

        // Collect elements to start, noting whether they are WAIT priority
        let mut to_go = Vec::new();
        for idx in start_index..end_index {
            let elem = &self.elements[idx];
            if elem.state != SequenceState::Interrupted {
                let is_wait = elem.priority == SequencePriority::Wait;
                to_go.push((idx, is_wait));
            }
        }

        to_go
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

/// Result of a state change on a sequence element.
/// The caller (SequenceManager) must process these effects.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct StateChangeEffects {
    /// Elements whose state should also be changed (cascade).
    pub cascade: Vec<(usize, SequenceState, CascadeFlags)>,
    /// Whether `Sequence::element_ready()` should be called.
    pub signal_ready: bool,
    /// Whether to start a postponed element.
    pub start_postponed: Option<usize>,
    /// Cross-sequence postponed successor to resume.  Set when an
    /// element with a non-empty `cross_postponed` link terminates or is
    /// interrupted — the sequence manager takes this (seq_id, elem_idx)
    /// pair and registers it back on the `elements_to_go` queue.
    pub resume_cross_postponed: Option<(SequenceId, usize)>,
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
            cascade: Vec::new(),
            signal_ready: false,
            start_postponed: None,
            resume_cross_postponed: None,
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
                    effects.start_postponed = Some(postponed_idx);
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
                    });
                }
                // Cascade
                Self::compute_cascade(
                    &self.elements,
                    elem_idx,
                    new_state,
                    flags,
                    &mut effects.cascade,
                );
            }

            SequenceState::Interrupted => {
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
                    });
                }
                // Cascade
                Self::compute_cascade(
                    &self.elements,
                    elem_idx,
                    new_state,
                    flags,
                    &mut effects.cascade,
                );
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
                            });
                        }
                        // Tell the sequence this element is done
                        effects.signal_ready = true;
                        // Start postponed if any
                        if let Some(postponed_idx) = self.elements[elem_idx].postponed_element_index
                        {
                            effects.start_postponed = Some(postponed_idx);
                        }
                        // Release cross-sequence postponed successor, if any.
                        if let Some(cross) = self.elements[elem_idx].cross_postponed.take() {
                            effects.resume_cross_postponed = Some(cross);
                        }
                    }
                    _ => {
                        debug_assert!(false, "Terminated from illegal state {:?}", old_state);
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
        elements: &[SequenceElement],
        elem_idx: usize,
        new_state: SequenceState,
        flags: CascadeFlags,
        cascade: &mut Vec<(usize, SequenceState, CascadeFlags)>,
    ) {
        let command_level = elements[elem_idx].command_level;

        if flags.contains(CascadeFlags::FOLLOWING) {
            // Cascade to ALL following elements
            if elem_idx + 1 < elements.len() {
                cascade.push((elem_idx + 1, new_state, CascadeFlags::FOLLOWING));
            }
        } else if flags.contains(CascadeFlags::NEXT_LEVEL) {
            // Find first following element with a higher command level
            let mut next = elem_idx + 1;
            while next < elements.len() && elements[next].command_level == command_level {
                next += 1;
            }
            if next < elements.len() {
                cascade.push((next, new_state, CascadeFlags::FOLLOWING));
            }
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
        let mut all_effects: Vec<StateChangeEffects> = Vec::new();

        let elem = &mut self.elements[elem_idx];

        // Determine priority if not yet set: ask the owning actor's
        // priority resolver and promote `None` to `Normal` so the stop
        // actually succeeds on commands like WAIT / FREEZE.
        if elem.priority == SequencePriority::NotYetSet {
            let mut resolved = resolver(elem);
            if resolved == SequencePriority::None {
                resolved = SequencePriority::Normal;
            }
            elem.priority = resolved;
        }

        // Is the priority weak enough to be stopped? (>= means weaker or equal)
        if elem.priority >= stop_priority {
            if elem.state == SequenceState::InProgress && elem.data.is_movement() {
                // Movements in progress are kept (for transition) but their
                // successor is interrupted
                let next_idx = elem_idx + 1;
                if next_idx < self.elements.len() {
                    all_effects.push(self.set_element_state(
                        next_idx,
                        SequenceState::Interrupted,
                        CascadeFlags::NEXT_LEVEL,
                    ));
                }
            } else {
                all_effects.push(self.set_element_state(
                    elem_idx,
                    SequenceState::Interrupted,
                    CascadeFlags::NEXT_LEVEL,
                ));
            }
        } else {
            // Can't stop this element, but try the next one.
            let next_idx = elem_idx + 1;
            if next_idx < self.elements.len() {
                let sub = self.stop_element(next_idx, stop_priority, resolver);
                all_effects.extend(sub);
                // We use positional adjacency, so there's no pointer to
                // clear after a recursive stop succeeds — the cascade
                // bookkeeping in `set_element_state` already handles
                // the interrupted-next propagation.
            }
        }

        // Unconditional postponed-element handling — runs after the
        // if/else above. Without this, a postponed sibling attached to
        // an Interrupted parent stays alive indefinitely.
        if let Some(postponed_idx) = self.elements[elem_idx].postponed_element_index {
            let sub = self.stop_element(postponed_idx, stop_priority, resolver);
            all_effects.extend(sub);
            // Null the postponed link when the recursive stop left it
            // INTERRUPTED so a subsequent `start_postponed` cascade
            // doesn't try to wake an already-interrupted element.
            if self.elements[postponed_idx].state == SequenceState::Interrupted {
                self.elements[elem_idx].postponed_element_index = None;
            }
        }

        all_effects
    }
}

// ═══════════════════════════════════════════════════════════════════
//  SequenceAction — dispatch events returned by hourglass
// ═══════════════════════════════════════════════════════════════════

/// An action the engine needs to perform on behalf of the sequence system.
/// Returned by [`SequenceManager::hourglass`].
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
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
pub struct SequenceManager {
    /// All active sequences, keyed by `SequenceId`. `BTreeMap` gives
    /// O(log N) id lookup without a side-car `id_to_index` table, and
    /// `BTreeMap::retain` in `friday_evening_cleanup` doesn't shift
    /// keys — so every `SequenceId` stored elsewhere (in
    /// `elements_to_go`, `actor_in_progress`, `cross_postponed`,
    /// `post_seek_sequence`, etc.) stays valid across cleanup.
    ///
    /// Iteration order is ascending by `SequenceId`, which since
    /// `launch_sequence` stamps monotonic ids matches the launch
    /// order — preserving the prior "iterate sequences in vec order,
    /// first match wins" semantic that several scans rely on.
    sequences: BTreeMap<SequenceId, Sequence>,

    /// Actor → every `SequenceElementRef` whose element is currently
    /// live (`Todo`, `InProgress`, or `Postponed`) and owned by that
    /// actor.
    ///
    /// Lets engine paths answer "does this actor already have work?"
    /// without scanning every active sequence. It is derived from
    /// `sequences` and serialized with the manager so snapshots remain
    /// self-contained.
    actor_live: BTreeMap<EntityId, BTreeSet<SequenceElementRef>>,

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
    actor_instructing: BTreeMap<EntityId, Vec<SequenceElementRef>>,

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
    pending_synchronous_actions: VecDeque<SequenceAction>,

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

/// Pending entity cleanup emitted by the sequence manager when an
/// element finishes.  Drained by the engine after each `hourglass`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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
}

/// A condolence card plus the portion of `SetState` that the original
/// performs only after `SendCondolationCard` returns.  Keeping the
/// continuation beside the card preserves the depth-first order across
/// Rust's borrow-safe dispatch boundary.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
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
    fn is_actor_live_state(state: SequenceState) -> bool {
        matches!(
            state,
            SequenceState::Todo | SequenceState::InProgress | SequenceState::Postponed
        )
    }

    fn insert_actor_live_ref(&mut self, owner: EntityId, elem_ref: SequenceElementRef) {
        self.actor_live.entry(owner).or_default().insert(elem_ref);
    }

    fn remove_actor_live_ref(&mut self, owner: EntityId, elem_ref: SequenceElementRef) {
        if let Some(set) = self.actor_live.get_mut(&owner) {
            set.remove(&elem_ref);
            if set.is_empty() {
                self.actor_live.remove(&owner);
            }
        }
    }

    pub fn new() -> Self {
        Self {
            sequences: BTreeMap::new(),
            actor_live: BTreeMap::new(),
            actor_in_progress: BTreeMap::new(),
            actor_instructing: BTreeMap::new(),
            elements_to_go: VecDeque::new(),
            pending_synchronous_actions: VecDeque::new(),
            pending_condolations: Vec::new(),
            next_sequence_id: 1,
            next_element_id: 1,
            halt_pending: false,
        }
    }

    /// Rebuild the actor element indexes from `sequences`.  This is
    /// still useful after older save loads and defensive repair paths.
    /// `sequences` itself is serialized, and `BTreeMap` preserves ids
    /// across cleanup, so no index-shift rebuild is needed on the
    /// cleanup path.
    pub fn rebuild_indices(&mut self) {
        self.actor_live.clear();
        self.actor_in_progress.clear();
        self.actor_instructing.clear();
        for (seq_id, seq) in &self.sequences {
            for (elem_idx, elem) in seq.elements.iter().enumerate() {
                let Some(owner) = elem.owner else {
                    continue;
                };
                let elem_ref = SequenceElementRef::new(*seq_id, elem_idx);
                if Self::is_actor_live_state(elem.state) {
                    self.actor_live.entry(owner).or_default().insert(elem_ref);
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

    #[cfg(test)]
    pub(crate) fn is_registered_to_go(&self, seq_id: SequenceId, elem_idx: usize) -> bool {
        self.elements_to_go.contains(&(seq_id, elem_idx))
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
        sequence.launch();

        // Start the first batch of elements
        let to_go = sequence.next_elements_go();

        self.sequences.insert(id, sequence);
        self.index_sequence_actor_refs(id);

        // Register elements for dispatch
        for (elem_idx, is_wait) in to_go {
            if is_wait {
                self.register_wait_element_to_go(id, elem_idx);
            } else {
                self.register_element_to_go(id, elem_idx);
            }
        }

        id
    }

    /// Launch a single sequence element by wrapping it in a new sequence.
    pub fn launch_element(&mut self, mut element: SequenceElement) -> SequenceId {
        element.command_level = 1;
        let mut seq = Sequence::new();
        seq.append_element(element);
        self.launch_sequence(seq)
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
        if let Some(elem_ref) = self
            .actor_instructing
            .get(&actor)
            .and_then(|stack| stack.last())
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

    /// Select an incoming element while the outgoing element's synchronous
    /// interruption callback runs. Nested Instruct calls use a stack because
    /// Original temporarily replaces the actor pointer at each recursive
    /// boundary.
    pub(crate) fn begin_instruct_callback(
        &mut self,
        owner: EntityId,
        sequence_id: SequenceId,
        element_index: usize,
    ) {
        self.actor_instructing
            .entry(owner)
            .or_default()
            .push(SequenceElementRef::new(sequence_id, element_index));
    }

    /// Close a matching [`Self::begin_instruct_callback`] boundary.
    pub(crate) fn end_instruct_callback(
        &mut self,
        owner: EntityId,
        sequence_id: SequenceId,
        element_index: usize,
    ) {
        let expected = SequenceElementRef::new(sequence_id, element_index);
        let stack = self
            .actor_instructing
            .get_mut(&owner)
            .unwrap_or_else(|| panic!("missing Instruct callback selection for {owner:?}"));
        let selected = stack
            .pop()
            .expect("Instruct callback selection stack is empty");
        assert_eq!(
            selected, expected,
            "Instruct callback selection closed out of order"
        );
        if stack.is_empty() {
            self.actor_instructing.remove(&owner);
        }
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
                self.pending_synchronous_actions.push_back(action);
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
        self.pending_synchronous_actions.push_back(action);
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
        while let Some(action) = self.pending_synchronous_actions.pop_front() {
            if matches!(
                action,
                SequenceAction::ExecuteImmediateOwner { .. }
                    | SequenceAction::ExecuteImmediateEngine { .. }
            ) {
                immediate.push(action);
            } else {
                retained.push_back(action);
            }
        }
        self.pending_synchronous_actions = retained;
        immediate
    }

    /// Pop the next synchronous action without disturbing the remainder.
    /// Script-native sequence launch uses this to stop exactly at a re-entrant
    /// SendMessage callback, then continue in order before the outer VM resumes.
    pub fn pop_pending_immediate_action(&mut self) -> Option<SequenceAction> {
        self.pending_synchronous_actions.pop_front()
    }

    /// Remove the first action that Original registration executes inline,
    /// while leaving ordinary manager-Hourglass work queued in order.
    ///
    /// This is used at callbacks that occur outside `SequenceManager::Hourglass`
    /// (notably director completion during Draw and anonymous-timer expiry
    /// after the manager phase). `ExecutedImmediately()` commands and
    /// RHPRIORITY_WAIT successors run inline; other priorities stay queued.
    pub fn pop_pending_registration_inline_action(&mut self) -> Option<SequenceAction> {
        let index = self
            .pending_synchronous_actions
            .iter()
            .position(|action| match action {
                SequenceAction::ExecuteImmediateOwner { .. }
                | SequenceAction::ExecuteImmediateEngine { .. } => true,
                SequenceAction::InstructOwner {
                    sequence_id,
                    element_index,
                    ..
                }
                | SequenceAction::EngineCommand {
                    sequence_id,
                    element_index,
                } => self
                    .get_element(*sequence_id, *element_index)
                    .is_some_and(|element| element.priority == SequencePriority::Wait),
            })?;
        self.pending_synchronous_actions.remove(index)
    }

    pub fn next_pending_immediate_action(&self) -> Option<&SequenceAction> {
        self.pending_synchronous_actions.front()
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
    pub fn take_pending_synchronous_actions(&mut self) -> Vec<SequenceAction> {
        self.pending_synchronous_actions.drain(..).collect()
    }

    /// Restore a parent callback's detached synchronous continuation after a
    /// nested callback has fully returned. Any actions still produced by the
    /// child stay in front, matching the Original's recursive call stack.
    pub fn restore_pending_synchronous_actions(&mut self, continuation: Vec<SequenceAction>) {
        self.pending_synchronous_actions.extend(continuation);
    }

    /// `true` iff there is at least one immediate-dispatch action awaiting
    /// drain, ignoring direct WAIT `Go()` actions in the same stream.
    pub fn has_pending_immediate_actions(&self) -> bool {
        self.pending_synchronous_actions.iter().any(|action| {
            matches!(
                action,
                SequenceAction::ExecuteImmediateOwner { .. }
                    | SequenceAction::ExecuteImmediateEngine { .. }
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

        loop {
            // WAIT Go() and ExecutedImmediately() run at registration,
            // before deferred non-WAIT work reaches the manager hourglass.
            while let Some(action) = self.pending_synchronous_actions.pop_front() {
                actions.push(action);
            }

            let Some(action) = self.pop_deferred_hourglass_action() else {
                break;
            };
            actions.push(action);
        }

        actions
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

    // ─── State change callbacks ─────────────────────────────────

    /// Called by the engine when an element has finished (terminated).
    /// Advances the sequence to the next command level if all elements at
    /// the current level are done.
    pub fn element_terminated(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let Some(seq) = self.sequences.get_mut(&seq_id) else {
            return;
        };

        let effects = seq.set_element_state(
            elem_idx,
            SequenceState::Terminated,
            CascadeFlags::NEXT_LEVEL,
        );

        self.process_effects(seq_id, effects);
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
        let blocked = seq
            .elements
            .get(elem_idx)
            .map(|elem| {
                elem.state == SequenceState::InProgress && elem.priority.is_non_interruptable()
            })
            .unwrap_or(false);
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

        self.process_effects(seq_id, effects);
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
            elem.priority = priority;
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

        self.process_effects(seq_id, effects);
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

        self.process_effects(seq_id, effects);
    }

    /// Hard-interrupt every `InProgress` or `Postponed` sequence
    /// element owned by `actor`, except those in `exempt_seq`.
    ///
    /// Used on death: the graceful `stop_owner` path rewrites an
    /// in-progress movement order to a `TransitionWalking*Waiting*` stop
    /// animation and lets the element keep playing — which is correct
    /// for a live halt but produces a "corpse walks a few more frames"
    /// visual for a dead actor.  Death needs to throw every surviving
    /// sequence away cleanly; only the damage sequence (which just had
    /// the dying/corpse-idle orders pushed onto it) survives so its
    /// `DyingSword` order becomes the actor's current order.
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
            self.process_effects(seq_id, effects);
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
        self.process_effects(seq_id, effects);
    }

    /// Whether the front order on the given element can be interrupted
    /// right now.
    ///
    /// "Over-top-special" orders cannot be cut immediately and are
    /// modeled with `Order::lock_ai`: these orders keep the actor's
    /// execution context locked until their current order finishes,
    /// so arbitration must split/postpone instead of tearing them
    /// down synchronously.
    pub fn can_interrupt_now(&self, seq_id: SequenceId, elem_idx: usize) -> bool {
        let Some(elem) = self.get_element(seq_id, elem_idx) else {
            tracing::warn!(
                ?seq_id,
                elem_idx,
                "can_interrupt_now called for missing sequence element"
            );
            return false;
        };
        let Some(order) = elem.orders.front() else {
            return true;
        };
        !order.lock_ai
    }

    /// Keep only the current/front order on an element.
    ///
    /// Used by the Instruct arbitration path when a current element
    /// cannot be interrupted immediately.  If the element has no
    /// orders there is nothing useful to preserve, so leave it empty
    /// and warn.
    pub fn truncate_to_first_order(&mut self, seq_id: SequenceId, elem_idx: usize) {
        let Some(elem) = self.get_element_mut(seq_id, elem_idx) else {
            tracing::warn!(
                ?seq_id,
                elem_idx,
                "truncate_to_first_order called for missing sequence element"
            );
            return;
        };
        if elem.orders.is_empty() {
            tracing::warn!(
                ?seq_id,
                elem_idx,
                "truncate_to_first_order called on element with no orders"
            );
            return;
        }
        elem.orders.truncate(1);
        elem.num_transition_orders = elem.num_transition_orders.min(elem.orders.len());
    }

    /// Cross-sequence arbitration fallback:
    ///
    /// 1. current element keeps only its front order and continues;
    /// 2. incoming foreign element runs after that front order;
    /// 3. a cloned continuation of the current element runs after the
    ///    foreign element.
    ///
    /// The intended chain is represented using `cross_postponed`.
    pub fn split_and_insert(
        &mut self,
        own_seq: SequenceId,
        own_idx: usize,
        foreign_seq: SequenceId,
        foreign_idx: usize,
    ) {
        let continuation_idx = {
            let Some(seq) = self.sequences.get_mut(&own_seq) else {
                tracing::warn!(
                    ?own_seq,
                    own_idx,
                    "split_and_insert called for missing own sequence"
                );
                return;
            };
            let Some(own) = seq.elements.get(own_idx) else {
                tracing::warn!(
                    ?own_seq,
                    own_idx,
                    "split_and_insert called for missing own element"
                );
                return;
            };
            let mut continuation = own.clone();
            continuation.state = SequenceState::Postponed;
            continuation.postponed_element_index = None;
            continuation.cross_postponed = None;
            continuation.pop_current_order();
            seq.elements.push(continuation);
            seq.elements.len() - 1
        };

        self.truncate_to_first_order(own_seq, own_idx);

        if let Some(own) = self.get_element_mut(own_seq, own_idx) {
            own.cross_postponed = Some((foreign_seq, foreign_idx));
        }
        if let Some(foreign) = self.get_element_mut(foreign_seq, foreign_idx) {
            foreign.cross_postponed = Some((own_seq, continuation_idx));
            foreign.orders.clear();
        }
        self.postpone_element(foreign_seq, foreign_idx);
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
        if let Some(tail) = self.get_element_mut(cur.0, cur.1) {
            tail.cross_postponed = Some(src_next);
        }
        if let Some(src) = self.get_element_mut(src_seq, src_idx) {
            src.cross_postponed = None;
        }
    }

    /// Process effects from a state change.
    fn process_effects(&mut self, seq_id: SequenceId, mut effects: StateChangeEffects) {
        if let Some(card) = effects.condolation.as_mut() {
            card.was_selected = self.current_element_for_actor(card.owner)
                == Some((card.seq_id, usize::from(card.elem_idx)));
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

        self.process_effects_after_condolation(seq_id, effects);
    }

    fn process_effects_after_condolation(
        &mut self,
        seq_id: SequenceId,
        effects: StateChangeEffects,
    ) {
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
            self.process_effects(seq_id, sub_effects);
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
            for (elem_idx, is_wait) in to_go {
                if is_wait {
                    self.register_wait_element_to_go(seq_id, elem_idx);
                } else {
                    self.register_element_to_go(seq_id, elem_idx);
                }
            }
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
    }

    fn resume_postponed_effects(
        &mut self,
        seq_id: SequenceId,
        start_postponed: Option<usize>,
        resume_cross_postponed: Option<(SequenceId, usize)>,
    ) {
        if let Some(postponed_idx) = start_postponed {
            self.register_element_to_go(seq_id, postponed_idx);
        }

        // Release the cross-sequence postponed successor and retain the
        // posture/action snapshot captured by its original Instruct. Original
        // postponement delays execution; it does not reinterpret the command
        // from the blocker's terminal pose.
        if let Some((succ_seq_id, succ_idx)) = resume_cross_postponed
            && let Some(succ_seq) = self.sequences.get_mut(&succ_seq_id)
            && let Some(succ_elem) = succ_seq.elements.get_mut(succ_idx)
            && succ_elem.state == SequenceState::Postponed
        {
            succ_elem.state = SequenceState::Todo;
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
        self.process_effects(seq_id, effects);
        true
    }

    // ─── Cleanup ────────────────────────────────────────────────

    /// Remove completed/interrupted sequences.
    pub fn friday_evening_cleanup(&mut self) {
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
        self.sequences.retain(|_, seq| !seq.is_to_be_deleted());

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
            self.process_effects(*seq_id, effects);
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
    /// Calls [`Sequence::stop_element`] on the live in-progress and postponed
    /// elements owned by `owner` (skipping only the in-progress default wait
    /// element), then runs [`Self::stop_pending_elements`] for the
    /// not-yet-launched queue. `RHElementActor::Stop` calls `Stop` on the
    /// actor's current element, whose postponed chain remains reachable even
    /// while a terminating injury's condolence callback is running. Rust must
    /// therefore include directly cross-postponed actor work as well as the
    /// ordinary same-sequence postponed chain.
    pub fn stop_owner(
        &mut self,
        owner: EntityId,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) {
        // Stop the owner's currently running or postponed element(s). A
        // postponed element can be the actor work hidden underneath the
        // terminating injury whose condolence callback called StopAll.
        // Waiting until that injury's continuation resumes the element is too
        // late: it can then preempt the replacement Wait installed by StopAll.
        let mut targets: Vec<(SequenceId, usize)> = Vec::new();
        for (seq_id, seq) in &self.sequences {
            for (elem_idx, elem) in seq.elements.iter().enumerate() {
                if elem.owner == Some(owner)
                    && matches!(
                        elem.state,
                        SequenceState::InProgress | SequenceState::Postponed
                    )
                    // Only the actor's current default wait is exempt in the
                    // original. Default waits are explicitly stamped with
                    // Wait priority; an authored/scripted Wait is ordinary
                    // stoppable work even while it is in progress.
                    && !(elem.state == SequenceState::InProgress
                        && elem.command == Command::Wait
                        && elem.priority == SequencePriority::Wait)
                {
                    targets.push((*seq_id, elem_idx));
                }
            }
        }
        let mut stopped = Vec::new();
        for (seq_id, elem_idx) in targets {
            let effects_vec = self
                .sequences
                .get_mut(&seq_id)
                .map(|seq| seq.stop_element(elem_idx, stop_priority, resolver))
                .unwrap_or_default();
            for effects in effects_vec {
                self.process_effects(seq_id, effects);
            }
            if self
                .get_element(seq_id, elem_idx)
                .is_some_and(|elem| elem.state == SequenceState::Interrupted)
            {
                stopped.push((seq_id, elem_idx));
            }
        }

        // A directly targeted cross-postponed element is not necessarily
        // reached through its blocker's `Sequence::stop_element` recursion.
        // Remove the now-dead links just as the original Stop method nulls its
        // postponed pointer after recursively interrupting it.
        if !stopped.is_empty() {
            for (seq_id, seq) in &mut self.sequences {
                for elem in &mut seq.elements {
                    if let Some(postponed_idx) = elem.postponed_element_index
                        && stopped.contains(&(*seq_id, postponed_idx))
                    {
                        elem.postponed_element_index = None;
                    }
                    if let Some(cross) = elem.cross_postponed
                        && stopped.contains(&cross)
                    {
                        elem.cross_postponed = None;
                    }
                }
            }
        }

        // Also stop not-yet-launched elements for this owner.
        self.stop_pending_elements(owner, stop_priority, resolver);
    }

    /// Stop not-yet-launched elements for a specific actor up to a priority.
    pub fn stop_pending_elements(
        &mut self,
        owner: EntityId,
        stop_priority: SequencePriority,
        resolver: &dyn Fn(&SequenceElement) -> SequencePriority,
    ) {
        let mut to_remove = Vec::new();

        for i in 0..self.elements_to_go.len() {
            let (seq_id, elem_idx) = self.elements_to_go[i];
            let Some(seq) = self.sequences.get(&seq_id) else {
                continue;
            };
            if elem_idx >= seq.elements.len() {
                continue;
            }
            if seq.elements[elem_idx].owner != Some(owner) {
                continue;
            }

            // Try to stop it
            let effects_vec = self
                .sequences
                .get_mut(&seq_id)
                .map(|seq| seq.stop_element(elem_idx, stop_priority, resolver))
                .unwrap_or_default();
            for effects in effects_vec {
                self.process_effects(seq_id, effects);
            }

            if let Some(seq) = self.sequences.get(&seq_id)
                && seq.elements[elem_idx].state == SequenceState::Interrupted
            {
                to_remove.push(i);
            }
        }

        // Remove stopped elements from the to-go list (in reverse to preserve indices)
        for &idx in to_remove.iter().rev() {
            self.elements_to_go.remove(idx);
        }
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
        let mut to_remove = Vec::new();
        let mut stopped = Vec::new();

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
                .map(|seq| seq.stop_element(elem_idx, stop_priority, resolver))
                .unwrap_or_default();
            for effects in effects_vec {
                self.process_effects(seq_id, effects);
            }

            if let Some(seq) = self.sequences.get(&seq_id)
                && seq.elements[elem_idx].state == SequenceState::Interrupted
            {
                to_remove.push(i);
                stopped.push((seq_id, elem_idx));
            }
        }

        let count = to_remove.len();
        for &idx in to_remove.iter().rev() {
            self.elements_to_go.remove(idx);
        }

        let mut postponed_targets = Vec::new();
        for (seq_id, seq) in &self.sequences {
            for (elem_idx, elem) in seq.elements.iter().enumerate() {
                if elem.owner == Some(owner)
                    && elem.command == command
                    && elem.state == SequenceState::Postponed
                {
                    postponed_targets.push((*seq_id, elem_idx));
                }
            }
        }

        let mut stopped_count = count;
        for (seq_id, elem_idx) in postponed_targets {
            let effects_vec = self
                .sequences
                .get_mut(&seq_id)
                .map(|seq| seq.stop_element(elem_idx, stop_priority, resolver))
                .unwrap_or_default();
            for effects in effects_vec {
                self.process_effects(seq_id, effects);
            }

            if let Some(seq) = self.sequences.get(&seq_id)
                && seq.elements[elem_idx].state == SequenceState::Interrupted
            {
                stopped.push((seq_id, elem_idx));
                stopped_count += 1;
            }
        }

        if !stopped.is_empty() {
            for seq in self.sequences.values_mut() {
                for elem in &mut seq.elements {
                    if let Some(cross) = elem.cross_postponed
                        && stopped.contains(&cross)
                    {
                        elem.cross_postponed = None;
                    }
                }
            }
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

    /// Apply MakeFast to all active/pending movement elements owned by
    /// `entity`. Sets the FAST flag, upgrades the element's action from
    /// walking to running, and rewrites any queued orders whose action
    /// is a walking / start-walking / stop-walking variant to the
    /// running equivalent.
    pub fn make_fast(&mut self, entity: EntityId) {
        for seq in self.sequences.values_mut() {
            if !seq.has_owner(entity) {
                continue;
            }
            for elem in &mut seq.elements {
                if !Self::should_rewrite(elem, entity) {
                    continue;
                }
                make_fast_element(elem);
            }
        }
    }

    /// Set `action` on the element at `(seq_id, elem_idx)` and on every
    /// subsequent element in the same sequence that is still owned by
    /// the same entity (walk stops when the owner changes).  Callers
    /// use this to force every queued movement order for one actor
    /// onto the same animation — e.g. `MakeCrouched`'s fallback.
    pub fn set_action_recursive(&mut self, seq_id: SequenceId, elem_idx: usize, action: OrderType) {
        // The action propagates through the entire postponed sub-chain
        // (and any cross-sequence postponed back-link) — not just the
        // next-chain tail.  Iterative worklist + visited set so we
        // cover postponed and `cross_postponed` transitively without
        // infinite loops.
        let mut visited: HashSet<(SequenceId, usize)> = HashSet::new();
        let mut worklist: Vec<(SequenceId, usize)> = vec![(seq_id, elem_idx)];
        while let Some((sid, start)) = worklist.pop() {
            let Some(seq) = self.sequences.get_mut(&sid) else {
                continue;
            };
            let Some(first) = seq.elements.get(start) else {
                continue;
            };
            let owner = first.owner;
            let len = seq.elements.len();
            for idx in start..len {
                if seq.elements[idx].owner != owner {
                    break;
                }
                if !visited.insert((sid, idx)) {
                    continue;
                }
                seq.elements[idx].set_action(action);
                if let Some(p) = seq.elements[idx].postponed_element_index
                    && !visited.contains(&(sid, p))
                {
                    worklist.push((sid, p));
                }
                if let Some((cs, ci)) = seq.elements[idx].cross_postponed
                    && !visited.contains(&(cs, ci))
                {
                    worklist.push((cs, ci));
                }
            }
        }
    }

    /// Apply MakeSlow to all active/pending movement elements owned by
    /// `entity`. Clears the FAST flag, downgrades running animations to
    /// walking, and rewrites queued transition orders accordingly.
    ///
    /// Symmetric counterpart to [`Self::make_fast`].
    pub fn make_slow(&mut self, entity: EntityId) {
        for seq in self.sequences.values_mut() {
            if !seq.has_owner(entity) {
                continue;
            }
            for elem in &mut seq.elements {
                if !Self::should_rewrite(elem, entity) {
                    continue;
                }
                make_slow_element(elem);
            }
        }
    }

    /// Apply MakeUpright to all active/pending elements owned by
    /// `entity`. Rewrites crouched-movement orders to upright variants
    /// and cancels pending `CrouchDown` sequence elements (their
    /// command is demoted to `Null`).
    pub fn make_upright(&mut self, entity: EntityId) {
        for seq in self.sequences.values_mut() {
            if !seq.has_owner(entity) {
                continue;
            }
            for elem in &mut seq.elements {
                if !Self::should_rewrite(elem, entity) {
                    continue;
                }
                make_upright_element(elem);
            }
        }
    }

    /// Apply MakeCrouched to all active/pending elements owned by
    /// `entity`. Clears the FAST flag, downgrades running/walking
    /// upright orders to crouched, and rewrites posture-transition
    /// orders accordingly.
    pub fn make_crouched(&mut self, entity: EntityId) {
        for seq in self.sequences.values_mut() {
            if !seq.has_owner(entity) {
                continue;
            }
            for elem in &mut seq.elements {
                if !Self::should_rewrite(elem, entity) {
                    continue;
                }
                make_crouched_element(elem);
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
        let mut changed = false;
        let mut to_interrupt: Vec<(SequenceId, usize)> = Vec::new();
        let Some(refs) = self.actor_in_progress.get(&entity) else {
            return false;
        };
        let refs: Vec<SequenceElementRef> = refs.iter().copied().collect();
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
            self.process_effects(seq_id, effects);
            changed = true;
        }
        changed
    }

    /// Whether the given element is owned by `entity` and eligible for
    /// a make_* rewrite (Todo / InProgress / Postponed, never terminal).
    ///
    /// Postponed elements waiting on a blocker need their
    /// `action`/orders rewritten too; otherwise once the blocker
    /// finishes and the postponed element resumes (state → `Todo`) it
    /// executes with its original (un-rewritten) `OrderType`.
    fn should_rewrite(elem: &SequenceElement, entity: EntityId) -> bool {
        elem.owner == Some(entity)
            && matches!(
                elem.state,
                SequenceState::Todo | SequenceState::InProgress | SequenceState::Postponed
            )
    }

    /// Resolve "next element in chain" for
    /// [`Self::is_next_movement`]/`is_next_movement_or_jump`. Follows the
    /// simple case: same sequence, next index, owner must match.
    ///
    /// Post-seek sequences are stored as separate `Sequence`s
    /// registered with the manager; walking them here would
    /// double-count elements that the manager already sees.  Keep
    /// this simple positional walk; the post-seek chain walkers above
    /// traverse the full tree when callers need it.
    fn next_element_in_chain(
        &self,
        seq_id: SequenceId,
        elem_idx: usize,
    ) -> Option<(SequenceId, usize)> {
        let seq = self.get_sequence(seq_id)?;
        let this = seq.elements.get(elem_idx)?;
        let next_idx = elem_idx + 1;
        let next = seq.elements.get(next_idx)?;
        if this.owner == next.owner {
            Some((seq_id, next_idx))
        } else {
            None
        }
    }

    /// Returns `true` when no further "real" sequence element follows
    /// this one — i.e. the sequence is effectively done after this
    /// element finishes.  `Wait` and `AssertPosition` are skipped
    /// (treated as non-actions).
    pub fn is_last_real_action(&self, seq_id: SequenceId, elem_idx: usize) -> bool {
        // A queued postponed element counts as a pending real action.
        // Both the intra-sequence hold (`postponed_element_index`) and
        // the cross-sequence hand-off (`cross_postponed`) are released
        // when this element terminates, so either form means "real
        // action coming" and we must report not-last.
        if let Some(this) = self.get_element(seq_id, elem_idx)
            && (this.postponed_element_index.is_some() || this.cross_postponed.is_some())
        {
            return false;
        }
        let mut cur = elem_idx;
        loop {
            let Some((next_seq, next_idx)) = self.next_element_in_chain(seq_id, cur) else {
                return true;
            };
            let Some(next_elem) = self.get_element(next_seq, next_idx) else {
                return true;
            };
            match next_elem.command {
                Command::Wait | Command::AssertPosition => {
                    cur = next_idx;
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

/// Apply MakeSlow to a single element in-place.
pub fn make_slow_element(elem: &mut SequenceElement) {
    use crate::order::OrderType;

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

/// Apply MakeUpright to a single element in-place. Cancels a pending
/// `CrouchDown` command by demoting it to `Null`.
pub fn make_upright_element(elem: &mut SequenceElement) {
    use crate::order::OrderType;

    // Cancel pending crouch-down.
    if elem.command == Command::CrouchDown {
        elem.command = Command::Null;
    }

    let SequenceElementData::Movement { action, .. } = &mut elem.data else {
        return;
    };
    *action = match *action {
        OrderType::WalkingUpright | OrderType::RunningUpright => *action,
        OrderType::WalkingCrouched => OrderType::WalkingUpright,
        other => other,
    };
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

/// Apply MakeCrouched to a single element in-place.
pub fn make_crouched_element(elem: &mut SequenceElement) {
    use crate::order::OrderType;

    let SequenceElementData::Movement { flags, action, .. } = &mut elem.data else {
        return;
    };
    *flags &= !MoveFlags::FAST;
    *action = match *action {
        OrderType::WalkingCrouched => *action,
        OrderType::WalkingUpright | OrderType::RunningUpright => OrderType::WalkingCrouched,
        other => other,
    };
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

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn make_simple_element(level: u16, cmd: Command, owner: Option<EntityId>) -> SequenceElement {
        SequenceElement::new(level, cmd, owner)
    }

    /// SequenceManager unit tests have no EngineInner owner callback. Resume the
    /// synchronous SetState continuation at the point where that callback would
    /// have returned.
    fn finish_test_condolations(mgr: &mut SequenceManager) {
        loop {
            let pending = mgr.drain_pending_condolations();
            if pending.is_empty() {
                break;
            }
            for dispatch in pending {
                mgr.finish_pending_condolation(dispatch);
            }
        }
    }

    #[test]
    fn sequence_command_level_grouping() {
        let mut seq = Sequence::new();

        // Level 1: two elements (run in parallel)
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));
        seq.append_element(make_simple_element(
            1,
            Command::WaitTimer,
            Some(EntityId::Pc(crate::entity_id::PcId(1))),
        ));

        // Level 2: one element (waits for level 1)
        seq.append_element(make_simple_element(
            2,
            Command::PassDoor,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));

        assert_eq!(seq.len(), 3);
        assert!(!seq.is_empty());
    }

    #[test]
    fn sequence_launch_and_advance() {
        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));
        seq.append_element(make_simple_element(
            1,
            Command::WaitTimer,
            Some(EntityId::Pc(crate::entity_id::PcId(1))),
        ));
        seq.append_element(make_simple_element(
            2,
            Command::PassDoor,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));

        assert!(seq.launch());

        // First call should return both level-1 elements
        let to_go = seq.next_elements_go();
        assert_eq!(to_go.len(), 2);
        assert_eq!(to_go[0].0, 0); // element index 0
        assert_eq!(to_go[1].0, 1); // element index 1
        assert_eq!(seq.running_elements, 2);

        // Simulate first element finishing
        let advance = seq.element_ready();
        assert!(!advance); // still one running

        // Second element finishes
        let advance = seq.element_ready();
        assert!(advance); // all done at this level

        // Next level starts
        let to_go = seq.next_elements_go();
        assert_eq!(to_go.len(), 1);
        assert_eq!(to_go[0].0, 2); // element index 2
    }

    /// The `RecordPlayAnim*` natives at natives/mod.rs:2680-2729 write
    /// `Field::AnimationId` as `FieldValue::Animation(OrderType)` on a
    /// generic sequence element.  The `Command::PlayAnim*` dispatch in
    /// `tick.rs` reads it back out via `get_property` and destructures
    /// the same variant to feed `force_animation`.  Verify the
    /// round-trip end-to-end.
    #[test]
    fn animation_id_property_roundtrip() {
        use crate::order::OrderType;

        let cases = [
            (Command::PlayAnim, OrderType::WaitingUpright),
            (Command::PlayAnimLoop, OrderType::WaitingCrouched),
            (Command::PlayAnimFreeze, OrderType::Taking),
            (Command::PlayAnimFrozen, OrderType::Pointing),
        ];
        for (cmd, anim) in cases {
            let mut elem = SequenceElement::new_generic(1, cmd, None);
            elem.set_property(Field::AnimationId, FieldValue::Animation(anim));
            let got = elem
                .get_property(Field::AnimationId)
                .expect("AnimationId round-trips via get_property");
            match got {
                FieldValue::Animation(a) => assert_eq!(*a, anim, "cmd {cmd:?}"),
                other => panic!("expected FieldValue::Animation, got {other:?}"),
            }
        }
    }

    #[test]
    fn sequence_is_to_be_deleted() {
        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));

        // Todo element → not deletable
        assert!(!seq.is_to_be_deleted());

        // Mark as terminated → deletable
        seq.elements[0].state = SequenceState::Terminated;
        assert!(seq.is_to_be_deleted());
    }

    #[test]
    fn sequence_has_owner() {
        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(5))),
        ));
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(3))),
        ));

        assert!(seq.has_owner(EntityId::Pc(crate::entity_id::PcId(5))));
        assert!(seq.has_owner(EntityId::Pc(crate::entity_id::PcId(3))));
        assert!(!seq.has_owner(EntityId::Pc(crate::entity_id::PcId(99))));

        // Terminated elements don't count
        seq.elements[0].state = SequenceState::Terminated;
        assert!(!seq.has_owner(EntityId::Pc(crate::entity_id::PcId(5))));
    }

    #[test]
    fn state_change_inprogress() {
        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));

        let effects = seq.set_element_state(0, SequenceState::InProgress, CascadeFlags::NEXT_LEVEL);
        assert!(effects.increment_in_progress);
        assert!(!effects.decrement_in_progress);
        assert!(!effects.signal_ready);
    }

    #[test]
    fn state_change_terminated_signals_ready() {
        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));
        // Must first go to InProgress
        seq.set_element_state(0, SequenceState::InProgress, CascadeFlags::NEXT_LEVEL);

        let effects = seq.set_element_state(0, SequenceState::Terminated, CascadeFlags::NEXT_LEVEL);
        assert!(effects.signal_ready);
        assert!(effects.decrement_in_progress);
        assert_eq!(
            effects.notify_owner,
            Some(EntityId::Pc(crate::entity_id::PcId(0)))
        );
    }

    #[test]
    fn state_change_interrupted_cascades() {
        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));
        seq.append_element(make_simple_element(
            2,
            Command::PassDoor,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));

        let effects =
            seq.set_element_state(0, SequenceState::Interrupted, CascadeFlags::NEXT_LEVEL);

        // Should cascade to the next level (element 1)
        assert_eq!(effects.cascade.len(), 1);
        assert_eq!(effects.cascade[0].0, 1); // element index 1
    }

    #[test]
    fn manager_launch_and_hourglass() {
        let mut mgr = SequenceManager::new();

        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));
        seq.append_element(make_simple_element(
            2,
            Command::Turn,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));

        let seq_id = mgr.launch_sequence(seq);

        // hourglass should return an action for the first element
        let actions = mgr.hourglass();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            SequenceAction::InstructOwner {
                owner,
                sequence_id,
                element_index,
            } => {
                assert_eq!(*owner, EntityId::Pc(crate::entity_id::PcId(0)));
                assert_eq!(*sequence_id, seq_id);
                assert_eq!(*element_index, 0);
            }
            other => panic!("expected InstructOwner, got {:?}", other),
        }

        // No more pending
        let actions = mgr.hourglass();
        assert!(actions.is_empty());
    }

    /// Original `RHSequence::NextSequenceElementsGo`
    /// (`original-code/RHsequence.cpp:235-289`) advances the cursor and
    /// running count first, then walks that stable range in element order:
    /// WAIT calls `Go()` inline, NORMAL is registered on the manager FIFO,
    /// and an immediate command executes inside that registration. A WAIT
    /// callback may terminate synchronously, enter `Ready()`, and dispatch a
    /// WAIT successor before the outer launch/callback chain unwinds.
    #[test]
    fn wait_go_is_emitted_at_launch_return_in_registration_order_and_reentrant() {
        let mut mgr = SequenceManager::new();
        let wait_owner = EntityId::Pc(crate::entity_id::PcId(1));

        let mut sequence = Sequence::new();
        let mut normal = make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        );
        normal.priority = SequencePriority::Normal;
        sequence.append_element(normal);

        let mut wait = make_simple_element(1, Command::Wait, Some(wait_owner));
        wait.priority = SequencePriority::Wait;
        sequence.append_element(wait);

        let mut immediate = make_simple_element(1, Command::LockUser, None);
        immediate.priority = SequencePriority::Normal;
        sequence.append_element(immediate);

        let sequence_id = mgr.launch_sequence(sequence);

        let launched = mgr
            .get_sequence(sequence_id)
            .expect("sequence is registered");
        assert!(launched.started);
        assert_eq!(launched.cursor, 3);
        assert_eq!(launched.running_elements, 3);
        assert!(
            launched
                .elements
                .iter()
                .all(|element| element.state == SequenceState::Todo),
            "emitting Go must not invent the callback's state transition"
        );
        assert_eq!(
            mgr.elements_to_go.iter().copied().collect::<Vec<_>>(),
            vec![(sequence_id, 0)],
            "only NORMAL non-immediate work belongs on the hourglass FIFO"
        );

        let synchronous = mgr.take_pending_synchronous_actions();
        assert_eq!(synchronous.len(), 2);
        assert!(matches!(
            synchronous[0],
            SequenceAction::InstructOwner {
                owner,
                sequence_id: action_sequence_id,
                element_index: 1,
            } if owner == wait_owner && action_sequence_id == sequence_id
        ));
        assert!(matches!(
            synchronous[1],
            SequenceAction::ExecuteImmediateEngine {
                sequence_id: action_sequence_id,
                element_index: 2,
            } if action_sequence_id == sequence_id
        ));
        let deferred = mgr.hourglass();
        assert_eq!(deferred.len(), 1);
        assert!(matches!(
            deferred[0],
            SequenceAction::InstructOwner {
                sequence_id: action_sequence_id,
                element_index: 0,
                ..
            } if action_sequence_id == sequence_id
        ));

        // Re-entrant completion: the callback for level 1 reaches Ready(),
        // which must emit the level-2 WAIT into the synchronous stream. It
        // must not fall back onto `elements_to_go` for another hourglass.
        let mut mgr = SequenceManager::new();
        let mut sequence = Sequence::new();
        let mut first = make_simple_element(1, Command::Wait, None);
        first.priority = SequencePriority::Wait;
        sequence.append_element(first);
        let mut successor = make_simple_element(2, Command::Wait, None);
        successor.priority = SequencePriority::Wait;
        sequence.append_element(successor);

        let sequence_id = mgr.launch_sequence(sequence);
        let first_action = mgr.take_pending_synchronous_actions();
        assert!(matches!(
            first_action.as_slice(),
            [SequenceAction::EngineCommand {
                sequence_id: action_sequence_id,
                element_index: 0,
            }] if *action_sequence_id == sequence_id
        ));

        mgr.element_in_progress(sequence_id, 0);
        mgr.element_terminated(sequence_id, 0);

        let successor_action = mgr.take_pending_synchronous_actions();
        assert!(matches!(
            successor_action.as_slice(),
            [SequenceAction::EngineCommand {
                sequence_id: action_sequence_id,
                element_index: 1,
            }] if *action_sequence_id == sequence_id
        ));
        assert!(mgr.elements_to_go.is_empty());
        assert!(mgr.hourglass().is_empty());
        assert_eq!(
            mgr.get_element(sequence_id, 0)
                .expect("first element")
                .state,
            SequenceState::Terminated
        );
        assert_eq!(
            mgr.get_element(sequence_id, 1)
                .expect("successor element")
                .state,
            SequenceState::Todo
        );
    }

    #[test]
    fn manager_wait_priority_go_bypasses_executed_immediately() {
        let mut mgr = SequenceManager::new();
        let owner = EntityId::Pc(crate::entity_id::PcId(0));
        let mut wait = make_simple_element(1, Command::Speak, Some(owner));
        wait.priority = SequencePriority::Wait;

        let seq_id = mgr.launch_element(wait);

        let actions = mgr.take_pending_synchronous_actions();
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0],
            SequenceAction::InstructOwner {
                owner: action_owner,
                sequence_id,
                element_index: 0,
            } if action_owner == owner && sequence_id == seq_id
        ));
        assert!(
            mgr.hourglass().is_empty(),
            "WAIT-priority Go must not remain deferred on elements_to_go"
        );
    }

    #[test]
    fn manager_reentrant_wait_completion_queues_next_level_inline() {
        let mut mgr = SequenceManager::new();
        let owner = EntityId::Pc(crate::entity_id::PcId(0));
        let mut first = make_simple_element(1, Command::Wait, Some(owner));
        first.priority = SequencePriority::Wait;
        let mut second = make_simple_element(2, Command::Wait, Some(owner));
        second.priority = SequencePriority::Wait;
        let mut seq = Sequence::new();
        seq.append_element(first);
        seq.append_element(second);

        let seq_id = mgr.launch_sequence(seq);
        let first_actions = mgr.take_pending_synchronous_actions();
        assert!(matches!(
            first_actions.as_slice(),
            [SequenceAction::InstructOwner {
                sequence_id,
                element_index: 0,
                ..
            }] if *sequence_id == seq_id
        ));

        // Simulate an Instruct callback that starts and completes the first
        // WAIT before the outer synchronous action drain returns. The owner
        // condolation is itself a synchronous boundary; finishing it resumes
        // Ready() and opens level 2 re-entrantly.
        mgr.element_in_progress(seq_id, 0);
        mgr.element_terminated(seq_id, 0);
        finish_test_condolations(&mut mgr);

        let reentrant_actions = mgr.take_pending_synchronous_actions();
        assert!(matches!(
            reentrant_actions.as_slice(),
            [SequenceAction::InstructOwner {
                owner: action_owner,
                sequence_id,
                element_index: 1,
            }] if *action_owner == owner && *sequence_id == seq_id
        ));
        assert!(
            mgr.hourglass().is_empty(),
            "the re-entrant next-level WAIT must not wait for hourglass"
        );
    }

    #[test]
    fn manager_element_terminated_advances() {
        let mut mgr = SequenceManager::new();

        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));
        seq.append_element(make_simple_element(
            2,
            Command::Turn,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));

        let seq_id = mgr.launch_sequence(seq);

        // Drain the first hourglass
        let _ = mgr.hourglass();

        // Mark element 0 as in-progress then terminated
        mgr.element_in_progress(seq_id, 0);
        mgr.element_terminated(seq_id, 0);
        finish_test_condolations(&mut mgr);

        // The next level's element should now be queued
        let actions = mgr.hourglass();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            SequenceAction::InstructOwner { element_index, .. } => assert_eq!(*element_index, 1),
            other => panic!("expected InstructOwner for element 1, got {:?}", other),
        }
    }

    #[test]
    fn finishing_condolation_stops_at_nested_card_before_cascade_continues() {
        let mut mgr = SequenceManager::new();
        let owner = Some(EntityId::Pc(crate::entity_id::PcId(0)));
        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(1, Command::Move, owner));
        seq.append_element(make_simple_element(2, Command::Turn, owner));
        seq.append_element(make_simple_element(3, Command::LookLeft, owner));
        let seq_id = mgr.launch_sequence(seq);

        mgr.element_interrupted(seq_id, 0, CascadeFlags::NEXT_LEVEL);
        let mut pending = mgr.drain_pending_condolations();
        assert_eq!(pending.len(), 1);

        // In RHSequenceElement::SetState, the outer card returns and the
        // cascade enters element 1, whose own SendCondolationCard must run
        // before CASCADE_FOLLOWING is allowed to touch element 2.
        mgr.finish_pending_condolation(pending.remove(0));

        assert_eq!(
            mgr.get_element(seq_id, 1).unwrap().state,
            SequenceState::Interrupted
        );
        assert_eq!(
            mgr.get_element(seq_id, 2).unwrap().state,
            SequenceState::Todo,
            "the nested owner callback is a synchronous boundary in the cascade"
        );
        let nested = mgr.drain_pending_condolations();
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].card.elem_idx, 1);
    }

    #[test]
    fn split_and_insert_preserves_current_front_then_foreign_then_continuation() {
        let mut mgr = SequenceManager::new();

        let mut current = make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        );
        current.priority = SequencePriority::Normal;
        current
            .orders
            .push_back(Order::test_new(OrderType::WalkingUpright, 10.0, 0.0));
        current
            .orders
            .push_back(Order::test_new(OrderType::WalkingUpright, 20.0, 0.0));
        current.orders.front_mut().unwrap().lock_ai = true;
        let current_seq = mgr.launch_element(current);
        mgr.element_in_progress(current_seq, 0);

        let mut foreign = make_simple_element(
            1,
            Command::Turn,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        );
        foreign.priority = SequencePriority::Preference;
        let foreign_seq = mgr.launch_element(foreign);

        assert!(
            !mgr.can_interrupt_now(current_seq, 0),
            "locked current order should use split_and_insert fallback"
        );
        mgr.split_and_insert(current_seq, 0, foreign_seq, 0);

        let current = mgr.get_element(current_seq, 0).unwrap();
        assert_eq!(current.orders.len(), 1);
        assert_eq!(current.cross_postponed, Some((foreign_seq, 0)));

        let foreign = mgr.get_element(foreign_seq, 0).unwrap();
        assert_eq!(foreign.state, SequenceState::Postponed);
        let continuation_ref = foreign
            .cross_postponed
            .expect("foreign should resume current continuation");
        assert_eq!(continuation_ref.0, current_seq);

        let continuation = mgr
            .get_element(continuation_ref.0, continuation_ref.1)
            .expect("continuation clone exists");
        assert_eq!(continuation.state, SequenceState::Postponed);
        assert_eq!(
            continuation.orders.len(),
            1,
            "continuation resumes after the preserved front order"
        );
        assert_eq!(continuation.orders.front().unwrap().target_x, 20.0);
    }

    #[test]
    fn stop_owner_interrupts_actor_work_postponed_by_injury() {
        let mut mgr = SequenceManager::new();
        let owner = EntityId::Soldier(crate::entity_id::SoldierId(7));

        // A preference action is current until an injury postpones it.
        let mut parry = make_simple_element(1, Command::ParrySword, Some(owner));
        parry.priority = SequencePriority::Preference;
        let parry_seq = mgr.launch_element(parry);
        let _ = mgr.hourglass();
        mgr.element_in_progress(parry_seq, 0);
        mgr.postpone_element(parry_seq, 0);

        // During the injury's terminal condolence callback, Actor::Stop sees
        // the injury as current in the original and recursively stops the
        // postponed parry. Model the cross-sequence postponed link explicitly.
        let mut injury = make_simple_element(1, Command::ReceiveSwordDamage, Some(owner));
        injury.priority = SequencePriority::Injury;
        let injury_seq = mgr.launch_element(injury);
        let _ = mgr.hourglass();
        mgr.element_in_progress(injury_seq, 0);
        mgr.get_element_mut(injury_seq, 0).unwrap().cross_postponed = Some((parry_seq, 0));

        mgr.stop_owner(owner, SequencePriority::Preference, &|elem| elem.priority);

        assert_eq!(
            mgr.get_element(injury_seq, 0).unwrap().state,
            SequenceState::InProgress,
            "Preference StopAll must not interrupt the stronger injury callback"
        );
        assert_eq!(
            mgr.get_element(parry_seq, 0).unwrap().state,
            SequenceState::Interrupted,
            "the parry hidden underneath the injury must not resume after StopAll"
        );
        assert_eq!(
            mgr.get_element(injury_seq, 0).unwrap().cross_postponed,
            None,
            "the injury must not retain a resumable link to stopped actor work"
        );
    }

    #[test]
    fn stop_pending_elements_matching_clears_cross_postponed_shoot_bow() {
        let mut mgr = SequenceManager::new();
        let owner = EntityId::Pc(crate::entity_id::PcId(0));

        let mut current = make_simple_element(1, Command::ShootBow, Some(owner));
        current.priority = SequencePriority::Preference;
        current
            .orders
            .push_back(Order::test_new(OrderType::ShootingWithBow, 10.0, 0.0));
        let current_seq = mgr.launch_element(current);
        mgr.element_in_progress(current_seq, 0);

        let mut queued = make_simple_element(1, Command::ShootBow, Some(owner));
        queued.priority = SequencePriority::Preference;
        let queued_seq = mgr.launch_element(queued);

        mgr.get_element_mut(current_seq, 0).unwrap().cross_postponed = Some((queued_seq, 0));
        mgr.postpone_element(queued_seq, 0);

        assert!(mgr.queued_element_exists(owner, Command::ShootBow));

        let resolver = |_elem: &SequenceElement| SequencePriority::Preference;
        let stopped = mgr.stop_pending_elements_matching(
            owner,
            Command::ShootBow,
            SequencePriority::Preference,
            &resolver,
        );

        assert_eq!(stopped, 1);
        assert_eq!(
            mgr.get_element(queued_seq, 0).unwrap().state,
            SequenceState::Interrupted
        );
        assert_eq!(
            mgr.get_element(current_seq, 0).unwrap().cross_postponed,
            None
        );
        assert!(!mgr.queued_element_exists(owner, Command::ShootBow));
    }

    #[test]
    fn truncate_to_first_order_discards_current_tail() {
        let mut mgr = SequenceManager::new();
        let mut elem = make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        );
        elem.orders
            .push_back(Order::test_new(OrderType::WalkingUpright, 10.0, 0.0));
        elem.orders
            .push_back(Order::test_new(OrderType::WalkingUpright, 20.0, 0.0));
        elem.num_transition_orders = 2;
        let seq_id = mgr.launch_element(elem);

        mgr.truncate_to_first_order(seq_id, 0);

        let elem = mgr.get_element(seq_id, 0).unwrap();
        assert_eq!(elem.orders.len(), 1);
        assert_eq!(elem.orders.front().unwrap().target_x, 10.0);
        assert_eq!(elem.num_transition_orders, 1);
    }

    #[test]
    fn manager_friday_evening_cleanup() {
        let mut mgr = SequenceManager::new();

        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));
        let seq_id = mgr.launch_sequence(seq);

        assert_eq!(mgr.sequence_count(), 1);

        // Mark element as terminated
        mgr.element_in_progress(seq_id, 0);
        mgr.element_terminated(seq_id, 0);

        // Now cleanup should remove it
        mgr.friday_evening_cleanup();
        assert_eq!(mgr.sequence_count(), 0);
    }

    #[test]
    fn manager_terminate_sequence() {
        let mut mgr = SequenceManager::new();

        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));
        seq.append_element(make_simple_element(
            2,
            Command::Turn,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));
        let seq_id = mgr.launch_sequence(seq);

        assert!(mgr.terminate_sequence(seq_id));

        // Both elements should be interrupted
        let s = mgr.get_sequence(seq_id).unwrap();
        assert_eq!(s.elements[0].state, SequenceState::Interrupted);
    }

    #[test]
    fn manager_immediate_commands() {
        let mut mgr = SequenceManager::new();

        let mut seq = Sequence::new();
        // LockUser executes immediately via engine
        seq.append_element(make_simple_element(1, Command::LockUser, None));
        let _seq_id = mgr.launch_sequence(seq);

        let actions = mgr.hourglass();
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0],
            SequenceAction::ExecuteImmediateEngine { .. }
        ));
    }

    /// Immediate-class commands must land on `pending_immediate_actions`
    /// synchronously inside `launch_sequence` so engine-side wrappers
    /// can drain them this frame: registration = dispatch.  Only
    /// non-immediate elements ever land on `elements_to_go`.
    #[test]
    fn manager_immediate_action_emitted_at_register_time() {
        let mut mgr = SequenceManager::new();
        assert!(!mgr.has_pending_immediate_actions());

        // Owner-only immediate (Speak) — must land on
        // `pending_immediate_actions` keyed to the owner.
        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Speak,
            Some(EntityId::Pc(crate::entity_id::PcId(7))),
        ));
        let seq_id = mgr.launch_sequence(seq);

        assert!(
            mgr.has_pending_immediate_actions(),
            "Speak should be queued onto pending_immediate_actions at launch_sequence time"
        );
        let actions = mgr.take_pending_immediate_actions();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            SequenceAction::ExecuteImmediateOwner {
                owner,
                sequence_id,
                element_index,
            } => {
                assert_eq!(*owner, EntityId::Pc(crate::entity_id::PcId(7)));
                assert_eq!(*sequence_id, seq_id);
                assert_eq!(*element_index, 0);
            }
            other => panic!("expected ExecuteImmediateOwner for Speak, got {:?}", other),
        }
        assert!(!mgr.has_pending_immediate_actions());

        // Engine-only immediate (LockUser) — must land on
        // `pending_immediate_actions` regardless of owner.
        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(1, Command::LockUser, None));
        let _seq_id = mgr.launch_sequence(seq);
        let actions = mgr.take_pending_immediate_actions();
        assert_eq!(actions.len(), 1);
        assert!(matches!(
            actions[0],
            SequenceAction::ExecuteImmediateEngine { .. }
        ));

        // SendMessage with owner — owner branch. Original:
        // RHsequenceelement.cpp:765-774 invokes the owner callback during
        // registration rather than putting the command in Hourglass.
        let mut seq = Sequence::new();
        seq.append_element(SequenceElement::new_send_message(
            1,
            Some(EntityId::Pc(crate::entity_id::PcId(3))),
            SendMessageCommand::new(41, -2, 3),
        ));
        let _seq_id = mgr.launch_sequence(seq);
        let actions = mgr.take_pending_immediate_actions();
        assert!(matches!(
            actions[0],
            SequenceAction::ExecuteImmediateOwner {
                owner: EntityId::Pc(crate::entity_id::PcId(3)),
                ..
            }
        ));

        // SendMessage without owner — engine branch.
        let mut seq = Sequence::new();
        seq.append_element(SequenceElement::new_send_message(
            1,
            None,
            SendMessageCommand::new(42, 4, -5),
        ));
        let _seq_id = mgr.launch_sequence(seq);
        let actions = mgr.take_pending_immediate_actions();
        assert!(matches!(
            actions[0],
            SequenceAction::ExecuteImmediateEngine { .. }
        ));

        // Non-immediate command (Move) — must NOT land on the
        // immediate queue; only on `elements_to_go` for the next
        // hourglass.
        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));
        let _seq_id = mgr.launch_sequence(seq);
        assert!(!mgr.has_pending_immediate_actions());
        let actions = mgr.hourglass();
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, SequenceAction::InstructOwner { .. })),
            "Move should produce InstructOwner via the hourglass elements_to_go path"
        );
    }

    /// `hourglass` must drain `pending_immediate_actions` before
    /// `elements_to_go` so immediate side effects land before
    /// non-immediate dispatches in the same frame: registration =
    /// dispatch, so the immediate fires synchronously while the
    /// non-immediate is still being queued.
    #[test]
    fn manager_hourglass_drains_immediates_first() {
        let mut mgr = SequenceManager::new();

        // Mix one non-immediate and one immediate at the same level —
        // the non-immediate is registered first, but the immediate
        // should appear first in the action stream.
        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));
        seq.append_element(make_simple_element(1, Command::CameraJumpTo, None));
        let _seq_id = mgr.launch_sequence(seq);

        let actions = mgr.hourglass();
        assert!(actions.len() >= 2);
        assert!(
            matches!(actions[0], SequenceAction::ExecuteImmediateEngine { .. }),
            "first action should be the CameraJumpTo immediate, got {:?}",
            actions[0]
        );
    }

    #[test]
    fn element_orders() {
        let mut elem = SequenceElement::new(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        );

        elem.push_order(Order::test_new(OrderType::WalkingUpright, 100.0, 200.0));
        elem.push_order(Order::test_new(OrderType::Turning, 150.0, 250.0));

        assert_eq!(elem.orders.len(), 2);
        assert_eq!(
            elem.current_order().unwrap().order_type,
            OrderType::WalkingUpright
        );
        assert_eq!(elem.next_order().unwrap().order_type, OrderType::Turning);

        // Proceed to next order
        let next = elem.proceed();
        assert!(next.is_some());
        assert_eq!(next.unwrap().order_type, OrderType::Turning);

        // Proceed past last
        let next = elem.proceed();
        assert!(next.is_none());
        assert!(elem.orders.is_empty());
    }

    #[test]
    fn generic_element_properties() {
        let mut elem = SequenceElement::new_generic(
            1,
            Command::WaitTimer,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        );
        elem.set_property(Field::Timer, FieldValue::Integer(50));

        match elem.get_property(Field::Timer) {
            Some(FieldValue::Integer(50)) => {}
            other => panic!("expected Integer(50), got {:?}", other),
        }
    }

    #[test]
    fn message_command_converts_legacy_fields_without_inventing_defaults() {
        let payload = SendMessageCommand::new(-17, 23, -42);
        let elem = SequenceElement::new_send_message(
            1,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
            payload,
        );

        assert_eq!(
            elem.sequence_command(),
            Ok(SequenceCommand::SendMessage(payload))
        );

        let mut missing = SequenceElement::new_generic(1, Command::SendMessage, None);
        missing.set_property(Field::Message, FieldValue::Integer(7));
        missing.set_property(Field::MessageArgument, FieldValue::Integer(8));
        assert_eq!(
            missing.sequence_command(),
            Err(SequenceInvariantError::MissingLegacyCommandField {
                command: Command::SendMessage,
                field: Field::MessageExtendedArgument,
            })
        );

        let wrong_subtype = SequenceElement::new(1, Command::SendMessage, None);
        assert_eq!(
            wrong_subtype.sequence_command(),
            Err(SequenceInvariantError::LegacyCommandRequiresGenericData {
                command: Command::SendMessage,
            })
        );
    }

    #[test]
    fn checked_order_mutation_preserves_queue_on_invariant_errors() {
        let mut elem = SequenceElement::new(1, Command::Move, None);
        elem.push_order(Order::test_new(OrderType::WalkingUpright, 1.0, 2.0));

        assert_eq!(
            elem.try_push_order(Order::test_new(OrderType::Invalid, 3.0, 4.0)),
            Err(SequenceInvariantError::InvalidOrderAction)
        );
        assert_eq!(elem.orders.len(), 1);

        assert_eq!(
            elem.try_insert_order(2, Order::test_new(OrderType::Turning, 5.0, 6.0),),
            Err(SequenceInvariantError::OrderInsertionOutOfBounds { index: 2, len: 1 })
        );
        assert_eq!(elem.orders.len(), 1);
        assert_eq!(elem.orders[0].target_x, 1.0);
    }

    #[test]
    fn checked_append_rejects_non_contiguous_command_levels() {
        let mut seq = Sequence::new();
        seq.append_element(SequenceElement::new(1, Command::Move, None));

        assert_eq!(
            seq.try_append_element(SequenceElement::new(3, Command::Turn, None)),
            Err(SequenceInvariantError::NonContiguousCommandLevel {
                previous: 1,
                next: 3,
            })
        );
        assert_eq!(seq.len(), 1);
    }

    #[test]
    fn movement_element_speed_factor() {
        let mut elem = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
            OrderType::WalkingUpright,
        );
        assert_eq!(elem.speed_factor(), 1.0);

        elem.set_speed_factor(0.5);
        assert_eq!(elem.speed_factor(), 0.5);
    }

    #[test]
    fn serde_roundtrip() {
        let mut seq = Sequence::new();
        seq.append_element(SequenceElement::new(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(5))),
        ));
        seq.append_element(SequenceElement::new_generic(1, Command::WaitTimer, None));
        seq.append_element(SequenceElement::new_movement(
            2,
            Command::PassDoor,
            Some(EntityId::Pc(crate::entity_id::PcId(1))),
            OrderType::WalkingUpright,
        ));

        let json = serde_json::to_string(&seq).unwrap();
        let back: Sequence = serde_json::from_str(&json).unwrap();

        assert_eq!(back.elements.len(), 3);
        assert_eq!(back.elements[0].command, Command::Move);
        assert_eq!(
            back.elements[0].owner,
            Some(EntityId::Pc(crate::entity_id::PcId(5)))
        );
        assert_eq!(back.elements[1].command, Command::WaitTimer);
        assert!(back.elements[1].data.is_generic());
        assert_eq!(back.elements[2].command, Command::PassDoor);
        assert!(back.elements[2].data.is_movement());
    }

    #[test]
    fn parallel_elements_at_same_level() {
        let mut mgr = SequenceManager::new();

        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(1))),
        ));
        seq.append_element(make_simple_element(
            2,
            Command::Turn,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));

        let seq_id = mgr.launch_sequence(seq);

        // Should get two actions (both level-1 elements)
        let actions = mgr.hourglass();
        assert_eq!(actions.len(), 2);

        // Terminate both
        mgr.element_in_progress(seq_id, 0);
        mgr.element_in_progress(seq_id, 1);
        mgr.element_terminated(seq_id, 0);
        finish_test_condolations(&mut mgr);

        // Level 2 not yet started — one still running
        let actions = mgr.hourglass();
        assert!(actions.is_empty());

        mgr.element_terminated(seq_id, 1);
        finish_test_condolations(&mut mgr);

        // Now level 2 should start
        let actions = mgr.hourglass();
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn element_about_to_be_launched() {
        let mut mgr = SequenceManager::new();

        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));
        let _seq_id = mgr.launch_sequence(seq);

        assert!(mgr.element_is_about_to_be_launched(
            EntityId::Pc(crate::entity_id::PcId(0)),
            Command::Move
        ));
        assert!(mgr.element_is_about_to_be_launched(
            EntityId::Pc(crate::entity_id::PcId(0)),
            Command::Null
        ));
        assert!(!mgr.element_is_about_to_be_launched(
            EntityId::Pc(crate::entity_id::PcId(1)),
            Command::Move
        ));
        assert!(!mgr.element_is_about_to_be_launched(
            EntityId::Pc(crate::entity_id::PcId(0)),
            Command::Turn
        ));
    }

    #[test]
    fn cancel_pending_move_commands() {
        let mut mgr = SequenceManager::new();

        let mut seq = Sequence::new();
        seq.append_element(make_simple_element(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));
        seq.append_element(make_simple_element(
            1,
            Command::Turn,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        ));
        let _seq_id = mgr.launch_sequence(seq);

        mgr.cancel_pending_move_commands(EntityId::Pc(crate::entity_id::PcId(0)));

        // Only Turn should remain (Move was cancelled)
        let actions = mgr.hourglass();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            SequenceAction::InstructOwner { element_index, .. } => assert_eq!(*element_index, 1),
            other => panic!("expected element 1, got {:?}", other),
        }
    }

    // ──────────────────────────────────────────────────────────
    //  Movement transition rewriters
    // ──────────────────────────────────────────────────────────

    fn movement_elem(owner: EntityId, action: OrderType) -> SequenceElement {
        SequenceElement::new_movement(1, Command::Move, Some(owner), action)
    }

    #[test]
    fn make_fast_rewrites_walking_orders_to_running() {
        let mut elem = movement_elem(
            EntityId::Pc(crate::entity_id::PcId(0)),
            OrderType::WalkingUpright,
        );
        elem.push_order(Order::test_new(OrderType::WalkingUpright, 0.0, 0.0));
        elem.push_order(Order::test_new(
            OrderType::TransitionWaitingUprightWalkingUpright,
            0.0,
            0.0,
        ));
        elem.push_order(Order::test_new(
            OrderType::TransitionWalkingUprightWaitingUpright,
            0.0,
            0.0,
        ));

        make_fast_element(&mut elem);

        let SequenceElementData::Movement { flags, action, .. } = &elem.data else {
            panic!("movement variant");
        };
        assert!(flags.contains(MoveFlags::FAST));
        assert_eq!(*action, OrderType::RunningUpright);
        for o in &elem.orders {
            assert_eq!(o.order_type, OrderType::RunningUpright);
        }
    }

    #[test]
    fn make_fast_preserves_unrelated_orders() {
        let mut elem = movement_elem(
            EntityId::Pc(crate::entity_id::PcId(0)),
            OrderType::WalkingUpright,
        );
        elem.push_order(Order::test_new(OrderType::Turning, 0.0, 0.0));
        elem.push_order(Order::test_new(OrderType::WalkingWithSword, 0.0, 0.0));

        make_fast_element(&mut elem);

        assert_eq!(elem.orders[0].order_type, OrderType::Turning);
        assert_eq!(elem.orders[1].order_type, OrderType::RunningWithSword);
    }

    #[test]
    fn make_slow_is_symmetric_to_make_fast() {
        let mut elem = movement_elem(
            EntityId::Pc(crate::entity_id::PcId(0)),
            OrderType::RunningUpright,
        );
        if let SequenceElementData::Movement { flags, .. } = &mut elem.data {
            *flags |= MoveFlags::FAST;
        }
        elem.push_order(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
        elem.push_order(Order::test_new(OrderType::RunningWithSword, 0.0, 0.0));
        elem.push_order(Order::test_new(
            OrderType::TransitionWaitingUprightRunningUpright,
            0.0,
            0.0,
        ));
        elem.push_order(Order::test_new(
            OrderType::TransitionRunningUprightWaitingUpright,
            0.0,
            0.0,
        ));

        make_slow_element(&mut elem);

        let SequenceElementData::Movement { flags, action, .. } = &elem.data else {
            panic!("movement variant");
        };
        assert!(!flags.contains(MoveFlags::FAST));
        assert_eq!(*action, OrderType::WalkingUpright);
        assert_eq!(elem.orders[0].order_type, OrderType::WalkingUpright);
        assert_eq!(elem.orders[1].order_type, OrderType::WalkingWithSword);
        assert_eq!(elem.orders[2].order_type, OrderType::WalkingUpright);
        assert_eq!(elem.orders[3].order_type, OrderType::WalkingUpright);
    }

    #[test]
    fn make_upright_rewrites_crouched_orders() {
        let mut elem = movement_elem(
            EntityId::Pc(crate::entity_id::PcId(0)),
            OrderType::WalkingCrouched,
        );
        elem.push_order(Order::test_new(OrderType::WalkingCrouched, 0.0, 0.0));
        elem.push_order(Order::test_new(
            OrderType::TransitionWaitingCrouchedWalkingCrouched,
            0.0,
            0.0,
        ));
        elem.push_order(Order::test_new(
            OrderType::TransitionWalkingCrouchedWaitingCrouched,
            0.0,
            0.0,
        ));

        make_upright_element(&mut elem);

        let SequenceElementData::Movement { action, .. } = &elem.data else {
            panic!("movement variant");
        };
        assert_eq!(*action, OrderType::WalkingUpright);
        for o in &elem.orders {
            assert_eq!(o.order_type, OrderType::WalkingUpright);
        }
    }

    #[test]
    fn make_upright_cancels_pending_crouch_down() {
        let mut elem = SequenceElement::new(
            1,
            Command::CrouchDown,
            Some(EntityId::Pc(crate::entity_id::PcId(0))),
        );
        make_upright_element(&mut elem);
        assert_eq!(elem.command, Command::Null);
    }

    #[test]
    fn make_crouched_rewrites_upright_orders_and_clears_fast() {
        let mut elem = movement_elem(
            EntityId::Pc(crate::entity_id::PcId(0)),
            OrderType::RunningUpright,
        );
        if let SequenceElementData::Movement { flags, .. } = &mut elem.data {
            *flags |= MoveFlags::FAST;
        }
        elem.push_order(Order::test_new(OrderType::RunningUpright, 0.0, 0.0));
        elem.push_order(Order::test_new(
            OrderType::TransitionWaitingUprightWalkingUpright,
            0.0,
            0.0,
        ));
        elem.push_order(Order::test_new(
            OrderType::TransitionRunningUprightWaitingUpright,
            0.0,
            0.0,
        ));

        make_crouched_element(&mut elem);

        let SequenceElementData::Movement { flags, action, .. } = &elem.data else {
            panic!("movement variant");
        };
        assert!(!flags.contains(MoveFlags::FAST));
        assert_eq!(*action, OrderType::WalkingCrouched);
        for o in &elem.orders {
            assert_eq!(o.order_type, OrderType::WalkingCrouched);
        }
    }

    #[test]
    fn set_action_recursive_walks_sequence() {
        let mut mgr = SequenceManager::new();
        let mut seq = Sequence::new();
        seq.append_element(SequenceElement::new_movement(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(1))),
            OrderType::RunningUpright,
        ));
        seq.append_element(SequenceElement::new_movement(
            2,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(1))),
            OrderType::RunningUpright,
        ));
        // Different owner — should terminate the walk.
        seq.append_element(SequenceElement::new_movement(
            3,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(2))),
            OrderType::RunningUpright,
        ));
        let seq_id = mgr.launch_sequence(seq);

        mgr.set_action_recursive(seq_id, 0, OrderType::WalkingCrouched);

        let s = mgr.get_sequence(seq_id).unwrap();
        for i in 0..2 {
            let SequenceElementData::Movement { action, .. } = s.elements[i].data else {
                panic!("movement variant");
            };
            assert_eq!(action, OrderType::WalkingCrouched);
        }
        // Third element's owner differs — untouched.
        let SequenceElementData::Movement { action, .. } = s.elements[2].data else {
            panic!("movement variant");
        };
        assert_eq!(action, OrderType::RunningUpright);
    }

    #[test]
    fn stop_movement_rewrites_order_and_shortens_only_element_destination() {
        let mut mgr = SequenceManager::new();
        let mut seq = Sequence::new();
        let mut elem = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(1))),
            OrderType::WalkingUpright,
        );
        elem.push_order(Order::test_new(OrderType::WalkingUpright, 100.0, 0.0));
        elem.push_order(Order::test_new(OrderType::WalkingUpright, 200.0, 0.0));
        seq.append_element(elem);
        let seq_id = mgr.launch_sequence(seq);

        // Advance to InProgress so stop_movement_for_owner applies.
        let _ = mgr.hourglass();
        mgr.element_in_progress(seq_id, 0);

        let mut cancellations: Vec<EntityId> = Vec::new();
        let mut next_order_id = 1u32;
        let changed = mgr.stop_movement_for_owner(
            EntityId::Pc(crate::entity_id::PcId(1)),
            crate::coordinates::MapPoint { x: 0.0, y: 0.0 },
            SequencePriority::NonInterruptable,
            &|_| SequencePriority::Normal,
            &mut next_order_id,
            &mut |id| cancellations.push(id),
        );
        assert!(changed);
        let s = mgr.get_sequence(seq_id).unwrap();
        let first = s.elements[0].current_order().unwrap();
        assert_eq!(
            first.order_type,
            OrderType::TransitionWalkingUprightWaitingUpright
        );
        assert_eq!(first.target_x, 100.0);
        let SequenceElementData::Movement { destination, .. } = &s.elements[0].data else {
            panic!("movement data");
        };
        assert!(destination.x <= 10.0 + 0.001);
        // Trailing order should have been dropped.
        assert_eq!(s.elements[0].orders.len(), 1);
        assert!(cancellations.is_empty()); // No MoveWaiting — no cancellation.

        // The generic half of Actor::Stop runs after StopMovement. Original
        // leaves this rewritten movement InProgress so the transition can
        // play; only movement actions without a rewrite arm were interrupted
        // above.
        mgr.stop_owner(
            EntityId::Pc(crate::entity_id::PcId(1)),
            SequencePriority::NonInterruptable,
            &|_| SequencePriority::Normal,
        );
        assert_eq!(
            mgr.get_element(seq_id, 0).unwrap().state,
            SequenceState::InProgress
        );
    }

    #[test]
    fn stop_movement_rewrite_does_not_cancel_path_for_move_waiting() {
        // Path cancellation only fires when the element is pushed to
        // `Interrupted` (default switch branch).  A successful rewrite
        // keeps the element in INPROGRESS and the path request stays
        // alive.
        let mut mgr = SequenceManager::new();
        let mut seq = Sequence::new();
        let mut elem = SequenceElement::new_movement(
            1,
            Command::MoveWaiting,
            Some(EntityId::Pc(crate::entity_id::PcId(1))),
            OrderType::WalkingUpright,
        );
        elem.push_order(Order::test_new(OrderType::WalkingUpright, 100.0, 0.0));
        seq.append_element(elem);
        let seq_id = mgr.launch_sequence(seq);
        let _ = mgr.hourglass();
        mgr.element_in_progress(seq_id, 0);

        let mut cancellations: Vec<EntityId> = Vec::new();
        let mut next_order_id = 1u32;
        mgr.stop_movement_for_owner(
            EntityId::Pc(crate::entity_id::PcId(1)),
            crate::coordinates::MapPoint::default(),
            SequencePriority::NonInterruptable,
            &|_| SequencePriority::Normal,
            &mut next_order_id,
            &mut |id| cancellations.push(id),
        );
        assert!(cancellations.is_empty());
        let s = mgr.get_sequence(seq_id).unwrap();
        assert_eq!(s.elements[0].command, Command::MoveWaiting);
    }

    #[test]
    fn stop_movement_cancels_path_on_interrupt() {
        // With an action that has no waiting-transition variant, the
        // element falls into the default branch and gets interrupted;
        // a MoveWaiting command goes through path cancellation.
        let mut mgr = SequenceManager::new();
        let mut seq = Sequence::new();
        let mut elem = SequenceElement::new_movement(
            1,
            Command::MoveWaiting,
            Some(EntityId::Pc(crate::entity_id::PcId(1))),
            OrderType::Turning,
        );
        elem.push_order(Order::test_new(OrderType::Turning, 100.0, 0.0));
        seq.append_element(elem);
        let seq_id = mgr.launch_sequence(seq);
        let _ = mgr.hourglass();
        mgr.element_in_progress(seq_id, 0);

        let mut cancellations: Vec<EntityId> = Vec::new();
        let mut next_order_id = 1u32;
        mgr.stop_movement_for_owner(
            EntityId::Pc(crate::entity_id::PcId(1)),
            crate::coordinates::MapPoint::default(),
            SequencePriority::NonInterruptable,
            &|_| SequencePriority::Normal,
            &mut next_order_id,
            &mut |id| cancellations.push(id),
        );
        assert_eq!(cancellations, vec![EntityId::Pc(crate::entity_id::PcId(1))]);
        let s = mgr.get_sequence(seq_id).unwrap();
        assert_eq!(s.elements[0].command, Command::Move);
        assert_eq!(s.elements[0].state, SequenceState::Interrupted);
    }

    #[test]
    fn stop_movement_interrupts_element_with_unknown_action() {
        let mut mgr = SequenceManager::new();
        let mut seq = Sequence::new();
        let mut elem = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(1))),
            OrderType::Turning,
        );
        elem.push_order(Order::test_new(OrderType::Turning, 100.0, 0.0));
        seq.append_element(elem);
        let seq_id = mgr.launch_sequence(seq);
        let _ = mgr.hourglass();
        mgr.element_in_progress(seq_id, 0);

        let mut cancellations: Vec<EntityId> = Vec::new();
        let mut next_order_id = 1u32;
        mgr.stop_movement_for_owner(
            EntityId::Pc(crate::entity_id::PcId(1)),
            crate::coordinates::MapPoint::default(),
            SequencePriority::NonInterruptable,
            &|_| SequencePriority::Normal,
            &mut next_order_id,
            &mut |id| cancellations.push(id),
        );
        let s = mgr.get_sequence(seq_id).unwrap();
        assert_eq!(s.elements[0].state, SequenceState::Interrupted);
    }

    #[test]
    fn insert_transition_start_splits_long_walking_order() {
        let mut elem = movement_elem(
            EntityId::Pc(crate::entity_id::PcId(0)),
            OrderType::WalkingUpright,
        );
        // Single walking order 100 units along +x.
        elem.push_order(Order::test_new(OrderType::WalkingUpright, 100.0, 0.0));

        let mut next_order_id = 1u32;
        let inserted = elem.insert_transition_start(
            OrderType::TransitionWaitingUprightWalkingUpright,
            OrderType::WalkingUpright,
            10.0,
            crate::coordinates::MapPoint { x: 0.0, y: 0.0 },
            &mut next_order_id,
        );

        assert!(inserted);
        assert_eq!(elem.orders.len(), 2);
        assert_eq!(
            elem.orders[0].order_type,
            OrderType::TransitionWaitingUprightWalkingUpright
        );
        assert!((elem.orders[0].target_x - 10.0).abs() < 0.01);
        assert_eq!(elem.orders[1].order_type, OrderType::WalkingUpright);
    }

    #[test]
    fn insert_transition_start_reports_short_order_relabel() {
        let mut elem = movement_elem(
            EntityId::Pc(crate::entity_id::PcId(0)),
            OrderType::WalkingUpright,
        );
        elem.push_order(Order::test_new(OrderType::WalkingUpright, 5.0, 0.0));

        let mut next_order_id = 1u32;
        let inserted = elem.insert_transition_start(
            OrderType::TransitionWaitingUprightWalkingUpright,
            OrderType::WalkingUpright,
            10.0,
            crate::coordinates::MapPoint { x: 0.0, y: 0.0 },
            &mut next_order_id,
        );

        assert!(
            inserted,
            "an in-place relabel is still a startup transition"
        );
        assert_eq!(elem.orders.len(), 1);
        assert_eq!(
            elem.orders[0].order_type,
            OrderType::TransitionWaitingUprightWalkingUpright
        );
    }

    #[test]
    fn pop_current_order_shrinks_remaining_transition_prefix() {
        let mut elem = movement_elem(
            EntityId::Pc(crate::entity_id::PcId(0)),
            OrderType::WalkingUpright,
        );
        elem.push_order(Order::test_new(
            OrderType::TransitionWaitingCapeWaitingUpright,
            0.0,
            0.0,
        ));
        elem.push_order(Order::test_new(OrderType::WalkingUpright, 10.0, 0.0));
        elem.num_transition_orders = 1;

        elem.pop_current_order().expect("transition order");
        assert_eq!(elem.num_transition_orders, 0);
        assert_eq!(
            elem.current_order().map(|order| order.order_type),
            Some(OrderType::WalkingUpright)
        );
    }

    #[test]
    fn insert_transition_end_appends_transition_before_last_order() {
        let mut elem = movement_elem(
            EntityId::Pc(crate::entity_id::PcId(0)),
            OrderType::WalkingUpright,
        );
        elem.push_order(Order::test_new(OrderType::WalkingUpright, 0.0, 0.0));
        elem.push_order(Order::test_new(OrderType::WalkingUpright, 100.0, 0.0));

        let mut next_order_id = 1u32;
        elem.insert_transition_end(
            OrderType::TransitionWalkingUprightWaitingUpright,
            OrderType::WalkingUpright,
            10.0,
            crate::coordinates::MapPoint { x: 0.0, y: 0.0 },
            1.0,
            &mut next_order_id,
        );

        // The last WalkingUpright order is relabelled to the transition,
        // and a new WalkingUpright order is inserted in front of it,
        // ~10 units back from (100,0) toward (0,0).
        assert_eq!(elem.orders.len(), 3);
        assert_eq!(elem.orders[0].order_type, OrderType::WalkingUpright);
        assert_eq!(elem.orders[1].order_type, OrderType::WalkingUpright);
        assert!((elem.orders[1].target_x - 90.0).abs() < 0.5);
        assert_eq!(
            elem.orders[2].order_type,
            OrderType::TransitionWalkingUprightWaitingUpright
        );
    }

    #[test]
    fn cleanup_duplicate_orders_removes_consecutive_matches() {
        let mut elem = movement_elem(
            EntityId::Pc(crate::entity_id::PcId(0)),
            OrderType::WalkingUpright,
        );
        elem.push_order(Order::test_new(OrderType::WalkingUpright, 10.0, 10.0));
        elem.push_order(Order::test_new(OrderType::WalkingUpright, 10.0, 10.0));
        elem.push_order(Order::test_new(OrderType::WalkingUpright, 20.0, 20.0));
        elem.push_order(Order::test_new(OrderType::WalkingUpright, 20.0, 20.0));

        elem.cleanup_duplicate_orders();

        assert_eq!(elem.orders.len(), 2);
        assert_eq!(elem.orders[0].target_x, 10.0);
        assert_eq!(elem.orders[1].target_x, 20.0);
    }

    #[test]
    fn is_next_movement_detects_same_owner_chain() {
        let mut mgr = SequenceManager::new();
        let mut seq = Sequence::new();
        seq.append_element(SequenceElement::new_movement(
            1,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(1))),
            OrderType::WalkingUpright,
        ));
        seq.append_element(SequenceElement::new_movement(
            2,
            Command::Move,
            Some(EntityId::Pc(crate::entity_id::PcId(1))),
            OrderType::WalkingUpright,
        ));
        seq.append_element(SequenceElement::new(
            3,
            Command::JumpCmd,
            Some(EntityId::Pc(crate::entity_id::PcId(1))),
        ));
        let seq_id = mgr.launch_sequence(seq);

        assert!(mgr.is_next_movement(seq_id, 0));
        assert!(!mgr.is_next_movement(seq_id, 1)); // next is Jump (Simple) — not movement
        assert!(mgr.is_next_movement_or_jump(seq_id, 1));
        assert!(!mgr.is_next_movement(seq_id, 2)); // last element — nothing next
    }

    #[test]
    fn non_interruptable_impossible_guard_only_protects_in_progress_owner() {
        let owner = EntityId::Pc(crate::entity_id::PcId(1));
        let mut mgr = SequenceManager::new();

        let mut todo = SequenceElement::new(1, Command::LeaveListen, Some(owner));
        todo.priority = SequencePriority::NonInterruptable;
        let todo_seq = mgr.launch_element(todo);
        mgr.element_impossible(todo_seq, 0);
        assert_eq!(
            mgr.get_element(todo_seq, 0).unwrap().state,
            SequenceState::Impossible,
            "preflight failure must reject a Todo non-interruptable element"
        );

        let mut active = SequenceElement::new(1, Command::EnterListen, Some(owner));
        active.priority = SequencePriority::NonInterruptable;
        let active_seq = mgr.launch_element(active);
        mgr.element_in_progress(active_seq, 0);
        mgr.element_impossible(active_seq, 0);
        assert_eq!(
            mgr.get_element(active_seq, 0).unwrap().state,
            SequenceState::InProgress,
            "an executing non-interruptable owner remains protected"
        );
    }
}
