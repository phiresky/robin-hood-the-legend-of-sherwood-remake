//! Projectile construction, per-frame flight, and impact resolution.

use super::*;

// ═══════════════════════════════════════════════════════════════════
//  Arrow spawn
// ═══════════════════════════════════════════════════════════════════

/// Parameters for spawning an arrow projectile.
pub struct SpawnArrowParams {
    pub shooter: EntityId,
    pub bow_point: WorldPoint3D,
    /// C++ `ShootWithBowAt` stores the shooter map position as
    /// `mposStartOfTrajectory` after the arrow's first `Hourglass()`.
    /// AI reactions use this origin, not the bow hand hotspot.
    pub trajectory_origin: MapPoint,
    pub target: EntityId,
    pub target_pos: MapPoint,
    pub trajectory: Vec<TrajectoryPoint>,
    pub damage: u16,
    pub layer: u16,
    /// Initial 3D velocity — `compute_initial_throw_velocity` output
    /// (after any target-leading correction).  The sprite facing is
    /// seeded from the XY of this vector, not from `target - bow` —
    /// the two diverge once leading is applied to moving targets.
    ///
    pub initial_velocity: WorldVec3D,
    /// Whether the precomputed trajectory ends inside a hole zone
    /// (before any far-edge fall-into-hole extension).  Pre-flags
    /// `ProjectileData::disappear` so `maybe_splash_on_landing` can
    /// route to the silent-disappear branch even if the extended final
    /// position tests outside the polygon due to boundary ray-cast
    /// tiebreaking.
    pub lands_in_hole: bool,
}

/// Build a new arrow projectile `Entity` for a fired shot.
///
/// Unlike the previous straight-line version, this takes a precomputed
/// ballistic trajectory and stores it on the projectile for per-frame
/// advancement in [`tick_arrows`].
pub fn spawn_arrow(params: SpawnArrowParams) -> Entity {
    let SpawnArrowParams {
        shooter,
        bow_point,
        trajectory_origin,
        target,
        target_pos: _,
        trajectory,
        damage,
        layer: _trajectory_layer,
        lands_in_hole,
        initial_velocity,
    } = params;
    let map_pos = MapPoint {
        x: bow_point.x,
        y: bow_point.y,
    };
    let end_pos = trajectory_end_or_start(&trajectory, bow_point, "arrow");

    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(map_pos);
    element.set_position(bow_point);
    // ComputeTrajectory detaches a projectile from world membership before
    // constructing its flight: layer = 0xFFFF, sector = NULL, obstacle =
    // NULL. It only restores a layer when the resolved landing point belongs
    // to a motion sector on that layer.
    element.clear_layer();
    let mut object = ObjectData {
        associated_action: Action::Bow,
        object_type: ObjectType::Arrow,
        animation: Animation::ObjectFlying,
        quantity: 1,
        ..ObjectData::default()
    };
    object.reference = Some(target);

    let projectile = ProjectileData {
        start: bow_point,
        end: end_pos,
        start_of_trajectory_x: trajectory_origin.x,
        start_of_trajectory_y: trajectory_origin.y,
        shooter: Some(shooter),
        flying: true,
        disappear: lands_in_hole,
        trajectory,
        damage,
        ..ProjectileData::default()
    };

    let mut arrow = ElementProjectile {
        element,
        object,
        projectile,
    };
    arrow.advance_trajectory_one_frame();
    arrow.projectile.flight_direction = crate::position_interface::vector_to_sector_0_to_15_iso(
        initial_velocity.x,
        initial_velocity.y,
    ) as u16;
    arrow.projectile.launch_segment_start = Some(bow_point);
    Entity::Projectile(arrow)
}

fn trajectory_end_or_start(
    trajectory: &[TrajectoryPoint],
    start: WorldPoint3D,
    projectile_kind: &'static str,
) -> WorldPoint3D {
    match trajectory.last() {
        Some(tp) => tp.position,
        None => {
            tracing::warn!(
                projectile_kind,
                ?start,
                "projectile spawn produced empty trajectory; keeping end at start"
            );
            start
        }
    }
}

