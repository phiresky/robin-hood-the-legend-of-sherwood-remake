use super::*;

#[test]
fn pc_auto_heal_and_projectile_damage_follow_cross_entity_creation_order() {
    // An arrow impact does not subtract life inline: it launches a damage
    // sequence element that only reaches the victim's Translate when the
    // sequence manager drains its to-go queue, and that drain runs after the
    // whole per-entity hourglass loop. The PC auto-heal (74 -> 75) therefore
    // always precedes the arrow's 10 damage within one frame — regardless of
    // which creation slot the arrow occupies: 74 -> 75 -> 65 in both orders.
    assert_eq!(immortal_pc_hit_by_creation_ordered_arrow(true), 65);
    assert_eq!(immortal_pc_hit_by_creation_ordered_arrow(false), 65);
}

#[test]
fn earlier_projectile_runs_before_later_bow_release_and_spawned_arrow_runs_again() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::bow_shot::{SpawnArrowParams, spawn_arrow};
    use crate::coordinates::{WorldPoint3D, WorldVec3D};
    use crate::element::{ActionState, Command, Posture, TrajectoryPoint};
    use crate::entity_id::{PcId, ProjectileId, SoldierId};
    use crate::movement::ActiveShot;
    use crate::order::{Order, OrderType};
    use crate::profiles::{
        BowProfile, BowShootMode, CharacterProfile, ProfileManager, SoldierProfile,
    };
    use crate::sequence::SequenceElement;
    use crate::weapons::ShootMode;

    let mut engine = EngineInner::new();
    let mut target = make_test_soldier(Posture::Upright);
    target
        .element_data_mut()
        .set_position_map(MapPoint::new(1000.0, 0.0));
    target.element_data_mut().set_position(WorldPoint3D {
        x: 1000.0,
        y: 0.0,
        z: 0.0,
    });
    let Entity::Soldier(target_data) = &mut target else {
        unreachable!();
    };
    target_data.soldier.soldier_profile_index = crate::profiles::SoldierProfileIdx(0);
    target_data.npc.life_points = 100;
    let target_id = engine.add_entity(target);
    assert_eq!(target_id, EntityId::Soldier(SoldierId(0)));

    let shooter_id = EntityId::Pc(PcId(2));
    let existing_arrow = spawn_arrow(SpawnArrowParams {
        shooter: shooter_id,
        bow_point: WorldPoint3D {
            x: 2000.0,
            y: 0.0,
            z: 25.0,
        },
        trajectory_origin: MapPoint::new(2000.0, 0.0),
        target: target_id,
        target_pos: MapPoint::new(1000.0, 0.0),
        trajectory: vec![TrajectoryPoint {
            position: WorldPoint3D {
                x: 1000.0,
                y: 0.0,
                z: 25.0,
            },
            time: 2,
        }],
        damage: 10,
        layer: 0,
        lands_in_hole: false,
        initial_velocity: WorldVec3D {
            x: -1.0,
            y: 0.0,
            z: 0.0,
        },
    });
    let existing_arrow_id = engine.add_entity(existing_arrow);
    assert_eq!(existing_arrow_id, EntityId::Projectile(ProjectileId(1)));

    let mut shooter = make_test_pc(Posture::Upright);
    shooter
        .element_data_mut()
        .set_position_map(MapPoint::new(0.0, 0.0));
    shooter.element_data_mut().set_position(WorldPoint3D {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
    assert_eq!(engine.add_entity(shooter), shooter_id);
    bind_test_bow_release_action(&mut engine, shooter_id);

    let mut shot_element =
        SequenceElement::new_interaction(1, Command::ShootBow, Some(shooter_id), Some(target_id));
    let order = Order::test_new(OrderType::ShootingWithBow, 0.0, 0.0);
    let order_id = order.order_id;
    shot_element.orders.push_back(order);
    let shot_sequence = engine.orders.sequence_manager.launch_element(shot_element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(shot_sequence, 0);
    {
        let shooter = engine
            .get_entity_mut(shooter_id)
            .expect("bow shooter present");
        let actor = shooter.actor_data_mut().expect("bow shooter actor data");
        actor.action_state = ActionState::AimingWithBow;
        actor.active_shot = ActiveShot {
            sequence_id: Some(shot_sequence),
            element_index: 0,
            target: Some(target_id),
            order_id: Some(order_id),
            released: false,
            shoot_mode: Some(ShootMode::Normal),
        };
        // This fixture primes the shooting sprite below before entering the
        // production owner loop. Mirror the actor's last-order identity so that an already-
        // running row is not mistaken for human action initialization.
        actor.last_execute_order_id = Some(order_id);
    }

    let mut profiles = ProfileManager::new();
    profiles.characters.push(CharacterProfile {
        shooting_weapon_id: 1,
        shooting: 100,
        ..CharacterProfile::default()
    });
    profiles.soldiers.push(SoldierProfile {
        hth_weapon_id: 1,
        ..SoldierProfile::default()
    });
    profiles.hth_weapons.push(Default::default());
    profiles.bows.push(BowProfile {
        normal_shoot: BowShootMode {
            range: 2000,
            damage: 10,
            ..BowShootMode::default()
        },
        ..BowProfile::default()
    });
    let mut assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };
    complete_test_runtime_fixture(&mut engine, &mut assets);
    bind_test_bow_release_action(&mut engine, shooter_id);
    let shoot_direction = crate::position_interface::vector_to_sector_0_to_15_iso(1000.0, 0.0);
    engine
        .get_entity_mut(shooter_id)
        .expect("bow shooter present after fixture")
        .element_data_mut()
        .set_direction_instantly(shoot_direction);
    let motion = engine
        .get_entity_mut(shooter_id)
        .expect("bow shooter present after fixture")
        .element_data_mut()
        .sprite
        .perform_action(
            sim,
            Some(order_id),
            OrderType::ShootingWithBow,
            shoot_direction as u16,
            crate::sprite::FrameProgression::Default,
            false,
        );
    assert_eq!(motion, crate::sprite::MotionState::Start);
    let motion = engine
        .get_entity_mut(shooter_id)
        .expect("bow shooter present after first animation pulse")
        .element_data_mut()
        .sprite
        .perform_action(
            sim,
            Some(order_id),
            OrderType::ShootingWithBow,
            shoot_direction as u16,
            crate::sprite::FrameProgression::Default,
            false,
        );
    assert_eq!(motion, crate::sprite::MotionState::InProgress);

    let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    let (_, visited) = engine.with_simulation_context(|engine, sim| {
        capture_ordered_gameplay_entities(|| {
            engine.tick_actor_owner_envelopes(sim, &assets, &positions)
        })
    });

    let shot_after = engine
        .get_entity(shooter_id)
        .expect("bow shooter remains")
        .actor_data()
        .expect("bow shooter actor data")
        .active_shot;
    assert!(
        shot_after.released,
        "prepared shooting action did not reach its release pulse: {shot_after:?}"
    );

    let spawned_arrow_id = EntityId::Projectile(ProjectileId(3));
    assert_eq!(
        visited,
        vec![target_id, existing_arrow_id, shooter_id, spawned_arrow_id],
        "the existing impact must run before bow release, and the appended arrow must be reached by the live-size loop"
    );
    let spawned_arrow = match engine.get_entity(spawned_arrow_id) {
        Some(Entity::Projectile(projectile)) => projectile,
        _ => panic!("bow release did not leave the spawned arrow alive"),
    };
    assert!(
        spawned_arrow.projectile.launch_segment_start.is_none(),
        "the spawned arrow's explicit primer must be consumed before its second, registered update"
    );
}

#[test]
#[should_panic(expected = "no original-game concrete-kind mapping for ObjectType::None")]
fn inactive_unsupported_projectile_mapping_panics_before_owner_slot_retention() {
    let mut engine = EngineInner::new();
    engine.add_entity(Entity::Projectile(crate::element::ElementProjectile {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::ObjectProjectile;
            initial_element.active = false;
            initial_element
        },
        object: crate::element::ObjectData {
            object_type: crate::element::ObjectType::None,
            ..Default::default()
        },
        projectile: Default::default(),
    }));
    engine.perform_hourglass(
        &mut HostDisplayState::default(),
        &mut InputState::default(),
        &LevelAssets::new(),
        &mut DevState::default(),
    );
}

