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
        .launch_element(SequenceElement::new(0, Command::Wait, Some(attacker)));
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
            .actor_data()
            .unwrap()
            .active_melee,
        before
    );
}

#[test]
fn production_owner_obeys_execution_frozen_but_not_frozen_all() {
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

    let mut frozen_all_engine = EngineInner::new();
    let attacker = frozen_all_engine.add_entity(make_test_pc(Posture::Upright));
    let victim = frozen_all_engine.add_entity(make_test_pc(Posture::Upright));
    install_selected_melee(&mut frozen_all_engine, attacker, victim, 1, true);
    frozen_all_engine.set_actors_frozen(true);
    run_owner_walk(&mut frozen_all_engine, &LevelAssets::new());
    assert!(
        !frozen_all_engine
            .get_entity(attacker)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_melee
            .is_active(),
        "FrozenAll freezes sprite progression, not base Actor Execute effects"
    );
}
