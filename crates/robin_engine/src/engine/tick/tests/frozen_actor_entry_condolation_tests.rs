use super::*;
use crate::ai::{AiEntityHandle, AiLockFlags, Stimulus, StimulusType};
use crate::element::{
    ActionState, ActorData, ActorSoldier, ElementData, ElementKind, HumanData, NpcData, SoldierData,
};
use crate::order::{Order, OrderType};
use crate::sequence::{CascadeFlags, SequenceElement};

#[test]
fn selected_terminal_card_precedes_frozen_actors_derived_tail() {
    let mut assets = LevelAssets::new();
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    profiles.soldiers.push(crate::profiles::SoldierProfile {
        hth_weapon_id: 1,
        ..Default::default()
    });
    profiles.hth_weapons.push(Default::default());
    let mut engine = EngineInner::new();
    let npc = NpcData {
        ai: crate::element::AiActorData {
            ai_brain: crate::element::AiBrain::Enemy(Box::default()),
            ..Default::default()
        },
        ..Default::default()
    };
    let owner = engine.add_test_entity(Entity::Soldier(ActorSoldier {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ActorSoldier;
            initial_element.active = true;
            initial_element
        },
        actor: ActorData {
            action_state: ActionState::WaitingSword,
            execution_frozen: true,
            ..ActorData::default()
        },
        human: HumanData::default(),
        npc,
        soldier: SoldierData {
            cached_camp: crate::element_kinds::Camp::Lacklandists,
            ..SoldierData::default()
        },
    }));
    let ai = engine
        .get_entity_mut(owner)
        .and_then(Entity::enemy_ai_mut)
        .expect("test owner has Enemy AI");
    ai.base.locks_flag_field = AiLockFlags::FREEZE;
    ai.hth_weapon_id = 1;

    let mut strike =
        SequenceElement::new_interaction(1, Command::SwordstrikeSmalltalkRight, Some(owner), None);
    strike
        .orders
        .push_back(Order::test_new(OrderType::StrikingRightSmalltalk, 0.0, 0.0));
    let sequence = engine.orders.sequence_manager.launch_element(strike);
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        sequence,
        0,
    );
    engine.element_interrupted(
        &crate::sim_rng::test_context(),
        &assets,
        &mut Vec::new(),
        sequence,
        0,
        CascadeFlags::NEXT_LEVEL,
    );

    engine.tick_actor_animation_action_change_slots_with_after_slot(
        &crate::sim_rng::test_context(),
        &assets,
        |engine, actor| {
            if actor == owner {
                engine
                    .get_entity_mut(owner)
                    .and_then(Entity::enemy_ai_mut)
                    .expect("test owner retains Enemy AI")
                    .base
                    .stimulus_queue
                    .push(Stimulus::with_human(StimulusType::EventOutOfView, 7));
            }
        },
    );

    let queue = &engine
        .get_entity(owner)
        .and_then(Entity::enemy_ai)
        .expect("test owner retains Enemy AI")
        .base
        .stimulus_queue;
    assert_eq!(queue.len(), 2);
    assert_eq!(queue[0].stimulus_type, StimulusType::EventDone);
    assert_eq!(queue[1].stimulus_type, StimulusType::EventOutOfView);
    assert_eq!(
        queue[1].info,
        crate::ai::StimulusInfo::Human(AiEntityHandle::new(7))
    );
}
