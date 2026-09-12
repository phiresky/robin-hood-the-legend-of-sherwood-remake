use super::*;

#[test]
fn reciprocal_swordfight_entry_preserves_existing_opponent_strength() {
    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let initiator = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let opponent = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    complete_test_runtime_fixture(&mut engine, &mut assets);

    engine
        .get_entity_mut(initiator)
        .and_then(Entity::human_data_mut)
        .unwrap()
        .relative_fighting_ability = 17;
    {
        let human = engine
            .get_entity_mut(opponent)
            .and_then(Entity::human_data_mut)
            .unwrap();
        human.opponents = vec![initiator].into();
        human.relative_fighting_ability = 42;
    }

    assert!(engine.enter_swordfight(&sim, &assets, initiator, opponent, false));

    let initiator_human = engine
        .get_entity(initiator)
        .and_then(Entity::human_data)
        .unwrap();
    assert_eq!(initiator_human.opponents, vec![opponent]);
    assert_eq!(initiator_human.relative_fighting_ability, 50);

    let opponent_human = engine
        .get_entity(opponent)
        .and_then(Entity::human_data)
        .unwrap();
    assert_eq!(opponent_human.opponents, vec![initiator]);
    assert_eq!(opponent_human.relative_fighting_ability, 42);
}

#[test]
fn terminal_sword_provoke_observes_promoted_opponent_before_post_seek_speak() {
    use crate::coordinates::{MapPoint, WorldPoint3D};
    use crate::element::Command;
    use crate::sequence::{Sequence, SequenceElement};

    // Linux3/Profile001/Savegame_014/replay-040, frame 18433: the terminal
    // step puts the old principal just beyond UBER while a reciprocal second
    // opponent remains between MAXIMAL and UBER. Original removes/promotes
    // inside human action execution, registers Provoke, and only then registers the
    // point-seek SpeakHeroReachDestination tail.
    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let old_principal = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let promoted = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    let weapon = profiles
        .hth_weapons
        .first_mut()
        .expect("complete actor fixture supplies an HtH weapon");
    weapon.distance[crate::weapons::WeaponDistance::Maximal as usize] = 90;
    weapon.distance[crate::weapons::WeaponDistance::Uber as usize] = 150;

    let positions = [
        (owner, 151.583_5_f32),
        (old_principal, 0.0_f32),
        (promoted, 18.567_36_f32),
    ];
    for (entity_id, x) in positions {
        let entity = engine.get_entity_mut(entity_id).unwrap();
        entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(x, 0.0, 0.0));
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(x, 0.0));
    }
    engine
        .get_entity_mut(owner)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents = vec![old_principal, promoted].into();
    for opponent in [old_principal, promoted] {
        engine
            .get_entity_mut(opponent)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents = vec![owner].into();
    }

    assert!(
        !engine.sword_movement_termination_warrants_provoke(&assets, owner),
        "the >UBER old principal must make a pre-removal snapshot false"
    );
    engine.quit_swordfight_with_far_opponents(&sim, &assets, owner);
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![promoted]
    );
    assert!(
        engine.sword_movement_termination_warrants_provoke(&assets, owner),
        "the promoted reciprocal opponent is inside the Provoke band"
    );

    engine.launch_sword_movement_termination_provoke(owner);
    let mut post_seek = Sequence::new();
    post_seek.append_element(SequenceElement::new(
        2,
        Command::SpeakHeroReachDestination,
        Some(owner),
    ));
    engine.launch_sequence(post_seek);
    let owner_registrations = engine
        .orders
        .sequence_manager
        .v48_elements_to_go()
        .into_iter()
        .filter_map(|(sequence_id, element_index)| {
            engine
                .orders
                .sequence_manager
                .get_element(sequence_id, element_index)
        })
        .filter(|element| element.owner == Some(owner))
        .map(|element| element.command)
        .collect::<Vec<_>>();
    assert_eq!(
        owner_registrations,
        vec![Command::Provoke, Command::SpeakHeroReachDestination],
        "terminal Execute must register exactly one Provoke before post-seek Speak"
    );
}

