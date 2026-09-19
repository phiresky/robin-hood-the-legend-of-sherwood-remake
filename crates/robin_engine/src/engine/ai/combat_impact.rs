//! Live combat impact callbacks and their synchronous tails.

use super::*;
use crate::ai::{AiState, EmoticonType, Stimulus, StimulusInfo, StimulusType, Substate};

impl EngineInner {
    fn combat_impact_timer(&mut self, owner: EntityId, frames: u32) {
        let frame = self.control.frame_counter;
        self.ai_mut(owner, "combat impact timer")
            .launch_timer(frames, frame);
    }
}

impl AiOwnerCtx<'_> {
    pub(in crate::engine) fn execute_ai_combat_impact_event(
        &mut self,
        stimulus: &Stimulus,
    ) -> bool {
        match stimulus.stimulus_type {
            StimulusType::EventQuitSwordfight => {
                let substate = self
                    .engine
                    .ai(self.owner, "combat quit substate")
                    .current_substate;
                if substate.is_real_swordfight() {
                    let entity = self.engine.expect_entity(self.owner, "combat quit forest");
                    let forest = entity.camp() == Camp::Royalists
                        && self.engine.world.weather.is_forest_level
                        && !entity.soldier_data().is_some_and(|soldier| soldier.rider);
                    if !forest || !self.execute_ai_merry_man_forest_cassos() {
                        self.duty_set_state(
                            AiState::Attacking,
                            Substate::AttackingQuittingSwordfight,
                        );
                        let left = self
                            .engine
                            .enemy_ai(self.owner, "combat quit left")
                            .left_combat_neighbour;
                        self.engine.apply_update_left_combat_neighbour(
                            self.owner.index(),
                            left,
                            None,
                        );
                        let right = self
                            .engine
                            .enemy_ai(self.owner, "combat quit right")
                            .right_combat_neighbour;
                        self.engine.apply_update_right_combat_neighbour(
                            self.owner.index(),
                            right,
                            None,
                        );
                        self.engine.combat_impact_timer(self.owner, 3);
                    }
                }
            }
            StimulusType::EventEnterSwordfight => {
                let StimulusInfo::Human(target) = stimulus.info else {
                    panic!("swordfight entry requires human target");
                };
                let frame = self.engine.control.frame_counter;
                let ai = self.engine.enemy_ai_mut(self.owner, "combat entry");
                ai.base.primary_target = Some(target);
                ai.enemy_seen_below = false;
                ai.base
                    .set_transient_emoticon(EmoticonType::XMark, 30, frame);

                self.duty_set_state(AiState::Attacking, Substate::AttackingSwordfight);
                self.engine.nearby_civilians_panic(self.tcx, self.owner);
                self.engine.combat_impact_timer(self.owner, 20);
            }
            StimulusType::EventGotHit => self.execute_ai_got_hit_live(stimulus),
            StimulusType::EventArrowLaunched => self.execute_ai_arrow_launched_live(stimulus),
            StimulusType::EventDoorCombat => {
                let StimulusInfo::DoorCombat(ref combat) = stimulus.info else {
                    panic!("door combat requires door-combat info");
                };
                let ai = self.engine.enemy_ai_mut(self.owner, "door combat");
                ai.base.primary_target = combat.adversary;
                ai.base.seek_position = combat.goal;
                ai.gather_direction = combat.direction;
                self.duty_set_state(AiState::Attacking, Substate::AttackingDoorFightDelay);
                self.engine
                    .combat_impact_timer(self.owner, u32::from(combat.delay));
            }
            StimulusType::EventSeesFriendInTrouble => {}
            _ => return false,
        }
        true
    }

    fn combat_impact_stop(&mut self) {
        self.stop_ai_owner();
    }

    fn execute_ai_arrow_launched_live(&mut self, stimulus: &Stimulus) {
        let substate = self
            .engine
            .ai(self.owner, "incoming arrow state")
            .current_substate;
        let protect = match substate {
            Substate::AttackingProtectingWithShield => {
                // The installed order can change before its sprite executes.
                // A pending lowering order must still react to another arrow.
                self.engine.actor_order_type(self.owner)
                    != Some(crate::order::OrderType::WaitingShield)
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
        self.engine
            .ai_mut(self.owner, "incoming arrow target")
            .primary_target = Some(shooter);
        self.combat_impact_stop();
        let target = self
            .engine
            .ai(self.owner, "incoming arrow target after stop")
            .primary_target
            .expect("incoming arrow requires primary target");
        let target = self
            .engine
            .expect_human_id_for_ai_handle(target.get(), "incoming arrow danger target");
        let danger = self
            .engine
            .expect_entity(target, "incoming arrow danger point")
            .element_data()
            .position();
        use crate::sequence::{Field, FieldValue, SequenceElement};
        let mut element = SequenceElement::new_generic(
            1,
            crate::element::Command::RaiseShieldInstantly,
            Some(self.owner),
        );
        element.set_property(
            Field::ShieldDangerPoint,
            FieldValue::Point3D {
                x: danger.x,
                y: danger.y,
                z: danger.z,
            },
        );
        self.engine.launch_element(self.tcx, element);
        let entity = self
            .engine
            .expect_entity_mut(self.owner, "incoming arrow shield pose");
        entity.set_posture(crate::element::Posture::Upright);
        entity
            .actor_data_mut()
            .expect("shield owner is actor")
            .action_state = crate::element::ActionState::HoldingShield;
        self.engine
            .refresh_retained_shield_obstacle(self.tcx.assets, self.owner);
        let ai = self.engine.ai_mut(self.owner, "incoming arrow focus");
        let target = ai.primary_target;
        self.engine.execute_ai_focus(self.owner, target);

        self.duty_set_state(AiState::Attacking, Substate::AttackingProtectingWithShield);
        self.engine.combat_impact_timer(self.owner, 15);
    }

    fn execute_ai_got_hit_live(&mut self, stimulus: &Stimulus) {
        let entity = self.engine.expect_entity(self.owner, "hit owner");
        let human = entity.human_data().expect("hit owner is human");
        if !human.opponents.is_empty() {
            let StimulusInfo::Human(attacker) = stimulus.info else {
                panic!("engaged hit requires human attacker");
            };
            let attacker = self
                .engine
                .expect_human_id_for_ai_handle(attacker.get(), "engaged hit attacker");
            if self
                .engine
                .expect_entity(attacker, "engaged hit camp")
                .camp()
                != entity.camp()
                && !human.opponents.contains(&attacker)
            {
                self.engine
                    .direct_enter_swordfight(self.tcx, self.owner, attacker);
            }
        } else if self.engine.ai(self.owner, "hit substate").current_substate
            == Substate::MenacingPcInComa
        {
            let StimulusInfo::Human(attacker) = stimulus.info else {
                panic!("menacing hit requires human attacker");
            };
            self.duty_set_state(
                AiState::Attacking,
                Substate::AttackingReturnToOtherPcAfterMenacing,
            );
            self.engine
                .ai_mut(self.owner, "menacing hit target")
                .primary_target = Some(attacker);
            use crate::sequence::{Field, FieldValue, SequenceElement};
            let mut element = SequenceElement::new_generic(
                1,
                crate::element::Command::EnterSwordfight,
                Some(self.owner),
            );
            element.set_property(Field::Opponent, FieldValue::Integer(0));
            element.set_property(Field::JumplineDestination, FieldValue::Integer(0));
            element.set_property(Field::SwordfightPrepared, FieldValue::Bool(false));
            self.engine.launch_element(self.tcx, element);
            let target = self
                .engine
                .ai(self.owner, "menacing hit direction target")
                .primary_target
                .expect("menacing hit direction requires target");
            let target = self
                .engine
                .expect_human_id_for_ai_handle(target.get(), "menacing hit direction");
            let there = self.engine.live_ai_position(target);
            let here = self.engine.live_ai_position(self.owner);
            let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                there.x - here.x,
                there.y - here.y,
            );
            self.engine
                .expect_entity_mut(self.owner, "menacing hit direction owner")
                .position_iface_mut()
                .set_direction(crate::position_interface::Direction::from_raw(i32::from(
                    direction,
                )));
        } else {
            self.combat_impact_stop();
            let StimulusInfo::Human(attacker) = stimulus.info else {
                self.engine
                    .ai_mut(self.owner, "nonhuman hit target")
                    .primary_target = None;
                return;
            };
            let target = self
                .engine
                .expect_human_id_for_ai_handle(attacker.get(), "hit attacker");
            if matches!(target, EntityId::Soldier(_)) {
                let brawl = self
                    .engine
                    .ai(target, "hit soldier substate")
                    .current_substate
                    .is_fight_for_money();
                if brawl {
                    self.engine
                        .ai_mut(self.owner, "brawl hit friend")
                        .friend_in_trouble = Some(attacker);
                    self.duty_set_state(AiState::Wondering, Substate::WonderingBrawlGotHit);
                    self.engine
                        .ai_mut(self.owner, "brawl hit emoticon")
                        .set_emoticon(EmoticonType::None);
                }
            } else {
                self.engine
                    .ai_mut(self.owner, "hit primary target")
                    .primary_target = Some(attacker);
                self.execute_ai_attack_enemy(attacker.get());
            }

            let npc = self.engine.ai_actor_mut(self.owner, "hit eye status");
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

            engine
                .ai_ctx(&crate::sim_rng::test_context(), &assets, owner)
                .execute_ai_combat_impact_event(&stimulus);

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
