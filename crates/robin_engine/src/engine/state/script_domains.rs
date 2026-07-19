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

/// Deterministic mission UI controls and the script-requested victory latch.
///
/// These values are queried or mutated by both ordinary engine systems and
/// script natives, so the native adapter only leases this single owner during
/// a callback. Presentation changes derived from the values remain effects.
#[derive(Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub(crate) struct MissionUiState {
    pub(crate) outline_display: bool,
    pub(crate) force_check: bool,
    pub(crate) men_to_blazon_conversion_mode: bool,
    pub(crate) blinking_blazons: u32,
    pub(crate) blink_expire_frame: u32,
}

impl Default for MissionUiState {
    fn default() -> Self {
        Self {
            outline_display: false,
            force_check: false,
            men_to_blazon_conversion_mode: false,
            blinking_blazons: 0,
            blink_expire_frame: u32::MAX,
        }
    }
}

impl MissionUiState {
    const BLINK_TIMEOUT: u32 = 50;

    pub(crate) fn set_blinking_blazons(&mut self, count: u32, frame_counter: u32) {
        self.blinking_blazons = count;
        self.blink_expire_frame = if count == 0 {
            u32::MAX
        } else {
            frame_counter.saturating_add(Self::BLINK_TIMEOUT)
        };
    }

    pub(crate) fn active_blinking_blazons(&self, frame_counter: u32) -> u32 {
        (frame_counter < self.blink_expire_frame)
            .then_some(self.blinking_blazons)
            .unwrap_or(0)
    }
}

/// Deterministic scroll state shared by engine systems and script natives.
#[derive(Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub(crate) struct ScrollState {
    pub(crate) status: BTreeMap<i32, i32>,
    pub(crate) attachments: BTreeMap<i32, i32>,
    pub(crate) attachment_dirty: BTreeSet<i32>,
}

/// Initialization-only production registrations emitted by mission scripts.
///
/// The Original applies these after the complete Initialize callback batch,
/// before initial zone occupants are populated. Keeping the ordered buffer on
/// the canonical script domain preserves that boundary without treating the
/// requests as host or presentation effects.
#[derive(Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub(crate) struct ProductionInitialization {
    pub(crate) sectors: Vec<(i32, i32, i32)>,
    pub(crate) points: Vec<(i32, i32)>,
}

/// Engine-owned deterministic state shared with mission-script natives.
///
/// `EngineInner` lends each native resume a typed mutable borrow. It is never
/// copied or parked in `ScriptEffects`.
#[derive(Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
/// Canonical deterministic state shared by the engine and script natives.
///
/// The fields remain engine-internal; the public type lets external native
/// adapters hold and pass the typed capability without exposing domain
/// implementation details.
#[doc(hidden)]
pub struct ScriptDomains {
    pub(crate) buildings: BuildingState,
    pub(crate) interactables: InteractableState,
    #[serde(default)]
    pub(crate) mission_ui: MissionUiState,
    pub(crate) production_initialization: ProductionInitialization,
    pub(crate) scrolls: ScrollState,
    pub(crate) zones: ZoneState,
}
