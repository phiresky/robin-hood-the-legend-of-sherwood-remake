use super::*;

fn assets_with_test_pc_profile() -> LevelAssets {
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles
        .characters
        .push(crate::profiles::CharacterProfile::default());
    LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    }
}

#[test]
fn waiting_alerted_execute_registers_corrective_leave_when_requested_state_is_nonattentive() {
    use crate::element::{ActionState, Camp, Command};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let enemy = engine
        .get_entity_mut(owner)
        .and_then(Entity::enemy_ai_mut)
        .expect("test soldier has enemy AI");
    enemy.attentive = true;
    enemy.will_be_attentive = false;
    engine
        .get_entity_mut(owner)
        .expect("test soldier remains live")
        .actor_data_mut()
        .expect("test soldier has actor state")
        .action_state = ActionState::Waiting;
    bind_test_action_point(
        &mut engine,
        owner,
        OrderType::WaitingAlerted,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );
    let mut wait = SequenceElement::new(1, Command::Wait, Some(owner));
    wait.orders
        .push_back(Order::test_new(OrderType::WaitingAlerted, 0.0, 0.0));
    let wait_sequence = engine.orders.sequence_manager.launch_element(wait);
    engine
        .orders
        .sequence_manager
        .element_in_progress(wait_sequence, 0);

    let (_, mut outcomes, executed) = engine.tick_actor_animation_for(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
    );
    assert_eq!(
        executed.map(|result| result.order_type),
        Some(OrderType::WaitingAlerted),
        "the regression must enter the actual soldier WaitingAlerted Execute arm"
    );
    assert_eq!(outcomes.execute_sides.waiting_alerted, [owner]);
    engine.drain_waiting_alerted(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        std::mem::take(&mut outcomes.execute_sides.waiting_alerted),
    );

    let matching: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| {
            element.owner == Some(owner) && element.command == Command::LeaveAttentiveMode
        })
        .map(|element| element.state)
        .collect();
    assert_eq!(matching, [SequenceState::Todo]);
    assert!(
        engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(owner, Command::LeaveAttentiveMode)
    );
}

#[test]
fn waiting_upright_execute_registers_corrective_enter_when_requested_state_is_attentive() {
    use crate::element::{ActionState, Camp, Command};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let enemy = engine
        .get_entity_mut(owner)
        .and_then(Entity::enemy_ai_mut)
        .expect("test soldier has enemy AI");
    enemy.attentive = false;
    enemy.will_be_attentive = true;
    engine
        .get_entity_mut(owner)
        .expect("test soldier remains live")
        .actor_data_mut()
        .expect("test soldier has actor state")
        .action_state = ActionState::Waiting;
    bind_test_action_point(
        &mut engine,
        owner,
        OrderType::WaitingUpright,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );
    let mut wait = SequenceElement::new(1, Command::Wait, Some(owner));
    wait.orders
        .push_back(Order::test_new(OrderType::WaitingUpright, 0.0, 0.0));
    let wait_sequence = engine.orders.sequence_manager.launch_element(wait);
    engine
        .orders
        .sequence_manager
        .element_in_progress(wait_sequence, 0);

    let (_, mut outcomes, executed) = engine.tick_actor_animation_for(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
    );
    assert_eq!(
        executed.map(|result| result.order_type),
        Some(OrderType::WaitingUpright),
        "the regression must enter the actual soldier WaitingUpright Execute arm"
    );
    assert_eq!(outcomes.execute_sides.waiting_upright, [owner]);
    engine.drain_waiting_upright(std::mem::take(&mut outcomes.execute_sides.waiting_upright));

    let matching: Vec<_> = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| {
            element.owner == Some(owner) && element.command == Command::EnterAttentiveMode
        })
        .map(|element| element.state)
        .collect();
    assert_eq!(matching, [SequenceState::Todo]);
    assert!(
        engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(owner, Command::EnterAttentiveMode)
    );
}

#[test]
fn waiting_upright_execute_needs_represented_attentive_state_for_correction() {
    use crate::element::{Camp, Command};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let mut entity = make_test_ai_soldier(Camp::Lacklandists);
    let Entity::Soldier(soldier) = &mut entity else {
        unreachable!("AI soldier fixture changed entity kind");
    };
    soldier.npc.ai_brain = crate::element::AiBrain::None;
    let owner = engine.add_entity(entity);
    bind_test_action_point(
        &mut engine,
        owner,
        OrderType::WaitingUpright,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );
    let mut wait = SequenceElement::new(1, Command::Wait, Some(owner));
    wait.orders
        .push_back(Order::test_new(OrderType::WaitingUpright, 0.0, 0.0));
    let wait_sequence = engine.orders.sequence_manager.launch_element(wait);
    engine
        .orders
        .sequence_manager
        .element_in_progress(wait_sequence, 0);

    let (_, outcomes, executed) = engine.tick_actor_animation_for(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
    );
    assert_eq!(
        executed.map(|result| result.order_type),
        Some(OrderType::WaitingUpright)
    );
    assert!(outcomes.execute_sides.waiting_upright.is_empty());
}

#[test]
fn waiting_alerted_execute_does_not_duplicate_a_leave_already_waiting_to_launch() {
    use crate::element::{Camp, Command};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    engine
        .get_entity_mut(owner)
        .and_then(Entity::enemy_ai_mut)
        .expect("test soldier has enemy AI")
        .will_be_attentive = false;
    engine.launch_element(SequenceElement::new(
        1,
        Command::LeaveAttentiveMode,
        Some(owner),
    ));

    engine.drain_waiting_alerted(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        vec![owner],
    );

    let matching = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .filter(|element| {
            element.owner == Some(owner) && element.command == Command::LeaveAttentiveMode
        })
        .count();
    assert_eq!(matching, 1);
}

#[test]
fn waiting_alerted_execute_preserves_attentive_requested_state() {
    use crate::element::{Camp, Command};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    let enemy = engine
        .get_entity_mut(owner)
        .and_then(Entity::enemy_ai_mut)
        .expect("test soldier has enemy AI");
    enemy.attentive = true;
    enemy.will_be_attentive = true;

    engine.drain_waiting_alerted(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        vec![owner],
    );

    assert!(
        !engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| {
                element.owner == Some(owner) && element.command == Command::LeaveAttentiveMode
            })
    );
}

pub(super) fn bind_test_action_point(
    engine: &mut EngineInner,
    id: EntityId,
    action: crate::order::OrderType,
    hotspot: crate::coordinates::SpriteLocalPoint,
    center: crate::coordinates::SpriteAnchor,
) {
    let script = crate::sprite_script::SpriteScript {
        action_id: action as u16,
        action_done: 0,
        average_speed: 0.0,
        hotspot,
        sum_distance: 0,
        frame_ids: vec![1],
        delays: vec![1],
        distances: vec![0],
        offsets: vec![SpriteFrameOffset::ZERO],
        sound_ids: vec![0],
    };
    let mut conversion =
        vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
    conversion[action as usize] = 0;
    let mut sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script]),
        std::sync::Arc::new(conversion),
    );
    sprite.center = center;
    let element = engine.get_entity_mut(id).unwrap().element_data_mut();
    let position = element.position_map();
    let direction = element.direction();
    element.sprite = sprite;
    element.set_position_map(position);
    element.set_direction_instantly(direction);
}

pub(super) fn bind_test_bow_release_action(engine: &mut EngineInner, id: EntityId) {
    let action = crate::order::OrderType::ShootingWithBow;
    let script = crate::sprite_script::SpriteScript {
        action_id: action as u16,
        action_done: 0,
        average_speed: 0.0,
        hotspot: crate::coordinates::SpriteLocalPoint::new(2.0, 3.0),
        sum_distance: 0,
        frame_ids: vec![1, 2, 3],
        delays: vec![0, 0, 0],
        distances: vec![0, 0, 0],
        offsets: vec![SpriteFrameOffset::ZERO; 3],
        sound_ids: vec![0, 0, 0],
    };
    let mut conversion =
        vec![crate::sprite_script::UNMAPPED; crate::sprite_script::NONANIMATION_END];
    conversion[action as usize] = 0;
    let mut sprite = crate::sprite::Sprite::new(
        std::sync::Arc::new(vec![script; 16]),
        std::sync::Arc::new(conversion),
    );
    let element = engine.get_entity_mut(id).unwrap().element_data_mut();
    let position = element.position_map();
    let direction = element.direction();
    sprite.center = crate::coordinates::SpriteAnchor::ZERO;
    element.sprite = sprite;
    element.set_position_map(position);
    element.set_direction_instantly(direction);
}

#[test]
fn postponed_generic_order_carrier_resumes_in_progress() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    let mut element = SequenceElement::new_generic(1, Command::Generic, Some(soldier));
    element.posture_after_transition = Posture::Upright;
    element.orders.push_back(Order::new(
        OrderType::WaitingUpright,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    let sequence = engine.orders.sequence_manager.launch_element(element);

    let mut display = HostDisplayState::default();
    let assets = LevelAssets::default();
    engine.hourglass_phase_sequences(&crate::sim_rng::test_context(), &mut display, &assets);

    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("generic carrier remains live while its order plays");
    assert_eq!(element.state, SequenceState::InProgress);
    assert_eq!(element.orders.len(), 1);
}

#[test]
fn manager_instruct_rejects_transition_terminated_element_before_priority_and_arbitration() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let mut owner_entity = make_test_soldier(Posture::Crouched);
    let crate::element::Entity::Soldier(owner_soldier) = &mut owner_entity else {
        unreachable!();
    };
    let mut enemy_ai = crate::ai_enemy::EnemyAi::default();
    enemy_ai.hth_weapon_id = 1;
    owner_soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::new(enemy_ai));
    let owner = engine.add_entity(owner_entity);
    {
        let soldier = engine.get_entity_mut(owner).unwrap();
        soldier.actor_data_mut().unwrap().action_state = ActionState::Waiting;
        soldier.enemy_ai_mut().unwrap().attentive = true;
    }

    let live_order_id = engine.orders.allocate_order_id();
    let mut live = SequenceElement::new(1, Command::Wait, Some(owner));
    live.priority = SequencePriority::Normal;
    live.posture_after_transition = Posture::Crouched;
    live.orders.push_back(Order::new(
        OrderType::WaitingUpright,
        0.0,
        0.0,
        live_order_id,
    ));
    let live_sequence = engine.orders.sequence_manager.launch_element(live);
    engine
        .orders
        .sequence_manager
        .element_in_progress(live_sequence, 0);
    engine.publish_selected_order_as_installed(owner);

    // CrouchUp cannot retain the soldier's attentive action state. With a
    // stamped Crouched posture, GenerateTransition follows Original's silent
    // leave-attentive arm and terminates the incoming element inline while
    // the posture transition remains valid. This is an ordinary manager
    // registration, not the script-sync path.
    let incoming = SequenceElement::new(1, Command::CrouchUp, Some(owner));
    let incoming_sequence = engine.orders.sequence_manager.launch_element(incoming);

    let mut assets = LevelAssets::default();
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    profiles.soldiers.push(Default::default());
    profiles.hth_weapons.push(Default::default());
    assert!(
        engine
            .get_entity(owner)
            .unwrap()
            .enemy_ai()
            .unwrap()
            .attentive
    );
    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(&crate::sim_rng::test_context(), &mut display, &assets);

    let incoming = engine
        .orders
        .sequence_manager
        .get_element(incoming_sequence, 0)
        .expect("transition-terminated element remains inspectable");
    assert_eq!(incoming.state, SequenceState::Terminated);
    assert_eq!(
        incoming.priority,
        SequencePriority::NotYetSet,
        "Actor::Instruct returns before DeterminePriority"
    );
    let (selected_sequence, selected_index, selected_order) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(owner)
        .expect("the pre-existing live owner element remains selected");
    assert_eq!((selected_sequence, selected_index), (live_sequence, 0));
    assert_eq!(selected_order.order_id, live_order_id);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(live_sequence, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );
}

#[test]
fn retained_waiting_sword_handoff_preserves_running_sprite_identity() {
    use crate::element::{Command, InstalledActorOrder, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    let old_order_id = engine.orders.allocate_order_id();
    let new_order_id = engine.orders.allocate_order_id();
    {
        let entity = engine.get_entity_mut(soldier).unwrap();
        let actor = entity.actor_data_mut().unwrap();
        actor.installed_order = Some(InstalledActorOrder {
            order_id: old_order_id,
            order_type: OrderType::WaitingSword,
        });
        actor.retained_waiting_sword_order_id = Some(old_order_id);
        entity.sprite_mut().last_processed_order_id = old_order_id.get();
        entity.sprite_mut().frame_count = 5;
    }

    let mut wait = SequenceElement::new_generic(1, Command::Wait, Some(soldier));
    wait.orders
        .push_back(Order::new(OrderType::WaitingSword, 0.0, 0.0, new_order_id));
    let sequence = engine.orders.sequence_manager.launch_element(wait);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    engine.publish_selected_order_as_installed(soldier);

    let entity = engine.get_entity(soldier).unwrap();
    let actor = entity.actor_data().unwrap();
    assert_eq!(actor.installed_order.unwrap().order_id, new_order_id);
    assert_eq!(actor.retained_waiting_sword_order_id, None);
    assert_eq!(actor.last_execute_order_id, Some(new_order_id));
    assert_eq!(entity.sprite().last_processed_order_id, new_order_id.get());
    assert_eq!(entity.sprite().frame_count, 5);
}

#[test]
fn sequence_phase_clears_unconsumed_waiting_sword_retention() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{Field, FieldValue, SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 1;
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    let opponent = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(soldier)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(opponent);
    engine
        .get_entity_mut(opponent)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents
        .push(soldier);
    engine
        .get_entity_mut(soldier)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::WaitingSword;

    let wait_order_id = engine.orders.allocate_order_id();
    let mut wait = SequenceElement::new_generic(1, Command::Wait, Some(soldier));
    wait.priority = SequencePriority::Wait;
    wait.posture_after_transition = Posture::Upright;
    wait.orders
        .push_back(Order::new(OrderType::WaitingSword, 0.0, 0.0, wait_order_id));
    let wait_sequence = engine.orders.sequence_manager.launch_element(wait);
    engine
        .orders
        .sequence_manager
        .element_in_progress(wait_sequence, 0);
    engine.publish_selected_order_as_installed(soldier);

    let mut enter = SequenceElement::new_generic(1, Command::EnterSwordfight, Some(soldier));
    enter.priority = SequencePriority::PostponeEverythingButInjuries;
    enter.posture_after_transition = Posture::Upright;
    enter.set_property(Field::Opponent, FieldValue::Element(opponent));
    let enter_sequence = engine.orders.sequence_manager.launch_element(enter);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(enter_sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    let actor = engine.get_entity(soldier).unwrap().actor_data().unwrap();
    assert_eq!(actor.installed_order, None);
    assert_eq!(actor.retained_waiting_sword_order_id, None);
}

#[test]
fn exhausted_generic_order_carrier_terminates_on_resume() {
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    let mut element = SequenceElement::new_generic(1, Command::Generic, Some(soldier));
    element.posture_after_transition = Posture::Upright;
    let sequence = engine.orders.sequence_manager.launch_element(element);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("terminated carrier remains until cleanup")
            .state,
        SequenceState::Terminated
    );
}

#[test]
fn accepted_zero_order_damage_preserves_in_progress_motion_edge() {
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(soldier)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .continuation
        .motion_state = MotionState::Done;

    // A malformed damage element is still accepted by Actor::Instruct, then
    // its command translation terminates it synchronously without an order.
    // Original writes IN_PROGRESS between those two events.
    let mut damage = SequenceElement::new_generic(1, Command::ReceiveSwordDamage, Some(soldier));
    damage.posture_after_transition = Posture::Upright;
    let sequence = engine.orders.sequence_manager.launch_element(damage);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("terminated damage element remains until cleanup")
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(soldier)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::InProgress,
        "accepted Actor::Instruct must expose its motion edge even when translation terminates"
    );
}

#[test]
fn manager_redundant_stop_parry_skips_instruct_motion_epilogue() {
    use crate::element::{ActionState, Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    {
        let actor = engine
            .get_entity_mut(soldier)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.action_state = ActionState::WaitingSword;
        actor.continuation.motion_state = MotionState::Terminated;
    }
    let mut stop = SequenceElement::new(1, Command::StopParrySword, Some(soldier));
    stop.posture_after_transition = Posture::Upright;
    let sequence_id = engine.orders.sequence_manager.launch_element(stop);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .expect("redundant stop-parry remains inspectable")
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(soldier)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::Terminated,
        "redundant StopParry returns before Actor::Instruct's motion epilogue"
    );
}

