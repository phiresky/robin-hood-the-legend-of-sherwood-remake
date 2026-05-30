//! Floating text / indicator system ("titbits").
//!
//! Titbits are small visual indicators that float above game entities:
//! unconscious stars, emoticons, quick-action icons, damage counters,
//! particle effects (smoke, dust, water splashes), hidden-character icons, etc.
//!
//! This module owns the **data model and update logic**; rendering
//! (`Refresh`, `RenderText`, sprite decompression) is handled separately
//! since it is tightly coupled to the draw manager and sprite system.

use serde::{Deserialize, Serialize};

use crate::coordinates::WorldPoint3D;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const COUNTER_LIMIT: u16 = 25;
/// Default per-frame delay for titbit sprite animations.
/// All titbit rows use a default delay of 2, meaning frames advance
/// every 3 calls (`frame_count > 2`).
const TITBIT_FRAME_DELAY: u16 = 2;
const TIME_BLINK_ON: u32 = 15;
const TIME_BLINK_OFF: u32 = 5;
/// Public re-export of `TIME_BLINK_OFF` for the renderer.
pub const TIME_BLINK_OFF_RAW: u32 = TIME_BLINK_OFF;
pub const DISTANCE_DOT: f32 = 10.0;
pub const GHOST_BLINK: u32 = 10;
pub const NUMBER_OF_QA_MEMORY: usize = 3;
pub const INVALID_ID: u32 = 0xFFFF_FFFF;

// ---------------------------------------------------------------------------
// TitbitId — nominal newtype for titbit identifiers
// ---------------------------------------------------------------------------

/// Unique titbit identifier.  Newtype around `NonMaxU32` so
/// `Option<TitbitId>` niche-optimizes to 4 bytes — the `0xFFFF_FFFF`
/// [`INVALID_ID`] sentinel literally cannot sit in a real id.
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
pub struct TitbitId(pub nonmax::NonMaxU32);

impl TitbitId {
    #[inline]
    pub fn new(v: u32) -> Option<Self> {
        nonmax::NonMaxU32::new(v).map(Self)
    }
    #[inline]
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

impl From<TitbitId> for u32 {
    #[inline]
    fn from(id: TitbitId) -> u32 {
        id.get()
    }
}
impl From<TitbitId> for usize {
    #[inline]
    fn from(id: TitbitId) -> usize {
        id.get() as usize
    }
}

impl std::fmt::Display for TitbitId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(f)
    }
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// The kind of floating indicator.
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
#[repr(u32)]
pub enum TitbitKind {
    GunImpact = 0,
    UnconsciousStar,
    WeakStunned,
    QuickAction,
    Counter,
    Smoke,
    Dust,
    Water,
    Lock,
    Emoticon,
    DangerPoint,
    Plouf,
    Ghost,
    AppleSmell,
    Speak,
    Hidden,
    WorkIcon,
    QuickActionRun,
}

/// Quick-action icon type shown above characters.
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
#[repr(u32)]
pub enum QuickAction {
    Walk = 0,
    Run,
    PlusQuick,
    Peanut,
    Up,
    Down,
    Barrel,
    Default,
    Ale,
    SwordFight,
    Stone,
    Execute,
    Hit,
    BowCivilian,
    BowOk,
    BowOut,
    BowVip,
    BowLongCivilian,
    BowLongOk,
    BowLongVip,
    LockPick,
    Shield,
    Duel,
    Ladder,
    Listen,
    Strangle,
    Net,
    Finish,
    ClimbOnShoulders,
    Wasp,
    Purse,
    Lever,
    Tie,
    Eat,
    Beggar,
    Obstacle,
    GiveMoney,
    Apple,
    Portal,
    Take,
    WakeUp,
    JumpDown,
    JumpUp,
    Shield2,
    Whistle,
    Heal,
    Bow,
    TargetHandled,
    TargetHit,
    InteractNpc,
    Speak,
    Search,
    HelpClimb,
    InteractPc,
    TargetFoot,
}

/// Sprite row indices for the titbit sprite sheet.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
#[repr(u16)]
pub enum SpriteRow {
    Impact = 0,
    OneStar,
    TwoStars,
    ThreeStars,
    FourStars,
    FiveStars,
    QuickActionTitbits,
    Smoke,
    Water,
    Lock,
    EmoticonGrowingQMark,
    EmoticonQMark,
    EmoticonXMark,
    EmoticonZzz,
    EmoticonThunderstorm,
    EmoticonCloud,
    EmoticonDrunken,
    EmoticonSun,
    EmoticonKo,
    Plouf,
    Ghost,
    AppleSmell,
    Speak,
    DangerPoint,
    Hidden,
    WorkIconArrows,
    WorkIconPurses,
    WorkIconStones,
    WorkIconApples,
    WorkIconBeer,
    WorkIconLegs,
    WorkIconPlants,
    WorkIconNets,
    WorkIconWasps,
    WorkIconBowTraining,
    WorkIconSwordTraining,
    WorkIconRegeneration,
    NumberOfRows,
}

/// Character indices for the "hidden in disguise" titbit.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
#[repr(u16)]
pub enum HiddenCharacter {
    Robin = 0,
    LittleJohn,
    Scarlet,
    Tuck,
    Stuteley,
    Marian,
    MerryManA,
    MerryManB,
    MerryManC,
}

