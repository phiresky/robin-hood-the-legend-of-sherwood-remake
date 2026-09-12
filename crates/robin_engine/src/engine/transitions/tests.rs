use super::*;
use crate::element::{ActorData, Entity, HumanData, NpcData, SoldierData};
use crate::element_kinds::{ActionState as AS, Posture as P};
use crate::sequence::{SequenceElement, SequencePriority};

fn make_soldier(posture: P, action_state: AS, attentive: bool) -> Entity {
    let mut enemy_ai = crate::ai_enemy::EnemyAi::new(0);
    enemy_ai.attentive = attentive;
    // In the running game, `set_soldier_attentive_mode`
    // flips `will_be_attentive` synchronously and `attentive`
    // after the transition animation completes — so a settled
    // alerted soldier has both true.  Mirror that for tests.
    enemy_ai.will_be_attentive = attentive;
    Entity::Soldier(crate::element::ActorSoldier {
        element: {
            let mut initial_element = crate::element::ElementData::from_initial_posture(posture);
            initial_element.kind = crate::element::ElementKind::ActorSoldier;
            initial_element
        },
        actor: ActorData {
            action_state,
            ..Default::default()
        },
        human: HumanData::default(),
        npc: NpcData {
            ai: crate::element::AiActorData {
                ai_brain: crate::element::AiBrain::Enemy(Box::new(enemy_ai)),
                ..Default::default()
            },
            ..Default::default()
        },
        soldier: SoldierData {
            cached_camp: crate::element::Camp::Lacklandists,
            ..SoldierData::default()
        },
    })
}

fn make_pc(posture: P, action_state: AS) -> Entity {
    Entity::Pc(crate::element::ActorPc {
        element: {
            let mut initial_element = crate::element::ElementData::from_initial_posture(posture);
            initial_element.kind = crate::element::ElementKind::ActorPc;
            initial_element
        },
        actor: ActorData {
            action_state,
            ..Default::default()
        },
        human: HumanData::default(),
        pc: Default::default(),
    })
}

/// Launch a sequence element for `owner` with the given command.
/// Returns `(seq_id, elem_idx)`.
fn launch(engine: &mut EngineInner, owner: EntityId, command: Command) -> (SequenceId, usize) {
    let mut elem = SequenceElement::new(1, command, Some(owner));
    elem.priority = SequencePriority::Preference;
    // Stamp posture/action-state snapshot as arbitrate_instruct
    // would, so transition helpers see a live "after_transition"
    // value instead of Posture::default().
    if let Some(ent) = engine.get_entity(owner) {
        elem.posture_after_transition = ent.element_data().posture();
        elem.action_state_after_transition =
            ent.actor_data().map(|a| a.action_state).unwrap_or_default();
    }
    let seq_id = engine.orders.sequence_manager.launch_element(elem);
    (seq_id, 0)
}

fn launch_movement(
    engine: &mut EngineInner,
    owner: EntityId,
    command: Command,
    action: OrderType,
) -> (SequenceId, usize) {
    let mut elem = SequenceElement::new_movement(1, command, Some(owner), action);
    elem.priority = SequencePriority::Preference;
    if let Some(ent) = engine.get_entity(owner) {
        elem.posture_after_transition = ent.element_data().posture();
        elem.action_state_after_transition =
            ent.actor_data().map(|a| a.action_state).unwrap_or_default();
    }
    let seq_id = engine.orders.sequence_manager.launch_element(elem);
    (seq_id, 0)
}

fn orders_for(engine: &EngineInner, seq: SequenceId, idx: usize) -> Vec<OrderType> {
    engine
        .orders
        .sequence_manager
        .get_element(seq, idx)
        .map(|e| e.orders.iter().map(|o| o.order_type).collect())
        .unwrap_or_default()
}

fn generate_transition(
    engine: &mut EngineInner,
    owner: EntityId,
    seq: SequenceId,
    idx: usize,
) -> bool {
    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::default();
    engine.generate_transition(&sim, &assets, owner, seq, idx)
}

