
use super::*;
use crate::coordinates::WorldPoint3D;
use crate::element::{
    ActorData, ActorPc, ActorSoldier, ElementData, ElementKind, ElementNet, HumanData, NetData,
    NpcData, ObjectData, PcData, Posture, ProjectileData, SoldierData,
};
use crate::profiles::{Action, CharacterProfile, ProfileManager, SoldierProfile};

/// Square root of [`SQUARE_RADIUS_NET_CAPTURE`] is 40, so any human
/// within 40 isometric units of the landing point qualifies (with Y
/// stretched).  Tests place victims well inside the radius (Δ = 5)
/// to avoid borderline arithmetic.
const LAND_X: f32 = 100.0;
const LAND_Y: f32 = 100.0;
const LAND_Z: f32 = 0.0;

fn make_engine() -> EngineInner {
    EngineInner::new()
}

fn make_net(landing: WorldPoint3D) -> Entity {
    let mut element = {
        let mut initial_element = ElementData::default();
        initial_element.kind = ElementKind::ObjectNet;
        initial_element.active = true;
        initial_element
    };
    element.set_position(landing);
    element.set_position_map(MapPoint::from_world_xyz(landing.x, landing.y, landing.z));
    Entity::Net(ElementNet {
        element,
        object: ObjectData {
            object_type: crate::element::ObjectType::Net,
            ..ObjectData::default()
        },
        projectile: ProjectileData {
            end: landing,
            flying: false,
            ..ProjectileData::default()
        },
        net: NetData::default(),
    })
}

fn run_net_owner_path(
    engine: &mut EngineInner,
    assets: &LevelAssets,
) -> Vec<(EntityId, crate::sprite::FrameProgression)> {
    let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    let (_, trace) = capture_net_sprite_progressions(|| {
        crate::sim_rng::with_seed(0x4E45_5431, |sim| {
            engine.tick_actor_owner_envelopes(sim, assets, &positions);
        });
    });
    trace
}

#[test]
fn owner_path_unfold_countdown_never_advances_sprite_including_zero_tick() {
    let mut engine = make_engine();
    let mut entity = make_net(WorldPoint3D::new(0.0, 0.0, 10.0));
    let Entity::Net(net) = &mut entity else {
        unreachable!()
    };
    net.projectile.flying = true;
    net.projectile.trajectory_frame_count = 1;
    net.net.time_till_unfolding = 1;
    net.object.animation = crate::element::Animation::ObjectFlying;
    let net_id = engine.add_entity(entity);

    let trace = run_net_owner_path(&mut engine, &LevelAssets::new());
    let Entity::Net(net) = engine.get_entity(net_id).unwrap() else {
        unreachable!()
    };
    assert_eq!(net.net.time_till_unfolding, 0);
    assert_eq!(
        net.object.animation,
        crate::element::Animation::NetUnfolding
    );
    assert!(
        trace.is_empty(),
        "zero-count transition must not advance the new row"
    );
}

#[test]
fn owner_path_ground_unfold_transitions_set_rows_without_advancing_them() {
    for (crumpled, source, expected) in [
        (
            false,
            crate::element::Animation::NetUnfolding,
            crate::element::Animation::ObjectLying,
        ),
        (
            true,
            crate::element::Animation::NetUnfoldingCrumpled,
            crate::element::Animation::NetLyingCrumpled,
        ),
    ] {
        let mut engine = make_engine();
        let mut entity = make_net(WorldPoint3D::new(0.0, 0.0, 0.0));
        let Entity::Net(net) = &mut entity else {
            unreachable!()
        };
        net.net.crumpled = crumpled;
        net.object.animation = source;
        let net_id = engine.add_entity(entity);
        let trace = run_net_owner_path(&mut engine, &LevelAssets::new());
        let Entity::Net(net) = engine.get_entity(net_id).unwrap() else {
            unreachable!()
        };
        assert_eq!(net.object.animation, expected);
        assert!(
            trace.is_empty(),
            "ground transition must only select its new row"
        );
    }
}