#[test]
fn inactive_projectile_virtual_results_are_applied_after_derived_tails() {
    use crate::element::{
        Animation, ElementData, ElementKind, ElementNet, ElementProjectile, ObjectData, ObjectType,
    };

    fn projectile(object_type: ObjectType, flying: bool) -> Entity {
        Entity::Projectile(ElementProjectile {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ObjectProjectile;
                initial_element.active = false;
                initial_element
            },
            object: ObjectData {
                object_type,
                animation: Animation::ObjectFlying,
                ..Default::default()
            },
            projectile: crate::element::ProjectileData {
                flying,
                ..Default::default()
            },
        })
    }

    let mut engine = EngineInner::new();
    let apple = engine.add_entity(projectile(ObjectType::Apple, false));
    let stone = engine.add_entity(projectile(ObjectType::Stone, false));
    let grounded_purse = engine.add_entity(projectile(ObjectType::Purse, false));
    let flying_purse = engine.add_entity(projectile(ObjectType::Purse, true));
    let grounded_coin = engine.add_entity(projectile(ObjectType::Coin, false));
    let flying_coin = engine.add_entity(projectile(ObjectType::Coin, true));
    let grounded_net = engine.add_entity(Entity::Net(ElementNet {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ObjectNet;
            initial_element.active = false;
            initial_element
        },
        object: ObjectData {
            object_type: ObjectType::Net,
            animation: Animation::NetUnfolding,
            ..Default::default()
        },
        projectile: crate::element::ProjectileData {
            flying: false,
            ..Default::default()
        },
        net: Default::default(),
    }));
    let flying_net = engine.add_entity(Entity::Net(ElementNet {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ObjectNet;
            initial_element.active = false;
            initial_element
        },
        object: ObjectData {
            object_type: ObjectType::Net,
            animation: Animation::ObjectFlying,
            ..Default::default()
        },
        projectile: crate::element::ProjectileData {
            flying: true,
            ..Default::default()
        },
        net: crate::element::NetData {
            time_till_unfolding: 1,
            ..Default::default()
        },
    }));
    let assets = LevelAssets::new();
    let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    let (_, tails) = capture_projectile_derived_tails(|| {
        engine.with_simulation_context(|engine, sim| {
            engine.tick_actor_owner_envelopes(sim, &assets, &positions)
        })
    });

    assert!(engine.get_entity(apple).is_some());
    assert!(engine.get_entity(stone).is_some());
    assert!(engine.get_entity(flying_purse).is_some());
    assert!(engine.get_entity(grounded_purse).is_some());
    assert!(engine.get_entity(grounded_coin).is_some());
    assert!(engine.get_entity(flying_coin).is_some());
    assert!(engine.get_entity(grounded_net).is_some());
    assert!(engine.get_entity(flying_net).is_some());
    for id in [apple, stone, flying_purse] {
        assert!(
            !engine.get_entity(id).unwrap().is_active(),
            "{id:?} must remain as an inactive tombstone"
        );
    }
    assert_eq!(
        tails,
        vec![
            (apple, ObjectType::Apple),
            (stone, ObjectType::Stone),
            (grounded_purse, ObjectType::Purse),
            (flying_purse, ObjectType::Purse),
            (grounded_coin, ObjectType::Coin),
            (flying_coin, ObjectType::Coin),
        ],
        "each inactive derived sprite tail must run before its virtual bool controls tombstone retention"
    );
    for id in [grounded_purse, grounded_coin] {
        let Entity::Projectile(projectile) = engine.get_entity(id).unwrap() else {
            unreachable!()
        };
        assert_eq!(projectile.object.animation, Animation::ObjectBursting);
    }
    let Entity::Net(net) = engine.get_entity(grounded_net).unwrap() else {
        unreachable!()
    };
    assert_eq!(net.object.animation, Animation::ObjectLying);
    let Entity::Net(net) = engine.get_entity(flying_net).unwrap() else {
        unreachable!()
    };
    assert_eq!(net.net.time_till_unfolding, 0);
    assert_eq!(net.object.animation, Animation::NetUnfolding);
}

