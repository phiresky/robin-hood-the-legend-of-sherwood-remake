//! Opponent list and swordfight engagement (enter/quit/principal).
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;
use crate::combat::{self};
use crate::element::{ActionState, Command, Entity, EntityId, Posture};
use crate::order::OrderType;
use crate::sequence::SequenceElementData;

thread_local! {
    /// Original's `RHElementActorHuman::mpSwordfightPreparationScope` is a
    /// non-serialized synchronous call-stack guard. Key the equivalent
    /// transient stack by the actor being prepared as well as their opponent:
    /// separate actors and nested preparations against a different opponent
    /// must remain admissible.
    static SWORDFIGHT_PREPARATION_STACK: std::cell::RefCell<Vec<(EntityId, EntityId)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

struct SwordfightPreparationGuard {
    pair: (EntityId, EntityId),
}

impl Drop for SwordfightPreparationGuard {
    fn drop(&mut self) {
        SWORDFIGHT_PREPARATION_STACK.with(|stack| {
            let popped = stack.borrow_mut().pop();
            assert_eq!(
                popped,
                Some(self.pair),
                "swordfight preparation scopes must unwind in call order"
            );
        });
    }
}

/// Run the synchronous `PrepareToEnterSwordFight` body unless this exact
/// actor/opponent pair is already being prepared higher in the callback stack.
///
/// Original provenance: `RHSwordfightPreparationScope` and
/// `RHElementActorHuman::PrepareToEnterSwordFight` in
/// `original-code/RHelementactorhuman.cpp:221-245,7757-7789`.
fn with_swordfight_preparation_scope<R>(
    actor: EntityId,
    opponent: EntityId,
    body: impl FnOnce() -> R,
) -> Option<R> {
    let pair = (actor, opponent);
    let already_preparing = SWORDFIGHT_PREPARATION_STACK
        .with(|stack| stack.borrow().iter().any(|active| *active == pair));
    if already_preparing {
        return None;
    }
    SWORDFIGHT_PREPARATION_STACK.with(|stack| stack.borrow_mut().push(pair));
    let _guard = SwordfightPreparationGuard { pair };
    Some(body())
}

pub(in crate::engine) fn active_swordfight_preparation() -> Option<(EntityId, EntityId)> {
    SWORDFIGHT_PREPARATION_STACK.with(|stack| stack.borrow().last().copied())
}

pub(in crate::engine) fn with_deferred_swordfight_preparation<R>(
    pair: (EntityId, EntityId),
    body: impl FnOnce() -> R,
) -> R {
    let already_active = SWORDFIGHT_PREPARATION_STACK
        .with(|stack| stack.borrow().iter().any(|active| *active == pair));
    if already_active {
        return body();
    }
    SWORDFIGHT_PREPARATION_STACK.with(|stack| stack.borrow_mut().push(pair));
    let _guard = SwordfightPreparationGuard { pair };
    body()
}

#[cfg(test)]
mod preparation_scope_tests {
    use super::{
        active_swordfight_preparation, with_deferred_swordfight_preparation,
        with_swordfight_preparation_scope,
    };
    use crate::element::{Command, EntityId, PcId, SoldierId};
    use crate::sequence::{SequenceElement, SequenceElementRef, SequenceManager};

    #[test]
    fn reciprocal_same_pair_reentry_allocates_once() {
        let actor = EntityId::Soldier(SoldierId(82));
        let opponent = EntityId::Pc(PcId(134));
        let mut allocated_reciprocals = Vec::new();

        let outer = with_swordfight_preparation_scope(actor, opponent, || {
            allocated_reciprocals.push(1);
            let nested = with_swordfight_preparation_scope(actor, opponent, || {
                allocated_reciprocals.push(2);
            });
            assert!(nested.is_none());
        });

        assert!(outer.is_some());
        assert_eq!(
            allocated_reciprocals,
            [1],
            "same-pair callback reentry must not allocate another reciprocal sequence"
        );
    }

    #[test]
    fn different_pair_nesting_remains_admissible() {
        let actor = EntityId::Soldier(SoldierId(82));
        let first_opponent = EntityId::Pc(PcId(134));
        let second_opponent = EntityId::Pc(PcId(135));
        let mut preparations = Vec::new();

        with_swordfight_preparation_scope(actor, first_opponent, || {
            preparations.push(first_opponent);
            let nested = with_swordfight_preparation_scope(actor, second_opponent, || {
                preparations.push(second_opponent);
            });
            assert!(nested.is_some());
        })
        .expect("outer preparation should run");

        assert_eq!(preparations, [first_opponent, second_opponent]);
    }

    #[test]
    fn deferred_same_pair_token_is_one_shot_and_cancellation_drops_it() {
        let actor = EntityId::Soldier(SoldierId(82));
        let opponent = EntityId::Pc(PcId(134));
        let pair = (actor, opponent);
        let mut manager = SequenceManager::new();

        let sequence_id = with_swordfight_preparation_scope(actor, opponent, || {
            let sequence_id = manager.launch_element(SequenceElement::new_generic(
                1,
                Command::EnterSwordfight,
                Some(opponent),
            ));
            manager.attach_swordfight_preparation(
                SequenceElementRef::new(sequence_id, 0),
                active_swordfight_preparation().expect("preparation scope must be active"),
            );
            sequence_id
        })
        .expect("outer preparation should run");

        let token = manager
            .take_swordfight_preparation(SequenceElementRef::new(sequence_id, 0))
            .expect("deferred Enter must inherit its preparation scope");
        let mut repeated_prepare_ran = false;
        with_deferred_swordfight_preparation(token, || {
            assert!(
                with_swordfight_preparation_scope(actor, opponent, || {
                    repeated_prepare_ran = true;
                })
                .is_none()
            );
        });
        assert!(!repeated_prepare_ran);
        assert!(
            manager
                .take_swordfight_preparation(SequenceElementRef::new(sequence_id, 0))
                .is_none(),
            "dispatch consumes the continuation token exactly once"
        );
        assert!(
            with_swordfight_preparation_scope(actor, opponent, || {}).is_some(),
            "a later independent admission remains allowed"
        );

        let cancelled_id = manager.launch_element(SequenceElement::new_generic(
            1,
            Command::EnterSwordfight,
            Some(opponent),
        ));
        manager.attach_swordfight_preparation(SequenceElementRef::new(cancelled_id, 0), pair);
        manager.element_impossible(cancelled_id, 0);
        assert!(
            manager
                .take_swordfight_preparation(SequenceElementRef::new(cancelled_id, 0))
                .is_none(),
            "terminal cancellation must not leak a preparation token"
        );
    }
}

fn opponent_order_debug_matches(frame: u32, owner: EntityId) -> bool {
    if std::env::var_os("PARITY_DEBUG_OPPONENT_ORDER").is_none() {
        return false;
    }
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for opponent-order diagnostic: {error}")
            })
        })
    };
    parse_filter("PARITY_DEBUG_OPPONENT_ORDER_FRAME").is_none_or(|value| value == frame)
        && parse_filter("PARITY_DEBUG_OPPONENT_ORDER_OWNER")
            .is_none_or(|value| value == owner.index())
}

/// `RHLineJump::GetAssociatedJumpLine()` — the paired jump line on the far
/// side of a table.  `None` when the aggressor is not fighting across a
/// table, or when the level's jump line has no associated partner.
fn associated_jump_line(
    engine: &EngineInner,
    aggressor_jump_line: Option<crate::jump_line::JumpLineIndex>,
) -> Option<crate::jump_line::JumpLineIndex> {
    aggressor_jump_line.and_then(|aggr| {
        engine
            .world
            .fast_grid
            .level
            .jump_lines
            .get(usize::from(aggr))
            .and_then(|jl| jl.associated_line_index)
            .and_then(crate::jump_line::JumpLineIndex::new)
    })
}

impl EngineInner {
    // ─── Tie-up (public, called from natives/UI) ────────────────────

    // ─── Opponent list management ────────────────────────────────