#[test]
fn sword_movement_start_gives_initiative_to_principal_promoted_by_far_pruning() {
    use crate::coordinates::{MapPoint, WorldPoint3D};

    // Human action execution leaves swordfights with far opponents immediately
    // after performing motion and only then handles its initial state. The old
    // principal can therefore disappear before the START initiative handoff.
    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let owner = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let old_principal = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let promoted = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    let weapon = profiles
        .hth_weapons
        .first_mut()
        .expect("complete actor fixture supplies an HtH weapon");
    weapon.distance[crate::weapons::WeaponDistance::Uber as usize] = 150;

    for (entity_id, x) in [
        (owner, 151.583_5_f32),
        (old_principal, 0.0_f32),
        (promoted, 18.567_36_f32),
    ] {
        let entity = engine.get_entity_mut(entity_id).unwrap();
        entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(x, 0.0, 0.0));
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(x, 0.0));
    }
    engine
        .get_entity_mut(owner)
        .unwrap()
        .human_data_mut()
        .unwrap()
        .opponents = vec![old_principal, promoted].into();
    for opponent in [old_principal, promoted] {
        engine
            .get_entity_mut(opponent)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .opponents = vec![owner].into();
    }

    engine.quit_swordfight_with_far_opponents(&sim, &assets, owner);
    engine.apply_sword_movement_start_initiative_transfer(owner);

    let owner_human = engine.get_entity(owner).unwrap().human_data().unwrap();
    assert_eq!(owner_human.opponents, vec![promoted]);
    assert!(!owner_human.smalltalk_initiative);
    let promoted_human = engine.get_entity(promoted).unwrap().human_data().unwrap();
    assert!(promoted_human.smalltalk_initiative);
    assert!(promoted_human.received_smalltalk_initiative);
    assert!(
        !engine
            .get_entity(old_principal)
            .unwrap()
            .human_data()
            .unwrap()
            .smalltalk_initiative,
        "the pruned old principal must not receive the START handoff"
    );
}