#[test]
fn manager_redundant_enter_attentive_skips_instruct_motion_epilogue() {
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    {
        let entity = engine.get_entity_mut(soldier).unwrap();
        let crate::element::Entity::Soldier(soldier_entity) = entity else {
            unreachable!("test owner must remain a soldier")
        };
        soldier_entity.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
        entity
            .enemy_ai_mut()
            .expect("test soldier must carry enemy AI")
            .attentive = true;
        entity.actor_data_mut().unwrap().continuation.motion_state = MotionState::Terminated;
    }
    let mut enter = SequenceElement::new(1, Command::EnterAttentiveMode, Some(soldier));
    enter.posture_after_transition = Posture::Upright;
    let sequence_id = engine.orders.sequence_manager.launch_element(enter);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence_id, 0)
            .expect("redundant attentive element remains inspectable")
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(soldier)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::Terminated,
        "Translate-time termination must bypass Actor::Instruct's IN_PROGRESS epilogue"
    );
}

#[test]
fn assert_position_translation_preserves_terminal_motion_edge() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceElementData, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let civilian = engine.add_entity(make_test_civilian(Posture::Upright));
    engine
        .get_entity_mut(civilian)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .continuation
        .motion_state = MotionState::Terminated;

    let mut assertion = SequenceElement::new_movement(
        1,
        Command::AssertPosition,
        Some(civilian),
        OrderType::WalkingUpright,
    );
    assertion.posture_after_transition = Posture::Upright;
    if let SequenceElementData::Movement {
        destination,
        tolerance,
        ..
    } = &mut assertion.data
    {
        *destination = MapPoint::ZERO;
        *tolerance = 10.0;
    }
    let sequence = engine.orders.sequence_manager.launch_element(assertion);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("position assertion remains inspectable")
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(civilian)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::Terminated,
        "Translate-time SetState must skip Actor::Instruct's IN_PROGRESS epilogue"
    );
}

#[test]
fn synchronous_accepted_wait_stamps_in_progress_motion_and_publishes_order() {
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};
    use crate::sprite::MotionState;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(soldier)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .continuation
        .motion_state = MotionState::Done;

    let mut wait = SequenceElement::new(1, Command::Wait, Some(soldier));
    wait.priority = SequencePriority::Wait;
    wait.posture_after_transition = Posture::Upright;
    let sequence = engine.launch_element(wait);
    engine
        .drain_script_synchronous_actions(&sim, &LevelAssets::default(), &mut Vec::new())
        .expect("synchronous Wait should dispatch");

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );
    let actor = engine.get_entity(soldier).unwrap().actor_data().unwrap();
    assert_eq!(actor.continuation.motion_state, MotionState::InProgress);
    assert!(
        actor.installed_order.is_some(),
        "accepted synchronous Instruct must publish its current order"
    );
}

#[test]
fn synchronous_accepted_zero_order_damage_stamps_in_progress_motion() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequenceElementData, SequencePriority, SequenceState};
    use crate::sprite::MotionState;
    use crate::weapons::SwordStrike;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_soldier(Posture::Upright));
    let retained_goal = MapPoint::new(70.0, 80.0);
    {
        let entity = engine.get_entity_mut(victim).unwrap();
        let actor = entity.actor_data_mut().unwrap();
        actor.action_state = crate::element::ActionState::ParryingSword;
        actor.continuation.motion_state = MotionState::Done;
        entity.position_iface_mut().set_map_goal(retained_goal);
    }

    // A successful parry while already in the parrying state translates to
    // no reaction order. Actor::Instruct detaches that accepted empty element
    // before its terminal condolence, so the interrupted movement goal stays
    // live for the postponed movement that resumes afterward.
    let mut damage = SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data = SequenceElementData::new_sword_damage(attacker, SwordStrike::A, 0);
    damage.priority = SequencePriority::Wait;
    damage.posture_after_transition = Posture::Upright;
    let sequence = engine.launch_element(damage);
    engine
        .drain_script_synchronous_actions(&sim, &LevelAssets::default(), &mut Vec::new())
        .expect("synchronous parried damage should dispatch");

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::InProgress,
        "accepted empty translation must retain Actor::Instruct's motion edge"
    );
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .position_iface()
            .map_goal(),
        retained_goal,
        "an accepted empty damage card must not clear the resuming movement goal"
    );
}

#[test]
fn synchronous_assert_position_skips_instruct_epilogue() {
    use crate::coordinates::MapPoint;
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceElementData, SequencePriority, SequenceState};
    use crate::sprite::MotionState;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let civilian = engine.add_entity(make_test_civilian(Posture::Upright));
    engine
        .get_entity_mut(civilian)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .continuation
        .motion_state = MotionState::Terminated;

    let mut assertion = SequenceElement::new_movement(
        1,
        Command::AssertPosition,
        Some(civilian),
        OrderType::WalkingUpright,
    );
    assertion.priority = SequencePriority::Wait;
    assertion.posture_after_transition = Posture::Upright;
    if let SequenceElementData::Movement {
        destination,
        tolerance,
        ..
    } = &mut assertion.data
    {
        *destination = MapPoint::ZERO;
        *tolerance = 10.0;
    }
    let sequence = engine.launch_element(assertion);
    engine
        .drain_script_synchronous_actions(&sim, &LevelAssets::default(), &mut Vec::new())
        .expect("synchronous AssertPosition should dispatch");

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(civilian)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::Terminated
    );
}

#[test]
fn synchronous_terminal_enter_swordfight_skips_instruct_epilogue() {
    use crate::element::{ActionState, Command, Posture};
    use crate::sequence::{Field, FieldValue, SequenceElement, SequencePriority, SequenceState};
    use crate::sprite::MotionState;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    let opponent = engine.add_entity(make_test_soldier(Posture::Upright));
    {
        let actor = engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.action_state = ActionState::WaitingSword;
        actor.continuation.motion_state = MotionState::Terminated;
    }

    let mut enter = SequenceElement::new_generic(1, Command::EnterSwordfight, Some(owner));
    enter.priority = SequencePriority::Wait;
    enter.posture_after_transition = Posture::Upright;
    enter.set_property(Field::Opponent, FieldValue::Element(opponent));
    let sequence = engine.launch_element(enter);
    engine
        .drain_script_synchronous_actions(&sim, &LevelAssets::default(), &mut Vec::new())
        .expect("synchronous satisfied EnterSwordfight should dispatch");

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::Terminated,
        "terminal Translate must bypass Actor::Instruct's motion epilogue"
    );
}

#[test]
fn synchronous_redundant_parry_skips_instruct_epilogue() {
    use crate::element::{ActionState, Command, Posture};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};
    use crate::sprite::MotionState;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    {
        let actor = engine
            .get_entity_mut(soldier)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.action_state = ActionState::ParryingSword;
        actor.continuation.motion_state = MotionState::Terminated;
    }

    let mut parry = SequenceElement::new(1, Command::ParrySword, Some(soldier));
    parry.priority = SequencePriority::Wait;
    parry.posture_after_transition = Posture::Upright;
    let sequence = engine.launch_element(parry);
    engine
        .drain_script_synchronous_actions(&sim, &LevelAssets::default(), &mut Vec::new())
        .expect("synchronous redundant ParrySword should dispatch");

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(soldier)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::Terminated
    );
}

#[test]
fn manager_redundant_parry_skips_instruct_epilogue_after_generated_transition() {
    use crate::element::{ActionState, Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    {
        let actor = engine
            .get_entity_mut(soldier)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.action_state = ActionState::ParryingSword;
        actor.continuation.motion_state = MotionState::Start;
    }

    // GenerateTransition first appends the parry-to-waiting prefix. Translate
    // then discovers that the live actor is already parrying and terminates
    // the incoming element synchronously. Actor::Instruct returns before its
    // IN_PROGRESS epilogue when that callback changes mpSequenceElement.
    let sequence =
        engine.launch_element(SequenceElement::new(1, Command::ParrySword, Some(soldier)));
    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap();
    assert_eq!(element.state, SequenceState::Terminated);
    assert_eq!(
        element.orders.len(),
        1,
        "generated prefix remains diagnostic"
    );
    assert_eq!(
        engine
            .get_entity(soldier)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::Start,
        "terminal Translate must preserve the preceding Execute edge"
    );
}

#[test]
fn redundant_raise_shield_preserves_prior_look_left_start_edge() {
    use crate::element::{ActionState, Command, InstalledActorOrder, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    let look_order_id = engine.orders.allocate_order_id();
    {
        let actor = engine
            .get_entity_mut(soldier)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.action_state = ActionState::HoldingShield;
        actor.continuation.motion_state = MotionState::Start;
        actor.installed_order = Some(InstalledActorOrder {
            order_id: look_order_id,
            order_type: OrderType::LookingLeft,
        });
    }

    // RHElementActorHuman::Translate terminates this command immediately.
    // Actor::Instruct observes that its selected element changed and returns
    // before publishing its ordinary accepted-instruction motion edge.
    let sequence =
        engine.launch_element(SequenceElement::new(1, Command::RaiseShield, Some(soldier)));
    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(soldier)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::Start,
        "terminal RaiseShield must not overwrite the preceding LookLeft edge"
    );
}

#[test]
fn soldier_moving_shield_still_raises_and_publishes_accepted_motion_edge() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    {
        let actor = engine
            .get_entity_mut(soldier)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.action_state = ActionState::MovingShield;
        actor.continuation.motion_state = MotionState::Start;
    }

    let sequence =
        engine.launch_element(SequenceElement::new(1, Command::RaiseShield, Some(soldier)));
    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .unwrap();
    assert_eq!(element.state, SequenceState::InProgress);
    assert_eq!(
        element.orders.front().map(|order| order.order_type),
        Some(OrderType::RaisingShield)
    );
    assert_eq!(
        engine
            .get_entity(soldier)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::InProgress
    );
}

#[test]
fn pc_moving_shield_terminates_raise_and_skips_instruct_motion_edge() {
    use crate::element::{ActionState, Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let pc = engine.add_entity(make_test_pc(Posture::Upright));
    {
        let actor = engine.get_entity_mut(pc).unwrap().actor_data_mut().unwrap();
        actor.action_state = ActionState::MovingShield;
        actor.continuation.motion_state = MotionState::Start;
    }

    let sequence = engine.launch_element(SequenceElement::new(1, Command::RaiseShield, Some(pc)));
    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::Start,
        "PC-only terminal MovingShield branch must skip the accepted-instruct epilogue"
    );
}

#[test]
fn synchronous_redundant_stop_parry_skips_instruct_epilogue() {
    use crate::element::{ActionState, Command, Posture};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};
    use crate::sprite::MotionState;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    {
        let actor = engine
            .get_entity_mut(soldier)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.action_state = ActionState::WaitingSword;
        actor.continuation.motion_state = MotionState::Terminated;
    }

    let mut stop = SequenceElement::new(1, Command::StopParrySword, Some(soldier));
    stop.priority = SequencePriority::Wait;
    stop.posture_after_transition = Posture::Upright;
    let sequence = engine.launch_element(stop);
    engine
        .drain_script_synchronous_actions(&sim, &LevelAssets::default(), &mut Vec::new())
        .expect("synchronous redundant StopParrySword should dispatch");

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(soldier)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::Terminated
    );
}

#[test]
fn synchronous_redundant_quit_swordfight_skips_instruct_epilogue() {
    use crate::element::{ActionState, Command, Posture};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};
    use crate::sprite::MotionState;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_pc(Posture::Upright));
    {
        let actor = engine
            .get_entity_mut(soldier)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.action_state = ActionState::Waiting;
        actor.continuation.motion_state = MotionState::Terminated;
    }

    let mut quit = SequenceElement::new(1, Command::QuitSwordfight, Some(soldier));
    quit.priority = SequencePriority::Wait;
    quit.posture_after_transition = Posture::Upright;
    let sequence = engine.launch_element(quit);
    engine
        .drain_script_synchronous_actions(&sim, &assets_with_test_pc_profile(), &mut Vec::new())
        .expect("synchronous redundant QuitSwordfight should dispatch");

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(soldier)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::Terminated
    );
}

#[test]
fn manager_redundant_quit_swordfight_skips_instruct_epilogue() {
    use crate::element::{ActionState, Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    {
        let actor = engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.action_state = ActionState::Waiting;
        actor.continuation.motion_state = MotionState::Terminated;
    }

    let sequence = engine.launch_element(SequenceElement::new(
        1,
        Command::QuitSwordfight,
        Some(owner),
    ));
    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &assets_with_test_pc_profile(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("redundant quit remains inspectable")
            .state,
        SequenceState::Terminated
    );
    let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
    assert_eq!(actor.installed_order, None);
    assert_eq!(
        actor.continuation.motion_state,
        MotionState::Terminated,
        "redundant QuitSwordfight returns before Actor::Instruct's motion epilogue"
    );
}

#[test]
fn synchronous_self_seek_skips_instruct_epilogue() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceElementData, SequencePriority, SequenceState};
    use crate::sprite::MotionState;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(soldier)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .continuation
        .motion_state = MotionState::Terminated;

    let mut seek =
        SequenceElement::new_movement(1, Command::Seek, Some(soldier), OrderType::WalkingUpright);
    seek.priority = SequencePriority::Wait;
    seek.posture_after_transition = Posture::Upright;
    if let SequenceElementData::Movement { element, .. } = &mut seek.data {
        *element = Some(soldier);
    }
    let sequence = engine.launch_element(seek);
    engine
        .drain_script_synchronous_actions(&sim, &LevelAssets::default(), &mut Vec::new())
        .expect("synchronous self Seek should dispatch");

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(soldier)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::Terminated
    );
}

#[test]
fn synchronous_inactive_sword_damage_skips_instruct_epilogue() {
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequenceElementData, SequencePriority, SequenceState};
    use crate::sprite::MotionState;
    use crate::weapons::SwordStrike;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let attacker = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_soldier(Posture::Upright));
    {
        let victim_entity = engine.get_entity_mut(victim).unwrap();
        victim_entity.element_data_mut().active = false;
        victim_entity
            .actor_data_mut()
            .unwrap()
            .continuation
            .motion_state = MotionState::Terminated;
    }

    let mut damage = SequenceElement::new(1, Command::ReceiveSwordDamage, Some(victim));
    damage.data = SequenceElementData::new_sword_damage(attacker, SwordStrike::E, 0);
    damage.priority = SequencePriority::Wait;
    damage.posture_after_transition = Posture::Upright;
    let sequence = engine.launch_element(damage);
    engine
        .drain_script_synchronous_actions(&sim, &LevelAssets::default(), &mut Vec::new())
        .expect("synchronous inactive ReceiveSwordDamage should dispatch");

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    assert_eq!(
        engine
            .get_entity(victim)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::Terminated
    );
}

