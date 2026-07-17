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
#[derive(Clone, Debug, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ScriptState {
    /// Cross-script global variables (`InitGlobal`/`SetGlobal`/`GetGlobal`).
    pub globals: BTreeMap<i32, i32>,
    /// Locations allocated by script natives, in handle-allocation order.
    pub computed_locations: Vec<ComputedScriptLocation>,
    /// State of an in-progress `Start`/`Then`/`Thanx` recording.
    pub sequence_recorder: SequenceRecorderState,
}

#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub struct ComputedScriptLocation {
    pub position: (f32, f32),
    pub layer_sector: Option<(u16, u16)>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SequenceRecorderState {
    pub recording: Option<RecordingSession>,
    /// Assigned by the original port's `Start`/`Then` implementation. It is
    /// currently not read, but remains serialized until shipped-save/script
    /// auditing proves it can be removed compatibly.
    pub sequence_id: i32,
}
