use super::*;

fn context() -> AiContext {
    AiContext {
        posture: crate::element::Posture::Upright,
        elevation: 105.001_01,
        ..AiContext::test_fixture()
    }
}

#[test]
fn door_transition_uses_literal_live_ground_position() {
    // Savegame_021 seed replay: the projected map coordinates differ by
    // only (5, 8), but elevation projection puts the target more than
    // seven world-Y units away while it is less than one unit lower.
    let target = crate::coordinates::WorldPoint3D::new(577.0, 2_472.219, 104.218_95);
    assert!(!enemy_is_below_me(
        &context(),
        Some(Position {
            x: 572.0,
            y: 2360.0,
            ..Position::default()
        }),
        Some(target),
    ));
}

#[test]
fn nearby_lower_target_is_below() {
    let target = crate::coordinates::WorldPoint3D::new(572.0, 2465.001, 104.0);
    assert!(enemy_is_below_me(
        &context(),
        Some(Position {
            x: 572.0,
            y: 2360.0,
            ..Position::default()
        }),
        Some(target),
    ));
}

#[test]
fn moving_archer_uses_post_execute_live_position_at_detection_boundary() {
    // schema14 seed1000000, linux3/P003/Savegame_043/replay-004,
    // frame 8310. Soldier 88's own Execute has advanced five map-Y units
    // before the NPC update checks whether its enemy is below. That movement is just
    // enough to put PC 169 inside the vertical-distance cone and select
    // EquipBowDown at frame 8320.
    let ctx = AiContext {
        posture: crate::element::Posture::Upright,
        elevation: 150.001,
        ..AiContext::test_fixture()
    };
    let target = crate::coordinates::WorldPoint3D::new(1_031.413_8, 2_002.76, 107.499_275);
    let owner_after_execute = Position {
        x: 1_061.203_9,
        y: 1_865.277_8,
        ..Position::default()
    };
    assert!(enemy_is_below_me(
        &ctx,
        Some(owner_after_execute),
        Some(target),
    ));

    let owner_before_execute = Position {
        y: owner_after_execute.y + 5.0,
        ..owner_after_execute
    };
    assert!(
        !enemy_is_below_me(&ctx, Some(owner_before_execute), Some(target)),
        "the tick-start position is across the exact Original cone boundary"
    );
}

#[test]
#[should_panic(expected = "target's literal live position")]
fn missing_required_target_geometry_is_not_fake_not_below() {
    let _ = enemy_is_below_me(&context(), Some(Position::default()), None);
}
