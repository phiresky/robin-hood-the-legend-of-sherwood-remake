use super::*;
use crate::coordinates::WorldPoint3D;
use crate::element::{
    ActiveFlight, ActorCivilian, ActorData, ActorPc, ActorSoldier, CivilianData, ElementData,
    ElementKind, HumanData, NpcData, PcData, SoldierData,
};
use crate::scb::{ClassEntry, SCB_VERSION, ScbFile};

#[test]
fn sector_to_angle_keeps_original_double_intermediate_rounding() {
    let direction_angle = sector_to_angle(9);
    assert_eq!(direction_angle.to_bits(), 0x4068_983d);

    // Profile angle getters likewise narrow 45 degrees to this FLOAT.
    // Three rotation ticks must reach the true-half-circle final angle
    // exactly, allowing ExecuteTrueCircleSwordStrikeAction to resume the
    // sprite animation on the terminal-direction Hourglass.
    let quarter_turn = ((45.0_f64 / 360.0) * 2.0 * f64::from(std::f32::consts::PI)) as f32;
    let initial_angle = direction_angle - quarter_turn;
    let final_angle = initial_angle + std::f32::consts::PI;
    let after_three_ticks = direction_angle + quarter_turn + quarter_turn + quarter_turn;

    assert_eq!(after_three_ticks.to_bits(), final_angle.to_bits());
    assert_eq!(final_angle.to_bits(), 0x40bf_b210);
}

#[test]
fn strike_collector_angles_and_push_width_keep_original_conversions() {
    let expected_angle = ((7.0_f64 / 360.0) * 2.0 * f64::from(std::f32::consts::PI)) as f32;
    assert_eq!(strike_profile_angle(7).to_bits(), expected_angle.to_bits());
    assert_eq!(angle_to_sector(-std::f32::consts::PI / 8.0), 14);
    assert_eq!(push_strike_half_width(5), 2.0);
    assert_eq!(push_strike_half_width(6), 3.0);
}

#[test]
fn strike_estimation_collects_inactive_principal_only() {
    let attacker = EntityId::Pc(crate::element::PcId(1));
    let principal = EntityId::Soldier(crate::element::SoldierId(2));
    let bystander = EntityId::Soldier(crate::element::SoldierId(3));

    assert!(should_collect_strike_estimation_human(
        principal,
        attacker,
        Some(principal),
        false,
    ));
    assert!(!should_collect_strike_estimation_human(
        bystander,
        attacker,
        Some(principal),
        false,
    ));
    assert!(should_collect_strike_estimation_human(
        bystander,
        attacker,
        Some(principal),
        true,
    ));
    assert!(!should_collect_strike_estimation_human(
        attacker,
        attacker,
        Some(attacker),
        true,
    ));
}

#[test]
fn push_warning_and_done_effect_keep_distinct_elevation_and_max_norm_gates() {
    assert!(push_strike_elevation_allows(
        PushStrikePositionSpace::Map,
        0.0,
        80.0,
    ));
    assert!(!push_strike_elevation_allows(
        PushStrikePositionSpace::Ground,
        0.0,
        80.0,
    ));

    // ExecutePushSwordStrike stores fabs(elevation difference) in ULONG,
    // so the fractional part is discarded before the <= 40.f gate.
    assert!(push_strike_elevation_allows(
        PushStrikePositionSpace::Ground,
        0.0,
        40.75,
    ));
    assert!(!push_strike_elevation_allows(
        PushStrikePositionSpace::Ground,
        0.0,
        41.0,
    ));

    assert!(push_strike_max_norm_allows(
        PushStrikePositionSpace::Map,
        149.999,
        0.0,
    ));
    assert!(!push_strike_max_norm_allows(
        PushStrikePositionSpace::Map,
        150.0,
        0.0,
    ));
    assert!(push_strike_max_norm_allows(
        PushStrikePositionSpace::Ground,
        160.0,
        0.0,
    ));
}

#[test]
fn full_circle_done_seed_uses_inclusive_unprojected_3d_range() {
    let attacker = WorldPoint3D::ZERO;
    let elevated_same_map = WorldPoint3D::new(0.0, 50.0, 50.0);

    assert_eq!(attacker.to_map(), elevated_same_map.to_map());
    assert!(!full_circle_strike_distance_is_in_range(
        attacker,
        elevated_same_map,
        0.0,
        60.0,
    ));

    let ordinary = WorldPoint3D::new(3.0, 4.0, 12.0);
    assert_eq!(full_circle_strike_distance(attacker, ordinary), 13.0);
    assert!(full_circle_strike_distance_is_in_range(
        attacker, ordinary, 13.0, 13.0,
    ));
    assert!(!full_circle_strike_distance_is_in_range(
        attacker, ordinary, 13.01, 20.0,
    ));
}

#[test]
fn sword_strike_range_rejects_nan_like_original_positive_comparisons() {
    assert!(!sword_strike_distance_is_in_range(f32::NAN, 0.0, 65.0));
    assert!(sword_strike_distance_is_in_range(0.0, 0.0, 65.0));
    assert!(sword_strike_distance_is_in_range(65.0, 0.0, 65.0));
}

#[test]
fn half_circle_done_seed_combines_3d_range_with_ground_space_sector() {
    let attacker = WorldPoint3D::ZERO;
    let elevated_same_map = WorldPoint3D::new(0.0, 10.0, 10.0);

    assert_eq!(attacker.to_map(), elevated_same_map.to_map());
    assert!(half_circle_strike_seed_allows(
        attacker,
        elevated_same_map,
        0.0,
        20.0,
        8,
        8,
    ));
    assert!(!half_circle_strike_seed_allows(
        attacker,
        elevated_same_map,
        0.0,
        10.0,
        8,
        8,
    ));

    let exact_boundary = WorldPoint3D::new(3.0, 4.0, 12.0);
    assert!(half_circle_strike_seed_allows(
        attacker,
        exact_boundary,
        13.0,
        13.0,
        0,
        15,
    ));
}

#[test]
fn sweep_state_uses_angles_returned_by_original_sword_getters() {
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_soldier(WorldPoint3D::new(10.0, 0.0, 0.0), None));
    let mut assets =
        assets_with_nonstraight_profile(SwordStrike::D, crate::profiles::WeaponThrustKind::Lateral);
    let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::D as usize];
    thrust.initial_angle = 5;
    thrust.final_angle = 5;
    thrust.rotation_angle = 5;

    engine.initialize_sweep(
        &assets,
        attacker,
        SwordStrike::D,
        Some(1),
        crate::profiles::WeaponThrustKind::Lateral,
        vec![victim],
    );
    let direction_angle = sector_to_angle(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .direction(),
    );
    let sweep = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .unwrap();
    let five_degrees = f32::from_bits(0x3db2_b8c3);
    assert_eq!(
        sweep.initial_angle.to_bits(),
        (direction_angle - five_degrees).to_bits()
    );
    assert_eq!(
        sweep.final_angle.to_bits(),
        (direction_angle + five_degrees).to_bits()
    );
    assert_eq!(sweep.rotation_per_frame.to_bits(), five_degrees.to_bits());

    install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, true);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sweep_state = None;
    engine.rebind_retained_sweep_to_active_strike(&assets, attacker);
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .unwrap()
            .rotation_per_frame
            .to_bits(),
        five_degrees.to_bits(),
        "loaded sweep reconstruction uses the same RHSword getter conversion"
    );

    // Keep a common authored angle as a control alongside the one-bit 5° case.
    assert_eq!(strike_profile_angle(45).to_bits(), 0x3f49_0fdb);
}

#[test]
fn circle_warning_tolerance_uses_radians_returned_by_sword_profile() {
    // The profile stores 180 degrees, but RHSword::GetStrikeRotationAngle
    // returns PI radians. At relative sector 8 Original therefore extends
    // the warning range by 15 units: 10 + (8 * 5 * PI) / (8 * PI).
    let tolerance = circle_warning_walking_tolerance(8, 180);
    assert_eq!(tolerance, 15);

    let base_max_distance = 60_u16;
    let walking_target_distance = 74.0;
    assert!(walking_target_distance <= f32::from(base_max_distance + tolerance));

    // Dividing by the raw profile degrees, as the old port did, would
    // reject this moving defender and suppress its WarnForStrike callback.
    let raw_degrees_tolerance = 10.0 + (8.0 * 5.0 * std::f32::consts::PI) / (8.0 * 180.0);
    assert!(walking_target_distance > f32::from(base_max_distance) + raw_degrees_tolerance);
    assert!(walking_target_distance > f32::from(base_max_distance));

    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    let target = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: walking_target_distance,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let actor = engine
            .get_entity_mut(target)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.action_state = ActionState::MovingSword;
        actor.installed_order = Some(crate::element::InstalledActorOrder {
            order_id: std::num::NonZeroU32::new(1).unwrap(),
            order_type: OrderType::WalkingWithSword,
        });
    }
    let assets = assets_with_sword_profile(0, base_max_distance);
    let collect = |engine: &EngineInner, max_distance| {
        collect_circle_warn_victims(
            &engine.world.entities,
            attacker,
            (0.0, 0.0),
            0,
            max_distance,
            180,
            |target_id| engine.live_actor_animation(target_id) == Some(OrderType::WalkingWithSword),
            &assets.profile_manager,
            &engine.world.fast_grid,
            crate::sight_obstacle::ObstacleList {
                static_obstacles: assets.static_sight_obstacles.as_slice(),
                dynamic_obstacles: &engine.world.dynamic_sight_obstacles,
                static_active: &engine.world.static_sight_obstacle_active,
            },
        )
    };
    assert_eq!(collect(&engine, base_max_distance), vec![target]);

    // Running shares the port's coarse MovingSword state with walking,
    // but Original's exact GetAnimation predicate does not extend it.
    engine
        .get_entity_mut(target)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .installed_order
        .as_mut()
        .unwrap()
        .order_type = OrderType::RunningWithSword;
    assert!(collect(&engine, base_max_distance).is_empty());

    engine
        .get_entity_mut(target)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .installed_order
        .as_mut()
        .unwrap()
        .order_type = OrderType::WaitingSword;
    assert!(collect(&engine, base_max_distance).is_empty());

    // The ordinary case above admits at 60 + 15 = 75. UWORD compound
    // assignment instead wraps 65530 + 15 to 9, excluding the same
    // target rather than comparing against an unbounded float sum.
    engine
        .get_entity_mut(target)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .installed_order
        .as_mut()
        .unwrap()
        .order_type = OrderType::WalkingWithSword;
    assert!(collect(&engine, u16::MAX - 5).is_empty());
}

#[test]
fn straight_strike_range_uses_stored_world_position() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    let target = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(60.0, 40.0, 40.0));

    // The isometric projection subtracts elevation from world Y, so
    // these actors are only 60 map units apart while Original's
    // ExecuteStraightSwordStrike range check sees all three components.
    assert_eq!(
        entity_distance(&engine.world.entities, attacker, target),
        60.0
    );
    assert_eq!(
        entity_world_distance(&engine.world.entities, attacker, target),
        (60.0_f32 * 60.0 + 40.0 * 40.0 + 40.0 * 40.0).sqrt()
    );
}

#[test]
fn swordfight_range_uses_stored_world_position_across_elevation() {
    let mut engine = EngineInner::new();
    let initiator = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    let opponent = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    engine
        .get_entity_mut(initiator)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(1028.4918, 2063.3013, 22.8174));
    engine
        .get_entity_mut(opponent)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(1032.8688, 1992.2421, 122.2636));

    // This is the replay geometry from S043 r004 f8369. Isometric map
    // projection puts the actors more than 150 units apart, while the
    // 3D GetPosition norm used by Original EnterSwordFight is in range.
    assert!(entity_distance(&engine.world.entities, initiator, opponent) > 150.0);
    assert!(entity_world_distance(&engine.world.entities, initiator, opponent) < 150.0);
}

#[test]
#[should_panic(expected = "straight-strike distance references missing victim Soldier")]
fn straight_strike_range_rejects_a_missing_victim() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    let missing = EntityId::Soldier(crate::entity_id::SoldierId(u32::MAX));

    let _ = entity_world_distance(&engine.world.entities, attacker, missing);
}

fn make_engine() -> EngineInner {
    let mut engine = EngineInner::new();
    // Every PC built by `make_pc` carries campaign-description index 0,
    // so the campaign character table needs a matching entry backing the
    // required live-PC identity.
    engine.mission_domain.campaign.characters = vec![crate::campaign::PcDescription {
        character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
        ..Default::default()
    }];
    engine
}

#[test]
fn fresh_selected_strike_uses_captured_stale_impossible_row_residue() {
    let mut engine = make_engine();
    let target = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    let selected_row = crate::sprite_script::SpriteScript {
        action_done: 2,
        frame_ids: vec![0, 1, 2],
        delays: vec![2, 2, 2],
        distances: vec![0; 3],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
        ..Default::default()
    };
    let stale_row = crate::sprite_script::SpriteScript {
        action_done: u16::MAX,
        frame_ids: vec![0, 1],
        delays: vec![2, 2],
        distances: vec![0; 2],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
        sound_ids: vec![0; 2],
        ..Default::default()
    };
    {
        let sprite = &mut engine
            .get_entity_mut(target)
            .unwrap()
            .element_data_mut()
            .sprite;
        let mut conversion =
            vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
        conversion[OrderType::StrikingRightSword as usize] = 0;
        sprite.scripts = std::sync::Arc::new(vec![selected_row, stale_row]);
        sprite.conversion = std::sync::Arc::new(conversion);
        sprite.current_row = 1;
        sprite.current_frame = 1;
        sprite.action_done_frame = u16::MAX;
        sprite.last_processed_order_id = 41;
    }

    let mut element =
        crate::sequence::SequenceElement::new(1, Command::SwordstrikeThrustE, Some(target));
    element.priority = crate::sequence::SequencePriority::Preference;
    let selected_order_id = engine.orders.allocate_order_id();
    element.orders.push_back(crate::order::Order::new(
        OrderType::StrikingRightSword,
        0.0,
        0.0,
        selected_order_id,
    ));
    let sequence_id = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);
    engine.publish_selected_order_as_installed(target);

    assert_eq!(
        engine.opponent_sword_strike_time_limit_for_actor(target, target),
        Some(i16::MIN),
        "replays without captured allocator residue use the reviewed strict impossible deadline"
    );

    let target_creation_order = engine.world.original_creation_order(target);
    let other_proposer = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    let other_creation_order = engine.world.original_creation_order(other_proposer);
    engine
        .control
        .original_impossible_action_done_deadlines
        .insert(
            (target_creation_order, target_creation_order),
            std::collections::VecDeque::from([-22478, 12]),
        );
    engine
        .control
        .original_impossible_action_done_deadlines
        .insert(
            (other_creation_order, target_creation_order),
            std::collections::VecDeque::from([30]),
        );
    assert_eq!(
        engine.opponent_sword_strike_time_limit_for_actor(target, target),
        Some(-22478),
        "schema-16 can carry the Original allocator-dependent wrapped SWORD"
    );
    assert_eq!(
        engine.opponent_sword_strike_time_limit_for_actor(target, target),
        Some(12),
        "repeated proposals against one target consume captured deadlines in invocation order"
    );
    assert_eq!(
        engine.opponent_sword_strike_time_limit_for_actor(other_proposer, target),
        Some(30),
        "different proposers targeting the same actor keep independent occurrence queues"
    );

    let sprite = &mut engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .sprite;
    sprite.last_processed_order_id = selected_order_id.get();
    sprite.action_done_frame = u16::MAX;
    assert_eq!(
        engine.opponent_sword_strike_time_limit_for_actor(target, target),
        Some(i16::MIN),
        "the current sprite's impossible marker retains the strict S075 behavior"
    );
}

fn empty_mission_script() -> crate::engine::types::MissionScript {
    let startup = ClassEntry {
        source_file: "melee_test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: Vec::new(),
        quads: Vec::new(),
    };
    crate::engine::types::MissionScript::from_scb(ScbFile {
        version: SCB_VERSION,
        classes: vec![startup],
    })
    .expect("minimal StartUp script must load")
}

fn make_soldier(
    pos: WorldPoint3D,
    sector: Option<crate::position_interface::SectorHandle>,
) -> Entity {
    let mut element = ElementData {
        kind: ElementKind::ActorSoldier,
        active: true,
        posture: Posture::Upright,
        ..ElementData::default()
    };
    element.set_position(pos);
    element.set_position_map(crate::coordinates::MapPoint::from_world_xyz(
        pos.x, pos.y, pos.z,
    ));
    element.set_sector(sector);
    Entity::Soldier(ActorSoldier {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData {
            life_points: 50,
            ai: crate::element::AiActorData {
                ai_brain: crate::element::AiBrain::Enemy(Box::default()),
                ..Default::default()
            },
        },
        soldier: SoldierData {
            cached_camp: crate::element::Camp::Lacklandists,
            ..SoldierData::default()
        },
    })
}

fn make_pc(pos: WorldPoint3D, sector: Option<crate::position_interface::SectorHandle>) -> Entity {
    let mut element = ElementData {
        kind: ElementKind::ActorPc,
        active: true,
        posture: Posture::Upright,
        ..ElementData::default()
    };
    element.set_position(pos);
    element.set_position_map(crate::coordinates::MapPoint::from_world_xyz(
        pos.x, pos.y, pos.z,
    ));
    element.set_sector(sector);
    Entity::Pc(ActorPc {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData {
            life_points: 50,
            profile_index: crate::profiles::CharacterProfileIdx(0),
            campaign_description_index: Some(0),
            ..PcData::default()
        },
    })
}

#[test]
fn autonomous_vip_combatant_death_does_not_latch_party_failure() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let Entity::Pc(pc) = engine.get_entity_mut(victim).unwrap() else {
        unreachable!()
    };
    pc.pc.mission_role = crate::human_control::MissionRole::Combatant;

    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(crate::profiles::CharacterProfile {
        vip: true,
        ..Default::default()
    });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };

    engine.apply_pc_kill_cascade(&sim, &assets, victim);

    assert!(engine.mission_domain.dead_pc.is_none());
}

#[test]
fn player_party_vip_death_still_latches_party_failure() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(crate::profiles::CharacterProfile {
        vip: true,
        ..Default::default()
    });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };

    engine.apply_pc_kill_cascade(&sim, &assets, victim);

    assert_eq!(engine.mission_domain.dead_pc, Some(victim));
}

#[test]
fn damage_dispatcher_disables_direction_on_live_reaction_orders() {
    for (command, expected) in [
        (Command::ReceiveDamage, OrderType::FallingBackUpright),
        (Command::ReceiveMobileDamage, OrderType::FallingBackUpright),
        (
            Command::ReceiveArrowDamage,
            OrderType::ExtractingArrowUpright,
        ),
        (Command::ReceiveStoneDamage, OrderType::FallingBackUpright),
    ] {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
        let mut damage = crate::sequence::SequenceElement::new_damage(
            1,
            command,
            Some(victim),
            Some(attacker),
            1,
            0,
        );
        engine.resolve_element_priority(&mut damage);
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        let mut display = crate::engine::HostDisplayState::default();
        engine.hourglass_phase_sequences(&sim, &mut display, &assets);

        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("translated damage command remains registered");
        assert_eq!(element.command, command);
        assert!(
            element
                .orders
                .iter()
                .any(|order| order.order_type == expected),
            "{command:?} must author {expected:?}"
        );
        assert!(
            !element
                .orders
                .iter()
                .find(|order| order.order_type == expected)
                .unwrap()
                .compute_direction
        );
    }
}

fn action_test_assets(actions: [crate::profiles::Action; 3]) -> LevelAssets {
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.characters.push(crate::profiles::CharacterProfile {
        actions,
        ..Default::default()
    });
    profiles
        .soldiers
        .push(crate::profiles::SoldierProfile::default());
    LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    }
}

fn make_civilian(pos: WorldPoint3D) -> Entity {
    let mut element = ElementData {
        kind: ElementKind::ActorCivilian,
        active: true,
        posture: Posture::Upright,
        ..ElementData::default()
    };
    element.set_position(pos);
    element.set_position_map(crate::coordinates::MapPoint::from_world_xyz(
        pos.x, pos.y, pos.z,
    ));
    Entity::Civilian(ActorCivilian {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData {
            life_points: 100,
            ..NpcData::default()
        },
        civilian: CivilianData {
            cached_camp: crate::element::Camp::Royalists,
            ..CivilianData::default()
        },
    })
}

/// Set up a live falling-hit Execute flight on `flyer` so the per-frame
/// `tick_push_flights` sweep fires `apply_domino_effect`.
fn give_flight(
    engine: &mut EngineInner,
    flyer: EntityId,
    antagonist: EntityId,
    inc_x: f32,
    inc_y: f32,
    frames: u16,
) {
    engine
        .get_entity_mut(flyer)
        .expect("test flight owner exists")
        .element_data_mut()
        .sprite
        .scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
        frame_ids: vec![0, 1],
        ..Default::default()
    }]);
    let flyer_pos = engine
        .get_entity(flyer)
        .unwrap()
        .element_data()
        .position_map();

    // Original owns combat flight from the live falling order's Execute
    // arm. Mirror that lifecycle instead of manufacturing an orphaned
    // `active_flight`, which production correctly holds until the order is
    // current and its START edge has changed posture to Flying.
    let damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveHitDamage,
        Some(flyer),
        Some(antagonist),
        1,
        0,
    );
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    let order_id = engine.push_new_order(
        sequence,
        0,
        crate::order::OrderType::FallingHitUpright,
        0.0,
        0.0,
    );
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    if let Some(entity) = engine.world.entities.get_mut(flyer) {
        entity.set_posture(Posture::Flying);
        let actor = entity
            .actor_data_mut()
            .expect("combat flight owner must be an actor");
        actor.installed_order = Some(crate::element::InstalledActorOrder {
            order_id,
            order_type: crate::order::OrderType::FallingHitUpright,
        });
        actor.active_flight = Some(ActiveFlight {
            increment_x: inc_x,
            increment_y: inc_y,
            goal_x: flyer_pos.x + inc_x * frames as f32,
            goal_y: flyer_pos.y + inc_y * frames as f32,
            frames_remaining: frames,
            antagonist: Some(antagonist),
            ..Default::default()
        });
    }
}

fn count_domino_hits_for(engine: &EngineInner, victim: EntityId, hitter: EntityId) -> usize {
    engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|s| s.elements.iter())
        .filter(|e| {
            e.command == Command::ReceiveHitDamage
                && e.owner == Some(victim)
                && match &e.data {
                    SequenceElementData::Damage {
                        origin,
                        damage,
                        concussion,
                        is_harder_hit,
                        ..
                    } => {
                        *origin == Some(hitter)
                            && *damage == 0
                            && *concussion == DOMINO_DAMAGE
                            && !*is_harder_hit
                    }
                    _ => false,
                }
        })
        .count()
}

#[test]
fn hit_translation_defers_flight_facing_until_first_execute() {
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 30.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.element_data_mut().set_layer(4);
        let position = victim_entity.position_iface_mut();
        position.set_direction_instantly(crate::position_interface::Direction::from_raw(5));
        position.set_move_box(crate::coordinates::MoveBox::from_coords(
            -5.0, -5.0, 5.0, 5.0,
        ));
    }

    let element = crate::sequence::SequenceElement::new(1, Command::ReceiveHitDamage, Some(victim));
    let seq_id = engine.launch_element(element);
    engine.dispatch_hit_fall_animation(
        &LevelAssets::default(),
        victim,
        Some(attacker),
        false,
        (seq_id, 0),
    );

    let queued = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap()
        .orders
        .back()
        .unwrap();
    assert_eq!(queued.order_type, OrderType::FallingHitUpright);
    assert_eq!(queued.antagonist, Some(attacker));
    assert!(!queued.compute_direction);
    let queued_type = queued.order_type;
    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(victim_entity.element_data().direction(), 5);
    assert_eq!(victim_entity.position_iface().layer_goal().get(), 0);
    assert!(victim_entity.actor_data().unwrap().active_flight.is_none());

    engine.initialize_hit_flight(&LevelAssets::default(), victim, Some(attacker), queued_type);

    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .position_iface()
            .layer_goal()
            .get(),
        4,
        "ReadyForTakeOff publishes its authored goal layer immediately"
    );
    assert_ne!(
        engine
            .get_entity(victim)
            .unwrap()
            .element_data()
            .direction(),
        5
    );
}

#[test]
fn hit_translation_without_animation_terminates_despite_retained_transition_order() {
    let mut engine = make_engine();
    let victim = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
    engine
        .get_entity_mut(victim)
        .expect("hit victim exists")
        .set_posture(Posture::Flying);

    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveHitDamage, Some(victim));
    damage.orders.push_back(crate::order::Order::new(
        OrderType::NonanimationEnd,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    damage.initialize_transition_orders();
    let sequence = engine.launch_element(damage);

    engine.dispatch_hit_fall_animation(&LevelAssets::default(), victim, None, false, (sequence, 0));

    let damage = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("terminated hit element remains inspectable");
    assert_eq!(damage.state, crate::sequence::SequenceState::Terminated);
    assert_eq!(
        damage.orders.len(),
        1,
        "termination retains the pre-translation order for diagnostics"
    );
}

fn initialized_hit_flight_delta(
    engine: &EngineInner,
    victim: EntityId,
) -> crate::coordinates::MapPoint {
    let victim = engine.get_entity(victim).unwrap();
    let flight = victim
        .actor_data()
        .unwrap()
        .active_flight
        .as_ref()
        .expect("unobstructed falling hit must initialize a flight");
    let position = victim.element_data().position_map();
    crate::coordinates::MapPoint::new(flight.goal_x - position.x, flight.goal_y - position.y)
}

fn authorize_test_hit_flight(engine: &mut EngineInner, victim: EntityId) {
    engine.world.fast_grid_mut().size_map(4, 4);
    engine.world.fast_grid_mut().allocate_layers(1);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_coords(
            -5.0, -5.0, 5.0, 5.0,
        ));
}

#[test]
fn charging_rider_falling_hit_normalizes_non_cardinal_sector_vector() {
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 32.0,
            y: 1.0,
            z: 0.0,
        },
        None,
    ));
    authorize_test_hit_flight(&mut engine, victim);
    {
        let Entity::Soldier(attacker) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        attacker.soldier.rider = true;
        attacker.actor.active_rider_charge = Some(crate::element::ActiveRiderCharge {
            pending_victims: vec![victim],
        });
        attacker.element.set_direction_instantly(11);
    }

    engine.initialize_hit_flight(
        &LevelAssets::new(),
        victim,
        Some(attacker),
        OrderType::FallingHitUpright,
    );

    let delta = initialized_hit_flight_delta(&engine, victim);
    assert_eq!(delta.x.to_bits(), 0xc1e9_801b);
    assert_eq!(delta.y.to_bits(), 0x40dd_e72e);
    assert!(
        delta.x < 0.0 && delta.y > 0.0,
        "direction 11 flies southwest"
    );
}

#[test]
fn antagonistless_falling_hit_normalizes_opposite_non_cardinal_sector_vector() {
    let mut engine = make_engine();
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 32.0,
            y: 1.0,
            z: 0.0,
        },
        None,
    ));
    authorize_test_hit_flight(&mut engine, victim);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .position_iface_mut()
        .set_direction_instantly(crate::position_interface::Direction::from_raw(3));

    engine.initialize_hit_flight(
        &LevelAssets::new(),
        victim,
        None,
        OrderType::FallingHitUpright,
    );

    let delta = initialized_hit_flight_delta(&engine, victim);
    assert_eq!(delta.x.to_bits(), 0xc1e9_801b);
    assert_eq!(delta.y.to_bits(), 0x40dd_e72e);
    assert!(
        delta.x < 0.0 && delta.y > 0.0,
        "opposite direction 11 flies southwest"
    );
}

#[test]
fn positioned_antagonist_falling_hit_keeps_radial_normalization() {
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: -2.0,
            y: -4.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 1.0,
            y: 1.0,
            z: 0.0,
        },
        None,
    ));
    authorize_test_hit_flight(&mut engine, victim);

    engine.initialize_hit_flight(
        &LevelAssets::new(),
        victim,
        Some(attacker),
        OrderType::FallingHitUpright,
    );

    let delta = initialized_hit_flight_delta(&engine, victim);
    // Adding the exact source component 0x4176_f53d to x=1 and
    // subtracting the origin rounds the observable displacement once;
    // the old per-component normalization instead produced 0x4176_f53e.
    assert_eq!(delta.x.to_bits(), 0x4176_f53c);
    assert_eq!(delta.y.to_bits(), 0x41cd_cc5e);
}

#[test]
fn ladder_fall_translation_retains_layer_goal_and_authors_landing_target() {
    let mut engine = make_engine();
    engine.scripts.mission = Some(empty_mission_script());

    let lift_sector = crate::sector::SectorNumber::new(42);
    let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
    level.sector_number_map.insert(lift_sector, 0);
    level.sectors.push(crate::fast_find_grid::GridSector {
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type: crate::sector::SectorType::LIFT,
        layer: 0,
        sector_number: lift_sector,
        door_index: None,
        lift_type: Some(crate::sector::LiftType::Ladder),
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: Some(0),
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    });
    engine
        .script_domains
        .interactables
        .doors
        .push(crate::gate::Door {
            point_out: crate::coordinates::MapPoint::new(30.0, 0.0),
            layer_out: 3,
            sector_out: crate::sector::SectorNumber::new(7),
            ..crate::gate::Door::default()
        });

    let victim = engine.add_entity(make_pc(
        WorldPoint3D::default(),
        crate::position_interface::SectorHandle::new(42),
    ));
    let damage = crate::sequence::SequenceElement::new(1, Command::ReceiveHitDamage, Some(victim));
    let sequence = engine.launch_element(damage);

    engine.translate_ladder_wall_fall(&LevelAssets::default(), victim, (sequence, 0));

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_entity.position_iface().layer_goal().get(),
        0,
        "translation must not publish the destination layer before arrival"
    );
    let flight = victim_entity
        .actor_data()
        .unwrap()
        .active_flight
        .as_ref()
        .expect("a non-trivial ladder fall installs a flight");
    assert_eq!(flight.goal_layer, 3);
    assert_eq!(
        flight.goal_sector,
        crate::position_interface::SectorHandle::new(7)
    );
    assert!(flight.ladder_fall);
}

