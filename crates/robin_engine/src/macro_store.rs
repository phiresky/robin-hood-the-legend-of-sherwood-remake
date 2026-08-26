//! Per-PC quick-action macro storage and dotted-chain geometry.
//!
//! Manual quick actions and the post-port Shift-click queue deliberately use
//! different serialized stores.  Original-compatible QA slots must never be
//! consumed merely because a PC also has automatic work pending.
//!
//! Two parallel collections per PC in [`MacroStore`]:
//!
//! * `slots` — exactly the three Original-compatible QA memory slots.
//! * `maul_titbits` — **one
//!   titbit ID per QA slot**.  The id is produced by an
//!   `AddTitbit(RHTITBIT_QUICKACTION, …)` call at the input-action site
//!   when the macro was recorded.  The portrait widget blits a *single*
//!   frame for that slot, resolved via `Titbits::get_phase`.
//!
//! The dotted-chain segments connecting the PC to each recorded step's
//! world position come from `Titbits::draw_lines`; we retain the recorded
//! `position` per step for that.

use serde::{Deserialize, Serialize};

use crate::coordinates::MapPoint;
use crate::element::{Command, EntityId};
use crate::element_kinds::QuickAction;
use crate::profiles::Action;
use crate::sequence::{Field, Sequence};

/// Number of Original-compatible, portrait-visible quick-action slots.
///
/// Automatic Shift-click queues live in [`AutoQueueStore`] and may grow beyond
/// this count; legacy saves and the portrait strip deliberately continue to
/// expose exactly these manual slots.
pub const NUMBER_OF_QA_MEMORY: usize = 3;

/// Map an `Action` to its frame index inside the
/// `RHID_QUICKACTION_TITBITS` sprite sheet.
///
/// The sprite sheet is indexed by the `RHQUICK_*` enum: each enumerator's
/// ordinal value is the frame row.  Each `AddTitbit(RHTITBIT_QUICKACTION,
/// …, RHQUICK_<X>, …)` call site picks the RHQUICK value for the action
/// it represents.
///
/// Returns `None` for actions that have no dedicated icon in the sheet
/// (e.g. contextual actions like `Climb`, `Jump`, `Search`, …).  These
/// fall through to the `RHQUICK_DEFAULT` fallback when they do reach
/// the titbit system.
pub fn action_to_qa_frame(action: Action) -> Option<u16> {
    // Frame indices = `RHQUICK_*` enum ordinals.  Keep in sync if the
    // enum is ever re-ordered.
    Some(match action {
        Action::Bow => 46,                                   // RHQUICK_BOW
        Action::Hit | Action::HitHard => 12,                 // RHQUICK_HIT
        Action::Purse => 30,                                 // RHQUICK_PURSE
        Action::Stone => 10,                                 // RHQUICK_STONE
        Action::Shield => 21,                                // RHQUICK_SHIELD
        Action::BigShield => 43,                             // RHQUICK_SHIELD_2
        Action::Strangle => 25,                              // RHQUICK_STRANGLE
        Action::Lever => 31,                                 // RHQUICK_LEVER
        Action::HelpToClimb => 52,                           // RHQUICK_HELP_CLIMB
        Action::Apple => 37,                                 // RHQUICK_APPLE
        Action::Ale | Action::Guzzle => 8,                   // RHQUICK_ALE
        Action::Eat => 33,                                   // RHQUICK_EAT
        Action::Listen => 24,                                // RHQUICK_LISTEN
        Action::Heal => 45,                                  // RHQUICK_HEAL
        Action::Net => 26,                                   // RHQUICK_NET
        Action::Beggar => 34,                                // RHQUICK_BEGGAR
        Action::WaspNest => 29,                              // RHQUICK_WASP
        Action::Whistle => 44,                               // RHQUICK_WHISTLE
        Action::Climb => 23,                                 // RHQUICK_LADDER
        Action::Search => 51,                                // RHQUICK_SEARCH
        Action::Resuscitate => 40,                           // RHQUICK_WAKE_UP
        Action::LittleJohnCarry | Action::FarmerCarry => 28, // RHQUICK_CLIMB_ON_SHOULDERS
        Action::Tie => 32,                                   // RHQUICK_TIE
        Action::Lockpick => 20,                              // RHQUICK_LOCKPICK
        Action::Execute => 11,                               // RHQUICK_EXECUTE
        // No dedicated RHQUICK_* icon — renderer skips per-step overlay.
        Action::NoAction | Action::Jump | Action::Test => return None,
    })
}

/// Spacing between dots on the dotted chain.
///
/// The engine's `dotted_start` phase (`titbit::DISTANCE_DOT`) must wrap
/// on the same constant or the marching-ants animation stutters.
/// Re-exported here for the macro-chain renderer.
pub use crate::titbit::DISTANCE_DOT;

/// Source-resolved movement geometry retained by a recorded group move.
///
/// Original `PerformMove(..., bRecordQA=true)` stores a concrete coordinate
/// `SEEK` against the already-resolved `RHSector*`, plus a post-seek arrival
/// speech.  It does not save the common click and run formation placement a
/// second time when the quick action is played.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, robin_state_hash_derive::StateHash,
)]
pub struct RecordedQaMoveRoute {
    pub goal_sector: crate::sector::SectorNumber,
    pub goal_sector_index: crate::fast_find_grid::SectorIndex,
    pub goal_layer: u16,
}