#[test]
fn soldier_death_detaches_guard_and_archery_before_forcing_quiet_music() {
    use crate::ai::{
        AiState, AlertLevel, ArcheryReservationRelease, GuardedPcEffect, PointArchery,
        ReservedShootingPoint, SectorArchery, Substate,
    };
    use crate::entity_id::PcId;
    use crate::sector::{ArcheryPointIdx, SectorNumber};
    use crate::sound::MusicMode;

    let mut engine = EngineInner::new();

    let old_guarded_pc = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let current_guarded_pc = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let EntityId::Pc(old_guarded_pc_typed) = old_guarded_pc else {
        panic!("test PC has a non-PC entity ID")
    };
    let EntityId::Pc(current_guarded_pc_typed) = current_guarded_pc else {
        panic!("test PC has a non-PC entity ID")
    };

    let mut victim = make_test_ai_soldier(crate::element::Camp::Lacklandists);
    let Entity::Soldier(victim_soldier) = &mut victim else {
        unreachable!("make_test_ai_soldier returned non-soldier")
    };
    victim_soldier.element.active = true;
    victim_soldier.npc.life_points = 100;
    let victim_id = engine.add_test_entity(victim);

    for guarded_pc in [old_guarded_pc, current_guarded_pc] {
        let Some(Entity::Pc(pc)) = engine.get_entity_mut(guarded_pc) else {
            panic!("test guarded PC exists")
        };
        pc.element.active = true;
        pc.pc.life_points = 100;
        pc.pc.guard = Some(victim_id);
    }

    engine.ai.global.archery_sectors.push(SectorArchery {
        points: vec![PointArchery {
            position: Default::default(),
            direction: 0,
            is_shooting_point: true,
            sector_index: SectorNumber::new(1),
            owner: Some(victim_id),
        }],
        polygon: Vec::new(),
        layer: 0,
        index_first_shooting_point: Some(ArcheryPointIdx(0)),
        index_last_shooting_point: Some(ArcheryPointIdx(0)),
        num_shooting_points: 1,
        num_owners: 1,
    });

    let Some(Entity::Soldier(victim_soldier)) = engine.get_entity_mut(victim_id) else {
        panic!("test victim exists")
    };
    let enemy = victim_soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("test victim has enemy AI");
    enemy.guarded_pc = Some(PcId(current_guarded_pc_typed.0));
    enemy.base.outbox.actor.set_guarded_pc = Some(GuardedPcEffect {
        old: Some(PcId(old_guarded_pc_typed.0)),
        new: Some(PcId(current_guarded_pc_typed.0)),
    });
    // Model the state change having synchronously cleared the AI-side shooting
    // point while its reciprocal/global release is still queued.
    enemy.my_shooting_point = None;
    enemy.my_archery_sector = Some(0);
    enemy.base.outbox.actor.archery_reservation_release = ArcheryReservationRelease {
        shooting_point: Some(ReservedShootingPoint {
            sector_index: 0,
            point_index: ArcheryPointIdx(0),
        }),
        release_sector: true,
    };
    enemy.base.current_state = AiState::Menacing;
    enemy.base.current_substate = Substate::MenacingPcInComa;
    enemy.base.current_music_alert_status = AlertLevel::Red;
    enemy.base.view_alert_status = AlertLevel::Red;
    enemy.base.outbox.actor.halt = true;
    engine.ai.global.overall_villain_alert_status = AlertLevel::Red;
    engine.ai.global.overall_alert_status = AlertLevel::Red;

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.handle_death(&crate::sim_rng::test_context(), &assets, victim_id);

    for guarded_pc in [old_guarded_pc, current_guarded_pc] {
        let Some(Entity::Pc(pc)) = engine.get_entity(guarded_pc) else {
            panic!("test guarded PC survives")
        };
        assert_eq!(pc.pc.guard, None);
    }
    assert_eq!(engine.ai.global.archery_sectors[0].points[0].owner, None);
    assert_eq!(engine.ai.global.archery_sectors[0].num_owners, 0);

    let Some(Entity::Soldier(victim_soldier)) = engine.get_entity(victim_id) else {
        panic!("test victim survives as a corpse")
    };
    let enemy = victim_soldier
        .npc
        .ai_brain
        .enemy()
        .expect("test victim retains enemy AI");
    assert_eq!(enemy.guarded_pc, None);
    assert_eq!(enemy.my_archery_sector, None);
    assert_eq!(enemy.base.current_state, AiState::Sleeping);
    assert_eq!(enemy.base.current_substate, Substate::SleepingForever);
    assert!(!enemy.base.outbox.actor.halt);
    assert_eq!(
        enemy.base.outbox.actor.archery_reservation_release,
        ArcheryReservationRelease::default()
    );
    assert!(enemy.base.outbox.music.instant_change);

    engine.update_overall_villain_alert(&assets.profile_manager);
    assert!(
        engine
            .feedback
            .pending_side_effects
            .sounds
            .iter()
            .any(|command| matches!(command, SoundCommand::ForceMusicMode(MusicMode::Quiet)))
    );
}

