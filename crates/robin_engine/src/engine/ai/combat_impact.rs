//! Live combat impact callbacks and their synchronous tails.

use super::*;
use crate::ai::{AiState, EmoticonType, Stimulus, StimulusInfo, StimulusType, Substate};
use crate::sim_rng::SimulationContext;

impl EngineInner {
    pub(in crate::engine) fn execute_ai_combat_impact_event(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> bool {
        match stimulus.stimulus_type {
            StimulusType::EventQuitSwordfight => {
                let substate = self
                    .world
                    .entities
                    .expect_ai_controller(owner, format_args!("combat quit substate"))
                    .current_substate;
                if substate.is_real_swordfight() {
                    let entity = self.expect_entity(owner, "combat quit forest");
                    let forest = entity.camp() == Camp::Royalists
                        && self.world.weather.is_forest_level
                        && !entity.soldier_data().is_some_and(|soldier| soldier.rider);
                    if !forest || !self.execute_ai_merry_man_forest_cassos(sim, assets, owner) {
                        self.duty_set_state(
                            sim,
                            assets,
                            owner,
                            AiState::Attacking,
                            Substate::AttackingQuittingSwordfight,
                        );
                        let left = self
                            .world
                            .entities
                            .expect_enemy_ai(owner, format_args!("combat quit left"))
                            .left_combat_neighbour;
                        self.apply_update_left_combat_neighbour(owner.index(), left, None);
                        let right = self
                            .world
                            .entities
                            .expect_enemy_ai(owner, format_args!("combat quit right"))
                            .right_combat_neighbour;
                        self.apply_update_right_combat_neighbour(owner.index(), right, None);
                        self.combat_impact_timer(owner, 3);
                    }
                }
            }
            StimulusType::EventEnterSwordfight => {
                let StimulusInfo::Human(target) = stimulus.info else {
                    panic!("swordfight entry requires human target");
                };
                let frame = self.control.frame_counter;
                let ai = self
                    .world
                    .entities
                    .expect_enemy_ai_mut(owner, format_args!("combat entry"));
                ai.base.primary_target = Some(target);
                ai.enemy_seen_below = false;
                ai.base
                    .set_transient_emoticon(EmoticonType::XMark, 30, frame);

                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    Substate::AttackingSwordfight,
                );
                self.nearby_civilians_panic(sim, assets, owner);
                self.combat_impact_timer(owner, 20);
            }
            StimulusType::EventGotHit => self.execute_ai_got_hit_live(sim, assets, owner, stimulus),
            StimulusType::EventArrowLaunched => {
                self.execute_ai_arrow_launched_live(sim, assets, owner, stimulus)
            }
            StimulusType::EventDoorCombat => {
                let StimulusInfo::DoorCombat(ref combat) = stimulus.info else {
                    panic!("door combat requires door-combat info");
                };
                let ai = self
                    .world
                    .entities
                    .expect_enemy_ai_mut(owner, format_args!("door combat"));
                ai.base.primary_target = combat.adversary;
                ai.base.seek_position = combat.goal;
                ai.gather_direction = combat.direction;
                self.duty_set_state(
                    sim,
                    assets,
                    owner,
                    AiState::Attacking,
                    Substate::AttackingDoorFightDelay,
                );
                self.combat_impact_timer(owner, u32::from(combat.delay));
            }
            StimulusType::EventSeesFriendInTrouble => {}
            _ => return false,
        }
        true
    }

