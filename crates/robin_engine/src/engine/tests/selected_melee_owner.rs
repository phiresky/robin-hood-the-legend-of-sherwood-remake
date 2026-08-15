use super::*;

use crate::coordinates::WorldPoint3D;
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

fn positions(
    engine: &EngineInner,
) -> crate::entities::EntitySlots<Option<crate::entities::BoundaryPosition>> {
    let mut positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    for (id, entity) in engine.world.entities.occupied() {
        positions[id] = Some(crate::entities::BoundaryPosition::of(entity.element_data()));
    }
    positions
}

fn bind_animation(engine: &mut EngineInner, actor: EntityId, action: OrderType) {
    bind_animations(engine, actor, &[action]);
}

fn bind_animations(engine: &mut EngineInner, actor: EntityId, actions: &[OrderType]) {
    let action = *actions.first().expect("test animation set is not empty");
    let mut conversion =
        vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
    for &mapped_action in actions {
        conversion[mapped_action as usize] = 0;
    }
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

fn install_selected_smalltalk(
    engine: &mut EngineInner,
    attacker: EntityId,
    victim: EntityId,
) -> crate::sequence::SequenceId {
    let seq_id = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(
            1,
            Command::SwordstrikeSmalltalkRight,
            Some(attacker),
        ));
    let order_id = engine.orders.allocate_order_id();
    let mut order = Order::new(OrderType::StrikingRightSmalltalk, 0.0, 0.0, order_id);
    order.antagonist = Some(victim);
    engine
        .orders
        .sequence_manager
        .push_order_on(seq_id, 0, order);
    engine
        .orders
        .sequence_manager
        .element_in_progress(seq_id, 0);
    engine
        .get_entity_mut(attacker)
        .expect("attacker exists")
        .element_data_mut()
        .active = true;
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
fn lateral_start_warns_in_original_actor_creation_order_before_rng() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let lower_slot_later = engine.add_entity(make_test_pc(Posture::Upright));
    let higher_slot_earlier = engine.add_entity(make_test_pc(Posture::Upright));
    let earlier_principal = engine.add_entity(make_test_pc(Posture::Upright));
    let later_principal = engine.add_entity(make_test_pc(Posture::Upright));

    bind_animation(&mut engine, attacker, OrderType::StrikingLeftSword);
    for victim in [lower_slot_later, higher_slot_earlier] {
        bind_animations(
            &mut engine,
            victim,
            &[
                OrderType::StrikingStraightSword,
                OrderType::StrikingStraightStrongSword,
                OrderType::ExecutingSword,
                OrderType::StrikingLeftSword,
                OrderType::StrikingRightSword,
                OrderType::StrikingSemiroundLeftSword,
                OrderType::StrikingSemiroundRightSword,
                OrderType::StrikingRoundLeftSword,
                OrderType::StrikingRoundRightSword,
                OrderType::TransitionWaitingSwordParryingSword,
            ],
        );
    }
    set_map_position(&mut engine, attacker, 0.0, 0.0);
    set_map_position(&mut engine, lower_slot_later, 0.0, 0.0);
    set_map_position(&mut engine, higher_slot_earlier, 0.0, 0.0);
    for victim in [lower_slot_later, higher_slot_earlier] {
        engine
            .get_entity_mut(victim)
            .unwrap()
            .element_data_mut()
            .active = true;
    }
    engine
        .get_entity_mut(higher_slot_earlier)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(earlier_principal);
    engine
        .get_entity_mut(lower_slot_later)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(later_principal);
    for principal in [earlier_principal, later_principal] {
        set_map_position(&mut engine, principal, 500.0, 500.0);
        engine
            .get_entity_mut(principal)
            .unwrap()
            .element_data_mut()
            .active = false;
    }

    // Rust allocated lower_slot_later first, but Original AddElement appended
    // higher_slot_earlier first. Inactive principals remain valid duel links
    // for the reactive proposal while staying out of the lateral warning arc.
    engine.world.install_original_creation_orders(
        std::collections::BTreeMap::from([
            (attacker, 100),
            (higher_slot_earlier, 101),
            (earlier_principal, 102),
            (later_principal, 103),
            (lower_slot_later, 104),
        ]),
        105,
    );

    let sequence = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(
            1,
            Command::SwordstrikeThrustD,
            Some(attacker),
        ));
    let order_id = engine.orders.allocate_order_id();
    let mut order = Order::new(OrderType::StrikingLeftSword, 0.0, 0.0, order_id);
    order.antagonist = Some(higher_slot_earlier);
    engine
        .orders
        .sequence_manager
        .push_order_on(sequence, 0, order);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .active = true;

    let mut assets = straight_warning_assets(0, 100);
    let thrust = &mut std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].thrusts
        [SwordStrike::D as usize];
    thrust.kind = crate::profiles::WeaponThrustKind::Lateral;
    thrust.direction = crate::profiles::WeaponThrustDirection::LeftToRight;
    thrust.initial_angle = 45;
    thrust.final_angle = 90;

    let (((), rng_trace), warnings) = super::super::melee::capture_strike_warnings(|| {
        crate::sim_rng::with_draw_trace(|| run_owner_walk(&mut engine, &assets))
    });
    assert_eq!(
        warnings,
        vec![
            (attacker, higher_slot_earlier),
            (attacker, lower_slot_later),
        ],
        "WarnForStrike callbacks must follow marrayActors/AddElement order, not PC slot order"
    );
    assert_eq!(
        rng_trace
            .iter()
            .filter(|&&site| site == crate::sim_rng::RngSite::SwordStrikeSelection)
            .count(),
        2,
        "both ordered PC callbacks must reach the RNG-sensitive reactive proposal: {rng_trace:?}"
    );
}