    /// Add `opponent` to `entity`'s opponent list at index 0
    /// (principal slot).
    ///
    /// - If the opponent is already in the list at index > 0, swap
    ///   it to the front and overwrite the jump line at slot 0;
    ///   return `false` (already known).
    /// - If already at index 0, just leave it (return `false`).
    /// - Otherwise insert `(opponent, jump_line)` at the front;
    ///   return `true` (new).
    ///
    /// Returns `true` when this is a fresh addition.  The
    /// fighting-ability recompute and smalltalk-initiative
    /// side-effects are kept at the call sites
    /// (`enter_swordfight` / `take_smalltalk_initiative`) because
    /// they need access to `&mut self` and the asset profile manager
    /// that this helper, scoped over the entity slice, can't reach.
    pub(crate) fn add_opponent(
        entities: &mut crate::entities::Entities,
        entity_id: EntityId,
        opponent_id: EntityId,
        jump_line: Option<crate::jump_line::JumpLineIndex>,
    ) -> bool {
        // Original's method receives typed human pointers. Preserve that
        // precondition instead of conflating an invalid owner/opponent with
        // the legitimate `false` result for an already-present opponent.
        entities
            .get(opponent_id)
            .unwrap_or_else(|| panic!("AddOpponent opponent {opponent_id:?} is missing"))
            .human_data()
            .unwrap_or_else(|| panic!("AddOpponent opponent {opponent_id:?} is not human"));

        let human = entities
            .get_mut(entity_id)
            .unwrap_or_else(|| panic!("AddOpponent owner {entity_id:?} is missing"))
            .human_data_mut()
            .unwrap_or_else(|| panic!("AddOpponent owner {entity_id:?} is not human"));
        human.opponents.add_principal(opponent_id, jump_line)
    }

    /// Remove `opponent` from `entity`'s opponent list.
    pub(super) fn remove_opponent(
        entities: &mut crate::entities::Entities,
        entity_id: EntityId,
        opponent_id: EntityId,
    ) -> bool {
        entities
            .get_mut(entity_id)
            .unwrap_or_else(|| panic!("DeleteOpponent owner {entity_id:?} is missing"))
            .human_data_mut()
            .unwrap_or_else(|| panic!("DeleteOpponent owner {entity_id:?} is not human"))
            .opponents
            .remove(opponent_id)
    }

    /// Remove one opponent with the authoritative side effects of C++
    /// `RHElementActorHuman::DeleteOpponent`.
    ///
    /// In particular, deleting the final entry recursively reaches
    /// `QuitSwordFight`, which synchronously sends `EVENT_QUIT_SWORDFIGHT` to
    /// a living soldier.  Callers which merely edit the vectors hide that
    /// callback and can leave an orphaned fighter executing its old combat
    /// substate.
    pub(super) fn delete_opponent(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        opponent_id: EntityId,
    ) -> bool {
        if !Self::remove_opponent(&mut self.world.entities, entity_id, opponent_id) {
            return false;
        }

        self.recompute_relative_fighting_ability(entity_id, assets);
        let principal = self
            .get_entity(entity_id)
            .and_then(Entity::human_data)
            .and_then(|human| human.opponents.first().copied());

        if let Some(principal_id) = principal {
            let principal_is_swordfighting = self
                .get_entity(principal_id)
                .and_then(Entity::human_data)
                .is_some_and(|human| !human.opponents.is_empty());
            if principal_is_swordfighting {
                self.take_smalltalk_initiative(entity_id);
            }
            return true;
        }

        let entity_is_dead = self.get_entity(entity_id).is_some_and(Entity::is_dead);
        if entity_is_dead {
            return true;
        }

        if matches!(self.world.entities.get(entity_id), Some(Entity::Pc(_))) {
            if let Some(pc) = self
                .world
                .entities
                .get_mut(entity_id)
                .and_then(Entity::pc_data_mut)
            {
                pc.melee_target = None;
            }
            self.enable_pc_actions_temp(assets, 0, entity_id);
        } else if matches!(self.world.entities.get(entity_id), Some(Entity::Soldier(_))) {
            self.dispatch_synchronous_ai_think_preserving_detection_fifo(
                sim,
                entity_id,
                assets,
                crate::ai::Stimulus::new(crate::ai::StimulusType::EventQuitSwordfight),
            );
        }

        true
    }

    /// Re-evaluate every entry in `entity_id`'s opponent list and refresh
    /// the per-opponent jump line after `entity_id`'s sector changed.
    ///
    /// For each opponent:
    /// - same sector as `entity_id` → clear the jump line on both
    ///   sides,
    /// - different sectors with a stale or missing jump line → ask
    ///   `is_table_swordfight_needed` from each side; if the two
    ///   answers pair up via `associated_line_index`, store each
    ///   side's own line; otherwise clear both.
    ///
    /// Only meaningful for swordfighters — callers gate on
    /// `is_swordfighting` before invoking.
    pub(crate) fn update_opponents_jump_lines(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        let this_sector_num = match self
            .get_entity(entity_id)
            .and_then(|e| e.element_data().sector())
        {
            Some(s) => i16::from(s),
            None => return,
        };
        let this_sector_idx = self
            .world
            .fast_grid
            .level
            .sector_number_map
            .get(&crate::sector::SectorNumber::new(this_sector_num))
            .copied();

        // Snapshot opponents + current jump-lines so we can mutate in a
        // second pass without holding a borrow on `self.world.entities`.
        let opponents: Vec<(EntityId, Option<crate::jump_line::JumpLineIndex>)> =
            match self.get_entity(entity_id).and_then(|e| e.human_data()) {
                Some(h) => h.opponents.iter_with_jump_lines().collect(),
                None => return,
            };

        // (slot_index, new_this_jl, opponent_id, new_opp_jl)
        let mut updates: Vec<(
            usize,
            Option<crate::jump_line::JumpLineIndex>,
            EntityId,
            Option<crate::jump_line::JumpLineIndex>,
        )> = Vec::new();

        for (i, (opp_id, current_jl)) in opponents.iter().enumerate() {
            let opp_sector_num = match self
                .get_entity(*opp_id)
                .and_then(|e| e.element_data().sector())
            {
                Some(s) => i16::from(s),
                None => continue,
            };

            if opp_sector_num == this_sector_num {
                // Same sector → clear if currently set.
                if current_jl.is_some() {
                    updates.push((i, None, *opp_id, None));
                }
                continue;
            }

            // Different sectors — check if the stored jump line is still
            // valid (this side's line lives in our sector and its
            // associated line lives in the opponent's sector).
            let opp_sector_idx = self
                .world
                .fast_grid
                .level
                .sector_number_map
                .get(&crate::sector::SectorNumber::new(opp_sector_num))
                .copied();

            let stale = match current_jl {
                None => true,
                Some(idx) => {
                    let jl = self.world.fast_grid.level.jump_lines.get(usize::from(*idx));
                    match jl {
                        None => true,
                        Some(jl_data) => {
                            let this_idx_match =
                                jl_data.sector_index.map(usize::from) == this_sector_idx;
                            let assoc_jl = jl_data.associated_line_index.and_then(|i| {
                                self.world.fast_grid.level.jump_lines.get(i as usize)
                            });
                            let assoc_idx_match =
                                assoc_jl.and_then(|aj| aj.sector_index).map(usize::from)
                                    == opp_sector_idx;
                            !this_idx_match || !assoc_idx_match
                        }
                    }
                }
            };

            if !stale {
                continue;
            }

            // Ask `is_table_swordfight_needed` from both sides; only
            // commit when each side's returned line is the other's
            // associated line.
            let new_this_idx = is_table_swordfight_needed(
                &self.world.entities,
                &self.world.fast_grid,
                &assets.profile_manager,
                entity_id,
                *opp_id,
            );
            let mut paired: Option<(
                crate::jump_line::JumpLineIndex,
                crate::jump_line::JumpLineIndex,
            )> = None;
            if let Some(this_raw) = new_this_idx {
                let new_opp_idx = is_table_swordfight_needed(
                    &self.world.entities,
                    &self.world.fast_grid,
                    &assets.profile_manager,
                    *opp_id,
                    entity_id,
                );
                if let Some(opp_raw) = new_opp_idx {
                    let opp_associated = self
                        .world
                        .fast_grid
                        .level
                        .jump_lines
                        .get(opp_raw as usize)
                        .and_then(|j| j.associated_line_index);
                    if opp_associated == Some(this_raw)
                        && let (Some(this_jl), Some(opp_jl)) = (
                            crate::jump_line::JumpLineIndex::new(this_raw),
                            crate::jump_line::JumpLineIndex::new(opp_raw),
                        )
                    {
                        paired = Some((this_jl, opp_jl));
                    }
                }
            }

            match paired {
                Some((this_jl, opp_jl)) => {
                    updates.push((i, Some(this_jl), *opp_id, Some(opp_jl)));
                }
                None => {
                    updates.push((i, None, *opp_id, None));
                }
            }
        }

        // Phase 2: write back.
        for (i, this_jl, opp_id, opp_jl) in updates {
            let owner_human = self
                .world
                .entities
                .get_mut(entity_id)
                .unwrap_or_else(|| {
                    panic!("opponent jump-line owner {entity_id:?} disappeared during refresh")
                })
                .human_data_mut()
                .unwrap_or_else(|| {
                    panic!("opponent jump-line owner {entity_id:?} stopped being human")
                });
            assert!(
                owner_human.opponents.update_jump_line_at(i, this_jl),
                "opponent slot {i} disappeared from {entity_id:?} during jump-line refresh"
            );

            // Mirror onto the opponent's slot for `entity_id`.
            // Original `UpdateOpponentJumpLine` asserts if the reciprocal
            // relationship is absent. The analysis phase above is read-only,
            // so a missing record here is malformed state, not a race to hide.
            let opponent_human = self
                .world
                .entities
                .get_mut(opp_id)
                .unwrap_or_else(|| {
                    panic!("opponent {opp_id:?} disappeared during jump-line refresh")
                })
                .human_data_mut()
                .unwrap_or_else(|| panic!("opponent {opp_id:?} is not human"));
            assert!(
                opponent_human.opponents.update_jump_line(entity_id, opp_jl),
                "opponent {opp_id:?} has no reciprocal record for {entity_id:?}"
            );
        }
    }

