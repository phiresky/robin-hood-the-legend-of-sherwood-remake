use super::*;
use crate::ai::{AiEntityHandle, AiState, Position, ReportType, Stimulus, StimulusType, Substate};
use crate::coordinates::WorldPoint3D;
use crate::element::{ActionState, Command, Posture};
use crate::profiles::ProfileRank;

fn fixture(substate: Substate) -> (EngineInner, LevelAssets, EntityId, EntityId) {
    let (mut engine, mut assets, owner, target) =
        super::battle_decision_observation_tests::fixture(false);
    let ai = engine.enemy_mut(owner);
    ai.base.current_state = substate.ai_state_family().unwrap();
    ai.base.current_substate = substate;
    ai.base.primary_target = None;
    ai.list_them.clear();
    crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
        profile.rank = ProfileRank::Soldier
    });
    ai.current_task_priority = crate::ai_enemy::task_priority::NONE;
    ai.new_task_priority = crate::ai_enemy::task_priority::ALERT;
    (engine, assets, owner, target)
}

fn soldier(engine: &mut EngineInner, assets: &mut LevelAssets, substate: Substate) -> EntityId {
    let id = engine.add_test_entity(crate::engine::test_support::actors::make_test_ai_soldier(
        Camp::Lacklandists,
    ));
    let sector = engine.world.fast_grid.level.sectors[0].sector_number;
    engine.place(id, WorldPoint3D::new(140.0, 100.0, 0.0));
    engine.elem_mut(id).set_sector_topology(
        crate::position_interface::SectorHandle::new(sector.get() as u16),
        Some(crate::fast_find_grid::SectorIndex::new(0).unwrap()),
    );
    crate::engine::complete_test_runtime_fixture(engine, assets);
    let ai = engine.enemy_mut(id);
    ai.base.current_state = substate.ai_state_family().unwrap();
    ai.base.current_substate = substate;
    crate::engine::test_support::actors::edit_enemy_profile(assets, ai, |profile| {
        profile.rank = ProfileRank::Soldier
    });
    id
}

fn event(engine: &mut EngineInner, assets: &LevelAssets, owner: EntityId, kind: StimulusType) {
    engine.execute_ai_handler_body(
        &crate::sim_rng::test_context(),
        assets,
        owner,
        &Stimulus::new(kind),
    );
}

#[test]
fn repeated_view_during_battle_preserves_first_enemy_insertion_order() {
    for substate in [
        Substate::AttackingReactiontimeTurning,
        Substate::AttackingReactiontime,
        Substate::AttackingReactiontimeRunning,
        Substate::AttackingOverviewLookLeft,
        Substate::AttackingOverviewLookRight,
        Substate::AttackingTooProudToAttackOverview,
    ] {
        let (mut engine, assets, owner, first) = fixture(substate);
        let second = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
            Posture::Upright,
        ));
        for target in [first, first, second, first, second] {
            let mut stimulus = Stimulus::new(StimulusType::EventView);
            stimulus.info = crate::ai::StimulusInfo::Human(AiEntityHandle::new(target.index()));
            engine.execute_ai_handler_body(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &stimulus,
            );
        }
        let ai = engine.enemy(owner);
        assert_eq!(
            ai.list_them,
            vec![first.index(), second.index()],
            "{substate:?}"
        );
        assert_eq!(ai.base.current_substate, substate);
    }
}

#[test]
fn misses_charly_notification_does_not_start_a_search() {
    for substate in [Substate::DefaultOnPost, Substate::DefaultLookingForCharly] {
        let (mut engine, assets, owner, _) = fixture(substate);
        let handled = engine.execute_ai_handler_body(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::new(StimulusType::EventMissesCharly),
        );
        assert!(!handled);
        let ai = engine.enemy(owner);
        assert_eq!(ai.base.current_substate, substate);
        assert!(!ai.seeking_charly);
        assert!(commands(&engine, owner).is_empty());
    }
}

fn commands(engine: &EngineInner, owner: EntityId) -> Vec<Command> {
    engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|s| s.elements.iter())
        .filter(|e| e.owner == Some(owner))
        .map(|e| e.command)
        .collect()
}

