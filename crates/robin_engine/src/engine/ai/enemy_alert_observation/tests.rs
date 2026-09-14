use super::*;
use crate::ai::{
    AiState, EnemyObservation, Hint, Noise, NoiseOrigin, NoiseType, Position, ReportType, Substate,
};
use crate::coordinates::{MapPoint, WorldPoint3D};
use crate::profiles::ProfileRank;

fn fixture(state: AiState, substate: Substate) -> (EngineInner, LevelAssets, EntityId, EntityId) {
    let (mut engine, assets, owner, target) =
        crate::engine::ai::battle_decision_observation_tests::fixture(false);
    let ai = engine
        .world
        .entities
        .expect_enemy_ai_mut(owner, format_args!("hearing fixture"));
    ai.base.current_state = state;
    ai.base.current_substate = substate;
    ai.base.primary_target = None;
    ai.current_task_priority = crate::ai_enemy::task_priority::NONE;
    ai.new_task_priority = crate::ai_enemy::task_priority::STRANGE_THING;
    ai.soldier_profile_rank = ProfileRank::Soldier;
    (engine, assets, owner, target)
}

fn noise(noise_type: NoiseType, position: Position) -> Noise {
    Noise {
        origin: NoiseOrigin::from_position(position),
        noise_type,
        volume: 200,
        elevation: 36,
        element_id: 0,
    }
}

fn turn_directions(engine: &EngineInner, owner: EntityId) -> Vec<u32> {
    engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|s| s.elements.iter())
        .filter(|e| e.owner == Some(owner))
        .filter_map(
            |e| match e.get_property(crate::sequence::Field::Direction) {
                Some(crate::sequence::FieldValue::Integer(direction)) => Some(*direction),
                _ => None,
            },
        )
        .collect()
}

#[test]
fn hearing_projects_origin_instead_of_using_recorded_noise_elevation() {
    let (mut engine, assets, owner, _) = fixture(AiState::Seeking, Substate::SeekingSeekpoint);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(
            f32::from_bits(0x4326_9901),
            f32::from_bits(0x43a1_5511),
            f32::from_bits(0x4210_0107),
        ));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(4);
    let position = Position {
        x: f32::from_bits(0x428b_1027),
        y: f32::from_bits(0x43af_c940),
        sector: engine.live_ai_position(owner).sector,
        level: 0,
    };
    engine.execute_ai_enemy_observation(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        EnemyObservation::Noise {
            noise: noise(NoiseType::ZingZing, position),
        },
    );
    assert!(
        turn_directions(&engine, owner).contains(&11),
        "ground projection selects11 while substituting recorded elevation selects10"
    );
    let ai = engine
        .world
        .entities
        .expect_enemy_ai(owner, format_args!("noise result"));
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingHeardstepsReactiontime
    );
    assert_eq!(ai.base.when_does_timer_ring, 101);
}

#[test]
fn zonk_keeps_absent_sector_and_layer_impact() {
    let (mut engine, assets, owner, _) = fixture(AiState::Default, Substate::DefaultOnPost);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(317.8, 1196.001, 480.00104));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(4);
    let position = Position {
        x: 341.81934,
        y: 716.62885,
        sector: None,
        level: u16::MAX,
    };
    let mut noise = noise(NoiseType::Zonk, position);
    noise.elevation = 480;
    engine.execute_ai_enemy_observation(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        EnemyObservation::Noise { noise },
    );
    let ai = engine
        .world
        .entities
        .expect_enemy_ai(owner, format_args!("impact result"));
    assert_eq!(ai.base.seek_position, position);
    assert_eq!(ai.base.current_substate, Substate::WonderingWatching);
    assert_eq!(ai.base.when_does_timer_ring, 150);
    assert!(turn_directions(&engine, owner).contains(&0));
}

