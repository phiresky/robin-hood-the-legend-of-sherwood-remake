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
}
