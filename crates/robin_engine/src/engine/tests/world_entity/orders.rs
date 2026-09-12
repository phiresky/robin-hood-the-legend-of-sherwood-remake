use super::*;

#[test]
fn removal_cleans_all_seats_and_owned_queues_without_reordering_survivors() {
    use crate::ai::{AiEntityHandle, Stimulus, StimulusInfo, StimulusType};
    use crate::engine::movement::{FailedPathRequest, PendingPathRequest, PendingPathRequestQueue};
    use crate::engine::seat::SeatState;
    use crate::order::{AiOrderIntent, OrderType};
    use crate::sequence::SequenceId;

    let mut engine = EngineInner::new();
    let removed = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let first = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let last = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let observer = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Royalists));
    engine.players.seats.push(SeatState::default()); // disconnected seat is still an owner
    for (index, seat) in engine.players.seats.iter_mut().enumerate() {
        seat.selection = vec![first, removed, last];
        seat.quick_select_groups[2] = vec![last, removed, first];
        seat.planned_shield_target = Some(if index == 0 {
            (first, removed)
        } else {
            (removed, last)
        });
        seat.follow_element = Some(if index == 0 { removed } else { last });
        seat.locker_active = true;
    }
    engine.players.selection_before_user_lock = vec![last, removed, first];
    let mut historical = Stimulus::new(StimulusType::EventDone);
    historical.owner = Some(AiEntityHandle::new(removed.index()));
    let mut stale = Stimulus::new(StimulusType::EventDone);
    stale.info = StimulusInfo::Human(AiEntityHandle::new(removed.index()));
    let mut stale_object = stale;
    stale_object.info = StimulusInfo::Object(AiEntityHandle::new(removed.index()));
    let final_stimulus = Stimulus::new(StimulusType::EventReachPoint);
    let queued = vec![historical, stale, stale_object, final_stimulus];
    let ai = engine
        .get_entity_mut(observer)
        .unwrap()
        .ai_controller_mut()
        .unwrap();
    ai.stimulus_queue = queued.clone();
    ai.outbox.detection.stimuli = queued;

    let request = |owner| PendingPathRequest::test_request(owner, SequenceId(1), 0);
    let mut targeting_removed = request(first);
    targeting_removed.antagonist = Some(removed);
    let requests = vec![
        request(first),
        request(removed),
        targeting_removed,
        request(last),
    ];
    engine.orders.pending_path_requests =
        PendingPathRequestQueue::restore_v48_waiting(requests.clone());
    engine.orders.failed_path_requests = requests
        .into_iter()
        .map(|request| FailedPathRequest::from_pending(request, 10))
        .collect();
    let intent = || AiOrderIntent::new(OrderType::WalkingUpright, 1.0, 2.0);
    let mut targeted_intent = intent();
    targeted_intent.target_actor = Some(removed.index());
    engine.orders.pending_move_requests = vec![
        (last, intent()),
        (first, targeted_intent),
        (removed, intent()),
        (first, intent()),
    ];

    engine.remove_entity(removed);
    engine.remove_entity(removed); // idempotent, including queue ordering
    assert!(engine.get_entity(removed).is_none());
    assert_eq!(
        u32::from(
            engine
                .get_entity(first)
                .unwrap()
                .element_data()
                .index_in_elements_list
        ),
        first.index()
    );
    for seat in &engine.players.seats {
        assert_eq!(seat.selection, [first, last]);
        assert_eq!(seat.quick_select_groups[2], [last, first]);
        assert_eq!(seat.planned_shield_target, None);
    }
    assert_eq!(engine.players.seats[0].follow_element, None);
    assert!(!engine.players.seats[0].locker_active);
    assert_eq!(engine.players.seats[1].follow_element, Some(last));
    assert!(engine.players.seats[1].locker_active);
    assert_eq!(engine.players.selection_before_user_lock, [last, first]);
    let ai = engine
        .get_entity(observer)
        .unwrap()
        .ai_controller()
        .unwrap();
    for stimuli in [&ai.stimulus_queue, &ai.outbox.detection.stimuli] {
        assert_eq!(stimuli.len(), 2);
        assert_eq!(
            stimuli[0].owner, historical.owner,
            "historical provenance is retained"
        );
        assert_eq!(stimuli[1].stimulus_type, final_stimulus.stimulus_type);
    }
    assert_eq!(
        engine
            .orders
            .pending_path_requests
            .v48_waiting()
            .iter()
            .map(|request| request.owner)
            .collect::<Vec<_>>(),
        [first, last]
    );
    assert_eq!(
        engine
            .orders
            .failed_path_requests
            .iter()
            .map(|request| request.owner)
            .collect::<Vec<_>>(),
        [first, last]
    );
    assert_eq!(
        engine
            .orders
            .pending_move_requests
            .iter()
            .map(|(owner, _)| *owner)
            .collect::<Vec<_>>(),
        [last, first]
    );
}

