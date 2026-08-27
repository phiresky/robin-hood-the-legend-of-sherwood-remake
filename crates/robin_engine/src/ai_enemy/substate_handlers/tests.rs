use super::*;

fn soldier_view_with_substate(
    handle: u32,
    substate: Substate,
) -> crate::ai_entity_view::AiEntityView {
    let entity = crate::element::Entity::Soldier(crate::element::ActorSoldier {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorSoldier,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    });
    let mut view = crate::ai_entity_view::entity_view_from_entity(
        &entity,
        handle,
        false,
        None,
        None,
        crate::order::OrderType::NonanimationEnd,
    );
    view.ai_state = AiState::Seeking;
    view.ai_substate = substate;
    view
}

fn civilian_view(handle: u32, position: Position) -> crate::ai_entity_view::AiEntityView {
    let entity = crate::element::Entity::Civilian(crate::element::ActorCivilian {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorCivilian,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        civilian: Default::default(),
    });
    let mut view = crate::ai_entity_view::entity_view_from_entity(
        &entity,
        handle,
        false,
        None,
        None,
        crate::order::OrderType::NonanimationEnd,
    );
    view.ai_state = AiState::Default;
    view.ai_substate = Substate::DefaultOnPost;
    view.position = position;
    view.forecasted_destination =
        crate::ai::PreparedForecastDestination::fixed(position, view.direction);
    view
}

#[test]
fn drinking_ale_completes_on_event_done_without_fabricated_timer() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(90);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingDrinkingAle;
    ai.base.blood_alcohol = 17;
    let ctx = AiContext::default();
    let tick = AiPerTickData::stub();

    ai.wondering_drinking_ale(&sim, StimulusType::EventTimer, &ctx, &tick);
    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
    assert_eq!(ai.base.blood_alcohol, 17);

    ai.wondering_drinking_ale(&sim, StimulusType::EventDone, &ctx, &tick);
    assert!(matches!(
        ai.base.outbox.reentrant.owner_work.as_slice(),
        [crate::ai::AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }]
    ));
    assert_eq!(
        ai.base.blood_alcohol, 17,
        "the animation completion path owns the profile-specific beer increment"
    );
}

#[test]
fn ale_reaction_uses_latched_position_after_bottle_becomes_inactive() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(90);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingAleReactiontime;
    ai.base.interesting_object = 321;
    ai.base.seek_position = Position {
        x: 632.4453,
        y: 1835.14,
        sector: None,
        level: 0,
    };
    ai.soldier_profile_beer = 1;
    let ctx = AiContext {
        self_is_active: true,
        ..AiContext::default()
    };

    // No view for object 321: the bottle was consumed while React's timer
    // was pending. Original still commits to the retained pointer/position.
    ai.wondering_ale_reactiontime(&sim, StimulusType::EventTimer, &ctx, &AiPerTickData::stub());

    assert_eq!(ai.base.current_state, AiState::Wondering);
    assert_eq!(ai.base.current_substate, Substate::WonderingApproachingAle);
    assert_eq!(ai.base.object_of_desire, 321);
    assert_eq!(ai.base.seek_position.x, 632.4453);
    assert_eq!(ai.base.seek_position.y, 1835.14);
    assert!(
        !ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(
                work,
                crate::ai::AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }
            ))
    );
}

#[test]
fn taking_money_event_done_selects_nearest_coin_and_starts_reaction_timer() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(90);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingTakingMoney;
    ai.base.interesting_object = 130;
    ai.other_seen_money = vec![131, 132];

    let mut farther = pc_view(crate::element::Posture::Upright);
    farther.position = Position {
        x: 40.0,
        y: 10.0,
        ..Position::default()
    };
    let mut nearer = pc_view(crate::element::Posture::Upright);
    nearer.position = Position {
        x: 8.0,
        y: 12.0,
        ..Position::default()
    };
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(131, farther);
    views.insert(132, nearer);
    let ctx = AiContext {
        frame: 9_247,
        position: Position::default(),
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventDone),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::WonderingMoneyReactiontime
    );
    assert_eq!(ai.base.interesting_object, 132);
    assert_eq!(ai.other_seen_money, vec![131]);
    assert_eq!(ai.base.when_does_timer_ring, 9_248);
}

#[test]
fn taking_projectile_derived_coin_preserves_typed_interaction_target() {
    let mut ai = EnemyAi::new(90);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingApproachingMoney;
    ai.base.interesting_object = 134;

    let mut coin = pc_view(crate::element::Posture::Upright);
    coin.kind = crate::ai_entity_view::EntityKind::Projectile;
    coin.object_type = crate::element_kinds::ObjectType::Coin;
    coin.position = Position::default();
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(134, coin);
    let ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    ai.wondering_approaching_money(StimulusType::EventReachPoint, &ctx, &AiPerTickData::stub());

    let mut sequences = ai
        .base
        .outbox
        .actor
        .launch_sequences
        .iter()
        .collect::<Vec<_>>();
    for work in &ai.base.outbox.reentrant.owner_work {
        match work {
            AiOwnerWork::ActorEffects(effects) => {
                sequences.extend(effects.launch_sequences.iter());
            }
            AiOwnerWork::StateChange(change) => {
                if let Some(effects) = &change.actor_effects_before_callback {
                    sequences.extend(effects.launch_sequences.iter());
                }
            }
            _ => {}
        }
    }
    let [sequence] = sequences.as_slice() else {
        panic!("money arrival must launch exactly one Take sequence")
    };
    let Some(element) = sequence.get(0) else {
        panic!("Take sequence must contain its interaction element")
    };
    assert!(matches!(
        element.data,
        crate::sequence::SequenceElementData::Interaction {
            antagonist: Some(crate::element::EntityId::Projectile(
                crate::entity_id::ProjectileId(134)
            ))
        }
    ));
}

fn money_race_context() -> AiContext {
    let mut coin = pc_view(crate::element::Posture::Upright);
    coin.kind = crate::ai_entity_view::EntityKind::Projectile;
    coin.object_type = crate::element_kinds::ObjectType::Coin;
    coin.position = Position {
        x: 100.0,
        y: 0.0,
        ..Position::default()
    };
    let mut rival = soldier_view_with_substate(91, Substate::WonderingApproachingMoney);
    rival.position = Position {
        x: 10.0,
        y: 0.0,
        ..Position::default()
    };
    rival.detection_position = crate::coordinates::MapPoint::new(10.0, 0.0);
    rival.detection_position_world = crate::coordinates::WorldPoint3D::new(10.0, 0.0, 0.0);
    let mut viewer = soldier_view_with_substate(90, Substate::WonderingApproachingMoney);
    viewer.direction = 4;
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(90, viewer);
    views.insert(91, rival);
    views.insert(134, coin);
    AiContext {
        frame: 9_253,
        direction: 4,
        self_eye_position: crate::coordinates::MapPoint::ZERO,
        self_eye_z: 45.0,
        self_view_radius: 400,
        sq_self_view_radius: 400.0 * 400.0,
        self_view_direction: [1.0, 0.0],
        self_real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    }
}

#[test]
fn approaching_money_timer_with_visible_rival_runs_instead_of_taking() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(90);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingApproachingMoney;
    ai.base.interesting_object = 134;
    let ctx = money_race_context();
    let mut tick = AiPerTickData::stub();
    let mut rival = alert_candidate(
        91,
        Position {
            x: 10.0,
            y: 0.0,
            ..Position::default()
        },
    );
    rival.ai_state = AiState::Wondering;
    rival.ai_substate = Substate::WonderingApproachingMoney;
    tick.camp_soldiers.push(rival);

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(ai.base.current_substate, Substate::WonderingRunningForMoney);
    assert_eq!(ai.base.when_does_timer_ring, 0);
    let deferred_order_count = ai
        .base
        .outbox
        .reentrant
        .owner_work
        .iter()
        .map(|work| match work {
            AiOwnerWork::ActorEffects(effects) => effects.orders.len(),
            AiOwnerWork::StateChange(change) => change
                .actor_effects_before_callback
                .as_ref()
                .map_or(0, |effects| effects.orders.len()),
            _ => 0,
        })
        .sum::<usize>();
    assert_eq!(ai.base.outbox.actor.orders.len() + deferred_order_count, 1);
    assert!(ai.base.outbox.actor.launch_sequences.is_empty());
}

#[test]
fn approaching_money_timer_without_visible_rival_only_rearms_poll() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(90);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingApproachingMoney;
    ai.base.interesting_object = 134;
    let ctx = money_race_context();

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::WonderingApproachingMoney
    );
    assert_eq!(ai.base.when_does_timer_ring, 9_273);
    assert!(ai.base.outbox.actor.orders.is_empty());
    assert!(ai.base.outbox.actor.launch_sequences.is_empty());
}

fn brawl_approach_fixture(friend_state: AiState, friend_x: f32) -> (EnemyAi, AiContext) {
    let mut ai = EnemyAi::new(88);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingBrawlApproaching;
    ai.base.friend_in_trouble = 90;
    ai.money_fight_enemies = vec![90, 91];

    let mut owner = soldier_view_with_substate(88, Substate::WonderingBrawlApproaching);
    owner.position = Position::default();
    let mut friend = soldier_view_with_substate(90, Substate::WonderingApproachingMoney);
    friend.ai_state = friend_state;
    friend.position = Position {
        x: friend_x,
        y: 0.0,
        ..Position::default()
    };
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(88, owner);
    views.insert(90, friend);
    let ctx = AiContext {
        position: Position::default(),
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };
    (ai, ctx)
}

fn launched_hit_targets(ai: &EnemyAi) -> Vec<crate::element::EntityId> {
    let mut targets = Vec::new();
    let mut inspect_effects = |effects: &crate::ai::AiActorOutbox| {
        for sequence in &effects.launch_sequences {
            for element in &sequence.elements {
                if element.command == crate::element::Command::HitCmd
                    && let crate::sequence::SequenceElementData::Interaction {
                        antagonist: Some(target),
                    } = element.data
                {
                    targets.push(target);
                }
            }
        }
    };
    inspect_effects(&ai.base.outbox.actor);
    for work in &ai.base.outbox.reentrant.owner_work {
        match work {
            AiOwnerWork::ActorEffects(effects) => inspect_effects(effects),
            AiOwnerWork::StateChange(change) => {
                if let Some(effects) = &change.actor_effects_before_callback {
                    inspect_effects(effects);
                }
            }
            _ => {}
        }
    }
    targets
}

#[test]
fn brawl_reach_near_awake_friend_stops_and_launches_hit() {
    let (mut ai, ctx) = brawl_approach_fixture(AiState::Wondering, 20.0);
    ai.wondering_brawl_approaching(
        &crate::sim_rng::test_context(),
        StimulusType::EventReachPoint,
        &ctx,
        &AiPerTickData::stub(),
    );

    assert_eq!(ai.base.current_substate, Substate::WonderingBrawlHitting);
    assert_eq!(
        launched_hit_targets(&ai),
        vec![crate::element::EntityId::Soldier(
            crate::entity_id::SoldierId(90)
        )]
    );
}

#[test]
fn brawl_reach_far_awake_friend_retries_approach_without_hit() {
    let (mut ai, ctx) = brawl_approach_fixture(AiState::Wondering, 40.0);
    ai.wondering_brawl_approaching(
        &crate::sim_rng::test_context(),
        StimulusType::EventReachPoint,
        &ctx,
        &AiPerTickData::stub(),
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::WonderingBrawlApproaching
    );
    assert!(launched_hit_targets(&ai).is_empty());
    assert_eq!(ai.base.last_goto_destination.x, 40.0);
}