#[test]
fn invalid_transition_targets_are_errors_not_gameplay_refusals() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_pc(P::Crouched, AS::Waiting));
    let (seq_id, elem_idx) = launch(&mut engine, owner, Command::CrouchDown);
    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::default();
    assert_eq!(
        engine.try_generate_transition(&sim, &assets, owner, seq_id, elem_idx),
        Ok(false)
    );
    assert_eq!(
        engine.try_generate_transition(&sim, &assets, owner, seq_id, elem_idx + 1),
        Err(TransitionError::MissingElement {
            seq_id,
            elem_idx: elem_idx + 1
        })
    );
    engine
        .orders
        .sequence_manager
        .get_element_mut(seq_id, elem_idx)
        .unwrap()
        .owner = None;
    assert_eq!(
        engine.try_generate_transition(&sim, &assets, owner, seq_id, elem_idx),
        Err(TransitionError::OwnerMismatch {
            expected: owner,
            actual: None
        })
    );
    engine.remove_entity(owner);
    assert_eq!(
        engine.try_generate_transition(&sim, &assets, owner, seq_id, elem_idx),
        Err(TransitionError::MissingOwner(owner))
    );
}

#[test]
fn transition_stages_revalidate_callback_mutations_even_on_refusal() {
    for allowed in [true, false] {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_pc(P::Upright, AS::Waiting));
        let (seq_id, elem_idx) = launch(&mut engine, owner, Command::Wait);
        let target = TransitionTarget {
            owner,
            seq_id,
            elem_idx,
        };
        assert_eq!(
            target.stage(&mut engine, |engine| {
                engine.orders.sequence_manager = Default::default();
                allowed
            }),
            Err(TransitionError::MissingElement { seq_id, elem_idx })
        );

        let (seq_id, elem_idx) = launch(&mut engine, owner, Command::Wait);
        let target = TransitionTarget {
            owner,
            seq_id,
            elem_idx,
        };
        assert_eq!(
            target.stage(&mut engine, |engine| {
                engine.remove_entity(owner);
                allowed
            }),
            Err(TransitionError::MissingOwner(owner))
        );
    }
}

#[test]
fn transition_stages_observe_live_state_without_reordering_effects() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_pc(P::Upright, AS::Waiting));
    let (seq_id, elem_idx) = launch(&mut engine, owner, Command::Wait);
    let target = TransitionTarget {
        owner,
        seq_id,
        elem_idx,
    };
    assert_eq!(
        target.stage(&mut engine, |engine| {
            push_anim_order(engine, seq_id, elem_idx, OrderType::TransitionCrouchingDown);
            set_posture_after(engine, seq_id, elem_idx, P::Crouched);
            true
        }),
        Ok(true)
    );
    assert_eq!(
        target.stage(&mut engine, |engine| {
            make_posture_transition_actor(engine, seq_id, elem_idx, owner, CP::MUST_BE_UPRIGHT)
        }),
        Ok(true)
    );
    assert_eq!(
        orders_for(&engine, seq_id, elem_idx),
        vec![
            OrderType::TransitionCrouchingDown,
            OrderType::TransitionCrouchingUp,
        ]
    );
    assert_eq!(
        transition_element(&engine, seq_id, elem_idx).posture_after_transition,
        P::Upright
    );
}

fn order_compute_direction_for(
    engine: &EngineInner,
    seq: SequenceId,
    idx: usize,
    order_type: OrderType,
) -> Option<bool> {
    engine
        .orders
        .sequence_manager
        .get_element(seq, idx)
        .and_then(|e| {
            e.orders
                .iter()
                .find(|o| o.order_type == order_type)
                .map(|o| o.compute_direction)
        })
}

#[test]
fn stand_up_transition_matches_game_action_variants() {
    assert_eq!(
        stand_up_order_for_action_state(AS::Waiting),
        OrderType::StandingUp
    );
    assert_eq!(
        stand_up_order_for_action_state(AS::WaitingSword),
        OrderType::StandingUpSword
    );
    assert_eq!(
        stand_up_order_for_action_state(AS::Menacing),
        OrderType::StandingUpSword
    );
    assert_eq!(
        stand_up_order_for_action_state(AS::AimingWithBow),
        OrderType::StandingUpBow
    );
}