impl HiddenCharacter {
    /// Pick the disguise sprite frame for a PC entering a HIDDEN titbit.
    /// Robin picks its own row, everyone else is disambiguated by the PC
    /// profile's filename.  Unknown overlay PCs fall back to the generic
    /// Merry Man frame until hackable per-profile titbit rows exist.
    pub fn for_pc(is_robin: bool, profile_filename: &str) -> Self {
        if is_robin {
            return Self::Robin;
        }
        // ASCII-case-insensitive to tolerate data-file variation,
        // matching the is_robin check in `engine/level_loading.rs`.
        if profile_filename.eq_ignore_ascii_case("LittleJohn") {
            Self::LittleJohn
        } else if profile_filename.eq_ignore_ascii_case("WillScarlet") {
            Self::Scarlet
        } else if profile_filename.eq_ignore_ascii_case("Friar Tuck") {
            Self::Tuck
        } else if profile_filename.eq_ignore_ascii_case("Stuteley") {
            Self::Stuteley
        } else if profile_filename.eq_ignore_ascii_case("LadyMarian") {
            Self::Marian
        } else if profile_filename.eq_ignore_ascii_case("MerryManA") {
            Self::MerryManA
        } else if profile_filename.eq_ignore_ascii_case("MerryManB") {
            Self::MerryManB
        } else if profile_filename.eq_ignore_ascii_case("MerryManC") {
            Self::MerryManC
        } else {
            tracing::trace!(
                "HiddenCharacter::for_pc: unknown PC profile filename {profile_filename:?}; \
                 using MerryManC fallback"
            );
            // TODO: let hackable character manifests define titbit rows.
            Self::MerryManC
        }
    }

    /// Phase value passed to `TitbitManager::add_titbit`; indexes into
    /// the hidden-titbit sprite row.
    #[inline]
    pub fn to_phase(self) -> u16 {
        self as u16
    }
}

/// Work icons displayed above PCs in Sherwood camp.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
#[repr(u32)]
pub enum WorkIcon {
    Arrows = 0,
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
    None,
}

// ---------------------------------------------------------------------------
// Opaque handle for game elements
// ---------------------------------------------------------------------------

/// Opaque handle referencing a game element (entity index).
///
/// [`INVALID`](ElementHandle::INVALID) is the null sentinel.
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
pub struct ElementHandle(pub u32);

impl ElementHandle {
    pub const INVALID: Self = Self(INVALID_ID);

    #[inline]
    pub fn is_valid(self) -> bool {
        self != Self::INVALID
    }
}

// ---------------------------------------------------------------------------
// TitbitInfo — one floating indicator
// ---------------------------------------------------------------------------

/// Data for a single floating indicator.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct TitbitInfo {
    pub kind: TitbitKind,
    pub phase: u16,

    pub sprite_row: u16,
    pub sprite_frame: u16,
    pub frame_count: u16,

    /// Entity this titbit is attached to / draws info from.
    pub element_supplier: ElementHandle,
    /// PC actor that manages this titbit (quick-action chain).
    pub element_manager: ElementHandle,

    pub layer: u16,
    pub position: WorldPoint3D,
    pub display_order: f32,

    pub blinking: bool,
    pub id: u32,
}

impl PartialEq for TitbitInfo {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.kind == other.kind
    }
}

impl PartialOrd for TitbitInfo {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.display_order.partial_cmp(&other.display_order)
    }
}

// ---------------------------------------------------------------------------
// TitbitManager
// ---------------------------------------------------------------------------

/// Manages all floating titbit indicators in the game world.
///
/// This struct owns the data and the per-frame update logic; rendering
/// is handled separately.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct TitbitManager {
    /// The live titbit list, kept sorted by `display_order`.
    titbits: Vec<TitbitInfo>,
    /// Monotonically increasing ID counter (serialized).
    current_id: u32,

    // -- Runtime state --
    //
    // The blink counter and dotted-line phase are kept in serde so
    // snapshots carry the active display phase — this manager is
    // engine-owned and drives deterministic titbit visibility / line
    // phase.
    blink_counter: u32,
    dotted_start: f32,
    current_index: u16,

    /// Frame counts per sprite row, indexed by `SpriteRow` discriminant.
    /// Populated from `TitbitRenderer::row_frame_counts()` after sprite load.
    row_frame_counts: Vec<u16>,
}

impl Default for TitbitManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TitbitManager {
    pub fn new() -> Self {
        Self {
            titbits: Vec::new(),
            current_id: 0,
            blink_counter: TIME_BLINK_ON,
            dotted_start: 0.0,
            current_index: 0,
            row_frame_counts: Vec::new(),
        }
    }

    /// Set per-row frame counts from loaded sprite data.
    /// Called after `TitbitRenderer::load()` with the actual frame counts
    /// extracted from the loaded resources.
    pub fn set_row_frame_counts(&mut self, counts: Vec<u16>) {
        self.row_frame_counts = counts;
    }

    /// Frame count for a sprite row.  Returns 1 if the row hasn't been
    /// loaded (prevents divide-by-zero / zero-limit issues).
    pub fn num_frames_for_row(&self, row: u16) -> u16 {
        self.row_frame_counts
            .get(row as usize)
            .copied()
            .filter(|&n| n > 0)
            .unwrap_or(1)
    }

    // ── Accessors ──

    pub fn titbits(&self) -> &[TitbitInfo] {
        &self.titbits
    }

    pub fn titbits_mut(&mut self) -> &mut Vec<TitbitInfo> {
        &mut self.titbits
    }

    pub fn blink_counter(&self) -> u32 {
        self.blink_counter
    }