#[test]
fn brawl_reach_sleeping_friend_removes_target_and_queues_done() {
    let (mut ai, ctx) = brawl_approach_fixture(AiState::Sleeping, 20.0);
    ai.wondering_brawl_approaching(
        &crate::sim_rng::test_context(),
        StimulusType::EventReachPoint,
        &ctx,
        &AiPerTickData::stub(),
    );

    assert_eq!(ai.base.current_substate, Substate::WonderingBrawlHitting);
    assert_eq!(ai.base.friend_in_trouble, 0);
    assert_eq!(ai.money_fight_enemies, vec![91]);
    assert_eq!(ai.base.outbox.reentrant.self_stimuli.len(), 1);
    assert_eq!(
        ai.base.outbox.reentrant.self_stimuli[0].stimulus_type,
        StimulusType::EventDone
    );
    assert!(launched_hit_targets(&ai).is_empty());
}

#[test]
fn brawl_reach_missing_friend_returns_to_duty_without_hit() {
    let (mut ai, ctx) = brawl_approach_fixture(AiState::Wondering, 20.0);
    ai.base.friend_in_trouble = 0;
    ai.wondering_brawl_approaching(
        &crate::sim_rng::test_context(),
        StimulusType::EventReachPoint,
        &ctx,
        &AiPerTickData::stub(),
    );

    assert_ne!(ai.base.current_substate, Substate::WonderingBrawlHitting);
    assert!(launched_hit_targets(&ai).is_empty());
}

#[test]
fn brawl_hitting_done_enqueues_only_180_degree_panic_sweep() {
    let mut ai = EnemyAi::new(88);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingBrawlHitting;
    ai.wondering_brawl_hitting(
        StimulusType::EventDone,
        &AiContext::default(),
        &AiPerTickData::stub(),
    );

    assert!(
        ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(work, AiOwnerWork::NearbyCiviliansPanic180))
    );
    assert!(
        !ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(work, AiOwnerWork::NearbyCiviliansPanic))
    );
}

#[test]
fn brawl_hitting_stages_panic_then_officer_then_tail() {
    let mut ai = EnemyAi::new(88);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingBrawlHitting;
    ai.base.think_recursion_depth = 1;
    ai.wondering_brawl_hitting(
        StimulusType::EventDone,
        &AiContext::default(),
        &AiPerTickData::stub(),
    );
    assert!(matches!(
        ai.base.outbox.reentrant.owner_work.as_slice(),
        [AiOwnerWork::NearbyCiviliansPanic180]
    ));
    ai.end_think(
        &crate::sim_rng::test_context(),
        &mut AiGlobalState::default(),
        &AiContext::default(),
        &AiPerTickData::stub(),
        None,
    );
    assert_eq!(ai.base.think_recursion_depth, 1);
    assert_eq!(ai.base.engine_deferred_end_think_frames, 1);
    assert!(ai.base.outbox.reentrant.brawl_hitting_completion_pending);

    // Simulate the engine consuming the civilian-sweep owner work. The
    // next stage may publish the officer call, but no brawler StateChange
    // tail is allowed to exist until that cross-NPC call has settled.
    ai.base.outbox.reentrant.owner_work.clear();
    let mut officer = alert_candidate(89, Position::default());
    officer.rank = ProfileRank::Officer;
    officer.ai_state = AiState::Default;
    officer.ai_substate = Substate::DefaultOnPost;
    let mut tick = AiPerTickData::stub();
    tick.camp_soldiers.push(officer);
    ai.brawl_hitting_notify_officer(&AiContext::default(), &tick);
    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
    assert!(matches!(
        ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
        [CrossNpcAction::SendStimulus {
            target: 89,
            stimulus_type: StimulusType::EventSeesBrawl,
            ..
        }]
    ));

    ai.base.outbox.reentrant.cross_npc_actions.clear();
    ai.resume_brawl_hitting_after_officer(&AiContext::default(), &AiPerTickData::stub());
    assert!(
        ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(work, AiOwnerWork::StateChange(_)))
    );
    ai.base.outbox.reentrant.brawl_hitting_completion_pending = false;
    ai.base.resolve_engine_completion_verdict();
    ai.base.close_engine_deferred_end_think_frames();
    assert_eq!(ai.base.think_recursion_depth, 0);
    assert_eq!(ai.base.engine_deferred_end_think_frames, 0);
}

#[test]
fn returning_soldier_with_far_civilian_antagonist_keeps_route_and_rearms_timer() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(195);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingSoldierReturnToOfficer;
    ai.base.antagonist = 85;
    ai.officers_position = Position {
        x: 1_503.6351,
        y: 1_097.0138,
        ..Position::default()
    };

    let mut antagonist = civilian_view(
        85,
        Position {
            x: 2_100.0,
            y: 1_800.0,
            ..Position::default()
        },
    );
    antagonist.ai_state = AiState::Fleeing;
    antagonist.ai_substate = Substate::FleeingHiding;
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(85, antagonist);
    let ctx = AiContext {
        frame: 14_748,
        position: Position {
            x: 2_006.4349,
            y: 1_735.3752,
            ..Position::default()
        },
        sq_standard_view_radius: 300.0 * 300.0,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    ai.seeking_soldier_return_to_officer(
        &sim,
        StimulusType::EventTimer,
        &ctx,
        &AiPerTickData::stub(),
    );

    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingSoldierReturnToOfficer
    );
    assert!(ai.base.timer_is_running);
    assert_eq!(ai.base.when_does_timer_ring, 14_768);
    assert!(ai.base.outbox.actor.orders.is_empty());
    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
}

#[test]
fn civilian_report_alert_officer_route_failure_seeks_retained_report_position() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(243);
    ai.base.couldnt_reachpoint = true;
    let report_position = Position {
        x: 1440.3243,
        y: 1604.3259,
        level: 0,
        sector: crate::position_interface::SectorHandle::new(18),
        ..Position::default()
    };
    let ctx = AiContext {
        frame: 44_683,
        position: Position {
            x: 1645.9982,
            y: 1818.9901,
            level: 0,
            sector: crate::position_interface::SectorHandle::new(18),
            ..Position::default()
        },
        ..AiContext::default()
    };

    ai.resume_civilian_report_after_alert_officer(
        &sim,
        report_position,
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
    );

    assert!(!ai.base.couldnt_reachpoint);
    assert_eq!(ai.seek_center, report_position);
    assert_ne!(ai.seek_center, ctx.position);
    assert!(ai.seek_flags.contains(SeekFlags::LOCATION_FIRST));
}

#[test]
fn send_charly_speech_completion_faces_live_friend_before_waiting() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(93);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingSendCharlyToOfficer;
    ai.base.friend_in_trouble = 94;

    let mut friend = soldier_view_with_substate(94, Substate::SeekingCharlySentToOfficer);
    friend.position = Position {
        x: 200.0,
        y: 100.0,
        ..Position::default()
    };
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(94, friend);
    let ctx = AiContext {
        frame: 12_391,
        position: Position {
            x: 100.0,
            y: 100.0,
            ..Position::default()
        },
        direction: 12,
        self_action_state: crate::element::ActionState::Waiting,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventMyTalk2),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingLookingResurrectedCharly
    );
    assert!(ai.base.timer_is_running);
    assert_eq!(ai.base.when_does_timer_ring, 12_491);
    let [turn] = ai.base.outbox.actor.orders.as_slice() else {
        panic!("live Charly must receive the authored Face turn");
    };
    assert_eq!(turn.order_type, crate::order::OrderType::Turning);
    assert_eq!(turn.explicit_direction, Some(4));
}

#[test]
fn send_charly_speech_completion_without_live_friend_still_waits() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(93);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingSendCharlyToOfficer;
    ai.base.friend_in_trouble = 94;
    let ctx = AiContext {
        frame: 12_391,
        direction: 12,
        self_action_state: crate::element::ActionState::Waiting,
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventMyTalk2),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingLookingResurrectedCharly
    );
    assert!(ai.base.timer_is_running);
    assert_eq!(ai.base.when_does_timer_ring, 12_491);
    assert!(ai.base.outbox.actor.orders.is_empty());
}

#[test]
fn seeking_got_stop_timer_wonders_without_alert_path() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(59);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingGotStopEvent;
    ai.base.current_music_alert_status = AlertLevel::Yellow;
    ai.base.view_alert_status = AlertLevel::Yellow;
    let ctx = AiContext {
        frame: 23_905,
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Wondering);
    assert_eq!(ai.base.current_substate, Substate::WonderingLooking1);
    assert_eq!(ai.base.current_music_alert_status, AlertLevel::Yellow);
    assert_eq!(ai.base.view_alert_status, AlertLevel::Yellow);
    assert_eq!(ai.base.current_emoticon_type, EmoticonType::QuestionMark);
    assert!(ai.base.timer_is_running);
    assert_eq!(ai.base.when_does_timer_ring, 23_935);
    assert!(!ai.changed_to_alert_path);
    assert!(ai.base.patrol_path.is_none());
    assert!(
        ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .all(|work| { !matches!(work, AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }) })
    );
}

#[test]
fn seeking_got_stop_timer_adopts_alert_path_before_wondering() {
    use crate::ai::{PathId, PatrolPath};
    use crate::level_data::RawHikingPath;

    let sim = crate::sim_rng::test_context();
    let paths = vec![
        RawHikingPath { waypoints: vec![] },
        RawHikingPath { waypoints: vec![] },
    ];
    let ordinary_path = PathId::new(0).unwrap();
    let alert_path = PathId::new(1).unwrap();
    let mut ai = EnemyAi::new(59);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingGotStopEvent;
    ai.base.alert_path_id = Some(alert_path);
    ai.base.has_patrol_path = true;
    ai.base.patrol_path = PatrolPath::new(ordinary_path, &paths);
    let ctx = AiContext {
        frame: 900,
        hiking_paths: std::sync::Arc::new(paths),
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert!(ai.changed_to_alert_path);
    let adopted = ai
        .base
        .patrol_path
        .as_ref()
        .expect("configured alert path must be installed");
    assert_eq!(adopted.hiking_path_index, alert_path);
    assert_eq!(adopted.current_waypoint_index, 0);
    assert!(adopted.forward);
    assert!(ai.base.has_patrol_path);
    assert_eq!(ai.base.current_state, AiState::Wondering);
    assert_eq!(ai.base.current_substate, Substate::WonderingLooking1);
    assert_eq!(ai.base.when_does_timer_ring, 930);
}

#[test]
fn officer_wait_for_instructed_group_keeps_full_original_seek_area_set() {
    let sim = crate::sim_rng::test_context();
    for member_substate in [
        Substate::SeekingSeekpoint,
        Substate::SeekingSeekpointWatching,
        Substate::SeekingSeekpointWatchingSidewards,
        Substate::SeekingSeekpointPassedAmbushPointLeft,
        Substate::SeekingSeekpointPassedAmbushPointRight,
        Substate::SeekingSeekpointCheckingAmbushPoint,
        Substate::SeekingSeekpointApproachingBeggar,
        Substate::SeekingSeekpointIdentifyingBeggar1,
        Substate::SeekingSeekpointIdentifyingBeggar2,
        Substate::SeekingNet,
    ] {
        let mut ai = EnemyAi::new(147);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingOfficerWaitForInstructedGroup;
        ai.alerted_us = vec![148];

        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(148, soldier_view_with_substate(148, member_substate));
        let ctx = AiContext {
            frame: 7_915,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.alerted_us, vec![148], "{member_substate:?}");
        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingOfficerWaitForInstructedGroup,
            "{member_substate:?}"
        );
        assert!(ai.base.timer_is_running, "{member_substate:?}");
        assert_eq!(ai.base.when_does_timer_ring, 7_945, "{member_substate:?}");
        assert!(
            ai.base.outbox.reentrant.owner_work.is_empty(),
            "{member_substate:?}"
        );
    }
}

