use super::*;

#[test]
fn set_actor_location_honolulu_finishes_before_same_callback_unlock() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

    engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerHonolulu",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("Honolulu action and continuation should finish synchronously");

    let entity = engine
        .get_entity(receiver)
        .expect("scripted receiver remains present");
    assert!(!entity.element_data().active);
    assert!(entity.element_data().in_honolulu);
    assert!(
        !entity
            .ai_controller()
            .expect("receiver NPC AI")
            .script_locked,
        "UnlockAI after SetActorLocation(NULL) observed and cleared the inline lock"
    );
}

#[test]
fn set_actor_location_preserves_original_partial_order_and_bool_contract() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let mut assets = LevelAssets::new();
    assets.scripts.location_count = 2;
    assets.scripts.point_count = 1;
    assets.scripts.location_positions = std::sync::Arc::new(vec![(12.0, 34.0), (90.0, 91.0)]);
    assets.scripts.location_layers = std::sync::Arc::new(vec![1, 1]);
    assets.scripts.location_sectors = std::sync::Arc::new(vec![7, 7]);
    engine.attach_script_bindings(&assets);

    {
        let element = engine
            .get_entity_mut(receiver)
            .expect("receiver")
            .element_data_mut();
        element.active = false;
        element.in_honolulu = true;
    }
    let sector_location = ScriptHandleCodec::location_handle_from_index(1);
    assert_eq!(
        engine
            .call_external_native(
                &crate::sim_rng::test_context(),
                &assets,
                "SetActorLocation",
                &[handle, sector_location]
            )
            .expect("wrong location type is a false result, not a driver error"),
        0
    );
    let element = engine.get_entity(receiver).unwrap().element_data();
    assert!(element.active && !element.in_honolulu);
    assert_eq!(
        element.position_map(),
        crate::coordinates::MapPoint::default()
    );

    let point_location = ScriptHandleCodec::location_handle_from_index(0);
    assert_eq!(
        engine
            .call_external_native(
                &crate::sim_rng::test_context(),
                &assets,
                "SetActorLocation",
                &[handle, point_location]
            )
            .expect("non-motion sector is a false result"),
        0
    );
    let element = engine.get_entity(receiver).unwrap().element_data();
    assert_eq!(
        element.position_map(),
        crate::coordinates::MapPoint::new(12.0, 34.0)
    );
    assert_eq!(element.layer(), 1);
    assert_eq!(element.sector().map(u16::from), Some(7));

    let mut level = crate::fast_find_grid::LevelGrid::default();
    level.sectors.push(crate::fast_find_grid::GridSector {
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
        layer: 1,
        sector_number: crate::sector::SectorNumber::new(7),
        door_index: None,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    });
    engine.world.fast_grid_mut().level = std::sync::Arc::new(level);
    assert_eq!(
        engine
            .call_external_native(
                &crate::sim_rng::test_context(),
                &assets,
                "SetActorLocation",
                &[handle, point_location]
            )
            .expect("valid point succeeds"),
        1
    );
}

#[test]
fn set_actor_location_preserves_authored_sparse_sector_identity() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let mut assets = LevelAssets::new();
    assets.scripts.location_count = 1;
    assets.scripts.point_count = 1;
    assets.scripts.location_positions = std::sync::Arc::new(vec![(12.0, 34.0)]);
    assets.scripts.location_layers = std::sync::Arc::new(vec![1]);
    assets.scripts.location_sectors = std::sync::Arc::new(vec![7]);
    let exact_index = crate::fast_find_grid::SectorIndex::new(1).unwrap();
    let exact_sector = crate::position_interface::SectorHandle::new(7)
        .unwrap()
        .with_arena_index(exact_index);
    assets.scripts.location_sector_handles = std::sync::Arc::new(vec![Some(exact_sector)]);
    engine.attach_script_bindings(&assets);

    let sector = crate::fast_find_grid::GridSector {
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
        layer: 1,
        sector_number: crate::sector::SectorNumber::new(7),
        door_index: None,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    };
    let mut level = crate::fast_find_grid::LevelGrid::default();
    level.sectors.push(sector.clone());
    level.sectors.push(sector);
    engine.world.fast_grid_mut().level = std::sync::Arc::new(level);

    let point_location = ScriptHandleCodec::location_handle_from_index(0);
    assert_eq!(
        engine
            .call_external_native(
                &crate::sim_rng::test_context(),
                &assets,
                "SetActorLocation",
                &[handle, point_location]
            )
            .expect("valid authored point succeeds"),
        1
    );
    assert_eq!(
        engine.get_entity(receiver).unwrap().element_data().sector(),
        Some(exact_sector),
        "the following RecordMove must see the same RHsector pointer as its authored goal"
    );
}

