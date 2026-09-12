use super::*;

const SPEECH_TIMING_PROFILE_ID: u32 = 0x1234_0000;

fn build_mytalk_timing_test() -> (EngineInner, EntityId, LevelAssets) {
    use crate::ai::{Remark, SpeechFlags};
    use crate::element::AiBrain;
    use crate::profiles::SoldierProfile;

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 100;

    let mut soldier_entity = make_test_soldier(crate::element::Posture::Upright);
    let Entity::Soldier(soldier) = &mut soldier_entity else {
        unreachable!();
    };
    soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
    soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("timing-test soldier has EnemyAi")
        .hth_weapon_id = 1;
    let ai = soldier.npc.ai_brain.base_mut().unwrap();
    ai.say_with_flags(Remark::Arrow, SpeechFlags::MYTALK_1 | SpeechFlags::ALWAYS);
    let soldier_id = engine.add_test_entity(soldier_entity);

    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .soldiers
        .push(SoldierProfile {
            profile_name: "timing-test-soldier".into(),
            exclamation_id: SPEECH_TIMING_PROFILE_ID,
            hth_weapon_id: 1,
            ..Default::default()
        });
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .hth_weapons
        .push(Default::default());
    (engine, soldier_id, assets)
}

fn mytalk_ai(engine: &EngineInner, soldier_id: EntityId) -> &crate::ai::AiController {
    engine
        .get_entity(soldier_id)
        .and_then(Entity::ai_controller)
        .expect("timing-test soldier has an AI controller")
}

#[derive(Clone, Copy)]
enum SpeechNpcKind {
    Soldier { vip: bool },
    Civilian { vip: bool },
}

fn add_speech_test_npc(
    engine: &mut EngineInner,
    assets: &mut LevelAssets,
    kind: SpeechNpcKind,
    speech_id: u32,
) -> EntityId {
    match kind {
        SpeechNpcKind::Soldier { vip } => {
            let profile_index = {
                let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
                if profiles.hth_weapons.is_empty() {
                    profiles.hth_weapons.push(Default::default());
                }
                let index = profiles.soldiers.len() as u32;
                profiles.soldiers.push(crate::profiles::SoldierProfile {
                    profile_name: format!("soldier-{index}"),
                    exclamation_id: speech_id,
                    hth_weapon_id: 1,
                    vip,
                    ..Default::default()
                });
                crate::profiles::SoldierProfileIdx(index)
            };
            let mut entity = make_test_soldier(crate::element::Posture::Upright);
            let Entity::Soldier(soldier) = &mut entity else {
                unreachable!()
            };
            soldier.soldier.soldier_profile_index = profile_index;
            soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
            soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("speech soldier has EnemyAi")
                .hth_weapon_id = 1;
            engine.add_test_entity(entity)
        }
        SpeechNpcKind::Civilian { vip } => {
            let profile_index = {
                let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
                let index = profiles.civilians.len() as u32;
                profiles.civilians.push(crate::profiles::CivilianProfile {
                    profile_name: format!("civilian-{index}"),
                    exclamation_id: speech_id,
                    civilian_type: if vip {
                        crate::profiles::CivilianType::Vip
                    } else {
                        crate::profiles::CivilianType::Man
                    },
                    ..Default::default()
                });
                crate::profiles::CivilianProfileIdx(index)
            };
            let mut entity = make_test_civilian(crate::element::Posture::Upright);
            let Entity::Civilian(civilian) = &mut entity else {
                unreachable!()
            };
            civilian.civilian.civilian_profile_index = profile_index;
            civilian.npc.ai_brain = crate::element::AiBrain::Friendly(Box::default());
            engine.add_test_entity(entity)
        }
    }
}

fn queue_and_settle_speech(
    engine: &mut EngineInner,
    assets: &LevelAssets,
    owner: EntityId,
    remark: crate::ai::Remark,
    flags: crate::ai::SpeechFlags,
) {
    engine
        .get_entity_mut(owner)
        .and_then(Entity::ai_controller_mut)
        .expect("speech test owner has AI")
        .say_with_flags(remark, flags);
    engine.drain_ai_owner_work_for(&crate::sim_rng::test_context(), assets, owner);
}