#[test]
fn entity_phase_completion_resumes_postponed_work_in_same_manager_drain() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));

    let mut blocker = SequenceElement::new_generic(1, Command::Generic, Some(owner));
    blocker.priority = SequencePriority::PostponeEverythingButInjuries;
    blocker.posture_after_transition = Posture::Upright;
    blocker.orders.push_back(Order::new(
        OrderType::TransitionRaisingSword,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    let blocker_sequence = engine.orders.sequence_manager.launch_element(blocker);
    // This fixture starts after instruction, at the actor-execution boundary.
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .element_in_progress(blocker_sequence, 0);

    let mut successor = SequenceElement::new_generic(1, Command::Generic, Some(owner));
    successor.priority = SequencePriority::Normal;
    successor.posture_after_transition = Posture::Upright;
    successor.orders.push_back(Order::new(
        OrderType::RunningWithSword,
        10.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    let successor_sequence = engine.orders.sequence_manager.launch_element(successor);
    // Consume the original launch registration: arbitration has postponed
    // this work behind the live blocker, so only blocker completion may
    // register it again.
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .postpone_element(successor_sequence, 0);
    engine
        .orders
        .sequence_manager
        .get_element_mut(blocker_sequence, 0)
        .unwrap()
        .cross_postponed = Some((successor_sequence, 0));

    // Actor execution ends before SequenceManager::Hourglass. The terminal
    // card is intentionally still pending when the sequence phase begins.
    engine
        .orders
        .sequence_manager
        .element_terminated(blocker_sequence, 0);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(successor_sequence, 0)
            .unwrap()
            .state,
        SequenceState::InProgress,
        "postponed work released by actor completion must be instructed by the same manager drain"
    );
}

#[test]
fn postponing_pathfinding_movement_restores_move_and_cancels_failure() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut movement = SequenceElement::new_movement(
        1,
        Command::MoveWaiting,
        Some(owner),
        OrderType::WalkingUpright,
    );
    movement.priority = SequencePriority::Normal;
    movement.orders.push_back(Order::new(
        OrderType::Freezing,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);
    engine.orders.failed_path_requests.push(
        crate::engine::movement::FailedPathRequest::from_pending(
            crate::engine::movement::PendingPathRequest::test_request(owner, movement_sequence, 0),
            0,
        ),
    );

    let mut blocker = SequenceElement::new(1, Command::LeaveAttentiveMode, Some(owner));
    blocker.priority = SequencePriority::PostponeEverythingButInjuries;
    let blocker_sequence = engine.orders.sequence_manager.launch_element(blocker);

    engine.engine_postpone(blocker_sequence, 0, movement_sequence, 0);

    let movement = engine
        .orders
        .sequence_manager
        .get_element(movement_sequence, 0)
        .expect("postponed movement remains registered");
    assert_eq!(movement.state, SequenceState::Postponed);
    assert_eq!(
        movement.command,
        Command::Move,
        "postponed MoveWaiting must be translated again when it resumes"
    );
    assert!(
        movement.orders.is_empty(),
        "postponed movement must discard its pathfinder freezing order"
    );
    assert!(
        engine.orders.failed_path_requests.is_empty(),
        "postponing MoveWaiting must cancel its pathfinder failure bookkeeping"
    );
}

#[test]
fn interrupted_postponed_successor_is_replaced_after_its_condolation() {
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut blocker = SequenceElement::new(1, Command::ReceiveSwordDamage, Some(owner));
    blocker.priority = SequencePriority::Injury;
    let blocker_sequence = engine.orders.sequence_manager.launch_element(blocker);
    engine
        .orders
        .sequence_manager
        .element_in_progress(blocker_sequence, 0);

    let mut existing = SequenceElement::new(1, Command::StopParrySword, Some(owner));
    existing.priority = SequencePriority::Preference;
    let existing_sequence = engine.orders.sequence_manager.launch_element(existing);
    engine
        .orders
        .sequence_manager
        .postpone_element(existing_sequence, 0);
    engine
        .orders
        .sequence_manager
        .get_element_mut(blocker_sequence, 0)
        .unwrap()
        .cross_postponed = Some((existing_sequence, 0));

    let mut waiter = SequenceElement::new(1, Command::WaitTimer, Some(owner));
    waiter.priority = SequencePriority::Normal;
    let waiter_sequence = engine.orders.sequence_manager.launch_element(waiter);

    engine.engine_postpone(blocker_sequence, 0, waiter_sequence, 0);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(waiter_sequence, 0)
            .unwrap()
            .state,
        SequenceState::Postponed
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(blocker_sequence, 0)
            .unwrap()
            .cross_postponed,
        None,
        "incoming waiter must stay invisible while the interrupted predecessor's card runs"
    );

    let mut pending = engine.orders.sequence_manager.drain_pending_condolations();
    assert_eq!(pending.len(), 1);
    engine
        .orders
        .sequence_manager
        .finish_pending_condolation(pending.remove(0));

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(blocker_sequence, 0)
            .unwrap()
            .cross_postponed,
        Some((waiter_sequence, 0)),
        "outer priority arbitration must install its waiter after the card returns"
    );
}

#[test]
fn fresh_group_route_translates_before_unexecuted_posture_recovery() {
    use crate::element::{Command, InstalledActorOrder, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{Sequence, SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));

    let speak = SequenceElement::new(1, Command::SpeakHeroReachDestination, Some(owner));
    let mut equip = SequenceElement::new(2, Command::EquipBow, Some(owner));
    equip.priority = SequencePriority::PostponeEverythingButInjuries;
    let equip_order = Order::new(
        OrderType::TransitionEquipBow,
        0.0,
        0.0,
        engine.orders.allocate_order_id(),
    );
    let equip_order_id = equip_order.order_id;
    equip.orders.push_back(equip_order);
    let mut recovery = Sequence::new();
    recovery.append_element(speak);
    recovery.append_element(equip);
    let recovery_id = engine.orders.sequence_manager.launch_sequence(recovery);
    engine
        .orders
        .sequence_manager
        .get_element_mut(recovery_id, 0)
        .unwrap()
        .state = SequenceState::Terminated;
    engine
        .orders
        .sequence_manager
        .element_in_progress(recovery_id, 1);
    {
        let actor = engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.installed_order = Some(InstalledActorOrder {
            order_id: equip_order_id,
            order_type: OrderType::TransitionEquipBow,
        });
        actor.last_execute_order_id = None;
    }

    let route_assert = SequenceElement::new_movement(
        1,
        Command::AssertPosition,
        Some(owner),
        OrderType::WalkingUpright,
    );
    let route_move =
        SequenceElement::new_movement(2, Command::Move, Some(owner), OrderType::WalkingUpright);
    let mut route = Sequence::new();
    route.append_element(route_assert);
    route.append_element(route_move);
    let route_id = engine.orders.sequence_manager.launch_sequence(route);
    engine
        .orders
        .sequence_manager
        .get_element_mut(route_id, 0)
        .unwrap()
        .state = SequenceState::Terminated;

    assert_eq!(
        engine.fresh_recovery_blocker_after_route_assert(owner, route_id, 1),
        Some((recovery_id, 1)),
        "a just-installed recovery must preserve Original's transient route request"
    );

    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .last_execute_order_id = Some(equip_order_id);
    assert_eq!(
        engine.fresh_recovery_blocker_after_route_assert(owner, route_id, 1),
        None,
        "an EquipBow that has already executed is a real blocker, not the same-drain FIFO seam"
    );
}

#[test]
fn postponing_resolved_movement_restores_untranslated_move() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut movement =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::WalkingUpright);
    movement.priority = SequencePriority::Normal;
    movement.orders.push_back(Order::new(
        OrderType::WalkingUpright,
        100.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);

    let mut blocker = SequenceElement::new(1, Command::LeaveAttentiveMode, Some(owner));
    blocker.priority = SequencePriority::PostponeEverythingButInjuries;
    let blocker_sequence = engine.orders.sequence_manager.launch_element(blocker);

    engine.engine_postpone(blocker_sequence, 0, movement_sequence, 0);

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(movement_sequence, 0)
            .expect("postponed movement remains registered")
            .command,
        Command::Move,
        "postponed MoveOk must discard its translated path and translate again on resume"
    );
}

#[test]
fn post_seek_handoff_clears_selected_movement_goal() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{Sequence, SequenceElement};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .position_iface_mut()
        .set_map_goal(crate::coordinates::MapPoint::new(70.0, 80.0));

    let mut post_seek = Sequence::new();
    post_seek.append_element(SequenceElement::new_generic(1, Command::Wait, Some(owner)));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .post_seek_sequence = Some(Box::new(post_seek));

    let seek =
        SequenceElement::new_movement(1, Command::Seek, Some(owner), OrderType::WalkingUpright);
    let seek_sequence = engine.orders.sequence_manager.launch_element(seek);
    engine
        .orders
        .sequence_manager
        .element_in_progress(seek_sequence, 0);

    assert!(engine.start_post_seek_sequence(
        &crate::sim_rng::test_context(),
        &LevelAssets::default(),
        owner,
        Some((seek_sequence, 0)),
    ));
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        crate::coordinates::MapPoint::ZERO,
        "SendCondolationCard clears the selected seek goal before post-seek launch"
    );
}

#[test]
fn post_seek_handoff_registers_parent_successor_before_post_seek_tail() {
    use crate::element::{Camp, Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{Sequence, SequenceElement};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let corpse = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));

    let mut parent = Sequence::new();
    parent.append_element(SequenceElement::new_movement(
        1,
        Command::MoveOk,
        Some(owner),
        OrderType::WalkingUpright,
    ));
    parent.append_element(SequenceElement::new_generic(
        2,
        Command::EnterHelpingClimb,
        Some(owner),
    ));
    let parent_id = engine.orders.sequence_manager.launch_sequence(parent);
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .element_in_progress(parent_id, 0);

    let mut post_seek = Sequence::new();
    post_seek.append_element(SequenceElement::new_interaction(
        1,
        Command::TakeCorpse,
        Some(owner),
        Some(corpse),
    ));
    engine
        .get_entity_mut(owner)
        .expect("post-seek owner remains live")
        .actor_data_mut()
        .expect("PC has actor state")
        .post_seek_sequence = Some(Box::new(post_seek));

    assert!(engine.start_post_seek_sequence(
        &crate::sim_rng::test_context(),
        &LevelAssets::default(),
        owner,
        Some((parent_id, 0)),
    ));

    let queued_commands: Vec<_> = engine
        .orders
        .sequence_manager
        .deferred_elements_to_go()
        .into_iter()
        .map(|(sequence_id, element_index)| {
            engine
                .orders
                .sequence_manager
                .get_element(sequence_id, element_index)
                .expect("deferred element remains registered")
                .command
        })
        .collect();
    assert_eq!(
        queued_commands,
        [Command::EnterHelpingClimb, Command::TakeCorpse],
        "Original Ready() registers the parent successor before StartPostSeekSequence launches its tail"
    );
}

#[test]
fn initial_seek_dispatch_clears_outgoing_movement_goal_until_first_execute() {
    use crate::element::{Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequenceElementData, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let stale_goal = MapPoint::new(70.0, 80.0);

    let mut outgoing =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::RunningUpright);
    outgoing.priority = SequencePriority::Normal;
    outgoing.posture_after_transition = Posture::Upright;
    outgoing.orders.push_back(Order::new(
        OrderType::RunningUpright,
        stale_goal.x,
        stale_goal.y,
        engine.orders.allocate_order_id(),
    ));
    let outgoing_sequence = engine.orders.sequence_manager.launch_element(outgoing);
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .element_in_progress(outgoing_sequence, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.actor_data_mut().unwrap().active_movement =
            ActiveMovement::new(outgoing_sequence, 0);
        let position = entity.position_iface_mut();
        position.set_move_box(crate::coordinates::MoveBox::from_coords(
            -5.0, -5.0, 5.0, 5.0,
        ));
        position.set_map_goal(stale_goal);
    }

    let new_goal = MapPoint::new(100.0, 0.0);
    let mut seek =
        SequenceElement::new_movement(1, Command::Seek, Some(owner), OrderType::WalkingUpright);
    seek.priority = SequencePriority::Normal;
    if let SequenceElementData::Movement {
        destination, flags, ..
    } = &mut seek.data
    {
        *destination = new_goal;
        *flags |= crate::sequence::MoveFlags::SEEK;
    } else {
        unreachable!("new_movement must produce movement data");
    }
    let seek_sequence = engine.orders.sequence_manager.launch_element(seek);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .position_iface()
            .map_goal(),
        MapPoint::ZERO,
        "Original interrupts its selected transient Seek before launching the concrete movement"
    );
    let transient_seek = engine
        .orders
        .sequence_manager
        .get_element(seek_sequence, 0)
        .expect("transient Seek wrapper remains inspectable");
    assert_eq!(transient_seek.state, SequenceState::Interrupted);

    let concrete_sequence = crate::sequence::SequenceId(seek_sequence.0 + 1);
    let concrete_seek = engine
        .orders
        .sequence_manager
        .get_element(concrete_sequence, 0)
        .expect("concrete seek movement should be launched separately");
    assert_eq!(concrete_seek.state, SequenceState::InProgress);
    assert_eq!(concrete_seek.command, Command::MoveOk);
    assert!(
        concrete_seek.current_order().is_some(),
        "the concrete movement is prepared, but its first Execute must install the new sprite goal"
    );
}

#[test]
fn same_building_entity_seek_keeps_replaced_movement_goal_when_translation_is_empty() {
    use crate::element::{Command, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{Order, OrderType};
    use crate::position_interface::SectorHandle;
    use crate::sequence::{SequenceElement, SequenceElementData, SequencePriority, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    install_test_building_sector(&mut engine, 42);
    let owner = engine.add_entity(make_test_civilian(Posture::Upright));
    let target = engine.add_entity(make_test_pc(Posture::Upright));
    let retained_goal = MapPoint::new(1137.9464, 490.93048);
    for entity in [owner, target] {
        engine
            .get_entity_mut(entity)
            .unwrap()
            .element_data_mut()
            .set_sector(SectorHandle::new(42));
    }

    let mut outgoing =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::RunningUpright);
    outgoing.priority = SequencePriority::Normal;
    outgoing.orders.push_back(Order::new(
        OrderType::RunningUpright,
        retained_goal.x,
        retained_goal.y,
        engine.orders.allocate_order_id(),
    ));
    let outgoing_sequence = engine.orders.sequence_manager.launch_element(outgoing);
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .element_in_progress(outgoing_sequence, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity.position_iface_mut().set_map_goal(retained_goal);
        let actor = entity.actor_data_mut().unwrap();
        actor.active_movement = ActiveMovement::new(outgoing_sequence, 0);
        actor.continuation.motion_state = MotionState::Terminated;
    }

    let mut seek =
        SequenceElement::new_movement(1, Command::Seek, Some(owner), OrderType::RunningUpright);
    seek.priority = SequencePriority::Normal;
    if let SequenceElementData::Movement { element, .. } = &mut seek.data {
        *element = Some(target);
    } else {
        unreachable!("new_movement must produce movement data");
    }
    let seek_sequence = engine.orders.sequence_manager.launch_element(seek);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    let outgoing = engine
        .orders
        .sequence_manager
        .get_element(outgoing_sequence, 0)
        .expect("replaced movement remains inspectable");
    assert_eq!(outgoing.state, SequenceState::Interrupted);
    let seek = engine
        .orders
        .sequence_manager
        .get_element(seek_sequence, 0)
        .expect("empty building Seek remains inspectable");
    assert_eq!(seek.command, Command::Move);
    assert_eq!(seek.state, SequenceState::Terminated);
    assert!(seek.orders.is_empty());
    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(entity.position_iface().map_goal(), retained_goal);
    assert_eq!(
        entity.actor_data().unwrap().continuation.motion_state,
        MotionState::InProgress
    );
}

#[test]
fn same_sector_seek_waiting_for_pass_door_installs_generated_transition() {
    use crate::element::{ActionState, Command, InstalledActorOrder, Posture};
    use crate::movement::ActiveMovement;
    use crate::order::{Order, OrderType};
    use crate::position_interface::SectorHandle;
    use crate::sequence::{SequenceElement, SequenceElementData, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let target = engine.add_entity(make_test_soldier(Posture::Upright));
    for entity in [owner, target] {
        engine
            .get_entity_mut(entity)
            .unwrap()
            .element_data_mut()
            .set_sector(SectorHandle::new(24));
    }

    // f4987 starts from the PC's selected bored Wait. Actor::Instruct
    // generates this exit transition before it interrupts that Wait
    // (`original-code/RHelementactor.cpp:1379-1458`). Install the same
    // owner-slot topology explicitly so this test isolates the subsequent
    // RefreshSeek(PassDoor) return.
    let old_order_id = engine.orders.allocate_order_id();
    let mut old_wait = SequenceElement::new(1, Command::Wait, Some(owner));
    old_wait.priority = SequencePriority::Wait;
    old_wait.orders.push_back(Order::new(
        OrderType::WaitingUprightBored,
        0.0,
        0.0,
        old_order_id,
    ));
    let old_sequence = engine.orders.sequence_manager.launch_element(old_wait);
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .element_in_progress(old_sequence, 0);
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        let actor = entity.actor_data_mut().unwrap();
        actor.action_state = ActionState::Bored;
        actor.installed_order = Some(InstalledActorOrder {
            order_id: old_order_id,
            order_type: OrderType::WaitingUprightBored,
        });
        entity.sprite_mut().last_processed_order_id = old_order_id.get();
    }

    let mut pass = SequenceElement::new_movement(
        1,
        Command::PassDoor,
        Some(target),
        OrderType::RunningUpright,
    );
    pass.priority = SequencePriority::NonInterruptable;
    let pass_sequence = engine.orders.sequence_manager.launch_element(pass);
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .element_in_progress(pass_sequence, 0);
    engine
        .get_entity_mut(target)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .active_movement = ActiveMovement::new(pass_sequence, 0);

    let transition = OrderType::TransitionWaitingUprightBoredWaitingUpright;
    let transition_order_id = engine.orders.allocate_order_id();
    let mut seek =
        SequenceElement::new_movement(1, Command::Seek, Some(owner), OrderType::WalkingUpright);
    seek.priority = SequencePriority::Normal;
    seek.posture_after_transition = Posture::Upright;
    seek.action_state_after_transition = ActionState::Waiting;
    seek.orders
        .push_back(Order::new(transition, 0.0, 0.0, transition_order_id));
    if let SequenceElementData::Movement {
        element, tolerance, ..
    } = &mut seek.data
    {
        *element = Some(target);
        *tolerance = 17.0;
    } else {
        unreachable!("new_movement must produce movement data");
    }
    let seek_sequence = engine.orders.sequence_manager.launch_element(seek);

    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut HostDisplayState::default(),
        &LevelAssets::new(),
    );

    let seek = engine
        .orders
        .sequence_manager
        .get_element(seek_sequence, 0)
        .expect("PassDoor-waiting Seek remains selected");
    assert_eq!(seek.command, Command::Move);
    assert_eq!(seek.state, SequenceState::InProgress);
    assert_eq!(seek.current_order().unwrap().order_type, transition);
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .installed_order,
        Some(InstalledActorOrder {
            order_id: transition_order_id,
            order_type: transition,
        }),
        "RefreshSeek's PassDoor return must preserve Actor::Instruct's generated transition"
    );
}