    /// Re-evaluate the opponent list after a change.
    ///
    /// - Empty list → quit the swordfight entirely.
    /// - Two or more opponents → re-pick the principal.
    /// - Exactly one → leave the principal where it is.
    pub(crate) fn evaluate_opponents(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        let count = self
            .get_entity(entity_id)
            .and_then(|e| e.human_data())
            .map(|h| h.opponents.len())
            .unwrap_or(0);

        if count == 0 {
            // Original `RHElementActorHuman::EvaluateOpponents` changes an
            // already-selected sword movement back to its ordinary upright
            // action before launching `QUIT_SWORDFIGHT`. The movement is then
            // postponed and translated again after the lowering transition,
            // retaining its destination and flags.
            if let Some((sequence_id, element_index)) = self
                .orders
                .sequence_manager
                .current_element_for_actor(entity_id)
            {
                self.rewrite_sword_movement_for_fight_exit(
                    sequence_id,
                    element_index,
                    entity_id,
                    true,
                );
            }

            // C++ `EvaluateOpponents` launches the explicit command;
            // relationship teardown alone does not own the visible
            // lowering-sword transition.
            self.launch_element(crate::sequence::SequenceElement::new(
                1,
                Command::QuitSwordfight,
                Some(entity_id),
            ));

            // The quit is announced to the owner immediately, in the same
            // call: an orphaned PC regains its temporarily disabled actions,
            // and a soldier's brain receives EVENT_QUIT_SWORDFIGHT before any
            // later phase of this frame — in particular before its own
            // RefreshDetection can emit VIEW / OUTOFVIEW. Deferring the
            // stimulus to the `Command::QuitSwordfight` dispatcher left the
            // soldier in its pre-quit substate while the falling-edge
            // OUTOFVIEW arrived, which changed how that event was routed.
            if matches!(self.world.entities.get(entity_id), Some(Entity::Pc(_))) {
                self.enable_pc_actions_temp(assets, 0, entity_id);
            } else if matches!(self.world.entities.get(entity_id), Some(Entity::Soldier(_))) {
                self.dispatch_synchronous_ai_think_preserving_detection_fifo(
                    sim,
                    entity_id,
                    assets,
                    crate::ai::Stimulus::new(crate::ai::StimulusType::EventQuitSwordfight),
                );
            }
        } else if count >= 2 {
            self.choose_principal_opponent(sim, entity_id);
        }
    }

    /// Change a movement which will survive a swordfight exit back to its
    /// ordinary upright form. Original asserts in debug builds that
    /// `EvaluateOpponents` sees a sword movement, but its shipped release
    /// expression maps `WalkingWithSword` to `WalkingUpright` and every other
    /// action to `RunningUpright`. An explicit quit can postpone any movement,
    /// so its cross-sequence successor uses the non-strict form.
    pub(super) fn rewrite_sword_movement_for_fight_exit(
        &mut self,
        sequence_id: crate::sequence::SequenceId,
        element_index: usize,
        owner: EntityId,
        strict: bool,
    ) {
        let element = self
            .orders
            .sequence_manager
            .get_element_mut(sequence_id, element_index)
            .unwrap_or_else(|| {
                panic!(
                    "fight exit: movement ({sequence_id:?}, {element_index}) \
                     for {owner:?} is stale"
                )
            });
        let SequenceElementData::Movement { action, .. } = &mut element.data else {
            return;
        };
        *action = match *action {
            OrderType::WalkingWithSword => OrderType::WalkingUpright,
            OrderType::RunningWithSword => OrderType::RunningUpright,
            other if !strict => other,
            // TODO: Legacy saves can resume a non-sword movement while the
            // actor still owns an opponent. Preserve the shipped release
            // behavior here even though Original's debug assertion rejects
            // that state.
            _ => OrderType::RunningUpright,
        };
    }

    /// Pick a new principal opponent from the entity's opponent list.
    ///
    /// 1. Build a candidate list of opponents within ±2 sectors of
    ///    the entity's facing direction.
    /// 2. If any face-cone candidates exist, pick one uniformly at
    ///    random.
    /// 3. Otherwise pick the nearest opponent by 2D distance.
    ///
    /// When the chosen opponent isn't already at index 0, swap them to
    /// the front and take the smalltalk initiative.
    pub(super) fn choose_principal_opponent(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        entity_id: EntityId,
    ) {
        let debug = opponent_order_debug_matches(self.control.frame_counter, entity_id);
        let (self_pos, self_dir, opponents) = {
            let Some(entity) = self.get_entity(entity_id) else {
                return;
            };
            let Some(human) = entity.human_data() else {
                return;
            };
            if human.opponents.len() < 2 {
                return;
            }
            let elem = entity.element_data();
            (elem.position_map(), elem.direction(), human.opponents.ids())
        };

        // Face-cone candidates: relative sector within ±2 of 0.
        let mut candidates: Vec<usize> = Vec::new();
        for (idx, opp_id) in opponents.iter().enumerate() {
            let pos_opp = match self
                .get_entity(*opp_id)
                .map(|e| e.element_data().position_map())
            {
                Some(p) => p,
                None => continue,
            };
            // Use the world aspect ratio (not the sword-fight one)
            // for the opponent-to-self angle in the principal-opponent
            // face cone.
            let dir_to = crate::position_interface::vector_to_sector_0_to_15_iso(
                pos_opp.x - self_pos.x,
                pos_opp.y - self_pos.y,
            );
            let relative = ((self_dir - dir_to) & 15) as u16;
            if relative <= 2 || relative >= 14 {
                candidates.push(idx);
            }
        }

        let new_principal = if !candidates.is_empty() {
            let rng_before = debug.then(|| self.control.rng.original_replay_cursor());
            let pick = crate::sim_rng::usize(
                sim,
                crate::sim_rng::RngSite::PrincipalOpponent,
                0..candidates.len(),
            );
            if debug {
                eprintln!(
                    "PARITY_OPPONENT_ORDER frame={} phase=choose owner={} before={opponents:?} candidates={candidates:?} pick={} rng_before={:?} rng_after={:?}",
                    self.control.frame_counter,
                    entity_id.index(),
                    candidates[pick],
                    rng_before.flatten(),
                    self.control.rng.original_replay_cursor(),
                );
            }
            candidates[pick]
        } else {
            // Nearest-opponent fallback.
            let mut best = 0usize;
            let mut best_dist = f32::MAX;
            for (idx, opp_id) in opponents.iter().enumerate() {
                let dist = entity_distance(&self.world.entities, entity_id, *opp_id);
                if dist < best_dist {
                    best_dist = dist;
                    best = idx;
                }
            }
            best
        };

        if new_principal != 0 {
            if let Some(entity) = self.world.entities.get_mut(entity_id)
                && let Some(human) = entity.human_data_mut()
            {
                assert!(human.opponents.promote(new_principal));
            }
            self.take_smalltalk_initiative(entity_id);
        }
        if debug {
            let after = self
                .world
                .entities
                .get(entity_id)
                .and_then(Entity::human_data)
                .map(|human| human.opponents.ids());
            eprintln!(
                "PARITY_OPPONENT_ORDER frame={} phase=choose_done owner={} chosen_index={} after={after:?}",
                self.control.frame_counter,
                entity_id.index(),
                new_principal,
            );
        }
    }