#[test]
fn straight_start_warns_principal_without_common_victim_filter() {
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

    // GetPossibleVictimsOfStraightSwordStrike is deliberately unlike the
    // other strike collectors: Original considers the principal opponent and
    // distance only. In particular it does not call
    // IsPossibleSwordStrikeVictim, whose first state guard rejects inactive
    // actors. WarnForStrike applies its own downstream state rules.
    engine
        .get_entity_mut(principal)
        .unwrap()
        .element_data_mut()
        .active = false;
    bind_animation(&mut engine, attacker, OrderType::StrikingStraightSword);
    install_selected_melee(&mut engine, attacker, principal);

    let (_, warnings) = super::super::melee::capture_strike_warnings(|| {
        run_owner_walk(&mut engine, &straight_warning_assets(10, 50))
    });
    assert_eq!(warnings, vec![(attacker, principal)]);
}

#[test]
fn straight_done_hits_principal_without_common_victim_filter() {
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
    engine
        .get_entity_mut(principal)
        .unwrap()
        .element_data_mut()
        .active = false;
    bind_animation(&mut engine, attacker, OrderType::StrikingStraightSword);
    install_selected_melee(&mut engine, attacker, principal);

    let assets = straight_warning_assets(10, 50);
    for _ in 0..4 {
        run_owner_walk(&mut engine, &assets);
    }

    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(principal)
            }),
        "GetPossibleVictimsOfStraightSwordStrike must use principal + distance only for the completed hit as well as its warning"
    );
}

#[test]
fn smalltalk_done_uses_isometric_facing_for_back_hit_gate() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    set_map_position(&mut engine, attacker, 0.0, 0.0);
    // For sector 2 and this relative position, a raw unit-circle dot
    // product is negative while Original's aspect-scaled map-plane vector
    // is positive.
    set_map_position(&mut engine, victim, 60.0, 100.0);
    {
        let victim = engine.get_entity_mut(victim).unwrap();
        victim.element_data_mut().set_direction_instantly(2);
        victim.actor_data_mut().unwrap().action_state = crate::element::ActionState::WaitingSword;
    }
    bind_animation(&mut engine, attacker, OrderType::StrikingRightSmalltalk);
    install_selected_smalltalk(&mut engine, attacker, victim);

    let assets = straight_warning_assets(0, 1);
    for _ in 0..4 {
        run_owner_walk(&mut engine, &assets);
    }

    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
            }),
        "smalltalk back-hit must use the Original isometric facing vector"
    );
}

#[test]
fn push_start_uses_original_aspect_scaled_rectangle() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    set_map_position(&mut engine, attacker, 0.0, 0.0);
    // This is the same side-boundary geometry as the completed-hit
    // regression below: an unscaled unit-circle vector rejects the victim,
    // while Original's ASPECT_RATIO-scaled direction admits it.
    set_map_position(&mut engine, victim, 37.676_39, -3.109_62);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .active = true;
    bind_animation(&mut engine, attacker, OrderType::StrikingStraightSword);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(3);
    install_selected_melee(&mut engine, attacker, victim);

    let mut assets = straight_warning_assets(0, 45);
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    let thrust = &mut profiles.hth_weapons[0].thrusts[SwordStrike::A as usize];
    thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
    thrust.repulsion = 20;

    let (_, warnings) =
        super::super::melee::capture_strike_warnings(|| run_owner_walk(&mut engine, &assets));

    assert_eq!(warnings, vec![(attacker, victim)]);
    assert!(
        !engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
            }),
        "strike START must warn the admitted push victim without applying DONE damage"
    );
}