#[test]
fn officer_group_instruction_retries_location_first_after_refusal() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(147);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingOfficerInstructGroupPointing;
    ai.base.seek_position = Position {
        x: 500.0,
        ..Position::default()
    };
    ai.alerted_us = vec![148, 149, 150];
    let ctx = AiContext {
        frame: 733,
        position: Position::default(),
        ..AiContext::default()
    };
    let tick = AiPerTickData::stub();
    let mut global = AiGlobalState::default();

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventDone),
        &mut global,
        &ctx,
        &tick,
        None,
    );

    let take_instruction = |ai: &mut EnemyAi| {
        let action = ai.base.outbox.reentrant.cross_npc_actions.remove(0);
        let CrossNpcAction::RequestThinkResult {
            target,
            info: StimulusInfo::Hint(hint),
            continuation,
            ..
        } = action
        else {
            panic!("group instruction must queue one result-bearing CallInstruction");
        };
        assert!(
            ai.base.outbox.reentrant.cross_npc_actions.is_empty(),
            "Original calls the next member only after this Think result"
        );
        (target, hint.seek_flags, continuation)
    };

    let (first, first_flags, first_continuation) = take_instruction(&mut ai);
    assert_eq!(first, 148);
    assert!(SeekFlags::from_bits_retain(first_flags).contains(SeekFlags::LOCATION_FIRST));
    ai.resolve_think_result(
        &sim,
        false,
        first,
        first_continuation,
        &mut global,
        None,
        &ctx,
        &tick,
    );

    let (second, second_flags, second_continuation) = take_instruction(&mut ai);
    assert_eq!(second, 149);
    assert!(
        SeekFlags::from_bits_retain(second_flags).contains(SeekFlags::LOCATION_FIRST),
        "a refused index-zero member must not consume LOCATION_FIRST"
    );
    ai.resolve_think_result(
        &sim,
        true,
        second,
        second_continuation,
        &mut global,
        None,
        &ctx,
        &tick,
    );

    let (third, third_flags, third_continuation) = take_instruction(&mut ai);
    assert_eq!(third, 150);
    assert!(
        !SeekFlags::from_bits_retain(third_flags).contains(SeekFlags::LOCATION_FIRST),
        "everyone after the first accepted member searches only the area"
    );
    ai.resolve_think_result(
        &sim,
        true,
        third,
        third_continuation,
        &mut global,
        None,
        &ctx,
        &tick,
    );

    assert_eq!(ai.alerted_us, vec![149, 150]);
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingOfficerWaitForInstructedGroup
    );
    assert_eq!(ai.base.when_does_timer_ring, 763);
    assert!(ai.pending_group_instruction_candidates.is_empty());
    assert_eq!(ai.pending_group_instruction_seek_flags, 0);
}

#[test]
fn avenger_roof_timeout_seeks_from_live_owner_position() {
    // Original calls SeekArea(Position(mpMe), ...), not with the cached
    // position used to reach the avenger. Keep those positions far apart
    // so using the stale center cannot accidentally select these points.
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(149);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingWaitForAvengerOnRoof;
    ai.base.primary_target = 0;
    ai.base.seek_position = Position {
        x: 241.0,
        y: 862.0,
        ..Position::default()
    };

    let live_position = Position {
        x: 1_800.0,
        y: 2_200.0,
        ..Position::default()
    };
    let mut global = AiGlobalState::default();
    global.seek_points = [(1_810.0, 2_200.0), (1_820.0, 2_200.0), (1_830.0, 2_200.0)]
        .into_iter()
        .enumerate()
        .map(|(id, (x, y))| SeekPoint {
            position: Position {
                x,
                y,
                ..Position::default()
            },
            frame_when_full_interest: 0,
            directions: vec![0],
            last_calculated_interest: 100,
            locked: false,
            id: id as u16,
        })
        .collect();
    let ctx = AiContext {
        frame: 12_000,
        position: live_position,
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut global,
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.seek_center, live_position);
    assert_ne!(ai.seek_center, ai.base.seek_position);
    assert!(
        ai.my_seek_points.iter().any(|&id| id < 3),
        "live-center seek must select one of the nearby authored points"
    );
}

#[test]
fn avenger_roof_timeout_refaces_detected_target_and_rearms_thirty_ticks() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(236);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingWaitForAvengerOnRoof;
    ai.base.primary_target = 295;

    // Keep the target visibly in front of the actor at direction 6 so
    // detection succeeds.  The inline `Face(element)` overload would
    // also measure this map-space vector, see direction 6 already held,
    // and synchronously short-circuit without authoring a Turn.
    let [target_dx, target_dy] = crate::position_interface::sector_to_vector_iso(6);
    let target_position = Position {
        x: target_dx * 10.0,
        y: target_dy * 10.0,
        ..Position::default()
    };
    // Original calls `Face(Position(mpPrimaryTarget))`.  That overload
    // subtracts the actor's raw body point, which is deliberately offset
    // here so the resulting vector is direction 1.  This distinguishes
    // it from `Face(element)` even though both inspect the same target.
    let [face_dx, face_dy] = crate::position_interface::sector_to_vector_iso(1);
    let body_position = crate::coordinates::WorldPoint3D::new(
        target_position.x - face_dx * 10.0,
        target_position.y - face_dy * 10.0,
        0.0,
    );
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(
        236,
        soldier_view_with_substate(236, Substate::AttackingWaitForAvengerOnRoof),
    );
    views.insert(295, civilian_view(295, target_position));
    let ctx = AiContext {
        frame: 30_055,
        direction: 6,
        self_action_state: crate::element::ActionState::Waiting,
        self_body_position_world: body_position,
        self_view_radius: 400,
        sq_self_view_radius: 400.0 * 400.0,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    let [turn] = ai.base.outbox.actor.orders.as_slice() else {
        panic!("detected avenger must receive exactly one authored Face turn");
    };
    assert_eq!(turn.order_type, crate::order::OrderType::Turning);
    assert_eq!(turn.explicit_direction, Some(1));
    assert!(ai.base.timer_is_running);
    assert_eq!(ai.base.when_does_timer_ring, 30_085);
    assert_eq!(
        ai.base.substate_at_last_timer_launch,
        Substate::AttackingWaitForAvengerOnRoof
    );
}

#[test]
fn officer_wait_for_instructed_group_prunes_taking_net() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(147);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingOfficerWaitForInstructedGroup;
    ai.alerted_us = vec![148];

    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(
        148,
        soldier_view_with_substate(148, Substate::SeekingTakingNet),
    );
    let ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert!(ai.alerted_us.is_empty());
    assert!(matches!(
        ai.base.outbox.reentrant.owner_work.as_slice(),
        [AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }]
    ));
}

#[test]
fn officer_wait_for_instructed_group_waits_for_approaching_charly() {
    let sim = crate::sim_rng::test_context();
    for (charly_substate, should_wait) in [
        (Substate::SeekingCharlySentToOfficer, true),
        (Substate::SeekingCharlyGoToOfficer, true),
        (Substate::DefaultOnPost, false),
    ] {
        let mut ai = EnemyAi::new(147);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingOfficerWaitForInstructedGroup;
        ai.base.my_reconnaissance_report.report_type = ReportType::MissedCharly;
        ai.base.my_reconnaissance_report.charly = 148;

        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(148, soldier_view_with_substate(148, charly_substate));
        let ctx = AiContext {
            frame: 7_915,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        if should_wait {
            assert!(ai.base.timer_is_running);
            assert_eq!(ai.base.when_does_timer_ring, 7_945);
            assert!(ai.base.outbox.reentrant.owner_work.is_empty());
        } else {
            assert!(matches!(
                ai.base.outbox.reentrant.owner_work.as_slice(),
                [AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }]
            ));
        }
    }
}

#[test]
fn archer_waiting_on_shooting_path_returns_to_duty_only_on_timer() {
    let sim = crate::sim_rng::test_context();
    for substate in [
        Substate::AttackingArcherWaitOnArcheryPath,
        Substate::AttackingArcherWaitOnArcheryPathBending,
    ] {
        let mut ai = EnemyAi::new(84);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = substate;

        ai.think_expected_attacking_event(
            &sim,
            &Stimulus::new(StimulusType::EventDone),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Attacking);
        assert_eq!(ai.base.current_substate, substate);
        assert!(ai.base.outbox.reentrant.owner_work.is_empty());

        ai.think_expected_attacking_event(
            &sim,
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );

        assert!(matches!(
            ai.base.outbox.reentrant.owner_work.as_slice(),
            [AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }]
        ));
    }
}

#[test]
fn fleeing_hiding_timer_invokes_enemy_return_to_duty() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(158);
    ai.base.current_state = AiState::Fleeing;
    ai.base.current_substate = Substate::FleeingHiding;

    let handled = ai.think_expected_fleeing_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &AiContext::default(),
        &AiPerTickData::stub(),
        None,
    );

    assert!(handled);
    assert!(matches!(
        ai.base.outbox.reentrant.owner_work.as_slice(),
        [AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }]
    ));
}

#[test]
fn approaching_new_enemy_close_gate_stretches_world_y() {
    // Task 274 frontier: the raw map-space delta fits inside the authored
    // 65+10 range, but Original SquareDistance stretches Y by the inverse
    // aspect ratio and therefore takes the re-approach arm.
    let owner = Position {
        x: 726.946_96,
        y: 2116.774,
        ..Position::default()
    };
    let target = Position {
        x: 710.678_9,
        y: 2049.651_1,
        ..Position::default()
    };
    let dx = target.x - owner.x;
    let dy = target.y - owner.y;
    assert!(dx * dx + dy * dy < 75_u32.pow(2) as f32);
    assert!(!approaching_new_enemy_is_close_enough(
        &target, 0.0, &owner, 0.0, 65,
    ));
}

#[test]
fn approaching_new_enemy_close_gate_accepts_flat_nearby_target() {
    let owner = Position {
        x: 100.0,
        y: 100.0,
        ..Position::default()
    };
    let target = Position {
        x: 110.0,
        y: 110.0,
        ..Position::default()
    };

    assert!(approaching_new_enemy_is_close_enough(
        &target, 0.0, &owner, 0.0, 65,
    ));
}

#[test]
fn approaching_new_enemy_close_gate_uses_literal_positions() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(58);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingApproachingNewEnemy;
    ai.base.primary_target = 103;

    let mut tick = AiPerTickData::stub();
    tick.fighter_registry
        .push(crate::ai_enemy::FighterSnapshot {
            handle: 58,
            sword_range_default: 65,
            ..crate::ai_enemy::FighterSnapshot::default()
        });
    tick.fighter_registry
        .push(crate::ai_enemy::FighterSnapshot {
            handle: 103,
            // `Position(target)` is a planning position and remains the
            // destination for the GoNear arm, but it is not GetPosition().
            position: Position {
                x: 500.0,
                y: 0.0,
                ..Position::default()
            },
            elevation: 0.0,
            ..crate::ai_enemy::FighterSnapshot::default()
        });
    tick.owner_live_position = Some(Position {
        x: 100.0,
        y: 100.0,
        ..Position::default()
    });
    tick.primary_target_snapshot_handle = 103;
    tick.primary_target_live_position = Some(Position {
        x: 110.0,
        y: 110.0,
        ..Position::default()
    });
    let ctx = AiContext {
        // The owner's planning position is also deliberately far from
        // the target's planning position.
        position: Position::default(),
        elevation: 0.0,
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
    assert_eq!(ai.base.outbox.actor.set_principal, Some(103));
}

#[test]
#[should_panic(expected = "primary target 103 is missing its required fighter snapshot")]
fn approaching_new_enemy_requires_primary_target_snapshot() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(58);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingApproachingNewEnemy;
    ai.base.primary_target = 103;

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
        &AiContext::default(),
        &AiPerTickData::stub(),
        None,
    );
}