    pub fn dotted_start(&self) -> f32 {
        self.dotted_start
    }

    // ── Add / Remove ──

    /// Add a titbit.  Returns its ID, or [`INVALID_ID`] if filtered out.
    ///
    /// `supplier_display_order` and `supplier_layer` should be provided
    /// when the element supplier is known.
    #[allow(clippy::too_many_arguments)]
    pub fn add_titbit(
        &mut self,
        position: WorldPoint3D,
        layer: u16,
        kind: TitbitKind,
        element_supplier: ElementHandle,
        phase: u16,
        element_manager: ElementHandle,
        run: bool,
        forced_id: u32,
        display_titbits_enabled: bool,
        supplier_display_order: Option<f32>,
        supplier_layer: Option<u16>,
    ) -> u32 {
        // Filter particle effects (Smoke / Dust / Water) when the user
        // has disabled them.
        if !display_titbits_enabled
            && matches!(
                kind,
                TitbitKind::Smoke | TitbitKind::Dust | TitbitKind::Water
            )
        {
            return INVALID_ID;
        }

        // Counter with phase 0 is a no-op.
        if kind == TitbitKind::Counter && phase == 0 {
            return INVALID_ID;
        }

        let id = if forced_id != INVALID_ID {
            forced_id
        } else {
            let id = self.current_id;
            self.current_id += 1;
            id
        };

        // Effective layer: prefer supplier's, fall back to parameter.
        let effective_layer = supplier_layer.unwrap_or(if layer as i16 == -1 { 0 } else { layer });

        // Position may be adjusted for gun impacts.
        let mut pos = position;
        let display_order = match kind {
            TitbitKind::GunImpact => {
                pos.z += 0.1;
                position.y + 0.01
            }
            TitbitKind::Smoke | TitbitKind::Dust | TitbitKind::Water | TitbitKind::Plouf => {
                position.y + 0.01
            }
            TitbitKind::Counter => supplier_display_order
                .map(|d| d + 1000.1)
                .unwrap_or(position.y + 1000.1),
            _ => supplier_display_order
                .map(|d| d + 0.01)
                .unwrap_or(position.y + 0.01),
        };

        // Defensive sweep: drop any stale UnconsciousStar titbits for
        // the same supplier before adding a new one.  Without this,
        // re-KO'ing the same actor before the prior stars expire could
        // leave duplicates.
        if kind == TitbitKind::UnconsciousStar && element_supplier.is_valid() {
            self.titbits.retain(|t| {
                !(t.kind == TitbitKind::UnconsciousStar && t.element_supplier == element_supplier)
            });
        }

        let info = TitbitInfo {
            kind,
            phase,
            sprite_row: initial_sprite_row_for_kind(kind),
            sprite_frame: 0,
            frame_count: 0,
            element_supplier,
            element_manager,
            layer: effective_layer,
            position: pos,
            display_order,
            blinking: false,
            id,
        };

        self.titbits.push(info);

        // If the quick-action also has a "run" variant, add that too.
        if run {
            assert_eq!(kind, TitbitKind::QuickAction);
            let mut run_info = self.titbits.last().unwrap().clone();
            run_info.kind = TitbitKind::QuickActionRun;
            run_info.display_order -= 0.001;
            self.titbits.push(run_info);
        }

        id
    }

    /// Remove all titbits of `kind` attached to `element`.
    pub fn remove_titbit(&mut self, kind: TitbitKind, element: ElementHandle) {
        self.titbits
            .retain(|t| !(t.kind == kind && t.element_supplier == element));
    }

    /// Check if a titbit of `kind` exists for `element`.
    pub fn titbit_exists(&self, kind: TitbitKind, element: ElementHandle) -> bool {
        self.titbits
            .iter()
            .any(|t| t.kind == kind && t.element_supplier == element)
    }

    /// Remove every titbit with `titbit_id`.  Low-level primitive used
    /// by `EngineInner::remove_quick_action_titbits_for(pc, slot)`
    /// after the id has been resolved from the PC's QA memory slot.
    /// Returns `true` iff at least one titbit was removed.
    pub fn remove_quick_action_titbits_by_id(&mut self, titbit_id: TitbitId) -> bool {
        let raw = titbit_id.get();
        let before = self.titbits.len();
        self.titbits.retain(|t| t.id != raw);
        self.titbits.len() < before
    }

    /// Remove all lock titbits.
    pub fn remove_lock(&mut self) -> bool {
        let before = self.titbits.len();
        self.titbits.retain(|t| t.kind != TitbitKind::Lock);
        self.titbits.len() < before
    }

    /// Remove unconscious-star titbits for a given element, but only
    /// if that element is no longer unconscious.
    ///
    /// `is_still_unconscious` should be provided by the caller (from
    /// the entity state).
    pub fn remove_unconscious_stars_if(
        &mut self,
        element: ElementHandle,
        is_still_unconscious: bool,
    ) -> bool {
        if is_still_unconscious {
            return false;
        }
        let before = self.titbits.len();
        self.titbits
            .retain(|t| !(t.kind == TitbitKind::UnconsciousStar && t.element_supplier == element));
        self.titbits.len() < before
    }

