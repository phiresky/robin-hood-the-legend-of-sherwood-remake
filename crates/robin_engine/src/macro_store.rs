//! Per-PC quick-action macro storage and dotted-chain geometry.
//!
//! Manual quick actions and the post-port Shift-click queue deliberately use
//! different serialized stores.  Original-compatible QA slots must never be
//! consumed merely because a PC also has automatic work pending.
//!
//! Each PC owns three quick-action slots. A slot owns its action payload,
//! interaction metadata, and marker ID together, so shifting a slot transfers
//! its complete state without synchronizing another representation.
//!
//! Manual dotted chains use the slots' retained titbit identities; automatic
//! queue entries additionally retain their command and marker operands.

use serde::{Deserialize, Serialize};

use crate::coordinates::MapPoint;
use crate::element::{Command, EntityId};
use crate::element_kinds::QuickAction;
use crate::player_command::{CompositeSwordTechnique, GestureQuality};
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
/// The sprite sheet is indexed by quick-action kind: each value's
/// ordinal value is the frame row, selecting the icon for that action.
///
/// Returns `None` for actions that have no dedicated icon in the sheet
/// (e.g. contextual actions like `Climb`, `Jump`, `Search`, …).  These
/// fall through to the default quick-action icon when they do reach
/// the titbit system.
pub fn action_to_qa_frame(action: Action) -> Option<u16> {
    // Frame indices are quick-action ordinals. Keep in sync if the
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
        // No dedicated quick-action icon — renderer skips the per-step overlay.
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
/// The original game stores a concrete coordinate for recorded movement
/// `SEEK` against the already-resolved sector, plus a post-seek arrival
/// speech.  It does not save the common click and run formation placement a
/// second time when the quick action is played.
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct RecordedQaMoveRoute {
    pub goal_sector: crate::sector::SectorNumber,
    pub goal_sector_index: crate::fast_find_grid::SectorIndex,
    pub goal_layer: u16,
}

/// Resolved command operands retained by an automatic queue entry.
/// Manual slots retain authored sequences instead of redispatching commands.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
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
        /// quick action. Required from SAVE55/NET21/REPLAY14 onward.
        route: RecordedQaMoveRoute,
    },
    /// Exact per-unit destination produced by the tactical formation planner.
    #[serde(rename = "AlliedMove")]
    TacticalMove {
        destination: MapPoint,
        running: bool,
        route: RecordedQaMoveRoute,
        formation: crate::tactical_control::TacticalFormation,
    },
    /// Interaction with a specific target entity (attack, heal, tie, …).
    ///
    /// `double_click` records whether the input was a left-double-click
    /// (set when the macro was recorded via the QUICKITOS_INTERRACT /
    /// left-double-click input). On replay we synthesise a leading
    /// single-click dispatch before the recorded double-click — engine
    /// state expects a single click to precede a double.
    Interaction {
        target: EntityId,
        command: Command,
        double_click: bool,
    },
    /// Interaction recorded by the original game's target click handling.
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
    /// Drop-ale seek-then-drop sequence. Replayed as
    /// `PlayerCommand::DropAleAt` with the complete route resolution captured
    /// at the input boundary. Re-querying an already-authorized point can
    /// select a different overlapping floor, so none of this metadata may be
    /// reconstructed during playback.
    DropAle {
        target_pos: MapPoint,
        running: bool,
        already_authorized: bool,
        #[serde(deserialize_with = "Option::deserialize")]
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
        #[serde(deserialize_with = "Option::deserialize")]
        goal_sector_index_override: Option<crate::fast_find_grid::SectorIndex>,
        #[serde(deserialize_with = "Option::deserialize")]
        recorded_gate_path: Option<crate::gate::RecordedGatePath>,
    },
    /// Enter-swordfight engagement on a target.
    Swordfight { target: EntityId, running: bool },
    /// Direct sword strike on a target (mid-swordfight).
    SwordStrike {
        target: EntityId,
        command: Command,
        composite: Option<CompositeSwordTechnique>,
        gesture_quality: GestureQuality,
        with_seek: bool,
        /// Exact seek tolerance captured with the resolved player command.
        /// Explicitly null for a direct strike and required in every current
        /// Rust macro payload.
        #[serde(deserialize_with = "Option::deserialize")]
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
    /// the stand-up message value, which is 1 for the down-arrow widget
    /// and 0 for the up-arrow.
    PostureToggle { to_crouch: bool },
}