#[test]
fn category_rejection_preserves_callback_latch_and_reason_specific_log_order() {
    use crate::ai::{LogLineType, Remark, SpeechFlags, StimulusType};

    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    let reason_five = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        211,
    );
    let reason_nine = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Civilian { vip: false },
        212,
    );

    queue_and_settle_speech(
        &mut engine,
        &assets,
        reason_five,
        Remark::VipWarcry,
        SpeechFlags::ALWAYS | SpeechFlags::MYTALK_1,
    );
    queue_and_settle_speech(
        &mut engine,
        &assets,
        reason_nine,
        Remark::Arrow,
        SpeechFlags::ALWAYS | SpeechFlags::MYTALK_1,
    );

    let relevant = |owner| {
        speech_log(&engine, owner)
            .into_iter()
            .filter(|(kind, _)| {
                matches!(
                    kind,
                    LogLineType::Speak | LogLineType::SpeakImpossible | LogLineType::Event
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        relevant(reason_five),
        vec![
            (LogLineType::Speak, Remark::VipWarcry as u16),
            (LogLineType::SpeakImpossible, 5),
            (LogLineType::Event, StimulusType::EventMyTalk1 as u16),
        ]
    );
    assert_eq!(
        relevant(reason_nine),
        vec![
            (LogLineType::Speak, Remark::Arrow as u16),
            (LogLineType::Event, StimulusType::EventMyTalk1 as u16),
            (LogLineType::SpeakImpossible, 9),
        ]
    );
    for owner in [reason_five, reason_nine] {
        let ai = mytalk_ai(&engine, owner);
        assert_eq!(ai.current_remark, Remark::TheSoundOfSilence);
        assert_eq!(ai.current_remark_flags, 0);
    }
}

#[test]
fn forbidden_scan_is_lazy_ordered_and_equal_deadline_is_live() {
    use crate::ai::{ForbiddenRemark, Remark, RemarkTargetFlags, SpeechFlags};

    let mut engine = EngineInner::new();
    engine.control.frame_counter = 10;
    let mut assets = LevelAssets::new();
    let owner = add_speech_test_npc(
        &mut engine,
        &mut assets,
        SpeechNpcKind::Soldier { vip: false },
        601,
    );
    let owner_creation_order = engine.world.original_creation_order(owner);
    engine.ai.global.forbidden_remarks = vec![
        ForbiddenRemark {
            remark: Remark::WaspSting,
            flags: RemarkTargetFlags::VILLAINS.bits(),
            speech_id: 0,
            guy_index: 0,
            bad_guy: true,
            forbidden_till_frame: 9,
        },
        ForbiddenRemark {
            remark: Remark::Arrow,
            flags: RemarkTargetFlags::THIS_GUY.bits(),
            speech_id: 0,
            guy_index: owner_creation_order as u16,
            bad_guy: true,
            forbidden_till_frame: 10,
        },
        ForbiddenRemark {
            remark: Remark::Arrow,
            flags: RemarkTargetFlags::VILLAINS.bits(),
            speech_id: 0,
            guy_index: 0,
            bad_guy: true,
            forbidden_till_frame: 8,
        },
    ];
    queue_and_settle_speech(
        &mut engine,
        &assets,
        owner,
        Remark::Arrow,
        SpeechFlags::empty(),
    );
    assert_eq!(last_speech_impossible(&engine, owner), Some(2));
    assert_eq!(engine.ai.global.forbidden_remarks.len(), 2);
    assert_eq!(
        engine.ai.global.forbidden_remarks[0].forbidden_till_frame,
        10
    );
    assert_eq!(
        engine.ai.global.forbidden_remarks[1].forbidden_till_frame,
        8
    );

    let before = engine.ai.global.forbidden_remarks.clone();
    queue_and_settle_speech(
        &mut engine,
        &assets,
        owner,
        Remark::Arrow,
        SpeechFlags::ALWAYS,
    );
    assert_eq!(engine.ai.global.forbidden_remarks.len(), before.len() + 1);
    assert_eq!(
        serde_json::to_value(&engine.ai.global.forbidden_remarks[..before.len()]).unwrap(),
        serde_json::to_value(&before).unwrap(),
        "ALWAYS skips the lazy scan, including expired-entry deletion"
    );
}

#[test]
fn original_pc_registry_is_independent_from_portrait_priority_order() {
    let mut engine = EngineInner::new();
    let first = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let second = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));

    // Authoritative topology can differ from Rust's provisional construction
    // slots. Installing it establishes the original game's element order once.
    engine.world.install_original_creation_orders(
        std::collections::BTreeMap::from([(first, 101), (second, 100)]),
        102,
    );
    assert_eq!(engine.world.original_pc_registry_ids, vec![second, first]);

    // Portrait sorting is a separate UI concern and must not mutate the
    // engine registry used by player-character gameplay loops.
    engine.world.pc_ids = vec![first, second];
    assert_eq!(engine.world.original_pc_registry_ids, vec![second, first]);

    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    assert_eq!(
        engine.ai_pc_snapshot_ids_for_test(&assets),
        vec![second, first],
        "AI snapshots must scan Original's registry, not portrait priority"
    );

    engine.remove_entity(second);
    assert_eq!(engine.world.pc_ids, vec![first]);
    assert_eq!(engine.world.original_pc_registry_ids, vec![first]);
}