/// The specific player command captured at a macro step — enough to
/// rebuild a [`PlayerCommand`](crate::player_command::PlayerCommand) at
/// playback time.  Replay clones each recorded sequence element and
/// relaunches it as a fresh command.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, robin_state_hash_derive::StateHash,
)]
pub enum QaReplayCommand {
    /// Group-move to a destination — relayed as `PlayerCommand::GroupMove`
    /// with a single-element `actors` vec (the replay target PC).
    Move {
        destination: MapPoint,
        running: bool,
        /// Exact resolved goal identity captured alongside the per-PC
        /// formation destination. Replaying a raw click would run formation
        /// placement a second time and can no longer reproduce the recorded
        /// quick action. Required from SAVE55/NET19/REPLAY13 onward.
        route: RecordedQaMoveRoute,
    },
    /// Interaction with a specific target entity (attack, heal, tie, …).
    ///
    /// `double_click` records whether the input was a left-double-click
    /// (set when the macro was recorded via the QUICKITOS_INTERRACT /
    /// RHMOUSE_LEFTDOUBLE input).  On replay we synthesise a leading
    /// single-click dispatch before the recorded double-click — engine
    /// state expects a single click to precede a double.
    Interaction {
        target: EntityId,
        command: Command,
        double_click: bool,
    },
    /// Interaction recorded through `RHElementTarget::MouseClicked`.
    ///
    /// Unlike the target's live click route, Original stores a coordinate
    /// `SEEK` (tolerance 0, flags 0) whose post-seek continuation is
    /// `TURN` followed by the interaction.  Keep the authored movement and
    /// turn geometry here so playback can clone that recorded shape instead
    /// of re-entering either the live target route or the generic
    /// entity-seek interaction path.
    TargetInteraction {
        target: EntityId,
        command: Command,
        destination: MapPoint,
        sector: Option<crate::position_interface::SectorHandle>,
        layer: u16,
        action: crate::order::OrderType,
        turn_point: MapPoint,
    },
    /// Read a scroll carried by / attached to a target NPC. Replayed
    /// through `PlayerCommand::LaunchScrollRead` so the seek + open
    /// scroll sequence is rebuilt from current engine state.
    ScrollRead { target: EntityId, running: bool },
    /// Ground-targeted ability (net, wasp-nest, purse) — the 3D target
    /// position (from `FastFindGrid::convert_2d_to_3d` at input time)
    /// and the caller-resolved titbit layer are captured; the target
    /// entity is *not*, since only the point is needed.
    GroundTarget {
        target_pos: crate::coordinates::WorldPoint3D,
        command: Command,
        target_field: Field,
        /// Titbit layer argument forwarded from
        /// `PlayerCommand::LaunchGroundTarget` (Net=0, Wasp/Purse =
        /// the selected layer at record time).  Captured verbatim so
        /// replay re-emits the same titbit layer regardless of the
        /// live `selected_layer` at playback.
        titbit_layer: u16,
    },
    /// Self ability (whistle, eat, parry, …).
    SelfAbility { command: Command },
    /// Drop-ale seek-then-drop sequence.  Replayed as
    /// `PlayerCommand::DropAleAt` so the engine rebuilds the Seek→DropAle
    /// pair from the captured destination point.
    DropAle { target_pos: MapPoint, running: bool },
    /// Enter-swordfight engagement on a target.
    Swordfight { target: EntityId, running: bool },
    /// Direct sword strike on a target (mid-swordfight).
    SwordStrike {
        target: EntityId,
        command: Command,
        with_seek: bool,
        /// Exact seek tolerance captured with the resolved player command.
        /// Missing legacy macro state retains the strike-specific fallback.
        #[serde(default)]
        seek_distance: Option<f32>,
    },
    /// Shield two-click completion. Original records the concrete
    /// `Seek(protected_pc, 50) -> RaiseShield` sequence and attaches the QA
    /// titbit to `protected_pc` at the projected danger point.
    ShieldRaise {
        protected_pc: EntityId,
        danger_point: crate::coordinates::WorldPoint3D,
        danger_point_layer: u16,
    },
    /// Quickitos posture toggle — `CrouchDown` / `StandUp` recorded so
    /// the macro can replay a mid-sequence posture change.  `to_crouch`
    /// = true means *crouch down*; the input source passes
    /// `MSG_STAND_UP.GetValue()` which is 1 for the down-arrow widget
    /// and 0 for the up-arrow.
    PostureToggle { to_crouch: bool },
}

/// One recorded action inside a macro slot.  One entry per appended
/// sequence element.
///
/// `position` drives the dotted chain; `replay` carries enough to
/// reconstruct a `PlayerCommand` so `EngineInner::start_quick_action`
/// can re-dispatch each step.  **There is no per-step titbit id**:
/// titbits are registered once per *slot* via `maul_titbits[level]`,
/// not once per step.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, robin_state_hash_derive::StateHash,
)]
pub struct QuickActionStep {
    pub action: Action,
    /// Captured world position of the interaction target (the titbit's
    /// recorded position).  Drives the dotted chain.
    #[serde(with = "map_point_tuple_serde")]
    pub position: MapPoint,
    /// The command to dispatch at playback time.
    pub replay: QaReplayCommand,
}

mod map_point_tuple_serde {
    use super::MapPoint;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(p: &MapPoint, s: S) -> Result<S::Ok, S::Error> {
        (p.x, p.y).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<MapPoint, D::Error> {
        let (x, y) = <(f32, f32)>::deserialize(d)?;
        Ok(MapPoint::new(x, y))
    }
}

/// One macro slot (one recorded sequence).
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct QuickActionSlot {
    pub steps: Vec<QuickActionStep>,
    /// Exact owner-local sequences restored from an Original v48 save.
    /// These cannot be losslessly reconstructed as high-level player
    /// commands, so playback launches copies of the retained payloads.
    legacy_action_sequence: Option<Sequence>,
    legacy_seek_sequence: Option<Sequence>,
    /// Exact non-sequence quick action restored from an Original save.
    legacy_quickito: Option<LegacyQuickito>,
}

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, robin_state_hash_derive::StateHash,
)]
pub(crate) struct LegacyQuickito {
    pub kind: QuickAction,
    pub interactor: Option<EntityId>,
    pub button: u16,
}

impl PartialEq for QuickActionSlot {
    fn eq(&self, other: &Self) -> bool {
        self.steps == other.steps
            && serde_json::to_value(&self.legacy_action_sequence)
                .expect("serialize legacy QA action for equality")
                == serde_json::to_value(&other.legacy_action_sequence)
                    .expect("serialize legacy QA action for equality")
            && serde_json::to_value(&self.legacy_seek_sequence)
                .expect("serialize legacy QA seek for equality")
                == serde_json::to_value(&other.legacy_seek_sequence)
                    .expect("serialize legacy QA seek for equality")
            && self.legacy_quickito == other.legacy_quickito
    }
}

