use super::*;
use crate::coordinates::{MapPoint, MapVec};
use crate::element::{
    ActionState, ActorData, ActorSoldier, ElementData, ElementKind, HumanData, NpcData, Posture,
    SoldierData,
};
use crate::fast_find_grid::GridLine;
use crate::order::{Order, OrderType};
use crate::sequence::SequenceElement;
use crate::sprite_script::SpriteScript;

fn dying_sprite() -> crate::sprite::Sprite {
    let action = OrderType::DyingSword;
    let script = SpriteScript {
        action_id: action as u16,
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
    let mut conversion = crate::engine::test_support::unmapped_conversion();
    conversion[action as usize] = 0;
    crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(conversion),
    )
}

fn dying_find_place_increment_after_crossing(
    patch_line_count: usize,
    precompute_increment: bool,
) -> (MapVec, MapVec, MapPoint) {
    let mut engine = EngineInner::new();
    engine.world.fast_grid_mut().size_map(4, 4);
    engine.world.fast_grid_mut().allocate_layers(1);

    // The lying box centered at (130,130) straddles this solid edge.
    // Death-place selection pushes it toward the click side (+Y), producing a
    // real generic-Execute movement segment.
    engine.world.fast_grid_mut().add_line(
        GridLine::new(
            MapPoint::new(100.0, 128.0),
            MapPoint::new(160.0, 128.0),
            true,
        ),
        0,
    );
    for offset in 0..patch_line_count {
        engine.world.fast_grid_mut().add_line(
            GridLine::new_patch(
                MapPoint::new(100.0, 131.0 + offset as f32),
                MapPoint::new(160.0, 131.0 + offset as f32),
                crate::patch::PatchIndex::new(offset as u32)
                    .expect("test patch index is representable"),
            ),
            0,
        );
    }
    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
        initial_element.kind = ElementKind::ActorSoldier;
        initial_element.active = true;
        initial_element.sprite = dying_sprite();
        initial_element
    };
    element.set_position_map(MapPoint::new(130.0, 130.0));
    // Aim horizontally before corpse placement so the relocation's +Y
    // displacement makes a successful post-cross recompute observably
    // different from the cached pre-Execute increment.
    element
        .sprite
        .position_iface
        .set_map_goal(MapPoint::new(200.0, 130.0));
    if precompute_increment {
        element.sprite.position_iface.compute_increment_all(false);
    }
    element.set_direction_instantly(13);
    let stale_increment = element.sprite.position_iface.raw_increment_map();
    let owner = engine.add_test_entity(Entity::Soldier(ActorSoldier {
        element,
        actor: ActorData {
            action_state: ActionState::WaitingSword,
            ..ActorData::default()
        },
        human: HumanData::default(),
        npc: NpcData::default(),
        soldier: crate::element::SoldierData {
            cached_camp: crate::element::Camp::Lacklandists,
            ..Default::default()
        },
    }));

    let mut dying = SequenceElement::new(1, Command::ReceiveSwordDamage, Some(owner));
    let mut order = Order::test_new(OrderType::DyingSword, 0.0, 0.0);
    order.compute_direction = false;
    dying.orders.push_back(order);
    let sequence_id = engine.orders.sequence_manager.launch_element(dying);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    engine.tick_actor_animation_action_change_slots(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
    );

    let entity = engine.get_entity(owner).expect("dying owner remains live");
    let old_position = entity.position_iface().old_map_position();
    let new_position = entity.element_data().position_map();
    let non_elevation_crossing_count = engine
        .world
        .fast_grid
        .get_actor_non_elevation_crossing_line_indices(0, old_position, new_position)
        .len();
    assert_eq!(old_position, MapPoint::new(130.0, 130.0));
    assert_eq!(non_elevation_crossing_count, patch_line_count);
    assert!(
        new_position.y > 132.0,
        "death-position search must cross both synthetic boundaries"
    );
    (
        stale_increment,
        entity.position_iface().raw_increment_map(),
        new_position,
    )
}

