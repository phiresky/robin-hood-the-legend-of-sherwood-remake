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

#[derive(serde::Serialize, serde::Deserialize)]
struct ParityMessengerState {
    view_locked: bool,
    selected_action: u32,
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

    /// Read-only schema-13 parity view of gameplay-authoritative global state.
    #[doc(hidden)]
    pub fn parity_engine_state(&self) -> ParityEngineState {
        let mission = &self.inner.mission_domain.state;
        let seat = &self.inner.players.seats[0];
        ParityEngineState {
            cheat_used_flags: self.inner.mission_domain.cheat_used_flags,
            next_creation_order: self.inner.world.next_original_creation_order,
            chorus_timer: self.inner.control.chorus_timer,
            force_check: self.inner.script_domains.mission_ui.force_check,
            men_to_blazon_conversion: self
                .inner
                .script_domains
                .mission_ui
                .men_to_blazon_conversion_mode,
            lock_engine: self.inner.control.simulation_gates.engine_locked(),
            freeze_all: self.inner.control.simulation_gates.actors_frozen(),
            locker: seat.locker_active,
            speed: self.inner.control.speed,
            speed_int: self.inner.control.speed_int,
            mission_won: mission.mission_won,
            mission_won_first_time: mission.mission_won_first_time,
            quit_won: mission.quit_won,
            quit_lost: mission.quit_lost,
            quit_interrupted: mission.quit_interrupted,
            script_globals: self.inner.scripts.globals.clone(),
        }
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

    /// Serialized messenger controller state that remains gameplay-visible.
    #[doc(hidden)]
    pub fn parity_messenger_controller_state(&self) -> serde_json::Value {
        serde_json::to_value(ParityMessengerState {
            view_locked: self.inner.players.view_locked,
            selected_action: self.inner.players.seats[0].selected_action as u32,
        })
        .expect("typed parity messenger state must serialize")
    }

    /// Serialized engine-global two-click shield controller. This is separate
    /// from each PC's active shield links in `pc_tail`.
    #[doc(hidden)]
    pub fn parity_shield_controller_state(&self) -> serde_json::Value {
        let entity = |id: EntityId| {
            assert!(
                matches!(id.kind(), crate::element::EntityIdKind::Pc),
                "shield controller protects non-PC entity {:?}",
                id.kind()
            );
            typed_entity_reference(id)
        };
        let shield = &self.inner.world.shield;
        let bits = |value: f32| projections::FloatBits {
            bits: value.to_bits(),
        };
        serde_json::to_value(projections::ShieldController {
            is_protected: shield.is_protected,
            protected_pc: shield.protected_pc.map(entity),
            danger_point: projections::Point3Bits {
                x: bits(shield.danger_point.x),
                y: bits(shield.danger_point.y),
                z: bits(shield.danger_point.z),
            },
        })
        .expect("typed shield controller parity must serialize")
    }

    /// Sparse, serialized sound-source manager state. Host channels and
    /// backend playback queues are deliberately absent; these are the source
    /// fields that survive Original save/load and feed later simulation.
    #[doc(hidden)]
    pub fn parity_sound_sources_state(&self) -> serde_json::Value {
        let float = typed_float;
        let sources = &self.inner.feedback.sound_sim.sources;
        let mut result = Vec::with_capacity(sources.num_sources());
        for index in 0..sources.num_sources() {
            let Some(source) = sources.get(index) else {
                result.push(None);
                continue;
            };
            let kind = match source.source_kind {
                crate::sound_source::SoundSourceKind::Single => 0,
                crate::sound_source::SoundSourceKind::Looped => 1,
                crate::sound_source::SoundSourceKind::Delayed => 2,
                crate::sound_source::SoundSourceKind::Volatile => 3,
            };
            let altitude = match source.altitude {
                crate::sound_geometry::SoundSourceAltitude::Ground => 0,
                crate::sound_geometry::SoundSourceAltitude::Middle => 1,
                crate::sound_geometry::SoundSourceAltitude::Top => 2,
                crate::sound_geometry::SoundSourceAltitude::NoAltitude => 3,
            };
            result.push(Some(projections::SoundSource {
                kind: kind,
                id: source.id,
                global: source.is_global,
                inner_distance: source.inner_distance,
                outer_distance: source.outer_distance,
                noise_covering_distance: source.noise_covering_distance,
                inner_volume: source.inner_volume,
                outer_volume: source.outer_volume,
                shape: source
                    .shape
                    .iter()
                    .map(|point| projections::Point2 {
                        x: float(point.x),
                        y: float(point.y),
                    })
                    .collect::<Vec<_>>(),
                altitude: altitude,
                min_delay: source.min_delay,
                max_delay: source.max_delay,
                delay_stepping: source.delay_stepping,
                timer: source.timer,
                active: source.active,
                ambience_enabled: source.ambience_enabled,
            }));
        }
        serde_json::to_value(result).expect("typed sound source parity must serialize")
    }

    /// Ordered deterministic source-completion deadlines. Looped sources have
    /// no completion entry; Single, Volatile, and Delayed sources retain the
    /// order in which Original queued their pending playback records.
    #[doc(hidden)]
    pub fn parity_sound_completion_frontier_state(&self) -> serde_json::Value {
        serde_json::to_value(
            self.inner
                .feedback
                .sound_sim
                .playing_sources
                .iter()
                .map(|playing| {
                    if self
                        .inner
                        .feedback
                        .sound_sim
                        .sources
                        .get(playing.source_index as usize)
                        .is_none()
                    {
                        panic!(
                            "sound completion frontier references missing source {}",
                            playing.source_index
                        );
                    }
                    projections::SoundCompletion {
                        source_index: playing.source_index,
                        finish_frame: playing.finish_frame,
                    }
                })
                .collect::<Vec<_>>(),
        )
        .expect("typed sound completion parity must serialize")
    }

    /// Serialized global AI-manager state. Mission-static seek/archery
    /// geometry is intentionally absent; Original persists only these ordered
    /// mutable statuses, reservations, counters, alerts, and saved RNG seed.
    #[doc(hidden)]
    pub fn parity_ai_global_state(&self) -> serde_json::Value {
        let entity = typed_entity_reference;
        let global = &self.inner.ai.global;
        serde_json::to_value(projections::GlobalAi {
            stupid_soldiers_cheat: global.stupid_soldiers_cheat,
            seek_points: global
                .seek_points
                .iter()
                .map(|point| projections::SeekPointStatus {
                    frame_when_full_interest: point.frame_when_full_interest,
                    last_calculated_interest: point.last_calculated_interest,
                    locked: point.locked,
                })
                .collect::<Vec<_>>(),
            archery_sectors: global
                .archery_sectors
                .iter()
                .map(|sector| projections::ArcherySector {
                    num_owners: sector.num_owners,
                    point_owners: sector
                        .points
                        .iter()
                        .map(|point| point.owner.map(&entity))
                        .collect::<Vec<_>>(),
                })
                .collect::<Vec<_>>(),
            green_alert_soldiers: global.green_alert_soldiers,
            yellow_alert_soldiers: global.yellow_alert_soldiers,
            red_alert_soldiers: global.red_alert_soldiers,
            overall_alert_status: global.overall_alert_status as u32,
            overall_villain_alert_status: global.overall_villain_alert_status as u32,
            saved_random_seed: global.saved_random_seed,
            forbidden_remarks: global
                .forbidden_remarks
                .iter()
                .map(|entry| projections::ForbiddenRemark {
                    remark: entry.remark as u32,
                    flags: entry.flags,
                    speech_id: entry.speech_id,
                    // This is deliberately the stored scalar, not a normalized
                    // entity reference. The original game stores creation order here;
                    // parity must expose any slot-vs-creation-order divergence.
                    guy_index: entry.guy_index,
                    bad_guy: entry.bad_guy,
                    forbidden_till_frame: entry.forbidden_till_frame,
                })
                .collect::<Vec<_>>(),
            current_speech_variant: global.current_speech_variant,
        })
        .expect("typed global AI parity must serialize")
    }
}