#[test]
fn grounded_arrow_exposes_terminal_active_frame_then_refresh_retires_its_slot() {
    use crate::element::{
        Animation, ElementData, ElementKind, ElementProjectile, ObjectData, ObjectType,
        ProjectileData,
    };

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let arrow = engine.add_entity(Entity::Projectile(ElementProjectile {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ObjectProjectile;
            initial_element.active = true;
            initial_element
        },
        object: ObjectData {
            object_type: ObjectType::Arrow,
            animation: Animation::ObjectFlying,
            ..Default::default()
        },
        projectile: ProjectileData {
            flying: false,
            velocity_increment: crate::coordinates::WorldVec3D::new(2.0, 1.0, -0.5),
            ..Default::default()
        },
    }));
    {
        let Entity::Projectile(projectile) = engine.get_entity_mut(arrow).unwrap() else {
            unreachable!()
        };
        projectile
            .element
            .set_position(crate::coordinates::WorldPoint3D::new(12.0, 8.0, 4.0));
        projectile
            .element
            .sprite
            .position_iface
            .set_old_position(crate::coordinates::WorldPoint3D::ZERO);
        projectile
            .element
            .sprite
            .position_iface
            .set_old_map_position(crate::coordinates::MapPoint::ZERO);
    }
    let assets = LevelAssets::new();

    engine.tick_projectile_or_net_hourglass(&sim, &assets, arrow);
    let Entity::Projectile(projectile) = engine.get_entity(arrow).unwrap() else {
        unreachable!()
    };
    assert!(
        projectile.element.active,
        "the original game records terminal update state before arrow refresh"
    );
    assert!(
        !projectile.element.sprite.position_iface.is_moving(),
        "active non-flying projectile ticking still updates movement bookkeeping"
    );
    engine.control.arrow_refresh_pending = true;
    engine.apply_pending_presentation_refresh(&sim);
    let Entity::Projectile(projectile) = engine.get_entity(arrow).unwrap() else {
        unreachable!()
    };
    assert!(
        !projectile.element.active,
        "the between-frame Refresh must retire a stationary empty arrow"
    );

    engine.tick_projectile_or_net_hourglass(&sim, &assets, arrow);
    assert!(
        engine.get_entity(arrow).is_some(),
        "inactive arrow must remain as a tombstone"
    );
}