#[test]
fn different_building_rewritten_seek_keeps_its_refresh_order() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::position_interface::SectorHandle;
    use crate::sequence::{MoveFlags, SequenceElement, SequenceElementData, SequenceState};

    let mut engine = EngineInner::new();
    install_test_building_sector(&mut engine, 42);
    {
        let mut level = (*engine.world.fast_grid.level).clone();
        let mut other_building = level.sectors[0].clone();
        other_building.sector_number = crate::sector::SectorNumber::new(43);
        level
            .sector_number_map
            .insert(other_building.sector_number, 1);
        level.sectors.push(other_building);
        engine.world.fast_grid_mut().level = std::sync::Arc::new(level);
    }
    let owner = engine.add_entity(make_test_civilian(Posture::Upright));
    let target = engine.add_entity(make_test_pc(Posture::Upright));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_sector(SectorHandle::new(42));
    engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .set_sector(SectorHandle::new(43));

    let mut movement =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::RunningUpright);
    if let SequenceElementData::Movement { flags, element, .. } = &mut movement.data {
        *flags = MoveFlags::SEEK;
        *element = Some(target);
    } else {
        unreachable!("new_movement must produce movement data");
    }
    let sequence = engine.orders.sequence_manager.launch_element(movement);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    let movement = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("rewritten Seek remains inspectable");
    assert_eq!(movement.state, SequenceState::InProgress);
    assert_eq!(movement.orders.len(), 1);
    assert_eq!(
        movement.current_order().map(|order| order.order_type),
        Some(OrderType::RefreshingSeek)
    );
}

fn assert_refreshing_seek_owner_envelope_ignores_stale_sprite_motion(
    stale_sprite_motion: crate::sprite::MotionState,
) {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::position_interface::SectorHandle;
    use crate::sequence::{
        MoveFlags, Sequence, SequenceElement, SequenceElementData, SequenceState,
    };

    let mut engine = EngineInner::new();
    install_test_building_sector(&mut engine, 42);
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let start = MapPoint::new(10.0, 20.0);
    let target_position = MapPoint::new(30.0, 40.0);
    let target = engine.add_entity(make_test_pc(Posture::Upright));
    {
        let element = engine.get_entity_mut(owner).unwrap().element_data_mut();
        element.set_position_map(start);
        element.set_sector(SectorHandle::new(42));
    }
    {
        let element = engine.get_entity_mut(target).unwrap().element_data_mut();
        element.set_position_map(target_position);
        element.set_sector(SectorHandle::new(42));
    }
    let post_seek_order_id = engine.orders.allocate_order_id();
    let mut post_seek_element = SequenceElement::new_generic(1, Command::Wait, Some(owner));
    post_seek_element.orders.push_back(crate::order::Order::new(
        OrderType::WaitingUpright,
        0.0,
        0.0,
        post_seek_order_id,
    ));
    let mut post_seek = Sequence::new();
    post_seek.append_element(post_seek_element);
    {
        let actor = engine
            .get_entity_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap();
        actor.seek_target = Some(target);
        actor.seek_distance = 4.0;
        actor.post_seek_sequence = Some(Box::new(post_seek));
    }

    // Gate traversal rewrites the original Seek as a trailing Move while
    // retaining RHMOVE_SEEK. In a building, Original does not teleport or
    // terminate this final element: it installs RHNONANIMATION_REFRESHING_SEEK.
    let destination = MapPoint::new(100.0, 200.0);
    let mut movement =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    if let SequenceElementData::Movement {
        destination: stored,
        flags,
        element,
        ..
    } = &mut movement.data
    {
        *stored = destination;
        *flags = MoveFlags::SEEK | MoveFlags::SEEK_IN_BUILDINGS;
        *element = Some(target);
    } else {
        unreachable!("new_movement must produce movement data");
    }
    let sequence = engine.orders.sequence_manager.launch_element(movement);

    let mut display = HostDisplayState::default();
    engine.hourglass_phase_sequences(
        &crate::sim_rng::test_context(),
        &mut display,
        &LevelAssets::default(),
    );

    let movement = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("final seek movement remains selected");
    assert_eq!(movement.state, SequenceState::InProgress);
    assert_eq!(movement.orders.len(), 1);
    assert_eq!(
        movement.current_order().map(|order| order.order_type),
        Some(OrderType::RefreshingSeek)
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .position_map(),
        start,
        "final Move|SEEK must not take the ordinary building teleport branch"
    );

    engine
        .get_entity_mut(owner)
        .unwrap()
        .element_data_mut()
        .sprite
        .last_motion_state = Some(stale_sprite_motion);
    let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    engine.tick_actor_owner_envelopes(
        &crate::sim_rng::test_context(),
        &LevelAssets::default(),
        &positions,
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .position_map(),
        target_position,
        "the explicit RefreshingSeek order refreshes on the following owner slot"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::Terminated,
        "same-building SEEK_IN_BUILDINGS starts the attached post-seek tail"
    );
    let actor = engine.get_entity(owner).unwrap().actor_data().unwrap();
    assert_eq!(
        actor.continuation.motion_state,
        crate::sprite::MotionState::InProgress,
        "RefreshingSeek returns InProgress independently of the stale sprite edge"
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .sprite
            .last_motion_state,
        Some(stale_sprite_motion),
        "the non-animation RefreshingSeek arm must not fabricate a sprite motion"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner),
        None,
        "the attached post-seek sequence waits for the later manager phase"
    );
    let (replacement_sequence, replacement_order) = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .find_map(|candidate| {
            candidate.elements.first().and_then(|element| {
                (element.owner == Some(owner)
                    && element.command == Command::Wait
                    && element.state == SequenceState::Todo)
                    .then(|| element.current_order().map(|order| (candidate.id, order)))
                    .flatten()
            })
        })
        .expect("same-building refresh queues the attached post-seek sequence");
    assert_ne!(replacement_sequence, sequence);
    assert_eq!(replacement_order.order_id, post_seek_order_id);
    assert_eq!(replacement_order.order_type, OrderType::WaitingUpright);
}

#[test]
fn refreshing_seek_owner_envelope_ignores_stale_aborted_sprite_motion() {
    assert_refreshing_seek_owner_envelope_ignores_stale_sprite_motion(
        crate::sprite::MotionState::Aborted,
    );
}

#[test]
fn refreshing_seek_owner_envelope_ignores_stale_done_sprite_motion() {
    assert_refreshing_seek_owner_envelope_ignores_stale_sprite_motion(
        crate::sprite::MotionState::Done,
    );
}

#[test]
fn point_refreshing_seek_returns_terminated_without_refreshing() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let mut movement =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::RefreshingSeek);
    movement
        .orders
        .push_back(Order::test_new(OrderType::RefreshingSeek, 0.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(movement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    assert_eq!(
        engine.tick_refreshing_seek_for_owner(
            &crate::sim_rng::test_context(),
            &LevelAssets::default(),
            owner,
        ),
        Some(MotionState::Terminated)
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::InProgress,
        "derived Execute returns Terminated; base Actor completion retires the element"
    );
}

#[test]
fn point_refreshing_seek_with_successor_projects_back_to_in_progress() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequenceState};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    let refreshing_order_id = engine.orders.allocate_order_id();
    let successor_order_id = engine.orders.allocate_order_id();
    let mut refreshing =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::RefreshingSeek);
    refreshing.orders.push_back(Order::new(
        OrderType::RefreshingSeek,
        0.0,
        0.0,
        refreshing_order_id,
    ));
    refreshing.orders.push_back(Order::new(
        OrderType::WaitingUpright,
        0.0,
        0.0,
        successor_order_id,
    ));
    let sequence = engine.orders.sequence_manager.launch_element(refreshing);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
    engine.tick_actor_owner_envelopes(
        &crate::sim_rng::test_context(),
        &LevelAssets::default(),
        &positions,
    );

    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner),
        Some((sequence, 0)),
        "DoNextOrder keeps the same element selected when Proceed returns another order"
    );
    let successor = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("point RefreshingSeek retains its same-element successor");
    assert_eq!(
        successor.state,
        SequenceState::InProgress,
        "same-element Proceed must not retire the movement element"
    );
    assert_eq!(successor.command, Command::Move);
    assert_eq!(
        successor.current_order().map(|order| order.order_id),
        Some(successor_order_id)
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .continuation
            .motion_state,
        MotionState::InProgress
    );
}

#[test]
fn parry_sword_queues_transition_and_hold_orders() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(soldier)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::WaitingSword;

    let seq_id =
        engine
            .orders
            .sequence_manager
            .launch_element(crate::sequence::SequenceElement::new(
                1,
                Command::ParrySword,
                Some(soldier),
            ));
    engine.dispatch_parry_sword(soldier, false, seq_id, 0);

    let elem = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("parry element should remain live");
    assert_eq!(elem.state, crate::sequence::SequenceState::InProgress);
    assert_eq!(
        elem.orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![
            OrderType::TransitionWaitingSwordParryingSword,
            OrderType::ParryingSword,
        ]
    );
}

#[test]
fn waiting_parry_survives_normal_movement_successor_replacement() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_pc(Posture::Upright));
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::Waiting;

    let mut existing =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    existing.priority = SequencePriority::Normal;
    let existing_sequence = engine.orders.sequence_manager.launch_element(existing);
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .postpone_element(existing_sequence, 0);

    let mut parry = SequenceElement::new(1, Command::ParrySword, Some(owner));
    parry.priority = SequencePriority::Preference;
    parry.posture_after_transition = Posture::Upright;
    let parry_sequence = engine.orders.sequence_manager.launch_element(parry);
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .get_element_mut(parry_sequence, 0)
        .unwrap()
        .cross_postponed = Some((existing_sequence, 0));

    let mut incoming =
        SequenceElement::new_movement(1, Command::MoveOk, Some(owner), OrderType::WalkingUpright);
    incoming.priority = SequencePriority::Normal;
    for order_type in [
        OrderType::TransitionWaitingUprightWalkingUpright,
        OrderType::WalkingUpright,
        OrderType::TransitionWalkingUprightWaitingUpright,
    ] {
        incoming
            .orders
            .push_back(Order::test_new(order_type, 0.0, 0.0));
    }
    let incoming_sequence = engine.orders.sequence_manager.launch_element(incoming);
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .element_in_progress(incoming_sequence, 0);

    assert!(
        engine.arbitrate_instruct(parry_sequence, 0),
        "Preference ParrySword should displace the current Normal movement"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(existing_sequence, 0)
            .unwrap()
            .state,
        SequenceState::Interrupted,
        "the incoming Normal movement replaces the existing Normal successor"
    );
    let incoming = engine
        .orders
        .sequence_manager
        .get_element(incoming_sequence, 0)
        .unwrap();
    assert_eq!(incoming.state, SequenceState::Postponed);
    assert_eq!(incoming.command, Command::Move);
    assert!(incoming.orders.is_empty());
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(parry_sequence, 0)
            .unwrap()
            .cross_postponed,
        Some((incoming_sequence, 0))
    );

    engine.dispatch_parry_sword(owner, false, parry_sequence, 0);

    let parry = engine
        .orders
        .sequence_manager
        .get_element(parry_sequence, 0)
        .expect("ParrySword remains live after translation from Waiting");
    assert_eq!(parry.state, SequenceState::InProgress);
    assert_eq!(
        parry
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![
            OrderType::TransitionWaitingSwordParryingSword,
            OrderType::ParryingSword,
        ]
    );
}

#[test]
fn parry_sword_terminates_when_either_parry_is_already_active() {
    use crate::element::{ActionState, Command, Posture};

    for (action_state, command, low) in [
        (ActionState::ParryingSword, Command::ParrySword, false),
        (ActionState::ParryingSwordLow, Command::ParrySword, false),
        (ActionState::ParryingSword, Command::ParrySwordLow, true),
        (ActionState::ParryingSwordLow, Command::ParrySwordLow, true),
    ] {
        let mut engine = EngineInner::new();
        let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
        engine
            .get_entity_mut(soldier)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .action_state = action_state;

        let seq_id =
            engine
                .orders
                .sequence_manager
                .launch_element(crate::sequence::SequenceElement::new(
                    1,
                    command,
                    Some(soldier),
                ));
        engine.dispatch_parry_sword(soldier, low, seq_id, 0);

        let elem = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .expect("parry element remains available for its condolence");
        assert_eq!(
            elem.state,
            crate::sequence::SequenceState::Terminated,
            "{command:?} must terminate from {action_state:?}"
        );
        assert!(
            elem.orders.is_empty(),
            "an already-active parade must not receive another hold order"
        );
    }
}

#[test]
fn stop_parry_sword_queues_exit_transition() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let soldier = engine.add_entity(make_test_soldier(Posture::Upright));
    engine
        .get_entity_mut(soldier)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .action_state = ActionState::ParryingSword;

    let seq_id =
        engine
            .orders
            .sequence_manager
            .launch_element(crate::sequence::SequenceElement::new(
                1,
                Command::StopParrySword,
                Some(soldier),
            ));
    engine.dispatch_stop_parry(soldier, seq_id, 0);

    let elem = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("stop-parry element should remain live");
    assert_eq!(elem.state, crate::sequence::SequenceState::InProgress);
    assert_eq!(
        elem.orders
            .iter()
            .map(|order| order.order_type)
            .collect::<Vec<_>>(),
        vec![OrderType::TransitionParryingSwordWaitingSword]
    );
}

/// A LeaningOut soldier that receives a command requiring Upright
/// (e.g. `Move`) must snap to Upright and queue the
/// `TransitionLeaningOutWaitingAlerted` animation so the lean-out-
/// window unstick transition plays.
#[test]
fn soldier_leaning_out_to_upright_on_move() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::LeaningOut));

    let changed = engine.auto_leave_disguise_if_needed(soldier_id, Command::Move);
    assert!(changed, "auto-leave should fire for LeaningOut + Move");

    let entity = engine.get_entity(soldier_id).expect("soldier present");
    assert_eq!(
        entity.element_data().posture,
        Posture::Upright,
        "posture should snap to Upright"
    );

    let next_order = engine
        .orders
        .sequence_manager
        .current_order_for_actor(soldier_id)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        next_order,
        Some(OrderType::TransitionLeaningOutWaitingAlerted),
        "lean-out transition animation should be queued"
    );
}

/// An Upright soldier invoked with a posture-neutral command should
/// not be touched by `auto_leave_disguise_if_needed`.
#[test]
fn soldier_upright_move_skips_auto_leave() {
    use crate::element::{Command, Posture};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));

    let changed = engine.auto_leave_disguise_if_needed(soldier_id, Command::Move);
    assert!(!changed, "no transition needed for an Upright soldier");

    let entity = engine.get_entity(soldier_id).expect("soldier present");
    assert_eq!(entity.element_data().posture, Posture::Upright);
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(soldier_id)
            .is_none(),
        "no animation should be queued"
    );
}

#[test]
fn fresh_wait_replaces_pre_init_upright_idle_with_authored_sitting_idle() {
    use crate::element::{ActionState, Posture};
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    let mut display = HostDisplayState::default();
    let assets = LevelAssets::default();

    // Mission/script initialization can make the actor execute an upright
    // wait before AI InitState evaluates its authored initial animation.
    engine.actor_wait(owner);
    engine.hourglass_phase_sequences(&crate::sim_rng::test_context(), &mut display, &assets);

    {
        let actor = engine.get_entity_mut(owner).expect("soldier present");
        actor.set_posture(Posture::Sitting);
        actor.actor_data_mut().expect("actor data").action_state = ActionState::Waiting;
    }

    // RHArtificialIntelligence::InitState calls Wait again after SetStates.
    engine.actor_wait(owner);
    engine.hourglass_phase_sequences(&crate::sim_rng::test_context(), &mut display, &assets);

    let order = engine
        .orders
        .sequence_manager
        .current_order_for_actor(owner)
        .map(|(_, _, order)| order.order_type);
    assert_eq!(order, Some(OrderType::Sitting));
    assert_eq!(
        engine
            .get_entity(owner)
            .expect("soldier present")
            .element_data()
            .posture,
        Posture::Sitting
    );
}