/// One recorded action inside a macro slot.  One entry per appended
/// sequence element.
///
/// `position` drives the dotted chain; `replay` carries enough to
/// reconstruct a `PlayerCommand` so `EngineInner::start_quick_action`
/// can re-dispatch each step.  **There is no per-step titbit id**:
/// titbits are registered once per slot,
/// not once per step.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
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
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct QuickActionSlot {
    /// Authored commands retained without executing their orders.
    action_sequence: Option<Sequence>,
    seek_sequence: Option<Sequence>,
    pub(crate) quickito: Quickito,
    titbit: Option<crate::titbit::TitbitId>,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct Quickito {
    pub kind: QuickAction,
    pub interactor: Option<EntityId>,
    pub button: u16,
}

impl PartialEq for QuickActionSlot {
    fn eq(&self, other: &Self) -> bool {
        serde_json::to_value(&self.action_sequence)
            .expect("serialize legacy QA action for equality")
            == serde_json::to_value(&other.action_sequence)
                .expect("serialize legacy QA action for equality")
            && serde_json::to_value(&self.seek_sequence)
                .expect("serialize legacy QA seek for equality")
                == serde_json::to_value(&other.seek_sequence)
                    .expect("serialize legacy QA seek for equality")
            && self.quickito == other.quickito
            && self.titbit == other.titbit
    }
}

impl QuickActionSlot {
    pub fn is_empty(&self) -> bool {
        self.action_sequence.is_none() && self.quickito.kind == QuickAction::None
    }
    pub fn len(&self) -> usize {
        self.action_sequence
            .as_ref()
            .map_or(0, |sequence| sequence.len())
            .max(usize::from(self.quickito.kind != QuickAction::None))
    }

    pub fn sequences(&self) -> Option<(&Sequence, Option<&Sequence>)> {
        self.action_sequence
            .as_ref()
            .map(|action| (action, self.seek_sequence.as_ref()))
    }

    pub(crate) fn retained_sequence_sizes(&self) -> (Option<usize>, Option<usize>) {
        (
            self.action_sequence.as_ref().map(|sequence| sequence.len()),
            self.seek_sequence.as_ref().map(|sequence| sequence.len()),
        )
    }

    pub fn quickito(&self) -> Option<Quickito> {
        (self.quickito.kind != QuickAction::None).then_some(self.quickito)
    }

    pub(crate) fn retained(
        action: Option<Sequence>,
        seek: Option<Sequence>,
        quickito: Quickito,
        titbit: Option<crate::titbit::TitbitId>,
    ) -> Self {
        Self {
            action_sequence: action,
            seek_sequence: seek,
            quickito,
            titbit,
        }
    }
}

/// Per-PC macro state — the recorded slots, the per-slot titbit ids,
/// and the slot currently being appended to.
///
/// `recording_slot` is the slot index currently being appended to when
/// the messenger's macro-recording flag is on and this PC is the target
/// (`qa_recording_for == Some(this pc)`).  `None` means "not recording".
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct PcMacroState {
    slots: [QuickActionSlot; NUMBER_OF_QA_MEMORY],
    /// Counters belong to memory positions and do not move with slot tetris.
    special_counts: [u16; NUMBER_OF_QA_MEMORY],
    recording_slot: Option<u8>,
}

impl Default for PcMacroState {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| QuickActionSlot::default()),
            special_counts: [0; NUMBER_OF_QA_MEMORY],
            recording_slot: None,
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
        self.slots.get(idx).is_some_and(|s| !s.is_empty())
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

    /// Read the retained marker ID, including a consumed slot awaiting tetris.
    pub fn get_slot_titbit(&self, slot: usize) -> Option<crate::titbit::TitbitId> {
        self.slots.get(slot).and_then(|slot| slot.titbit)
    }

    /// Write a slot's titbit id.  Called from the
    /// `set_quick_action_sequence` flow once the recorder knows which
    /// titbit id to associate with the slot.
    pub fn set_slot_titbit(&mut self, slot: usize, id: crate::titbit::TitbitId) {
        if let Some(slot) = self.slots.get_mut(slot) {
            slot.titbit = Some(id);
        }
    }

    pub fn clear_slot_titbit(&mut self, slot: usize) {
        if let Some(slot) = self.slots.get_mut(slot) {
            slot.titbit = None;
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

        self.recording_slot = Some(slot_idx);
    }

    /// Stop recording while preserving the slot's retained payload.
    pub fn stop_recording(&mut self) {
        self.recording_slot = None;
    }

    /// Replace the armed slot's commands while preserving its marker identity.
    pub fn retain_sequence(&mut self, action: Sequence, seek: Option<Sequence>) {
        if let Some(slot) = self.recording_slot {
            let slot = usize::from(slot);
            let metadata = self.slots[slot].quickito;
            let titbit = self.slots[slot].titbit;
            self.slots[slot] = QuickActionSlot::retained(
                Some(action),
                seek,
                Quickito {
                    kind: QuickAction::None,
                    ..metadata
                },
                titbit,
            );
            self.special_counts[slot] = 0;
        }
    }
    pub fn retain_quickito(&mut self, quickito: Quickito) {
        if let Some(slot) = self.recording_slot {
            let slot = usize::from(slot);
            let titbit = self.slots[slot].titbit;
            self.slots[slot] = QuickActionSlot::retained(None, None, quickito, titbit);
            self.special_counts[slot] = 0;
        }
    }

    /// Clear a slot, as the cleanup / abort paths do once a macro has
    /// fired.
    pub fn clear_slot(&mut self, slot_idx: usize) {
        if let Some(s) = self.slots.get_mut(slot_idx) {
            s.action_sequence = None;
            s.seek_sequence = None;
            s.quickito = Quickito::default();
            s.titbit = None;
        }
        if self.recording_slot == Some(slot_idx as u8) {
            self.recording_slot = None;
        }
    }

    /// Consume a quick action while retaining its marker identity until tetris.
    pub(crate) fn complete_slot(&mut self, slot: usize) {
        let titbit = self.slots[slot].titbit;
        self.clear_slot(slot);
        self.slots[slot].titbit = titbit;
    }

    pub(crate) fn complete_sequence_slot(&mut self, slot: usize) {
        let metadata = self.slots[slot].quickito;
        self.complete_slot(slot);
        self.slots[slot].quickito = metadata;
        self.reset_special_count(slot);
    }

    pub(crate) fn special_count(&self, slot: usize) -> u16 {
        self.special_counts[slot]
    }

    pub(crate) fn reset_special_count(&mut self, slot: usize) {
        self.special_counts[slot] = 0;
    }

    pub(crate) fn deactivate_slot(&mut self, slot: usize) {
        let metadata = self.slots[slot].quickito;
        self.clear_slot(slot);
        self.slots[slot].quickito = Quickito {
            kind: QuickAction::None,
            ..metadata
        };
        self.reset_special_count(slot);
    }

    pub(crate) fn adopt_slot(&mut self, slot: usize, value: QuickActionSlot, special_count: u16) {
        self.slots[slot] = value;
        self.special_counts[slot] = special_count;
    }

    /// Shift every later slot down by one. Called once every PC has completed
    /// a given macro slot so the remaining slots collapse forward.
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
        }
        let last = NUMBER_OF_QA_MEMORY - 1;
        self.slots[last] = QuickActionSlot::default();
        if let Some(rs) = self.recording_slot
            && (rs as usize) >= slot_idx
        {
            self.recording_slot = None;
        }
    }
}

