use serde::{Deserialize, Serialize};

/// Engine-owned deterministic state shared with mission-script natives.
///
/// During the legacy five-field script transaction this value is moved, never
/// copied, into `GameHost` beside the other temporarily leased engine state.
/// Domain-specific state is added here one owner at a time.
#[derive(Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub(crate) struct ScriptDomains {}
