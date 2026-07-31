use super::*;

use crate::element::{Command, Posture};
use crate::order::{Order, OrderType};
use crate::sequence::SequenceElement;
use crate::weapons::SwordStrike;

fn straight_warning_assets(min_distance: u16, max_distance: u16) -> LevelAssets {
    let mut profiles = crate::profiles::ProfileManager::new();
    let mut weapon = crate::profiles::HtHWeaponProfile::default();
    let thrust = &mut weapon.thrusts[SwordStrike::A as usize];
    thrust.kind = crate::profiles::WeaponThrustKind::Straight;
    thrust.minimal_distance = min_distance;
    thrust.maximal_distance = max_distance;
    profiles.hth_weapons.push(weapon);
    profiles.characters.push(crate::profiles::CharacterProfile {
        hth_weapon_id: 1,
        fighting: 100,
        ..crate::profiles::CharacterProfile::default()
    });
    LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    }
}

fn set_map_position(engine: &mut EngineInner, actor: EntityId, x: f32, y: f32) {
    engine
        .get_entity_mut(actor)
        .expect("test actor exists")
        .element_data_mut()
        .set_position_map(MapPoint::new(x, y));
}

fn positions(engine: &EngineInner) -> crate::entities::EntitySlots<Option<MapPoint>> {
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for (id, entity) in engine.world.entities.occupied() {
        positions[id] = Some(entity.element_data().position_map());
    }
    positions
}

fn bind_animation(engine: &mut EngineInner, actor: EntityId, action: OrderType) {
    let mut conversion =
        vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
    conversion[action as usize] = 0;
    let script = crate::sprite_script::SpriteScript {
        action_id: action as u16,
        action_done: 1,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0, 0, 0],
    };
    engine
        .get_entity_mut(actor)
        .unwrap()
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
}

fn install_selected_melee(
    engine: &mut EngineInner,
    attacker: EntityId,
    victim: EntityId,
) -> crate::sequence::SequenceId {
    let seq_id = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(
            1,
            Command::SwordstrikeThrustA,
            Some(attacker),
        ));
    let order_id = engine.orders.allocate_order_id();
    let mut order = Order::new(OrderType::StrikingStraightSword, 0.0, 0.0, order_id);
    order.antagonist = Some(victim);
    engine
        .orders
        .sequence_manager
        .push_order_on(seq_id, 0, order);
    engine
        .orders
        .sequence_manager
        .element_in_progress(seq_id, 0);
    let entity = engine.get_entity_mut(attacker).expect("attacker exists");
    entity.element_data_mut().active = true;
    seq_id
}

fn run_owner_walk(engine: &mut EngineInner, assets: &LevelAssets) {
    let positions = positions(engine);
    engine.tick_actor_owner_envelopes(&crate::sim_rng::test_context(), assets, &positions);
}

#[test]
fn production_owner_rejects_latent_melee_under_higher_priority_current_arm() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    bind_animation(&mut engine, attacker, OrderType::WaitingUpright);
    let melee_sequence = install_selected_melee(&mut engine, attacker, victim);
    engine.orders.sequence_manager.element_interrupted(
        melee_sequence,
        0,
        crate::sequence::CascadeFlags::empty(),
    );
    let interrupt = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(0, Command::PlayAnim, Some(attacker)));
    let order_id = engine.orders.allocate_order_id();
    engine.orders.sequence_manager.push_order_on(
        interrupt,
        0,
        Order::new(OrderType::WaitingUpright, 0.0, 0.0, order_id),
    );
    engine
        .orders
        .sequence_manager
        .element_in_progress(interrupt, 0);
    run_owner_walk(&mut engine, &LevelAssets::new());
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .sprite
            .last_processed_order_id,
        order_id.get(),
        "latent melee must not suppress the actual selected generic Execute arm"
    );
}

#[test]
fn production_owner_obeys_execution_frozen() {
    let mut frozen_actor_engine = EngineInner::new();
    let attacker = frozen_actor_engine.add_entity(make_test_pc(Posture::Upright));
    let victim = frozen_actor_engine.add_entity(make_test_pc(Posture::Upright));
    bind_animation(
        &mut frozen_actor_engine,
        attacker,
        OrderType::StrikingStraightSword,
    );
    let sequence = install_selected_melee(&mut frozen_actor_engine, attacker, victim);
    frozen_actor_engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .execution_frozen = true;
    run_owner_walk(&mut frozen_actor_engine, &LevelAssets::new());
    assert_eq!(
        frozen_actor_engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::InProgress
    );
}