#[test]
fn far_opponent_removal_retains_owner_strength_and_runs_reciprocal_delete() {
    use crate::coordinates::{MapPoint, WorldPoint3D};

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();

    let owner = engine.add_test_entity(make_test_soldier(crate::element::Posture::Upright));
    let near = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let mut far_entity = make_test_pc(crate::element::Posture::Upright);
    far_entity
        .element_data_mut()
        .set_position(WorldPoint3D::new(1_000.0, 0.0, 0.0));
    far_entity
        .element_data_mut()
        .set_position_map(MapPoint::new(1_000.0, 0.0));
    let far = engine.add_test_entity(far_entity);
    let mut far_partner_entity = make_test_soldier(crate::element::Posture::Upright);
    far_partner_entity
        .element_data_mut()
        .set_position(WorldPoint3D::new(1_000.0, 0.0, 0.0));
    far_partner_entity
        .element_data_mut()
        .set_position_map(MapPoint::new(1_000.0, 0.0));
    let far_partner = engine.add_test_entity(far_partner_entity);
    complete_test_runtime_fixture(&mut engine, &mut assets);

    {
        let human = engine
            .get_entity_mut(owner)
            .and_then(Entity::human_data_mut)
            .unwrap();
        human.opponents = vec![near, far].into();
        human.relative_fighting_ability = 17;
    }
    engine
        .get_entity_mut(near)
        .and_then(Entity::human_data_mut)
        .unwrap()
        .opponents = vec![owner].into();
    {
        let human = engine
            .get_entity_mut(far)
            .and_then(Entity::human_data_mut)
            .unwrap();
        human.opponents = vec![owner, far_partner].into();
        human.smalltalk_initiative = false;
        human.received_smalltalk_initiative = false;
    }
    {
        let human = engine
            .get_entity_mut(far_partner)
            .and_then(Entity::human_data_mut)
            .unwrap();
        human.opponents = vec![far].into();
        human.smalltalk_initiative = true;
    }

    engine.quit_swordfight_with_far_opponents(&sim, &assets, owner);

    let owner_human = engine.get_entity(owner).unwrap().human_data().unwrap();
    assert_eq!(owner_human.opponents, vec![near]);
    assert_eq!(owner_human.relative_fighting_ability, 17);

    let far_human = engine.get_entity(far).unwrap().human_data().unwrap();
    assert_eq!(far_human.opponents, vec![far_partner]);
    assert!(far_human.smalltalk_initiative);
    assert!(far_human.received_smalltalk_initiative);
    assert!(
        !engine
            .get_entity(far_partner)
            .unwrap()
            .human_data()
            .unwrap()
            .smalltalk_initiative
    );
}