    /// Promote `new_opponent` to principal opponent (front of list).
    ///
    /// If the opponent is already in the list, swap it to index 0.
    /// If not found, request an enter-swordfight with the new target.
    ///
    pub(crate) fn set_as_new_principal_opponent(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        new_opponent_id: EntityId,
    ) {
        let found = {
            let Some(entity) = self.world.entities.get(entity_id) else {
                return;
            };
            let Some(human) = entity.human_data() else {
                return;
            };
            human.opponents.iter().position(|&id| id == new_opponent_id)
        };

        if let Some(idx) = found {
            // Swap to front — makes this opponent the principal.
            if let Some(entity) = self.world.entities.get_mut(entity_id)
                && let Some(human) = entity.human_data_mut()
            {
                assert!(human.opponents.promote(idx));
            }
            self.take_smalltalk_initiative(entity_id);
        } else {
            // Gate on `can_enter_swordfight_with` and launch a
            // PostponeEverythingButInjuries `EnterSwordfight` element
            // rather than calling `enter_swordfight` directly.  This
            // lets the priority arbitration postpone/interrupt the
            // pending enter against any concurrent injury-priority
            // work, and defers the distance/LOS/sword-hurt guards to
            // the EnterSwordfight dispatcher.
            if can_enter_swordfight_with(
                &self.world.entities,
                entity_id,
                new_opponent_id,
                &assets.profile_manager,
                &self.world.fast_grid,
            ) {
                let mut elem = crate::sequence::SequenceElement::new_generic(
                    1,
                    Command::EnterSwordfight,
                    Some(entity_id),
                );
                elem.set_property(
                    crate::sequence::Field::Opponent,
                    crate::sequence::FieldValue::Element(new_opponent_id),
                );
                elem.set_property(
                    crate::sequence::Field::JumplineDestination,
                    crate::sequence::FieldValue::Integer(0),
                );
                self.launch_element(elem);
            }
        }
    }

    /// Enter swordfight between two entities.
    ///
    /// `initiator` is the entity requesting entry; `opponent` is the
    /// target.  `sword_hurted` is true when the initiator enters
    /// because they were hit — applies the single-opponent
    /// restriction.
    ///
    /// Returns true if swordfight was entered.
    #[cfg(test)]
    pub(crate) fn enter_swordfight(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        initiator: EntityId,
        opponent: EntityId,
        sword_hurted: bool,
    ) -> bool {
        self.enter_swordfight_with_jump_line(sim, assets, initiator, opponent, sword_hurted, None)
    }

    /// Variant of [`enter_swordfight`] that threads the
    /// table-swordfight jump line through to `add_opponent`: the
    /// aggressor gets `aggressor_jump_line` (their side of the
    /// table), and the opponent gets the associated paired line on
    /// the far side.
    pub(crate) fn enter_swordfight_with_jump_line(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        initiator: EntityId,
        opponent: EntityId,
        sword_hurted: bool,
        aggressor_jump_line: Option<crate::jump_line::JumpLineIndex>,
    ) -> bool {
        self.enter_swordfight_impl(
            sim,
            assets,
            initiator,
            opponent,
            sword_hurted,
            aggressor_jump_line,
            true,
        )
    }

    /// Direct `RHElementActorHuman::EnterSwordFight` entry used by both
    /// `ReconsiderSwordfight` and the already-swordfighting `EVENT_GOTHIT`
    /// arm. Unlike the ENTER_SWORDFIGHT command's `Translate` path, the
    /// direct Original call does not first invoke
    /// `pOpponent->PrepareToEnterSwordFight(this)`.
    pub(crate) fn direct_enter_swordfight(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        initiator: EntityId,
        opponent: EntityId,
    ) -> bool {
        self.enter_swordfight_impl(sim, assets, initiator, opponent, false, None, false)
    }