#[test]
fn owner_path_stationary_net_moving_stays_moving_with_frozen_progression() {
    let mut engine = make_engine();
    let mut entity = make_net(WorldPoint3D::new(0.0, 0.0, 0.0));
    let Entity::Net(net) = &mut entity else {
        unreachable!()
    };
    net.object.animation = crate::element::Animation::NetMoving;
    net.net.landed_animation_resolved = true;
    let net_id = engine.add_entity(entity);
    let trace = run_net_owner_path(&mut engine, &LevelAssets::new());
    let Entity::Net(net) = engine.get_entity(net_id).unwrap() else {
        unreachable!()
    };
    assert_eq!(net.object.animation, crate::element::Animation::NetMoving);
    assert_eq!(
        trace,
        vec![(net_id, crate::sprite::FrameProgression::Frozen)]
    );
}

#[test]
fn owner_path_crumpled_net_being_taken_does_not_advance_sprite() {
    // Original-game net updates have a branch for
    // normal net-taking animation, but deliberately none for the
    // distinct crumpled-net-taking row.
    for (animation, expected) in [
        (
            crate::element::Animation::NetBeingTaken,
            vec![crate::sprite::FrameProgression::FreezeWhenTerminated],
        ),
        (crate::element::Animation::NetBeingTakenCrumpled, Vec::new()),
    ] {
        let mut engine = make_engine();
        let mut entity = make_net(WorldPoint3D::new(0.0, 0.0, 0.0));
        let Entity::Net(net) = &mut entity else {
            unreachable!()
        };
        net.element.active = false;
        net.object.animation = animation;
        let net_id = engine.add_entity(entity);

        let trace = run_net_owner_path(&mut engine, &LevelAssets::new());
        assert_eq!(
            trace,
            expected
                .into_iter()
                .map(|progression| (net_id, progression))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn owner_path_frozen_all_keeps_net_physics_but_suppresses_sprite_call() {
    let mut engine = make_engine();
    let mut entity = make_net(WorldPoint3D::new(1.0, 2.0, 10.0));
    let Entity::Net(net) = &mut entity else {
        unreachable!()
    };
    net.projectile.flying = true;
    net.projectile.trajectory_frame_count = 1;
    net.projectile.velocity_increment = WorldVec3D::new(3.0, 4.0, 1.0);
    net.object.animation = crate::element::Animation::ObjectFlying;
    let net_id = engine.add_entity(entity);
    engine.set_actors_frozen(true);

    let trace = run_net_owner_path(&mut engine, &LevelAssets::new());
    let Entity::Net(net) = engine.get_entity(net_id).unwrap() else {
        unreachable!()
    };
    assert_eq!(net.projectile.frame_count, 1);
    assert_eq!(
        net.element.position_map(),
        MapPoint::from_world_xyz(4.0, 6.0, 11.0)
    );
    assert!(
        trace.is_empty(),
        "FrozenAll must suppress the selected sprite call"
    );
}

fn make_soldier(pos: WorldPoint3D, profile_idx: u32, rider: bool) -> Entity {
    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
        initial_element.kind = ElementKind::ActorSoldier;
        initial_element.active = true;
        initial_element
    };
    element.set_position(pos);
    element.set_position_map(MapPoint::from_world_xyz(pos.x, pos.y, pos.z));
    Entity::Soldier(ActorSoldier {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData {
            life_points: 50,
            // Level loading copies the soldier profile's HtH weapon onto
            // the brain; the fighter-registry scan reached from net
            // capture requires it.
            ai: crate::element::AiActorData {
                ai_brain: crate::element::AiBrain::Enemy(Box::new(crate::ai_enemy::EnemyAi {
                    hth_weapon_id: 1,
                    ..crate::ai_enemy::EnemyAi::default()
                })),
                ..Default::default()
            },
        },
        soldier: SoldierData {
            soldier_profile_index: crate::profiles::SoldierProfileIdx(profile_idx),
            cached_camp: crate::element::Camp::Lacklandists,
            rider,
            ..SoldierData::default()
        },
    })
}

/// Add a soldier and complete the runtime identity production spawn
/// paths install: the enemy AI's self handle must reference the
/// soldier's real entity slot and its melee weapon must resolve to the
/// registered test HtH profile, or capture-time synchronous AI thinks
/// reject the fighter registry.
fn add_soldier(
    engine: &mut EngineInner,
    pos: WorldPoint3D,
    profile_idx: u32,
    rider: bool,
) -> EntityId {
    let id = engine.add_entity(make_soldier(pos, profile_idx, rider));
    let enemy = engine
        .world
        .entities
        .get_mut(id)
        .and_then(Entity::enemy_ai_mut)
        .expect("test soldier has an enemy AI brain");
    enemy.base.me = id.index();
    enemy.hth_weapon_id = 1;
    id
}

fn make_pc(pos: WorldPoint3D, profile_idx: u32) -> Entity {
    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
        initial_element.kind = ElementKind::ActorPc;
        initial_element.active = true;
        initial_element
    };
    element.set_position(pos);
    element.set_position_map(MapPoint::from_world_xyz(pos.x, pos.y, pos.z));
    Entity::Pc(ActorPc {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData {
            profile_index: crate::profiles::CharacterProfileIdx(profile_idx),
            life_points: 50,
            ..PcData::default()
        },
    })
}

/// Build a [`LevelAssets`] with three character/soldier profiles set
/// up so:
///   * soldier 0 — plain Royalist (vip = false)
///   * soldier 1 — VIP Royalist
///   * character 0 — plain PC (no Net action)
///   * character 1 — Stuteley (Net action present)
fn assets_with_profiles() -> LevelAssets {
    let mut pm = ProfileManager::new();
    pm.soldiers.push(SoldierProfile::default());
    pm.soldiers.push(SoldierProfile {
        vip: true,
        ..SoldierProfile::default()
    });
    pm.characters.push(CharacterProfile {
        hth_weapon_id: 1,
        ..CharacterProfile::default()
    });
    pm.characters.push(CharacterProfile {
        actions: [Action::Net, Action::NoAction, Action::NoAction],
        hth_weapon_id: 1,
        ..CharacterProfile::default()
    });
    pm.hth_weapons
        .push(crate::profiles::HtHWeaponProfile::default());
    let mut assets = LevelAssets::new();
    assets.profile_manager = std::sync::Arc::new(pm);
    assets
}

fn count_receive_net_for(engine: &EngineInner, victim_id: EntityId) -> usize {
    engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|s| s.elements.iter())
        .filter(|e| e.owner == Some(victim_id) && e.command == Command::ReceiveNet)
        .count()
}