    fn combat_impact_timer(&mut self, owner: EntityId, frames: u32) {
        let frame = self.control.frame_counter;
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("combat impact timer"))
            .launch_timer(frames, frame);
    }

    fn combat_impact_stop(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        self.stop_ai_owner(sim, assets, owner);
    }

    fn execute_ai_arrow_launched_live(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) {
        let substate = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("incoming arrow state"))
            .current_substate;
        let protect = match substate {
            Substate::AttackingProtectingWithShield => {
                // The installed order can change before its sprite executes.
                // A pending lowering order must still react to another arrow.
                self.actor_order_type(owner) != Some(crate::order::OrderType::WaitingShield)
            }
            Substate::AttackingAdvancingWithShield | Substate::AttackingRunningToPhalanx => true,
            _ => false,
        };
        if !protect {
            return;
        }
        let StimulusInfo::Human(shooter) = stimulus.info else {
            panic!("incoming arrow requires shooter");
        };
        self.world
            .entities
            .expect_ai_controller_mut(owner, format_args!("incoming arrow target"))
            .primary_target = Some(shooter);
        self.combat_impact_stop(sim, assets, owner);
        let target = self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("incoming arrow target after stop"))
            .primary_target
            .expect("incoming arrow requires primary target");
        let target =
            self.expect_human_id_for_ai_handle(target.get(), "incoming arrow danger target");
        let danger = self
            .expect_entity(target, "incoming arrow danger point")
            .element_data()
            .position();
        use crate::sequence::{Field, FieldValue, SequenceElement};
        let mut element = SequenceElement::new_generic(
            1,
            crate::element::Command::RaiseShieldInstantly,
            Some(owner),
        );
        element.set_property(
            Field::ShieldDangerPoint,
            FieldValue::Point3D {
                x: danger.x,
                y: danger.y,
                z: danger.z,
            },
        );
        self.launch_element(sim, assets, element);
        let entity = self.expect_entity_mut(owner, "incoming arrow shield pose");
        entity.set_posture(crate::element::Posture::Upright);
        entity
            .actor_data_mut()
            .expect("shield owner is actor")
            .action_state = crate::element::ActionState::HoldingShield;
        self.refresh_retained_shield_obstacle(assets, owner);
        let ai = self
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("incoming arrow focus"));
        let target = ai.primary_target;
        self.execute_ai_focus(owner, target);

        self.duty_set_state(
            sim,
            assets,
            owner,
            AiState::Attacking,
            Substate::AttackingProtectingWithShield,
        );
        self.combat_impact_timer(owner, 15);
    }

    fn execute_ai_got_hit_live(
        &mut self,
        sim: &SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) {
        let entity = self.expect_entity(owner, "hit owner");
        let human = entity.human_data().expect("hit owner is human");
        if !human.opponents.is_empty() {
            let StimulusInfo::Human(attacker) = stimulus.info else {
                panic!("engaged hit requires human attacker");
            };
            let attacker =
                self.expect_human_id_for_ai_handle(attacker.get(), "engaged hit attacker");
            if self.expect_entity(attacker, "engaged hit camp").camp() != entity.camp()
                && !human.opponents.contains(&attacker)
            {
                self.direct_enter_swordfight(sim, assets, owner, attacker);
            }
        } else if self
            .world
            .entities
            .expect_ai_controller(owner, format_args!("hit substate"))
            .current_substate
            == Substate::MenacingPcInComa
        {
            let StimulusInfo::Human(attacker) = stimulus.info else {
                panic!("menacing hit requires human attacker");
            };
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Attacking,
                Substate::AttackingReturnToOtherPcAfterMenacing,
            );
            self.world
                .entities
                .expect_ai_controller_mut(owner, format_args!("menacing hit target"))
                .primary_target = Some(attacker);
            use crate::sequence::{Field, FieldValue, SequenceElement};
            let mut element = SequenceElement::new_generic(
                1,
                crate::element::Command::EnterSwordfight,
                Some(owner),
            );
            element.set_property(Field::Opponent, FieldValue::Integer(0));
            element.set_property(Field::JumplineDestination, FieldValue::Integer(0));
            element.set_property(Field::SwordfightPrepared, FieldValue::Bool(false));
            self.launch_element(sim, assets, element);
            let target = self
                .world
                .entities
                .expect_ai_controller(owner, format_args!("menacing hit direction target"))
                .primary_target
                .expect("menacing hit direction requires target");
            let target = self.expect_human_id_for_ai_handle(target.get(), "menacing hit direction");
            let there = self.live_ai_position(target);
            let here = self.live_ai_position(owner);
            let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                there.x - here.x,
                there.y - here.y,
            );
            self.expect_entity_mut(owner, "menacing hit direction owner")
                .position_iface_mut()
                .set_direction(crate::position_interface::Direction::from_raw(i32::from(
                    direction,
                )));
        } else {
            self.combat_impact_stop(sim, assets, owner);
            let StimulusInfo::Human(attacker) = stimulus.info else {
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("nonhuman hit target"))
                    .primary_target = None;
                return;
            };
            let target = self.expect_human_id_for_ai_handle(attacker.get(), "hit attacker");
            if matches!(target, EntityId::Soldier(_)) {
                let brawl = self
                    .world
                    .entities
                    .expect_ai_controller(target, format_args!("hit soldier substate"))
                    .current_substate
                    .is_fight_for_money();
                if brawl {
                    self.world
                        .entities
                        .expect_ai_controller_mut(owner, format_args!("brawl hit friend"))
                        .friend_in_trouble = Some(attacker);
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Wondering,
                        Substate::WonderingBrawlGotHit,
                    );
                    self.world
                        .entities
                        .expect_ai_controller_mut(owner, format_args!("brawl hit emoticon"))
                        .set_emoticon(EmoticonType::None);
                }
            } else {
                self.world
                    .entities
                    .expect_ai_controller_mut(owner, format_args!("hit primary target"))
                    .primary_target = Some(attacker);
                self.execute_ai_attack_enemy(sim, assets, owner, attacker.get());
            }

            let npc = self
                .world
                .entities
                .expect_ai_actor_data_mut(owner, format_args!("hit eye status"));
            crate::ai_vision::set_view_status(npc, crate::element::EyeStatus::DieOrGetUnconscious);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::Command;
    use crate::order::OrderType;

    #[test]
    fn incoming_arrow_checks_installed_shield_order_instead_of_last_sprite() {
        for (installed, displayed, protects) in [
            (
                Some(OrderType::LoweringShield),
                OrderType::WaitingShield,
                true,
            ),
            (
                Some(OrderType::WaitingShield),
                OrderType::LoweringShield,
                false,
            ),
            (None, OrderType::WaitingShield, true),
        ] {
            let (mut engine, assets, owner, target) =
                super::super::battle_decision_observation_tests::fixture(false);
            let installed = installed.map(|action| engine.install_test_order(owner, action));
            engine.install_actor_order(owner, installed);
            let entity = engine.ent_mut(owner);
            entity.sprite_mut().last_action = displayed;
            let actor = entity.actor_data_mut().unwrap();
            actor.action_state = crate::element::ActionState::HoldingShield;
            entity.enemy_ai_mut().unwrap().base.current_substate =
                Substate::AttackingProtectingWithShield;
            let mut stimulus = Stimulus::new(StimulusType::EventArrowLaunched);
            stimulus.info = StimulusInfo::Human(crate::ai::AiEntityHandle::new(target.index()));

            engine.execute_ai_combat_impact_event(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &stimulus,
            );

            let queued_raise = engine
                .orders
                .sequence_manager
                .sequences_iter()
                .any(|sequence| {
                    sequence.elements.iter().any(|element| {
                        element.owner == Some(owner)
                            && element.command == Command::RaiseShieldInstantly
                    })
                });
            assert_eq!(
                queued_raise, protects,
                "installed={installed:?}, displayed={displayed:?}"
            );
        }
    }
}
