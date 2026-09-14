use super::*;

#[test]
fn approach_tolerance_requires_the_actor_layer_even_when_ai_position_is_snapped() {
    let destination = Position {
        x: 296.64883,
        y: 1408.1284,
        sector: SectorHandle::new(99),
        level: 3,
    };
    let snapped = Position {
        x: 320.92307,
        y: 1423.2,
        sector: SectorHandle::new(99),
        level: 3,
    };
    for actor_layer in [2, 3] {
        let mut ai = AiController::new(183);
        ai.prepare_approach(30, GotoFlags::RUN, 1);
        let admitted = ai.prepare_move_request(
            destination,
            GotoFlags::RUN | GotoFlags::NEAR,
            snapped,
            actor_layer,
            SectorHandle::new(98),
            crate::order::OrderType::WalkingUpright,
            false,
            1,
        );
        assert_eq!(admitted.is_some(), actor_layer == 2);
        assert_eq!(ai.already_on_point, actor_layer == 3);
    }
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
