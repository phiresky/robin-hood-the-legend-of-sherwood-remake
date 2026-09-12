use super::*;

#[test]
fn go_near_uses_live_actor_layer_while_door_position_is_snapped() {
    let mut ai = AiController::new(183);
    ai.think_recursion_depth = 1;
    let destination = Position {
        x: 296.64883,
        y: 1408.1284,
        sector: crate::position_interface::SectorHandle::new(99),
        level: 3,
    };
    let ctx = AiContext {
        // The actor's AI position has already snapped
        // across the door and is within the 30-unit tolerance.
        position: Position {
            x: 320.92307,
            y: 1423.2,
            sector: crate::position_interface::SectorHandle::new(99),
            level: 3,
        },
        // The actor itself is still physically on layer 2. Original's
        // separate location-level versus actor-layer gate fails.
        self_layer: 2,
        ..AiContext::test_fixture()
    };

    ai.go_near(destination, 30, GotoFlags::RUN, &ctx);

    assert!(!ai.already_on_point);
    assert_eq!(ai.outbox.actor.orders.len(), 1);
    assert_eq!(ai.outbox.actor.orders[0].target_layer, Some(3));
}

/// The original game dispatches its completion event before
/// decrementing the decision recursion depth, so a same-frame
/// completion cascade keeps every ancestor frame open and the depth
/// climbs one per nested decision until the 100.. return-to-duty failsafe.
/// The queued-dispatch port must therefore skip the decrement whenever a
/// completion event is queued and record the open frame for the engine
/// drain to close.
#[test]
fn end_think_keeps_frame_open_while_completion_event_is_queued() {
    let mut ai = AiController::new(17);

    // Queued completion: frame stays open, depth is preserved.
    ai.think_recursion_depth = 1;
    ai.already_on_point = true;
    assert!(ai.end_think_completion_events());
    assert_eq!(
        ai.outbox.reentrant.self_stimuli,
        [StimulusType::EventReachPoint]
    );
    assert_eq!(
        ai.think_recursion_depth, 1,
        "queuing decision completion must not unwind"
    );
    assert_eq!(ai.open_end_think_frames, 1);

    // A deeper cascade level stacks another open frame.
    ai.outbox.reentrant.self_stimuli.clear();
    ai.think_recursion_depth = 2;
    ai.already_turned = true;
    assert!(ai.end_think_completion_events());
    assert_eq!(ai.think_recursion_depth, 2);
    assert_eq!(ai.open_end_think_frames, 2);

    // Innermost Think (no latch): the whole chain of open ancestor
    // frames unwinds with it, like the original game's stacked tick completion
    // decrements while returning out of the recursion.
    ai.outbox.reentrant.self_stimuli.clear();
    ai.think_recursion_depth = 3;
    assert!(ai.end_think_completion_events());
    assert_eq!(ai.think_recursion_depth, 0);
    assert_eq!(ai.open_end_think_frames, 0);

    // 100..111 with a pending latch still reports the typed
    // return-to-duty fallback to the caller.
    ai.think_recursion_depth = 100;
    ai.already_on_point = true;
    assert!(!ai.end_think_completion_events());
}

#[test]
fn end_think_keeps_engine_deferred_goto_frame_open_until_authorized() {
    let mut ai = AiController::new(17);
    ai.think_recursion_depth = 1;
    ai.completion_latch_inside_think = true;
    ai.outbox.actor.orders.push(AiOrderIntent::new(
        crate::order::OrderType::RunningUpright,
        100.0,
        200.0,
    ));

    assert!(ai.end_think_completion_events());
    assert_eq!(ai.think_recursion_depth, 1);
    assert_eq!(ai.open_end_think_frames, 1);
    assert_eq!(ai.engine_deferred_end_think_frames, 1);

    // The engine has consumed the intent and authorized it without an
    // EVENT_* completion. Returning through the suspended Original
    // Tick completion closes the retained frame.
    ai.outbox.actor.orders.clear();
    ai.close_engine_deferred_end_think_frames();
    assert_eq!(ai.think_recursion_depth, 0);
    assert_eq!(ai.open_end_think_frames, 0);
    assert_eq!(ai.engine_deferred_end_think_frames, 0);
}

#[test]
fn panic_retry_side_uses_original_creation_order_parity() {
    assert_eq!(panic_retry_side(68), 12);
    assert_eq!(panic_retry_side(69), 4);

    let rust_entity_slot = 37;
    assert_ne!(
        panic_retry_side(68),
        panic_retry_side(rust_entity_slot),
        "trace owner creation-order 68 must not inherit entity-slot 37 parity",
    );
}