#[test]
fn soldier_death_detaches_both_combat_neighbours_without_touching_another_line() {
    use crate::entity_id::SoldierId;

    let mut engine = EngineInner::new();
    // Reserve handle zero, which EnemyAi uses as its null neighbour sentinel.
    // This models Lane 36's Soldier 90 — dying Soldier 91 — Soldier 54 line.
    let _sentinel =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let left = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let victim = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let right = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let unrelated_left =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let unrelated_right =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));

    let left_handle = left.index();
    let victim_handle = victim.index();
    let right_handle = right.index();
    let unrelated_left_handle = unrelated_left.index();
    let unrelated_right_handle = unrelated_right.index();
    for (entity, left_neighbour, right_neighbour) in [
        (
            left,
            None,
            Some(crate::ai::AiEntityHandle::new(victim_handle)),
        ),
        (
            victim,
            Some(crate::ai::AiEntityHandle::new(left_handle)),
            Some(crate::ai::AiEntityHandle::new(right_handle)),
        ),
        (
            right,
            Some(crate::ai::AiEntityHandle::new(victim_handle)),
            None,
        ),
        (
            unrelated_left,
            None,
            Some(crate::ai::AiEntityHandle::new(unrelated_right_handle)),
        ),
        (
            unrelated_right,
            Some(crate::ai::AiEntityHandle::new(unrelated_left_handle)),
            None,
        ),
    ] {
        let Some(Entity::Soldier(soldier)) = engine.get_entity_mut(entity) else {
            panic!("combat-neighbour fixture contains a non-soldier")
        };
        let enemy = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test soldier has enemy AI");
        enemy.left_combat_neighbour = left_neighbour;
        enemy.right_combat_neighbour = right_neighbour;
    }

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.handle_death(&crate::sim_rng::test_context(), &assets, victim);

    let links = |engine: &EngineInner, handle: u32| {
        let Some(Entity::Soldier(soldier)) = engine
            .world
            .entities
            .get(EntityId::Soldier(SoldierId(handle)))
        else {
            panic!("combat-neighbour fixture soldier disappeared")
        };
        let enemy = soldier
            .npc
            .ai_brain
            .enemy()
            .expect("test soldier retains enemy AI");
        (enemy.left_combat_neighbour, enemy.right_combat_neighbour)
    };

    assert_eq!(links(&engine, left_handle), (None, None));
    assert_eq!(links(&engine, victim_handle), (None, None));
    assert_eq!(links(&engine, right_handle), (None, None));
    assert_eq!(
        links(&engine, unrelated_left_handle),
        (
            None,
            Some(crate::ai::AiEntityHandle::new(unrelated_right_handle))
        )
    );
    assert_eq!(
        links(&engine, unrelated_right_handle),
        (
            Some(crate::ai::AiEntityHandle::new(unrelated_left_handle)),
            None
        )
    );
}

#[test]
fn soldier_death_applies_queued_reciprocal_combat_neighbour_clears() {
    use crate::ai::CrossNpcAction;
    use crate::entity_id::SoldierId;

    let mut engine = EngineInner::new();
    let _sentinel =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let old_left = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let victim = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let old_right =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let victim_handle = victim.index();
    let old_left_handle = old_left.index();
    let old_right_handle = old_right.index();

    for (entity, left_neighbour, right_neighbour) in [
        (
            old_left,
            None,
            Some(crate::ai::AiEntityHandle::new(victim_handle)),
        ),
        (
            old_right,
            Some(crate::ai::AiEntityHandle::new(victim_handle)),
            None,
        ),
    ] {
        let Some(Entity::Soldier(soldier)) = engine.get_entity_mut(entity) else {
            panic!("queued combat-neighbour fixture contains a non-soldier")
        };
        let enemy = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("test soldier has enemy AI");
        enemy.left_combat_neighbour = left_neighbour;
        enemy.right_combat_neighbour = right_neighbour;
    }
    let Some(Entity::Soldier(victim_soldier)) = engine.get_entity_mut(victim) else {
        panic!("queued combat-neighbour victim is not a soldier")
    };
    let victim_enemy = victim_soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("test victim has enemy AI");
    assert_eq!(victim_enemy.left_combat_neighbour, None);
    assert_eq!(victim_enemy.right_combat_neighbour, None);
    victim_enemy
        .base
        .outbox
        .reentrant
        .cross_npc_actions
        .extend([
            CrossNpcAction::SetRightCombatNeighbour {
                target: old_left_handle,
                neighbour: None,
            },
            CrossNpcAction::SetLeftCombatNeighbour {
                target: old_right_handle,
                neighbour: None,
            },
        ]);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.handle_death(&crate::sim_rng::test_context(), &assets, victim);

    let links = |engine: &EngineInner, handle: u32| {
        let Some(Entity::Soldier(soldier)) = engine
            .world
            .entities
            .get(EntityId::Soldier(SoldierId(handle)))
        else {
            panic!("queued combat-neighbour fixture soldier disappeared")
        };
        let enemy = soldier
            .npc
            .ai_brain
            .enemy()
            .expect("test soldier retains enemy AI");
        (enemy.left_combat_neighbour, enemy.right_combat_neighbour)
    };
    assert_eq!(links(&engine, old_left_handle), (None, None));
    assert_eq!(links(&engine, victim_handle), (None, None));
    assert_eq!(links(&engine, old_right_handle), (None, None));
}