#[test]
fn frame_sound_refresh_waits_for_the_post_snapshot_presentation_boundary() {
    use crate::coordinates::{SpriteFrameOffset, SpriteLocalPoint};
    use crate::element::{ElementData, ElementFx, ElementKind, Entity, FxData};
    use crate::sprite::Sprite;
    use crate::sprite_script::SpriteScript;

    let mut sprite = Sprite::new(
        std::sync::Arc::new(vec![SpriteScript {
            action_id: 0,
            action_done: 0,
            average_speed: 0.0,
            hotspot: SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![SpriteFrameOffset::ZERO],
            sound_ids: vec![49],
        }]),
        std::sync::Arc::new(vec![0]),
    );
    sprite.current_frame = 0;
    sprite.frame_count = 0;

    let mut engine = EngineInner::new();
    let fx_id = engine.add_entity(Entity::Fx(ElementFx {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::Fx;
            initial_element.sprite = sprite;
            initial_element
        },
        fx: FxData::default(),
    }));
    let sim = crate::sim_rng::test_context();

    engine.apply_pending_presentation_refresh(&sim);
    assert_eq!(
        engine
            .get_entity(fx_id)
            .unwrap()
            .element_data()
            .sprite
            .last_sound_id,
        0,
        "a restored frame has no pending Refresh before its first parity snapshot"
    );
    assert!(engine.feedback.pending_side_effects.sounds.is_empty());

    engine.control.arrow_refresh_pending = true;
    engine.apply_pending_presentation_refresh(&sim);
    assert_eq!(
        engine
            .get_entity(fx_id)
            .unwrap()
            .element_data()
            .sprite
            .last_sound_id,
        49
    );
    assert!(matches!(
        engine.feedback.pending_side_effects.sounds.as_slice(),
        [super::super::super::SoundCommand::Fx { fx_id: 49, .. }]
    ));
}

