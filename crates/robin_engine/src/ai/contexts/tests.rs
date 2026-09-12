use super::*;

#[test]
fn view_radius_cache_zero_replaces_alternating_viewers() {
    let first = crate::element::EntityId::from(crate::entity_id::SoldierId(7));
    let second = crate::element::EntityId::from(crate::entity_id::SoldierId(9));
    let ctx = AiContext::test_fixture();
    for surface in [None, crate::position_interface::ObstacleHandle::new(3)] {
        assert_eq!(
            ctx.compute_view_radius_cached(first, surface, || 125.0),
            125.0
        );
        assert_eq!(ctx.compute_view_radius_cached(second, surface, || 0.0), 0.0);
        // Zero is a miss even for its writer; A must also recompute after B.
        let recomputed = std::cell::Cell::new(false);
        assert_eq!(
            ctx.compute_view_radius_cached(second, surface, || {
                recomputed.set(true);
                0.0
            }),
            0.0
        );
        assert!(recomputed.get());
        assert_eq!(
            ctx.compute_view_radius_cached(first, surface, || 90.0),
            90.0
        );
        assert_eq!(
            ctx.compute_view_radius_cached(first, surface, || panic!("cache hit")),
            90.0
        );
    }
}

#[test]
fn view_radius_cache_zero_survives_seed_clone_absorb_and_publication() {
    let first = crate::element::EntityId::from(crate::entity_id::SoldierId(7));
    let second = crate::element::EntityId::from(crate::entity_id::SoldierId(9));
    let ctx = AiContext {
        frame: 40,
        ..AiContext::test_fixture()
    };
    let mut persistent = crate::ai_vision::ViewRadiusCache::default();
    let surfaces = [None, crate::position_interface::ObstacleHandle::new(3)];
    for surface in surfaces {
        persistent.set(surface, first, 40, 125.0);
    }
    ctx.seed_view_radius_cache(&persistent);
    let nested = ctx.clone();
    for surface in surfaces {
        assert_eq!(
            nested.compute_view_radius_cached(second, surface, || 0.0),
            0.0
        );
        // Clones stay isolated until the caller explicitly absorbs them.
        assert_eq!(
            ctx.compute_view_radius_cached(first, surface, || panic!("isolated parent")),
            125.0
        );
        assert_eq!(persistent.get(surface, first, 40), Some(125.0));
    }
    ctx.absorb_view_radius_cache(&nested);
    ctx.commit_view_radius_cache(&mut persistent);
    for surface in surfaces {
        assert_eq!(persistent.get(surface, first, 40), None);
        assert_eq!(persistent.get(surface, second, 40), None);
        assert_eq!(ctx.view_radius_cache.borrow()[&surface], (second, 0.0));
    }
    let next = AiContext {
        frame: 40,
        ..AiContext::test_fixture()
    };
    next.seed_view_radius_cache(&persistent);
    for surface in surfaces {
        assert_eq!(next.view_radius_cache.borrow()[&surface], (second, 0.0));
        assert_eq!(
            next.compute_view_radius_cached(second, surface, || 0.0),
            0.0
        );
        assert_eq!(
            next.compute_view_radius_cached(first, surface, || 90.0),
            90.0
        );
    }
    next.commit_view_radius_cache(&mut persistent);
    for surface in surfaces {
        assert_eq!(persistent.get(surface, first, 40), Some(90.0));
    }
    let later = AiContext {
        frame: 41,
        ..AiContext::test_fixture()
    };
    later.seed_view_radius_cache(&persistent);
    assert!(later.view_radius_cache.borrow().is_empty());
}

fn seek_point(x: f32) -> SeekPoint {
    SeekPoint {
        position: Position {
            x,
            ..Position::default()
        },
        frame_when_full_interest: 0,
        directions: Vec::new(),
        last_calculated_interest: 100,
        locked: false,
        id: 0,
    }
}

#[test]
fn near_seek_candidates_use_truncated_uword_distances() {
    let sim = crate::sim_rng::test_context();
    let mut global = AiGlobalState::default();
    global.seek_points.push(seek_point(9.1));
    let mut target = Position::default();
    let me = Position {
        x: 31.9,
        ..Position::default()
    };

    // A single-precision comparison would accept 9.1 < 31.9 * 0.3 (9.57).
    // Original narrows both sides first, so 9 < 9 is false.
    assert!(!global.set_pos_on_near_seek_point(&sim, me, &mut target, 0.3, 0));

    global.seek_points[0].position.x = 8.9;
    assert!(global.set_pos_on_near_seek_point(&sim, me, &mut target, 0.3, 0));
    assert_eq!(target.x, 8.9);
}

#[test]
fn near_seek_layer_penalty_wraps_as_uword() {
    let sim = crate::sim_rng::test_context();
    let mut global = AiGlobalState::default();
    let mut point = seek_point(65_500.0);
    point.position.level = 1;
    global.seek_points.push(point);
    let mut target = Position::default();

    // 16-bit 65500 + 100 wraps to 64 and is below the limit.
    assert!(global.set_pos_on_near_seek_point(&sim, Position::default(), &mut target, 0.0, 65,));
    assert_eq!(target.x, 65_500.0);
}

#[test]
fn initialized_soldier_camps_control_hostility_after_snapshots() {
    use crate::diplomacy::{DiplomacyDefinition, DiplomacyRule, DiplomacyState, Relationship};
    use crate::element::Camp;

    let ordinary = DiplomacyState::default();
    let neutral = DiplomacyState::from_definition(
        true,
        true,
        Some(&DiplomacyDefinition {
            player_coalition: vec![0],
            relationships: vec![DiplomacyRule {
                first: 0,
                second: 1,
                relationship: Relationship::Neutral,
            }],
        }),
    )
    .unwrap();
    let mut disabled = neutral.clone();
    disabled.set_npc_faction_wars(false);
    for (camps, expected_ordinary, expected_neutral) in [
        (vec![], false, false),
        (vec![Camp::Royalists], false, false),
        (vec![Camp::Lacklandists], false, false),
        (vec![Camp::Custom(2)], false, false),
        (vec![Camp::Royalists, Camp::Lacklandists], true, false),
        (vec![Camp::Custom(2), Camp::Custom(3)], true, true),
        (
            vec![Camp::Royalists, Camp::Lacklandists, Camp::Custom(2)],
            true,
            true,
        ),
    ] {
        let global = AiGlobalState {
            soldier_camps: camps.into_iter().collect(),
            ..Default::default()
        };
        let json = serde_json::to_value(&global).unwrap();
        assert!(json.get("there_are_royalist_soldiers").is_none());
        assert!(json.get("there_are_lacklandist_soldiers").is_none());
        let from_json: AiGlobalState = serde_json::from_value(json).unwrap();
        let from_native: AiGlobalState = bitcode::decode(&bitcode::encode(&global)).unwrap();
        for restored in [&global, &from_json, &from_native] {
            assert_eq!(restored.soldier_camps, global.soldier_camps);
            assert_eq!(
                robin_util::state_hash::compute(restored),
                robin_util::state_hash::compute(&global)
            );
            assert_eq!(restored.npcs_can_be_enemies(&ordinary), expected_ordinary);
            assert_eq!(restored.npcs_can_be_enemies(&neutral), expected_neutral);
            assert!(!restored.npcs_can_be_enemies(&disabled));
        }
    }
}