#[test]
fn enemy_ai_hero_cross_owner_combat_neighbours_preserve_pc_kind() {
    use crate::ai::CrossNpcAction;
    use crate::element::{AiActorData, AiBrain};

    let mut engine = EngineInner::new();
    // EnemyAi reserves raw handle zero as its null human pointer.
    let _sentinel =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let make_enemy_ai_hero = || {
        let mut entity = make_test_pc(crate::element::Posture::Upright);
        let Entity::Pc(pc) = &mut entity else {
            unreachable!()
        };
        pc.element.active = true;
        pc.pc.command_interface = crate::human_control::CommandInterface::None;
        pc.pc.mission_role = crate::human_control::MissionRole::Combatant;
        pc.pc.combat_stance = crate::human_control::CombatStance::Aggressive;
        pc.pc.ai = Some(Box::new(AiActorData {
            ai_brain: AiBrain::Enemy(Box::default()),
            ..AiActorData::default()
        }));
        entity
    };
    let owner = engine.add_test_entity(make_enemy_ai_hero());
    let left = engine.add_test_entity(make_enemy_ai_hero());
    for id in [owner, left] {
        engine
            .get_entity_mut(id)
            .and_then(Entity::enemy_ai_mut)
            .expect("AI-controlled hero has EnemyAi")
            .base
            .me = id.index();
    }
    engine
        .get_entity_mut(owner)
        .and_then(Entity::ai_controller_mut)
        .expect("AI-controlled hero has AI controller")
        .outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::UpdateLeftCombatNeighbour {
            target: owner.index(),
            old_left: None,
            new_left: Some(crate::ai::AiEntityHandle::new(left.index())),
        });

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.process_synchronous_reentrant_actions_for(
        &crate::sim_rng::test_context(),
        owner,
        &assets,
    );

    let owner_enemy = engine
        .get_entity(owner)
        .and_then(Entity::enemy_ai)
        .expect("owner PC retains EnemyAi");
    assert_eq!(
        owner_enemy.left_combat_neighbour,
        Some(crate::ai::AiEntityHandle::new(left.index()))
    );
    let left_enemy = engine
        .get_entity(left)
        .and_then(Entity::enemy_ai)
        .expect("left PC retains EnemyAi");
    assert_eq!(
        left_enemy.right_combat_neighbour,
        Some(crate::ai::AiEntityHandle::new(owner.index()))
    );
}

#[test]
fn enemy_ai_hero_death_detaches_pc_combat_neighbours() {
    use crate::element::{AiActorData, AiBrain};

    let mut engine = EngineInner::new();
    let _sentinel =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let make_enemy_ai_hero = || {
        let mut entity = make_test_pc(crate::element::Posture::Upright);
        let Entity::Pc(pc) = &mut entity else {
            unreachable!()
        };
        pc.element.active = true;
        pc.pc.life_points = 100;
        pc.pc.command_interface = crate::human_control::CommandInterface::None;
        pc.pc.mission_role = crate::human_control::MissionRole::Combatant;
        pc.pc.combat_stance = crate::human_control::CombatStance::Aggressive;
        pc.pc.ai = Some(Box::new(AiActorData {
            ai_brain: AiBrain::Enemy(Box::default()),
            ..AiActorData::default()
        }));
        entity
    };
    let left = engine.add_test_entity(make_enemy_ai_hero());
    let victim = engine.add_test_entity(make_enemy_ai_hero());
    let right = engine.add_test_entity(make_enemy_ai_hero());
    for id in [left, victim, right] {
        engine
            .get_entity_mut(id)
            .and_then(Entity::enemy_ai_mut)
            .expect("AI-controlled hero has EnemyAi")
            .base
            .me = id.index();
    }
    {
        let enemy = engine
            .get_entity_mut(left)
            .and_then(Entity::enemy_ai_mut)
            .expect("left PC has EnemyAi");
        enemy.right_combat_neighbour = Some(crate::ai::AiEntityHandle::new(victim.index()));
    }
    {
        let enemy = engine
            .get_entity_mut(victim)
            .and_then(Entity::enemy_ai_mut)
            .expect("victim PC has EnemyAi");
        enemy.left_combat_neighbour = Some(crate::ai::AiEntityHandle::new(left.index()));
        enemy.right_combat_neighbour = Some(crate::ai::AiEntityHandle::new(right.index()));
    }
    {
        let enemy = engine
            .get_entity_mut(right)
            .and_then(Entity::enemy_ai_mut)
            .expect("right PC has EnemyAi");
        enemy.left_combat_neighbour = Some(crate::ai::AiEntityHandle::new(victim.index()));
    }

    let mut assets = LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .characters
        .push(crate::profiles::CharacterProfile::default());
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.handle_death(&crate::sim_rng::test_context(), &assets, victim);

    assert_eq!(
        engine
            .get_entity(left)
            .and_then(Entity::enemy_ai)
            .expect("left PC retains EnemyAi")
            .right_combat_neighbour,
        None
    );
    assert_eq!(
        engine
            .get_entity(right)
            .and_then(Entity::enemy_ai)
            .expect("right PC retains EnemyAi")
            .left_combat_neighbour,
        None
    );
}