#[test]
fn disappearing_arrow_human_hit_still_exposes_terminal_active_frame() {
    use crate::element::{
        Animation, ElementData, ElementKind, ElementProjectile, ObjectData, ObjectType,
        ProjectileData,
    };

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let arrow = engine.add_entity(Entity::Projectile(ElementProjectile {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ObjectProjectile;
            initial_element.active = true;
            initial_element
        },
        object: ObjectData {
            object_type: ObjectType::Arrow,
            animation: Animation::ObjectFlying,
            ..Default::default()
        },
        projectile: ProjectileData {
            disappear: true,
            flying: false,
            ..Default::default()
        },
    }));

    // Human impact returns true immediately, before projectile ticking can
    // inspect the trajectory's unrelated future disappear-on-landing state.
    engine.deactivate_projectile_tombstone(arrow, false);
    let Entity::Projectile(projectile) = engine.get_entity(arrow).unwrap() else {
        unreachable!()
    };
    assert!(
        projectile.element.active,
        "a future hole landing must not retire an arrow on its human-hit frame"
    );

    engine.control.arrow_refresh_pending = true;
    engine.apply_pending_presentation_refresh(&sim);
    let Entity::Projectile(projectile) = engine.get_entity(arrow).unwrap() else {
        unreachable!()
    };
    assert!(
        !projectile.element.active,
        "the stationary arrow is retired by the following Refresh"
    );
}

#[test]
fn falling_arrow_refresh_follows_fx_merged_display_order() {
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::element::{
        Animation, ElementData, ElementFx, ElementKind, ElementProjectile, FxData, ObjectData,
        ObjectType, ProjectileData, TrajectoryPoint,
    };
    use crate::sim_rng::{RngSite, SimulationContext};

    fn falling_arrow(position: WorldPoint3D) -> Entity {
        let mut element = {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ObjectProjectile;
            initial_element.active = true;
            initial_element
        };
        element.set_position(position);
        Entity::Projectile(ElementProjectile {
            element,
            object: ObjectData {
                object_type: ObjectType::Arrow,
                animation: Animation::ObjectFlying,
                ..Default::default()
            },
            projectile: ProjectileData {
                flying: true,
                falling: true,
                trajectory: vec![TrajectoryPoint { position, time: 1 }],
                trajectory_frame_count: 1,
                ..Default::default()
            },
        })
    }

    let mut engine = EngineInner::new();
    let shallower = engine.add_entity(falling_arrow(WorldPoint3D::new(0.0, 10.0, 0.0)));
    let deeper = engine.add_entity(falling_arrow(WorldPoint3D::new(100.0, 20.0, 0.0)));

    // This rising masking edge classifies only the deeper arrow as behind the
    // FX. The phase-three display-sort merge therefore extracts it
    // before the shallower arrow despite their initial world-Y order.
    let mut fx_element = {
        let mut initial_element = ElementData::default();
        initial_element.kind = ElementKind::Fx;
        initial_element.active = true;
        initial_element
    };
    fx_element.set_position(WorldPoint3D::new(50.0, 50.0, 1.0));
    let fx = engine.add_entity(Entity::Fx(ElementFx {
        element: fx_element,
        fx: FxData {
            display_polyline: vec![MapPoint::new(-10.0, 0.0), MapPoint::new(110.0, 30.0)],
            ..Default::default()
        },
    }));
    assert!(
        engine
            .get_entity(shallower)
            .unwrap()
            .element_data()
            .position()
            .y
            < engine
                .get_entity(deeper)
                .unwrap()
                .element_data()
                .position()
                .y
    );
    let draw_order = engine.compute_display_order();
    let relevant: Vec<_> = draw_order
        .ids
        .into_iter()
        .filter(|id| [shallower, deeper, fx].contains(id))
        .collect();
    assert_eq!(relevant, vec![deeper, fx, shallower]);

    let seed = (0..1024)
        .find(|&seed| {
            let probe = SimulationContext::with_seed(seed);
            let first = crate::sim_rng::u32(&probe, RngSite::ArrowFallingFrame, 0..3);
            let second = crate::sim_rng::u32(&probe, RngSite::ArrowFallingFrame, 0..3);
            first != second
        })
        .expect("a seed with distinct first two modulo-three draws");
    let expected = SimulationContext::with_seed(seed);
    let deeper_frame = crate::sim_rng::u32(&expected, RngSite::ArrowFallingFrame, 0..3) as u16 + 3;
    let shallower_frame =
        crate::sim_rng::u32(&expected, RngSite::ArrowFallingFrame, 0..3) as u16 + 3;
    assert_ne!(deeper_frame, shallower_frame);

    engine.control.arrow_refresh_pending = true;
    engine.apply_pending_presentation_refresh(&SimulationContext::with_seed(seed));
    let Entity::Projectile(deeper_arrow) = engine.get_entity(deeper).unwrap() else {
        unreachable!()
    };
    let Entity::Projectile(shallower_arrow) = engine.get_entity(shallower).unwrap() else {
        unreachable!()
    };
    assert_eq!(deeper_arrow.element.sprite.current_frame, deeper_frame);
    assert_eq!(
        shallower_arrow.element.sprite.current_frame,
        shallower_frame
    );
}