fn civilian(engine: &mut EngineInner) -> EntityId {
    let mut entity = crate::engine::test_support::actors::make_test_civilian(Posture::Upright);
    entity.npc_data_mut().unwrap().ai_brain = crate::element::AiBrain::Friendly(Box::default());
    entity.element_data_mut().active = true;
    entity
        .element_data_mut()
        .set_position(WorldPoint3D::new(200.0, 100.0, 0.0));
    entity.element_data_mut().set_sector_topology(
        crate::position_interface::SectorHandle::new(1),
        Some(crate::fast_find_grid::SectorIndex::new(0).unwrap()),
    );
    engine.add_test_entity(entity)
}

#[test]
fn officer_wait_retains_each_search_family_and_prunes_taken_nets() {
    for state in [
        Substate::SeekingSeekpoint,
        Substate::SeekingSeekpointWatching,
        Substate::SeekingSeekpointWatchingSidewards,
        Substate::SeekingSeekpointPassedAmbushPointLeft,
        Substate::SeekingSeekpointPassedAmbushPointRight,
        Substate::SeekingSeekpointCheckingAmbushPoint,
        Substate::SeekingSeekpointApproachingBeggar,
        Substate::SeekingSeekpointIdentifyingBeggar1,
        Substate::SeekingSeekpointIdentifyingBeggar2,
        Substate::SeekingNet,
        Substate::SeekingTakingNet,
    ] {
        let (mut engine, mut assets, owner, _) =
            fixture(Substate::SeekingOfficerWaitForInstructedGroup);
        let member = soldier(&mut engine, &mut assets, state);
        engine.control.frame_counter = 7915;
        engine.enemy_mut(owner).alerted_us = vec![member.index()];
        event(&mut engine, &assets, owner, StimulusType::EventTimer);
        let ai = engine.enemy(owner);
        if state == Substate::SeekingTakingNet {
            assert!(ai.alerted_us.is_empty());
            assert_ne!(
                ai.base.current_substate,
                Substate::SeekingOfficerWaitForInstructedGroup
            );
        } else {
            assert_eq!(ai.alerted_us, [member.index()], "{state:?}");
            assert_eq!(
                ai.base.current_substate,
                Substate::SeekingOfficerWaitForInstructedGroup
            );
            assert!(ai.base.timer_is_running);
            assert_eq!(ai.base.when_does_timer_ring, 7945);
        }
    }
}

#[test]
fn officer_waits_for_approaching_charly_but_finishes_when_search_ends() {
    for (state, wait) in [
        (Substate::SeekingCharlySentToOfficer, true),
        (Substate::SeekingCharlyGoToOfficer, true),
        (Substate::DefaultOnPost, false),
    ] {
        let (mut engine, mut assets, owner, _) =
            fixture(Substate::SeekingOfficerWaitForInstructedGroup);
        let charly = soldier(&mut engine, &mut assets, state);
        let ai = engine.enemy_mut(owner);
        ai.base.my_reconnaissance_report.report_type = ReportType::MissedCharly;
        ai.base.my_reconnaissance_report.charly = Some(AiEntityHandle::new(charly.index()));
        event(&mut engine, &assets, owner, StimulusType::EventTimer);
        let ai = engine.enemy(owner);
        assert_eq!(
            ai.base.current_substate == Substate::SeekingOfficerWaitForInstructedGroup,
            wait
        );
        if wait {
            assert_eq!(ai.base.when_does_timer_ring, 130);
        }
    }
}

#[test]
fn officer_missing_unconscious_reporter_increments_without_rearming_timer() {
    let (mut engine, mut assets, owner, _) =
        fixture(Substate::SeekingOfficerWaitForInstructedSoldier);
    let member = soldier(&mut engine, &mut assets, Substate::SeekingSeekpoint);
    engine.human_mut(member).unconscious = true;
    let ai = engine.enemy_mut(owner);
    ai.base.antagonist = Some(AiEntityHandle::new(member.index()));
    ai.missed_soldier_timer = 11;
    ai.base.timer_is_running = false;
    ai.base.when_does_timer_ring = 0;
    event(&mut engine, &assets, owner, StimulusType::EventTimer);
    let ai = engine.enemy(owner);
    assert_eq!(ai.missed_soldier_timer, 12);
    assert!(!ai.base.timer_is_running);
    assert_eq!(ai.base.when_does_timer_ring, 0);
}