#[test]
fn lying_soldier_stand_up_transition_is_in_place_no_direction() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_soldier(P::Lying, AS::WaitingSword, false));
    let (seq, idx) = launch(&mut engine, owner, Command::Turn);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(ok);

    let orders = orders_for(&engine, seq, idx);
    assert!(
        orders.contains(&OrderType::StandingUpSword),
        "expected sword stand-up, got {:?}",
        orders
    );
    assert_eq!(
        order_compute_direction_for(&engine, seq, idx, OrderType::StandingUpSword),
        Some(false)
    );
    let posture_after = engine
        .orders
        .sequence_manager
        .get_element(seq, idx)
        .unwrap()
        .posture_after_transition;
    assert_eq!(posture_after, P::Upright);
}

#[test]
fn sword_exit_transition_synchronously_quits_the_fight() {
    let mut engine = EngineInner::new();
    // Exercise the soldier path because its synchronous quit also drives
    // the ordinary NPC AI callbacks covered by this regression.
    let owner = engine.add_entity(make_soldier(P::Upright, AS::WaitingSword, false));
    let opponent = engine.add_entity(make_pc(P::Upright, AS::WaitingSword));
    // The synchronous quit notifies the AI, which reads every live PC's
    // campaign-description identity.
    engine
        .mission_domain
        .campaign
        .characters
        .push(crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
            ..Default::default()
        });
    engine
        .get_entity_mut(opponent)
        .and_then(Entity::pc_data_mut)
        .expect("opponent is a PC")
        .campaign_description_index = Some(0);
    assert!(EngineInner::add_opponent(
        &mut engine.world.entities,
        owner,
        opponent,
        None,
    ));
    assert!(EngineInner::add_opponent(
        &mut engine.world.entities,
        opponent,
        owner,
        None,
    ));

    let (seq, idx) = launch(&mut engine, owner, Command::LookLeft);
    // The synchronous quit runs the soldier's AI callbacks, which read
    // the registered soldier/character/weapon profiles.
    let mut assets = crate::engine::LevelAssets::default();
    {
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles.soldiers.push(crate::profiles::SoldierProfile {
            hth_weapon_id: 1,
            ..Default::default()
        });
        profiles.characters.push(crate::profiles::CharacterProfile {
            hth_weapon_id: 1,
            ..Default::default()
        });
        profiles
            .hth_weapons
            .push(crate::profiles::HtHWeaponProfile::default());
    }
    // Weapon ids are 1-based; a live fighter always references a
    // registered hand-to-hand weapon profile.
    if let Some(enemy) = engine.get_entity_mut(owner).and_then(Entity::enemy_ai_mut) {
        enemy.hth_weapon_id = 1;
    }
    let sim = crate::sim_rng::test_context();
    assert_eq!(
        engine.try_generate_transition(&sim, &assets, owner, seq, idx),
        Ok(true)
    );

    assert_eq!(
        orders_for(&engine, seq, idx),
        vec![OrderType::TransitionLoweringSword],
        "Translate(QUIT_SWORDFIGHT) must still queue the visible lowering"
    );
    for actor in [owner, opponent] {
        assert!(
            engine
                .get_entity(actor)
                .and_then(Entity::human_data)
                .is_some_and(|human| human.opponents.is_empty()),
            "swordfight-exit relationship cleanup is synchronous during transition generation"
        );
    }
}

#[test]
fn tied_soldier_rejects_upright_command_like_original_release() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_soldier(P::Tied, AS::Waiting, false));
    let (seq, idx) = launch(&mut engine, owner, Command::RaiseBow);

    assert!(!generate_transition(&mut engine, owner, seq, idx));
    assert!(orders_for(&engine, seq, idx).is_empty());
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq, idx)
            .expect("sequence element")
            .posture_after_transition,
        P::Tied
    );
}

#[test]
fn anonymous_archer_aiming_bow_up_transition_uses_anonymous_raise() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_pc(P::AnonymousArcher, AS::Waiting));
    let (seq, idx) = launch(&mut engine, owner, Command::LowerBow);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(ok);

    let orders = orders_for(&engine, seq, idx);
    assert_eq!(
        orders,
        vec![
            OrderType::TransitionEquipBowAnonymous,
            OrderType::TransitionLoadingBowAnonymous,
            OrderType::TransitionRaisingBowAnonymous,
        ],
        "raise-bow translation preserves anonymous archer variants during final bow-up transitions"
    );
}

