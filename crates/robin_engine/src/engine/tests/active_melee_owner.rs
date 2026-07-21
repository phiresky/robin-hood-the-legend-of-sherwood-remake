use super::*;

use crate::element::{Command, Posture};
use crate::movement::{ActiveMelee, MELEE_HIT_FRAME, MELEE_STRIKE_DURATION};
use crate::order::{Order, OrderType};
use crate::sequence::SequenceElement;
use crate::weapons::SwordStrike;

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
    frames_remaining: u16,
    hit_applied: bool,
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
    engine.orders.sequence_manager.push_order_on(
        seq_id,
        0,
        Order::new(OrderType::StrikingStraightSword, 0.0, 0.0, order_id),
    );
    engine
        .orders
        .sequence_manager
        .element_in_progress(seq_id, 0);
    let mut melee = ActiveMelee::new(victim, SwordStrike::A, Some(seq_id), 0);
    melee.frames_remaining = frames_remaining;
    melee.hit_applied = hit_applied;
    melee.order_id = Some(order_id);
    let entity = engine.get_entity_mut(attacker).expect("attacker exists");
    entity.element_data_mut().active = true;
    entity
        .actor_data_mut()
        .expect("attacker has actor data")
        .active_melee = melee;
    seq_id
}

fn run_owner_walk(engine: &mut EngineInner, assets: &LevelAssets) {
    let positions = positions(engine);
    engine.tick_actor_owner_envelopes(&crate::sim_rng::test_context(), assets, &positions);
}

#[test]
fn production_owner_executes_selected_melee_once_and_closes_completion_before_tail() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    install_selected_melee(&mut engine, attacker, victim, 1, true);
    let assets = LevelAssets::new();
    let positions = positions(&engine);
    let mut observed_after_tail = false;
    engine.tick_actor_owner_envelopes_with_test_owner_hook(
        &crate::sim_rng::test_context(),
        &assets,
        &positions,
        |engine, owner| {
            if owner == attacker {
                observed_after_tail = true;
                assert!(
                    !engine
                        .get_entity(attacker)
                        .unwrap()
                        .actor_data()
                        .unwrap()
                        .active_melee
                        .is_active(),
                    "completion must close before the Human/PC tail returns"
                );
            }
        },
    );
    assert!(observed_after_tail);
}

#[test]
fn production_owner_rejects_latent_melee_under_higher_priority_current_arm() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    bind_animation(&mut engine, attacker, OrderType::WaitingUpright);
    let melee_sequence = install_selected_melee(
        &mut engine,
        attacker,
        victim,
        MELEE_STRIKE_DURATION - MELEE_HIT_FRAME,
        false,
    );
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
    let before = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_melee;
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
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_melee,
        before
    );
}

#[test]
fn production_owner_obeys_execution_frozen() {
    let mut frozen_actor_engine = EngineInner::new();
    let attacker = frozen_actor_engine.add_entity(make_test_pc(Posture::Upright));
    let victim = frozen_actor_engine.add_entity(make_test_pc(Posture::Upright));
    install_selected_melee(&mut frozen_actor_engine, attacker, victim, 1, true);
    frozen_actor_engine
        .get_entity_mut(attacker)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .execution_frozen = true;
    run_owner_walk(&mut frozen_actor_engine, &LevelAssets::new());
    assert!(
        frozen_actor_engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_melee
            .is_active()
    );
}

#[test]
fn frozen_all_bound_melee_animation_leaves_sprite_strike_and_order_untouched() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    bind_animation(&mut engine, attacker, OrderType::StrikingStraightSword);
    let sequence =
        install_selected_melee(&mut engine, attacker, victim, MELEE_STRIKE_DURATION, false);
    let before_melee = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_melee;
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
    assert_eq!(entity.actor_data().unwrap().active_melee, before_melee);
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
fn frozen_all_fallback_timer_does_not_hit_or_complete() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    let sequence = install_selected_melee(&mut engine, attacker, victim, 1, true);
    let before = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_melee;
    let before_state = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap()
        .state;
    engine.set_actors_frozen(true);
    run_owner_walk(&mut engine, &LevelAssets::new());
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_melee,
        before
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        before_state
    );
}

#[test]
fn selected_melee_start_is_not_double_advanced_by_generic_actor_execute() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    bind_animation(&mut engine, attacker, OrderType::StrikingStraightSword);
    install_selected_melee(&mut engine, attacker, victim, MELEE_STRIKE_DURATION, false);
    run_owner_walk(&mut engine, &LevelAssets::new());
    let entity = engine.get_entity(attacker).unwrap();
    assert_eq!(entity.element_data().sprite.current_frame, 0);
    assert_eq!(
        entity.actor_data().unwrap().action_state,
        crate::element::ActionState::WaitingSword,
        "WaitingSword belongs to the live MotionState::Start transition"
    );
    assert!(entity.actor_data().unwrap().active_melee.sprite_driving_hit);
}

#[test]
fn same_owner_replacement_after_selection_cancels_melee_execute_arm() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    bind_animation(&mut engine, attacker, OrderType::StrikingStraightSword);
    let melee_sequence =
        install_selected_melee(&mut engine, attacker, victim, MELEE_STRIKE_DURATION, false);
    let before = engine
        .get_entity(attacker)
        .unwrap()
        .actor_data()
        .unwrap()
        .active_melee;
    let assets = LevelAssets::new();
    engine.tick_actor_animation_action_change_slots_with_hooks(
        &crate::sim_rng::test_context(),
        &assets,
        |_, _| {},
        |_, _| {},
        |engine, owner, _, melee| {
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
        |_, _| {},
    );
    assert_eq!(
        engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_melee,
        before
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
