use super::*;
use crate::ai::AiLockFlags;
use crate::engine::TickCtx;

fn fixture() -> (EngineInner, LevelAssets, EntityId) {
    let (mut engine, assets, owner, _) =
        super::super::battle_decision_observation_tests::fixture(false);
    let ai = engine.observation_ai_mut(owner);
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.base.substate_at_last_timer_launch = Substate::DefaultOnPost;
    ai.base.timer_is_running = false;
    ai.base.primary_target = None;
    ai.list_them.clear();
    (engine, assets, owner)
}

fn admit(
    engine: &mut EngineInner,
    assets: &LevelAssets,
    owner: EntityId,
    event: StimulusType,
) -> bool {
    engine.begin_enemy_think(
        TickCtx::new(&crate::sim_rng::test_context(), assets),
        owner,
        &Stimulus::new(event),
    )
}

#[test]
fn normal_event_admission_updates_priority() {
    let (mut engine, assets, owner) = fixture();
    assert!(admit(&mut engine, &assets, owner, StimulusType::EventTimer));
    assert!(admit(&mut engine, &assets, owner, StimulusType::EventView));
    assert_eq!(
        engine.observation_ai(owner).new_task_priority,
        crate::ai_enemy::task_priority::ENEMY
    );
}

#[test]
fn script_lock_retains_observations_but_discards_gameflow() {
    let (mut engine, assets, owner) = fixture();
    let ai = engine.observation_ai_mut(owner);
    ai.base.script_locked = true;
    ai.base.remember_events = true;
    for event in [
        StimulusType::EventView,
        StimulusType::EventDone,
        StimulusType::EventReachPoint,
    ] {
        assert!(!admit(&mut engine, &assets, owner, event));
    }
    let queue = &engine.observation_ai(owner).base.stimulus_queue;
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].stimulus_type, StimulusType::EventView);
}

#[test]
fn actor_freeze_retains_timer() {
    let (mut engine, assets, owner) = fixture();
    engine.observation_ai_mut(owner).base.locks_flag_field = AiLockFlags::FREEZE;
    assert!(!admit(
        &mut engine,
        &assets,
        owner,
        StimulusType::EventTimer
    ));
    let queue = &engine.observation_ai(owner).base.stimulus_queue;
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].stimulus_type, StimulusType::EventTimer);
}

#[test]
fn global_freeze_discards_timer() {
    let (mut engine, assets, owner) = fixture();
    engine.ai.global.freeze = true;
    assert!(!admit(
        &mut engine,
        &assets,
        owner,
        StimulusType::EventTimer
    ));
    assert!(engine.observation_ai(owner).base.stimulus_queue.is_empty());
}

#[test]
fn unconscious_script_driven_actor_refuses_look_there() {
    let (mut engine, assets, owner) = fixture();
    engine.human_mut(owner).unconscious = true;
    engine.observation_ai_mut(owner).base.current_substate = Substate::DefaultScriptDriven;
    assert!(!admit(
        &mut engine,
        &assets,
        owner,
        StimulusType::CallLookThere
    ));
    let ai = engine.observation_ai(owner);
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultScriptDriven);
    assert!(
        ai.base
            .ai_log
            .last()
            .is_some_and(|line| line.line_type == LogLineType::EventRefused && line.info == 8)
    );
}

#[test]
fn unconsciousness_executes_state_and_eye_changes_before_return() {
    let (mut engine, assets, owner) = fixture();
    assert!(!admit(
        &mut engine,
        &assets,
        owner,
        StimulusType::EventLoseConsciousness
    ));
    let ai = engine.observation_ai(owner);
    assert_eq!(ai.base.current_state, AiState::Sleeping);
    assert_eq!(ai.base.current_substate, Substate::SleepingUnconscious);
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .ai_actor_data()
            .unwrap()
            .eye_status,
        EyeStatus::DieOrGetUnconscious
    );
}

#[test]
fn corpse_rejects_unconsciousness_without_replacing_death_state() {
    let (mut engine, assets, owner) = fixture();
    engine.npc_mut(owner).life_points = 0;
    let ai = engine.observation_ai_mut(owner);
    ai.base.current_state = AiState::Sleeping;
    ai.base.current_substate = Substate::SleepingForever;
    let eyes = engine.ent(owner).ai_actor_data().unwrap().eye_status;
    assert!(!admit(
        &mut engine,
        &assets,
        owner,
        StimulusType::EventLoseConsciousness
    ));
    assert_eq!(
        engine.observation_ai(owner).base.current_substate,
        Substate::SleepingForever
    );
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .ai_actor_data()
            .unwrap()
            .eye_status,
        eyes
    );
}

#[test]
fn carried_unconscious_actor_refuses_recovery() {
    let (mut engine, assets, owner) = fixture();
    let entity = engine.ent_mut(owner);
    entity
        .element_data_mut()
        .publish_order_posture(Posture::Carried);
    entity.human_data_mut().unwrap().unconscious = true;
    let ai = engine.observation_ai_mut(owner);
    ai.base.current_state = AiState::Sleeping;
    ai.base.current_substate = Substate::SleepingUnconscious;
    assert!(!admit(
        &mut engine,
        &assets,
        owner,
        StimulusType::EventFitAgain
    ));
    assert_eq!(
        engine.observation_ai(owner).base.current_substate,
        Substate::SleepingUnconscious
    );
    assert_eq!(
        engine
            .observation_ai(owner)
            .base
            .ai_log
            .last()
            .unwrap()
            .info,
        7
    );
}

#[test]
fn after_script_is_rejected_at_each_admission_gate() {
    for gate in 0..4 {
        let (mut engine, assets, owner) = fixture();
        match gate {
            0 => engine.ai.global.freeze = true,
            1 => engine.observation_ai_mut(owner).base.locks_flag_field = AiLockFlags::BUSY,
            2 => engine.human_mut(owner).unconscious = true,
            _ => engine.npc_mut(owner).life_points = 0,
        }
        assert!(!admit(
            &mut engine,
            &assets,
            owner,
            StimulusType::EventAfterScriptGoOn
        ));
    }
}

#[test]
fn wasp_and_net_states_accept_only_matching_release_or_unconsciousness() {
    for (substate, release, wrong_release, reason) in [
        (
            Substate::WonderingWaspInArmour,
            StimulusType::EventWaspAway,
            StimulusType::EventNetAway,
            4,
        ),
        (
            Substate::WonderingUnderNet,
            StimulusType::EventNetAway,
            StimulusType::EventWaspAway,
            5,
        ),
    ] {
        let (mut engine, assets, owner) = fixture();
        let ai = engine.observation_ai_mut(owner);
        ai.base.current_state = AiState::Wondering;
        ai.base.current_substate = substate;
        assert!(!admit(&mut engine, &assets, owner, wrong_release));
        assert_eq!(
            engine
                .observation_ai(owner)
                .base
                .ai_log
                .last()
                .unwrap()
                .info,
            reason
        );
        assert!(admit(&mut engine, &assets, owner, release));
        assert!(!admit(
            &mut engine,
            &assets,
            owner,
            StimulusType::EventLoseConsciousness
        ));
        assert_eq!(
            engine.observation_ai(owner).base.current_substate,
            Substate::SleepingUnconscious
        );
    }
}