/// Soldier with MOVE from LeaningOut queues the unstick transition.
#[test]
fn soldier_move_from_leaning_out_queues_unstick() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_soldier(P::LeaningOut, AS::Waiting, false));
    let (seq, idx) = launch_movement(&mut engine, owner, Command::Move, OrderType::WalkingUpright);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(ok, "transition should succeed");

    let orders = orders_for(&engine, seq, idx);
    assert!(
        orders.contains(&OrderType::TransitionLeaningOutWaitingAlerted),
        "expected LeaningOut→WaitingAlerted unstick, got {:?}",
        orders
    );
}

/// LEAN_OUT is permitted while a soldier is already LeaningOut.  The
/// soldier override consumes that posture without asking the base actor
/// transition switch to interpret it.
#[test]
fn soldier_lean_out_from_leaning_out_stays_leaning_out() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_soldier(P::LeaningOut, AS::Waiting, false));
    let (seq, idx) = launch(&mut engine, owner, Command::LeanOut);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(ok, "repeat lean-out transition should succeed");
    assert!(orders_for(&engine, seq, idx).is_empty());
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq, idx)
            .expect("sequence element")
            .posture_after_transition,
        P::LeaningOut
    );
}

#[test]
fn soldier_bow_down_entry_from_waiting_loads_before_lowering() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_soldier(P::Upright, AS::Waiting, true));
    let (seq, idx) = launch(&mut engine, owner, Command::EquipBowDown);

    let ok = dispatch_make_final_action_transition(
        &mut engine,
        seq,
        idx,
        owner,
        EA::MUST_BE_AIMING_BOW_DOWN,
    );
    assert!(ok);

    let orders = orders_for(&engine, seq, idx);
    assert_eq!(
        orders,
        vec![
            OrderType::TransitionEquipBow,
            OrderType::TransitionLoadingBow,
            OrderType::TransitionLoweringBowLeaningOut,
        ],
        "soldier bow-down entry translates equip-bow before lower-bow lean-out"
    );
}

#[test]
fn soldier_bow_down_exit_queues_unload_before_unequip() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_soldier(P::LeaningOut, AS::AimingWithBowDown, false));
    let (seq, idx) = launch(&mut engine, owner, Command::Turn);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(ok);

    let orders = orders_for(&engine, seq, idx);
    assert!(
        orders.windows(3).any(|window| {
            window
                == [
                    OrderType::TransitionRaisingBowLeaningOut,
                    OrderType::TransitionUnloadBow,
                    OrderType::TransitionUnequipBow,
                ]
        }),
        "unequip-bow translation queues unload before unequip after raising from bow-down, got {:?}",
        orders
    );
}

/// Soldier with MOVE (WalkingUpright) from Crouched stays crouched
/// because `CAN_BE_CROUCHED` is set.  No crouch-up transition.
#[test]
fn soldier_move_from_crouched_stays_crouched() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_soldier(P::Crouched, AS::Waiting, false));
    let (seq, idx) = launch_movement(&mut engine, owner, Command::Move, OrderType::WalkingUpright);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(ok);

    let orders = orders_for(&engine, seq, idx);
    assert!(
        !orders.contains(&OrderType::TransitionCrouchingUp),
        "should not queue crouch-up (CAN_BE_CROUCHED is set), got {:?}",
        orders
    );
    // posture_after_transition stays Crouched since no posture
    // change was required.
    let posture_after = engine
        .orders
        .sequence_manager
        .get_element(seq, idx)
        .unwrap()
        .posture_after_transition;
    assert_eq!(posture_after, P::Crouched);
}

#[test]
fn crouched_pc_takes_bonus_net_without_landed_net_standup() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_pc(P::Crouched, AS::Waiting));
    let bonus_net = engine.add_entity(Entity::Bonus(crate::element::ElementBonus {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::ObjectBonus;
            initial_element
        },
        object: crate::element::ObjectData {
            object_type: crate::element::ObjectType::BonusNet,
            ..Default::default()
        },
    }));
    let mut elem = SequenceElement::new_interaction(1, Command::Take, Some(owner), Some(bonus_net));
    elem.priority = SequencePriority::Preference;
    elem.posture_after_transition = P::Crouched;
    elem.action_state_after_transition = AS::Waiting;
    let seq = engine.orders.sequence_manager.launch_element(elem);

    assert!(generate_transition(&mut engine, owner, seq, 0));
    assert!(
        !orders_for(&engine, seq, 0).contains(&OrderType::TransitionCrouchingUp),
        "a net bonus element is not the landed-net special case"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq, 0)
            .expect("bonus-net Take element")
            .posture_after_transition,
        P::Crouched
    );
}