#[test]
fn pc_hit_translation_inherits_silent_human_say_ouch() {
    let mut engine = make_engine();
    let victim = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let damage = crate::sequence::SequenceElement::new(1, Command::ReceiveHitDamage, Some(victim));
    let sequence_id = engine.launch_element(damage);

    engine.apply_hit_damage(
        &crate::sim_rng::test_context(),
        &LevelAssets::default(),
        victim,
        None,
        1,
        false,
        (sequence_id, 0),
    );

    assert!(
        engine.feedback.sound_sim.pending_exclamations.is_empty(),
        "PC inherits RHElementActorHuman::SayOuch's no-op on TranslateHitDamage"
    );
}

#[test]
fn scroll_civilian_hit_keeps_immunity_but_still_translates_reaction() {
    let mut engine = make_engine();
    let _null_slot = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let attacker = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let victim = engine.add_entity(make_civilian(WorldPoint3D {
        x: 20.0,
        ..WorldPoint3D::default()
    }));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        let civilian = match victim_entity {
            Entity::Civilian(civilian) => civilian,
            other => panic!("test civilian changed kind to {:?}", other.kind()),
        };
        civilian.npc.attached_scroll = Some(crate::entity_id::EntityId::Scroll(
            crate::entity_id::ScrollId(u32::MAX),
        ));
        civilian.npc.ai_brain = crate::element::AiBrain::Friendly(Box::new(
            crate::ai_friendly::FriendlyAi::new(victim.index()),
        ));
    }
    let damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveHitDamage,
        Some(victim),
        Some(attacker),
        0,
        3,
    );
    let sequence = engine.launch_element(damage);
    let mut assets = LevelAssets::default();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .civilians
        .push(crate::profiles::CivilianProfile::default());

    engine.dispatch_receive_damage(
        &crate::sim_rng::test_context(),
        &assets,
        victim,
        sequence,
        0,
    );

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_entity.human_data().unwrap().concussion_of_the_brain,
        0,
        "the attached-scroll civilian override must still suppress concussion"
    );
    let damage = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap();
    assert_eq!(damage.state, crate::sequence::SequenceState::InProgress);
    assert!(
        damage
            .orders
            .iter()
            .any(|order| order.order_type == OrderType::FallingHitUpright),
        "TranslateHitDamage must still author the hit reaction after the no-op concussion override"
    );
}

#[test]
fn conscious_hit_applies_ai_eye_status_synchronously() {
    let mut engine = make_engine();
    let null_slot = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
    engine
        .get_entity_mut(null_slot)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    let attacker = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
            y: 0.0,
            z: 0.0,
        },
        None,
    ));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    let damage = crate::sequence::SequenceElement::new(1, Command::ReceiveHitDamage, Some(victim));
    let seq_id = engine.launch_element(damage);
    let assets = assets_with_sword_profile(1, 50);

    engine.apply_hit_damage(
        &crate::sim_rng::test_context(),
        &assets,
        victim,
        Some(attacker),
        1,
        true,
        (seq_id, 0),
    );

    // EVENT_GOTHIT first runs StopAll (which queues Unfocus) and only
    // then sets EYES_DIE_OR_GET_UNCONSCIOUS. Exercise the complete
    // fixed-point drain: applying the tail eye write through the earlier
    // recovery channel made this pass immediately after Translate but
    // regress to LookForward once the queued Unfocus was drained.
    engine.drain_pending_for_npc(&crate::sim_rng::test_context(), victim, &assets);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_entity.npc_data().unwrap().eye_status,
        EyeStatus::DieOrGetUnconscious
    );
    assert_eq!(
        victim_entity
            .ai_controller()
            .unwrap()
            .outbox
            .recovery
            .set_eye_status,
        None,
        "the synchronous EVENT_GOTHIT write must not wait for the next owner slot"
    );
}

#[test]
fn conscious_lying_hit_applies_concussion_and_got_hit_before_terminating() {
    let mut engine = make_engine();
    let null_slot = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
    engine
        .get_entity_mut(null_slot)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    let attacker = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
            y: 0.0,
            z: 0.0,
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.set_posture(Posture::Lying);
        victim_entity.npc_data_mut().unwrap().eye_status = EyeStatus::Closed;
        victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
    }
    let damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveHitDamage,
        Some(victim),
        Some(attacker),
        0,
        3,
    );
    let seq_id = engine.launch_element(damage);
    let assets = assets_with_sword_profile(1, 50);

    engine.dispatch_receive_damage(&crate::sim_rng::test_context(), &assets, victim, seq_id, 0);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_entity.human_data().unwrap().concussion_of_the_brain,
        6,
        "AddConcussionOfTheBrain scales the incoming 3 by 100 / 50 life before the lying early exit"
    );
    assert_eq!(
        victim_entity.npc_data().unwrap().eye_status,
        EyeStatus::DieOrGetUnconscious,
        "EVENT_GOTHIT runs before the lying early exit"
    );
    let damage = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(damage.state, crate::sequence::SequenceState::Terminated);
    assert!(
        damage.orders.is_empty(),
        "an already-lying victim must not receive another fall order"
    );
}

#[test]
fn lying_arrow_victim_speaks_before_posture_termination() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let lying = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    let upright = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 40.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    engine
        .get_entity_mut(lying)
        .unwrap()
        .set_posture(Posture::Lying);
    for victim in [lying, upright] {
        engine
            .get_entity_mut(victim)
            .unwrap()
            .enemy_ai_mut()
            .unwrap()
            .hth_weapon_id = 1;
    }

    let mut assets = assets_with_sword_profile(1, 50);
    std::sync::Arc::make_mut(&mut assets.profile_manager).soldiers[0].exclamation_id = 0x5744_0000;

    for victim in [lying, upright] {
        let damage = crate::sequence::SequenceElement::new_damage(
            1,
            Command::ReceiveArrowDamage,
            Some(victim),
            Some(attacker),
            1,
            0,
        );
        let sequence = engine.launch_element(damage);
        engine.dispatch_receive_damage(&sim, &assets, victim, sequence, 0);
    }

    assert_eq!(
        engine
            .feedback
            .sound_sim
            .pending_exclamations
            .iter()
            .map(|pending| (pending.actor_id, pending.exclamation_id))
            .collect::<Vec<_>>(),
        vec![(lying.index(), crate::ai::Remark::Wounded as u16)],
        "the lying actor speaks first and its type-wide Wounded forbid rejects the later actor"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(lying),
        None,
        "the lying damage element still terminates after SayOuch"
    );
}

fn assets_with_sword_profile(energy: u16, max_distance: u16) -> LevelAssets {
    assets_with_sword_profile_effects(energy, max_distance, 4, 0)
}

fn assets_with_sword_profile_effects(
    energy: u16,
    max_distance: u16,
    cutting: u16,
    stunning: u16,
) -> LevelAssets {
    let mut profile_manager = crate::profiles::ProfileManager::new();
    let mut weapon = crate::profiles::HtHWeaponProfile::default();
    weapon.distance[crate::weapons::WeaponDistance::Maximal as usize] = max_distance;
    weapon.thrusts[SwordStrike::A as usize].energy = energy;
    weapon.thrusts[SwordStrike::A as usize].minimal_distance = 0;
    weapon.thrusts[SwordStrike::A as usize].maximal_distance = max_distance;
    weapon.thrusts[SwordStrike::A as usize].cutting = cutting;
    weapon.thrusts[SwordStrike::A as usize].stunning = stunning;
    profile_manager.hth_weapons.push(weapon);
    profile_manager
        .characters
        .push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..crate::profiles::CharacterProfile::default()
        });
    profile_manager
        .soldiers
        .push(crate::profiles::SoldierProfile {
            hth_weapon_id: 1,
            fighting: 20,
            ..crate::profiles::SoldierProfile::default()
        });

    LevelAssets {
        profile_manager: std::sync::Arc::new(profile_manager),
        ..LevelAssets::default()
    }
}

#[test]
fn postponed_non_entry_strike_translates_after_antagonist_dies() {
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
    let target = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 20.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    match engine.get_entity_mut(target).unwrap() {
        Entity::Pc(pc) => pc.pc.life_points = 0,
        _ => unreachable!("test target must remain a PC"),
    }

    for (command, strike, expected_order) in [
        (
            Command::SwordstrikeThrustB,
            SwordStrike::B,
            OrderType::StrikingStraightStrongSword,
        ),
        (
            Command::SwordstrikeThrustC,
            SwordStrike::C,
            OrderType::ExecutingSword,
        ),
    ] {
        let element = crate::sequence::SequenceElement::new_interaction(
            1,
            command,
            Some(attacker),
            Some(target),
        );
        let sequence = engine.launch_element(element);
        engine.dispatch_sword_strike(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            attacker,
            target,
            strike,
            sequence,
            0,
        );

        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap();
        assert_eq!(element.state, crate::sequence::SequenceState::InProgress);
        let order = element.current_order().unwrap();
        assert_eq!(order.order_type, expected_order);
        assert_eq!(order.antagonist, Some(target));
    }

    let thrust_a = crate::sequence::SequenceElement::new_interaction(
        1,
        Command::SwordstrikeThrustA,
        Some(attacker),
        Some(target),
    );
    let sequence = engine.launch_element(thrust_a);
    engine.dispatch_sword_strike(
        &crate::sim_rng::test_context(),
        &LevelAssets::default(),
        attacker,
        target,
        SwordStrike::A,
        sequence,
        0,
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Impossible,
        "Thrust A must retain CanEnterSwordfightWith's dead-target admission check"
    );
}

#[test]
fn thrust_a_accepts_an_existing_opponent_during_ordinary_door_transit() {
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D::default(),
        crate::position_interface::SectorHandle::new(42),
    ));
    let target = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
            ..WorldPoint3D::default()
        },
        crate::position_interface::SectorHandle::new(43),
    ));
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(target);
    {
        let target_entity = engine.get_entity_mut(target).unwrap();
        target_entity
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
        let target_actor = target_entity.actor_data_mut().unwrap();
        target_actor.active_door_pass = Some(crate::element::ActiveDoorPass {
            door_index: crate::gate::DoorIndex::new(7).expect("valid door index"),
            direct: true,
            position_direct: true,
            steps: std::collections::VecDeque::new(),
            triggers_fired: 0,
            current_action: OrderType::WalkingWithSword,
            current_reverse: false,
            saved_action_state: None,
        });
        target_entity.position_iface_mut().set_door_for_test(
            crate::position_interface::DoorHandle::new(7).expect("valid door index"),
        );
    }
    assert!(engine.get_entity(target).unwrap().is_in_door_transit());

    let assets = assets_with_sword_profile(1, 50);
    assert!(can_enter_swordfight_with(
        &engine.world.entities,
        attacker,
        target,
        &assets.profile_manager,
        &engine.world.fast_grid,
    ));

    let strike = crate::sequence::SequenceElement::new_interaction(
        1,
        Command::SwordstrikeThrustA,
        Some(attacker),
        Some(target),
    );
    let sequence = engine.launch_element(strike);
    engine.dispatch_sword_strike(
        &crate::sim_rng::test_context(),
        &assets,
        attacker,
        target,
        SwordStrike::A,
        sequence,
        0,
    );

    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap();
    assert_eq!(element.state, crate::sequence::SequenceState::InProgress);
    let order = element.current_order().unwrap();
    assert_eq!(order.order_type, OrderType::StrikingStraightSword);
    assert_eq!(order.antagonist, Some(target));
}

fn make_enemy_strike_pair(
    engine: &mut EngineInner,
    pending_consideration: bool,
) -> (EntityId, EntityId) {
    let attacker = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let target = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        soldier.actor.action_state = ActionState::WaitingSword;
        soldier.human.opponents.push(target);
        let crate::element::AiBrain::Enemy(ai) = &mut soldier.npc.ai_brain else {
            unreachable!()
        };
        ai.base.current_state = crate::ai::AiState::Attacking;
        ai.base.current_substate = crate::ai::Substate::AttackingSwordfight;
        ai.base.primary_target = target.index();
        ai.hth_weapon_id = 1;
        ai.pending_sword_strike_consideration = pending_consideration;
    }
    {
        let target_entity = engine.get_entity_mut(target).unwrap();
        target_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        target_entity
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
        // ProposeGoodSwordStrike owns a required sprite timing read for
        // its principal opponent.  Keep this shared synthetic duel
        // fixture structurally valid instead of relying on Sprite's
        // asset-less default row.
        target_entity.element_data_mut().sprite.scripts =
            std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                action_done: 1,
                frame_ids: vec![0, 1, 2],
                delays: vec![1, 1, 1],
                distances: vec![0, 0, 0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
                sound_ids: vec![0, 0, 0],
                ..Default::default()
            }]);
    }
    (attacker, target)
}

fn make_enemy_ai_hero_strike_pair(engine: &mut EngineInner) -> (EntityId, EntityId) {
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let target = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    for (owner, opponent, camp, authorize_strike) in [
        (attacker, target, crate::element::Camp::Custom(2), true),
        (target, attacker, crate::element::Camp::Custom(3), false),
    ] {
        let Entity::Pc(pc) = engine.get_entity_mut(owner).unwrap() else {
            unreachable!()
        };
        pc.actor.action_state = ActionState::WaitingSword;
        pc.human.opponents.push(opponent);
        pc.pc.cached_camp = camp;
        pc.pc.command_interface = crate::human_control::CommandInterface::None;
        pc.pc.mission_role = crate::human_control::MissionRole::Combatant;
        pc.pc.combat_stance = crate::human_control::CombatStance::Aggressive;
        let mut ai = crate::ai_enemy::EnemyAi::new(owner.index());
        ai.base.current_state = crate::ai::AiState::Attacking;
        ai.base.current_substate = crate::ai::Substate::AttackingSwordfight;
        ai.base.primary_target = opponent.index();
        ai.hth_weapon_id = 1;
        ai.pending_sword_strike_consideration = authorize_strike;
        pc.pc.ai = Some(Box::new(crate::element::AiActorData {
            ai_brain: crate::element::AiBrain::Enemy(Box::new(ai)),
            ..Default::default()
        }));
        pc.element.sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
            action_done: 1,
            frame_ids: vec![0, 1, 2],
            delays: vec![1, 1, 1],
            distances: vec![0, 0, 0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0, 0, 0],
            ..Default::default()
        }]);
        pc.element.sprite.conversion =
            std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }

    (attacker, target)
}

#[test]
fn enemy_ai_hero_consumes_enemy_sword_strike_proposal() {
    let mut engine = make_engine();
    let (attacker, _) = make_enemy_ai_hero_strike_pair(&mut engine);
    let mut assets = assets_with_sword_profile(7, 30);
    std::sync::Arc::make_mut(&mut assets.profile_manager).characters[0].fighting = 100;
    engine.control.rng = SimulationRng::with_original_replay(vec![0]);

    engine.with_simulation_context(|engine, sim| {
        engine.consume_pending_enemy_sword_attack_for(sim, &assets, attacker);
    });

    let ai = engine
        .get_entity(attacker)
        .and_then(Entity::enemy_ai)
        .expect("AI-controlled hero must retain its Enemy AI");
    assert!(ai.pending_special_strike);
    assert!(
        engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(attacker, Command::is_swordstrike),
        "the authorized AI-controlled hero proposal must launch a real strike"
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .current_outline,
        crate::element::OutlineColorName::Default,
        "attacking another AI-controlled hero must not use the player-warning hulk delay"
    );
}

#[test]
fn enemy_ai_hero_knockout_runs_shared_ai_cleanup() {
    let mut engine = make_engine();
    let (victim, _) = make_enemy_ai_hero_strike_pair(&mut engine);
    {
        let entity = engine.get_entity_mut(victim).unwrap();
        let human = entity.human_data_mut().unwrap();
        human.unconscious = true;
        human.concussion_of_the_brain = 25;
        let ai_actor = entity.ai_actor_data_mut().unwrap();
        ai_actor.alerted = true;
        ai_actor.maximal_detection_suspect = 40;
    }
    let assets = assets_with_sword_profile(7, 30);

    engine.apply_knockout_side_effects(
        &crate::sim_rng::test_context(),
        &assets,
        victim,
        true,
        true,
    );

    let ai_actor = engine
        .get_entity(victim)
        .and_then(Entity::ai_actor_data)
        .expect("AI-controlled hero retains AI actor data");
    assert_eq!(ai_actor.maximal_detection_suspect, 0);
    assert_eq!(
        ai_actor.eye_status,
        crate::element::EyeStatus::DieOrGetUnconscious
    );
    assert!(ai_actor.inform_my_friends);
}

#[test]
fn enemy_ai_hero_empty_opponent_evaluation_delivers_quit_event() {
    let mut engine = make_engine();
    let (owner, _) = make_enemy_ai_hero_strike_pair(&mut engine);
    engine
        .get_entity_mut(owner)
        .and_then(Entity::human_data_mut)
        .expect("AI-controlled hero has HumanData")
        .opponents
        .clear();
    let assets = assets_with_sword_profile(7, 30);

    engine.evaluate_opponents(&crate::sim_rng::test_context(), &assets, owner);

    let ai = engine
        .get_entity(owner)
        .and_then(Entity::ai_controller)
        .expect("AI-controlled hero retains its AI");
    assert_eq!(
        ai.current_substate,
        crate::ai::Substate::AttackingQuittingSwordfight
    );
    assert!(ai.ai_log.iter().any(|entry| {
        entry.line_type == crate::ai::LogLineType::Event
            && entry.info == crate::ai::StimulusType::EventQuitSwordfight as u16
    }));
}

#[test]
fn entering_attacking_swordfight_without_reconsideration_does_not_propose() {
    let mut engine = make_engine();
    let (attacker, _) = make_enemy_strike_pair(&mut engine, false);
    let assets = assets_with_sword_profile(7, 30);
    engine.control.rng = SimulationRng::with_original_replay(Vec::new());

    engine.with_simulation_context(|engine, sim| {
        engine.tick_enemy_sword_attacks(sim, &assets);
    });

    assert_eq!(engine.control.rng.original_replay_cursor(), Some(0));
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(attacker, Command::is_swordstrike),
        "entering AttackingSwordfight alone must not propose a strike"
    );
}

#[test]
fn special_strike_cancellation_closes_its_set_state_callback_boundary() {
    let mut engine = make_engine();
    let (attacker, _) = make_enemy_strike_pair(&mut engine, false);
    let assets = assets_with_sword_profile(7, 30);
    {
        let ai = engine
            .get_entity_mut(attacker)
            .and_then(Entity::enemy_ai_mut)
            .unwrap();
        ai.begin_special_strike();
        ai.base.outbox.reentrant.owner_work.clear();
    }

    engine.with_simulation_context(|engine, sim| {
        engine.tick_enemy_sword_attacks(sim, &assets);
    });

    let ai = engine
        .get_entity(attacker)
        .and_then(Entity::enemy_ai)
        .unwrap();
    assert!(!ai.pending_special_strike);
    assert_eq!(
        ai.base.current_substate,
        crate::ai::Substate::AttackingSwordfight
    );
    assert!(
        ai.base.outbox.reentrant.owner_work.is_empty(),
        "the cancellation SetState callback must run synchronously"
    );
}

#[test]
fn sword_strike_consideration_latch_is_one_shot_when_honour_rejects() {
    let mut engine = make_engine();
    let (attacker, target) = make_enemy_strike_pair(&mut engine, true);
    let assets = assets_with_sword_profile(7, 30);
    engine.control.rng = SimulationRng::with_original_replay(Vec::new());
    engine
        .get_entity_mut(target)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::Waiting;

    engine.with_simulation_context(|engine, sim| {
        engine.tick_enemy_sword_attacks(sim, &assets);
    });
    let cursor_after_first = engine.control.rng.original_replay_cursor().unwrap();
    assert_eq!(cursor_after_first, 0, "honour rejection precedes proposal");
    let pending_after_first = engine
        .get_entity(attacker)
        .and_then(Entity::enemy_ai)
        .unwrap()
        .pending_sword_strike_consideration;
    assert!(!pending_after_first, "the authorization must be one-shot");

    engine.with_simulation_context(|engine, sim| {
        engine.tick_enemy_sword_attacks(sim, &assets);
    });
    assert_eq!(
        engine.control.rng.original_replay_cursor(),
        Some(cursor_after_first),
        "the rejected, consumed latch must not retry next frame"
    );
}

#[test]
fn sword_strike_honour_reads_live_animation_not_action_change_history() {
    let mut engine = make_engine();
    let (attacker, target) = make_enemy_strike_pair(&mut engine, true);
    let assets = assets_with_sword_profile(7, 30);
    engine.control.rng = SimulationRng::with_original_replay(Vec::new());
    {
        let target = engine.get_entity_mut(target).unwrap();
        let actor = target.actor_data_mut().unwrap();
        actor.old_action = OrderType::Invalid;
        // The live animation is the installed order (the Original's
        // mpOrder), not the action-change history in `old_action`.
        actor.installed_order = Some(crate::element::InstalledActorOrder {
            order_id: std::num::NonZeroU32::new(1).unwrap(),
            order_type: OrderType::BeingHitSword,
        });
        target.element_data_mut().sprite.last_action = OrderType::BeingHitSword;
    }

    engine.with_simulation_context(|engine, sim| {
        engine.tick_enemy_sword_attacks(sim, &assets);
    });

    assert_eq!(
        engine.control.rng.original_replay_cursor(),
        Some(0),
        "GetAnimation recovery rejection must precede strike selection"
    );
    assert!(
        !engine
            .get_entity(attacker)
            .and_then(Entity::enemy_ai)
            .unwrap()
            .pending_sword_strike_consideration,
        "the rejected reconsideration remains a one-shot event"
    );
}

#[test]
fn owner_scoped_sword_consideration_precedes_later_owner_rng() {
    let mut engine = make_engine();
    let (attacker, _) = make_enemy_strike_pair(&mut engine, true);
    let assets = assets_with_sword_profile(7, 30);
    {
        let sprite = &mut engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .sprite;
        sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
            action_done: 0,
            frame_ids: vec![0],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
            ..Default::default()
        }]);
        sprite.conversion = std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }
    engine.control.rng = SimulationRng::with_original_replay(vec![85, 36]);

    let later_roll = engine.with_simulation_context(|engine, sim| {
        engine.consume_pending_enemy_sword_attack_for(sim, &assets, attacker);
        crate::sim_rng::u32(sim, crate::sim_rng::RngSite::ScriptRand, 0..100)
    });

    assert_eq!(
        later_roll, 36,
        "the reconsidering owner must consume its strike roll before a later owner's script"
    );
    assert_eq!(engine.control.rng.original_replay_cursor(), Some(2));
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(attacker, Command::is_swordstrike),
        "the first roll rejects the strike and must not borrow the later owner's lower roll"
    );
}

#[test]
fn event_authorized_parade_reconsideration_reaches_strike_proposal() {
    let mut engine = make_engine();
    let (attacker, _) = make_enemy_strike_pair(&mut engine, true);
    let assets = assets_with_sword_profile(7, 30);
    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        soldier.npc.ai_brain.base_mut().unwrap().current_substate =
            crate::ai::Substate::AttackingSwordfightParade;
        soldier.human.tiredness = TIREDNESS_WEAK_THRESHOLD;
        let crate::element::AiBrain::Enemy(ai) = &mut soldier.npc.ai_brain else {
            unreachable!()
        };
        ai.next_sword_strike_frame = u32::MAX;
        soldier.element.sprite.scripts =
            std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                action_done: 0,
                frame_ids: vec![0],
                delays: vec![1],
                distances: vec![0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
                sound_ids: vec![0],
                ..Default::default()
            }]);
        soldier.element.sprite.conversion =
            std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }
    engine.control.rng = SimulationRng::with_original_replay(vec![85]);

    engine.with_simulation_context(|engine, sim| {
        engine.consume_pending_enemy_sword_attack_for(sim, &assets, attacker);
    });

    assert_eq!(
        engine.control.rng.original_replay_cursor(),
        Some(1),
        "ReconsiderSwordfight already passed Original's state, cooldown, and tiredness gates"
    );
    assert!(
        !engine
            .get_entity(attacker)
            .and_then(Entity::enemy_ai)
            .unwrap()
            .pending_sword_strike_consideration
    );
}

#[test]
fn deferred_combat_insult_depends_on_inline_strike_result() {
    fn install_minimal_sprite(engine: &mut EngineInner, attacker: EntityId) {
        let sprite = &mut engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .sprite;
        sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
            action_done: 0,
            frame_ids: vec![0],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
            ..Default::default()
        }]);
        sprite.conversion = std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }

    let assets = assets_with_sword_profile(7, 30);

    // A failed proposal leaves Original in ordinary Swordfight and the
    // caller's following statement says CombatInsult.
    let mut rejected = make_engine();
    let (rejected_attacker, _) = make_enemy_strike_pair(&mut rejected, true);
    install_minimal_sprite(&mut rejected, rejected_attacker);
    rejected
        .get_entity_mut(rejected_attacker)
        .and_then(Entity::enemy_ai_mut)
        .unwrap()
        .pending_combat_insult_after_strike_consideration = true;
    rejected.control.rng = SimulationRng::with_original_replay(vec![85]);
    rejected.with_simulation_context(|engine, sim| {
        engine.consume_pending_enemy_sword_attack_for(sim, &assets, rejected_attacker);
    });
    let rejected_ai = rejected
        .get_entity(rejected_attacker)
        .and_then(Entity::enemy_ai)
        .unwrap();
    assert!(
        rejected_ai
            .base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(
                work,
                crate::ai::AiOwnerWork::Speech(attempt)
                    if attempt.remark == crate::ai::Remark::CombatInsult
            ))
    );

    // A successful proposal changes Original to SpecialStrike before the
    // same following statement tests the substate, suppressing the bark.
    let mut accepted = make_engine();
    let (accepted_attacker, _) = make_enemy_strike_pair(&mut accepted, true);
    install_minimal_sprite(&mut accepted, accepted_attacker);
    accepted
        .get_entity_mut(accepted_attacker)
        .and_then(Entity::enemy_ai_mut)
        .unwrap()
        .pending_combat_insult_after_strike_consideration = true;
    accepted.control.rng = SimulationRng::with_original_replay(vec![0]);
    accepted.with_simulation_context(|engine, sim| {
        engine.consume_pending_enemy_sword_attack_for(sim, &assets, accepted_attacker);
    });
    let accepted_ai = accepted
        .get_entity(accepted_attacker)
        .and_then(Entity::enemy_ai)
        .unwrap();
    assert!(accepted_ai.pending_special_strike);
    assert!(
        !accepted_ai
            .base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(
                work,
                crate::ai::AiOwnerWork::Speech(attempt)
                    if attempt.remark == crate::ai::Remark::CombatInsult
            ))
    );
}

#[test]
fn reactive_counterstrike_uses_difficulty_modified_soldier_fighting_ability() {
    let mut engine = make_engine();
    engine.control.sim_config.difficulty = crate::player_profile::DifficultyLevel::Hard;
    let (victim, attacker) = make_enemy_strike_pair(&mut engine, false);
    for actor in [victim, attacker] {
        let sprite = &mut engine
            .get_entity_mut(actor)
            .unwrap()
            .element_data_mut()
            .sprite;
        sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
            action_done: 0,
            frame_ids: vec![0],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
            ..Default::default()
        }]);
        sprite.conversion = std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }
    let mut assets = assets_with_sword_profile(7, 30);
    std::sync::Arc::get_mut(&mut assets.profile_manager)
        .unwrap()
        .soldiers[0]
        .fighting = 40;
    {
        let ai = engine
            .get_entity_mut(victim)
            .and_then(Entity::enemy_ai_mut)
            .unwrap();
        ai.known_enemy_strike_1 = Some(SwordStrike::A);
    }

    // The replay victim is still performing a selected smalltalk parry
    // when the reactive counterstrike replaces it. StopAll must mark the
    // interrupted element's condolence as coming from Halt; otherwise its
    // later EventDone immediately leaves the new SpecialStrike substate.
    let old_parry =
        engine
            .orders
            .sequence_manager
            .launch_element(crate::sequence::SequenceElement::new(
                1,
                Command::ParrySmalltalkLeft,
                Some(victim),
            ));
    engine
        .orders
        .sequence_manager
        .element_in_progress(old_parry, 0);

    // 65 rejects raw fighting 40 and produces a parade, but Hard's
    // Lacklandist modifier raises it to 80, allowing the counterstrike.
    engine.control.rng = SimulationRng::with_original_replay(vec![65]);
    engine.with_simulation_context(|engine, sim| {
        engine.consider_to_begin_parade(
            sim,
            &assets,
            victim,
            attacker,
            Some(SwordStrike::A),
            SwordStrike::A,
        );
        engine.dispatch_condolations(sim, &assets);
    });

    let ai = engine
        .get_entity(victim)
        .and_then(Entity::enemy_ai)
        .unwrap();
    assert!(ai.pending_special_strike);
    assert_eq!(
        ai.base.current_substate,
        crate::ai::Substate::AttackingSwordfightSpecialStrike
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(victim, Command::is_swordstrike)
    );
}

