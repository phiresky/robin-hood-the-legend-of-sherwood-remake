//! Deterministic consequences of completing an Original Linux-v48 load.
//!
//! These values are deliberately not another serialized-state slice. Original
//! mutates or rebuilds them while unwinding `RHEngine::Serialize`: active NPC
//! remarks are completed during each local-AI read, the process-wide RNG is
//! reseeded by `SerializeAllAI`, its global forbidden-remark list is cleared,
//! and PC produced noise is reconstructed only after sequence pointers have
//! been fixed. The remaining click/selection/trajectory values are host-owned
//! transient state in Rust and are returned as an explicit output.

use thiserror::Error;

use crate::{
    ai::Remark,
    element::{Entity, EntityId},
    engine::{EngineInner, LevelAssets},
};

use super::{
    LegacySaveAbiProfile,
    adopt::{LegacyEntityFixups, LegacySaveAdoptError},
    payload_dispatch::{LegacyElementPayload, LegacyElementPayloadStream},
    post_tail::LegacyGlobalAiState,
};

/// Whether a load should reproduce Original's ordinary `srand(saved_seed)` or
/// retain the parity replay's authoritative ordered draw stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyRngRestorePolicy {
    /// Normal interactive Original-save loading.
    RestoreSavedSeed,
    /// Replay loading: callback RNG draws come from the recorded stream and
    /// that stream must remain installed after adoption.
    PreserveRecordedGlobalDrawStream,
}

/// Exact host-owned transient resets performed by Original after a read.
///
/// The fields are named separately so the host coordinator cannot silently
/// substitute a broad `InputState::default()` and reset unrelated UI state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LegacyPostLoadHostOutput {
    pub clear_element_old_click: bool,
    pub clear_ignore_next_left_click: bool,
    pub clear_ignore_next_drag: bool,
    pub clear_next_left_double_is_simple: bool,
    pub clear_trajectory_preview: bool,
    pub clear_target_drag: bool,
    pub clear_focus: bool,
    pub clear_multi_selection: bool,
    pub clear_multi_unselection: bool,
    pub clear_draw_multi_selection: bool,
    pub reset_selected_layer: bool,
    pub clear_selected_sector: bool,
    pub clear_selected_patch: bool,
    pub clear_display_door: bool,
    /// `RHEngine::mbValidity = false`: invalidate host-rendered background
    /// state before the first post-load display.
    pub invalidate_background_cache: bool,
    /// Original deletes its transient thunder-effect list.
    pub clear_transient_thunders: bool,
    /// Original resets `RHSightObstacle` and every projection area's
    /// last-viewed cache. Rust computes sight from current state and has no
    /// persistent equivalent, so the host only needs to invalidate any
    /// derived visualization/cache it owns.
    pub invalidate_last_viewed_caches: bool,
}

impl LegacyPostLoadHostOutput {
    const ORIGINAL_READ_RESET: Self = Self {
        clear_element_old_click: true,
        clear_ignore_next_left_click: true,
        clear_ignore_next_drag: true,
        clear_next_left_double_is_simple: true,
        clear_trajectory_preview: true,
        clear_target_drag: true,
        clear_focus: true,
        clear_multi_selection: true,
        clear_multi_unselection: true,
        clear_draw_multi_selection: true,
        reset_selected_layer: true,
        clear_selected_sector: true,
        clear_selected_patch: true,
        clear_display_door: true,
        invalidate_background_cache: true,
        clear_transient_thunders: true,
        invalidate_last_viewed_caches: true,
    };
}

#[derive(Debug, Error)]
pub enum LegacyPostLoadAdoptError {
    #[error(transparent)]
    Reference(#[from] LegacySaveAdoptError),
    #[error("post-load consequences currently support Linux-i386 v48 saves, not {0:?}")]
    UnsupportedAbi(LegacySaveAbiProfile),
    #[error("saved NPC creation order {creation_order} resolves to missing entity {entity_id}")]
    MissingNpc {
        creation_order: u32,
        entity_id: EntityId,
    },
    #[error("saved NPC creation order {creation_order} resolves to non-NPC entity {entity_id}")]
    WrongEntityKind {
        creation_order: u32,
        entity_id: EntityId,
    },
    #[error(
        "saved NPC creation order {creation_order} has unknown current-remark enum value {value}"
    )]
    UnknownCurrentRemark { creation_order: u32, value: i32 },
}

/// Fully validated, infallible post-load mutation plan.
#[derive(Clone, Debug)]
pub struct LegacyPostLoadAdoptionPlan {
    active_remark_completions: Vec<(EntityId, u16)>,
    saved_rng_seed: u64,
    rng_policy: LegacyRngRestorePolicy,
}