impl QuickActionSlot {
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
            && self.legacy_action_sequence.is_none()
            && self.legacy_quickito.is_none()
    }
    pub fn len(&self) -> usize {
        self.steps
            .len()
            .max(
                self.legacy_action_sequence
                    .as_ref()
                    .map_or(0, Sequence::len),
            )
            .max(usize::from(self.legacy_quickito.is_some()))
    }

    pub(crate) fn legacy_sequences(&self) -> Option<(&Sequence, Option<&Sequence>)> {
        self.legacy_action_sequence
            .as_ref()
            .map(|action| (action, self.legacy_seek_sequence.as_ref()))
    }

    pub(crate) fn legacy_quickito(&self) -> Option<LegacyQuickito> {
        self.legacy_quickito
    }
}

/// Per-PC macro state — the recorded slots, the per-slot titbit ids,
/// and the slot currently being appended to.
///
/// `recording_slot` is the slot index currently being appended to when
/// the messenger's macro-recording flag is on and this PC is the target
/// (`qa_recording_for == Some(this pc)`).  `None` means "not recording".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, robin_state_hash_derive::StateHash)]
pub struct PcMacroState {
    slots: Vec<QuickActionSlot>,
    /// One titbit ID per QA slot, `None` when empty.  Set at the
    /// `AddTitbit(RHTITBIT_QUICKACTION, …)` / `SetQuickActionSequence`
    /// site in the input-action flow.
    maul_titbits: Vec<Option<crate::titbit::TitbitId>>,
    recording_slot: Option<u8>,
    /// Whether the armed manual slot still contains its previous QA. Original
    /// keeps that state live until the first replacement action is captured.
    ///
    /// This replaces the old serialized `recording_backup` representation.
    /// It is not wire-compatible with SAVE53/NET17/REPLAY11; this state layout
    /// starts at SAVE54/NET18/REPLAY12.
    #[serde(default)]
    recording_replaces_existing: bool,
}

impl Default for PcMacroState {
    fn default() -> Self {
        Self {
            slots: (0..NUMBER_OF_QA_MEMORY)
                .map(|_| QuickActionSlot::default())
                .collect(),
            maul_titbits: vec![None; NUMBER_OF_QA_MEMORY],
            recording_slot: None,
            recording_replaces_existing: false,
        }
    }
}

/// Elevation at which a shifting titbit starts falling in.
pub const SHIFT_STEP: f32 = 24.0;

/// Pixels of shift decay per frame.
pub const SHIFT_FALL_PER_REFRESH: f32 = 10.0;

/// Number of blink phases a QA slot strobes through when a macro fizzles.
pub const BLINK_PHASE_INIT: u16 = 6;

/// Ticks per blink phase.
pub const BLINK_PHASE_LENGTH: u16 = 5;

impl PcMacroState {
    pub fn slot(&self, idx: usize) -> Option<&QuickActionSlot> {
        self.slots.get(idx)
    }

    pub fn has_macro(&self, idx: usize) -> bool {
        self.slots.get(idx).map(|s| !s.is_empty()).unwrap_or(false)
    }

    /// Slots in recorded order.  Useful for "render every non-empty slot's
    /// icon strip next to the portrait".
    pub fn slots(&self) -> &[QuickActionSlot] {
        &self.slots
    }

    pub fn is_recording(&self) -> bool {
        self.recording_slot.is_some()
    }

    pub fn recording_slot(&self) -> Option<u8> {
        self.recording_slot
    }

    pub fn first_empty_slot(&self) -> Option<u8> {
        self.slots
            .iter()
            .position(QuickActionSlot::is_empty)
            .map(|slot| slot as u8)
    }

    /// Read a slot's titbit id.  Returns `None` for an empty slot.
    pub fn get_slot_titbit(&self, slot: usize) -> Option<crate::titbit::TitbitId> {
        self.maul_titbits.get(slot).copied().flatten()
    }

    /// Write a slot's titbit id.  Called from the
    /// `set_quick_action_sequence` flow once the recorder knows which
    /// titbit id to associate with the slot.
    pub fn set_slot_titbit(&mut self, slot: usize, id: crate::titbit::TitbitId) {
        if let Some(cell) = self.maul_titbits.get_mut(slot) {
            *cell = Some(id);
        }
    }

    pub fn clear_slot_titbit(&mut self, slot: usize) {
        if let Some(cell) = self.maul_titbits.get_mut(slot) {
            *cell = None;
        }
    }

    /// Begin recording into `slot_idx`. Previous contents remain in their
    /// canonical slot until the first new step is appended, so arming and
    /// canceling alone does not mutate the active QA or its titbit.
    pub fn begin_recording(&mut self, slot_idx: u8) {
        assert!(
            (slot_idx as usize) < NUMBER_OF_QA_MEMORY,
            "slot_idx {slot_idx} out of range 0..{NUMBER_OF_QA_MEMORY}"
        );
        let slot = usize::from(slot_idx);
        self.recording_replaces_existing = !self.slots[slot].is_empty();
        self.recording_slot = Some(slot_idx);
    }

    /// Stop recording.  Keeps whatever was appended; the slot is
    /// committed at this point.
    pub fn stop_recording(&mut self) {
        self.recording_replaces_existing = false;
        self.recording_slot = None;
    }

    pub fn recording_replaces_existing_slot(&self) -> Option<u8> {
        self.recording_slot
            .filter(|_| self.recording_replaces_existing)
    }

    /// Append a step if currently recording.  No-op otherwise.
    pub fn append_if_recording(&mut self, step: QuickActionStep) {
        if let Some(idx) = self.recording_slot {
            let idx = usize::from(idx);
            if self.recording_replaces_existing {
                self.slots[idx] = QuickActionSlot::default();
                self.maul_titbits[idx] = None;
                self.recording_replaces_existing = false;
            }
            self.slots[idx].steps.push(step);
        }
    }

    /// Clear a slot, as the cleanup / abort paths do once a macro has
    /// fired.
    pub fn clear_slot(&mut self, slot_idx: usize) {
        if let Some(s) = self.slots.get_mut(slot_idx) {
            s.steps.clear();
            s.legacy_action_sequence = None;
            s.legacy_seek_sequence = None;
            s.legacy_quickito = None;
        }
        if let Some(cell) = self.maul_titbits.get_mut(slot_idx) {
            *cell = None;
        }
        if self.recording_slot == Some(slot_idx as u8) {
            self.recording_slot = None;
            self.recording_replaces_existing = false;
        }
    }