/// One automatic Shift-click item. Unlike an Original macro slot, a queue
/// item always contains exactly one resolved command.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
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
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    PartialEq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
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

    pub(crate) fn last_step_mut(&mut self, actor: EntityId) -> Option<&mut QuickActionStep> {
        self.entries
            .iter_mut()
            .find(|(id, _)| *id == actor)
            .and_then(|(_, queue)| queue.last_mut())
            .map(|entry| &mut entry.step)
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
            QaReplayCommand::Move { running, .. }
            | QaReplayCommand::TacticalMove { running, .. }
            | QaReplayCommand::DropAle { running, .. } => {
                *running = true;
                Some(index)
            }
            _ => None,
        }
    }
}

/// Authoritative per-PC quick-action state. Keeping it separate from entity
/// components allows recording and validation to borrow actor state directly.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequence::SequenceElement;

    fn action_sequence(action: Action) -> Sequence {
        let command = match action {
            Action::Bow => Command::ShootBow,
            Action::Hit => Command::HitCmd,
            Action::Stone => Command::ThrowStone,
            _ => panic!("unsupported test action {action:?}"),
        };
        let mut sequence = Sequence::new();
        sequence.append_element(SequenceElement::new(
            1,
            command,
            Some(EntityId::Pc(crate::entity_id::PcId(1))),
        ));
        sequence
    }

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
    fn sword_macro_preserves_resolved_seek_distance_and_rejects_legacy_field_omission() {
        let target = EntityId::new(9, crate::element::EntityIdKind::Soldier);
        let replay = QaReplayCommand::SwordStrike {
            target,
            command: Command::SwordstrikeThrustA,
            composite: Some(CompositeSwordTechnique::RisingFeint),
            gesture_quality: GestureQuality::GOOD,
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
                composite: Some(CompositeSwordTechnique::RisingFeint),
                gesture_quality,
                ..
            } if gesture_quality == GestureQuality::GOOD
        ));

        let mut pre_gesture = encoded.clone();
        let payload = pre_gesture
            .get_mut("SwordStrike")
            .and_then(serde_json::Value::as_object_mut)
            .expect("externally tagged sword macro");
        payload.remove("composite");
        payload.remove("gesture_quality");
        assert!(
            serde_json::from_value::<QaReplayCommand>(pre_gesture).is_err(),
            "a native sword macro without gesture fields must not enter current QA state"
        );

        let mut legacy = encoded;
        legacy
            .get_mut("SwordStrike")
            .and_then(serde_json::Value::as_object_mut)
            .expect("externally tagged sword macro")
            .remove("seek_distance");
        assert!(
            serde_json::from_value::<QaReplayCommand>(legacy).is_err(),
            "a Rust sword macro without seek_distance must not enter current QA state"
        );

        let direct = QaReplayCommand::SwordStrike {
            target,
            command: Command::SwordstrikeThrustF,
            composite: None,
            gesture_quality: GestureQuality::PERFECT,
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

        let json = serde_json::to_value(&step).expect("serialize shield macro as JSON");
        let from_json: QuickActionStep =
            serde_json::from_value(json).expect("roundtrip shield macro JSON");
        assert_eq!(from_json, step);

        let encoded = bitcode::encode(&step);
        let from_bitcode: QuickActionStep =
            bitcode::decode(&encoded).expect("roundtrip shield macro bitcode");
        assert_eq!(from_bitcode, step);
        assert_eq!(bitcode::encode(&from_bitcode), encoded);

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

        let encoded = bitcode::encode(&command);
        let from_bitcode: crate::player_command::PlayerCommand =
            bitcode::decode(&encoded).expect("roundtrip shield command bitcode");
        assert_eq!(bitcode::encode(&from_bitcode), encoded);

        let changed_layer = QuickActionStep {
            replay: QaReplayCommand::ShieldRaise {
                protected_pc,
                danger_point: crate::coordinates::WorldPoint3D::new(120.0, 205.0, 25.0),
                danger_point_layer: 7,
            },
            ..step.clone()
        };
        let changed_height = QuickActionStep {
            replay: QaReplayCommand::ShieldRaise {
                protected_pc,
                danger_point: crate::coordinates::WorldPoint3D::new(120.0, 206.0, 26.0),
                danger_point_layer: 6,
            },
            ..step.clone()
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

        let json = serde_json::to_value(&replay).expect("serialize recorded group move");
        assert_eq!(
            serde_json::from_value::<QaReplayCommand>(json.clone())
                .expect("roundtrip recorded group move JSON"),
            replay
        );
        let bitcode = bitcode::encode(&replay);
        assert_eq!(
            bitcode::decode::<QaReplayCommand>(&bitcode)
                .expect("roundtrip recorded group move bitcode"),
            replay
        );

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
    fn drop_ale_macro_serialization_preserves_route_and_rejects_legacy_metadata_omission() {
        let source_index = crate::fast_find_grid::SectorIndex::new(17).unwrap();
        let goal_index = crate::fast_find_grid::SectorIndex::new(23).unwrap();
        let replay = QaReplayCommand::DropAle {
            target_pos: MapPoint::new(50.0, 75.0),
            running: true,
            already_authorized: true,
            goal_override: Some((crate::sector::SectorNumber::new(42), 3)),
            goal_sector_index_override: Some(goal_index),
            recorded_gate_path: Some(crate::gate::RecordedGatePath {
                source_sector: crate::sector::SectorNumber::new(7),
                source_sector_index: Some(source_index),
                source_layer: 2,
                outcome: crate::gate::RecordedGateOutcome::Success(vec![
                    crate::gate::GatePathStep {
                        door_index: crate::gate::DoorIndex::new(11).expect("valid door index"),
                        direct: false,
                    },
                ]),
            }),
        };

        let json = serde_json::to_value(&replay).expect("serialize resolved DropAle macro");
        let decoded_json: QaReplayCommand =
            serde_json::from_value(json.clone()).expect("roundtrip resolved DropAle macro JSON");
        assert_eq!(decoded_json, replay);
        let bytes = bitcode::encode(&replay);
        let decoded_bitcode: QaReplayCommand =
            bitcode::decode(&bytes).expect("roundtrip resolved DropAle macro bitcode");
        assert_eq!(decoded_bitcode, replay);
        assert_eq!(bitcode::encode(&decoded_bitcode), bytes);

        let mut changed_route = replay.clone();
        let QaReplayCommand::DropAle {
            recorded_gate_path: Some(route),
            ..
        } = &mut changed_route
        else {
            panic!("resolved DropAle fixture lost its gate route")
        };
        let crate::gate::RecordedGateOutcome::Success(gates) = &mut route.outcome else {
            panic!("resolved DropAle fixture route must succeed")
        };
        gates[0].direct = true;
        assert_ne!(
            robin_util::state_hash::compute(&changed_route),
            robin_util::state_hash::compute(&replay),
            "recorded gate direction must participate in StateHash"
        );

        let mut changed_goal = replay.clone();
        let QaReplayCommand::DropAle {
            goal_sector_index_override,
            ..
        } = &mut changed_goal
        else {
            panic!("resolved DropAle fixture changed variant")
        };
        *goal_sector_index_override = crate::fast_find_grid::SectorIndex::new(24);
        assert_ne!(
            robin_util::state_hash::compute(&changed_goal),
            robin_util::state_hash::compute(&replay),
            "exact goal arena must participate in StateHash"
        );

        let mut legacy = json;
        let fields = legacy
            .get_mut("DropAle")
            .and_then(serde_json::Value::as_object_mut)
            .expect("externally tagged DropAle macro");
        fields.remove("already_authorized");
        fields.remove("goal_override");
        fields.remove("goal_sector_index_override");
        fields.remove("recorded_gate_path");
        assert!(
            serde_json::from_value::<QaReplayCommand>(legacy).is_err(),
            "a Rust DropAle macro without resolved route metadata must not enter the current schema"
        );
    }

    #[test]
    fn adopted_legacy_sequence_is_a_live_macro_and_clears_atomically() {
        let pc = EntityId::new(7, crate::element::EntityIdKind::Pc);
        let mut sequence = Sequence::new();
        sequence.append_element(SequenceElement::new(1, Command::Wait, Some(pc)));
        let mut store = MacroStore::new();
        store.get_or_insert(pc).adopt_slot(
            1,
            QuickActionSlot::retained(Some(sequence), None, Quickito::default(), None),
            0,
        );

        let state = store.get(pc).expect("adopted PC macro state");
        assert!(state.has_macro(1));
        assert_eq!(state.slot(1).expect("slot").len(), 1);
        assert!(state.slot(1).expect("slot").sequences().is_some());

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
        store.get_or_insert(pc).adopt_slot(
            2,
            QuickActionSlot::retained(
                None,
                None,
                Quickito {
                    kind: QuickAction::Interact,
                    interactor: Some(target),
                    button: 0x0008,
                },
                None,
            ),
            0,
        );

        let state = store.get(pc).expect("adopted PC macro state");
        assert!(state.has_macro(2));
        assert_eq!(state.slot(2).expect("slot").len(), 1);
        assert_eq!(
            state.slot(2).expect("slot").quickito(),
            Some(Quickito {
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
        s.retain_sequence(action_sequence(Action::Bow), None);
        assert_eq!(s.slots[0].len(), 1);

        // Re-arming keeps the live slot intact until a new sequence is retained.
        s.stop_recording();
        s.begin_recording(0);
        assert_eq!(s.slots[0].len(), 1);
        assert!(s.is_recording());
    }

    #[test]
    fn retained_sequence_presence_is_independent_of_element_count_and_seek_storage() {
        let mut state = PcMacroState::default();
        state.adopt_slot(
            0,
            QuickActionSlot::retained(Some(Sequence::new()), None, Quickito::default(), None),
            0,
        );
        state.adopt_slot(
            1,
            QuickActionSlot::retained(None, Some(Sequence::new()), Quickito::default(), None),
            0,
        );
        assert!(state.has_macro(0));
        assert!(!state.has_macro(1));
        assert_eq!(
            state.slot(0).unwrap().retained_sequence_sizes(),
            (Some(0), None)
        );
        assert_eq!(
            state.slot(1).unwrap().retained_sequence_sizes(),
            (None, Some(0))
        );
    }

    #[test]
    fn completion_and_deactivation_preserve_distinct_retained_metadata() {
        let target = EntityId::new(9, crate::element::EntityIdKind::Soldier);
        let titbit = Some(crate::titbit::TitbitId::new(41).unwrap());
        let metadata = Quickito {
            kind: QuickAction::None,
            interactor: Some(target),
            button: 8,
        };
        let sequence_slot = QuickActionSlot::retained(
            Some(Sequence::new()),
            Some(Sequence::new()),
            metadata,
            titbit,
        );
        let mut state = PcMacroState::default();
        state.adopt_slot(0, sequence_slot, 7);
        state.complete_sequence_slot(0);
        assert!(!state.has_macro(0));
        assert_eq!(state.slots[0].quickito, metadata);
        assert_eq!(state.get_slot_titbit(0), titbit);
        assert_eq!(state.special_count(0), 0);
        assert_eq!(state.slots[0].retained_sequence_sizes(), (None, None));

        let quickito_slot = QuickActionSlot::retained(
            None,
            None,
            Quickito {
                kind: QuickAction::Interact,
                ..metadata
            },
            titbit,
        );
        state.adopt_slot(1, quickito_slot.clone(), 8);
        state.complete_slot(1);
        assert!(!state.has_macro(1));
        assert_eq!(state.slots[1].quickito, Quickito::default());
        assert_eq!(state.get_slot_titbit(1), titbit);
        assert_eq!(state.special_count(1), 8);

        state.adopt_slot(2, quickito_slot, 9);
        state.deactivate_slot(2);
        assert!(!state.has_macro(2));
        assert_eq!(state.slots[2].quickito, metadata);
        assert_eq!(state.get_slot_titbit(2), None);
        assert_eq!(state.special_count(2), 0);
    }

    #[test]
    fn tetris_moves_retained_payloads_but_leaves_special_counts_at_memory_positions() {
        let mut state = PcMacroState::default();
        for slot in 0..NUMBER_OF_QA_MEMORY {
            state.adopt_slot(
                slot,
                QuickActionSlot::retained(
                    Some(Sequence::new()),
                    None,
                    Quickito {
                        button: slot as u16 + 10,
                        ..Quickito::default()
                    },
                    Some(crate::titbit::TitbitId::new(slot as u32 + 20).unwrap()),
                ),
                slot as u16 + 30,
            );
        }
        let second = state.slots[1].clone();
        let third = state.slots[2].clone();
        state.complete_sequence_slot(0);
        state.do_tetris(0);
        assert_eq!(state.slots, [second, third, QuickActionSlot::default()]);
        assert_eq!(state.special_counts, [0, 31, 32]);
    }

    #[test]
    fn recording_into_seek_only_slot_discards_stale_payload_but_keeps_interaction_metadata() {
        let mut state = PcMacroState::default();
        let metadata = Quickito {
            kind: QuickAction::None,
            interactor: Some(EntityId::new(9, crate::element::EntityIdKind::Soldier)),
            button: 8,
        };
        state.adopt_slot(
            0,
            QuickActionSlot::retained(
                None,
                Some(Sequence::new()),
                metadata,
                Some(crate::titbit::TitbitId::new(41).unwrap()),
            ),
            7,
        );
        state.begin_recording(0);
        state.retain_sequence(action_sequence(Action::Bow), None);
        assert_eq!(state.slots[0].retained_sequence_sizes(), (Some(1), None));
        assert_eq!(state.slots[0].quickito, metadata);
        assert_eq!(
            state.get_slot_titbit(0),
            Some(crate::titbit::TitbitId::new(41).unwrap())
        );
        assert_eq!(state.special_count(0), 0);
        let marker = crate::titbit::TitbitId::new(42).unwrap();
        state.set_slot_titbit(0, marker);
        state.retain_sequence(action_sequence(Action::Bow), None);
        assert_eq!(state.get_slot_titbit(0), Some(marker));
        assert_eq!(state.slots[0].len(), 1);
    }

    #[test]
    fn canceling_empty_recording_preserves_previous_slot_and_titbit() {
        let mut state = PcMacroState::default();
        state.begin_recording(0);
        state.retain_sequence(action_sequence(Action::Bow), None);
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
    fn captured_sequence_replaces_armed_slot_and_keeps_assigned_marker() {
        let mut state = PcMacroState::default();
        state.begin_recording(0);
        state.retain_sequence(action_sequence(Action::Bow), None);
        state.stop_recording();
        state.set_slot_titbit(
            0,
            crate::titbit::TitbitId::new(42).expect("valid test titbit"),
        );

        state.begin_recording(0);
        state.retain_sequence(action_sequence(Action::Hit), None);

        let slot = state.slot(0).expect("replacement slot");
        assert_eq!(slot.len(), 1);
        assert_eq!(
            slot.sequences().unwrap().0.elements[0].command,
            Command::HitCmd
        );
        assert_eq!(
            state.get_slot_titbit(0),
            Some(crate::titbit::TitbitId::new(42).unwrap())
        );
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
    fn retain_sequence_is_noop_without_recording() {
        let mut s = PcMacroState::default();
        s.retain_sequence(action_sequence(Action::Bow), None);
        assert!(s.slots.iter().all(|sl| sl.is_empty()));
    }

    #[test]
    fn stop_preserves_last_retained_sequence() {
        let mut s = PcMacroState::default();
        s.begin_recording(1);
        s.retain_sequence(action_sequence(Action::Hit), None);
        s.retain_sequence(action_sequence(Action::Hit), None);
        s.stop_recording();

        assert!(!s.is_recording());
        assert!(s.has_macro(1));
        assert!(!s.has_macro(0));
        assert_eq!(s.slot(1).unwrap().len(), 1);
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
        s.retain_sequence(action_sequence(Action::Stone), None);
        s.set_slot_titbit(2, crate::titbit::TitbitId::new(42).unwrap());
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
        s.retain_sequence(action_sequence(Action::Bow), None);
        s.set_slot_titbit(1, crate::titbit::TitbitId::new(101).unwrap());
        s.stop_recording();
        s.begin_recording(2);
        s.retain_sequence(action_sequence(Action::Hit), None);
        s.set_slot_titbit(2, crate::titbit::TitbitId::new(202).unwrap());
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
        s.retain_sequence(action_sequence(Action::Hit), None);
        s.set_slot_titbit(2, crate::titbit::TitbitId::new(55).unwrap());
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
        store
            .get_or_insert(a)
            .retain_sequence(action_sequence(Action::Bow), None);
        assert!(store.get(a).unwrap().has_macro(0));
        assert!(store.get(b).is_none());
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
        s.retain_quickito(Quickito {
            kind: QuickAction::GoDown,
            ..Quickito::default()
        });
        s.retain_quickito(Quickito {
            kind: QuickAction::GoUp,
            ..Quickito::default()
        });
        s.stop_recording();
        let slot = s.slot(0).unwrap();
        assert_eq!(slot.len(), 1);
        assert_eq!(slot.quickito().unwrap().kind, QuickAction::GoUp);

        // Round-trip through JSON.
        let json = serde_json::to_string(&s).unwrap();
        let back: PcMacroState = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn serde_roundtrip() {
        let mut s = PcMacroState::default();
        s.begin_recording(0);
        s.retain_sequence(action_sequence(Action::Bow), None);
        s.set_slot_titbit(0, crate::titbit::TitbitId::new(99).unwrap());
        s.stop_recording();

        let json = serde_json::to_string(&s).unwrap();
        let back: PcMacroState = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
