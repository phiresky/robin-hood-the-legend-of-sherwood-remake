use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub(crate) struct ZoneState {
    pub(crate) scripts: Vec<crate::sector::ScriptSectorData>,
}

#[derive(Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub(crate) struct BuildingState {
    pub(crate) occupants: Vec<Vec<i32>>,
    pub(crate) arrow_reserves: Vec<bool>,
    pub(crate) actor_building: BTreeMap<i32, i32>,
    pub(crate) active: Vec<bool>,
    pub(crate) gates: Vec<Vec<i32>>,
}

#[derive(Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub(crate) struct InteractableState {
    pub(crate) doors: Vec<crate::gate::Door>,
    pub(crate) patches: Vec<crate::patch::Patch>,
}

/// Deterministic scroll state shared by engine systems and script natives.
#[derive(Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub(crate) struct ScrollState {
    pub(crate) status: BTreeMap<i32, i32>,
    pub(crate) attachments: BTreeMap<i32, i32>,
    pub(crate) attachment_dirty: BTreeSet<i32>,
}

/// Engine-owned deterministic state shared with mission-script natives.
///
/// During the legacy five-field script transaction this value is moved, never
/// copied, into `GameHost` beside the other temporarily leased engine state.
/// Domain-specific state is added here one owner at a time.
#[derive(Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub(crate) struct ScriptDomains {
    pub(crate) buildings: BuildingState,
    pub(crate) interactables: InteractableState,
    pub(crate) scrolls: ScrollState,
    pub(crate) zones: ZoneState,
}