    fn enter_swordfight_impl(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        initiator: EntityId,
        opponent: EntityId,
        sword_hurted: bool,
        aggressor_jump_line: Option<crate::jump_line::JumpLineIndex>,
        prepare_opponent: bool,
    ) -> bool {
        let scratch = self.build_sim_scratch(sim, assets);
        // PC initiators clear shield-protection before entering the
        // fight to unlink any active shield-protection.  NPC
        // sword-fights don't carry the protection link.
        let initiator_is_pc = self
            .expect_entity(initiator, "enter_swordfight initiator")
            .is_pc();
        if initiator_is_pc {
            self.set_shield_protected(initiator, None);
        }

        // C++ `EnterSwordFight` calls `ClearShootList()` before the
        // validity gates.
        self.clear_pc_shoot_list(initiator);
        if initiator_is_pc {
            // Repeated PC bow clicks have not necessarily reached the
            // retained pointer FIFO yet, so remove their eager Rust manager
            // registrations too. NPC ShootBow elements are different: they
            // remain linked through the sequence's postponed pointer even
            // after Original clears `mlpsequenceShootList`. Interrupting one
            // here invents a condolation/EventDone and severs the link.
            let resolver = Self::priority_resolver(&self.world.entities);
            self.orders.sequence_manager.stop_pending_elements_matching(
                initiator,
                crate::element::Command::ShootBow,
                crate::sequence::SequencePriority::Preference,
                &resolver,
            );
        }

        // Cancel any pending AI bow shot.
        if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(initiator)
            && let Some(ai) = s.npc.ai_brain.base_mut()
        {
            ai.outbox.actor.shoot_target = None;
        }

        // ENTER_SWORDFIGHT's Translate calls PrepareToEnterSwordFight before
        // calling EnterSwordFight. ReconsiderSwordfight calls EnterSwordFight
        // directly, so it must bypass this opponent Stop/Think boundary.
        let should_prepare_opponent = prepare_opponent
            && !self
                .expect_entity(opponent, "enter_swordfight opponent")
                .human_data()
                .map(|h| !h.opponents.is_empty())
                .unwrap_or(false);
        if should_prepare_opponent {
            // EVENT_ENTER_SWORDFIGHT can synchronously re-enter admission for
            // this pair before either opponent list has been published.
            // Original suppresses only the nested Prepare body; the nested
            // EnterSwordFight call continues and publishes the relationship.
            let _ = with_swordfight_preparation_scope(opponent, initiator, || {
                self.stop_owner_current(opponent, crate::sequence::SequencePriority::Preference);
                // Original `PrepareToEnterSwordFight` calls `Stop(PREFERENCE)`
                // before `Think(EVENT_ENTER_SWORDFIGHT)`. `Stop` reaches
                // `SetState(INTERRUPTED)`, whose `SendCondolationCard` callback
                // is synchronous: the interrupted command's EventDone/
                // EventImpossible reaction must therefore finish while the NPC
                // is still in its old AI substate. Leaving the card queued until
                // after EventEnterSwordfight lets that old completion run as a
                // swordfight completion and can immediately quit the new fight.
                self.dispatch_condolations_for_owner_boundary(sim, opponent, assets);
                // Actor::Stop resumes after that synchronous card and only now
                // calls StopNotYetLaunchedSequenceElements. The old completion
                // can have queued fresh overview work (LookLeft in the retained
                // linux3 control), which must be included in this trailing scan.
                // The finish phase snapshots the queue before its scan, matching
                // Original's fixed `uwNumberOfSeqElements`, but closes each
                // stopped root's card before advancing to the next captured root.
                self.stop_owner_pending_after_callback(
                    sim,
                    assets,
                    opponent,
                    crate::sequence::SequencePriority::Preference,
                );
                // Synchronous Think on the opponent if they're a soldier.
                let is_soldier = matches!(
                    self.expect_entity(opponent, "enter_swordfight opponent"),
                    Entity::Soldier(_)
                );
                if is_soldier {
                    let ctx = {
                        let entity = self
                            .world
                            .entities
                            .get(opponent)
                            .expect("opponent existence checked above");
                        crate::engine::ai::build_ai_context_from_entity(
                            entity,
                            self.control.frame_counter,
                            None,
                            self.world.weather.is_forest_level,
                            self.world.weather.ambiance,
                            self.ai.standard_view_polygon_radius,
                            &scratch.ai_entity_views,
                            &scratch.ai_sight_obstacles,
                            &self.world.fast_grid,
                            &assets.hiking_paths,
                            &assets.hiking_waypoint_sectors,
                            &self.ai.global.all_soldier_handles,
                            self.control.sim_config.difficulty,
                        )
                    };
                    let stimulus = crate::ai::Stimulus::with_human(
                        crate::ai::StimulusType::EventEnterSwordfight,
                        initiator.index(),
                    );
                    let tick_data = self.build_npc_tick_data_for_target(
                        sim,
                        opponent,
                        &scratch,
                        assets,
                        Some(initiator),
                    );
                    self.dispatch_think_with_drain(
                        sim, opponent, &stimulus, &ctx, &tick_data, assets,
                    );
                }
            });
        }

        if !can_enter_swordfight_with(
            &self.world.entities,
            initiator,
            opponent,
            &assets.profile_manager,
            &self.world.fast_grid,
        ) {
            tracing::warn!(
                ?initiator,
                ?opponent,
                "enter_swordfight: rejected by can_enter_swordfight_with"
            );
            return false;
        }

        let already_opponent = self
            .expect_entity(initiator, "enter_swordfight initiator")
            .human_data()
            .map(|h| h.opponents.contains(&opponent))
            .unwrap_or(false);

        if !already_opponent {
            // Cross-sector elevation gate: reject if elevation
            // difference exceeds threshold AND entities are in
            // different sectors.  Lives here (not in
            // `can_enter_swordfight_with`) so already-paired fighters
            // can re-enter swordfight after one drifts onto a
            // different-sector elevation.
            {
                let (elev_a, elev_b, sector_a, sector_b) = {
                    let entity_a = self.world.entities.get(initiator);
                    let entity_b = self.world.entities.get(opponent);
                    let (Some(ea), Some(eb)) = (entity_a, entity_b) else {
                        return false;
                    };
                    (
                        ea.position_iface().get_elevation(),
                        eb.position_iface().get_elevation(),
                        ea.element_data().sector(),
                        eb.element_data().sector(),
                    )
                };
                if (elev_a - elev_b).abs() > MAX_ELEVATION_SWORDFIGHT && sector_a != sector_b {
                    tracing::debug!(
                        ?initiator,
                        ?opponent,
                        elev_a,
                        elev_b,
                        "enter_swordfight: cross-sector elevation diff too large"
                    );
                    return false;
                }
            }

            // Distance check: 3D distance must be within both
            // combatants' UBER sword range.
            {
                // Original `EnterSwordFight` compares
                // `(GetPosition() - pOpponent->GetPosition()).Norm()`.  Use
                // the stored three-dimensional world positions here: map
                // projection can make fighters on different elevations look
                // farther apart and reject a valid engagement before LOS.
                let dist = entity_world_distance(&self.world.entities, initiator, opponent);
                let uber_a = self
                    .world
                    .entities
                    .get(initiator)
                    .and_then(|e| get_hth_weapon_id_full(e, &assets.profile_manager))
                    .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                    .map(|p| p.distance[3] as f32)
                    .unwrap_or(70.0);
                let uber_b = self
                    .world
                    .entities
                    .get(opponent)
                    .and_then(|e| get_hth_weapon_id_full(e, &assets.profile_manager))
                    .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                    .map(|p| p.distance[3] as f32)
                    .unwrap_or(70.0);
                if dist > uber_a || dist > uber_b {
                    tracing::debug!(
                        ?initiator,
                        ?opponent,
                        dist,
                        uber_a,
                        uber_b,
                        "enter_swordfight: too far apart"
                    );
                    return false;
                }
            }

            // LOS check: verify line of sight between combatants
            // between upright eye points, matching C++
            // `ComputeEyesPoint(..., RHPOSTURE_UPRIGHT)`.
            {
                let eye_a = self
                    .get_entity(initiator)
                    .and_then(|e| e.compute_eyes_point(Some(Posture::Upright)));
                let eye_b = self
                    .get_entity(opponent)
                    .and_then(|e| e.compute_eyes_point(Some(Posture::Upright)));
                if let (Some(a), Some(b)) = (eye_a, eye_b)
                    && !crate::sight_obstacle::is_reachable_3d(
                        self.sight_obstacles(assets),
                        [a.x, a.y, a.z],
                        [b.x, b.y, b.z],
                        crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
                    )
                {
                    tracing::debug!(
                        ?initiator,
                        ?opponent,
                        "enter_swordfight: LOS blocked (3D opaque)"
                    );
                    return false;
                }
            }

            // Single-opponent restriction when hurt.
            // Don't enter if the initiator already has opponents, or if
            // the opponent's principal opponent already has multiple opponents.
            if sword_hurted {
                let initiator_opp_count = self
                    .world
                    .entities
                    .get(initiator)
                    .and_then(|e| e.human_data())
                    .map(|h| h.opponents.len())
                    .unwrap_or(0);
                if initiator_opp_count >= 1 {
                    return false;
                }

                // If the opponent is already fighting and their principal
                // opponent has >1 opponents, don't pile in.
                let principal_opp_id = self
                    .world
                    .entities
                    .get(opponent)
                    .and_then(|e| e.human_data())
                    .and_then(|h| h.opponents.first().copied());

                if let Some(principal_id) = principal_opp_id {
                    let principal_opp_count = self
                        .world
                        .entities
                        .get(principal_id)
                        .and_then(|e| e.human_data())
                        .map(|h| h.opponents.len())
                        .unwrap_or(0);
                    if principal_opp_count > 1 {
                        return false;
                    }
                }
            }

            // Don't enter swordfight with a charging knight.
            let opponent_is_charging_rider = {
                let e = self.expect_entity(opponent, "enter_swordfight opponent");
                e.soldier_data().map(|s| s.rider).unwrap_or(false)
                    && e.actor_data()
                        .map(|a| a.action_state == ActionState::MovingFast)
                        .unwrap_or(false)
            };
            if opponent_is_charging_rider {
                return false;
            }
        }

        // Clear step-back flag on swordfight entry.
        if let Some(entity) = self.world.entities.get_mut(initiator)
            && let Some(hd) = entity.human_data_mut()
        {
            hd.last_motion_was_step_back_in_combat = false;
        }

        // Multi-opponent purging.
        let opponent_is_swordfighting = self
            .expect_entity(opponent, "enter_swordfight opponent")
            .human_data()
            .map(|h| !h.opponents.is_empty())
            .unwrap_or(false);

        if !opponent_is_swordfighting {
            // Launch an EnterSwordfight sequence element on the
            // opponent so they raise their sword.  (The stop +
            // soldier think from `prepare_to_enter_swordfight`
            // already ran at the top of this function.)
            //
            // `RHElementActorHuman::EnterSwordFight`
            // (original-code/RHelementactorhuman.cpp:7674-7681) stores
            // `pJumpLine->GetAssociatedJumpLine()` — the far side of the
            // table — in the reciprocal element's
            // RHFIELD_JUMPLINE_DESTINATION, not a null line.  That property
            // is what the opponent's own ENTER_SWORDFIGHT translation reads
            // to run the table-swordfight slot search and launch its
            // approach movement, so dropping it silently skipped the whole
            // cross-sector positioning half of the fight.
            let reciprocal_jump_line = associated_jump_line(self, aggressor_jump_line);
            let mut seq = crate::sequence::Sequence::new();
            let mut elem = crate::sequence::SequenceElement::new_generic(
                1,
                Command::EnterSwordfight,
                Some(opponent),
            );
            elem.set_property(
                crate::sequence::Field::Opponent,
                crate::sequence::FieldValue::Element(initiator),
            );
            elem.set_property(
                crate::sequence::Field::JumplineDestination,
                match reciprocal_jump_line {
                    Some(line) => crate::sequence::FieldValue::LineId(line),
                    None => crate::sequence::FieldValue::Integer(0),
                },
            );
            seq.append_element(elem);
            self.launch_sequence(seq);
        } else if !already_opponent {
            // Part 1: walk the opponent's existing opponent list.
            // If any of their opponents have >1 opponents themselves,
            // break those fights to make room for the new 1-on-1.
            let mut ally_index = 0;
            loop {
                let Some(ally_id) = self
                    .world
                    .entities
                    .get(opponent)
                    .and_then(Entity::human_data)
                    .and_then(|human| human.opponents.get(ally_index).copied())
                else {
                    break;
                };
                let ally_opp_count = self
                    .world
                    .entities
                    .get(ally_id)
                    .and_then(|e| e.human_data())
                    .map(|h| h.opponents.len())
                    .unwrap_or(0);
                if ally_opp_count > 1 {
                    self.delete_opponent(sim, assets, ally_id, opponent);
                    self.delete_opponent(sim, assets, opponent, ally_id);
                }
                // Original indexes the live list while DeleteOpponent may
                // shrink it, so a removed slot advances past the element
                // shifted into its place.
                ally_index += 1;
            }

            // Part 2: if both sides still have opponents, purge all
            // opponents from the royalist side.
            let initiator_has_opps = self
                .expect_entity(initiator, "enter_swordfight initiator")
                .human_data()
                .map(|h| !h.opponents.is_empty())
                .unwrap_or(false);
            let opponent_has_opps = self
                .expect_entity(opponent, "enter_swordfight opponent")
                .human_data()
                .map(|h| !h.opponents.is_empty())
                .unwrap_or(false);

            if initiator_has_opps && opponent_has_opps {
                let initiator_camp = entity_camp(&self.world.entities, initiator);
                let human_to_purge = if initiator_camp == crate::element::Camp::Royalists {
                    initiator
                } else {
                    opponent
                };

                let mut purge_index = 0;
                loop {
                    let Some(opp_id) = self
                        .world
                        .entities
                        .get(human_to_purge)
                        .and_then(Entity::human_data)
                        .and_then(|human| human.opponents.get(purge_index).copied())
                    else {
                        break;
                    };
                    self.delete_opponent(sim, assets, opp_id, human_to_purge);
                    self.delete_opponent(sim, assets, human_to_purge, opp_id);
                    // Match the mutable C++ list walk rather than draining a
                    // snapshot: deletion shifts the next entry left while the
                    // loop counter still advances.
                    purge_index += 1;
                }
            }
        }

        // Add opponents.
        tracing::info!(
            ?initiator,
            ?opponent,
            "enter_swordfight: SUCCESS — adding opponents"
        );
        // The aggressor stores `aggressor_jump_line` (their side of
        // the table), the opponent stores the associated paired line
        // on the far side.  When no table fight is involved, both
        // sides store `None`.
        let opponent_jump_line = associated_jump_line(self, aggressor_jump_line);
        let debug_opponent = opponent_order_debug_matches(self.control.frame_counter, opponent);
        let debug_initiator = opponent_order_debug_matches(self.control.frame_counter, initiator);
        let opponent_before = debug_opponent.then(|| {
            self.world
                .entities
                .get(opponent)
                .and_then(Entity::human_data)
                .map(|human| human.opponents.ids())
        });
        let initiator_before = debug_initiator.then(|| {
            self.world
                .entities
                .get(initiator)
                .and_then(Entity::human_data)
                .map(|human| human.opponents.ids())
        });
        let opponent_added = Self::add_opponent(
            &mut self.world.entities,
            opponent,
            initiator,
            opponent_jump_line,
        );
        if debug_opponent {
            let after = self
                .world
                .entities
                .get(opponent)
                .and_then(Entity::human_data)
                .map(|human| human.opponents.ids());
            eprintln!(
                "PARITY_OPPONENT_ORDER frame={} phase=enter_opponent owner={} other={} before={:?} after={after:?} fresh={} rng={:?}",
                self.control.frame_counter,
                opponent.index(),
                initiator.index(),
                opponent_before.flatten(),
                opponent_added,
                self.control.rng.original_replay_cursor(),
            );
        }
        // Original `AddOpponent` owns these side effects and performs them
        // immediately after each *fresh* insertion. In particular, merely
        // re-entering an already-established fight must not reset initiative.
        // Preserve the call order because the first initiative check happens
        // before the reciprocal list entry is installed.
        if opponent_added {
            self.recompute_relative_fighting_ability(opponent, assets);
            self.take_smalltalk_initiative(opponent);
        }
        let initiator_added = Self::add_opponent(
            &mut self.world.entities,
            initiator,
            opponent,
            aggressor_jump_line,
        );
        if debug_initiator {
            let after = self
                .world
                .entities
                .get(initiator)
                .and_then(Entity::human_data)
                .map(|human| human.opponents.ids());
            eprintln!(
                "PARITY_OPPONENT_ORDER frame={} phase=enter_initiator owner={} other={} before={:?} after={after:?} fresh={} rng={:?}",
                self.control.frame_counter,
                initiator.index(),
                opponent.index(),
                initiator_before.flatten(),
                initiator_added,
                self.control.rng.original_replay_cursor(),
            );
        }
        if initiator_added {
            self.recompute_relative_fighting_ability(initiator, assets);
            self.take_smalltalk_initiative(initiator);
        }

        // Cancel pending pathfinder requests and active paths only
        // for entities that are ENTERING combat fresh (not already in
        // a sword or shield state).  This happens via the
        // `prepare_to_enter_swordfight` step, which is only called
        // when the entity wasn't already swordfighting.  Clearing
        // movement for an already-fighting entity would cancel their
        // in-progress walk-away / strafe during combat.
        let initiator_fresh = self
            .expect_entity(initiator, "enter_swordfight initiator")
            .actor_data()
            .map(|a| !a.action_state.is_sword() && !a.action_state.is_shield())
            .unwrap_or(true);
        let opponent_fresh = self
            .expect_entity(opponent, "enter_swordfight opponent")
            .actor_data()
            .map(|a| !a.action_state.is_sword() && !a.action_state.is_shield())
            .unwrap_or(true);
        // Whenever a movement element is torn down, the failed-path
        // retries also get cleaned out — otherwise stale 100-frame
        // retry entries fire `element_impossible` / hero-speech
        // after the swordfight starts.  The cancel-requests half is
        // a no-op post-pathfinder refactor (sequence-element
        // interruption tears down in-flight requests), so we only
        // retain the failed-path cleanup.
        if initiator_fresh {
            self.orders
                .failed_path_requests
                .retain(|r| r.owner != initiator);
        }
        if opponent_fresh {
            self.orders
                .failed_path_requests
                .retain(|r| r.owner != opponent);
        }

        // `EnterSwordFight` mutates the relationship and initiator PC UI.
        // The ENTER_SWORDFIGHT element that called us owns its pose
        // transition; the reciprocal element does the same for the opponent.
        // Original first forwards targeted `MSG_SELECT_ACTION(NO_ACTION)`.
        // `RHEngine::SelectAction` clears the target PC's current action even
        // when that PC is not selected; only the selected branch performs the
        // wider UI/action cleanup. `DisableAllActionsTemp` then saves the
        // cleared action. Preserve that ordering so quitting the fight cannot
        // resurrect an action that was armed before entry.
        if self
            .world
            .entities
            .get(initiator)
            .is_some_and(Entity::is_pc)
        {
            self.set_pc_action_from_message(
                assets,
                0,
                initiator,
                crate::profiles::Action::NoAction,
            );
        }
        if let Some(pc) = self
            .world
            .entities
            .get_mut(initiator)
            .and_then(Entity::pc_data_mut)
        {
            if pc.melee_target.is_none() {
                pc.melee_target = Some(opponent);
            }
            pc.disable_all_actions_temp();
        }
        // Set PC melee target.
        Self::set_pc_melee_target(&mut self.world.entities, initiator, opponent);
        Self::set_pc_melee_target(&mut self.world.entities, opponent, initiator);
        // The opponent's `prepare_to_enter_swordfight` think fires
        // at the top of this function; no second dispatch needed
        // here.

        true
    }

