use super::*;
use crate::ai::{DoorSeekInfo, House, cache_npc_villain_authorized_direct};
use crate::ai_entity_view::{AiEntityView, AiEntityViewMap, EntityKind, NetCoverInfo};
use crate::coordinates::MapPoint;
use crate::element::{Camp, DetectableType, EyeStatus, Posture};
use crate::entity_id::{EntityId, SoldierId};
use crate::gate::{Door, DoorIndex, DoorType};
use crate::order::OrderType;
use crate::position_interface::SectorHandle;
use crate::sight_obstacle::{ObstaclePoint, SharedSightObstacles, SightObstacle};
use std::sync::Arc;

fn test_position(x: f32, y: f32) -> Position {
    Position {
        x,
        y,
        sector: None,
        level: 0,
    }
}

fn soldier_view(pos: Position) -> AiEntityView {
    AiEntityView {
        original_creation_order: 41,
        position: pos,
        detection_position: crate::coordinates::MapPoint::new(pos.x, pos.y),
        detection_position_world: crate::coordinates::WorldPoint3D::new(pos.x, pos.y, 0.0),
        direction: 0,
        posture: Posture::Upright,
        camp: Camp::Royalists,
        is_pc: false,
        is_robin: false,
        is_vip: false,
        is_beggar: false,
        is_child: false,
        kind: EntityKind::Soldier,
        is_tower_guard: false,
        is_swordfighting: false,
        is_able_to_fight: true,
        active: true,
        is_unconscious: false,
        action_state: crate::element::ActionState::Waiting,
        is_moving_map: false,
        passing_door: false,
        obstacle_idx: None,
        in_building: false,
        building_sector: None,
        script_locked: false,
        forecasted_destination: crate::ai::PreparedForecastDestination::fixed(pos, 0),
        ai_state: AiState::Default,
        ai_substate: Substate::DefaultOnPost,
        current_animation: OrderType::WaitingUprightBored,
        elevation: 0.0,
        object_type: crate::element_kinds::ObjectType::None,
        is_dead: false,
        is_carried: false,
        is_archer: false,
        is_rider: false,
        stuck_under_net: false,
        covering_nets: Vec::new(),
        in_coma: false,
        guard: None,
        has_patrol_path: false,
        initial_position: pos,
        number_of_arrows: 0,
        rank: ProfileRank::Soldier,
        reported_to_officer: false,
        looted_after_money_fight: false,
        current_money: 0,
        macro_in_progress: false,
        path_current_waypoint_index: 0,
        path_last_waypoint_index: 0,
        path_forward_movement: true,
        patrol_hiking_path_index: None,
        interesting_object: None,
        report_type: ReportType::Nothing,
        report_seek_position: pos,
        report_seen_bodies: Vec::new(),
        report_charly: None,
    }
}

fn camp_soldier(handle: u32, position: Position) -> CampSoldierInfo {
    CampSoldierInfo {
        handle,
        active: true,
        position,
        position_world: crate::coordinates::WorldPoint3D::new(position.x, position.y, 0.0),
        direction: 0,
        rank: ProfileRank::Soldier,
        ai_state: AiState::Default,
        ai_substate: Substate::None,
        is_able_to_fight: true,
        is_dead: false,
        knocked_out_in_money_fight: false,
        primary_target: None,
        pride: 0,
        is_able_to_help: true,
        script_locked: false,
        ai_lock_frozen: false,
        layer: 0,
        report_type: ReportType::Nothing,
        report_seek_position: Position::default(),
        report_seen_bodies: Vec::new(),
        report_charly: None,
        alert_soldiers_point: Position::default(),
        patrol_chief: None,
        antagonist: None,
        detected_body: None,
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
    }
}

fn charly_to_officer_context(
    officer_position: Position,
    obstacles: Vec<SightObstacle>,
) -> AiContext {
    let mut officer = soldier_view(officer_position);
    officer.rank = ProfileRank::Officer;
    officer.ai_state = AiState::Default;
    officer.ai_substate = Substate::DefaultOnPost;

    // The acting Charly (handle 1) must be present in the entity view so
    // detection can resolve its viewer identity; keep its view fields in
    // lockstep with the context's self geometry below.
    let mut charly = soldier_view(test_position(0.0, 0.0));
    charly.direction = 4;

    let mut views = AiEntityViewMap::new();
    views.insert(1, charly);
    views.insert(2, officer);
    let obstacle_count = obstacles.len();
    AiContext {
        position: test_position(0.0, 0.0),
        frame: 100,
        direction: 4,
        posture: Posture::Upright,
        self_eye_position: crate::coordinates::MapPoint::new(0.0, 0.0),
        self_eye_z: 45.0,
        self_upright_eye_world: crate::coordinates::WorldPoint3D::new(0.0, 0.0, 45.0),
        self_view_direction: [1.0, 0.0],
        self_view_radius: 400,
        self_real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        self_eye_status: EyeStatus::LookForward,
        sq_self_view_radius: 400.0 * 400.0,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        sight_obstacles: SharedSightObstacles {
            static_obstacles: Arc::new(obstacles),
            dynamic_obstacles: Arc::new(Vec::new()),
            static_active: Arc::new(vec![true; obstacle_count]),
        },
        ..AiContext::test_fixture()
    }
}

