use super::*;

fn assert_duty_call(flow: crate::ai::AiFlow<bool>, think_result: bool, ai: &EnemyAi) {
    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
    let call = flow.expect_err("expected an engine-owned duty call");
    assert_eq!(call.flags, DutyFlags::empty());
    assert_eq!(call.think_result, think_result);
    assert!(matches!(call.tail, crate::ai::DutyTail::None));
    assert!(call.after.is_empty());
}

fn soldier_view_with_substate(
    handle: u32,
    substate: Substate,
) -> crate::ai_entity_view::AiEntityView {
    let entity = crate::element::Entity::Soldier(crate::element::ActorSoldier {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::ActorSoldier;
            initial_element
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
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::ActorCivilian;
            initial_element
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
    let ctx = AiContext::test_fixture();
    let tick = AiPerTickData::stub();

    let flow = ai.wondering_drinking_ale(
        ThinkEnv::new(&sim, &ctx, &tick, None),
        StimulusType::EventTimer,
    );
    assert_eq!(flow.unwrap(), false);
    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
    assert_eq!(ai.base.blood_alcohol, 17);

    let flow = ai.wondering_drinking_ale(
        ThinkEnv::new(&sim, &ctx, &tick, None),
        StimulusType::EventDone,
    );
    assert_duty_call(flow, false, &ai);
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
    ai.base.interesting_object = Some(AiEntityHandle::new(321));
    ai.base.seek_position = Position {
        x: 632.4453,
        y: 1835.14,
        sector: None,
        level: 0,
    };
    ai.soldier_profile_beer = 1;
    let ctx = AiContext {
        self_is_active: true,
        ..AiContext::test_fixture()
    };

    // No view for object 321: the bottle was consumed while React's timer
    // was pending. The original game still commits to the retained identity and position.
    let flow = ai.wondering_ale_reactiontime(
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        StimulusType::EventTimer,
    );

    assert_eq!(ai.base.current_state, AiState::Wondering);
    assert_eq!(ai.base.current_substate, Substate::WonderingApproachingAle);
    assert_eq!(ai.base.object_of_desire, Some(AiEntityHandle::new(321)));
    assert_eq!(ai.base.seek_position.x, 632.4453);
    assert_eq!(ai.base.seek_position.y, 1835.14);
    assert!(flow.is_ok());
}

#[test]
fn reliable_ale_only_expands_zero_beer_eligibility_outdoors_for_non_vips() {
    let mut ai = EnemyAi::new(90);
    ai.soldier_profile_beer = 0;
    let outdoors = AiContext {
        self_is_active: true,
        in_building: false,
        ..AiContext::test_fixture()
    };
    let indoors = AiContext {
        self_is_active: true,
        in_building: true,
        ..AiContext::test_fixture()
    };

    assert!(!ai.answer_question_ex(Question::ShallITakeAle, &outdoors, false));
    ai.ale_reliable_distraction = true;
    assert!(ai.answer_question_ex(Question::ShallITakeAle, &outdoors, false));
    assert!(!ai.answer_question_ex(Question::ShallITakeAle, &indoors, false));

    // VIP profiles cache the new-rule eligibility as false even when the
    // setting itself is on.
    ai.ale_reliable_distraction = false;
    assert!(!ai.answer_question_ex(Question::ShallITakeAle, &outdoors, false));
    ai.soldier_profile_beer = 35;
    assert!(
        ai.answer_question_ex(Question::ShallITakeAle, &outdoors, false),
        "positive authored beer remains eligible"
    );
}

#[test]
fn taking_money_event_done_selects_nearest_coin_and_starts_reaction_timer() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(90);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingTakingMoney;
    ai.base.interesting_object = Some(AiEntityHandle::new(130));
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
        ..AiContext::test_fixture()
    };

    ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        &Stimulus::new(StimulusType::EventDone),
        &mut AiGlobalState::default(),
    )
    .unwrap();

    assert_eq!(
        ai.base.current_substate,
        Substate::WonderingMoneyReactiontime
    );
    assert_eq!(ai.base.interesting_object, Some(AiEntityHandle::new(132)));
    assert_eq!(ai.other_seen_money, vec![131]);
    assert_eq!(ai.base.when_does_timer_ring, 9_248);
}

