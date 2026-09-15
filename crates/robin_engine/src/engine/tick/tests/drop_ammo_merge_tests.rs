use super::*;
use crate::campaign::{Campaign, PcDescription};
use crate::element::{ActorPc, ElementData, ElementKind, EntityId, Posture};
use crate::profiles::{Action, CharacterProfileIdx};
use crate::sequence::{Field, FieldValue, SequenceElement};

fn count_bonuses(engine: &EngineInner, action: Action) -> Vec<(EntityId, u16)> {
    engine
        .world
        .entities
        .bonuses()
        .filter_map(|(entity_id, bonus)| {
            if bonus.element.active && bonus.object.associated_action == action {
                Some((entity_id.into(), bonus.object.quantity))
            } else {
                None
            }
        })
        .collect()
}

/// Build an engine with one PC at the origin, a campaign with one
/// PcDescription whose status starts with `bow_ammo` arrows, and a
/// move-box that lets `find_authorized_position_toward` return a
/// valid drop position on the empty FastFindGrid.
fn build_engine_with_pc(bow_ammo: u16) -> (EngineInner, EntityId, LevelAssets) {
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let pm = std::sync::Arc::make_mut(&mut assets.profile_manager);
    pm.characters.push(crate::profiles::CharacterProfile {
        index: 0,
        filename: "TEST_PC".into(),
        profile_name: "TEST".into(),
        ..Default::default()
    });
    let mut ale_conversion = crate::engine::test_support::unmapped_conversion();
    ale_conversion[crate::order::OrderType::ObjectLying as usize] = 0;
    assets.accessory_sprite_prototypes.insert(
        crate::element::ObjectType::Ale,
        crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
                action_id: crate::order::OrderType::ObjectLying as u16,
                frame_ids: vec![1],
                delays: vec![0],
                distances: vec![0],
                offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
                sound_ids: vec![0],
                ..Default::default()
            }]),
            std::sync::Arc::new(ale_conversion),
        ),
    );

    let mut campaign = Campaign::default();
    let mut desc = PcDescription {
        character_profile_idx: Some(CharacterProfileIdx(0)),
        ..Default::default()
    };
    desc.status.set_ammo(Action::Bow, bow_ammo);
    campaign.characters.push(desc);
    engine.mission_domain.campaign = campaign;

    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
        initial_element.kind = ElementKind::ActorPc;
        initial_element.active = true;
        initial_element
    };
    let mut pc_conversion = crate::engine::test_support::unmapped_conversion();
    pc_conversion[crate::order::OrderType::DroppingAle as usize] = 0;
    element.sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![crate::sprite_script::SpriteScript {
            action_id: crate::order::OrderType::DroppingAle as u16,
            action_done: 1,
            hotspot: crate::coordinates::SpriteLocalPoint::new(8.0, 4.0),
            frame_ids: vec![1, 2, 3],
            delays: vec![0, 0, 0],
            distances: vec![0, 0, 0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
            sound_ids: vec![0, 0, 0],
            ..Default::default()
        }]),
        std::sync::Arc::new(pc_conversion),
    );
    // Sprite::new owns a fresh PositionInterface, so place the fixture
    // after installing its authored DropAle script. Otherwise the shared
    // DropAmmo tests silently run from the sprite default at (0, 0).
    element.set_position_map(crate::coordinates::MapPoint { x: 100.0, y: 100.0 });
    element.set_direction_instantly(0);
    // Seed a non-empty move box so try_get_drop_position's
    // is_somewhere check passes.  The exact dims don't matter on
    // an empty grid.
    element
        .sprite
        .position_iface
        .set_move_box(crate::coordinates::MoveBox::from_corners(
            crate::coordinates::MapVec::new(-5.0, -5.0),
            crate::coordinates::MapVec::new(5.0, 5.0),
        ));

    let pc_id = engine.add_test_entity(crate::element::Entity::Pc(ActorPc {
        element,
        actor: Default::default(),
        human: Default::default(),
        pc: crate::element::PcData {
            profile_index: CharacterProfileIdx(0),
            campaign_description_index: Some(0),
            ..Default::default()
        },
    }));

    super::complete_test_runtime_fixture(&mut engine, &mut assets);
    (engine, pc_id, assets)
}

fn drop_ammo_and_tick(
    engine: &mut EngineInner,
    pc_id: EntityId,
    amount: u32,
    assets: &LevelAssets,
) {
    let mut elem = SequenceElement::new_generic(1, crate::element::Command::DropAmmo, Some(pc_id));
    elem.set_property(Field::ActionId, FieldValue::Integer(Action::Bow as u32));
    elem.set_property(Field::Amount, FieldValue::Integer(amount));
    engine.launch_element(&crate::sim_rng::test_context(), &assets, elem);

    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    engine.perform_hourglass(&mut display, &mut InputState::default(), assets, &mut dev);
}