    /// Add a "plus quick" duplicate of an existing titbit (marks it running).
    pub fn set_run(&mut self, titbit_id: TitbitId, phase: u16) {
        let raw = titbit_id.get();
        // Collect clones first to avoid borrowing issues.
        let to_add: Vec<_> = self
            .titbits
            .iter()
            .filter(|t| t.id == raw && t.phase == phase)
            .map(|t| {
                let mut dup = t.clone();
                dup.phase = QuickAction::PlusQuick as u16;
                dup.display_order += 0.001;
                dup
            })
            .collect();
        self.titbits.extend(to_add);
    }

    /// Get the phase of the first titbit matching `titbit_id`.
    /// Returns `0xFFFF` if not found.
    pub fn get_phase(&self, titbit_id: TitbitId) -> u16 {
        let raw = titbit_id.get();
        self.titbits
            .iter()
            .find(|t| t.id == raw)
            .map(|t| t.phase)
            .unwrap_or(0xFFFF)
    }

    /// Check if a quick-action-run titbit exists for the given ID.
    pub fn is_running_for_qa(&self, titbit_id: TitbitId) -> bool {
        let raw = titbit_id.get();
        self.titbits
            .iter()
            .any(|t| t.id == raw && t.kind == TitbitKind::QuickActionRun)
    }

    // ── Blinking ──

    /// Set blinking state for all titbits matching `titbit_id`.
    pub fn set_blinking(&mut self, titbit_id: TitbitId, blinking: bool) {
        let raw = titbit_id.get();
        for t in &mut self.titbits {
            if t.id == raw {
                t.blinking = blinking;
            }
        }
    }

    /// Reset all titbits to non-blinking.
    pub fn reset_blinking(&mut self) {
        for t in &mut self.titbits {
            t.blinking = false;
        }
    }

    // ── Per-frame logic ──

    /// Prepare for rendering: advance blink/dotted timers, update display
    /// orders from element positions, and sort.
    ///
    /// `get_display_order` is a callback that returns the current
    /// display order for an element handle.
    pub fn prepare_refresh(&mut self, get_display_order: impl Fn(ElementHandle) -> Option<f32>) {
        // Advance blink counter.
        if self.blink_counter == 0 {
            self.blink_counter = TIME_BLINK_ON;
        } else {
            self.blink_counter -= 1;
        }

        // Advance dotted-line animation.
        self.dotted_start += 1.0;
        if self.dotted_start > DISTANCE_DOT {
            self.dotted_start -= DISTANCE_DOT;
        }

        self.current_index = 0;

        // Only the kinds listed in the match below re-anchor to their
        // supplier each frame.  Particle/debug kinds and Counter fall
        // through and keep the display_order assigned at creation time.
        //
        // `require_supplier` flags the kinds (Lock/Hidden/Emoticon/
        // WorkIcon) that always need a valid supplier — warn if one
        // goes missing instead of silently leaving the order stale.
        for t in &mut self.titbits {
            let (offset, require_supplier) = match t.kind {
                TitbitKind::Lock => (-0.01, true),
                TitbitKind::Hidden => (0.01, true),
                TitbitKind::Emoticon | TitbitKind::WorkIcon => (0.01, true),
                TitbitKind::WeakStunned
                | TitbitKind::UnconsciousStar
                | TitbitKind::AppleSmell
                | TitbitKind::Speak => (0.01, false),
                TitbitKind::QuickAction | TitbitKind::QuickActionRun => (-0.01, false),
                _ => continue,
            };
            match get_display_order(t.element_supplier) {
                Some(order) => t.display_order = order + offset,
                None if require_supplier => {
                    tracing::warn!(
                        "titbit prepare_refresh: missing supplier for {:?} (id {}); supplier is required for this kind",
                        t.kind,
                        t.id
                    );
                }
                None => {}
            }
        }

        // Sort by display order (stable sort preserves insertion order for ties).
        self.titbits
            .sort_by(|a, b| a.display_order.partial_cmp(&b.display_order).unwrap());
    }

    /// Per-frame update: advance animations, expire finished titbits.
    ///
    /// Callbacks supply entity queries:
    /// - `is_element_active` — is the referenced element still alive/active?
    /// - `query_state` — returns element-specific state for update decisions.
    pub fn update(&mut self, query: &dyn TitbitUpdateQuery) {
        let mut i = self.titbits.len();
        while i > 0 {
            i -= 1;
            let action = Self::compute_update_action(&self.titbits[i], query, self);
            match action {
                UpdateAction::Keep => {}
                UpdateAction::Remove => {
                    self.titbits.remove(i);
                }
                UpdateAction::Mutate(f) => {
                    f(&mut self.titbits[i]);
                }
            }
        }
    }