#[test]
fn look_there_precheck_uses_raw_owner_geometry_during_door_transit() {
    let mut ai = EnemyAi::new(124);
    let raw_owner = crate::coordinates::WorldPoint3D::new(722.0, 1695.0, 160.0);
    let raw_friend = crate::coordinates::WorldPoint3D::new(713.0, 1663.0, 250.0);

    let mut owner_view = soldier_view(test_position(1709.0, 2228.0));
    owner_view.camp = Camp::Lacklandists;
    owner_view.passing_door = true;
    owner_view.detection_position_world = raw_owner;
    let mut friend_view = soldier_view(test_position(713.0, 1413.0));
    friend_view.camp = Camp::Lacklandists;
    friend_view.detection_position_world = raw_friend;

    let mut views = AiEntityViewMap::new();
    views.insert(124, owner_view);
    views.insert(184, friend_view);
    let ctx = AiContext {
        // AI Position() has snapped the owner to the distant gate point,
        // while the raw element position remains within the 100-unit
        // world radius of the raised friend.
        position: test_position(1709.0, 2228.0),
        self_body_position_world: raw_owner,
        camp: Camp::Lacklandists,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    assert!(ai.hey_folks_look_there(
        &test_position(1154.0, 1860.0),
        100,
        LookThereContinuation::EventView {
            enemy: 342,
            enemy_pos: test_position(1154.0, 1860.0),
        },
        &ctx,
    ));
    assert!(matches!(
        ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
        [CrossNpcAction::BroadcastLookThere { caller: 124, .. }]
    ));
}

#[test]
fn detecting_360_uses_raw_active_actor_geometry_during_door_transit() {
    let ai = EnemyAi::new(1);
    for (posture, unconscious) in [(Posture::Upright, true), (Posture::Tied, false)] {
        // AI Position() has already snapped the target to its far gate
        // endpoint, while detection-point calculation still reads the live raw
        // actor position near the observer.
        let mut target = soldier_view(test_position(900.0, 0.0));
        target.detection_position = MapPoint::new(20.0, 0.0);
        target.detection_position_world = crate::coordinates::WorldPoint3D::new(20.0, 0.0, 0.0);
        target.posture = posture;
        target.is_unconscious = unconscious;
        target.is_able_to_fight = false;
        target.passing_door = true;

        let mut views = AiEntityViewMap::new();
        views.insert(2, target.clone());
        let ctx = AiContext {
            // Self is also on a door rail: broad AI inside-building state
            // and snapped Position() must not replace the current-sector
            // or direct upright-eye geometry used by this overload.
            position: test_position(-900.0, 0.0),
            in_building: true,
            building_sector: None,
            self_upright_eye_world: crate::coordinates::WorldPoint3D::new(0.0, 0.0, 45.0),
            sq_self_view_radius: 200.0 * 200.0,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::test_fixture()
        };
        assert!(
            ai.is_detecting_360_degrees(2, &ctx),
            "active {posture:?} target must use raw actor geometry"
        );

        let mut inactive = target.clone();
        inactive.active = false;
        let mut views = AiEntityViewMap::new();
        views.insert(2, inactive);
        assert!(!ai.is_detecting_360_degrees(
            2,
            &AiContext {
                entity_views: crate::ai_entity_view::shared_entity_views(views),
                ..ctx.clone()
            }
        ));

        let mut indoor = target;
        indoor.in_building = true;
        let mut views = AiEntityViewMap::new();
        views.insert(2, indoor);
        assert!(!ai.is_detecting_360_degrees(
            2,
            &AiContext {
                entity_views: crate::ai_entity_view::shared_entity_views(views),
                ..ctx
            }
        ));
    }
}

#[test]
fn normal_detection_uses_raw_pass_door_target_position() {
    let ai = EnemyAi::new(1);
    let mut target = soldier_view(test_position(900.0, 0.0));
    target.detection_position = MapPoint::new(100.0, 0.0);
    target.detection_position_world = crate::coordinates::WorldPoint3D::new(100.0, 0.0, 0.0);
    target.passing_door = true;
    // Self view for the acting soldier (handle 1), matching the context's
    // self geometry so the viewer identity resolves during detection.
    let mut viewer = soldier_view(test_position(0.0, 0.0));
    viewer.direction = 4;
    let mut views = AiEntityViewMap::new();
    views.insert(1, viewer);
    views.insert(2, target);
    let ctx = AiContext {
        direction: 4,
        self_eye_position: MapPoint::ZERO,
        self_eye_z: 45.0,
        self_view_direction: [1.0, 0.0],
        self_view_radius: 400,
        self_real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        self_eye_status: EyeStatus::LookForward,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    assert!(ai.is_detecting(2, &ctx));
}

fn charly_heading_to_officer() -> EnemyAi {
    let mut ai = EnemyAi::new(1);
    ai.base.antagonist = Some(AiEntityHandle::new(2));
    ai.set_state(AiState::Seeking, Substate::SeekingCharlyGoToOfficer);
    ai
}

fn opaque_wall_across_x_axis() -> SightObstacle {
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
fn detection_180_uses_raw_actor_xy_instead_of_ai_position() {
    let ai = EnemyAi::new(1);
    let mut viewer = soldier_view(test_position(0.0, 0.0));
    viewer.direction = 4;
    let mut target = soldier_view(test_position(200.0, 0.0));
    // AI Position() can be displaced from the raw element anchor, for
    // example while passing a door. The original game's standalone 180° path
    // computes detection points from the raw actor position instead.
    target.position = test_position(200.0, 30.0);

    // This wall intersects the AI-position ray (0,0)->(200,30) at
    // y=15, but not the original-compatible raw ray (0,0)->(200,0).
    let mut wall = SightObstacle::new_default(0);
    wall.obstacle_points = vec![
        ObstaclePoint {
            x: 95.0,
            y: 10.0,
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
            x: 105.0,
            y: 20.0,
            z_top: 80.0,
            z_bottom: 0.0,
        },
        ObstaclePoint {
            x: 95.0,
            y: 20.0,
            z_top: 80.0,
            z_bottom: 0.0,
        },
    ];
    wall.top_plane_points = [[95.0, 10.0, 80.0], [105.0, 10.0, 80.0], [95.0, 20.0, 80.0]];
    wall.bottom_plane_points = [[95.0, 10.0, 0.0], [105.0, 10.0, 0.0], [95.0, 20.0, 0.0]];
    wall.rebuild_geometry();

    let mut views = AiEntityViewMap::new();
    views.insert(1, viewer);
    views.insert(2, target);
    let ctx = AiContext {
        position: test_position(0.0, 0.0),
        direction: 4,
        posture: Posture::Upright,
        self_eye_position: MapPoint::ZERO,
        self_eye_z: 45.0,
        self_view_radius: 400,
        sq_self_view_radius: 400.0 * 400.0,
        self_view_direction: [1.0, 0.0],
        self_real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        sight_obstacles: SharedSightObstacles {
            static_obstacles: Arc::new(vec![wall]),
            dynamic_obstacles: Arc::new(Vec::new()),
            static_active: Arc::new(vec![true]),
        },
        ..AiContext::test_fixture()
    };

    assert!(ai.is_detecting_180_degrees(2, &ctx));
}

#[test]
fn beer_competition_uses_original_npc_registry_order() {
    let ale = AiEntityHandle::new(99);
    let mut ai = EnemyAi::new(1);
    ai.base.interesting_object = Some(ale);

    let owner_position = test_position(0.0, 0.0);
    let first_position = test_position(100.0, 0.0);
    let second_position = test_position(200.0, 0.0);

    let mut owner = soldier_view(owner_position);
    owner.original_creation_order = 1;
    owner.direction = 4;
    let mut first = soldier_view(first_position);
    first.original_creation_order = 10;
    first.ai_substate = Substate::WonderingAleReactiontime;
    first.interesting_object = Some(ale);
    let mut second = soldier_view(second_position);
    second.original_creation_order = 20;
    second.ai_substate = Substate::WonderingAleReactiontime;
    second.interesting_object = Some(ale);
    let mut ale_view = soldier_view(test_position(300.0, 0.0));
    ale_view.kind = EntityKind::Bonus;

    let mut views = AiEntityViewMap::new();
    views.insert(1, owner);
    // Runtime handles deliberately disagree with Original registration
    // order: the earlier NPC has the larger handle.
    views.insert(3, first);
    views.insert(2, second);
    views.insert(99, ale_view);
    let ctx = AiContext {
        position: owner_position,
        direction: 4,
        self_eye_position: MapPoint::ZERO,
        self_eye_z: 45.0,
        self_view_radius: 400,
        sq_self_view_radius: 400.0 * 400.0,
        self_view_direction: [1.0, 0.0],
        self_real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    crate::sight_obstacle::begin_parity_visibility_capture();
    assert_eq!(ai.is_beer_still_available(&ctx), Some(first_position));
    let queries = crate::sight_obstacle::take_parity_visibility_capture();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].destination[0], first_position.x);
}

#[test]
fn detection_180_los_uses_stored_world_point_without_projection_round_trip() {
    let ai = EnemyAi::new(1);
    let mut viewer = soldier_view(test_position(1373.0, 595.0));
    viewer.direction = 4;

    // These are representative moving-actor coordinates where
    // `(world_y - z) + z` rounds one ULP away from the stored world Y.
    // Original passes the stored 3D detection point verbatim
    // to grid reachability testing.
    let raw = crate::coordinates::WorldPoint3D::new(
        1_555.961_5,
        f32::from_bits(1_143_810_793),
        46.786_65,
    );
    let mut target = soldier_view(test_position(raw.x, raw.y - raw.z));
    target.detection_position = MapPoint::from_world_xyz(raw.x, raw.y, raw.z);
    target.detection_position_world = raw;
    target.elevation = raw.z;
    assert_ne!(
        (target.detection_position.y + target.elevation).to_bits(),
        raw.y.to_bits(),
        "fixture must distinguish projection round-trip Y from stored world Y"
    );

    let mut views = AiEntityViewMap::new();
    views.insert(1, viewer);
    views.insert(2, target);
    let ctx = AiContext {
        direction: 4,
        self_eye_position: MapPoint::new(1373.0, 595.0),
        self_eye_z: 45.0,
        elevation: 45.0,
        self_view_radius: 400,
        sq_self_view_radius: 400.0 * 400.0,
        self_view_direction: [1.0, 0.0],
        self_real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    crate::sight_obstacle::begin_parity_visibility_capture();
    assert!(ai.is_detecting_180_degrees(2, &ctx));
    let queries = crate::sight_obstacle::take_parity_visibility_capture();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].destination[0].to_bits(), raw.x.to_bits());
    assert_eq!(queries[0].destination[1].to_bits(), raw.y.to_bits());
    assert_eq!(
        queries[0].destination[2].to_bits(),
        (raw.z + crate::stealth::detection_z_for_posture(Posture::Upright, false)).to_bits()
    );
}

fn standalone_180_context(target: AiEntityView, radius: u16) -> AiContext {
    let mut viewer = soldier_view(test_position(0.0, 0.0));
    viewer.direction = 4;
    let mut views = AiEntityViewMap::new();
    views.insert(1, viewer);
    views.insert(2, target);
    AiContext {
        direction: 4,
        self_eye_position: MapPoint::ZERO,
        self_eye_z: 45.0,
        self_view_radius: radius,
        sq_self_view_radius: (radius as f32) * (radius as f32),
        self_view_direction: [1.0, 0.0],
        self_real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    }
}

#[test]
fn standalone_180_allows_active_target_inside_building() {
    let mut target = soldier_view(test_position(100.0, 0.0));
    target.in_building = true;
    target.building_sector = SectorHandle::new(7);
    let ctx = standalone_180_context(target, 400);

    assert!(context_detects_180_degrees(1, 2, &ctx));
}

#[test]
fn standalone_180_has_no_generic_standard_view_aabb() {
    // 500 is outside the generic NearbyCiviliansPanic standard-radius
    // AABB (400), but inside this civilian's live 600-unit radius.
    let target = soldier_view(test_position(500.0, 0.0));
    let ctx = standalone_180_context(target, 600);

    assert!(context_detects_180_degrees(1, 2, &ctx));
}

#[test]
fn standalone_180_close_sideways_shortcut_precedes_opaque_los() {
    let target = soldier_view(test_position(0.0, 20.0));
    let mut ctx = standalone_180_context(target, 400);
    let mut wall = opaque_wall_across_x_axis();
    for point in &mut wall.obstacle_points {
        let (x, y) = (point.x, point.y);
        point.x = y;
        point.y = x - 85.0;
    }
    for point in wall
        .top_plane_points
        .iter_mut()
        .chain(wall.bottom_plane_points.iter_mut())
    {
        let (x, y) = (point[0], point[1]);
        point[0] = y;
        point[1] = x - 85.0;
    }
    wall.rebuild_geometry();
    ctx.sight_obstacles = SharedSightObstacles {
        static_obstacles: Arc::new(vec![wall]),
        dynamic_obstacles: Arc::new(Vec::new()),
        static_active: Arc::new(vec![true]),
    };

    crate::sight_obstacle::begin_parity_visibility_capture();
    assert!(context_detects_180_degrees(1, 2, &ctx));
    assert!(crate::sight_obstacle::take_parity_visibility_capture().is_empty());
}

#[test]
fn standalone_180_applies_dynamic_ground_radius_before_los() {
    // Raw real radius accepts 399, while view-radius calculation projects the
    // 400-unit sphere at eye Z=45 to about 397.46 on the ground.
    let target = soldier_view(test_position(399.0, 0.0));
    let ctx = standalone_180_context(target, 400);

    crate::sight_obstacle::begin_parity_visibility_capture();
    assert!(!context_detects_180_degrees(1, 2, &ctx));
    assert!(crate::sight_obstacle::take_parity_visibility_capture().is_empty());
}

#[test]
fn money_fight_enemy_rebuild_rechecks_current_unconscious_before_detection() {
    let mut ai = EnemyAi::new(1);
    let owner_position = test_position(0.0, 0.0);
    let candidate_position = test_position(100.0, 0.0);

    let owner = soldier_view(owner_position);
    let mut candidate = soldier_view(candidate_position);
    candidate.is_able_to_fight = false;
    candidate.is_unconscious = true;
    candidate.ai_state = AiState::Wondering;
    candidate.ai_substate = Substate::WonderingBrawlHitting;

    let mut views = AiEntityViewMap::new();
    views.insert(1, owner);
    views.insert(2, candidate);
    let ctx = AiContext {
        position: owner_position,
        self_eye_position: MapPoint::new(0.0, 0.0),
        self_eye_z: 45.0,
        self_upright_eye_world: crate::coordinates::WorldPoint3D::new(0.0, 0.0, 45.0),
        self_view_radius: 400,
        sq_self_view_radius: 400.0 * 400.0,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    // This top-of-tick entry is intentionally stale: the candidate was
    // conscious when the snapshot was built, then knocked out earlier in
    // the same creation-order AI pass.
    tick.camp_soldiers.push(CampSoldierInfo {
        handle: 2,
        active: true,
        position: candidate_position,
        position_world: crate::coordinates::WorldPoint3D::new(100.0, 0.0, 0.0),
        direction: 0,
        rank: ProfileRank::Soldier,
        ai_state: AiState::Wondering,
        ai_substate: Substate::WonderingBrawlHitting,
        is_able_to_fight: true,
        is_dead: false,
        knocked_out_in_money_fight: false,
        primary_target: None,
        pride: 0,
        is_able_to_help: false,
        script_locked: false,
        ai_lock_frozen: false,
        layer: 0,
        report_type: ReportType::Nothing,
        report_seek_position: Position::default(),
        report_seen_bodies: Vec::new(),
        report_charly: None,
        alert_soldiers_point: Position::default(),
        patrol_chief: None,
        antagonist: None,
        detected_body: None,
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

    crate::sight_obstacle::begin_parity_visibility_capture();
    ai.create_new_list_of_money_fight_enemies(&tick, &ctx);
    let queries = crate::sight_obstacle::take_parity_visibility_capture();

    assert!(queries.is_empty(), "lifecycle gate must precede detection");
    assert!(ai.money_fight_enemies.is_empty());
}

#[test]
fn money_fight_morale_coalesces_ordered_camp_and_sleeper_snapshots() {
    let mut ai = EnemyAi::new(1);
    ai.soldier_profile_money = 40;

    let owner_position = test_position(0.0, 0.0);
    let sleeping_position = test_position(300.0, 0.0);
    let fighter_position = test_position(100.0, 0.0);
    let dead_position = test_position(200.0, 0.0);
    let disjoint_sleeping_position = test_position(250.0, 0.0);

    let mut sleeping_view = soldier_view(sleeping_position);
    sleeping_view.original_creation_order = 3;
    sleeping_view.ai_substate = Substate::SleepingUnconscious;
    sleeping_view.is_unconscious = true;
    let mut fighter_view = soldier_view(fighter_position);
    fighter_view.original_creation_order = 2;
    fighter_view.ai_substate = Substate::WonderingBrawlHitting;
    let mut dead_view = soldier_view(dead_position);
    dead_view.original_creation_order = 4;
    dead_view.is_dead = true;
    let mut disjoint_sleeping_view = soldier_view(disjoint_sleeping_position);
    disjoint_sleeping_view.original_creation_order = 5;
    disjoint_sleeping_view.ai_substate = Substate::SleepingUnconscious;
    disjoint_sleeping_view.is_unconscious = true;

    let mut views = AiEntityViewMap::new();
    let mut owner_view = soldier_view(owner_position);
    owner_view.original_creation_order = 1;
    views.insert(1, owner_view);
    views.insert(2, fighter_view);
    views.insert(3, sleeping_view);
    views.insert(4, dead_view);
    views.insert(5, disjoint_sleeping_view);
    let ctx = AiContext {
        position: owner_position,
        self_eye_position: MapPoint::ZERO,
        self_eye_z: 45.0,
        self_upright_eye_world: crate::coordinates::WorldPoint3D::new(0.0, 0.0, 45.0),
        self_view_radius: 400,
        sq_self_view_radius: 400.0 * 400.0,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let mut sleeping = camp_soldier(3, sleeping_position);
    // Deliberately stale: the original game classifies the current AI substate
    // after the visibility query, not this earlier camp snapshot.
    sleeping.ai_substate = Substate::WonderingBrawlHitting;
    sleeping.knocked_out_in_money_fight = true;
    let mut fighter = camp_soldier(2, fighter_position);
    fighter.ai_substate = Substate::None;
    // Deliberately stale alive snapshot: an earlier actor killed this
    // soldier before our turn, so the current view must suppress LOS.
    let dead = camp_soldier(4, dead_position);
    let mut tick = AiPerTickData::stub();
    // Both source snapshots retain camp/handle order. Self and dead
    // entries do not query; handle 3 overlaps and must coalesce.
    tick.camp_soldiers = vec![camp_soldier(1, owner_position), fighter, sleeping, dead];
    tick.camp_unconscious_soldiers = vec![
        CampUnconsciousSoldierInfo {
            handle: 3,
            knocked_out_in_money_fight: true,
        },
        // Preexisting sleepers are absent from `camp_soldiers` in the
        // main detection builder and must still participate once.
        CampUnconsciousSoldierInfo {
            handle: 5,
            knocked_out_in_money_fight: true,
        },
    ];

    crate::sight_obstacle::begin_parity_visibility_capture();
    assert!(!ai.wants_to_continue_money_fight(&tick, &ctx));
    let queries = crate::sight_obstacle::take_parity_visibility_capture();
    assert_eq!(queries.len(), 3);
    assert_eq!(
        queries
            .iter()
            .map(|query| query.destination[0])
            .collect::<Vec<_>>(),
        vec![100.0, 300.0, 250.0],
        "one query per live candidate in authored camp-registry order"
    );
}

#[test]
fn detection_180_accepts_an_active_unconscious_target() {
    let ai = EnemyAi::new(1);
    let mut viewer = soldier_view(test_position(0.0, 0.0));
    viewer.direction = 4;
    let mut target = soldier_view(test_position(100.0, 0.0));
    target.is_able_to_fight = false;
    target.is_unconscious = true;
    target.active = true;

    let mut views = AiEntityViewMap::new();
    views.insert(1, viewer);
    views.insert(2, target);
    let ctx = AiContext {
        direction: 4,
        self_eye_position: MapPoint::ZERO,
        self_eye_z: 45.0,
        self_view_radius: 400,
        sq_self_view_radius: 400.0 * 400.0,
        self_view_direction: [1.0, 0.0],
        self_real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    assert!(ai.is_detecting_180_degrees(2, &ctx));
}

#[test]
fn charly_inside_view_cone_queues_synchronous_officer_report_without_transitioning() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = charly_heading_to_officer();
    let ctx = charly_to_officer_context(test_position(200.0, 0.0), Vec::new());

    ai.think_expected_event(
        sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.current_substate, Substate::SeekingCharlyGoToOfficer);
    assert!(matches!(
        ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
        [CrossNpcAction::ReportBackToOfficer {
            officer: 2,
            charly: 1,
        }]
    ));
    assert_eq!(ai.base.when_does_timer_ring, 0);
}

#[test]
fn accepted_officer_report_enters_seen_and_arms_ten_frame_timer() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = charly_heading_to_officer();
    let ctx = charly_to_officer_context(test_position(200.0, 0.0), Vec::new());

    ai.resolve_charly_officer_report(sim, true, &ctx, &AiPerTickData::stub());

    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingCharlyGoToOfficerSeen
    );
    assert_eq!(ai.base.when_does_timer_ring, 110);
    assert_eq!(
        ai.base.substate_at_last_timer_launch,
        Substate::SeekingCharlyGoToOfficerSeen
    );
}

#[test]
fn refused_officer_report_returns_charly_to_duty() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = charly_heading_to_officer();
    let ctx = charly_to_officer_context(test_position(200.0, 0.0), Vec::new());

    ai.resolve_charly_officer_report(sim, false, &ctx, &AiPerTickData::stub());

    // The refused report enters return-to-duty handling, which suspends its common
    // tail at the owner boundary so the engine can run patrol initialization
    // in between. Drain that continuation directly for the unit check.
    let resume = std::mem::take(&mut ai.base.outbox.reentrant.owner_work)
        .into_iter()
        .find_map(|work| match work {
            AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { flags, .. } => Some(flags),
            _ => None,
        })
        .expect("refused report queues the return-to-duty continuation");
    ai.resume_return_to_duty_after_patrol_init(sim, resume, &ctx, false);

    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
    assert_eq!(ai.base.antagonist, None);
}

#[test]
fn normal_detection_uses_raw_active_outside_gate_not_able_to_fight() {
    let ai = charly_heading_to_officer();
    let mut ctx = charly_to_officer_context(test_position(200.0, 0.0), Vec::new());
    let officer = Arc::make_mut(&mut ctx.entity_views)
        .get_mut(&2)
        .expect("officer view");
    officer.is_able_to_fight = false;
    officer.is_unconscious = true;
    officer.active = true;

    assert!(ai.is_detecting(2, &ctx));

    Arc::make_mut(&mut ctx.entity_views)
        .get_mut(&2)
        .expect("officer view")
        .active = false;
    assert!(!ai.is_detecting(2, &ctx));
}

#[test]
fn normal_detection_same_building_uses_exact_body_and_door_gates() {
    let ai = charly_heading_to_officer();
    let mut ctx = charly_to_officer_context(test_position(200.0, 0.0), Vec::new());
    let building = SectorHandle::new(7);
    ctx.building_sector = building;
    let officer = Arc::make_mut(&mut ctx.entity_views)
        .get_mut(&2)
        .expect("officer view");
    officer.building_sector = building;
    officer.in_building = true;
    officer.active = false;
    officer.is_able_to_fight = false;
    assert!(ai.is_detecting(2, &ctx));

    for gate in 0..3 {
        {
            let officer = Arc::make_mut(&mut ctx.entity_views)
                .get_mut(&2)
                .expect("officer view");
            officer.is_dead = gate == 0;
            officer.is_unconscious = gate == 1;
            officer.passing_door = gate == 2;
        }
        assert!(!ai.is_detecting(2, &ctx), "same-building gate {gate}");
        let officer = Arc::make_mut(&mut ctx.entity_views)
            .get_mut(&2)
            .expect("officer view");
        officer.is_dead = false;
        officer.is_unconscious = false;
        officer.passing_door = false;
    }
}

#[test]
fn normal_detection_does_not_treat_viewer_door_transit_as_building_sector() {
    let ai = charly_heading_to_officer();
    let mut ctx = charly_to_officer_context(test_position(200.0, 0.0), Vec::new());
    ctx.in_building = true;
    ctx.building_sector = None;

    assert!(ai.is_detecting(2, &ctx));
}

#[test]
fn normal_detection_projects_radius_on_target_obstacle_top_plane() {
    use crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA;

    let ai = charly_heading_to_officer();
    let target = test_position(380.0, 0.0);
    let clear_ctx = charly_to_officer_context(target, Vec::new());
    assert!(ai.is_detecting(2, &clear_ctx));

    let mut platform = SightObstacle::new(0, SIGHTOBSTACLE_PROJECTION_AREA);
    platform.obstacle_points = vec![
        ObstaclePoint {
            x: 350.0,
            y: -20.0,
            z_top: 200.0,
            z_bottom: 0.0,
        },
        ObstaclePoint {
            x: 410.0,
            y: -20.0,
            z_top: 200.0,
            z_bottom: 0.0,
        },
        ObstaclePoint {
            x: 410.0,
            y: 20.0,
            z_top: 200.0,
            z_bottom: 0.0,
        },
        ObstaclePoint {
            x: 350.0,
            y: 20.0,
            z_top: 200.0,
            z_bottom: 0.0,
        },
    ];
    platform.top_plane_points = [
        [350.0, -20.0, 200.0],
        [410.0, -20.0, 200.0],
        [350.0, 20.0, 200.0],
    ];
    platform.bottom_plane_points = [[350.0, -20.0, 0.0], [410.0, -20.0, 0.0], [350.0, 20.0, 0.0]];
    platform.rebuild_geometry();
    let mut platform_ctx = charly_to_officer_context(target, vec![platform]);
    let officer = Arc::make_mut(&mut platform_ctx.entity_views)
        .get_mut(&2)
        .expect("officer view");
    officer.elevation = 200.0;
    officer.obstacle_idx = crate::position_interface::ObstacleHandle::new(0);

    assert!(!ai.is_detecting(2, &platform_ctx));
}

#[test]
fn charly_outside_view_cone_retries_after_ten_frames() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = charly_heading_to_officer();
    // Within the 360-degree radius and unobstructed, but behind Charly.
    let ctx = charly_to_officer_context(test_position(-200.0, 0.0), Vec::new());

    ai.think_expected_event(
        sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.current_substate, Substate::SeekingCharlyGoToOfficer);
    assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
    assert_eq!(
        ai.base.outbox.actor.unalert_near_charly_seekers,
        Some(CharlySeekerTarget::SelfNpc)
    );
    assert_eq!(ai.base.when_does_timer_ring, 110);
    assert_eq!(
        ai.base.substate_at_last_timer_launch,
        Substate::SeekingCharlyGoToOfficer
    );
}

#[test]
fn charly_cannot_report_through_opaque_obstruction_and_retries() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = charly_heading_to_officer();
    let ctx =
        charly_to_officer_context(test_position(200.0, 0.0), vec![opaque_wall_across_x_axis()]);

    ai.think_expected_event(
        sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.current_substate, Substate::SeekingCharlyGoToOfficer);
    assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
    assert_eq!(ai.base.when_does_timer_ring, 110);
    assert_eq!(
        ai.base.substate_at_last_timer_launch,
        Substate::SeekingCharlyGoToOfficer
    );
}