#[test]
fn net_captures_three_normal_soldiers() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let assets = assets_with_profiles();
    let landing = WorldPoint3D {
        x: LAND_X,
        y: LAND_Y,
        z: LAND_Z,
    };
    let net_id = engine.add_entity(make_net(landing));
    let soldiers: Vec<EntityId> = (0..3)
        .map(|i| {
            add_soldier(
                &mut engine,
                WorldPoint3D {
                    x: LAND_X + i as f32 * 5.0,
                    y: LAND_Y,
                    z: 0.0,
                },
                0, // plain non-VIP profile
                false,
            )
        })
        .collect();

    engine.apply_net_falling_effect(sim, &assets, net_id);

    let net = match engine.get_entity(net_id).unwrap() {
        Entity::Net(n) => n,
        _ => panic!("not a net"),
    };
    assert!(
        !net.net.crumpled,
        "net should not crumple on plain soldiers"
    );
    assert_eq!(
        net.net.victims.len(),
        3,
        "all three soldiers in range should be captured"
    );
    for s in &soldiers {
        assert!(net.net.victims.contains(s));
        assert_eq!(count_receive_net_for(&engine, *s), 1);
        // Counter is bumped synchronously even though the posture
        // snap waits for the ReceiveNet damage element next frame.
        assert_eq!(
            engine
                .get_entity(*s)
                .unwrap()
                .human_data()
                .unwrap()
                .stuck_under_nets_counter,
            1,
            "counter should be incremented eagerly on capture"
        );
        assert_eq!(
            engine.get_entity(*s).unwrap().element_data().posture(),
            Posture::Upright,
            "posture stays Upright until the ReceiveNet handler runs"
        );
    }
}

#[test]
fn ordered_net_dispatch_applies_landing_capture_inline() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let assets = assets_with_profiles();
    let landing = WorldPoint3D {
        x: LAND_X,
        y: LAND_Y,
        z: LAND_Z,
    };
    let mut net = make_net(landing);
    let Entity::Net(net_data) = &mut net else {
        unreachable!();
    };
    net_data.projectile.flying = true;
    net_data.net.was_flying = true;
    let net_id = engine.add_entity(net);
    let victim_id = add_soldier(&mut engine, landing, 0, false);

    engine.tick_net(sim, &assets, net_id);

    assert_eq!(
        engine
            .get_entity(victim_id)
            .expect("landing victim present")
            .human_data()
            .expect("landing victim human")
            .stuck_under_nets_counter,
        1,
        "net landing must capture before dispatch advances to the victim's later slot"
    );
    assert_eq!(count_receive_net_for(&engine, victim_id), 1);
}