#[test]
fn reactive_strike_recognition_uses_command_not_replacement_animation() {
    fn run(remembered: SwordStrike, busy: bool) -> (usize, crate::ai::Substate, usize) {
        let mut engine = make_engine();
        let (victim, attacker) = make_enemy_strike_pair(&mut engine, false);
        engine.world.fast_grid_mut().size_map(4, 4);
        engine.world.fast_grid_mut().allocate_layers(1);
        engine.world.fast_grid_mut().add_sector(
            crate::fast_find_grid::GridSector {
                points: vec![
                    crate::coordinates::MapPoint::new(0.0, 0.0),
                    crate::coordinates::MapPoint::new(256.0, 0.0),
                    crate::coordinates::MapPoint::new(256.0, 256.0),
                    crate::coordinates::MapPoint::new(0.0, 256.0),
                ],
                bounding_box: crate::coordinates::MapBBox::from_coords(0.0, 0.0, 256.0, 256.0),
                sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
                layer: 0,
                sector_number: crate::sector::SectorNumber::new(0),
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
            },
            0,
        );
        {
            let victim_element = engine.get_entity_mut(victim).unwrap().element_data_mut();
            victim_element.set_position(WorldPoint3D::new(100.0, 100.0, 0.0));
            victim_element.set_sector(crate::position_interface::SectorHandle::new(0));
            victim_element.sprite.position_iface.set_move_box(
                crate::coordinates::MoveBox::from_coords(-5.0, -5.0, 5.0, 5.0),
            );
        }
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            // Keep the attacker inside the H thrust's 50-unit desired
            // push-back distance.  At 80 units Original returns the
            // victim's current point as a successful step-back goal;
            // synchronous GoTo completion then restores ordinary
            // swordfight state before WarnForStrike returns, so that
            // fixture cannot distinguish H's PushAside geometry.
            .set_position(WorldPoint3D::new(130.0, 100.0, 0.0));
        for actor in [victim, attacker] {
            let sprite = &mut engine
                .get_entity_mut(actor)
                .unwrap()
                .element_data_mut()
                .sprite;
            let mut scripts = vec![
                crate::sprite_script::SpriteScript {
                    action_done: 10,
                    frame_ids: (0..16).collect(),
                    delays: vec![1; 16],
                    distances: vec![0; 16],
                    offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 16],
                    sound_ids: vec![0; 16],
                    ..Default::default()
                };
                16
            ];
            // The incoming H animation has ten frames left, while the
            // victim's authored parry transition starts in three. This
            // satisfies Original's strict startup deadline and lets the
            // test observe the later PushAside geometry branch.
            scripts[1].action_done = 3;
            sprite.scripts = std::sync::Arc::new(scripts);
            let mut conversion = vec![0; crate::sprite_script::NONANIMATION_END];
            conversion[OrderType::TransitionWaitingSwordParryingSword as usize] = 1;
            sprite.conversion = std::sync::Arc::new(conversion);
        }

        let mut old_movement = crate::sequence::SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(victim),
            OrderType::WalkingWithSword,
        );
        old_movement.priority = crate::sequence::SequencePriority::Normal;
        let old_sequence = engine.launch_element(old_movement);
        let old_order =
            engine.push_new_order(old_sequence, 0, OrderType::WalkingWithSword, 90.0, 100.0);
        engine
            .orders
            .sequence_manager
            .element_in_progress(old_sequence, 0);
        {
            let actor = engine
                .get_entity_mut(victim)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.action_state = ActionState::MovingSword;
            actor.installed_order = Some(crate::element::InstalledActorOrder {
                order_id: old_order,
                order_type: OrderType::WalkingWithSword,
            });
            actor.active_movement = crate::movement::ActiveMovement::new(old_sequence, 0);
        }

        // The selected request remains F while its installed replacement
        // row is H, exactly separating Original GetCommand from
        // GetAnimation at WarnForStrike.
        let strike_element = crate::sequence::SequenceElement::new_interaction(
            1,
            Command::SwordstrikeThrustF,
            Some(attacker),
            Some(victim),
        );
        let strike_sequence = engine.launch_element(strike_element);
        let strike_order = engine.push_new_order(
            strike_sequence,
            0,
            OrderType::StrikingRoundLeftSword,
            100.0,
            100.0,
        );
        engine
            .orders
            .sequence_manager
            .element_in_progress(strike_sequence, 0);
        {
            let attacker_entity = engine.get_entity_mut(attacker).unwrap();
            let actor = attacker_entity.actor_data_mut().unwrap();
            actor.action_state = ActionState::WaitingSword;
            actor.installed_order = Some(crate::element::InstalledActorOrder {
                order_id: strike_order,
                order_type: OrderType::StrikingRoundLeftSword,
            });
            let sprite = &mut attacker_entity.element_data_mut().sprite;
            sprite.current_row = 0;
            sprite.current_frame = 0;
            sprite.frame_count = 0;
            sprite.action_done_frame = 10;
            sprite.action_done_counter = 1;
            sprite.last_action = OrderType::StrikingRoundLeftSword;
        }

        let mut assets = assets_with_sword_profile(7, 30);
        let profiles = std::sync::Arc::get_mut(&mut assets.profile_manager).unwrap();
        profiles.soldiers[0].fighting = 50;
        profiles.hth_weapons[0].thrusts[SwordStrike::H as usize].kind =
            crate::profiles::WeaponThrustKind::PushAside;
        profiles.hth_weapons[0].thrusts[SwordStrike::H as usize].maximal_distance = 30;
        let ai = engine
            .get_entity_mut(victim)
            .and_then(Entity::enemy_ai_mut)
            .unwrap();
        ai.known_enemy_strike_1 = Some(remembered);
        if busy {
            ai.base.locks_flag_field = crate::ai::AiLockFlags::BUSY;
        }

        // 65 selects parade at ability 50. Only the H animation's
        // PushAside geometry can turn that parade into a step-back.
        engine.control.rng = SimulationRng::with_original_replay(vec![85]);
        engine.with_simulation_context(|engine, sim| {
            engine.warn_for_strike(sim, &assets, attacker, &[victim], SwordStrike::H);
        });
        let ai = engine
            .get_entity(victim)
            .and_then(Entity::enemy_ai)
            .unwrap();
        if busy {
            assert_eq!(
                ai.base.stimulus_queue[0].stimulus_type,
                crate::ai::StimulusType::EventSwordStrike,
            );
            assert_eq!(
                ai.base.stimulus_queue[0].info,
                crate::ai::StimulusInfo::Human(attacker.index()),
                "queued EVENT_SWORDSTRIKE must retain the attacking human"
            );
        }
        (
            engine.control.rng.original_replay_cursor().unwrap(),
            ai.base.current_substate,
            ai.base.stimulus_queue.len(),
        )
    }

    assert_eq!(
        run(SwordStrike::F, false),
        (1, crate::ai::Substate::AttackingSwordfightStepBack, 0,),
        "command F must admit the proposal while animation H supplies PushAside geometry"
    );
    assert_eq!(
        run(SwordStrike::H, false),
        (0, crate::ai::Substate::AttackingSwordfight, 0),
        "remembering only replacement H must not admit selected command F"
    );
    let locked = run(SwordStrike::F, true);
    assert_eq!(
        locked.0, 0,
        "BUSY StartThink must not reach the proposal RNG"
    );
    assert_eq!(
        locked.1,
        crate::ai::Substate::AttackingSwordfight,
        "BUSY warning must not launch a parade or counter-strike"
    );
    assert_eq!(locked.2, 1);
}

#[test]
fn reactive_zero_distance_step_back_completes_before_returning() {
    for rider in [false, true] {
        let mut engine = make_engine();
        let (victim, attacker) = make_enemy_strike_pair(&mut engine, false);
        let Entity::Soldier(victim_soldier) = engine.get_entity_mut(victim).unwrap() else {
            unreachable!()
        };
        victim_soldier.soldier.rider = rider;
        engine.world.fast_grid_mut().size_map(4, 4);
        engine.world.fast_grid_mut().allocate_layers(1);
        let sector_points = vec![
            crate::coordinates::MapPoint::new(0.0, 0.0),
            crate::coordinates::MapPoint::new(256.0, 0.0),
            crate::coordinates::MapPoint::new(256.0, 256.0),
            crate::coordinates::MapPoint::new(0.0, 256.0),
        ];
        engine.world.fast_grid_mut().add_sector(
            crate::fast_find_grid::GridSector {
                points: sector_points,
                bounding_box: crate::coordinates::MapBBox::from_coords(0.0, 0.0, 256.0, 256.0),
                sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
                layer: 0,
                sector_number: crate::sector::SectorNumber::new(0),
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
            },
            0,
        );
        {
            let victim_element = engine.get_entity_mut(victim).unwrap().element_data_mut();
            victim_element.set_position(WorldPoint3D::new(100.0, 100.0, 0.0));
            victim_element.set_sector(crate::position_interface::SectorHandle::new(0));
            victim_element.sprite.position_iface.set_move_box(
                crate::coordinates::MoveBox::from_coords(-5.0, -5.0, 5.0, 5.0),
            );
        }
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .set_position(WorldPoint3D::new(180.0, 100.0, 0.0));
        for actor in [victim, attacker] {
            let sprite = &mut engine
                .get_entity_mut(actor)
                .unwrap()
                .element_data_mut()
                .sprite;
            sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                action_done: 0,
                frame_ids: vec![0],
                delays: vec![1],
                distances: vec![0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
                sound_ids: vec![0],
                ..Default::default()
            }]);
            sprite.conversion =
                std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
        }

        // The replay victim is already moving with a sword. StopAll calls
        // StopMovement, whose switch preserves only ordinary upright or
        // crouched locomotion; WalkingWithSword falls through to
        // INTERRUPTED and clears mpOrder. The immediately following GoTo
        // therefore reads NONANIMATION_END. Since the attacker is already
        // beyond the desired separation, the proposed step-back goal is
        // the victim's current point and GoTo must take its synchronous
        // already-at-destination exit without publishing a replacement
        // movement.
        let mut old_movement = crate::sequence::SequenceElement::new_movement(
            1,
            Command::MoveOk,
            Some(victim),
            OrderType::WalkingWithSword,
        );
        old_movement.priority = crate::sequence::SequencePriority::Normal;
        let old_sequence = engine.launch_element(old_movement);
        let old_order =
            engine.push_new_order(old_sequence, 0, OrderType::WalkingWithSword, 90.0, 100.0);
        engine
            .orders
            .sequence_manager
            .element_in_progress(old_sequence, 0);
        {
            let actor = engine
                .get_entity_mut(victim)
                .unwrap()
                .actor_data_mut()
                .unwrap();
            actor.action_state = ActionState::MovingSword;
            actor.installed_order = Some(crate::element::InstalledActorOrder {
                order_id: old_order,
                order_type: OrderType::WalkingWithSword,
            });
            actor.active_movement = crate::movement::ActiveMovement::new(old_sequence, 0);
        }
        let mut assets = assets_with_sword_profile(7, 30);
        let profiles = std::sync::Arc::get_mut(&mut assets.profile_manager).unwrap();
        profiles.soldiers[0].fighting = 50;
        let incoming_thrust = &mut profiles.hth_weapons[0].thrusts[SwordStrike::A as usize];
        incoming_thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
        incoming_thrust.maximal_distance = 30;
        {
            let ai = engine
                .get_entity_mut(victim)
                .and_then(Entity::enemy_ai_mut)
                .unwrap();
            ai.known_enemy_strike_1 = Some(SwordStrike::A);
        }

        // 65 rejects an offensive response at fighting ability 50, selecting
        // the parade path. A push-aside strike turns that parade into a
        // step-back GoTo, which Original launches synchronously before this
        // callback returns.
        engine.control.rng = SimulationRng::with_original_replay(vec![65]);
        engine.with_simulation_context(|engine, sim| {
            engine.consider_to_begin_parade(
                sim,
                &assets,
                victim,
                attacker,
                Some(SwordStrike::A),
                SwordStrike::A,
            );
        });

        let ai = engine
            .get_entity(victim)
            .and_then(Entity::enemy_ai)
            .unwrap();
        assert_eq!(
            ai.base.current_substate,
            crate::ai::Substate::AttackingSwordfight,
            "an already-at-goal step-back synchronously handles EVENT_REACHPOINT"
        );
        assert!(ai.base.timer_is_running);
        assert_eq!(ai.base.when_does_timer_ring, 20);
        assert!(
            ai.base
                .last_goto_flags
                .contains(crate::ai::GotoFlags::SWORD)
        );
        assert_eq!(
            ai.base.last_goto_flags.contains(crate::ai::GotoFlags::RUN),
            !rider,
            "Original's rider step-back omits GOTO_RUN"
        );
        let owned_elements: Vec<_> = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .filter(|element| element.owner == Some(victim))
            .map(|element| {
                (
                    element.command,
                    element.state,
                    element.current_order().map(|order| order.order_type),
                )
            })
            .collect();
        assert!(
            !owned_elements
                .iter()
                .any(|(command, _, _)| *command == Command::EnterSwordfight),
            "a soldier already in WaitingSword must not receive a spurious raise-sword prefix: {owned_elements:?}"
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .live_element_for_actor_matching(victim, |element| {
                    matches!(element.command, Command::Move | Command::MoveOk)
                })
                .is_none(),
            "the already-at-goal GoTo must not leave a stale movement that can displace a same-frame smalltalk parry: {owned_elements:?}"
        );
    }
}

fn assets_with_nonstraight_profile(
    strike: SwordStrike,
    kind: crate::profiles::WeaponThrustKind,
) -> LevelAssets {
    let mut profile_manager = crate::profiles::ProfileManager::new();
    let mut weapon = crate::profiles::HtHWeaponProfile::default();
    let thrust = &mut weapon.thrusts[strike as usize];
    thrust.kind = kind;
    thrust.direction = crate::profiles::WeaponThrustDirection::LeftToRight;
    thrust.minimal_distance = 0;
    thrust.maximal_distance = 100;
    thrust.initial_angle = 0;
    thrust.final_angle = 180;
    thrust.rotation_angle = 90;
    thrust.repulsion = 100;
    thrust.cutting = 100;
    profile_manager.hth_weapons.push(weapon);
    profile_manager
        .characters
        .push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..crate::profiles::CharacterProfile::default()
        });
    profile_manager
        .soldiers
        .push(crate::profiles::SoldierProfile {
            hth_weapon_id: 1,
            ..crate::profiles::SoldierProfile::default()
        });

    LevelAssets {
        profile_manager: std::sync::Arc::new(profile_manager),
        ..LevelAssets::default()
    }
}

#[test]
fn civilian_health_counts_toward_round_strike_and_warcry() {
    let mut engine = make_engine();
    let (attacker, _) = make_enemy_strike_pair(&mut engine, true);
    {
        let sprite = &mut engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .sprite;
        sprite.scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
            action_done: 0,
            frame_ids: vec![0],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
            ..Default::default()
        }]);
        sprite.conversion = std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }
    engine.add_entity(make_civilian(WorldPoint3D {
        x: 15.0,
        y: 100.0,
        z: 0.0,
    }));

    let mut assets = assets_with_nonstraight_profile(
        SwordStrike::H,
        crate::profiles::WeaponThrustKind::TrueCircle,
    );
    std::sync::Arc::make_mut(&mut assets.profile_manager).soldiers[0].fighting = 100;
    engine.control.rng = SimulationRng::with_original_replay(vec![0]);

    engine.with_simulation_context(|engine, sim| {
        engine.consume_pending_enemy_sword_attack_for(sim, &assets, attacker);
    });

    assert!(
        engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(attacker, |command| {
                command == Command::SwordstrikeThrustH
            }),
        "the PC and civilian are two live round-strike victims"
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .and_then(Entity::enemy_ai)
            .expect("fixture attacker keeps Enemy AI")
            .base
            .current_remark,
        crate::ai::Remark::Warcry,
        "Original says REMARK_WARCRY when selecting thrust H"
    );
}

fn soldier_life(engine: &EngineInner, soldier_id: EntityId) -> i16 {
    match engine
        .get_entity(soldier_id)
        .expect("test soldier must remain present")
    {
        Entity::Soldier(soldier) => soldier.npc.life_points,
        _ => panic!("test victim must be a soldier"),
    }
}

fn install_test_melee_order(
    engine: &mut EngineInner,
    attacker: EntityId,
    target: EntityId,
    strike: SwordStrike,
    past_action_done: bool,
) -> crate::engine::tick::MeleeOwnerSelection {
    let order_type = strike_to_animation(strike);
    let sequence = engine.orders.sequence_manager.launch_element(
        crate::sequence::SequenceElement::new_interaction(
            1,
            strike.to_command(),
            Some(attacker),
            Some(target),
        ),
    );
    let order_id = engine.orders.allocate_order_id();
    let mut order = crate::order::Order::new(order_type, 0.0, 0.0, order_id);
    order.antagonist = Some(target);
    engine
        .orders
        .sequence_manager
        .push_order_on(sequence, 0, order);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    let script = crate::sprite_script::SpriteScript {
        action_id: order_type as u16,
        action_done: 1,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0, 0, 0],
        ..Default::default()
    };
    let mut conversion =
        vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
    conversion[order_type as usize] = 0;
    let entity = engine.get_entity_mut(attacker).unwrap();
    let position_iface = entity.element_data().sprite.position_iface.clone();
    let mut sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    sprite.position_iface = position_iface;
    entity.element_data_mut().sprite = sprite;
    let direction = entity.element_data().direction() as u16;
    let sim = crate::sim_rng::test_context();
    let sprite = &mut entity.element_data_mut().sprite;
    assert_eq!(
        sprite.perform_action(
            &sim,
            Some(order_id),
            order_type,
            direction,
            crate::sprite::FrameProgression::Default,
            false,
        ),
        crate::sprite::MotionState::Start
    );
    while sprite.frames_from_now_till_action_done() > 0 {
        assert_eq!(
            sprite.perform_action(
                &sim,
                Some(order_id),
                order_type,
                direction,
                crate::sprite::FrameProgression::Default,
                false,
            ),
            crate::sprite::MotionState::InProgress
        );
    }
    if past_action_done {
        assert_eq!(
            sprite.perform_action(
                &sim,
                Some(order_id),
                order_type,
                direction,
                crate::sprite::FrameProgression::Default,
                false,
            ),
            crate::sprite::MotionState::Done
        );
    }
    crate::engine::tick::MeleeOwnerSelection {
        seq_id: sequence,
        elem_idx: 0,
        order_id,
    }
}

#[test]
fn completed_missed_sword_strike_adds_tiredness_once() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let target = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 500.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets = assets_with_sword_profile(7, 30);

    install_test_melee_order(&mut engine, attacker, target, SwordStrike::A, true);

    engine.tick_melee_strikes(sim, &assets);

    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .tiredness,
        7,
        "out-of-range strikes still cost tiredness when the active strike terminates"
    );
}

#[test]
fn empty_true_circle_sweep_advances_until_rotation_complete() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    if let Some(actor) = engine.get_entity_mut(attacker).unwrap().actor_data_mut() {
        actor.sweep_state = Some(crate::movement::SweepState {
            pending_victims: Vec::new(),
            current_angle: 0.0,
            final_angle: std::f32::consts::PI * 2.0,
            rotation_per_frame: std::f32::consts::PI,
            direction: crate::profiles::WeaponThrustDirection::LeftToRight,
            strike: SwordStrike::H,
            strike_kind: crate::profiles::WeaponThrustKind::TrueCircle,
            ..Default::default()
        });
    }

    engine.tick_sweep_for(sim, &LevelAssets::default(), attacker, false);
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .is_some(),
        "true-circle sweep with no victims must still rotate instead of clearing immediately"
    );

    engine.tick_sweep_for(sim, &LevelAssets::default(), attacker, false);
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .is_some(),
        "the tick that reaches the final angle must retain it for the terminal Execute call"
    );

    engine.tick_sweep_for(sim, &LevelAssets::default(), attacker, false);
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .is_none(),
        "empty true-circle sweep should clear after presenting its terminal angle"
    );
}

#[test]
fn circle_done_initialization_advances_without_rotating_or_hitting() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 90.0,
            z: 0.0,
        },
        None,
    ));
    let assets = assets_with_nonstraight_profile(
        SwordStrike::F,
        crate::profiles::WeaponThrustKind::TrueHalfCircle,
    );

    engine.initialize_sweep(
        &assets,
        attacker,
        SwordStrike::F,
        Some(1),
        crate::profiles::WeaponThrustKind::TrueHalfCircle,
        vec![victim],
    );
    let initial_angle = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .unwrap()
        .current_angle;

    engine.tick_sweep_for(sim, &assets, attacker, true);

    let attacker_entity = engine.get_entity(attacker).unwrap();
    let sweep = attacker_entity
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("true half-circle must retain its initialized sweep");
    assert!(
        (sweep.current_angle - (initial_angle + std::f32::consts::FRAC_PI_2)).abs() < f32::EPSILON,
        "ExecuteCircleSwordStrike advances its internal angle at the DONE-call tail"
    );
    assert_eq!(
        attacker_entity.element_data().direction(),
        0,
        "the DONE call must not rotate the true-circle sprite"
    );
    assert_eq!(
        soldier_life(&engine, victim),
        50,
        "the DONE effect branch only initializes victims and cannot hit"
    );
}

#[test]
fn lateral_done_initialization_does_not_advance_or_hit() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 90.0,
            z: 0.0,
        },
        None,
    ));
    let assets =
        assets_with_nonstraight_profile(SwordStrike::D, crate::profiles::WeaponThrustKind::Lateral);
    let selected = install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, false);

    let phase = engine.tick_nonstraight_melee_for(sim, &assets, attacker, selected);
    assert!(
        phase == strikes::SweepTickPhase::Initialized,
        "the lateral DONE branch must initialize a sweep"
    );
    let initial_current = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .unwrap()
        .current_angle;
    engine.tick_sweep_for(sim, &assets, attacker, true);

    let current = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("lateral victim must remain pending after DONE")
        .current_angle;
    assert_eq!(
        current, initial_current,
        "ExecuteLateralSwordStrike uses an else-if, so DONE cannot also run its IN_PROGRESS advance"
    );
    assert_eq!(
        soldier_life(&engine, victim),
        50,
        "lateral initialization cannot hit until a later Hourglass"
    );
}

#[test]
fn lateral_done_keeps_actor_scan_order_and_does_not_recover_out_of_arc_antagonist() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    // Facing south (sector 8), thrust D covers sectors 4..=9. Keep the
    // valid victims on either side of the out-of-arc antagonist in actor
    // creation order so the assertion also guards the collector FIFO.
    let first_in_arc = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let antagonist = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: -20.0,
            y: 120.0,
            z: 0.0,
        },
        None,
    ));
    let second_in_arc = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 120.0,
            z: 0.0,
        },
        None,
    ));
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(8);

    let mut assets =
        assets_with_nonstraight_profile(SwordStrike::D, crate::profiles::WeaponThrustKind::Lateral);
    let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::D as usize];
    thrust.initial_angle = 90;
    thrust.final_angle = 22;
    thrust.rotation_angle = 45;

    assert_eq!(
        crate::position_interface::vector_to_sector_0_to_15(-20.0, 20.0),
        10,
        "the interaction antagonist must be outside thrust D's actor-scan arc"
    );
    let selected =
        install_test_melee_order(&mut engine, attacker, antagonist, SwordStrike::D, false);

    assert_eq!(
        engine.tick_nonstraight_melee_for(sim, &assets, attacker, selected),
        strikes::SweepTickPhase::Initialized
    );
    let pending = &engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("the in-arc actors initialize the lateral sweep")
        .pending_victims;
    assert_eq!(pending, &[first_in_arc, second_in_arc]);
    assert!(!pending.contains(&antagonist));
}

#[test]
fn lateral_seed_uses_ground_direction_instead_of_map_direction() {
    let mut engine = make_engine();
    // Both actors have the same ground Y, so the victim is due west
    // (sector 12).  Its lower elevation projects six units south in map
    // space, which moves the same vector into sector 11.
    let attacker = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let victim = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 100.0,
        });
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D {
            x: -15.0,
            y: 100.0,
            z: 94.0,
        });
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(9);

    let mut assets =
        assets_with_nonstraight_profile(SwordStrike::E, crate::profiles::WeaponThrustKind::Lateral);
    let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::E as usize];
    thrust.direction = crate::profiles::WeaponThrustDirection::RightToLeft;
    thrust.initial_angle = 45;
    thrust.final_angle = 90;

    let attacker_map = engine
        .get_entity(attacker)
        .unwrap()
        .element_data()
        .position_map();
    let victim_map = engine
        .get_entity(victim)
        .unwrap()
        .element_data()
        .position_map();
    let attacker_ground = engine
        .get_entity(attacker)
        .unwrap()
        .element_data()
        .position();
    let victim_ground = engine.get_entity(victim).unwrap().element_data().position();
    assert_eq!(
        crate::position_interface::vector_to_sector_0_to_15(
            victim_ground.x - attacker_ground.x,
            victim_ground.y - attacker_ground.y,
        ),
        12,
    );
    assert_eq!(
        crate::position_interface::vector_to_sector_0_to_15(
            victim_map.x - attacker_map.x,
            victim_map.y - attacker_map.y,
        ),
        11,
        "the old map-space seed would admit this victim at the arc boundary"
    );

    let victims = engine.execute_multi_target_strike(&assets, attacker, SwordStrike::E, Some(1));
    assert!(
        victims.is_empty(),
        "Original seeds lateral victims from ground-space direction, where this actor is sector 12 and outside sectors 5..=11"
    );
}

#[test]
fn interrupted_lateral_sweep_is_retained_and_rebound_by_next_strike() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let unreached_victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: -10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    let mut profile_manager = crate::profiles::ProfileManager::new();
    let mut weapon = crate::profiles::HtHWeaponProfile::default();
    let retained = &mut weapon.thrusts[SwordStrike::D as usize];
    retained.kind = crate::profiles::WeaponThrustKind::Lateral;
    retained.direction = crate::profiles::WeaponThrustDirection::RightToLeft;
    retained.minimal_distance = 0;
    retained.maximal_distance = 100;
    retained.rotation_angle = 5;
    retained.cutting = 1;
    let replacement = &mut weapon.thrusts[SwordStrike::E as usize];
    replacement.kind = crate::profiles::WeaponThrustKind::Lateral;
    replacement.direction = crate::profiles::WeaponThrustDirection::LeftToRight;
    replacement.minimal_distance = 0;
    replacement.maximal_distance = 100;
    replacement.rotation_angle = 90;
    replacement.cutting = 100;
    profile_manager.hth_weapons.push(weapon);
    profile_manager
        .characters
        .push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..crate::profiles::CharacterProfile::default()
        });
    profile_manager
        .soldiers
        .push(crate::profiles::SoldierProfile {
            hth_weapon_id: 1,
            ..crate::profiles::SoldierProfile::default()
        });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profile_manager),
        ..LevelAssets::default()
    };

    let retained_selection =
        install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, true);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sweep_state = Some(crate::movement::SweepState {
        pending_victims: vec![victim, unreached_victim],
        initial_angle: 0.0,
        current_angle: 0.0,
        final_angle: -std::f32::consts::PI,
        rotation_per_frame: -5.0_f32.to_radians(),
        direction: crate::profiles::WeaponThrustDirection::RightToLeft,
        strike: SwordStrike::D,
        attacker_profile_idx: Some(1),
        strike_kind: crate::profiles::WeaponThrustKind::Lateral,
    });

    engine.stop_owner_active_mechanics(attacker);
    let retained_after_interrupt = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("interrupting the D sequence must retain its human-owned sweep");
    assert_eq!(retained_after_interrupt.strike, SwordStrike::D);
    assert_eq!(
        retained_after_interrupt.pending_victims,
        vec![victim, unreached_victim]
    );

    let replacement_order_id = engine.orders.allocate_order_id();
    let replacement_element = engine
        .orders
        .sequence_manager
        .get_element_mut(retained_selection.seq_id, retained_selection.elem_idx)
        .expect("retained strike element exists");
    replacement_element.command = SwordStrike::E.to_command();
    let replacement_order = replacement_element
        .orders
        .front_mut()
        .expect("retained strike order exists");
    replacement_order.order_type = strike_to_animation(SwordStrike::E);
    replacement_order.antagonist = Some(victim);
    replacement_order.reseed_id(replacement_order_id);
    // A live replacement strike is published as the actor's installed
    // order at Instruct; Execute's Start arm resolves the strike from
    // that installed animation, not from the sequence element.
    engine.publish_selected_order_as_installed(attacker);
    {
        let entity = engine.get_entity_mut(attacker).unwrap();
        let sprite = &mut entity.element_data_mut().sprite;
        sprite.scripts = std::sync::Arc::new(vec![
            crate::sprite_script::SpriteScript {
                action_done: 3,
                frame_ids: vec![0, 1, 2, 3],
                delays: vec![1, 1, 1, 1],
                distances: vec![0, 0, 0, 0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 4],
                sound_ids: vec![0; 4],
                ..Default::default()
            };
            16
        ]);
        sprite.conversion = std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }

    engine.tick_melee_strikes(sim, &assets);
    let retained_on_start = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("the replacement strike START must not consume the retained sweep");
    let replacement_direction_angle = sector_to_angle(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .direction(),
    );
    assert_eq!(retained_on_start.strike, SwordStrike::E);
    assert_eq!(
        retained_on_start.pending_victims,
        vec![victim, unreached_victim],
        "the START warning forecast rebases geometry but keeps the interrupted victim FIFO"
    );
    assert_eq!(retained_on_start.initial_angle, replacement_direction_angle);
    assert_eq!(retained_on_start.current_angle, replacement_direction_angle);
    assert_eq!(retained_on_start.final_angle, replacement_direction_angle);
    assert_eq!(soldier_life(&engine, victim), 50);

    engine.rebind_retained_sweep_to_active_strike(&assets, attacker);
    engine.tick_sweep_for(sim, &assets, attacker, false);

    let retained_after_hit = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("a lateral sweep remains allocated until its animation terminates");
    assert_eq!(retained_after_hit.strike, SwordStrike::E);
    assert_eq!(
        retained_after_hit.direction,
        crate::profiles::WeaponThrustDirection::LeftToRight
    );
    assert!(
        (retained_after_hit.rotation_per_frame - std::f32::consts::FRAC_PI_2).abs() < f32::EPSILON,
        "the retained geometry must advance using E's rotation, not D's"
    );
}

#[test]
fn interrupted_push_victims_are_rebound_by_replacement_lateral_start() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .unconscious = true;
    engine
        .get_entity_mut(victim)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .pending_push_swordfight = vec![victim];

    engine.stop_owner_active_mechanics(attacker);
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .pending_push_swordfight,
        vec![victim],
        "interrupting PushAside preserves Original's human-owned victim list"
    );

    let assets =
        assets_with_nonstraight_profile(SwordStrike::D, crate::profiles::WeaponThrustKind::Lateral);
    let selected = install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, false);
    engine.publish_selected_order_as_installed(attacker);
    {
        let sprite = &mut engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .sprite;
        sprite.last_processed_order_id = u32::MAX;
        sprite.last_action = crate::order::OrderType::WaitingSword;
        sprite.current_frame = 0;
        sprite.frame_count = 0;
    }

    engine.tick_selected_melee_owner(&sim, &assets, attacker, selected);

    let actor = engine.get_entity(attacker).unwrap().actor_data().unwrap();
    assert_eq!(
        actor
            .sweep_state
            .as_ref()
            .expect("replacement lateral owns the retained push list")
            .pending_victims,
        vec![victim],
    );
    assert!(
        actor.pending_push_swordfight.is_empty(),
        "Original has one shared victim list, not duplicate push/sweep ownership"
    );
}