#[test]
#[should_panic(expected = "primary target 103 does not match tick snapshot handle 102")]
fn approaching_new_enemy_rejects_stale_primary_target_geometry() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(58);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingApproachingNewEnemy;
    ai.base.primary_target = 103;

    let mut tick = AiPerTickData::stub();
    tick.fighter_registry
        .push(crate::ai_enemy::FighterSnapshot {
            handle: 103,
            ..crate::ai_enemy::FighterSnapshot::default()
        });
    tick.primary_target_snapshot_handle = 102;

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
        &AiContext::default(),
        &tick,
        None,
    );
}

#[test]
fn bow_running_behind_shield_faces_target_with_its_elevation() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(74);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingBowRunningBehindShieldBearer;
    ai.base.primary_target = 170;

    let mut target = pc_view(crate::element::Posture::Upright);
    target.position = Position {
        x: 265.357_67,
        y: 1023.334_5,
        ..Position::default()
    };
    target.elevation = 151.123_84;
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(170, target);
    let ctx = AiContext {
        position: Position {
            x: 436.932_5,
            y: 1227.554,
            ..Position::default()
        },
        elevation: 45.0,
        direction: 3,
        self_action_state: crate::element::ActionState::Moving,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    // A flat map-space face selects sector 15 at this boundary. The
    // Original element overload adds `(SWORD)151.12384 - 45 == 106`
    // to dy and therefore authors sector 14.
    assert_eq!(
        crate::position_interface::vector_to_sector_0_to_15_iso(
            265.357_67 - 436.932_5,
            1023.334_5 - 1227.554
        ),
        15
    );

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    let [turn] = ai.base.outbox.actor.orders.as_slice() else {
        panic!("shield-bearer arrival must author exactly one turn");
    };
    assert_eq!(turn.order_type, crate::order::OrderType::Turning);
    assert_eq!(turn.explicit_direction, Some(14));
}

#[test]
fn shield_reestablish_uses_raw_door_passing_target_position() {
    // Schema-15 SuN1Sh1nE Savegame_013 replay-006 frame 2027. PC 174's
    // AI position is projected onto its active door endpoint, while
    // RHElement::GetPosition still reports its raw actor position.
    // RHFIELD_SHIELD_DANGER_POINT uses the latter.
    use crate::sequence::{Field, FieldValue};

    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(82);
    ai.base.owner_entity_id = Some(crate::element::EntityId::Soldier(
        crate::element::SoldierId(82),
    ));
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingProtectingWithShield;
    ai.base.primary_target = 174;

    let mut tick = AiPerTickData::stub();
    tick.fighter_registry
        .push(crate::ai_enemy::FighterSnapshot {
            handle: 82,
            action_state: crate::element::ActionState::Waiting,
            ..crate::ai_enemy::FighterSnapshot::default()
        });
    tick.fighter_registry
        .push(crate::ai_enemy::FighterSnapshot {
            handle: 174,
            position: Position {
                x: 598.0,
                y: 2490.781_3,
                ..Position::default()
            },
            raw_position: Position {
                x: 591.982_4,
                y: 2475.868_2,
                ..Position::default()
            },
            elevation: 0.0,
            ..crate::ai_enemy::FighterSnapshot::default()
        });
    let ctx = AiContext {
        frame: 2027,
        position: Position {
            x: 725.584_17,
            y: 2499.990_2,
            ..Position::default()
        },
        ..AiContext::default()
    };

    ai.attacking_protecting_with_shield(&sim, StimulusType::EventTimer, &ctx, &tick);

    let element = ai.base.outbox.actor.launch_sequences[0]
        .elements
        .first()
        .expect("RaiseShield sequence contains its command");
    assert!(matches!(
        element.get_property(Field::ShieldDangerPoint),
        Some(FieldValue::Point3D {
            x: 591.982_4,
            y: 2475.868_2,
            z: 0.0,
        })
    ));
}

#[test]
fn door_fight_wait_timer_starts_battle_overview() {
    // Savegame_033 replay-024 frame 24033: entering BattleDecisions
    // directly selected SeekArea and consumed an RNG draw that Original
    // did not make. RHArtificialMalignity handles this timer by calling
    // GetBattleOverview and beginning the observation sequence instead.
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(95);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingDoorFightWaiting;
    ai.list_them.push(170);

    ai.attacking_door_fight_waiting(
        &sim,
        StimulusType::EventTimer,
        &mut AiGlobalState::default(),
        &AiContext::default(),
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingOverviewLookLeft
    );
    assert!(ai.list_them.is_empty());
    assert_eq!(
        ai.base.outbox.actor.look_sidewards,
        Some(crate::ai::LookDirection::Left)
    );
}

#[test]
fn phalanx_shield_reestablish_uses_raw_door_passing_target_position() {
    // Schema-14 seed 1000000, SuN1Sh1nE Savegame_024 replay-004
    // frame 1254. Soldier 46 is crossing a door: its AI Position() is
    // gate point (1158, 1627), while RHElement::GetPosition() still uses
    // the live actor point. Those points face different sectors from
    // Soldier 95, so the shield command must retain the raw one.
    use crate::sequence::{Field, FieldValue};

    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(95);
    ai.base.owner_entity_id = Some(crate::element::EntityId::Soldier(
        crate::element::SoldierId(95),
    ));
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingPhalanx;
    ai.base.primary_target = 46;

    let raw_target = Position {
        x: 1_137.708_7,
        y: 1_652.301,
        ..Position::default()
    };
    let target_elevation = 150.001_f32;
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry
        .push(crate::ai_enemy::FighterSnapshot {
            handle: 95,
            action_state: crate::element::ActionState::Waiting,
            ..crate::ai_enemy::FighterSnapshot::default()
        });
    tick.fighter_registry
        .push(crate::ai_enemy::FighterSnapshot {
            handle: 46,
            position: Position {
                x: 1_158.0,
                y: 1_627.0,
                ..Position::default()
            },
            raw_position: raw_target,
            elevation: target_elevation,
            ..crate::ai_enemy::FighterSnapshot::default()
        });

    ai.attacking_phalanx(
        &sim,
        StimulusType::EventTimer,
        &mut AiGlobalState::default(),
        &AiContext::default(),
        &tick,
        None,
    );

    let element = ai.base.outbox.actor.launch_sequences[0]
        .elements
        .first()
        .expect("RaiseShield sequence contains its command");
    assert!(matches!(
        element.get_property(Field::ShieldDangerPoint),
        Some(FieldValue::Point3D { x, y, z })
            if *x == raw_target.x
                && *y == raw_target.y + target_elevation
                && *z == target_elevation
    ));
}

#[test]
fn officer_wait_missed_soldier_does_not_relaunch_timer() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingOfficerWaitForInstructedSoldier;
    ai.base.antagonist = 2;
    ai.missed_soldier_timer = 11;

    let mut tick = AiPerTickData::stub();
    tick.camp_soldiers.push(crate::ai_enemy::CampSoldierInfo {
        handle: 2,
        active: true,
        position: Position::default(),
        position_world: crate::coordinates::WorldPoint3D::ZERO,
        direction: 0,
        rank: ProfileRank::Soldier,
        ai_state: AiState::Seeking,
        ai_substate: Substate::SeekingSeekpoint,
        is_able_to_fight: true,
        is_dead: false,
        knocked_out_in_money_fight: false,
        primary_target: 0,
        pride: 0,
        is_able_to_help: true,
        script_locked: false,
        ai_lock_frozen: false,
        layer: 0,
        report_type: ReportType::Nothing,
        report_seek_position: Position::default(),
        report_seen_bodies: Vec::new(),
        report_charly: 0,
        alert_soldiers_point: Position::default(),
        patrol_chief: None,
        antagonist: 1,
        detected_body: 0,
        blood_alcohol: 0,
        duty_flag: false,
        is_tower_guard: false,
        company_number: 0,
        in_building: false,
        forecast_destination: None,
        detectable_bodies: Vec::new(),
        seek_position: Position::default(),
        current_task_priority: 0,
        minimal_task_priority: 0,
        view_direction: [1.0, 0.0],
        view_radius: 300,
        real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        eye_blind: false,
    });

    let mut self_view = pc_view(crate::element::Posture::Upright);
    self_view.is_able_to_fight = true;
    self_view.active = true;
    let mut missed_view = pc_view(crate::element::Posture::Upright);
    missed_view.is_able_to_fight = false;
    missed_view.is_unconscious = true;
    missed_view.active = true;
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(1, self_view);
    views.insert(2, missed_view);
    let ctx = AiContext {
        frame: 34_522,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(ai.missed_soldier_timer, 12);
    assert!(!ai.base.timer_is_running);
    assert_eq!(ai.base.when_does_timer_ring, 0);
}

#[test]
fn goto_post_arrival_runs_enemy_attentive_tail_after_turn_request() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(57);
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultGotoPost;
    ai.base.initial_view_direction = 4;
    ai.attentive = true;
    ai.will_be_attentive = true;
    let ctx = AiContext {
        direction: 0,
        self_action_state: crate::element::ActionState::Waiting,
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.current_substate, Substate::DefaultGotoPostTurn);
    let state_change = ai
        .base
        .outbox
        .reentrant
        .owner_work
        .iter()
        .find_map(|work| match work {
            crate::ai::AiOwnerWork::StateChange(change) => Some(change),
            _ => None,
        })
        .expect("goto-post arrival must use EnemyAi::set_state");
    let turn_prefix = state_change
        .actor_effects_before_callback
        .as_ref()
        .expect("FaceTo must precede the virtual SetState call");
    assert_eq!(turn_prefix.orders.len(), 1);
    assert_eq!(
        turn_prefix.orders[0].order_type,
        crate::order::OrderType::Turning
    );
    let attentive = ai
        .base
        .outbox
        .actor
        .set_attentive_mode
        .expect("Default SetState must queue its attentive-mode tail");
    assert!(!attentive.target);
    assert!(!attentive.fast_officer_variant);
    assert_eq!(ai.base.view_alert_status, crate::ai::AlertLevel::Green);
}

#[test]
fn reached_beggar_launches_one_ordered_turn_then_response_sequence() {
    use crate::element::{Command, EntityId, Posture};
    use crate::entity_id::SoldierId;
    use crate::sequence::{Field, FieldValue};

    for (archer, response, timer) in [
        (false, Command::StartMenace, 30),
        (true, Command::EquipBow, 100),
    ] {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(1);
        ai.base.owner_entity_id = Some(EntityId::Soldier(SoldierId(1)));
        ai.set_state(
            AiState::Seeking,
            Substate::SeekingSeekpointApproachingBeggar,
        );
        ai.beggar_to_examine = 17;
        ai.beggar_is_npc = false;
        ai.is_archer_unit = archer;

        let mut beggar = pc_view(Posture::SimulatingBeggar);
        beggar.position = Position {
            x: 140.0,
            y: 80.0,
            ..Position::default()
        };
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(17, beggar);
        let ctx = AiContext {
            frame: 400,
            position: Position {
                x: 100.0,
                y: 100.0,
                ..Position::default()
            },
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        assert!(ai.base.outbox.actor.orders.is_empty());
        assert!(ai.base.outbox.actor.launch_commands.is_empty());
        let [sequence] = ai.base.outbox.actor.launch_sequences.as_slice() else {
            panic!("beggar identification must launch exactly one sequence");
        };
        assert_eq!(sequence.elements.len(), 2);
        assert_eq!(sequence.elements[0].command, Command::TurnFast);
        assert_eq!(sequence.elements[0].command_level, 1);
        assert_eq!(sequence.elements[1].command, response);
        assert_eq!(sequence.elements[1].command_level, 2);
        let expected_direction =
            crate::position_interface::vector_to_sector_0_to_15_iso(40.0, -20.0);
        assert!(matches!(
            sequence.elements[0].get_property(Field::Direction),
            Some(FieldValue::Integer(direction)) if *direction == expected_direction as u32
        ));
        assert_eq!(ai.base.when_does_timer_ring, 400 + timer);
    }
}

#[test]
fn identified_npc_beggar_shows_face_then_identifies_himself() {
    use crate::ai::{AiOwnerWork, Remark};
    use crate::element::Command;

    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.set_state(
        AiState::Seeking,
        Substate::SeekingSeekpointIdentifyingBeggar1,
    );
    ai.base.outbox = crate::ai::AiOutbox::default();
    ai.beggar_to_examine = 70;
    ai.beggar_is_npc = true;

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &AiContext::default(),
        &AiPerTickData::stub(),
        None,
    );

    let prefix = ai
        .base
        .outbox
        .reentrant
        .owner_work
        .iter()
        .find_map(|work| match work {
            AiOwnerWork::StateChange(change) => change.actor_effects_before_callback.as_ref(),
            _ => None,
        })
        .expect("beggar response must precede the identifying-2 SetState callback");
    assert_eq!(prefix.launch_on_target, vec![(70, Command::BeggarShowFace)]);
    assert_eq!(
        prefix.say_on_target,
        vec![(70, Remark::CivBeggarIdentifiesHimself)]
    );
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingSeekpointIdentifyingBeggar2
    );
}