#[test]
fn push_done_uses_original_aspect_scaled_rectangle() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    set_map_position(&mut engine, attacker, 0.0, 0.0);
    // At sector 3 the unscaled unit-circle vector puts this actor about
    // 11.55 units from the strike axis and rejects it. Original's
    // GetDirectionVector applies ASPECT_RATIO first, yielding about 5.67 and
    // admitting it inside this profile's 10-unit half-width.
    set_map_position(&mut engine, victim, 37.676_39, -3.109_62);
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .active = true;
    bind_animation(&mut engine, attacker, OrderType::StrikingStraightSword);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(3);
    install_selected_melee(&mut engine, attacker, victim);

    let mut assets = straight_warning_assets(0, 45);
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    let thrust = &mut profiles.hth_weapons[0].thrusts[SwordStrike::A as usize];
    thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
    thrust.repulsion = 20;
    for _ in 0..4 {
        run_owner_walk(&mut engine, &assets);
    }

    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
            }),
        "push rectangle must use Original's ASPECT_RATIO-scaled facing vector"
    );
}

#[test]
fn push_done_uses_ground_positions_but_warning_keeps_map_positions() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    bind_animation(&mut engine, attacker, OrderType::StrikingStraightSword);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .position_iface_mut()
        .set_position(WorldPoint3D::new(0.0, 50.0, 0.0));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .position_iface_mut()
        .set_position(WorldPoint3D::new(0.0, 5.0, 2.0));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .active = true;
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(0);
    install_selected_melee(&mut engine, attacker, victim);

    let mut assets = straight_warning_assets(0, 45);
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    let thrust = &mut profiles.hth_weapons[0].thrusts[SwordStrike::A as usize];
    thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
    thrust.repulsion = 20;

    let (_, warnings) =
        super::super::melee::capture_strike_warnings(|| run_owner_walk(&mut engine, &assets));
    assert!(
        warnings.is_empty(),
        "the START warning uses map Y, where elevation makes the front distance 47 > 45"
    );
    for _ in 0..3 {
        run_owner_walk(&mut engine, &assets);
    }
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
            }),
        "the DONE effect uses ground Y, where the front distance is exactly 45"
    );
}

#[test]
fn push_done_flat_positions_match_warning_geometry() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    bind_animation(&mut engine, attacker, OrderType::StrikingStraightSword);
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .position_iface_mut()
        .set_position(WorldPoint3D::new(0.0, 50.0, 0.0));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .position_iface_mut()
        .set_position(WorldPoint3D::new(0.0, 5.0, 0.0));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .active = true;
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(0);
    install_selected_melee(&mut engine, attacker, victim);

    let mut assets = straight_warning_assets(0, 45);
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    let thrust = &mut profiles.hth_weapons[0].thrusts[SwordStrike::A as usize];
    thrust.kind = crate::profiles::WeaponThrustKind::PushAside;
    thrust.repulsion = 20;

    let (_, warnings) =
        super::super::melee::capture_strike_warnings(|| run_owner_walk(&mut engine, &assets));
    assert_eq!(warnings, vec![(attacker, victim)]);
    for _ in 0..3 {
        run_owner_walk(&mut engine, &assets);
    }
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
            }),
        "flat map and ground positions agree at the inclusive maximum range"
    );
}