#[test]
fn goto_post_registers_facing_before_leaving_attentive_mode() {
    let (mut engine, assets, owner, _) = fixture(Substate::DefaultGotoPost);
    let ai = engine.enemy_mut(owner);
    ai.base.initial_view_direction = 4;
    ai.attentive = true;
    ai.will_be_attentive = true;
    event(&mut engine, &assets, owner, StimulusType::EventReachPoint);
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .enemy_ai()
            .unwrap()
            .base
            .current_substate,
        Substate::DefaultGotoPostTurn
    );
    let commands = commands(&engine, owner);
    let turn = commands.iter().position(|c| *c == Command::Turn).unwrap();
    let leave = commands
        .iter()
        .position(|c| *c == Command::LeaveAttentiveMode)
        .unwrap();
    assert!(turn < leave);
}

#[test]
fn unrelated_arrow_and_combat_alert_events_do_not_start_search() {
    for (state, stimulus) in [
        (Substate::SeekingArrowJustWatching, StimulusType::EventDone),
        (Substate::SeekingCombatAlert, StimulusType::EventTimer),
    ] {
        let (mut engine, assets, owner, _) = fixture(state);
        event(&mut engine, &assets, owner, stimulus);
        let ai = engine.enemy(owner);
        assert_eq!(ai.base.current_substate, state);
        assert!(!ai.base.timer_is_running);
        assert!(commands(&engine, owner).is_empty());
    }
}

#[test]
fn normal_parry_timer_stops_only_normal_parry_and_rearms_twenty_frames() {
    for (action, stop) in [
        (ActionState::WaitingSword, false),
        (ActionState::ParryingSword, true),
        (ActionState::ParryingSwordLow, false),
    ] {
        let (mut engine, assets, owner, _) = fixture(Substate::AttackingSwordfightParade);
        engine.set_action_state_of(owner, action);
        event(&mut engine, &assets, owner, StimulusType::EventTimer);
        assert_eq!(
            commands(&engine, owner).contains(&Command::StopParrySword),
            stop
        );
        let ai = engine.enemy(owner);
        assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
        assert_eq!(ai.base.when_does_timer_ring, 120);
    }
}

#[test]
fn go_to_officer_call_accepts_default_and_rejects_combat() {
    for (state, accepted) in [
        (Substate::DefaultOnPost, true),
        (Substate::AttackingSwordfight, false),
    ] {
        let (mut engine, mut assets, owner, _) = fixture(state);
        let caller = soldier(&mut engine, &mut assets, Substate::DefaultOnPost);
        assert_eq!(
            engine.execute_ai_enemy_event(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                &Stimulus::with_human(StimulusType::CallGoToOfficer, caller.index())
            ),
            accepted
        );
        let ai = engine.enemy(owner);
        if accepted {
            assert_eq!(
                ai.base.current_substate,
                Substate::SeekingCharlySentToOfficer
            );
            assert_eq!(
                ai.base.antagonist,
                Some(AiEntityHandle::new(caller.index()))
            );
            assert!(ai.reported_to_officer);
        } else {
            assert_eq!(ai.base.current_substate, state);
        }
    }
}

#[test]
fn call_alert_keeps_running_macro_for_each_caller_role() {
    for role in 0..3 {
        let (mut engine, mut assets, owner, _) = fixture(Substate::DefaultInMacro);
        let caller = if role == 1 {
            let id = civilian(&mut engine);
            crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
            id
        } else {
            soldier(&mut engine, &mut assets, Substate::DefaultOnPost)
        };
        if role == 0 {
            crate::engine::test_support::actors::edit_enemy_profile(
                &mut assets,
                engine.enemy_mut(caller),
                |profile| profile.rank = ProfileRank::Officer,
            );
        }
        let ai = engine.enemy_mut(owner);
        if role == 2 {
            crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
                profile.rank = ProfileRank::Officer
            });
        }
        ai.base.macro_in_progress = true;
        ai.base.macro_timer_is_running = true;
        ai.base.when_does_macro_timer_ring = 10054;
        assert!(engine.execute_ai_enemy_event(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::with_human(StimulusType::CallAlert, caller.index())
        ));
        let ai = engine.enemy(owner);
        assert!(ai.base.macro_in_progress);
        assert!(ai.base.macro_timer_is_running);
        assert_eq!(ai.base.when_does_macro_timer_ring, 10054);
        assert_eq!(
            ai.base.antagonist,
            Some(AiEntityHandle::new(caller.index()))
        );
        assert_eq!(
            ai.base.current_substate,
            [
                Substate::SeekingGroupCalledByOfficer,
                Substate::SeekingWaitForAlertingCivilian,
                Substate::SeekingOfficerWaitForAlertingSoldier
            ][role]
        );
    }
}

