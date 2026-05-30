//! Per-PC quick-action macro storage and dotted-chain geometry.
//!
//! Two arrays per PC:
//!
//! * `slots: [QuickActionSlot; NUMBER_OF_QA_MEMORY]` — up to 3 in-flight
//!   recorded macro sequences.
//! * `maul_titbits: [Option<TitbitId>; NUMBER_OF_QA_MEMORY]` — **one
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
use crate::profiles::Action;
use crate::sequence::Field;

/// Maximum number of quick-action macros a single PC can hold.
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
#[derive(
    Debug, Clone, Default, Serialize, Deserialize, PartialEq, robin_state_hash_derive::StateHash,
)]
pub struct QuickActionSlot {
    pub steps: Vec<QuickActionStep>,
}

impl QuickActionSlot {
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
    pub fn len(&self) -> usize {
        self.steps.len()
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
    slots: [QuickActionSlot; NUMBER_OF_QA_MEMORY],
    /// One titbit ID per QA slot, `None` when empty.  Set at the
    /// `AddTitbit(RHTITBIT_QUICKACTION, …)` / `SetQuickActionSequence`
    /// site in the input-action flow.
    maul_titbits: [Option<crate::titbit::TitbitId>; NUMBER_OF_QA_MEMORY],
    recording_slot: Option<u8>,
}

impl Default for PcMacroState {
    fn default() -> Self {
        Self {
            slots: Default::default(),
            maul_titbits: [None; NUMBER_OF_QA_MEMORY],
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
        self.slots.get(idx).map(|s| !s.is_empty()).unwrap_or(false)
    }

    /// Slots in recorded order.  Useful for "render every non-empty slot's
    /// icon strip next to the portrait".
    pub fn slots(&self) -> &[QuickActionSlot; NUMBER_OF_QA_MEMORY] {
        &self.slots
    }

    pub fn is_recording(&self) -> bool {
        self.recording_slot.is_some()
    }

    pub fn recording_slot(&self) -> Option<u8> {
        self.recording_slot
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

    /// Begin recording into `slot_idx`, clearing any previous contents
    /// (the recorder overwrites the slot when arming and the slot was
    /// previously populated).
    pub fn begin_recording(&mut self, slot_idx: u8) {
        assert!(
            (slot_idx as usize) < NUMBER_OF_QA_MEMORY,
            "slot_idx {slot_idx} out of range 0..{NUMBER_OF_QA_MEMORY}"
        );
        self.slots[slot_idx as usize].steps.clear();
        self.maul_titbits[slot_idx as usize] = None;
        self.recording_slot = Some(slot_idx);
    }

    /// Stop recording.  Keeps whatever was appended; the slot is
    /// committed at this point.
    pub fn stop_recording(&mut self) {
        self.recording_slot = None;
    }

    /// Append a step if currently recording.  No-op otherwise.
    pub fn append_if_recording(&mut self, step: QuickActionStep) {
        if let Some(idx) = self.recording_slot {
            self.slots[idx as usize].steps.push(step);
        }
    }

    /// Clear a slot, as the cleanup / abort paths do once a macro has
    /// fired.
    pub fn clear_slot(&mut self, slot_idx: usize) {
        if let Some(s) = self.slots.get_mut(slot_idx) {
            s.steps.clear();
        }
        if let Some(cell) = self.maul_titbits.get_mut(slot_idx) {
            *cell = None;
        }
        if self.recording_slot == Some(slot_idx as u8) {
            self.recording_slot = None;
        }
    }

    /// Shift slots `slot_idx+1 .. NUMBER_OF_QA_MEMORY` down by one,
    /// emptying the final slot.  Called once every PC has completed
    /// a given macro slot so the remaining slots collapse forward.
    ///
    /// Shifted state: `steps` and `maul_titbits[i] = maul_titbits[i+1]`.
    ///
    /// The recording-state guard is defensive — the tetris message is
    /// only posted after every PC has completed slot N, ruling out a
    /// "recording into slot N while slot N tetrises" race.  Kept as a
    /// guard against that invariant breaking.
    pub fn do_tetris(&mut self, slot_idx: usize) {
        if slot_idx >= NUMBER_OF_QA_MEMORY {
            return;
        }
        for i in slot_idx..NUMBER_OF_QA_MEMORY - 1 {
            self.slots.swap(i, i + 1);
            self.maul_titbits[i] = self.maul_titbits[i + 1];
        }
        let last = NUMBER_OF_QA_MEMORY - 1;
        self.slots[last] = QuickActionSlot::default();
        self.maul_titbits[last] = None;
        if let Some(rs) = self.recording_slot
            && (rs as usize) >= slot_idx
        {
            self.recording_slot = None;
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

    /// Append to any PC currently recording.  Convenience wrapper for
    /// the `qa_recording_for == Some(pc)` branch.
    pub fn append(&mut self, pc: EntityId, step: QuickActionStep) {
        self.get_or_insert(pc).append_if_recording(step);
    }

    /// Drop all macros for a PC — used on comabort/death.
    pub fn remove(&mut self, pc: EntityId) {
        self.entries.retain(|(id, _)| *id != pc);
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

    fn step(action: Action, x: f32, y: f32) -> QuickActionStep {
        QuickActionStep {
            action,
            position: MapPoint::new(x, y),
            replay: QaReplayCommand::Move {
                destination: MapPoint::new(x, y),
                running: false,
            },
        }
    }

    #[test]
    fn begin_recording_clears_slot() {
        let mut s = PcMacroState::default();
        s.begin_recording(0);
        s.append_if_recording(step(Action::Bow, 10.0, 10.0));
        assert_eq!(s.slots[0].len(), 1);

        // Re-arming the same slot wipes it (overwrite behaviour).
        s.stop_recording();
        s.begin_recording(0);
        assert_eq!(s.slots[0].len(), 0);
        assert!(s.is_recording());
        assert!(s.get_slot_titbit(0).is_none());
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
        let a = EntityId(1);
        let b = EntityId(2);
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