#[test]
fn persistent_life_and_concussion_are_visible_after_engine_yield_in_same_callback() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

    let life = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerLife",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("life setter resumes");
    assert_eq!(life, 37);
    assert_eq!(
        engine
            .get_entity(receiver)
            .expect("receiver")
            .npc_data()
            .expect("NPC")
            .life_points,
        37
    );

    let concussion = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerConcussion",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("concussion setter resumes");
    assert_eq!(concussion, 123);
    assert_eq!(
        engine
            .get_entity(receiver)
            .expect("receiver")
            .human_data()
            .expect("human")
            .concussion_of_the_brain,
        123
    );
}

#[test]
fn persistent_setters_use_original_narrowing_and_virtual_kill_path() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

    // The VM hands the trailing arguments to the native through a
    // signed-byte read, so every script constant above 127 already arrives
    // negative (250 becomes -6). The concussion setter then narrows the
    // sign-extended amount to UWORD and zeroes any value that re-reads
    // negative as SWORD.
    for amount in [-1, 65_535, 250] {
        assert_eq!(
            engine
                .call_external_native(
                    &crate::sim_rng::test_context(),
                    &assets,
                    "SetPersistentProperty",
                    &[handle, 3, amount],
                )
                .expect("concussion setter"),
            1
        );
        let human = engine
            .get_entity(receiver)
            .expect("receiver")
            .human_data()
            .expect("human");
        assert_eq!(
            human.concussion_of_the_brain, 0,
            "byte-narrowed amount {amount:#x} is negative when re-read as SWORD"
        );
        assert!(!human.unconscious);
    }

    {
        let entity = engine.get_entity_mut(receiver).unwrap();
        entity.npc_data_mut().unwrap().alerted = true;
        entity.enemy_ai_mut().unwrap().forced_attentive = true;
    }

    let sequence_count = engine.orders.sequence_manager.sequence_count();
    assert_eq!(
        engine
            .call_external_native(
                &crate::sim_rng::test_context(),
                &assets,
                "SetPersistentProperty",
                &[handle, 2, 65_535]
            )
            .expect("life setter"),
        1
    );
    let entity = engine.get_entity(receiver).expect("receiver");
    assert_eq!(entity.npc_data().expect("NPC").life_points, 0);
    assert_eq!(
        entity.ai_controller().expect("NPC AI").current_substate,
        crate::ai::Substate::SleepingForever
    );
    assert!(!entity.npc_data().expect("NPC").alerted);
    let enemy = entity.enemy_ai().expect("soldier enemy AI");
    assert_eq!(
        enemy.base.current_music_alert_status,
        crate::ai::AlertLevel::Green
    );
    assert_eq!(
        enemy.base.view_alert_status,
        crate::ai::AlertLevel::Yellow,
        "virtual Kill preserves the forced-attentive Green→Yellow view override"
    );
    assert_eq!(
        engine.orders.sequence_manager.sequence_count(),
        sequence_count,
        "script SetLifePoints must not synthesize a ReceiveDamage sequence"
    );
}

#[test]
fn scripted_invulnerable_life_setter_forces_literal_one_hundred() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    {
        let entity = engine.get_entity_mut(receiver).expect("receiver");
        entity.human_data_mut().unwrap().invulnerable = true;
        entity.npc_data_mut().unwrap().life_points = 80;
    }

    assert_eq!(
        engine
            .call_external_native(
                &crate::sim_rng::test_context(),
                &assets,
                "SetPersistentProperty",
                &[handle, 2, 0]
            )
            .expect("scripted invulnerable life setter"),
        1
    );
    assert_eq!(
        engine
            .get_entity(receiver)
            .unwrap()
            .npc_data()
            .unwrap()
            .life_points,
        100,
        "RHElementActorHuman::SetLifePoints stores literal 100 for invulnerable humans"
    );
}