#[test]
fn smalltalk_done_uses_ground_positions_for_back_hit_gate() {
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Upright));
    engine
        .get_entity_mut(attacker)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(0.0, 0.0, 0.0));
    engine
        .get_entity_mut(victim)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(1.0, 0.0, 10.0));
    {
        let victim = engine.get_entity_mut(victim).unwrap();
        victim.element_data_mut().set_direction_instantly(13);
        victim.actor_data_mut().unwrap().action_state = crate::element::ActionState::WaitingSword;
    }

    let [dx, dy] = crate::position_interface::sector_to_vector_iso(13);
    let attacker_entity = engine.get_entity(attacker).unwrap();
    let victim_entity = engine.get_entity(victim).unwrap();
    let attacker_map = attacker_entity.element_data().position_map();
    let victim_map = victim_entity.element_data().position_map();
    let attacker_ground = attacker_entity.ground_position();
    let victim_ground = victim_entity.ground_position();
    assert!(
        dx * (victim_map.x - attacker_map.x) + dy * (victim_map.y - attacker_map.y) > 0.0,
        "the projected-map test must classify this setup as a back hit"
    );
    assert!(
        dx * (victim_ground.x - attacker_ground.x) + dy * (victim_ground.y - attacker_ground.y)
            < 0.0,
        "Original's ground-position test must classify this setup as a miss"
    );

    bind_animation(&mut engine, attacker, OrderType::StrikingRightSmalltalk);
    install_selected_smalltalk(&mut engine, attacker, victim);

    let assets = straight_warning_assets(0, 1);
    for _ in 0..4 {
        run_owner_walk(&mut engine, &assets);
    }

    assert!(
        !engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.command == Command::ReceiveSwordDamage && element.owner == Some(victim)
            }),
        "smalltalk back-hit must use Original GetPositionGround coordinates"
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

/// Build a Lacklandist enemy soldier for the learning-by-looking tests.
fn learning_test_soldier(engine: &mut EngineInner) -> EntityId {
    let mut soldier = make_test_soldier(Posture::Upright);
    let Entity::Soldier(s) = &mut soldier else {
        unreachable!("make_test_soldier returned a non-soldier")
    };
    s.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
    s.soldier.cached_camp = crate::element::Camp::Lacklandists;
    s.soldier.soldier_profile_index = crate::profiles::SoldierProfileIdx(0);
    let id = engine.add_entity(soldier);
    let entity = engine.get_entity_mut(id).expect("test soldier exists");
    let Entity::Soldier(s) = entity else {
        unreachable!()
    };
    let enemy = s
        .npc
        .ai_brain
        .enemy_mut()
        .expect("test soldier has enemy AI");
    enemy.base.current_state = crate::ai::AiState::Attacking;
    id
}

fn learning_test_assets(raw_fighting: u16) -> LevelAssets {
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.soldiers.push(crate::profiles::SoldierProfile {
        fighting: raw_fighting,
        ..Default::default()
    });
    LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    }
}

/// Original's `MakeBadSwordstrikeExperience` dispatch filters friends with the
/// virtual `GetFightingAbility()`, which applies the difficulty modifier for
/// Lacklandist soldiers. A raw capacity of 40 doubles to 80 on Hard and must
/// pass the `MIN_CAPACITY_LEARNING_BY_LOOKING` (70) gate.
#[test]
fn learning_by_looking_uses_difficulty_modified_fighting_ability() {
    let assets = learning_test_assets(40);
    let mut engine = EngineInner::new();
    engine.control.sim_config.difficulty = crate::player_profile::DifficultyLevel::Hard;
    let learner = learning_test_soldier(&mut engine);
    let friend = learning_test_soldier(&mut engine);
    set_map_position(&mut engine, learner, 100.0, 100.0);
    set_map_position(&mut engine, friend, 150.0, 100.0);

    engine.make_bad_sword_strike_experience(&assets, learner, SwordStrike::H, true);

    let friend_known = engine
        .get_entity(friend)
        .unwrap()
        .enemy_ai()
        .unwrap()
        .known_enemy_strike_1;
    assert_eq!(
        friend_known,
        Some(SwordStrike::H),
        "Hard difficulty doubles a Lacklandist's fighting ability (40 -> 80), so the \
         friend must learn the circular strike by looking"
    );
}

/// On Medium the raw capacity stays 40 (< 70), so the same friend must NOT
/// learn by looking.
#[test]
fn learning_by_looking_respects_unmodified_ability_on_medium() {
    let assets = learning_test_assets(40);
    let mut engine = EngineInner::new();
    engine.control.sim_config.difficulty = crate::player_profile::DifficultyLevel::Medium;
    let learner = learning_test_soldier(&mut engine);
    let friend = learning_test_soldier(&mut engine);
    set_map_position(&mut engine, learner, 100.0, 100.0);
    set_map_position(&mut engine, friend, 150.0, 100.0);

    engine.make_bad_sword_strike_experience(&assets, learner, SwordStrike::H, true);

    let friend_known = engine
        .get_entity(friend)
        .unwrap()
        .enemy_ai()
        .unwrap()
        .known_enemy_strike_1;
    assert_eq!(
        friend_known, None,
        "a raw fighting ability of 40 stays below the learning-by-looking gate on Medium"
    );
}