#[test]
fn second_apply_pass_does_not_double_capture() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    // The capture sweep runs every frame the net is descending
    // within the apply threshold of landing — the dedup guard
    // ensures each victim is only captured once.
    let mut engine = make_engine();
    let assets = assets_with_profiles();
    let landing = WorldPoint3D {
        x: LAND_X,
        y: LAND_Y,
        z: LAND_Z,
    };
    let net_id = engine.add_entity(make_net(landing));
    let victim_id = add_soldier(
        &mut engine,
        WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: 0.0,
        },
        0,
        false,
    );
    engine.apply_net_falling_effect(sim, &assets, net_id);
    engine.apply_net_falling_effect(sim, &assets, net_id);
    engine.apply_net_falling_effect(sim, &assets, net_id);

    let net = match engine.get_entity(net_id).unwrap() {
        Entity::Net(n) => n,
        _ => panic!("not a net"),
    };
    assert_eq!(net.net.victims, vec![victim_id]);
    assert_eq!(count_receive_net_for(&engine, victim_id), 1);
    assert_eq!(
        engine
            .get_entity(victim_id)
            .unwrap()
            .human_data()
            .unwrap()
            .stuck_under_nets_counter,
        1,
        "dedup guard prevents counter double-bump on repeat ApplyEffect"
    );
}

#[test]
fn net_crumples_when_only_rider_in_range() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let assets = assets_with_profiles();
    let landing = WorldPoint3D {
        x: LAND_X,
        y: LAND_Y,
        z: LAND_Z,
    };
    let net_id = engine.add_entity(make_net(landing));
    let rider_id = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: LAND_X + 5.0,
            y: LAND_Y,
            z: 0.0,
        },
        0,
        true, // rider
    ));

    engine.apply_net_falling_effect(sim, &assets, net_id);

    let net = match engine.get_entity(net_id).unwrap() {
        Entity::Net(n) => n,
        _ => panic!("not a net"),
    };
    assert!(
        net.net.crumpled,
        "net should crumple when only a rider is in range"
    );
    assert!(net.net.victims.is_empty(), "no victims when crumpled");
    assert_eq!(count_receive_net_for(&engine, rider_id), 0);
}

#[test]
fn selective_immunity_skips_all_resistant_types_and_captures_an_ally() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    engine
        .control
        .sim_config
        .item_gameplay
        .net_selective_immunity = true;
    let assets = assets_with_profiles();
    let landing = WorldPoint3D::new(LAND_X, LAND_Y, LAND_Z);
    let net_id = engine.add_entity(make_net(landing));
    let rider_id = add_soldier(
        &mut engine,
        WorldPoint3D::new(LAND_X, LAND_Y, LAND_Z),
        0,
        true,
    );
    let victim_id = add_soldier(
        &mut engine,
        WorldPoint3D::new(LAND_X + 5.0, LAND_Y, LAND_Z),
        0,
        false,
    );
    let Entity::Soldier(victim) = engine
        .get_entity_mut(victim_id)
        .expect("test allied victim")
    else {
        unreachable!()
    };
    victim.soldier.cached_camp = crate::element::Camp::Royalists;
    let vip_id = add_soldier(
        &mut engine,
        WorldPoint3D::new(LAND_X + 10.0, LAND_Y, LAND_Z),
        1,
        false,
    );
    let stuteley_id =
        engine.add_entity(make_pc(WorldPoint3D::new(LAND_X + 15.0, LAND_Y, LAND_Z), 1));

    engine.apply_net_falling_effect(sim, &assets, net_id);

    let Entity::Net(net) = engine.get_entity(net_id).unwrap() else {
        panic!("test net changed entity kind");
    };
    assert!(!net.net.crumpled);
    assert_eq!(net.net.victims, vec![victim_id]);
    assert_eq!(count_receive_net_for(&engine, rider_id), 0);
    assert_eq!(count_receive_net_for(&engine, vip_id), 0);
    assert_eq!(count_receive_net_for(&engine, stuteley_id), 0);
    assert_eq!(count_receive_net_for(&engine, victim_id), 1);
}