#[test]
fn distraction_noise_records_impact_before_investigation() {
    let (mut engine, assets, owner, _) = fixture(AiState::Default, Substate::DefaultOnPost);
    let position = Position {
        x: 140.0,
        y: 90.0,
        ..engine.live_ai_position(owner)
    };
    engine.execute_ai_enemy_observation(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        EnemyObservation::Noise {
            noise: noise(NoiseType::Distraction, position),
        },
    );
    let ai = engine
        .world
        .entities
        .expect_enemy_ai(owner, format_args!("distraction result"));
    assert!(ai.investigating_distraction);
    assert_eq!(ai.base.seek_position, position);
    assert_eq!(
        ai.base.my_reconnaissance_report.report_type,
        ReportType::Noise
    );
    assert_eq!(ai.base.my_reconnaissance_report.seek_position, position);
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingHeardstepsPreReactiontime
    );
    assert!(ai.base.timer_is_running);
}

#[test]
fn logs_and_drawbridge_draw_cooldown_only_from_default_state() {
    for kind in [NoiseType::Logs, NoiseType::Drawbridge] {
        for state in [AiState::Default, AiState::Seeking, AiState::Attacking] {
            let (mut engine, assets, owner, _) = fixture(state, Substate::DefaultOnPost);
            let position = Position {
                x: 500.0,
                y: 500.0,
                ..engine.live_ai_position(owner)
            };
            let (_, draws) = crate::sim_rng::with_draw_trace(|| {
                engine.execute_ai_enemy_observation(
                    &crate::sim_rng::test_context(),
                    &assets,
                    owner,
                    EnemyObservation::Noise {
                        noise: noise(kind, position),
                    },
                )
            });
            assert_eq!(
                draws
                    .iter()
                    .filter(|site| **site == crate::sim_rng::RngSite::SoldierNoiseCooldown)
                    .count(),
                usize::from(state == AiState::Default)
            );
            let ai = engine
                .world
                .entities
                .expect_enemy_ai(owner, format_args!("noise cooldown"));
            if state == AiState::Default {
                assert_eq!(ai.base.current_substate, Substate::WonderingWatching);
                assert!((170..230).contains(&ai.base.when_does_timer_ring));
            } else {
                assert_eq!(ai.base.current_state, state);
            }
        }
    }
}

#[test]
fn look_there_and_combat_alert_keep_distinct_macro_behavior() {
    for combat in [false, true] {
        let (mut engine, assets, owner, _) = fixture(AiState::Default, Substate::DefaultOnPost);
        let position = Position {
            x: 500.0,
            y: 500.0,
            ..engine.live_ai_position(owner)
        };
        engine
            .world
            .entities
            .expect_ai_controller_mut(owner, format_args!("alert macro"))
            .macro_in_progress = true;
        let operation = if combat {
            EnemyObservation::CombatAlert { position }
        } else {
            EnemyObservation::LookThere { position }
        };
        engine.execute_ai_enemy_observation(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            operation,
        );
        let ai = engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("alert result"));
        assert_eq!(ai.base.seek_position, position);
        assert_eq!(ai.base.macro_in_progress, combat);
        assert_eq!(
            ai.base.current_substate,
            if combat {
                Substate::SeekingCombatAlertReactiontime
            } else {
                Substate::WonderingWatching
            }
        );
        if !combat {
            assert_eq!(ai.base.when_does_timer_ring, 200);
        }
    }
}

#[test]
fn tower_alert_selects_rank_branch_and_preserves_running_macro() {
    for rank in [
        ProfileRank::Soldier,
        ProfileRank::Officer,
        ProfileRank::Knight,
    ] {
        let (mut engine, assets, owner, caller) =
            fixture(AiState::Default, Substate::DefaultOnPost);
        let position = Position {
            x: 500.0,
            y: 500.0,
            ..engine.live_ai_position(owner)
        };
        let ai = engine
            .world
            .entities
            .expect_enemy_ai_mut(owner, format_args!("tower alert rank"));
        ai.soldier_profile_rank = rank;
        ai.base.macro_in_progress = true;
        engine.execute_ai_enemy_observation(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            EnemyObservation::TowerGuardAlert {
                hint: Hint {
                    seek_point: position,
                    who_tells_me: crate::ai::AiEntityHandle::new(caller.index()),
                    seek_flags: 0,
                },
            },
        );
        let ai = engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("tower alert result"));
        assert_eq!(
            ai.base.current_substate,
            if rank == ProfileRank::Knight {
                Substate::SeekingKnightWatchingTowerGuard
            } else {
                Substate::WonderingWatchingTowerGuard
            }
        );
        assert_eq!(ai.base.seek_position, position);
        assert_eq!(
            ai.base.my_reconnaissance_report.report_type,
            ReportType::Enemy
        );
        assert_eq!(ai.base.when_does_timer_ring, 200);
        assert!(ai.base.macro_in_progress);
    }
}