    /// Shift every later slot down by one. Called once every PC has completed
    /// a given macro slot so the remaining slots collapse forward.
    ///
    /// Shifted state: `steps` and `maul_titbits[i] = maul_titbits[i+1]`.
    ///
    /// The recording-state guard is defensive — the tetris message is
    /// only posted after every PC has completed slot N, ruling out a
    /// "recording into slot N while slot N tetrises" race.  Kept as a
    /// guard against that invariant breaking.
    pub fn do_tetris(&mut self, slot_idx: usize) {
        if slot_idx >= self.slots.len() {
            return;
        }
        for i in slot_idx..self.slots.len() - 1 {
            self.slots.swap(i, i + 1);
            self.maul_titbits[i] = self.maul_titbits[i + 1];
        }
        if self.slots.len() > NUMBER_OF_QA_MEMORY {
            self.slots.pop();
            self.maul_titbits.pop();
        } else {
            let last = NUMBER_OF_QA_MEMORY - 1;
            self.slots[last] = QuickActionSlot::default();
            self.maul_titbits[last] = None;
        }
        if let Some(rs) = self.recording_slot
            && (rs as usize) >= slot_idx
        {
            self.recording_slot = None;
            self.recording_replaces_existing = false;
        }
    }
}

/// One automatic Shift-click item. Unlike an Original macro slot, a queue
/// item always contains exactly one resolved command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, robin_state_hash_derive::StateHash)]
pub struct AutoQueueEntry {
    pub step: QuickActionStep,
    pub titbit: Option<crate::titbit::TitbitId>,
}

/// Serialized, PC-keyed automatic queue storage.
///
/// This is intentionally not part of [`MacroStore`]. A manual QA in slot 0
/// and an automatic item at queue position 0 are different state, even when
/// their commands happen to be identical.
#[derive(
    Debug, Clone, Default, Serialize, Deserialize, PartialEq, robin_state_hash_derive::StateHash,
)]
pub struct AutoQueueStore {
    entries: Vec<(EntityId, Vec<AutoQueueEntry>)>,
}

impl AutoQueueStore {
    pub fn get(&self, pc: EntityId) -> Option<&[AutoQueueEntry]> {
        self.entries
            .iter()
            .find(|(id, _)| *id == pc)
            .map(|(_, queue)| queue.as_slice())
    }

    pub fn len(&self, pc: EntityId) -> usize {
        self.get(pc).map_or(0, <[AutoQueueEntry]>::len)
    }

    pub fn is_empty(&self, pc: EntityId) -> bool {
        self.len(pc) == 0
    }

    pub fn push(&mut self, pc: EntityId, step: QuickActionStep) {
        if let Some((_, queue)) = self.entries.iter_mut().find(|(id, _)| *id == pc) {
            queue.push(AutoQueueEntry { step, titbit: None });
        } else {
            self.entries
                .push((pc, vec![AutoQueueEntry { step, titbit: None }]));
        }
    }

    pub fn set_last_titbit(&mut self, pc: EntityId, titbit: crate::titbit::TitbitId) {
        let queue = self
            .entries
            .iter_mut()
            .find(|(id, _)| *id == pc)
            .map(|(_, queue)| queue)
            .unwrap_or_else(|| panic!("automatic quick-action queue for {pc:?} disappeared"));
        queue
            .last_mut()
            .unwrap_or_else(|| panic!("automatic quick-action queue for {pc:?} is empty"))
            .titbit = Some(titbit);
    }

    pub fn pop_front(&mut self, pc: EntityId) -> Option<AutoQueueEntry> {
        let index = self.entries.iter().position(|(id, _)| *id == pc)?;
        if self.entries[index].1.is_empty() {
            panic!("automatic quick-action queue entry for {pc:?} is empty");
        }
        let entry = self.entries[index].1.remove(0);
        if self.entries[index].1.is_empty() {
            self.entries.remove(index);
        }
        Some(entry)
    }

    /// Upgrade the newest pending movement to running.
    pub fn make_last_move_running(&mut self, pc: EntityId) -> Option<usize> {
        let queue = self
            .entries
            .iter_mut()
            .find(|(id, _)| *id == pc)
            .map(|(_, queue)| queue)?;
        let index = queue.len().checked_sub(1)?;
        match &mut queue[index].step.replay {
            QaReplayCommand::Move { running, .. } | QaReplayCommand::DropAle { running, .. } => {
                *running = true;
                Some(index)
            }
            _ => None,
        }
    }
}

/// PC-keyed macro store — each PC owns its own slots / titbit-id arrays.
///
/// A flat map instead of a field on a PC struct because entities are
/// id-keyed and there isn't a central per-PC struct to hang this off
/// of.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct MacroStore {
    entries: Vec<(EntityId, PcMacroState)>,
}

impl MacroStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, pc: EntityId) -> Option<&PcMacroState> {
        self.entries
            .iter()
            .find(|(id, _)| *id == pc)
            .map(|(_, s)| s)
    }

    pub fn get_or_insert(&mut self, pc: EntityId) -> &mut PcMacroState {
        if let Some(idx) = self.entries.iter().position(|(id, _)| *id == pc) {
            &mut self.entries[idx].1
        } else {
            self.entries.push((pc, PcMacroState::default()));
            &mut self.entries.last_mut().unwrap().1
        }
    }

    /// Iterate over all (pc, state) pairs — used by the renderer to draw
    /// the per-PC dotted chains.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (EntityId, &mut PcMacroState)> {
        self.entries.iter_mut().map(|(id, s)| (*id, s))
    }

    pub fn get_mut(&mut self, pc: EntityId) -> Option<&mut PcMacroState> {
        self.entries
            .iter_mut()
            .find(|(id, _)| *id == pc)
            .map(|(_, s)| s)
    }

    pub(crate) fn adopt_legacy_sequence_slot(
        &mut self,
        pc: EntityId,
        slot: usize,
        action: Sequence,
        seek: Option<Sequence>,
        titbit: Option<crate::titbit::TitbitId>,
    ) {
        assert!(
            slot < NUMBER_OF_QA_MEMORY,
            "legacy QA slot {slot} is out of range"
        );
        let state = self.get_or_insert(pc);
        state.slots[slot].steps.clear();
        state.slots[slot].legacy_action_sequence = Some(action);
        state.slots[slot].legacy_seek_sequence = seek;
        state.slots[slot].legacy_quickito = None;
        state.maul_titbits[slot] = titbit;
    }

    pub(crate) fn adopt_legacy_quickito_slot(
        &mut self,
        pc: EntityId,
        slot: usize,
        quickito: LegacyQuickito,
        titbit: Option<crate::titbit::TitbitId>,
    ) {
        assert!(
            slot < NUMBER_OF_QA_MEMORY,
            "legacy Quickito slot {slot} is out of range"
        );
        assert_ne!(quickito.kind, QuickAction::None, "empty legacy Quickito");
        let state = self.get_or_insert(pc);
        state.slots[slot].steps.clear();
        state.slots[slot].legacy_action_sequence = None;
        state.slots[slot].legacy_seek_sequence = None;
        state.slots[slot].legacy_quickito = Some(quickito);
        state.maul_titbits[slot] = titbit;
    }

    /// Append to any PC currently recording.  Convenience wrapper for
    /// the `qa_recording_for == Some(pc)` branch.
    pub fn append(&mut self, pc: EntityId, step: QuickActionStep) {
        self.get_or_insert(pc).append_if_recording(step);
    }
}