fn run_find_door_authorization_case(
    door_type: DoorType,
    active: bool,
    locked_npc_villain: bool,
    building_full: bool,
    actor_is_rider: bool,
) -> EnemyAi {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let center = Position {
        x: 0.0,
        y: 0.0,
        sector: SectorHandle::new(7),
        level: 0,
    };
    let position_in = Position {
        x: 50.0,
        y: 60.0,
        sector: SectorHandle::new(8),
        level: 2,
    };
    let point_out = MapPoint::new(10.0, 0.0);

    let door = Door {
        door_type,
        active,
        locked_npc_villain,
        ..Default::default()
    };

    let mut global = AiGlobalState::default();
    global.door_seek_infos.push(DoorSeekInfo {
        door_index: DoorIndex::new(0).expect("valid door index"),
        door_type,
        point_out,
        position_in,
        sector_out: 7,
        sector_out_index: None,
        sector_in: 8,
        layer_out: 0,
        npc_villain_authorized_direct: cache_npc_villain_authorized_direct(&door),
    });
    let occupant_ids = if building_full {
        vec![EntityId::Soldier(SoldierId(0)); usize::from(u16::MAX)]
    } else {
        Vec::new()
    };
    global.houses.push(House {
        sector_index: 8,
        occupant_ids,
        ..House::default()
    });

    let mut ai = EnemyAi::new(1);
    let ctx = AiContext {
        frame: 100,
        camp: Camp::Lacklandists,
        in_building: true,
        building_sector: SectorHandle::new(9),
        self_is_rider: actor_is_rider,
        ..AiContext::test_fixture()
    };
    let seek_direction =
        crate::position_interface::vector_to_sector_0_to_15_iso(point_out.x, point_out.y) as u16;

    ai.seek_area(
        sim,
        center,
        0,
        SeekFlags::HOUSE | SeekFlags::LOCATION_FIRST,
        seek_direction,
        &mut global,
        &ctx,
        &AiPerTickData::stub(),
    );

    // The indoor caller must enter the three-frame watching delay after
    // selecting the personal seek point, regardless of authorization.
    // This pins the exact state/timer ordering around the door decision.
    assert_eq!(ai.my_seek_points, vec![1111]);
    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingSeekpointWatchingSidewards
    );
    assert!(ai.base.timer_is_running);
    assert_eq!(ai.base.when_does_timer_ring, 103);
    assert_eq!(
        ai.base.substate_at_last_timer_launch,
        Substate::SeekingSeekpointWatchingSidewards
    );

    ai
}

#[test]
fn find_door_enemy_could_be_behind_applies_every_original_authorization_gate() {
    let center = Position {
        x: 0.0,
        y: 0.0,
        sector: SectorHandle::new(7),
        level: 0,
    };
    let behind_door = Position {
        x: 50.0,
        y: 60.0,
        sector: SectorHandle::new(8),
        level: 2,
    };

    let cases = [
        (
            "authorized",
            DoorType::Building,
            true,
            false,
            false,
            false,
            behind_door,
        ),
        (
            "building type",
            DoorType::Default,
            true,
            false,
            false,
            false,
            center,
        ),
        (
            "active state",
            DoorType::Building,
            false,
            false,
            false,
            false,
            center,
        ),
        (
            "building capacity",
            DoorType::Building,
            true,
            false,
            true,
            false,
            center,
        ),
        (
            "rider",
            DoorType::Building,
            true,
            false,
            false,
            true,
            center,
        ),
        (
            "villain lock",
            DoorType::Building,
            true,
            true,
            false,
            false,
            center,
        ),
    ];

    for (name, door_type, active, locked, full, rider, expected) in cases {
        let ai = run_find_door_authorization_case(door_type, active, locked, full, rider);
        assert_eq!(ai.seek_center, expected, "{name} gate");
        assert_eq!(
            ai.personal_seek_point_1
                .as_ref()
                .map(|point| point.position),
            Some(expected),
            "{name} gate must be applied before the personal point is created"
        );
    }
}

#[test]
fn enemy_ai_defaults() {
    let ai = EnemyAi::new(42);
    assert_eq!(ai.base.me, 42);
    assert_eq!(ai.current_task_priority, task_priority::NONE);
    assert_eq!(ai.base.current_state, AiState::Default);
    assert!(!ai.tower_guard);
    assert!(!ai.combat_trainer);
}

#[test]
fn repeated_directed_panic_preserves_existing_red_alert_until_engine_boundary() {
    let mut ai = EnemyAi::new(53);
    ai.base.current_state = AiState::Fleeing;
    ai.base.current_substate = Substate::FleeingPanic;
    ai.set_alert_status(crate::ai::AlertLevel::Red);

    let center = test_position(667.0, 824.0);
    let incoming_runs = crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8;
    let existing_runs = incoming_runs.saturating_add(3);
    ai.base.lasting_panic_runs = existing_runs;
    ai.panic_from_position(center, incoming_runs);

    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
    assert_eq!(ai.base.lasting_panic_runs, existing_runs);
    assert_eq!(ai.base.view_alert_status, crate::ai::AlertLevel::Red);
    assert_eq!(
        ai.base.current_music_alert_status,
        crate::ai::AlertLevel::Red
    );
    assert!(
        ai.base.outbox.reentrant.owner_work.is_empty(),
        "Original skips state change when panic begins from fleeing panic"
    );
    let request = ai
        .base
        .outbox
        .actor
        .begin_panic
        .expect("repeated panic still reaches the engine door/search boundary");
    assert_eq!(request.center, Some(center));
    assert_eq!(request.runs, incoming_runs);
    assert!(!request.is_new_panic);
}

#[test]
fn reinitialize_them_list_preserves_order_and_omits_all_unavailable_observations() {
    use crate::ai_entity_view::AiObservationUnavailable;
    let mut ai = EnemyAi::new(1);
    ai.list_them = vec![99];
    ai.base.primary_target = Some(AiEntityHandle::new(3));
    let mut views = AiEntityViewMap::new();
    views.insert(2, soldier_view(test_position(0.0, 0.0)));
    views.insert(6, soldier_view(test_position(0.0, 0.0)));
    let mut dead = soldier_view(test_position(0.0, 0.0));
    dead.is_dead = true;
    views.insert(7, dead);
    let mut ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        self_seen_enemy_handles: vec![6, 3, 4, 5, 7, 2, 6],
        ..AiContext::test_fixture()
    };
    let snapshot = std::sync::Arc::get_mut(&mut ctx.entity_views).unwrap();
    snapshot
        .unavailable_entities
        .insert(4, AiObservationUnavailable::MissingLayer);
    snapshot
        .unavailable_entities
        .insert(5, AiObservationUnavailable::ExcludedEntity);
    let original_seen = ctx.self_seen_enemy_handles.clone();

    ai.reinitialize_them_list(&ctx, &AiPerTickData::stub());

    assert_eq!(ai.list_them, vec![6, 2, 6]);
    assert_eq!(
        ctx.self_seen_enemy_handles, original_seen,
        "do not mutate detectable retention"
    );
    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(3)));
}

#[test]
fn unavailable_owner_preserves_battle_and_target_selection_skip_policy() {
    use crate::ai_entity_view::AiObservationUnavailable;
    for reason in [
        None,
        Some(AiObservationUnavailable::MissingLayer),
        Some(AiObservationUnavailable::ExcludedEntity),
    ] {
        let mut ctx = AiContext::test_fixture();
        if let Some(reason) = reason {
            std::sync::Arc::make_mut(&mut ctx.entity_views)
                .unavailable_entities
                .insert(1, reason);
        }
        let mut ai = EnemyAi::new(1);
        ai.list_them = vec![2, 3];
        ai.base.current_state = AiState::Attacking;
        ai.base.primary_target = Some(AiEntityHandle::new(2));
        let before = bitcode::encode(&ai);
        let tick = AiPerTickData::stub();
        assert_eq!(
            ai.get_new_primary_target(PrimaryTargetFlags::VIPS_ALLOWED, &ctx, &tick),
            None
        );
        ai.battle_decisions(
            &crate::sim_rng::test_context(),
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
            None,
        );
        assert_eq!(
            bitcode::encode(&ai),
            before,
            "unavailable owner must not mutate AI: {reason:?}"
        );
    }
}

#[test]
fn reinitialize_them_list_does_not_preserve_unseen_primary_target() {
    let mut ai = EnemyAi::new(1);
    ai.base.primary_target = Some(AiEntityHandle::new(2));
    ai.list_them = vec![2, 3];

    ai.reinitialize_them_list(&AiContext::test_fixture(), &AiPerTickData::stub());

    assert!(ai.list_them.is_empty());
    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(2)));
}

#[test]
fn set_state() {
    let mut ai = EnemyAi::new(1);
    ai.set_state(AiState::Attacking, Substate::AttackingSwordfight);
    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
    // The transition queues an inline FilterAIEvent notification
    // for the post-think dispatcher to drain (matching the reference
    // enemy state transition).
    let [AiOwnerWork::StateChange(notification)] = ai.base.outbox.reentrant.owner_work.as_slice()
    else {
        panic!("expected one state-change notification");
    };
    assert_eq!(notification.outgoing_state, AiState::Default);
    assert_eq!(notification.outgoing_substate, Substate::DefaultOnPost);
    assert_eq!(notification.incoming_state, AiState::Attacking);
    assert_eq!(
        notification.incoming_substate,
        Substate::AttackingSwordfight
    );
    assert_eq!(notification.source, AiStateChangeSource::Null);
    assert!(notification.actor_effects_before_callback.is_none());
}