    /// Remove this entity from every swordfight relationship.
    ///
    /// This mirrors C++ `RHElementActorHuman::QuitSwordFight`: despite
    /// its name, the method only unlinks opponents and performs PC/AI
    /// bookkeeping. It does not mutate a survivor's action state or
    /// launch the visible lowering-sword transition; the explicit
    /// `Command::QuitSwordfight` dispatcher owns that.
    pub(crate) fn quit_swordfight(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        // Original walks a fixed count from the quitter's live list while
        // every reciprocal `DeleteOpponent` mutates only the other actor.
        // Snapshotting that list gives the same ownership without holding a
        // borrow across the synchronous callbacks.
        let opponents: Vec<EntityId> = self
            .world
            .entities
            .get(entity_id)
            .unwrap_or_else(|| panic!("QuitSwordFight owner {entity_id:?} is missing"))
            .human_data()
            .unwrap_or_else(|| panic!("QuitSwordFight owner {entity_id:?} is not human"))
            .opponents
            .ids();

        // Route every reciprocal unlink through the authoritative
        // DeleteOpponent translation. It owns strength recomputation, the
        // logical opponent-list initiative reset, and final-opponent PC/AI
        // callbacks.
        for opp_id in &opponents {
            assert!(
                self.delete_opponent(sim, assets, *opp_id, entity_id),
                "QuitSwordFight owner {entity_id:?} was absent from reciprocal opponent {opp_id:?}"
            );
        }

        // The post-loop self-cleanup is gated on `!is_dead()`: a
        // dead PC shouldn't have its `disabled_actions_temp`
        // re-enabled, and a dead soldier shouldn't be re-pumped
        // through `think`.
        let entity_is_dead = self
            .expect_entity(entity_id, "quit_swordfight quitter")
            .is_dead();

        // Always clear the entity's own opponent list. Relationship
        // cleanup deliberately leaves its current action and sequence
        // untouched.
        let mut enable_self_actions = false;
        {
            let entity = self.expect_entity_mut(entity_id, "quit_swordfight quitter");
            if let Some(human) = entity.human_data_mut() {
                human.opponents.clear();
            }
            if !entity_is_dead && let Some(pc) = entity.pc_data_mut() {
                pc.melee_target = None;
                enable_self_actions = true;
            }
        }
        if enable_self_actions {
            self.enable_pc_actions_temp(assets, 0, entity_id);
        }

        // When a non-dead soldier voluntarily quits a swordfight,
        // immediately pump EventQuitSwordfight into its own AI so it
        // can re-plan, rather than waiting for the next AI tick.
        if !entity_is_dead
            && matches!(
                self.expect_entity(entity_id, "quit_swordfight quitter"),
                Entity::Soldier(_)
            )
        {
            self.dispatch_synchronous_ai_think_preserving_detection_fifo(
                sim,
                entity_id,
                assets,
                crate::ai::Stimulus::new(crate::ai::StimulusType::EventQuitSwordfight),
            );
        }
    }