#[test]
fn net_capture_circle_keeps_original_strict_radius_boundary() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    engine
        .control
        .sim_config
        .item_gameplay
        .net_selective_immunity = true;
    let assets = assets_with_profiles();
    let landing = WorldPoint3D::new(LAND_X, LAND_Y, LAND_Z);
    let net_id = engine.add_entity(make_net(landing));
    let inside_id = add_soldier(
        &mut engine,
        WorldPoint3D::new(LAND_X + 39.999, LAND_Y, LAND_Z),
        0,
        false,
    );
    let boundary_id = add_soldier(
        &mut engine,
        WorldPoint3D::new(LAND_X + 40.0, LAND_Y, LAND_Z),
        0,
        false,
    );

    engine.apply_net_falling_effect(sim, &assets, net_id);

    let Entity::Net(net) = engine.get_entity(net_id).unwrap() else {
        panic!("test net changed entity kind");
    };
    assert_eq!(net.net.victims, vec![inside_id]);
    assert_eq!(count_receive_net_for(&engine, boundary_id), 0);
}

#[test]
fn net_crumples_on_vip_soldier_alone() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let assets = assets_with_profiles();
    let landing = WorldPoint3D {
        x: LAND_X,
        y: LAND_Y,
        z: LAND_Z,
    };
    let net_id = engine.add_entity(make_net(landing));
    // Profile 1 = VIP soldier
    let vip_id = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: 0.0,
        },
        1,
        false,
    ));

    engine.apply_net_falling_effect(sim, &assets, net_id);

    let net = match engine.get_entity(net_id).unwrap() {
        Entity::Net(n) => n,
        _ => panic!("not a net"),
    };
    assert!(net.net.crumpled);
    assert!(net.net.victims.is_empty());
    assert_eq!(count_receive_net_for(&engine, vip_id), 0);
}

#[test]
fn net_with_existing_victim_ignores_new_rider() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    // Pre-existing victim simulates a previous capture-sweep
    // call's captures; a Rider seen on a subsequent sweep triggers
    // the "new arrivants won't be caught" branch — the net does
    // NOT crumple, and the existing victim list is kept.
    let mut engine = make_engine();
    let assets = assets_with_profiles();
    let landing = WorldPoint3D {
        x: LAND_X,
        y: LAND_Y,
        z: LAND_Z,
    };
    let net_id = engine.add_entity(make_net(landing));
    let existing_id = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: 0.0,
        },
        0,
        false,
    ));
    let rider_id = engine.add_entity(make_soldier(
        WorldPoint3D {
            x: LAND_X + 5.0,
            y: LAND_Y,
            z: 0.0,
        },
        0,
        true,
    ));
    // Seed the net with an already-captured victim so the
    // crumple guard sees a non-empty list.
    if let Some(Entity::Net(n)) = engine.world.entities.get_mut(net_id) {
        n.net.victims.push(existing_id);
    }

    engine.apply_net_falling_effect(sim, &assets, net_id);

    let net = match engine.get_entity(net_id).unwrap() {
        Entity::Net(n) => n,
        _ => panic!("not a net"),
    };
    assert!(
        !net.net.crumpled,
        "rider with existing victim must not crumple"
    );
    // No new captures were added (rider triggered the early return
    // before the existing victim could be re-processed; existing
    // entry is dedup'd against itself).
    assert_eq!(net.net.victims, vec![existing_id]);
    assert_eq!(count_receive_net_for(&engine, rider_id), 0);
}

#[test]
fn net_skips_humans_outside_radius() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let assets = assets_with_profiles();
    let landing = WorldPoint3D {
        x: LAND_X,
        y: LAND_Y,
        z: LAND_Z,
    };
    let net_id = engine.add_entity(make_net(landing));
    let near_id = add_soldier(
        &mut engine,
        WorldPoint3D {
            x: LAND_X + 5.0,
            y: LAND_Y,
            z: 0.0,
        },
        0,
        false,
    );
    // 200 units away in X — way outside SQUARE_RADIUS_NET_CAPTURE.
    let far_id = add_soldier(
        &mut engine,
        WorldPoint3D {
            x: LAND_X + 200.0,
            y: LAND_Y,
            z: 0.0,
        },
        0,
        false,
    );

    engine.apply_net_falling_effect(sim, &assets, net_id);

    let net = match engine.get_entity(net_id).unwrap() {
        Entity::Net(n) => n,
        _ => panic!("not a net"),
    };
    assert_eq!(net.net.victims, vec![near_id]);
    assert_eq!(count_receive_net_for(&engine, near_id), 1);
    assert_eq!(count_receive_net_for(&engine, far_id), 0);
}