#[test]
fn rejected_civilian_alert_still_replaces_antagonist() {
    let (mut engine, mut assets, owner, _) = fixture(Substate::SeekingRunningToOfficer);
    let caller = civilian(&mut engine);
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    assert!(!engine.execute_ai_enemy_event(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        &Stimulus::with_human(StimulusType::CallAlert, caller.index())
    ));
    let ai = engine.enemy(owner);
    assert_eq!(
        ai.base.antagonist,
        Some(AiEntityHandle::new(caller.index()))
    );
    assert_eq!(ai.base.current_substate, Substate::SeekingRunningToOfficer);
}

#[test]
fn group_arrival_compares_raw_direction_before_registering_a_turn() {
    for (action, direction, gather, turn) in [
        (ActionState::Moving, 8, 8, true),
        (ActionState::Waiting, 4, 4, false),
        (ActionState::Waiting, 13, 29, true),
    ] {
        let (mut engine, assets, owner, _) = fixture(Substate::SeekingGroupGoToOfficer);
        let entity = engine.ent_mut(owner);
        entity.actor_data_mut().unwrap().action_state = action;
        entity
            .position_iface_mut()
            .set_direction_instantly(crate::position_interface::Direction::from_raw(direction));
        let ai = entity.enemy_ai_mut().unwrap();
        ai.gather_position_instructed = true;
        ai.gather_direction = gather;
        engine.execute_ai_officer_rendezvous_event(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::new(StimulusType::EventReachPoint),
        );
        let elements: Vec<_> = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|s| &s.elements)
            .filter(|e| e.owner == Some(owner) && e.command == Command::Turn)
            .collect();
        assert_eq!(!elements.is_empty(), turn);
        if turn {
            assert!(
                matches!(elements[0].get_property(crate::sequence::Field::Direction), Some(crate::sequence::FieldValue::Integer(d)) if *d == u32::from(gather))
            );
        } else {
            assert!(
                engine
                    .get_entity(owner)
                    .unwrap()
                    .enemy_ai()
                    .unwrap()
                    .base
                    .already_turned
            );
        }
    }
}

#[test]
fn apple_interrupt_rule_uses_actual_combat_relationship() {
    for interrupt in [false, true] {
        let (mut engine, assets, owner, target) = fixture(Substate::AttackingSwordfight);
        engine.human_mut(owner).opponents.push(target);
        engine.human_mut(target).opponents.push(owner);
        let mut config = crate::engine::SimConfig {
            item_gameplay: crate::gameplay_config::ItemGameplayConfig::classic(),
            ..Default::default()
        };
        config.item_gameplay.apple_combat_interrupt = interrupt;
        let sim = crate::sim_rng::SimulationContext::with_seed_and_config(7, config);
        engine.execute_ai_enemy_event(
            &sim,
            &assets,
            owner,
            &Stimulus::with_position(StimulusType::EventApple, Position::default()),
        );
        let ai = engine.enemy(owner);
        assert_eq!(
            ai.base.current_substate,
            if interrupt {
                Substate::WonderingAppleSauceInTheVisor
            } else {
                Substate::AttackingSwordfight
            }
        );
        if interrupt {
            assert_eq!(ai.base.when_does_timer_ring, 160);
            assert!(commands(&engine, owner).contains(&Command::QuitSwordfight));
        } else {
            assert!(
                engine
                    .get_entity(owner)
                    .unwrap()
                    .human_data()
                    .unwrap()
                    .opponents
                    .contains(&target)
            );
        }
    }
}

#[test]
fn engaged_hit_ignores_existing_opponent_and_friendly_attacker() {
    for friend in [false, true] {
        let (mut engine, mut assets, owner, target) = fixture(Substate::AttackingSwordfight);
        engine.human_mut(owner).opponents.push(target);
        let attacker = if friend {
            soldier(&mut engine, &mut assets, Substate::DefaultOnPost)
        } else {
            target
        };
        engine.execute_ai_combat_impact_event(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::with_human(StimulusType::EventGotHit, attacker.index()),
        );
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .human_data()
                .unwrap()
                .opponents,
            vec![target]
        );
        assert!(!commands(&engine, owner).contains(&Command::EnterSwordfight));
    }
}