#[test]
fn combat_alert_ignores_timer_until_reaching_the_alert_point() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.set_state(AiState::Seeking, Substate::SeekingCombatAlert);

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &AiContext::default(),
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(ai.base.current_substate, Substate::SeekingCombatAlert);
}

#[test]
fn combat_alert_reachpoint_starts_lost_enemy_seek() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.set_state(AiState::Seeking, Substate::SeekingCombatAlert);
    ai.base.seek_position = Position {
        x: 120.0,
        y: 240.0,
        ..Position::default()
    };
    let mut global = AiGlobalState::default();

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut global,
        &AiContext::default(),
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(ai.seek_center, ai.base.seek_position);
    assert!(ai.seek_flags.is_empty());
    assert!(ai.personal_seek_point_2.is_some());
    assert_ne!(ai.base.current_substate, Substate::SeekingCombatAlert);
}

#[test]
fn heardsteps_arrival_starts_zero_radius_walking_seek() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(138);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingHeardsteps;
    ai.base.seek_position = Position {
        x: 900.0,
        y: 700.0,
        ..Position::default()
    };
    let here = Position {
        x: 1630.6875,
        y: 1630.921875,
        ..Position::default()
    };
    let ctx = AiContext {
        frame: 1257,
        position: here,
        camp: crate::element::Camp::Lacklandists,
        self_animation: crate::order::OrderType::TransitionWalkingUprightWaitingUpright,
        ..AiContext::default()
    };
    let mut global = AiGlobalState::default();

    let (_, draws) = crate::sim_rng::with_draw_trace(|| {
        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut global,
            &ctx,
            &AiPerTickData::stub(),
            None,
        );
    });

    assert_eq!(
        draws,
        vec![
            crate::sim_rng::RngSite::SeekPointDirectionPattern,
            crate::sim_rng::RngSite::SeekPointAcceptance,
        ]
    );
    assert_eq!(ai.seek_center, here);
    assert_eq!(
        ai.seek_flags,
        SeekFlags::LOCATION_FIRST | SeekFlags::WALKING
    );
    assert_eq!(ai.base.current_substate, Substate::SeekingSeekpoint);
    assert_eq!(ai.actual_seek_point, Some(1111));
    assert_eq!(
        ai.base.seek_position,
        Position {
            x: 900.0,
            y: 700.0,
            ..Position::default()
        },
        "selecting a route point must preserve the semantic heard-steps position"
    );
    assert!(
        ai.personal_seek_point_1
            .as_ref()
            .is_some_and(|point| point.locked)
    );
    // The old shortcut authored a Face/Turn order here. SeekArea owns the
    // post-arrival lifecycle instead; at this pure-AI boundary it has no
    // live actor order (the zero-distance completion is settled by the
    // enclosing Think lifecycle).
    assert!(ai.base.outbox.actor.orders.is_empty());
}

#[test]
fn reaching_near_officer_redispatches_reachpoint_synchronously() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.set_state(AiState::Seeking, Substate::SeekingRunningToOfficer);
    ai.base.antagonist = 2;
    let mut tick = AiPerTickData::stub();
    tick.camp_soldiers.push(crate::ai_enemy::CampSoldierInfo {
        handle: 2,
        active: true,
        position: Position::default(),
        position_world: crate::coordinates::WorldPoint3D::ZERO,
        direction: 0,
        rank: ProfileRank::Officer,
        ai_state: AiState::Default,
        ai_substate: Substate::DefaultOnPost,
        is_able_to_fight: true,
        is_dead: false,
        knocked_out_in_money_fight: false,
        primary_target: 0,
        pride: 0,
        is_able_to_help: true,
        script_locked: false,
        ai_lock_frozen: false,
        layer: 0,
        report_type: ReportType::Nothing,
        report_seek_position: Position::default(),
        report_seen_bodies: Vec::new(),
        report_charly: 0,
        alert_soldiers_point: Position::default(),
        patrol_chief: None,
        antagonist: 0,
        detected_body: 0,
        blood_alcohol: 0,
        duty_flag: false,
        is_tower_guard: false,
        company_number: 0,
        in_building: false,
        forecast_destination: None,
        detectable_bodies: Vec::new(),
        seek_position: Position::default(),
        current_task_priority: 0,
        minimal_task_priority: 0,
        view_direction: [1.0, 0.0],
        view_radius: 400,
        real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        eye_blind: false,
    });

    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    let mut officer = soldier_view_with_substate(2, Substate::DefaultOnPost);
    officer.ai_state = AiState::Default;
    officer.position = Position::default();
    views.insert(2, officer);
    let ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingRunningToOfficerSeen
    );
    assert_eq!(
        ai.base.outbox.reentrant.self_stimuli,
        vec![StimulusType::EventReachPoint]
    );
    assert!(!ai.base.timer_is_running);
}

#[test]
fn running_to_officer_tracks_rejected_civilian_alert_antagonist() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.set_state(AiState::Seeking, Substate::SeekingRunningToOfficer);
    ai.base.antagonist = 2;
    ai.gather_position = Position {
        x: 964.0,
        y: 2695.0,
        ..Position::default()
    };
    let civilian_position = Position {
        x: 847.573_975,
        y: 2436.898_19,
        ..Position::default()
    };
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(2, civilian_view(2, civilian_position));
    let ctx = AiContext {
        frame: 31_608,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.gather_position, civilian_position);
    assert_eq!(ai.base.last_goto_destination, civilian_position);
    assert_eq!(ai.base.when_does_timer_ring, 31_658);
}

#[test]
fn parade_timer_stops_only_an_active_normal_parry() {
    let sim = crate::sim_rng::test_context();

    for (action_state, should_stop) in [
        (crate::element::ActionState::WaitingSword, false),
        (crate::element::ActionState::ParryingSword, true),
        (crate::element::ActionState::ParryingSwordLow, false),
    ] {
        let mut ai = EnemyAi::new(1);
        ai.set_state(AiState::Attacking, Substate::AttackingSwordfightParade);
        let ctx = AiContext {
            frame: 325,
            self_action_state: action_state,
            ..AiContext::default()
        };

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        );

        // The stop-parry command is issued before the handler's SetState
        // suspends the actor-outbox prefix into the queued state-change
        // owner work; collect commands from both places.
        let mut launch_commands: Vec<crate::element::Command> = Vec::new();
        for work in &ai.base.outbox.reentrant.owner_work {
            if let crate::ai::AiOwnerWork::StateChange(notification) = work
                && let Some(effects) = &notification.actor_effects_before_callback
            {
                launch_commands.extend(effects.launch_commands.iter().copied());
            }
        }
        launch_commands.extend(ai.base.outbox.actor.launch_commands.iter().copied());
        assert_eq!(
            launch_commands == vec![crate::element::Command::StopParrySword],
            should_stop,
            "unexpected stop-parry emission for {action_state:?}"
        );
        assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
        assert_eq!(ai.base.when_does_timer_ring, 345);
    }
}

fn pc_view(posture: crate::element::Posture) -> crate::ai_entity_view::AiEntityView {
    let entity = crate::element::Entity::Pc(crate::element::ActorPc {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorPc,
            posture,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    crate::ai_entity_view::entity_view_from_entity(
        &entity,
        41,
        false,
        None,
        None,
        crate::order::OrderType::NonanimationEnd,
    )
}

fn alert_candidate(handle: u32, position: Position) -> crate::ai_enemy::CampSoldierInfo {
    crate::ai_enemy::CampSoldierInfo {
        handle,
        active: true,
        position,
        position_world: crate::coordinates::WorldPoint3D::new(position.x, position.y, 0.0),
        direction: 0,
        rank: ProfileRank::Soldier,
        ai_state: AiState::Default,
        ai_substate: Substate::DefaultOnPost,
        is_able_to_fight: true,
        is_dead: false,
        knocked_out_in_money_fight: false,
        primary_target: 0,
        pride: 0,
        is_able_to_help: true,
        script_locked: false,
        ai_lock_frozen: false,
        layer: 0,
        report_type: ReportType::Nothing,
        report_seek_position: Position::default(),
        report_seen_bodies: Vec::new(),
        report_charly: 0,
        alert_soldiers_point: Position::default(),
        patrol_chief: None,
        antagonist: 0,
        detected_body: 0,
        blood_alcohol: 0,
        duty_flag: false,
        is_tower_guard: false,
        company_number: 0,
        in_building: false,
        forecast_destination: None,
        detectable_bodies: Vec::new(),
        seek_position: Position::default(),
        current_task_priority: 0,
        minimal_task_priority: 0,
        view_direction: [1.0, 0.0],
        view_radius: 300,
        real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        eye_blind: false,
    }
}

#[test]
fn officer_body_reaction_uses_stretched_max_norm_to_delegate() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(185);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingBodyReactiontime;
    ai.base.detected_body = 179;
    ai.soldier_profile_rank = ProfileRank::Officer;
    ai.soldier_profile_initiative = 0;

    let mut body = pc_view(crate::element::Posture::Lying);
    body.kind = crate::ai_entity_view::EntityKind::Soldier;
    body.is_pc = false;
    body.position = Position {
        x: 50.0,
        y: 149.5,
        ..Position::default()
    };
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(179, body);
    let mut friend = soldier_view_with_substate(181, Substate::DefaultOnPost);
    friend.position = Position {
        x: 20.0,
        ..Position::default()
    };
    views.insert(181, friend);
    let ctx = AiContext {
        frame: 1_200,
        position: Position::default(),
        direction: 7,
        self_is_active: true,
        in_building: false,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };
    let mut tick = AiPerTickData::stub();
    tick.camp_soldiers.push(alert_candidate(
        181,
        Position {
            x: 20.0,
            ..Position::default()
        },
    ));

    let raw_max_norm = 50.0_f32.max(149.5);
    let stretched_max_norm = ai_max_norm_distance(
        &ctx.entity_view(179).unwrap().position,
        0.0,
        &ctx.position,
        0.0,
    );
    assert!(raw_max_norm < 150.0);
    assert!(stretched_max_norm > 150.0);

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingOfficerLookingForSoldiers1
    );
    assert_eq!(ai.base.when_does_timer_ring, 1_220);
    assert!(
        ai.base
            .outbox
            .actor
            .orders
            .iter()
            .any(|order| { order.order_type == crate::order::OrderType::Turning })
    );
    let friend = (
        crate::element::EntityId::Soldier(crate::entity_id::SoldierId(181)),
        crate::element::DetectableType::Friend,
    );
    assert!(
        ai.base.outbox.actor.add_detectables.contains(&friend)
            || ai.base.outbox.reentrant.owner_work.iter().any(|work| {
                matches!(work, AiOwnerWork::StateChange(change)
                if change.actor_effects_before_callback.as_ref().is_some_and(
                    |effects| effects.add_detectables.contains(&friend)
                ))
            })
    );
}