    fn compute_update_action(
        t: &TitbitInfo,
        query: &dyn TitbitUpdateQuery,
        mgr: &TitbitManager,
    ) -> UpdateAction {
        match t.kind {
            TitbitKind::GunImpact => {
                if t.phase + 1 >= 6 {
                    UpdateAction::Remove
                } else {
                    UpdateAction::Mutate(Box::new(|t| t.phase += 1))
                }
            }

            TitbitKind::WeakStunned => {
                if !query.is_weak_or_stunned(t.element_supplier) {
                    UpdateAction::Remove
                } else {
                    // Advance sprite frame for star rotation.
                    let num_frames = mgr.num_frames_for_row(t.sprite_row);
                    UpdateAction::Mutate(Box::new(move |t| {
                        t.frame_count += 1;
                        if t.frame_count > TITBIT_FRAME_DELAY {
                            t.frame_count = 0;
                            t.sprite_frame += 1;
                            if t.sprite_frame >= num_frames {
                                t.sprite_frame = 0;
                            }
                        }
                    }))
                }
            }

            TitbitKind::UnconsciousStar => {
                if t.element_supplier.is_valid() {
                    if !query.is_unconscious_and_alive(t.element_supplier) {
                        UpdateAction::Remove
                    } else {
                        // Advance sprite frame for star rotation.
                        let num_frames = mgr.num_frames_for_row(t.sprite_row);
                        UpdateAction::Mutate(Box::new(move |t| {
                            t.frame_count += 1;
                            if t.frame_count > TITBIT_FRAME_DELAY {
                                t.frame_count = 0;
                                t.sprite_frame += 1;
                                if t.sprite_frame >= num_frames {
                                    t.sprite_frame = 0;
                                }
                            }
                        }))
                    }
                } else {
                    // Misused as "teleport stars" — count down phase.
                    // phase=N takes exactly N ticks before deletion, with
                    // the last visible tick at phase=1.  Delete when
                    // entering with phase <= 1 (post-decrement would be 0).
                    if t.phase <= 1 {
                        UpdateAction::Remove
                    } else {
                        UpdateAction::Mutate(Box::new(|t| t.phase -= 1))
                    }
                }
            }

            TitbitKind::Counter => {
                if t.sprite_frame >= COUNTER_LIMIT {
                    UpdateAction::Remove
                } else {
                    UpdateAction::Mutate(Box::new(|t| t.sprite_frame += 1))
                }
            }

            TitbitKind::Smoke => {
                // Flow:
                //   if (rand & 3) ++sprite_frame;
                //   if (sprite_frame == limit) delete;
                //   else            z += 2 + rand%4;
                // The z rise fires on *every* non-terminating tick,
                // regardless of whether the sprite advanced; the second
                // rand is consumed even when advance was false.
                let limit = mgr.num_frames_for_row(SpriteRow::Smoke as u16);
                let advance = query.random_u32() & 3 != 0;
                let new_frame = if advance {
                    t.sprite_frame + 1
                } else {
                    t.sprite_frame
                };
                if new_frame == limit {
                    UpdateAction::Remove
                } else {
                    let rise = 2.0 + (query.random_u32() % 4) as f32;
                    UpdateAction::Mutate(Box::new(move |t| {
                        t.sprite_frame = new_frame;
                        t.position.z += rise;
                    }))
                }
            }

            TitbitKind::Water => {
                let limit = mgr.num_frames_for_row(SpriteRow::Water as u16);
                if t.sprite_frame + 1 >= limit {
                    UpdateAction::Remove
                } else {
                    UpdateAction::Mutate(Box::new(|t| {
                        t.sprite_frame += 1;
                        t.position.z += 1.0;
                    }))
                }
            }

            TitbitKind::Plouf => {
                let limit = mgr.num_frames_for_row(SpriteRow::Plouf as u16);
                if t.sprite_frame + 1 >= limit {
                    UpdateAction::Remove
                } else {
                    UpdateAction::Mutate(Box::new(|t| {
                        t.sprite_frame += 1;
                        t.position.z += 1.0;
                    }))
                }
            }

            TitbitKind::Lock => {
                if !query.is_follow_element(t.element_supplier) {
                    UpdateAction::Remove
                } else {
                    UpdateAction::Keep
                }
            }

            TitbitKind::Ghost => {
                // Ghosts disappear after one frame.
                UpdateAction::Remove
            }

            TitbitKind::Hidden => {
                if !query.is_hidden_posture(t.element_supplier) {
                    UpdateAction::Remove
                } else {
                    UpdateAction::Keep
                }
            }

            TitbitKind::Emoticon => {
                // Emoticon animation is driven per-frame.
                if t.sprite_row == 0 {
                    UpdateAction::Keep
                } else if t.sprite_row == SpriteRow::EmoticonGrowingQMark as u16 {
                    UpdateAction::Mutate(Box::new(|t| {
                        t.frame_count += 1;
                        if t.frame_count > TITBIT_FRAME_DELAY {
                            t.frame_count = 0;
                            t.sprite_frame += 1;
                            if t.sprite_frame >= 8 {
                                t.sprite_frame = 0;
                            }
                        }
                    }))
                } else {
                    let num_frames = mgr.num_frames_for_row(t.sprite_row);
                    UpdateAction::Mutate(Box::new(move |t| {
                        t.frame_count += 1;
                        if t.frame_count > TITBIT_FRAME_DELAY {
                            t.frame_count = 0;
                            t.sprite_frame += 1;
                            if t.sprite_frame >= num_frames {
                                t.sprite_frame = 0;
                            }
                        }
                    }))
                }
            }

            TitbitKind::Dust => {
                let limit = mgr.num_frames_for_row(t.sprite_row);
                let advance = query.random_u32() & 3 != 0;
                let new_frame = if advance {
                    t.sprite_frame + 1
                } else {
                    t.sprite_frame
                };
                if new_frame == limit {
                    UpdateAction::Remove
                } else {
                    UpdateAction::Mutate(Box::new(move |t| {
                        t.sprite_frame = new_frame;
                        t.position.z += 1.0;
                    }))
                }
            }

            // These kinds have no per-frame update in the reference.
            TitbitKind::DangerPoint
            | TitbitKind::AppleSmell
            | TitbitKind::Speak
            | TitbitKind::WorkIcon
            | TitbitKind::QuickAction
            | TitbitKind::QuickActionRun => UpdateAction::Keep,
        }
    }
}