#[test]
fn taking_projectile_derived_coin_preserves_typed_interaction_target() {
    let mut ai = EnemyAi::new(90);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingApproachingMoney;
    ai.base.interesting_object = Some(AiEntityHandle::new(134));

    let mut coin = pc_view(crate::element::Posture::Upright);
    coin.kind = crate::ai_entity_view::EntityKind::Projectile;
    coin.object_type = crate::element_kinds::ObjectType::Coin;
    coin.position = Position::default();
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(134, coin);
    let ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
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
        ..AiContext::test_fixture()
    }
}

#[test]
fn approaching_money_timer_with_visible_rival_runs_instead_of_taking() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(90);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingApproachingMoney;
    ai.base.interesting_object = Some(AiEntityHandle::new(134));
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
        ThinkEnv::new(&sim, &ctx, &tick, None),
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
    )
    .unwrap();

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
    ai.base.interesting_object = Some(AiEntityHandle::new(134));
    let ctx = money_race_context();

    ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
    )
    .unwrap();

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
    ai.base.friend_in_trouble = Some(AiEntityHandle::new(90));
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
        ..AiContext::test_fixture()
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
        ThinkEnv::new(
            &crate::sim_rng::test_context(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        ),
        StimulusType::EventReachPoint,
    )
    .unwrap();

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
        ThinkEnv::new(
            &crate::sim_rng::test_context(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        ),
        StimulusType::EventReachPoint,
    )
    .unwrap();

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
        ThinkEnv::new(
            &crate::sim_rng::test_context(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        ),
        StimulusType::EventReachPoint,
    )
    .unwrap();

    assert_eq!(ai.base.current_substate, Substate::WonderingBrawlHitting);
    assert_eq!(ai.base.friend_in_trouble, None);
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
    ai.base.friend_in_trouble = None;
    ai.wondering_brawl_approaching(
        ThinkEnv::new(
            &crate::sim_rng::test_context(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        ),
        StimulusType::EventReachPoint,
    )
    .unwrap();

    assert_ne!(ai.base.current_substate, Substate::WonderingBrawlHitting);
    assert!(launched_hit_targets(&ai).is_empty());
}

#[test]
fn brawl_hitting_done_enqueues_only_180_degree_panic_sweep() {
    let mut ai = EnemyAi::new(88);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingBrawlHitting;
    ai.wondering_brawl_hitting(StimulusType::EventDone);

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
fn send_charly_speech_completion_faces_live_friend_before_waiting() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(93);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingSendCharlyToOfficer;
    ai.base.friend_in_trouble = Some(AiEntityHandle::new(94));

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
        ..AiContext::test_fixture()
    };

    ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        &Stimulus::new(StimulusType::EventMyTalk2),
        &mut AiGlobalState::default(),
    )
    .unwrap();

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
    ai.base.friend_in_trouble = Some(AiEntityHandle::new(94));
    let ctx = AiContext {
        frame: 12_391,
        direction: 12,
        self_action_state: crate::element::ActionState::Waiting,
        ..AiContext::test_fixture()
    };

    ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        &Stimulus::new(StimulusType::EventMyTalk2),
        &mut AiGlobalState::default(),
    )
    .unwrap();

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
        ..AiContext::test_fixture()
    };

    let flow = ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
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
    assert!(flow.is_ok());
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
        ..AiContext::test_fixture()
    };

    ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
    )
    .unwrap();

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
            ..AiContext::test_fixture()
        };

        ai.think_expected_event(
            ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
        )
        .unwrap();

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
        ..AiContext::test_fixture()
    };

    let flow = ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
    );

    assert!(ai.alerted_us.is_empty());
    assert_duty_call(flow, false, &ai);
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
        ai.base.my_reconnaissance_report.charly = Some(AiEntityHandle::new(148));

        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(148, soldier_view_with_substate(148, charly_substate));
        let ctx = AiContext {
            frame: 7_915,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::test_fixture()
        };

        let flow = ai.think_expected_event(
            ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
        );

        if should_wait {
            assert_eq!(flow.unwrap(), false);
            assert!(ai.base.timer_is_running);
            assert_eq!(ai.base.when_does_timer_ring, 7_945);
            assert!(ai.base.outbox.reentrant.owner_work.is_empty());
        } else {
            assert_duty_call(flow, false, &ai);
        }
    }
}