/// Build the dotted-chain segments for one macro slot.
///
/// ```text
/// from = pc.position_map();
/// for step in slot.steps:
///     to = step.position;  // flattened: y -= z
///     draw_dotted_line(from, to, ...);
///     from = to;
/// ```
///
/// Returns the `(from, to)` pairs in draw order; the renderer feeds
/// each into `DrawManager::draw_dotted_line` with `DISTANCE_DOT` spacing
/// and the global titbit dotted-start phase (one per game).
pub fn dotted_chain_segments(
    pc_position_map: MapPoint,
    slot: &QuickActionSlot,
) -> Vec<(MapPoint, MapPoint)> {
    let mut segs = Vec::with_capacity(slot.steps.len());
    let mut from = pc_position_map;
    for step in &slot.steps {
        let to = step.position;
        segs.push((from, to));
        from = to;
    }
    segs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequence::SequenceElement;

    fn route() -> RecordedQaMoveRoute {
        RecordedQaMoveRoute {
            goal_sector: crate::sector::SectorNumber::new(1),
            goal_sector_index: crate::fast_find_grid::SectorIndex::new(0)
                .expect("valid test sector index"),
            goal_layer: 0,
        }
    }

    fn step(action: Action, x: f32, y: f32) -> QuickActionStep {
        QuickActionStep {
            action,
            position: MapPoint::new(x, y),
            replay: QaReplayCommand::Move {
                destination: MapPoint::new(x, y),
                running: false,
                route: route(),
            },
        }
    }

    #[test]
    fn sword_macro_preserves_resolved_seek_distance_and_defaults_legacy_field() {
        let target = EntityId::new(9, crate::element::EntityIdKind::Soldier);
        let replay = QaReplayCommand::SwordStrike {
            target,
            command: Command::SwordstrikeThrustA,
            with_seek: true,
            seek_distance: Some(63.0),
        };
        let encoded = serde_json::to_value(replay).expect("serialize sword macro");
        let decoded: QaReplayCommand =
            serde_json::from_value(encoded.clone()).expect("roundtrip sword macro");
        assert!(matches!(
            decoded,
            QaReplayCommand::SwordStrike {
                seek_distance: Some(63.0),
                ..
            }
        ));

        let mut legacy = encoded;
        legacy
            .get_mut("SwordStrike")
            .and_then(serde_json::Value::as_object_mut)
            .expect("externally tagged sword macro")
            .remove("seek_distance");
        let decoded: QaReplayCommand =
            serde_json::from_value(legacy).expect("deserialize legacy sword macro");
        assert!(matches!(
            decoded,
            QaReplayCommand::SwordStrike {
                seek_distance: None,
                ..
            }
        ));

        let direct = QaReplayCommand::SwordStrike {
            target,
            command: Command::SwordstrikeThrustF,
            with_seek: false,
            seek_distance: None,
        };
        let decoded: QaReplayCommand = serde_json::from_value(
            serde_json::to_value(direct).expect("serialize direct sword macro"),
        )
        .expect("roundtrip direct sword macro");
        assert!(matches!(
            decoded,
            QaReplayCommand::SwordStrike {
                with_seek: false,
                seek_distance: None,
                ..
            }
        ));
    }

    #[test]
    fn shield_macro_and_player_command_roundtrip_all_formats_and_hash_geometry() {
        let protected_pc = EntityId::new(11, crate::element::EntityIdKind::Pc);
        let step = QuickActionStep {
            action: Action::Shield,
            position: MapPoint::new(120.0, 180.0),
            replay: QaReplayCommand::ShieldRaise {
                protected_pc,
                danger_point: crate::coordinates::WorldPoint3D::new(120.0, 205.0, 25.0),
                danger_point_layer: 6,
            },
        };

        let json = serde_json::to_value(step).expect("serialize shield macro as JSON");
        let from_json: QuickActionStep =
            serde_json::from_value(json).expect("roundtrip shield macro JSON");
        assert_eq!(from_json, step);

        let encoded = bitcode::serialize(&step).expect("serialize shield macro as bitcode");
        let from_bitcode: QuickActionStep =
            bitcode::deserialize(&encoded).expect("roundtrip shield macro bitcode");
        assert_eq!(from_bitcode, step);
        assert_eq!(
            bitcode::serialize(&from_bitcode).expect("re-encode shield macro bitcode"),
            encoded
        );

        let bincode_config = bincode::config::standard();
        let encoded = bincode::serde::encode_to_vec(step, bincode_config)
            .expect("serialize shield macro as bincode");
        let (from_bincode, consumed): (QuickActionStep, usize) =
            bincode::serde::decode_from_slice(&encoded, bincode_config)
                .expect("roundtrip shield macro bincode");
        assert_eq!(consumed, encoded.len());
        assert_eq!(from_bincode, step);
        assert_eq!(
            bincode::serde::encode_to_vec(from_bincode, bincode_config)
                .expect("re-encode shield macro bincode"),
            encoded
        );

        let command = crate::player_command::PlayerCommand::RaiseShieldWithDanger {
            actor: EntityId::new(10, crate::element::EntityIdKind::Pc),
            protected_pc,
            danger_point: crate::coordinates::WorldPoint3D::new(120.0, 205.0, 25.0),
            danger_point_layer: 6,
        };
        let json = serde_json::to_value(&command).expect("serialize shield command as JSON");
        let from_json: crate::player_command::PlayerCommand =
            serde_json::from_value(json).expect("roundtrip shield command JSON");
        assert!(matches!(
            &from_json,
            crate::player_command::PlayerCommand::RaiseShieldWithDanger {
                actor,
                protected_pc: decoded_protected,
                danger_point,
                danger_point_layer: 6,
            } if *actor == EntityId::new(10, crate::element::EntityIdKind::Pc)
                && *decoded_protected == protected_pc
                && *danger_point == crate::coordinates::WorldPoint3D::new(120.0, 205.0, 25.0)
        ));

        let encoded = bitcode::serialize(&command).expect("serialize shield command as bitcode");
        let from_bitcode: crate::player_command::PlayerCommand =
            bitcode::deserialize(&encoded).expect("roundtrip shield command bitcode");
        assert_eq!(
            bitcode::serialize(&from_bitcode).expect("re-encode shield command bitcode"),
            encoded
        );

        let encoded = bincode::serde::encode_to_vec(&command, bincode_config)
            .expect("serialize shield command as bincode");
        let (from_bincode, consumed): (crate::player_command::PlayerCommand, usize) =
            bincode::serde::decode_from_slice(&encoded, bincode_config)
                .expect("roundtrip shield command bincode");
        assert_eq!(consumed, encoded.len());
        assert_eq!(
            bincode::serde::encode_to_vec(&from_bincode, bincode_config)
                .expect("re-encode shield command bincode"),
            encoded
        );

        let changed_layer = QuickActionStep {
            replay: QaReplayCommand::ShieldRaise {
                protected_pc,
                danger_point: crate::coordinates::WorldPoint3D::new(120.0, 205.0, 25.0),
                danger_point_layer: 7,
            },
            ..step
        };
        let changed_height = QuickActionStep {
            replay: QaReplayCommand::ShieldRaise {
                protected_pc,
                danger_point: crate::coordinates::WorldPoint3D::new(120.0, 206.0, 26.0),
                danger_point_layer: 6,
            },
            ..step
        };
        let baseline_hash = robin_util::state_hash::compute(&step);
        assert_ne!(
            robin_util::state_hash::compute(&changed_layer),
            baseline_hash
        );
        assert_ne!(
            robin_util::state_hash::compute(&changed_height),
            baseline_hash
        );

        let changed_command = crate::player_command::PlayerCommand::RaiseShieldWithDanger {
            actor: EntityId::new(10, crate::element::EntityIdKind::Pc),
            protected_pc,
            danger_point: crate::coordinates::WorldPoint3D::new(120.0, 206.0, 26.0),
            danger_point_layer: 7,
        };
        assert_ne!(
            robin_util::state_hash::compute(&command),
            robin_util::state_hash::compute(&changed_command)
        );
    }

    #[test]
    fn group_move_route_roundtrips_and_rejects_unresolved_json() {
        let exact = crate::fast_find_grid::SectorIndex::new(37).unwrap();
        let replay = QaReplayCommand::Move {
            destination: MapPoint::new(125.0, 250.0),
            running: true,
            route: RecordedQaMoveRoute {
                goal_sector: crate::sector::SectorNumber::new(421),
                goal_sector_index: exact,
                goal_layer: 6,
            },
        };

        let json = serde_json::to_value(replay).expect("serialize recorded group move");
        assert_eq!(
            serde_json::from_value::<QaReplayCommand>(json.clone())
                .expect("roundtrip recorded group move JSON"),
            replay
        );
        let bitcode = bitcode::serialize(&replay).expect("serialize recorded group move bitcode");
        assert_eq!(
            bitcode::deserialize::<QaReplayCommand>(&bitcode)
                .expect("roundtrip recorded group move bitcode"),
            replay
        );
        let config = bincode::config::standard();
        let bincode = bincode::serde::encode_to_vec(replay, config)
            .expect("serialize recorded group move bincode");
        let (decoded, consumed): (QaReplayCommand, usize) =
            bincode::serde::decode_from_slice(&bincode, config)
                .expect("roundtrip recorded group move bincode");
        assert_eq!(consumed, bincode.len());
        assert_eq!(decoded, replay);

        let mut legacy = json;
        legacy
            .get_mut("Move")
            .and_then(serde_json::Value::as_object_mut)
            .expect("externally tagged group move")
            .remove("route");
        assert!(
            serde_json::from_value::<QaReplayCommand>(legacy).is_err(),
            "an unresolved Rust group move must not enter current QA state"
        );
    }

    #[test]
    fn adopted_legacy_sequence_is_a_live_macro_and_clears_atomically() {
        let pc = EntityId::new(7, crate::element::EntityIdKind::Pc);
        let mut sequence = Sequence::new();
        sequence.append_element(SequenceElement::new(1, Command::Wait, Some(pc)));
        let mut store = MacroStore::new();
        store.adopt_legacy_sequence_slot(pc, 1, sequence, None, None);

        let state = store.get(pc).expect("adopted PC macro state");
        assert!(state.has_macro(1));
        assert_eq!(state.slot(1).expect("slot").len(), 1);
        assert!(state.slot(1).expect("slot").legacy_sequences().is_some());

        store
            .get_mut(pc)
            .expect("adopted PC macro state")
            .clear_slot(1);
        assert!(!store.get(pc).expect("adopted PC macro state").has_macro(1));
    }

    #[test]
    fn adopted_legacy_quickito_is_a_live_macro_and_clears_atomically() {
        let pc = EntityId::new(8, crate::element::EntityIdKind::Pc);
        let target = EntityId::new(9, crate::element::EntityIdKind::Soldier);
        let mut store = MacroStore::new();
        store.adopt_legacy_quickito_slot(
            pc,
            2,
            LegacyQuickito {
                kind: QuickAction::Interact,
                interactor: Some(target),
                button: 0x0008,
            },
            None,
        );

        let state = store.get(pc).expect("adopted PC macro state");
        assert!(state.has_macro(2));
        assert_eq!(state.slot(2).expect("slot").len(), 1);
        assert_eq!(
            state.slot(2).expect("slot").legacy_quickito(),
            Some(LegacyQuickito {
                kind: QuickAction::Interact,
                interactor: Some(target),
                button: 0x0008,
            })
        );

        store
            .get_mut(pc)
            .expect("adopted PC macro state")
            .clear_slot(2);
        assert!(!store.get(pc).expect("adopted PC macro state").has_macro(2));
    }

    #[test]
    fn begin_recording_keeps_occupied_slot_live() {
        let mut s = PcMacroState::default();
        s.begin_recording(0);
        s.append_if_recording(step(Action::Bow, 10.0, 10.0));
        assert_eq!(s.slots[0].len(), 1);

        // Re-arming keeps the live slot intact until a new step commits.
        s.stop_recording();
        s.begin_recording(0);
        assert_eq!(s.slots[0].len(), 1);
        assert!(s.is_recording());
    }

    #[test]
    fn canceling_empty_recording_preserves_previous_slot_and_titbit() {
        let mut state = PcMacroState::default();
        state.begin_recording(0);
        state.append_if_recording(step(Action::Bow, 10.0, 10.0));
        state.stop_recording();
        let titbit = crate::titbit::TitbitId::new(42).expect("valid test titbit");
        state.set_slot_titbit(0, titbit);

        state.begin_recording(0);
        assert_eq!(state.slot(0).expect("armed slot").len(), 1);
        assert_eq!(state.get_slot_titbit(0), Some(titbit));
        state.stop_recording();

        assert_eq!(state.slot(0).expect("restored slot").len(), 1);
        assert_eq!(state.get_slot_titbit(0), Some(titbit));
    }

    #[test]
    fn first_captured_step_atomically_replaces_armed_slot() {
        let mut state = PcMacroState::default();
        state.begin_recording(0);
        state.append_if_recording(step(Action::Bow, 10.0, 10.0));
        state.stop_recording();
        state.set_slot_titbit(
            0,
            crate::titbit::TitbitId::new(42).expect("valid test titbit"),
        );

        state.begin_recording(0);
        state.append_if_recording(step(Action::Hit, 20.0, 20.0));

        let slot = state.slot(0).expect("replacement slot");
        assert_eq!(slot.steps.len(), 1);
        assert_eq!(slot.steps[0].action, Action::Hit);
        assert!(state.get_slot_titbit(0).is_none());
    }

    #[test]
    fn automatic_queue_is_independent_and_expands_beyond_portrait_memory() {
        let pc = EntityId::new(11, crate::element::EntityIdKind::Pc);
        let mut queue = AutoQueueStore::default();
        for index in 0..6 {
            queue.push(pc, step(Action::Bow, index as f32, 0.0));
        }

        assert_eq!(queue.len(pc), 6);
        assert_eq!(queue.pop_front(pc).expect("front").step.position.x, 0.0);
        assert_eq!(queue.len(pc), 5);
        assert_eq!(queue.get(pc).expect("tail")[0].step.position.x, 1.0);
    }

    #[test]
    fn automatic_queue_serde_roundtrip_preserves_multiple_pcs_items_and_titbits() {
        let first_pc = EntityId::new(11, crate::element::EntityIdKind::Pc);
        let second_pc = EntityId::new(12, crate::element::EntityIdKind::Pc);
        let mut queue = AutoQueueStore::default();
        queue.push(first_pc, step(Action::Bow, 1.0, 2.0));
        queue.set_last_titbit(
            first_pc,
            crate::titbit::TitbitId::new(41).expect("valid titbit"),
        );
        queue.push(first_pc, step(Action::Hit, 3.0, 4.0));
        queue.set_last_titbit(
            first_pc,
            crate::titbit::TitbitId::new(42).expect("valid titbit"),
        );
        queue.push(second_pc, step(Action::Stone, 5.0, 6.0));
        queue.set_last_titbit(
            second_pc,
            crate::titbit::TitbitId::new(43).expect("valid titbit"),
        );

        let json = serde_json::to_string(&queue).expect("serialize automatic queues");
        let decoded: AutoQueueStore =
            serde_json::from_str(&json).expect("deserialize automatic queues");

        assert_eq!(decoded, queue);
        assert_eq!(decoded.len(first_pc), 2);
        assert_eq!(decoded.len(second_pc), 1);
        assert_eq!(
            decoded.get(first_pc).expect("first PC queue")[1].titbit,
            crate::titbit::TitbitId::new(42)
        );
        assert_eq!(
            robin_util::state_hash::compute(&decoded),
            robin_util::state_hash::compute(&queue),
            "serialized automatic work must retain deterministic provenance"
        );
    }

    #[test]
    fn automatic_queue_content_and_order_participate_in_state_hash() {
        let pc = EntityId::new(11, crate::element::EntityIdKind::Pc);
        let mut bow_then_hit = AutoQueueStore::default();
        bow_then_hit.push(pc, step(Action::Bow, 1.0, 2.0));
        bow_then_hit.push(pc, step(Action::Hit, 3.0, 4.0));
        let mut hit_then_bow = AutoQueueStore::default();
        hit_then_bow.push(pc, step(Action::Hit, 3.0, 4.0));
        hit_then_bow.push(pc, step(Action::Bow, 1.0, 2.0));

        assert_ne!(
            robin_util::state_hash::compute(&bow_then_hit),
            robin_util::state_hash::compute(&hit_then_bow)
        );
        assert_ne!(
            robin_util::state_hash::compute(&bow_then_hit),
            robin_util::state_hash::compute(&AutoQueueStore::default())
        );
    }

    #[test]
    fn append_is_noop_without_recording() {
        let mut s = PcMacroState::default();
        s.append_if_recording(step(Action::Bow, 10.0, 10.0));
        assert!(s.slots.iter().all(|sl| sl.is_empty()));
    }

    #[test]
    fn stop_commits_slot() {
        let mut s = PcMacroState::default();
        s.begin_recording(1);
        s.append_if_recording(step(Action::Hit, 1.0, 2.0));
        s.append_if_recording(step(Action::Hit, 3.0, 4.0));
        s.stop_recording();

        assert!(!s.is_recording());
        assert!(s.has_macro(1));
        assert!(!s.has_macro(0));
        assert_eq!(s.slot(1).unwrap().len(), 2);
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn begin_recording_panics_on_invalid_slot() {
        PcMacroState::default().begin_recording(NUMBER_OF_QA_MEMORY as u8);
    }

    #[test]
    fn clear_slot_also_stops_recording_and_resets_titbit() {
        let mut s = PcMacroState::default();
        s.begin_recording(2);
        s.set_slot_titbit(2, crate::titbit::TitbitId::new(42).unwrap());
        s.append_if_recording(step(Action::Stone, 0.0, 0.0));
        s.clear_slot(2);
        assert!(!s.is_recording());
        assert!(!s.has_macro(2));
        assert!(s.get_slot_titbit(2).is_none());
    }

    #[test]
    fn slot_titbit_roundtrip() {
        let mut s = PcMacroState::default();
        // Default is INVALID — empty slots have no titbit id.
        assert!(s.get_slot_titbit(0).is_none());
        s.set_slot_titbit(1, crate::titbit::TitbitId::new(7).unwrap());
        assert_eq!(
            s.get_slot_titbit(1),
            Some(crate::titbit::TitbitId::new(7).unwrap())
        );
        assert!(s.get_slot_titbit(0).is_none());
    }

    #[test]
    fn do_tetris_shifts_higher_slots_down() {
        let mut s = PcMacroState::default();
        // slot 0: empty (just completed)
        s.begin_recording(1);
        s.set_slot_titbit(1, crate::titbit::TitbitId::new(101).unwrap());
        s.append_if_recording(step(Action::Bow, 1.0, 1.0));
        s.stop_recording();
        s.begin_recording(2);
        s.set_slot_titbit(2, crate::titbit::TitbitId::new(202).unwrap());
        s.append_if_recording(step(Action::Hit, 2.0, 2.0));
        s.stop_recording();

        s.do_tetris(0);

        // Slot 0 now holds what slot 1 used to hold, slot 1 holds slot 2's,
        // and slot 2 is empty.
        assert!(s.has_macro(0));
        assert_eq!(
            s.get_slot_titbit(0),
            Some(crate::titbit::TitbitId::new(101).unwrap())
        );
        assert!(s.has_macro(1));
        assert_eq!(
            s.get_slot_titbit(1),
            Some(crate::titbit::TitbitId::new(202).unwrap())
        );
        assert!(!s.has_macro(2));
        assert!(s.get_slot_titbit(2).is_none());
    }

    #[test]
    fn do_tetris_on_last_slot_just_clears_it() {
        let mut s = PcMacroState::default();
        s.begin_recording(2);
        s.set_slot_titbit(2, crate::titbit::TitbitId::new(55).unwrap());
        s.append_if_recording(step(Action::Hit, 0.0, 0.0));
        s.stop_recording();

        s.do_tetris(2);

        assert!(!s.has_macro(2));
        assert!(s.get_slot_titbit(2).is_none());
    }

    #[test]
    fn store_isolates_pcs() {
        let mut store = MacroStore::new();
        let a = EntityId::Pc(crate::entity_id::PcId(1));
        let b = EntityId::Pc(crate::entity_id::PcId(2));
        store.get_or_insert(a).begin_recording(0);
        store.append(a, step(Action::Bow, 1.0, 1.0));
        assert!(store.get(a).unwrap().has_macro(0));
        assert!(store.get(b).is_none());
    }

    #[test]
    fn dotted_chain_matches_original_walk() {
        let mut slot = QuickActionSlot::default();
        slot.steps.push(step(Action::Bow, 10.0, 0.0));
        slot.steps.push(step(Action::Hit, 20.0, 0.0));
        slot.steps.push(step(Action::Heal, 20.0, 10.0));

        let segs = dotted_chain_segments(MapPoint::new(0.0, 0.0), &slot);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0], (MapPoint::new(0.0, 0.0), MapPoint::new(10.0, 0.0)));
        assert_eq!(
            segs[1],
            (MapPoint::new(10.0, 0.0), MapPoint::new(20.0, 0.0))
        );
        assert_eq!(
            segs[2],
            (MapPoint::new(20.0, 0.0), MapPoint::new(20.0, 10.0))
        );
    }

    #[test]
    fn dotted_chain_empty_slot_is_empty() {
        let segs = dotted_chain_segments(MapPoint::new(5.0, 5.0), &QuickActionSlot::default());
        assert!(segs.is_empty());
    }

    #[test]
    fn action_to_qa_frame_known_mappings() {
        assert_eq!(action_to_qa_frame(Action::Bow), Some(46));
        assert_eq!(action_to_qa_frame(Action::Hit), Some(12));
        assert_eq!(action_to_qa_frame(Action::HitHard), Some(12));
        assert_eq!(action_to_qa_frame(Action::Stone), Some(10));
        assert_eq!(action_to_qa_frame(Action::Lockpick), Some(20));
        assert_eq!(action_to_qa_frame(Action::NoAction), None);
        assert_eq!(action_to_qa_frame(Action::Jump), None);
    }

    #[test]
    fn posture_toggle_roundtrip_through_slot() {
        let mut s = PcMacroState::default();
        s.begin_recording(0);
        s.append_if_recording(QuickActionStep {
            action: Action::NoAction,
            position: MapPoint::new(0.0, 0.0),
            replay: QaReplayCommand::PostureToggle { to_crouch: true },
        });
        s.append_if_recording(QuickActionStep {
            action: Action::NoAction,
            position: MapPoint::new(0.0, 0.0),
            replay: QaReplayCommand::PostureToggle { to_crouch: false },
        });
        s.stop_recording();
        let slot = s.slot(0).unwrap();
        assert_eq!(slot.len(), 2);
        match slot.steps[0].replay {
            QaReplayCommand::PostureToggle { to_crouch } => assert!(to_crouch),
            _ => panic!("wrong replay variant"),
        }
        match slot.steps[1].replay {
            QaReplayCommand::PostureToggle { to_crouch } => assert!(!to_crouch),
            _ => panic!("wrong replay variant"),
        }

        // Round-trip through JSON.
        let json = serde_json::to_string(&s).unwrap();
        let back: PcMacroState = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn serde_roundtrip() {
        let mut s = PcMacroState::default();
        s.begin_recording(0);
        s.set_slot_titbit(0, crate::titbit::TitbitId::new(99).unwrap());
        s.append_if_recording(step(Action::Bow, 12.0, 34.0));
        s.stop_recording();

        let json = serde_json::to_string(&s).unwrap();
        let back: PcMacroState = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