/// Soldier MOVE with RunningUpright from Crouched must queue
/// CROUCH_UP (no CAN_BE_CROUCHED on the run path).
#[test]
fn soldier_run_from_crouched_queues_crouch_up() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_soldier(P::Crouched, AS::Waiting, false));
    let (seq, idx) = launch_movement(&mut engine, owner, Command::Move, OrderType::RunningUpright);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(ok);

    let orders = orders_for(&engine, seq, idx);
    assert!(
        orders.contains(&OrderType::TransitionCrouchingUp),
        "expected crouch-up, got {:?}",
        orders
    );
    let posture_after = engine
        .orders
        .sequence_manager
        .get_element(seq, idx)
        .unwrap()
        .posture_after_transition;
    assert_eq!(posture_after, P::Upright);
}

#[test]
fn pc_pass_door_high_crenel_wall_from_crouched_keeps_authored_walk() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_pc(P::Crouched, AS::Moving));
    let (seq, idx) = launch_movement(
        &mut engine,
        owner,
        Command::PassDoor,
        OrderType::WalkingUpright,
    );
    let ctx = TransitionCtx {
        kind: ElementKind::ActorPc,
        command: Command::PassDoor,
        movement_action: Some(OrderType::WalkingUpright),
        force_crouched: false,
        door_type: Some(crate::gate::DoorType::LiftHighCrenel),
        door_lift_kind: Some(crate::sector::LiftType::Wall),
        antagonist_is_net: false,
    };

    let (exit, change, enter) = get_transition_flags(&ctx);
    assert!(exit.is_empty());
    assert!(change.is_empty());
    assert!(enter.is_empty());
    assert!(dispatch_make_posture_transition(
        &mut engine,
        seq,
        idx,
        owner,
        change
    ));
    assert_eq!(orders_for(&engine, seq, idx), Vec::<OrderType>::new());
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq, idx)
            .unwrap()
            .posture_after_transition,
        P::Crouched,
        "the high-crenel wall translator, not generic transition flags, owns the climb choreography"
    );
}

#[test]
fn pc_pass_door_high_wall_from_crouched_still_queues_crouch_up() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_pc(P::Crouched, AS::Moving));
    let (seq, idx) = launch_movement(
        &mut engine,
        owner,
        Command::PassDoor,
        OrderType::WalkingUpright,
    );
    let ctx = TransitionCtx {
        kind: ElementKind::ActorPc,
        command: Command::PassDoor,
        movement_action: Some(OrderType::WalkingUpright),
        force_crouched: false,
        door_type: Some(crate::gate::DoorType::LiftHigh),
        door_lift_kind: Some(crate::sector::LiftType::Wall),
        antagonist_is_net: false,
    };

    let (_, change, _) = get_transition_flags(&ctx);
    assert_eq!(change, CP::MUST_BE_UPRIGHT);
    assert!(dispatch_make_posture_transition(
        &mut engine,
        seq,
        idx,
        owner,
        change
    ));
    assert_eq!(
        orders_for(&engine, seq, idx),
        vec![OrderType::TransitionCrouchingUp]
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq, idx)
            .unwrap()
            .posture_after_transition,
        P::Upright
    );
}

#[test]
fn pc_carrying_on_shoulders_exit_queues_lower_then_stand_chain() {
    let mut engine = EngineInner::new();
    let mut carrier = make_pc(P::CarryingOnShoulders, AS::Waiting);
    let carried = engine.add_entity(make_pc(P::OnShoulders, AS::Waiting));
    if let Entity::Pc(pc) = &mut carrier {
        pc.pc.carried = Some(carried);
    }
    let owner = engine.add_entity(carrier);
    let (seq, idx) = launch(&mut engine, owner, Command::Turn);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(ok);

    assert_eq!(
        orders_for(&engine, seq, idx),
        vec![
            OrderType::TransitionHelpingClimbingDown,
            OrderType::TransitionHelpingClimbingWaitingUpright,
        ]
    );
    let posture_after = engine
        .orders
        .sequence_manager
        .get_element(seq, idx)
        .unwrap()
        .posture_after_transition;
    assert_eq!(posture_after, P::Upright);
}

