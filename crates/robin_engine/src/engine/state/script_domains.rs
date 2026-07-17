use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

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
    pub(crate) scrolls: ScrollState,
}