#[test]
fn idle_wait_runs_while_future_owner_action_is_behind_ownerless_timer() {
    use crate::element::{Command, Posture};
    use crate::sequence::{
        Field, FieldValue, Sequence, SequenceElement, SequencePriority, SequenceState,
    };

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    // The officer-conversation regression leaves the actor idle after its
    // Turn while a later owned PlayAnim waits behind an ownerless Timer.
    // Original Actor::Hourglass sees no current order and launches its
    // low-priority Wait; future command levels do not suppress that idle.
    let mut sequence = Sequence::new();
    let mut timer = SequenceElement::new_generic(1, Command::Timer, None);
    timer.set_property(Field::Timer, FieldValue::Integer(50));
    sequence.append_element(timer);
    sequence.append_element(SequenceElement::new(2, Command::PlayAnim, Some(owner)));
    let scripted_sequence = engine.orders.sequence_manager.launch_sequence(sequence);

    let future = engine
        .orders
        .sequence_manager
        .get_element(scripted_sequence, 1)
        .expect("future actor action exists");
    assert_eq!(future.state, SequenceState::Todo);
    assert!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
            .is_none(),
        "a future command level is not the actor's current order"
    );

    engine.ensure_wait_element(owner);
    let mut display = HostDisplayState::default();
    let assets = LevelAssets::default();
    engine.hourglass_phase_sequences(&crate::sim_rng::test_context(), &mut display, &assets);

    let (wait_sequence, wait_index) = engine
        .orders
        .sequence_manager
        .current_element_for_actor(owner)
        .expect("idle actor must execute a default Wait");
    let wait = engine
        .orders
        .sequence_manager
        .get_element(wait_sequence, wait_index)
        .expect("default Wait remains live");
    assert_eq!(wait.command, Command::Wait);
    assert_eq!(wait.priority, SequencePriority::Wait);
    assert_eq!(wait.state, SequenceState::InProgress);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(scripted_sequence, 1)
            .expect("future actor action remains queued")
            .state,
        SequenceState::Todo
    );
}

#[test]
fn transition_resumed_pass_door_reach_event_obeys_real_action_followers() {
    use crate::element::Command;
    use crate::order::OrderType;
    use crate::sequence::{Sequence, SequenceElement};

    let capture = |with_move_follower: bool| {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
        let mut assets = LevelAssets::new();
        complete_test_runtime_fixture(&mut engine, &mut assets);

        let mut route = Sequence::new();
        route.append_element(SequenceElement::new_movement(
            1,
            Command::PassDoor,
            Some(owner),
            OrderType::WalkingUpright,
        ));
        if with_move_follower {
            route.append_element(SequenceElement::new_movement(
                2,
                Command::AssertPosition,
                Some(owner),
                OrderType::WalkingUpright,
            ));
            route.append_element(SequenceElement::new_movement(
                3,
                Command::Move,
                Some(owner),
                OrderType::WalkingUpright,
            ));
        }
        let route_id = engine.orders.sequence_manager.launch_sequence(route);
        engine
            .orders
            .sequence_manager
            .element_in_progress(route_id, 0);
        engine
            .orders
            .sequence_manager
            .element_terminated(route_id, 0);

        let sim = crate::sim_rng::test_context();
        let ((), stimuli) = crate::engine::soldier_helpers::capture_condolation_stimuli(|| {
            engine.dispatch_condolations_for_owner_boundary(&sim, owner, &assets);
        });
        stimuli
            .into_iter()
            .filter(|(event_owner, stimulus)| {
                *event_owner == owner && *stimulus == crate::ai::StimulusType::EventReachPoint
            })
            .count()
    };

    assert_eq!(
        capture(true),
        0,
        "PassDoor -> AssertPosition -> Move must not report the door as the route endpoint"
    );
    assert_eq!(
        capture(false),
        1,
        "a route-terminal PassDoor must report exactly one EventReachPoint through condolence"
    );
}

#[test]
fn play_anim_uses_custom_wrapper_instead_of_requested_animation_semantics() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{Field, FieldValue, SequenceElement, SequenceState};

    for (command, wrapper) in [
        (Command::PlayAnim, OrderType::PlayCustom),
        (Command::PlayAnimLoop, OrderType::PlayCustomLooped),
        (Command::PlayAnimFreeze, OrderType::PlayCustomFreeze),
        (Command::PlayAnimFrozen, OrderType::PlayCustomFrozen),
    ] {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_test_soldier(Posture::Upright));
        engine
            .get_entity_mut(owner)
            .expect("test soldier exists")
            .actor_data_mut()
            .expect("test soldier is an actor")
            .action_state = ActionState::Bored;

        let mut element = SequenceElement::new_generic(1, command, Some(owner));
        element.set_property(
            Field::AnimationId,
            FieldValue::Animation(OrderType::Pointing),
        );
        let sequence = engine.orders.sequence_manager.launch_element(element);
        let mut display = HostDisplayState::default();
        engine.hourglass_phase_sequences(
            &crate::sim_rng::test_context(),
            &mut display,
            &LevelAssets::default(),
        );

        let element = engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .expect("PlayAnim element remains live");
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(
            element.current_order().map(|order| order.order_type),
            Some(wrapper),
            "Pointing is only the requested sprite animation; Original executes the command wrapper"
        );
        let actor = engine
            .get_entity(owner)
            .expect("test soldier remains")
            .actor_data()
            .expect("test soldier remains an actor");
        assert_eq!(
            actor.action_state,
            ActionState::Bored,
            "translating custom Pointing must not apply Pointing's Waiting state"
        );
        assert_eq!(
            actor.continuation.motion_state,
            crate::sprite::MotionState::InProgress,
            "accepted {command:?} must project Actor::Instruct's IN_PROGRESS edge"
        );
    }
}

/// An attentive-mode transition on an idle soldier queues
/// `TransitionWaitingUprightWaitingAlerted` as an order on the
/// sequence element.
#[test]
fn soldier_enter_attentive_mode_queues_transition_anim() {
    let mut display = HostDisplayState::default();
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));

    // Launch the EnterAttentiveMode element first; `ensure_wait_element`
    // is a no-op once another live element exists for the actor.  This
    // matches level-load ordering: spawn → (maybe scripted elements) →
    // ensure_wait_element covers only the actors left idle.
    // Stamp `posture_after_transition = Upright` at launch.
    let mut elem = SequenceElement::new(1, Command::EnterAttentiveMode, Some(soldier_id));
    elem.posture_after_transition = Posture::Upright;
    engine.launch_element(elem);
    engine.ensure_wait_element(soldier_id);

    let mut assets = LevelAssets::default();
    let mut dev = crate::engine::DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let active = engine
        .orders
        .sequence_manager
        .current_order_for_actor(soldier_id)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        active,
        Some(OrderType::TransitionWaitingUprightWaitingAlerted),
        "the transition order should be the front of the actor's current element"
    );
}

/// Regression: calling `set_soldier_attentive_mode` on an Upright
/// soldier (the path real game code hits via `pending_set_attentive_mode`
/// when an enemy spots the PC) must queue the alerted-transition
/// animation.  The previous bug left
/// `SequenceElement::posture_after_transition` at `Posture::Undefined`
/// because only `ensure_wait_element` and `auto_leave_disguise_if_needed`
/// stamped it; `arbitrate_instruct` now stamps it unconditionally
/// (`set_posture_after_transition(get_posture())`).
#[test]
fn set_soldier_attentive_mode_plays_transition_from_upright() {
    let mut display = HostDisplayState::default();
    use crate::element::Posture;
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));

    // Drive the engine-side helper the way the AI does it — no explicit
    // posture stamping; arbitrate_instruct must supply it.  Launch the
    // attentive element before `ensure_wait_element` so the latter
    // no-ops (matching the AI drain ordering in `tick_enemy_ai` where
    // `set_soldier_attentive_mode` fires from the per-NPC pending drain
    // and only actors left idle get a Wait element).
    engine.set_soldier_attentive_mode(soldier_id, true, false);
    engine.ensure_wait_element(soldier_id);

    let mut assets = LevelAssets::default();
    let mut dev = crate::engine::DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let active = engine
        .orders
        .sequence_manager
        .current_order_for_actor(soldier_id)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        active,
        Some(OrderType::TransitionWaitingUprightWaitingAlerted),
        "transition-to-alerted animation should be the actor's current order"
    );
}

#[test]
fn deferred_attentive_then_forget_preserves_launch_but_clears_local_flags() {
    use crate::ai::AttentiveModeEffect;
    use crate::element::{AiBrain, Command, Posture};

    for forget_after in [false, true] {
        let sim = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        let mut entity = make_test_soldier(Posture::Upright);
        let Entity::Soldier(soldier) = &mut entity else {
            unreachable!();
        };
        soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
        let enemy = soldier.npc.ai_brain.enemy_mut().unwrap();
        enemy.forced_attentive = true;
        enemy.attentive = false;
        enemy.will_be_attentive = false;
        let mut request = AttentiveModeEffect::new(true, false);
        request.forget_after = forget_after;
        enemy.base.outbox.actor.set_attentive_mode = Some(request);
        let soldier_id = engine.add_entity(entity);

        engine.drain_pending_for_npc(&sim, soldier_id, &LevelAssets::default());

        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| sequence.elements.iter())
                .any(|element| element.command == Command::EnterAttentiveMode),
            "SetState's earlier SetAttentiveMode call must still launch"
        );
        let enemy = engine.get_entity(soldier_id).unwrap().enemy_ai().unwrap();
        assert_eq!(enemy.will_be_attentive, !forget_after);
        assert!(!enemy.attentive);
        assert!(enemy.forced_attentive);
    }
}

#[test]
fn set_soldier_attentive_mode_plays_transition_while_movement_is_postponed() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut movement = SequenceElement::new_movement(
        1,
        Command::MoveOk,
        Some(soldier_id),
        OrderType::RunningUpright,
    );
    movement.priority = SequencePriority::Normal;
    movement.orders.push_back(Order::new(
        OrderType::RunningUpright,
        100.0,
        0.0,
        engine.orders.allocate_order_id(),
    ));
    let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
    let _ = engine.orders.sequence_manager.hourglass();
    engine
        .orders
        .sequence_manager
        .element_in_progress(movement_sequence, 0);

    engine.set_soldier_attentive_mode(soldier_id, true, false);

    // The attentive element is only registered with the manager here; its
    // Instruct (and the movement postpone it causes) runs at the next
    // manager hourglass, matching the deferred launch semantics.
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(movement_sequence, 0)
            .expect("movement remains registered")
            .state,
        SequenceState::InProgress
    );

    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::default();
    let mut dev = crate::engine::DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let movement = engine
        .orders
        .sequence_manager
        .get_element(movement_sequence, 0)
        .expect("postponed movement remains registered");
    assert_eq!(movement.state, SequenceState::Postponed);
    assert_eq!(movement.command, Command::Move);
    assert!(movement.orders.is_empty());

    // The attentive element's transition generation must first stop the
    // running actor (its action state exits MOVING before entering the
    // alerted stance), so the stop transition fronts the order queue with
    // the alerted transition queued behind it.
    let (attentive_seq, attentive_idx, front) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(soldier_id)
        .expect("attentive element should be current after the postpone");
    assert_eq!(
        front.order_type,
        OrderType::TransitionRunningUprightWaitingUpright
    );
    let attentive_orders: Vec<OrderType> = engine
        .orders
        .sequence_manager
        .get_element(attentive_seq, attentive_idx)
        .expect("attentive element remains registered")
        .orders
        .iter()
        .map(|order| order.order_type)
        .collect();
    assert!(
        attentive_orders.contains(&OrderType::TransitionWaitingUprightWaitingAlerted),
        "postponing a movement must not suppress the attentive transition, got {attentive_orders:?}",
    );
}

#[test]
fn leave_attentive_translation_keeps_transition_after_attentive_was_already_cleared() {
    use crate::element::{Camp, Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    engine
        .get_entity_mut(soldier_id)
        .unwrap()
        .set_posture(Posture::Upright);
    let sequence = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(
            1,
            Command::LeaveAttentiveMode,
            Some(soldier_id),
        ));
    engine
        .orders
        .sequence_manager
        .get_element_mut(sequence, 0)
        .expect("leave element remains registered")
        .posture_after_transition = Posture::Upright;

    let barrier = crate::engine::sequence_runtime::NpcAttentionCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        next_order_id: &mut engine.orders.next_order_id,
    }
    .dispatch(soldier_id, Command::LeaveAttentiveMode, sequence, 0);

    assert_eq!(
        barrier,
        crate::engine::sequence_runtime::OwnerActionBarrier::Reach
    );
    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("leave element remains registered");
    assert_eq!(element.state, SequenceState::InProgress);
    assert_eq!(element.orders.len(), 1);
    assert_eq!(
        element.orders.front().unwrap().order_type,
        OrderType::TransitionWaitingAlertedWaitingUpright
    );

    let non_upright = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(
            1,
            Command::LeaveAttentiveMode,
            Some(soldier_id),
        ));
    engine
        .orders
        .sequence_manager
        .get_element_mut(non_upright, 0)
        .expect("non-upright leave remains registered")
        .posture_after_transition = Posture::Crouched;
    let barrier = crate::engine::sequence_runtime::NpcAttentionCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        next_order_id: &mut engine.orders.next_order_id,
    }
    .dispatch(soldier_id, Command::LeaveAttentiveMode, non_upright, 0);
    assert_eq!(
        barrier,
        crate::engine::sequence_runtime::OwnerActionBarrier::Skip
    );
    let element = engine
        .orders
        .sequence_manager
        .get_element(non_upright, 0)
        .expect("non-upright leave remains registered");
    assert_eq!(element.state, SequenceState::Terminated);
    assert!(element.orders.is_empty());
}

#[test]
fn enter_attentive_translation_still_suppresses_an_already_satisfied_enter() {
    use crate::element::{Camp, Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_ai_soldier(Camp::Lacklandists));
    engine
        .get_entity_mut(soldier_id)
        .unwrap()
        .set_posture(Posture::Upright);
    engine
        .get_entity_mut(soldier_id)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .attentive = true;
    let sequence = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(
            1,
            Command::EnterAttentiveMode,
            Some(soldier_id),
        ));
    engine
        .orders
        .sequence_manager
        .get_element_mut(sequence, 0)
        .expect("enter element remains registered")
        .posture_after_transition = Posture::Upright;

    let barrier = crate::engine::sequence_runtime::NpcAttentionCommandContext {
        entities: &mut engine.world.entities,
        sequence_manager: &mut engine.orders.sequence_manager,
        next_order_id: &mut engine.orders.next_order_id,
    }
    .dispatch(soldier_id, Command::EnterAttentiveMode, sequence, 0);

    assert_eq!(
        barrier,
        crate::engine::sequence_runtime::OwnerActionBarrier::Skip
    );
    let element = engine
        .orders
        .sequence_manager
        .get_element(sequence, 0)
        .expect("enter element remains registered");
    assert_eq!(element.state, SequenceState::Terminated);
    assert!(element.orders.is_empty());
}

#[test]
fn arbitration_ignores_serialized_order_ai_lock_like_original() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut current = SequenceElement::new(1, Command::Move, Some(owner));
    current.priority = SequencePriority::Normal;
    current
        .orders
        .push_back(Order::test_new(OrderType::WalkingUpright, 10.0, 0.0));
    current
        .orders
        .push_back(Order::test_new(OrderType::WalkingUpright, 20.0, 0.0));
    current.orders.front_mut().unwrap().lock_ai = true;
    let current_seq = engine.orders.sequence_manager.launch_element(current);
    engine
        .orders
        .sequence_manager
        .element_in_progress(current_seq, 0);

    let mut incoming = SequenceElement::new(1, Command::Turn, Some(owner));
    incoming.priority = SequencePriority::Preference;
    let incoming_seq = engine.orders.sequence_manager.launch_element(incoming);

    let accepted = engine.arbitrate_instruct(incoming_seq, 0);
    assert!(
        accepted,
        "Original CanInterruptNow always accepts a live current order"
    );

    let current = engine
        .orders
        .sequence_manager
        .get_element(current_seq, 0)
        .unwrap();
    assert_eq!(current.state, SequenceState::Postponed);
    assert!(
        current.orders.is_empty(),
        "Original postponement discards the translated current order chain"
    );

    let incoming = engine
        .orders
        .sequence_manager
        .get_element(incoming_seq, 0)
        .unwrap();
    assert_eq!(incoming.state, SequenceState::Todo);
    assert_eq!(incoming.cross_postponed, Some((current_seq, 0)));
}

#[test]
fn duplicate_instruct_does_not_arbitrate_an_element_against_itself() {
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    let mut element = SequenceElement::new(1, Command::Move, Some(owner));
    element.priority = SequencePriority::Normal;
    let sequence = engine.orders.sequence_manager.launch_element(element);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    assert!(engine.arbitrate_instruct(sequence, 0));
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(sequence, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );
}