#[test]
fn interrupted_circle_sweep_preserves_geometry_before_replacement_action_point() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    engine
        .get_entity_mut(victim)
        .and_then(Entity::enemy_ai_mut)
        .unwrap()
        .hth_weapon_id = 1;

    let mut profile_manager = crate::profiles::ProfileManager::new();
    let mut weapon = crate::profiles::HtHWeaponProfile::default();
    for strike in [SwordStrike::I, SwordStrike::F] {
        let thrust = &mut weapon.thrusts[strike as usize];
        thrust.kind = crate::profiles::WeaponThrustKind::TrueCircle;
        thrust.direction = crate::profiles::WeaponThrustDirection::LeftToRight;
        thrust.minimal_distance = 0;
        thrust.maximal_distance = 100;
        thrust.initial_angle = 0;
        thrust.final_angle = 360;
        thrust.rotation_angle = 45;
    }
    profile_manager.hth_weapons.push(weapon);
    profile_manager
        .characters
        .push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..crate::profiles::CharacterProfile::default()
        });
    profile_manager
        .soldiers
        .push(crate::profiles::SoldierProfile {
            hth_weapon_id: 1,
            ..crate::profiles::SoldierProfile::default()
        });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profile_manager),
        ..LevelAssets::default()
    };

    let retained_selection =
        install_test_melee_order(&mut engine, attacker, victim, SwordStrike::I, true);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(7);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sweep_state = Some(crate::movement::SweepState {
        pending_victims: vec![victim],
        initial_angle: 0.0,
        current_angle: 0.0,
        final_angle: std::f32::consts::TAU,
        rotation_per_frame: std::f32::consts::FRAC_PI_4,
        direction: crate::profiles::WeaponThrustDirection::LeftToRight,
        strike: SwordStrike::I,
        attacker_profile_idx: Some(1),
        strike_kind: crate::profiles::WeaponThrustKind::TrueCircle,
    });

    let replacement_order_id = engine.orders.allocate_order_id();
    let replacement_element = engine
        .orders
        .sequence_manager
        .get_element_mut(retained_selection.seq_id, retained_selection.elem_idx)
        .expect("retained strike element exists");
    replacement_element.command = SwordStrike::F.to_command();
    let replacement_order = replacement_element
        .orders
        .front_mut()
        .expect("retained strike order exists");
    replacement_order.order_type = strike_to_animation(SwordStrike::F);
    replacement_order.antagonist = Some(victim);
    replacement_order.reseed_id(replacement_order_id);
    engine.publish_selected_order_as_installed(attacker);
    {
        let entity = engine.get_entity_mut(attacker).unwrap();
        let sprite = &mut entity.element_data_mut().sprite;
        sprite.use_alternate_profile = false;
        sprite.scripts = std::sync::Arc::new(vec![
            crate::sprite_script::SpriteScript {
                action_done: 5,
                frame_ids: vec![0, 1, 2, 3, 4, 5, 6],
                delays: vec![1; 7],
                distances: vec![0; 7],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 7],
                sound_ids: vec![0; 7],
                ..Default::default()
            };
            16
        ]);
        sprite.conversion = std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }

    engine.tick_melee_strikes(sim, &assets);
    {
        let sprite = &mut engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .sprite;
        assert_eq!(sprite.action_done_frame, 5);
        assert_eq!(sprite.action_done_counter, 0);
        sprite.current_frame = 3;
        sprite.frame_count = 0;
    }
    engine.tick_melee_strikes(sim, &assets);

    let attacker_entity = engine.get_entity(attacker).unwrap();
    let retained_before_action = attacker_entity
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("replacement pre-action frames retain the interrupted circle sweep");
    assert_eq!(
        retained_before_action.strike,
        SwordStrike::F,
        "Original reads replacement F's effect parameters before its action point"
    );
    assert_eq!(retained_before_action.current_angle, 0.0);
    assert_eq!(
        attacker_entity.element_data().direction(),
        7,
        "the interrupted circle geometry must not rotate replacement strike F before its action point"
    );

    engine.tick_melee_strikes(sim, &assets);
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("replacement circle initializes its own sweep at action done")
            .strike,
        SwordStrike::F,
    );
}

#[test]
fn interrupted_h_circle_runs_replacement_i_effect_without_advancing_geometry() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: -10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let unreached_victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 90.0,
            z: 0.0,
        },
        None,
    ));
    for target in [victim, unreached_victim] {
        engine
            .get_entity_mut(target)
            .and_then(Entity::enemy_ai_mut)
            .unwrap()
            .hth_weapon_id = 1;
    }

    let mut profile_manager = crate::profiles::ProfileManager::new();
    let mut weapon = crate::profiles::HtHWeaponProfile::default();
    for strike in [SwordStrike::H, SwordStrike::I] {
        let thrust = &mut weapon.thrusts[strike as usize];
        thrust.kind = crate::profiles::WeaponThrustKind::TrueCircle;
        thrust.minimal_distance = 0;
        thrust.maximal_distance = 100;
        thrust.initial_angle = 0;
        thrust.final_angle = 360;
        thrust.rotation_angle = 22;
    }
    weapon.thrusts[SwordStrike::H as usize].direction =
        crate::profiles::WeaponThrustDirection::LeftToRight;
    weapon.thrusts[SwordStrike::I as usize].direction =
        crate::profiles::WeaponThrustDirection::RightToLeft;
    profile_manager.hth_weapons.push(weapon);
    profile_manager
        .characters
        .push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..crate::profiles::CharacterProfile::default()
        });
    profile_manager
        .soldiers
        .push(crate::profiles::SoldierProfile {
            hth_weapon_id: 1,
            ..crate::profiles::SoldierProfile::default()
        });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profile_manager),
        ..LevelAssets::default()
    };

    let retained_selection =
        install_test_melee_order(&mut engine, attacker, victim, SwordStrike::H, true);
    let retained_initial_angle = 0.1;
    let retained_current_angle = 1.251_917_2;
    let retained_final_angle = std::f32::consts::TAU + retained_initial_angle;
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(7);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sweep_state = Some(crate::movement::SweepState {
        pending_victims: vec![victim, unreached_victim],
        initial_angle: retained_initial_angle,
        current_angle: retained_current_angle,
        final_angle: retained_final_angle,
        rotation_per_frame: 0.383_972_44,
        direction: crate::profiles::WeaponThrustDirection::LeftToRight,
        strike: SwordStrike::H,
        attacker_profile_idx: Some(1),
        strike_kind: crate::profiles::WeaponThrustKind::TrueCircle,
    });

    let replacement_order_id = engine.orders.allocate_order_id();
    let replacement_element = engine
        .orders
        .sequence_manager
        .get_element_mut(retained_selection.seq_id, retained_selection.elem_idx)
        .expect("retained strike element exists");
    replacement_element.command = SwordStrike::I.to_command();
    let replacement_order = replacement_element
        .orders
        .front_mut()
        .expect("retained strike order exists");
    replacement_order.order_type = strike_to_animation(SwordStrike::I);
    replacement_order.antagonist = Some(victim);
    replacement_order.reseed_id(replacement_order_id);
    engine.publish_selected_order_as_installed(attacker);
    {
        let entity = engine.get_entity_mut(attacker).unwrap();
        let sprite = &mut entity.element_data_mut().sprite;
        sprite.use_alternate_profile = false;
        sprite.scripts = std::sync::Arc::new(vec![
            crate::sprite_script::SpriteScript {
                action_done: 5,
                frame_ids: vec![0, 1, 2, 3, 4, 5, 6],
                delays: vec![1; 7],
                distances: vec![0; 7],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 7],
                sound_ids: vec![0; 7],
                ..Default::default()
            };
            16
        ]);
        sprite.conversion = std::sync::Arc::new(vec![0; crate::sprite_script::NONANIMATION_END]);
    }

    engine.tick_melee_strikes(sim, &assets);
    {
        let sprite = &mut engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .sprite;
        assert_eq!(sprite.action_done_frame, 5);
        sprite.current_frame = 2;
        sprite.frame_count = 0;
    }
    engine.tick_melee_strikes(sim, &assets);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .filter(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
            })
            .count(),
        1,
        "the first replacement pre-action effect consumes the reached retained victim"
    );
    engine.tick_melee_strikes(sim, &assets);
    {
        let sprite = &mut engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .sprite;
        sprite.current_frame = 4;
        sprite.frame_count = 1;
        assert_eq!(
            sprite.frames_from_now_till_action_done(),
            0,
            "the forecast can reach zero one tick before the exact action point"
        );
        assert_ne!(sprite.current_frame, sprite.action_done_frame);
    }
    engine.tick_selected_sweep_phase(sim, &assets, attacker, strikes::SweepTickPhase::InProgress);

    let queued_damage: Vec<&crate::sequence::SequenceElement> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| {
            element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
        })
        .collect();
    assert_eq!(
        queued_damage.len(),
        1,
        "replacement I's right-to-left effect must reach the retained H victim before I's action point"
    );
    assert!(matches!(
        &queued_damage[0].data,
        crate::sequence::SequenceElementData::Damage {
            sword_strike: Some(SwordStrike::I),
            ..
        }
    ));

    let attacker_entity = engine.get_entity(attacker).unwrap();
    let retained = &attacker_entity.human_data().unwrap().sword_sweep;
    assert_eq!(retained.victims, vec![unreached_victim]);
    assert_eq!(retained.initial_angle, retained_initial_angle);
    assert_eq!(retained.current_angle, retained_current_angle);
    assert_eq!(retained.final_angle, retained_final_angle);
    let executable = attacker_entity
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("the unreached victim keeps retained geometry executable");
    assert_eq!(executable.strike, SwordStrike::I);
    assert_eq!(executable.current_angle, retained_current_angle);
    assert_eq!(
        attacker_entity
            .element_data()
            .sprite
            .frames_from_now_till_action_done(),
        0,
        "the zero-forecast pre-action effect must not advance the replacement sprite"
    );
    assert_eq!(
        attacker_entity.element_data().direction(),
        7,
        "the pre-action effect must not rotate the replacement sprite"
    );
}

#[test]
fn replacement_half_circle_start_rebases_retained_angles_before_effect() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let retained_victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: -10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let replacement_target = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    for target in [retained_victim, replacement_target] {
        engine
            .get_entity_mut(target)
            .and_then(Entity::enemy_ai_mut)
            .unwrap()
            .hth_weapon_id = 1;
    }
    let mut assets = assets_with_nonstraight_profile(
        SwordStrike::G,
        crate::profiles::WeaponThrustKind::TrueHalfCircle,
    );
    std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::G as usize]
        .direction = crate::profiles::WeaponThrustDirection::RightToLeft;
    let selected = install_test_melee_order(
        &mut engine,
        attacker,
        replacement_target,
        SwordStrike::G,
        false,
    );
    // Actor::Instruct publishes the selected G order to mpOrder before
    // Execute reaches its START warning boundary. The low-level fixture
    // installs the sequence order directly, so mirror that publication.
    engine.publish_selected_order_as_installed(attacker);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(0);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sweep_state = Some(crate::movement::SweepState {
        pending_victims: vec![retained_victim],
        // Stale left-to-right H geometry already spans the victim's
        // sector. Without the START warning-query rebase, G's first
        // IN_PROGRESS effect would queue a second sword hit.
        initial_angle: 0.0,
        current_angle: std::f32::consts::PI,
        final_angle: std::f32::consts::TAU,
        rotation_per_frame: std::f32::consts::FRAC_PI_2,
        direction: crate::profiles::WeaponThrustDirection::LeftToRight,
        strike: SwordStrike::H,
        attacker_profile_idx: Some(1),
        strike_kind: crate::profiles::WeaponThrustKind::TrueCircle,
    });
    {
        // The shared order fixture advances to the action point for most
        // sweep tests. Rewind only its sprite identity so the established
        // selected-owner dispatcher observes G's real START boundary.
        let sprite = &mut engine
            .get_entity_mut(attacker)
            .unwrap()
            .element_data_mut()
            .sprite;
        sprite.last_processed_order_id = u32::MAX;
        sprite.last_action = crate::order::OrderType::WaitingSword;
        sprite.current_frame = 0;
        sprite.frame_count = 0;
    }

    // Human::Execute calls WarnForStrike on MotionState::Start. Its
    // half-circle victim query mutates the shared angles even though the
    // retained victim FIFO belongs to the interrupted H strike.
    engine.tick_selected_melee_owner(&sim, &assets, attacker, selected);
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .sprite
            .last_motion_state,
        Some(crate::sprite::MotionState::Start),
        "the first selected-owner tick must exercise G's START warning boundary"
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .direction(),
        0,
        "replacement G must turn right while the stale H victim remains on the left"
    );

    let sweep = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("replacement G keeps the interrupted victim FIFO");
    assert_eq!(sweep.pending_victims, vec![retained_victim]);
    assert_eq!(sweep.strike, SwordStrike::G);
    assert_eq!(
        sweep.direction,
        crate::profiles::WeaponThrustDirection::RightToLeft
    );
    let replacement_direction_angle = sector_to_angle(0);
    assert!((sweep.initial_angle - replacement_direction_angle).abs() < f32::EPSILON);
    assert!((sweep.current_angle - replacement_direction_angle).abs() < f32::EPSILON);
    assert!(
        (sweep.final_angle - (replacement_direction_angle - std::f32::consts::PI)).abs()
            < f32::EPSILON
    );

    engine.tick_selected_melee_owner(&sim, &assets, attacker, selected);
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .sprite
            .last_motion_state,
        Some(crate::sprite::MotionState::InProgress),
        "the second selected-owner tick must exercise retained G geometry before DONE"
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.command == Command::ReceiveSwordDamage
                    && element.owner == Some(retained_victim)
            }),
        "G's first effect must use the START-rebased half-circle angles"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .is_some(),
        "the focused effect check keeps the replacement strike selected"
    );
}

#[test]
fn terminal_true_circle_direction_is_presented_before_done_progresses() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets = assets_with_nonstraight_profile(
        SwordStrike::G,
        crate::profiles::WeaponThrustKind::TrueHalfCircle,
    );
    let selected = install_test_melee_order(&mut engine, attacker, victim, SwordStrike::G, true);

    let terminal_angle = sector_to_angle(13);
    {
        let entity = engine.get_entity_mut(attacker).unwrap();
        entity.element_data_mut().set_direction_instantly(15);
        let sprite = &mut entity.element_data_mut().sprite;
        assert_eq!(sprite.current_frame, sprite.action_done_frame);
        assert_eq!(sprite.frame_count, sprite.action_done_counter);
        entity.actor_data_mut().unwrap().sweep_state = Some(crate::movement::SweepState {
            pending_victims: Vec::new(),
            initial_angle: terminal_angle + std::f32::consts::PI,
            current_angle: terminal_angle,
            final_angle: terminal_angle,
            rotation_per_frame: -std::f32::consts::FRAC_PI_2,
            // Deliberately stale retained metadata: Original dispatches
            // terminal action semantics from the current G call.
            direction: crate::profiles::WeaponThrustDirection::RightToLeft,
            strike: SwordStrike::F,
            attacker_profile_idx: Some(1),
            strike_kind: crate::profiles::WeaponThrustKind::FalseHalfCircle,
        });
    }

    engine.tick_selected_melee_owner(sim, &assets, attacker, selected);

    let attacker_entity = engine.get_entity(attacker).unwrap();
    assert_eq!(
        attacker_entity.element_data().direction(),
        13,
        "Original presents the terminal true-circle angle before the exact action-done call advances the sprite"
    );
    let sprite = &attacker_entity.element_data().sprite;
    let current_g_row = sprite
        .row_for_action(strike_to_animation(SwordStrike::G))
        .expect("current G animation remains mapped");
    assert_eq!(
        sprite.current_row,
        current_g_row + 13,
        "terminal presentation must force the current G animation row, not retained F"
    );
    assert_eq!(
        sprite.current_frame,
        sprite.action_done_frame + 1,
        "the zero-delay fixture advances one frame after presenting the terminal angle"
    );
    assert_eq!(
        sprite.frame_count, 0,
        "the zero-delay next frame begins at counter zero"
    );
    assert!(
        attacker_entity.actor_data().unwrap().sweep_state.is_none(),
        "the terminal presentation call clears an exhausted sweep mirror"
    );
}

#[test]
fn replacement_true_circle_uses_current_direction_at_action_done() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets = assets_with_nonstraight_profile(
        SwordStrike::G,
        crate::profiles::WeaponThrustKind::TrueHalfCircle,
    );
    let selected = install_test_melee_order(&mut engine, attacker, victim, SwordStrike::G, true);

    let current_angle = sector_to_angle(13);
    {
        let entity = engine.get_entity_mut(attacker).unwrap();
        entity.element_data_mut().set_direction_instantly(15);
        entity.actor_data_mut().unwrap().sweep_state = Some(crate::movement::SweepState {
            pending_victims: Vec::new(),
            initial_angle: current_angle,
            current_angle,
            final_angle: current_angle + std::f32::consts::FRAC_PI_2,
            rotation_per_frame: std::f32::consts::FRAC_PI_2,
            // Stale retained right-to-left/false-F metadata says the
            // sweep is complete. Current G is a left-to-right true
            // circle and must keep rotating without progressing sprite.
            direction: crate::profiles::WeaponThrustDirection::RightToLeft,
            strike: SwordStrike::F,
            attacker_profile_idx: Some(1),
            strike_kind: crate::profiles::WeaponThrustKind::FalseHalfCircle,
        });
    }

    engine.tick_selected_melee_owner(sim, &assets, attacker, selected);

    let attacker_entity = engine.get_entity(attacker).unwrap();
    let sprite = &attacker_entity.element_data().sprite;
    assert_eq!(attacker_entity.element_data().direction(), 13);
    assert_eq!(sprite.current_frame, sprite.action_done_frame);
    assert_eq!(sprite.frame_count, sprite.action_done_counter);
    assert!(
        attacker_entity.actor_data().unwrap().sweep_state.is_some(),
        "current G's left-to-right direction keeps the retained geometry rotating"
    );
}

#[test]
fn saved_human_sweep_is_rehydrated_for_the_live_strike_order() {
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets =
        assets_with_nonstraight_profile(SwordStrike::E, crate::profiles::WeaponThrustKind::Lateral);
    install_test_melee_order(&mut engine, attacker, victim, SwordStrike::E, true);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .sword_sweep = crate::element::HumanSwordSweepState {
        victims: vec![victim],
        initial_angle: 0.0,
        current_angle: 0.0,
        final_angle: std::f32::consts::PI,
    };
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .is_none()
    );

    engine.rebind_retained_sweep_to_active_strike(&assets, attacker);

    let sweep = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("serialized human sweep must regain its executable mirror");
    assert_eq!(sweep.pending_victims, vec![victim]);
    assert_eq!(sweep.initial_angle, 0.0);
    assert_eq!(sweep.current_angle, 0.0);
    assert_eq!(sweep.final_angle, std::f32::consts::PI);
    assert_eq!(sweep.strike, SwordStrike::E);
    assert_eq!(
        sweep.strike_kind,
        crate::profiles::WeaponThrustKind::Lateral
    );

    engine.tick_sweep_for(&crate::sim_rng::test_context(), &assets, attacker, false);
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .sword_sweep
            .victims
            .is_empty(),
        "consuming the executable victim must consume the serialized human mirror too"
    );
    engine.rebind_retained_sweep_to_active_strike(&assets, attacker);
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .is_none(),
        "the consumed save victim must not be rehydrated and hit again next frame"
    );
}

#[test]
fn saved_empty_true_circle_sweep_is_rehydrated_and_rotates() {
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let target = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let mut assets = assets_with_nonstraight_profile(
        SwordStrike::H,
        crate::profiles::WeaponThrustKind::TrueCircle,
    );
    std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::H as usize]
        .direction = crate::profiles::WeaponThrustDirection::RightToLeft;
    install_test_melee_order(&mut engine, attacker, target, SwordStrike::H, true);

    let current_angle = sector_to_angle(3);
    {
        let entity = engine.get_entity_mut(attacker).unwrap();
        entity.element_data_mut().set_direction_instantly(10);
        entity.human_data_mut().unwrap().sword_sweep = crate::element::HumanSwordSweepState {
            victims: Vec::new(),
            initial_angle: current_angle + std::f32::consts::FRAC_PI_2,
            current_angle,
            final_angle: current_angle - std::f32::consts::PI,
        };
        assert!(entity.actor_data().unwrap().sweep_state.is_none());
    }

    engine.rebind_retained_sweep_to_active_strike(&assets, attacker);

    let sweep = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("an empty loaded true-circle still owns executable angle state");
    assert!(sweep.pending_victims.is_empty());
    assert_eq!(sweep.current_angle.to_bits(), current_angle.to_bits());

    engine.tick_sweep_for(&crate::sim_rng::test_context(), &assets, attacker, false);
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .direction(),
        3,
        "ExecuteTrueCircleSwordStrikeAction presents its saved angle even with no victims"
    );
}

#[test]
fn lateral_start_rebases_retained_serialized_human_victims() {
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 1_000.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets =
        assets_with_nonstraight_profile(SwordStrike::D, crate::profiles::WeaponThrustKind::Lateral);
    let selected = install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, false);
    engine.publish_selected_order_as_installed(attacker);
    {
        let entity = engine.get_entity_mut(attacker).unwrap();
        entity.human_data_mut().unwrap().sword_sweep = crate::element::HumanSwordSweepState {
            victims: vec![victim],
            initial_angle: 0.1,
            current_angle: 0.1,
            final_angle: 0.1 - std::f32::consts::PI,
        };
        assert!(entity.actor_data().unwrap().sweep_state.is_none());
        let sprite = &mut entity.element_data_mut().sprite;
        sprite.last_processed_order_id = u32::MAX;
        sprite.last_action = crate::order::OrderType::WaitingSword;
        sprite.current_frame = 0;
        sprite.frame_count = 0;
    }

    engine.tick_selected_melee_owner(&crate::sim_rng::test_context(), &assets, attacker, selected);

    let attacker = engine.get_entity(attacker).unwrap();
    assert_eq!(
        attacker.element_data().sprite.last_motion_state,
        Some(crate::sprite::MotionState::Start)
    );
    let direction_angle = sector_to_angle(attacker.element_data().direction());
    let sweep = attacker
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("the START warning must make the retained rider-charge list executable");
    assert_eq!(sweep.pending_victims, vec![victim]);
    assert_eq!(sweep.strike, SwordStrike::D);
    assert_eq!(sweep.initial_angle, direction_angle);
    assert_eq!(sweep.current_angle, direction_angle);
    assert_eq!(sweep.final_angle, direction_angle + std::f32::consts::PI);
    assert_eq!(
        attacker.human_data().unwrap().sword_sweep.victims,
        vec![victim],
        "rebasing geometry must preserve Original's shared victim FIFO"
    );
}

#[test]
fn terminated_lateral_sweep_cannot_rehydrate_into_a_fresh_strike() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets =
        assets_with_nonstraight_profile(SwordStrike::D, crate::profiles::WeaponThrustKind::Lateral);

    engine.initialize_sweep(
        &assets,
        attacker,
        SwordStrike::D,
        Some(1),
        crate::profiles::WeaponThrustKind::Lateral,
        vec![victim],
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .sword_sweep
            .victims,
        vec![victim]
    );

    engine.complete_melee_strike(&sim, &assets, attacker, None, 0, SwordStrike::D, Some(1));

    let attacker_entity = engine.get_entity(attacker).unwrap();
    assert!(
        attacker_entity.actor_data().unwrap().sweep_state.is_none(),
        "termination clears the executable sweep"
    );
    assert!(
        attacker_entity
            .human_data()
            .unwrap()
            .sword_sweep
            .victims
            .is_empty(),
        "Original deletes the human-owned victim list on RHMOTION_TERMINATED"
    );

    install_test_melee_order(&mut engine, attacker, victim, SwordStrike::D, true);
    engine.rebind_retained_sweep_to_active_strike(&assets, attacker);
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .is_none(),
        "a fresh lateral strike must wait for its own action-done initialization"
    );
}

#[test]
fn later_circle_frame_tests_existing_angle_before_tail_advance() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets = assets_with_nonstraight_profile(
        SwordStrike::F,
        crate::profiles::WeaponThrustKind::FalseHalfCircle,
    );
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sweep_state = Some(crate::movement::SweepState {
        pending_victims: vec![victim],
        initial_angle: 0.0,
        current_angle: 0.0,
        final_angle: std::f32::consts::PI,
        rotation_per_frame: std::f32::consts::FRAC_PI_2,
        direction: crate::profiles::WeaponThrustDirection::LeftToRight,
        strike: SwordStrike::F,
        attacker_profile_idx: Some(1),
        strike_kind: crate::profiles::WeaponThrustKind::FalseHalfCircle,
    });

    let queued_damage_count = |engine: &EngineInner| {
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .filter(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
            })
            .count()
    };

    engine.tick_sweep_for(sim, &assets, attacker, false);
    assert_eq!(
        queued_damage_count(&engine),
        0,
        "the victim in the newly reached sector cannot be tested before the circle tail advance"
    );
    let sweep = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("pending final-sector victim must keep the sweep alive");
    assert!((sweep.current_angle - std::f32::consts::FRAC_PI_2).abs() < f32::EPSILON);

    engine.tick_sweep_for(sim, &assets, attacker, false);
    assert_eq!(
        queued_damage_count(&engine),
        1,
        "the next IN_PROGRESS effect must test the angle reached by the prior tail advance"
    );
}

#[test]
fn circle_tail_retains_candidate_past_final_in_the_same_sector() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let pending_victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets = assets_with_nonstraight_profile(
        SwordStrike::F,
        crate::profiles::WeaponThrustKind::FalseHalfCircle,
    );
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sweep_state = Some(crate::movement::SweepState {
        pending_victims: vec![pending_victim],
        initial_angle: 0.0,
        current_angle: 0.0,
        final_angle: 0.70,
        rotation_per_frame: 0.75,
        direction: crate::profiles::WeaponThrustDirection::LeftToRight,
        strike: SwordStrike::F,
        attacker_profile_idx: Some(1),
        strike_kind: crate::profiles::WeaponThrustKind::FalseHalfCircle,
    });

    engine.tick_sweep_for(sim, &assets, attacker, false);

    let current = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("unreached victim keeps the circle sweep observable")
        .current_angle;
    assert!(
        (current - 0.75).abs() < f32::EPSILON,
        "a candidate past 0.70 in the same final sector must be retained instead of clamped"
    );
}

#[test]
fn lateral_advance_is_raw_and_does_not_use_circle_final_clamping() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let pending_victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets =
        assets_with_nonstraight_profile(SwordStrike::D, crate::profiles::WeaponThrustKind::Lateral);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sweep_state = Some(crate::movement::SweepState {
        pending_victims: vec![pending_victim],
        initial_angle: 0.0,
        current_angle: 0.0,
        final_angle: 0.70,
        rotation_per_frame: 1.20,
        direction: crate::profiles::WeaponThrustDirection::LeftToRight,
        strike: SwordStrike::D,
        attacker_profile_idx: Some(1),
        strike_kind: crate::profiles::WeaponThrustKind::Lateral,
    });

    engine.tick_sweep_for(sim, &assets, attacker, false);

    let current = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .sweep_state
        .as_ref()
        .expect("unreached victim keeps the lateral sweep observable")
        .current_angle;
    assert!(
        (current - 1.20).abs() < f32::EPSILON,
        "lateral Execute applies its signed rotation directly even past final_angle"
    );
}

#[test]
fn push_victims_queue_damage_in_creation_fifo() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let first_victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 80.0,
            z: 0.0,
        },
        None,
    ));
    let second_victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 60.0,
            z: 0.0,
        },
        None,
    ));
    for victim in [first_victim, second_victim] {
        engine
            .get_entity_mut(victim)
            .unwrap()
            .element_data_mut()
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                crate::coordinates::MapVec::new(-5.0, -5.0),
                crate::coordinates::MapVec::new(5.0, 5.0),
            ));
    }
    let assets = assets_with_nonstraight_profile(
        SwordStrike::D,
        crate::profiles::WeaponThrustKind::PushAside,
    );
    let selected =
        install_test_melee_order(&mut engine, attacker, first_victim, SwordStrike::D, false);

    assert_eq!(
        engine.tick_nonstraight_melee_for(sim, &assets, attacker, selected),
        strikes::SweepTickPhase::InProgress
    );

    let first_life = soldier_life(&engine, first_victim);
    let second_life = soldier_life(&engine, second_victim);
    let damage_fifo: Vec<EntityId> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| element.command == Command::ReceiveSwordDamage)
        .filter_map(|element| element.owner)
        .collect();
    assert_eq!(
        damage_fifo,
        vec![first_victim, second_victim],
        "push damage launches must retain the original actor-list victim FIFO; lives were {first_life}/{second_life}"
    );
}

#[test]
fn push_replacement_executes_without_advancing_retained_circle_sweep() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 140.0,
            z: 0.0,
        },
        None,
    ));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .sprite
        .position_iface
        .set_move_box(crate::coordinates::MoveBox::from_corners(
            crate::coordinates::MapVec::new(-5.0, -5.0),
            crate::coordinates::MapVec::new(5.0, 5.0),
        ));
    let assets = assets_with_nonstraight_profile(
        SwordStrike::A,
        crate::profiles::WeaponThrustKind::PushAside,
    );
    let selected = install_test_melee_order(&mut engine, attacker, victim, SwordStrike::A, false);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(8);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sweep_state = Some(crate::movement::SweepState {
        pending_victims: vec![victim],
        initial_angle: 2.0,
        current_angle: 5.5,
        final_angle: 5.5,
        rotation_per_frame: std::f32::consts::FRAC_PI_4,
        direction: crate::profiles::WeaponThrustDirection::LeftToRight,
        strike: SwordStrike::F,
        attacker_profile_idx: Some(1),
        strike_kind: crate::profiles::WeaponThrustKind::TrueHalfCircle,
    });

    engine.tick_selected_melee_owner(sim, &assets, attacker, selected);

    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .direction(),
        8,
        "PushAside must not present the retained F sweep's terminal direction"
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .sweep_state
            .as_ref()
            .expect("PushAside leaves interrupted sweep storage dormant")
            .strike,
        SwordStrike::F,
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
            }),
        "the replacement PushAside must still execute and queue its damage"
    );
}