#[test]
fn patrol_coordinate_uses_enemy_virtual_state_before_walk_and_run() {
    for (distance, expected_substate, expected_order) in [
        (
            45.0,
            Substate::DefaultPatrolEnroute,
            crate::order::OrderType::WalkingUpright,
        ),
        (
            60.0,
            Substate::DefaultPatrolEnrouteRunning,
            crate::order::OrderType::RunningUpright,
        ),
    ] {
        let mut ai = EnemyAi::new(1);
        ai.base.patrol_chief = Some(crate::element::EntityId::Soldier(
            crate::entity_id::SoldierId(2),
        ));
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultOnPost;
        ai.attentive = true;
        ai.will_be_attentive = true;
        ai.base.current_music_alert_status = AlertLevel::Yellow;
        ai.base.view_alert_status = AlertLevel::Yellow;

        let ctx = AiContext {
            position: Position {
                x: 100.0,
                y: 100.0,
                sector: SectorHandle::new(1),
                level: 0,
            },
            self_animation: crate::order::OrderType::WaitingAlerted,
            ..AiContext::test_fixture()
        };
        let target = Position {
            x: ctx.position.x + distance,
            ..ctx.position
        };

        ai.coordinate_patrol(
            &StimulusInfo::Position(target),
            &ctx,
            Position {
                x: ctx.position.x + 100.0,
                ..ctx.position
            },
        );

        assert_eq!(ai.base.current_state, AiState::Default);
        assert_eq!(ai.base.current_substate, expected_substate);
        assert_eq!(ai.base.current_music_alert_status, AlertLevel::Green);
        assert_eq!(ai.base.view_alert_status, AlertLevel::Green);

        let [AiOwnerWork::StateChange(notification)] =
            ai.base.outbox.reentrant.owner_work.as_slice()
        else {
            panic!("patrol state change must retain the stop-all prefix");
        };
        let prefix = notification
            .actor_effects_before_callback
            .as_ref()
            .expect("stop-all must precede the state-change callback");
        assert!(prefix.halt);

        let attentive = ai
            .base
            .outbox
            .actor
            .set_attentive_mode
            .expect("default state change must request leaving attentive mode");
        assert!(!attentive.target);
        let [order] = ai.base.outbox.actor.orders.as_slice() else {
            panic!("patrol coordinate must queue one replacement movement");
        };
        assert_eq!(order.order_type, expected_order);
        assert!(
            order.after_attentive_mode,
            "movement must remain behind the LeaveAttentiveMode element"
        );
    }

    let mut already_unalerted = EnemyAi::new(1);
    already_unalerted.base.patrol_chief = Some(crate::element::EntityId::Soldier(
        crate::entity_id::SoldierId(2),
    ));
    already_unalerted.base.current_state = AiState::Default;
    already_unalerted.base.current_substate = Substate::DefaultPatrolEnroute;
    let ctx = AiContext {
        position: Position {
            x: 100.0,
            y: 100.0,
            sector: SectorHandle::new(1),
            ..Position::default()
        },
        ..AiContext::test_fixture()
    };
    already_unalerted.coordinate_patrol(
        &StimulusInfo::Position(Position {
            x: ctx.position.x + 45.0,
            ..ctx.position
        }),
        &ctx,
        Position {
            x: ctx.position.x + 100.0,
            ..ctx.position
        },
    );
    let [order] = already_unalerted.base.outbox.actor.orders.as_slice() else {
        panic!("already-unalerted patrol update must retain its movement");
    };
    assert!(
        !order.after_attentive_mode,
        "a no-change attentive-mode request must not defer movement instruction"
    );
}

#[test]
fn set_state_rejects_mismatched_numeric_substate_family() {
    // Verify the family predicate used by `debug_assert_eq!` without
    // adding a runtime rejection in release builds.
    assert_eq!(
        Substate::SleepingForever.ai_state_family(),
        Some(AiState::Sleeping)
    );
    assert_ne!(
        Substate::SleepingForever.ai_state_family(),
        Some(AiState::Default)
    );
}

#[test]
fn archery_release_is_outbox_work_and_special_strike_remains_a_latch() {
    let mut ai = EnemyAi::new(1);
    ai.my_shooting_point = Some((2, 3));
    ai.my_archery_sector = Some(2);
    ai.pending_special_strike = true;

    ai.set_state(AiState::Default, Substate::DefaultOnPost);

    assert_eq!(ai.my_shooting_point, None);
    assert_eq!(ai.my_archery_sector, Some(2));
    assert_eq!(
        ai.base.outbox.actor.archery_reservation_release,
        ArcheryReservationRelease {
            shooting_point: Some(ReservedShootingPoint {
                sector_index: 2,
                point_index: crate::sector::ArcheryPointIdx(3),
            }),
            release_sector: true,
        }
    );

    assert!(ai.pending_special_strike);
}

#[test]
fn special_strike_latch_tracks_preparation_and_in_flight_lifecycle() {
    let mut ai = EnemyAi::new(1);
    ai.set_state(AiState::Attacking, Substate::AttackingSwordfight);
    ai.base.outbox.reentrant.owner_work.clear();

    ai.begin_special_strike();
    assert!(ai.pending_special_strike);
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingSwordfightSpecialStrike
    );

    ai.reconcile_special_strike(true, 40);
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingSwordfightSpecialStrike
    );

    for locks in [
        crate::ai::AiLockFlags::BUSY,
        crate::ai::AiLockFlags::FREEZE,
        crate::ai::AiLockFlags::BUSY | crate::ai::AiLockFlags::FREEZE,
    ] {
        let mut locked = EnemyAi::new(1);
        locked.set_state(AiState::Attacking, Substate::AttackingSwordfight);
        locked.base.outbox.reentrant.owner_work.clear();
        locked.begin_special_strike();
        locked.base.locks_flag_field = locks;
        locked.reconcile_special_strike(false, 41);
        assert!(locked.pending_special_strike, "locks={locks:?}");
        assert_eq!(
            locked.base.current_substate,
            Substate::AttackingSwordfightSpecialStrike,
            "all non-script AI locks must retain the EventDone-driven substate edge: {locks:?}"
        );
    }

    ai.base.non_script_lock(crate::ai::AiLockFlags::FREEZE);
    let sim = crate::sim_rng::test_context();
    let mut global = AiGlobalState::default();
    let ctx = AiContext {
        frame: 41,
        ..AiContext::test_fixture()
    };
    let tick = AiPerTickData::stub();
    ai.think(
        &sim,
        &Stimulus::new(StimulusType::EventDone),
        &mut global,
        &ctx,
        &tick,
        None,
    );
    ai.reconcile_special_strike(false, 41);
    assert!(ai.pending_special_strike);
    assert_eq!(
        ai.base
            .stimulus_queue
            .iter()
            .map(|stimulus| stimulus.stimulus_type)
            .collect::<Vec<_>>(),
        vec![StimulusType::EventDone],
        "decision entry must retain the terminal event while Strangle holds FREEZE"
    );
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingSwordfightSpecialStrike,
        "AILOCK_FREEZE must retain the EventDone-driven substate edge"
    );
    ai.base.non_script_unlock(crate::ai::AiLockFlags::FREEZE);
    let retained = ai.base.stimulus_queue.remove(0);
    let ctx = AiContext {
        frame: 42,
        ..AiContext::test_fixture()
    };
    ai.think(&sim, &retained, &mut global, &ctx, &tick, None);
    assert!(!ai.pending_special_strike);
    assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
    assert_eq!(ai.next_sword_strike_frame, 62);

    ai.begin_special_strike();
    ai.set_state(AiState::Attacking, Substate::AttackingSwordfightParade);
    ai.reconcile_special_strike(false, 62);
    assert!(!ai.pending_special_strike);
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingSwordfightParade,
        "cancellation cleanup must preserve a newer combat reaction"
    );
}

#[test]
fn guarded_pc_relationship_uses_typed_optional_ids_and_delta() {
    let mut ai = EnemyAi::new(1);
    let guarded = PcId(17);

    ai.set_guarded_pc(Some(guarded));
    assert_eq!(ai.guarded_pc, Some(guarded));
    assert_eq!(
        ai.base.outbox.actor.set_guarded_pc,
        Some(GuardedPcEffect {
            old: None,
            new: Some(guarded),
        })
    );

    ai.set_guarded_pc(None);
    assert_eq!(ai.guarded_pc, None);
    assert_eq!(
        ai.base.outbox.actor.set_guarded_pc,
        Some(GuardedPcEffect {
            old: Some(guarded),
            new: None,
        })
    );

    let encoded = serde_json::to_string(&ai).expect("serialize typed guard relationship");
    let decoded: EnemyAi =
        serde_json::from_str(&encoded).expect("deserialize typed guard relationship");
    assert_eq!(decoded.guarded_pc, None);
    assert_eq!(
        decoded.base.outbox.actor.set_guarded_pc,
        Some(GuardedPcEffect {
            old: Some(guarded),
            new: None,
        })
    );
}

#[test]
fn return_to_duty_resets() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingSwordfight;
    ai.current_task_priority = task_priority::ENEMY;
    let ctx = AiContext::test_fixture();
    let tick = AiPerTickData::stub();
    ai.return_to_duty(sim, DutyFlags::empty(), &ctx, &tick);

    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert!(!ai.base.needs_patrol_reinit);
    assert!(matches!(
        ai.base.outbox.reentrant.owner_work.as_slice(),
        [AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. }]
    ));

    ai.resume_return_to_duty_after_patrol_init(sim, DutyFlags::empty(), &ctx, false);
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.current_task_priority, task_priority::NONE);
}

#[test]
fn return_to_duty_releases_archery_reservation_through_virtual_set_state() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(40);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingArcherWaitOnArcheryPath;
    ai.my_archery_sector = Some(1);
    ai.my_shooting_point = Some((1, 5));

    ai.resume_return_to_duty_after_patrol_init(
        &sim,
        DutyFlags::empty(),
        &AiContext::test_fixture(),
        false,
    );

    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.my_shooting_point, None);
    assert_eq!(ai.my_archery_sector, Some(1));
    let release = ai.base.outbox.actor.archery_reservation_release;
    assert!(release.release_sector);
    let point = release
        .shooting_point
        .expect("return-to-duty must release the occupied shooting point");
    assert_eq!(point.sector_index, 1);
    assert_eq!(u16::from(point.point_index), 5);
}

#[test]
fn return_to_duty_deletes_beggars_added_earlier_in_same_dispatch() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    let target = EntityId::Pc(PcId(171));
    ai.base
        .outbox
        .actor
        .add_detectable((target, DetectableType::Beggar));
    ai.base
        .outbox
        .actor
        .add_detectable((target, DetectableType::Enemy));

    ai.return_to_duty(
        &sim,
        DutyFlags::empty(),
        &AiContext::test_fixture(),
        &AiPerTickData::stub(),
    );

    assert_eq!(
        &ai.base.outbox.actor.detectable_mutations[..3],
        &[
            crate::ai::DetectableMutation::Add(target, DetectableType::Beggar),
            crate::ai::DetectableMutation::Add(target, DetectableType::Enemy),
            crate::ai::DetectableMutation::DeleteType(DetectableType::Beggar),
        ]
    );
    assert!(
        ai.base
            .outbox
            .actor
            .deleted_detectable_types()
            .contains(&DetectableType::Beggar)
    );
}

#[test]
fn return_to_duty_virtual_timer_reset_precedes_common_bored_timer_launch() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.base.timer_is_running = true;
    ai.base.when_does_timer_ring = 999;
    ai.base.likes_to_sit_around = true;
    let ctx = AiContext {
        frame: 254,
        posture: Posture::Sitting,
        position: ai.base.initial_position,
        ..AiContext::test_fixture()
    };

    ai.resume_return_to_duty_after_patrol_init(&sim, DutyFlags::empty(), &ctx, false);

    assert_eq!(ai.base.current_substate, Substate::DefaultOnPost);
    assert!(
        ai.base.timer_is_running,
        "the common tail's later timer launch must win over the state change's reset"
    );
    assert!((324..394).contains(&ai.base.when_does_timer_ring));
}

#[test]
fn return_to_duty_virtual_timer_reset_stays_cleared_without_common_launch() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.base.timer_is_running = true;
    ai.base.when_does_timer_ring = 999;

    ai.resume_return_to_duty_after_patrol_init(
        &sim,
        DutyFlags::empty(),
        &AiContext::test_fixture(),
        false,
    );

    assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
    assert!(
        !ai.base.timer_is_running,
        "state change must still clear the old timer when the common tail launches none"
    );
}

#[test]
fn high_recursion_return_to_duty_keeps_close_point_as_latch_after_deferred_resume() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(90);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.base.think_recursion_depth = 100;
    let ctx = AiContext {
        position: ai.base.initial_position,
        self_animation: OrderType::WaitingAlerted,
        ..AiContext::test_fixture()
    };

    ai.return_to_duty(&sim, DutyFlags::empty(), &ctx, &AiPerTickData::stub());
    let (flags, high_recursion_failsafe) = std::mem::take(&mut ai.base.outbox.reentrant.owner_work)
        .into_iter()
        .find_map(|work| match work {
            AiOwnerWork::ResumeHighRecursionReturnToDutyAfterPatrolInit { flags, .. } => {
                Some((flags, true))
            }
            _ => None,
        })
        .expect("high-recursion ReturnToDuty must queue its common tail");
    assert!(high_recursion_failsafe);

    // Rust releases the AI borrow for patrol initialization and may not resume
    // this owner work until the deferred recursion stack has unwound.
    ai.base.think_recursion_depth = 0;
    ai.base.open_end_think_frames = 0;
    ai.resume_return_to_duty_after_patrol_init(&sim, flags, &ctx, high_recursion_failsafe);

    assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
    assert!(ai.base.outbox.reentrant.self_stimuli.is_empty());
    assert!(ai.base.already_on_point);
    assert!(!ai.base.completion_latch_inside_think);
    assert_eq!(ai.base.think_recursion_depth, 0);
}

