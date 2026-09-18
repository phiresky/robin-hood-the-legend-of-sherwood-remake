//! Live panic segments and their synchronous movement-failure recovery.

use super::*;
use crate::ai::{AiState, AlertLevel, GotoFlags, Position, Stimulus, StimulusType, Substate};

fn panic_retry_side(creation_order: u32) -> u8 {
    if creation_order & 1 != 0 { 4 } else { 12 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spent_panic_hides_blinks_and_consumes_only_required_random_draws() {
        for directed in [false, true] {
            for stimulus in [
                StimulusType::EventReachPoint,
                StimulusType::EventCouldntReachPoint,
            ] {
                let mut engine = EngineInner::new();
                let owner = engine.add_test_entity(
                    crate::engine::test_support::actors::make_test_ai_soldier(
                        crate::element::Camp::Lacklandists,
                    ),
                );
                let mut assets = LevelAssets::new();
                crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
                engine.control.frame_counter = 70;
                engine.enter_ai_think_frame(owner);
                let entity = engine.get_entity_mut(owner).unwrap();
                entity.enemy_ai_mut().unwrap().fleeing_seen_enemy_counter = 20;
                let ai = entity.ai_controller_mut().unwrap();
                ai.current_state = AiState::Fleeing;
                ai.current_substate = Substate::FleeingPanic;
                ai.directed_panic = directed;
                ai.lasting_panic_runs = 0;
                ai.panic_center_x = 100.0;
                entity.npc_data_mut().unwrap().detectable_lists
                    [crate::element::DetectableType::Enemy as usize]
                    .push(crate::element::Detectable {
                        detectable_type: crate::element::DetectableType::Enemy,
                        seen_now: true,
                        seen_last_frame: true,
                        ..Default::default()
                    });
                let (_, draws) = crate::sim_rng::with_draw_trace(|| {
                    engine
                        .execute_ai_common_fleeing_event(
                            &crate::sim_rng::test_context(),
                            &assets,
                            owner,
                            &Stimulus::new(stimulus),
                        )
                        .expect("spent panic event must be handled");
                });
                let ai = engine.get_entity(owner).unwrap().ai_controller().unwrap();
                assert_eq!(
                    engine
                        .get_entity(owner)
                        .unwrap()
                        .enemy_ai()
                        .unwrap()
                        .fleeing_seen_enemy_counter,
                    if stimulus == StimulusType::EventReachPoint {
                        0
                    } else {
                        20
                    },
                    "only arrival renews the soldier's panic sighting budget",
                );
                assert_eq!(ai.current_substate, Substate::FleeingHiding);
                assert_eq!(ai.view_alert_status, AlertLevel::Yellow);
                assert!(ai.timer_is_running);
                assert!(
                    (70 + crate::parameters_ai::AI_MIN_PANIC_HIDING_TIME as u32
                        ..70 + crate::parameters_ai::AI_MIN_PANIC_HIDING_TIME as u32
                            + crate::parameters_ai::AI_DELTA_PANIC_HIDING_TIME as u32)
                        .contains(&ai.when_does_timer_ring)
                );
                assert_eq!(draws.len(), if directed { 1 } else { 2 });
                assert!(
                    draws
                        .iter()
                        .all(|site| *site == crate::sim_rng::RngSite::AiPanic)
                );
                let detectable = &engine
                    .get_entity(owner)
                    .unwrap()
                    .npc_data()
                    .unwrap()
                    .detectable_lists[crate::element::DetectableType::Enemy as usize][0];
                assert!(!detectable.seen_now && !detectable.seen_last_frame);
            }
        }
    }

    #[test]
    fn retry_turn_uses_creation_order_instead_of_entity_slot() {
        assert_eq!(panic_retry_side(68), 12);
        assert_eq!(panic_retry_side(69), 4);
        assert_ne!(panic_retry_side(68), panic_retry_side(37));
    }
}

impl EngineInner {
    pub(in crate::engine) fn execute_ai_enemy_fleeing_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        let event = stimulus.stimulus_type;
        if !matches!(
            event,
            StimulusType::EventReachPoint
                | StimulusType::EventDone
                | StimulusType::EventTimer
                | StimulusType::CallYourTalk1
                | StimulusType::CallYourTalk2
                | StimulusType::CallYourTalk3
                | StimulusType::EventMyTalk1
                | StimulusType::EventMyTalk2
                | StimulusType::EventMyTalk3
        ) {
            return None;
        }
        match self.observation_ai(owner).base.current_substate {
            Substate::FleeingRunToAlertSoldiers => {
                if event == StimulusType::EventReachPoint {
                    let ai = self.observation_ai(owner);
                    let center = ai.base.seek_position;
                    let flags = ai.seek_flags.bits();
                    if !self.execute_ai_alert_soldiers(sim, assets, owner, center, flags) {
                        self.duty_set_state(
                            sim,
                            assets,
                            owner,
                            AiState::Fleeing,
                            Substate::FleeingRunToDoor,
                        );
                        self.execute_ai_callback(
                            sim,
                            assets,
                            owner,
                            &Stimulus::new(StimulusType::EventReachPoint),
                        );
                    }
                }
            }
            Substate::FleeingRetireFromCombat => {
                if event == StimulusType::EventReachPoint {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Fleeing,
                        Substate::FleeingRetireFromCombatTurn,
                    );
                    let position = self.observation_ai(owner).base.seek_position;
                    self.duty_face_position_signed_elevation(
                        sim, assets, owner, position, -1, true,
                    );
                }
            }
            Substate::FleeingRetireFromCombatTurn => {
                if event == StimulusType::EventDone {
                    let target = self.observation_ai(owner).base.primary_target;
                    let sees_target = target.is_some_and(|target| {
                        let target = self
                            .expect_human_id_for_ai_handle(target.get(), "retiring primary target");
                        self.live_ai_detects_180(assets, owner, target)
                    });
                    if sees_target {
                        self.execute_battle_decisions(sim, assets, owner);
                    } else {
                        self.execute_ai_get_battle_overview(sim, assets, owner, 0);
                    }
                }
            }
            Substate::FleeingMerryManRunToLeaveMap => match event {
                StimulusType::EventTimer => {
                    if self
                        .expect_entity(owner, "forest fleeing actor")
                        .actor_data()
                        .expect("forest fleeing owner must be actor")
                        .action_state
                        != crate::element::ActionState::MovingFast
                        && self
                            .observation_ai(owner)
                            .base
                            .last_goto_destination
                            .sector
                            .is_some()
                    {
                        self.observation_stop(sim, assets, owner);
                        let destination = self.observation_ai(owner).base.last_goto_destination;
                        self.duty_go_to(sim, assets, owner, destination, GotoFlags::RUN);
                    }
                    self.observation_timer(owner, 30);
                }
                StimulusType::EventReachPoint => {
                    let position = self.live_ai_position(owner);
                    let destination = self.observation_ai(owner).base.last_goto_destination;
                    if (position.x - destination.x)
                        .abs()
                        .max((position.y - destination.y).abs())
                        < 10.0
                    {
                        self.duty_set_state(
                            sim,
                            assets,
                            owner,
                            AiState::Fleeing,
                            Substate::FleeingMerryManLeaveMap,
                        );
                        let door = self
                            .observation_ai(owner)
                            .base
                            .my_door_index
                            .expect("forest exit requires selected door");
                        let point = self
                            .script_domains
                            .interactables
                            .doors
                            .get(usize::from(door))
                            .expect("forest exit selected door must exist")
                            .point_out;
                        let mut movement = crate::sequence::SequenceElement::new_movement(
                            1,
                            crate::element::Command::Move,
                            Some(owner),
                            crate::order::OrderType::RunningUpright,
                        );
                        let crate::sequence::SequenceElementData::Movement {
                            destination,
                            flags,
                            ..
                        } = &mut movement.data
                        else {
                            unreachable!("movement constructor must produce movement data");
                        };
                        *destination = crate::coordinates::MapPoint::new(point.x, point.y);
                        *flags = crate::sequence::MoveFlags::MAP;
                        self.launch_element(sim, assets, movement);
                    } else {
                        self.duty_go_to(sim, assets, owner, destination, GotoFlags::RUN);
                        self.observation_timer(owner, 30);
                    }
                }
                _ => {}
            },
            Substate::FleeingMerryManLeaveMap => {
                if event == StimulusType::EventReachPoint {
                    self.observation_ai_mut(owner)
                        .base
                        .non_script_lock(crate::ai::AiLockFlags::FREEZE);
                    self.world
                        .entities
                        .expect_entity_mut(owner, format_args!("forest exit actor"))
                        .element_data_mut()
                        .active = false;
                }
            }
            _ => return None,
        }
        Some(false)
    }

    pub(in crate::engine) fn execute_ai_common_fleeing_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: &Stimulus,
    ) -> Option<bool> {
        let event = stimulus.stimulus_type;
        let ai = self.ai(owner, "common fleeing owner");
        match ai.current_substate {
            Substate::FleeingPanic => {
                let no_runs = ai.lasting_panic_runs == 0;
                if no_runs {
                    let entity = self
                        .world
                        .entities
                        .expect_entity_mut(owner, format_args!("panic counter"));
                    if let Some(friendly) = entity.friendly_ai_mut() {
                        friendly.fleeing_seen_enemy_counter = 0;
                    } else if event == StimulusType::EventReachPoint {
                        // Arrival renews the soldier's sighting budget. A failed
                        // final segment enters hiding through failure recovery
                        // and retains the budget already spent while fleeing.
                        entity
                            .enemy_ai_mut()
                            .expect("panic owner needs AI role")
                            .fleeing_seen_enemy_counter = 0;
                    }
                }
                if matches!(
                    event,
                    StimulusType::EventReachPoint | StimulusType::EventCouldntReachPoint
                ) {
                    self.execute_ai_panic_segment(sim, assets, owner, event);
                }
            }
            Substate::FleeingRunToHide | Substate::FleeingRunToDoor => {
                if event == StimulusType::EventReachPoint {
                    self.duty_set_state(
                        sim,
                        assets,
                        owner,
                        AiState::Fleeing,
                        Substate::FleeingHiding,
                    );
                    self.execute_ai_set_alert_status(
                        assets,
                        owner,
                        AlertLevel::Yellow,
                        crate::ai::AlertFlags::empty(),
                    );
                    let ai = self.ai_mut(owner, "hide alert");
                    ai.clear_emoticon();
                    let center = (ai.panic_center_x, ai.panic_center_y);
                    let position = self.live_ai_position(owner);
                    let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
                        center.0 - position.x,
                        center.1 - position.y,
                    ) as u16;
                    self.duty_face_direction(sim, assets, owner, direction);
                    let actor = self.ai_actor_mut(owner, "hide blinks");
                    for enemy in
                        &mut actor.detectable_lists[crate::element::DetectableType::Enemy as usize]
                    {
                        enemy.seen_now = false;
                        enemy.seen_last_frame = false;
                    }
                    let frames = crate::parameters_ai::AI_MIN_PANIC_HIDING_TIME as u32
                        + crate::sim_rng::u32(
                            sim,
                            crate::sim_rng::RngSite::AiPanic,
                            0..crate::parameters_ai::AI_DELTA_PANIC_HIDING_TIME as u32,
                        );
                    self.world
                        .entities
                        .expect_ai_controller_mut(owner, format_args!("hide timer"))
                        .launch_timer(frames, self.control.frame_counter);
                }
            }
            Substate::FleeingHiding => {
                if event == StimulusType::EventTimer {
                    self.execute_ai_return_to_duty(
                        sim,
                        assets,
                        owner,
                        crate::ai::DutyFlags::empty(),
                    );
                }
            }
            _ => return None,
        }
        Some(false)
    }

    pub(in crate::engine) fn execute_ai_panic_segment(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        stimulus: StimulusType,
    ) {
        assert!(matches!(
            stimulus,
            StimulusType::EventReachPoint | StimulusType::EventCouldntReachPoint
        ));
        let runs = self.ai(owner, "panic runs").lasting_panic_runs;
        if runs == 0 {
            self.duty_set_state(
                sim,
                assets,
                owner,
                AiState::Fleeing,
                Substate::FleeingHiding,
            );
            let ai = self.ai(owner, "panic facing");
            let direction = if ai.directed_panic {
                let position = self.live_ai_position(owner);
                crate::position_interface::vector_to_sector_0_to_15_iso(
                    ai.panic_center_x - position.x,
                    ai.panic_center_y - position.y,
                ) as u16
            } else {
                crate::sim_rng::u32(sim, crate::sim_rng::RngSite::AiPanic, 0..16) as u16
            };
            self.duty_face_direction(sim, assets, owner, direction);
            let ai = self.ai_mut(owner, "panic hiding");
            ai.clear_emoticon();
            self.execute_ai_set_alert_status(
                assets,
                owner,
                AlertLevel::Yellow,
                crate::ai::AlertFlags::empty(),
            );
            let npc = self.ai_actor_mut(owner, "panic blink");
            for detectable in
                &mut npc.detectable_lists[crate::element::DetectableType::Enemy as usize]
            {
                detectable.seen_now = false;
                detectable.seen_last_frame = false;
            }
            let frames = crate::parameters_ai::AI_MIN_PANIC_HIDING_TIME as u32
                + crate::sim_rng::u32(
                    sim,
                    crate::sim_rng::RngSite::AiPanic,
                    0..crate::parameters_ai::AI_DELTA_PANIC_HIDING_TIME as u32,
                );
            let frame = self.control.frame_counter;
            self.ai_mut(owner, "panic hiding timer")
                .launch_timer(frames, frame);
            return;
        }
        if stimulus == StimulusType::EventCouldntReachPoint {
            self.ai_mut(owner, "panic retry").first_try = false;
            self.execute_ai_panic_fallback(sim, assets, owner);
            return;
        }
        self.ai_mut(owner, "panic segment count").lasting_panic_runs = runs.wrapping_sub(1);
        let ai = self.ai(owner, "panic direction");
        let sector = if !ai.directed_panic {
            crate::sim_rng::u32(sim, crate::sim_rng::RngSite::AiPanic, 0..16) as u8
        } else {
            let position = self.live_ai_position(owner);
            let base = crate::position_interface::vector_to_sector_0_to_15(
                position.x - ai.panic_center_x,
                position.y - ai.panic_center_y,
            ) as u8;
            let (side, count, offset) = if ai.first_try {
                (0, 5, 2)
            } else {
                (
                    panic_retry_side(self.world.original_creation_order(owner)),
                    7,
                    3,
                )
            };
            let jitter = crate::sim_rng::u32(sim, crate::sim_rng::RngSite::AiPanic, 0..count) as u8;
            base.wrapping_add(side)
                .wrapping_add(jitter)
                .wrapping_sub(offset)
                & 15
        };
        let (vx, vy) = crate::element::direction_vector_16(sector as i16);
        let distance = (crate::parameters_ai::AI_MIN_PANIC_RUN_SEGMENT_DISTANCE as u32
            + crate::sim_rng::u32(
                sim,
                crate::sim_rng::RngSite::AiPanic,
                0..crate::parameters_ai::AI_DELTA_PANIC_RUN_SEGMENT_DISTANCE as u32,
            )) as f32;
        self.ai_mut(owner, "panic first try").first_try = true;
        let position = self.live_ai_position(owner);
        let destination = Position {
            x: position.x + vx * distance,
            y: position.y + vy * distance,
            ..position
        };
        let mut flags = GotoFlags::RUN | GotoFlags::STRAIGHT | GotoFlags::ASK_OBSTACLE;
        if self.ai(owner, "panic movement flags").lasting_panic_runs > 0 {
            flags |= GotoFlags::DONT_STOP;
        }
        self.duty_go_to(sim, assets, owner, destination, flags);
    }

    fn execute_ai_panic_fallback(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let position = self.live_ai_position(owner);
        let sector = super::ai_view_position_sector(
            self,
            self.expect_entity(owner, "panic fallback sector")
                .element_data(),
        );
        let anchor = self
            .ai(owner, "panic fallback owner")
            .nearest_seek_point_to_flee(&self.ai.global.seek_points, position, sector);
        if let Some(index) = anchor {
            let destination = self.ai.global.seek_points[index].position;
            let mut flags = GotoFlags::RUN;
            if self.ai(owner, "panic fallback runs").lasting_panic_runs > 0 {
                flags |= GotoFlags::DONT_STOP;
            }
            self.duty_go_to(sim, assets, owner, destination, flags);
        } else {
            self.execute_ai_callback(
                sim,
                assets,
                owner,
                &Stimulus::new(StimulusType::EventReachPoint),
            );
        }
        let ai = self.ai_mut(owner, "panic fallback result");
        if ai.couldnt_reachpoint {
            ai.couldnt_reachpoint = false;
            ai.lasting_panic_runs = ai.lasting_panic_runs.wrapping_sub(1);
            self.execute_ai_callback(
                sim,
                assets,
                owner,
                &Stimulus::new(StimulusType::EventReachPoint),
            );
        }
    }
}