#[test]
fn push_strike_does_not_recover_antagonist_outside_rectangle() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let target = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 11.0,
            y: 80.0,
            z: 0.0,
        },
        None,
    ));
    let mut assets = assets_with_nonstraight_profile(
        SwordStrike::A,
        crate::profiles::WeaponThrustKind::PushAside,
    );
    std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::A as usize]
        .repulsion = 20;
    let selected = install_test_melee_order(&mut engine, attacker, target, SwordStrike::A, false);

    assert_eq!(
        engine.tick_nonstraight_melee_for(sim, &assets, attacker, selected),
        strikes::SweepTickPhase::InProgress
    );

    assert!(
        !engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(target)
            }),
        "Original's PushAside scan rejects side projection 11 outside half-width 10 even when the actor is the interaction antagonist"
    );
    assert_eq!(soldier_life(&engine, target), 50);
}

#[test]
fn launching_sword_damage_does_not_add_attacker_tiredness() {
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets = assets_with_sword_profile(7, 30);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .tiredness = 11;

    crate::sim_rng::with_seed(1, |sim| {
        engine.queue_sword_damage(sim, &assets, victim, attacker, SwordStrike::A, 1);
    });

    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .tiredness,
        11,
        "damage application is victim-count dependent and must not charge strike energy"
    );
}

#[test]
fn helping_climb_shoulder_damage_keeps_posture_until_fall_executes() {
    let sim = crate::sim_rng::SimulationContext::with_seed(0x183);
    let mut engine = make_engine();
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    engine
        .get_entity_mut(victim)
        .expect("test victim must exist")
        .set_posture(Posture::HelpingToClimb);

    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(crate::sequence::SequenceElement::new(
        1,
        Command::ReceiveSwordDamage,
        Some(victim),
    ));
    let sequence_id = engine.orders.sequence_manager.launch_sequence(sequence);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
    engine.translate_shoulder_damage(&sim, &assets, victim, (sequence_id, 0));

    assert_eq!(
        engine
            .get_entity(victim)
            .expect("test victim must remain live")
            .element_data()
            .posture,
        Posture::HelpingToClimb,
        "TranslateShoulderDamage only queues FallingBackUpright; its Execute START changes posture on the actor's next slot"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .expect("damage element must remain registered")
            .orders
            .back()
            .expect("shoulder damage must queue a fall order")
            .order_type,
        OrderType::FallingBackUpright
    );
}

#[test]
fn shoulder_damage_dispatches_partner_fall_without_direction_recompute() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    let carrier = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let carried = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    engine
        .get_entity_mut(carrier)
        .unwrap()
        .set_posture(Posture::HelpingToClimb);
    engine
        .get_entity_mut(carrier)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .carried = Some(carried);
    engine
        .get_entity_mut(carried)
        .unwrap()
        .set_posture(Posture::OnShoulders);
    engine
        .get_entity_mut(carried)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .carrier = Some(carrier);

    let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
    let mut damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveDamage,
        Some(carrier),
        Some(attacker),
        1,
        0,
    );
    engine.resolve_element_priority(&mut damage);
    engine.orders.sequence_manager.launch_element(damage);
    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let partner_fall = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .find(|element| element.command == Command::Fall && element.owner == Some(carried))
        .expect("shoulder damage must dispatch Fall to the carried partner");
    let order = partner_fall
        .orders
        .iter()
        .find(|order| order.order_type == OrderType::FallingShoulders)
        .expect("partner Fall command must translate to FallingShoulders");
    assert!(!order.compute_direction);
}

#[test]
fn slope_translate_roll_order_keeps_its_source_authored_direction_recompute() {
    let mut engine = make_engine();
    let victim = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let mut obstacle = crate::sight_obstacle::SightObstacle::new_default(0);
    obstacle.top_plane_points = [[0.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 1.0, 0.0]];
    let mut assets = LevelAssets::new();
    assets.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);
    {
        let victim = engine.get_entity_mut(victim).unwrap();
        victim.element_data_mut().set_obstacle_index(
            crate::position_interface::ObstacleHandle::new(0),
            Some(crate::position_interface::PlaneZCoeffs {
                az: 1.0,
                bz: 0.0,
                dz: 0.0,
            }),
        );
        victim
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                crate::coordinates::MapVec::new(-5.0, -5.0),
                crate::coordinates::MapVec::new(5.0, 5.0),
            ));
    }
    let damage = crate::sequence::SequenceElement::new(1, Command::ReceiveDamage, Some(victim));
    let sequence = engine.orders.sequence_manager.launch_element(damage);

    engine.try_queue_roll(&assets, victim, (sequence, 0));

    let rolling = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap()
        .orders
        .iter()
        .find(|order| order.order_type == OrderType::Rolling)
        .expect("TranslateRoll must append its Rolling order");
    assert!(rolling.compute_direction);
}

#[test]
fn parried_damage_still_learns_attackers_live_strike() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets = assets_with_sword_profile(7, 30);

    let mut attacker_sequence = crate::sequence::Sequence::new();
    attacker_sequence.append_element(crate::sequence::SequenceElement::new(
        1,
        Command::SwordstrikeThrustE,
        Some(attacker),
    ));
    let attacker_sequence_id = engine
        .orders
        .sequence_manager
        .launch_sequence(attacker_sequence);
    engine
        .orders
        .sequence_manager
        .element_in_progress(attacker_sequence_id, 0);

    let mut damage_sequence = crate::sequence::Sequence::new();
    let mut damage_element =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage_element.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::E, 1);
    damage_sequence.append_element(damage_element);
    let damage_sequence_id = engine
        .orders
        .sequence_manager
        .launch_sequence(damage_sequence);
    engine
        .orders
        .sequence_manager
        .element_in_progress(damage_sequence_id, 0);

    let Entity::Soldier(soldier) = engine.get_entity_mut(victim).unwrap() else {
        unreachable!()
    };
    soldier.actor.action_state = ActionState::ParryingSword;
    let crate::element::AiBrain::Enemy(ai) = &mut soldier.npc.ai_brain else {
        unreachable!()
    };
    ai.known_enemy_strike_1 = Some(SwordStrike::D);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::E),
        Some(1),
        (damage_sequence_id, 0),
    );

    let Entity::Soldier(soldier) = engine.get_entity(victim).unwrap() else {
        unreachable!()
    };
    let crate::element::AiBrain::Enemy(ai) = &soldier.npc.ai_brain else {
        unreachable!()
    };
    assert_eq!(ai.known_enemy_strike_1, Some(SwordStrike::E));
    assert_eq!(
        ai.known_enemy_strike_2, None,
        "a low-skill guard forgets its previous strike when the parried live strike is learned"
    );
}

#[test]
fn push_damage_virtual_say_ouch_is_silent_for_pc() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets = assets_with_nonstraight_profile(
        SwordStrike::H,
        crate::profiles::WeaponThrustKind::TrueCircle,
    );
    let damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    let sequence_id = engine.orders.sequence_manager.launch_element(damage);

    assert!(engine.apply_push_effect(
        &sim,
        &assets,
        victim,
        attacker,
        &PushStrikeInfo { repulsion: 100 },
        combat::SwordDamageResult::NO_DAMAGE_PARRIED,
        (sequence_id, 0),
        false,
    ));
    assert!(
        engine.feedback.sound_sim.pending_exclamations.is_empty(),
        "PC inherits RHElementActorHuman::SayOuch's no-op on TranslatePushDamage"
    );
}

#[test]
fn push_damage_command_disables_direction_on_fall_and_successors() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let victim = engine
            .get_entity_mut(victim)
            .expect("push victim remains live");
        victim.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        victim.human_data_mut().unwrap().concussion_of_the_brain = STUNNING_THRESHOLD + 1;
        victim.enemy_ai_mut().unwrap().hth_weapon_id = 1;
    }
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    let assets = assets_with_nonstraight_profile(
        SwordStrike::H,
        crate::profiles::WeaponThrustKind::TrueCircle,
    );
    let damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveSwordDamage,
        Some(victim),
        Some(attacker),
        1,
        0,
    );
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    assert!(engine.apply_push_effect(
        &sim,
        &assets,
        victim,
        attacker,
        &PushStrikeInfo { repulsion: 100 },
        combat::SwordDamageResult::STUNNING_DAMAGE,
        (sequence, 0),
        false,
    ));

    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("translated push damage remains registered");
    assert_eq!(
        element
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![
            OrderType::FallingPushedWithSword,
            OrderType::StandingUpSword,
            OrderType::BeingStunnedSword,
        ]
    );
    assert!(
        element
            .orders
            .iter()
            .filter(|order| order.order_type != OrderType::Rolling)
            .all(|order| !order.compute_direction),
        "TranslatePushDamage sets bComputeDirection=false on the fall, stand-up, and stunned orders"
    );
}

#[test]
fn pc_hurt_speech_uses_applied_life_loss_not_attempted_damage() {
    let mut engine = make_engine();
    let victim = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);

    // The protected task-188 control attempts more than twenty points of
    // damage, but only 18 LP are ultimately stored (82 -> 64).
    let attempted_damage = 25;
    assert!(attempted_damage > 20);
    engine.pc_life_points_speech(&assets, victim, 82, 64);
    assert!(
        engine.feedback.sound_sim.pending_exclamations.is_empty(),
        "RHElementActorPC::SetLifePoints compares the applied LP delta"
    );

    engine.pc_life_points_speech(&assets, victim, 82, 61);
    assert_eq!(
        engine
            .feedback
            .sound_sim
            .pending_exclamations
            .iter()
            .map(|pending| pending.exclamation_id)
            .collect::<Vec<_>>(),
        vec![HERO_HURT]
    );
}

#[test]
fn push_strike_does_not_inform_soldier_of_good_strike() {
    use crate::ai::{AiState, LogLineType, StimulusType, Substate};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = attacker.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
        ai.hth_weapon_id = 1;
    }
    let assets = assets_with_nonstraight_profile(
        SwordStrike::H,
        crate::profiles::WeaponThrustKind::TrueCircle,
    );
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::H, 1);
    let sequence_id = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::H),
        Some(1),
        (sequence_id, 0),
    );

    let ai = engine
        .get_entity(attacker)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert!(
        !ai.ai_log.iter().any(|entry| {
            entry.line_type == LogLineType::Event
                && entry.info == StimulusType::EventGoodStrike as u16
        }),
        "Original skips TranslateSwordDamage, and therefore EVENT_GOOD_STRIKE, for push strikes"
    );
    assert_eq!(
        ai.current_substate,
        Substate::AttackingSwordfightSpecialStrike
    );
}

#[test]
fn ordinary_cutting_strike_still_informs_soldier_of_good_strike() {
    use crate::ai::{AiState, LogLineType, StimulusType, Substate};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        soldier.human.opponents.push(victim);
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = attacker.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
        ai.hth_weapon_id = 1;
    }
    engine
        .get_entity_mut(victim)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(attacker);
    let assets = assets_with_sword_profile(1, 50);
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence_id = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence_id, 0),
    );

    let ai = engine
        .get_entity(attacker)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert!(ai.ai_log.iter().any(|entry| {
        entry.line_type == LogLineType::Event && entry.info == StimulusType::EventGoodStrike as u16
    }));
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![victim],
        "a conscious surviving victim must retain the swordfight"
    );
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![attacker]
    );
    let damage = engine
        .orders
        .sequence_manager
        .get_element(sequence_id, 0)
        .expect("cutting damage command remains registered");
    assert!(
        damage
            .orders
            .iter()
            .filter(|order| order.order_type != OrderType::Rolling)
            .all(|order| !order.compute_direction),
        "TranslateSwordDamage sets bComputeDirection=false on its cutting-hit order"
    );
}

#[test]
fn pc_shoulder_sword_damage_skips_good_strike_but_keeps_fall_translation() {
    use crate::ai::{AiState, LogLineType, StimulusType, Substate};
    use crate::sequence::SequencePriority;

    for posture in [
        Posture::HelpingToClimb,
        Posture::CarryingOnShoulders,
        Posture::OnShoulders,
    ] {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
        let victim = engine.add_entity(make_pc(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::default()
            },
            None,
        ));
        let partner = engine.add_entity(make_pc(WorldPoint3D::default(), None));
        {
            let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
                unreachable!()
            };
            soldier.human.opponents.push(victim);
            let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
            ai.base.me = attacker.index();
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
            ai.hth_weapon_id = 1;
        }
        engine
            .get_entity_mut(victim)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
        engine.get_entity_mut(victim).unwrap().set_posture(posture);
        if posture == Posture::OnShoulders {
            engine
                .get_entity_mut(victim)
                .unwrap()
                .human_data_mut()
                .unwrap()
                .carrier = Some(partner);
            let partner_entity = engine.get_entity_mut(partner).unwrap();
            partner_entity.set_posture(Posture::CarryingOnShoulders);
            partner_entity.pc_data_mut().unwrap().carried = Some(victim);
        } else {
            engine
                .get_entity_mut(victim)
                .unwrap()
                .pc_data_mut()
                .unwrap()
                .carried = Some(partner);
            let partner_entity = engine.get_entity_mut(partner).unwrap();
            partner_entity.set_posture(Posture::OnShoulders);
            partner_entity.human_data_mut().unwrap().carrier = Some(victim);
        }

        let assets = assets_with_sword_profile(1, 50);
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        let sequence_id = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::A),
            Some(1),
            (sequence_id, 0),
        );

        let attacker_ai = engine
            .get_entity(attacker)
            .unwrap()
            .ai_controller()
            .unwrap();
        assert!(
            !attacker_ai.ai_log.iter().any(|entry| {
                entry.line_type == LogLineType::Event
                    && entry.info == StimulusType::EventGoodStrike as u16
            }),
            "PC posture {posture:?} must use the PC shoulder override without EventGoodStrike"
        );
        assert_eq!(
            attacker_ai.current_substate,
            Substate::AttackingSwordfightSpecialStrike,
            "suppressed EventGoodStrike must not advance the attacker AI"
        );

        let damage = engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .expect("shoulder damage command remains registered");
        let expected_fall = if posture == Posture::OnShoulders {
            assert_eq!(damage.priority, SequencePriority::NonInterruptable);
            OrderType::FallingShoulders
        } else {
            OrderType::FallingBackUpright
        };
        assert!(
            damage
                .orders
                .iter()
                .any(|order| order.order_type == expected_fall),
            "PC posture {posture:?} must retain its shoulder fall translation"
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| sequence.elements.iter())
                .any(|element| element.command == Command::Fall && element.owner == Some(partner)),
            "PC posture {posture:?} must still dispatch Fall to its shoulder partner"
        );
    }
}

#[test]
fn non_pc_helping_to_climb_still_informs_soldier_of_good_strike() {
    use crate::ai::{AiState, LogLineType, StimulusType, Substate};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = attacker.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
        ai.hth_weapon_id = 1;
    }
    engine
        .get_entity_mut(victim)
        .unwrap()
        .set_posture(Posture::HelpingToClimb);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    let assets = assets_with_sword_profile(1, 50);
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence_id = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence_id, 0),
    );

    let attacker_ai = engine
        .get_entity(attacker)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert!(attacker_ai.ai_log.iter().any(|entry| {
        entry.line_type == LogLineType::Event && entry.info == StimulusType::EventGoodStrike as u16
    }));
}

#[test]
fn lateral_done_processes_victims_in_original_actor_order_before_good_strike() {
    use crate::ai::{AiState, LogLineType, Remark, StimulusType, Substate};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
    // Allocate the survivor first so typed entity iteration disagrees with
    // Original's actor registry below. This is the Save016 shape: the
    // later-ID victim must knock out and unlink the attacker first.
    let survivor = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 20.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    let knockout = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    engine.world.install_original_creation_orders(
        [(attacker, 0), (knockout, 1), (survivor, 2)]
            .into_iter()
            .collect(),
        3,
    );

    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        // Only the first Original-order victim is an opponent. Its KO
        // therefore synchronously sends EventQuitSwordfight to the
        // attacker before damage reaches the later survivor.
        soldier.human.opponents.push(knockout);
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = attacker.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
        ai.hth_weapon_id = 1;
    }
    engine
        .get_entity_mut(knockout)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(attacker);
    // Keep the later victim conscious while retaining real cutting damage,
    // so it would emit GoodStrike if processed before the KO callback.
    engine
        .get_entity_mut(survivor)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .invulnerable = true;

    let mut assets = assets_with_sword_profile_effects(1, 100, 4, 100);
    let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::A as usize];
    thrust.kind = crate::profiles::WeaponThrustKind::Lateral;
    thrust.direction = crate::profiles::WeaponThrustDirection::LeftToRight;
    thrust.initial_angle = 0;
    thrust.final_angle = 180;
    thrust.rotation_angle = 90;

    let victims = engine.execute_multi_target_strike(&assets, attacker, SwordStrike::A, Some(1));
    assert_eq!(
        victims,
        [knockout, survivor],
        "DONE membership is unchanged, but follows Original GetActor FIFO rather than typed IDs"
    );

    for victim in victims {
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        let sequence_id = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence_id, 0);
        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::A),
            Some(1),
            (sequence_id, 0),
        );
    }

    assert!(
        engine
            .get_entity(knockout)
            .unwrap()
            .human_data()
            .unwrap()
            .unconscious,
        "first victim must exercise the synchronous knockout/quit arm"
    );
    assert!(
        !engine
            .get_entity(survivor)
            .unwrap()
            .human_data()
            .unwrap()
            .unconscious,
        "later cutting victim must remain a genuine surviving control"
    );
    let ai = engine
        .get_entity(attacker)
        .unwrap()
        .ai_controller()
        .unwrap();
    assert_eq!(ai.current_substate, Substate::AttackingQuittingSwordfight);
    assert!(
        ai.ai_log.iter().any(|entry| {
            entry.line_type == LogLineType::Event
                && entry.info == StimulusType::EventGoodStrike as u16
        }),
        "later survivor must deliver a real GoodStrike after the first victim quits"
    );
    assert!(
        !ai.ai_log.iter().any(|entry| {
            entry.line_type == LogLineType::Speak && entry.info == Remark::GoodStrikeCombat as u16
        }),
        "later GoodStrike is delivered after quit and must not start speech"
    );
    assert!(
        engine.feedback.sound_sim.pending_exclamations.is_empty(),
        "ignored later GoodStrike must not leave a pending combat exclamation"
    );
}

#[test]
fn surviving_sword_knockout_quits_before_good_strike_and_fall_translation() {
    use crate::ai::{AiState, LogLineType, StimulusType, Substate};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::default(), None));
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        soldier.human.opponents.push(victim);
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = attacker.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
        ai.hth_weapon_id = 1;
    }
    engine
        .get_entity_mut(victim)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(attacker);

    let mut assets = assets_with_sword_profile_effects(1, 50, 4, 100);
    let mut obstacle = crate::sight_obstacle::SightObstacle::new_default(0);
    obstacle.top_plane_points = [[0.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 1.0, 0.0]];
    assets.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);
    let victim_entity = engine.get_entity_mut(victim).unwrap();
    victim_entity.element_data_mut().set_obstacle_index(
        crate::position_interface::ObstacleHandle::new(0),
        Some(crate::position_interface::PlaneZCoeffs {
            az: 1.0,
            bz: 0.0,
            dz: 0.0,
        }),
    );
    victim_entity
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_corners(
            crate::coordinates::MapVec::new(-5.0, -5.0),
            crate::coordinates::MapVec::new(5.0, 5.0),
        ));
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence_id = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence_id, 0),
    );

    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(victim_entity.human_data().unwrap().unconscious);
    assert!(
        victim_entity.pc_data().unwrap().life_points > 0,
        "fixture must exercise the surviving-knockout arm"
    );
    assert!(victim_entity.human_data().unwrap().opponents.is_empty());
    let attacker_entity = engine.get_entity(attacker).unwrap();
    assert!(attacker_entity.human_data().unwrap().opponents.is_empty());
    let ai = attacker_entity.ai_controller().unwrap();
    assert_eq!(ai.current_substate, Substate::AttackingQuittingSwordfight);
    let good_strike_index = ai
        .ai_log
        .iter()
        .position(|entry| {
            entry.line_type == LogLineType::Event
                && entry.info == StimulusType::EventGoodStrike as u16
        })
        .expect("soldier origin must receive EVENT_GOOD_STRIKE");
    let quit_index = ai
        .ai_log
        .iter()
        .position(|entry| {
            entry.line_type == LogLineType::ChangeState
                && entry.info == Substate::AttackingQuittingSwordfight as u16
        })
        .expect("reciprocal unlink must synchronously enter the quitting substate");
    assert!(
        quit_index < good_strike_index,
        "SetConcussionOfTheBrain quits before TranslateSwordDamage informs the hitter"
    );
    let translated_orders = &engine
        .orders
        .sequence_manager
        .get_element(sequence_id, 0)
        .expect("knockout damage element remains registered")
        .orders;
    assert_eq!(
        translated_orders.front().map(|order| order.order_type),
        Some(OrderType::FallingBackUpright),
        "TranslateSwordDamage's second quit remains before its knockout fall"
    );
    assert!(
        translated_orders
            .iter()
            .any(|order| order.order_type == OrderType::Rolling),
        "the real surviving-KO translation must still append Roll"
    );
}

#[test]
fn preexisting_unconscious_smalltalk_hit_preserves_closed_eyes_and_plain_quit() {
    use crate::ai::{LogLineType, StimulusType};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.element_data_mut().posture = Posture::Upright;
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        victim_entity.human_data_mut().unwrap().unconscious = true;
        victim_entity.npc_data_mut().unwrap().eye_status = EyeStatus::Closed;
        victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
        victim_entity
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
    }
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(victim);

    let assets = assets_with_sword_profile(1, 50);

    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data = crate::sequence::SequenceElementData::new_sword_damage(
        attacker,
        SwordStrike::SmalltalkRight,
        1,
    );
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.dispatch_receive_damage(&sim, &assets, victim, sequence, 0);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(victim_entity.human_data().unwrap().unconscious);
    assert_eq!(
        victim_entity.npc_data().unwrap().eye_status,
        EyeStatus::Closed
    );
    assert!(victim_entity.human_data().unwrap().opponents.is_empty());
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents
            .is_empty(),
        "TranslateSwordDamage's plain quit removes the reciprocal opponent"
    );
    assert_eq!(
        victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .filter(|entry| {
                entry.line_type == LogLineType::Event
                    && entry.info == StimulusType::EventQuitSwordfight as u16
            })
            .count(),
        1,
        "the pre-existing-unconscious translation owns exactly one plain quit"
    );
    assert_eq!(
        victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .filter(|entry| {
                entry.line_type == LogLineType::Event
                    && entry.info == StimulusType::EventLoseConsciousness as u16
            })
            .count(),
        0,
        "the hit must not replay SetConcussion's KO callback"
    );
    assert_eq!(
        engine
            .feedback
            .titbit_manager
            .titbits()
            .iter()
            .filter(|titbit| {
                titbit.kind == crate::titbit::TitbitKind::UnconsciousStar
                    && titbit
                        .element_supplier
                        .is_some_and(|supplier| supplier.0 == victim.index())
            })
            .count(),
        0,
        "the hit must not recreate the existing unconscious star"
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .orders
            .iter()
            .any(|order| order.order_type == OrderType::FallingBackSword),
        "upright WaitingSword translation still queues FallingBackSword"
    );
}

#[test]
fn protected_preexisting_unconscious_smalltalk_hit_has_no_translation() {
    use crate::ai::{LogLineType, StimulusType};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.element_data_mut().posture = Posture::Upright;
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        victim_entity.human_data_mut().unwrap().unconscious = true;
        victim_entity.npc_data_mut().unwrap().eye_status = EyeStatus::Closed;
        victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
        victim_entity
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
    }
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(victim);

    let mut assets = assets_with_sword_profile(1, 50);
    std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0]
        .protection_by_localization = [99; 5];

    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data = crate::sequence::SequenceElementData::new_sword_damage(
        attacker,
        SwordStrike::SmalltalkRight,
        1,
    );
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.dispatch_receive_damage(&sim, &assets, victim, sequence, 0);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_entity.human_data().unwrap().opponents,
        vec![attacker],
        "NO_DAMAGE must not enter TranslateSwordDamage's plain-quit path"
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![victim]
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .orders
            .is_empty(),
        "NO_DAMAGE must not translate a FallingBack/Roll order"
    );
    assert!(
        !victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .any(|entry| {
                entry.line_type == LogLineType::Event
                    && matches!(
                        entry.info,
                        value if value == StimulusType::EventQuitSwordfight as u16
                            || value == StimulusType::EventLoseConsciousness as u16
                    )
            }),
        "NO_DAMAGE must neither quit nor replay the knockout callback"
    );
}

#[test]
fn grounded_preexisting_unconscious_smalltalk_hit_terminates_without_quit() {
    use crate::ai::{LogLineType, StimulusType};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.element_data_mut().posture = Posture::Lying;
        victim_entity.human_data_mut().unwrap().unconscious = true;
        victim_entity.npc_data_mut().unwrap().eye_status = EyeStatus::Closed;
        victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
        victim_entity
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
    }
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(victim);

    let assets = assets_with_sword_profile(1, 50);
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data = crate::sequence::SequenceElementData::new_sword_damage(
        attacker,
        SwordStrike::SmalltalkRight,
        1,
    );
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.dispatch_receive_damage(&sim, &assets, victim, sequence, 0);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_entity.npc_data().unwrap().eye_status,
        EyeStatus::Closed
    );
    assert_eq!(
        victim_entity.human_data().unwrap().opponents,
        vec![attacker]
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![victim]
    );
    let damage = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap();
    assert_eq!(damage.state, crate::sequence::SequenceState::Terminated);
    assert!(damage.orders.is_empty());
    assert!(
        !victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .any(|entry| {
                entry.line_type == LogLineType::Event
                    && matches!(
                        entry.info,
                        value if value == StimulusType::EventQuitSwordfight as u16
                            || value == StimulusType::EventLoseConsciousness as u16
                    )
            })
    );
}

#[test]
fn lethal_sword_hit_kills_unconscious_npc_before_say_ouch_translation() {
    use crate::ai::{AiState, LogLineType, Remark, Substate};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.npc_data_mut().unwrap().life_points = 15;
        victim_entity.human_data_mut().unwrap().unconscious = true;
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let ai = victim_entity.enemy_ai_mut().unwrap();
        ai.hth_weapon_id = 1;
        ai.base.current_state = AiState::Sleeping;
        ai.base.current_substate = Substate::SleepingUnconscious;
    }

    let assets = assets_with_sword_profile_effects(1, 50, 100, 0);
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence, 0),
    );

    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(victim_entity.is_dead());
    assert!(!victim_entity.human_data().unwrap().unconscious);
    let ai = victim_entity.ai_controller().unwrap();
    assert_eq!(ai.current_substate, Substate::SleepingForever);
    assert!(ai.ai_log.iter().any(|entry| {
        entry.line_type == LogLineType::Speak && entry.info == Remark::Dies as u16
    }));
    assert!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .orders
            .iter()
            .any(|order| order.order_type == OrderType::DyingSword),
        "TranslateSwordDamage must retain ownership of the dying visual after synchronous Kill"
    );
}

#[test]
fn nonlethal_sword_hit_keeps_unconscious_npc_silent() {
    use crate::ai::{AiState, LogLineType, Substate};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.human_data_mut().unwrap().unconscious = true;
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let ai = victim_entity.enemy_ai_mut().unwrap();
        ai.hth_weapon_id = 1;
        ai.base.current_state = AiState::Sleeping;
        ai.base.current_substate = Substate::SleepingUnconscious;
    }

    let assets = assets_with_sword_profile_effects(1, 50, 1, 0);
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence, 0),
    );

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(victim_entity.npc_data().unwrap().life_points, 49);
    assert!(victim_entity.human_data().unwrap().unconscious);
    assert!(
        !victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .any(|entry| entry.line_type == LogLineType::Speak),
        "the ordinary unconscious SayOuch early return must remain intact for survivors"
    );
}

#[test]
fn killing_seeking_enemy_clears_only_its_beggar_detectables() {
    use crate::ai::{AiState, Substate};
    use crate::element::{Detectable, DetectableType};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        let ai = victim_entity.enemy_ai_mut().unwrap();
        ai.hth_weapon_id = 1;
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingArrowReactiontime;
        let npc = victim_entity.npc_data_mut().unwrap();
        for detectable_type in [DetectableType::Enemy, DetectableType::Beggar] {
            npc.detectable_lists[detectable_type as usize].push(Detectable {
                element: Some(attacker),
                detectable_type,
                ..Detectable::default()
            });
        }
    }
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.apply_nonvisual_death_cascade(
        &sim,
        &assets_with_sword_profile_effects(1, 50, 100, 0),
        victim,
        (sequence, 0),
        true,
    );

    let victim_entity = engine.get_entity(victim).unwrap();
    let npc = victim_entity.npc_data().unwrap();
    assert!(npc.detectable_lists[DetectableType::Beggar as usize].is_empty());
    assert_eq!(
        npc.detectable_lists[DetectableType::Enemy as usize]
            .iter()
            .map(|detectable| detectable.element)
            .collect::<Vec<_>>(),
        vec![Some(attacker)],
        "Enemy SetState only deletes the Beggar bucket when leaving Seeking"
    );
    let ai = victim_entity.ai_controller().unwrap();
    assert_eq!(ai.current_state, AiState::Sleeping);
    assert_eq!(ai.current_substate, Substate::SleepingForever);
}