#[test]
fn repeated_checkpoint_charly_calls_preserve_immediate_original_order() {
    use crate::element::{DetectableType, EntityId};
    use crate::entity_id::SoldierId;

    let mut cleared = AiController::new(17);
    cleared.set_checkpoint_charly(Some(AiEntityHandle::new(10)));
    cleared.set_checkpoint_charly(None);
    assert_eq!(
        cleared.outbox.actor.detectable_mutations,
        vec![
            DetectableMutation::DeleteType(DetectableType::MissedFriend),
            DetectableMutation::Add(
                EntityId::Soldier(SoldierId(10)),
                DetectableType::MissedFriend
            ),
            DetectableMutation::DeleteType(DetectableType::MissedFriend),
            DetectableMutation::DeleteType(DetectableType::MissedFriend),
        ]
    );

    let mut replaced = AiController::new(17);
    replaced.set_checkpoint_charly(Some(AiEntityHandle::new(10)));
    replaced.set_checkpoint_charly(Some(AiEntityHandle::new(11)));
    assert_eq!(
        replaced.outbox.actor.detectable_mutations,
        vec![
            DetectableMutation::DeleteType(DetectableType::MissedFriend),
            DetectableMutation::Add(
                EntityId::Soldier(SoldierId(10)),
                DetectableType::MissedFriend
            ),
            DetectableMutation::DeleteType(DetectableType::MissedFriend),
            DetectableMutation::Add(
                EntityId::Soldier(SoldierId(11)),
                DetectableType::MissedFriend
            ),
        ]
    );
}

#[test]
fn consider_report_charly_preserves_release_build_detectable_append() {
    use crate::element::{DetectableType, EntityId};
    use crate::entity_id::SoldierId;

    let mut ai = AiController::new(17);
    let report = ReconnaissanceReport {
        charly: Some(AiEntityHandle::new(91)),
        ..Default::default()
    };

    ai.consider_report_merged(
        &report,
        crate::ai_enemy::ReportUpdateFlags::UPDATE_CHARLY.bits(),
        &crate::ai_entity_view::AiEntityViewMap::new(),
    );

    assert!(ai.outbox.actor.added_detectables().is_empty());
    assert_eq!(
        ai.outbox.actor.appended_detectables(),
        vec![(
            EntityId::Soldier(SoldierId(91)),
            DetectableType::MissedFriend,
        )],
        "detectable insertion must append even if live storage already has Charly"
    );
}

#[test]
fn relative_synchronize_indices_narrow_like_original_uword() {
    assert_eq!(resolve_synchronize_index(7, 500), 500);
    assert_eq!(resolve_synchronize_index(3, 1002), 5);
    assert_eq!(resolve_synchronize_index(1, 998), u16::MAX);
    assert_eq!(resolve_synchronize_index(0, u16::MAX), 64_535);
}

#[test]
fn friend_check_look_count_narrows_instead_of_saturating() {
    assert_eq!(friend_check_look_count(254, 1), 255);
    assert_eq!(friend_check_look_count(255, 1), 0);
}

#[test]
fn fleeing_seek_distance_uses_original_uword_wrap_and_sentinel() {
    let point = |x, sector| SeekPoint {
        position: Position {
            x,
            sector,
            ..Position::default()
        },
        frame_when_full_interest: 0,
        directions: Vec::new(),
        last_calculated_interest: 100,
        locked: false,
        id: 0,
    };
    let ai = AiController::new(17);

    // The minimum distance begins at 0xffff and the comparison is strict.
    assert_eq!(
        ai.nearest_seek_point_to_flee(&[point(65_535.0, None)], Position::default(), None,),
        None
    );

    // The +1000 sector penalty uses 16-bit arithmetic: 65000 + 1000
    // wraps to 464, making the far cross-sector point win here.
    assert_eq!(
        ai.nearest_seek_point_to_flee(
            &[point(65_000.0, SectorHandle::new(1)), point(1_000.0, None),],
            Position::default(),
            None,
        ),
        Some(0)
    );
}

#[test]
fn direct_cross_npc_say_is_drained_synchronously() {
    let mut ai = AiController::new(17);
    ai.outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::Say {
            target: 23,
            remark: Remark::ArchersBehindShieldBearers,
        });

    let actions = ai.take_pending_synchronous_cross_npc_actions();

    assert!(matches!(
        actions.as_slice(),
        [CrossNpcAction::Say {
            target: 23,
            remark: Remark::ArchersBehindShieldBearers
        }]
    ));
    assert!(ai.outbox.reentrant.cross_npc_actions.is_empty());
}