#[test]
fn menacing_hit_sets_direction_without_a_turn_sequence() {
    let (mut engine, assets, owner, target) = fixture(Substate::MenacingPcInComa);
    let here = engine.live_ai_position(owner);
    let there = engine.live_ai_position(target);
    let direction =
        crate::position_interface::vector_to_sector_0_to_15_iso(there.x - here.x, there.y - here.y);
    engine.execute_ai_combat_impact_event(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        &Stimulus::with_human(StimulusType::EventGotHit, target.index()),
    );
    let entity = engine.ent(owner);
    assert_eq!(
        entity.enemy_ai().unwrap().base.current_substate,
        Substate::AttackingReturnToOtherPcAfterMenacing
    );
    assert_eq!(
        entity.enemy_ai().unwrap().base.primary_target,
        Some(AiEntityHandle::new(target.index()))
    );
    assert_eq!(
        i16::from(entity.position_iface().get_direction_goal()),
        direction
    );
    assert!(!commands(&engine, owner).contains(&Command::Turn));
    assert!(commands(&engine, owner).contains(&Command::EnterSwordfight));
}

#[test]
fn body_arrival_uses_live_distance_and_does_not_turn_toward_a_nearby_corpse() {
    for (distance, dead) in [(20.0, true), (200.0, true), (20.0, false)] {
        let (mut engine, assets, owner, body) = fixture(Substate::SeekingBody);
        let here = engine.pos_of(owner);
        let entity = engine.ent_mut(body);
        entity.element_data_mut().set_position(WorldPoint3D::new(
            here.x + distance,
            here.y,
            here.z,
        ));
        entity.position_iface_mut().set_posture(if dead {
            Posture::Dead
        } else {
            Posture::Upright
        });
        if let Entity::Pc(pc) = entity {
            pc.pc.life_points = if dead { 0 } else { 100 };
        }
        let body_position = engine.live_ai_position(body);
        let ai = engine.enemy_mut(owner);
        ai.base.detected_body = Some(AiEntityHandle::new(body.index()));
        ai.base.seek_position = body_position;
        event(&mut engine, &assets, owner, StimulusType::EventReachPoint);
        let ai = engine.enemy(owner);
        if dead && distance < 60.0 {
            assert_eq!(
                ai.base.current_substate,
                Substate::SeekingBodyLookingDeadBody
            );
            assert!(ai.already_seen_bodies.contains(&body.index()));
            assert!(!commands(&engine, owner).contains(&Command::Turn));
        } else {
            assert_ne!(
                ai.base.current_substate,
                Substate::SeekingBodyLookingDeadBody
            );
        }
        if !dead {
            assert_ne!(ai.base.current_state, AiState::Seeking);
        }
        if dead && distance > 60.0 {
            assert!(
                engine
                    .get_entity(owner)
                    .unwrap()
                    .npc_data()
                    .unwrap()
                    .detectable_lists[crate::element::DetectableType::Body as usize]
                    .iter()
                    .any(|d| d.element == Some(body))
            );
        }
    }
}

#[test]
fn arrow_timer_launches_plain_running_movement_without_near_tolerance() {
    let (mut engine, assets, owner, target) = fixture(Substate::SeekingArrowReactiontime);
    let destination = engine.live_ai_position(target);
    engine.enemy_mut(owner).base.seek_position = destination;
    event(&mut engine, &assets, owner, StimulusType::EventTimer);
    let ai = engine.enemy(owner);
    assert_eq!(ai.base.current_substate, Substate::SeekingArrow);
    assert_eq!(ai.base.last_goto_flags, crate::ai::GotoFlags::RUN);
    assert_eq!(ai.base.last_goto_destination, destination);
    assert!(!ai.base.stop_before_end_of_path);
}

#[test]
fn bow_transition_states_ignore_coordinate_and_preserve_timer() {
    for state in [
        Substate::AttackingBowObservingLoading,
        Substate::AttackingBowLoading,
        Substate::AttackingBowAiming,
    ] {
        let (mut engine, assets, owner, _) = fixture(state);
        let ai = engine.enemy_mut(owner);
        ai.base.timer_is_running = true;
        ai.base.when_does_timer_ring = 777;
        event(&mut engine, &assets, owner, StimulusType::CallCoordinate);
        let ai = engine.enemy(owner);
        assert_eq!(ai.base.current_substate, state);
        assert!(ai.base.timer_is_running);
        assert_eq!(ai.base.when_does_timer_ring, 777);
        assert!(commands(&engine, owner).is_empty());
    }
}