#[test]
fn lethal_push_runs_npc_kill_cascade_before_owning_the_fall() {
    use crate::ai::{AiState, AlertLevel, Substate};
    use crate::element::{Detectable, DetectableType};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    let observer = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.npc_data_mut().unwrap().life_points = 1;
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        victim_entity
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
        let ai = victim_entity.enemy_ai_mut().unwrap();
        ai.hth_weapon_id = 1;
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfight;
        ai.base.current_music_alert_status = AlertLevel::Red;
        ai.base.view_alert_status = AlertLevel::Red;
    }
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(victim);
    {
        let observer_npc = engine
            .get_entity_mut(observer)
            .unwrap()
            .npc_data_mut()
            .unwrap();
        observer_npc.ai_brain.enemy_mut().unwrap().hth_weapon_id = 1;
        observer_npc.detectable_lists[DetectableType::Friend as usize].extend([
            Detectable {
                element: Some(victim),
                detectable_type: DetectableType::Friend,
                ..Detectable::default()
            },
            Detectable {
                element: Some(victim),
                detectable_type: DetectableType::Friend,
                ..Detectable::default()
            },
        ]);
        observer_npc.detectable_lists[DetectableType::MissedFriend as usize].push(Detectable {
            element: Some(victim),
            detectable_type: DetectableType::MissedFriend,
            ..Detectable::default()
        });
    }

    let mut assets = assets_with_sword_profile_effects(1, 50, 100, 0);
    let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::A as usize];
    thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
    thrust.repulsion = 100;
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    let score_before = engine
        .mission_domain
        .campaign
        .get_value(crate::campaign::CampaignValue::Score);
    let killed_allied_before = engine.mission_domain.mission_stat.killed_allied_count;

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence, 0),
    );

    let victim_entity = engine.get_entity(victim).unwrap();
    let victim_ai = victim_entity.ai_controller().unwrap();
    assert!(victim_entity.is_dead());
    assert_eq!(victim_ai.current_state, AiState::Sleeping);
    assert_eq!(victim_ai.current_substate, Substate::SleepingForever);
    assert_eq!(victim_ai.current_music_alert_status, AlertLevel::Green);
    assert_eq!(victim_ai.view_alert_status, AlertLevel::Green);
    assert!(victim_entity.human_data().unwrap().opponents.is_empty());
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents
            .is_empty()
    );
    let observer_npc = engine.get_entity(observer).unwrap().npc_data().unwrap();
    let remaining_friends = &observer_npc.detectable_lists[DetectableType::Friend as usize];
    assert_eq!(
        remaining_friends.len(),
        1,
        "Original death fan-out deletes only the first duplicate Friend entry"
    );
    assert_eq!(remaining_friends[0].element, Some(victim));
    assert!(
        observer_npc.detectable_lists[DetectableType::MissedFriend as usize].is_empty(),
        "the ordinary unique MissedFriend entry is still removed"
    );
    let damage = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("push damage remains the visual owner");
    assert_eq!(
        damage
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![OrderType::FallingPushedWithSword]
    );
    assert_eq!(
        engine
            .mission_domain
            .campaign
            .get_value(crate::campaign::CampaignValue::Score),
        score_before + 50,
        "the Lacklandist lethal push applies the Kill score exactly once"
    );
    assert_eq!(
        engine.mission_domain.mission_stat.killed_allied_count, killed_allied_before,
        "an enemy death must not enter the allied-death statistic arm"
    );
}

#[test]
fn surviving_push_does_not_run_npc_kill_cascade() {
    use crate::ai::{AiState, AlertLevel, Substate};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        victim_entity
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker);
        let ai = victim_entity.enemy_ai_mut().unwrap();
        ai.hth_weapon_id = 1;
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingSwordfight;
        ai.base.current_music_alert_status = AlertLevel::Red;
        ai.base.view_alert_status = AlertLevel::Red;
    }
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(victim);

    let mut assets = assets_with_sword_profile_effects(1, 50, 4, 0);
    let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::A as usize];
    thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
    thrust.repulsion = 100;
    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence, 0),
    );

    let victim_entity = engine.get_entity(victim).unwrap();
    let victim_ai = victim_entity.ai_controller().unwrap();
    assert!(get_life_points(victim_entity) > 0);
    assert_eq!(victim_ai.current_state, AiState::Attacking);
    assert_eq!(victim_ai.current_substate, Substate::AttackingSwordfight);
    assert_eq!(victim_ai.current_music_alert_status, AlertLevel::Red);
    assert_eq!(
        victim_entity.human_data().unwrap().opponents,
        vec![attacker]
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![victim]
    );
}

#[test]
fn surviving_push_sword_knockout_applies_one_ko_callback_and_star() {
    use crate::ai::{LogLineType, StimulusType};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    let mut assets = assets_with_sword_profile_effects(1, 50, 4, 100);
    let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::A as usize];
    thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
    thrust.repulsion = 100;

    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    let sequence_id = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::A),
        Some(1),
        (sequence_id, 0),
    );

    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(victim_entity.human_data().unwrap().unconscious);
    assert!(
        get_life_points(victim_entity) > 0,
        "fixture must exercise a surviving push knockout"
    );
    let lose_consciousness_callbacks = victim_entity
        .ai_controller()
        .unwrap()
        .ai_log
        .iter()
        .filter(|entry| {
            entry.line_type == LogLineType::Event
                && entry.info == StimulusType::EventLoseConsciousness as u16
        })
        .count();
    assert_eq!(
        lose_consciousness_callbacks, 1,
        "TranslatePushDamage must not repeat SetConcussionOfTheBrain's synchronous callback"
    );
    assert_eq!(
        victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .filter(|entry| {
                entry.line_type == LogLineType::Event
                    && entry.info == StimulusType::EventQuitSwordfight as u16
            })
            .count(),
        2,
        "fresh animated push owns SetConcussion's first quit and TranslatePushDamage's second quit"
    );
    assert_eq!(
        engine
            .feedback
            .titbit_manager
            .titbits()
            .iter()
            .filter(|titbit| {
                titbit.kind == crate::titbit::TitbitKind::UnconsciousStar
                    && titbit
                        .element_supplier
                        .is_some_and(|supplier| supplier.0 == victim.index())
            })
            .count(),
        1,
        "a fresh push knockout creates one unconscious-star visual"
    );
}

#[test]
fn no_animation_fresh_push_knockout_does_not_repeat_ko_side_effects() {
    use crate::ai::{LogLineType, StimulusType};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.element_data_mut().posture = Posture::Carried;
        victim_entity.human_data_mut().unwrap().unconscious = true;
        victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
    }
    let assets = assets_with_sword_profile(1, 50);
    // Model SetConcussionOfTheBrain's already-completed fresh-KO prefix,
    // then enter the no-animation TranslatePushDamage arm.
    engine.apply_knockout_side_effects(&sim, &assets, victim, true, false);
    let damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .set_translating_element(Some((
            victim,
            crate::sequence::SequenceElementRef::new(sequence, 0),
        )));

    assert!(engine.apply_push_effect(
        &sim,
        &assets,
        victim,
        attacker,
        &PushStrikeInfo { repulsion: 100 },
        combat::SwordDamageResult::STUNNING_DAMAGE,
        (sequence, 0),
        true,
    ));
    engine.orders.sequence_manager.set_translating_element(None);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(victim_entity.human_data().unwrap().unconscious);
    assert_eq!(victim_entity.element_data().posture, Posture::Lying);
    assert_eq!(
        victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .filter(|entry| {
                entry.line_type == LogLineType::Event
                    && entry.info == StimulusType::EventLoseConsciousness as u16
            })
            .count(),
        1,
        "the no-animation TranslatePushDamage arm must not repeat a fresh KO callback"
    );
}

#[test]
fn preexisting_unconscious_push_preserves_closed_eyes_without_replaying_ko() {
    use crate::ai::{LogLineType, StimulusType};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::ZERO
        },
        None,
    ));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.element_data_mut().posture = Posture::Upright;
        victim_entity.human_data_mut().unwrap().unconscious = true;
        victim_entity.npc_data_mut().unwrap().eye_status = EyeStatus::Closed;
        victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
    }
    engine
        .get_entity_mut(victim)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(attacker);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(victim);
    let assets = assets_with_sword_profile(1, 50);
    let damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    let sequence = engine.orders.sequence_manager.launch_element(damage);

    assert!(engine.apply_push_effect(
        &sim,
        &assets,
        victim,
        attacker,
        &PushStrikeInfo { repulsion: 100 },
        combat::SwordDamageResult::STUNNING_DAMAGE,
        (sequence, 0),
        false,
    ));

    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(victim_entity.human_data().unwrap().unconscious);
    assert_eq!(
        victim_entity.npc_data().unwrap().eye_status,
        EyeStatus::Closed
    );
    assert_eq!(
        victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .filter(|entry| {
                entry.line_type == LogLineType::Event
                    && entry.info == StimulusType::EventLoseConsciousness as u16
            })
            .count(),
        0,
        "TranslatePushDamage must not replay SetConcussion's conscious-to-unconscious callback"
    );
    assert_eq!(
        engine
            .feedback
            .titbit_manager
            .titbits()
            .iter()
            .filter(|titbit| {
                titbit.kind == crate::titbit::TitbitKind::UnconsciousStar
                    && titbit
                        .element_supplier
                        .is_some_and(|supplier| supplier.0 == victim.index())
            })
            .count(),
        0,
        "TranslatePushDamage must not recreate the existing unconscious star"
    );
    assert!(
        victim_entity.human_data().unwrap().opponents.is_empty(),
        "the animated translation removes the victim's opponent"
    );
    assert!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents
            .is_empty(),
        "the animated translation removes the reciprocal opponent"
    );
    assert_eq!(
        victim_entity
            .ai_controller()
            .unwrap()
            .ai_log
            .iter()
            .filter(|entry| {
                entry.line_type == LogLineType::Event
                    && entry.info == StimulusType::EventQuitSwordfight as u16
            })
            .count(),
        1,
        "pre-existing unconscious animated translation owns exactly one plain quit"
    );
    let orders = &engine
        .orders
        .sequence_manager
        .get_sequence(sequence)
        .unwrap()
        .elements[0]
        .orders;
    assert!(
        orders
            .iter()
            .any(|order| order.order_type == OrderType::FallingPushedWithSword),
        "the already-unconscious victim still receives the authored push animation"
    );
}

#[test]
fn parried_true_circle_still_queues_push_fall() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let assets = assets_with_nonstraight_profile(
        SwordStrike::H,
        crate::profiles::WeaponThrustKind::TrueCircle,
    );

    let mut damage_sequence = crate::sequence::Sequence::new();
    let mut damage_element =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage_element.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::H, 1);
    damage_sequence.append_element(damage_element);
    let damage_sequence_id = engine
        .orders
        .sequence_manager
        .launch_sequence(damage_sequence);
    engine
        .orders
        .sequence_manager
        .element_in_progress(damage_sequence_id, 0);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::ParryingSword;
    let victim_position_before = engine
        .get_entity(victim)
        .unwrap()
        .element_data()
        .position_map();
    let victim_moving_before = engine
        .get_entity(victim)
        .unwrap()
        .position_iface()
        .is_moving_map();

    engine.apply_sword_damage(
        &sim,
        &assets,
        victim,
        Some(attacker),
        Some(SwordStrike::H),
        Some(1),
        (damage_sequence_id, 0),
    );

    let damage = engine
        .orders
        .sequence_manager
        .get_element(damage_sequence_id, 0)
        .expect("parried push damage must retain its sequence element");
    assert_eq!(
        damage
            .orders
            .back()
            .expect("Original TranslatePushDamage queues a fall even when the hit is parried")
            .order_type,
        OrderType::FallingPushedWithSword
    );
    assert!(
        damage
            .orders
            .iter()
            .filter(|order| order.order_type != OrderType::Rolling)
            .all(|order| !order.compute_direction),
        "TranslatePushDamage sets bComputeDirection=false on the falling-pushed order"
    );
    let victim_after_translation = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_after_translation.element_data().position_map(),
        victim_position_before,
        "TranslatePushDamage only queues the falling order; ExecuteFallingPushed owns movement"
    );
    assert_eq!(
        victim_after_translation.position_iface().is_moving_map(),
        victim_moving_before,
        "translation must not introduce movement before the falling order executes"
    );

    // Model the replay boundary: the damage element has authored a push
    // fall, but the victim's still-selected order is its postponed parry.
    // ReadyForTakeOff must not initialize until FallingPushedWithSword
    // becomes current and reports Start.
    engine
        .orders
        .sequence_manager
        .postpone_element(damage_sequence_id, 0);
    let mut parry_sequence = crate::sequence::Sequence::new();
    let mut parry_element =
        crate::sequence::SequenceElement::new(1, Command::ParrySword, Some(victim));
    parry_element.orders.push_back(crate::order::Order::new(
        OrderType::ParryingSword,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    parry_sequence.append_element(parry_element);
    let parry_sequence_id = engine
        .orders
        .sequence_manager
        .launch_sequence(parry_sequence);
    engine
        .orders
        .sequence_manager
        .element_in_progress(parry_sequence_id, 0);
    assert!(
        engine
            .get_entity(victim)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_flight
            .is_none(),
        "TranslatePushDamage must not run ReadyForTakeOff eagerly"
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(victim)
            .map(|(_, _, order)| order.order_type),
        Some(OrderType::ParryingSword)
    );
    engine.tick_push_flights(&sim, &assets);
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .element_data()
            .position_map(),
        victim_position_before,
        "prepared push flight must wait behind the still-selected parry order"
    );

    engine
        .orders
        .sequence_manager
        .element_terminated(parry_sequence_id, 0);
    engine
        .orders
        .sequence_manager
        .element_in_progress(damage_sequence_id, 0);
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.set_posture(Posture::Flying);
        victim_entity
            .actor_data_mut()
            .unwrap()
            .execute_order_initialising = true;
    }
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(victim)
            .map(|(_, _, order)| order.order_type),
        Some(OrderType::FallingPushedWithSword)
    );
    let fall_script = crate::sprite_script::SpriteScript {
        action_id: OrderType::FallingBackSword as u16,
        action_done: 1,
        frame_ids: vec![1, 2],
        delays: vec![0, 0],
        distances: vec![0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
        sound_ids: vec![0, 0],
        ..Default::default()
    };
    let mut fall_conversion =
        vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
    fall_conversion[OrderType::FallingBackSword as usize] = 0;
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        let position_iface = victim_entity.element_data().sprite.position_iface.clone();
        let mut sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![fall_script; 16]),
            std::sync::Arc::new(fall_conversion),
        );
        sprite.position_iface = position_iface;
        victim_entity.element_data_mut().sprite = sprite;
    }
    let material_before_takeoff = engine.get_entity(victim).unwrap().element_data().material();
    engine.initialize_push_flight(
        &assets,
        victim,
        (damage_sequence_id, 0),
        OrderType::FallingPushedWithSword,
    );
    assert_eq!(
        engine.get_entity(victim).unwrap().element_data().material(),
        material_before_takeoff,
        "ReadyForTakeOff installs only the goal obstacle/plane, not its material"
    );
    let rejected_flight = engine
        .get_entity(victim)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_flight
        .expect("ReadyForTakeOff must retain a fully rejected flight");
    assert_eq!(rejected_flight.increment_x, 0.0);
    assert_eq!(rejected_flight.increment_y, 0.0);
    assert_eq!(rejected_flight.increment_z, 0.0);
    let accepted_increment = 1.0;
    engine
        .get_entity_mut(victim)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_flight = Some(ActiveFlight {
        increment_x: accepted_increment,
        goal_x: victim_position_before.x + 8.0,
        goal_y: victim_position_before.y,
        frames_remaining: 8,
        antagonist: Some(attacker),
        ..Default::default()
    });
    engine.tick_push_flights(&sim, &assets);
    let victim_after_fall_start = engine.get_entity(victim).unwrap();
    assert_eq!(
        victim_after_fall_start.element_data().posture,
        Posture::Flying
    );
    assert_eq!(
        victim_after_fall_start.element_data().position_map(),
        crate::coordinates::MapPoint::new(
            victim_position_before.x + accepted_increment,
            victim_position_before.y
        ),
        "PerformFlight applies its first increment on the Start Execute"
    );
    assert_eq!(
        victim_after_fall_start
            .actor_data()
            .unwrap()
            .active_flight
            .unwrap()
            .frames_remaining,
        7
    );

    engine
        .get_entity_mut(victim)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .execute_order_initialising = false;
    engine.tick_push_flights(&sim, &assets);
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .element_data()
            .position_map(),
        crate::coordinates::MapPoint::new(
            victim_position_before.x + 2.0 * accepted_increment,
            victim_position_before.y
        ),
        "the following Execute applies the second push-flight increment"
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(attacker, |command| {
                command == Command::Provoke
            }),
        "parried push strikes still skip the later provoke branch"
    );
}

#[test]
fn pushed_flight_starts_from_cached_takeoff_elevation_after_installing_goal_plane() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        crate::position_interface::SectorHandle::new(32),
    ));

    // The landing projection is ten units above the takeoff point. An
    // empty test grid rejects the horizontal push, which isolates the
    // vertical ReadyForTakeOff behavior: installing this goal plane must
    // not eagerly lift the actor before PerformFlight's first increment.
    let mut obstacle = crate::sight_obstacle::SightObstacle::new(
        0,
        crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
    );
    obstacle.set_projection_area_ref(
        crate::position_interface::Layer::ZERO,
        crate::fast_find_grid::SectorIndex::new(32).unwrap(),
    );
    obstacle.obstacle_points = vec![
        crate::sight_obstacle::ObstaclePoint {
            x: -1000.0,
            y: -1000.0,
            z_top: 10.0,
            z_bottom: 0.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: 1000.0,
            y: -1000.0,
            z_top: 10.0,
            z_bottom: 0.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: 1000.0,
            y: 1000.0,
            z_top: 10.0,
            z_bottom: 0.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: -1000.0,
            y: 1000.0,
            z_top: 10.0,
            z_bottom: 0.0,
        },
    ];
    obstacle.top_plane_points = [
        [-1000.0, -1000.0, 10.0],
        [1000.0, -1000.0, 10.0],
        [-1000.0, 1000.0, 10.0],
    ];
    obstacle.rebuild_geometry();

    let mut assets = assets_with_nonstraight_profile(
        SwordStrike::H,
        crate::profiles::WeaponThrustKind::TrueCircle,
    );
    assets.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);

    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::H, 1);
    damage.orders.push_back(crate::order::Order::new(
        OrderType::FallingPushedWithSword,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .set_posture(Posture::Flying);

    engine.initialize_push_flight(
        &assets,
        victim,
        (sequence, 0),
        OrderType::FallingPushedWithSword,
    );
    let flight = engine
        .get_entity(victim)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_flight
        .expect("elevated landing plane must author a flight");
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .sprite
        .scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
        frame_ids: vec![0, 1],
        ..Default::default()
    }]);
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .position_iface()
            .get_elevation()
            .to_bits(),
        0.0_f32.to_bits(),
        "SetObstacle must preserve ReadyForTakeOff's cached starting 3D point"
    );

    engine.tick_push_flights(&sim, &assets);
    let position = engine
        .get_entity(victim)
        .unwrap()
        .position_iface()
        .get_position();
    assert_eq!(
        position.z.to_bits(),
        flight.increment_z.to_bits(),
        "the first PerformFlight tick advances from takeoff Z, not the landing plane"
    );
    assert_eq!(
        position.y.to_bits(),
        (100.0_f32 + flight.increment_y).to_bits(),
        "PerformFlight accumulates the authored world-space Y increment before re-projecting map Y"
    );
}

#[test]
fn hit_flight_starts_from_cached_takeoff_elevation_after_installing_goal_plane() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        crate::position_interface::SectorHandle::new(32),
    ));

    let mut obstacle = crate::sight_obstacle::SightObstacle::new(
        0,
        crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
    );
    obstacle.set_projection_area_ref(
        crate::position_interface::Layer::ZERO,
        crate::fast_find_grid::SectorIndex::new(32).unwrap(),
    );
    obstacle.obstacle_points = vec![
        crate::sight_obstacle::ObstaclePoint {
            x: -1000.0,
            y: -1000.0,
            z_top: 10.0,
            z_bottom: 0.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: 1000.0,
            y: -1000.0,
            z_top: 10.0,
            z_bottom: 0.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: 1000.0,
            y: 1000.0,
            z_top: 10.0,
            z_bottom: 0.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: -1000.0,
            y: 1000.0,
            z_top: 10.0,
            z_bottom: 0.0,
        },
    ];
    obstacle.top_plane_points = [
        [-1000.0, -1000.0, 10.0],
        [1000.0, -1000.0, 10.0],
        [-1000.0, 1000.0, 10.0],
    ];
    obstacle.rebuild_geometry();
    let mut assets = LevelAssets::new();
    assets.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);

    let mut damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveHitDamage,
        Some(victim),
        Some(attacker),
        1,
        0,
    );
    let mut fall = crate::order::Order::new(
        OrderType::FallingHitUpright,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    );
    fall.antagonist = Some(attacker);
    damage.orders.push_back(fall);
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .set_posture(Posture::Flying);

    engine.initialize_hit_flight(
        &assets,
        victim,
        Some(attacker),
        OrderType::FallingHitUpright,
    );
    let flight = engine
        .get_entity(victim)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_flight
        .expect("elevated landing plane must author a hit flight");
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .sprite
        .scripts = std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
        frame_ids: vec![0, 1],
        ..Default::default()
    }]);
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .position_iface()
            .get_elevation()
            .to_bits(),
        0.0_f32.to_bits(),
        "FallingHit must retain ReadyForTakeOff's cached starting 3D point"
    );

    engine.tick_push_flights(&sim, &assets);
    let position = engine
        .get_entity(victim)
        .unwrap()
        .position_iface()
        .get_position();
    assert_eq!(position.z.to_bits(), flight.increment_z.to_bits());
    assert_eq!(
        position.y.to_bits(),
        (100.0_f32 + flight.increment_y).to_bits(),
        "FallingHit accumulates the authored world-space Y increment"
    );
}

#[test]
fn damage_to_already_dead_pc_does_not_repeat_virtual_kill() {
    let sim = crate::sim_rng::SimulationContext::with_seed(0x181);
    let mut engine = make_engine();
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let Entity::Pc(pc) = engine.get_entity_mut(victim).unwrap() else {
        unreachable!()
    };
    pc.pc.life_points = 0;
    pc.pc.trumpet_enabled = false;

    let seed_before = sim.seed();
    engine.handle_post_damage(
        &sim,
        &LevelAssets::new(),
        victim,
        0,
        false,
        None,
        false,
        (crate::sequence::SequenceId(999), 0),
        None,
    );

    assert_eq!(
        sim.seed(),
        seed_before,
        "SetLifePoints returns before the repeated Kill cascade can select a replacement peasant"
    );
    let Entity::Pc(pc) = engine.get_entity(victim).unwrap() else {
        unreachable!()
    };
    assert!(!pc.pc.trumpet_enabled);
}

#[test]
fn charge_hit_on_already_dead_pc_does_not_repeat_virtual_kill_rng() {
    let sim = crate::sim_rng::SimulationContext::with_seed(0x182);
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;
    {
        let victim = engine.get_entity_mut(victim).unwrap();
        victim.pc_data_mut().unwrap().life_points = 0;
        victim.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
    }

    // Keep an eligible replacement in the gang so replaying the PC Kill
    // cascade would observably consume CampaignReinforcementPeasant.
    engine.mission_domain.campaign.characters = vec![
        crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
            instanced: true,
            ..Default::default()
        },
        crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(1)),
            ..Default::default()
        },
    ];
    engine.mission_domain.campaign.gang_indices = vec![0, 1];

    let mut assets = assets_with_nonstraight_profile(
        SwordStrike::Charge,
        crate::profiles::WeaponThrustKind::Straight,
    );
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .push(crate::profiles::CharacterProfile::default());
    let damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveSwordDamage,
        Some(victim),
        Some(attacker),
        1,
        0,
    );
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    let (_, draws) = crate::sim_rng::with_draw_trace(|| {
        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::Charge),
            Some(1),
            (sequence, 0),
        );
    });

    assert_eq!(
        draws,
        vec![
            crate::sim_rng::RngSite::SwordDamageProtection,
            crate::sim_rng::RngSite::SwordDamageProtection,
            crate::sim_rng::RngSite::MeleeProvoke,
        ],
        "SetLifePoints returns before Kill, while TranslatePushDamage only owns the visual response"
    );
    assert_eq!(engine.mission_domain.campaign.gang_indices, vec![0, 1]);
    assert!(
        !engine
            .get_entity(victim)
            .unwrap()
            .pc_data()
            .unwrap()
            .trumpet_enabled,
        "an already-dead PC must not be offered another replacement"
    );
}

#[test]
fn lethal_sword_hit_preserves_queued_second_damage_fifo() {
    let sim = crate::sim_rng::SimulationContext::with_seed(0x38);
    let mut engine = make_engine();
    let attacker_a = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let attacker_b = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    for attacker in [attacker_a, attacker_b] {
        let Entity::Soldier(attacker_entity) = engine.get_entity_mut(attacker).unwrap() else {
            unreachable!()
        };
        let crate::element::AiBrain::Enemy(attacker_ai) = &mut attacker_entity.npc.ai_brain else {
            unreachable!()
        };
        attacker_ai.hth_weapon_id = 1;
    }
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.pc_data_mut().unwrap().life_points = 1;
        victim_entity.actor_data_mut().unwrap().action_state =
            crate::element::ActionState::WaitingSword;
    }

    let queue_damage = |engine: &mut EngineInner, attacker| {
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        engine.resolve_element_priority(&mut damage);
        engine.orders.sequence_manager.launch_element(damage)
    };
    let first_damage = queue_damage(&mut engine, attacker_a);
    let second_damage = queue_damage(&mut engine, attacker_b);

    let mut unrelated = crate::sequence::SequenceElement::new(1, Command::WaitTimer, Some(victim));
    engine.resolve_element_priority(&mut unrelated);
    let unrelated = engine.orders.sequence_manager.launch_element(unrelated);

    let assets = assets_with_sword_profile(200, 30);
    let (_, draws) = crate::sim_rng::with_draw_trace(|| {
        engine.hourglass_phase_sequences(
            &sim,
            &mut crate::engine::HostDisplayState::default(),
            &assets,
        );
    });

    assert_eq!(
        draws,
        vec![
            crate::sim_rng::RngSite::SwordDamageProtection,
            crate::sim_rng::RngSite::SwordDamageProtection,
            crate::sim_rng::RngSite::MeleeProvoke,
            crate::sim_rng::RngSite::SwordDamageProtection,
            crate::sim_rng::RngSite::SwordDamageProtection,
            crate::sim_rng::RngSite::MeleeProvoke,
        ],
        "both simultaneous sword hits must execute their exact damage RNG sites"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(second_damage, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::InProgress,
        "the already-dead second hit must translate into its own live dying order"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(unrelated, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Interrupted,
        "death cleanup must still discard unrelated queued owner work"
    );
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .pc_data()
            .unwrap()
            .life_points,
        0
    );
    assert_ne!(
        engine
            .orders
            .sequence_manager
            .get_element(first_damage, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Todo
    );
    assert_eq!(engine.actor_command(victim), Command::ReceiveSwordDamage);
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .actor_data()
            .unwrap()
            .installed_order
            .map(|order| order.order_type),
        Some(crate::order::OrderType::DyingSword),
        "the second damage card replaces the first while retaining Original's dying-sword lifecycle"
    );
}

#[test]
fn sword_damage_on_dying_pc_preserves_the_fresh_sprite_start() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let Entity::Soldier(attacker_entity) = engine.get_entity_mut(attacker).unwrap() else {
        unreachable!()
    };
    let crate::element::AiBrain::Enemy(attacker_ai) = &mut attacker_entity.npc.ai_brain else {
        unreachable!()
    };
    attacker_ai.hth_weapon_id = 1;
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.pc_data_mut().unwrap().life_points = 0;
        victim_entity.set_posture(Posture::Dead);
        let actor = victim_entity.actor_data_mut().unwrap();
        actor.action_state = crate::element::ActionState::WaitingSword;
        actor.continuation.motion_state = crate::sprite::MotionState::Start;
    }

    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    engine.resolve_element_priority(&mut damage);
    let damage_sequence = engine.orders.sequence_manager.launch_element(damage);

    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets_with_sword_profile(200, 30));

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(damage_sequence, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        crate::sprite::MotionState::Start,
        "TranslateSwordDamage changes the selected pointer before Actor::Instruct can stamp InProgress"
    );
}

#[test]
fn lethal_sword_damage_to_grounded_non_rider_publishes_dead_before_terminating() {
    for initial_posture in [
        Posture::Lying,
        Posture::StuckUnderNet,
        Posture::Flying,
        Posture::Carried,
        Posture::OnShoulders,
        Posture::Tied,
    ] {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        {
            let victim_entity = engine.get_entity_mut(victim).unwrap();
            victim_entity.element_data_mut().posture = initial_posture;
            victim_entity.npc_data_mut().unwrap().life_points = 1;
            victim_entity.enemy_ai_mut().unwrap().hth_weapon_id = 1;
        }
        let assets = assets_with_sword_profile_effects(200, 50, 100, 0);
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::A),
            Some(1),
            (sequence, 0),
        );

        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("grounded sword damage remains registered");
        assert_eq!(
            engine.get_entity(victim).unwrap().element_data().posture,
            Posture::Dead,
            "TranslateSwordDamage must publish Dead for lethal {initial_posture:?} non-riders"
        );
        assert_eq!(element.state, crate::sequence::SequenceState::Terminated);
        assert!(
            element.orders.is_empty(),
            "grounded lethal {initial_posture:?} must not author a replacement animation"
        );
    }
}