#[test]
fn review2_combat_alert_preserves_original_busy_lock_acceptance() {
    use crate::ai::{AiLockFlags, Position, StimulusType};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    engine
        .get_entity_mut(soldier_id)
        .and_then(Entity::ai_controller_mut)
        .expect("review2 combat-alert soldier has AI")
        .locks_flag_field = AiLockFlags::BUSY;
    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    let global = engine.ai.global.clone();
    engine
        .get_entity_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("review2 officer has EnemyAi")
        .command_soldiers_to_attack(
            Position {
                x: 100.0,
                ..Default::default()
            },
            &global,
            None,
            &ctx,
            &tick,
        );
    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);

    assert_eq!(
        engine
            .get_entity(officer_id)
            .and_then(Entity::enemy_ai)
            .expect("review2 officer retains EnemyAi")
            .alerted_us
            .as_slice(),
        &[soldier_id.index()],
        "AI decisions report success when decision entry retains a BUSY stimulus"
    );
    assert_eq!(
        engine
            .get_entity(soldier_id)
            .and_then(Entity::ai_controller)
            .expect("review2 combat-alert soldier retains AI")
            .stimulus_queue
            .last()
            .map(|s| s.stimulus_type),
        Some(StimulusType::CallCombatAlert)
    );
}

#[test]
fn final_review_combat_alert_all_refused_enters_reserve_without_success_remark() {
    use crate::ai::{AiState, Position, Remark, Substate};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    let global = engine.ai.global.clone();
    engine
        .get_entity_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("combat-alert caller has EnemyAi")
        .command_soldiers_to_attack(
            Position {
                x: 300.0,
                ..Default::default()
            },
            &global,
            None,
            &ctx,
            &tick,
        );
    engine
        .get_entity_mut(soldier_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("combat-alert recipient has EnemyAi")
        .set_state(AiState::Fleeing, Substate::FleeingRunToDoor);

    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);

    let officer = engine
        .get_entity(officer_id)
        .and_then(Entity::enemy_ai)
        .expect("combat-alert caller retains EnemyAi");
    assert!(officer.alerted_us.is_empty());
    assert_eq!(officer.base.current_state, AiState::Attacking);
    assert_eq!(officer.base.current_substate, Substate::AttackingReserve);
    assert_ne!(officer.base.current_remark, Remark::OfficerGivesAttackOrder);
    assert!(
        !engine
            .orders
            .sequence_manager
            .sequences_iter()
            .any(|sequence| {
                sequence.elements.iter().any(|element| {
                    element.owner == Some(officer_id)
                        && matches!(
                            element.command,
                            crate::element::Command::GatherSoldiers
                                | crate::element::Command::Point
                        )
                })
            })
    );
}