#[test]
fn return_to_duty_marks_only_its_new_orders_behind_real_attentive_transition() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.attentive = true;
    ai.will_be_attentive = true;
    let post = Position {
        x: 200.0,
        y: 100.0,
        sector: SectorHandle::new(1),
        level: 0,
    };
    let here = Position { x: 100.0, ..post };
    ai.base.initial_position = post;
    ai.base
        .outbox
        .actor
        .orders
        .push(crate::order::AiOrderIntent::new(
            OrderType::Turning,
            10.0,
            0.0,
        ));
    let ctx = AiContext {
        position: here,
        self_animation: OrderType::WaitingAlerted,
        ..AiContext::test_fixture()
    };

    ai.resume_return_to_duty_after_patrol_init(&sim, DutyFlags::empty(), &ctx, false);

    let [preexisting, return_to_post] = ai.base.outbox.actor.orders.as_slice() else {
        panic!("expected the pre-existing control and one return-to-duty move")
    };
    assert!(!preexisting.after_attentive_mode);
    assert_eq!(return_to_post.order_type, OrderType::WalkingUpright);
    assert!(
        return_to_post.after_attentive_mode,
        "Original state change launches attentive-mode exit before its following movement"
    );

    let mut already_unalerted = EnemyAi::new(1);
    already_unalerted.base.initial_position = post;
    already_unalerted.resume_return_to_duty_after_patrol_init(
        &sim,
        DutyFlags::empty(),
        &AiContext {
            position: here,
            ..AiContext::test_fixture()
        },
        false,
    );
    let [order] = already_unalerted.base.outbox.actor.orders.as_slice() else {
        panic!("already-unalerted return must still author its move")
    };
    assert!(
        !order.after_attentive_mode,
        "a no-change attentive-mode request launches no transition to wait behind"
    );
}

#[test]
fn return_to_duty_detects_patrol_chief_at_raw_body_but_approaches_door_endpoint() {
    let sim = crate::sim_rng::test_context();
    let chief_id = crate::element::EntityId::Soldier(crate::entity_id::SoldierId(2));
    let gate_endpoint = test_position(1_000.0, 0.0);
    let mut chief = soldier_view(gate_endpoint);
    chief.passing_door = true;
    chief.detection_position_world = crate::coordinates::WorldPoint3D::new(100.0, 0.0, 0.0);

    let mut views = AiEntityViewMap::new();
    views.insert(chief_id.index(), chief);
    let ctx = AiContext {
        position: test_position(0.0, 0.0),
        self_body_position_world: crate::coordinates::WorldPoint3D::ZERO,
        self_upright_eye_world: crate::coordinates::WorldPoint3D::new(0.0, 0.0, 45.0),
        self_view_radius: 200,
        sq_self_view_radius: 200.0 * 200.0,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut ai = EnemyAi::new(1);
    ai.base.patrol_chief = Some(chief_id);

    ai.base
        .return_to_duty_common_stuff(&sim, DutyFlags::empty(), &ctx);

    assert_eq!(ai.base.current_substate, Substate::DefaultGotoChief);
    let [order] = ai.base.outbox.actor.orders.as_slice() else {
        panic!("detecting the chief must queue the approach");
    };
    assert_eq!((order.target_x, order.target_y), (1_000.0, 0.0));
}

#[test]
fn return_to_duty_does_not_detect_far_raw_chief_at_near_door_endpoint() {
    let sim = crate::sim_rng::test_context();
    let chief_id = crate::element::EntityId::Soldier(crate::entity_id::SoldierId(2));
    let mut chief = soldier_view(test_position(100.0, 0.0));
    chief.passing_door = true;
    chief.detection_position_world = crate::coordinates::WorldPoint3D::new(1_000.0, 0.0, 0.0);

    let mut views = AiEntityViewMap::new();
    views.insert(chief_id.index(), chief);
    let ctx = AiContext {
        position: test_position(0.0, 0.0),
        self_body_position_world: crate::coordinates::WorldPoint3D::ZERO,
        self_upright_eye_world: crate::coordinates::WorldPoint3D::new(0.0, 0.0, 45.0),
        self_view_radius: 200,
        sq_self_view_radius: 200.0 * 200.0,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut ai = EnemyAi::new(1);
    ai.base.patrol_chief = Some(chief_id);

    ai.base
        .return_to_duty_common_stuff(&sim, DutyFlags::empty(), &ctx);

    assert_ne!(ai.base.current_substate, Substate::DefaultGotoChief);
    assert!(
        ai.base
            .outbox
            .actor
            .orders
            .iter()
            .all(|order| (order.target_x, order.target_y) != (100.0, 0.0)),
        "the near AI Position endpoint must not admit the raw-far chief"
    );
}

#[test]
fn return_to_duty_remembered_ale_saves_patrol_return_point() {
    let sim = crate::sim_rng::test_context();
    let here = Position {
        x: 125.0,
        y: 250.0,
        sector: SectorHandle::new(4),
        level: 2,
    };
    let ale_position = Position {
        x: 400.0,
        y: 500.0,
        sector: SectorHandle::new(7),
        level: 0,
    };
    let ale = 77;
    let mut views = AiEntityViewMap::new();
    views.insert(ale, soldier_view(ale_position));
    let ctx = AiContext {
        position: here,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingDrinkingAle;
    ai.other_seen_ale.push(ale);

    ai.return_to_duty(&sim, DutyFlags::empty(), &ctx, &AiPerTickData::stub());

    assert_eq!(ai.base.current_state, AiState::Wondering);
    assert_eq!(ai.base.current_substate, Substate::WonderingApproachingAle);
    assert_eq!(ai.base.interesting_object, Some(AiEntityHandle::new(ale)));
    assert_eq!(ai.return_to_patrol_point, here);
    assert_eq!(ai.base.last_goto_destination, ale_position);
}

#[test]
fn one_point_enemy_path_dispatches_virtual_return_before_patrol_init_resume() {
    use crate::ai::{PathId, PatrolPath};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let paths = vec![RawHikingPath {
        waypoints: vec![RawWaypoint {
            x: 699,
            y: 1464,
            sector: 50,
            level: 1,
            command: WaypointCommand::None,
        }],
    }];
    let mut ai = EnemyAi::new(142);
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultGotoRouteTurn;
    ai.base.has_patrol_path = true;
    ai.base.patrol_path = PatrolPath::new(PathId::new(0).unwrap(), &paths);
    let ctx = AiContext {
        position: Position {
            x: 698.99304,
            y: 1464.0072,
            sector: SectorHandle::new(50),
            level: 1,
        },
        direction: 3,
        self_is_soldier: true,
        posture: Posture::Upright,
        self_action_state: crate::element::ActionState::Waiting,
        self_animation: OrderType::NonanimationEnd,
        hiking_paths: Arc::new(paths),
        ..AiContext::test_fixture()
    };
    let sim = crate::sim_rng::test_context();

    ai.base
        .think_expected_event_common_stuff(&sim, &Stimulus::new(StimulusType::EventDone), &ctx);

    let virtual_requests = std::mem::take(&mut ai.base.outbox.reentrant.owner_work);
    assert!(matches!(
        virtual_requests.as_slice(),
        [AiOwnerWork::VirtualReturnToDuty {
            flags,
            owner_boundary_positions
        }] if flags.is_empty() && owner_boundary_positions.is_empty()
    ));
    assert!(
        !ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .any(|work| matches!(work, AiOwnerWork::ResumeReturnToDutyAfterPatrolInit { .. })),
        "the common controller must not skip the Enemy ReturnToDuty override"
    );

    ai.return_to_duty(&sim, DutyFlags::empty(), &ctx, &AiPerTickData::stub());

    assert!(matches!(
        ai.base.outbox.reentrant.owner_work.as_slice(),
        [AiOwnerWork::ResumeReturnToDutyAfterPatrolInit {
            flags,
            owner_boundary_positions,
            ..
        }] if flags.is_empty()
            && owner_boundary_positions.is_empty()
    ));
}

#[test]
fn return_to_duty_virtual_state_tail_clears_guarded_pc() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    let guarded = PcId(17);
    ai.base.current_state = AiState::Menacing;
    ai.base.current_substate = Substate::MenacingPcInComa;
    ai.guarded_pc = Some(guarded);

    ai.resume_return_to_duty_after_patrol_init(
        &sim,
        DutyFlags::empty(),
        &AiContext::test_fixture(),
        false,
    );

    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.guarded_pc, None);
    assert_eq!(
        ai.base.outbox.actor.set_guarded_pc,
        Some(GuardedPcEffect {
            old: Some(guarded),
            new: None,
        }),
        "the owner-boundary drain must clear the PC's reciprocal guard before later NPCs scan"
    );
}

#[test]
fn return_to_duty_virtual_state_tail_clears_combat_neighbours() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(223);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingWatching;
    ai.left_combat_neighbour = Some(AiEntityHandle::new(225));
    ai.right_combat_neighbour = Some(AiEntityHandle::new(227));

    ai.resume_return_to_duty_after_patrol_init(
        &sim,
        DutyFlags::empty(),
        &AiContext::test_fixture(),
        false,
    );

    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.left_combat_neighbour, None);
    assert_eq!(ai.right_combat_neighbour, None);
    assert!(
        matches!(
            ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
            [
                CrossNpcAction::SetRightCombatNeighbour {
                    target: 225,
                    neighbour: None,
                },
                CrossNpcAction::SetLeftCombatNeighbour {
                    target: 227,
                    neighbour: None,
                },
            ]
        ),
        "enemy state change must publish both reciprocal unlinks at the owner boundary"
    );
}

#[test]
fn return_to_duty_clears_shield_pair_before_bearer_protection_timer() {
    let sim = crate::sim_rng::test_context();
    let mut archer = EnemyAi::new(64);
    archer.is_archer_unit = true;
    archer.base.current_state = AiState::Attacking;
    archer.base.current_substate = Substate::AttackingBowShooting;
    archer.shield_bearer_before_me = Some(AiEntityHandle::new(58));

    archer.resume_return_to_duty_after_patrol_init(
        &sim,
        DutyFlags::empty(),
        &AiContext::test_fixture(),
        false,
    );

    assert_eq!(archer.base.current_state, AiState::Default);
    assert_eq!(archer.shield_bearer_before_me, None);
    let reciprocal = std::mem::take(&mut archer.base.outbox.reentrant.cross_npc_actions);
    assert!(matches!(
        reciprocal.as_slice(),
        [CrossNpcAction::SetArcherBehindMe {
            target: 58,
            archer: None
        }]
    ));

    // Drain the reciprocal write before the later shield-bearer owner. Its
    // next protection timer must now take Original's danger-over path into
    // battle-overview evaluation, rather than the stale archer-behind-me arm.
    let mut bearer = EnemyAi::new(58);
    bearer.base.current_state = AiState::Attacking;
    bearer.base.current_substate = Substate::AttackingProtectingWithShield;
    bearer.base.primary_target = Some(AiEntityHandle::new(101));
    bearer.archer_behind_me = Some(AiEntityHandle::new(64));
    for action in reciprocal {
        if let CrossNpcAction::SetArcherBehindMe { target, archer } = action {
            assert_eq!(target, bearer.base.me);
            bearer.archer_behind_me = archer;
        }
    }
    assert_eq!(bearer.archer_behind_me, None);

    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.push(FighterSnapshot {
        handle: 58,
        action_state: crate::element::ActionState::HoldingShield,
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.push(FighterSnapshot {
        handle: 101,
        action_state: crate::element::ActionState::Waiting,
        ..FighterSnapshot::default()
    });
    bearer.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &AiContext::test_fixture(),
        &tick,
        None,
    );

    assert_eq!(
        bearer.base.current_substate,
        Substate::AttackingOverviewLookLeft
    );
    assert_eq!(
        bearer.base.outbox.actor.look_sidewards,
        Some(crate::ai::LookDirection::Left)
    );
}

#[test]
fn return_to_duty_clears_archer_pair_when_bearer_leaves_protection() {
    let sim = crate::sim_rng::test_context();
    let mut bearer = EnemyAi::new(58);
    bearer.base.current_state = AiState::Attacking;
    bearer.base.current_substate = Substate::AttackingProtectingWithShield;
    bearer.archer_behind_me = Some(AiEntityHandle::new(64));

    bearer.resume_return_to_duty_after_patrol_init(
        &sim,
        DutyFlags::empty(),
        &AiContext::test_fixture(),
        false,
    );

    assert_eq!(bearer.base.current_state, AiState::Default);
    assert_eq!(bearer.archer_behind_me, None);
    assert!(matches!(
        bearer.base.outbox.reentrant.cross_npc_actions.as_slice(),
        [CrossNpcAction::SetShieldBearerBeforeMe {
            target: 64,
            shield_bearer: None,
        }]
    ));
}

#[test]
fn seek_flags() {
    let flags = SeekFlags::BODY_SEEK | SeekFlags::LOOK_FOR_HELP_AFTER;
    assert!(flags.contains(SeekFlags::BODY_SEEK));
    assert!(flags.contains(SeekFlags::LOOK_FOR_HELP_AFTER));
    assert!(!flags.contains(SeekFlags::HOUSE));
}

#[test]
fn able_to_help_matches_original_state_gates() {
    assert!(soldier_is_able_to_help_state(
        true,
        AiState::Default,
        Substate::None
    ));
    assert!(soldier_is_able_to_help_state(
        true,
        AiState::Wondering,
        Substate::WonderingMoneyReactiontime
    ));
    assert!(soldier_is_able_to_help_state(
        true,
        AiState::Seeking,
        Substate::SeekingRunningToOfficer
    ));
    assert!(!soldier_is_able_to_help_state(
        true,
        AiState::Seeking,
        Substate::SeekingSeekpoint
    ));
    assert!(!soldier_is_able_to_help_state(
        true,
        AiState::Attacking,
        Substate::AttackingSwordfight
    ));
    assert!(!soldier_is_able_to_help_state(
        false,
        AiState::Default,
        Substate::None
    ));
}

#[test]
fn tower_guard_defers_battle_decisions_until_alert_calls_return() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.tower_guard = true;
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingTowerGuardAlert;
    ai.base.seek_position = test_position(120.0, 80.0);

    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventDone),
        &mut AiGlobalState::default(),
        &AiContext::test_fixture(),
        &AiPerTickData::stub(),
        None,
    );

    assert!(matches!(
        ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
        [CrossNpcAction::ResumeTowerGuardBattleDecisions { caller: 1 }]
    ));
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingTowerGuardAlert,
        "battle planning must not run before the synchronous recipient batch"
    );
}