/// Spawn a net projectile entity flying toward `target_pos`.
///
/// Creates an `Entity::Net` with a precomputed ballistic trajectory
/// using `MASS_NET` / `APEX_NET`.
pub fn spawn_net(
    thrower: EntityId,
    throw_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    layer: u16,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    let dx = target_pos.x - throw_pos.x;
    let dy = target_pos.y - throw_pos.y;
    let dz = target_pos.z - throw_pos.z;
    let direction_vec = WorldVec3D {
        x: dx,
        y: dy,
        z: dz,
    };

    let velocity = compute_initial_throw_velocity(direction_vec, APEX_NET, MASS_NET, 0, None);
    // Nets bounce with `(0.1, 0.1)` — heavily damped, so the net skips
    // once and settles.
    let trajectory = compute_trajectory_ballistic_bounce(
        throw_pos,
        velocity,
        MASS_NET,
        false,
        obstacle_check,
        (0.1, 0.1),
    );
    let end_pos = trajectory_end_or_start(&trajectory, throw_pos, "net");

    let map_pos = MapPoint {
        x: throw_pos.x,
        y: throw_pos.y,
    };
    let mut element = ElementData {
        kind: ElementKind::ObjectNet,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(map_pos);
    element.set_position(throw_pos);
    element.set_layer(layer);
    element.set_direction_instantly(crate::position_interface::vector_to_sector_0_to_15_iso(
        dx, dy,
    ));
    let object = ObjectData {
        associated_action: Action::Net,
        object_type: ObjectType::BonusNet,
        animation: Animation::ObjectFlying,
        quantity: 1,
        ..ObjectData::default()
    };

    // Sum the precomputed waypoint times for the net's frames-left
    // counter at spawn.  Time-till-unfolding is `frames_left - 15`,
    // clamped at a minimum of 1.
    let total_trajectory_frames: u32 = trajectory.iter().map(|p| p.time as u32).sum();
    let time_till_unfolding = total_trajectory_frames.saturating_sub(15).max(1) as u32;

    let projectile = ProjectileData {
        start: throw_pos,
        end: end_pos,
        start_of_trajectory_x: throw_pos.x,
        start_of_trajectory_y: throw_pos.y,
        shooter: Some(thrower),
        frame_count: 0,
        flying: true,
        trajectory,
        damage: 0,
        ..ProjectileData::default()
    };

    let net = crate::element::NetData {
        crumpled: false,
        was_flying: true,
        time_till_unfolding,
        ..Default::default()
    };

    let mut net_entity = crate::element::ElementNet {
        element,
        object,
        projectile,
        net,
    };
    // Advance one trajectory step before handing the net to the engine
    // so it's already one step in when the engine picks it up.
    // `detect_initial_net_crumple` runs against `projectile.end` (the
    // trajectory's last waypoint), which this primer does not modify —
    // only the first waypoint is consumed.
    net_entity.advance_trajectory_one_frame();
    Entity::Net(net_entity)
}

/// Spawn a wasp nest projectile entity flying toward `target_pos`.
///
/// Creates an `Entity::Projectile` with a ballistic trajectory using
/// `MASS_WASP_NEST` / `APEX_WASP_NEST`.
pub fn spawn_wasp_nest(
    thrower: EntityId,
    throw_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    layer: u16,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    let dx = target_pos.x - throw_pos.x;
    let dy = target_pos.y - throw_pos.y;
    let dz = target_pos.z - throw_pos.z;
    let direction_vec = WorldVec3D {
        x: dx,
        y: dy,
        z: dz,
    };

    let velocity =
        compute_initial_throw_velocity(direction_vec, APEX_WASP_NEST, MASS_WASP_NEST, 0, None);
    // Wasp nests bounce with the coin bounce factors `(0.33, 0.3)`.
    let trajectory = compute_trajectory_ballistic_bounce(
        throw_pos,
        velocity,
        MASS_WASP_NEST,
        false,
        obstacle_check,
        (0.33, 0.3),
    );
    let end_pos = trajectory_end_or_start(&trajectory, throw_pos, "wasp_nest");

    let map_pos = MapPoint {
        x: throw_pos.x,
        y: throw_pos.y,
    };
    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(map_pos);
    element.set_position(throw_pos);
    element.set_layer(layer);
    element.set_direction_instantly(crate::position_interface::vector_to_sector_0_to_15_iso(
        dx, dy,
    ));
    let object = ObjectData {
        associated_action: Action::WaspNest,
        object_type: ObjectType::BonusWaspNest,
        animation: Animation::ObjectFlying,
        quantity: 1,
        ..ObjectData::default()
    };

    let projectile = ProjectileData {
        start: throw_pos,
        end: end_pos,
        start_of_trajectory_x: throw_pos.x,
        start_of_trajectory_y: throw_pos.y,
        shooter: Some(thrower),
        frame_count: 0,
        flying: true,
        trajectory,
        damage: 0,
        ..ProjectileData::default()
    };

    let mut wasp_nest = ElementProjectile {
        element,
        object,
        projectile,
    };
    // Advance one trajectory step before handing the wasp nest to the
    // engine so it's already one step in when it joins the active
    // element list.
    wasp_nest.advance_trajectory_one_frame();
    Entity::Projectile(wasp_nest)
}

/// Number of wasps a wasp nest bursts into on impact.
pub const NUMBER_OF_WASPS: u16 = 20;

/// Spawn a wasp at `position`, attached to `nest_id`.
///
/// Copies the nest's position into the wasp and queues the
/// `BonusOne` animation.  Per-frame AI (direction change / victim
/// choice / sting) lives in `EngineInner::tick_wasp_nests`.
pub fn spawn_wasp(nest_id: EntityId, position: WorldPoint3D, layer: u16) -> Entity {
    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(MapPoint::from_world_xyz(position.x, position.y, position.z));
    element.set_position(position);
    element.set_layer(layer);

    let object = ObjectData {
        associated_action: Action::NoAction,
        object_type: ObjectType::Wasp,
        animation: Animation::BonusOne,
        quantity: 1,
        ..ObjectData::default()
    };

    let mut projectile = ProjectileData {
        start: position,
        end: position,
        start_of_trajectory_x: position.x,
        start_of_trajectory_y: position.y,
        shooter: None,
        frame_count: 0,
        // Inert projectile flag: wasps don't consume ballistic
        // trajectories (they fly under AI control in
        // `EngineInner::tick_wasp_nests`).
        flying: false,
        damage: 0,
        ..ProjectileData::default()
    };
    projectile.wasp.source_nest = Some(nest_id);

    Entity::Projectile(ElementProjectile {
        element,
        object,
        projectile,
    })
}

/// Spawn an apple projectile flying toward `target_pos`.
///
/// Creates an `Entity::Projectile` with a ballistic trajectory using
/// `MASS_APPLE` / `APEX_APPLE`.
///
/// `target_forecasted_movement`: when the victim is an NPC, callers
/// should look up `PositionInterface::get_forecasted_movement()` on
/// the NPC so the shot leads the target's current motion; pass `None`
/// for FX / static targets.
#[allow(clippy::too_many_arguments)]
pub fn spawn_apple(
    thrower: EntityId,
    throw_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    target: Option<EntityId>,
    target_forecasted_movement: Option<WorldVec3D>,
    layer: u16,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    spawn_throwable(
        thrower,
        throw_pos,
        target_pos,
        target,
        target_forecasted_movement,
        layer,
        MASS_APPLE,
        APEX_APPLE,
        0,
        Action::Apple,
        ObjectType::Apple,
        obstacle_check,
    )
}

/// Spawn a stone projectile flying toward `target_pos`.
///
/// Creates an `Entity::Projectile` with a fast near-flat ballistic
/// trajectory.  Unlike the other throwables, stones use `flight_time = 1`
/// in `compute_initial_throw_velocity`, which skips the apex-driven
/// branch and sets `velocity = 0.5 * direction` directly — so
/// `APEX_STONE` is effectively unused, but `MASS_STONE` still drives
/// the gravity applied during trajectory integration.
///
/// `target_forecasted_movement`: see `spawn_apple` for how callers
/// supply this.
#[allow(clippy::too_many_arguments)]
pub fn spawn_stone(
    thrower: EntityId,
    throw_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    target: Option<EntityId>,
    target_forecasted_movement: Option<WorldVec3D>,
    layer: u16,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    spawn_throwable(
        thrower,
        throw_pos,
        target_pos,
        target,
        target_forecasted_movement,
        layer,
        MASS_STONE,
        APEX_STONE,
        1,
        Action::Stone,
        ObjectType::Stone,
        obstacle_check,
    )
}

/// Shared spawn path for non-bouncing small throwables (apple, stone).
/// Bounce-on-landing projectiles (net, purse, wasp nest) use the
/// dedicated bounce-trajectory path.
///
/// `flight_time` is forwarded to `compute_initial_throw_velocity`.
/// Apple passes `0` (compute from apex), stone passes `1` (fast flat
/// throw, apex unused).
#[allow(clippy::too_many_arguments)]
fn spawn_throwable(
    thrower: EntityId,
    throw_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    target: Option<EntityId>,
    target_forecasted_movement: Option<WorldVec3D>,
    layer: u16,
    mass: f32,
    apex: f32,
    flight_time: u16,
    action: Action,
    object_type: ObjectType,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    let dx = target_pos.x - throw_pos.x;
    let dy = target_pos.y - throw_pos.y;
    let dz = target_pos.z - throw_pos.z;
    let direction_vec = WorldVec3D {
        x: dx,
        y: dy,
        z: dz,
    };

    let velocity = compute_initial_throw_velocity(
        direction_vec,
        apex,
        mass,
        flight_time,
        target_forecasted_movement,
    );
    let (trajectory, terminal_obstacle) = compute_trajectory_ballistic_with_terminal_obstacle(
        throw_pos,
        velocity,
        mass,
        false,
        obstacle_check,
    );
    let end_pos = trajectory_end_or_start(&trajectory, throw_pos, "throwable");

    let map_pos = MapPoint {
        x: throw_pos.x,
        y: throw_pos.y,
    };
    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(map_pos);
    element.set_position(throw_pos);
    element.set_layer(layer);
    element.set_direction_instantly(crate::position_interface::vector_to_sector_0_to_15_iso(
        dx, dy,
    ));
    if let Some(check) = obstacle_check {
        let plane = terminal_obstacle_plane(terminal_obstacle, check.sight_obstacles);
        bind_trajectory_obstacle(&mut element, terminal_obstacle, plane);
    }
    let object = ObjectData {
        associated_action: action,
        object_type,
        animation: Animation::ObjectFlying,
        quantity: 1,
        reference: target,
        ..ObjectData::default()
    };

    let projectile = ProjectileData {
        start: throw_pos,
        end: end_pos,
        start_of_trajectory_x: throw_pos.x,
        start_of_trajectory_y: throw_pos.y,
        shooter: Some(thrower),
        flying: true,
        trajectory,
        damage: 0,
        ..ProjectileData::default()
    };

    let mut throwable = ElementProjectile {
        element,
        object,
        projectile,
    };
    // Advance one trajectory step before handing the projectile to
    // the engine so it's already one step in when it joins the active
    // element list.  Without this, the projectile would wait an extra
    // frame.
    throwable.advance_trajectory_one_frame();
    Entity::Projectile(throwable)
}

// ═══════════════════════════════════════════════════════════════════
//  Purse / coin spawn
// ═══════════════════════════════════════════════════════════════════

/// Number of coins ejected on impact.  Aliased to
/// `crate::inventory::COINS_PER_PURSE` so the burst routine reads with
/// the same name as the projectile-settings constant.
pub const NUMBER_OF_COINS_IN_PURSE: u16 = crate::inventory::COINS_PER_PURSE;

/// Mass for a single coin's ballistic ejection (same as arrow-flat /
/// stone — 0.1).
pub const MASS_COIN: f32 = 0.1;

/// Coin bounce factors `(vertical, horizontal)`.
pub const BOUNCE_COIN: (f32, f32) = (0.33, 0.3);

/// Maximum random horizontal scatter for a coin's landing point, in
/// map units.  The goal vector is `unit_sector * (10 + rand() & 31)` —
/// a `[10..=41]` random magnitude before multiplying by the unit
/// sector vector.
pub const COIN_SCATTER_MIN: f32 = 10.0;
pub const COIN_SCATTER_RANGE: f32 = 32.0;

/// Apex height for a tossed coin.  The coin scatter trajectory uses
/// the small fixed apex of 3.
pub const APEX_COIN: f32 = 3.0;

/// Apex used by civilians tossing a coin to a PC-beggar — a gentler
/// arc than the purse-burst scatter.
pub const APEX_BEGGAR_COIN: f32 = 1.0;

/// Number of attempts the scatter loop makes when picking each coin's
/// landing point.
pub const COIN_SCATTER_ATTEMPTS: u32 = 7;

/// Spawn a thrown-purse projectile.
///
/// Creates an `Entity::Projectile` with `ObjectType::Purse` whose
/// ballistic trajectory uses `MASS_PURSE` / `APEX_PURSE`.  When the
/// trajectory finishes, the purse-handling tick
/// (`EngineInner::tick_purses_and_coins`) detects the impact and calls
/// into the burst routine to eject coins.
pub fn spawn_purse(
    thrower: EntityId,
    throw_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    _layer: u16,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    let dx = target_pos.x - throw_pos.x;
    let dy = target_pos.y - throw_pos.y;
    let dz = target_pos.z - throw_pos.z;
    let direction_vec = WorldVec3D {
        x: dx,
        y: dy,
        z: dz,
    };

    let velocity = compute_initial_throw_velocity(direction_vec, APEX_PURSE, MASS_PURSE, 0, None);
    let (
        trajectory,
        terminal_obstacle,
        terminal_impact,
        terminal_lands_in_hole,
        terminal_lands_in_water,
        terminal_impact_index,
    ) = compute_trajectory_ballistic_with_terminal_metadata(
        throw_pos,
        velocity,
        MASS_PURSE,
        false,
        obstacle_check,
    );
    let end_pos = trajectory_end_or_start(&trajectory, throw_pos, "purse");

    let map_pos = MapPoint {
        x: throw_pos.x,
        y: throw_pos.y,
    };
    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(map_pos);
    element.set_position(throw_pos);
    // ComputeTrajectory clears live membership before rebuilding the arc.
    element.clear_layer();
    element.set_sector(None);
    // Purse sprite direction stays at the newly-created master's default;
    // the flight direction is separate Projectile state.
    if let Some(check) = obstacle_check {
        let terminal_membership =
            terminal_impact && !terminal_lands_in_hole && !terminal_lands_in_water;
        let bound = terminal_membership.then_some(terminal_obstacle).flatten();
        bind_trajectory_obstacle(
            &mut element,
            bound,
            terminal_obstacle_plane(bound, check.sight_obstacles),
        );
        if terminal_membership && let Some(end) = trajectory.last().map(|point| point.position) {
            let resolution = if let Some(obstacle) = terminal_obstacle {
                check
                    .fast_find_grid
                    .resolve_projectile_landing_with_obstacle(
                        end.to_map(),
                        Some(obstacle),
                        check.sight_obstacles,
                    )
            } else {
                check
                    .fast_find_grid
                    .resolve_projectile_ground_landing(end.to_map())
            };
            element.set_sector(resolution.sector);
            if resolution.sector.is_some() && !resolution.blocked_by_motion_obstacle {
                element.set_layer(resolution.layer);
            }
        }
    }
    let object = ObjectData {
        associated_action: Action::Purse,
        object_type: ObjectType::Purse,
        animation: Animation::ObjectFlying,
        // The per-purse value for inventory accounting is one purse,
        // not the coin count.
        quantity: 1,
        ..ObjectData::default()
    };

    let trajectory_runtime = vec![
        crate::element::TrajectoryPointRuntime {
            bounce: false,
            material: crate::element::GameMaterial::NumberOfMaterials.as_u32(),
        };
        trajectory.len()
    ];

    let mut projectile = ProjectileData {
        start: throw_pos,
        end: end_pos,
        start_of_trajectory_x: throw_pos.x,
        start_of_trajectory_y: throw_pos.y,
        shooter: Some(thrower),
        frame_count: 0,
        flying: true,
        dive: terminal_lands_in_water,
        disappear: terminal_lands_in_hole,
        trajectory,
        trajectory_runtime,
        terminal_material_pending: terminal_impact,
        terminal_material_impact_index: terminal_impact_index.map(|index| {
            u16::try_from(index).expect("purse collision waypoint index does not fit u16")
        }),
        damage: 0,
        ..ProjectileData::default()
    };
    // Populate the purse's coin count from the bonus master during
    // creation; the impact handler later asserts
    // `>= NUMBER_OF_COINS_IN_PURSE` and decrements.
    projectile.purse.number_of_coins = NUMBER_OF_COINS_IN_PURSE;

    let purse = ElementProjectile {
        element,
        object,
        projectile,
    };
    Entity::Projectile(purse)
}

/// Spawn one coin projectile.
///
/// Two call sites share this:
///
/// * Purse-burst coins — `source_purse` is `Some(purse_id)` and `apex`
///   is [`APEX_COIN`].
/// * Civilian-tossed coins (give-money-to-beggar) — `source_purse` is
///   `None` and `apex` is [`APEX_BEGGAR_COIN`].
///
/// `target_pos` is the landing point; Original explicitly disables bounce
/// for coins. The goal layer/sector
/// are stored on the projectile so the coin can snap to them on
/// landing — see [`PurseData::layer_goal`] and
/// [`PurseData::sector_goal`].
#[allow(clippy::too_many_arguments)]
pub fn spawn_coin(
    source_purse: Option<EntityId>,
    source_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    _layer: u16,
    layer_goal: u16,
    sector_goal: Option<crate::position_interface::SectorHandle>,
    apex: f32,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    let dx = target_pos.x - source_pos.x;
    let dy = target_pos.y - source_pos.y;
    let dz = target_pos.z - source_pos.z;
    let direction_vec = WorldVec3D {
        x: dx,
        y: dy,
        z: dz,
    };

    let velocity = compute_initial_throw_velocity(direction_vec, apex, MASS_COIN, 0, None);
    let (
        trajectory,
        terminal_obstacle,
        terminal_impact,
        terminal_hole,
        terminal_water,
        terminal_impact_index,
    ) = compute_trajectory_ballistic_with_terminal_metadata(
        source_pos,
        velocity,
        MASS_COIN,
        false,
        obstacle_check,
    );
    let end_pos = trajectory_end_or_start(&trajectory, source_pos, "coin");

    let map_pos = MapPoint {
        x: source_pos.x,
        y: source_pos.y,
    };
    let mut element = ElementData {
        kind: ElementKind::ObjectProjectile,
        active: true,
        posture: Posture::Undefined,
        ..ElementData::default()
    };
    element.set_position_map(map_pos);
    element.set_position(source_pos);
    // ComputeTrajectory clears live membership. Saved goal membership lives
    // separately in PurseData and Coin::HitObstacle installs it only on an
    // ordinary dry terminal impact.
    element.clear_layer();
    element.set_sector(None);
    if let Some(check) = obstacle_check {
        let bound = (terminal_impact && !terminal_hole && !terminal_water)
            .then_some(terminal_obstacle)
            .flatten();
        bind_trajectory_obstacle(
            &mut element,
            bound,
            terminal_obstacle_plane(bound, check.sight_obstacles),
        );
        if terminal_impact
            && !terminal_hole
            && !terminal_water
            && let Some(end) = trajectory.last().map(|point| point.position)
        {
            let resolution = if let Some(obstacle) = terminal_obstacle {
                check
                    .fast_find_grid
                    .resolve_projectile_landing_with_obstacle(
                        end.to_map(),
                        Some(obstacle),
                        check.sight_obstacles,
                    )
            } else {
                check
                    .fast_find_grid
                    .resolve_projectile_ground_landing(end.to_map())
            };
            element.set_sector(resolution.sector);
            if resolution.sector.is_some() && !resolution.blocked_by_motion_obstacle {
                element.set_layer(resolution.layer);
            }
        }
    }
    let object = ObjectData {
        associated_action: Action::Purse,
        object_type: ObjectType::Coin,
        animation: Animation::ObjectFlying,
        quantity: 1,
        ..ObjectData::default()
    };

    let trajectory_runtime = vec![
        crate::element::TrajectoryPointRuntime {
            bounce: false,
            material: crate::element::GameMaterial::NumberOfMaterials.as_u32(),
        };
        trajectory.len()
    ];
    let mut projectile = ProjectileData {
        start: source_pos,
        end: end_pos,
        start_of_trajectory_x: source_pos.x,
        start_of_trajectory_y: source_pos.y,
        // Burst-spawned coins carry no shooter; their owner identity
        // flows through `source_purse` instead.  Beggar coins have
        // neither a shooter nor a source purse.
        shooter: None,
        frame_count: 0,
        flying: true,
        dive: terminal_water,
        disappear: terminal_hole,
        trajectory,
        trajectory_runtime,
        terminal_material_pending: terminal_impact,
        terminal_material_impact_index: terminal_impact_index.map(|index| {
            u16::try_from(index).expect("coin collision waypoint index does not fit u16")
        }),
        damage: 0,
        ..ProjectileData::default()
    };
    projectile.purse.source_purse = source_purse;
    projectile.purse.layer_goal = layer_goal;
    projectile.purse.sector_goal = sector_goal;

    let coin = ElementProjectile {
        element,
        object,
        projectile,
    };
    Entity::Projectile(coin)
}

// ═══════════════════════════════════════════════════════════════════
//  Per-frame arrow tick
// ═══════════════════════════════════════════════════════════════════

/// Outcome of an arrow tick — the engine applies damage and despawn
/// decisions after the mutable-borrow loop releases.
pub struct ArrowTickResult {
    pub arrow: EntityId,
    pub hit_target: Option<EntityId>,
    /// Entity whose shield blocked the arrow (mutually exclusive with
    /// `hit_target`).  When set, the engine should trigger a parry-shield
    /// animation instead of applying damage.
    pub shield_hit: Option<EntityId>,
    /// FX-target the projectile connected with (mutually exclusive
    /// with `hit_target`/`shield_hit`), paired with the activation
    /// command to dispatch.  Different projectile types launch
    /// different activation commands: arrows → `Command::ActivateArrow`,
    /// apples → `Command::ActivateApple`, stones →
    /// `Command::ActivateStone`.
    pub fx_target_hit: Option<(EntityId, Command)>,
    pub despawn: bool,
    /// Damage to apply if there's a hit.  Precomputed at spawn time
    /// from the shooter's bow profile.
    pub damage: u16,
    /// Impact sound to play at [`Self::impact_pos`] on this tick.  Set
    /// on the tick a projectile first stops flying; the engine routes
    /// it through `pending_side_effects.sounds`.  Per-type FX ids:
    /// arrow 510, apple 509, stone 508.
    pub impact_fx: Option<u32>,
    /// Map-space position of the projectile at impact, for locating
    /// the impact FX sound.  Only meaningful when `impact_fx.is_some()`.
    pub impact_pos: MapPoint,
    /// Previous 3D projectile position for C++ `SetPosition(old)` on
    /// human hits whose `HitHuman` handler returns true.  Arrows only
    /// use this in the damage branch; pass-through and ricochet keep
    /// their current position/falling trajectory.
    pub human_hit_old_position: Option<WorldPoint3D>,
}

/// Original `RHElementArrow::GetOrientations` points from the arrow's current
/// position to the *next queued trajectory point*. The current per-frame
/// increment is not equivalent: `Hourglass` has already removed the point
/// which produced that increment, and the next ballistic segment can have a
/// different vertical pitch.
fn current_arrow_orientation(proj: &mut ElementProjectile) -> (u16, i16) {
    let Some(next) = proj.projectile.trajectory.first() else {
        return (
            proj.projectile.last_orientation_sector,
            proj.projectile.last_orientation_azimuth,
        );
    };
    let current = proj.element.position();
    let dx = next.position.x - current.x;
    let dy = next.position.y - current.y;
    let dz = next.position.z - current.z;
    let norm_sq = dx * dx + dy * dy + dz * dz;
    if norm_sq == 0.0 {
        // Original does not guard SBGeoVector3D::Normalize here. On the
        // shipped i386 build, the resulting zero-segment direction flows
        // through GetSector0to15/acos and both integer conversions collapse
        // to zero. This is observable when an arrow reaches a trajectory
        // point which remains queued for another presentation refresh.
        proj.projectile.last_orientation_sector = 0;
        proj.projectile.last_orientation_azimuth = 0;
        return (0, 0);
    }

    let inv_norm = 1.0 / norm_sq.sqrt();
    let nx = dx * inv_norm;
    let ny = dy * inv_norm;
    let nz = dz * inv_norm;
    let sector = crate::position_interface::vector_to_sector_0_to_15_iso(nx, ny) as u16 & 15;
    let ground_norm = (nx * nx + ny * ny).sqrt().min(1.0);
    let mut azimuth = (ground_norm.acos() * 180.0 / std::f32::consts::PI).min(60.0) as i16;
    if nz < 0.0 {
        azimuth = -azimuth;
    }
    proj.projectile.last_orientation_sector = sector;
    proj.projectile.last_orientation_azimuth = azimuth;
    (sector, azimuth)
}

fn apply_arrow_falling_sprite_visual(
    sim: &crate::sim_rng::SimulationContext,
    proj: &mut ElementProjectile,
) {
    // C++ `RHElementArrow::Refresh` renders falling arrows with
    // `ForceSpriteRow(mubFallingDirection)`,
    // `ForceSprite(mubFallingDirection, (rand() % 3) + 3)`, then rotates
    // the row by -2 sectors for the next refresh.
    let row = proj.projectile.falling_direction;
    let frame =
        (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::ArrowFallingFrame, 0..3) as u16) + 3;
    proj.element.sprite.force_sprite_row_raw(row);
    proj.element.sprite.force_sprite(row, frame);
    proj.projectile.falling_direction = (row + 14) % 16;
}