fn speech_log(engine: &EngineInner, owner: EntityId) -> Vec<(crate::ai::LogLineType, u16)> {
    engine
        .get_entity(owner)
        .and_then(Entity::ai_controller)
        .expect("speech test owner has AI")
        .ai_log
        .iter()
        .map(|line| (line.line_type, line.info))
        .collect()
}

fn last_speech_impossible(engine: &EngineInner, owner: EntityId) -> Option<u16> {
    speech_log(engine, owner)
        .into_iter()
        .rev()
        .find_map(|(kind, info)| (kind == crate::ai::LogLineType::SpeakImpossible).then_some(info))
}

fn exclamation_for(
    engine: &EngineInner,
    owner: EntityId,
) -> Option<(crate::sound::ExclamationGroup, u32, u16, i32)> {
    engine
        .feedback
        .pending_side_effects
        .sounds
        .iter()
        .rev()
        .find_map(|command| match command {
            crate::engine::SoundCommand::Exclamation {
                group,
                profile_id,
                exclamation_id,
                variant,
                actor_id: Some(actor_id),
                ..
            } if *actor_id == owner => Some((*group, *profile_id, *exclamation_id, *variant)),
            _ => None,
        })
}

fn make_alert_soldier_owner(engine: &mut EngineInner) -> EntityId {
    use crate::element::AiBrain;

    let owner = engine.add_test_entity(make_test_civilian(crate::element::Posture::Upright));
    let Entity::Civilian(civilian) = engine
        .get_entity_mut(owner)
        .expect("soldier-alert test civilian exists")
    else {
        panic!("soldier-alert test owner changed kind")
    };
    civilian.element.active = true;
    civilian.npc.life_points = 100;
    civilian.civilian.cached_camp = crate::element::Camp::Lacklandists;
    civilian.npc.ai_brain =
        AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(owner.index())));
    owner
}

fn check_detectable_snapshot_and_drain_matrix() {
    use crate::ai::DetectableMutation::{Add, Append, DeleteEntity, DeleteType};
    use crate::element::DetectableType::Friend;
    let sim = crate::sim_rng::test_context();
    let mut base = EngineInner::new();
    let owner = base.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target = base.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let sibling = base.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let cases = [
        (vec![Add(target, Friend), DeleteType(Friend)], vec![]),
        (vec![DeleteType(Friend), Add(target, Friend)], vec![target]),
        (vec![Append(target, Friend), DeleteType(Friend)], vec![]),
        (
            vec![DeleteType(Friend), Append(target, Friend)],
            vec![target],
        ),
        (
            vec![Add(target, Friend), DeleteEntity(target, Friend)],
            vec![],
        ),
        (
            vec![DeleteEntity(target, Friend), Add(target, Friend)],
            vec![target],
        ),
        (
            vec![Append(target, Friend), DeleteEntity(target, Friend)],
            vec![],
        ),
        (
            vec![DeleteEntity(target, Friend), Append(target, Friend)],
            vec![target],
        ),
        (vec![Add(target, Friend), Add(target, Friend)], vec![target]),
        (
            vec![Add(target, Friend), Append(target, Friend)],
            vec![target, target],
        ),
        (
            vec![Append(target, Friend), Add(target, Friend)],
            vec![target],
        ),
        (
            vec![
                Append(target, Friend),
                Append(target, Friend),
                DeleteEntity(target, Friend),
            ],
            vec![target],
        ),
        (
            vec![Append(target, Friend), Append(target, Friend)],
            vec![target, target],
        ),
        (
            vec![
                Add(target, Friend),
                Add(sibling, Friend),
                DeleteEntity(target, Friend),
            ],
            vec![sibling],
        ),
    ];
    for (operations, expected) in cases {
        base.get_entity_mut(owner)
            .unwrap()
            .ai_controller_mut()
            .unwrap()
            .outbox
            .actor
            .detectable_mutations = operations.clone();
        // Construct one state at a time instead of an array of whole engines.
        for restore in 0..3 {
            let mut engine = match restore {
                0 => base.clone(),
                1 => serde_json::from_str(&serde_json::to_string(&base).unwrap()).unwrap(),
                2 => super::super::snapshot::decode_native_engine_inner(
                    &super::super::snapshot::encode_native_engine_inner(&base),
                )
                .unwrap(),
                _ => unreachable!("three snapshot paths"),
            };
            assert_eq!(
                engine
                    .get_entity(owner)
                    .unwrap()
                    .ai_controller()
                    .unwrap()
                    .outbox
                    .actor
                    .detectable_mutations,
                operations
            );
            engine.drain_pending_for_npc(&sim, owner, &LevelAssets::default());
            let actor = engine.get_entity(owner).unwrap().ai_actor_data().unwrap();
            let actual = actor.detectable_lists[Friend as usize]
                .iter()
                .map(|entry| entry.element.expect("test detectable has a target"))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "operations: {operations:?}");
        }
    }
}