#[test]
fn command_soldiers_to_attack_does_not_overwrite_acceptor_gather_instruction() {
    use crate::ai::{AiState, Position, Remark, Substate};
    use crate::profiles::ProfileRank;

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, refused_id, mut assets) = setup_review2_officer_and_soldier();
    let accepted_id =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let Entity::Soldier(accepted) = engine
        .get_entity_mut(accepted_id)
        .expect("partial-refusal acceptor exists")
    else {
        panic!("partial-refusal acceptor changed kind")
    };
    accepted.element.active = true;
    accepted.element.set_position_map(MapPoint::new(0.0, 80.0));
    accepted.npc.life_points = 100;
    let accepted_ai = accepted
        .npc
        .ai_brain
        .enemy_mut()
        .expect("partial-refusal acceptor has EnemyAi");
    accepted_ai.base.me = accepted_id.index();
    accepted_ai.soldier_profile_rank = ProfileRank::Soldier;
    accepted_ai.set_state(AiState::Default, Substate::DefaultOnPost);
    accepted_ai.gather_direction = 10;
    complete_test_runtime_fixture(&mut engine, &mut assets);
    install_test_open_field_bbox(&mut engine);
    engine
        .get_entity_mut(officer_id)
        .expect("partial-refusal officer exists")
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_coords(
            -5.0, -5.0, 5.0, 5.0,
        ));

    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    assert_eq!(tick.camp_soldiers.len(), 2);
    let global = engine.ai.global.clone();
    let grid = &engine.world.fast_grid;
    engine
        .world
        .entities
        .get_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("partial-refusal caller has EnemyAi")
        .command_soldiers_to_attack(
            Position {
                x: 300.0,
                ..Default::default()
            },
            &global,
            Some(grid),
            &ctx,
            &tick,
        );
    engine
        .get_entity_mut(refused_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("partial-refusal rejector has EnemyAi")
        .set_state(AiState::Fleeing, Substate::FleeingRunToDoor);

    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);

    let officer = engine
        .get_entity(officer_id)
        .and_then(Entity::enemy_ai)
        .expect("partial-refusal caller retains EnemyAi");
    assert_eq!(officer.alerted_us, vec![accepted_id.index()]);
    assert_eq!(
        officer.base.current_substate,
        Substate::AttackingOfficerGivingOrders
    );
    assert_eq!(officer.base.current_remark, Remark::OfficerGivesAttackOrder);
    let accepted = engine
        .get_entity(accepted_id)
        .and_then(Entity::enemy_ai)
        .expect("partial-refusal acceptor retains EnemyAi");
    assert!(
        !accepted.gather_position_instructed,
        "officer attack commands only use accepted soldiers to orient the officer; they never assign formation slots"
    );
    assert_eq!(
        accepted.gather_direction, 10,
        "a combat alert must preserve a direction authored by an independent state such as DoorFight"
    );
    let refused = engine
        .get_entity(refused_id)
        .and_then(Entity::enemy_ai)
        .expect("partial-refusal rejector retains EnemyAi");
    assert!(!refused.gather_position_instructed);
}

