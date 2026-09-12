use super::*;

#[test]
fn enemy_ai_hero_knockout_runs_shared_ai_cleanup() {
    let mut engine = make_engine();
    let (victim, _) = make_enemy_ai_hero_strike_pair(&mut engine);
    {
        let entity = engine.get_entity_mut(victim).unwrap();
        let human = entity.human_data_mut().unwrap();
        human.unconscious = true;
        human.concussion_of_the_brain = 25;
        let ai_actor = entity.ai_actor_data_mut().unwrap();
        ai_actor.alerted = true;
        ai_actor.maximal_detection_suspect = 40;
    }
    let assets = assets_with_sword_profile(7, 30);

    engine.apply_knockout_side_effects(
        &crate::sim_rng::test_context(),
        &assets,
        victim,
        true,
        true,
    );

    let ai_actor = engine
        .get_entity(victim)
        .and_then(Entity::ai_actor_data)
        .expect("AI-controlled hero retains AI actor data");
    assert_eq!(ai_actor.maximal_detection_suspect, 0);
    assert_eq!(
        ai_actor.eye_status,
        crate::element::EyeStatus::DieOrGetUnconscious
    );
    assert!(ai_actor.inform_my_friends);
}

#[test]
fn enemy_ai_hero_empty_opponent_evaluation_delivers_quit_event() {
    let mut engine = make_engine();
    let (owner, _) = make_enemy_ai_hero_strike_pair(&mut engine);
    engine
        .get_entity_mut(owner)
        .and_then(Entity::human_data_mut)
        .expect("AI-controlled hero has HumanData")
        .opponents
        .clear();
    let assets = assets_with_sword_profile(7, 30);

    engine.evaluate_opponents(&crate::sim_rng::test_context(), &assets, owner);

    let ai = engine
        .get_entity(owner)
        .and_then(Entity::ai_controller)
        .expect("AI-controlled hero retains its AI");
    assert_eq!(
        ai.current_substate,
        crate::ai::Substate::AttackingQuittingSwordfight
    );
    assert!(ai.ai_log.iter().any(|entry| {
        entry.line_type == crate::ai::LogLineType::Event
            && entry.info == crate::ai::StimulusType::EventQuitSwordfight as u16
    }));
}

#[test]
fn deleting_final_opponent_synchronously_quits_enemy_ai_hero_ai() {
    use crate::ai::{AiState, LogLineType, StimulusType, Substate};
    use crate::profiles::{CharacterProfile, HtHWeaponProfile, ProfileManager, SoldierProfile};

    let sim = crate::sim_rng::test_context();
    let mut engine = make_engine();
    let pc = engine.add_test_entity(make_pc(
        WorldPoint3D {
            x: 0.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));
    let opponent = engine.add_test_entity(make_soldier(
        WorldPoint3D {
            x: 10.0,
            y: 100.0,
            z: 0.0,
        },
        None,
    ));

    let Entity::Pc(pc_entity) = engine.get_entity_mut(pc).unwrap() else {
        unreachable!()
    };
    pc_entity.human.opponents = vec![opponent].into();
    let mut enemy_ai = crate::ai_enemy::EnemyAi::new(pc.index());
    enemy_ai.base.current_state = AiState::Attacking;
    enemy_ai.base.current_substate = Substate::AttackingSwordfightSpecialStrike;
    enemy_ai.hth_weapon_id = 1;
    pc_entity.pc.life_points = 100;
    pc_entity.pc.command_interface = crate::human_control::CommandInterface::None;
    pc_entity.pc.mission_role = crate::human_control::MissionRole::Combatant;
    pc_entity.pc.combat_stance = crate::human_control::CombatStance::Aggressive;
    pc_entity.pc.ai = Some(Box::new(crate::element::AiActorData {
        ai_brain: crate::element::AiBrain::Enemy(Box::new(enemy_ai)),
        ..Default::default()
    }));
    engine
        .get_entity_mut(opponent)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .hth_weapon_id = 1;

    let mut profiles = ProfileManager::new();
    profiles.hth_weapons.push(HtHWeaponProfile::default());
    profiles.characters.push(CharacterProfile {
        hth_weapon_id: 1,
        ..CharacterProfile::default()
    });
    profiles.soldiers.push(SoldierProfile {
        hth_weapon_id: 1,
        hostile: true,
        ..SoldierProfile::default()
    });
    let assets = LevelAssets {
        profile_manager: std::sync::Arc::new(profiles),
        ..LevelAssets::new()
    };

    assert!(engine.delete_opponent(&sim, &assets, pc, opponent));

    let ai = engine.get_entity(pc).unwrap().ai_controller().unwrap();
    assert_eq!(ai.current_substate, Substate::AttackingQuittingSwordfight);
    assert!(ai.ai_log.iter().any(|entry| {
        entry.line_type == LogLineType::Event
            && entry.info == StimulusType::EventQuitSwordfight as u16
    }));
}