#[test]
fn officer_detection_uses_officer_facing() {
    let officer = CampSoldierInfo {
        handle: 2,
        active: true,
        position: Position {
            x: 0.0,
            y: 0.0,
            sector: None,
            level: 0,
        },
        position_world: crate::coordinates::WorldPoint3D::ZERO,
        direction: 4,
        rank: ProfileRank::Officer,
        ai_state: AiState::Default,
        ai_substate: Substate::None,
        is_able_to_fight: true,
        is_dead: false,
        knocked_out_in_money_fight: false,
        primary_target: None,
        pride: 0,
        is_able_to_help: true,
        script_locked: false,
        ai_lock_frozen: false,
        layer: 0,
        report_type: ReportType::Nothing,
        report_seek_position: Position::default(),
        report_seen_bodies: Vec::new(),
        report_charly: None,
        alert_soldiers_point: Position::default(),
        patrol_chief: None,
        antagonist: None,
        detected_body: None,
        blood_alcohol: 0,
        duty_flag: false,
        is_tower_guard: false,
        company_number: 0,
        in_building: false,
        forecast_destination: Some(crate::ai::PreparedForecastDestination::fixed(
            Position::default(),
            0,
        )),
        detectable_bodies: Vec::new(),
        seek_position: Position::default(),
        current_task_priority: 0,
        minimal_task_priority: 0,
        view_direction: [1.0, 0.0],
        view_radius: 400,
        real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
        eye_blind: false,
    };
    let ahead = Position {
        x: 100.0,
        y: 0.0,
        sector: None,
        level: 0,
    };
    let behind = Position {
        x: -100.0,
        y: 0.0,
        sector: None,
        level: 0,
    };

    assert!(soldier_detects_position_180(&officer, ahead, 350.0 * 350.0));
    assert!(!soldier_detects_position_180(
        &officer,
        behind,
        350.0 * 350.0
    ));
}

#[test]
fn task_priority_ordering() {
    const { assert!(task_priority::ENEMY > task_priority::BODY) };
    const { assert!(task_priority::BODY > task_priority::SEEKING) };
    const { assert!(task_priority::ALERT_IGNORE_ENEMY > task_priority::ENEMY) };
}

#[test]
fn start_think_allows_normal_events() {
    let mut ai = EnemyAi::new(1);
    let ctx = AiContext::test_fixture();
    let stimulus = Stimulus::new(StimulusType::EventTimer);
    assert!(ai.start_think(&stimulus, &ctx, false));
    assert_eq!(ai.base.think_recursion_depth, 1);
}

#[test]
fn enter_swordfight_event_does_not_reenter_swordfight() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = EnemyAi::new(1);
    let mut global = AiGlobalState::default();
    let ctx = AiContext::test_fixture();
    let tick = AiPerTickData::stub();
    let stimulus = Stimulus::with_human(StimulusType::EventEnterSwordfight, 2);

    let _ = ai.think(sim, &stimulus, &mut global, &ctx, &tick, None);

    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(2)));
    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
    assert_eq!(ai.base.outbox.actor.enter_swordfight, None);
}

#[test]
fn start_think_blocks_when_script_locked() {
    let mut ai = EnemyAi::new(1);
    ai.base.script_locked = true;
    ai.base.remember_events = true;
    let ctx = AiContext::test_fixture();
    let stimulus = Stimulus::new(StimulusType::EventView);
    assert!(!ai.start_think(&stimulus, &ctx, false));
    assert_eq!(ai.base.stimulus_queue.len(), 1);
}

#[test]
fn start_think_retains_ailock_freeze() {
    let mut ai = EnemyAi::new(1);
    ai.base.locks_flag_field = AiLockFlags::FREEZE;
    let ctx = AiContext::test_fixture();
    let stimulus = Stimulus::new(StimulusType::EventTimer);
    assert!(!ai.start_think(&stimulus, &ctx, false));
    assert_eq!(ai.base.stimulus_queue.len(), 1);
    assert_eq!(
        ai.base.stimulus_queue[0].stimulus_type,
        StimulusType::EventTimer
    );
}

#[test]
fn start_think_discards_static_ai_freeze() {
    let mut ai = EnemyAi::new(1);
    let ctx = AiContext::test_fixture();
    let stimulus = Stimulus::new(StimulusType::EventTimer);
    assert!(!ai.start_think(&stimulus, &ctx, true));
    assert!(ai.base.stimulus_queue.is_empty());
}

#[test]
fn start_think_rejects_look_there_for_physically_unconscious_script_driven_actor() {
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultScriptDriven;
    let ctx = AiContext {
        self_is_unconscious: true,
        ..AiContext::test_fixture()
    };

    assert!(!ai.start_think(&Stimulus::new(StimulusType::CallLookThere), &ctx, false,));
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultScriptDriven);
    assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
    assert!(
        ai.base
            .ai_log
            .last()
            .is_some_and(|line| line.line_type == LogLineType::EventRefused && line.info == 8)
    );
}

#[test]
fn periodic_timer_restart_obeys_static_ai_freeze() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = EnemyAi::new(1);
    ai.base.current_substate = Substate::AttackingObserve;
    let ctx = AiContext::test_fixture();
    let tick = AiPerTickData::stub();
    let mut global = AiGlobalState {
        freeze: true,
        ..AiGlobalState::default()
    };

    ai.the_16th_frame(
        sim, 0, &ctx, &global, &tick, None, false, false, false, false,
    );
    assert!(!ai.base.timer_is_running);

    global.freeze = false;
    ai.the_16th_frame(
        sim, 0, &ctx, &global, &tick, None, false, false, false, false,
    );
    assert!(ai.base.timer_is_running);
}

#[test]
fn periodic_bored_roll_reads_live_animation_not_action_change_history() {
    let sim = crate::sim_rng::test_context();
    let seed_before = sim.seed();
    let mut ai = EnemyAi::new(1);
    let mut stale_view = soldier_view(Position::default());
    stale_view.current_animation = OrderType::Invalid;
    let mut views = AiEntityViewMap::new();
    views.insert(1, stale_view);
    let ctx = AiContext {
        self_animation: OrderType::WaitingUprightBored,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    ai.the_16th_frame(
        &sim,
        16,
        &ctx,
        &AiGlobalState::default(),
        &AiPerTickData::stub(),
        None,
        true,
        false,
        true,
        false,
    );

    assert_ne!(
        sim.seed(),
        seed_before,
        "animation query must consume the bored-roll draw from the live sprite action"
    );
}

#[test]
fn periodic_smalltalk_command_advances_reachpoint_stuck_counter() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.base.current_substate = Substate::AttackingMovingAroundOldEnemy;
    ai.base.stuck_counter = 2;
    let ctx = AiContext::test_fixture();
    let global = AiGlobalState::default();
    let tick = AiPerTickData::stub();

    ai.the_16th_frame(
        &sim, 0, &ctx, &global, &tick, None, false, false, true, false,
    );
    assert_eq!(
        ai.base.stuck_counter, 3,
        "Original monitors smalltalk strike/parry commands while waiting for EVENT_REACHPOINT"
    );

    ai.the_16th_frame(
        &sim, 0, &ctx, &global, &tick, None, false, false, false, false,
    );
    assert_eq!(
        ai.base.stuck_counter, 3,
        "an unrelated command in the same movement substate leaves the counter untouched"
    );
}

#[test]
fn periodic_post_refresh_queued_goto_suppresses_stuck_counter() {
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToPhalanx;
    ai.base.stuck_counter = 2;

    // Arrow-protection refresh has already launched direct movement. At this
    // boundary the movement element is queued for launch, so the pending
    // movement check succeeds and the watchdog resets rather than
    // counting the actor's still-selected Wait command.
    ai.the_16th_frame_after_refresh(0, &AiContext::test_fixture(), true, true);

    assert_eq!(ai.base.stuck_counter, 0);
}

#[test]
fn periodic_post_refresh_without_queued_element_keeps_idle_increment() {
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToPhalanx;
    ai.base.stuck_counter = 2;

    // An already-on-point movement returns before sequence launch, while a
    // denied route deletes its candidate sequence. Neither case leaves a
    // manager element to suppress the live Wait/smalltalk arm.
    ai.the_16th_frame_after_refresh(0, &AiContext::test_fixture(), true, false);

    assert_eq!(ai.base.stuck_counter, 3);
}