#[test]
fn synchronizer_registration_is_drained_before_later_owner_slots() {
    // The original game mutates the synchronization target inline. The
    // target can reach its waypoint in a later element update slot in
    // this same frame and must already see the waiter in its list.
    let mut ai = AiController::new(89);
    ai.outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::RegisterSynchronizingActor {
            target: 96,
            actor: 89,
        });

    assert!(ai.has_pending_synchronous_cross_npc_actions());
    let actions = ai.take_pending_synchronous_cross_npc_actions();

    assert!(matches!(
        actions.as_slice(),
        [CrossNpcAction::RegisterSynchronizingActor {
            target: 96,
            actor: 89
        }]
    ));
    assert!(ai.outbox.reentrant.cross_npc_actions.is_empty());
}

#[test]
fn reciprocal_combat_neighbour_updates_are_drained_synchronously() {
    let mut ai = AiController::new(17);
    ai.outbox.reentrant.cross_npc_actions.extend([
        CrossNpcAction::UpdateLeftCombatNeighbour {
            target: 17,
            old_left: None,
            new_left: Some(AiEntityHandle::new(23)),
        },
        CrossNpcAction::UpdateRightCombatNeighbour {
            target: 17,
            old_right: None,
            new_right: Some(AiEntityHandle::new(29)),
        },
    ]);

    let actions = ai.take_pending_synchronous_cross_npc_actions();

    assert!(matches!(
        actions.as_slice(),
        [
            CrossNpcAction::UpdateLeftCombatNeighbour {
                target: 17,
                old_left: None,
                new_left: Some(new_left),
            },
            CrossNpcAction::UpdateRightCombatNeighbour {
                target: 17,
                old_right: None,
                new_right: Some(new_right),
            }
        ] if new_left.get() == 23 && new_right.get() == 29
    ));
    assert!(ai.outbox.reentrant.cross_npc_actions.is_empty());
}

#[test]
fn direct_combat_neighbour_setters_are_drained_synchronously() {
    let mut ai = AiController::new(17);
    ai.outbox.reentrant.cross_npc_actions.extend([
        CrossNpcAction::SetLeftCombatNeighbour {
            target: 23,
            neighbour: None,
        },
        CrossNpcAction::SetRightCombatNeighbour {
            target: 29,
            neighbour: None,
        },
    ]);

    assert!(ai.has_pending_synchronous_cross_npc_actions());
    let actions = ai.take_pending_synchronous_cross_npc_actions();

    assert!(matches!(
        actions.as_slice(),
        [
            CrossNpcAction::SetLeftCombatNeighbour {
                target: 23,
                neighbour: None,
            },
            CrossNpcAction::SetRightCombatNeighbour {
                target: 29,
                neighbour: None,
            }
        ]
    ));
    assert!(ai.outbox.reentrant.cross_npc_actions.is_empty());
}

#[test]
fn reciprocal_archer_shield_setters_are_drained_synchronously() {
    // The original game's archer and shield-bearer updates mutate
    // the reciprocal AI object inline. A later owner in the same element
    // refresh pass must therefore see the updated relationship.
    let mut ai = AiController::new(81);
    ai.outbox.reentrant.cross_npc_actions.extend([
        CrossNpcAction::SetShieldBearerBeforeMe {
            target: 86,
            shield_bearer: None,
        },
        CrossNpcAction::SetArcherBehindMe {
            target: 81,
            archer: None,
        },
    ]);

    assert!(ai.has_pending_synchronous_cross_npc_actions());
    let actions = ai.take_pending_synchronous_cross_npc_actions();

    assert!(matches!(
        actions.as_slice(),
        [
            CrossNpcAction::SetShieldBearerBeforeMe {
                target: 86,
                shield_bearer: None,
            },
            CrossNpcAction::SetArcherBehindMe {
                target: 81,
                archer: None,
            }
        ]
    ));
    assert!(ai.outbox.reentrant.cross_npc_actions.is_empty());
}

#[test]
fn raise_shield_records_the_targets_world_ground_point() {
    use crate::element::EntityId;
    use crate::entity_id::EntityIdKind;
    use crate::sequence::{Field, FieldValue};

    let mut ai = AiController::new(17);
    ai.owner_entity_id = Some(EntityId::new(17, EntityIdKind::Soldier));
    ai.raise_shield(
        Position {
            x: 1083.0,
            y: 1563.0,
            ..Position::default()
        },
        160.0,
    );

    let element = ai.outbox.actor.launch_sequences[0]
        .elements
        .first()
        .expect("RaiseShield sequence contains its command");
    assert!(matches!(
        element.get_property(Field::ShieldDangerPoint),
        Some(FieldValue::Point3D {
            x: 1083.0,
            y: 1723.0,
            z: 160.0,
        })
    ));
}

#[test]
fn lower_shield_requests_the_explicit_command() {
    let mut ai = AiController::new(17);

    ai.lower_shield();

    assert!(ai.outbox.actor.lower_shield);
    assert!(
        ai.outbox.actor.orders.is_empty(),
        "LowerShield must not be flattened into a Generic animation order"
    );
}