    /// Set the melee target on a PC entity.
    pub(super) fn set_pc_melee_target(
        entities: &mut crate::entities::Entities,
        pc_id: EntityId,
        opponent_id: EntityId,
    ) {
        if let Some(entity) = entities.get_mut(pc_id)
            && let Some(pc) = entity.pc_data_mut()
        {
            pc.melee_target = Some(opponent_id);
        }
    }

    /// Remove opponents that are too far away, re-evaluate swordfight state.
    ///
    /// Called from the AI tick when soldiers re-evaluate their combat state.
    pub(crate) fn quit_swordfight_with_far_opponents(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        let (opponents, uber_range) = {
            let entity = match self.world.entities.get(entity_id) {
                Some(e) => e,
                None => return,
            };
            let range = get_hth_weapon_id_full(entity, &assets.profile_manager)
                .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                .map(|p| p.distance[3] as f32) // UBER range
                .unwrap_or(70.0);
            let opps = entity
                .human_data()
                .map(|h| h.opponents.ids())
                .unwrap_or_default();
            (opps, range)
        };

        let mut removed: Vec<EntityId> = Vec::new();
        for opp_id in opponents {
            let dist = entity_distance(&self.world.entities, entity_id, opp_id);
            let opp_uber = self
                .world
                .entities
                .get(opp_id)
                .and_then(|e| get_hth_weapon_id_full(e, &assets.profile_manager))
                .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                .map(|p| p.distance[3] as f32)
                .unwrap_or(70.0);

            if dist > uber_range && dist > opp_uber {
                // Original `QuitSwordfightWithFarOpponents` deliberately
                // edits the owner's list directly, so its cached relative
                // fighting ability is retained.  The reciprocal removal does
                // go through `DeleteOpponent`, including its strength,
                // initiative, and final-opponent quit side effects.
                assert!(Self::remove_opponent(
                    &mut self.world.entities,
                    entity_id,
                    opp_id
                ));
                assert!(self.delete_opponent(sim, assets, opp_id, entity_id));
                removed.push(opp_id);
            }
        }
        if removed.is_empty() {
            return;
        }

        // After removing the far opponent from both lists,
        // `evaluate_opponents` runs on the opponent AND on self —
        // dispatches to `quit_swordfight` when the list is empty or
        // `choose_principal_opponent` when two or more remain.
        let remaining = self
            .world
            .entities
            .get(entity_id)
            .and_then(|e| e.human_data())
            .map(|h| h.opponents.len())
            .unwrap_or(0);
        tracing::debug!(
            "quit_swordfight_with_far_opponents: {:?} removed {} far opponents, {} remaining",
            entity_id,
            removed.len(),
            remaining
        );
        for opp_id in &removed {
            self.evaluate_opponents(sim, assets, *opp_id);
        }
        self.evaluate_opponents(sim, assets, entity_id);
    }

    // ─── Experience points ──────────────────────────────────────────

