use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::sequence::RecordingSession;

/// Mutable state whose lifetime is the mission script, rather than one
/// particular native-dispatch adapter.
///
/// Handles stored in VM heaps can refer to `computed_locations`, and an open
/// sequence recording can span multiple native calls. Consequently all of
/// these values are part of rollback/save state and have exactly one owner:
/// [`crate::engine::MissionScript`].
#[derive(
    Clone,
    Debug,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ScriptState {
    /// Cross-script global variables (`InitGlobal`/`SetGlobal`/`GetGlobal`).
    pub globals: BTreeMap<i32, i32>,
    /// Locations allocated by script natives, in handle-allocation order.
    /// Original-game location-storage order. `None` preserves a
    /// missing native location: the original game inserts that empty slot in
    /// the storage list, so it still shifts every later location handle.
    pub computed_locations: Vec<Option<ComputedScriptLocation>>,
    /// State of an in-progress `Start`/`Then`/`Thanx` recording.
    pub sequence_recorder: SequenceRecorderState,
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ComputedScriptLocation {
    pub position: (f32, f32),
    /// Runtime-created points may lack spatial attachment. Legacy-loaded
    /// locations preserve layer and sector independently because an Original
    /// point can carry a layer while its serialized sector pointer is null.
    pub layer: Option<u16>,
    pub sector: Option<u16>,
    /// Exact live counterpart of `sector` when the location was copied from
    /// a complete saved position. Legacy snapshots retain the number-only field above.
    #[serde(default)]
    pub sector_handle: Option<crate::position_interface::SectorHandle>,
    /// Serialized script-point flags. New runtime points use
    /// `active = true`, `legacy_dummy = false`.
    pub active: bool,
    pub legacy_dummy: bool,
}

#[derive(
    Clone,
    Debug,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SequenceRecorderState {
    pub recording: Option<RecordingSession>,
    /// Assigned by the earlier port's chained script-step handling. It is
    /// currently not read and is retained only until the recorder itself can
    /// be simplified.
    pub sequence_id: i32,
}
