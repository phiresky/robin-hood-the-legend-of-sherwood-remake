//! Visibility-loss fallthrough and pursuit against live actor state.

#[cfg(test)]
mod tests;

use super::*;
use crate::ai::{AiEntityHandle, AiState, Stimulus, StimulusInfo, Substate};
use crate::ai_enemy::AiMapVec;
use crate::ai_enemy::{SeekFlags, UNDEFINED_DIRECTION};

impl EngineInner {
    #[cfg(test)]
    pub(in crate::engine) fn execute_ai_out_of_view(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> bool {
        AiOwnerCtx::new(self, tcx, owner).execute_ai_out_of_view(stimulus)
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
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_out_of_view(&mut self, stimulus: &Stimulus) -> bool {
        let ai = self.engine.enemy_ai(self.owner, "visibility loss");
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
            self.engine.reinitialize_live_ai_enemies(self.owner);
            return true;
        }
        if (bow || substate.is_any_swordfight())
            && ai.base.primary_target == Some(target)
            && self.engine.patrol_member_visible(
                self.tcx.assets,
                self.owner,
                self.engine
                    .expect_human_id_for_ai_handle(target.get(), "visibility loss target"),
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
        if (bow || substate.is_any_swordfight() || moving)
            && self.engine.live_enemy_is_behind_me(self.owner)
        {
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
            let target = self
                .engine
                .expect_human_id_for_ai_handle(target.get(), "visibility loss forecast");
            self.execute_live_out_of_view_seek(target);
        } else if matches!(
            substate,
            Substate::AttackingTooProudToAttackRetire
                | Substate::AttackingTooProudToAttackRetireTurn
                | Substate::AttackingReactiontimeBending
        ) {
        } else {
            self.engine.reinitialize_live_ai_enemies(self.owner);
            if substate == Substate::AttackingWaitForAvengerOnRoof {
                if self
                    .engine
                    .enemy_ai(self.owner, "roof visibility loss")
                    .list_them
                    .is_empty()
                {
                    self.execute_ai_seek_area(
                        self.engine.live_ai_position(self.owner),
                        crate::parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                        SeekFlags::empty(),
                        UNDEFINED_DIRECTION,
                    );
                } else {
                    self.execute_ai_get_battle_overview(0);
                }
            }
        }
        false
    }

    fn execute_live_out_of_view_seek(&mut self, target: EntityId) {
        let input = extract_exact_forecast_input(
            self.engine,
            self.engine
                .expect_entity(target, "visibility loss forecast target"),
            selected_actor_is_passing_door(&self.engine.entities(), &self.engine.seq(), target),
        )
        .expect("visibility loss forecast requires an actor");
        let direction = self
            .engine
            .enemy_ai(self.owner, "lost direction")
            .pc_gone_away_in_this_direction;
        let forecast = crate::ai::prepare_forecast_destination_for_ia(
            &input,
            &self.engine.script_domains.interactables.doors,
            &self.engine.world.fast_grid.level.sectors,
            &self.engine.world.fast_grid.level.sector_number_map,
        )
        .resolve_retaining_direction(self.tcx.sim, direction);
        let ai = self
            .engine
            .enemy_ai_mut(self.owner, "visibility loss forecast");
        ai.base.seek_position = forecast.position;
        ai.pc_gone_away_in_this_direction = forecast.direction;
        ai.missed_pc = Some(AiEntityHandle::new(target.index()));
        ai.pc_missed = true;
        self.engine.reinitialize_live_ai_enemies(self.owner);
        if !self
            .engine
            .enemy_ai(self.owner, "visibility loss enemies")
            .list_them
            .is_empty()
        {
            return;
        }
        if !self
            .engine
            .expect_entity(self.owner, "visibility loss opponents")
            .human_data()
            .expect("visibility loss requires human")
            .opponents
            .is_empty()
        {
            self.execute_ai_end_swordfight();
        }
        self.finish_live_lost_enemy_pursuit();
    }

    pub(super) fn finish_live_lost_enemy_pursuit(&mut self) {
        self.engine.execute_ai_unfocus(self.owner);

        let ai = self.engine.enemy_ai(self.owner, "lost pursuit policy");
        let missed = self.engine.expect_human_id_for_ai_handle(
            ai.missed_pc.expect("lost opponent").get(),
            "lost opponent",
        );
        let entity = self.engine.expect_entity(self.owner, "lost pursuit owner");
        let follow = ai.base.blood_alcohol as i32
            <= crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
            && (!(entity.is_active()
                && !self
                    .engine
                    .entity_data_in_building_sector(entity.element_data()))
                || (!ai.combat_trainer && ai.company_number != 100));
        if self
            .engine
            .expect_entity(missed, "lost pursuit target")
            .is_pc()
            && follow
        {
            self.execute_ai_speech(crate::ai::AiSpeechAttempt {
                remark: crate::ai::Remark::HuntsEnemy,
                flags: 0,
            });
            let ai = self.engine.enemy_ai(self.owner, "lost target search");
            let center = ai.base.seek_position;
            let direction = ai.pc_gone_away_in_this_direction;
            self.execute_ai_seek_area(
                center,
                crate::parameters_ai::AI_LOST_ENEMY_SEEK_RADIUS as u16,
                crate::ai_enemy::SeekFlags::LOCATION_FIRST | crate::ai_enemy::SeekFlags::HOUSE,
                direction,
            );
        } else {
            let direction = (self.engine.live_ai_position(missed).map_point()
                - self.engine.live_ai_position(self.owner).map_point())
            .sector_with_aspect(crate::position_interface::ASPECT_RATIO);
            self.engine
                .entities_mut()
                .expect_entity_mut(self.owner, format_args!("instant AI direction owner"))
                .element_data_mut()
                .set_direction_instantly(direction as i16);

            self.execute_ai_get_battle_overview(0);
        }
    }
}