#[test]
fn net_crumples_on_stuteley_pc() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let assets = assets_with_profiles();
    let landing = WorldPoint3D {
        x: LAND_X,
        y: LAND_Y,
        z: LAND_Z,
    };
    let net_id = engine.add_entity(make_net(landing));
    // Character profile 1 = Stuteley (Action::Net present).
    let _ = engine.add_entity(make_pc(
        WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: 0.0,
        },
        1,
    ));

    engine.apply_net_falling_effect(sim, &assets, net_id);

    let net = match engine.get_entity(net_id).unwrap() {
        Entity::Net(n) => n,
        _ => panic!("not a net"),
    };
    assert!(net.net.crumpled);
    assert!(net.net.victims.is_empty());
}

#[test]
fn unapply_clears_victims_and_releases_counters() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let assets = assets_with_profiles();
    let landing = WorldPoint3D {
        x: LAND_X,
        y: LAND_Y,
        z: LAND_Z,
    };
    let net_id = engine.add_entity(make_net(landing));
    let victim_id = add_soldier(
        &mut engine,
        WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: 0.0,
        },
        0,
        false,
    );
    engine.add_detectable_for_all_npc(victim_id, crate::element::DetectableType::Body);
    let body_slot = crate::element::DetectableType::Body as usize;
    assert!(
        engine
            .get_entity(victim_id)
            .and_then(Entity::npc_data)
            .expect("test victim NPC")
            .detectable_lists[body_slot]
            .iter()
            .any(|detectable| detectable.element == Some(victim_id))
    );

    // First, fire the apply sweep so the victim is registered.
    // The sweep eagerly increments stuck_under_nets_counter; the
    // posture snap to StuckUnderNet would normally happen the
    // next frame inside `EngineInner::apply_net` (the ReceiveNet
    // damage handler). We don't run the per-tick dispatcher in
    // this unit test, so set the posture by hand to simulate the
    // post-dispatch state.
    engine.apply_net_falling_effect(sim, &assets, net_id);
    if let Some(entity) = engine.world.entities.get_mut(victim_id) {
        entity.set_posture_stuck_under_net_for_human();
    }
    assert_eq!(
        engine
            .get_entity(victim_id)
            .unwrap()
            .human_data()
            .unwrap()
            .stuck_under_nets_counter,
        1,
        "apply_net_falling_effect should eagerly increment counter to 1"
    );
    assert_eq!(
        engine
            .get_entity(victim_id)
            .unwrap()
            .element_data()
            .posture(),
        Posture::StuckUnderNet
    );

    engine.unapply_net_effect(sim, &assets, net_id);

    let net = match engine.get_entity(net_id).unwrap() {
        Entity::Net(n) => n,
        _ => panic!("not a net"),
    };
    assert!(net.net.victims.is_empty(), "victims drained");
    let v = engine.get_entity(victim_id).unwrap();
    assert_eq!(v.human_data().unwrap().stuck_under_nets_counter, 0);
    assert_eq!(v.element_data().posture(), Posture::Lying);
    assert!(
        v.npc_data().expect("test victim NPC").detectable_lists[body_slot]
            .iter()
            .all(|detectable| detectable.element != Some(victim_id))
    );
}

#[test]
fn vip_soldier_says_vip_net_no_remark() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    // The VipNetNo remark fires for VIP soldiers in the crumple
    // radius. We populate an EnemyAi brain so the say() call has
    // somewhere to land.
    use crate::ai::AiController;
    use crate::ai_enemy::EnemyAi;
    let mut engine = make_engine();
    let assets = assets_with_profiles();
    let landing = WorldPoint3D {
        x: LAND_X,
        y: LAND_Y,
        z: LAND_Z,
    };
    let net_id = engine.add_entity(make_net(landing));
    let mut vip = make_soldier(
        WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: 0.0,
        },
        1, // VIP profile
        false,
    );
    if let Entity::Soldier(ref mut s) = vip {
        s.npc.ai_brain = crate::element::AiBrain::Enemy(Box::new(EnemyAi {
            base: AiController::default(),
            ..Default::default()
        }));
    }
    let vip_id = engine.add_entity(vip);

    engine.apply_net_falling_effect(sim, &assets, net_id);

    let entity = engine.get_entity(vip_id).unwrap();
    let remark = entity
        .npc_data()
        .and_then(|n| n.ai_brain.base())
        .map(|b| b.current_remark)
        .unwrap();
    assert_eq!(remark, crate::ai::Remark::VipNetNo);
}

