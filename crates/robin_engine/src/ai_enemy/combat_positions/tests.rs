use super::*;
use crate::element::{EntityId, Posture, SoldierId};
use crate::sight_obstacle::{ObstaclePoint, SharedSightObstacles, SightObstacle};
use crate::sim_rng::{RngSite, SimulationContext, with_draw_trace};

fn position(x: f32, y: f32) -> Position {
    Position {
        x,
        y,
        sector: None,
        level: 0,
    }
}

#[test]
fn reconsider_phalanx_attack_gate_uses_literal_body_positions() {
    const OWNER: u32 = 169;
    const TARGET: u32 = 295;

    let target = crate::element::Entity::Pc(crate::element::ActorPc {
        element: {
            let mut initial_element =
                crate::element::ElementData::from_initial_posture(crate::element::Posture::Upright);
            initial_element.kind = crate::element::ElementKind::ActorPc;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    let mut target_view = crate::ai_entity_view::entity_view_from_entity(
        &target,
        295,
        false,
        None,
        None,
        crate::order::OrderType::NonanimationEnd,
    );
    // Savegame_033 replay-015 frame 11767: the AI-facing point is less
    // than 100 units away, while the literal actor bodies are farther
    // apart. The original game's squared-distance calculation reads the position and keeps the
    // formation; using the AI point recursively broke all four members.
    target_view.position = position(90.0, 0.0);
    target_view.detection_position_world = crate::coordinates::WorldPoint3D::new(120.0, 0.0, 0.0);
    target_view.active = true;
    target_view.camp = crate::element::Camp::Royalists;
    target_view.is_able_to_fight = true;
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(TARGET, target_view);

    let mut ai = EnemyAi::new(OWNER);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingPhalanx;
    ai.list_them.push(TARGET);
    ai.left_combat_neighbour = Some(AiEntityHandle::new(170));

    let ctx = AiContext {
        position: position(0.0, 0.0),
        self_body_position_world: crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0),
        camp: crate::element::Camp::Lacklandists,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.push(FighterSnapshot {
        handle: TARGET,
        position: position(90.0, 0.0),
        raw_position: position(120.0, 0.0),
        is_friendly: false,
        is_able_to_fight: true,
        ..FighterSnapshot::default()
    });

    assert!(!ai.reconsider_phalanx(&SimulationContext::with_seed(0), &ctx, &tick, None));
    assert_eq!(ai.base.current_substate, Substate::AttackingPhalanx);
    assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
}

#[test]
fn vector_derived_phalanx_slot_retains_anchor_sector_identity() {
    use crate::fast_find_grid::SectorIndex;

    let current = Position {
        x: 20.0,
        y: 20.0,
        sector: crate::position_interface::SectorHandle::new(18)
            .map(|sector| sector.with_arena_index(SectorIndex::new(40).unwrap())),
        level: 0,
    };
    let anchor = Position {
        x: 80.0,
        y: 20.0,
        sector: crate::position_interface::SectorHandle::new(18)
            .map(|sector| sector.with_arena_index(SectorIndex::new(41).unwrap())),
        level: 0,
    };
    let derived = Position {
        x: anchor.x - 20.0,
        y: anchor.y,
        ..anchor
    };

    assert_eq!(derived.sector.unwrap().arena_index(), SectorIndex::new(41));
    assert!(inherited_position_crosses_sector_identity(
        &current, &derived
    ));
    assert!(!inherited_position_crosses_sector_identity(
        &anchor, &derived
    ));
}

#[test]
fn nescafe_phalanx_uses_raw_body_distance_then_ai_facing_chain_anchors() {
    use crate::position_interface::SectorHandle;

    // Schema-16 seed 1,000,000, Nescafe Restart, frame 1187. Original
    // Soldier co160 (Rust 129) is passing door 95. AI `Position()` snaps
    // it to the sector-0 endpoint (1307,2245), but its raw body is still
    // near (1306.8123,2262.1873). The original game's maximum-norm distance reads the raw
    // body and gets 43, so co160 beats co163/Rust132 at distance 58.
    // Measuring the snapped point changes the ordering. After selection,
    // Phalanx-position selection deliberately returns to AI-facing current/seek
    // positions and derives the slot from the sector-0 chain end.
    let sector_18 = SectorHandle::new(18).unwrap();
    let sector_0 = SectorHandle::new(0).unwrap();
    let mut ai = EnemyAi::new(128);
    ai.base.me = 128;
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.push(FighterSnapshot {
        handle: 128,
        is_friendly: true,
        is_shield_bearer: true,
        position: Position {
            x: 1263.1832,
            y: 2281.7712,
            sector: Some(sector_18),
            level: 0,
        },
        raw_position: Position {
            x: 1263.1832,
            y: 2281.7712,
            sector: Some(sector_18),
            level: 0,
        },
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.push(FighterSnapshot {
        handle: 132,
        is_friendly: true,
        is_shield_bearer: true,
        current_substate: Substate::AttackingProtectingWithShield,
        left_combat_neighbour: Some(AiEntityHandle::new(130)),
        right_combat_neighbour: Some(AiEntityHandle::new(129)),
        position: Position {
            x: 1322.0,
            y: 2276.0,
            sector: Some(sector_18),
            level: 0,
        },
        raw_position: Position {
            x: 1322.0,
            y: 2276.0,
            sector: Some(sector_18),
            level: 0,
        },
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.extend([
        FighterSnapshot {
            handle: 129,
            is_friendly: true,
            is_shield_bearer: true,
            current_substate: Substate::AttackingRunningToPhalanx,
            left_combat_neighbour: Some(AiEntityHandle::new(133)),
            shield_bearer_direction: 8,
            position: Position {
                x: 1307.0,
                y: 2245.0,
                sector: Some(sector_0),
                level: 0,
            },
            raw_position: Position {
                x: 1306.8123,
                y: 2262.1873,
                sector: Some(sector_18),
                level: 0,
            },
            shield_bearer_seek_position: Position {
                x: 1310.3472,
                y: 2209.295,
                sector: Some(sector_0),
                level: 0,
            },
            ..FighterSnapshot::default()
        },
        FighterSnapshot {
            handle: 133,
            is_friendly: true,
            is_shield_bearer: true,
            current_substate: Substate::AttackingPhalanx,
            left_combat_neighbour: Some(AiEntityHandle::new(130)),
            right_combat_neighbour: Some(AiEntityHandle::new(129)),
            position: Position {
                x: 1337.8904,
                y: 2213.9985,
                sector: Some(sector_0),
                level: 0,
            },
            raw_position: Position {
                x: 1337.8904,
                y: 2213.9985,
                sector: Some(sector_0),
                level: 0,
            },
            direction: 10,
            ..FighterSnapshot::default()
        },
        FighterSnapshot {
            handle: 130,
            is_friendly: true,
            is_shield_bearer: true,
            current_substate: Substate::AttackingPhalanx,
            right_combat_neighbour: Some(AiEntityHandle::new(133)),
            position: Position {
                x: 1359.2433,
                y: 2211.426,
                sector: Some(sector_0),
                level: 0,
            },
            raw_position: Position {
                x: 1359.2433,
                y: 2211.426,
                sector: Some(sector_0),
                level: 0,
            },
            direction: 10,
            ..FighterSnapshot::default()
        },
    ]);
    let ctx = AiContext {
        position: tick.fighter_registry[0].position,
        ..AiContext::test_fixture()
    };

    assert_eq!(ai.get_nearest_free_shield_bearer(&ctx, &tick), Some(129));
    let (slot, _, left, right, crosses_sector) = ai
        .find_phalanx_place(&ctx, &tick, None)
        .expect("nearby protecting shield bearer provides a slot");

    assert_eq!(slot.sector, Some(sector_0));
    assert!(crosses_sector);
    // With no grid fixture both slots are authorized, so this focused
    // control chooses the closer right slot. Crucially it is derived from
    // co160's future seek anchor, not its raw body or door endpoint.
    assert_eq!((left, right), (Some(AiEntityHandle::new(129)), None));
    assert_eq!(slot.x.to_bits(), 1285.3472_f32.to_bits());
    assert_eq!(slot.y.to_bits(), 2209.295_f32.to_bits());
}

#[test]
fn shield_danger_point_uses_raw_target_position_during_door_pass() {
    // Schema-14 seed 1000000, SuN1Sh1nE Profile_004/Savegame_013
    // replay-008 frame 1173. PC 171's AI Position() is the door endpoint,
    // but arrow-protection refresh stores the PC element's raw position in
    // shield-danger point. The two points face different sectors.
    let target = FighterSnapshot {
        handle: 171,
        position: position(572.0, 2360.0),
        raw_position: position(578.74, 2388.01),
        elevation: 85.44939,
        ..FighterSnapshot::default()
    };

    let (danger_position, danger_elevation) =
        shield_danger_point(Some(&target), None).expect("fighter has a raw position");

    assert_eq!(danger_position, target.raw_position);
    assert_eq!(danger_elevation, target.elevation);
    assert_ne!(danger_position, target.position);
}

#[test]
fn combat_neighbour_ranking_uses_literal_body_position_during_door_pass() {
    // Schema-16 seed 2,000,000, linux2/Profile_002/Savegame_003,
    // replay-037 frame 15008. Soldier 130's AI Position() is the nearby
    // door endpoint (1173, 1849), while its literal body remains at
    // (1159.50, 1829.36). AI squared-distance uses
    // the latter, making Soldier 178 the nearest right neighbour.
    let mut ai = EnemyAi::new(186);
    ai.base.list_us = vec![186, 130, 178];

    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.extend([
        FighterSnapshot {
            handle: 130,
            position: position(1173.0, 1849.0),
            raw_position: position(1159.4979, 1829.3608),
            elevation: 1.4621211,
            direction: 7,
            is_soldier: true,
            rank: ProfileRank::Soldier,
            ..FighterSnapshot::default()
        },
        FighterSnapshot {
            handle: 178,
            position: position(1220.2673, 1886.527),
            raw_position: position(1220.2673, 1886.527),
            direction: 8,
            is_soldier: true,
            rank: ProfileRank::Soldier,
            ..FighterSnapshot::default()
        },
    ]);
    let ctx = AiContext {
        position: position(1231.5779, 1845.2806),
        direction: 8,
        ..AiContext::test_fixture()
    };

    assert_eq!(
        ai.propose_left_and_right_neighbour(&ctx, &tick),
        (None, Some(AiEntityHandle::new(178)))
    );
}

#[test]
fn combat_neighbour_ranking_truncates_distance_before_tie_breaking() {
    // Schema-16 seed 3,000,000, linux2/Profile_002/Savegame_029,
    // replay-009 frame 5282. Soldier 64 is closer than Soldier 55 by less
    // than one squared-distance unit (151.55 versus 151.99), but Original
    // narrows both results to unsigned 32-bit 151 and keeps Soldier 55 because it
    // appears first in the ally list.
    let mut ai = EnemyAi::new(113);
    ai.base.list_us = vec![113, 55, 64];

    let soldier_55 = FighterSnapshot {
        handle: 55,
        position: position(420.24728, 1758.1467),
        raw_position: position(420.24728, 1758.1467),
        elevation: 4.186_21,
        direction: 12,
        is_soldier: true,
        rank: ProfileRank::Soldier,
        ..FighterSnapshot::default()
    };
    let soldier_64 = FighterSnapshot {
        handle: 64,
        position: position(420.18027, 1757.8624),
        raw_position: position(420.18027, 1757.8624),
        elevation: 4.382771,
        direction: 12,
        is_soldier: true,
        rank: ProfileRank::Soldier,
        ..FighterSnapshot::default()
    };
    let ctx = AiContext {
        position: position(431.41672, 1755.0808),
        elevation: 4.6533546,
        direction: 13,
        ..AiContext::test_fixture()
    };
    let distance_55 = ai_square_distance(
        &soldier_55.raw_position,
        soldier_55.elevation,
        &ctx.position,
        ctx.elevation,
    );
    let distance_64 = ai_square_distance(
        &soldier_64.raw_position,
        soldier_64.elevation,
        &ctx.position,
        ctx.elevation,
    );
    assert!(distance_64 < distance_55);
    assert_eq!(distance_55 as u32, distance_64 as u32);

    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.extend([soldier_55, soldier_64]);

    assert_eq!(
        ai.propose_left_and_right_neighbour(&ctx, &tick),
        (Some(AiEntityHandle::new(55)), None)
    );
}

#[test]
fn combat_neighbour_distance_ulong_truncates_valid_geometry() {
    assert_eq!(combat_neighbour_distance_ulong(151.99), 151);
    let largest_below_two_to_32 = f32::from_bits(0x4f7f_ffff);
    assert_eq!(
        combat_neighbour_distance_ulong(largest_below_two_to_32),
        4_294_967_040
    );
}

#[test]
#[should_panic(expected = "outside the original-game 32-bit unsigned domain")]
fn combat_neighbour_distance_ulong_rejects_invalid_geometry() {
    let _ = combat_neighbour_distance_ulong(f32::NAN);
}

#[test]
#[should_panic(expected = "outside the original-game 32-bit unsigned domain")]
fn combat_neighbour_distance_ulong_rejects_negative_geometry() {
    let _ = combat_neighbour_distance_ulong(-1.0);
}

#[test]
#[should_panic(expected = "outside the original-game 32-bit unsigned domain")]
fn combat_neighbour_distance_ulong_rejects_infinite_geometry() {
    let _ = combat_neighbour_distance_ulong(f32::INFINITY);
}

#[test]
#[should_panic(expected = "outside the original-game 32-bit unsigned domain")]
fn combat_neighbour_distance_ulong_rejects_two_to_32() {
    let _ = combat_neighbour_distance_ulong(4_294_967_296.0_f32);
}

#[test]
fn phalanx_nearest_enemy_truncates_distance_before_tie_breaking() {
    assert_eq!(
        nearest_phalanx_enemy_index([(0, 120.9), (1, 120.1), (2, 121.0)]),
        Some(0),
        "Original-game 16-bit narrowing keeps the first enemy within a shared integer bucket"
    );
    assert_eq!(
        nearest_phalanx_enemy_index([(0, 120.9), (1, 119.9)]),
        Some(1)
    );
}

#[test]
fn already_in_cover_position_does_not_require_reachability() {
    // nicouzouf Savegame_010 replay-012 frame 515: the archer is already
    // behind Soldier 58. The direct cover corridor is obstructed, but
    // Original only compares this ideal offset with the archer's current
    // position and therefore keeps the relationship while shooting.
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.push(FighterSnapshot {
        handle: 58,
        position: position(1144.9557, 408.22668),
        direction: 7,
        current_substate: Substate::AttackingProtectingWithShield,
        ..FighterSnapshot::default()
    });
    let archer_position = position(1123.7424, 396.0593);
    let cover = EnemyAi::default()
        .shield_bearer_cover_position(58, &tick)
        .expect("linked shield bearer has an ideal cover position");

    assert!(max_norm(pos_diff(&archer_position, &cover)) < archer::COVER_POINT_TOLERANCE as f32);
}

#[test]
fn shield_bearer_cover_preserves_original_aspect_then_distance_rounding() {
    // Schema-16 seed 2,000,000, linux3/Profile_003/Savegame_029,
    // replay-017 frame 12826. The original game first multiplies
    // sector 10's Y component by ASPECT_RATIO, then operator*= applies
    // distance 30. Reassociating those products raises the destination Y
    // from bits 0x4268_b9b6 to 0x4268_b9b7 and eventually changes a
    // bit-exact visibility endpoint by two ULPs.
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.push(FighterSnapshot {
        handle: 82,
        position: position(1_072.624_8, 70.348_755),
        direction: 10,
        current_substate: Substate::AttackingProtectingWithShield,
        ..FighterSnapshot::default()
    });

    let cover = EnemyAi::default()
        .shield_bearer_cover_position(82, &tick)
        .expect("protecting shield bearer has a cover position");

    assert_eq!(cover.x.to_bits(), 0x4488_bad1);
    assert_eq!(cover.y.to_bits(), 0x4268_b9b6);
    assert_eq!(cover.x, 1_093.838);
    assert_eq!(cover.y, 58.181_36);
}

#[test]
fn nearest_shield_bearer_includes_inactive_running_to_phalanx_soldier() {
    // nicouzouf Profile_001 Savegame_045 replay-014 frame 1054. Soldier
    // 62 is inactive/script-locked while running to its phalanx slot, but
    // Original's global soldier scan still chooses it over Soldier 60.
    let ai = EnemyAi::new(66);
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.push(FighterSnapshot {
        handle: 66,
        raw_position: position(549.00867, 517.99506),
        is_friendly: true,
        is_archer_unit: true,
        ..FighterSnapshot::default()
    });
    let active_bearer = FighterSnapshot {
        handle: 60,
        position: position(669.8923, 746.43475),
        raw_position: position(669.8923, 746.43475),
        is_friendly: true,
        is_able_to_fight: true,
        is_shield_bearer: true,
        current_substate: Substate::AttackingPhalanx,
        ..FighterSnapshot::default()
    };
    let inactive_bearer = FighterSnapshot {
        handle: 62,
        position: position(484.0, 701.0),
        raw_position: position(484.0, 701.0),
        is_friendly: true,
        is_able_to_fight: false,
        is_shield_bearer: true,
        current_substate: Substate::AttackingRunningToPhalanx,
        ..FighterSnapshot::default()
    };
    tick.fighter_registry.push(active_bearer.clone());
    tick.fighter_registry.push(inactive_bearer);
    // The radius-limited/able-only list omits Soldier 62, which is why it
    // cannot be the backing collection for this Original global scan.
    tick.nearby_fighters.push(active_bearer);
    let ctx = AiContext {
        position: position(549.00867, 517.99506),
        ..AiContext::test_fixture()
    };

    assert_eq!(ai.get_nearest_free_shield_bearer(&ctx, &tick), Some(62));
}

#[test]
fn arrow_protection_counts_inactive_seeking_orphan_from_complete_registry() {
    // linux3 Profile_003 Savegame_071 replay-012 frame 5208: Soldiers
    // 250..255 are inactive but remain Seeking orphan archers within the
    // Original 500-unit camp-soldier scan. The swordfight-oriented nearby
    // cache excludes them through combat readiness and must not define this
    // decision's candidate domain.
    let ai = EnemyAi::new(219);
    let ctx = AiContext {
        position: position(1_520.0, 900.0),
        ..AiContext::test_fixture()
    };
    let orphan = FighterSnapshot {
        handle: 250,
        position: position(1_328.0, 1_033.0),
        raw_position: position(1_328.0, 1_033.0),
        elevation: 0.0,
        is_friendly: true,
        is_able_to_fight: false,
        is_archer_unit: true,
        ai_state: AiState::Seeking,
        shield_bearer_before_me: None,
        is_tower_guard: false,
        ..FighterSnapshot::default()
    };
    let mut tick = AiPerTickData::stub();
    // The registry always leads with the scanning soldier's own entry;
    // Squared distance measures from its raw element position.
    tick.fighter_registry.push(FighterSnapshot {
        handle: 219,
        position: position(1_520.0, 900.0),
        raw_position: position(1_520.0, 900.0),
        elevation: 0.0,
        is_friendly: true,
        ai_state: AiState::Attacking,
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.push(orphan.clone());
    assert_eq!(
        ai.number_of_nearby_archers_who_need_protection(&ctx, &tick),
        1,
        "inactive is not an Original admission gate"
    );

    tick.fighter_registry[1].ai_state = AiState::Default;
    assert_eq!(
        ai.number_of_nearby_archers_who_need_protection(&ctx, &tick),
        0,
        "the explicit AI-state gate still rejects an inactive Default archer"
    );

    tick.fighter_registry[1] = FighterSnapshot {
        position: position(2_100.0, 900.0),
        raw_position: position(2_100.0, 900.0),
        ..orphan
    };
    assert_eq!(
        ai.number_of_nearby_archers_who_need_protection(&ctx, &tick),
        0,
        "the complete registry must still obey the strict 500-unit radius"
    );
}

#[test]
fn arrow_protection_sees_reciprocal_unlink_emitted_by_same_think() {
    // Schema-12 SuN1Sh1nE Savegame_013 replay-005 frame 2165. Breaking
    // the phalanx makes Soldier 81 choose Observe. The original game's state change
    // synchronously unlinks archer 86 before enemy approach reconsideration
    // scans for archers needing protection, so 86 is already orphaned.
    // Rust's reciprocal write is queued until the owner-boundary drain;
    // this tactical scan must overlay that ordered write.
    let mut ai = EnemyAi::new(81);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingPhalanx;
    ai.archer_behind_me = Some(AiEntityHandle::new(86));
    ai.set_state(AiState::Attacking, Substate::AttackingApproachToObserve);

    assert_eq!(ai.archer_behind_me, None);
    assert!(
        ai.base
            .outbox
            .reentrant
            .cross_npc_actions
            .iter()
            .any(|action| matches!(
                action,
                CrossNpcAction::SetShieldBearerBeforeMe {
                    target: 86,
                    shield_bearer: None
                }
            ))
    );

    let owner_position = position(900.0, 2500.0);
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.push(FighterSnapshot {
        handle: 81,
        position: owner_position,
        raw_position: owner_position,
        is_friendly: true,
        is_shield_bearer: true,
        ai_state: AiState::Attacking,
        current_substate: Substate::AttackingPhalanx,
        archer_behind_me: Some(AiEntityHandle::new(86)),
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.push(FighterSnapshot {
        handle: 86,
        position: position(920.0, 2510.0),
        raw_position: position(920.0, 2510.0),
        is_friendly: true,
        is_archer_unit: true,
        ai_state: AiState::Attacking,
        shield_bearer_before_me: Some(AiEntityHandle::new(81)),
        ..FighterSnapshot::default()
    });
    let ctx = AiContext {
        position: owner_position,
        ..AiContext::test_fixture()
    };

    assert_eq!(
        ai.number_of_nearby_archers_who_need_protection(&ctx, &tick),
        1
    );
}

#[test]
fn phalanx_advance_uses_original_aspect_aware_normalization() {
    // Schema-14 Savegame_034/replay-009, frame 34182. Soldier 52 is
    // the center of the three-man phalanx and PC 167 is its target.
    // These are the literal normalized-vector results and
    // the aspect-adjusted normal calculation in the original game.
    let center = position(720.15155, 2198.4492);
    let target = position(967.95605, 2068.5835);
    let (forward, right) = phalanx_advance_vectors(pos_diff(&target, &center));

    assert!((forward.0 - 51.67761).abs() < 0.0001);
    assert!((forward.1 - -27.082436).abs() < 0.0001);
    assert!((right.0 - 16.863138).abs() < 0.0001);
    assert!((right.1 - 10.586092).abs() < 0.0001);

    let new_center = (center.x + forward.0, center.y + forward.1);
    let left_slot = (new_center.0 - right.0, new_center.1 - right.1);
    let right_slot = (new_center.0 + right.0, new_center.1 + right.1);
    assert!((left_slot.0 - 754.966).abs() < 0.001);
    assert!((left_slot.1 - 2160.7808).abs() < 0.001);
    assert!((right_slot.0 - 788.6923).abs() < 0.001);
    assert!((right_slot.1 - 2181.953).abs() < 0.001);
}

fn enemy(handle: HumanHandle, x: f32) -> PhalanxEnemySnapshot {
    PhalanxEnemySnapshot {
        handle,
        position: position(x, 0.0),
        world_position: crate::coordinates::WorldPoint3D::new(x, 0.0, 0.0),
        direction: 4,
        posture: Posture::Upright,
        elevation: 0.0,
        is_rider: false,
        active: true,
        able_to_fight: true,
        dead: false,
        unconscious: false,
        friend: false,
        in_building: false,
        obstacle: None,
    }
}

fn member(
    handle: HumanHandle,
    radius: f32,
    current_them_list: Vec<PhalanxEnemySnapshot>,
    detectable_enemies: Vec<PhalanxEnemySnapshot>,
) -> PhalanxMemberThemList {
    PhalanxMemberThemList {
        handle,
        entity: EntityId::Soldier(SoldierId(handle)),
        current_them_list,
        detectable_enemies,
        position: position(0.0, 0.0),
        world_position: crate::coordinates::WorldPoint3D::ZERO,
        direction: 4,
        posture: Posture::Upright,
        elevation: 0.0,
        is_rider: false,
        active: true,
        in_building: false,
        view_radius: radius as u16,
        view_direction: [1.0, 0.0],
        real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        sq_view_radius: radius * radius,
    }
}

fn opaque_wall() -> SightObstacle {
    let mut wall = SightObstacle::new_default(0);
    wall.obstacle_points = vec![
        ObstaclePoint {
            x: 95.0,
            y: -10.0,
            z_top: 80.0,
            z_bottom: 0.0,
        },
        ObstaclePoint {
            x: 105.0,
            y: -10.0,
            z_top: 80.0,
            z_bottom: 0.0,
        },
        ObstaclePoint {
            x: 105.0,
            y: 10.0,
            z_top: 80.0,
            z_bottom: 0.0,
        },
        ObstaclePoint {
            x: 95.0,
            y: 10.0,
            z_top: 80.0,
            z_bottom: 0.0,
        },
    ];
    wall.top_plane_points = [
        [95.0, -10.0, 80.0],
        [105.0, -10.0, 80.0],
        [95.0, 10.0, 80.0],
    ];
    wall.bottom_plane_points = [[95.0, -10.0, 0.0], [105.0, -10.0, 0.0], [95.0, 10.0, 0.0]];
    wall.rebuild_geometry();
    wall
}

#[test]
fn phalanx_uses_each_members_heterogeneous_view_radius() {
    let target = enemy(9, 150.0);
    let leftmost = member(1, 300.0, Vec::new(), Vec::new());
    let right = member(2, 100.0, vec![target], Vec::new());
    let mut merged = Vec::new();

    let leftmost_kept: Vec<&PhalanxEnemySnapshot> = leftmost.current_them_list.iter().collect();
    let right_kept: Vec<&PhalanxEnemySnapshot> = right.current_them_list.iter().collect();
    let ctx = AiContext::test_fixture();
    append_phalanx_member_enemies(&mut merged, &leftmost, &leftmost_kept, &ctx);
    append_phalanx_member_enemies(&mut merged, &right, &right_kept, &ctx);

    assert!(merged.is_empty());
}

#[test]
fn phalanx_los_uses_stored_3d_target_before_detection_offset() {
    // Seed3 Savegame_024 replay-032: this leaning target's stored world Y
    // differs by one ULP from `map_y + elevation`. Original
    // Detection-point calculation starts from the stored 3D point.
    let mut target = enemy(30, 1029.8252);
    target.position.y = 1846.1154;
    target.world_position = crate::coordinates::WorldPoint3D::new(1029.8252, 1982.2124, 136.09698);
    target.elevation = 136.09698;
    target.posture = Posture::LeaningOut;
    target.direction = 11;

    let mut viewer = member(2, 1000.0, Vec::new(), Vec::new());
    viewer.position = position(931.252, 1726.3948);
    viewer.world_position = crate::coordinates::WorldPoint3D::new(931.252, 1726.3948, 0.0);

    crate::sight_obstacle::begin_parity_visibility_capture();
    assert!(phalanx_member_detects_360(
        &viewer,
        &target,
        crate::sight_obstacle::ObstacleList::empty(),
    ));
    let queries = crate::sight_obstacle::take_parity_visibility_capture();

    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].destination[1].to_bits(), 1_157_160_897);
}

#[test]
fn inactive_phalanx_member_is_traversed_without_detecting_enemies() {
    let target = enemy(9, 40.0);
    let mut inactive = member(2, 300.0, vec![target.clone()], vec![target]);
    inactive.active = false;
    let kept: Vec<&PhalanxEnemySnapshot> = inactive.current_them_list.iter().collect();
    let mut merged = Vec::new();

    append_phalanx_member_enemies(&mut merged, &inactive, &kept, &AiContext::test_fixture());

    assert!(
        merged.is_empty(),
        "Original follows the inactive neighbour link but both detection variants reject its viewer before LOS"
    );
}

#[test]
fn phalanx_rejects_occluded_persistent_and_detectable_entries() {
    let target = enemy(9, 200.0);
    let member = member(2, 300.0, vec![target.clone()], vec![target]);

    let mut clear_merged = Vec::new();
    let kept: Vec<&PhalanxEnemySnapshot> = member.current_them_list.iter().collect();
    let clear_ctx = AiContext::test_fixture();
    append_phalanx_member_enemies(&mut clear_merged, &member, &kept, &clear_ctx);
    assert_eq!(clear_merged, vec![9]);

    let obstacles = vec![opaque_wall()];
    let active = vec![true];
    let blocked_ctx = AiContext {
        sight_obstacles: SharedSightObstacles {
            static_obstacles: std::sync::Arc::new(obstacles),
            dynamic_obstacles: std::sync::Arc::new(Vec::new()),
            static_active: std::sync::Arc::new(active),
        },
        ..AiContext::test_fixture()
    };
    let mut blocked_merged = Vec::new();
    append_phalanx_member_enemies(&mut blocked_merged, &member, &kept, &blocked_ctx);
    assert!(blocked_merged.is_empty());
}

#[test]
fn phalanx_night_detection_orders_light_rays_before_target_los() {
    let mut target = enemy(9, 600.0);
    target.position.y = 500.0;
    target.world_position = crate::coordinates::WorldPoint3D::new(600.0, 500.0, 0.0);
    let mut member = member(2, 500.0, Vec::new(), vec![target]);
    member.position = position(500.0, 500.0);
    member.world_position = crate::coordinates::WorldPoint3D::new(500.0, 500.0, 0.0);

    let mut ctx = AiContext {
        is_night_or_fog: true,
        ..AiContext::test_fixture()
    };
    let fast_grid = std::sync::Arc::make_mut(&mut ctx.fast_grid);
    fast_grid.size_map(20, 20);
    fast_grid.allocate_layers(1);
    let barycentres = [(750.0, 500.0), (760.0, 510.0), (770.0, 490.0)];
    for (index, &(x, y)) in barycentres.iter().enumerate() {
        let points = vec![
            crate::coordinates::MapPoint::new(x - 4.0, y - 4.0),
            crate::coordinates::MapPoint::new(x + 4.0, y - 4.0),
            crate::coordinates::MapPoint::new(x + 4.0, y + 4.0),
            crate::coordinates::MapPoint::new(x - 4.0, y + 4.0),
        ];
        let mut bounding_box = crate::coordinates::MapBBox::new();
        for &point in &points {
            bounding_box.expand_point(point);
        }
        fast_grid.add_sector(
            crate::fast_find_grid::GridSector {
                points,
                bounding_box,
                sector_type: crate::sector::SectorType::SHADOW,
                layer: 0,
                sector_number: crate::sector::SectorNumber::new(index as i16 + 1),
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
            },
            0,
        );
        std::sync::Arc::make_mut(&mut fast_grid.level)
            .shadow_data
            .insert(
                index as u32,
                crate::sector::ShadowData {
                    barycentre_2d: crate::coordinates::MapPoint::new(x, y),
                    barycentre_3d_x: x,
                    barycentre_3d_y: y,
                    barycentre_3d_z: 45.0,
                    radius: 4.0,
                },
            );
    }

    crate::sight_obstacle::begin_parity_visibility_capture();
    let mut merged = Vec::new();
    append_phalanx_member_enemies(&mut merged, &member, &[], &ctx);
    let queries = crate::sight_obstacle::take_parity_visibility_capture();

    assert_eq!(merged, vec![9]);
    assert_eq!(queries.len(), 4);
    assert_eq!(
        queries
            .iter()
            .map(|query| query.destination)
            .collect::<Vec<_>>(),
        vec![
            [750.0, 500.0, 45.0],
            [760.0, 510.0, 45.0],
            [770.0, 490.0, 45.0],
            [600.0, 500.0, 45.0],
        ]
    );
    assert!(queries.iter().all(|query| query.result));
    let cached_radius = ctx.compute_view_radius_cached(member.entity, None, || {
        panic!("the phalanx member's ground radius should remain cached for its caller")
    });
    assert!(cached_radius > 0.0);
}

#[test]
fn sober_drunk_combat_gate_preserves_original_draws_and_short_circuit() {
    let two_draw_seed = (0..10_000)
        .find(|seed| {
            let sim = SimulationContext::with_seed(*seed);
            crate::sim_rng::u16(&sim, RngSite::DrunkCombatFreeze, 0..100) != 0
                && crate::sim_rng::u16(&sim, RngSite::DrunkCombatFreeze, 0..100) != 0
        })
        .expect("find a seed whose first two drunk gates do not freeze a sober soldier");
    let sim = SimulationContext::with_seed(two_draw_seed);
    let (freezes, trace) = with_draw_trace(|| drunk_combat_freezes(&sim, 0));
    assert!(!freezes);
    assert_eq!(
        trace,
        vec![RngSite::DrunkCombatFreeze, RngSite::DrunkCombatFreeze],
        "a sober soldier must still consume both Original drunk gates"
    );

    let short_circuit_seed = (0..10_000)
        .find(|seed| {
            let sim = SimulationContext::with_seed(*seed);
            crate::sim_rng::u16(&sim, RngSite::DrunkCombatFreeze, 0..100) == 0
        })
        .expect("find a seed whose first drunk gate freezes a sober soldier");
    let sim = SimulationContext::with_seed(short_circuit_seed);
    let (freezes, trace) = with_draw_trace(|| drunk_combat_freezes(&sim, 0));
    assert!(freezes);
    assert_eq!(
        trace,
        vec![RngSite::DrunkCombatFreeze],
        "a successful first gate must preserve Original || short-circuiting"
    );
}

#[test]
fn swordfight_range_checks_use_original_uword_truncation() {
    assert_eq!(original_uword_norm((90.7, 0.0)), 90);
    assert_eq!(original_uword_norm((91.0, 0.0)), 91);
    assert!(original_uword_norm((90.7, 0.0)) <= 90);
}

#[test]
fn swordfight_facing_guard_uses_ground_positions_before_rng() {
    // Schema-14 task 168 frame 2155: projected map positions misleadingly
    // put PC252 in Soldier137's facing sector because their elevations
    // differ. The original-game ground position puts the PC to the east, so
    // Swordfight reconsideration returns before its combat RNG gates.
    let soldier = position(1720.6782, 1984.8649);
    let pc = position(1749.6063, 1954.4141);
    let soldier_elevation = 17.4139;
    let pc_elevation = 45.0639;

    let projected_sector = vec_to_sector(pc.x - soldier.x, pc.y - soldier.y);
    assert_eq!(projected_sector, 1);
    assert_eq!((1_i32 + 16 - projected_sector as i32) % 16, 0);
    let ground_sector = vec_to_sector(
        pc.x - soldier.x,
        (pc.y + pc_elevation) - (soldier.y + soldier_elevation),
    );
    assert_eq!(ground_sector, 4);
    assert_eq!((1_i32 + 16 - ground_sector as i32) % 16, 13);
    assert!(!is_facing_swordfight_target(
        &soldier,
        soldier_elevation,
        1,
        &pc,
        pc_elevation,
    ));

    let sim = SimulationContext::with_seed(0);
    let (_, trace) = with_draw_trace(|| {
        if is_facing_swordfight_target(&soldier, soldier_elevation, 1, &pc, pc_elevation) {
            let _ = drunk_combat_freezes(&sim, 0);
        }
    });
    assert!(
        trace.is_empty(),
        "the facing return must precede combat RNG"
    );
}

#[test]
fn swordfight_facing_guard_uses_live_position_during_door_pass() {
    // Schema-14 Nescafe Profile_003/Savegame_001 replay-012 frame 1448:
    // The player-character position forecasts the far side of door 95, but the original game's
    // facing guard reads ground position and still sees the live PC.
    let soldier = position(1355.0133, 2248.718);
    let live_pc = position(1307.6046, 2_248.182);
    let forecast_pc = position(1304.0, 2276.0);
    let elevation = 45.0;
    let primary = FighterSnapshot {
        handle: 252,
        position: forecast_pc,
        elevation,
        ..FighterSnapshot::default()
    };
    let mut tick = AiPerTickData::stub();
    tick.primary_target_snapshot_handle = Some(AiEntityHandle::new(252));
    tick.primary_target_live_position = Some(live_pc);

    assert!(!is_facing_swordfight_target(
        &soldier,
        elevation,
        12,
        &primary.position,
        primary.elevation,
    ));
    assert!(is_facing_swordfight_target(
        &soldier,
        elevation,
        12,
        &swordfight_facing_target_position(&primary, &tick, |_| {
            panic!("stable principal must use the tick-captured literal position")
        }),
        primary.elevation,
    ));
}

#[test]
fn swordfight_facing_guard_uses_literal_position_after_principal_refresh() {
    // SuN1Sh1nE Savegame_024 replay-037 frame 922: Position(45)
    // forecast movement north far enough to fail this guard, while the
    // literal ground position of actor 45 remained due east and entered RNG.
    let soldier = position(972.988_8, 2075.3225);
    let forecast = position(1019.0, 2089.0);
    let live = position(1022.0, 2069.0);
    let target_elevation = 6.0522804;
    let refreshed_primary = FighterSnapshot {
        handle: 45,
        position: forecast,
        elevation: target_elevation,
        ..FighterSnapshot::default()
    };
    let mut tick = AiPerTickData::stub();
    tick.primary_target_snapshot_handle = Some(AiEntityHandle::new(152));
    tick.primary_target_live_position = Some(position(1031.0, 2089.0));
    let resolved = swordfight_facing_target_position(&refreshed_primary, &tick, |target| {
        assert_eq!(target, refreshed_primary.handle);
        live
    });

    assert!(!is_facing_swordfight_target(
        &soldier,
        0.0,
        4,
        &forecast,
        target_elevation,
    ));
    assert!(is_facing_swordfight_target(
        &soldier,
        0.0,
        4,
        &resolved,
        target_elevation,
    ));
}

#[test]
fn swordfight_step_in_uses_live_exact_target_sector() {
    use crate::fast_find_grid::SectorIndex;

    const TARGET: u32 = 137;
    let target = crate::element::Entity::Pc(crate::element::ActorPc {
        element: {
            let mut initial_element =
                crate::element::ElementData::from_initial_posture(crate::element::Posture::Upright);
            initial_element.kind = crate::element::ElementKind::ActorPc;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    let mut target_view = crate::ai_entity_view::entity_view_from_entity(
        &target,
        313,
        false,
        None,
        None,
        crate::order::OrderType::NonanimationEnd,
    );
    let exact_sector = crate::position_interface::SectorHandle::new(88)
        .unwrap()
        .with_arena_index(SectorIndex::new(114).unwrap());
    target_view.position = Position {
        x: 650.92444,
        y: 1537.1555,
        sector: Some(exact_sector),
        level: 2,
    };
    target_view.active = true;
    target_view.camp = crate::element::Camp::Royalists;
    target_view.detection_position = crate::coordinates::MapPoint::new(640.0, 1520.0);
    target_view.detection_position_world =
        crate::coordinates::WorldPoint3D::new(target_view.position.x, target_view.position.y, 0.0);
    let mut legacy_view = target_view.clone();
    legacy_view.position.sector = crate::position_interface::SectorHandle::new(88);
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(TARGET, target_view);
    views.insert(TARGET + 1, legacy_view);
    let owner_position = Position {
        x: 550.92444,
        y: 1537.1555,
        sector: crate::position_interface::SectorHandle::new(70),
        level: 1,
    };
    let ctx = AiContext {
        position: owner_position,
        self_layer: 1,
        direction: vec_to_sector(650.92444 - owner_position.x, 1537.1555 - owner_position.y),
        camp: crate::element::Camp::Lacklandists,
        is_swordfighting: true,
        self_is_active: true,
        self_upright_eye_world: crate::coordinates::WorldPoint3D::new(
            owner_position.x,
            owner_position.y,
            0.0,
        ),
        sq_self_view_radius: 1_000_000.0,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let resolved = live_swordfight_target_position(TARGET, &ctx);
    assert_eq!((resolved.x, resolved.y), (650.92444, 1537.1555));
    assert_eq!(resolved.sector, Some(exact_sector));
    assert_eq!(resolved.level, 2);
    assert_eq!(
        resolved.sector.and_then(|sector| sector.arena_index()),
        SectorIndex::new(114),
        "a number-only fighter snapshot must not replace the live target's exact sector"
    );
    assert_eq!(
        live_swordfight_target_position(TARGET + 1, &ctx).sector,
        crate::position_interface::SectorHandle::new(88),
        "legacy number-only live views remain number-only; this boundary must not guess an arena slot"
    );
    let literal = literal_swordfight_target_position(TARGET, &ctx);
    assert_eq!((literal.x, literal.y), (640.0, 1520.0));
    assert_eq!(literal.sector, Some(exact_sector));

    let number_only_target = Position {
        x: 650.92444,
        y: 1537.1555,
        sector: crate::position_interface::SectorHandle::new(88),
        level: 2,
    };
    let target_fighter = FighterSnapshot {
        handle: TARGET,
        position: number_only_target,
        raw_position: number_only_target,
        is_swordfighting: true,
        is_able_to_fight: true,
        sword_range_maximal: 50,
        ..FighterSnapshot::default()
    };
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.push(FighterSnapshot {
        handle: 182,
        position: owner_position,
        raw_position: owner_position,
        principal_opponent: Some(AiEntityHandle::new(TARGET)),
        is_friendly: true,
        sword_range_default: 50,
        sword_range_maximal: 50,
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.push(target_fighter.clone());
    tick.reconsider_swordfight_enemies.push(target_fighter);

    let seed = (0..10_000)
        .find(|seed| {
            let sim = SimulationContext::with_seed(*seed);
            !drunk_combat_freezes(&sim, 0)
        })
        .expect("find a sober two-draw seed");
    let mut ai = EnemyAi::new(182);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingSwordfight;
    ai.base.primary_target = Some(AiEntityHandle::new(TARGET));
    ai.sword_range = 50;
    ai.reconsider_swordfight(
        &SimulationContext::with_seed(seed),
        false,
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(
        ai.base
            .last_goto_destination
            .sector
            .and_then(|sector| sector.arena_index()),
        SectorIndex::new(114),
        "the production too-far approach writer must carry the live target's exact arena identity"
    );
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingMovingAroundOldEnemy
    );
}

fn lost_enemy_reconsider_fixture(company_number: u16) -> (EnemyAi, AiContext, AiPerTickData) {
    const OWNER: u32 = 21;
    const TARGET: u32 = 20;

    let target = crate::element::Entity::Pc(crate::element::ActorPc {
        element: {
            let mut initial_element =
                crate::element::ElementData::from_initial_posture(crate::element::Posture::Upright);
            initial_element.kind = crate::element::ElementKind::ActorPc;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    let mut target_view = crate::ai_entity_view::entity_view_from_entity(
        &target,
        51,
        false,
        None,
        None,
        crate::order::OrderType::NonanimationEnd,
    );
    target_view.position = position(100.0, 0.0);
    target_view.camp = crate::element::Camp::Royalists;
    target_view.active = false;

    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(TARGET, target_view);
    let ctx = AiContext {
        position: position(0.0, 0.0),
        self_is_active: true,
        camp: crate::element::Camp::Lacklandists,
        is_swordfighting: true,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.push(FighterSnapshot {
        handle: OWNER,
        principal_opponent: Some(AiEntityHandle::new(TARGET)),
        is_friendly: true,
        ..FighterSnapshot::default()
    });
    tick.primary_target_snapshot_handle = Some(AiEntityHandle::new(TARGET));
    tick.primary_target_is_pc = true;
    tick.primary_target_forecast = Some(crate::ai::PreparedForecastDestination::fixed(
        position(0.0, 100.0),
        4,
    ));

    let mut ai = EnemyAi::new(OWNER);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingSwordfight;
    ai.base.primary_target = Some(AiEntityHandle::new(TARGET));
    ai.company_number = company_number;
    (ai, ctx, tick)
}

#[test]
fn lost_enemy_overview_faces_live_target_not_forecast_destination() {
    // randomguy Profile_004/Savegame_030 replay-014 frame 3456:
    // Original forecasts the missed PC for possible pursuit, but the
    // no-follow branch snaps Soldier 21 toward the PC's current Position.
    let (mut ai, ctx, tick) = lost_enemy_reconsider_fixture(100);
    let sim = SimulationContext::with_seed(0);
    ai.reconsider_swordfight(
        &sim,
        false,
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(ai.base.seek_position, position(0.0, 100.0));
    assert_eq!(vec_to_sector(100.0, 0.0), 4);
    assert_eq!(vec_to_sector(0.0, 100.0), 8);
    let direction_prefixes: Vec<_> = ai
        .base
        .outbox
        .reentrant
        .owner_work
        .iter()
        .filter_map(|work| match work {
            crate::ai::AiOwnerWork::ActorEffects(effects) => effects.set_direction_instantly,
            crate::ai::AiOwnerWork::StateChange(change) => change
                .actor_effects_before_callback
                .as_ref()
                .and_then(|effects| effects.set_direction_instantly),
            _ => None,
        })
        .collect();
    assert_eq!(
        direction_prefixes,
        vec![4],
        "the live-target snap must cross exactly one pre-StopAll actor boundary"
    );
    assert_eq!(ai.base.outbox.actor.set_direction_instantly, None);
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingOverviewLookLeft
    );
}

#[test]
fn lost_enemy_follow_path_keeps_forecast_as_seek_center_without_direction_snap() {
    let (mut ai, ctx, tick) = lost_enemy_reconsider_fixture(0);
    let sim = SimulationContext::with_seed(0);
    ai.reconsider_swordfight(
        &sim,
        false,
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(ai.base.outbox.actor.set_direction_instantly, None);
    assert_eq!(ai.seek_center, position(0.0, 100.0));
    assert_eq!(ai.base.current_state, AiState::Seeking);
}

#[test]
fn lost_enemy_refreshes_forecast_with_swordfight_principal() {
    // Savegame_029 replay-032 frame 5296: the AI member still named PC
    // 168, while the principal opponent was PC 169. The old forecast
    // centered the area search on 168 and changed its RNG draw count.
    const OLD_TARGET: u32 = 20;
    const NEW_PRINCIPAL: u32 = 22;
    let (mut ai, mut ctx, mut tick) = lost_enemy_reconsider_fixture(0);
    debug_assert_eq!(
        ai.base.primary_target,
        Some(AiEntityHandle::new(OLD_TARGET))
    );

    let old_view = ctx.entity_view(OLD_TARGET).unwrap().clone();
    let mut new_view = old_view.clone();
    new_view.position = position(300.0, 400.0);
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(OLD_TARGET, old_view);
    views.insert(NEW_PRINCIPAL, new_view);
    ctx.entity_views = crate::ai_entity_view::shared_entity_views(views);

    tick.fighter_registry[0].principal_opponent = Some(AiEntityHandle::new(NEW_PRINCIPAL));
    let refreshed_forecast = position(500.0, 600.0);
    tick.enemy_detectable_forecasts.push((
        NEW_PRINCIPAL,
        crate::ai::PreparedForecastDestination::fixed(refreshed_forecast, 7),
    ));

    let sim = SimulationContext::with_seed(0);
    ai.reconsider_swordfight(
        &sim,
        false,
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(ai.missed_pc, Some(AiEntityHandle::new(NEW_PRINCIPAL)));
    assert_eq!(ai.seek_center, refreshed_forecast);
    assert_ne!(ai.seek_center, position(0.0, 100.0));
}

#[test]
fn direct_fighter_lookup_reaches_beyond_nearby_radius_snapshot() {
    let ai = EnemyAi::default();
    let mut tick = AiPerTickData::stub();
    tick.nearby_fighters.push(FighterSnapshot {
        handle: 1,
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.push(FighterSnapshot {
        handle: 2,
        ..FighterSnapshot::default()
    });

    assert_eq!(
        ai.find_fighter(1, &tick).map(|fighter| fighter.handle),
        Some(1)
    );
    assert_eq!(
        ai.find_fighter(2, &tick).map(|fighter| fighter.handle),
        Some(2)
    );
    assert!(ai.find_fighter(3, &tick).is_none());
}

#[test]
fn sword_strike_honour_uses_synchronous_fighter_snapshot() {
    for (recovery, sword_action, expected) in [
        (false, true, true),
        (true, true, false),
        (false, false, false),
    ] {
        let target = FighterSnapshot {
            is_in_recovery_animation: recovery,
            in_sword_action_state: sword_action,
            ..FighterSnapshot::default()
        };
        assert_eq!(
            EnemyAi::sword_strike_honour_allows_proposal(&target),
            expected,
            "recovery={recovery}, sword_action={sword_action}"
        );
    }
}

#[test]
fn failed_observation_step_back_panics_without_speaking() {
    let mut ai = EnemyAi::new(91);
    let enemy_pos = position(663.922_5, 2096.012);

    let request = ai.panic_after_failed_observation_step_back(enemy_pos);

    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
    assert!(ai.base.outbox.actor.begin_panic.is_none());
    assert_eq!(request.center, Some(enemy_pos));
    assert_eq!(request.runs, parameters_ai::AI_STANDARD_PANIC_RUNS as u8);
    assert!(
        ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .all(|work| !matches!(work, AiOwnerWork::Speech(_))),
        "Original's Panic fallback does not call Flee's Say(REMARK_PANIC)"
    );
}