#[test]
fn drop_ale_spawns_object_other_and_survives_its_next_live_owner_slot() {
    let (mut engine, pc_id, assets) = build_engine_with_pc(0);
    let expected_action_point = engine
        .get_entity(pc_id)
        .unwrap()
        .current_gameplay_point_map()
        .unwrap();
    engine.mission_domain.campaign.characters[0]
        .status
        .set_ammo(Action::Ale, 1);
    engine.launch_element(
        &crate::sim_rng::test_context(),
        &assets,
        SequenceElement::new(1, crate::element::Command::DropAle, Some(pc_id)),
    );

    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    // Translation installs the authored drop order; the bottle itself is
    // created only when that animation reaches its DONE action point.
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
    let (_, _, order) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(pc_id)
        .expect("DropAle must install its animation order");
    assert_eq!(order.order_type, crate::order::OrderType::DroppingAle);
    assert_eq!(
        engine.mission_domain.campaign.characters[0]
            .status
            .get_ammo(Action::Ale),
        1,
        "translation must not consume ale before the action point"
    );

    // Drive the real owner envelope across the sprite's authored DONE
    // frame. This is the lifecycle that the schema-14 Save028 replay
    // exercises; directly injecting ExecuteSideOutcomes would miss a
    // dropped callback between generic execution and the actor update.
    for _ in 0..4 {
        engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
    }
    assert_eq!(
        engine.mission_domain.campaign.characters[0]
            .status
            .get_ammo(Action::Ale),
        0,
        "DropAle DONE must consume one ale at the action point"
    );
    assert_eq!(
        engine
            .feedback
            .sound_sim
            .pending_exclamations
            .iter()
            .map(|pending| (pending.actor_id, pending.exclamation_id))
            .collect::<Vec<_>>(),
        vec![(pc_id.index(), crate::engine::melee::HERO_OUT_OF_AMMO)],
        "the last ale must synchronously queue HERO_OUT_OF_AMMO"
    );
    let ale_id = engine
        .world
        .entities
        .occupied()
        .find_map(|(id, entity)| {
            (entity
                .object_data()
                .is_some_and(|object| object.object_type == crate::element::ObjectType::Ale))
            .then_some(id)
        })
        .expect("completed ale dropping must append its ale element");
    let ale = engine.get_entity(ale_id).unwrap();
    assert_eq!(ale.kind(), ElementKind::ObjectOther);
    assert_eq!(ale.element_data().position_map(), expected_action_point);
    assert!(!ale.element_data().blipped);
    assert_eq!(ale.sprite().frame_count, 0);
    assert_eq!(
        ale.object_data().unwrap().animation,
        crate::element::Animation::ObjectLying
    );
    assert_eq!(
        ale.original_hourglass_class(),
        crate::element::OriginalHourglassClass::Ale
    );

    // The next frame resolves the appended slot through the real live
    // owner coordinator. A stale ObjectBonus label would panic here.
    engine.perform_hourglass(&mut display, &mut InputState::default(), &assets, &mut dev);
    assert!(engine.get_entity(ale_id).is_some_and(Entity::is_active));
}

#[test]
fn three_drops_at_same_position_merge_into_one_pile() {
    let (mut engine, pc_id, assets) = build_engine_with_pc(/* bow_ammo */ 10);

    drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);
    drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);
    drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);

    let bonuses = count_bonuses(&engine, Action::Bow);
    assert_eq!(
        bonuses.len(),
        1,
        "three same-position drops should leave one merged pile, got {bonuses:?}"
    );
    assert_eq!(bonuses[0].1, 3, "merged quantity");

    // last_dropped_ammo should point at the surviving pile.
    let pc = engine.get_entity(pc_id).unwrap();
    let pc_data = match pc {
        crate::element::Entity::Pc(p) => &p.pc,
        _ => unreachable!(),
    };
    assert_eq!(pc_data.last_dropped_ammo, Some(bonuses[0].0));
    assert_eq!(pc_data.last_ammo_dropping_position.x, 100.0);
}

#[test]
fn drop_over_pile_cap_spawns_fresh_and_bumps_facing() {
    let (mut engine, pc_id, assets) = build_engine_with_pc(20);

    // Fill a pile to the cap (5).
    for _ in 0..5 {
        drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);
    }
    let bonuses = count_bonuses(&engine, Action::Bow);
    assert_eq!(bonuses.len(), 1, "five drops merge into one pile");
    assert_eq!(bonuses[0].1, 5, "pile capped at 5");

    let dir_before = engine.get_entity(pc_id).unwrap().element_data().direction();

    // Sixth drop overflows the cap → new pile, facing rotates +1.
    drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);

    let bonuses = count_bonuses(&engine, Action::Bow);
    assert_eq!(
        bonuses.len(),
        2,
        "cap-overflow drop should spawn a fresh pile, got {bonuses:?}"
    );
    // The fresh pile is the one with quantity 1.
    let fresh_qty = bonuses.iter().find(|(_, q)| *q == 1).map(|(_, q)| *q);
    assert_eq!(fresh_qty, Some(1));

    let dir_after = engine.get_entity(pc_id).unwrap().element_data().direction();
    assert_eq!(
        dir_after,
        (dir_before + 1).rem_euclid(16),
        "PC facing should rotate +1 sector on cap overflow"
    );
}

#[test]
fn moving_between_drops_breaks_merge() {
    let (mut engine, pc_id, assets) = build_engine_with_pc(10);

    drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);

    // Teleport the PC sideways before the second drop — same as
    // walking off the original tile.
    if let Some(entity) = engine.world.entities.get_mut(pc_id) {
        entity
            .element_data_mut()
            .set_position_map(crate::coordinates::MapPoint { x: 200.0, y: 200.0 });
    }

    drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);

    let bonuses = count_bonuses(&engine, Action::Bow);
    assert_eq!(
        bonuses.len(),
        2,
        "moving between drops invalidates the merge gate, got {bonuses:?}"
    );
}