#[test]
fn interrupt_callback_arbitrates_nested_work_against_incoming_selection() {
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut outgoing = SequenceElement::new(1, Command::SwordstrikeSmalltalkLeft, Some(owner));
    outgoing.priority = SequencePriority::Wait;
    // Interrupt arbitration requires the in-progress element to carry its
    // current order, mirroring the assertion in the original manager.
    outgoing.orders.push_back(crate::order::Order::test_new(
        crate::order::OrderType::WaitingUpright,
        0.0,
        0.0,
    ));
    let outgoing_sequence = engine.orders.sequence_manager.launch_element(outgoing);
    engine
        .orders
        .sequence_manager
        .element_in_progress(outgoing_sequence, 0);

    let mut incoming = SequenceElement::new(1, Command::ReceiveSwordDamage, Some(owner));
    incoming.priority = SequencePriority::Injury;
    let incoming_sequence = engine.orders.sequence_manager.launch_element(incoming);
    assert!(engine.arbitrate_instruct(incoming_sequence, 0));

    engine
        .orders
        .sequence_manager
        .begin_instruct_callback(owner, incoming_sequence, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner),
        Some((incoming_sequence, 0)),
        "the outgoing SetState callback must observe incoming injury as selected"
    );

    let mut nested = SequenceElement::new(1, Command::Turn, Some(owner));
    nested.priority = SequencePriority::Normal;
    let nested_sequence = engine.orders.sequence_manager.launch_element(nested);
    assert!(
        !engine.arbitrate_instruct(nested_sequence, 0),
        "recursive normal work must arbitrate against the selected injury"
    );
    assert_ne!(
        engine
            .orders
            .sequence_manager
            .get_element(nested_sequence, 0)
            .unwrap()
            .state,
        SequenceState::InProgress
    );

    assert!(
        engine
            .orders
            .sequence_manager
            .end_instruct_callback(owner, incoming_sequence, 0),
        "rejected recursive work must not supersede the incoming selection"
    );
}

#[test]
fn nested_instruct_callback_permanently_supersedes_its_parent_selection() {
    use crate::element::{Command, Posture};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    let outer = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(
            1,
            Command::QuitSwordfight,
            Some(owner),
        ));
    let nested = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::LookLeft, Some(owner)));

    engine
        .orders
        .sequence_manager
        .begin_instruct_callback(owner, outer, 0);
    engine
        .orders
        .sequence_manager
        .begin_instruct_callback(owner, nested, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner),
        Some((nested, 0))
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .end_instruct_callback(owner, nested, 0),
        "the recursive selection itself remains current"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_element_for_actor(owner),
        None,
        "returning from recursive Instruct must not restore the overwritten parent pointer"
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .end_instruct_callback(owner, outer, 0),
        "the outer Instruct must detect that recursive work superseded it"
    );
}

#[test]
fn done_propagation_requires_the_current_order_identity() {
    use crate::element::{Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{CascadeFlags, SequenceElement};
    use crate::sprite::MotionState;

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let stale_order_id = engine.orders.allocate_order_id();
    let mut interrupted = SequenceElement::new_generic(1, Command::Generic, Some(owner));
    interrupted.orders.push_back(Order::new(
        OrderType::WaitingUpright,
        0.0,
        0.0,
        stale_order_id,
    ));
    let interrupted_sequence = engine.orders.sequence_manager.launch_element(interrupted);
    engine
        .orders
        .sequence_manager
        .element_in_progress(interrupted_sequence, 0);
    engine.orders.sequence_manager.element_interrupted(
        interrupted_sequence,
        0,
        CascadeFlags::empty(),
    );

    let replacement_order_id = engine.orders.allocate_order_id();
    let mut replacement = SequenceElement::new_generic(1, Command::Generic, Some(owner));
    replacement.orders.push_back(Order::new(
        OrderType::WaitingAlerted,
        0.0,
        0.0,
        replacement_order_id,
    ));
    let replacement_sequence = engine.orders.sequence_manager.launch_element(replacement);
    engine
        .orders
        .sequence_manager
        .element_in_progress(replacement_sequence, 0);

    {
        let sprite = &mut engine
            .get_entity_mut(owner)
            .unwrap()
            .element_data_mut()
            .sprite;
        sprite.last_motion_state = Some(MotionState::Done);
        sprite.last_processed_order_id = stale_order_id.get();
    }
    engine.propagate_done_to_current_orders();

    assert!(
        !engine
            .orders
            .sequence_manager
            .get_element(replacement_sequence, 0)
            .unwrap()
            .current_order()
            .unwrap()
            .done,
        "a stale Done reported by the interrupted order must not complete its replacement"
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .element_data()
            .sprite
            .last_motion_state,
        None,
        "the stale transient result must still be consumed"
    );

    {
        let sprite = &mut engine
            .get_entity_mut(owner)
            .unwrap()
            .element_data_mut()
            .sprite;
        sprite.last_motion_state = Some(MotionState::Done);
        sprite.last_processed_order_id = replacement_order_id.get();
    }
    engine.propagate_done_to_current_orders();

    assert!(
        engine
            .orders
            .sequence_manager
            .get_element(replacement_sequence, 0)
            .unwrap()
            .current_order()
            .unwrap()
            .done,
        "Done from the currently dispatched order ID must still propagate"
    );
}

#[test]
fn pc_shoot_bow_waits_through_load_and_wait_then_retries_only_while_aiming() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let sim = crate::sim_rng::test_context();
    let assets = crate::engine::LevelAssets::new();
    let mut engine = EngineInner::new();
    let pc = engine.add_entity(make_test_pc(Posture::Upright));
    engine
        .get_entity_mut(pc)
        .unwrap()
        .element_data_mut()
        .sprite
        .last_action = OrderType::TransitionLoadingBow;

    // A missing antagonist makes the eventual Translate deterministic and
    // side-effect free; this regression is about Human::Instruct admission,
    // not projectile construction.
    let incoming = SequenceElement::new_interaction(1, Command::ShootBow, Some(pc), None);
    let incoming_seq = engine.launch_element_for_owner(&sim, &assets, incoming);

    let held = engine
        .orders
        .sequence_manager
        .get_element(incoming_seq, 0)
        .unwrap();
    assert_eq!(held.state, SequenceState::Todo);
    assert_eq!(held.priority, SequencePriority::NotYetSet);
    assert_eq!(held.posture_after_transition, Posture::Undefined);
    assert_eq!(held.action_state_after_transition, ActionState::Waiting);
    assert!(held.orders.is_empty());
    assert_eq!(held.cross_postponed, None);
    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .human_data()
            .unwrap()
            .pending_shoots,
        [crate::sequence::SequenceElementRef::new(incoming_seq, 0)]
    );

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::default();

    // Loading has reported DONE, but the sprite still names the completed
    // loading animation. The following Wait frame is not sufficient either.
    engine.process_shoot_list_for(&sim, &assets, pc);
    engine
        .get_entity_mut(pc)
        .unwrap()
        .element_data_mut()
        .sprite
        .last_action = OrderType::WaitingUpright;
    engine.process_shoot_list_for(&sim, &assets, pc);
    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .human_data()
            .unwrap()
            .pending_shoots
            .len(),
        1
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(incoming_seq, 0)
            .unwrap()
            .priority,
        SequencePriority::NotYetSet
    );

    // Only the bow-aiming idle admits the retained element. Instruct then
    // reaches Translate and consumes the FIFO entry (the deliberately absent
    // target makes this particular element Impossible).
    engine
        .get_entity_mut(pc)
        .unwrap()
        .element_data_mut()
        .sprite
        .last_action = OrderType::AimingWithBow;
    engine.process_shoot_list_for(&sim, &assets, pc);
    assert!(
        engine
            .get_entity(pc)
            .unwrap()
            .human_data()
            .unwrap()
            .pending_shoots
            .is_empty()
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(incoming_seq, 0)
            .unwrap()
            .state,
        SequenceState::Impossible
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(incoming_seq, 0)
            .unwrap()
            .priority,
        SequencePriority::Normal,
        "the retained shot must run Actor::DeterminePriority when readmitted"
    );
}

#[test]
fn pc_shoot_list_readmits_retained_terminated_element() {
    use crate::element::{ActionState, Command, Posture};
    use crate::order::{Order, OrderType};
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::default();
    let mut engine = EngineInner::new();
    let pc = engine.add_entity(make_test_pc(Posture::Upright));
    engine
        .get_entity_mut(pc)
        .unwrap()
        .element_data_mut()
        .sprite
        .last_action = OrderType::TransitionLoadingBow;

    let incoming = SequenceElement::new_interaction(1, Command::ShootBow, Some(pc), None);
    let incoming_seq = engine.launch_element_for_owner(&sim, &assets, incoming);
    engine
        .orders
        .sequence_manager
        .element_terminated(incoming_seq, 0);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(incoming_seq, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );

    // Model the first fresh bow-Wait slot after the retained shot terminated.
    // Original serializes this as command Wait with animation AimingWithBow.
    let wait_order_id = engine.orders.allocate_order_id();
    let mut wait = SequenceElement::new_generic(1, Command::Wait, Some(pc));
    wait.priority = SequencePriority::Wait;
    wait.posture_after_transition = Posture::Upright;
    wait.action_state_after_transition = ActionState::AimingWithBow;
    wait.orders.push_back(Order::new(
        OrderType::AimingWithBow,
        0.0,
        0.0,
        wait_order_id,
    ));
    let wait_seq = engine.orders.sequence_manager.launch_element(wait);
    engine
        .orders
        .sequence_manager
        .element_in_progress(wait_seq, 0);
    bind_test_action_point(
        &mut engine,
        pc,
        OrderType::AimingWithBow,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );
    {
        let entity = engine.get_entity_mut(pc).unwrap();
        entity.actor_data_mut().unwrap().action_state = ActionState::AimingWithBow;
        let sprite = entity.sprite_mut();
        sprite.last_action = OrderType::AimingWithBow;
        sprite.last_processed_order_id = wait_order_id.get();
        sprite.frame_count = u16::MAX;
    }

    // Retail drops the entry assertion and therefore re-enters Instruct, but
    // its post-GenerateTransition terminal-state guard returns false. The
    // retained pointer stays queued and, crucially, the live Wait is not
    // interrupted and recreated with a fresh order ID.
    engine.process_shoot_list_for(&sim, &assets, pc);

    assert_eq!(
        engine
            .get_entity(pc)
            .unwrap()
            .human_data()
            .unwrap()
            .pending_shoots,
        [crate::sequence::SequenceElementRef::new(incoming_seq, 0)],
        "the post-transition terminal guard must reject and retain the pointer"
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(incoming_seq, 0)
            .unwrap()
            .state,
        SequenceState::Terminated
    );
    let (selected_seq, _, selected_order) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(pc)
        .expect("the live bow Wait must remain selected");
    assert_eq!(selected_seq, wait_seq);
    assert_eq!(selected_order.order_id, wait_order_id);

    let retained = std::collections::BTreeSet::from([incoming_seq]);
    engine
        .orders
        .sequence_manager
        .friday_evening_cleanup_preserving(&retained);
    assert!(
        engine
            .orders
            .sequence_manager
            .get_element(incoming_seq, 0)
            .is_some(),
        "Friday cleanup must preserve the backing allocation of a retained raw shoot pointer"
    );

    let (_, _, result) = engine.tick_actor_animation_for(&sim, &assets, pc);
    assert_eq!(
        result.unwrap().motion,
        crate::sprite::MotionState::InProgress
    );
    assert_eq!(
        engine.get_entity(pc).unwrap().sprite().frame_count,
        0,
        "the second Wait tick must increment the START sentinel instead of restarting it"
    );
}

#[test]
fn started_pass_door_rejects_new_move() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let sim = crate::sim_rng::test_context();
    let assets = crate::engine::LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut current_pass =
        SequenceElement::new_movement(1, Command::PassDoor, Some(owner), OrderType::WalkingUpright);
    current_pass.priority = SequencePriority::NonInterruptable;
    let pass_seq = engine.orders.sequence_manager.launch_element(current_pass);
    engine
        .orders
        .sequence_manager
        .element_in_progress(pass_seq, 0);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sequence_element_started = true;

    let incoming =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let incoming_seq = engine.launch_element_for_owner(&sim, &assets, incoming);

    let pass = engine
        .orders
        .sequence_manager
        .get_element(pass_seq, 0)
        .unwrap();
    assert_eq!(pass.state, SequenceState::InProgress);

    let incoming = engine
        .orders
        .sequence_manager
        .get_element(incoming_seq, 0)
        .unwrap();
    assert_eq!(incoming.state, SequenceState::Impossible);
}

#[test]
fn parity_pass_door_snapshot_reads_selected_movement_without_runtime_latch() {
    use crate::element::{Command, Posture};
    use crate::gate::DoorIndex;
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceElementData};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));
    let mut pass =
        SequenceElement::new_movement(1, Command::PassDoor, Some(owner), OrderType::WalkingUpright);
    let SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut pass.data
    else {
        unreachable!("new_movement must create movement data")
    };
    *gate_id = Some(DoorIndex(51));
    *direction = -1;
    let sequence = engine.orders.sequence_manager.launch_element(pass);
    engine
        .orders
        .sequence_manager
        .element_in_progress(sequence, 0);

    assert!(
        engine
            .get_entity(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_door_pass
            .is_none(),
        "loaded selected PassDoor fixtures need not reconstruct physical choreography"
    );
    assert_eq!(
        engine.actor_selected_pass_door(owner),
        Some((DoorIndex(51), -1))
    );
}

#[test]
fn executing_pass_door_postpones_new_move() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequencePriority, SequenceState};

    let sim = crate::sim_rng::test_context();
    let assets = crate::engine::LevelAssets::new();
    let mut engine = EngineInner::new();
    let owner = engine.add_entity(make_test_soldier(Posture::Upright));

    let mut current_pass =
        SequenceElement::new_movement(1, Command::PassDoor, Some(owner), OrderType::WalkingUpright);
    current_pass.priority = SequencePriority::NonInterruptable;
    let pass_seq = engine.orders.sequence_manager.launch_element(current_pass);
    engine
        .orders
        .sequence_manager
        .element_in_progress(pass_seq, 0);
    // Execute clears this flag at the end of the actor's frame.
    engine
        .get_entity_mut(owner)
        .unwrap()
        .actor_data_mut()
        .unwrap()
        .sequence_element_started = false;

    let incoming =
        SequenceElement::new_movement(1, Command::Move, Some(owner), OrderType::WalkingUpright);
    let incoming_seq = engine.launch_element_for_owner(&sim, &assets, incoming);

    let pass = engine
        .orders
        .sequence_manager
        .get_element(pass_seq, 0)
        .unwrap();
    assert_eq!(pass.state, SequenceState::InProgress);
    assert_eq!(pass.cross_postponed, Some((incoming_seq, 0)));

    let incoming = engine
        .orders
        .sequence_manager
        .get_element(incoming_seq, 0)
        .unwrap();
    assert_eq!(incoming.state, SequenceState::Postponed);
}

/// A Crouched soldier receiving `ENTER_ATTENTIVE_MODE` must first
/// auto-stand (CROUCH_UP) before the alerted transition can play,
/// because `get_transition_flags_soldier` for this command sets
/// `CHANGEPOSTURE_MUST_BE_UPRIGHT` without `CAN_BE_CROUCHED`.
/// Posture transition generation auto-inserts a `CROUCH_UP` translate and flips the element's
/// `posture_after_transition` to Upright; the soldier's own Translate
/// then queues the transition animation on the now-Upright element.
///
/// The "Consider as done" else-branch at
/// the soldier command only fires when GenerateTransition couldn't promote
/// posture to Upright (e.g. on a ladder).  That arm
/// isn't reachable from Crouched once GenerateTransition is wired in.
#[test]
fn soldier_enter_attentive_mode_from_crouched_stands_first() {
    let mut display = HostDisplayState::default();
    use crate::element::{Command, Posture};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Crouched));

    // Leave `posture_after_transition` undefined: the deferred Instruct
    // stamps it from the actor's live (Crouched) posture, and transition
    // generation then promotes it to Upright via the crouch-up animation.
    // A pre-stamped posture would skip that transition pass entirely.
    let elem = SequenceElement::new(1, Command::EnterAttentiveMode, Some(soldier_id));
    engine.launch_element(elem);
    engine.ensure_wait_element(soldier_id);

    let mut assets = LevelAssets::default();
    let mut dev = crate::engine::DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    // `MakePostureTransition` translates the CROUCH_UP then the element's
    // `posture_after_transition` is Upright; the ENTER_ATTENTIVE_MODE
    // Translate queues the alerted transition animation on top.  The
    // actor's current order is whatever sits at the front of the order
    // queue — the crouch-up animation runs first.
    let front = engine
        .orders
        .sequence_manager
        .current_order_for_actor(soldier_id)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        front,
        Some(crate::order::OrderType::TransitionCrouchingUp),
        "crouch-up transition animation should play first"
    );
}

// ─── Waypoint-script VM dispatch ───────────────────────────────────
//
// Covers the per-waypoint VM wiring added to `MissionScript`:
// `bind_waypoint` + the shared ScriptVmKey driver. Each scripted waypoint
// carries its own VM and `Initialize()` + `ReachPoint(actor)` dispatch
// into that VM.

