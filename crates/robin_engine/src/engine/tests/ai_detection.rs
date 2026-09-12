use super::*;

fn mixed_enemy_fifo_fixture(
    pc_first: bool,
) -> (EngineInner, LevelAssets, EntityId, EntityId, EntityId) {
    use crate::ai::{AiLockFlags, AiState, Substate};
    use crate::ai_enemy::task_priority;
    use crate::element::{Camp, Detectable, DetectableType, Entity};

    let mut engine = EngineInner::new();
    let observer_id = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    let pc_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let royalist_id = engine.add_test_entity(make_test_ai_soldier(Camp::Royalists));

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("mixed-fifo observer exists")
    else {
        panic!("mixed-fifo observer changed kind")
    };
    observer.element.active = true;
    observer
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0));
    observer.element.set_position_map(MapPoint::new(0.0, 0.0));
    observer.element.set_direction_instantly(4);
    observer.npc.life_points = 100;
    observer.npc.view_direction = [1.0, 0.0];
    observer.npc.view_radius = 300;
    observer.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
    observer.npc.eye_status = crate::element::EyeStatus::Stare;

    let Entity::Pc(pc) = engine.get_entity_mut(pc_id).expect("mixed-fifo PC exists") else {
        panic!("mixed-fifo PC changed kind")
    };
    pc.element.active = true;
    pc.element
        .set_position(crate::coordinates::WorldPoint3D::new(80.0, 0.0, 0.0));
    pc.element.set_position_map(MapPoint::new(80.0, 0.0));
    pc.pc.life_points = 100;

    let Entity::Soldier(royalist) = engine
        .get_entity_mut(royalist_id)
        .expect("mixed-fifo Royalist target exists")
    else {
        panic!("mixed-fifo Royalist target changed kind")
    };
    royalist.element.active = true;
    royalist
        .element
        .set_position(crate::coordinates::WorldPoint3D::new(120.0, 0.0, 0.0));
    royalist.element.set_position_map(MapPoint::new(120.0, 0.0));
    royalist.npc.life_points = 100;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let profile = std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .get_mut(0)
        .expect("fixture installs the PC character profile");
    profile.detection_speed_in_city = 100;
    profile.detection_speed_in_forest = 100;

    let Entity::Soldier(observer) = engine
        .get_entity_mut(observer_id)
        .expect("mixed-fifo observer exists after fixture")
    else {
        panic!("mixed-fifo observer changed kind after fixture")
    };
    let ai = observer
        .npc
        .ai_brain
        .enemy_mut()
        .expect("mixed-fifo observer has EnemyAi");
    ai.base.me = observer_id.index();
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.base.locks_flag_field = AiLockFlags::BUSY;
    ai.base.got_the_beggar_trick = true;

    observer.npc.detectable_lists[DetectableType::Enemy as usize].clear();
    observer.npc.detection_suspects[DetectableType::Enemy as usize] = 999;
    let ordered = if pc_first {
        [pc_id, royalist_id]
    } else {
        [royalist_id, pc_id]
    };
    for target_id in ordered {
        observer.npc.detectable_lists[DetectableType::Enemy as usize].push(Detectable {
            element: Some(target_id),
            detectable_type: DetectableType::Enemy,
            // Keep the oracle on VIEW order, not shadow predetection.
            shadow_seen_last_frame: true,
            ..Detectable::default()
        });
    }

    (engine, assets, observer_id, pc_id, royalist_id)
}

fn make_discovery_bonus(x: f32) -> Entity {
    let mut element = {
        let mut initial_element = crate::element::ElementData::default();
        initial_element.kind = crate::element::ElementKind::ObjectBonus;
        initial_element.active = true;
        initial_element.blipped = true;
        initial_element
    };
    element.set_position(crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0));
    element.set_position_map(MapPoint::new(x, 0.0));
    Entity::Bonus(crate::element::ElementBonus {
        element,
        object: crate::element::ObjectData {
            object_type: crate::element::ObjectType::BonusApple,
            ..Default::default()
        },
    })
}

fn make_blipped_non_bonus(kind: crate::element::ElementKind) -> Entity {
    let mut element = {
        let mut initial_element = crate::element::ElementData::default();
        initial_element.kind = kind;
        initial_element.active = true;
        initial_element.blipped = true;
        initial_element
    };
    element.set_position(crate::coordinates::WorldPoint3D::new(10.0, 0.0, 0.0));
    element.set_position_map(MapPoint::new(10.0, 0.0));
    match kind {
        crate::element::ElementKind::ObjectScroll => {
            Entity::Scroll(crate::element::ElementScroll {
                element,
                object: crate::element::ObjectData {
                    object_type: crate::element::ObjectType::Scroll,
                    ..Default::default()
                },
                ..Default::default()
            })
        }
        crate::element::ElementKind::ObjectProjectile => {
            Entity::Projectile(crate::element::ElementProjectile {
                element,
                object: crate::element::ObjectData {
                    object_type: crate::element::ObjectType::Arrow,
                    ..Default::default()
                },
                projectile: Default::default(),
            })
        }
        crate::element::ElementKind::ObjectNet => Entity::Net(crate::element::ElementNet {
            element,
            object: crate::element::ObjectData {
                object_type: crate::element::ObjectType::Net,
                ..Default::default()
            },
            projectile: Default::default(),
            net: Default::default(),
        }),
        _ => panic!("unsupported non-bonus discovery fixture {kind:?}"),
    }
}

fn run_owner_envelopes(engine: &mut EngineInner, assets: &LevelAssets) {
    let mut positions = engine.boundary_positions_snapshot();
    crate::sim_rng::with_seed(0xB0A0_0013, |sim| {
        engine.tick_actor_owner_envelopes(sim, assets, &positions);
    });
}

mod combat;
mod movement;
mod orders;
mod perception;
mod projectiles;
mod state;