fn initial_sprite_row_for_kind(kind: TitbitKind) -> u16 {
    match kind {
        TitbitKind::GunImpact => SpriteRow::Impact as u16,
        TitbitKind::QuickAction | TitbitKind::QuickActionRun => {
            SpriteRow::QuickActionTitbits as u16
        }
        TitbitKind::Smoke => SpriteRow::Smoke as u16,
        TitbitKind::Water => SpriteRow::Water as u16,
        TitbitKind::Lock => SpriteRow::Lock as u16,
        TitbitKind::Plouf => SpriteRow::Plouf as u16,
        TitbitKind::Ghost => SpriteRow::Ghost as u16,
        TitbitKind::DangerPoint => SpriteRow::DangerPoint as u16,
        TitbitKind::Hidden => SpriteRow::Hidden as u16,
        // These rows are supplier-state dependent and are refreshed by
        // `EngineInner::refresh_titbit_positions`.
        TitbitKind::UnconsciousStar
        | TitbitKind::WeakStunned
        | TitbitKind::Emoticon
        | TitbitKind::AppleSmell
        | TitbitKind::Speak
        | TitbitKind::WorkIcon
        | TitbitKind::Counter
        | TitbitKind::Dust => 0,
    }
}

// ---------------------------------------------------------------------------
// Update query trait — supplies entity queries
// ---------------------------------------------------------------------------

enum UpdateAction {
    Keep,
    Remove,
    Mutate(Box<dyn FnOnce(&mut TitbitInfo)>),
}

/// Trait that engine integration code implements to answer entity
/// queries needed during [`TitbitManager::update`].
pub trait TitbitUpdateQuery {
    /// Is the element in weak-stunned or apple-sauce swordfight state?
    fn is_weak_or_stunned(&self, element: ElementHandle) -> bool;

    /// Is the element unconscious and not dead?
    fn is_unconscious_and_alive(&self, element: ElementHandle) -> bool;

    /// Is this element the current "follow" element (camera target)?
    fn is_follow_element(&self, element: ElementHandle) -> bool;

    /// Is the element in a hidden posture (spy or tree)?
    fn is_hidden_posture(&self, element: ElementHandle) -> bool;

    /// Return a pseudo-random u32 (for particle animation).
    fn random_u32(&self) -> u32;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn pt3(x: f32, y: f32, z: f32) -> WorldPoint3D {
        WorldPoint3D { x, y, z }
    }

    #[test]
    fn hidden_character_for_pc_matches_expected_mapping() {
        // Robin short-circuits regardless of profile filename.
        assert_eq!(
            HiddenCharacter::for_pc(true, "anything"),
            HiddenCharacter::Robin
        );
        let cases = [
            ("LittleJohn", HiddenCharacter::LittleJohn),
            ("WillScarlet", HiddenCharacter::Scarlet),
            ("Friar Tuck", HiddenCharacter::Tuck),
            ("Stuteley", HiddenCharacter::Stuteley),
            ("LadyMarian", HiddenCharacter::Marian),
            ("MerryManA", HiddenCharacter::MerryManA),
            ("MerryManB", HiddenCharacter::MerryManB),
            ("MerryManC", HiddenCharacter::MerryManC),
        ];
        for (filename, expected) in cases {
            assert_eq!(
                HiddenCharacter::for_pc(false, filename),
                expected,
                "filename {filename:?} should map to {expected:?}"
            );
            // Tolerate case variation from the data files.
            assert_eq!(
                HiddenCharacter::for_pc(false, &filename.to_uppercase()),
                expected,
                "uppercase {filename:?} should map to {expected:?}"
            );
        }
        // Phase values are the `#[repr(u16)]` discriminants: Robin=0 …
        assert_eq!(HiddenCharacter::Robin.to_phase(), 0);
        assert_eq!(HiddenCharacter::LittleJohn.to_phase(), 1);
        assert_eq!(HiddenCharacter::MerryManC.to_phase(), 8);
    }

    #[test]
    fn hidden_character_for_pc_uses_generic_fallback_on_unknown() {
        assert_eq!(
            HiddenCharacter::for_pc(false, "SomeStrangerNotShipped"),
            HiddenCharacter::MerryManC
        );
    }

    #[test]
    fn test_add_and_remove_titbit() {
        let mut mgr = TitbitManager::new();

        let id = mgr.add_titbit(
            pt3(100.0, 200.0, 0.0),
            0,
            TitbitKind::Emoticon,
            ElementHandle(1),
            0,
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            true,
            Some(200.0),
            Some(0),
        );
        assert_ne!(id, INVALID_ID);
        assert_eq!(mgr.titbits().len(), 1);
        assert!(mgr.titbit_exists(TitbitKind::Emoticon, ElementHandle(1)));

        mgr.remove_titbit(TitbitKind::Emoticon, ElementHandle(1));
        assert_eq!(mgr.titbits().len(), 0);
        assert!(!mgr.titbit_exists(TitbitKind::Emoticon, ElementHandle(1)));
    }

    #[test]
    fn test_counter_phase_zero_rejected() {
        let mut mgr = TitbitManager::new();
        let id = mgr.add_titbit(
            pt3(0.0, 0.0, 0.0),
            0,
            TitbitKind::Counter,
            ElementHandle::INVALID,
            0, // phase 0 → rejected
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            true,
            None,
            None,
        );
        assert_eq!(id, INVALID_ID);
        assert_eq!(mgr.titbits().len(), 0);
    }