#[test]
#[should_panic(expected = "archery path sector")]
fn shooting_path_arrival_requires_a_real_archery_sector() {
    let (mut engine, assets, owner, _) = fixture(Substate::AttackingArcherRunOnShootingPath);
    event(&mut engine, &assets, owner, StimulusType::EventReachPoint);
}

#[test]
#[should_panic(expected = "archery path reserved point")]
fn shooting_path_final_sprint_requires_a_reserved_point() {
    let (mut engine, assets, owner, _) =
        fixture(Substate::AttackingArcherRunOnShootingPathFinalSprint);
    event(&mut engine, &assets, owner, StimulusType::EventReachPoint);
}

#[test]
fn civilian_beggar_sighting_scrubs_every_observer_without_requeuing_current_beggar() {
    for already_examining in [false, true] {
        let (mut engine, mut assets, owner, _) = fixture(Substate::SeekingSeekpoint);
        let observer = soldier(&mut engine, &mut assets, Substate::SeekingSeekpoint);
        let beggar = civilian(&mut engine);
        let position = engine.live_ai_position(owner);
        let entity = engine.ent_mut(beggar);
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(position.x, position.y, 0.0));
        entity.element_data_mut().set_sector_topology(
            position.sector,
            position.sector.and_then(|s| s.arena_index()),
        );
        for id in [owner, observer] {
            engine.npc_mut(id).detectable_lists[crate::element::DetectableType::Beggar as usize]
                .push(crate::element::Detectable {
                    element: Some(beggar),
                    detectable_type: crate::element::DetectableType::Beggar,
                    ..Default::default()
                });
        }
        if already_examining {
            engine.enemy_mut(owner).beggar_to_examine = Some(AiEntityHandle::new(beggar.index()));
        }
        engine.execute_ai_enemy_event(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::with_human(StimulusType::EventSeesBeggar, beggar.index()),
        );
        let ai = engine.enemy(owner);
        assert_eq!(
            ai.beggars_to_control,
            if already_examining {
                vec![]
            } else {
                vec![beggar.index()]
            }
        );
        assert_eq!(
            ai.positions_of_beggars_to_control.len(),
            usize::from(!already_examining)
        );
        for id in [owner, observer] {
            assert!(
                engine
                    .get_entity(id)
                    .unwrap()
                    .npc_data()
                    .unwrap()
                    .detectable_lists[crate::element::DetectableType::Beggar as usize]
                    .iter()
                    .all(|d| d.element != Some(beggar))
            );
        }
    }
}

#[test]
fn ale_eligibility_requires_outdoor_beer_preference_or_enabled_reliable_rule() {
    for (beer, reliable, indoors, take) in [
        (0, false, false, false),
        (0, true, false, true),
        (0, true, true, false),
        (35, false, false, true),
    ] {
        let (mut engine, mut assets, owner, _) = fixture(Substate::WonderingAleReactiontime);
        if indoors {
            engine.world.fast_grid_mut().level_mut().sectors[0].sector_type |=
                crate::sector::SectorType::BUILDING;
        }
        let position = engine.live_ai_position(owner);
        let mut element = crate::element::ElementData::default();
        element.kind = crate::element::ElementKind::ObjectOther;
        element.active = true;
        element.set_position(WorldPoint3D::new(position.x + 200.0, position.y, 0.0));
        element.set_sector_topology(
            position.sector,
            position.sector.and_then(|s| s.arena_index()),
        );
        let bottle = engine.add_test_entity(Entity::Bonus(crate::element::ElementBonus {
            element,
            object: crate::element::ObjectData {
                object_type: crate::element_kinds::ObjectType::Ale,
                ..Default::default()
            },
        }));
        let ai = engine.enemy_mut(owner);
        crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
            profile.beer = beer
        });
        ai.base.interesting_object = Some(AiEntityHandle::new(bottle.index()));
        engine
            .control
            .sim_config
            .item_gameplay
            .ale_reliable_distraction = reliable;
        let sim =
            crate::sim_rng::SimulationContext::with_seed_and_config(1, engine.control.sim_config);
        engine.execute_ai_ale_reaction(&sim, &assets, owner);
        assert_eq!(
            engine
                .get_entity(owner)
                .unwrap()
                .enemy_ai()
                .unwrap()
                .base
                .object_of_desire
                == Some(AiEntityHandle::new(bottle.index())),
            take
        );
    }
}