#[test]
fn fleeing_hiding_timer_invokes_enemy_return_to_duty() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(158);
    ai.base.current_state = AiState::Fleeing;
    ai.base.current_substate = Substate::FleeingHiding;

    let handled = ai.think_expected_fleeing_event(
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        crate::ai_enemy::ThinkEnv {
            sim: &sim,
            ctx: &AiContext::test_fixture(),
            tick: &AiPerTickData::stub(),
            grid: None,
        },
    );

    assert_duty_call(handled, true, &ai);
}

#[test]
fn officer_wait_missed_soldier_does_not_relaunch_timer() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingOfficerWaitForInstructedSoldier;
    ai.base.antagonist = Some(AiEntityHandle::new(2));
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
        primary_target: None,
        pride: 0,
        is_able_to_help: true,
        script_locked: false,
        ai_lock_frozen: false,
        layer: 0,
        alert_soldiers_point: Position::default(),
        patrol_chief: None,
        antagonist: Some(AiEntityHandle::new(1)),
        detected_body: None,
        blood_alcohol: 0,
        duty_flag: false,
        is_tower_guard: false,
        company_number: 0,
        in_building: false,
        detectable_bodies: Vec::new(),
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
        ..AiContext::test_fixture()
    };

    ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &tick, None),
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
    )
    .unwrap();

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
        ..AiContext::test_fixture()
    };

    ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
    )
    .unwrap();

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
        .expect("facing must precede the state change");
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
        .expect("default state change must queue its attentive-mode tail");
    assert!(!attentive.target);
    assert!(!attentive.fast_officer_variant);
    assert_eq!(ai.base.view_alert_status, crate::ai::AlertLevel::Green);
}

#[test]
fn combat_alert_ignores_timer_until_reaching_the_alert_point() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.set_state(AiState::Seeking, Substate::SeekingCombatAlert);

    ai.think_expected_event(
        ThinkEnv::new(
            &sim,
            &AiContext::test_fixture(),
            &AiPerTickData::stub(),
            None,
        ),
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
    )
    .unwrap();

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
        ThinkEnv::new(
            &sim,
            &AiContext::test_fixture(),
            &AiPerTickData::stub(),
            None,
        ),
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut global,
    )
    .unwrap();

    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(ai.seek_center, ai.base.seek_position);
    assert!(ai.seek_flags.is_empty());
    assert!(ai.personal_seek_point_2.is_some());
    assert_ne!(ai.base.current_substate, Substate::SeekingCombatAlert);
}

#[test]
fn reaching_near_officer_redispatches_reachpoint_synchronously() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.set_state(AiState::Seeking, Substate::SeekingRunningToOfficer);
    ai.base.antagonist = Some(AiEntityHandle::new(2));
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
        primary_target: None,
        pride: 0,
        is_able_to_help: true,
        script_locked: false,
        ai_lock_frozen: false,
        layer: 0,
        alert_soldiers_point: Position::default(),
        patrol_chief: None,
        antagonist: None,
        detected_body: None,
        blood_alcohol: 0,
        duty_flag: false,
        is_tower_guard: false,
        company_number: 0,
        in_building: false,
        detectable_bodies: Vec::new(),
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
        ..AiContext::test_fixture()
    };

    ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &tick, None),
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
    )
    .unwrap();

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
    ai.base.antagonist = Some(AiEntityHandle::new(2));
    ai.gather_position = Position {
        x: 964.0,
        y: 2695.0,
        ..Position::default()
    };
    let civilian_position = Position {
        x: 847.574,
        y: 2_436.898_2,
        ..Position::default()
    };
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(2, civilian_view(2, civilian_position));
    let ctx = AiContext {
        frame: 31_608,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
    )
    .unwrap();

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
            ..AiContext::test_fixture()
        };

        ai.think_expected_event(
            ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
            &Stimulus::new(StimulusType::EventTimer),
            &mut AiGlobalState::default(),
        )
        .unwrap();

        // The stop-parry command is issued before the handler's state change
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
        element: {
            let mut initial_element = crate::element::ElementData::from_initial_posture(posture);
            initial_element.kind = crate::element::ElementKind::ActorPc;
            initial_element
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
        primary_target: None,
        pride: 0,
        is_able_to_help: true,
        script_locked: false,
        ai_lock_frozen: false,
        layer: 0,
        alert_soldiers_point: Position::default(),
        patrol_chief: None,
        antagonist: None,
        detected_body: None,
        blood_alcohol: 0,
        duty_flag: false,
        is_tower_guard: false,
        company_number: 0,
        in_building: false,
        detectable_bodies: Vec::new(),
        current_task_priority: 0,
        minimal_task_priority: 0,
        view_direction: [1.0, 0.0],
        view_radius: 300,
        real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        eye_blind: false,
    }
}