    #[test]
    fn test_particle_filtered_when_disabled() {
        let mut mgr = TitbitManager::new();
        let id = mgr.add_titbit(
            pt3(0.0, 0.0, 0.0),
            0,
            TitbitKind::Smoke,
            ElementHandle::INVALID,
            0,
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            false, // display disabled
            None,
            None,
        );
        assert_eq!(id, INVALID_ID);
    }

    #[test]
    fn test_quick_action_with_run() {
        let mut mgr = TitbitManager::new();
        let id = mgr.add_titbit(
            pt3(50.0, 60.0, 0.0),
            0,
            TitbitKind::QuickAction,
            ElementHandle(2),
            QuickAction::Walk as u16,
            ElementHandle(2),
            true, // add run variant
            INVALID_ID,
            true,
            Some(60.0),
            Some(0),
        );
        assert_ne!(id, INVALID_ID);
        // Should have both the quick action and the run variant.
        assert_eq!(mgr.titbits().len(), 2);
        assert!(mgr.is_running_for_qa(TitbitId::new(id).unwrap()));
    }

    #[test]
    fn test_remove_quick_action() {
        let mut mgr = TitbitManager::new();
        let id = mgr.add_titbit(
            pt3(0.0, 0.0, 0.0),
            0,
            TitbitKind::QuickAction,
            ElementHandle(3),
            QuickAction::Stone as u16,
            ElementHandle(3),
            true,
            INVALID_ID,
            true,
            Some(0.0),
            Some(0),
        );
        assert_eq!(mgr.titbits().len(), 2);
        assert!(mgr.remove_quick_action_titbits_by_id(TitbitId::new(id).unwrap()));
        assert_eq!(mgr.titbits().len(), 0);
    }

    #[test]
    fn test_blinking() {
        let mut mgr = TitbitManager::new();
        let id = mgr.add_titbit(
            pt3(0.0, 0.0, 0.0),
            0,
            TitbitKind::QuickAction,
            ElementHandle(4),
            0,
            ElementHandle(4),
            false,
            INVALID_ID,
            true,
            Some(0.0),
            Some(0),
        );
        mgr.set_blinking(TitbitId::new(id).unwrap(), true);
        assert!(mgr.titbits()[0].blinking);

        mgr.reset_blinking();
        assert!(!mgr.titbits()[0].blinking);
    }

    #[test]
    fn test_set_run() {
        let mut mgr = TitbitManager::new();
        let id = mgr.add_titbit(
            pt3(0.0, 0.0, 0.0),
            0,
            TitbitKind::QuickAction,
            ElementHandle(5),
            QuickAction::Bow as u16,
            ElementHandle(5),
            false,
            INVALID_ID,
            true,
            Some(0.0),
            Some(0),
        );
        assert_eq!(mgr.titbits().len(), 1);
        mgr.set_run(TitbitId::new(id).unwrap(), QuickAction::Bow as u16);
        assert_eq!(mgr.titbits().len(), 2);
        assert_eq!(mgr.titbits()[1].phase, QuickAction::PlusQuick as u16);
    }

    #[test]
    fn fixed_row_titbits_start_on_their_render_rows() {
        let mut mgr = TitbitManager::new();
        let cases = [
            (TitbitKind::QuickAction, SpriteRow::QuickActionTitbits),
            (TitbitKind::Smoke, SpriteRow::Smoke),
            (TitbitKind::Water, SpriteRow::Water),
            (TitbitKind::Plouf, SpriteRow::Plouf),
            (TitbitKind::Ghost, SpriteRow::Ghost),
            (TitbitKind::DangerPoint, SpriteRow::DangerPoint),
        ];

        for (kind, row) in cases {
            let id = mgr.add_titbit(
                pt3(0.0, 0.0, 0.0),
                0,
                kind,
                ElementHandle::INVALID,
                0,
                ElementHandle::INVALID,
                false,
                INVALID_ID,
                true,
                None,
                None,
            );
            let titbit = mgr.titbits().iter().find(|t| t.id == id).unwrap();
            assert_eq!(titbit.sprite_row, row as u16, "kind {kind:?}");
        }
    }

    #[test]
    fn test_get_phase() {
        let mut mgr = TitbitManager::new();
        let id = mgr.add_titbit(
            pt3(0.0, 0.0, 0.0),
            0,
            TitbitKind::QuickAction,
            ElementHandle(6),
            QuickAction::Ale as u16,
            ElementHandle(6),
            false,
            INVALID_ID,
            true,
            Some(0.0),
            Some(0),
        );
        assert_eq!(
            mgr.get_phase(TitbitId::new(id).unwrap()),
            QuickAction::Ale as u16
        );
        assert_eq!(mgr.get_phase(TitbitId::new(999).unwrap()), 0xFFFF);
    }

    #[test]
    fn test_serde_roundtrip() {
        let mut mgr = TitbitManager::new();
        mgr.add_titbit(
            pt3(10.0, 20.0, 5.0),
            1,
            TitbitKind::Emoticon,
            ElementHandle(7),
            0,
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            true,
            Some(20.0),
            Some(1),
        );

        let json = serde_json::to_string(&mgr).unwrap();
        let mgr2: TitbitManager = serde_json::from_str(&json).unwrap();
        assert_eq!(mgr2.titbits().len(), 1);
        assert_eq!(mgr2.titbits()[0].kind, TitbitKind::Emoticon);
        assert_eq!(mgr2.current_id, mgr.current_id);
        // Runtime state is reset on deser.
        assert_eq!(mgr2.blink_counter, TIME_BLINK_ON);
    }