/// Apply the presentation pass which Original runs after the parity snapshot.
/// The engine calls this immediately before the next `PerformHourglass`, which
/// exposes the same row/frame at the next snapshot and puts falling-arrow RNG
/// before that frame's simulation draws.
pub(crate) fn refresh_arrow_after_previous_hourglass(
    sim: &crate::sim_rng::SimulationContext,
    proj: &mut ElementProjectile,
) {
    if !proj.element.active {
        return;
    }
    let trajectory_empty = proj.projectile.trajectory.is_empty();
    let world_position_is_moving = proj.element.sprite.position_iface.is_moving();
    if trajectory_empty && !world_position_is_moving {
        // Original tests the empty-trajectory/settled-position retirement
        // condition before inspecting mbFlying or entering its falling-arrow
        // visual branch. A non-falling arrow can therefore still have
        // mbFlying=true after consuming a zero-length final waypoint and be
        // retired here, before its next Hourglass reaches HitObstacle.
        // Preserve the already-published endpoint sprite and retire without
        // another tumble draw.
        // Refresh retires the arrow before another Projectile::Hourglass can
        // call NewMove. Settle the exposed movement snapshot at the endpoint
        // as Original's retired sprite state records it.
        proj.element.sprite.position_iface.new_move();
        proj.element.active = false;
        return;
    }

    if proj.projectile.falling {
        apply_arrow_falling_sprite_visual(sim, proj);
    } else {
        let (sector, azimuth) = current_arrow_orientation(proj);
        let frame = ((azimuth as f32 * 0.066_666_67_f32 + 0.5_f32) as i32 + 4) as u16;
        proj.element.sprite.force_sprite_row_raw(sector);
        proj.element.sprite.force_sprite(sector, frame);
    }
}

