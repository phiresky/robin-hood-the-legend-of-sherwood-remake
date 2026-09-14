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