    /// Award sword kill experience to the attacker.
    pub(super) fn award_sword_kill_xp(
        &mut self,
        assets: &LevelAssets,
        attacker_id: EntityId,
        victim_id: EntityId,
    ) {
        // Only PCs can receive XP (they have HumanStatus via Campaign)
        let attacker_is_pc = self
            .expect_entity(attacker_id, "sword-kill XP attacker")
            .kind()
            .is_pc();
        if !attacker_is_pc {
            return;
        }

        let mut xp = combat::SWORD_KILL_EXPERIENCE_POINTS;

        // Bonus if victim was more skilled than attacker.
        let victim_capacity: u32 = self
            .get_entity(victim_id)
            .map(|e| {
                fighting_ability_from_profile(
                    e,
                    &assets.profile_manager,
                    self.control.sim_config.difficulty,
                ) as u32
            })
            .unwrap_or(0);
        let attacker_capacity: u32 = self
            .get_entity(attacker_id)
            .map(|e| {
                fighting_ability_from_profile(
                    e,
                    &assets.profile_manager,
                    self.control.sim_config.difficulty,
                ) as u32
            })
            .unwrap_or(0);

        if victim_capacity > attacker_capacity {
            xp += victim_capacity - attacker_capacity;
        }

        // Apply XP through campaign
        let profile_idx = self
            .get_entity(attacker_id)
            .and_then(|e| match e {
                Entity::Pc(pc) => Some(pc.pc.profile_index),
                _ => None,
            })
            .unwrap_or_default();
        if let Some(campaign) = Some(&mut self.mission_domain.campaign) {
            // The PC experience-add awards a campaign-score bonus
            // whenever the call crosses a 100-XP boundary.
            campaign.add_pc_experience(
                usize::from(profile_idx),
                crate::pc_status::SkillName::HandToHand,
                xp,
            );
            tracing::debug!(
                attacker = ?attacker_id,
                xp,
                "Awarded sword kill XP"
            );
        }
    }

    // ─── PC coma / amulet death-save ────────────────────────────────

    /// Close the PC wounded/coma virtual boundary at a damage-apply site.
    ///
    /// The wounding entry points dispatch `GetWounded` virtually, so a VIP
    /// PC establishes its amulet coma (5-HP floor, maximum concussion)
    /// *inside* the damage call — before the damage element is translated
    /// and before the shared `SayOuch` classifies the result. Rust's shared
    /// damage primitives cannot mutate campaign state, so every wounding
    /// site closes that boundary here, immediately after writing the life
    /// points.
    ///
    /// Returns `true` when the coma save activated, in which case the
    /// caller's downstream death handling must be skipped.
    pub(in crate::engine) fn close_pc_wounded_coma_boundary(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        damage: u16,
        life_points_before: i16,
        life_points_after: i16,
    ) -> bool {
        let victim_is_pc = self
            .get_entity(victim_id)
            .is_some_and(|victim| victim.kind().is_pc());
        let coma_saved = victim_is_pc
            && life_points_before > 0
            && damage > 0
            && life_points_after <= 0
            && self.try_pc_coma_save(sim, assets, victim_id, damage);

        // The coma branch marks the campaign coma, stores the 5-HP floor
        // through the PC life-point setter (which emits HERO_HURT for a drop
        // greater than twenty), and only then applies maximum concussion.
        // The shared SayOuch path skips unconscious actors, so preserve that
        // life-point-setter callback explicitly at this boundary.
        if coma_saved && 5 < life_points_before - 20 {
            self.hero_speaking(assets, victim_id, HERO_HURT);
        }
        coma_saved
    }

    /// Check if a PC should be saved from death by an amulet (coma mechanic).
    ///
    /// If the PC is a VIP, not already in coma, and the campaign has amulets,
    /// the PC survives with 5 HP + max concussion instead of dying.
    ///
    /// Returns `true` if the coma save activated (caller should NOT
    /// proceed with normal death handling).
    pub(super) fn try_pc_coma_save(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        pc_id: EntityId,
        damage: u16,
    ) -> bool {
        let (is_pc, life_points, is_vip, status_idx) = {
            let entity = match self.get_entity(pc_id) {
                Some(e) => e,
                None => return false,
            };
            match entity {
                Entity::Pc(pc) => {
                    let vip = assets
                        .profile_manager
                        .get_character(pc.pc.profile_index)
                        .map(|p| p.vip)
                        .unwrap_or(false);
                    let Some(status_idx) = self.pc_description_index_for_pc_data(&pc.pc) else {
                        return false;
                    };
                    (true, pc.pc.life_points, vip, status_idx)
                }
                _ => return false,
            }
        };
        if !is_pc || damage < life_points as u16 {
            return false;
        }

        // Check if already in coma
        let in_coma = Some(&self.mission_domain.campaign)
            .and_then(|c| c.characters.get(status_idx))
            .map(|desc| desc.status.in_coma)
            .unwrap_or(false);
        if in_coma {
            return false;
        }

        // Check amulets
        let has_amulets = is_vip
            && Some(&self.mission_domain.campaign)
                .map(|c| c.values[crate::campaign::CampaignValue::Amulets] >= 1)
                .unwrap_or(false);
        if !has_amulets {
            return false;
        }

        // Activate coma save
        tracing::info!(entity = ?pc_id, "PC coma save activated — amulet consumed");

        // Set life to 5, max concussion, consume amulet
        if let Some(Entity::Pc(pc)) = self.world.entities.get_mut(pc_id) {
            pc.pc.life_points = 5;
            pc.human.concussion_of_the_brain = combat::CONCUSSION_MAX;
            pc.human.unconscious = true;
            pc.element.set_posture(Posture::Lying);
        }
        // `RHElementActorPC::GetWounded` reaches the coma save through
        // `SetPosture( RHPOSTURE_LYING )` (RHelementactorpc.cpp:8977), and
        // `RHElementActorHuman::SetPosture` (RHelementactorhuman.cpp:13549-13574)
        // runs `RHEngine::UpdateIntersectingCorpses( this, true )`
        // synchronously on the lying transition. The wounding site is the
        // *attacker's* creation slot, so the victim's own owner boundary —
        // and the end-of-tick drain — are both too late: an actor whose slot
        // falls between them samples the body's repulsive radius on the very
        // next frame and would see the full corpse radius instead of the
        // shrunken intersecting-corpse one.
        self.process_corpse_intersection_update_for(pc_id);
        if let Some(campaign) = Some(&mut self.mission_domain.campaign) {
            if let Some(desc) = campaign.characters.get_mut(status_idx) {
                desc.status.in_coma = true;
            }
            campaign.values[crate::campaign::CampaignValue::Amulets] -= 1;
        }
        // Play the PC-in-coma jingle once at the coma-transition
        // site (the dominant trigger in the reference is the portrait
        // burn invoked by the messenger when the PC enters coma).
        self.feedback
            .pending_side_effects
            .sounds
            .push(super::SoundCommand::Jingle(crate::sound::Jingle::PcInComa));
        // Wipe the PC's quick-action macro slots so a later coma
        // revive doesn't bring back pre-coma macro bindings.
        for slot in 0..crate::macro_store::NUMBER_OF_QA_MEMORY as u8 {
            self.abort_quick_action(pc_id, slot);
        }
        if let Some(entity) = self.world.entities.get_mut(pc_id)
            && let Some(pc) = entity.pc_data_mut()
        {
            pc.portrait.burned = true;
            pc.portrait.open = false;
        }
        // Stop derived live execution state.  Do not rewrite the actor's
        // action state here: Original's RHElementActorPC::GetWounded coma
        // branch calls SetConcussionOfTheBrain (which quits swordfight) and
        // SetPosture(LYING), but deliberately preserves the interrupted
        // action state.  The next damage/wait animation is selected from
        // that state (for example WAITING_SWORD chooses the sword-specific
        // unconscious animation).
        if let Some(entity) = self.world.entities.get_mut(pc_id)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.clear_path();
        }
        self.quit_swordfight(sim, assets, pc_id);

        // Add unconscious star titbit (event-driven creation).
        self.add_unconscious_star(pc_id);
        true
    }
}