pub(crate) fn make_arrow_falling_down(
    _sim: &crate::sim_rng::SimulationContext,
    proj: &mut ElementProjectile,
    thrown_away_by_shield: bool,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) {
    let (sector, _) = current_arrow_orientation(proj);
    proj.projectile.falling = true;
    proj.projectile.flying = true;

    let (falling_direction, velocity) = if thrown_away_by_shield {
        let direction = (sector + 4) & 15;
        let (dx, dy) = crate::element::direction_vector_16(
            i16::try_from(direction).expect("arrow direction sector fits in i16"),
        );
        (
            direction as u16,
            WorldVec3D {
                x: dx * 30.0,
                y: dy * ASPECT_RATIO * 30.0,
                z: -20.0,
            },
        )
    } else {
        let direction = sector ^ 8;
        let (dx, dy) = crate::element::direction_vector_16(
            i16::try_from(direction).expect("arrow direction sector fits in i16"),
        );
        (
            direction as u16,
            WorldVec3D {
                x: dx * 30.0,
                y: dy * ASPECT_RATIO * 10.0,
                z: 0.0,
            },
        )
    };

    proj.projectile.falling_direction = falling_direction;
    // The deflection trajectory is integrated through the same
    // solid-sight-obstacle clipping as a launched shot, so a deflected arrow
    // stops at the wall or floor it is thrown into instead of sailing through
    // it. Segment clipping also shortens the first waypoint's frame count,
    // which is directly observable as this tick's movement increment.
    let (
        trajectory,
        terminal_obstacle,
        terminal_impact,
        terminal_lands_in_hole,
        terminal_lands_in_water,
        _,
    ) = compute_trajectory_ballistic_impl(
        proj.element.position(),
        velocity,
        MASS_ARROW_HIGH,
        false,
        obstacle_check,
        None,
    );
    proj.projectile.trajectory = trajectory;
    proj.projectile.trajectory_frame_count = 0;
    proj.projectile.launch_segment_start = None;
    preserve_falling_hole_disappearance(proj, terminal_lands_in_hole);
    // Original ComputeTrajectory records WATER separately from HOLE.  The
    // terminal Projectile::Hourglass uses this flag to return before
    // HitObstacle can apply the ordinary bare-ground +0.001 elevation snap.
    proj.projectile.dive |= terminal_lands_in_water;

    // Recomputing a trajectory drops the projectile's current membership and
    // re-derives it from where the new trajectory ends, so the deflected
    // arrow reports the layer and sector it is about to land in for the whole
    // of its fall rather than only once it settles.
    proj.element.clear_layer();
    proj.element.set_sector(None);
    // A water or hole classification returns from `ComputeTrajectory`
    // (`RHelementprojectile.cpp:742-756`) *before* the membership block, so the
    // `SetLayer( (UWORD)-1 ); SetSector( 0 ); SetObstacle( 0 )` of lines
    // 379-381 is all the projectile ever gets: no layer, no sector, and no
    // terminal obstacle.
    let terminal_membership =
        terminal_impact && !terminal_lands_in_hole && !terminal_lands_in_water;
    if let Some(check) = obstacle_check {
        let bound = terminal_membership.then_some(terminal_obstacle).flatten();
        let plane = terminal_obstacle_plane(bound, check.sight_obstacles);
        bind_trajectory_obstacle(&mut proj.element, bound, plane);
    }
    if terminal_membership
        && let Some(check) = obstacle_check
        && let Some(end) = proj.projectile.trajectory.last().map(|tp| tp.position)
    {
        let resolution = if let Some(obstacle) = terminal_obstacle {
            check
                .fast_find_grid
                .resolve_projectile_landing_with_obstacle(
                    end.to_map(),
                    Some(obstacle),
                    check.sight_obstacles,
                )
        } else {
            check
                .fast_find_grid
                .resolve_projectile_ground_landing(end.to_map())
        };
        proj.element.set_sector(resolution.sector);
        if resolution.sector.is_some() && !resolution.blocked_by_motion_obstacle {
            proj.element.set_layer(resolution.layer);
        }
    }

    // C++ `RHElementArrow::MakeFallingDown` calls `Hourglass()` after
    // recomputing the trajectory, so the ricochet visibly advances on
    // the same tick as the shield/target impact.  That nested hourglass
    // opens with its own `NewMove`, which re-anchors the old position onto
    // the impact point before the deflection step is applied.
    proj.element.sprite.position_iface.new_move();
    proj.advance_trajectory_one_frame();
}

pub(super) fn preserve_falling_hole_disappearance(
    proj: &mut ElementProjectile,
    terminal_lands_in_hole: bool,
) {
    // `ComputeTrajectory` does not clear `mbDisappear`, and
    // `AddTrajectoryFallIntoHole` sets it before checking whether there are
    // enough waypoints to append the visual far-edge extension. A short
    // ricochet can therefore have only one terminal waypoint and must still
    // disappear silently when that point lies in a hole.
    proj.projectile.disappear |= terminal_lands_in_hole;
}

/// Resolve Original's `GetHitShieldHolder` query for one already-advanced
/// projectile segment. This is also used by the explicit pre-publication
/// purse Hourglass, before that purse has an entity-array slot of its own.
pub(crate) fn projectile_shield_holder(
    entities: &Entities,
    shooter: Option<EntityId>,
    old: WorldPoint3D,
    new: WorldPoint3D,
    increment: WorldVec3D,
) -> Option<EntityId> {
    let flight_dir = (increment.x, increment.y * INVERSE_ASPECT_RATIO);
    for (actor_id, actor) in entities.actors() {
        let holder: EntityId = actor_id.into();
        if Some(holder) == shooter || !actor.is_active() || actor.is_dead() {
            continue;
        }
        let Some(actor_data) = actor.actor_data() else {
            continue;
        };
        if !actor_data.action_state.is_shield() {
            continue;
        }
        let Some(obstacle) = actor_data.shield_obstacle.as_ref() else {
            continue;
        };
        let (look_x, look_y) =
            crate::element::direction_vector_16(actor.element_data().direction());
        if look_x * flight_dir.0 + look_y * flight_dir.1 < 0.0
            && obstacle.is_blocking_ray_3d([new.x, new.y, new.z], [old.x, old.y, old.z])
        {
            return Some(holder);
        }
    }
    None
}