/// Build a minimal SCB with one class `TestWaypoint` that exposes
/// empty `Initialize` and `ReachPoint` functions (body: just
/// `BeginFunction` + `Return`).  Returns the parsed `ScbFile` shaped
/// for `MissionScript::from_scb`.
fn scripted_waypoint_scb() -> crate::scb::ScbFile {
    use crate::scb::{ClassEntry, Function, ScbFile};
    use crate::vm::{Opcode, Quad};

    let begin = Quad {
        operation: Opcode::BeginFunction as u8,
        operands: [0; 8],
    };
    let ret = Quad {
        operation: Opcode::Return as u8,
        operands: [0; 8],
    };

    let waypoint_class = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "TestWaypoint".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![
            Function {
                name: "Initialize".into(),
                address: 0,
                num_parameters: 0,
                size_of_return_value: 0,
                size_of_parameters: 0,
                size_of_volatile: 0,
                size_of_temporary: 0,
            },
            Function {
                name: "ReachPoint".into(),
                address: 2,
                num_parameters: 1,
                size_of_return_value: 0,
                size_of_parameters: 0,
                size_of_volatile: 0,
                size_of_temporary: 0,
            },
        ],
        quads: vec![begin, ret, begin, ret],
    };
    // `MissionScript::from_scb` requires a `StartUp` class to bind the
    // global instance against. Supply a stub so `from_scb` succeeds.
    let startup = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: Vec::new(),
        quads: Vec::new(),
    };

    ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: vec![startup, waypoint_class],
    }
}

/// `bind_waypoint` inserts a `ScriptInstance` keyed by `(path, wp)`
/// without running callbacks through a bypass path.
#[test]
fn bind_waypoint_inserts_instance() {
    let scb = scripted_waypoint_scb();
    let mut script = MissionScript::from_scb(scb).expect("from_scb");
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entity_store = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut entity_store,
        &mut ai_global,
        &mut fast_grid,
    );

    assert!(script.bind_waypoint(
        crate::ai::PathId::new(2).unwrap(),
        3,
        "TestWaypoint",
        &mut script_domains,
        &capabilities,
    ));
    assert!(
        script
            .waypoint_instances
            .contains_key(&(crate::ai::PathId::new(2).unwrap(), 3))
    );
}

#[test]
#[should_panic(expected = "Waypoint script class 'NonExistent'")]
fn bind_waypoint_rejects_missing_referenced_class() {
    let mut script = MissionScript::from_scb(scripted_waypoint_scb()).expect("from_scb");
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entity_store = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut entity_store,
        &mut ai_global,
        &mut fast_grid,
    );
    script.bind_waypoint(
        crate::ai::PathId::new(4).unwrap(),
        0,
        "NonExistent",
        &mut script_domains,
        &capabilities,
    );
}

/// The Engine driver dispatches `ReachPoint(actor)` against the bound
/// waypoint instance and distinguishes a missing VM from a missing method.
#[test]
fn waypoint_driver_dispatches_and_distinguishes_missing_vm() {
    let scb = scripted_waypoint_scb();
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(MissionScript::from_scb(scb).expect("from_scb"));
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    engine
        .with_script_session(
            &crate::sim_rng::test_context(),
            &assets,
            |script, script_domains, capabilities| {
                assert!(script.bind_waypoint(
                    crate::ai::PathId::new(0).unwrap(),
                    0,
                    "TestWaypoint",
                    script_domains,
                    capabilities,
                ));
            },
        )
        .expect("mission installed");

    // Bound: call dispatches cleanly.
    let actor_handle = 42;
    let ret = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Waypoint(crate::ai::PathId::new(0).unwrap(), 0),
            "ReachPoint",
            &[actor_handle],
            crate::natives::ScriptCallFrame::default(),
        )
        .expect("ReachPoint");
    assert_eq!(ret, 0, "empty ReachPoint should return 0");

    // A missing required VM is structural, not an optional-method default.
    let missing = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Waypoint(crate::ai::PathId::new(7).unwrap(), 9),
            "ReachPoint",
            &[actor_handle],
            crate::natives::ScriptCallFrame::default(),
        )
        .expect_err("missing instance is an error");
    assert!(missing.contains("required VM is not bound"));

    // Missing function on a bound instance: also `Ok(0)`.
    let ret_no_fn = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Waypoint(crate::ai::PathId::new(0).unwrap(), 0),
            "NotAFunction",
            &[],
            crate::natives::ScriptCallFrame::default(),
        )
        .expect("missing function should be Ok(0)");
    assert_eq!(ret_no_fn, 0);
}

/// AI: `execute_waypoint_script(path, wp)` sets the pending dispatch
/// slot; the old unconditional `EventAfterScriptGoOn` fire-and-forget
/// behaviour was replaced by the engine-side drain.
#[test]
fn execute_waypoint_script_queues_pending_dispatch() {
    let mut ai = crate::ai::AiController::default();
    assert!(ai.outbox.reentrant.waypoint_script_reach_point.is_none());
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());

    let pid = crate::ai::PathId::new(5).unwrap();
    ai.execute_waypoint_script(pid, 2);

    assert_eq!(
        ai.outbox.reentrant.waypoint_script_reach_point,
        Some((pid, 2))
    );
    // AI must NOT pre-emptively queue `EventAfterScriptGoOn` — that
    // happens only after the engine dispatches `ReachPoint` and
    // confirms the script didn't transition into `DefaultScriptDriven`.
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());
}

/// `initialize_mission_script_with` walks the supplied hiking paths,
/// binds every `WaypointCommand::Script` waypoint, and runs
/// `Initialize()` on each.  Verifies the end-to-end level-load path
/// registers instances keyed by `(path_idx, wp_idx)`.
#[test]
fn initialize_mission_script_binds_waypoint_classes() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let scb = scripted_waypoint_scb();
    let mission_script = MissionScript::from_scb(scb).expect("from_scb");

    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.scripts.mission = Some(mission_script);

    let paths = vec![
        RawHikingPath {
            waypoints: vec![
                RawWaypoint {
                    x: 0,
                    y: 0,
                    sector: 0,
                    level: 0,
                    command: WaypointCommand::None,
                },
                RawWaypoint {
                    x: 10,
                    y: 10,
                    sector: 0,
                    level: 0,
                    command: WaypointCommand::Script("TestWaypoint".into()),
                },
            ],
        },
        RawHikingPath {
            waypoints: vec![RawWaypoint {
                x: 20,
                y: 20,
                sector: 0,
                level: 0,
                command: WaypointCommand::Script("TestWaypoint".into()),
            }],
        },
    ];

    let assets = crate::engine::LevelAssets::new();
    engine.initialize_mission_script_with(sim, &assets, 0, &paths);

    let script = engine.scripts.mission.as_ref().expect("mission_script");
    // Two `Script` waypoints, both bound.
    assert!(
        script
            .waypoint_instances
            .contains_key(&(crate::ai::PathId::new(0).unwrap(), 1))
    );
    assert!(
        script
            .waypoint_instances
            .contains_key(&(crate::ai::PathId::new(1).unwrap(), 0))
    );
    // The `None`-command waypoint doesn't get a binding.
    assert!(
        !script
            .waypoint_instances
            .contains_key(&(crate::ai::PathId::new(0).unwrap(), 0))
    );
    assert_eq!(script.waypoint_instances.len(), 2);
}

#[test]
fn mission_startup_runs_after_script_sector_occupants_are_initialized() {
    use crate::engine::script::{
        MissionInitializationPhase, capture_mission_initialization_phases,
    };

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.scripts.mission =
        Some(MissionScript::from_scb(scripted_waypoint_scb()).expect("minimal mission script"));
    let assets = crate::engine::LevelAssets::new();

    let (_, phases) = capture_mission_initialization_phases(|| {
        engine.initialize_mission_script_with(&sim, &assets, 0, &[]);
    });

    assert_eq!(
        phases,
        vec![
            MissionInitializationPhase::ScriptSectorOccupants,
            MissionInitializationPhase::StartUpInitialize,
        ],
        "RHEngine::Initialize populates sector occupants before calling IEngineScript::Initialize"
    );
}

/// Waypoint-script heaps round-trip through plain serde: heap bytes
/// written to the instance before serialising must come back
/// verbatim on deserialise.  This is the path `Engine::restore` uses
/// (via the full `EngineInner` serde derive), not a bespoke helper.
#[test]
fn waypoint_script_heap_round_trips_through_serde() {
    use crate::scb::{ClassEntry, Function, ScbFile};
    use crate::vm::{Opcode, Quad};

    // Class with a non-zero heap so we can poke distinct bytes in.
    let begin = Quad {
        operation: Opcode::BeginFunction as u8,
        operands: [0; 8],
    };
    let ret = Quad {
        operation: Opcode::Return as u8,
        operands: [0; 8],
    };
    let waypoint_class = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "HeapWaypoint".into(),
        size_of_member_variables: 8,
        member_variables: Vec::new(),
        functions: vec![Function {
            name: "Initialize".into(),
            address: 0,
            num_parameters: 0,
            size_of_return_value: 0,
            size_of_parameters: 0,
            size_of_volatile: 0,
            size_of_temporary: 0,
        }],
        quads: vec![begin, ret],
    };
    let startup = ClassEntry {
        source_file: "test.scs".into(),
        class_name: "StartUp".into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: Vec::new(),
        quads: Vec::new(),
    };
    let scb = ScbFile {
        version: crate::scb::SCB_VERSION,
        classes: vec![startup, waypoint_class],
    };

    let mut script = MissionScript::from_scb(scb).expect("from_scb");
    let mut script_domains = crate::engine::ScriptDomains::default();
    let mut entity_store = crate::entities::Entities::new();
    let mut ai_global = crate::ai::AiGlobalState::default();
    let mut fast_grid = crate::fast_find_grid::FastFindGrid::default();
    let sim = crate::sim_rng::test_context();
    let capabilities = crate::natives::NativeSessionCapabilities::new(
        &sim,
        &mut entity_store,
        &mut ai_global,
        &mut fast_grid,
    );
    assert!(script.bind_waypoint(
        crate::ai::PathId::new(3).unwrap(),
        7,
        "HeapWaypoint",
        &mut script_domains,
        &capabilities,
    ));

    // Poke distinct bytes into the heap so a zero reset is detectable.
    script
        .waypoint_instances
        .get_mut(&(crate::ai::PathId::new(3).unwrap(), 7))
        .unwrap()
        .vm
        .heap
        .copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22]);

    // Serialise → deserialise → heap bytes must match.
    let json = serde_json::to_string(&script).expect("serialize");
    let restored: crate::engine::types::MissionScript =
        serde_json::from_str(&json).expect("deserialize");

    let inst = restored
        .waypoint_instances
        .get(&(crate::ai::PathId::new(3).unwrap(), 7))
        .expect("restored");
    assert_eq!(
        inst.vm.heap,
        &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22]
    );
}

/// Leaning-out soldiers that receive `Command::ShootBow` must keep
/// the lean-out pose — `GetTransitionFlags` pairs `MUST_BE_UPRIGHT`
/// with `CAN_BE_LEANING_OUT` for SHOOT_BOW, so the auto-leave should
/// skip.
#[test]
fn soldier_leaning_out_keeps_pose_for_shoot_bow() {
    use crate::element::{Command, Posture};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::LeaningOut));

    let changed = engine.auto_leave_disguise_if_needed(soldier_id, Command::ShootBow);
    assert!(
        !changed,
        "ShootBow + LeaningOut must stay in lean-out pose (CAN_BE_LEANING_OUT)"
    );

    let entity = engine.get_entity(soldier_id).expect("soldier present");
    assert_eq!(entity.element_data().posture, Posture::LeaningOut);
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(soldier_id)
            .is_none(),
        "no unstick animation should be queued"
    );
}

/// The `auto_leave_disguise_if_needed` path should set
/// `posture_after_transition` and `action_state_after_transition`
/// on the in-flight sequence element.
#[test]
fn soldier_leaning_out_updates_sequence_element_fields() {
    use crate::element::{ActionState, Command, Posture};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::LeaningOut));

    // Launch a Move sequence element so there's an element to decorate.
    let elem = SequenceElement::new_movement(
        1,
        Command::Move,
        Some(soldier_id),
        crate::order::OrderType::WalkingUpright,
    );
    let seq_id = engine.launch_element(elem);

    let changed = engine.auto_leave_disguise_if_needed(soldier_id, Command::Move);
    assert!(changed);

    // Locate the element and verify the post-transition fields snap.
    let found = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .find(|s| s.id == seq_id)
        .and_then(|s| s.elements.iter().find(|e| e.command == Command::Move));
    let elem = found.expect("sequence element present");
    assert_eq!(elem.posture_after_transition, Posture::Upright);
    assert_eq!(elem.action_state_after_transition, ActionState::Waiting);
}

/// Regression: the synchronous `Instruct`-equivalent fires inside
/// `launch_element` for owned elements, so an element launched
/// mid-tick should be dispatched and reach `InProgress` during the
/// same `perform_hourglass` pass rather than idling one frame in
/// `Todo`.  The previous two-phase flow (launch → Todo → next-tick
/// arbitrate → dispatch) introduced a one-frame skew between launch
/// and visible state — `Instruct` runs synchronously inside
/// `LaunchSequenceElement` and ends with state `InProgress` after the
/// translate step inside the same call.
#[test]
fn launched_owned_element_reaches_in_progress_in_same_tick() {
    let mut display = HostDisplayState::default();
    use crate::element::{Command, Posture};
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let soldier_id = engine.add_entity(make_test_soldier(Posture::Upright));

    // Launch a SitDown element — the NPC translate arm pushes a single
    // TransitionWaitingUprightSitting animation order onto it and flips
    // the element to InProgress inside the same hourglass pass.
    let elem = SequenceElement::new(1, Command::SitDown, Some(soldier_id));
    let seq_id = engine.launch_element(elem);
    engine.ensure_wait_element(soldier_id);

    let mut assets = LevelAssets::new();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let elem_state = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("element still present")
        .state;
    assert_eq!(
        elem_state,
        SequenceState::InProgress,
        "launched element must reach InProgress inside the same tick as launch; got {elem_state:?}"
    );
}

#[test]
fn equip_bow_translate_plays_transition_orders() {
    let mut display = HostDisplayState::default();
    use crate::element::{ActionState, Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceState};

    let mut engine = EngineInner::new();
    let pc_id = engine.add_entity(make_test_pc(Posture::Upright));

    let elem = SequenceElement::new(1, Command::EquipBow, Some(pc_id));
    let seq_id = engine.launch_element(elem);
    engine.ensure_wait_element(pc_id);

    let mut assets = LevelAssets::new();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let elem = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("EquipBow element still present");
    assert_eq!(elem.state, SequenceState::InProgress);
    assert_eq!(
        elem.action_state_after_transition,
        ActionState::AimingWithBow
    );
    assert!(
        elem.orders
            .iter()
            .any(|order| order.order_type == OrderType::TransitionEquipBow),
        "EquipBow should queue the take-bow transition"
    );
    assert!(
        elem.orders
            .iter()
            .any(|order| order.order_type == OrderType::TransitionLoadingBow),
        "EquipBow should queue the loading transition"
    );
}

// ─── NPC translate dispatch ────────────────────────────────────────
//
// The four NPC-specific commands each push a single one-shot
// animation order with `compute_direction = false` and bind sequence
// termination to its DONE.

/// Drive `perform_hourglass` once, asserting the launched element
/// pushed the expected animation onto its order queue and that the
/// order is what the animation driver sees via `current_order_for_actor`.
/// `BEGGAR_SHOW_FACE` runs against a civilian (only civilians can be
/// beggars); the others use a soldier.
fn assert_npc_translate_books(
    command: crate::element::Command,
    expected_anim: crate::order::OrderType,
) {
    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let actor = match command {
        crate::element::Command::BeggarShowFace => {
            let actor = engine.add_entity(make_test_civilian(crate::element::Posture::Upright));
            let Entity::Civilian(civilian) = engine
                .get_entity_mut(actor)
                .expect("new BeggarShowFace civilian should exist")
            else {
                panic!("new BeggarShowFace actor should be a civilian");
            };
            civilian.npc.ai_brain = crate::element::AiBrain::Friendly(Box::new(
                crate::ai_friendly::FriendlyAi::new(actor.index()),
            ));
            actor
        }
        _ => engine.add_entity(make_test_soldier(crate::element::Posture::Upright)),
    };

    let elem = crate::sequence::SequenceElement::new(1, command, Some(actor));
    let seq_id = engine.launch_element(elem);
    engine.ensure_wait_element(actor);

    complete_test_runtime_fixture(&mut engine, &mut assets);
    let _ = engine.perform_hourglass(&mut display, &assets, &mut dev);

    let (order_seq, _, order_type) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(actor)
        .map(|(s, e, o)| (s, e, o.order_type))
        .expect("front order should be set");
    assert_eq!(
        order_seq, seq_id,
        "front order should live on the launched element for {command:?}",
    );
    assert_eq!(
        order_type, expected_anim,
        "wrong animation queued for {command:?}",
    );
    let elem_state = engine
        .orders
        .sequence_manager
        .get_element(seq_id, 0)
        .expect("element present")
        .state;
    assert_eq!(
        elem_state,
        crate::sequence::SequenceState::InProgress,
        "element should stay InProgress while the anim is playing",
    );
}

