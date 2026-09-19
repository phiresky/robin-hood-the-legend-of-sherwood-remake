use super::*;
use crate::engine::TickCtx;

impl EngineInner {
    #[cfg(test)]
    pub(in crate::engine) fn tick_enemy_ai(&mut self, tcx: TickCtx<'_>) {
        // This detection-only test seam predates the production owner walk.
        // Preserve its contract that all PCs have refreshed their noise before
        // the first NPC is evaluated.
        let pc_ids = self.world.pc_ids.clone();
        for pc_id in pc_ids {
            self.refresh_pc_produced_noise_for(pc_id);
        }
        self.tick_enemy_ai_inner(tcx, false);
    }

    /// Legacy test coordinator for complete NPC updates without actor movement.
    ///
    /// Each NPC consumes only its own body/recovery work and refreshes its
    /// own view immediately before its creation-ordered detection refresh.
    /// The direct `tick_enemy_ai` entry point remains detection-only for
    /// focused tests that construct already-refreshed vision state.
    #[cfg(test)]
    pub(in crate::engine) fn tick_enemy_ai_with_creation_ordered_prelude(
        &mut self,
        tcx: TickCtx<'_>,
    ) {
        self.tick_enemy_ai_inner(tcx, true);
    }

    /// Initialize transient actor counters before the fused owner pass.
    pub(in crate::engine) fn prepare_npc_owner_pass(&mut self) {
        if !self.ai.global.primary_target_multiplicity_initialized {
            // The human actor's primary-target multiplicity is temporary initialization state and
            // is explicitly absent from the save stream. Loading a save into
            // newly-created actors therefore starts every counter at zero,
            // even if restored AI state already describes a swordfight.
            self.ai.global.primary_target_multiplicity_scratch.clear();
            self.ai.global.primary_target_multiplicity_initialized = true;
        }
    }

    /// Run one NPC's complete post-human envelope using live inputs sampled at
    /// this legacy slot. No later owner's view or forecast is constructed.
    pub(in crate::engine) fn tick_npc_owner_pass(&mut self, tcx: TickCtx<'_>, npc_id: EntityId) {
        self.debug_refresh_view_lifecycle("npc_tail_enter", npc_id, None);
        let entity = self.expect_entity(npc_id, "NPC owner before its fused legacy-slot envelope");
        assert!(
            entity.ai_actor_data().is_some(),
            "fused AI owner {} has no AI actor data",
            npc_id.index()
        );
        // FrozenAll is volatile script state. Sample it at the consuming NPC
        // slot rather than caching it before earlier owners run callbacks.
        if self.actors_frozen() {
            self.debug_refresh_view_lifecycle("npc_tail_frozen_skip", npc_id, None);
            self.tick_npc_post_detection_tail_for_npc(tcx, npc_id);
            return;
        }

        self.tick_inform_my_friends_for_npc(npc_id);
        self.refresh_npc_view_for_npc(npc_id);
        self.tick_enemy_ai_refresh_detection(tcx, npc_id);
        self.tick_npc_post_detection_tail_for_npc(tcx, npc_id);
    }

    pub(in crate::engine) fn tick_enemy_ai_blip_detection_for_owner(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
    ) -> Option<crate::sprite::MotionState> {
        self.tick_enemy_ai_blip_detection(tcx, owner)
    }

    #[cfg(test)]
    fn tick_enemy_ai_inner(&mut self, tcx: TickCtx<'_>, run_owner_envelope: bool) {
        if self.actors_frozen() {
            // Frozen-all skips patrol/view/detection/ambush/deafness but the
            // original still enters each NPC's busy/ladder/speech/lock gate,
            // where all three deadlines are extended before returning.
            if run_owner_envelope {
                let npc_ids: Vec<_> = self.entities().ai_owner_ids().collect();
                for npc_id in npc_ids {
                    self.tick_npc_post_detection_tail_for_npc(tcx, npc_id);
                }
            }
            return;
        }
        if !self.ai.global.primary_target_multiplicity_initialized {
            self.ai.global.primary_target_multiplicity_scratch.clear();
            self.ai.global.primary_target_multiplicity_initialized = true;
        }

        // ── 2a. Listen/object blip work. ────────────────────────
        // NPC-owned SeesBlip remains inside its creation-ordered
        // detection-refresh slot below.
        let pc_ids = self.world.pc_ids.clone();
        for pc_id in pc_ids {
            self.tick_enemy_ai_blip_detection(tcx, pc_id);
        }

        // Test drivers explicitly choose either a complete NPC envelope or
        // detection alone. Geometry arguments never select scheduling phases.
        let owners: Vec<_> = self.entities().ai_owner_ids().collect();
        for npc_id in owners {
            if run_owner_envelope {
                self.tick_inform_my_friends_for_npc(npc_id);
                self.refresh_npc_view_for_npc(npc_id);
            }
            self.tick_enemy_ai_refresh_detection(tcx, npc_id);
            if run_owner_envelope {
                self.tick_npc_post_detection_tail_for_npc(tcx, npc_id);
            }
        }

        // Sword strikes are launched by `engine::melee::tick_enemy_sword_attacks`.
        // Keep this AI pass to target selection, pursuit, and swordfight
        // requests; applying direct damage here would bypass the
        // wait-timer + interaction sequence timing.
    }
}