#[test]
fn scripted_pc_concussion_and_ko_unselect_immediately() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let mut assets = LevelAssets::new();
    let make_pc = || {
        let mut entity = Entity::Pc(crate::element::ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: crate::element::PcData {
                playable: true,
                ..crate::element::PcData::default()
            },
        });
        let row = crate::sprite_script::SpriteScript {
            action_id: 0,
            action_done: 0,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![0],
            delays: vec![0],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        entity.element_data_mut().sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![row]),
            std::sync::Arc::new(vec![
                crate::sprite_script::UNMAPPED;
                crate::sprite_script::NONANIMATION_END
            ]),
        );
        entity
    };
    let persistent_pc = engine.add_entity(make_pc());
    let posture_pc = engine.add_entity(make_pc());
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    let persistent_handle = ScriptHandleCodec::actor_handle(persistent_pc);
    let posture_handle = ScriptHandleCodec::actor_handle(posture_pc);
    engine.players.seats[0].selection = vec![persistent_pc, posture_pc];

    // The native's trailing arguments pass through a signed-byte read, so
    // the KO amount must sit in 70..=127 to survive narrowing above the
    // concussion threshold (CONCUSSION_MAX itself would wrap to 44).
    engine
        .call_external_native(
            &crate::sim_rng::test_context(),
            &assets,
            "SetPersistentProperty",
            &[persistent_handle, 3, 100],
        )
        .expect("persistent concussion");
    assert_eq!(engine.players.seats[0].selection, vec![posture_pc]);

    engine
        .call_external_native(
            &crate::sim_rng::test_context(),
            &assets,
            "SetActorPosture",
            &[posture_handle, 17],
        )
        .expect("posture KO");
    assert!(engine.players.seats[0].selection.is_empty());
}

#[test]
fn posture_wait_uses_real_instruction_path_before_callback_resumes() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);

    let posture = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerPosture",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("posture WAIT should dispatch and resume");
    assert_eq!(posture, 2);
    let (sequence_id, element_index) = engine
        .orders
        .sequence_manager
        .current_element_for_actor(receiver)
        .expect("WAIT is the actor's live canonical instruction");
    let wait = engine
        .orders
        .sequence_manager
        .get_element(sequence_id, element_index)
        .expect("WAIT element");
    assert_eq!(wait.command, Command::Wait);
    assert_eq!(wait.state, SequenceState::InProgress);
}

#[test]
fn anonymous_archer_accepts_a_non_pc_human_and_adds_hidden_titbit_inline() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    assert_eq!(
        engine
            .call_external_native(
                &crate::sim_rng::test_context(),
                &assets,
                "SetActorPosture",
                &[handle, 100]
            )
            .expect("AnonymousArcher should accept a human soldier"),
        0
    );
    let entity = engine.get_entity(receiver).expect("receiver");
    assert_eq!(entity.element_data().posture, Posture::AnonymousArcher);
    assert_eq!(
        entity.actor_data().expect("actor").action_state,
        crate::element::ActionState::Waiting
    );
    assert!(engine.feedback.titbit_manager.titbit_exists(
        crate::titbit::TitbitKind::Hidden,
        crate::titbit::ElementHandle(receiver.index()),
    ));
}

#[test]
#[should_panic(expected = "SetActorPosture(UPRIGHT) from CarryingCorpse requires a PC")]
fn upright_from_carrying_corpse_rejects_a_non_pc_human() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    engine
        .get_entity_mut(receiver)
        .expect("receiver")
        .set_posture(Posture::CarryingCorpse);

    let _ = engine.call_external_native(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        "SetActorPosture",
        &[handle, 0],
    );
}

#[test]
fn action_state_set_get_resumes_after_real_wait_instruction() {
    let (mut engine, receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    let state = engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerActionState",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("action-state setter should dispatch WAIT and resume");
    assert_eq!(state, crate::element::ActionState::Bored as i32);
    assert_eq!(
        engine
            .get_entity(receiver)
            .expect("receiver")
            .actor_data()
            .expect("actor")
            .action_state,
        crate::element::ActionState::Bored
    );
    let current = engine
        .orders
        .sequence_manager
        .current_element_for_actor(receiver)
        .expect("WAIT is live before callback return");
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(current.0, current.1)
            .expect("WAIT element")
            .command,
        Command::Wait
    );
}