#[test]
fn successful_projectile_human_hit_rewind_settles_and_deletes_trajectory() {
    use crate::element::{
        ElementData, ElementKind, ElementProjectile, ObjectData, ObjectType, ProjectileData,
        TrajectoryPoint,
    };

    let mut engine = EngineInner::new();
    let old = crate::coordinates::WorldPoint3D::new(12.0, 8.0, 4.0);
    let projectile = engine.add_entity(Entity::Projectile(ElementProjectile {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ObjectProjectile;
            initial_element.active = true;
            initial_element
        },
        object: ObjectData {
            object_type: ObjectType::Arrow,
            ..Default::default()
        },
        projectile: ProjectileData {
            flying: false,
            trajectory: vec![TrajectoryPoint {
                position: crate::coordinates::WorldPoint3D::new(20.0, 10.0, 2.0),
                time: 4,
            }],
            ..Default::default()
        },
    }));

    engine.rewind_projectile_to_human_hit_old_position(projectile, old);
    let Entity::Projectile(projectile) = engine.get_entity(projectile).unwrap() else {
        unreachable!()
    };
    assert!(projectile.projectile.trajectory.is_empty());
    assert_eq!(projectile.element.position(), old);
    assert!(!projectile.element.sprite.position_iface.is_moving());
    assert_eq!(
        projectile.element.position_map(),
        projectile.element.sprite.position_iface.old_map_position()
    );
    let position = projectile
        .element
        .sprite
        .position_iface
        .v48_serialized_state();
    assert_eq!(position.computed_position.bits(), 7);
    assert_eq!(position.computed_increment.bits(), 2);
    assert_eq!(position.increment, projectile.projectile.velocity_increment);
}

#[test]
fn latent_active_shot_does_not_block_higher_selected_nonbow_order() {
    use crate::element::{Command, Posture};
    use crate::movement::ActiveShot;
    use crate::order::Order;
    use crate::sequence::SequenceElement;
    use crate::weapons::ShootMode;

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let mut selected = SequenceElement::new(1, Command::Wait, Some(owner));
    let order = Order::test_new(OrderType::WaitingUpright, 0.0, 0.0);
    let order_id = order.order_id;
    selected.orders.push_back(order);
    let selected_seq = engine.orders.sequence_manager.launch_element(selected);
    engine
        .orders
        .sequence_manager
        .element_in_progress(selected_seq, 0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_shot = ActiveShot {
        sequence_id: Some(selected_seq),
        element_index: 0,
        target: Some(owner),
        order_id: Some(order_id),
        released: false,
        shoot_mode: Some(ShootMode::Normal),
    };

    assert!(engine.selected_bow_order(owner).is_none());
    let (_, _, executed) = engine.tick_actor_animation_for(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
    );
    assert!(
        executed.is_some(),
        "latent active_shot must not suppress the exact selected nonbow Execute arm"
    );
}

#[test]
fn direct_drop_uses_the_same_one_shot_corpse_exit_initialization() {
    use crate::movement::AbilityKind;

    let (mut engine, carrier, body, _) =
        corpse_exit_initialization_fixture(true, crate::element::Command::WhistleCmd);
    assert_eq!(
        engine
            .get_entity(carrier)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_ability
            .kind,
        Some(AbilityKind::Drop)
    );

    engine.tick_actor_animation_action_change_slots(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
    );
    let body_entity = engine.get_entity(body).unwrap();
    assert_eq!(body_entity.element_data().direction(), 9);
    assert_eq!(body_entity.position_iface().get_direction_goal().as_u8(), 9);

    engine
        .get_entity_mut(body)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(2);
    engine.tick_actor_animation_action_change_slots(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
    );
    let body_entity = engine.get_entity(body).unwrap();
    assert_eq!(body_entity.element_data().direction(), 2);
    assert_eq!(body_entity.position_iface().get_direction_goal().as_u8(), 2);
}

#[test]
fn bound_bow_transition_advances_through_production_owner_coordinator() {
    use crate::element::{ActionState, Command, Posture};
    use crate::movement::ActiveShot;
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};
    use crate::weapons::ShootMode;

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let script = SpriteScript {
        action_id: OrderType::TransitionEquipBow as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::TransitionEquipBow as usize] = 0;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    let mut element = SequenceElement::new(1, Command::ShootBow, Some(owner));
    let order = Order::test_new(OrderType::TransitionEquipBow, 0.0, 0.0);
    let order_id = order.order_id;
    element.orders.push_back(order);
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    let actor = engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap();
    actor.action_state = ActionState::Waiting;
    actor.active_shot = ActiveShot {
        sequence_id: Some(sequence),
        element_index: 0,
        target: Some(owner),
        order_id: Some(order_id),
        released: false,
        shoot_mode: Some(ShootMode::Normal),
    };
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    positions[owner] = Some(crate::entities::BoundaryPosition::of(
        engine.get_entity(owner).unwrap().element_data(),
    ));

    engine.tick_actor_owner_envelopes(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        &positions,
    );

    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(entity.sprite().last_action, OrderType::TransitionEquipBow);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        ActionState::AimingWithBow
    );
}