/// Advance every arrow projectile by one frame along its precomputed
/// ballistic trajectory.
///
/// Pops waypoints from the trajectory list, interpolates position
/// between them, and checks for victim proximity each frame.
///
/// When the arrow comes within [`HIT_DISTANCE`] of any living human,
/// or the trajectory runs out, the arrow is flagged for despawn and
/// the engine applies damage.
pub fn tick_arrows(
    sim: &crate::sim_rng::SimulationContext,

    entities: &mut Entities,
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
) -> Vec<ArrowTickResult> {
    tick_arrows_matching(sim, entities, sight_obstacles, None, None, false, &[], None)
}

/// Advance every projectile except the ones listed in `skip_arrow_ids`.
///
/// Used for bow arrows released from the sequence-manager phase: C++
/// already called `pArrow->Hourglass()` before insertion, and the global
/// element hourglass pass for that frame has already finished.
pub fn tick_arrows_excluding(
    sim: &crate::sim_rng::SimulationContext,

    entities: &mut Entities,
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
    skip_arrow_ids: &[EntityId],
) -> Vec<ArrowTickResult> {
    tick_arrows_matching(
        sim,
        entities,
        sight_obstacles,
        None,
        None,
        false,
        skip_arrow_ids,
        None,
    )
}

/// Advance and resolve collision for one active projectile.
///
/// Used immediately after spawning a bow arrow to match C++
/// `ShootWithBowAt`, which calls `pArrow->Hourglass()` before the
/// arrow enters the engine element list.
pub fn tick_arrow(
    sim: &crate::sim_rng::SimulationContext,

    entities: &mut Entities,
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
    arrow_id: EntityId,
) -> Vec<ArrowTickResult> {
    tick_arrows_matching(
        sim,
        entities,
        sight_obstacles,
        obstacle_check,
        Some(arrow_id),
        true,
        &[],
        None,
    )
}

/// Production variant of [`tick_arrow`] whose actor collision scans follow
/// Original's combined `marrayActors` order.
pub(crate) fn tick_arrow_in_actor_order(
    sim: &crate::sim_rng::SimulationContext,
    entities: &mut Entities,
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
    arrow_id: EntityId,
    actor_order: &[EntityId],
) -> Vec<ArrowTickResult> {
    tick_arrows_matching(
        sim,
        entities,
        sight_obstacles,
        obstacle_check,
        Some(arrow_id),
        true,
        &[],
        Some(actor_order),
    )
}

/// Advance one projectile already present in the engine element array.
///
/// Unlike [`tick_arrow`], this does not treat the projectile's spawn-time
/// priming step as its current-frame advancement. It is used by the engine's
/// creation-ordered entity pass so projectile and PC hourglasses can retain
/// their relative element-array order.
pub fn tick_existing_projectile(
    sim: &crate::sim_rng::SimulationContext,

    entities: &mut Entities,
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
    projectile_id: EntityId,
) -> Vec<ArrowTickResult> {
    tick_arrows_matching(
        sim,
        entities,
        sight_obstacles,
        obstacle_check,
        Some(projectile_id),
        false,
        &[],
        None,
    )
}

/// Production variant of [`tick_existing_projectile`] whose actor collision
/// scans follow Original's combined `marrayActors` order.
pub(crate) fn tick_existing_projectile_in_actor_order(
    sim: &crate::sim_rng::SimulationContext,
    entities: &mut Entities,
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
    projectile_id: EntityId,
    actor_order: &[EntityId],
) -> Vec<ArrowTickResult> {
    tick_arrows_matching(
        sim,
        entities,
        sight_obstacles,
        obstacle_check,
        Some(projectile_id),
        false,
        &[],
        Some(actor_order),
    )
}