#[test]
fn grounded_sword_damage_preserves_living_and_dead_rider_posture_controls() {
    for (life_points, rider, expected_state) in [
        (50, false, crate::sequence::SequenceState::Terminated),
        (1, true, crate::sequence::SequenceState::InProgress),
    ] {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: 10.0,
                ..WorldPoint3D::ZERO
            },
            None,
        ));
        {
            let Entity::Soldier(victim_entity) = engine.get_entity_mut(victim).unwrap() else {
                unreachable!()
            };
            victim_entity.element.posture = Posture::Lying;
            victim_entity.npc.life_points = life_points;
            victim_entity.soldier.rider = rider;
            victim_entity
                .npc
                .ai_brain
                .enemy_mut()
                .unwrap()
                .hth_weapon_id = 1;
        }
        let assets = if rider {
            assets_with_sword_profile_effects(200, 50, 100, 0)
        } else {
            assets_with_sword_profile_effects(1, 50, 1, 0)
        };
        let mut damage =
            crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
        damage.data =
            crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
        let sequence = engine.orders.sequence_manager.launch_element(damage);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);

        engine.apply_sword_damage(
            &sim,
            &assets,
            victim,
            Some(attacker),
            Some(SwordStrike::A),
            Some(1),
            (sequence, 0),
        );

        assert_eq!(
            engine.get_entity(victim).unwrap().element_data().posture,
            Posture::Lying,
            "living grounded actors and lethal riders bypass the Dead rewrite"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .state,
            expected_state,
            "dead riders fall through while living grounded non-riders terminate"
        );
    }
}

#[test]
fn sword_damage_amulet_coma_preserves_carried_body_and_terminates_during_translation() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let carried = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let Entity::Soldier(attacker_entity) = engine.get_entity_mut(attacker).unwrap() else {
        unreachable!()
    };
    let crate::element::AiBrain::Enemy(attacker_ai) = &mut attacker_entity.npc.ai_brain else {
        unreachable!()
    };
    attacker_ai.hth_weapon_id = 1;
    let sprite_script = crate::sprite_script::SpriteScript {
        action_id: crate::order::OrderType::WaitingUpright as u16,
        action_done: 0,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1],
        delays: vec![1],
        distances: vec![0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
        sound_ids: vec![0],
    };
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![sprite_script]),
        std::sync::Arc::new(vec![0]),
    );
    let mut assets = assets_with_sword_profile(200, 30);
    std::sync::Arc::make_mut(&mut assets.profile_manager).characters[0].vip = true;
    engine.mission_domain.campaign.values[crate::campaign::CampaignValue::Amulets] = 1;

    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.pc_data_mut().unwrap().life_points = 1;
        victim_entity.set_posture(Posture::CarryingCorpse);
        victim_entity.pc_data_mut().unwrap().carried = Some(carried);
        victim_entity
            .pc_data_mut()
            .unwrap()
            .set_live_carried_posture(Posture::Tied);
        victim_entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
        victim_entity
            .position_iface_mut()
            .set_map_goal(crate::coordinates::MapPoint::new(25.0, 100.0));
        victim_entity
            .actor_data_mut()
            .unwrap()
            .continuation
            .motion_state = crate::sprite::MotionState::Start;
    }
    {
        let carried_entity = engine.get_entity_mut(carried).unwrap();
        carried_entity.set_posture(Posture::Carried);
        carried_entity.human_data_mut().unwrap().carrier = Some(victim);
        carried_entity.actor_data_mut().unwrap().execution_frozen = true;
    }

    let mut damage =
        crate::sequence::SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data =
        crate::sequence::SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 1);
    engine.resolve_element_priority(&mut damage);
    engine.orders.sequence_manager.launch_element(damage);

    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert!(engine.mission_domain.campaign.characters[0].status.in_coma);
    assert_eq!(victim_entity.element_data().posture, Posture::Lying);
    assert_eq!(victim_entity.pc_data().unwrap().carried, Some(carried));
    assert_eq!(
        victim_entity.actor_data().unwrap().action_state,
        ActionState::Moving,
        "the coma posture change bypasses PC::TranslateSwordDamage's CarryingCorpse arm"
    );
    let carried_entity = engine.get_entity(carried).unwrap();
    assert_eq!(carried_entity.element_data().posture, Posture::Carried);
    assert_eq!(carried_entity.human_data().unwrap().carrier, Some(victim));
    assert!(carried_entity.actor_data().unwrap().execution_frozen);
    assert_eq!(
        carried_entity.actor_data().unwrap().installed_order,
        None,
        "the bypassed DropCorpse must not launch the carried body's Wait singleton"
    );
    assert_eq!(
        victim_entity.position_iface().map_goal(),
        crate::coordinates::MapPoint::ZERO,
        "translation-time termination must clear the interrupted movement goal"
    );
    assert_eq!(
        victim_entity
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        crate::sprite::MotionState::Start,
        "Actor::Instruct must preserve the motion produced before damage translation"
    );
}

#[test]
fn consecutive_lethal_arrow_damage_preserves_new_amulet_coma() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let sprite_script = crate::sprite_script::SpriteScript {
        action_id: crate::order::OrderType::WaitingUpright as u16,
        action_done: 0,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1],
        delays: vec![1],
        distances: vec![0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
        sound_ids: vec![0],
    };
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![sprite_script]),
        std::sync::Arc::new(vec![0]),
    );
    let mut assets = assets_with_sword_profile(200, 30);
    std::sync::Arc::make_mut(&mut assets.profile_manager).characters[0].vip = true;
    engine.mission_domain.campaign.values[crate::campaign::CampaignValue::Amulets] = 1;

    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.pc_data_mut().unwrap().life_points = 10;
        victim_entity
            .position_iface_mut()
            .set_map_goal(crate::coordinates::MapPoint::new(25.0, 100.0));
        victim_entity
            .actor_data_mut()
            .unwrap()
            .continuation
            .motion_state = crate::sprite::MotionState::Start;
    }

    let mut damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveArrowDamage,
        Some(victim),
        Some(attacker),
        10,
        0,
    );
    engine.resolve_element_priority(&mut damage);
    engine.orders.sequence_manager.launch_element(damage);

    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    {
        let victim_entity = engine.get_entity(victim).unwrap();
        assert!(engine.mission_domain.campaign.characters[0].status.in_coma);
        assert_eq!(victim_entity.pc_data().unwrap().life_points, 5);
        assert_eq!(victim_entity.element_data().posture, Posture::Lying);
        assert_eq!(
            victim_entity.position_iface().map_goal(),
            crate::coordinates::MapPoint::ZERO,
            "post-damage Lying translation must terminate and clear the movement goal"
        );
        assert_eq!(
            victim_entity
                .actor_data()
                .unwrap()
                .continuation
                .motion_state,
            crate::sprite::MotionState::Start,
            "terminal arrow translation must preserve the pre-damage motion state"
        );
    }
    assert_eq!(
        engine.mission_domain.campaign.values[crate::campaign::CampaignValue::Amulets],
        0,
        "the first lethal arrow must establish coma and consume one amulet"
    );

    let mut second_damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveArrowDamage,
        Some(victim),
        Some(attacker),
        10,
        0,
    );
    engine.resolve_element_priority(&mut second_damage);
    engine.orders.sequence_manager.launch_element(second_damage);
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let victim_entity = engine.get_entity(victim).unwrap();
    assert_eq!(victim_entity.pc_data().unwrap().life_points, 5);
    assert!(!victim_entity.is_dead());
    assert!(engine.mission_domain.campaign.characters[0].status.in_coma);
    assert_eq!(
        engine.mission_domain.campaign.values[crate::campaign::CampaignValue::Amulets],
        0,
        "the second lethal arrow must not consume another amulet"
    );
}

#[test]
fn same_frame_arrow_after_death_replaces_dying_order_and_then_rolls() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .life_points = 1;

    let mut obstacle = crate::sight_obstacle::SightObstacle::new_default(0);
    obstacle.top_plane_points = [[0.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 1.0, 0.0]];
    let mut assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
    assets.static_sight_obstacles = std::sync::Arc::new(vec![obstacle]);
    {
        let victim = engine.get_entity_mut(victim).unwrap();
        victim.element_data_mut().set_obstacle_index(
            crate::position_interface::ObstacleHandle::new(0),
            Some(crate::position_interface::PlaneZCoeffs {
                az: 1.0,
                bz: 0.0,
                dz: 0.0,
            }),
        );
        victim
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                crate::coordinates::MapVec::new(-5.0, -5.0),
                crate::coordinates::MapVec::new(5.0, 5.0),
            ));
    }

    let mut launched = Vec::new();
    for _ in 0..2 {
        let mut damage = crate::sequence::SequenceElement::new_damage(
            1,
            Command::ReceiveArrowDamage,
            Some(victim),
            Some(attacker),
            1,
            0,
        );
        engine.resolve_element_priority(&mut damage);
        launched.push(engine.orders.sequence_manager.launch_element(damage));
    }

    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(victim),
        Some((launched[1], 0)),
        "the second injury must replace the first dying element"
    );
    let second = engine
        .orders
        .sequence_manager
        .get_element(launched[1], 0)
        .expect("second arrow damage remains registered");
    assert_eq!(second.state, crate::sequence::SequenceState::InProgress);
    assert_eq!(
        second
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![OrderType::DyingUpright, OrderType::Rolling],
        "TranslateArrowDamage must author DyingUpright before TranslateRoll"
    );
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .actor_data()
            .unwrap()
            .installed_order
            .as_ref()
            .map(|order| order.order_type),
        Some(OrderType::DyingUpright)
    );
}

#[test]
fn arrow_damage_to_dead_grounded_actor_sets_dead_and_terminates_without_orders() {
    for (initial_posture, use_pc) in [
        (Posture::Lying, true),
        (Posture::StuckUnderNet, true),
        (Posture::Flying, true),
        (Posture::Carried, true),
        // PC virtual dispatch intercepts OnShoulders.  A Soldier reaches
        // RHElementActorHuman's literal OnShoulders fallthrough.
        (Posture::OnShoulders, false),
        (Posture::Tied, true),
    ] {
        let sim = crate::sim_rng::test_context();
        let mut engine = make_engine();
        let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
        let victim = engine.add_entity(if use_pc {
            make_pc(WorldPoint3D::ZERO, None)
        } else {
            make_soldier(WorldPoint3D::ZERO, None)
        });
        {
            let victim = engine.get_entity_mut(victim).unwrap();
            let (_, life_points) = victim
                .human_and_life_points_mut()
                .expect("grounded test victim must be human");
            *life_points = 0;
            victim.element_data_mut().posture = initial_posture;
        }
        let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
        let mut damage = crate::sequence::SequenceElement::new_damage(
            1,
            Command::ReceiveArrowDamage,
            Some(victim),
            Some(attacker),
            1,
            0,
        );
        engine.resolve_element_priority(&mut damage);
        let sequence = engine.orders.sequence_manager.launch_element(damage);

        let mut display = crate::engine::HostDisplayState::default();
        engine.hourglass_phase_sequences(&sim, &mut display, &assets);

        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("dead-body arrow damage remains registered");
        assert_eq!(
            element.state,
            crate::sequence::SequenceState::Terminated,
            "{initial_posture:?} must enter the terminating fallthrough"
        );
        assert!(element.orders.is_empty());
        assert_eq!(
            engine.get_entity(victim).unwrap().element_data().posture,
            Posture::Dead,
            "TranslateArrowDamage changes dead {initial_posture:?} non-riders to Dead"
        );
    }
}

#[test]
fn arrow_damage_to_pc_on_shoulders_uses_virtual_shoulder_translation() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_soldier(WorldPoint3D::ZERO, None));
    let carrier = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    let victim = engine.add_entity(make_pc(WorldPoint3D::ZERO, None));
    engine
        .get_entity_mut(carrier)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .carried = Some(victim);
    {
        let victim = engine.get_entity_mut(victim).unwrap();
        victim.element_data_mut().posture = Posture::OnShoulders;
        victim.human_data_mut().unwrap().carrier = Some(carrier);
    }

    let assets = action_test_assets([crate::profiles::Action::NoAction; 3]);
    let mut damage = crate::sequence::SequenceElement::new_damage(
        1,
        Command::ReceiveArrowDamage,
        Some(victim),
        Some(attacker),
        1,
        0,
    );
    engine.resolve_element_priority(&mut damage);
    let sequence = engine.orders.sequence_manager.launch_element(damage);
    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("shoulder arrow damage remains registered");
    assert_ne!(element.state, crate::sequence::SequenceState::Terminated);
    assert_eq!(
        element.orders.front().map(|order| order.order_type),
        Some(OrderType::FallingShoulders),
        "PC virtual TranslateArrowDamage must dispatch TranslateShoulderDamage"
    );
    assert_ne!(
        engine.get_entity(victim).unwrap().element_data().posture,
        Posture::Dead,
        "PC OnShoulders must not enter Human's dead-grounded fallthrough"
    );
}

/// `SwordstrikeThrustA` promotes both principal opponents before
/// the strike, so clicking a secondary opponent during a
/// swordfight switches the primary target.
#[test]
fn thrust_a_promotes_clicked_secondary_opponent() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let pc = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let current = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let clicked = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 20.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    if let Some(human) = engine.get_entity_mut(pc).unwrap().human_data_mut() {
        human.opponents = vec![current, clicked].into();
    }
    if let Some(human) = engine.get_entity_mut(clicked).unwrap().human_data_mut() {
        human.opponents = vec![current, pc].into();
    }
    engine
        .get_entity_mut(pc)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(8);
    let direction_before_dispatch = engine.get_entity(pc).unwrap().element_data().direction();
    let direction_goal_before_dispatch = engine
        .get_entity(pc)
        .unwrap()
        .position_iface()
        .get_direction_goal();

    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(crate::sequence::SequenceElement::new_interaction(
        1,
        Command::SwordstrikeThrustA,
        Some(pc),
        Some(clicked),
    ));
    let seq_id = engine.launch_sequence(sequence);
    let action_state_before_dispatch = engine
        .get_entity(pc)
        .unwrap()
        .actor_data()
        .unwrap()
        .action_state;

    engine.dispatch_sword_strike(
        sim,
        &LevelAssets::default(),
        pc,
        clicked,
        SwordStrike::A,
        seq_id,
        0,
    );
    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .actor_data()
            .unwrap()
            .action_state,
        action_state_before_dispatch,
        "Instruct must not apply the Execute MotionState::Start WaitingSword transition"
    );
    assert_eq!(
        engine.get_entity(pc).unwrap().element_data().direction(),
        direction_before_dispatch,
        "strike translation must leave facing to the following Execute call"
    );
    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .position_iface()
            .get_direction_goal(),
        direction_goal_before_dispatch,
        "strike translation must not install the Execute-time facing goal"
    );

    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![clicked, current],
        "thrust-A against an existing secondary opponent must make it principal"
    );
    assert_eq!(
        engine
            .get_entity(clicked)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![pc, current],
        "the attacker is also promoted as the target's principal opponent"
    );
}

#[test]
fn melee_direction_uses_original_aspect_ratio_classifier() {
    let mut engine = make_engine();
    let attacker = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 663.552_37,
            y: 1_755.932_5,
            z: 0.0,
        },
        None,
    ));
    let target = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 726.867_3,
            y: 1_763.275_3,
            z: 0.0,
        },
        None,
    ));

    assert_eq!(direction_to(&engine.world.entities, attacker, target), 5);
}

#[test]
fn enter_swordfight_instruct_queues_transition_without_execute_side_effects() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let owner = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let opponent = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_direction_goal(7);

    let mut element =
        crate::sequence::SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
    element.set_property(
        crate::sequence::Field::Opponent,
        crate::sequence::FieldValue::Element(opponent),
    );
    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(element);
    let seq_id = engine.launch_sequence(sequence);

    engine.dispatch_enter_swordfight(
        sim,
        &LevelAssets::default(),
        owner,
        Some(opponent),
        seq_id,
        0,
    );

    let owner_entity = engine.get_entity(owner).unwrap();
    assert_eq!(
        owner_entity.actor_data().unwrap().action_state,
        ActionState::Waiting,
        "Instruct must not apply the raising-sword Execute state"
    );
    assert_eq!(
        i16::from(owner_entity.position_iface().get_direction_goal()),
        7,
        "Instruct must not apply the raising-sword Execute facing"
    );
    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .unwrap();
    assert_eq!(element.state, crate::sequence::SequenceState::InProgress);
    let order = element.current_order().unwrap();
    assert_eq!(
        order.order_type,
        crate::order::OrderType::TransitionRaisingSword
    );
    assert_eq!(order.antagonist, Some(opponent));
    assert!(
        owner_entity
            .human_data()
            .unwrap()
            .opponents
            .contains(&opponent),
        "relationship changes still belong to Instruct"
    );
}

#[test]
fn failed_enter_swordfight_retires_matching_postponed_thrust_a() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let owner = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let opponent = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::default()
        },
        None,
    ));

    let postponed = engine.launch_element(crate::sequence::SequenceElement::new_interaction(
        1,
        Command::SwordstrikeThrustA,
        Some(owner),
        Some(opponent),
    ));
    let mut enter =
        crate::sequence::SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
    enter.set_property(
        crate::sequence::Field::Opponent,
        crate::sequence::FieldValue::Element(opponent),
    );
    let admission = engine.launch_element(enter);
    engine
        .orders
        .sequence_manager
        .set_cross_postponed_link((admission, 0), Some((postponed, 0)));

    let Entity::Pc(opponent_entity) = engine.get_entity_mut(opponent).unwrap() else {
        unreachable!("test opponent must remain a PC")
    };
    opponent_entity.pc.life_points = 0;

    engine.dispatch_enter_swordfight(
        &sim,
        &LevelAssets::default(),
        owner,
        Some(opponent),
        admission,
        0,
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(admission, 0)
            .unwrap()
            .cross_postponed,
        None,
        "failed admission must sever the restart edge before terminal callbacks"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(postponed, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Impossible,
        "the matching THRUST_A prerequisite must not recreate the failed admission"
    );
}

#[test]
fn failed_enter_swordfight_leaves_mismatched_postponed_work_untouched() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let owner = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let opponent = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    let postponed = engine.launch_element(crate::sequence::SequenceElement::new_interaction(
        1,
        Command::SwordstrikeThrustB,
        Some(owner),
        Some(opponent),
    ));
    let mut enter =
        crate::sequence::SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
    enter.set_property(
        crate::sequence::Field::Opponent,
        crate::sequence::FieldValue::Element(opponent),
    );
    let admission = engine.launch_element(enter);
    engine
        .orders
        .sequence_manager
        .set_cross_postponed_link((admission, 0), Some((postponed, 0)));
    let Entity::Pc(opponent_entity) = engine.get_entity_mut(opponent).unwrap() else {
        unreachable!("test opponent must remain a PC")
    };
    opponent_entity.pc.life_points = 0;

    engine.dispatch_enter_swordfight(
        &sim,
        &LevelAssets::default(),
        owner,
        Some(opponent),
        admission,
        0,
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(admission, 0)
            .unwrap()
            .cross_postponed,
        Some((postponed, 0)),
        "failure cleanup is specific to the THRUST_A admission prerequisite"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(postponed, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Todo
    );
}

#[test]
fn successful_enter_swordfight_retains_postponed_thrust_a() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let owner = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let opponent = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            ..WorldPoint3D::default()
        },
        None,
    ));
    let postponed = engine.launch_element(crate::sequence::SequenceElement::new_interaction(
        1,
        Command::SwordstrikeThrustA,
        Some(owner),
        Some(opponent),
    ));
    let mut enter =
        crate::sequence::SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
    enter.set_property(
        crate::sequence::Field::Opponent,
        crate::sequence::FieldValue::Element(opponent),
    );
    let admission = engine.launch_element(enter);
    engine
        .orders
        .sequence_manager
        .set_cross_postponed_link((admission, 0), Some((postponed, 0)));

    engine.dispatch_enter_swordfight(
        &sim,
        &LevelAssets::default(),
        owner,
        Some(opponent),
        admission,
        0,
    );

    assert!(
        engine
            .get_entity(owner)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents
            .contains(&opponent),
        "control admission must succeed"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(admission, 0)
            .unwrap()
            .cross_postponed,
        Some((postponed, 0)),
        "successful admission retains the normal prerequisite chain"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(postponed, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Todo
    );
}

/// Build a cross-sector `EnterSwordfight` dispatch where `crowding`
/// fighters from the owner's sector already engage the opponent, and the
/// element carries no jump line.  Returns the engine, the owner and the
/// launched sequence id after dispatch.
fn dispatch_crowded_cross_sector_swordfight(
    crowding: usize,
) -> (EngineInner, EntityId, EntityId, crate::sequence::SequenceId) {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let owner_sector = crate::position_interface::SectorHandle::new(1);
    let opponent_sector = crate::position_interface::SectorHandle::new(2);
    assert_ne!(owner_sector, opponent_sector);

    let owner = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        owner_sector,
    ));
    let opponent = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 60.0,
            y: 100.0,
            z: 0.0,
        },
        opponent_sector,
    ));
    for index in 0..crowding {
        let fighter = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: index as f32 * 10.0,
                y: 120.0,
                z: 0.0,
            },
            owner_sector,
        ));
        engine
            .get_entity_mut(opponent)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(fighter);
    }
    assert_eq!(
        number_of_table_swordfight_opponents(
            &engine.world.entities,
            opponent,
            i16::from(owner_sector.unwrap()),
        ),
        crowding as u32,
    );

    let mut element =
        crate::sequence::SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
    element.set_property(
        crate::sequence::Field::Opponent,
        crate::sequence::FieldValue::Element(opponent),
    );
    // Deliberately no `Field::JumplineDestination`: the original's
    // `pJumpLine` is null here, which must not switch the occupancy gate
    // off.
    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(element);
    let seq_id = engine.launch_sequence(sequence);

    engine.dispatch_enter_swordfight(
        &sim,
        &LevelAssets::default(),
        owner,
        Some(opponent),
        seq_id,
        0,
    );
    (engine, owner, opponent, seq_id)
}

/// Original `RHElementActorHuman::Translate` case
/// `RHCOMMAND_ENTER_SWORDFIGHT` (`RHelementactorhuman.cpp:1324-1372`)
/// runs the cross-sector occupancy gate for every unprepared element that
/// has an opponent — `if( pJumpLine != 0 )` guards only the inner
/// slot-search half.  So a jump-line-less element still interrupts when
/// `GetNumberOfTableSwordfightOpponents` reports 3+ fighters on our side,
/// and the PC never reaches the `TransitionRaisingSword` order.
#[test]
fn crowded_cross_sector_swordfight_interrupts_without_a_jump_line() {
    let (engine, owner, opponent, seq_id) = dispatch_crowded_cross_sector_swordfight(3);

    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("interrupted element must survive the dispatch");
    // `pSequenceElement->SetState( RHSEQ_INTERRUPTED )` — the default
    // cascade is `CASCADE_NEXT_LEVEL` (`RHsequenceelement.h:152`).  This
    // must not be Impossible/Terminated: Interrupted abandons the
    // postponed successor instead of resuming it.
    assert_eq!(element.state, crate::sequence::SequenceState::Interrupted);
    assert!(
        element.current_order().is_none(),
        "the interrupt returns before InsertOrderAsLast"
    );

    let owner_entity = engine.get_entity(owner).unwrap();
    assert_eq!(
        owner_entity.actor_data().unwrap().action_state,
        ActionState::Waiting,
        "the crowded-out PC keeps waiting instead of raising its sword"
    );
    assert!(
        !owner_entity
            .human_data()
            .unwrap()
            .opponents
            .contains(&opponent),
        "the interrupt returns before EnterSwordFight, so no relationship forms"
    );
}

/// Counterpart of the gate above: with fewer than 3 fighters already on
/// our side and no jump line, the original falls straight through the
/// `if( pJumpLine != 0 )` block and enters the swordfight normally
/// (`RHelementactorhuman.cpp:1338-1370`).
#[test]
fn uncrowded_cross_sector_swordfight_enters_without_a_jump_line() {
    let (engine, owner, opponent, seq_id) = dispatch_crowded_cross_sector_swordfight(2);

    let element = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("in-progress element must survive the dispatch");
    assert_eq!(element.state, crate::sequence::SequenceState::InProgress);
    assert_eq!(
        element.current_order().unwrap().order_type,
        crate::order::OrderType::TransitionRaisingSword
    );
    assert!(
        engine
            .get_entity(owner)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents
            .contains(&opponent),
    );
}

#[test]
fn enter_swordfight_instruct_preserves_live_sprite_destination() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let owner = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let retained_goal = crate::coordinates::MapPoint::new(768.0, 1796.0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .position_iface_mut()
        .set_map_goal(retained_goal);

    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(crate::sequence::SequenceElement::new_generic(
        1,
        Command::EnterSwordfight,
        Some(owner),
    ));
    let seq_id = engine.launch_sequence(sequence);
    engine.dispatch_enter_swordfight(&sim, &LevelAssets::default(), owner, None, seq_id, 0);

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        retained_goal,
        "translation must not apply TransitionRaisingSword's zero destination before Execute"
    );
}

#[test]
fn satisfied_enter_swordfight_skips_outer_instruct_epilogue() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let owner = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let opponent = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    if let Some(actor) = engine.get_entity_mut(owner).unwrap().actor_data_mut() {
        actor.action_state = ActionState::WaitingSword;
    }
    if let Some(human) = engine.get_entity_mut(owner).unwrap().human_data_mut() {
        human.opponents = vec![opponent].into();
    }
    if let Some(human) = engine.get_entity_mut(opponent).unwrap().human_data_mut() {
        human.opponents = vec![owner].into();
    }

    let mut element =
        crate::sequence::SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
    element.set_property(
        crate::sequence::Field::Opponent,
        crate::sequence::FieldValue::Element(opponent),
    );
    let mut sequence = crate::sequence::Sequence::new();
    sequence.append_element(element);
    let seq_id = engine.launch_sequence(sequence);

    let barrier = engine.dispatch_enter_swordfight(
        &sim,
        &LevelAssets::default(),
        owner,
        Some(opponent),
        seq_id,
        0,
    );

    assert_eq!(
        barrier,
        crate::engine::sequence_runtime::OwnerActionBarrier::Skip,
        "terminal Translate changes the selected element before Actor::Instruct's epilogue"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Terminated
    );
}

#[test]
fn reconsider_rebalance_updates_opponents_without_recursive_enter_command() {
    use crate::ai::EnterSwordfightRequest;

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let owner = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let old_primary = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let replacement = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 20.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    if let Some(human) = engine.get_entity_mut(owner).unwrap().human_data_mut() {
        human.opponents = vec![old_primary, replacement].into();
    }
    if let Some(human) = engine.get_entity_mut(replacement).unwrap().human_data_mut() {
        human.opponents = vec![owner].into();
    }
    let replacement_handle = (0..3)
        .find(|slot| engine.world.entities.id_at_legacy_slot(*slot) == Some(replacement))
        .expect("replacement PC must occupy a legacy entity slot");
    let Entity::Soldier(soldier) = engine.get_entity_mut(owner).unwrap() else {
        unreachable!()
    };
    soldier
        .npc
        .ai_brain
        .enemy_mut()
        .unwrap()
        .base
        .outbox
        .actor
        .enter_swordfight = Some(EnterSwordfightRequest::Rebalance(replacement_handle));

    engine.drain_pending_for_npc(&sim, owner, &LevelAssets::default());

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents
            .first(),
        Some(&replacement),
        "direct EnterSwordFight must promote the replacement opponent"
    );
    let Entity::Soldier(soldier) = engine.get_entity(owner).unwrap() else {
        unreachable!()
    };
    assert_eq!(
        soldier.npc.ai_brain.enemy().unwrap().base.primary_target,
        replacement_handle,
        "successful rebalance must promote the AI primary target"
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(owner, Command::EnterSwordfight),
        "ReconsiderSwordfight's direct call must not author another Enter command"
    );
}

#[test]
fn reconsider_rebalance_rejection_preserves_opponent_and_ai_primary_target() {
    use crate::ai::EnterSwordfightRequest;

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let owner = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let old_primary = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let replacement = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 20.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    if let Some(human) = engine.get_entity_mut(owner).unwrap().human_data_mut() {
        human.opponents = vec![old_primary].into();
    }
    if let Some(human) = engine.get_entity_mut(replacement).unwrap().human_data_mut() {
        human.unconscious = true;
    }
    let old_primary_handle = (0..3)
        .find(|slot| engine.world.entities.id_at_legacy_slot(*slot) == Some(old_primary))
        .expect("old primary PC must occupy a legacy entity slot");
    let replacement_handle = (0..3)
        .find(|slot| engine.world.entities.id_at_legacy_slot(*slot) == Some(replacement))
        .expect("replacement PC must occupy a legacy entity slot");
    let Entity::Soldier(soldier) = engine.get_entity_mut(owner).unwrap() else {
        unreachable!()
    };
    let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
    ai.base.primary_target = old_primary_handle;
    ai.base.outbox.actor.enter_swordfight =
        Some(EnterSwordfightRequest::Rebalance(replacement_handle));

    engine.drain_pending_for_npc(&sim, owner, &LevelAssets::default());

    let Entity::Soldier(soldier) = engine.get_entity(owner).unwrap() else {
        unreachable!()
    };
    assert_eq!(soldier.human.opponents, vec![old_primary]);
    assert_eq!(
        soldier.npc.ai_brain.enemy().unwrap().base.primary_target,
        old_primary_handle,
        "failed EnterSwordFight must preserve the old AI primary target"
    );
}