#[test]
fn tower_alert_faces_caller_moved_by_state_callback() {
    use crate::engine::test_support::asm::*;
    use crate::engine::types::MissionScript;
    use crate::natives::{NativeFn, ScriptHandleCodec};
    use crate::scb::{ClassEntry, Function, ScbFile};
    let (mut engine, mut assets, owner, caller) =
        fixture(AiState::Default, Substate::DefaultOnPost);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .script_class = "MoveCaller".into();
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(0);
    assets.scripts.location_count = 1;
    assets.scripts.point_count = 1;
    assets.scripts.location_positions = std::sync::Arc::new(vec![(100.0, 600.0)]);
    assets.scripts.location_layers = std::sync::Arc::new(vec![0]);
    assets.scripts.location_sectors = std::sync::Arc::new(vec![1]);
    assets.scripts.location_sector_handles =
        std::sync::Arc::new(vec![engine.live_ai_position(owner).sector]);
    engine.scripts.mission = Some(
        MissionScript::from_scb(ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![
                empty_startup_class("tower.scs".into()),
                ClassEntry {
                    source_file: "tower.scs".into(),
                    class_name: "MoveCaller".into(),
                    size_of_member_variables: 0,
                    member_variables: vec![],
                    functions: vec![Function {
                        name: "FilterAIEvent".into(),
                        address: 0,
                        num_parameters: 3,
                        size_of_return_value: 4,
                        size_of_parameters: 12,
                        size_of_volatile: 0,
                        size_of_temporary: 8,
                    }],
                    quads: vec![
                        q_begin_function(0, 2),
                        q_aff1_get_param(0xC000, 4),
                        q_aff0_iconstant(0xC004, AiState::Wondering.state_change_event_code()),
                        q_ieq(0xC000, 0xC000, 0xC004),
                        q_if_not_zero_goto(0xC000, 7),
                        q_aff0_iconstant(0xC000, 1),
                        q_return_val(0xC000),
                        q_aff0_iconstant(0xC000, ScriptHandleCodec::actor_handle(caller)),
                        q_aff0_iconstant(0xC004, ScriptHandleCodec::location_handle_from_index(0)),
                        q_native_param(0xC000),
                        q_native_param(0xC004),
                        q_native_call(NativeFn::SetActorLocation as u32),
                        q_aff0_iconstant(0xC000, 1),
                        q_return_val(0xC000),
                        q_end_function(),
                    ],
                },
            ],
        })
        .unwrap(),
    );
    engine.attach_script_bindings(&assets);
    engine
        .scripts
        .mission
        .as_mut()
        .unwrap()
        .bind_actor(ScriptHandleCodec::actor_handle(owner), "MoveCaller");
    let position = Position {
        x: 500.0,
        y: 500.0,
        ..engine.live_ai_position(owner)
    };
    engine.execute_ai_enemy_observation(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        EnemyObservation::TowerGuardAlert {
            hint: Hint {
                seek_point: position,
                who_tells_me: crate::ai::AiEntityHandle::new(caller.index()),
                seek_flags: 0,
            },
        },
    );
    assert_eq!(
        engine.live_ai_position(caller).map_point(),
        MapPoint::new(100.0, 600.0)
    );
    let expected = crate::position_interface::vector_to_sector_0_to_15_iso(0.0, 500.0) as u32;
    assert!(
        turn_directions(&engine, owner).contains(&expected),
        "Face(caller) reads its position after the state callback"
    );
    assert_eq!(
        engine
            .world
            .entities
            .expect_ai_controller(owner, format_args!("tower seek point"))
            .seek_position,
        position
    );
}