#[test]
fn direct_ai_owner_boundary_preserves_preexisting_foreign_condolation() {
    use crate::element::Command;
    use crate::sequence::SequenceElement;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let foreign_a =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let foreign_b =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let terminate = |engine: &mut EngineInner, card_owner| {
        let sequence = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::Wait, Some(card_owner)));
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .orders
            .sequence_manager
            .element_terminated(sequence, 0);
    };
    terminate(&mut engine, foreign_a);
    terminate(&mut engine, owner);
    terminate(&mut engine, foreign_b);

    // Exercise the nested global drain in `drain_pending_for_npc_mode`, not
    // merely the idle direct-boundary endpoint. The owner's Halt and its
    // pre-existing root must close now, while foreign A/B stay queued.
    let live_owner_sequence = engine
        .orders
        .sequence_manager
        .launch_element(SequenceElement::new(1, Command::Wait, Some(owner)));
    engine
        .orders
        .sequence_manager
        .element_in_progress(live_owner_sequence, 0);
    engine
        .get_entity_mut(owner)
        .and_then(Entity::ai_controller_mut)
        .expect("direct-boundary owner has AI")
        .outbox
        .actor
        .halt = true;

    let ((), stimuli) = crate::engine::soldier_helpers::capture_condolation_stimuli(|| {
        engine.drain_direct_ai_owner_boundary_without_forecast(&sim, owner, &assets);
    });

    let backlog = engine.orders.sequence_manager.drain_pending_condolations();
    assert_eq!(
        backlog
            .iter()
            .map(|dispatch| dispatch.card.owner)
            .collect::<Vec<_>>(),
        vec![foreign_a, foreign_b],
        "pre-existing foreign cards retain their FIFO around owner Halt recursion"
    );
    assert!(
        stimuli.iter().any(|(stimulus_owner, stimulus)| {
            *stimulus_owner == owner && *stimulus == crate::ai::StimulusType::EventDone
        }),
        "the owner's pre-existing terminal root must be delivered, not dropped"
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(owner, |_| true),
        "the selected owner's Halt must still close causally inside the direct boundary"
    );
}