#[test]
fn instructed_soldier_adds_officers_selected_body_after_speech() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(89);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingSoldierGetInstructedByOfficer;
    ai.base.antagonist = Some(AiEntityHandle::new(91));

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
        primary_target: None,
        pride: 0,
        is_able_to_help: true,
        script_locked: false,
        ai_lock_frozen: false,
        layer: 0,
        alert_soldiers_point: alert_point,
        patrol_chief: None,
        antagonist: Some(AiEntityHandle::new(89)),
        detected_body: Some(AiEntityHandle::new(97)),
        blood_alcohol: 0,
        duty_flag: false,
        is_tower_guard: false,
        company_number: 0,
        in_building: false,
        detectable_bodies: Vec::new(),
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
        ..AiContext::test_fixture()
    };

    // Speech completion carries no body payload. Original reads the
    // officer's selected body directly before sending CALL_YOURTALK_2.
    ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &tick, None),
        &Stimulus::new(StimulusType::EventMyTalk2),
        &mut AiGlobalState::default(),
    )
    .unwrap();

    assert!(matches!(
        ai.base.outbox.reentrant.cross_npc_actions.first(),
        Some(CrossNpcAction::SendStimulus {
            target: 91,
            stimulus_type: StimulusType::CallYourTalk2,
            info: StimulusInfo::None,
            ..
        })
    ));
    let mut added_detectables = ai.base.outbox.actor.added_detectables().clone();
    for work in &ai.base.outbox.reentrant.owner_work {
        match work {
            AiOwnerWork::ActorEffects(effects) => {
                added_detectables.extend(effects.added_detectables().iter().copied());
            }
            AiOwnerWork::StateChange(change) => {
                if let Some(effects) = &change.actor_effects_before_callback {
                    added_detectables.extend(effects.added_detectables().iter().copied());
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
    ai.base.detected_body = Some(AiEntityHandle::new(170));

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
        ..AiContext::test_fixture()
    };

    ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
    )
    .unwrap();

    assert_ne!(
        ai.base.current_substate,
        Substate::SeekingBodyLookingDeadBody
    );
    let mut added_detectables = ai.base.outbox.actor.added_detectables().clone();
    for work in &ai.base.outbox.reentrant.owner_work {
        match work {
            AiOwnerWork::ActorEffects(effects) => {
                added_detectables.extend(effects.added_detectables().iter().copied());
            }
            AiOwnerWork::StateChange(change) => {
                if let Some(effects) = &change.actor_effects_before_callback {
                    added_detectables.extend(effects.added_detectables().iter().copied());
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
    ai.base.detected_body = Some(AiEntityHandle::new(170));
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
        ..AiContext::test_fixture()
    };

    ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
    )
    .unwrap();

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
    ai.base.detected_body = Some(AiEntityHandle::new(63));

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
        ..AiContext::test_fixture()
    };

    let flow = ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
    );

    assert_ne!(
        ai.base.current_substate,
        Substate::SeekingBodyLookingDeadBody
    );
    assert!(ai.already_seen_bodies.is_empty());
    assert_duty_call(flow, false, &ai);
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
        ..AiContext::test_fixture()
    };

    ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &AiPerTickData::stub(), None),
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
    )
    .unwrap();

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
    // Original-game expected-event handling covers only
    // EVENT_TIMER and EVENT_MYTALK_1 for the just-watching substate.
    for substate in [
        Substate::SeekingArrowJustWatching,
        Substate::SeekingArrowJustWatchingSidewards,
    ] {
        let mut ai = EnemyAi::new(1);
        ai.set_state(AiState::Seeking, substate);
        let mut global = AiGlobalState::default();

        ai.think_expected_event(
            ThinkEnv::new(
                sim,
                &AiContext::test_fixture(),
                &AiPerTickData::stub(),
                None,
            ),
            &Stimulus::new(StimulusType::EventDone),
            &mut global,
        )
        .unwrap();

        assert_eq!(ai.base.current_state, AiState::Seeking);
        assert_eq!(ai.base.current_substate, substate);
    }
}