#[test]
fn capture_sets_victim_display_order_behind_net() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    // Capture should mark the victim's sprite as
    // `display_order_ref = Some(net_id)` + `behind = true`.
    let mut engine = make_engine();
    let assets = assets_with_profiles();
    let landing = WorldPoint3D {
        x: LAND_X,
        y: LAND_Y,
        z: LAND_Z,
    };
    let net_id = engine.add_entity(make_net(landing));
    // The victim already has a default Sprite (non-Option).
    let victim_id = add_soldier(
        &mut engine,
        WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: 0.0,
        },
        0,
        false,
    );

    engine.apply_net_falling_effect(sim, &assets, net_id);

    let sprite = engine.get_entity(victim_id).unwrap().sprite();
    assert_eq!(sprite.display_order_ref, Some(net_id));
    assert!(sprite.behind_display_order_ref);

    engine.unapply_net_effect(sim, &assets, net_id);
    let sprite = engine.get_entity(victim_id).unwrap().sprite();
    assert_eq!(
        sprite.display_order_ref, None,
        "unapply should clear the behind-net reference"
    );
    assert!(!sprite.behind_display_order_ref);
}

#[test]
fn landing_registers_repulsive_points() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = make_engine();
    let assets = assets_with_profiles();
    let landing = WorldPoint3D {
        x: LAND_X,
        y: LAND_Y,
        z: LAND_Z,
    };
    let net_id = engine.add_entity(make_net(landing));

    // Manually fire the landing-time helper (no flight ticking).
    engine.register_net_repulsive_points(net_id);

    let net = match engine.get_entity(net_id).unwrap() {
        Entity::Net(n) => n,
        _ => panic!("not a net"),
    };
    assert_eq!(net.net.repulsive_point_ids.len(), 2);
    // Two repulsive points should be registered on AiGlobalState.
    let registered_ids: Vec<i32> = engine
        .ai
        .global
        .repulsive_points
        .iter()
        .map(|p| p.id)
        .collect();
    for id in &net.net.repulsive_point_ids {
        assert!(registered_ids.contains(id));
    }

    engine.unapply_net_effect(sim, &assets, net_id);
    // After unapply: zero repulsive points left.
    assert!(engine.ai.global.repulsive_points.is_empty());
}

#[test]
fn taking_net_animation_dispatched_for_pc() {
    // PC taking a net should pick `OrderType::TakingNet` in the
    // dispatcher. Soldiers picking purses still get `Taking`.
    // We verify by populating PC + net + manually launching a
    // Take element, then asserting the active_ai_anim type.
    use crate::sequence::SequenceElement;
    let mut engine = make_engine();
    let assets = assets_with_profiles();
    let landing = WorldPoint3D {
        x: LAND_X,
        y: LAND_Y,
        z: LAND_Z,
    };
    let net_id = engine.add_entity(make_net(landing));
    let pc_id = engine.add_entity(make_pc(
        WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: 0.0,
        },
        1, // Stuteley (has Action::Net)
    ));
    // The full hourglass resolves the PC's portrait/ammo state through
    // its campaign-description identity; install it like production
    // roster construction does.
    engine
        .mission_domain
        .campaign
        .characters
        .push(crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(1)),
            ..Default::default()
        });
    engine
        .world
        .entities
        .get_mut(pc_id)
        .and_then(Entity::pc_data_mut)
        .expect("test PC data")
        .campaign_description_index = Some(0);

    // Fire the landing path so the net is actually on the ground.
    let sim = crate::sim_rng::test_context();
    engine.snap_net_to_landing_obstacle(&sim, &assets, net_id);

    // Launch Take(antagonist=net) targeting the PC.
    let elem = SequenceElement::new_interaction(1, Command::Take, Some(pc_id), Some(net_id));
    engine.launch_element(elem);
    // Process the pending element so the dispatcher runs.
    let mut dev = crate::engine::DevState::default();
    let mut display = crate::engine::HostDisplayState::default();
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);

    let active_anim = engine
        .orders
        .sequence_manager
        .current_order_for_actor(pc_id)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        active_anim,
        Some(crate::order::OrderType::TakingNet),
        "PC picking up a net should play TakingNet, not generic Taking"
    );
}
