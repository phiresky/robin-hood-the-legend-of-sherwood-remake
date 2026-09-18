//! Read-only parity projections, separate from the mutation facade.

use super::*;

#[path = "parity_state/manager_projections.rs"]
mod manager_projections;
#[cfg(test)]
#[path = "parity_state/manager_tests.rs"]
mod manager_tests;

#[path = "parity_state/projections.rs"]
mod projections;

#[path = "parity_state/entity_runtime.rs"]
mod entity_runtime;

#[path = "parity_state/projectile_projections.rs"]
mod projectile_projections;

#[cfg(test)]
#[path = "parity_state/projectile_tests.rs"]
mod projectile_tests;

#[cfg(test)]
#[path = "parity_state/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "parity_state/golden.rs"]
mod golden;

#[cfg(test)]
#[path = "parity_state/npc_tests.rs"]
mod npc_tests;

#[path = "parity_state/human_projections.rs"]
mod human_projections;
#[cfg(test)]
#[path = "parity_state/human_tests.rs"]
mod human_tests;

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum ParityEntityKind {
    Pc,
    Soldier,
    Civilian,
    Fx,
    Target,
    Bonus,
    Scroll,
    Projectile,
    Net,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ParityEntityReference {
    kind: ParityEntityKind,
    index: u32,
}

#[cfg(test)]
fn parity_entity_reference(id: EntityId) -> serde_json::Value {
    serde_json::to_value(typed_entity_reference(id))
        .expect("typed parity entity reference must serialize")
}

fn typed_entity_reference(id: EntityId) -> ParityEntityReference {
    use crate::element::EntityIdKind;
    let kind = match id.kind() {
        EntityIdKind::Pc => ParityEntityKind::Pc,
        EntityIdKind::Soldier => ParityEntityKind::Soldier,
        EntityIdKind::Civilian => ParityEntityKind::Civilian,
        EntityIdKind::Fx => ParityEntityKind::Fx,
        EntityIdKind::Target => ParityEntityKind::Target,
        EntityIdKind::Bonus => ParityEntityKind::Bonus,
        EntityIdKind::Scroll => ParityEntityKind::Scroll,
        EntityIdKind::Projectile => ParityEntityKind::Projectile,
        EntityIdKind::Net => ParityEntityKind::Net,
    };
    ParityEntityReference {
        kind,
        index: id.index(),
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ParityFloat {
    bits: u32,
    value: f32,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ParityGameUiState {
    campaign_map: bool,
    campaign_map_displayed: bool,
    post_initialized: bool,
    start_mission_disabled_temp: bool,
    quit_mission_disabled_temp: bool,
    start_mission_enabled: bool,
    quit_mission_enabled: bool,
}

#[cfg(test)]
fn parity_float(value: f32) -> serde_json::Value {
    serde_json::to_value(typed_float(value)).expect("typed parity float must serialize")
}

fn typed_float(value: f32) -> ParityFloat {
    ParityFloat {
        bits: value.to_bits(),
        value,
    }
}

fn typed_seek_point(
    point: &crate::ai::SeekPoint,
    position: projections::AiPosition,
) -> projections::SeekPoint<'_> {
    projections::SeekPoint {
        position,
        frame_when_full_interest: point.frame_when_full_interest,
        directions: std::borrow::Cow::Borrowed(&point.directions),
        last_calculated_interest: point.last_calculated_interest,
        locked: point.locked,
    }
}

impl Engine {
    /// Complete serialized position and sprite frontier for one entity.
    ///
    /// Returned as `serde_json::Value` because parity comparison
    /// (`robin_parity`) and save tests index it as a JSON subset; the schema
    /// itself is the typed [`projections::EntityRuntime`].
    #[doc(hidden)]
    pub fn parity_entity_runtime_state(
        &self,
        id: EntityId,
        assets: &LevelAssets,
    ) -> serde_json::Value {
        serde_json::to_value(
            entity_runtime::EntityRuntimeProjector::new(self, id, assets).project(),
        )
        .expect("typed entity parity envelope must serialize")
    }

    /// Exact serialized game mission/controller latches. Host widgets
    /// mirror these values but do not own their authoritative state.
    #[doc(hidden)]
    pub fn parity_game_ui_state(&self) -> serde_json::Value {
        let ui = &self.inner.script_domains.mission_ui;
        serde_json::to_value(ParityGameUiState {
            campaign_map: ui.campaign_map,
            campaign_map_displayed: ui.campaign_map_displayed,
            post_initialized: ui.game_post_initialized,
            start_mission_disabled_temp: ui.start_mission_disabled_temp,
            quit_mission_disabled_temp: ui.quit_mission_disabled_temp,
            start_mission_enabled: ui.start_mission_enabled,
            quit_mission_enabled: ui.quit_mission_enabled,
        })
        .expect("typed parity UI state must serialize")
    }
}