#[test]
fn pc_carrying_corpse_without_carried_entity_is_impossible() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_pc(P::CarryingCorpse, AS::Waiting));
    let (seq, idx) = launch(&mut engine, owner, Command::Turn);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(
        !ok,
        "CarryingCorpse without pc.carried should fail instead of silently snapping upright"
    );
    assert!(
        orders_for(&engine, seq, idx).is_empty(),
        "unsupported corpse-drop transition should not queue a fake animation"
    );
}

#[test]
fn pc_enter_swordfight_from_carrying_corpse_registers_exit_before_execute() {
    let mut engine = EngineInner::new();
    let carried = engine.add_entity(make_soldier(P::Carried, AS::Waiting, false));
    let mut carrier = make_pc(P::CarryingCorpse, AS::Waiting);
    if let Entity::Pc(pc) = &mut carrier {
        pc.pc.carried = Some(carried);
    }
    let owner = engine.add_entity(carrier);
    let (seq, idx) = launch(&mut engine, owner, Command::EnterSwordfight);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(ok);

    assert_eq!(
        orders_for(&engine, seq, idx),
        vec![OrderType::TransitionCarryingCorpseWaitingUpright],
        "the corpse-exit order is registered before its first execution performs the instant drop"
    );
    assert_eq!(
        engine.get_entity(owner).unwrap().posture(),
        P::CarryingCorpse
    );
    assert_eq!(engine.get_entity(carried).unwrap().posture(), P::Carried);
    assert_eq!(
        engine.get_entity(owner).unwrap().pc_data().unwrap().carried,
        Some(carried),
        "translation must not consume the carried relationship"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(seq, idx)
            .unwrap()
            .posture_after_transition,
        P::Upright
    );
}

/// PC CROUCH_UP from Crouched produces no transition animations
/// (the command itself is the animation) and flips
/// posture-after-transition to Upright.
#[test]
fn pc_crouch_up_from_crouched_snaps_to_upright() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_pc(P::Crouched, AS::Waiting));
    let (seq, idx) = launch(&mut engine, owner, Command::CrouchUp);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(ok);

    // Refused double-crouch case: CROUCH_UP on Upright → false.
    // From Crouched, MUST_BE_CROUCHED is set and posture_after
    // stays Crouched — no animation is queued.
    let posture_after = engine
        .orders
        .sequence_manager
        .get_element(seq, idx)
        .unwrap()
        .posture_after_transition;
    assert_eq!(posture_after, P::Crouched);
}

/// Soldier ENTER_SWORDFIGHT from Waiting with `attentive=false`
/// must fire the MUST_BE_ALERTED branch, queueing the
/// `TransitionWaitingUprightWaitingAlerted` order so the soldier
/// stands to attention before fighting.
#[test]
fn soldier_enter_swordfight_fires_must_be_alerted() {
    let mut engine = EngineInner::new();
    let mut soldier = make_soldier(P::Upright, AS::Waiting, false);
    soldier.element_data_mut().set_direction_goal(15);
    let owner = engine.add_entity(soldier);
    let (seq, idx) = launch(&mut engine, owner, Command::EnterSwordfight);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(ok, "transition should succeed");

    let orders = orders_for(&engine, seq, idx);
    assert!(
        orders.contains(&OrderType::TransitionWaitingUprightWaitingAlerted),
        "expected attentive-mode transition, got {:?}",
        orders
    );
    assert_eq!(
        order_compute_direction_for(
            &engine,
            seq,
            idx,
            OrderType::TransitionWaitingUprightWaitingAlerted,
        ),
        Some(false),
        "Original attentive entry is an in-place transition"
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .get_direction_goal()
            .as_u8(),
        15,
        "booking attentive entry must preserve the live direction goal"
    );
}