#[test]
fn find_place_to_die_multi_line_crossing_computes_uncached_increment() {
    let (stale, recomputed, position) = dying_find_place_increment_after_crossing(2, false);
    assert_ne!(recomputed, stale, "corpse relocation ended at {position:?}");
    let dx = 200.0 - position.x;
    let dy = 130.0 - position.y;
    let norm = (dx * dx + dy * dy).sqrt();
    let expected = MapVec::new(dx / norm, dy / norm);
    assert!((recomputed.x - expected.x).abs() < 1.0e-6);
    assert!((recomputed.y - expected.y).abs() < 1.0e-6);
}

#[test]
fn find_place_to_die_single_non_elevation_crossing_retains_increment() {
    let (stale, retained, _) = dying_find_place_increment_after_crossing(1, true);
    assert_eq!(retained, stale);
}

#[test]
fn find_place_to_die_multi_non_elevation_crossing_retains_cached_increment() {
    let (stale, retained, _) = dying_find_place_increment_after_crossing(2, true);
    assert_eq!(retained, stale);
}

#[test]
fn delayed_position_multi_non_elevation_crossing_recomputes_invalid_increment() {
    let mut engine = EngineInner::new();
    engine.world.fast_grid_mut().size_map(4, 4);
    engine.world.fast_grid_mut().allocate_layers(1);
    for (offset, patch_index) in [(131.0, 0), (132.0, 1)] {
        engine.world.fast_grid_mut().add_line(
            GridLine::new_patch(
                MapPoint::new(100.0, offset),
                MapPoint::new(160.0, offset),
                crate::patch::PatchIndex::new(patch_index)
                    .expect("test patch index is representable"),
            ),
            0,
        );
    }

    let stale = MapVec::new(1.0, 0.0);
    let destination = MapPoint::new(130.0, 134.0);
    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Tied);
        initial_element.kind = ElementKind::ActorSoldier;
        initial_element.active = true;
        initial_element
    };
    element.set_position_map(MapPoint::new(130.0, 130.0));
    element.sprite.position_iface.set_map_increment(stale);
    // The outgoing movement condolence writes the zero idle goal and
    // invalidates the cached increment before corpse placement commits.
    element.sprite.position_iface.set_map_goal(MapPoint::ZERO);
    element.set_position_map_delayed(destination);
    let owner = engine.add_test_entity(Entity::Soldier(ActorSoldier {
        element,
        actor: ActorData::default(),
        human: HumanData {
            unconscious: true,
            ..HumanData::default()
        },
        npc: NpcData::default(),
        soldier: crate::element::SoldierData {
            cached_camp: crate::element::Camp::Lacklandists,
            ..Default::default()
        },
    }));

    let mut wait = SequenceElement::new(1, Command::Wait, Some(owner));
    let mut order = Order::test_new(OrderType::BeingTied, 0.0, 0.0);
    order.compute_direction = false;
    wait.orders.push_back(order);
    let sequence_id = engine.orders.sequence_manager.launch_element(wait);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence_id, 0);

    let crossing_count = engine
        .world
        .fast_grid
        .get_actor_crossing_line_indices(0, MapPoint::new(130.0, 130.0), destination)
        .len();
    let elevation_count = engine
        .world
        .fast_grid
        .get_crossing_elevation_line_indices(0, MapPoint::new(130.0, 130.0), destination)
        .len();
    assert_eq!(crossing_count, 2);
    assert_eq!(elevation_count, 0);

    engine.apply_delayed_actor_position(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
    );

    let position = engine
        .get_entity(owner)
        .expect("delayed-position owner remains live")
        .position_iface();
    let recomputed = position.get_increment_map();
    let dx = -destination.x;
    let dy = -destination.y;
    let norm = (dx * dx + dy * dy).sqrt();
    let expected = MapVec::new(dx / norm, dy / norm);
    assert_ne!(recomputed, stale);
    assert!((recomputed.x - expected.x).abs() < 1.0e-6);
    assert!((recomputed.y - expected.y).abs() < 1.0e-6);
}