#[test]
fn final_review_combat_alert_requires_recipient_360_detection() {
    use crate::ai::Position;

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    engine
        .get_entity_mut(soldier_id)
        .and_then(|entity| match entity {
            Entity::Soldier(soldier) => Some(&mut soldier.npc),
            _ => None,
        })
        .expect("360-degree recipient is a soldier")
        .view_radius = 10;
    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    let global = engine.ai.global.clone();
    let start = engine
        .get_entity_mut(officer_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("360-degree caller has EnemyAi")
        .command_soldiers_to_attack(
            Position {
                x: 300.0,
                ..Default::default()
            },
            &global,
            None,
            &ctx,
            &tick,
        );
    assert_eq!(start, crate::ai_enemy::CommandSoldiersStart::Rejected);
    assert!(
        engine
            .get_entity(officer_id)
            .and_then(Entity::ai_controller)
            .expect("360-degree caller retains AI")
            .outbox
            .reentrant
            .cross_npc_actions
            .is_empty()
    );
}

#[test]
fn closure_review_combat_alert_uses_exact_is_able_to_fight_under_retained_lock() {
    use crate::ai::{AiLockFlags, AiState, Substate};
    use crate::element::Posture;

    #[derive(Clone, Copy, Debug)]
    enum Ineligible {
        Fleeing,
        Menacing,
        Tied,
        Carried,
        GotHit,
        GotHitStandingUp,
        Hitting,
    }

    let sim = crate::sim_rng::test_context();
    for case in [
        Ineligible::Fleeing,
        Ineligible::Menacing,
        Ineligible::Tied,
        Ineligible::Carried,
        Ineligible::GotHit,
        Ineligible::GotHitStandingUp,
        Ineligible::Hitting,
    ] {
        let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(soldier_id)
            .expect("eligibility recipient exists")
        else {
            panic!("eligibility recipient changed kind")
        };
        soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("eligibility recipient has EnemyAi")
            .base
            .locks_flag_field = AiLockFlags::BUSY | AiLockFlags::FREEZE;
        match case {
            Ineligible::Fleeing => soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("eligibility recipient has EnemyAi")
                .set_state(AiState::Fleeing, Substate::FleeingRunToDoor),
            Ineligible::Menacing => soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("eligibility recipient has EnemyAi")
                .set_state(AiState::Menacing, Substate::MenacingPcInComa),
            Ineligible::Tied => soldier.element.publish_order_posture(Posture::Tied),
            Ineligible::Carried => soldier.human.carrier = Some(officer_id),
            Ineligible::GotHit => soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("eligibility recipient has EnemyAi")
                .set_state(AiState::Attacking, Substate::AttackingGotHit),
            Ineligible::GotHitStandingUp => soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("eligibility recipient has EnemyAi")
                .set_state(AiState::Attacking, Substate::AttackingGotHitStandingUp),
            Ineligible::Hitting => soldier
                .npc
                .ai_brain
                .enemy_mut()
                .expect("eligibility recipient has EnemyAi")
                .set_state(AiState::Attacking, Substate::AttackingHitting),
        }

        let (start, tick) = start_review_command_soldiers(&mut engine, &sim, &assets, officer_id);
        let candidate = tick
            .camp_soldiers
            .iter()
            .find(|candidate| candidate.handle == soldier_id.index())
            .expect("ineligible active recipient remains represented in camp snapshot");
        assert!(!candidate.is_able_to_fight, "case {case:?}");
        assert_eq!(
            start,
            crate::ai_enemy::CommandSoldiersStart::Rejected,
            "case {case:?} must be rejected before retained-lock Think"
        );
        assert!(
            engine
                .get_entity(officer_id)
                .and_then(Entity::ai_controller)
                .expect("eligibility caller retains AI")
                .outbox
                .reentrant
                .cross_npc_actions
                .is_empty(),
            "case {case:?} must not be called"
        );
    }
}

#[test]
fn closure_review_combat_alert_closed_eyes_do_not_disable_360_detection() {
    use crate::ai::{AiLockFlags, StimulusType};
    use crate::element::EyeStatus;

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, assets) = setup_review2_officer_and_soldier();
    let Entity::Soldier(soldier) = engine
        .get_entity_mut(soldier_id)
        .expect("closed-eye recipient exists")
    else {
        panic!("closed-eye recipient changed kind")
    };
    soldier.npc.eye_status = EyeStatus::Closed;
    soldier
        .npc
        .ai_brain
        .enemy_mut()
        .expect("closed-eye recipient has EnemyAi")
        .base
        .locks_flag_field = AiLockFlags::BUSY;

    let (start, tick) = start_review_command_soldiers(&mut engine, &sim, &assets, officer_id);
    let candidate = tick
        .camp_soldiers
        .iter()
        .find(|candidate| candidate.handle == soldier_id.index())
        .expect("closed-eye recipient is in camp snapshot");
    assert!(candidate.eye_blind);
    assert!(candidate.is_able_to_fight);
    assert_eq!(start, crate::ai_enemy::CommandSoldiersStart::Pending);

    engine.drain_direct_ai_owner_boundary(&sim, officer_id, &assets);
    assert_eq!(
        engine
            .get_entity(officer_id)
            .and_then(Entity::enemy_ai)
            .expect("closed-eye caller retains EnemyAi")
            .alerted_us,
        vec![soldier_id.index()]
    );
    assert_eq!(
        engine
            .get_entity(soldier_id)
            .and_then(Entity::ai_controller)
            .expect("closed-eye recipient retains AI")
            .stimulus_queue
            .last()
            .map(|stimulus| stimulus.stimulus_type),
        Some(StimulusType::CallCombatAlert),
        "BUSY retains and accepts the stimulus after eligibility"
    );
}