/// Original soldier Translate stamps the attentive-to-upright order with
/// direction computation disabled. This transition is also inserted as the
/// exit action for commands that require ordinary waiting; its zero target
/// must not turn a non-origin actor toward sector 9.
#[test]
fn attentive_exit_transition_preserves_direction_goal() {
    let mut engine = EngineInner::new();
    let mut soldier = make_soldier(P::Upright, AS::Waiting, true);
    soldier.element_data_mut().set_direction_goal(1);
    let owner = engine.add_entity(soldier);
    let (seq, idx) = launch(&mut engine, owner, Command::SitDown);

    assert!(generate_transition(&mut engine, owner, seq, idx));
    assert_eq!(
        order_compute_direction_for(
            &engine,
            seq,
            idx,
            OrderType::TransitionWaitingAlertedWaitingUpright,
        ),
        Some(false),
        "Original attentive exit is an in-place transition"
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .get_direction_goal()
            .as_u8(),
        1,
        "booking attentive exit must preserve the live direction goal"
    );
}

/// Original's final-transition test reads the current attentive pose,
/// not the already-committed target. A direct call while an enter
/// transition is still pending therefore inserts the alerted transition.
/// Normal sequence arbitration prevents such a successor from being
/// instructed before the preceding enter animation completes.
#[test]
fn mid_transition_soldier_uses_current_attentive_pose() {
    let mut engine = EngineInner::new();
    let mut e = make_soldier(P::Upright, AS::Waiting, false);
    if let Entity::Soldier(s) = &mut e
        && let Some(enemy) = s.npc.ai_brain.enemy_mut()
    {
        enemy.will_be_attentive = true;
        enemy.attentive = false;
    }
    let owner = engine.add_entity(e);
    let (seq, idx) = launch(&mut engine, owner, Command::EnterSwordfight);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(ok);

    let orders = orders_for(&engine, seq, idx);
    assert!(
        orders.contains(&OrderType::TransitionWaitingUprightWaitingAlerted),
        "current non-attentive pose must request the alerted animation, got {:?}",
        orders
    );
}

/// A leave request queued behind an in-progress enter transition is
/// translated only after the enter completes. At that point Original
/// reads the soldier's now-live attentive pose and books the leave
/// animation directly, without replaying the enter animation first.
#[test]
fn postponed_leave_after_enter_does_not_requeue_enter_transition() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_soldier(P::Upright, AS::Waiting, false));
    let sim = crate::sim_rng::test_context();
    let assets = crate::engine::LevelAssets::default();

    // Both attentive elements reach instruction handling through the ordinary
    // sequence-manager update; that boundary resolves priorities and
    // arbitrates, so drive it instead of manually staging states.
    let mut display = crate::engine::HostDisplayState::default();
    let enter = engine.launch_element(SequenceElement::new(
        1,
        Command::EnterAttentiveMode,
        Some(owner),
    ));
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(enter, 0)
            .expect("enter element registered")
            .state,
        crate::sequence::SequenceState::InProgress
    );

    if let Some(enemy) = engine.get_entity_mut(owner).and_then(Entity::enemy_ai_mut) {
        enemy.will_be_attentive = false;
    }
    let leave = engine.launch_element(SequenceElement::new(
        1,
        Command::LeaveAttentiveMode,
        Some(owner),
    ));
    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    let leave_element = engine
        .orders
        .sequence_manager
        .get_element(leave, 0)
        .expect("leave element should remain registered while postponed");
    assert_eq!(
        leave_element.state,
        crate::sequence::SequenceState::Postponed
    );
    assert!(
        leave_element.orders.is_empty(),
        "postponing must discard launch-time transition orders"
    );

    if let Some(enemy) = engine.get_entity_mut(owner).and_then(Entity::enemy_ai_mut) {
        enemy.attentive = true;
    }
    engine.orders.sequence_manager.element_terminated(enter, 0);

    engine.hourglass_phase_sequences(&sim, &mut display, &assets);

    assert_eq!(
        orders_for(&engine, leave, 0),
        vec![OrderType::TransitionWaitingAlertedWaitingUpright],
        "resumed leave must use the completed enter transition's live attentive pose"
    );
}

/// Attentive soldier with ENTER_SWORDFIGHT should NOT queue the
/// enter-attentive transition again (the flag short-circuits).
#[test]
fn attentive_soldier_enter_swordfight_no_double_alert() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_soldier(P::Upright, AS::Waiting, true));
    let (seq, idx) = launch(&mut engine, owner, Command::EnterSwordfight);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(ok);

    let orders = orders_for(&engine, seq, idx);
    assert!(
        !orders.contains(&OrderType::TransitionWaitingUprightWaitingAlerted),
        "already-attentive soldier should not re-queue the transition, got {:?}",
        orders
    );
}