/// Give the default (empty) test grid a real map bounding box.
///
/// Position authorization rejects boxes wholly outside the level's map
/// bbox, and a default-constructed grid has no bbox at all — every
/// placement query fails. Tests that exercise formation placement need
/// an open field instead.
pub(super) fn install_test_open_field_bbox(engine: &mut EngineInner) {
    let mut level = (*engine.world.fast_grid.level).clone();
    level.map_bbox = MapBBox::from_coords(-10_000.0, -10_000.0, 10_000.0, 10_000.0);
    engine.world.fast_grid_mut().level = std::sync::Arc::new(level);
}

pub(super) fn install_test_building_sector(engine: &mut EngineInner, raw_sector: u16) {
    let _sector = crate::position_interface::SectorHandle::new(raw_sector)
        .expect("test building sector must be non-zero");
    let mut level = crate::fast_find_grid::LevelGrid::default();
    level
        .sector_number_map
        .insert(crate::sector::SectorNumber::new(raw_sector as i16), 0);
    level.sectors.push(crate::fast_find_grid::GridSector {
        points: Vec::new(),
        bounding_box: MapBBox::new(),
        sector_type: crate::sector::SectorType::BUILDING,
        layer: 0,
        sector_number: crate::sector::SectorNumber::new(raw_sector as i16),
        door_index: None,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    });
    engine.world.fast_grid_mut().level = std::sync::Arc::new(level);
}

