//! Existing tick phase implementations; scheduling remains in the parent tick spine.

use super::*;

impl EngineInner {
    /// Advance queued pathfinding and failed-path deadlines before any entity
    /// refresh observes their state.
    ///
    /// The original game processes path requests once before collision and
    /// entity updates, returning at most one completed
    /// request and begins at most one successor at that scheduling point.
    pub(in crate::engine) fn hourglass_phase_paths(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        // Rust computes A* synchronously, but the queue retains the original
        // one-call latency and one-completion-per-frame observation order.
        // Original starts its successor before returning the completed head
        // to the engine, so the scheduler closes that operation before the
        // coordinator applies cross-owner consequences.
        self.trace_path_barrier("enter");
        let completed = self.path_schedule_context().process_requests(
            assets.navigation.pathfinder_graph.as_ref(),
            sim.config().synchronous_pathfinding,
        );
        self.trace_path_barrier("after_schedule");
        self.trace_path_barrier_completed("completed", &completed);
        self.apply_completed_path_work(sim, assets, completed);

        // ── Failed-path timeout ───────────────────────────────────
        // Move / Seek elements whose pathfind failed stay in `InProgress`
        // with empty orders for up to 100 frames without redispatch. Timeouts
        // mark the element `Impossible` and fire
        // `HERO_UNABLE_TO_DO_SOMETHING` for PCs. Classify one entry at a time
        // because each owner's condolation is synchronous and may invalidate
        // a later failed request before Original inspects it.
        while let Some(expired) = self.path_schedule_context().take_next_expired_failure() {
            let request = expired.request;
            if expired.owner_is_pc {
                self.hero_speaking(
                    assets,
                    request.owner,
                    crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                );
            }

            if let Some(element) = self
                .orders
                .sequence_manager
                .get_element_mut(request.seq_id, request.elem_idx)
            {
                element.command = crate::element::Command::MoveOk;
            }
            self.orders
                .sequence_manager
                .element_impossible(request.seq_id, request.elem_idx);
            // Marking a sequence element impossible invokes the
            // owner's removal notification synchronously inside
            // path-processing phase. Close only this timeout's owner
            // boundary here, before collision and every element update;
            // leaving the card queued until the actor's insertion-order slot
            // lets earlier actors consume RNG before EVENT_COULDNT_REACHPOINT.
            self.dispatch_condolations_for_owner_boundary(sim, request.owner, assets);
            tracing::debug!(
                actor = ?request.owner,
                seq_id = ?request.seq_id,
                elem_idx = request.elem_idx,
                age = expired.age,
                "failed_path: 100-frame timeout expired — marking Impossible",
            );
        }

        // The original game's collision check follows path-request processing. Its only
        // implemented response is a human standing inside a non-stopped
        // mobile's motion polygon: launch RECEIVE_MOBILE_DAMAGE for 50/50
        // while the mobile moved last tick, otherwise 10/10.
        let mut humans: Vec<(EntityId, crate::coordinates::MapPoint)> = self
            .world
            .entities
            .humans()
            .map(|(id, human)| (id.into(), human.element_data().position_map()))
            .collect();
        humans.reverse();
        let mut impacts = Vec::new();
        for (human_id, position) in humans {
            for mobile in &self.world.mobile_elements {
                if !mobile.stopped && mobile.contains_point(position) {
                    let amount = if mobile.is_moving() { 50 } else { 10 };
                    impacts.push((human_id, mobile.sprite_ids[0], amount));
                }
            }
        }
        for (human_id, mobile_child, amount) in impacts {
            self.launch_element(crate::sequence::SequenceElement::new_damage(
                1,
                Command::ReceiveMobileDamage,
                Some(human_id),
                Some(mobile_child),
                amount,
                amount,
            ));
        }
    }
}