#[test]
fn got_hit_direct_entry_authors_reciprocal_enter_on_attacker() {
    use crate::ai::EnterSwordfightRequest;

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let existing_opponent = engine.add_entity(make_pc(
        WorldPoint3D {
            x: -10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let attacker = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let Entity::Soldier(attacker_soldier) = engine.get_entity_mut(attacker).unwrap() else {
        unreachable!()
    };
    attacker_soldier.soldier.cached_camp = crate::element::Camp::Royalists;

    if let Some(human) = engine.get_entity_mut(victim).unwrap().human_data_mut() {
        human.opponents = vec![existing_opponent].into();
    }
    if let Some(human) = engine
        .get_entity_mut(existing_opponent)
        .unwrap()
        .human_data_mut()
    {
        human.opponents = vec![victim].into();
    }

    let mut strike_element =
        crate::sequence::SequenceElement::new(1, Command::SwordstrikeThrustA, Some(attacker));
    strike_element.priority = crate::sequence::SequencePriority::Preference;
    let mut strike = crate::sequence::Sequence::new();
    strike.append_element(strike_element);
    let strike_id = engine.launch_sequence(strike);
    let strike_order_id = engine.orders.allocate_order_id();
    let mut strike_order = crate::order::Order::new(
        crate::order::OrderType::StrikingStraightSword,
        0.0,
        0.0,
        strike_order_id,
    );
    strike_order.antagonist = Some(victim);
    engine
        .orders
        .sequence_manager
        .push_order_on(strike_id, 0, strike_order);
    engine
        .orders
        .sequence_manager
        .element_in_progress(strike_id, 0);

    let attacker_handle = (0..3)
        .find(|slot| engine.world.entities.id_at_legacy_slot(*slot) == Some(attacker))
        .expect("attacker must occupy a legacy entity slot");
    let Entity::Soldier(soldier) = engine.get_entity_mut(victim).unwrap() else {
        unreachable!()
    };
    soldier
        .npc
        .ai_brain
        .enemy_mut()
        .unwrap()
        .base
        .outbox
        .actor
        .enter_swordfight = Some(EnterSwordfightRequest::Direct(attacker_handle));

    engine.drain_pending_for_npc(&sim, victim, &LevelAssets::default());

    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![attacker, existing_opponent],
        "Original AddOpponent installs the new attacker as principal"
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![victim],
        "direct entry synchronously installs the reciprocal relationship"
    );
    let (enter_sequence, enter_index) = engine
        .orders
        .sequence_manager
        .pending_elements_for_owner(attacker)
        .into_iter()
        .find(|(sequence, index)| {
            engine
                .orders
                .sequence_manager
                .get_element(*sequence, *index)
                .is_some_and(|element| element.command == Command::EnterSwordfight)
        })
        .expect("the reciprocal ENTER_SWORDFIGHT must be attacker-owned");
    let enter = engine
        .orders
        .sequence_manager
        .get_element(enter_sequence, enter_index)
        .unwrap();
    assert_eq!(enter.owner, Some(attacker));
    assert!(matches!(
        enter.get_property(crate::sequence::Field::Opponent),
        Some(crate::sequence::FieldValue::Element(opponent)) if *opponent == victim
    ));
    assert!(
        !engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(victim, Command::EnterSwordfight),
        "EVENT_GOTHIT must not defer a self-owned Engage command"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(strike_id, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::InProgress,
        "the direct call bypasses PrepareToEnterSwordFight; interruption belongs to the reciprocal command scheduler"
    );

    let mut display = crate::engine::HostDisplayState::default();
    engine.hourglass_phase_sequences(&sim, &mut display, &LevelAssets::default());
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(enter_sequence, enter_index)
            .unwrap()
            .state,
        crate::sequence::SequenceState::InProgress,
        "the reciprocal high-priority ENTER becomes attacker-current"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(strike_id, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Postponed,
        "the reciprocal ENTER displaces the attacker's Preference strike"
    );
}

#[test]
fn direct_enter_swordfight_accepts_typed_slot_zero_opponent() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let opponent = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let initiator = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    assert_eq!(opponent.index(), 0, "control requires typed slot zero");

    assert!(engine.direct_enter_swordfight(&sim, &LevelAssets::default(), initiator, opponent,));
    assert_eq!(
        engine
            .get_entity(initiator)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![opponent]
    );
    assert_eq!(
        engine
            .get_entity(opponent)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![initiator]
    );
}

#[test]
fn reconsider_direct_entry_does_not_prepare_or_stop_opponent() {
    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let initiator = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let opponent = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    let mut selected = crate::sequence::Sequence::new();
    selected.append_element(crate::sequence::SequenceElement::new(
        1,
        Command::Point,
        Some(opponent),
    ));
    let selected_id = engine.launch_sequence(selected);
    engine
        .orders
        .sequence_manager
        .element_in_progress(selected_id, 0);
    engine
        .get_entity_mut(opponent)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .continuation
        .motion_state = crate::sprite::MotionState::InProgress;

    assert!(engine.direct_enter_swordfight(&sim, &LevelAssets::default(), initiator, opponent,));

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(selected_id, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::InProgress,
        "direct EnterSwordFight must not run PrepareToEnterSwordFight's Stop"
    );
    assert_eq!(
        engine
            .get_entity(opponent)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        crate::sprite::MotionState::InProgress
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(opponent, Command::EnterSwordfight),
        "direct entry still queues the reciprocal Enter command"
    );
}

#[test]
fn selected_pc_entering_swordfight_does_not_restore_armed_action_on_quit() {
    use crate::profiles::Action;

    let sim = crate::sim_rng::test_context();
    let assets = action_test_assets([Action::Bow, Action::Apple, Action::Purse]);
    let mut engine = make_engine();
    let pc = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let opponent = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    engine.players.seats[0].selection.push(pc);
    {
        let pc_data = engine.get_entity_mut(pc).unwrap().pc_data_mut().unwrap();
        pc_data.current_action = Action::Purse;
        pc_data.disabled_actions = vec![false; 3];
        pc_data.disabled_actions_temp = vec![false; 3];
    }

    assert!(engine.enter_swordfight(&sim, &assets, pc, opponent, false,));
    {
        let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
        assert_eq!(pc_data.current_action, Action::NoAction);
        assert_eq!(pc_data.saved_action, Action::NoAction);
        assert_eq!(pc_data.disabled_actions_temp, vec![true; 3]);
    }

    engine.quit_swordfight(&sim, &assets, pc);
    let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
    assert_eq!(pc_data.current_action, Action::NoAction);
    assert_eq!(pc_data.disabled_actions_temp, vec![false; 3]);
}

#[test]
fn unselected_pc_entering_swordfight_saves_targeted_no_action() {
    use crate::profiles::Action;

    let sim = crate::sim_rng::test_context();
    let assets = action_test_assets([Action::Bow, Action::Apple, Action::Purse]);
    let mut engine = make_engine();
    let pc = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let opponent = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    {
        let pc_data = engine.get_entity_mut(pc).unwrap().pc_data_mut().unwrap();
        pc_data.current_action = Action::Bow;
        pc_data.disabled_actions = vec![false; 3];
        pc_data.disabled_actions_temp = vec![false; 3];
    }

    assert!(engine.enter_swordfight(&sim, &assets, pc, opponent, false,));
    {
        let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
        assert_eq!(pc_data.current_action, Action::NoAction);
        assert_eq!(pc_data.saved_action, Action::NoAction);
        assert_eq!(pc_data.disabled_actions_temp, vec![true; 3]);
    }

    engine.quit_swordfight(&sim, &assets, pc);
    let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
    assert_eq!(pc_data.current_action, Action::NoAction);
    assert_eq!(pc_data.disabled_actions_temp, vec![false; 3]);
    assert!(
        !engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(pc, Command::EquipBow),
        "quitting must not restore the action that was armed before entry"
    );
}

#[test]
fn quit_swordfight_resets_moving_survivor_smalltalk_initiative() {
    use crate::profiles::Action;

    let sim = crate::sim_rng::test_context();
    let assets = action_test_assets([Action::NoAction; 3]);
    let mut engine = make_engine();
    let quitter = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let survivor = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let principal = engine.add_entity(make_pc(WorldPoint3D::default(), None));

    {
        let human = engine
            .get_entity_mut(quitter)
            .unwrap()
            .human_data_mut()
            .unwrap();
        human.opponents = vec![survivor].into();
    }
    {
        let survivor_entity = engine.get_entity_mut(survivor).unwrap();
        survivor_entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
        let human = survivor_entity.human_data_mut().unwrap();
        human.opponents = vec![quitter, principal].into();
        human.smalltalk_initiative = false;
        human.received_smalltalk_initiative = false;
    }
    {
        let human = engine
            .get_entity_mut(principal)
            .unwrap()
            .human_data_mut()
            .unwrap();
        human.opponents = vec![survivor].into();
        human.smalltalk_initiative = true;
    }

    engine.quit_swordfight(&sim, &assets, quitter);

    let survivor_human = engine.get_entity(survivor).unwrap().human_data().unwrap();
    assert_eq!(survivor_human.opponents, vec![principal]);
    assert!(survivor_human.smalltalk_initiative);
    assert!(survivor_human.received_smalltalk_initiative);
    assert!(
        !engine
            .get_entity(principal)
            .unwrap()
            .human_data()
            .unwrap()
            .smalltalk_initiative,
        "mutual principal must lose initiative even while the survivor is Moving"
    );
}

#[test]
fn quit_swordfight_does_not_reset_initiative_without_surviving_opponents() {
    use crate::profiles::Action;

    let sim = crate::sim_rng::test_context();
    let assets = action_test_assets([Action::NoAction; 3]);
    let mut engine = make_engine();
    let quitter = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let survivor = engine.add_entity(make_pc(WorldPoint3D::default(), None));

    {
        let human = engine
            .get_entity_mut(quitter)
            .unwrap()
            .human_data_mut()
            .unwrap();
        human.opponents = vec![survivor].into();
    }
    {
        let survivor_entity = engine.get_entity_mut(survivor).unwrap();
        survivor_entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
        let human = survivor_entity.human_data_mut().unwrap();
        human.opponents = vec![quitter].into();
        human.smalltalk_initiative = false;
        human.received_smalltalk_initiative = false;
    }

    engine.quit_swordfight(&sim, &assets, quitter);

    let survivor_human = engine.get_entity(survivor).unwrap().human_data().unwrap();
    assert!(survivor_human.opponents.is_empty());
    assert!(!survivor_human.smalltalk_initiative);
    assert!(!survivor_human.received_smalltalk_initiative);
}

#[test]
fn enabling_temp_actions_restores_matching_slot_after_targeted_selection_collapse() {
    use crate::profiles::Action;

    let assets = action_test_assets([Action::Bow, Action::Apple, Action::Purse]);
    let mut engine = make_engine();
    let pc = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    let companion = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    engine.players.seats[0].selection = vec![pc, companion];
    {
        let pc_data = engine.get_entity_mut(pc).unwrap().pc_data_mut().unwrap();
        pc_data.current_action = Action::NoAction;
        pc_data.saved_action = Action::Purse;
        pc_data.disabled_actions = vec![false; 3];
        pc_data.disabled_actions_temp = vec![true; 3];
    }
    engine
        .get_entity_mut(companion)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .current_action = Action::Bow;

    engine.enable_pc_actions_temp(&assets, 0, pc);

    let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
    assert_eq!(pc_data.current_action, Action::Purse);
    assert_eq!(pc_data.disabled_actions_temp, vec![false; 3]);
    assert_eq!(
        engine
            .get_entity(companion)
            .unwrap()
            .pc_data()
            .unwrap()
            .current_action,
        Action::Bow,
        "RHMessenger removes the companion before fanning out the targeted restored action"
    );
    assert_eq!(engine.players.seats[0].selection, vec![pc]);
    assert_eq!(engine.players.seats[0].selected_action, Action::Purse);
    assert!(
        engine
            .feedback
            .pending_side_effects
            .invalidate_trajectory_preview
    );
}

#[test]
fn enabling_temp_actions_does_not_restore_action_absent_from_profile_slots() {
    use crate::profiles::Action;

    let assets = action_test_assets([Action::Bow, Action::Apple, Action::Purse]);
    let mut engine = make_engine();
    let pc = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    engine.players.seats[0].selection.push(pc);
    {
        let pc_data = engine.get_entity_mut(pc).unwrap().pc_data_mut().unwrap();
        pc_data.current_action = Action::NoAction;
        pc_data.saved_action = Action::Stone;
        pc_data.disabled_actions = vec![false; 3];
        pc_data.disabled_actions_temp = vec![true; 3];
    }

    engine.enable_pc_actions_temp(&assets, 0, pc);

    let pc_data = engine.get_entity(pc).unwrap().pc_data().unwrap();
    assert_eq!(pc_data.current_action, Action::NoAction);
    assert_eq!(pc_data.disabled_actions_temp, vec![false; 3]);
    assert_eq!(engine.players.seats[0].selected_action, Action::NoAction);
    assert!(
        !engine
            .feedback
            .pending_side_effects
            .invalidate_trajectory_preview
    );
}

#[test]
fn preparing_swordfight_orders_done_enter_then_queues_reciprocal() {
    use crate::ai::{AiState, LogLineType, StimulusType, Substate};
    use crate::profiles::{CharacterProfile, HtHWeaponProfile, ProfileManager, SoldierProfile};

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let initiator = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let opponent = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(initiator).unwrap() else {
            unreachable!()
        };
        soldier.soldier.cached_camp = crate::element::Camp::Royalists;
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = initiator.index();
        ai.hth_weapon_id = 1;
    }
    {
        let Entity::Soldier(soldier) = engine.get_entity_mut(opponent).unwrap() else {
            unreachable!()
        };
        let ai = soldier.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = opponent.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingOfficerGivingOrders;
        ai.hth_weapon_id = 1;
    }

    // Give the opponent a selected command for PrepareToEnterSwordFight's
    // Stop(PREFERENCE) to interrupt. Its condolence sends EventDone.
    let mut selected = crate::sequence::Sequence::new();
    selected.append_element(crate::sequence::SequenceElement::new(
        1,
        Command::Point,
        Some(opponent),
    ));
    let selected_id = engine.launch_sequence(selected);
    engine
        .orders
        .sequence_manager
        .element_in_progress(selected_id, 0);

    let mut profiles = ProfileManager::new();
    profiles.hth_weapons.push(HtHWeaponProfile {
        distance: [30, 50, 60, 70],
        ..HtHWeaponProfile::default()
    });
    profiles.characters.push(CharacterProfile {
        hth_weapon_id: 1,
        ..CharacterProfile::default()
    });
    profiles.soldiers.push(SoldierProfile {
        hth_weapon_id: 1,
        hostile: true,
        ..SoldierProfile::default()
    });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };

    assert!(engine.enter_swordfight(sim, &assets, initiator, opponent, false));

    assert!(
        engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched_or_postponed_by_current(
                opponent,
                Command::EnterSwordfight,
            ),
        "non-Wait reciprocal enter remains on the manager FIFO after EnterSwordFight returns"
    );
    let ai = engine
        .get_entity(opponent)
        .unwrap()
        .ai_controller()
        .unwrap();
    let events: Vec<_> = ai
        .ai_log
        .iter()
        .filter(|entry| entry.line_type == LogLineType::Event)
        .map(|entry| entry.info)
        .collect();
    assert_eq!(
        events,
        vec![
            StimulusType::EventDone as u16,
            StimulusType::EventEnterSwordfight as u16
        ],
        "the interrupted command must complete in the old substate before swordfight entry"
    );
    assert_eq!(ai.current_substate, Substate::AttackingSwordfight);
}

#[test]
fn deleting_final_opponent_synchronously_quits_soldier_ai() {
    use crate::ai::{AiState, LogLineType, StimulusType, Substate};
    use crate::profiles::{CharacterProfile, HtHWeaponProfile, ProfileManager, SoldierProfile};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let soldier = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let opponent = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    if let Some(human) = engine.get_entity_mut(soldier).unwrap().human_data_mut() {
        human.opponents = vec![opponent].into();
    }
    let Entity::Soldier(soldier_entity) = engine.get_entity_mut(soldier).unwrap() else {
        unreachable!()
    };
    let ai = soldier_entity.npc.ai_brain.enemy_mut().unwrap();
    ai.base.me = soldier.index();
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
    ai.hth_weapon_id = 1;
    let Entity::Soldier(opponent_entity) = engine.get_entity_mut(opponent).unwrap() else {
        unreachable!()
    };
    let opponent_ai = opponent_entity.npc.ai_brain.enemy_mut().unwrap();
    opponent_ai.base.me = opponent.index();
    opponent_ai.hth_weapon_id = 1;

    let mut profiles = ProfileManager::new();
    profiles.hth_weapons.push(HtHWeaponProfile::default());
    profiles.characters.push(CharacterProfile {
        hth_weapon_id: 1,
        ..CharacterProfile::default()
    });
    profiles.soldiers.push(SoldierProfile {
        hth_weapon_id: 1,
        hostile: true,
        ..SoldierProfile::default()
    });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };

    assert!(engine.delete_opponent(&sim, &assets, soldier, opponent));

    let ai = engine.get_entity(soldier).unwrap().ai_controller().unwrap();
    assert_eq!(
        ai.current_substate,
        Substate::AttackingQuittingSwordfight,
        "DeleteOpponent must synchronously deliver the final-opponent quit event"
    );
    assert!(ai.ai_log.iter().any(|entry| {
        entry.line_type == LogLineType::Event
            && entry.info == StimulusType::EventQuitSwordfight as u16
    }));
}

#[test]
fn deleting_final_opponent_synchronously_quits_enemy_ai_hero_ai() {
    use crate::ai::{AiState, LogLineType, StimulusType, Substate};
    use crate::profiles::{CharacterProfile, HtHWeaponProfile, ProfileManager, SoldierProfile};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let pc = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let opponent = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    let Entity::Pc(pc_entity) = engine.get_entity_mut(pc).unwrap() else {
        unreachable!()
    };
    pc_entity.human.opponents = vec![opponent].into();
    let mut enemy_ai = crate::ai_enemy::EnemyAi::new(pc.index());
    enemy_ai.base.current_state = AiState::Attacking;
    enemy_ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
    enemy_ai.hth_weapon_id = 1;
    pc_entity.pc.life_points = 100;
    pc_entity.pc.command_interface = crate::human_control::CommandInterface::None;
    pc_entity.pc.mission_role = crate::human_control::MissionRole::Combatant;
    pc_entity.pc.combat_stance = crate::human_control::CombatStance::Aggressive;
    pc_entity.pc.ai = Some(Box::new(crate::element::AiActorData {
        ai_brain: crate::element::AiBrain::Enemy(Box::new(enemy_ai)),
        ..Default::default()
    }));
    engine
        .get_entity_mut(opponent)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;

    let mut profiles = ProfileManager::new();
    profiles.hth_weapons.push(HtHWeaponProfile::default());
    profiles.characters.push(CharacterProfile {
        hth_weapon_id: 1,
        ..CharacterProfile::default()
    });
    profiles.soldiers.push(SoldierProfile {
        hth_weapon_id: 1,
        hostile: true,
        ..SoldierProfile::default()
    });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };

    assert!(engine.delete_opponent(&sim, &assets, pc, opponent));

    let ai = engine.get_entity(pc).unwrap().ai_controller().unwrap();
    assert_eq!(ai.current_substate, Substate::AttackingQuittingSwordfight);
    assert!(ai.ai_log.iter().any(|entry| {
        entry.line_type == LogLineType::Event
            && entry.info == StimulusType::EventQuitSwordfight as u16
    }));
}

/// Bud-Spencer-style line of three: PC punches the first soldier,
/// who is launched along +X into a second soldier directly in
/// front, and a third soldier behind the second. The flight tick
/// should fire a domino RECEIVE_HIT_DAMAGE on both downstream
/// soldiers, citing the PC as origin.
#[test]
fn domino_propagates_to_actors_in_flight_path() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let hitter = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let flyer = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let mid = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 16.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let far = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 22.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    // 5 frames of +X motion at 1 unit per frame — short enough to
    // stay inside DOMINO_DISTANCE for the front pair.
    give_flight(&mut engine, flyer, hitter, 1.0, 0.0, 5);

    engine.tick_push_flights(sim, &LevelAssets::default());

    assert_eq!(
        count_domino_hits_for(&engine, mid, hitter),
        1,
        "soldier directly in front should take a domino hit"
    );
    assert_eq!(
        count_domino_hits_for(&engine, far, hitter),
        1,
        "soldier further along the flight axis should also take a domino hit"
    );
    assert_eq!(
        count_domino_hits_for(&engine, hitter, hitter),
        0,
        "hitter must never domino itself"
    );
    assert_eq!(
        count_domino_hits_for(&engine, flyer, hitter),
        0,
        "flyer is not its own domino victim"
    );
}

/// ApplyDominoEffect measures the literal world X/Y ground plane. An
/// elevated victim can therefore be inside the 15-unit world radius even
/// when projecting elevation into map Y would put it outside the radius.
#[test]
fn elevated_domino_uses_world_ground_xy_not_projected_map_y() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let hitter = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 110.0,
            z: 0.0,
        },
        None,
    ));
    let flyer = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 10.0,
        },
        None,
    ));
    let victim = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 0.0,
            y: 90.0,
            z: 18.0,
        },
        None,
    ));

    // The generic actor fixture finishes by authoring a map point, which
    // intentionally flattens actors without a level plane. Restore the
    // literal 3D positions needed by this elevated-flight boundary.
    engine
        .get_entity_mut(flyer)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 10.0,
        });
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D {
            x: 0.0,
            y: 90.0,
            z: 18.0,
        });

    give_flight(&mut engine, flyer, hitter, 0.0, -1.0, 5);
    let flight = engine
        .get_entity_mut(flyer)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_flight
        .as_mut()
        .expect("test flight remains active");
    flight.geometry = crate::element::FlightGeometry::World3d;
    flight.increment_z = 1.0;

    engine.tick_push_flights(sim, &LevelAssets::default());

    let flyer_element = engine.get_entity(flyer).unwrap().element_data();
    let victim_element = engine.get_entity(victim).unwrap().element_data();
    let world_y_delta = victim_element.position().y - flyer_element.position().y;
    let map_y_delta = victim_element.position_map().y - flyer_element.position_map().y;
    assert_eq!(world_y_delta, -9.0);
    assert_eq!(map_y_delta, -16.0);

    assert_eq!(
        count_domino_hits_for(&engine, victim, hitter),
        1,
        "world ground delta is 9 units after the flight step; projected map Y would incorrectly measure 16"
    );
}

/// Actors behind the flight vector (negative dot product) are
/// outside the punch arc and must not take damage.
#[test]
fn domino_skips_actors_behind_flight_direction() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let hitter = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let flyer = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    // Sits "behind" the flyer relative to its +X motion.
    let behind = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 5.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    give_flight(&mut engine, flyer, hitter, 1.0, 0.0, 5);
    engine.tick_push_flights(sim, &LevelAssets::default());

    assert_eq!(
        count_domino_hits_for(&engine, behind, hitter),
        0,
        "actor behind the flyer should not be domino-hit (negative dot product)"
    );
}

/// The Chebyshev pre-filter (`MaxNorm < DOMINO_DISTANCE`) and the
/// Euclidean check both have to fire. Place a candidate just past
/// the radius and assert it is skipped.
#[test]
fn domino_respects_distance_radius() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let hitter = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let flyer = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    // 16 map units away on the X axis — outside DOMINO_DISTANCE = 15.
    let far = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 26.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    give_flight(&mut engine, flyer, hitter, 1.0, 0.0, 5);
    engine.tick_push_flights(sim, &LevelAssets::default());

    assert_eq!(
        count_domino_hits_for(&engine, far, hitter),
        0,
        "actor outside DOMINO_DISTANCE must not be domino-hit"
    );
}

/// Non-upright actors (lying, dead, etc.) are excluded — they're
/// already on the ground and the upright-only filter rejects them.
#[test]
fn domino_skips_non_upright_actors() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let hitter = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let flyer = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let mut lying_entity = make_soldier(
        WorldPoint3D {
            x: 16.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    );
    lying_entity.set_posture(Posture::Lying);
    let lying = engine.add_entity(lying_entity);

    give_flight(&mut engine, flyer, hitter, 1.0, 0.0, 5);
    engine.tick_push_flights(sim, &LevelAssets::default());

    assert_eq!(
        count_domino_hits_for(&engine, lying, hitter),
        0,
        "lying actor must not be domino-hit (filtered by Posture::Upright)"
    );
}

/// Rolling and ladder/wall flights set `antagonist = None`, so the
/// per-frame sweep skips them entirely. Verify by giving the flyer
/// a None-antagonist flight even though there's a candidate
/// directly in the flight path.
#[test]
fn no_domino_when_flight_has_no_antagonist() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let _hitter = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let flyer = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let candidate = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: 16.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    // No antagonist — mirrors the rolling / ladder-wall fall path.
    let flyer_pos = engine
        .get_entity(flyer)
        .unwrap()
        .element_data()
        .position_map();
    if let Some(entity) = engine.world.entities.get_mut(flyer)
        && let Some(actor) = entity.actor_data_mut()
    {
        actor.active_flight = Some(ActiveFlight {
            increment_x: 1.0,
            increment_y: 0.0,
            goal_x: flyer_pos.x + 5.0,
            goal_y: flyer_pos.y,
            frames_remaining: 5,
            antagonist: None,
            ..Default::default()
        });
    }

    engine.tick_push_flights(sim, &LevelAssets::default());

    let any_hit = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|s| s.elements.iter())
        .any(|e| e.command == Command::ReceiveHitDamage && e.owner == Some(candidate));
    assert!(
        !any_hit,
        "antagonist=None flight (rolling / ladder-fall) must not domino"
    );
}

/// Regression: cheat-driven `apply_concussion` on a PC must seed
/// `concussion_healing_timeout` with the PC profile's `wake_up`,
/// not the soldier fallback constant.  Before the asset-context
/// plumbing landed, the cheat path hard-coded
/// `SOLDIER_CONCUSSION_HEALING_SPEED` because `&LevelAssets`
/// wasn't reachable from `dispatch_console_command`.
#[test]
fn apply_concussion_uses_pc_profile_wake_up() {
    use crate::engine::LevelAssets;
    use crate::profiles::{CharacterProfile, CharacterProfileIdx, ProfileManager};

    const PC_WAKE_UP: u16 = 555;

    let mut engine = make_engine();
    // Forest of Barnsdale/Charnwood/Ashby missions use the forest proto
    // flag too, but only the Sherwood HQ mission grants PC immunity.
    engine.world.weather.is_forest_level = true;

    // PC with profile_index 0 — `make_pc` defaults to that.
    let pc_id = engine.add_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        None,
    ));
    // Sanity: the helper does default to index 0.
    assert_eq!(
        engine
            .get_entity(pc_id)
            .unwrap()
            .pc_data()
            .unwrap()
            .profile_index,
        CharacterProfileIdx(0)
    );

    // Build a `LevelAssets` whose `ProfileManager` has a single PC
    // profile at index 0 with a distinctive `wake_up`.
    let mut profile_manager = ProfileManager::new();
    profile_manager.characters.push(CharacterProfile {
        wake_up: PC_WAKE_UP,
        ..CharacterProfile::default()
    });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profile_manager),
        ..LevelAssets::default()
    };

    // Drive the cheat-equivalent call: 100 concussion → KO →
    // healing-timeout init.
    let outcome =
        engine.apply_concussion(&crate::sim_rng::test_context(), &assets, pc_id, 100, false);
    assert_eq!(outcome, combat::ConcussionOutcome::WentUnconscious);

    let timeout = engine
        .get_entity(pc_id)
        .unwrap()
        .human_data()
        .unwrap()
        .concussion_healing_timeout;
    assert_eq!(
        timeout, PC_WAKE_UP,
        "cheat-driven KO on a PC must seed `concussion_healing_timeout` with \
             the PC profile's `wake_up`, not the soldier fallback constant \
             ({SOLDIER_CONCUSSION_HEALING_SPEED})"
    );
}

#[test]
fn concussion_context_uses_campaign_description_identity_not_ui_list_index() {
    let mut engine = make_engine();
    engine.mission_domain.campaign.characters = vec![
        crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
            status: crate::pc_status::PcStatus {
                in_coma: true,
                ..Default::default()
            },
            ..Default::default()
        },
        crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
            status: crate::pc_status::PcStatus {
                in_coma: false,
                ..Default::default()
            },
            ..Default::default()
        },
    ];

    let mut pc = make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        None,
    );
    let pc_data = pc.pc_data_mut().unwrap();
    pc_data.list_index = 0;
    pc_data.campaign_description_index = Some(1);

    let ctx = concussion_ctx_full(
        &pc,
        false,
        Some(&engine.mission_domain.campaign),
        engine.control.sim_config.difficulty,
    );
    assert!(
        !ctx.is_in_coma,
        "the UI list index must not borrow another same-profile PC's coma status"
    );

    engine.mission_domain.campaign.characters[1].status.in_coma = true;
    let ctx = concussion_ctx_full(
        &pc,
        false,
        Some(&engine.mission_domain.campaign),
        engine.control.sim_config.difficulty,
    );
    assert!(ctx.is_in_coma);
}

#[test]
fn swordfight_distance_keeps_original_strict_minimum_boundary() {
    use super::evaluate::{
        SwordfightDistanceAdjustment as Adjustment, swordfight_distance_adjustment,
    };

    // Savegame_008/replay-012 reaches this representable distance after
    // one ordinary 12-unit swordfight correction. Original compares it
    // directly with the 45-unit MINIMAL range and requests another move.
    assert_eq!(
        swordfight_distance_adjustment(44.999_71, 45.0, 65.0, 65.0, false),
        Adjustment::Farther,
    );
    assert_eq!(
        swordfight_distance_adjustment(45.0, 45.0, 65.0, 65.0, false),
        Adjustment::None,
    );
}

#[test]
fn evaluated_step_back_aborted_before_motion_terminal_preserves_history() {
    let mut engine = make_engine();
    let owner = engine.add_entity(make_pc(WorldPoint3D::default(), None));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .last_motion_was_step_back_in_combat = false;

    engine.launch_evaluated_step_back(owner, crate::coordinates::MapPoint::new(12.0, 0.0), 0);
    let (sequence_id, element_index) = engine
        .orders
        .sequence_manager
        .live_element_for_actor_matching(owner, |element| {
            element.movement_flags_for_test().is_some_and(|flags| {
                flags.contains(crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT)
            })
        })
        .expect("evaluated step-back movement must be registered");

    engine
        .orders
        .sequence_manager
        .element_impossible(sequence_id, element_index);
    assert!(
        !engine
            .get_entity(owner)
            .unwrap()
            .human_data()
            .unwrap()
            .last_motion_was_step_back_in_combat,
        "requesting and then aborting a step-back before RHMOTION_TERMINATED must not publish completed-step history"
    );
}

#[test]
fn swordfight_distance_keeps_original_strict_maximum_and_step_back_guards() {
    use super::evaluate::{
        SwordfightDistanceAdjustment as Adjustment, swordfight_distance_adjustment,
    };

    assert_eq!(
        swordfight_distance_adjustment(65.000_01, 45.0, 65.0, 60.0, false),
        Adjustment::Closer,
    );
    assert_eq!(
        swordfight_distance_adjustment(65.0, 45.0, 65.0, 60.0, false),
        Adjustment::None,
    );
    assert_eq!(
        swordfight_distance_adjustment(65.000_01, 45.0, 65.0, 60.0, true),
        Adjustment::None,
    );
}