#[test]
fn wake_up_translate_books_turning_then_waking_up_with_antagonist() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;

    let mut dev = DevState::default();
    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let rescuer = engine.add_entity(make_test_pc(Posture::Upright));
    let target = engine.add_entity(make_test_soldier(Posture::Lying));

    bind_test_action_point(
        &mut engine,
        rescuer,
        OrderType::WakingUp,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );

    let elem = SequenceElement::new_interaction(1, Command::WakeUp, Some(rescuer), Some(target));
    let seq_id = engine.launch_element(elem);
    engine.ensure_wait_element(rescuer);

    complete_test_runtime_fixture(&mut engine, &mut assets);
    let _ = engine.perform_hourglass(&mut display, &assets, &mut dev);

    let (order_seq, order_elem, order) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(rescuer)
        .expect("WakeUp should queue an animation order");
    assert_eq!(order_seq, seq_id);
    assert_eq!(order.order_type, OrderType::Turning);
    let orders = &engine
        .orders
        .sequence_manager
        .get_element(order_seq, order_elem)
        .unwrap()
        .orders;
    assert_eq!(orders.len(), 2);
    assert_eq!(orders[0].order_type, OrderType::Turning);
    assert!(orders[0].compute_direction);
    assert_eq!(orders[1].order_type, OrderType::WakingUp);
    assert_eq!(orders[1].antagonist, Some(target));
}

#[test]
fn waking_up_done_clears_target_concussion_and_waits() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use super::animation::{AnimCompletionOutcomes, ExecuteSideOutcomes};
    use crate::combat::CONCUSSION_THRESHOLD;
    use crate::element::{ActionState, Posture};
    use crate::order::OrderType;
    use crate::sequence::SequenceState;

    let mut engine = EngineInner::new();
    let rescuer = engine.add_entity(make_test_pc(Posture::Upright));
    let target = engine.add_entity(make_test_soldier(Posture::Lying));
    {
        let target_entity = engine.get_entity_mut(target).expect("target present");
        target_entity.human_data_mut().unwrap().unconscious = true;
        target_entity
            .human_data_mut()
            .unwrap()
            .concussion_of_the_brain = CONCUSSION_THRESHOLD;
        target_entity.npc_data_mut().unwrap().life_points = 30;
        target_entity.actor_data_mut().unwrap().action_state = ActionState::Moving;
    }

    let outcomes = AnimCompletionOutcomes {
        execute_sides: ExecuteSideOutcomes {
            waking_up_done: vec![(rescuer, target)],
            ..Default::default()
        },
        ..Default::default()
    };
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .soldiers
        .resize_with(1, crate::profiles::SoldierProfile::default);

    // The wake target already owns the ordinary unconscious idle Wait.
    // Original target->Wait() must replace this equal-priority element,
    // rather than merely ensuring that some Wait exists.
    let stale_wait = engine.actor_wait(target);
    engine
        .drain_script_synchronous_actions(sim, &assets, &mut Vec::new())
        .expect("initial unconscious Wait should translate synchronously");
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(target)
            .map(|(_, _, order)| order.order_type),
        Some(OrderType::BeingUnconscious)
    );

    engine.process_anim_completion_outcomes(sim, outcomes, &assets);
    engine
        .drain_script_synchronous_actions(sim, &assets, &mut Vec::new())
        .expect("wake completion's fresh Wait should translate synchronously");

    let target_entity = engine.get_entity(target).expect("target present");
    assert_eq!(target_entity.element_data().posture, Posture::Lying);
    assert_eq!(
        target_entity.human_data().unwrap().concussion_of_the_brain,
        0
    );
    assert!(!target_entity.human_data().unwrap().unconscious);
    // The recovery Wait has translated (StandingUp is current below) but its
    // animation has not reached the START edge yet — posture and action
    // state both flip to Upright/Waiting only on that edge, so the actor
    // still reports the pre-wake movement state at this boundary.
    assert_eq!(
        target_entity.actor_data().unwrap().action_state,
        ActionState::Moving
    );
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(stale_wait, 0)
            .expect("stale unconscious Wait remains inspectable")
            .state,
        SequenceState::Interrupted,
        "fresh target->Wait() replaces the stale unconscious idle"
    );
    let (fresh_wait, current_order) = engine
        .orders
        .sequence_manager
        .current_order_for_actor(target)
        .map(|(seq_id, _, order)| (seq_id, order.order_type))
        .expect("fresh recovery Wait should be current");
    assert_eq!(current_order, OrderType::StandingUp);
    assert_ne!(fresh_wait, stale_wait);
}

/// `Point` → `Pointing` animation.
#[test]
fn npc_translate_point_books_pointing_anim() {
    use crate::element::Command;
    use crate::order::OrderType;
    assert_npc_translate_books(Command::Point, OrderType::Pointing);
}

/// `SitDown` → `TransitionWaitingUprightSitting` animation.
#[test]
fn npc_translate_sit_down_books_sit_transition() {
    use crate::element::Command;
    use crate::order::OrderType;
    assert_npc_translate_books(Command::SitDown, OrderType::TransitionWaitingUprightSitting);
}

/// `BeggarShowFace` → `BeggarShowingFace` animation.  Targets a
/// civilian, since only civilians can be beggars.
#[test]
fn npc_translate_beggar_show_face_books_show_face_anim() {
    use crate::element::Command;
    use crate::order::OrderType;
    assert_npc_translate_books(Command::BeggarShowFace, OrderType::BeggarShowingFace);
}

/// `EnterLeisure` → `TransitionWaitingUprightSpecial` animation.
#[test]
fn npc_translate_enter_leisure_books_special_transition() {
    use crate::element::Command;
    use crate::order::OrderType;
    assert_npc_translate_books(
        Command::EnterLeisure,
        OrderType::TransitionWaitingUprightSpecial,
    );
}

#[test]
fn get_killed_at_bottom_kills_lying_victim_immediately() {
    use crate::ai::{AiState, Substate};
    use crate::campaign::CampaignValue;
    use crate::combat::CONCUSSION_MAX;
    use crate::element::{Camp, Command, Posture};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let killer = engine.add_entity(make_test_pc(Posture::Upright));
    let victim = engine.add_entity(make_test_soldier(Posture::Lying));
    if let Some(crate::element::Entity::Soldier(soldier)) = engine.world.entities.get_mut(victim) {
        soldier.npc.life_points = 30;
        soldier.soldier.cached_max_life_points = 30;
        soldier.soldier.cached_camp = Camp::Lacklandists;
        soldier.human.unconscious = true;
        soldier.human.concussion_of_the_brain = CONCUSSION_MAX;
        soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
        let ai = soldier
            .npc
            .ai_brain
            .base_mut()
            .expect("test soldier has enemy AI");
        ai.current_state = AiState::Sleeping;
        ai.current_substate = Substate::SleepingUnconscious;
    }

    let elem =
        SequenceElement::new_interaction(1, Command::GetKilledAtBottom, Some(victim), Some(killer));
    engine.launch_element(elem);
    engine.ensure_wait_element(victim);
    let score_before = engine.mission_domain.campaign.values[CampaignValue::Score];

    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::new();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let entity = engine.get_entity(victim).expect("victim still present");
    assert!(entity.is_dead());
    assert_eq!(entity.element_data().posture, Posture::DeadBack);
    assert!(!entity.human_data().unwrap().unconscious);
    assert_eq!(entity.human_data().unwrap().concussion_of_the_brain, 0);
    let ai = entity.ai_controller().expect("dead soldier retains its AI");
    assert_eq!(ai.current_state, AiState::Sleeping);
    assert_eq!(ai.current_substate, Substate::SleepingForever);
    assert!(
        entity.npc_data().unwrap().inform_my_friends,
        "a PC execution must preserve NPC::Kill's killer notification"
    );
    assert_eq!(
        engine.mission_domain.campaign.values[CampaignValue::Score],
        score_before + 50,
        "Soldier::GetWounded awards the fight score after a Lacklandist dies"
    );
}

#[test]
fn get_killed_at_bottom_uses_vip_pc_amulet_coma_save_and_preserves_existing_coma() {
    use crate::campaign::CampaignValue;
    use crate::element::{Command, Posture};
    use crate::sequence::SequenceElement;

    let mut engine = EngineInner::new();
    let killer = engine.add_entity(make_test_soldier(Posture::Upright));
    let victim = engine.add_entity(make_test_pc(Posture::Lying));
    if let Some(crate::element::Entity::Pc(pc)) = engine.world.entities.get_mut(victim) {
        pc.pc.life_points = 80;
        pc.human.unconscious = true;
    }
    bind_test_action_point(
        &mut engine,
        victim,
        crate::order::OrderType::BeingUnconscious,
        crate::coordinates::SpriteLocalPoint::ZERO,
        crate::coordinates::SpriteAnchor::ZERO,
    );

    let elem =
        SequenceElement::new_interaction(1, Command::GetKilledAtBottom, Some(victim), Some(killer));
    engine.launch_element(elem);
    engine.ensure_wait_element(victim);

    let mut display = HostDisplayState::default();
    let mut assets = LevelAssets::new();
    let mut dev = DevState::default();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    let mut profiles = (*assets.profile_manager).clone();
    profiles.characters[0].vip = true;
    assets.profile_manager = std::sync::Arc::new(profiles);
    engine.mission_domain.campaign.values[CampaignValue::Amulets] = 1;

    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let entity = engine.get_entity(victim).expect("victim still present");
    assert!(!entity.is_dead());
    assert_eq!(entity.human_life_points(), 5);
    assert_eq!(entity.element_data().posture, Posture::Lying);
    assert!(
        entity.human_data().unwrap().unconscious,
        "the amulet coma save must skip the virtual Kill cascade"
    );
    assert!(engine.mission_domain.campaign.characters[0].status.in_coma);
    assert_eq!(
        engine.mission_domain.campaign.values[CampaignValue::Amulets],
        0
    );

    // A second execution models another guard completing STRIKING_DOWN_SWORD
    // against the already-comatose PC. Original PC::GetWounded does nothing
    // when lethal damage arrives while bInComa is set.
    let elem =
        SequenceElement::new_interaction(1, Command::GetKilledAtBottom, Some(victim), Some(killer));
    engine.launch_element(elem);
    engine.ensure_wait_element(victim);
    engine.perform_hourglass(&mut display, &assets, &mut dev);

    let entity = engine.get_entity(victim).expect("victim still present");
    assert!(!entity.is_dead());
    assert_eq!(entity.human_life_points(), 5);
    assert_eq!(entity.element_data().posture, Posture::Lying);
}

/// When the `TransitionWaitingUprightSitting` animation completes,
/// the actor's posture flips to `Sitting`.
#[test]
fn npc_sit_down_anim_completion_flips_posture_to_sitting() {
    use super::animation::{ExecuteSideOutcomes, apply_npc_execute_side_effects};
    use crate::element::{ActionState, EntityId, Posture};
    use crate::order::OrderType;
    use crate::sprite::MotionState;

    let mut entity = make_test_soldier(Posture::Upright);
    let mut outcomes = ExecuteSideOutcomes::default();

    apply_npc_execute_side_effects(
        &mut entity,
        OrderType::TransitionWaitingUprightSitting,
        MotionState::Terminated,
        None,
        EntityId::Pc(crate::entity_id::PcId(0)),
        &mut outcomes,
    );

    assert_eq!(entity.element_data().posture, Posture::Sitting);
    assert_eq!(
        entity.actor_data().expect("actor data").action_state,
        ActionState::Waiting,
    );
}

/// A sitting NPC who receives `Point` first stands up: the auto-leave
/// path snaps the posture to `Upright` and queues the
/// `TransitionSittingWaitingUpright` animation on the actor's
/// `order_queue` so the visible stand-up plays before the gesture.
#[test]
fn sitting_npc_point_auto_stands_up() {
    use crate::element::{Command, Posture};
    use crate::order::OrderType;

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_test_soldier(Posture::Sitting));

    let changed = engine.auto_leave_disguise_if_needed(actor, Command::Point);
    assert!(changed, "auto-leave should fire for Sitting + Point");

    let entity = engine.get_entity(actor).expect("entity present");
    assert_eq!(entity.element_data().posture, Posture::Upright);

    let next_order = engine
        .orders
        .sequence_manager
        .current_order_for_actor(actor)
        .map(|(_, _, o)| o.order_type);
    assert_eq!(
        next_order,
        Some(OrderType::TransitionSittingWaitingUpright),
        "stand-up transition should be queued on the owning sequence element",
    );
}

/// `EnterLeisure` on an already-leisuring NPC must not auto-leave
/// leisure first — `GetTransitionFlags` sets
/// `CHANGEPOSTURE_CAN_BE_LEISURING` for this command.
#[test]
fn enter_leisure_on_leisuring_npc_skips_auto_leave() {
    use crate::element::{Command, Posture};

    let mut engine = EngineInner::new();
    let actor = engine.add_entity(make_test_soldier(Posture::Leisure));

    let changed = engine.auto_leave_disguise_if_needed(actor, Command::EnterLeisure);
    assert!(
        !changed,
        "leisure-leisure re-entry should be a no-op (CAN_BE_LEISURING exempt)",
    );

    let entity = engine.get_entity(actor).expect("entity present");
    assert_eq!(entity.element_data().posture, Posture::Leisure);
    assert!(
        engine
            .orders
            .sequence_manager
            .current_order_for_actor(actor)
            .is_none(),
        "no transition animation should be queued",
    );
}

/// When the `TransitionWaitingUprightSpecial` animation completes,
/// the actor's posture flips to `Leisure`.
#[test]
fn npc_enter_leisure_anim_completion_flips_posture_to_leisure() {
    use super::animation::{ExecuteSideOutcomes, apply_npc_execute_side_effects};
    use crate::element::{ActionState, EntityId, Posture};
    use crate::order::OrderType;
    use crate::sprite::MotionState;

    let mut entity = make_test_soldier(Posture::Upright);
    let mut outcomes = ExecuteSideOutcomes::default();

    apply_npc_execute_side_effects(
        &mut entity,
        OrderType::TransitionWaitingUprightSpecial,
        MotionState::Done,
        None,
        EntityId::Pc(crate::entity_id::PcId(0)),
        &mut outcomes,
    );

    assert_eq!(entity.element_data().posture, Posture::Leisure);
    assert_eq!(
        entity.actor_data().expect("actor data").action_state,
        ActionState::Waiting,
    );
}

/// `remove_quick_action_titbits_for(pc, level)` looks up the
/// per-level titbit entry on the PC, drops every titbit with that id,
/// and reports whether anything was removed.
#[test]
fn remove_quick_action_titbits_for_matches_original_signature() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::EntityId;
    use crate::titbit::{ElementHandle, INVALID_ID, QuickAction, TitbitKind};

    let mut engine = EngineInner::new();
    let pc = EntityId::Pc(crate::entity_id::PcId(42));
    let slot: u8 = 1;

    // Empty slot → early-returns on the sentinel id.
    assert!(!engine.remove_quick_action_titbits_for(pc, slot));

    // Add a QA titbit and wire its id into the PC's macro slot.
    let pc_handle = ElementHandle(pc.index());
    let titbit_id = engine.feedback.titbit_manager.add_titbit(
        WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        0,
        TitbitKind::QuickAction,
        pc_handle,
        QuickAction::Bow as u16,
        pc_handle,
        false,
        INVALID_ID,
        true,
        Some(0.0),
        Some(0),
    );
    assert_ne!(titbit_id, INVALID_ID);
    engine
        .players
        .macro_store
        .get_or_insert(pc)
        .set_slot_titbit(
            slot as usize,
            crate::titbit::TitbitId::new(titbit_id).unwrap(),
        );

    // Populated slot → drops the titbit and reports success.
    assert!(engine.remove_quick_action_titbits_for(pc, slot));
    assert!(
        !engine
            .feedback
            .titbit_manager
            .titbits()
            .iter()
            .any(|t| t.id == titbit_id),
        "titbit with id {titbit_id} should be gone"
    );

    // Second call after the list is empty: slot still holds the stale
    // id (the caller clears the level entry after this returns), but
    // no titbit matches, so it returns false.
    assert!(!engine.remove_quick_action_titbits_for(pc, slot));
}