#[test]
fn periodic_phalanx_goto_does_not_hide_same_call_idle_actor() {
    // schema14 seed1000000, linux2/P002/Savegame_032/replay-008,
    // frame 17254. Arrow-protection refresh changes the soldier from
    // Reactiontime/Wait to RunningToPhalanx and launches movement, but the
    // same-call stuck check still observes the actor's current Wait and
    // advances its counter. Rust must not substitute its deferred order
    // for that live actor/sequence-manager observation.
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.base.stuck_counter = 2;
    ai.list_them.push(2);

    let exact_position = |x, y| Position {
        sector: SectorHandle::new(0),
        ..test_position(x, y)
    };
    let owner_position = exact_position(500.0, 500.0);
    let enemy_position = exact_position(1_500.0, 500.0);
    let mut owner_view = soldier_view(owner_position);
    owner_view.camp = Camp::Royalists;
    let mut enemy_view = soldier_view(enemy_position);
    enemy_view.camp = Camp::Lacklandists;
    enemy_view.action_state = crate::element::ActionState::AimingWithBow;
    let mut views = AiEntityViewMap::new();
    views.insert(1, owner_view);
    views.insert(2, enemy_view);
    let ctx = AiContext {
        position: owner_position,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let mut tick = AiPerTickData::stub();
    tick.seen_last_frame_enemies.push(2);
    tick.fighter_registry.push(FighterSnapshot {
        handle: 1,
        position: owner_position,
        raw_position: owner_position,
        is_friendly: true,
        is_soldier: true,
        is_shield_bearer: true,
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.push(FighterSnapshot {
        handle: 2,
        position: enemy_position,
        raw_position: enemy_position,
        is_able_to_fight: true,
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.push(FighterSnapshot {
        handle: 3,
        position: exact_position(600.0, 500.0),
        raw_position: exact_position(600.0, 500.0),
        direction: 0,
        is_friendly: true,
        is_soldier: true,
        is_shield_bearer: true,
        current_substate: Substate::AttackingPhalanx,
        ..FighterSnapshot::default()
    });

    ai.the_16th_frame(
        &sim,
        0,
        &ctx,
        &AiGlobalState::default(),
        &tick,
        None,
        true,
        false,
        true,
        false,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRunningToPhalanx
    );
    assert_ne!(ai.base.last_goto_destination, Position::default());
    assert_eq!(
        ai.base.stuck_counter, 3,
        "the deferred phalanx movement does not hide the current idle actor"
    );
}

#[test]
fn periodic_phalanx_goto_does_not_fake_wait_during_attentive_transition() {
    // Seed3 linux2/P002/Savegame_030/replay-007 frame 6887. The shield
    // refresh queues phalanx movement while EnterAttentive remains the
    // selected command. Original's subsequent command dispatch does not
    // enter the Wait/smalltalk stuck-counter arm.
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.base.stuck_counter = 0;
    ai.list_them.push(2);

    let exact_position = |x, y| Position {
        sector: SectorHandle::new(0),
        ..test_position(x, y)
    };
    let owner_position = exact_position(500.0, 500.0);
    let enemy_position = exact_position(1_500.0, 500.0);
    let mut owner_view = soldier_view(owner_position);
    owner_view.camp = Camp::Royalists;
    let mut enemy_view = soldier_view(enemy_position);
    enemy_view.camp = Camp::Lacklandists;
    enemy_view.action_state = crate::element::ActionState::AimingWithBow;
    let mut views = AiEntityViewMap::new();
    views.insert(1, owner_view);
    views.insert(2, enemy_view);
    let ctx = AiContext {
        position: owner_position,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let mut tick = AiPerTickData::stub();
    tick.seen_last_frame_enemies.push(2);
    tick.fighter_registry.push(FighterSnapshot {
        handle: 1,
        position: owner_position,
        raw_position: owner_position,
        is_friendly: true,
        is_soldier: true,
        is_shield_bearer: true,
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.push(FighterSnapshot {
        handle: 2,
        position: enemy_position,
        raw_position: enemy_position,
        is_able_to_fight: true,
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.push(FighterSnapshot {
        handle: 3,
        position: exact_position(600.0, 500.0),
        raw_position: exact_position(600.0, 500.0),
        direction: 0,
        is_friendly: true,
        is_soldier: true,
        is_shield_bearer: true,
        current_substate: Substate::AttackingPhalanx,
        ..FighterSnapshot::default()
    });

    ai.the_16th_frame(
        &sim,
        0,
        &ctx,
        &AiGlobalState::default(),
        &tick,
        None,
        false,
        false,
        false,
        false,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRunningToPhalanx
    );
    assert!(!ai.base.outbox.actor.orders.is_empty());
    assert_eq!(
        ai.base.stuck_counter, 0,
        "a shield action must not substitute for selected-command classification"
    );
}

#[test]
fn periodic_phalanx_already_on_point_does_not_fake_a_pending_sequence() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.base.stuck_counter = 2;
    ai.list_them.push(2);

    // Direction zero puts the open left slot 25 pixels to the left of
    // the existing shield bearer. The owner is already exactly there,
    // so arrow-protection refresh's movement completes without registering a
    // movement sequence.
    let exact_position = |x, y| Position {
        sector: SectorHandle::new(0),
        ..test_position(x, y)
    };
    let owner_position = exact_position(575.0, 500.0);
    let enemy_position = exact_position(1_500.0, 500.0);
    let mut owner_view = soldier_view(owner_position);
    owner_view.camp = Camp::Royalists;
    owner_view.current_animation = OrderType::WaitingUpright;
    let mut enemy_view = soldier_view(enemy_position);
    enemy_view.camp = Camp::Lacklandists;
    enemy_view.action_state = crate::element::ActionState::AimingWithBow;
    let mut views = AiEntityViewMap::new();
    views.insert(1, owner_view);
    views.insert(2, enemy_view);
    let ctx = AiContext {
        position: owner_position,
        self_animation: OrderType::WaitingUpright,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let mut tick = AiPerTickData::stub();
    tick.seen_last_frame_enemies.push(2);
    tick.fighter_registry.push(FighterSnapshot {
        handle: 1,
        position: owner_position,
        raw_position: owner_position,
        is_friendly: true,
        is_soldier: true,
        is_shield_bearer: true,
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.push(FighterSnapshot {
        handle: 2,
        position: enemy_position,
        raw_position: enemy_position,
        is_able_to_fight: true,
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.push(FighterSnapshot {
        handle: 3,
        position: exact_position(600.0, 500.0),
        raw_position: exact_position(600.0, 500.0),
        direction: 0,
        is_friendly: true,
        is_soldier: true,
        is_shield_bearer: true,
        current_substate: Substate::AttackingPhalanx,
        ..FighterSnapshot::default()
    });

    ai.the_16th_frame(
        &sim,
        0,
        &ctx,
        &AiGlobalState::default(),
        &tick,
        None,
        true,
        false,
        true,
        false,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRunningToPhalanx
    );
    assert!(ai.base.outbox.actor.orders.is_empty());
    assert!(
        ai.base
            .outbox
            .reentrant
            .self_stimuli
            .iter()
            .any(|stimulus| stimulus.stimulus_type == StimulusType::EventReachPoint)
    );
    assert_eq!(
        ai.base.stuck_counter, 3,
        "already-on-point movement leaves no pending sequence to suppress the original game's counter"
    );
}

#[test]
fn start_think_handles_lose_consciousness() {
    let mut ai = EnemyAi::new(1);
    let ctx = AiContext::test_fixture();
    let stimulus = Stimulus::new(StimulusType::EventLoseConsciousness);
    assert!(!ai.start_think(&stimulus, &ctx, false));
    assert_eq!(ai.base.current_state, AiState::Sleeping);
    assert_eq!(ai.base.current_substate, Substate::SleepingUnconscious);
}

#[test]
fn forget_attentive_events_preserve_the_forced_script_latch() {
    for stimulus_type in [
        StimulusType::EventLoseConsciousness,
        StimulusType::EventWasp,
        StimulusType::EventNet,
    ] {
        for forced_attentive in [false, true] {
            let sim = crate::sim_rng::test_context();
            let mut ai = EnemyAi::new(1);
            ai.attentive = true;
            ai.will_be_attentive = true;
            ai.forced_attentive = forced_attentive;
            let mut global = AiGlobalState::default();
            let ctx = AiContext::test_fixture();
            let tick = AiPerTickData::stub();

            ai.think(
                &sim,
                &Stimulus::new(stimulus_type),
                &mut global,
                &ctx,
                &tick,
                None,
            );

            assert!(!ai.attentive, "{stimulus_type:?}");
            assert!(!ai.will_be_attentive, "{stimulus_type:?}");
            assert_eq!(
                ai.forced_attentive, forced_attentive,
                "{stimulus_type:?} must not clear the script-owned latch"
            );
            assert!(
                ai.base
                    .outbox
                    .actor
                    .set_attentive_mode
                    .is_some_and(|request| request.forget_after),
                "{stimulus_type:?} must retain the state change's attentive transition before forgetting its flags"
            );

            if stimulus_type == StimulusType::EventLoseConsciousness {
                ai.think(
                    &sim,
                    &Stimulus::new(StimulusType::EventFitAgain),
                    &mut global,
                    &ctx,
                    &tick,
                    None,
                );
                assert_eq!(ai.base.current_substate, Substate::SleepingAwakening);
                assert_eq!(ai.forced_attentive, forced_attentive);
            }
        }
    }
}

#[test]
fn start_think_blocks_dead() {
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Sleeping;
    ai.base.current_substate = Substate::SleepingForever;
    let ctx = AiContext {
        self_is_dead: true,
        ..AiContext::test_fixture()
    };
    let stimulus = Stimulus::new(StimulusType::EventLoseConsciousness);
    assert!(!ai.start_think(&stimulus, &ctx, false));
    assert_eq!(ai.base.current_state, AiState::Sleeping);
    assert_eq!(ai.base.current_substate, Substate::SleepingForever);
    assert_eq!(ai.base.outbox.recovery.set_eye_status, None);
}

#[test]
fn start_think_blocks_fitagain_when_carried() {
    let mut ai = EnemyAi::new(1);
    ai.base.current_substate = Substate::SleepingUnconscious;
    let ctx = AiContext {
        posture: crate::element::Posture::Carried,
        ..AiContext::test_fixture()
    };
    let stimulus = Stimulus::new(StimulusType::EventFitAgain);
    assert!(!ai.start_think(&stimulus, &ctx, false));
}

#[test]
fn update_task_priority_maps_correctly() {
    let mut ai = EnemyAi::new(1);
    let s = Stimulus::new(StimulusType::EventView);
    ai.update_new_task_priority(&s);
    assert_eq!(ai.new_task_priority, task_priority::ENEMY);

    let s = Stimulus::new(StimulusType::EventSeesBody);
    ai.update_new_task_priority(&s);
    assert_eq!(ai.new_task_priority, task_priority::BODY);
}

#[test]
fn watching_for_more_money_skips_looted_victims_and_marks_next() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = EnemyAi::new(1);
    ai.base.me = 1;
    ai.set_state(AiState::Wondering, Substate::WonderingWatchingForMoreMoney);

    let me = test_position(0.0, 0.0);
    let mut looted = soldier_view(test_position(10.0, 0.0));
    looted.is_able_to_fight = false;
    looted.is_unconscious = true;
    looted.looted_after_money_fight = true;
    let mut unlooted = soldier_view(test_position(20.0, 0.0));
    unlooted.is_able_to_fight = false;
    unlooted.is_unconscious = true;

    let mut views = AiEntityViewMap::new();
    views.insert(1, soldier_view(me));
    views.insert(2, looted);
    views.insert(3, unlooted);
    let ctx = AiContext {
        position: me,
        sq_standard_view_radius: 500.0 * 500.0,
        sq_self_view_radius: 500.0 * 500.0,
        move_box: crate::coordinates::MoveBox::from_coords(-5.0, -5.0, 5.0, 5.0),
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.camp_unconscious_soldiers = vec![2, 3]
        .into_iter()
        .map(|handle| CampUnconsciousSoldierInfo {
            handle,
            knocked_out_in_money_fight: true,
        })
        .collect();
    let mut global = AiGlobalState::default();

    let stimulus = Stimulus::new(StimulusType::EventDone);
    let _ = ai.think(sim, &stimulus, &mut global, &ctx, &tick, None);

    assert_eq!(ai.base.detected_body, Some(AiEntityHandle::new(3)));
    assert_eq!(
        ai.base.current_substate,
        Substate::WonderingApproachingToLoot
    );
    assert!(matches!(
        ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
        [CrossNpcAction::SetLootedAfterMoneyFight {
            target: 3,
            looted: true
        }]
    ));
}

#[test]
fn run_to_examine_body_uses_stuck_under_net_cover_info() {
    let mut ai = EnemyAi::new(1);
    ai.base.me = 1;
    let me = test_position(0.0, 0.0);
    let body = test_position(40.0, 0.0);
    let net = test_position(42.0, 0.0);

    let mut victim = soldier_view(body);
    victim.is_able_to_fight = false;
    victim.is_unconscious = true;
    victim.stuck_under_net = true;
    victim.covering_nets.push(NetCoverInfo {
        handle: 77,
        position: net,
        radius: 40.0,
    });

    let mut views = AiEntityViewMap::new();
    views.insert(1, soldier_view(me));
    views.insert(2, victim);
    let ctx = AiContext {
        position: me,
        sq_standard_view_radius: 500.0 * 500.0,
        sq_self_view_radius: 500.0 * 500.0,
        move_box: crate::coordinates::MoveBox::from_coords(-5.0, -5.0, 5.0, 5.0),
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    ai.run_to_examine_body(2, &ctx, &AiPerTickData::stub(), None);

    assert_eq!(ai.base.detected_body, Some(AiEntityHandle::new(2)));
    assert_eq!(ai.base.interesting_object, Some(AiEntityHandle::new(77)));
    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(ai.base.current_substate, Substate::SeekingNet);
}

#[test]
#[should_panic(expected = "soldier 1 cannot examine missing body 77")]
fn run_to_examine_body_rejects_a_missing_required_body() {
    let mut ai = EnemyAi::new(1);
    ai.run_to_examine_body(77, &AiContext::test_fixture(), &AiPerTickData::stub(), None);
}

#[test]
fn make_battle_predecisions_returns_valid() {
    crate::sim_rng::with_seed(1, |sim| {
        let mut ai = EnemyAi::new(1);
        ai.list_them.push(99);
        ai.base.list_us.push(1);
        let mut views = AiEntityViewMap::new();
        views.insert(1, soldier_view(Position::default()));
        let ctx = AiContext {
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::test_fixture()
        };
        let tick = AiPerTickData::stub();
        let d = ai.make_battle_predecisions(sim, &ctx, &tick);
        assert!(d == Decision::PredecisionOffensive || d == Decision::PredecisionDefensive);
    });
}

#[test]
fn answer_question_task_priority() {
    let ctx = AiContext::test_fixture();
    let mut ai = EnemyAi::new(1);
    // Equal priorities → HasTheNewTaskPriority is true.
    assert!(ai.answer_question(Question::HasTheNewTaskPriority, &ctx));
    // Lower new priority while Seeking → false.
    ai.base.current_state = AiState::Seeking;
    ai.current_task_priority = 50;
    ai.new_task_priority = 10;
    assert!(!ai.answer_question(Question::HasTheNewTaskPriority, &ctx));
    // Lower new priority in Default state with NONE minimal → true.
    ai.base.current_state = AiState::Default;
    ai.minimal_task_priority = task_priority::NONE;
    assert!(ai.answer_question(Question::HasTheNewTaskPriority, &ctx));
}

#[test]
fn send_out_soldier_uses_live_patrol_not_theoretical_patrol() {
    let ctx = AiContext {
        self_is_active: true,
        in_building: false,
        ..AiContext::test_fixture()
    };
    let mut ai = EnemyAi::new(1);
    ai.soldier_profile_initiative = 60;

    // Answering the question checks the patrol list here.
    // A save may retain a theoretical patrol after its live patrol has
    // emptied; that must not make the officer delegate body examination.
    ai.base
        .theoretical_patrol
        .push(EntityId::Soldier(crate::entity_id::SoldierId(2)));
    assert!(!ai.answer_question(Question::ShallISendOutSoldier, &ctx));

    ai.base
        .patrol
        .push(EntityId::Soldier(crate::entity_id::SoldierId(2)));
    assert!(ai.answer_question(Question::ShallISendOutSoldier, &ctx));
}

#[test]
fn hard_reaction_time_fix_selects_the_intended_multiplier() {
    let ctx = AiContext {
        difficulty: crate::player_profile::DifficultyLevel::Hard,
        camp: crate::element::Camp::Lacklandists,
        frame: 10,
        ..AiContext::test_fixture()
    };

    let mut original = EnemyAi::new(1);
    original.soldier_profile_iq = 50;
    original.react(100, &ctx, &AiPerTickData::stub());
    assert_eq!(original.base.when_does_timer_ring, 111);

    let mut fixed_tick = AiPerTickData::stub();
    fixed_tick.fix_hard_reaction_times = true;
    let mut fixed = EnemyAi::new(1);
    fixed.soldier_profile_iq = 50;
    fixed.react(100, &ctx, &fixed_tick);
    assert_eq!(fixed.base.when_does_timer_ring, 36);
}

#[test]
fn get_new_primary_target_empty() {
    let mut ai = EnemyAi::new(1);
    let ctx = AiContext::test_fixture();
    let tick = AiPerTickData::stub();
    assert_eq!(
        ai.get_new_primary_target(PrimaryTargetFlags::empty(), &ctx, &tick),
        None
    );
}

#[test]
fn leaving_phalanx_clears_reciprocal_combat_neighbour_links() {
    let mut ai = EnemyAi::new(74);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingPhalanx;
    ai.left_combat_neighbour = Some(AiEntityHandle::new(75));
    ai.right_combat_neighbour = Some(AiEntityHandle::new(72));

    ai.set_state(AiState::Attacking, Substate::AttackingOverviewLookLeft);

    assert_eq!(ai.left_combat_neighbour, None);
    assert_eq!(ai.right_combat_neighbour, None);
    assert!(matches!(
        ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
        [
            CrossNpcAction::SetRightCombatNeighbour {
                target: 75,
                neighbour: None
            },
            CrossNpcAction::SetLeftCombatNeighbour {
                target: 72,
                neighbour: None
            }
        ]
    ));
}

#[test]
fn entering_phalanx_preserves_preassigned_combat_neighbour_links() {
    let mut ai = EnemyAi::new(73);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingOverviewLookLeft;
    ai.left_combat_neighbour = Some(AiEntityHandle::new(72));

    ai.set_state(AiState::Attacking, Substate::AttackingRunningToPhalanx);

    assert_eq!(ai.left_combat_neighbour, Some(AiEntityHandle::new(72)));
    assert_eq!(ai.right_combat_neighbour, None);
    assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
}

#[test]
fn running_to_phalanx_preserves_existing_neighbours_null_primary_target() {
    let mut ai = EnemyAi::new(78);
    ai.right_combat_neighbour = Some(AiEntityHandle::new(70));
    ai.list_them = vec![170];

    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.push(FighterSnapshot {
        handle: 70,
        is_soldier: true,
        primary_target: None,
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.push(FighterSnapshot {
        handle: 170,
        is_pc: true,
        is_able_to_fight: true,
        ..FighterSnapshot::default()
    });

    assert_eq!(ai.phalanx_neighbour_primary_target(&tick), Some(None));
}

#[test]
fn running_to_phalanx_uses_right_soldier_after_non_soldier_left_neighbour() {
    let mut ai = EnemyAi::new(78);
    ai.left_combat_neighbour = Some(AiEntityHandle::new(169));
    ai.right_combat_neighbour = Some(AiEntityHandle::new(70));

    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.push(FighterSnapshot {
        handle: 169,
        is_pc: true,
        ..FighterSnapshot::default()
    });
    tick.fighter_registry.push(FighterSnapshot {
        handle: 70,
        is_soldier: true,
        primary_target: Some(AiEntityHandle::new(170)),
        ..FighterSnapshot::default()
    });

    assert_eq!(
        ai.phalanx_neighbour_primary_target(&tick),
        Some(Some(AiEntityHandle::new(170)))
    );
}

#[test]
fn get_new_primary_target_uses_live_positions_when_timer_snapshot_is_incomplete() {
    let mut ai = EnemyAi::new(1);
    ai.list_them = vec![198, 199];
    let mut views = AiEntityViewMap::new();
    let mut owner = soldier_view(test_position(0.0, 0.0));
    owner.camp = Camp::Lacklandists;
    views.insert(1, owner);
    views.insert(198, soldier_view(test_position(100.0, 0.0)));
    views.insert(199, soldier_view(test_position(110.0, 0.0)));
    let ctx = AiContext {
        position: test_position(0.0, 0.0),
        camp: Camp::Lacklandists,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    // Off-detection timer contexts historically cached only the old
    // primary target. Original still scores every persistent list entry
    // from its live position.
    tick.enemy_sq_distances = vec![(198, 10_000)];
    tick.primary_target_multiplicity = vec![(198, 1)];

    let target = ai.get_new_primary_target(
        PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
        &ctx,
        &tick,
    );

    assert_eq!(target, Some(AiEntityHandle::new(199)));
}

#[test]
fn get_new_primary_target_scores_raw_world_position_not_ai_door_endpoint() {
    let mut ai = EnemyAi::new(1);
    ai.list_them = vec![170, 169];

    let mut owner = soldier_view(test_position(0.0, 0.0));
    owner.camp = Camp::Lacklandists;
    let occupied = soldier_view(test_position(60.0, 0.0));
    let mut passing_door = soldier_view(test_position(177.0, 0.0));
    passing_door.detection_position = MapPoint::new(159.0, 0.0);
    passing_door.detection_position_world = crate::coordinates::WorldPoint3D::new(159.0, 0.0, 0.0);
    passing_door.passing_door = true;

    let mut views = AiEntityViewMap::new();
    views.insert(1, owner);
    views.insert(170, occupied);
    views.insert(169, passing_door);
    let ctx = AiContext {
        position: test_position(0.0, 0.0),
        camp: Camp::Lacklandists,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.primary_target_multiplicity = vec![(170, 1), (169, 0)];

    assert_eq!(
        ai.get_new_primary_target(
            PrimaryTargetFlags::UNOCCUPIED_PREFERRED | PrimaryTargetFlags::VIPS_ALLOWED,
            &ctx,
            &tick,
        ),
        Some(AiEntityHandle::new(169))
    );
}

#[test]
fn too_proud_range_gate_uses_isometric_world_distance() {
    // Task 94 representative geometry: Soldier72 versus PC107 at frame
    // 815. Raw map max-norm is 41.82 (inside the 50-unit sword range),
    // while the isometric maximum-norm distance is 83.64 (outside).
    let me_position = test_position(621.35455, 822.2824);
    let target_position = test_position(615.2868, 780.4628);

    let mut ai = EnemyAi::new(72);
    ai.soldier_profile_pride = 1;
    ai.base.current_substate = Substate::AttackingOfficerGivingOrdersWaiting;
    ai.list_them = vec![107];

    let mut target_view = soldier_view(target_position);
    target_view.is_pc = true;
    target_view.kind = EntityKind::Pc;
    let mut views = AiEntityViewMap::new();
    let mut owner_view = soldier_view(me_position);
    owner_view.camp = Camp::Lacklandists;
    views.insert(72, owner_view);
    views.insert(107, target_view);
    let ctx = AiContext {
        position: me_position,
        self_body_position_world: crate::coordinates::WorldPoint3D::new(
            me_position.x,
            me_position.y,
            0.0,
        ),
        elevation: 0.0,
        camp: Camp::Lacklandists,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let mut tick = AiPerTickData::stub();
    tick.fighter_registry = vec![
        FighterSnapshot {
            handle: 72,
            position: me_position,
            raw_position: me_position,
            sword_range_maximal: 50,
            ..FighterSnapshot::default()
        },
        FighterSnapshot {
            handle: 107,
            position: target_position,
            raw_position: target_position,
            is_pc: true,
            ..FighterSnapshot::default()
        },
    ];

    assert!(ai.is_too_proud_to_attack(&ctx, &tick, None));
    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(107)));
}

#[test]
fn too_proud_range_gate_uses_raw_target_body_during_door_transit() {
    // Save067/r005 and Save068/r005: the target's AI Position() is the
    // committed point inside gate 108, but the original game's maximum-norm distance
    // reads the raw element position directly. The live body is within
    // this knight's sword reach, so pride must not suppress the attack.
    let me_position = test_position(2230.0, 405.0);
    let raw_target = test_position(2268.0, 393.0);
    let snapped_door_target = test_position(2301.0, 381.0);

    let mut ai = EnemyAi::new(266);
    ai.soldier_profile_pride = 80;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.list_them = vec![320];

    let mut target_view = soldier_view(snapped_door_target);
    target_view.is_pc = true;
    target_view.kind = EntityKind::Pc;
    target_view.detection_position = MapPoint::new(raw_target.x, raw_target.y);
    target_view.detection_position_world =
        crate::coordinates::WorldPoint3D::new(raw_target.x, raw_target.y, 0.0);
    target_view.passing_door = true;
    let mut owner_view = soldier_view(me_position);
    owner_view.camp = Camp::Lacklandists;
    let mut views = AiEntityViewMap::new();
    views.insert(266, owner_view);
    views.insert(320, target_view);
    let ctx = AiContext {
        position: me_position,
        self_body_position_world: crate::coordinates::WorldPoint3D::new(
            me_position.x,
            me_position.y,
            0.0,
        ),
        camp: Camp::Lacklandists,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let mut tick = AiPerTickData::stub();
    tick.fighter_registry = vec![
        FighterSnapshot {
            handle: 266,
            position: me_position,
            raw_position: me_position,
            sword_range_maximal: 50,
            ..FighterSnapshot::default()
        },
        FighterSnapshot {
            handle: 320,
            position: snapped_door_target,
            raw_position: raw_target,
            is_pc: true,
            ..FighterSnapshot::default()
        },
    ];

    assert!(!ai.is_too_proud_to_attack(&ctx, &tick, None));
    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(320)));
}

#[test]
fn too_proud_reselection_observes_live_battle_decision_multiplicity() {
    // Frame 471 regression: battle planning resets its personal
    // Them-list, adds the nearby friends' claims, and only then calls
    // the too-proud-to-attack check. Its strongly-unoccupied re-pick must read those
    // within-call mutations, not the owner-start tick snapshot.
    let me_position = test_position(0.0, 0.0);
    let nearer_position = test_position(300.0, 0.0);
    let unoccupied_position = test_position(350.0, 0.0);

    let mut ai = EnemyAi::new(114);
    ai.soldier_profile_pride = 1;
    ai.base.current_substate = Substate::AttackingObserve;
    ai.list_them = vec![171, 174];

    let mut nearer_view = soldier_view(nearer_position);
    nearer_view.is_pc = true;
    nearer_view.kind = EntityKind::Pc;
    let mut unoccupied_view = soldier_view(unoccupied_position);
    unoccupied_view.is_pc = true;
    unoccupied_view.kind = EntityKind::Pc;
    let mut views = AiEntityViewMap::new();
    let mut owner_view = soldier_view(me_position);
    owner_view.camp = Camp::Lacklandists;
    views.insert(114, owner_view);
    views.insert(171, nearer_view);
    views.insert(174, unoccupied_view);
    let ctx = AiContext {
        position: me_position,
        camp: Camp::Lacklandists,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let mut tick = AiPerTickData::stub();
    tick.primary_target_multiplicity = vec![(171, 0), (174, 1)];
    tick.fighter_registry = vec![
        FighterSnapshot {
            handle: 114,
            position: me_position,
            sword_range_maximal: 50,
            ..FighterSnapshot::default()
        },
        FighterSnapshot {
            handle: 171,
            position: nearer_position,
            is_pc: true,
            ..FighterSnapshot::default()
        },
        FighterSnapshot {
            handle: 174,
            position: unoccupied_position,
            is_pc: true,
            ..FighterSnapshot::default()
        },
    ];

    let live_decision_multiplicity = std::collections::BTreeMap::from([(171, 1), (174, 0)]);
    let _ = ai.is_too_proud_to_attack(&ctx, &tick, Some(&live_decision_multiplicity));

    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(174)));
}

fn perpendicular_out_of_view_context(stare_y: f32) -> AiContext {
    AiContext {
        frame: 920,
        position: test_position(1_546.658_2, 318.299_56),
        // `enemy_is_behind_me` reads the raw ground-space body
        // point, which for this ground-level fixture coincides with the
        // AI position.
        self_body_position_world: crate::coordinates::WorldPoint3D {
            x: 1_546.658_2,
            y: 318.299_56,
            z: 0.0,
        },
        direction: 14,
        self_stare_point: crate::coordinates::GroundPoint::new(1_500.696_3, stare_y),
        ..AiContext::test_fixture()
    }
}

#[test]
fn direction_table_keeps_exact_perpendicular_stare_in_front() {
    let ai = EnemyAi::new(111);
    let ctx = perpendicular_out_of_view_context(344.662_23);

    assert!(
        !ai.enemy_is_behind_me(&ctx),
        "Original's literal direction-14 vector produces an exact-zero dot product"
    );
}

#[test]
fn direction_table_still_rejects_stare_slightly_behind() {
    let ai = EnemyAi::new(111);
    let ctx = perpendicular_out_of_view_context(344.672_24);

    assert!(ai.enemy_is_behind_me(&ctx));
}

#[test]
fn queued_out_of_view_does_not_substitute_actor_principal_for_ai_target() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(111);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingSwordfight;
    ai.base.primary_target = Some(AiEntityHandle::new(84));
    ai.list_them = vec![84, 171];

    let target_position = test_position(5.0, 0.0);
    let mut target = soldier_view(target_position);
    target.kind = EntityKind::Pc;
    target.is_pc = true;
    let mut views = AiEntityViewMap::new();
    views.insert(171, target);
    let mut ctx = perpendicular_out_of_view_context(344.672_24);
    ctx.self_upright_eye_world = crate::coordinates::WorldPoint3D::new(0.0, 0.0, 45.0);
    ctx.sq_self_view_radius = 100.0 * 100.0;
    ctx.entity_views = crate::ai_entity_view::shared_entity_views(views);
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry.push(FighterSnapshot {
        handle: 111,
        principal_opponent: Some(AiEntityHandle::new(171)),
        ..FighterSnapshot::default()
    });
    tick.enemy_detectable_forecasts.push((
        171,
        crate::ai::PreparedForecastDestination::fixed(target_position, 0),
    ));

    crate::sight_obstacle::begin_parity_visibility_capture();
    ai.think_unexpected_event(
        &sim,
        &Stimulus::with_human(StimulusType::EventOutOfView, 171),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );
    let visibility_queries = crate::sight_obstacle::take_parity_visibility_capture();

    assert!(
        visibility_queries.is_empty(),
        "Original compares EVENT_OUTOFVIEW with the independent AI primary target"
    );
}

#[test]
fn out_of_view_removes_non_primary_target_at_perpendicular_boundary() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(111);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingSwordfight;
    ai.base.primary_target = Some(AiEntityHandle::new(84));
    ai.list_them = vec![84, 171];

    let mut primary = soldier_view(test_position(1_519.443_7, 309.084_5));
    primary.kind = EntityKind::Pc;
    primary.is_pc = true;
    let mut lost = soldier_view(test_position(1_226.175_4, 315.871_6));
    lost.kind = EntityKind::Pc;
    lost.is_pc = true;
    let mut views = AiEntityViewMap::new();
    views.insert(84, primary);
    views.insert(171, lost);
    let mut ctx = perpendicular_out_of_view_context(344.662_23);
    ctx.entity_views = crate::ai_entity_view::shared_entity_views(views);
    ctx.self_seen_enemy_handles = vec![84];

    let mut tick = AiPerTickData::stub();
    tick.enemy_detectable_forecasts.push((
        171,
        crate::ai::PreparedForecastDestination::fixed(test_position(1_226.175_4, 315.871_6), 4),
    ));

    ai.think_unexpected_event(
        &sim,
        &Stimulus::with_human(StimulusType::EventOutOfView, 171),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(ai.list_them, vec![84]);
    assert_eq!(ai.missed_pc, Some(AiEntityHandle::new(171)));
}

#[test]
fn out_of_view_uses_exact_stimulus_target_forecast_after_detectable_removal() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(111);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingObserve;
    // A preceding queued Think selected another primary target before
    // this falling-edge OUTOFVIEW was delivered.
    ai.base.primary_target = Some(AiEntityHandle::new(84));
    ai.list_them = vec![171];

    let forecast_position = test_position(1_226.175_4, 315.871_6);
    let mut lost = soldier_view(forecast_position);
    lost.kind = EntityKind::Pc;
    lost.is_pc = true;
    let mut views = AiEntityViewMap::new();
    views.insert(171, lost);
    let ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let mut tick = AiPerTickData::stub();
    tick.primary_target_snapshot_handle = Some(AiEntityHandle::new(171));
    tick.primary_target_forecast = Some(crate::ai::PreparedForecastDestination::fixed(
        forecast_position,
        4,
    ));
    // The live detectable list has already dropped 171, so there is no
    // entry in `enemy_detectable_forecasts`.

    ai.think_unexpected_event(
        &sim,
        &Stimulus::with_human(StimulusType::EventOutOfView, 171),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(ai.missed_pc, Some(AiEntityHandle::new(171)));
    assert_eq!(ai.base.seek_position, forecast_position);
}