#[test]
fn officer_body_reaction_examines_body_within_stretched_threshold() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(185);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingBodyReactiontime;
    ai.base.detected_body = 179;
    ai.soldier_profile_rank = ProfileRank::Officer;
    ai.soldier_profile_initiative = 0;

    let mut body = pc_view(crate::element::Posture::Lying);
    body.kind = crate::ai_entity_view::EntityKind::Soldier;
    body.is_pc = false;
    body.position = Position {
        x: 50.0,
        y: 80.0,
        ..Position::default()
    };
    let destination = body.position;
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(179, body);
    let ctx = AiContext {
        frame: 1_200,
        position: Position::default(),
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    assert!(ai_max_norm_distance(&destination, 0.0, &ctx.position, 0.0) <= 150.0);
    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.current_substate, Substate::SeekingBody);
    assert_eq!(ai.base.seek_position, destination);
    assert!(
        ai.base.outbox.actor.focus == Some(179)
            || ai.base.outbox.reentrant.owner_work.iter().any(|work| {
                matches!(work, AiOwnerWork::StateChange(change)
                if change.actor_effects_before_callback.as_ref().is_some_and(
                    |effects| effects.focus == Some(179)
                ))
            })
    );
    assert!(
        ai.base
            .outbox
            .actor
            .orders
            .iter()
            .any(|order| { order.order_type == crate::order::OrderType::RunningUpright })
    );
    assert!(ai.base.outbox.actor.add_detectables.is_empty());
}

fn run_approaching_sleeping_enemy(target_live: Position) -> EnemyAi {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(94);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingApproachingSleepingEnemy;
    ai.base.primary_target = 41;

    let mut target = pc_view(crate::element::Posture::Upright);
    target.position = target_live;
    target.detection_position_world.z = 0.0;
    target.is_unconscious = true;
    target.in_coma = false;
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(41, target);
    let ctx = AiContext {
        position: Position::default(),
        elevation: 0.0,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };
    let mut tick = AiPerTickData::stub();
    tick.owner_live_position = Some(Position::default());
    tick.primary_target_snapshot_handle = 41;
    tick.primary_target_live_position = Some(target_live);

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventDone),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );
    ai
}

#[test]
fn approaching_sleeping_enemy_uses_stretched_world_distance() {
    let ai = run_approaching_sleeping_enemy(Position {
        x: 15.0,
        y: 34.0,
        ..Position::default()
    });

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingApproachingSleepingEnemy,
        "raw map max-norm is only 34, but Original Distance stretches Y and exceeds 40"
    );
    let [movement] = ai.base.outbox.actor.orders.as_slice() else {
        panic!("distant sleeping target should author exactly one GoNear movement")
    };
    assert_eq!((movement.target_x, movement.target_y), (15.0, 34.0));
    assert_eq!(movement.tolerance, 20.0);
    assert!(ai.base.outbox.actor.launch_sequences.is_empty());
}

#[test]
fn approaching_sleeping_enemy_close_target_launches_down_strike() {
    let ai = run_approaching_sleeping_enemy(Position {
        x: 10.0,
        y: 10.0,
        ..Position::default()
    });

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingKillingSleepingEnemy
    );
    assert!(ai.base.outbox.actor.orders.is_empty());
    let [sequence] = ai.base.outbox.actor.launch_sequences.as_slice() else {
        panic!("close sleeping target should launch exactly one strike sequence")
    };
    assert_eq!(
        sequence.get(0).map(|element| element.command),
        Some(crate::element::Command::SwordstrikeDown)
    );
}

fn run_reactiontime_turning(
    frame: u32,
    owner_forecast: Position,
    owner_live: Position,
    target_forecast: Position,
    target_live: Position,
) -> EnemyAi {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(155);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontimeTurning;
    ai.base.primary_target = 345;

    let mut target = pc_view(crate::element::Posture::Upright);
    target.position = target_forecast;
    target.detection_position =
        crate::coordinates::MapPoint::new(target_forecast.x, target_forecast.y);
    target.detection_position_world =
        crate::coordinates::WorldPoint3D::new(target_live.x, target_live.y + 480.0, 480.0);
    target.current_animation = crate::order::OrderType::WaitingUpright;
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(345, target);
    let ctx = AiContext {
        frame,
        position: owner_forecast,
        self_body_position_world: crate::coordinates::WorldPoint3D::new(
            owner_live.x,
            owner_live.y + 480.0,
            480.0,
        ),
        elevation: 480.0,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    ai.think_expected_attacking_event(
        &sim,
        &Stimulus::new(StimulusType::EventDone),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );
    ai
}

#[test]
fn reactiontime_turning_uses_live_distance_during_door_pass() {
    let owner_forecast = Position {
        x: 367.0,
        y: 757.0,
        ..Position::default()
    };
    let target_forecast = Position {
        x: 360.0,
        y: 750.0,
        ..Position::default()
    };
    let owner_live = owner_forecast;
    let target_live = Position {
        x: 354.0,
        y: 731.0,
        ..Position::default()
    };

    let forecast_max_norm = (target_forecast.x - owner_forecast.x)
        .abs()
        .max((target_forecast.y - owner_forecast.y).abs());
    let live_distance = ai_square_distance(&target_live, 480.0, &owner_live, 480.0).sqrt();
    assert!(forecast_max_norm < 30.0);
    assert!((47.0..48.0).contains(&live_distance));

    let ai = run_reactiontime_turning(
        36_150,
        owner_forecast,
        owner_live,
        target_forecast,
        target_live,
    );

    assert_eq!(ai.base.current_substate, Substate::AttackingReactiontime);
    assert_eq!(
        ai.base.when_does_timer_ring,
        36_150 + parameters_ai::AI_QUICK_ENEMY_REACTIONTIME as u32
    );
}

#[test]
fn reactiontime_turning_uses_one_tick_timer_when_live_target_is_close() {
    let owner_live = Position {
        x: 367.0,
        y: 757.0,
        ..Position::default()
    };
    let target_live = Position {
        x: 380.0,
        y: 757.0,
        ..Position::default()
    };
    let owner_forecast = owner_live;
    let target_forecast = Position {
        x: 500.0,
        y: 900.0,
        ..Position::default()
    };

    let forecast_max_norm = (target_forecast.x - owner_forecast.x)
        .abs()
        .max((target_forecast.y - owner_forecast.y).abs());
    let live_distance = ai_square_distance(&target_live, 480.0, &owner_live, 480.0).sqrt();
    assert!(forecast_max_norm > 30.0);
    assert_eq!(live_distance, 13.0);

    let ai = run_reactiontime_turning(
        36_151,
        owner_forecast,
        owner_live,
        target_forecast,
        target_live,
    );

    assert_eq!(ai.base.current_substate, Substate::AttackingReactiontime);
    assert_eq!(ai.base.when_does_timer_ring, 36_152);
}

#[test]
fn instructed_soldier_adds_officers_selected_body_after_speech() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(89);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingSoldierGetInstructedByOfficer;
    ai.base.antagonist = 91;

    let alert_point = Position {
        x: 2100.0,
        y: 1600.0,
        ..Position::default()
    };
    let officer_position = Position {
        x: 2050.0,
        y: 1550.0,
        ..Position::default()
    };
    let mut tick = AiPerTickData::stub();
    tick.camp_soldiers.push(crate::ai_enemy::CampSoldierInfo {
        handle: 91,
        active: true,
        position: officer_position,
        position_world: crate::coordinates::WorldPoint3D::ZERO,
        direction: 0,
        rank: ProfileRank::Officer,
        ai_state: AiState::Seeking,
        ai_substate: Substate::SeekingOfficerWaitForInstructedSoldier,
        is_able_to_fight: true,
        is_dead: false,
        knocked_out_in_money_fight: false,
        primary_target: 0,
        pride: 0,
        is_able_to_help: true,
        script_locked: false,
        ai_lock_frozen: false,
        layer: 0,
        report_type: ReportType::Nothing,
        report_seek_position: Position::default(),
        report_seen_bodies: Vec::new(),
        report_charly: 0,
        alert_soldiers_point: alert_point,
        patrol_chief: None,
        antagonist: 89,
        detected_body: 97,
        blood_alcohol: 0,
        duty_flag: false,
        is_tower_guard: false,
        company_number: 0,
        in_building: false,
        forecast_destination: None,
        detectable_bodies: Vec::new(),
        seek_position: Position::default(),
        current_task_priority: 0,
        minimal_task_priority: 0,
        view_direction: [1.0, 0.0],
        view_radius: 400,
        real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        eye_blind: false,
    });

    let mut body = pc_view(crate::element::Posture::Tied);
    body.kind = crate::ai_entity_view::EntityKind::Soldier;
    body.is_pc = false;
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(97, body);
    let ctx = AiContext {
        camp: crate::element::Camp::Lacklandists,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    // Speech completion carries no body payload. Original reads the
    // officer's selected body directly before sending CALL_YOURTALK_2.
    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventMyTalk2),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert!(matches!(
        ai.base.outbox.reentrant.cross_npc_actions.first(),
        Some(CrossNpcAction::SendStimulus {
            target: 91,
            stimulus_type: StimulusType::CallYourTalk2,
            info: StimulusInfo::None,
            ..
        })
    ));
    let mut added_detectables = ai.base.outbox.actor.add_detectables.clone();
    for work in &ai.base.outbox.reentrant.owner_work {
        match work {
            AiOwnerWork::ActorEffects(effects) => {
                added_detectables.extend(effects.add_detectables.iter().copied());
            }
            AiOwnerWork::StateChange(change) => {
                if let Some(effects) = &change.actor_effects_before_callback {
                    added_detectables.extend(effects.add_detectables.iter().copied());
                }
            }
            _ => {}
        }
    }
    assert_eq!(
        added_detectables,
        vec![(
            crate::element::EntityId::Soldier(crate::entity_id::SoldierId(97)),
            crate::element::DetectableType::Body,
        )]
    );
    assert_eq!(ai.base.alert_soldiers_point, alert_point);
    assert_eq!(ai.officers_position, officer_position);
}

#[test]
fn seeking_body_reach_rejects_a_body_outside_the_live_sixty_unit_gate() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(61);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingBody;
    ai.base.detected_body = 170;

    let mut owner = pc_view(crate::element::Posture::Upright);
    owner.detection_position_world = crate::coordinates::WorldPoint3D::new(413.38, 1850.28, 150.0);
    let mut body = pc_view(crate::element::Posture::Dead);
    body.is_able_to_fight = false;
    body.is_dead = true;
    body.detection_position_world = crate::coordinates::WorldPoint3D::new(737.20, 1869.22, 0.0);

    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(61, owner);
    views.insert(170, body);
    let ctx = AiContext {
        position: Position {
            x: 413.38,
            y: 1700.28,
            ..Position::default()
        },
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_ne!(
        ai.base.current_substate,
        Substate::SeekingBodyLookingDeadBody
    );
    let mut added_detectables = ai.base.outbox.actor.add_detectables.clone();
    for work in &ai.base.outbox.reentrant.owner_work {
        match work {
            AiOwnerWork::ActorEffects(effects) => {
                added_detectables.extend(effects.add_detectables.iter().copied());
            }
            AiOwnerWork::StateChange(change) => {
                if let Some(effects) = &change.actor_effects_before_callback {
                    added_detectables.extend(effects.add_detectables.iter().copied());
                }
            }
            _ => {}
        }
    }
    assert_eq!(
        added_detectables,
        vec![(
            crate::element::EntityId::Pc(crate::entity_id::PcId(170)),
            crate::element::DetectableType::Body,
        )]
    );
}