#[test]
fn recorded_timer_is_registered_before_thanx_returns() {
    let (mut engine, _receiver, handle) = engine_with_receiver();
    let assets = LevelAssets::new();

    engine
        .call_script_vm(
            &crate::sim_rng::test_context(),
            &assets,
            super::ScriptVmKey::Actor(handle),
            "TriggerTimer",
            &[],
            crate::natives::ScriptCallFrame::actor(handle),
        )
        .expect("recorded timer should execute immediately");

    assert_eq!(engine.orders.timer_elements.len(), 1);
    assert_eq!(engine.orders.timer_elements[0].remaining, 12);
    let timer_ref = engine.orders.timer_elements[0].element_ref;
    let timer = engine
        .orders
        .sequence_manager
        .get_element(timer_ref.sequence_id, timer_ref.element_index)
        .expect("timer sequence element");
    assert_eq!(timer.command, Command::Timer);
    assert_eq!(
        timer.state,
        SequenceState::Todo,
        "Original ExecutedImmediately registers Timer before Go, so it stays TODO until expiry terminates it"
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_pending_immediate_actions()
    );
}

#[test]
fn recorded_lock_user_clears_and_restores_selection_in_original_order() {
    let (mut engine, _receiver, _handle) = engine_with_receiver();
    let assets = LevelAssets::new();
    let pc_id = engine.add_entity(Entity::Pc(crate::element::ActorPc {
        element: ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: crate::element::PcData {
            playable: true,
            current_action: crate::profiles::Action::Bow,
            ..crate::element::PcData::default()
        },
    }));
    engine.players.seats[0].selection.push(pc_id);

    // MSG_UNLOCK_USER only adds the saved selection. With no preceding lock,
    // it must leave the live selection untouched.
    engine
        .call_external_native(&crate::sim_rng::test_context(), &assets, "Start", &[])
        .unwrap();
    engine
        .call_external_native(
            &crate::sim_rng::test_context(),
            &assets,
            "RecordUnLockUser",
            &[],
        )
        .unwrap();
    engine
        .call_external_native(&crate::sim_rng::test_context(), &assets, "Thanx", &[])
        .unwrap();
    assert_eq!(engine.players.seats[0].selection, vec![pc_id]);

    engine
        .call_external_native(&crate::sim_rng::test_context(), &assets, "Start", &[])
        .unwrap();
    engine
        .call_external_native(
            &crate::sim_rng::test_context(),
            &assets,
            "RecordLockUser",
            &[],
        )
        .unwrap();
    engine
        .call_external_native(&crate::sim_rng::test_context(), &assets, "Thanx", &[])
        .unwrap();
    assert!(engine.players.user_locked);
    assert!(engine.players.seats[0].selection.is_empty());
    assert_eq!(
        engine
            .get_entity(pc_id)
            .unwrap()
            .pc_data()
            .unwrap()
            .current_action,
        crate::profiles::Action::NoAction
    );

    engine
        .call_external_native(&crate::sim_rng::test_context(), &assets, "Start", &[])
        .unwrap();
    engine
        .call_external_native(
            &crate::sim_rng::test_context(),
            &assets,
            "RecordUnLockUser",
            &[],
        )
        .unwrap();
    engine
        .call_external_native(&crate::sim_rng::test_context(), &assets, "Thanx", &[])
        .unwrap();
    assert!(!engine.players.user_locked);
    assert_eq!(engine.players.seats[0].selection, vec![pc_id]);
    assert_eq!(engine.players.selection_before_user_lock, vec![pc_id]);

    // The original saved list is not consumed; repeated Unlock remains an
    // additive idempotent selection restore.
    engine
        .call_external_native(&crate::sim_rng::test_context(), &assets, "Start", &[])
        .unwrap();
    engine
        .call_external_native(
            &crate::sim_rng::test_context(),
            &assets,
            "RecordUnLockUser",
            &[],
        )
        .unwrap();
    engine
        .call_external_native(&crate::sim_rng::test_context(), &assets, "Thanx", &[])
        .unwrap();
    assert_eq!(engine.players.seats[0].selection, vec![pc_id]);
    assert_eq!(engine.players.selection_before_user_lock, vec![pc_id]);
    assert!(engine.feedback.pending_side_effects.pending_reset_input);
}