#[test]
fn bow_transition_states_ignore_shield_bearer_coordinate_calls() {
    let sim = crate::sim_rng::test_context();

    // Original-game expected attacking-event handling covers only
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
            ThinkEnv::new(
                &sim,
                &AiContext::test_fixture(),
                &AiPerTickData::stub(),
                None,
            ),
            &Stimulus::new(StimulusType::CallCoordinate),
            &mut AiGlobalState::default(),
        )
        .unwrap();

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
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::ActorSoldier;
            initial_element
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
        x: 1_033.585_9,
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
            y: 2_031.790_4,
            ..Position::default()
        },
        elevation: 27.711_25,
        direction: 6,
        self_action_state: crate::element::ActionState::Waiting,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    let chief_position = ctx.entity_position(47).expect("chief view");

    // The old cached-position overload selects the current sector and
    // incorrectly completes synchronously. The original-game entity-facing path
    // adds `signed_16(25.100779) - 27.71125` to Y and selects sector 5.
    assert_eq!(
        crate::position_interface::vector_to_sector_0_to_15_iso(
            chief_position.x - ctx.position.x,
            chief_position.y - ctx.position.y,
        ),
        6
    );

    ai.think_expected_event(
        ThinkEnv::new(&sim, &ctx, &tick, None),
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
    )
    .unwrap();

    assert!(!ai.base.already_turned);
    let [crate::ai::AiOwnerWork::StateChange(notification)] =
        ai.base.outbox.reentrant.owner_work.as_slice()
    else {
        panic!("goto-chief arrival must stage exactly one state change");
    };
    let effects = notification
        .actor_effects_before_callback
        .as_ref()
        .expect("facing must precede state change");
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
        ThinkEnv::new(
            &sim,
            &AiContext::test_fixture(),
            &AiPerTickData::stub(),
            None,
        ),
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
    )
    .unwrap();
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
        ThinkEnv::new(
            &sim,
            &AiContext::test_fixture(),
            &AiPerTickData::stub(),
            None,
        ),
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
    )
    .unwrap();
}

#[test]
fn charly_defence_completion_relays_talk_to_officer() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(55);
    ai.base.antagonist = Some(AiEntityHandle::new(90));
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
        ThinkEnv::new(
            &sim,
            &AiContext::test_fixture(),
            &AiPerTickData::stub(),
            None,
        ),
        StimulusType::EventMyTalk1,
    )
    .unwrap();
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
fn group_called_by_officer_moves_before_single_state_transition() {
    let mut ai = EnemyAi::new(53);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingGroupCalledByOfficer;
    ai.base.antagonist = Some(AiEntityHandle::new(60));
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
        ..AiContext::test_fixture()
    };

    ai.seeking_group_called_by_officer(StimulusType::EventTimer, &ctx, &AiPerTickData::stub());

    assert_eq!(ai.base.current_substate, Substate::SeekingGroupGoToOfficer);
    let [crate::ai::AiOwnerWork::StateChange(notification)] =
        ai.base.outbox.reentrant.owner_work.as_slice()
    else {
        panic!("group approach must publish exactly one state-change boundary");
    };
    let prefix = notification
        .actor_effects_before_callback
        .as_ref()
        .expect("Original movement precedes state change");
    assert_eq!(
        prefix.orders.len(),
        1,
        "movement belongs before state change"
    );
    assert!(
        prefix.set_attentive_mode.is_none() && prefix.additional_set_attentive_modes.is_empty(),
        "the raw movement request must not inject a same-state attentive request"
    );
    let requests = ai.base.outbox.actor.take_attentive_modes();
    assert_eq!(
        requests.len(),
        1,
        "only the explicit state change may request attention"
    );
    assert!(requests[0].target);
    assert!(!requests[0].fast_officer_variant);
}