    struct DummyQuery;
    impl TitbitUpdateQuery for DummyQuery {
        fn is_weak_or_stunned(&self, _: ElementHandle) -> bool {
            false
        }
        fn is_unconscious_and_alive(&self, _: ElementHandle) -> bool {
            false
        }
        fn is_follow_element(&self, _: ElementHandle) -> bool {
            false
        }
        fn is_hidden_posture(&self, _: ElementHandle) -> bool {
            false
        }
        fn random_u32(&self) -> u32 {
            0
        }
    }

    struct FixedRandomQuery {
        value: u32,
    }

    impl TitbitUpdateQuery for FixedRandomQuery {
        fn is_weak_or_stunned(&self, _: ElementHandle) -> bool {
            false
        }
        fn is_unconscious_and_alive(&self, _: ElementHandle) -> bool {
            false
        }
        fn is_follow_element(&self, _: ElementHandle) -> bool {
            false
        }
        fn is_hidden_posture(&self, _: ElementHandle) -> bool {
            false
        }
        fn random_u32(&self) -> u32 {
            self.value
        }
    }

    #[test]
    fn test_update_gun_impact_expires() {
        let mut mgr = TitbitManager::new();
        mgr.add_titbit(
            pt3(0.0, 0.0, 0.0),
            0,
            TitbitKind::GunImpact,
            ElementHandle::INVALID,
            0,
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            true,
            None,
            None,
        );

        let q = DummyQuery;
        // Impact lasts 6 frames (phase 0..5).
        for _ in 0..5 {
            mgr.update(&q);
            assert_eq!(mgr.titbits().len(), 1);
        }
        mgr.update(&q);
        assert_eq!(mgr.titbits().len(), 0);
    }

    #[test]
    fn dust_advances_like_reference_particle() {
        let mut mgr = TitbitManager::new();
        mgr.set_row_frame_counts(vec![3]);
        mgr.add_titbit(
            pt3(0.0, 0.0, 4.0),
            0,
            TitbitKind::Dust,
            ElementHandle::INVALID,
            0,
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            true,
            None,
            None,
        );

        mgr.update(&FixedRandomQuery { value: 1 });

        let dust = &mgr.titbits()[0];
        assert_eq!(dust.sprite_frame, 1);
        assert_eq!(dust.position.z, 5.0);
    }

    #[test]
    fn dust_expires_at_frame_limit() {
        let mut mgr = TitbitManager::new();
        mgr.set_row_frame_counts(vec![1]);
        mgr.add_titbit(
            pt3(0.0, 0.0, 4.0),
            0,
            TitbitKind::Dust,
            ElementHandle::INVALID,
            0,
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            true,
            None,
            None,
        );

        mgr.update(&FixedRandomQuery { value: 1 });

        assert!(mgr.titbits().is_empty());
    }

    #[test]
    fn test_update_ghost_removed_immediately() {
        let mut mgr = TitbitManager::new();
        mgr.add_titbit(
            pt3(0.0, 0.0, 0.0),
            0,
            TitbitKind::Ghost,
            ElementHandle::INVALID,
            0,
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            true,
            None,
            None,
        );
        mgr.update(&DummyQuery);
        assert_eq!(mgr.titbits().len(), 0);
    }

    #[test]
    fn test_update_counter_expires_at_limit() {
        let mut mgr = TitbitManager::new();
        mgr.add_titbit(
            pt3(0.0, 0.0, 0.0),
            0,
            TitbitKind::Counter,
            ElementHandle::INVALID,
            5, // non-zero phase
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            true,
            None,
            None,
        );
        let q = DummyQuery;
        // Counter increments sprite_frame each tick; removed when frame >= COUNTER_LIMIT.
        // So it survives COUNTER_LIMIT ticks (0→25), then removed on tick COUNTER_LIMIT+1.
        for _ in 0..COUNTER_LIMIT {
            mgr.update(&q);
            assert_eq!(mgr.titbits().len(), 1);
        }
        mgr.update(&q);
        assert_eq!(mgr.titbits().len(), 0);
    }

    #[test]
    fn test_remove_lock() {
        let mut mgr = TitbitManager::new();
        mgr.add_titbit(
            pt3(0.0, 0.0, 0.0),
            0,
            TitbitKind::Lock,
            ElementHandle(10),
            0,
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            true,
            Some(0.0),
            Some(0),
        );
        assert!(mgr.remove_lock());
        assert!(!mgr.remove_lock()); // already gone
    }

    #[test]
    fn prepare_refresh_preserves_particle_display_order() {
        // Particle/debug kinds (DangerPoint here) are not listed in the
        // `prepare_refresh` re-anchor match — their display_order must
        // survive untouched across frames.  Regression test for the old
        // `_ => 0.01` fallthrough that clobbered them every tick.
        let mut mgr = TitbitManager::new();
        mgr.add_titbit(
            pt3(0.0, 0.0, 0.0),
            0,
            TitbitKind::DangerPoint,
            ElementHandle(7),
            0,
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            true,
            Some(50.0),
            Some(0),
        );
        let created_order = mgr.titbits()[0].display_order;

        // Callback returns a very different value; if prepare_refresh
        // touched the particle's display_order it would be `99.0 + _`.
        mgr.prepare_refresh(|_| Some(99.0));

        assert_eq!(mgr.titbits()[0].display_order, created_order);
    }
}