#[test]
fn battle_observe_route_settles_before_source_ordered_tail() {
    use crate::ai::{
        AiContext, AiOwnerWork, AiState, Decision, GotoFlags, LogLineType, Position, Substate,
    };
    use crate::coordinates::MapPoint;
    use crate::gate::{Door, GateType};
    use crate::sector::SectorNumber;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.scripts.mission = Some(
        crate::engine::MissionScript::from_scb(crate::scb::ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![crate::scb::ClassEntry {
                source_file: "battle_observe_owner_boundary_test.scs".into(),
                class_name: "StartUp".into(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: Vec::new(),
                quads: Vec::new(),
            }],
        })
        .expect("minimal mission exposes the installed test jump"),
    );

    // Save052's shape: the first approach crosses sectors and cannot construct
    // a route, while the target's jump gate provides Original's avenger-on-
    // roof recovery point.
    let owner_id = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let target_id = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let owner_position = Position {
        x: 10.0,
        y: 10.0,
        sector: crate::position_interface::SectorHandle::new(1),
        level: 0,
    };
    let target_position = Position {
        x: 100.0,
        y: 200.0,
        sector: crate::position_interface::SectorHandle::new(2),
        level: 1,
    };
    for (id, position) in [(owner_id, owner_position), (target_id, target_position)] {
        let entity = engine
            .get_entity_mut(id)
            .expect("battle-observe actor exists");
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(position.x, position.y));
        entity.element_data_mut().set_sector(position.sector);
        entity.element_data_mut().set_layer(position.level);
    }
    engine
        .get_entity_mut(target_id)
        .and_then(Entity::pc_data_mut)
        .expect("battle-observe target is a PC")
        .has_jump = true;
    engine.script_domains.interactables.doors = vec![Door {
        gate_type: GateType::Jump,
        sector_out: SectorNumber::new(1),
        sector_in: SectorNumber::new(2),
        point_out: MapPoint::new(50.0, 100.0),
        point_in: MapPoint::new(50.0, 150.0),
        layer_out: 0,
        layer_in: 1,
        ..Door::default()
    }];

    let ctx = AiContext {
        position: owner_position,
        ..AiContext::test_fixture()
    };
    let ai = engine
        .get_entity_mut(owner_id)
        .and_then(Entity::enemy_ai_mut)
        .expect("battle-observe owner has Enemy AI");
    ai.base.me = owner_id.index();
    ai.base.primary_target = Some(crate::ai::AiEntityHandle::new(target_id.index()));
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.base.think_recursion_depth = 1;
    ai.base.outbox.actor.set_focus(target_id.index());
    ai.base
        .go_near(target_position, 50, GotoFlags::empty(), &ctx);
    let first_route = std::mem::take(&mut ai.base.outbox.actor);
    assert_eq!(
        first_route.focus,
        Some(crate::ai::AiEntityHandle::new(target_id.index()))
    );
    assert_eq!(first_route.orders.len(), 1);
    ai.base
        .outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ActorEffects(first_route));
    ai.base.outbox.reentrant.battle_observe_completion_pending = true;
    ai.base
        .outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ResumeBattleObserveAfterGoNear {
            target: target_id.index(),
            target_position,
        });

    engine.drain_ai_owner_work_for(&sim, &assets, owner_id);

    let ai = engine
        .get_entity(owner_id)
        .and_then(Entity::enemy_ai)
        .expect("battle-observe owner retains Enemy AI");
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRunToAvengerOnRoof
    );
    assert_eq!(ai.base.last_goto_destination.x, 50.0);
    assert_eq!(ai.base.last_goto_destination.y, 100.0);
    assert_eq!(ai.base.seek_position, target_position);
    assert!(!ai.base.couldnt_reachpoint);
    assert!(!ai.base.outbox.reentrant.battle_observe_completion_pending);
    assert_eq!(
        ai.base
            .ai_log
            .iter()
            .filter(|line| {
                line.line_type == LogLineType::BattleDecision
                    && line.info == Decision::Observe as u16
            })
            .count(),
        0,
        "Original roof fallback returns before the Observe decision log"
    );

    // A reachable route consumes the same typed tail, enters Approach, and
    // publishes the Observe decision exactly once.
    let reachable_owner =
        engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let reachable_target = engine.add_test_entity(make_test_pc(crate::element::Posture::Upright));
    let reachable_owner_position = Position {
        x: 200.0,
        y: 200.0,
        sector: crate::position_interface::SectorHandle::new(1),
        level: 0,
    };
    let reachable_target_position = Position {
        x: 300.0,
        y: 200.0,
        sector: crate::position_interface::SectorHandle::new(1),
        level: 0,
    };
    for (id, position) in [
        (reachable_owner, reachable_owner_position),
        (reachable_target, reachable_target_position),
    ] {
        let entity = engine.get_entity_mut(id).expect("reachable actor exists");
        entity.element_data_mut().active = true;
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(position.x, position.y));
        entity.element_data_mut().set_sector(position.sector);
        entity.element_data_mut().set_layer(position.level);
    }
    let reachable_ctx = AiContext {
        position: reachable_owner_position,
        ..AiContext::test_fixture()
    };
    let ai = engine
        .get_entity_mut(reachable_owner)
        .and_then(Entity::enemy_ai_mut)
        .expect("reachable owner has Enemy AI");
    ai.base.me = reachable_owner.index();
    ai.base.primary_target = Some(crate::ai::AiEntityHandle::new(reachable_target.index()));
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.base.think_recursion_depth = 1;
    ai.base.outbox.actor.set_focus(reachable_target.index());
    ai.base.go_near(
        reachable_target_position,
        5,
        GotoFlags::empty(),
        &reachable_ctx,
    );
    let reachable_route = std::mem::take(&mut ai.base.outbox.actor);
    ai.base
        .outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ActorEffects(reachable_route));
    ai.base.outbox.reentrant.battle_observe_completion_pending = true;
    ai.base
        .outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ResumeBattleObserveAfterGoNear {
            target: reachable_target.index(),
            target_position: reachable_target_position,
        });

    engine.drain_ai_owner_work_for(&sim, &assets, reachable_owner);

    let ai = engine
        .get_entity(reachable_owner)
        .and_then(Entity::enemy_ai)
        .expect("reachable owner retains Enemy AI");
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingApproachToObserve
    );
    assert_eq!(ai.base.last_goto_destination, reachable_target_position);
    assert!(!ai.base.couldnt_reachpoint);
    assert!(!ai.base.outbox.reentrant.battle_observe_completion_pending);
    assert_eq!(
        ai.base
            .ai_log
            .iter()
            .filter(|line| {
                line.line_type == LogLineType::BattleDecision
                    && line.info == Decision::Observe as u16
            })
            .count(),
        1
    );
}