#[test]
fn instructed_soldier_reads_officers_selected_body_on_speech_completion() {
    check_instructed_soldier_body_registration(false);
}

#[test]
fn instructed_soldier_retains_existing_body_registration_on_speech_completion() {
    check_instructed_soldier_body_registration(true);
}

fn check_instructed_soldier_body_registration(already_registered: bool) {
    let (mut engine, mut assets, owner, body) =
        fixture(Substate::SeekingSoldierGetInstructedByOfficer);
    let officer = soldier(
        &mut engine,
        &mut assets,
        Substate::SeekingOfficerWaitForInstructedSoldier,
    );
    let position = engine.live_ai_position(officer);
    let ai = engine.enemy_mut(officer);
    ai.base.detected_body = Some(AiEntityHandle::new(body.index()));
    ai.base.alert_soldiers_point = position;
    ai.base.antagonist = Some(AiEntityHandle::new(owner.index()));
    engine.enemy_mut(owner).base.antagonist = Some(AiEntityHandle::new(officer.index()));
    if already_registered {
        engine.execute_ai_add_detectable(owner, body, crate::element::DetectableType::Body);
    }
    event(&mut engine, &assets, owner, StimulusType::EventMyTalk2);
    let ai = engine.enemy(owner);
    assert_eq!(ai.base.alert_soldiers_point, position);
    assert_eq!(ai.officers_position, position);
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .npc_data()
            .unwrap()
            .detectable_lists[crate::element::DetectableType::Body as usize]
            .iter()
            .filter(|d| d.element == Some(body))
            .count(),
        1
    );
}

#[test]
fn hiding_timer_returns_to_duty_inline() {
    let (mut engine, assets, owner, _) = fixture(Substate::FleeingHiding);
    event(&mut engine, &assets, owner, StimulusType::EventTimer);
    assert_ne!(
        engine
            .get_entity(owner)
            .unwrap()
            .enemy_ai()
            .unwrap()
            .base
            .current_state,
        AiState::Fleeing
    );
}

#[test]
fn unengaged_hit_uses_live_relationship_instead_of_stale_swordfight_state() {
    for state in [Substate::DefaultOnPost, Substate::AttackingSwordfight] {
        let (mut engine, mut assets, owner, target) = fixture(state);
        for weapon in &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons {
            weapon.distance[crate::weapons::WeaponDistance::Uber as usize] = 70;
        }
        let here = engine.pos_of(owner);
        engine.place(target, WorldPoint3D::new(here.x + 10.0, here.y, here.z));
        engine.execute_ai_combat_impact_event(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::with_human(StimulusType::EventGotHit, target.index()),
        );
        let entity = engine.ent(owner);
        assert_eq!(
            entity.enemy_ai().unwrap().base.primary_target,
            Some(AiEntityHandle::new(target.index()))
        );
        assert_eq!(
            entity.enemy_ai().unwrap().base.current_substate,
            Substate::AttackingSwordfight
        );
        assert!(
            entity.human_data().unwrap().opponents.is_empty(),
            "{state:?}"
        );
        let entry = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| &sequence.elements)
            .find(|element| {
                element.owner == Some(owner) && element.command == Command::EnterSwordfight
            })
            .expect("unengaged hit registers swordfight entry for sequence execution");
        assert!(
            matches!(entry.get_property(crate::sequence::Field::Opponent), Some(crate::sequence::FieldValue::Element(opponent)) if *opponent == target),
            "{state:?}"
        );
    }
}

#[test]
fn engaged_hit_adds_a_new_hostile_attacker_inline() {
    let (mut engine, mut assets, owner, target) = fixture(Substate::AttackingSwordfight);
    let previous = soldier(&mut engine, &mut assets, Substate::AttackingSwordfight);
    for weapon in &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons {
        weapon.distance[crate::weapons::WeaponDistance::Uber as usize] = 70;
    }
    engine.human_mut(owner).opponents.push(previous);
    let here = engine.pos_of(owner);
    engine.place(target, WorldPoint3D::new(here.x + 10.0, here.y, here.z));
    engine.execute_ai_combat_impact_event(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        &Stimulus::with_human(StimulusType::EventGotHit, target.index()),
    );
    assert!(
        engine
            .get_entity(owner)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents
            .contains(&target)
    );
}