#[test]
fn group_synchronous_reachpoint_same_direction_still_authors_turn() {
    let mut ai = EnemyAi::new(53);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingGroupGoToOfficer;
    // A non-waiting actor does not qualify for the original game's facing
    // same-direction shortcut, even when the already-at-destination movement
    // completion is surfaced recursively.
    ai.gather_position_instructed = true;
    ai.gather_direction = 8;
    let ctx = AiContext {
        self_action_state: crate::element::ActionState::Moving,
        direction: 8,
        ..AiContext::test_fixture()
    };

    ai.seeking_group_go_to_officer(
        ThinkEnv::new(
            &crate::sim_rng::test_context(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        ),
        StimulusType::EventReachPoint,
    )
    .unwrap();

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
    // Movement setup accepted an already-at-destination request without replacing
    // the actor's pre-existing Wait action. Facing therefore observes
    // Waiting while the recursive ReachPoint frame is still open.
    ai.gather_position_instructed = true;
    ai.gather_direction = 4;
    let ctx = AiContext {
        self_action_state: crate::element::ActionState::Waiting,
        direction: 4,
        ..AiContext::test_fixture()
    };

    ai.seeking_group_go_to_officer(
        ThinkEnv::new(
            &crate::sim_rng::test_context(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        ),
        StimulusType::EventReachPoint,
    )
    .unwrap();

    assert!(
        ai.base.outbox.actor.orders.is_empty(),
        "a completed movement already facing the gather direction must not turn"
    );
    assert!(
        ai.base.already_turned,
        "facing's shortcut must schedule synchronous completion"
    );
}

#[test]
fn group_reachpoint_keeps_raw_wrapped_gather_direction_for_face_to() {
    let mut ai = EnemyAi::new(66);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingGroupGoToOfficer;
    ai.gather_position_instructed = true;
    // The soldier-alert loop cursor can exceed 15. The original game's facing logic
    // compares this raw value before the Turn order projects it to a
    // direction sector, so 29 must not short-circuit against sector 13.
    ai.gather_direction = 29;
    let ctx = AiContext {
        self_action_state: crate::element::ActionState::Waiting,
        direction: 13,
        ..AiContext::test_fixture()
    };

    ai.seeking_group_go_to_officer(
        ThinkEnv::new(
            &crate::sim_rng::test_context(),
            &ctx,
            &AiPerTickData::stub(),
            None,
        ),
        StimulusType::EventReachPoint,
    )
    .unwrap();

    let [turn] = ai.base.outbox.actor.orders.as_slice() else {
        panic!("raw gather direction 29 must author one Turn against sector 13");
    };
    assert_eq!(turn.order_type, crate::order::OrderType::Turning);
    assert_eq!(turn.explicit_direction, Some(29));
    assert!(!ai.base.already_turned);
    assert_eq!(ai.base.current_substate, Substate::SeekingGroupGoToOfficer);
}

#[test]
fn charly_lecture_ignores_unrelated_stimulus() {
    let mut ai = EnemyAi::new(55);
    ai.base.antagonist = Some(AiEntityHandle::new(90));
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
        ThinkEnv::new(
            &crate::sim_rng::test_context(),
            &AiContext::test_fixture(),
            &AiPerTickData::stub(),
            None,
        ),
        StimulusType::EventDone,
    )
    .unwrap();
}

#[test]
#[should_panic(expected = "called soldier 55 requires officer 90 in the camp snapshot")]
fn called_soldier_requires_the_officer_snapshot() {
    let mut ai = EnemyAi::new(55);
    ai.base.antagonist = Some(AiEntityHandle::new(90));
    ai.seeking_soldier_called_by_officer(
        StimulusType::EventTimer,
        &AiContext::test_fixture(),
        &AiPerTickData::stub(),
    );
}