#[test]
fn resumed_return_to_duty_translates_its_goto_on_the_owner_work_boundary() {
    use crate::ai::{AiOwnerWork, AiState, DutyFlags, Substate};
    use crate::coordinates::MapPoint;
    use crate::element::Command;

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(make_test_ai_soldier(crate::element::Camp::Lacklandists));
    let mut assets = LevelAssets::new();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    let sector = crate::position_interface::SectorHandle::new(1);
    let entity = engine
        .get_entity_mut(owner)
        .expect("return-to-duty owner exists");
    entity.element_data_mut().active = true;
    entity
        .element_data_mut()
        .set_position_map(MapPoint::new(100.0, 100.0));
    entity.element_data_mut().set_sector(sector);
    let ai = entity
        .enemy_ai_mut()
        .expect("return-to-duty owner has Enemy AI");
    ai.base.me = owner.index();
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingGroupGetInstructedByOfficer;
    ai.base.initial_position = crate::ai::Position {
        x: 300.0,
        y: 100.0,
        sector,
        level: 0,
    };
    ai.base
        .outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ResumeReturnToDutyAfterPatrolInit {
            flags: DutyFlags::empty(),
            defer_clear_patrol_close_post: false,
            owner_boundary_positions: vec![(
                owner.index(),
                crate::ai::Position {
                    x: 100.0,
                    y: 100.0,
                    sector,
                    level: 0,
                },
            )],
        });

    engine.drain_ai_owner_work_for(&sim, &assets, owner);

    let ai = engine
        .get_entity(owner)
        .and_then(Entity::enemy_ai)
        .expect("return-to-duty owner retains Enemy AI");
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
    assert!(
        !ai.base.outbox.actor.has_boundary_work(),
        "common return-to-duty movement must be translated before the resumed owner work returns"
    );
    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .any(|element| element.owner == Some(owner) && element.command == Command::Move),
        "the return-to-post movement must already exist in the sequence manager"
    );
}