#[test]
fn seeking_body_reach_does_not_turn_toward_a_nearby_dead_body() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(61);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingBody;
    ai.base.detected_body = 170;
    ai.base.seek_position = Position {
        x: 120.0,
        y: 100.0,
        ..Position::default()
    };

    let mut owner = pc_view(crate::element::Posture::Upright);
    owner.detection_position_world = crate::coordinates::WorldPoint3D::new(100.0, 250.0, 150.0);
    let mut body = pc_view(crate::element::Posture::Dead);
    body.is_able_to_fight = false;
    body.is_dead = true;
    body.detection_position_world = crate::coordinates::WorldPoint3D::new(120.0, 250.0, 150.0);

    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(61, owner);
    views.insert(170, body);
    let ctx = AiContext {
        direction: 9,
        position: Position {
            x: 100.0,
            y: 100.0,
            ..Position::default()
        },
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingBodyLookingDeadBody
    );
    assert!(ai.already_seen_bodies.contains(&170));
    assert!(ai.base.outbox.actor.launch_commands.is_empty());
    assert!(ai.base.outbox.actor.launch_sequences.is_empty());
}

#[test]
fn seeking_body_reach_returns_to_duty_when_the_nearby_body_recovered() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(61);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingBody;
    ai.base.detected_body = 63;

    let mut owner = pc_view(crate::element::Posture::Upright);
    owner.detection_position_world =
        crate::coordinates::WorldPoint3D::new(413.38, 1850.2786, 150.001);
    let mut recovered_body = pc_view(crate::element::Posture::Upright);
    recovered_body.is_dead = false;
    recovered_body.is_unconscious = false;
    recovered_body.detection_position_world =
        crate::coordinates::WorldPoint3D::new(410.0648, 1850.421, 150.001);

    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(61, owner);
    views.insert(63, recovered_body);
    let ctx = AiContext {
        position: Position {
            x: 413.38,
            y: 1700.2776,
            ..Position::default()
        },
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_ne!(
        ai.base.current_substate,
        Substate::SeekingBodyLookingDeadBody
    );
    assert!(ai.already_seen_bodies.is_empty());
    assert!(
        ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(work, AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }))
    );
}

#[test]
fn arrow_reactiontime_uses_plain_goto_without_near_tolerance() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingArrowReactiontime;
    ai.base.seek_position = Position {
        x: 1030.0,
        y: 2424.0,
        sector: crate::position_interface::SectorHandle::new(18),
        ..Position::default()
    };
    let ctx = AiContext {
        position: Position {
            x: 706.0,
            y: 2666.0,
            sector: crate::position_interface::SectorHandle::new(18),
            ..Position::default()
        },
        self_is_soldier: true,
        ..AiContext::default()
    };

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.current_substate, Substate::SeekingArrow);
    assert_eq!(ai.base.last_goto_flags, GotoFlags::RUN);
    assert!(!ai.base.stop_before_end_of_path);
    let [movement] = ai.base.outbox.actor.orders.as_slice() else {
        panic!("arrow reactiontime must author exactly one movement")
    };
    assert_eq!((movement.target_x, movement.target_y), (1030.0, 2424.0));
    assert_eq!(movement.tolerance, 0.0);
}

#[test]
fn arrow_watching_ignores_event_done() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    // Original: RHArtificialMalignity::ThinkExpectedEvent handles only
    // EVENT_TIMER and EVENT_MYTALK_1 for the just-watching substate.
    for substate in [
        Substate::SeekingArrowJustWatching,
        Substate::SeekingArrowJustWatchingSidewards,
    ] {
        let mut ai = EnemyAi::new(1);
        ai.set_state(AiState::Seeking, substate);
        let mut global = AiGlobalState::default();

        ai.think_expected_event(
            sim,
            &Stimulus::new(StimulusType::EventDone),
            &mut global,
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Seeking);
        assert_eq!(ai.base.current_substate, substate);
    }
}

#[test]
fn bow_transition_states_ignore_shield_bearer_coordinate_calls() {
    let sim = crate::sim_rng::test_context();

    // RHArtificialMalignity::ThinkExpectedAttackingEvent only handles
    // CALL_COORDINATE while the archer is in BOW_SHOOTING.  A shield
    // bearer can still make the synchronous call while its archer is
    // loading or aiming; these substates deliberately ignore it.
    for substate in [
        Substate::AttackingBowObservingLoading,
        Substate::AttackingBowLoading,
        Substate::AttackingBowAiming,
    ] {
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = substate;
        ai.base.timer_is_running = true;
        ai.base.when_does_timer_ring = 777;

        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::CallCoordinate),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Attacking);
        assert_eq!(ai.base.current_substate, substate);
        assert!(ai.base.timer_is_running);
        assert_eq!(ai.base.when_does_timer_ring, 777);
        assert!(ai.base.outbox.actor.orders.is_empty());
        assert!(ai.base.outbox.actor.launch_sequences.is_empty());
    }
}

#[test]
fn goto_chief_reach_faces_live_chief_with_elevation() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(53);
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultGotoChief;
    ai.base.patrol_chief = Some(crate::element::EntityId::Soldier(
        crate::entity_id::SoldierId(47),
    ));
    let chief = crate::element::Entity::Soldier(crate::element::ActorSoldier {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorSoldier,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    });
    let mut chief_view = crate::ai_entity_view::entity_view_from_entity(
        &chief,
        47,
        false,
        None,
        None,
        crate::order::OrderType::NonanimationEnd,
    );
    chief_view.position = Position {
        x: 1033.585_9,
        y: 2036.767,
        ..Position::default()
    };
    chief_view.elevation = 25.100_779;
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(47, chief_view);
    let ctx = AiContext {
        frame: 34_866,
        position: Position {
            x: 1021.08,
            y: 2031.790_4,
            ..Position::default()
        },
        elevation: 27.711_25,
        direction: 6,
        self_action_state: crate::element::ActionState::Waiting,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };
    let mut tick = AiPerTickData::stub();
    tick.patrol_chief_position = ctx.entity_position(47).expect("chief view");

    // The old cached-position overload selects the current sector and
    // incorrectly completes synchronously. The Original entity overload
    // adds `(SWORD)25.100779 - 27.71125` to Y and selects sector 5.
    assert_eq!(
        crate::position_interface::vector_to_sector_0_to_15_iso(
            tick.patrol_chief_position.x - ctx.position.x,
            tick.patrol_chief_position.y - ctx.position.y,
        ),
        6
    );

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert!(!ai.base.already_turned);
    let [crate::ai::AiOwnerWork::StateChange(notification)] =
        ai.base.outbox.reentrant.owner_work.as_slice()
    else {
        panic!("goto-chief arrival must stage exactly one state change");
    };
    let effects = notification
        .actor_effects_before_callback
        .as_ref()
        .expect("Face must precede SetState");
    let [turn] = effects.orders.as_slice() else {
        panic!("elevation-aware chief facing must author exactly one turn");
    };
    assert_eq!(turn.order_type, crate::order::OrderType::Turning);
    assert_eq!(turn.explicit_direction, Some(5));
    assert_eq!(
        ai.base.current_substate,
        Substate::DefaultPatrolEnrouteWaiting
    );
    assert_eq!(ai.base.when_does_timer_ring, 35_066);
}

#[test]
#[should_panic(expected = "running on shooting path has no archery sector")]
fn shooting_path_does_not_fabricate_an_end_of_path_recovery() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.set_state(
        AiState::Attacking,
        Substate::AttackingArcherRunOnShootingPath,
    );
    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
        &AiContext::default(),
        &AiPerTickData::stub(),
        None,
    );
}

#[test]
#[should_panic(expected = "final sprint has no reserved shooting point")]
fn shooting_path_final_sprint_requires_its_reserved_point() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.set_state(
        AiState::Attacking,
        Substate::AttackingArcherRunOnShootingPathFinalSprint,
    );
    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
        &AiContext::default(),
        &AiPerTickData::stub(),
        None,
    );
}

#[test]
#[should_panic(expected = "receiving CALL_REPORT requires civilian 42 view")]
fn civilian_report_does_not_fabricate_enemy_data_when_sender_is_missing() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.set_state(AiState::Seeking, Substate::SeekingWaitForAlertingCivilian);
    let mut stimulus = Stimulus::new(StimulusType::CallReport);
    stimulus.info = StimulusInfo::Hint(Hint {
        seek_point: Position::default(),
        seek_flags: 0,
        who_tells_me: 42,
    });
    ai.think_expected_event(
        &sim,
        &stimulus,
        &mut AiGlobalState::default(),
        &AiContext::default(),
        &AiPerTickData::stub(),
        None,
    );
}

#[test]
fn charly_defence_completion_relays_talk_to_officer() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(55);
    ai.base.antagonist = 90;
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingCharlyGetLectureByOfficer;

    ai.seeking_charly_get_lecture_by_officer(StimulusType::CallYourTalk1);

    let speech_attempts = ai
        .base
        .outbox
        .reentrant
        .owner_work
        .iter()
        .filter_map(|work| match work {
            AiOwnerWork::Speech(attempt) => Some(attempt),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [attempt] = speech_attempts.as_slice() else {
        panic!("Charly must queue exactly one defence line");
    };
    assert_eq!(attempt.remark, Remark::CharlyDefendsHimself);
    assert_eq!(
        SpeechFlags::from_bits_truncate(attempt.flags),
        SpeechFlags::MYTALK_1
    );
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingCharlyGetLectureByOfficer2
    );

    ai.seeking_charly_get_lecture_by_officer2(
        &sim,
        StimulusType::EventMyTalk1,
        &AiContext::default(),
        &AiPerTickData::stub(),
    );
    assert!(matches!(
        ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
        [CrossNpcAction::SendStimulus {
            target: 90,
            stimulus_type: StimulusType::CallYourTalk1,
            ..
        }]
    ));
}

#[test]
fn rankless_heardsteps_pre_reaction_uses_linux_v48_investigate_result() {
    let mut ai = EnemyAi::new(125);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingHeardstepsPreReactiontime;
    ai.soldier_profile_rank = ProfileRank::None;
    ai.base.seek_position = Position {
        x: 100.0,
        y: 50.0,
        ..Position::default()
    };
    let ctx = AiContext {
        frame: 54_726,
        self_is_active: false,
        in_building: false,
        ..AiContext::default()
    };

    ai.seeking_heardsteps_pre_reactiontime(StimulusType::EventTimer, &ctx, &AiPerTickData::stub());

    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingHeardstepsReactiontime
    );
    assert!(ai.base.timer_is_running);
    assert_eq!(
        ai.base.when_does_timer_ring,
        ctx.frame + parameters_ai::AI_FIRST_LOOK_TIME as u32
    );
}

#[test]
fn inactive_ranked_soldier_still_declines_to_follow_steps() {
    let mut ai = EnemyAi::new(126);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingHeardstepsPreReactiontime;
    ai.soldier_profile_rank = ProfileRank::Soldier;
    ai.soldier_profile_duty = false;
    let ctx = AiContext {
        frame: 100,
        self_is_active: false,
        in_building: false,
        ..AiContext::default()
    };

    ai.seeking_heardsteps_pre_reactiontime(StimulusType::EventTimer, &ctx, &AiPerTickData::stub());

    assert_eq!(ai.base.current_substate, Substate::SeekingJustWatching);
}