fn run_synchronous_charly_report(officer_state: crate::ai::AiState) -> EngineInner {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::element::EyeStatus;

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 100;
    engine.world.weather.ambiance = crate::engine::types::Ambiance::Night;
    // Occupy slot 0 with a non-human entity: handle 0 is the null element
    // in AI handle space, so Charly must not land there or his viewer
    // identity cannot be resolved from the entity-view snapshot.
    engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::Target;
            initial_element
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let charly_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let officer_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    for (id, x) in [(charly_id, 0.0), (officer_id, 200.0)] {
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(id)
            .expect("test report soldier exists")
        else {
            panic!("test report entity changed kind")
        };
        soldier.element.active = true;
        soldier.element.set_position_map(MapPoint::new(x, 0.0));
        soldier.element.set_direction_instantly(4);
        soldier.npc.view_direction = [1.0, 0.0];
        soldier.npc.view_radius = 400;
        soldier.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
        soldier.npc.eye_status = EyeStatus::LookForward;
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test report soldier has enemy AI")
            .base
            .me = id.index();
    }

    {
        let charly = engine
            .get_entity_mut(charly_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("test Charly has enemy AI");
        charly.base.antagonist = Some(crate::ai::AiEntityHandle::new(officer_id.index()));
        charly.set_state(AiState::Seeking, Substate::SeekingCharlyGoToOfficer);
        charly.base.launch_timer(0, 100);
        charly.base.timer_is_running = false;
    }
    {
        let officer = engine
            .get_entity_mut(officer_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("test officer has enemy AI");
        let officer_substate = match officer_state {
            AiState::Default => Substate::DefaultOnPost,
            AiState::Attacking => Substate::AttackingSwordfight,
            other => panic!("unsupported Charly-report officer state: {other:?}"),
        };
        officer.set_state(officer_state, officer_substate);
    }

    let scratch = engine.build_sim_scratch(&assets);
    let ctx = {
        let entity = engine
            .get_entity(charly_id)
            .expect("test Charly exists for context");
        crate::engine::ai::build_ai_context_from_entity(
            entity,
            engine.control.frame_counter,
            None,
            engine.world.weather.is_forest_level,
            engine.world.weather.ambiance,
            engine.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &engine.world.fast_grid,
            &assets.navigation.hiking_paths,
            &assets.navigation.hiking_waypoint_sectors,
            &engine.ai.global.all_soldier_handles,
            engine.control.sim_config.difficulty,
        )
    };
    assert!(ctx.is_night_or_fog);
    let tick = engine.build_npc_tick_data(sim, charly_id, &assets);
    engine.dispatch_think_with_drain(
        sim,
        charly_id,
        &Stimulus::new(StimulusType::EventTimer),
        &ctx,
        &tick,
        &assets,
    );
    engine
}

fn run_synchronous_civilian_alert(
    soldier_state: crate::ai::AiState,
    trigger: crate::ai::StimulusType,
    direct_owner_self_stimulus: bool,
) -> EngineInner {
    use crate::ai::{AiState, Stimulus, StimulusType, Substate};
    use crate::element::AiBrain;

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    engine.control.frame_counter = 100;
    engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::Target;
            initial_element
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let civilian_id = engine.add_test_entity(make_test_civilian(crate::element::Posture::Upright));
    let soldier_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .civilians
        .push(crate::profiles::CivilianProfile::default());
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let Entity::Civilian(civilian) = engine
        .get_entity_mut(civilian_id)
        .expect("test civilian exists")
    else {
        panic!("test civilian changed kind")
    };
    civilian.element.active = true;
    civilian.element.set_position_map(MapPoint::new(0.0, 0.0));
    civilian.civilian.cached_camp = crate::element::Camp::Royalists;
    civilian.npc.ai_brain = AiBrain::Friendly(Box::new(crate::ai_friendly::FriendlyAi::new(
        civilian_id.index(),
    )));
    let friendly = civilian
        .npc
        .ai_brain
        .friendly_mut()
        .expect("test civilian has FriendlyAi");
    friendly.base.owner_entity_id = Some(civilian_id);
    friendly.base.antagonist = Some(crate::ai::AiEntityHandle::new(soldier_id.index()));
    friendly.base.my_reconnaissance_report.update(
        crate::ai::ReportType::Enemy,
        crate::ai::Position {
            x: 300.0,
            y: 20.0,
            sector: None,
            level: 0,
        },
    );
    friendly.set_state(AiState::Seeking, Substate::SeekingCivilianRunningToSoldier);
    civilian.npc.detectable_lists[crate::element::DetectableType::Friend as usize].push(
        crate::element::Detectable {
            element: Some(soldier_id),
            detectable_type: crate::element::DetectableType::Friend,
            ..Default::default()
        },
    );

    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("test soldier exists")
    else {
        panic!("test soldier changed kind")
    };
    soldier.element.active = true;
    soldier.element.set_position_map(MapPoint::new(20.0, 0.0));
    let enemy = soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("test soldier has EnemyAi");
    enemy.base.me = soldier_id.index();
    enemy.base.owner_entity_id = Some(soldier_id);
    enemy.set_state(
        soldier_state,
        if soldier_state == AiState::Default {
            Substate::DefaultOnPost
        } else {
            Substate::AttackingSwordfight
        },
    );

    complete_test_runtime_fixture(&mut engine, &mut assets);

    let scratch = engine.build_sim_scratch(&assets);
    let ctx = {
        let entity = engine
            .get_entity(civilian_id)
            .expect("test civilian exists for context");
        crate::engine::ai::build_ai_context_from_entity(
            entity,
            engine.control.frame_counter,
            None,
            engine.world.weather.is_forest_level,
            engine.world.weather.ambiance,
            engine.ai.standard_view_polygon_radius,
            &scratch.ai_entity_views,
            &scratch.ai_sight_obstacles,
            &engine.world.fast_grid,
            &assets.navigation.hiking_paths,
            &assets.navigation.hiking_waypoint_sectors,
            &engine.ai.global.all_soldier_handles,
            engine.control.sim_config.difficulty,
        )
    };
    let tick = engine.build_npc_tick_data(sim, civilian_id, &assets);
    if direct_owner_self_stimulus {
        engine
            .get_entity_mut(civilian_id)
            .and_then(Entity::friendly_ai_mut)
            .expect("direct-owner civilian has FriendlyAi")
            .base
            .outbox
            .reentrant
            .self_stimuli
            .push(trigger.into());
        engine.drain_direct_ai_owner_boundary(sim, civilian_id, &assets);
    } else {
        let stimulus = if trigger == StimulusType::EventSeesSoldier {
            Stimulus::with_human(trigger, soldier_id.index())
        } else {
            Stimulus::new(trigger)
        };
        engine.dispatch_think_with_drain(sim, civilian_id, &stimulus, &ctx, &tick, &assets);
    }
    engine
}

fn setup_review2_officer_and_soldier() -> (EngineInner, EntityId, EntityId, LevelAssets) {
    use crate::ai::{AiState, Substate};
    use crate::profiles::ProfileRank;

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 100;
    // AI human handles use zero as missing, so keep production NPCs off slot 0.
    engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::Target;
            initial_element
        },
        fx: Default::default(),
        target: Default::default(),
    }));
    let officer_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let soldier_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    for (id, rank, x) in [
        (officer_id, ProfileRank::Officer, 0.0),
        (soldier_id, ProfileRank::Soldier, 40.0),
    ] {
        let Entity::Soldier(soldier) = engine.get_entity_mut(id).expect("review2 soldier exists")
        else {
            panic!("review2 entity changed kind")
        };
        soldier.element.active = true;
        soldier.element.set_position_map(MapPoint::new(x, 0.0));
        soldier.element.index_in_elements_list = id.index() as u16;
        soldier.npc.life_points = 100;
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("review2 soldier has EnemyAi");
        ai.base.me = id.index();
        ai.soldier_profile_rank = rank;
        ai.set_state(AiState::Default, Substate::DefaultOnPost);
    }
    complete_test_runtime_fixture(&mut engine, &mut assets);
    (engine, officer_id, soldier_id, assets)
}