#[test]
fn get_report_from_soldier_closes_body_deletions_at_owner_boundary() {
    use crate::ai::{AiState, Position, ReportType, Stimulus, StimulusType, Substate};
    use crate::element::{Detectable, DetectableType, Posture};

    let sim = crate::sim_rng::test_context();
    let (mut engine, officer_id, soldier_id, mut assets) = setup_review2_officer_and_soldier();
    let mut add_body = || {
        let id = engine.add_test_entity(make_test_pc(Posture::Lying));
        let Entity::Pc(body) = engine.get_entity_mut(id).expect("report body exists") else {
            panic!("report body changed kind")
        };
        body.element.active = true;
        body.pc.life_points = 0;
        id
    };
    let prefix = add_body();
    let unknown_a = add_body();
    let already_known = add_body();
    let unknown_b = add_body();
    let suffix = add_body();
    complete_test_runtime_fixture(&mut engine, &mut assets);

    {
        let officer = engine
            .get_entity_mut(officer_id)
            .and_then(Entity::enemy_ai_mut)
            .expect("report officer has EnemyAi");
        officer.set_state(
            AiState::Seeking,
            Substate::SeekingOfficerWaitForInstructedSoldier,
        );
        officer.base.antagonist = Some(crate::ai::AiEntityHandle::new(soldier_id.index()));
        officer.base.my_reconnaissance_report.report_type = ReportType::Body;
        officer.base.my_reconnaissance_report.seek_position = Position {
            x: 91.0,
            ..Default::default()
        };
        officer.base.my_reconnaissance_report.seen_bodies =
            vec![unknown_a.index(), already_known.index(), unknown_b.index()];
    }
    {
        let Entity::Soldier(soldier) = engine
            .get_entity_mut(soldier_id)
            .expect("reporting soldier exists")
        else {
            panic!("reporting entity changed kind")
        };
        let ai = soldier
            .npc
            .ai_brain
            .enemy_mut()
            .expect("reporting soldier has EnemyAi");
        ai.base.my_reconnaissance_report.report_type = ReportType::Nothing;
        ai.base.my_reconnaissance_report.seek_position = Position {
            x: 17.0,
            ..Default::default()
        };
        ai.base
            .my_reconnaissance_report
            .seen_bodies
            .push(already_known.index());
        soldier.npc.detectable_lists[DetectableType::Body as usize] =
            [prefix, unknown_a, already_known, unknown_b, suffix]
                .into_iter()
                .map(|id| Detectable {
                    element: Some(id),
                    detectable_type: DetectableType::Body,
                    ..Default::default()
                })
                .collect();
    }

    let report_before = engine
        .get_entity(soldier_id)
        .and_then(Entity::enemy_ai)
        .expect("reporting soldier retains EnemyAi")
        .base
        .my_reconnaissance_report
        .clone();
    let (ctx, tick) = review2_context_and_tick(&engine, &sim, &assets, officer_id);
    engine.dispatch_think_with_drain(
        &sim,
        officer_id,
        &Stimulus::with_human(StimulusType::CallReport, soldier_id.index()),
        &ctx,
        &tick,
        &assets,
    );

    let recipient = engine
        .get_entity(soldier_id)
        .expect("reporting soldier remains present");
    let body_handles: Vec<_> = recipient
        .npc_data()
        .expect("recipient remains NPC")
        .detectable_lists[DetectableType::Body as usize]
        .iter()
        .map(|detectable| detectable.element.expect("body detectable stays typed"))
        .collect();
    assert_eq!(body_handles, vec![prefix, already_known, suffix]);
    let report_after = &recipient
        .enemy_ai()
        .expect("recipient retains EnemyAi")
        .base
        .my_reconnaissance_report;
    assert_eq!(report_after.report_type, report_before.report_type);
    assert_eq!(report_after.seek_position, report_before.seek_position);
    assert_eq!(report_after.seen_bodies, report_before.seen_bodies);
    assert_eq!(report_after.charly, report_before.charly);
    assert_eq!(report_after.charly_seen, report_before.charly_seen);
}