fn tick_arrows_matching(
    sim: &crate::sim_rng::SimulationContext,

    entities: &mut Entities,
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
    only_arrow_id: Option<EntityId>,
    primed_segment_already_advanced: bool,
    skip_arrow_ids: &[EntityId],
    actor_order: Option<&[EntityId]>,
) -> Vec<ArrowTickResult> {
    let mut results = Vec::new();

    // Snapshot living humans for line-segment hit detection.  Computes
    // the perpendicular distance from the target's 3D belt (or eyes
    // for stones) to the arrow's movement line, and filters by posture.
    //
    // Excluded postures: `Lying`, `Carried`, `Dead`, `DeadBack`,
    // `StuckUnderNet`, `Tied`, `Tree` — targets in these states are
    // un-hittable (on the ground, restrained, or camouflaged).
    // `LeaningOut` falls through to the default branch but gets a
    // second belt→eyes pass for arrows.
    struct HumanSnapshot {
        id: EntityId,
        /// Belt point (arrows + apples) and eyes point (stones) in 3D.
        /// Pre-computed so the per-projectile loop stays cheap.
        belt: WorldPoint3D,
        eyes: WorldPoint3D,
        /// True when posture == LeaningOut — arrows get an eye-level
        /// re-check after the belt miss.
        leaning_out: bool,
        is_pc: bool,
        is_civilian: bool,
        camp: Option<crate::element::Camp>,
        holding_shield: bool,
        position_map: MapPoint,
    }
    let actor_ids: Vec<EntityId> = actor_order.map_or_else(
        || {
            entities
                .actors()
                .map(|(actor_id, _)| actor_id.into())
                .collect()
        },
        <[EntityId]>::to_vec,
    );
    let human_snapshots: Vec<HumanSnapshot> = actor_ids
        .iter()
        .filter_map(|&entity_id| {
            let e = entities.get(entity_id).unwrap_or_else(|| {
                panic!("projectile actor registry contains missing entity {entity_id}")
            });
            if !e.is_human() || !e.is_active() {
                return None;
            }
            // Posture filter: skip targets that can't be hit by
            // arrows (lying on ground, carried, dead, netted, tied,
            // hiding in a tree).
            let posture = e.element_data().posture;
            use crate::element::Posture::*;
            if matches!(
                posture,
                Lying | Carried | Dead | DeadBack | StuckUnderNet | Tied | Tree
            ) {
                return None;
            }
            let Some(belt) = e.compute_belt_point() else {
                tracing::warn!(
                    entity = entity_id.index(),
                    "Projectile hit snapshot skipped: human missing belt hotspot"
                );
                return None;
            };
            let Some(eyes) = e.compute_eyes_point(None) else {
                tracing::warn!(
                    entity = entity_id.index(),
                    "Projectile hit snapshot skipped: human missing eyes hotspot"
                );
                return None;
            };
            let camp = match e {
                Entity::Pc(_) => Some(crate::element::Camp::Royalists),
                Entity::Soldier(s) => Some(s.soldier.cached_camp),
                Entity::Civilian(c) => Some(c.civilian.cached_camp),
                _ => None,
            };
            let Some(actor) = e.actor_data() else {
                tracing::warn!(
                    entity = entity_id.index(),
                    "Projectile hit snapshot skipped: human missing actor data"
                );
                return None;
            };
            let holding_shield = actor.action_state.is_shield();
            Some(HumanSnapshot {
                id: entity_id,
                belt,
                eyes,
                leaning_out: posture == crate::element::Posture::LeaningOut,
                is_pc: e.is_pc(),
                is_civilian: e.is_civilian(),
                camp,
                holding_shield,
                position_map: e.element_data().position_map(),
            })
        })
        .collect();

    // `RHElementProjectile::FindHumanVictim` reads its shooter through the
    // raw `mpShooter` pointer captured at construction
    // (`original-code/RHelementprojectile.cpp:103`), which nothing but
    // deserialization ever rewrites. The scan aborts on `mpShooter == NULL`
    // alone (`:1801`) and otherwise only asks the shooter for `IsSoldier()`
    // / `GetCamp()` / `IsPC()` (`:1833-1857`), no matter what has happened
    // to him since the shot. Those answers stay valid for a shooter who has
    // since been killed, knocked down, netted or tied, so his arrows keep
    // hunting victims for the rest of their flight. The hittable-victim
    // snapshot above deliberately drops exactly those states, so the
    // shooter's prefilter traits are collected separately over every human
    // actor.
    struct ShooterTraits {
        id: EntityId,
        is_pc: bool,
        is_soldier: bool,
        camp: Option<crate::element::Camp>,
    }
    let shooter_traits: Vec<ShooterTraits> = actor_ids
        .iter()
        .filter_map(|&entity_id| {
            let e = entities.get(entity_id).unwrap_or_else(|| {
                panic!("projectile actor registry contains missing entity {entity_id}")
            });
            if !e.is_human() {
                return None;
            }
            let camp = match e {
                Entity::Pc(_) => Some(crate::element::Camp::Royalists),
                Entity::Soldier(s) => Some(s.soldier.cached_camp),
                Entity::Civilian(c) => Some(c.civilian.cached_camp),
                _ => None,
            };
            Some(ShooterTraits {
                id: entity_id,
                is_pc: e.is_pc(),
                is_soldier: e.is_soldier(),
                camp,
            })
        })
        .collect();

    // Snapshot FX targets that can be activated by a passing
    // projectile.  Each projectile type checks a specific filter bit
    // and launches a dedicated activation command — the per-projectile
    // loop below matches the projectile's `ObjectType` against the
    // target's filter bits.  The hit test uses the target's 3D center
    // and the perpendicular distance to the arrow's movement line.
    struct FxTargetSnapshot {
        id: EntityId,
        center: WorldPoint3D,
        position_map: MapPoint,
        action_filter: crate::element::TargetFilter,
    }
    let fx_target_snapshots: Vec<FxTargetSnapshot> = entities
        .targets()
        .filter_map(|(target_id, e)| {
            let entity_id = EntityId::Target(target_id);
            if !e.element.active {
                return None;
            }
            let filter = e.target.action_filter;
            // Projectile-activation filters only — keeps the per-tick
            // inner loop small.
            if !filter.intersects(
                crate::element::TargetFilter::ARROW
                    | crate::element::TargetFilter::APPLE
                    | crate::element::TargetFilter::STONE,
            ) {
                return None;
            }
            let Some(center) = Entity::Target(e.clone()).compute_target_center() else {
                tracing::warn!(
                    entity = entity_id.index(),
                    "Projectile hit snapshot skipped: FX target missing center hotspot"
                );
                return None;
            };
            Some(FxTargetSnapshot {
                id: entity_id,
                center,
                position_map: e.element.position_map(),
                action_filter: filter,
            })
        })
        .collect();

    // Snapshot shield holders for arrow-shield intersection — iterates
    // all actors holding a shield and checks their shield obstacle
    // geometry against each projectile's path.
    struct ShieldSnapshot {
        holder_id: EntityId,
        /// Look direction with Y un-compressed by inverse aspect ratio,
        /// for the dot-product "arrow from front" check.
        look_dir: (f32, f32),
        obstacle: crate::sight_obstacle::SightObstacle,
    }
    let shield_snapshots: Vec<ShieldSnapshot> = actor_ids
        .iter()
        .filter_map(|&entity_id| {
            let e = entities.get(entity_id).unwrap_or_else(|| {
                panic!("projectile actor registry contains missing entity {entity_id}")
            });
            if !e.is_active() || e.is_dead() {
                return None;
            }
            let actor = e.actor_data()?;
            if !actor.action_state.is_shield() {
                return None;
            }
            let obstacle = actor.shield_obstacle.as_ref()?.clone();
            let (dx, dy) = crate::element::direction_vector_16(e.element_data().direction());
            // Original starts with GetDirectionVector (Y compressed by
            // ASPECT_RATIO), then un-compresses it for this dot product. The
            // two factors cancel back to the raw 16-sector direction.
            let look_dir = (dx, dy);
            Some(ShieldSnapshot {
                holder_id: entity_id,
                look_dir,
                obstacle,
            })
        })
        .collect();

    tracing::trace!(
        holders = ?shield_snapshots.iter().map(|s| s.holder_id).collect::<Vec<_>>(),
        "Projectile tick shield-holder snapshot"
    );

    for (projectile_id, entity) in entities.projectiles_mut() {
        let idx = projectile_id.0 as usize;
        let arrow_id = EntityId::Projectile(crate::entity_id::ProjectileId(idx as u32));
        if let Some(only_arrow_id) = only_arrow_id
            && only_arrow_id != arrow_id
        {
            continue;
        }
        if skip_arrow_ids.contains(&arrow_id) {
            continue;
        }
        if !entity.element.active {
            continue;
        }
        let proj = entity;
        // `Entity::Projectile` is shared by arrows, apples, stones,
        // purses, coins, nets, wasp nests, and wasps.  Purses, coins,
        // wasp nests, and wasps follow their own per-tick update paths
        // (`EngineInner::tick_purses_and_coins`, `EngineInner::tick_wasp_nests`)
        // — skip them here so the proximity / shield / FX-target paths
        // below don't misfire.
        if matches!(
            proj.object.object_type,
            ObjectType::Purse
                | ObjectType::Coin
                | ObjectType::WaspNest
                | ObjectType::BonusWaspNest
                | ObjectType::Wasp
        ) {
            continue;
        }

        let is_burster = matches!(
            proj.object.object_type,
            ObjectType::Apple | ObjectType::Stone
        );

        // Grounded Apple/Stone work belongs to the derived virtual owner
        // path. Projectile::Hourglass returns without advancing or removing
        // them; the caller must run the landed sprite tail before applying
        // that saved base result.
        if !proj.projectile.flying {
            continue;
        }
        // RHElementProjectile::Hourglass calls NewMove before advancing its
        // trajectory. Spawned projectiles have already consumed their primer
        // step, so this snapshots that primer position exactly as Original's
        // explicit pre-add Hourglass does.
        proj.element.sprite.position_iface.new_move();
        let hourglass_old_position = proj.element.position();

        // Distinct impact FX ids per projectile type.  Arrows play
        // their 510 only on shield deflection (which has its own
        // path), so non-shield arrow impacts stay silent.
        let impact_fx = match proj.object.object_type {
            ObjectType::Apple => Some(509u32),
            ObjectType::Stone => Some(508u32),
            _ => None,
        };

        let _target_id = proj.object.reference;
        let damage = proj.projectile.damage;
        let shooter_id = proj.projectile.shooter;
        let primed_segment_start = proj.projectile.launch_segment_start.take();
        let has_primed_segment = primed_segment_start.is_some();
        let already_advanced_this_segment = has_primed_segment && primed_segment_already_advanced;

        // ── Trajectory advancement ────────────────────────────────

        if has_primed_segment {
            // Spawn primed the first segment. The immediate single-arrow
            // path has already advanced it to match C++ `pArrow->Hourglass()`;
            // the general tick path keeps the old public behavior and applies
            // the stored increment below. Both paths still run collision
            // against `primed_segment_start -> current/new`.
        } else if proj.projectile.trajectory_frame_count == 0 {
            if !proj.projectile.trajectory.is_empty() {
                // Pop the next trajectory waypoint.
                let point = proj.projectile.trajectory.remove(0);
                let time = point.time.max(1);
                proj.projectile.trajectory_frame_count = time - 1;

                // Compute per-frame increment toward this waypoint.
                let current = proj.element.position();
                let factor = 1.0 / time as f32;
                proj.projectile.velocity_increment = WorldVec3D {
                    x: (point.position.x - current.x) * factor,
                    y: (point.position.y - current.y) * factor,
                    z: (point.position.z - current.z) * factor,
                };

                // Update end position.
                proj.projectile.end = point.position;
            } else {
                // Trajectory exhausted — projectile lands / impacts
                // terrain.  Apples and stones force the burst
                // animation and keep the sprite alive for a few
                // frames; arrows just despawn.
                //
                // Elevation snap on landing.  Branch on the obstacle
                // at the landing point:
                //   * No obstacle → snap elevation to 0.001 (absolute
                //     ground), unconditionally.
                //   * Obstacle present → snap elevation to
                //     `top_plane_z + 0.001`, gated on
                //     `layer != 0xFFFF && object_type != Arrow`
                //     (arrows stuck in walls and unassigned-layer
                //     projectiles keep their trajectory-end elevation).
                //
                // The obstacle is the one the trajectory builder struck
                // when it terminated the arc, bound onto the projectile at
                // launch and carried through the flight. It is emphatically
                // not "whichever projection polygon happens to cover the
                // landing point on screen": an arrow that lands on open
                // ground in front of a building is under that building's
                // projection polygon yet hit no obstacle at all, and gets
                // the flat 0.001 ground snap.
                let pos = proj.element.position();
                let (top_plane_z, new_z) = if proj.projectile.disappear || proj.projectile.dive {
                    // Original tests `mbDive` / `mbDisappear` before
                    // `HitObstacle`. Reaching water or a hole therefore
                    // preserves the trajectory-end elevation instead of
                    // applying the ordinary +0.001 ground/obstacle snap.
                    (None, None)
                } else {
                    let top_plane_z = proj.element.obstacle_index().map(|handle| {
                        let index = usize::from(u16::from(handle));
                        let obstacle = sight_obstacles.get(index).unwrap_or_else(|| {
                            panic!(
                                "landed projectile obstacle {index} is absent from its source list"
                            )
                        });
                        crate::position_interface::PlaneZCoeffs::from_plane_points(
                            &obstacle.top_plane_points,
                        )
                        .compute_z(pos.x, pos.y)
                    });

                    let new_z = match top_plane_z {
                        None => Some(0.001),
                        Some(z) => {
                            if !matches!(proj.object.object_type, ObjectType::Arrow)
                                && proj.element.layer() != 0xFFFF
                            {
                                Some(z + 0.001)
                            } else {
                                None
                            }
                        }
                    };
                    (top_plane_z, new_z)
                };
                tracing::trace!(
                    target: "arrow_landing",
                    arrow = arrow_id.index(),
                    object_type = ?proj.object.object_type,
                    obstacle = ?proj.element.obstacle_index().map(u16::from),
                    layer = proj.element.layer(),
                    ?pos,
                    ?top_plane_z,
                    ?new_z,
                    "projectile landing elevation snap"
                );
                if let Some(z) = new_z {
                    let mut p = proj.element.position();
                    p.z = z;
                    proj.element.set_position(p);
                    proj.element.set_position_map_preserving_3d(p.to_map());
                }
                let impact_pos = proj.element.position_map();
                proj.projectile.flying = false;
                if proj.projectile.dive {
                    // Projectile::Hourglass returns false for mbDive and the
                    // engine retires the arrow on this same boundary. Keep
                    // the tombstone so parity can still observe its settled
                    // terminal state.
                    proj.element.active = false;
                }
                let despawn = if is_burster {
                    set_projectile_animation(proj, Animation::ObjectBursting);
                    false
                } else {
                    true
                };
                results.push(ArrowTickResult {
                    arrow: arrow_id,
                    hit_target: None,
                    shield_hit: None,
                    fx_target_hit: None,
                    despawn,
                    damage,
                    impact_fx,
                    impact_pos,
                    human_hit_old_position: None,
                });
                continue;
            }
        } else {
            proj.projectile.trajectory_frame_count -= 1;
        }

        if !already_advanced_this_segment {
            // Apply the per-frame increment to position.
            let mut p = proj.element.position();
            p.x += proj.projectile.velocity_increment.x;
            p.y += proj.projectile.velocity_increment.y;
            p.z += proj.projectile.velocity_increment.z;
            proj.element.set_position(p);

            // Update the 2D map position from 3D (project Z onto Y for
            // isometric display: map.y = pos.y - pos.z).
            proj.element
                .set_position_map_preserving_3d(MapPoint::from_world_xyz(
                    proj.element.position().x,
                    proj.element.position().y,
                    proj.element.position().z,
                ));
        }
        // Increment lifetime counter for diagnostics/replay state. C++
        // projectile lifetime is governed by trajectory exhaustion and
        // impact side effects; it has no hard timeout.
        if !already_advanced_this_segment {
            proj.projectile.frame_count = proj.projectile.frame_count.saturating_add(1);
        }

        // ── Falling arrows skip all collision checks ──────────────
        // Once an arrow is deflected (by a shield or target), it
        // tumbles to the ground without hitting anything.  It
        // continues advancing along its deflected trajectory until it
        // runs out (handled by the trajectory advancement code above).
        if proj.projectile.falling {
            continue;
        }

        let arrow_new = proj.element.position();
        // FindHumanVictim reads mpSprite->GetOldPosition(), which NewMove
        // saved before UpdatePosition. Do not reconstruct that point as
        // `arrow_new - increment`: f32 addition/subtraction is not reversible
        // at large map coordinates, and a one-bit shift can reject a target
        // exactly at the segment endpoint through the strict range gate.
        let arrow_old = primed_segment_start.unwrap_or(hourglass_old_position);

        // ── Shield intersection check ─────────────────────────────
        // Before checking victim proximity, check if any shield blocks
        // the projectile path.  This check runs for **every**
        // projectile type, but only arrows are deflected into the
        // falling state on a shield hit.  Apples and stones keep
        // flying along their existing trajectory; the caller plays
        // the per-type impact FX and launches a `ParryShield` on the
        // holder, then this frame terminates early — the apple/stone
        // carries on its trajectory next tick.
        let vx = proj.projectile.velocity_increment.x;
        let vy = proj.projectile.velocity_increment.y;

        let mut shield_blocker = None;
        if already_advanced_this_segment
            || !proj.projectile.trajectory.is_empty()
            || proj.projectile.trajectory_frame_count > 0
            || vx != 0.0
            || vy != 0.0
        {
            // Shield obstacle geometry lives in ground/world space, the same
            // space every other 3D sight-ray query uses, so the projectile
            // endpoints go in as world XYZ rather than screen-projected map
            // coordinates.
            let old_pos = [arrow_old.x, arrow_old.y, arrow_old.z];
            let new_pos = [arrow_new.x, arrow_new.y, arrow_new.z];

            // Flight direction with Y un-compressed.
            let flight_dir = (vx, vy * INVERSE_ASPECT_RATIO);

            for shield in &shield_snapshots {
                if Some(shield.holder_id) == shooter_id {
                    continue;
                }
                // (a) Arrow from front: dot(look_dir, flight_dir) < 0.
                let dot = shield.look_dir.0 * flight_dir.0 + shield.look_dir.1 * flight_dir.1;
                // (b) Arrow path intersects shield geometry.
                let blocking = dot < 0.0 && shield.obstacle.is_blocking_ray_3d(new_pos, old_pos);
                tracing::trace!(
                    ?arrow_id,
                    holder = ?shield.holder_id,
                    ?dot,
                    ?old_pos,
                    ?new_pos,
                    obstacle_id = shield.obstacle.id,
                    obstacle_points = ?shield.obstacle.obstacle_points,
                    blocking,
                    "Projectile shield-holder intersection test"
                );
                if blocking {
                    shield_blocker = Some(shield.holder_id);
                    break;
                }
            }
        }

        if let Some(holder) = shield_blocker {
            // Arrow path: deflect 90° right, set falling=true,
            // recompute trajectory.
            //
            // Apple/Stone path: keep flying along the existing
            // trajectory.  The per-type FX (509/508) plays at the
            // shield holder's position.  This frame terminates early
            // (no human / FX-target check), so `continue` after
            // reporting.
            if matches!(proj.object.object_type, ObjectType::Arrow) {
                make_arrow_falling_down(sim, proj, true, obstacle_check);

                results.push(ArrowTickResult {
                    arrow: arrow_id,
                    hit_target: None,
                    shield_hit: Some(holder),
                    fx_target_hit: None,
                    despawn: false, // Don't despawn — arrow falls to ground.
                    damage,
                    // Silent — see note above.
                    impact_fx: None,
                    impact_pos: proj.element.position_map(),
                    human_hit_old_position: None,
                });
            } else {
                // Apple / stone: keep flying on current trajectory; the
                // engine plays the per-type FX at the holder's position
                // and launches ParryShield.  `impact_pos` is the
                // projectile's current position — the engine caller
                // replaces it with the holder's position using
                // `shield_hit` as the anchor.
                results.push(ArrowTickResult {
                    arrow: arrow_id,
                    hit_target: None,
                    shield_hit: Some(holder),
                    fx_target_hit: None,
                    despawn: false,
                    damage,
                    impact_fx,
                    impact_pos: proj.element.position_map(),
                    human_hit_old_position: None,
                });
            }
            continue;
        }

        // ── Victim / FX-target hit detection ──────────────────────
        // For each living human / FX target, compute the perpendicular
        // distance from the target's 3D anchor point to the arrow's
        // movement line (old_pos → new_pos).  A target is hit when
        //   (a) perpendicular distance ≤ HIT_DISTANCE, and
        //   (b) the segment is long enough to reach it from old_pos
        //       (so a slow-moving arrow doesn't "teleport" onto a
        //       distant target that happens to be near its final line).
        // This catches fast arrows that would otherwise tunnel past a
        // target between frames, which the old 2D point check missed.
        /// Perpendicular offset from `p` to the line through `a→b`.
        ///
        /// C++ uses `SBGeoPoint3D::DistanceVectorToLine` for both the
        /// hit-radius norm and the leaning-out arrow retry's coarse
        /// `MaxNorm` gate. Return an infinite vector when the segment
        /// has zero length so callers naturally reject the line test.
        fn point_to_line_delta(p: WorldPoint3D, a: WorldPoint3D, b: WorldPoint3D) -> WorldVec3D {
            let abx = b.x - a.x;
            let aby = b.y - a.y;
            let abz = b.z - a.z;
            let ab_len_sq = abx * abx + aby * aby + abz * abz;
            if ab_len_sq < 1e-6 {
                return WorldVec3D {
                    x: f32::MAX,
                    y: f32::MAX,
                    z: f32::MAX,
                };
            }
            let apx = p.x - a.x;
            let apy = p.y - a.y;
            let apz = p.z - a.z;
            let t = (apx * abx + apy * aby + apz * abz) / ab_len_sq;
            WorldVec3D {
                x: p.x - (a.x + t * abx),
                y: p.y - (a.y + t * aby),
                z: p.z - (a.z + t * abz),
            }
        }
        fn point_to_line_distance(p: WorldPoint3D, a: WorldPoint3D, b: WorldPoint3D) -> f32 {
            let delta = point_to_line_delta(p, a, b);
            (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt()
        }
        fn distance(a: WorldPoint3D, b: WorldPoint3D) -> f32 {
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let dz = b.z - a.z;
            (dx * dx + dy * dy + dz * dz).sqrt()
        }
        fn projectile_victim_prefilter_allows(
            shooter: Option<&ShooterTraits>,
            victim: &HumanSnapshot,
        ) -> bool {
            let Some(shooter) = shooter else {
                return true;
            };
            let same_camp = matches!(
                (shooter.camp, victim.camp),
                (Some(shooter_camp), Some(victim_camp)) if shooter_camp == victim_camp
            );

            // C++ filters these candidates inside `FindHumanVictim`
            // before geometric hit selection:
            //   - soldier projectiles do not hit civilians
            //   - soldier projectiles do not hit same-camp humans
            //   - PC projectiles do not hit shield-holding PCs
            // The separate forest GoodSoldier rule is covered by the
            // same-camp soldier branch for Royalist soldiers.
            !(shooter.is_soldier && (victim.is_civilian || same_camp)
                || shooter.is_pc && victim.is_pc && victim.holding_shield)
        }

        // Segment length (range of this frame's movement).
        let range = distance(arrow_old, arrow_new);
        // C++ compares floating-point segment endpoint distances exactly
        // (`vtRange.Norm() <= range`).  Rust's f32 integration can land
        // a target a tiny fraction past the nominal endpoint on flat
        // shots, so allow a sub-pixel tolerance while keeping the same
        // old/current range gate.

        // Pick the aim anchor by projectile type — arrows and apples
        // aim for the belt, stones aim for the eyes.
        let uses_eyes_anchor = matches!(proj.object.object_type, ObjectType::Stone);

        // C++ `FindHumanVictim` returns no human victim when the
        // projectile is not moving.  FX target handling is separate in
        // `FindTargetVictim` and keeps the point fallback below.
        let use_range_gate = range > 0.0;

        let mut hit_victim = None;
        let shooter_snapshot =
            shooter_id.and_then(|id| shooter_traits.iter().find(|traits| traits.id == id));
        if let Some(id) = shooter_id
            && shooter_snapshot.is_none()
        {
            tracing::warn!(
                arrow = arrow_id.index(),
                shooter = %id,
                "projectile shooter is not a human actor; C++ mpShooter is always \
                 RHElementActorHuman*, so no camp prefilter can be applied"
            );
        }
        // C++ `RHElementProjectile::FindHumanVictim` returns null when
        // `mpShooter == NULL`; only FX target collision still runs.
        if let Some(shooter_snapshot) = shooter_snapshot {
            for snap in &human_snapshots {
                if Some(snap.id) == shooter_id {
                    continue;
                }
                if !projectile_victim_prefilter_allows(Some(shooter_snapshot), snap) {
                    continue;
                }
                let anchor = if uses_eyes_anchor {
                    snap.eyes
                } else {
                    snap.belt
                };
                let hit = if use_range_gate {
                    // Range gate: the old_pos→target distance must be
                    // within this frame's reach (segment length).
                    let old_to_target = distance(arrow_old, anchor);
                    let line_distance = point_to_line_distance(anchor, arrow_old, arrow_new);
                    old_to_target <= range && line_distance <= HIT_DISTANCE
                } else {
                    false
                };
                if hit {
                    hit_victim = Some((snap.id, snap.position_map));
                    // Original's `break` exits only the posture switch, not
                    // the surrounding marrayActors scan.  A later eligible
                    // human therefore replaces the current candidate.
                    continue;
                }

                // Leaning-out re-check for arrows only.  If the belt's
                // perpendicular distance-to-flight-line has max component
                // <= 100, try again at eye level. This matches the C++
                // `vtDistance.MaxNorm()` gate after the belt check.
                if snap.leaning_out
                    && matches!(proj.object.object_type, ObjectType::Arrow)
                    && use_range_gate
                {
                    let belt_line_delta = point_to_line_delta(anchor, arrow_old, arrow_new);
                    let max_norm = belt_line_delta
                        .x
                        .abs()
                        .max(belt_line_delta.y.abs())
                        .max(belt_line_delta.z.abs());
                    if max_norm <= 100.0 {
                        let eyes = snap.eyes;
                        // C++ uses the arrow's current position for the
                        // leaning-out eye retry, unlike the primary human
                        // belt check's "deguillaumized" old-position gate.
                        let new_to_eyes = distance(arrow_new, eyes);
                        if new_to_eyes <= range
                            && point_to_line_distance(eyes, arrow_old, arrow_new) <= HIT_DISTANCE
                        {
                            hit_victim = Some((snap.id, snap.position_map));
                            // As with the belt hit above, Original continues
                            // the actor scan and ultimately returns the last
                            // eligible human.
                            continue;
                        }
                    }
                }
            }
        }

        // FX-target segment/point check (same semantics).  C++
        // `FindTargetVictim` only returns FX targets whose action
        // filter matches the projectile type, so nonmatching targets
        // are ignored before `HitTarget` can run.
        let (required_filter, activation_command) = match proj.object.object_type {
            ObjectType::Arrow => (crate::element::TargetFilter::ARROW, Command::ActivateArrow),
            ObjectType::Apple => (crate::element::TargetFilter::APPLE, Command::ActivateApple),
            ObjectType::Stone => (crate::element::TargetFilter::STONE, Command::ActivateStone),
            _ => (
                crate::element::TargetFilter::empty(),
                Command::ActivateArrow,
            ),
        };
        let mut fx_target_hit: Option<(EntityId, Command, MapPoint)> = None;
        if !required_filter.is_empty() {
            for snap in &fx_target_snapshots {
                if !snap.action_filter.contains(required_filter) {
                    continue;
                }
                let hit = if use_range_gate {
                    // C++ `FindTargetVictim` gates FX targets by
                    // distance from the projectile's current position
                    // to target center, not by old position.  This
                    // catches the scripted-target case where the
                    // current frame reaches the center exactly.
                    let new_to_target = distance(arrow_new, snap.center);
                    new_to_target <= range
                        && point_to_line_distance(snap.center, arrow_old, arrow_new) <= HIT_DISTANCE
                } else {
                    // With no movement C++ still evaluates
                    // `vtRange.Norm() <= range`, so only an exact center
                    // overlap can activate the target. Do not use the
                    // normal hit-radius fallback here.
                    distance(arrow_new, snap.center) <= 0.01
                };
                if hit {
                    fx_target_hit = Some((snap.id, activation_command, snap.position_map));
                    break;
                }
            }
        }

        if let Some((victim, victim_position_map)) = hit_victim {
            let impact_pos = victim_position_map;
            proj.projectile.flying = false;
            let despawn = if is_burster {
                set_projectile_animation(proj, Animation::ObjectBursting);
                false
            } else {
                true
            };
            results.push(ArrowTickResult {
                arrow: arrow_id,
                hit_target: Some(victim),
                shield_hit: None,
                fx_target_hit: None,
                despawn,
                damage,
                impact_fx,
                impact_pos,
                human_hit_old_position: Some(arrow_old),
            });
        } else if let Some((fx_id, fx_command, fx_position_map)) = fx_target_hit {
            let impact_pos = fx_position_map;
            proj.projectile.flying = false;
            // RHElementProjectile::Hourglass handles a successful HitTarget
            // synchronously: stop flight and DeleteTrajectory before the
            // frame snapshot. Unlike HitHuman it does not rewind to the old
            // position, so the following Refresh keeps the arrow for this
            // moving frame; next Hourglass calls NewMove and the subsequent
            // Refresh retires the now-stationary empty arrow.
            proj.projectile.trajectory.clear();
            let despawn = if is_burster {
                set_projectile_animation(proj, Animation::ObjectBursting);
                false
            } else {
                true
            };
            results.push(ArrowTickResult {
                arrow: arrow_id,
                hit_target: None,
                shield_hit: None,
                fx_target_hit: Some((fx_id, fx_command)),
                despawn,
                damage,
                impact_fx,
                impact_pos,
                human_hit_old_position: None,
            });
        }
    }

    results
}

// ═══════════════════════════════════════════════════════════════════
//  Hit application
// ═══════════════════════════════════════════════════════════════════

/// Apply an arrow impact to the target human.
///
/// Returns `true` if the victim died from the hit.
pub fn apply_arrow_hit(
    entities: &mut Entities,
    victim_id: EntityId,
    shooter_id: EntityId,
    damage: u16,
    arrow_flight_direction: i16,
) -> bool {
    // Arrows pass `concussion = damage` — the arrow damage element
    // uses a single value for both fields.
    apply_projectile_hit(
        entities,
        victim_id,
        shooter_id,
        damage,
        damage,
        arrow_flight_direction,
    )
}

/// Apply a generic projectile hit (piercing damage + concussion) to a
/// human.  Factored from [`apply_arrow_hit`] so stones can pass a
/// distinct concussion (e.g. damage=10, concussion=100 for stones —
/// much higher KO potential than arrows).
pub fn apply_projectile_hit(
    entities: &mut Entities,
    victim_id: EntityId,
    shooter_id: EntityId,
    damage: u16,
    concussion: u16,
    arrow_flight_direction: i16,
) -> bool {
    // Resolve shooter PC-ness before the victim mutable borrow. C++
    // projectile damage carries a real origin pointer; missing shooter
    // state is invalid and must not become "not a PC" silently.
    let Some(shooter) = entities.get(shooter_id) else {
        tracing::warn!(
            ?victim_id,
            ?shooter_id,
            "projectile hit skipped: missing shooter before damage"
        );
        return false;
    };
    let shooter_is_pc = shooter.is_pc();

    let victim = match entities.get_mut(victim_id) {
        Some(e) => e,
        None => {
            tracing::warn!(
                ?victim_id,
                ?shooter_id,
                "projectile hit skipped: missing victim before damage"
            );
            return false;
        }
    };

    // Snap the victim to face the arrow's opposite direction (toward
    // the shooter) when struck.
    victim
        .element_data_mut()
        .set_direction_instantly(arrow_flight_direction ^ 8);

    let ctx = ConcussionContext {
        is_invulnerable: victim.is_immortal(),
        ..ConcussionContext::default()
    };
    // Read actual max HP from the entity.
    let max_hp: i16 = match &*victim {
        Entity::Pc(_) => 100,
        Entity::Soldier(s) => {
            use crate::element::Human;
            Human::max_life_points(s)
        }
        Entity::Civilian(_) => 100,
        _ => 100,
    };

    // Snapshot the pre-hit unconscious state so we can detect the KO
    // transition triggered by the concussion add and forward the
    // shooter attribution into `inform_my_friends`.
    let Some(human) = victim.human_data() else {
        tracing::warn!(
            ?victim_id,
            "projectile hit skipped: human victim missing human data before damage"
        );
        return false;
    };
    let was_unconscious = human.unconscious;

    let died = match victim {
        Entity::Pc(pc) => combat::receive_piercing_damage(
            &mut pc.human,
            &mut pc.pc.life_points,
            damage,
            concussion,
            max_hp,
            &ctx,
        ),
        Entity::Soldier(s) => combat::receive_piercing_damage(
            &mut s.human,
            &mut s.npc.life_points,
            damage,
            concussion,
            max_hp,
            &ctx,
        ),
        Entity::Civilian(c) => combat::receive_piercing_damage(
            &mut c.human,
            &mut c.npc.life_points,
            damage,
            concussion,
            max_hp,
            &ctx,
        ),
        _ => return false,
    };

    // Detect a fresh KO transition (was conscious, now unconscious).
    // Set `inform_my_friends` only on this transition; the flag is
    // consumed at that NPC's next owner slot by
    // `tick_inform_my_friends_for_npc`, which broadcasts the body to
    // nearby NPCs. Without this, a stone-KO'd soldier would not be
    // detected by his friends, breaking witness wiring for PC-thrown
    // stones.
    let Some(human) = victim.human_data() else {
        tracing::warn!(
            ?victim_id,
            "projectile hit skipped: human victim missing human data after damage"
        );
        return false;
    };
    let now_unconscious = human.unconscious;
    if !was_unconscious
        && now_unconscious
        && let Some(npc) = victim.npc_data_mut()
    {
        npc.inform_my_friends = shooter_is_pc;
    }

    died
}