fn review2_context_and_tick(
    engine: &EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    id: EntityId,
) -> (crate::ai::AiContext, crate::ai::AiPerTickData) {
    let scratch = engine.build_sim_scratch(assets);
    let ctx = crate::engine::ai::build_ai_context_from_entity(
        engine.get_entity(id).expect("review2 context owner exists"),
        engine.control.frame_counter,
        None,
        engine.world.weather.is_forest_level,
        engine.world.weather.ambiance,
        engine.ai.standard_view_polygon_radius,
        &scratch.ai_entity_views,
        &scratch.ai_sight_obstacles,
        &engine.world.fast_grid,
        &assets.navigation.hiking_paths,
        &assets.navigation.hiking_waypoint_sectors,
        &engine.ai.global.all_soldier_handles,
        engine.control.sim_config.difficulty,
    );
    let tick = engine.build_npc_tick_data(sim, id, assets);
    (ctx, tick)
}

fn start_review_command_soldiers(
    engine: &mut EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    officer_id: EntityId,
) -> (
    crate::ai_enemy::CommandSoldiersStart,
    crate::ai::AiPerTickData,
) {
    use crate::ai::Position;

    let (ctx, tick) = review2_context_and_tick(engine, sim, assets, officer_id);
    let global = engine.ai.global.clone();
    let start = engine
        .get_entity_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("review command caller has EnemyAi")
        .command_soldiers_to_attack(
            Position {
                x: 300.0,
                ..Default::default()
            },
            &global,
            None,
            &ctx,
            &tick,
        );
    (start, tick)
}

fn queue_review2_wrong_kind_think(
    engine: &mut EngineInner,
    officer_id: EntityId,
    civilian_id: EntityId,
    stimulus_type: crate::ai::StimulusType,
    continuation: crate::ai::ThinkResultContinuation,
) {
    engine
        .get_entity_mut(officer_id)
        .and_then(Entity::ai_controller_mut)
        .expect("review2 wrong-kind caller has AI")
        .outbox
        .reentrant
        .cross_npc_actions
        .push(crate::ai::CrossNpcAction::RequestThinkResult {
            target: civilian_id.index(),
            caller: officer_id.index(),
            stimulus_type,
            info: crate::ai::StimulusInfo::Human(crate::ai::AiEntityHandle::new(
                officer_id.index(),
            )),
            continuation,
        });
}

mod combat;
mod movement;
mod orders;
mod perception;
mod projectiles;
mod state;