#[test]
fn group_called_by_officer_moves_before_single_state_transition() {
    let mut ai = EnemyAi::new(53);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingGroupCalledByOfficer;
    ai.base.antagonist = 60;
    ai.attentive = true;
    ai.will_be_attentive = true;
    ai.gather_position_instructed = true;
    ai.gather_position = Position {
        x: 700.0,
        y: 1800.0,
        level: 0,
        sector: crate::position_interface::SectorHandle::new(1),
    };
    let ctx = AiContext {
        frame: 24_365,
        position: Position {
            x: 734.0,
            y: 1796.0,
            level: 0,
            sector: crate::position_interface::SectorHandle::new(1),
        },
        self_layer: 0,
        ..AiContext::default()
    };

    ai.seeking_group_called_by_officer(StimulusType::EventTimer, &ctx, &AiPerTickData::stub());

    assert_eq!(ai.base.current_substate, Substate::SeekingGroupGoToOfficer);
    let [crate::ai::AiOwnerWork::StateChange(notification)] =
        ai.base.outbox.reentrant.owner_work.as_slice()
    else {
        panic!("group approach must publish exactly one SetState boundary");
    };
    let prefix = notification
        .actor_effects_before_callback
        .as_ref()
        .expect("Original GoTo precedes SetState");
    assert_eq!(prefix.orders.len(), 1, "GoTo belongs before SetState");
    assert!(
        prefix.set_attentive_mode.is_none() && prefix.additional_set_attentive_modes.is_empty(),
        "the raw GoTo must not inject a same-state attentive request"
    );
    let requests = ai.base.outbox.actor.take_attentive_modes();
    assert_eq!(
        requests.len(),
        1,
        "only the explicit SetState may request attention"
    );
    assert!(requests[0].target);
    assert!(!requests[0].fast_officer_variant);
}

#[test]
fn group_synchronous_reachpoint_same_direction_still_authors_turn() {
    let mut ai = EnemyAi::new(53);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingGroupGoToOfficer;
    // A non-waiting actor does not qualify for Original FaceTo's
    // same-direction shortcut, even when the already-at-destination GoTo
    // completion is surfaced recursively.
    ai.base.open_end_think_frames = 1;
    ai.gather_position_instructed = true;
    ai.gather_direction = 8;
    let ctx = AiContext {
        self_action_state: crate::element::ActionState::Moving,
        direction: 8,
        ..AiContext::default()
    };

    ai.seeking_group_go_to_officer(
        &crate::sim_rng::test_context(),
        StimulusType::EventReachPoint,
        &ctx,
        &AiPerTickData::stub(),
    );

    let [turn] = ai.base.outbox.actor.orders.as_slice() else {
        panic!("group ReachPoint must author one Turn");
    };
    assert_eq!(turn.order_type, crate::order::OrderType::Turning);
}

#[test]
fn group_synchronous_reachpoint_retained_waiting_uses_same_direction_shortcut() {
    let mut ai = EnemyAi::new(171);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingGroupGoToOfficer;
    // GoTo accepted an already-at-destination request without replacing
    // the actor's pre-existing Wait action. FaceTo therefore observes
    // Waiting while the recursive ReachPoint frame is still open.
    ai.base.open_end_think_frames = 1;
    ai.gather_position_instructed = true;
    ai.gather_direction = 4;
    let ctx = AiContext {
        self_action_state: crate::element::ActionState::Waiting,
        direction: 4,
        ..AiContext::default()
    };

    ai.seeking_group_go_to_officer(
        &crate::sim_rng::test_context(),
        StimulusType::EventReachPoint,
        &ctx,
        &AiPerTickData::stub(),
    );

    assert!(
        ai.base.outbox.actor.orders.is_empty(),
        "a completed movement already facing the gather direction must not turn"
    );
    assert!(
        ai.base.already_turned,
        "FaceTo's shortcut must schedule the synchronous EventDone"
    );
}

#[test]
fn group_reachpoint_keeps_raw_wrapped_gather_direction_for_face_to() {
    let mut ai = EnemyAi::new(66);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingGroupGoToOfficer;
    ai.gather_position_instructed = true;
    // AlertSoldiers' raw loop cursor can exceed 15. Original FaceTo
    // compares this raw value before the Turn order projects it to a
    // direction sector, so 29 must not short-circuit against sector 13.
    ai.gather_direction = 29;
    let ctx = AiContext {
        self_action_state: crate::element::ActionState::Waiting,
        direction: 13,
        ..AiContext::default()
    };

    ai.seeking_group_go_to_officer(
        &crate::sim_rng::test_context(),
        StimulusType::EventReachPoint,
        &ctx,
        &AiPerTickData::stub(),
    );

    let [turn] = ai.base.outbox.actor.orders.as_slice() else {
        panic!("raw gather direction 29 must author one Turn against sector 13");
    };
    assert_eq!(turn.order_type, crate::order::OrderType::Turning);
    assert_eq!(turn.explicit_direction, Some(29));
    assert!(!ai.base.already_turned);
    assert_eq!(ai.base.current_substate, Substate::SeekingGroupGoToOfficer);
}

#[test]
fn officer_ignores_missed_patrol_member_when_deciding_to_follow_nearby_steps() {
    let mut ai = EnemyAi::new(125);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingHeardstepsPreReactiontime;
    ai.soldier_profile_rank = ProfileRank::Officer;
    let missed_member = crate::element::EntityId::Soldier(crate::entity_id::SoldierId(131));
    ai.base.theoretical_patrol.push(missed_member);
    ai.base.missed_patrol_members.push(missed_member);
    ai.base.seek_position = Position {
        x: 1569.0,
        y: 705.0,
        ..Position::default()
    };
    let ctx = AiContext {
        frame: 54_703,
        position: Position {
            x: 1642.0,
            y: 636.0,
            ..Position::default()
        },
        ..AiContext::default()
    };
    let mut tick = AiPerTickData::stub();
    let mut stale_member = alert_candidate(131, Position::default());
    stale_member.patrol_chief = Some(crate::element::EntityId::Soldier(
        crate::entity_id::SoldierId(125),
    ));
    tick.camp_soldiers.push(stale_member);

    ai.seeking_heardsteps_pre_reactiontime(StimulusType::EventTimer, &ctx, &tick);

    assert!(ai.base.patrol.is_empty());
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingHeardstepsReactiontime
    );
}

#[test]
fn charly_lecture_ignores_unrelated_stimulus() {
    let mut ai = EnemyAi::new(55);
    ai.base.antagonist = 90;
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingCharlyGetLectureByOfficer;

    ai.seeking_charly_get_lecture_by_officer(StimulusType::EventTimer);

    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
    assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingCharlyGetLectureByOfficer
    );
}

#[test]
#[should_panic(expected = "looting soldier 55 is missing its required owner entity view")]
fn looting_requires_the_owner_entity_view() {
    let mut ai = EnemyAi::new(55);
    ai.wondering_looting(
        &crate::sim_rng::test_context(),
        StimulusType::EventDone,
        &AiContext::default(),
        &AiPerTickData::stub(),
    );
}

#[test]
#[should_panic(expected = "called soldier 55 requires officer 90 in the camp snapshot")]
fn called_soldier_requires_the_officer_snapshot() {
    let mut ai = EnemyAi::new(55);
    ai.base.antagonist = 90;
    ai.seeking_soldier_called_by_officer(
        StimulusType::EventTimer,
        &AiContext::default(),
        &AiPerTickData::stub(),
    );
}

#[test]
#[should_panic(expected = "shield bearer 55 is missing its required fighter snapshot")]
fn shield_bearer_requires_the_owner_fighter_snapshot() {
    let mut ai = EnemyAi::new(55);
    ai.attacking_protecting_with_shield(
        &crate::sim_rng::test_context(),
        StimulusType::EventTimer,
        &AiContext::default(),
        &AiPerTickData::stub(),
    );
}

#[test]
fn advancing_shield_uses_live_target_sector_for_indexed_route() {
    use crate::fast_find_grid::SectorIndex;
    use crate::gate::{Door, DoorIndex, build_gate_links, find_path_gates_with_sector_indices};
    use crate::sector::SectorNumber;

    let sim = crate::sim_rng::test_context();
    let arena = |index| SectorIndex::new(index).unwrap();
    let sector = |public, index| {
        SectorHandle::new(public)
            .unwrap()
            .with_arena_index(arena(index))
    };
    let source = Position {
        x: 100.0,
        y: 100.0,
        sector: Some(sector(0, 10)),
        level: 0,
    };
    let target = Position {
        x: 735.0,
        y: 1_659.0,
        sector: Some(sector(88, 12)),
        level: 2,
    };

    let mut ai = EnemyAi::new(150);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingAdvancingWithShield;
    ai.base.primary_target = 282;

    let mut target_view = pc_view(crate::element::Posture::Upright);
    target_view.position = target;
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(282, target_view);
    let ctx = AiContext {
        frame: 7_654,
        position: source,
        self_layer: source.level,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::default()
    };

    // Deliberately omit the target from the fighter registry. Original reads
    // the live element position for this GoNear destination.
    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventDone),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    let [order] = ai.base.outbox.actor.orders.as_slice() else {
        panic!("advancing shield must author exactly one GoNear movement");
    };
    assert_eq!(order.order_type, crate::order::OrderType::RunningUpright);
    assert_eq!(order.target_sector, target.sector);
    assert_eq!(order.target_sector_index, Some(arena(12)));
    assert_eq!(order.target_layer, Some(2));

    let mut doors = (0..114)
        .map(|_| Door {
            active: false,
            ..Door::default()
        })
        .collect::<Vec<_>>();
    doors[111] = Door {
        active: true,
        point_out: crate::coordinates::MapPoint::new(100.0, 100.0),
        point_in: crate::coordinates::MapPoint::new(400.0, 800.0),
        sector_out: SectorNumber::new(0),
        sector_in: SectorNumber::new(70),
        sector_out_index: Some(arena(10)),
        sector_in_index: Some(arena(11)),
        ..Door::default()
    };
    doors[113] = Door {
        active: true,
        point_out: crate::coordinates::MapPoint::new(735.0, 1_659.0),
        point_in: crate::coordinates::MapPoint::new(500.0, 1_000.0),
        sector_out: SectorNumber::new(88),
        sector_in: SectorNumber::new(70),
        sector_out_index: Some(arena(12)),
        sector_in_index: Some(arena(11)),
        ..Door::default()
    };
    build_gate_links(&mut doors);

    let route = find_path_gates_with_sector_indices(
        &doors,
        (source.x, source.y),
        source.sector.unwrap().get(),
        source.sector.unwrap().arena_index(),
        (order.target_x, order.target_y),
        order.target_sector.unwrap().get(),
        order.target_sector_index,
        None,
        false,
        &|_| true,
        &|_| None,
    )
    .expect("exact live target identity must launch the indexed route");
    assert_eq!(
        route
            .iter()
            .map(|step| (step.door_index, step.direct))
            .collect::<Vec<_>>(),
        vec![(DoorIndex(111), true), (DoorIndex(113), false)]
    );

    // Public sector 88 has a distinct duplicate in the arena. Losing the
    // live target's identity would make this route unresolvable.
    assert!(
        find_path_gates_with_sector_indices(
            &doors,
            (source.x, source.y),
            source.sector.unwrap().get(),
            source.sector.unwrap().arena_index(),
            (order.target_x, order.target_y),
            88,
            Some(arena(13)),
            None,
            false,
            &|_| true,
            &|_| None,
        )
        .is_none()
    );
}

#[test]
#[should_panic(
    expected = "required entity view for handle 282 missing (advancing shield primary target)"
)]
fn advancing_shield_requires_live_primary_target_view() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(150);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingAdvancingWithShield;
    ai.base.primary_target = 282;

    // A combat-registry snapshot cannot substitute for the live RHElement
    // position used by Original's Position(mpPrimaryTarget).
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry
        .push(crate::ai_enemy::FighterSnapshot {
            handle: 282,
            position: Position::default(),
            ..crate::ai_enemy::FighterSnapshot::default()
        });

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventDone),
        &mut AiGlobalState::default(),
        &AiContext::default(),
        &tick,
        None,
    );
}