#[test]
fn frozen_all_bound_melee_animation_leaves_sprite_strike_and_order_untouched() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    set_map_position(&mut engine, attacker, 0.0, 0.0);
    set_map_position(&mut engine, victim, 40.0, 0.0);
    bind_animation(&mut engine, attacker, OrderType::StrikingStraightSword);
    let sequence = install_selected_melee(&mut engine, attacker, victim);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(8);
    let before_action_state = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .action_state;
    let before_sprite = engine
        .get_entity(attacker)
        .unwrap()
        .element_data()
        .sprite
        .clone();
    let before_element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap()
        .clone();
    engine.set_actors_frozen(true);
    let (_, rng_trace) =
        crate::sim_rng::with_draw_trace(|| run_owner_walk(&mut engine, &LevelAssets::new()));
    let entity = engine.get_entity(attacker).unwrap();
    let target_direction = crate::position_interface::vector_to_sector_0_to_15(40.0, 0.0);
    assert_eq!(
        i16::from(entity.position_iface().get_direction_goal()),
        target_direction
    );
    assert_eq!(
        entity.element_data().direction(),
        7,
        "FrozenAll preserves SetDirection(goal) plus exactly one Turn before the sprite boundary"
    );
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        before_action_state
    );
    assert!(
        rng_trace.is_empty(),
        "FrozenAll must not run strike-start WarnForStrike RNG: {rng_trace:?}"
    );
    assert!(entity.actor_data().unwrap().sweep_state.is_none());
    assert_eq!(
        entity.element_data().sprite.current_frame,
        before_sprite.current_frame
    );
    assert_eq!(
        entity.element_data().sprite.last_processed_order_id,
        before_sprite.last_processed_order_id
    );
    let after_element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap();
    assert_eq!(after_element.state, before_element.state);
    assert_eq!(after_element.priority, before_element.priority);
    assert_eq!(after_element.orders.len(), before_element.orders.len());
    assert_eq!(
        after_element.orders.front().unwrap().order_id,
        before_element.orders.front().unwrap().order_id
    );
}

#[test]
fn selected_melee_start_is_not_double_advanced_by_generic_actor_execute() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    bind_animation(&mut engine, attacker, OrderType::StrikingStraightSword);
    install_selected_melee(&mut engine, attacker, victim);
    run_owner_walk(&mut engine, &straight_warning_assets(0, 100));
    let entity = engine.get_entity(attacker).unwrap();
    assert_eq!(entity.element_data().sprite.current_frame, 0);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        crate::element::ActionState::WaitingSword,
        "WaitingSword belongs to the live MotionState::Start transition"
    );
}

#[test]
fn straight_start_does_not_warn_or_draw_for_out_of_range_or_nonprincipal_target() {
    for case in ["out_of_range", "nonprincipal"] {
        let mut engine = EngineInner::new();
        let attacker = engine.add_entity(make_test_pc(Posture::Upright));
        let principal = engine.add_entity(make_test_pc(Posture::Upright));
        let nominal_target = if case == "nonprincipal" {
            engine.add_entity(make_test_pc(Posture::Upright))
        } else {
            principal
        };
        set_map_position(&mut engine, attacker, 0.0, 0.0);
        set_map_position(&mut engine, principal, 200.0, 0.0);
        if nominal_target != principal {
            set_map_position(&mut engine, nominal_target, 20.0, 0.0);
        }
        engine
            .get_entity_mut(attacker)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents
            .push(principal);
        bind_animation(&mut engine, attacker, OrderType::StrikingStraightSword);
        install_selected_melee(&mut engine, attacker, nominal_target);
        let assets = straight_warning_assets(10, 50);
        let (((), rng_trace), warnings) = super::super::melee::capture_strike_warnings(|| {
            crate::sim_rng::with_draw_trace(|| run_owner_walk(&mut engine, &assets))
        });
        assert!(warnings.is_empty(), "{case} warning: {warnings:?}");
        assert!(rng_trace.is_empty(), "{case} RNG: {rng_trace:?}");
    }
}

#[test]
fn eligible_principal_is_warned_once_on_start_and_not_again_in_progress() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let principal = engine.add_entity(make_test_pc(Posture::Upright));
    set_map_position(&mut engine, attacker, 0.0, 0.0);
    set_map_position(&mut engine, principal, 20.0, 0.0);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(principal);
    bind_animation(&mut engine, attacker, OrderType::StrikingStraightSword);
    install_selected_melee(&mut engine, attacker, principal);
    let assets = straight_warning_assets(10, 50);

    let (_, start_warnings) =
        super::super::melee::capture_strike_warnings(|| run_owner_walk(&mut engine, &assets));
    assert_eq!(start_warnings, vec![(attacker, principal)]);

    let (_, in_progress_warnings) =
        super::super::melee::capture_strike_warnings(|| run_owner_walk(&mut engine, &assets));
    assert!(
        in_progress_warnings.is_empty(),
        "WarnForStrike must not repeat after MotionState::Start: {in_progress_warnings:?}"
    );
}

#[test]
fn same_owner_replacement_after_selection_cancels_melee_execute_arm() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    bind_animation(&mut engine, attacker, OrderType::StrikingStraightSword);
    let melee_sequence = install_selected_melee(&mut engine, attacker, victim);
    let assets = LevelAssets::new();
    engine.tick_actor_animation_action_change_slots_with_hooks(
        &crate::sim_rng::test_context(),
        &assets,
        |_, _| {},
        |_, _| {},
        |engine, owner, _, melee, _, _, _| {
            if owner != attacker {
                return;
            }
            let selected = melee.expect("melee was selected at Execute entry");
            engine.orders.sequence_manager.element_interrupted(
                melee_sequence,
                0,
                crate::sequence::CascadeFlags::empty(),
            );
            let replacement = engine
                .orders
                .sequence_manager
                .launch_element(SequenceElement::new(1, Command::Wait, Some(owner)));
            let order_id = engine.orders.allocate_order_id();
            engine.orders.sequence_manager.push_order_on(
                replacement,
                0,
                Order::new(OrderType::WaitingUpright, 0.0, 0.0, order_id),
            );
            engine
                .orders
                .sequence_manager
                .element_in_progress(replacement, 0);
            engine.tick_selected_melee_owner(
                &crate::sim_rng::test_context(),
                &assets,
                owner,
                selected,
            );
        },
        |_, _, _| {},
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .element_data()
            .sprite
            .current_frame,
        0
    );
}
