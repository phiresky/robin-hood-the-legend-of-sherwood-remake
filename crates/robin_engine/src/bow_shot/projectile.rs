//! Projectile construction, per-frame flight, and impact resolution.

use super::*;

// ═══════════════════════════════════════════════════════════════════
//  Arrow spawn
// ═══════════════════════════════════════════════════════════════════

/// Parameters for spawning an arrow projectile.
pub struct SpawnArrowParams {
    pub shooter: EntityId,
    pub bow_point: WorldPoint3D,
    /// The original game stores the shooter map position after the arrow's
    /// first simulation step.
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
/// advancement during its entity update.
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

    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
        initial_element.kind = ElementKind::ObjectProjectile;
        initial_element.active = true;
        initial_element
    };
    // The position interface initializes both serialized posture slots to
    // UPRIGHT. Sprite state stores its maximum-value order sentinel as 65535;
    // Rust's one-based order-ID projection therefore observes 65536.
    element
        .sprite
        .position_iface
        .initialize_constructor_posture(Posture::Upright);
    element.sprite.last_processed_order_id = u32::from(u16::MAX) + 1;
    element.set_position_map(map_pos);
    element.set_position(bow_point);
    // Trajectory calculation detaches a projectile from world membership before
    // constructing its flight: layer = 0xFFFF, with no sector or obstacle.
    // It only restores a layer when the resolved landing point belongs
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
    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Undefined);
        initial_element.kind = ElementKind::ObjectNet;
        initial_element.active = true;
        initial_element
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
    let time_till_unfolding = total_trajectory_frames.saturating_sub(15).max(1);

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
    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Undefined);
        initial_element.kind = ElementKind::ObjectProjectile;
        initial_element.active = true;
        initial_element
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
    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Undefined);
        initial_element.kind = ElementKind::ObjectProjectile;
        initial_element.active = true;
        initial_element
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
        ThrowRequest {
            thrower,
            throw_pos,
            target_pos,
            target,
            target_forecasted_movement,
            layer,
        },
        ThrowableKind::Apple,
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
        ThrowRequest {
            thrower,
            throw_pos,
            target_pos,
            target,
            target_forecasted_movement,
            layer,
        },
        ThrowableKind::Stone,
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
#[derive(serde::Serialize, serde::Deserialize)]
struct ThrowRequest {
    thrower: EntityId,
    throw_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    target: Option<EntityId>,
    target_forecasted_movement: Option<WorldVec3D>,
    layer: u16,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
enum ThrowableKind {
    Apple,
    Stone,
}

fn spawn_throwable(
    request: ThrowRequest,
    kind: ThrowableKind,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> Entity {
    let ThrowRequest {
        thrower,
        throw_pos,
        target_pos,
        target,
        target_forecasted_movement,
        layer,
    } = request;
    let (mass, apex, flight_time, action, object_type) = match kind {
        ThrowableKind::Apple => (MASS_APPLE, APEX_APPLE, 0, Action::Apple, ObjectType::Apple),
        ThrowableKind::Stone => (MASS_STONE, APEX_STONE, 1, Action::Stone, ObjectType::Stone),
    };
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
    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Undefined);
        initial_element.kind = ElementKind::ObjectProjectile;
        initial_element.active = true;
        initial_element
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
    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Undefined);
        initial_element.kind = ElementKind::ObjectProjectile;
        initial_element.active = true;
        initial_element
    };
    element.set_position_map(map_pos);
    element.set_position(throw_pos);
    // Trajectory calculation clears live membership before rebuilding the arc.
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
            element.set_sector_topology(
                resolution.sector,
                resolution.sector.and_then(|sector| sector.arena_index()),
            );
            if resolution.sector.is_some() && !resolution.blocked_by_motion_obstacle {
                element.set_layer(
                    resolution
                        .layer
                        .expect("authorized projectile landing has no resolved layer")
                        .get(),
                );
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
pub fn spawn_coin(
    source_purse: Option<EntityId>,
    source_pos: WorldPoint3D,
    target_pos: WorldPoint3D,
    _layer: Option<crate::position_interface::Layer>,
    layer_goal: Option<crate::position_interface::Layer>,
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
    let mut element = {
        let mut initial_element = ElementData::from_initial_posture(Posture::Undefined);
        initial_element.kind = ElementKind::ObjectProjectile;
        initial_element.active = true;
        initial_element
    };
    element.set_position_map(map_pos);
    element.set_position(source_pos);
    // Trajectory calculation clears live membership. Saved goal membership lives
    // separately in PurseData and coin obstacle impact installs it only on an
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
            element.set_sector_topology(
                resolution.sector,
                resolution.sector.and_then(|sector| sector.arena_index()),
            );
            if resolution.sector.is_some() && !resolution.blocked_by_motion_obstacle {
                element.set_layer(
                    resolution
                        .layer
                        .expect("authorized projectile landing has no resolved layer")
                        .get(),
                );
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

/// Original-game arrow orientation points from the arrow's current
/// position to the *next queued trajectory point*. The current per-frame
/// increment is not equivalent: the update has already removed the point
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
        // The original game does not guard 3D vector normalization here. On the
        // shipped i386 build, the resulting zero-segment direction flows
        // through direction-sector selection / acos and both integer conversions collapse
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
    // The original game renders falling arrows using their falling direction,
    // It selects one of three falling frames from the falling direction, then rotates
    // the row by -2 sectors for the next refresh.
    let row = proj.projectile.falling_direction;
    let frame =
        (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::ArrowFallingFrame, 0..3) as u16) + 3;
    proj.element.sprite.force_sprite_row_raw(row);
    proj.element.sprite.force_sprite(row, frame);
    proj.projectile.falling_direction = (row + 14) % 16;
}

/// Apply the presentation pass which Original runs after the parity snapshot.
/// The engine calls this immediately before the next simulation tick, which
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
        // condition before inspecting the flying flag or entering its falling-arrow
        // visual branch. A non-falling arrow can therefore still have
        // marked flying after consuming a zero-length final waypoint and be
        // retired here, before its next update handles obstacle impact.
        // Preserve the already-published endpoint sprite and retire without
        // another tumble draw.
        // Presentation refresh retires the arrow before another projectile tick can
        // begin another movement step. Settle the movement snapshot at the endpoint
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
    proj: &mut ElementProjectile,
    thrown_away_by_shield: bool,
    obstacle_check: Option<&TrajectoryObstacleCheck<'_>>,
) -> bool {
    let (sector, _) = current_arrow_orientation(proj);
    proj.projectile.falling = true;
    proj.projectile.flying = true;

    let (falling_direction, velocity) = if thrown_away_by_shield {
        let direction = (sector + 4) & 15;
        let (dx, dy) = crate::element::direction_vector_16(
            i16::try_from(direction).expect("arrow direction sector fits in i16"),
        );
        (
            direction,
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
            direction,
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
    // The original game records water separately from holes. The
    // terminal projectile ticking uses this flag to return before
    // Obstacle impact can apply the ordinary bare-ground +0.001 elevation snap.
    proj.projectile.dive |= terminal_lands_in_water;

    // Recomputing a trajectory drops the projectile's current membership and
    // re-derives it from where the new trajectory ends, so the deflected
    // arrow reports the layer and sector it is about to land in for the whole
    // of its fall rather than only once it settles.
    proj.element.clear_layer();
    proj.element.set_sector(None);
    // A water or hole classification returns from trajectory calculation
    // *before* the membership block, so the
    // clearing the layer, sector, and obstacle across lines
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
        proj.element.set_sector_topology(
            resolution.sector,
            resolution.sector.and_then(|sector| sector.arena_index()),
        );
        if resolution.sector.is_some() && !resolution.blocked_by_motion_obstacle {
            proj.element.set_layer(
                resolution
                    .layer
                    .expect("authorized projectile landing has no resolved layer")
                    .get(),
            );
        }
    }

    // The original game advances the projectile after
    // recomputing the trajectory, so the ricochet visibly advances on
    // the same tick as the shield/target impact.  That nested hourglass
    // opens with its own movement step, which re-anchors the old position onto
    // the impact point before the deflection step is applied.
    proj.element.sprite.position_iface.new_move();
    proj.advance_trajectory_one_frame()
}

pub(super) fn preserve_falling_hole_disappearance(
    proj: &mut ElementProjectile,
    terminal_lands_in_hole: bool,
) {
    // Trajectory calculation does not clear the disappear flag, and
    // fall-into-hole trajectory creation sets it before checking whether there are
    // enough waypoints to append the visual far-edge extension. A short
    // ricochet can therefore have only one terminal waypoint and must still
    // disappear silently when that point lies in a hole.
    proj.projectile.disappear |= terminal_lands_in_hole;
}

/// Resolve the original game's shield-holder hit query for one already-advanced
/// projectile segment. This is also used by the explicit pre-publication
/// purse update, before that purse has an entity-array slot of its own.
pub(crate) fn projectile_shield_holder(
    entities: &Entities,
    actor_order: &[EntityId],
    old: WorldPoint3D,
    new: WorldPoint3D,
    increment: WorldVec3D,
) -> Option<EntityId> {
    let flight_dir = (increment.x, increment.y * INVERSE_ASPECT_RATIO);
    let crosses_ground = (new.z > 0.0 && old.z < 0.0) || (new.z < 0.0 && old.z > 0.0);
    // The original game scans the complete actor registry for humans holding
    // shields. Active, alive, and shooter identity are not filters
    // during projectile setup.
    for &holder in actor_order {
        let actor = entities
            .get(holder)
            .expect("projectile actor registry contains missing entity");
        let Some(actor_data) = actor.actor_data() else {
            continue;
        };
        if !actor.is_human() || !actor_data.action_state.is_shield() {
            continue;
        }
        let Some(obstacle) = actor_data.shield_obstacle.as_ref() else {
            continue;
        };
        let (look_x, look_y) =
            crate::element::direction_vector_16(actor.element_data().direction());
        if look_x * flight_dir.0 + look_y * flight_dir.1 < 0.0
            && (crosses_ground
                || obstacle.is_blocking_ray_3d([new.x, new.y, new.z], [old.x, old.y, old.z]))
        {
            return Some(holder);
        }
    }
    None
}

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

pub(crate) fn projectile_human_victim(
    entities: &Entities,
    actor_order: &[EntityId],
    diplomacy: &crate::diplomacy::DiplomacyState,
    projectile_id: EntityId,
    old: WorldPoint3D,
) -> Option<EntityId> {
    let Entity::Projectile(projectile) = entities
        .get(projectile_id)
        .expect("human collision query lost its projectile")
    else {
        panic!("human collision query requires a projectile");
    };
    let shooter_id = projectile.projectile.shooter?;
    let shooter = entities
        .get(shooter_id)
        .expect("projectile shooter reference is missing");
    let new = projectile.element.position();
    let range = distance(old, new);
    if range == 0.0 {
        return None;
    }
    let mut victim = None;
    for &id in actor_order {
        if id == shooter_id {
            continue;
        }
        let human = entities
            .get(id)
            .expect("projectile actor registry contains missing entity");
        if !human.is_human() || !human.is_active() {
            continue;
        }
        let posture = human.element_data().posture();
        if matches!(
            posture,
            Posture::Lying
                | Posture::Carried
                | Posture::Dead
                | Posture::DeadBack
                | Posture::StuckUnderNet
                | Posture::Tied
                | Posture::Tree
        ) {
            continue;
        }
        let protected_relationship = !diplomacy.is_hostile(shooter.camp(), human.camp())
            || (!diplomacy.npc_faction_wars() && !shooter.is_pc() && !human.is_pc());
        if (diplomacy.enabled() && protected_relationship)
            || (shooter.is_soldier() && (human.is_civilian() || protected_relationship))
            || (shooter.is_pc()
                && human.is_pc()
                && human
                    .actor_data()
                    .expect("human has no actor data")
                    .action_state
                    .is_shield())
        {
            continue;
        }
        let anchor = if projectile.object.object_type == ObjectType::Stone {
            human.compute_eyes_point(None)
        } else {
            human.compute_belt_point()
        };
        let Some(anchor) = anchor else {
            tracing::warn!(?id, "projectile candidate is missing its body hotspot");
            continue;
        };
        if distance(old, anchor) <= range
            && point_to_line_distance(anchor, old, new) <= HIT_DISTANCE
        {
            victim = Some(id);
            continue;
        }
        if posture == Posture::LeaningOut && projectile.object.object_type == ObjectType::Arrow {
            let delta = point_to_line_delta(anchor, old, new);
            if delta.x.abs().max(delta.y.abs()).max(delta.z.abs()) <= 100.0 {
                let Some(eyes) = human.compute_eyes_point(None) else {
                    tracing::warn!(
                        ?id,
                        "leaning projectile candidate is missing its eye hotspot"
                    );
                    continue;
                };
                if distance(new, eyes) <= range
                    && point_to_line_distance(eyes, old, new) <= HIT_DISTANCE
                {
                    victim = Some(id);
                }
            }
        }
    }
    victim
}

pub(crate) fn projectile_target_victim(
    entities: &Entities,
    projectile_id: EntityId,
    old: WorldPoint3D,
) -> Option<(EntityId, Command)> {
    let Entity::Projectile(projectile) = entities
        .get(projectile_id)
        .expect("target collision query lost its projectile")
    else {
        panic!("target collision query requires a projectile");
    };
    let (filter, command) = match projectile.object.object_type {
        ObjectType::Arrow => (TargetFilter::ARROW, Command::ActivateArrow),
        ObjectType::Apple => (TargetFilter::APPLE, Command::ActivateApple),
        ObjectType::Stone => (TargetFilter::STONE, Command::ActivateStone),
        _ => return None,
    };
    let new = projectile.element.position();
    let range = distance(old, new);
    for (target_id, target) in entities.targets() {
        if !target.element.active || !target.target.action_filter.contains(filter) {
            continue;
        }
        let id = EntityId::Target(target_id);
        let Some(center) = entities
            .get(id)
            .expect("target iterator lost entity")
            .compute_target_center()
        else {
            tracing::warn!(?id, "projectile target is missing its center hotspot");
            continue;
        };
        let hit = if range > 0.0 {
            distance(new, center) <= range
                && point_to_line_distance(center, old, new) <= HIT_DISTANCE
        } else {
            distance(new, center) <= 0.01
        };
        if hit {
            return Some((id, command));
        }
    }
    None
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
fn apply_projectile_hit(
    entities: &mut Entities,
    victim_id: EntityId,
    shooter_id: EntityId,
    damage: u16,
    concussion: u16,
    arrow_flight_direction: i16,
) -> bool {
    // Resolve shooter PC-ness before the victim mutable borrow. Original-game
    // projectile damage carries an origin reference; a missing shooter
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