impl LegacyPostLoadAdoptionPlan {
    pub fn preflight(
        engine: &EngineInner,
        payloads: &LegacyElementPayloadStream,
        global_ai: &LegacyGlobalAiState,
        entities: &LegacyEntityFixups,
        rng_policy: LegacyRngRestorePolicy,
    ) -> Result<Self, LegacyPostLoadAdoptError> {
        let mut active_remark_completions = Vec::new();
        for record in &payloads.records {
            let local_ai = match &record.payload {
                LegacyElementPayload::ActorNpcSoldier(saved) => &saved.npc.local_ai,
                LegacyElementPayload::ActorNpcCivilian(saved) => &saved.npc.local_ai,
                _ => continue,
            };
            let raw = local_ai.common.current_remark;
            let remark = u32::try_from(raw)
                .ok()
                .and_then(|value| Remark::try_from(value).ok())
                .ok_or(LegacyPostLoadAdoptError::UnknownCurrentRemark {
                    creation_order: record.header.creation_order,
                    value: raw,
                })?;
            let entity_id = entities
                .by_creation_order
                .get(&record.header.creation_order)
                .copied()
                .ok_or(LegacySaveAdoptError::MissingCreationOrderReference {
                    creation_order: record.header.creation_order,
                })?;
            let runtime = engine.world.entities.get(entity_id).ok_or(
                LegacyPostLoadAdoptError::MissingNpc {
                    creation_order: record.header.creation_order,
                    entity_id,
                },
            )?;
            if !matches!(runtime, Entity::Soldier(_) | Entity::Civilian(_)) {
                return Err(LegacyPostLoadAdoptError::WrongEntityKind {
                    creation_order: record.header.creation_order,
                    entity_id,
                });
            }
            if remark != Remark::TheSoundOfSilence {
                active_remark_completions.push((entity_id, local_ai.common.current_remark_flags));
            }
        }

        Ok(Self {
            active_remark_completions,
            // C `srand` receives an unsigned int. Preserve the producer's
            // signed 32-bit time_t bit pattern before widening for Rust.
            saved_rng_seed: u64::from(global_ai.saved_random_seed.0 as u32),
            rng_policy,
        })
    }

    /// Apply after all ordinary serialized entity/sequence state has landed.
    ///
    /// Remark callbacks deliberately run before seed restoration: that is
    /// where they occur in Original's stream order. Produced noise is rebuilt
    /// last because it reads the fixed-up active order.
    pub(crate) fn apply(
        self,
        engine: &mut EngineInner,
        assets: &LevelAssets,
    ) -> LegacyPostLoadHostOutput {
        engine.complete_legacy_loaded_remarks(&self.active_remark_completions, assets);
        engine.restore_loaded_active_movements();
        engine.restore_loaded_active_shots();
        crate::abilities::restore_loaded_active_abilities(
            &mut engine.world.entities,
            &engine.orders.sequence_manager,
        );

        if self.rng_policy == LegacyRngRestorePolicy::RestoreSavedSeed {
            engine.restore_rng_from_seed(self.saved_rng_seed);
        }
        engine.ai.global.forbidden_remarks.clear();
        engine.refresh_legacy_loaded_produced_noise();

        LegacyPostLoadHostOutput::ORIGINAL_READ_RESET
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ai::{ForbiddenRemark, RemarkTargetFlags},
        engine::EngineInner,
    };

    #[test]
    fn signed_time_t_keeps_original_srand_bit_pattern() {
        assert_eq!(u64::from((-6_i32) as u32), 0xffff_fffa);
    }

    #[test]
    fn apply_restores_normal_rng_clears_only_global_forbids_and_returns_exact_resets() {
        let mut engine = EngineInner::new();
        engine.ai.global.forbidden_remarks.push(ForbiddenRemark {
            remark: Remark::Arrow,
            flags: RemarkTargetFlags::THIS_TYPE.bits(),
            speech_id: 7,
            guy_index: 1,
            bad_guy: true,
            forbidden_till_frame: 99,
        });
        let plan = LegacyPostLoadAdoptionPlan {
            active_remark_completions: Vec::new(),
            saved_rng_seed: 0xffff_fffa,
            rng_policy: LegacyRngRestorePolicy::RestoreSavedSeed,
        };

        let output = plan.apply(&mut engine, &LevelAssets::default());

        assert_eq!(engine.control.rng.seed(), 0xffff_fffa);
        assert!(engine.ai.global.forbidden_remarks.is_empty());
        assert_eq!(output, LegacyPostLoadHostOutput::ORIGINAL_READ_RESET);
    }

    #[test]
    fn replay_policy_does_not_replace_recorded_draw_stream() {
        let mut engine = EngineInner::new();
        engine.control.rng = crate::engine::SimulationRng::with_original_replay(vec![11, 22]);
        let plan = LegacyPostLoadAdoptionPlan {
            active_remark_completions: Vec::new(),
            saved_rng_seed: 123,
            rng_policy: LegacyRngRestorePolicy::PreserveRecordedGlobalDrawStream,
        };

        plan.apply(&mut engine, &LevelAssets::default());

        let draw = engine.with_simulation_context(|_, sim| {
            crate::sim_rng::u32(sim, crate::sim_rng::RngSite::TitbitUpdate, ..)
        });
        assert_eq!(draw, 11);
    }
}