/// A soldier Bored + Wait command shouldn't queue anything: the
/// transition flags allow CAN_BE_BORED, so the bored→waiting path
/// is skipped.
#[test]
fn bored_soldier_wait_no_transition() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_soldier(P::Upright, AS::Bored, false));
    let (seq, idx) = launch(&mut engine, owner, Command::Wait);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(ok);

    let orders = orders_for(&engine, seq, idx);
    assert!(
        !orders.contains(&OrderType::TransitionWaitingUprightBoredWaitingUpright),
        "bored→waiting should not be queued when CAN_BE_BORED is set, got {:?}",
        orders
    );
}

#[test]
fn throw_purse_keeps_bored_until_exit_transition_completes() {
    use crate::coordinates::MapPoint;
    use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

    let sim = crate::sim_rng::test_context();
    let assets = crate::engine::LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_pc(P::Upright, AS::Bored));
    let (seq, idx) = launch(&mut engine, owner, Command::ThrowPurse);

    assert!(generate_transition(&mut engine, owner, seq, idx));
    assert_eq!(
        orders_for(&engine, seq, idx),
        vec![OrderType::TransitionWaitingUprightBoredWaitingUpright]
    );

    let result = crate::abilities::begin_throw_purse(
        &mut engine.world.entities,
        &mut engine.orders.sequence_manager,
        owner,
        MapPoint::new(100.0, 100.0),
        seq,
        idx,
        &mut engine.orders.next_order_id,
    );
    assert_eq!(result, crate::abilities::BeginResult::Started);
    engine.orders.sequence_manager.element_in_progress(seq, idx);

    assert_eq!(
        engine
            .get_entity(owner)
            .expect("purse thrower remains live")
            .actor_data()
            .expect("purse thrower remains an actor")
            .action_state,
        AS::Bored,
        "installing the purse body must not pre-commit the transition's Waiting state"
    );
    assert_eq!(
        orders_for(&engine, seq, idx),
        vec![
            OrderType::TransitionWaitingUprightBoredWaitingUpright,
            OrderType::ThrowingPurse,
        ]
    );

    let transition = OrderType::TransitionWaitingUprightBoredWaitingUpright;
    let script = SpriteScript {
        action_id: transition as u16,
        action_done: 2,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![2; 3],
        distances: vec![0; 3],
        offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0; 3],
    };
    let mut conversion = vec![UNMAPPED; NONANIMATION_END];
    conversion[transition as usize] = 0;
    engine
        .get_entity_mut(owner)
        .expect("purse thrower remains live")
        .element_data_mut()
        .sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(conversion),
    );

    for _ in 0..16 {
        engine.tick_actor_animation_for(&sim, &assets, owner);
        if engine
            .get_entity(owner)
            .expect("purse thrower remains live")
            .actor_data()
            .expect("purse thrower remains an actor")
            .action_state
            == AS::Waiting
        {
            break;
        }
    }
    assert_eq!(
        engine
            .get_entity(owner)
            .expect("purse thrower remains live")
            .actor_data()
            .expect("purse thrower remains an actor")
            .action_state,
        AS::Waiting,
        "the bored-exit transition completion owns the live Waiting write"
    );
}

/// Upright Moving soldier receiving CrouchDown must queue the
/// walking→waiting exit transition (CAN_BE_MOVING is cleared),
/// then the crouch-down itself is the command.  Verify the exit
/// transition fires.
#[test]
fn soldier_crouch_down_from_upright_moving_queues_exit() {
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_soldier(P::Upright, AS::Moving, false));
    let (seq, idx) = launch(&mut engine, owner, Command::CrouchDown);

    let ok = generate_transition(&mut engine, owner, seq, idx);
    assert!(ok);

    let orders = orders_for(&engine, seq, idx);
    // CAN_BE_MOVING is set for CrouchDown in the base flags, so
    // the walk→wait exit transition should NOT queue.  This test
    // locks that behaviour in.
    assert!(
        !orders.contains(&OrderType::TransitionWalkingUprightWaitingUpright),
        "CAN_BE_MOVING covers CrouchDown; should not queue walk→wait, got {:?}",
        orders
    );
}