#[test]
fn unbound_bow_transition_still_uses_generic_execute() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let script = SpriteScript {
        action_id: OrderType::TransitionEquipBow as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::TransitionEquipBow as usize] = 0;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    let mut element = SequenceElement::new(1, Command::Generic, Some(owner));
    element
        .orders
        .push_back(Order::test_new(OrderType::TransitionEquipBow, 0.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    positions[owner] = Some(crate::entities::BoundaryPosition::of(
        engine.get_entity(owner).unwrap().element_data(),
    ));

    engine.tick_actor_owner_envelopes(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        &positions,
    );

    assert_eq!(
        engine.get_entity(owner).unwrap().sprite().last_action,
        OrderType::TransitionEquipBow
    );
    assert!(
        !engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_shot
            .is_active()
    );
}

#[test]
fn terminal_bow_owner_defers_its_exposed_generic_successor_until_next_hourglass() {
    use crate::element::{Command, Posture};
    use crate::movement::ActiveShot;
    use crate::order::Order;
    use crate::sequence::SequenceElement;
    use crate::weapons::ShootMode;

    let sim_context = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let mut element = SequenceElement::new(1, Command::ShootBow, Some(owner));
    let bow_order = Order::test_new(OrderType::ShootingWithBow, 0.0, 0.0);
    let bow_order_id = bow_order.order_id;
    element.orders.push_back(bow_order);
    element
        .orders
        .push_back(Order::test_new(OrderType::WaitingUpright, 0.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    let actor = engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap();
    actor.active_shot = ActiveShot {
        sequence_id: Some(sequence),
        element_index: 0,
        target: Some(owner),
        order_id: Some(bow_order_id),
        released: false,
        shoot_mode: Some(ShootMode::Normal),
    };
    // The hook models terminal work from an already-entered specialized bow
    // Execute arm. Preserve its selected-order history just as a live prior
    // actor update would have done.
    actor.last_execute_order_id = Some(bow_order_id);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let initial_action = engine
        .get_entity(owner)
        .unwrap()
        .element_data()
        .sprite
        .last_action;

    engine.tick_actor_animation_action_change_slots_with_hooks(
        &sim_context,
        &assets,
        |_, _| {},
        |_, _| {},
        |engine, selected_owner, _, _, bow, _, _| {
            assert_eq!(selected_owner, owner);
            assert_eq!(bow, Some((sequence, 0, bow_order_id)));
            engine
                .orders
                .sequence_manager
                .get_element_mut(sequence, 0)
                .unwrap()
                .pop_current_order();
            engine
                .get_entity_mut(owner)
                .unwrap()
                .actor_data_mut()
                .unwrap()
                .active_shot = ActiveShot::default();
        },
        |_, _, _| {},
    );

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .sprite
            .last_action,
        initial_action,
        "the successor exposed by terminal bow work must not enter generic Execute in the same owner slot"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .unwrap()
            .2
            .order_type,
        OrderType::WaitingUpright
    );

    let (_, _, next_execute) = engine.tick_actor_animation_for(&sim_context, &assets, owner);
    assert_eq!(
        next_execute.unwrap().order_type,
        OrderType::WaitingUpright,
        "the exposed generic successor must become eligible at the next Execute boundary"
    );
}

#[test]
fn execution_frozen_selected_bow_does_not_advance_or_fire() {
    use crate::element::{Command, Posture};
    use crate::movement::ActiveShot;
    use crate::order::Order;
    use crate::sequence::SequenceElement;
    use crate::weapons::ShootMode;

    let mut engine = EngineInner::new();
    let shooter = engine.add_entity(make_test_pc(Posture::Upright));
    let target = engine.add_entity(make_test_pc(Posture::Upright));
    let mut element =
        SequenceElement::new_interaction(1, Command::ShootBow, Some(shooter), Some(target));
    let mut order = Order::test_new(OrderType::ShootingWithBow, 0.0, 0.0);
    order.antagonist = Some(target);
    let order_id = order.order_id;
    element.orders.push_back(order);
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    let actor = engine
        .get_entity_mut(shooter)
        .unwrap()
        .actor_data_mut()
        .unwrap();
    actor.execution_frozen = true;
    actor.active_shot = ActiveShot {
        sequence_id: Some(sequence),
        element_index: 0,
        target: Some(target),
        order_id: Some(order_id),
        released: false,
        shoot_mode: Some(ShootMode::Normal),
    };
    let before = engine
        .get_entity(shooter)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_shot;

    assert!(
        engine
            .tick_bow_shot_for(
                &crate::sim_rng::test_context(),
                &LevelAssets::new(),
                shooter,
                order_id
            )
            .is_empty()
    );
    assert_eq!(
        engine
            .get_entity(shooter)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_shot,
        before
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(shooter)
            .unwrap()
            .2
            .order_id,
        order_id
    );
    assert_eq!(
        engine
            .get_entity(shooter)
            .unwrap()
            .sprite()
            .last_processed_order_id,
        u32::MAX
    );
}

#[test]
fn production_throw_apple_owner_emits_terminal_projectile_effect() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let sim = crate::sim_rng::test_context();
    let mut assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let target = engine.add_entity(make_test_pc(Posture::Upright));
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(40.0, 0.0));
    bind_test_action_point(
        &mut engine,
        owner,
        OrderType::ThrowingApple,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );
    let script = SpriteScript {
        action_id: OrderType::ThrowingApple as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[OrderType::ThrowingApple as usize] = 0;
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    let element =
        SequenceElement::new_interaction(1, Command::ThrowApple, Some(owner), Some(target));
    let sequence = engine.orders.sequence_manager.launch_element(element);
    assert_eq!(
        crate::abilities::begin_throw_apple(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            owner,
            target,
            sequence,
            0,
            &mut engine.orders.next_order_id,
        ),
        crate::abilities::BeginResult::Started
    );
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    for _ in 0..10 {
        let positions_before_movement = engine.boundary_positions_snapshot();
        let mut display = CameraDisplayState::default();
        engine.tick_actor_owner_envelopes_with_display(
            &sim,
            &mut display,
            &assets,
            &positions_before_movement,
        );
    }

    assert_eq!(engine.world.entities.projectiles().count(), 1);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Terminated
    );
}

#[test]
fn selected_listen_done_does_not_clear_newer_bow_action() {
    use crate::element::{ActionState, Posture};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    {
        let pc = engine.get_entity_mut(owner).unwrap();
        pc.actor_data_mut().unwrap().action_state = ActionState::Waiting;
        pc.pc_data_mut().unwrap().current_action = crate::profiles::Action::Bow;
    }
    engine.players.seats[0].selection.push(owner);
    engine.players.seats[0].selected_action = crate::profiles::Action::Bow;

    engine.apply_listen_done_action_handoff(owner);

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .pc_data()
            .unwrap()
            .current_action,
        crate::profiles::Action::Bow,
        "a stale Listen completion must be filtered by the messenger-global action"
    );
    assert_eq!(
        engine.players.seats[0].selected_action,
        crate::profiles::Action::Bow
    );
}
