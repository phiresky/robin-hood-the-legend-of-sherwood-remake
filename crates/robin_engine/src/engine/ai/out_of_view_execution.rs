//! Visibility-loss fallthrough and pursuit against live actor state.

#[cfg(test)]
mod tests;

use super::*;
use crate::ai::{AiEntityHandle, AiState, Stimulus, StimulusInfo, Substate};
use crate::ai_enemy::AiMapVec;
use crate::ai_enemy::{SeekFlags, UNDEFINED_DIRECTION};
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_out_of_view(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> bool {
        let ai = self.enemy_ai(owner, "visibility loss");
        if ai.base.current_state != AiState::Attacking {
            return false;
        }
        let StimulusInfo::Human(target) = stimulus.info else {
            panic!("visibility loss requires a human");
        };
        let substate = ai.base.current_substate;
        let bow = matches!(
            substate,
            Substate::AttackingBowObservingLoading
                | Substate::AttackingBowObserving
                | Substate::AttackingBowShooting
                | Substate::AttackingBowLoading
                | Substate::AttackingBowAiming
        );
        if bow && ai.enemy_seen_below {
            self.reinitialize_live_ai_enemies(owner);
            return true;
        }
        if (bow || substate.is_any_swordfight())
            && ai.base.primary_target == Some(target)
            && self.patrol_member_visible(
                assets,
                owner,
                self.expect_human_id_for_ai_handle(target.get(), "visibility loss target"),
            )
        {
            return false;
        }
        let moving = matches!(
            substate,
            Substate::AttackingReactiontimeRunning
                | Substate::AttackingApproachToObserve
                | Substate::AttackingAdvancingWithShield
        );
        if (bow || substate.is_any_swordfight() || moving) && self.live_enemy_is_behind_me(owner) {
            return false;
        }
        if bow
            || substate.is_any_swordfight()
            || moving
            || matches!(
                substate,
                Substate::AttackingReactiontime
                    | Substate::AttackingQuittingSwordfight
                    | Substate::AttackingReserve
                    | Substate::AttackingLastReserve
                    | Substate::AttackingObserve
                    | Substate::AttackingObserveAndMove
                    | Substate::AttackingHitting
                    | Substate::AttackingProtectingWithShield
                    | Substate::AttackingPhalanx
                    | Substate::AttackingTooProudToAttack
                    | Substate::AttackingTooProudToAttackApproach
            )
        {
            let target =
                self.expect_human_id_for_ai_handle(target.get(), "visibility loss forecast");
            self.execute_live_out_of_view_seek(sim, assets, owner, target);
        } else if matches!(
            substate,
            Substate::AttackingTooProudToAttackRetire
                | Substate::AttackingTooProudToAttackRetireTurn
                | Substate::AttackingReactiontimeBending
        ) {
        } else {
            self.reinitialize_live_ai_enemies(owner);
            if substate == Substate::AttackingWaitForAvengerOnRoof {
                if self
                    .enemy_ai(owner, "roof visibility loss")
                    .list_them
                    .is_empty()
                {
                    self.execute_ai_seek_area(
                        sim,
                        assets,
                        owner,
                        self.live_ai_position(owner),
                        crate::parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                        SeekFlags::empty(),
                        UNDEFINED_DIRECTION,
                    );
                } else {
                    self.execute_ai_get_battle_overview(sim, assets, owner, 0);
                }
            }
        }
        false
    }

    fn live_enemy_is_behind_me(&self, owner: EntityId) -> bool {
        let entity = self.expect_entity(owner, "visibility loss stare");
        let ground = entity.element_data().position();
        let stare = entity
            .ai_actor_data()
            .expect("visibility loss requires NPC view")
            .stare_point;
        let dx = (stare.x - ground.x) * crate::position_interface::ASPECT_RATIO;
        let dy = stare.y - ground.y;
        let (look_x, look_y) =
            crate::element::direction_vector_16(entity.element_data().direction());
        look_x * dx + look_y * dy < 0.0
    }

    fn execute_live_out_of_view_seek(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: EntityId,
    ) {
        let input = extract_exact_forecast_input(
            self,
            self.expect_entity(target, "visibility loss forecast target"),
            selected_actor_is_passing_door(
                &self.world.entities,
                &self.orders.sequence_manager,
                target,
            ),
        )
        .expect("visibility loss forecast requires an actor");
        let direction = self
            .enemy_ai(owner, "lost direction")
            .pc_gone_away_in_this_direction;
        let forecast = crate::ai::prepare_forecast_destination_for_ia(
            &input,
            &self.script_domains.interactables.doors,
            &self.world.fast_grid.level.sectors,
            &self.world.fast_grid.level.sector_number_map,
        )
        .resolve_retaining_direction(sim, direction);
        let ai = self.enemy_ai_mut(owner, "visibility loss forecast");
        ai.base.seek_position = forecast.position;
        ai.pc_gone_away_in_this_direction = forecast.direction;
        ai.missed_pc = Some(AiEntityHandle::new(target.index()));
        ai.pc_missed = true;
        self.reinitialize_live_ai_enemies(owner);
        if !self
            .enemy_ai(owner, "visibility loss enemies")
            .list_them
            .is_empty()
        {
            return;
        }
        if !self
            .expect_entity(owner, "visibility loss opponents")
            .human_data()
            .expect("visibility loss requires human")
            .opponents
            .is_empty()
        {
            self.execute_ai_end_swordfight(sim, assets, owner);
        }
        self.finish_live_lost_enemy_pursuit(sim, assets, owner);
    }

    pub(super) fn finish_live_lost_enemy_pursuit(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        self.execute_ai_unfocus(owner);

        let ai = self.enemy_ai(owner, "lost pursuit policy");
        let missed = self.expect_human_id_for_ai_handle(
            ai.missed_pc.expect("lost opponent").get(),
            "lost opponent",
        );
        let entity = self.expect_entity(owner, "lost pursuit owner");
        let follow = ai.base.blood_alcohol as i32
            <= crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            && (!(entity.is_active()
                && !self.entity_data_in_building_sector(entity.element_data()))
                || (!ai.combat_trainer && ai.company_number != 100));
        if self.expect_entity(missed, "lost pursuit target").is_pc() && follow {
            self.execute_ai_speech(
                sim,
                assets,
                owner,
                crate::ai::AiSpeechAttempt {
                    remark: crate::ai::Remark::HuntsEnemy,
                    flags: 0,
                },
            );
            let ai = self.enemy_ai(owner, "lost target search");
            let center = ai.base.seek_position;
            let direction = ai.pc_gone_away_in_this_direction;
            self.execute_ai_seek_area(
                sim,
                assets,
                owner,
                center,
                crate::parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                crate::ai_enemy::SeekFlags::LOCATION_FIRST | crate::ai_enemy::SeekFlags::HOUSE,
                direction,
            );
        } else {
            let direction = (self.live_ai_position(missed).map_point()
                - self.live_ai_position(owner).map_point())
            .sector_with_aspect(crate::position_interface::ASPECT_RATIO);
            self.world
                .entities
                .expect_entity_mut(owner, format_args!("instant AI direction owner"))
                .element_data_mut()
                .set_direction_instantly(direction as i16);

            self.execute_ai_get_battle_overview(sim, assets, owner, 0);
        }
    }
}
