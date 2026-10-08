//! Per-tick helpers for actor-vs-actor anti-collision.
//!
//! The pure math lives in [`crate::position_interface::compute_deviated_future`]
//! / [`crate::repulsive`]; this module glues it to the engine's entity
//! iteration and gather filters for the disturbing-element loop.

use crate::ai::RepulsivePoint as StaticRepulsivePoint;
use crate::coordinates::{MapBBox, MapPoint, MapVec, MoveBox, MoveBoxHalfDiagonal};
use crate::element::{Entity, EntityId};
use crate::element_kinds::{ElementKind, Posture};
use crate::entities::EntityNeighbours;
use crate::fast_find_grid::FastFindGrid;
use crate::position_interface::{RADIUS_GUY, compute_deviated_future};
use crate::profiles::ProfileManager;
use crate::repulsive::{RepulsiveLine, RepulsivePoint};

/// Both constants are used for the repulsive lines built around
/// motion-sector perimeters.
pub const RADIUS_OBSTACLE_LINE: f32 = 0.0;
pub const ACTIONRADIUS_OBSTACLE: f32 = 5.0;

pub const ACTIONRADIUS_GUY: f32 = 12.0;
pub const RADIUS_CORPSE: f32 = 10.0;
pub const ACTIONRADIUS_CORPSE: f32 = 15.0;
const RADIUS_SWORDFIGHTING_GUY: f32 = 4.0;

/// Box half-diagonal around the acting actor's future position used
/// to pre-filter neighbours.
pub const MAX_REPULSIVE_DISTANCE: f32 = 60.0;

thread_local! {
    /// Current engine frame for the opt-in goal-owner anti-collision trace.
    /// This is process-local diagnostic context, never serialized game state.
    static GOAL_OWNER_ANTI_FRAME: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

pub(super) fn with_goal_owner_anti_frame<T>(frame: u32, f: impl FnOnce() -> T) -> T {
    if !super::diagnostics::config().goal_owner_enabled() {
        return f();
    }
    GOAL_OWNER_ANTI_FRAME.with(|slot| {
        let previous = slot.replace(Some(frame));
        let result = f();
        slot.set(previous);
        result
    })
}

pub(super) fn goal_owner_anti_debug_frame(mover: EntityId) -> Option<u32> {
    let frame = GOAL_OWNER_ANTI_FRAME.with(std::cell::Cell::get)?;
    super::diagnostics::config()
        .goal_owner_matches(frame, mover)
        .then_some(frame)
}

#[derive(Clone, Copy)]
pub(super) struct CollisionWorld<'a> {
    pub neighbours: EntityNeighbours<'a>,
    pub profiles: &'a ProfileManager,
}

/// Inputs retained only while the mover's position interface is mutably borrowed.
#[derive(Debug, Clone, Copy)]
pub(super) struct CollisionMover {
    pub id: EntityId,
    pub active: bool,
    pub position_map: MapPoint,
    pub layer: u16,
    pub sector: Option<crate::position_interface::SectorHandle>,
    pub element_kind: ElementKind,
    pub target_element: Option<EntityId>,
    pub is_swordfighting: bool,
}

impl CollisionMover {
    pub fn new(id: EntityId, entity: &Entity) -> Self {
        let elem = entity.element_data();
        Self {
            id,
            active: elem.active,
            position_map: elem.position_map(),
            layer: elem.optional_layer().map_or(u16::MAX, |layer| layer.get()),
            sector: elem.sector(),
            element_kind: elem.kind,
            target_element: entity.position_iface().target_element(),
            is_swordfighting: entity
                .human_data()
                .is_some_and(|human| !human.opponents.is_empty()),
        }
    }
}

/// Filter static (Lua-authored) repulsive points by the mover's
/// layer, element kind, and bounding box.  The point list lives on
/// `EngineInner::ai_global.repulsive_points` since the Lua
/// `AddRepulsivePoint` native stores there (natives/mod.rs:4280).
///
/// `flags` bit layout:
/// bit 0 = affects PCs, bit 1 = soldiers, bit 2 = civilians, bit 3 = animals.
pub(super) fn gather_static_repulsive_points(
    mover: &CollisionMover,
    static_points: &[StaticRepulsivePoint],
    box_future: &MapBBox,
) -> Vec<RepulsivePoint> {
    let affect_bit = match mover.element_kind {
        ElementKind::ActorPc => 1,
        ElementKind::ActorSoldier => 2,
        ElementKind::ActorCivilian => 4,
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for sp in static_points {
        if sp.position.level != mover.layer {
            continue;
        }
        if (sp.flags & affect_bit) == 0 {
            continue;
        }
        let p = crate::coordinates::MapPoint::new(sp.position.x, sp.position.y);
        if !box_future.contains_point(p) {
            continue;
        }
        out.push(RepulsivePoint {
            position: p,
            radius: sp.radius,
            action_radius: sp.action_radius,
            force_a: sp.force_a,
            force_b: sp.force_b,
            // The spatial grid deserializes static points into a default
            // POINT_TOTAL instance. The limit vectors and concavity are
            // nevertheless retained on the authoritative AI owner so a
            // subsequent legacy save remains lossless.
            is_total: true,
            is_concave: sp.concave,
            limit_left: sp.limit_left,
            limit_right: sp.limit_right,
        });
    }
    out
}

/// Computes the primary repulsive point for an actor, covering the
/// human, animal, and object cases.  Returns `None` when the actor's
/// posture contributes no repulsive zone (ladder / wall / carried /
/// flying).
///
/// For static scenery and animal-specific geometry we currently
/// handle the common single-point case here; animals' secondary
/// front/back points and their body line are assembled by the caller
/// from the entity when it passes the candidate filters.
pub fn entity_repulsive_point(
    entity: &Entity,
    profile_manager: &ProfileManager,
) -> Option<RepulsivePoint> {
    let elem = entity.element_data();
    let pos = elem.position_map();
    let posture = elem.posture();

    if !entity.is_human() {
        // Object-level repulsive point: emits one point using the
        // object's per-type radius and a fixed action radius of 10,
        // only when the radius is non-zero.
        match entity {
            Entity::Bonus(b) => {
                use crate::element_kinds::ObjectType;
                // Ground-dropped ale bottles / purses carry the
                // *accessory* object type on an `Entity::Bonus` (see
                // `engine::ale::spawn_dropped_ale`).  Ale uses radius
                // 5, purse uses radius 7 (matching the in-flight
                // purse case in the projectile arm below).  All other
                // bonus variants are non-repulsive.
                let radius = match b.object.object_type {
                    ObjectType::Ale => 5.0,
                    ObjectType::Purse => 7.0,
                    _ => return None,
                };
                return Some(RepulsivePoint::new(pos, radius, 10.0));
            }
            Entity::Scroll(_) => {
                // Scrolls are non-repulsive.
                return None;
            }
            Entity::Projectile(proj) => {
                use crate::element_kinds::ObjectType;
                let radius = match proj.object.object_type {
                    ObjectType::Ale => 5.0,
                    ObjectType::Purse => 7.0,
                    // Coins are explicitly non-repulsive even though
                    // they have a non-zero hit radius — the original
                    // game overrides them out of the anti-collision
                    // list.  Cape / Apple / Arrow / Stone / Wasp /
                    // WaspNest are all non-repulsive too.
                    _ => return None,
                };
                return Some(RepulsivePoint::new(pos, radius, 10.0));
            }
            Entity::Net(net) => {
                // Only crumpled-false nets already landed with
                // victims contribute, and they emit two concentric
                // repulsive points.
                if net.projectile.flying || net.net.crumpled || net.net.victims.is_empty() {
                    return None;
                }
                // The second point is returned through
                // `entity_extra_repulsive_points` so the caller can
                // pick both up.
                return Some(RepulsivePoint::new(pos, 40.0, 15.0));
            }
            Entity::Fx(_) | Entity::Target(_) => {
                return None;
            }
            _ => return None,
        }
    }

    // Rider override: radius 20, action radius 35.
    if entity_is_rider(entity) {
        return Some(RepulsivePoint::new(pos, 20.0, 35.0));
    }

    // Swordfighting override.  Only applies to the active-upright
    // cases below; the lying/corpse branch ignores it.  Uses radius 4
    // and an action radius equal to half the sword's max range.
    let swordfighting = entity
        .human_data()
        .map(|h| !h.opponents.is_empty())
        .unwrap_or(false)
        && matches!(
            posture,
            Posture::Upright
                | Posture::HelpingToClimb
                | Posture::CarryingOnShoulders
                | Posture::LeaningOut
                | Posture::Leisure
        );
    if swordfighting && let Some(ar) = swordfighting_action_radius(entity, profile_manager) {
        return Some(RepulsivePoint::new(pos, RADIUS_SWORDFIGHTING_GUY, ar));
    }

    match posture {
        Posture::Upright
        | Posture::HelpingToClimb
        | Posture::CarryingOnShoulders
        | Posture::LeaningOut
        | Posture::Leisure
        | Posture::Crouched
        | Posture::Siesta
        | Posture::CarryingCorpse
        | Posture::Spy
        | Posture::Cloaked
        | Posture::AnonymousArcher => Some(RepulsivePoint::new(pos, RADIUS_GUY, ACTIONRADIUS_GUY)),

        Posture::Lying
        | Posture::Dead
        | Posture::DeadBack
        | Posture::StuckUnderNet
        | Posture::Tied => {
            let small = entity
                .human_data()
                .map(|h| h.small_repulsive_radius)
                .unwrap_or(false);
            if small {
                Some(RepulsivePoint::new(pos, 5.0, 7.0))
            } else {
                Some(RepulsivePoint::new(pos, RADIUS_CORPSE, ACTIONRADIUS_CORPSE))
            }
        }

        Posture::SimulatingBeggar | Posture::Sitting | Posture::Tree => {
            // Offset the repulsive point 10 units behind the actor's
            // facing direction so the "seated" character's
            // personal-space zone sits in front of them.
            let dir = elem.direction() as u16 & 15;
            let (dx, dy) = direction_vector(dir);
            let offset_pos =
                crate::coordinates::MapPoint::new(pos.x - 10.0 * dx, pos.y - 10.0 * dy);
            Some(RepulsivePoint::new(
                offset_pos,
                RADIUS_CORPSE,
                ACTIONRADIUS_CORPSE,
            ))
        }

        // No repulsive zone: on-ladder / on-wall / carried / on-shoulders
        // / flying / undefined / unused.
        _ => None,
    }
}

/// Secondary repulsive points produced by specific entity subtypes.
/// Animals emit nothing here (single-point).  Landed nets with
/// victims emit an outer ring in addition to the inner point.
pub fn entity_extra_repulsive_points(entity: &Entity) -> Vec<RepulsivePoint> {
    if let Entity::Net(net) = entity
        && !net.projectile.flying
        && !net.net.crumpled
        && !net.net.victims.is_empty()
    {
        let elem = entity.element_data();
        let pos = elem.position_map();
        return vec![RepulsivePoint::new(pos, 15.0, 30.0)];
    }
    Vec::new()
}

/// Compute the swordfighting action-radius override — half the
/// actor's sword max range.  Returns `None` when the actor has no
/// lookup-able weapon profile (e.g. civilian without a sword).
fn swordfighting_action_radius(entity: &Entity, profile_manager: &ProfileManager) -> Option<f32> {
    let idx = crate::engine::melee::get_hth_weapon_id_full(entity, profile_manager)?;
    let profile = profile_manager.get_hth_weapon(idx)?;
    let max = profile.distance[crate::weapons::WeaponDistance::Maximal as usize];
    Some(0.5 * max as f32)
}

/// True when the entity is a mounted soldier.
fn entity_is_rider(entity: &Entity) -> bool {
    matches!(entity, Entity::Soldier(s) if s.soldier.rider)
}

/// Convert a 16-sector compass direction (0 = north / -Y, CW) into a
/// unit vector.
fn direction_vector(dir: u16) -> (f32, f32) {
    // Element direction-vector lookup uses
    // Aspect-corrected direction-sector assignment: the compass table is
    // Euclidean in X, but its Y component is compressed for the isometric
    // map. This matters for the offset repulsive center of sitting actors.
    let (x, y) = crate::element_kinds::direction_vector_16(dir as i16);
    (x, y * crate::position_interface::ASPECT_RATIO)
}

/// Gather the disturbing-actor filter for the anti-collision loop. Mobile
/// perimeter lines are supplied separately to [`apply_anti_collision_step`]
/// because their master elements do not occupy entity slots.
///
/// Neighbours are borrowed directly from the entity arena. `box_future`
/// bounds the mover's prospective position; neighbours outside are rejected.
///
/// Movement direction is passed in via `increment` (the unit vector
/// the mover is currently heading along).  The "dot product ≥ 5"
/// prefilter rejects neighbours that are fully behind the mover's
/// direction of travel.
pub(super) fn gather_disturbing(
    mover: &CollisionMover,
    world: CollisionWorld<'_>,
    box_future: &MapBBox,
    increment: MapVec,
) -> (Vec<RepulsivePoint>, Vec<crate::repulsive::RepulsiveLine>) {
    let mut points = Vec::new();
    let lines = Vec::new();
    for (other_id, other) in world.neighbours.occupied() {
        if other_id == mover.id {
            continue;
        }
        let elem = other.element_data();
        if !elem.active {
            continue;
        }
        if elem.optional_layer().map(|layer| layer.get()) != Some(mover.layer) {
            continue;
        }
        // Strict sector equality — sector handles compare directly,
        // so a sectorless mover rejects sectored neighbours and vice
        // versa.
        if elem.sector() != mover.sector {
            continue;
        }
        // Target-element filter: mover never treats its own target
        // as disturbing.  Actors walking up to a horse they'll
        // mount, carrying onto a corpse they'll pick up, etc. need
        // to pass through without deviation.
        if let Some(tgt) = mover.target_element
            && tgt == other_id
        {
            continue;
        }
        // Objects and actors share the ignored-for-anti-collision
        // check.
        if other
            .actor_data()
            .is_some_and(|actor| actor.is_ignored_for_anti_collision)
        {
            continue;
        }
        if !other.is_actor() && !other.is_object() {
            continue;
        }
        let is_object = other.is_object();
        if !is_object {
            // Actor-specific filters.
            if elem.position_map().x == mover.position_map.x
                && elem.position_map().y == mover.position_map.y
            {
                continue;
            }
            if other.is_human() && elem.posture() == Posture::Carried {
                continue;
            }
            // Swordfighters close on downed opponents without being
            // repelled by them — skip Lying / Dead / StuckUnderNet
            // postures when the mover is a swordfighting human.
            // DeadBack is *not* in the skip set — that looks like a
            // bug in the original game, but we preserve it so
            // behaviour matches.
            if mover.is_swordfighting
                && matches!(
                    elem.posture(),
                    Posture::Lying | Posture::Dead | Posture::StuckUnderNet
                )
            {
                continue;
            }
        }
        if !box_future.contains_point(elem.position_map()) {
            continue;
        }
        if !is_object {
            let rel = MapVec::new(
                elem.position_map().x - mover.position_map.x,
                elem.position_map().y - mover.position_map.y,
            );
            let dot = increment.x * rel.x + increment.y * rel.y;
            if dot < 5.0 {
                continue;
            }
        }
        if let Some(pt) = entity_repulsive_point(other, world.profiles) {
            points.push(pt);
        }
        points.extend(entity_extra_repulsive_points(other));
    }
    (points, lines)
}

/// Full state passed to [`apply_anti_collision_step`] — a mutable
/// borrow of the actor's `PositionInterface` (which owns the
/// persistent `deviated` / `blocked_count` / `box_blocked` / `radius`
/// fields directly) plus per-tick transient context.
pub struct AntiCollisionState<'a> {
    pub pi: &'a mut crate::position_interface::PositionInterface,
    /// Zero-centred move box for the mover.  Supplies the extents
    /// needed by `is_straight_movement_authorized` / the
    /// `find_authorized_position` fallback.
    pub move_box: crate::coordinates::MoveBox,
    /// Half-diagonal used by `is_reachable_thick`.
    pub half_diagonal: MoveBoxHalfDiagonal,
    /// Current movement goal (for the break-through barge).
    pub goal_map: MapPoint,
}

impl AntiCollisionState<'_> {
    fn update_box_blocked(&mut self, point: MapPoint) -> bool {
        let p = &mut *self.pi;
        if p.box_blocked.is_somewhere() && p.box_blocked.contains_point(point) {
            p.blocked_count = p.blocked_count.saturating_add(1);
            if p.radius > 1.0 {
                p.radius -= 0.2;
            }
            false
        } else {
            let half = crate::coordinates::MapVec::new(0.49, 0.49);
            p.box_blocked.expand_point(point + half);
            p.box_blocked.expand_point(point - half);
            p.blocked_count = 0;
            p.radius = p.radius_initial;
            true
        }
    }
}

/// Compute the deviated step for an actor whose naive next position
/// would be `origin + (nx, ny) * speed`, taking into account other
/// actors' repulsive zones.  Returns `(new_dx, new_dy)` — the deltas
/// the caller should add to `elem.position_map`.
///
/// `state` is the per-actor persistent anti-collision state.  When
/// `Some` with a grid supplied, the full pipeline runs: after
/// deviation the corridor is checked against
/// `is_straight_movement_authorized` and `is_reachable_thick`; if
/// that fails the blocked counter climbs and the break-through
/// barge / `find_authorized_position` escape hatch fires.  When
/// `state` is `None`, only the pure deviation math runs (for
/// standalone call sites and unit tests).
pub(super) fn apply_anti_collision_step(
    mover: &CollisionMover,
    world: CollisionWorld<'_>,
    static_points: &[StaticRepulsivePoint],
    grid: Option<&FastFindGrid>,
    mut state: Option<&mut AntiCollisionState>,
    nx: f32,
    ny: f32,
    speed: f32,
    anti_collision_on: bool,
) -> (f32, f32) {
    let mut mover = *mover;
    if let Some(state) = state.as_deref() {
        mover.position_map = state.pi.map_position();
        mover.target_element = state.pi.target_element();
    }
    let mover = &mover;
    let naive = (nx * speed, ny * speed);
    // Sprite motion only updates anti-collision position when the
    // owning actor is active. Inactive actors still execute scripted motion,
    // but commit the naive step without touching persistent deviation state.
    if !anti_collision_on || !mover.active {
        return naive;
    }

    // Anti-collision position updating derives the future box's
    // half diagonal from the current radius. Repeated blocked moves can
    // shrink that radius below RADIUS_GUY, and the narrower query can exclude
    // a neighbour that would otherwise enable the obstacle-point pass.
    let actor_radius = state.as_deref().map(|s| s.pi.radius).unwrap_or(RADIUS_GUY);
    let future = MapPoint::new(
        mover.position_map.x + naive.0,
        mover.position_map.y + naive.1,
    );
    let half = MAX_REPULSIVE_DISTANCE + actor_radius;
    let box_future = MapBBox::from_corners(
        MapPoint::new(future.x - half, future.y - half),
        MapPoint::new(future.x + half, future.y + half),
    );

    let increment = MapVec::new(nx, ny);
    let (mut points, mut lines) = gather_disturbing(mover, world, &box_future, increment);
    // Supported mobile records have no embedded sight obstacles. Their
    // complete sight-obstacle box stays unset, so they contribute no objects
    // to this collision query, including after movement or save restoration.
    // TODO: when embedded mobile sight obstacles are supported, query their
    // live geometry here and retain contributing membership for this solve.
    points.extend(gather_static_repulsive_points(
        mover,
        static_points,
        &box_future,
    ));
    if let Some(grid) = grid {
        // The level-authored obstacle points/lines are only added
        // when at least one actor-contributed (or mobile) repulsive
        // object already made the list, and each obstacle is then
        // re-filtered by Euclidean distance to the *current*
        // position.  Both conditions are required so stray level
        // geometry doesn't push actors around far from any
        // neighbour.
        if !points.is_empty() || !lines.is_empty() {
            let obstacle_lines = gather_level_repulsive_lines(grid, mover.layer, &box_future);
            let obstacle_points = gather_level_repulsive_points(grid, mover.layer, &box_future);
            for p in obstacle_points {
                let rel = MapVec::new(
                    mover.position_map.x - p.position.x,
                    mover.position_map.y - p.position.y,
                );
                let dist = rel.length();
                // The original threshold is `input_action_radius +
                // radius`.  In our `RepulsivePoint`, `action_radius`
                // already stores `input_action_radius + radius`, so
                // the threshold becomes `p.action_radius +
                // p.radius`.
                if dist <= p.action_radius + p.radius {
                    points.push(p);
                }
            }
            for l in obstacle_lines {
                let rel = MapVec::new(mover.position_map.x - l.a.x, mover.position_map.y - l.a.y);
                let dist = rel.x * l.normal.x + rel.y * l.normal.y;
                if dist <= l.action_radius + l.radius {
                    lines.push(l);
                }
            }
        }
    }

    if let Some(frame) = goal_owner_anti_debug_frame(mover.id) {
        let relevant_neighbours = world
            .neighbours
            .occupied()
            .filter(|(_, candidate)| {
                box_future.contains_point(candidate.element_data().position_map())
            })
            .map(|(id, candidate)| (id, candidate.element_data().position_map()))
            .collect::<Vec<_>>();
        eprintln!(
            "[GOAL_OWNER frame={frame} owner={:?} stage=anti_gather origin_bits={:08x},{:08x} increment_bits={:08x},{:08x} speed_bits={:08x} future_bits={:08x},{:08x} goal_bits={:08x},{:08x} layer={} radius_bits={:08x} was_deviated={} blocked_count={} neighbours={relevant_neighbours:?} points={points:?} lines={lines:?}]",
            mover.id,
            mover.position_map.x.to_bits(),
            mover.position_map.y.to_bits(),
            nx.to_bits(),
            ny.to_bits(),
            speed.to_bits(),
            future.x.to_bits(),
            future.y.to_bits(),
            state.as_deref().map_or(0, |s| s.goal_map.x.to_bits()),
            state.as_deref().map_or(0, |s| s.goal_map.y.to_bits()),
            mover.layer,
            actor_radius.to_bits(),
            state.as_deref().is_some_and(|s| s.pi.deviated),
            state.as_deref().map_or(0, |s| s.pi.blocked_count),
        );
    }

    let was_deviated = state.as_deref().is_some_and(|s| s.pi.deviated);
    if was_deviated {
        tracing::trace!(
            mover = ?mover.id,
            origin = ?mover.position_map,
            future = ?future,
            increment = ?increment,
            speed,
            points = ?points,
            lines = ?lines,
            "anti-collision deviation inputs"
        );
    }

    let lists_empty = points.is_empty() && lines.is_empty();
    if lists_empty {
        // No repulsive objects.  Three sub-cases:
        //   * Not deviated → commit naive.
        //   * Deviated + old trajectory reachable → clear flag, commit
        //     naive.
        //   * Deviated + !reachable → *fall through* to the
        //     authorized-commit / blocked-count / break-through-toward-
        //     goal pipeline.  An earlier port returned `naive` here,
        //     which stranded actors at the edge of unreachable
        //     regions because the safety valve never fired.
        let was_deviated = state.as_deref().is_some_and(|s| s.pi.deviated);
        if !was_deviated {
            return naive;
        }
        let reachable = match (state.as_deref(), grid) {
            (Some(s), Some(g)) => g.is_reachable_thick(
                future.to_geo().into(),
                s.goal_map.to_geo().into(),
                mover.layer,
                s.half_diagonal,
            ),
            _ => false,
        };
        if reachable {
            if let Some(s) = state.as_deref_mut() {
                s.pi.deviated = false;
            }
            return naive;
        }
        // Deviated && !reachable: fall through.
    }

    // Anti-collision position updating gathers repulsive
    // objects first, but returns immediately when that gathered pass has a
    // zero movement vector.  In particular it does not run the later
    // no-new-deviation recovery that clears the deviation flag and rebuilds the
    // cached increment.  Keep this after the empty-list arm above: Original
    // tries to recover the old trajectory first when there are no repulsive
    // objects, even if the requested step itself is zero.
    if future == mover.position_map {
        return (0.0, 0.0);
    }

    // The original game forwards the animation's requested distance unchanged to
    // every repulsive object's deviation calculation. Recomputing the norm from
    // the rounded `(increment * distance)` vector changes the deviation by
    // ULPs even when the cached increment is unit length.
    let (deviated_future, deviated) = compute_deviated_future(
        mover.position_map,
        future,
        speed,
        actor_radius,
        points,
        lines,
    );
    if deviated || was_deviated {
        tracing::trace!(
            mover = ?mover.id,
            deviated,
            deviated_future = ?deviated_future,
            actor_radius,
            "anti-collision deviation result"
        );
    }

    // The original game computes the future point from position plus scaled increment and assigns
    // that point directly when none of the gathered repulsive objects
    // actually deflects the actor. Returning `ptFuture - position` here made
    // the caller add the rounded delta a second time. At map coordinates in
    // the thousands that cancellation can move the result by several ULPs,
    // eventually changing exact goal-reached decisions and patrol history.
    if !deviated && !state.as_deref().is_some_and(|s| s.pi.deviated) {
        return naive;
    }

    // Without state-tracking, commit the deviated future directly.
    let Some(state) = state else {
        return (
            deviated_future.x - mover.position_map.x,
            deviated_future.y - mover.position_map.y,
        );
    };

    if !deviated {
        // Deviation loop didn't deflect.
        //   * Not previously deviated → commit.
        //   * Was deviated + reachable → clear flag, commit.
        //   * Was deviated + !reachable → *fall through* (same
        //     fall-through-on-unreachable behaviour as the pre-loop
        //     arm above; lets the blocked-count and
        //     break-through-toward-goal passes run for stranded
        //     actors that the deviation math couldn't help).
        if !state.pi.deviated {
            return (
                deviated_future.x - mover.position_map.x,
                deviated_future.y - mover.position_map.y,
            );
        }
        let reachable = match grid {
            Some(g) => g.is_reachable_thick(
                deviated_future.to_geo().into(),
                state.goal_map.to_geo().into(),
                mover.layer,
                state.half_diagonal,
            ),
            None => false,
        };
        if reachable {
            state.pi.deviated = false;
            return naive;
        }
        // Was deviated && !reachable: fall through.
    }

    // Deviation happened — verify the corridor is walkable.  When the
    // grid is unavailable (tests, non-level callers) commit the
    // deviated step unchecked (matches previous behaviour).
    let grid = match grid {
        Some(g) => g,
        None => {
            state.pi.deviated = true;
            return (
                deviated_future.x - mover.position_map.x,
                deviated_future.y - mover.position_map.y,
            );
        }
    };

    let straight_authorized = grid.is_straight_movement_authorized(
        mover.position_map.to_geo().into(),
        deviated_future.to_geo().into(),
        mover.layer,
        &state.move_box,
    );
    let reachable_to_goal = grid.is_reachable_thick(
        deviated_future.to_geo().into(),
        state.goal_map.to_geo().into(),
        mover.layer,
        state.half_diagonal,
    );
    let can_commit = straight_authorized && reachable_to_goal;

    if can_commit {
        // Commit the deviation and track it in the blocked-box so
        // repeated moves in the same cell bump the blocked counter.
        if state.update_box_blocked(deviated_future) {
            let step = (
                deviated_future.x - mover.position_map.x,
                deviated_future.y - mover.position_map.y,
            );
            state.pi.deviated = true;
            return step;
        }
    } else {
        // Corridor blocked — bump counter + shrink radius.
        state.pi.blocked_count = state.pi.blocked_count.saturating_add(1);
        if state.pi.radius > 1.0 {
            state.pi.radius -= 0.2;
        }
    }

    // Break-through barge: charge toward the goal; if the straight
    // move isn't authorised, shrink it until it is, and if even that
    // fails widen the box and ask the grid for any authorised cell
    // nearby.
    if state.pi.blocked_count > 0 {
        let to_goal = MapVec::new(
            state.goal_map.x - mover.position_map.x,
            state.goal_map.y - mover.position_map.y,
        );
        let len = to_goal.length();
        let n = if len > 0.0 {
            MapVec::new(to_goal.x / len, to_goal.y / len)
        } else {
            MapVec::ZERO
        };
        let mut barge = MapVec::new(n.x * speed, n.y * speed);

        // The barge faces along its own charge vector, and does so
        // before the authorisation tests below — a mover that ends up
        // completely stuck still turns to face the goal it is trying to
        // reach.  Unlike the committed-deviation facing applied by the
        // caller, this one is binned without the isometric Y-stretch and
        // ignores a reversing order's flipped facing.
        state
            .pi
            .set_direction(crate::position_interface::Direction::from_raw(
                crate::position_interface::vector_to_sector_0_to_15(barge.x, barge.y) as i32,
            ));

        let mut barge_future = MapPoint::new(
            mover.position_map.x + barge.x,
            mover.position_map.y + barge.y,
        );

        // Inset the move box by 1 unit.
        let box_inset = if let Some(r) = state.move_box.0 {
            MoveBox::from_corners(
                MapVec::new(r.min().x + 1.0, r.min().y + 1.0),
                MapVec::new(r.max().x - 1.0, r.max().y - 1.0),
            )
        } else {
            MoveBox::new()
        };

        if grid.is_position_authorized(&box_inset.translated(barge_future), mover.layer) {
            state.pi.deviated = true;
            return (barge.x, barge.y);
        }

        let mut slower = speed;
        while slower > 0.1 {
            if grid.is_position_authorized(&box_inset.translated(barge_future), mover.layer) {
                state.pi.deviated = true;
                return (barge.x, barge.y);
            }
            slower *= 0.8;
            barge = barge.scale(0.8);
            barge_future = MapPoint::new(
                mover.position_map.x + barge.x,
                mover.position_map.y + barge.y,
            );
        }

        // Widen the box a touch and hand it to the grid's
        // nearest-authorised-position search.  Success teleports the
        // actor to the found cell's centre.
        let mut widened_map = state.move_box.translated(barge_future);
        if let Some(r) = widened_map.0 {
            widened_map = MapBBox::from_corners(
                MapPoint::new(r.min().x - 0.2, r.min().y - 0.2),
                MapPoint::new(r.max().x + 0.2, r.max().y + 0.2),
            );
        }
        if grid.find_authorized_position(&mut widened_map, mover.layer) {
            let c = widened_map.center();
            state.pi.deviated = true;
            return (c.x - mover.position_map.x, c.y - mover.position_map.y);
        }

        // No barge possible — stay put, as in the shipped game.
        // The blocked counter keeps climbing so AI eventually
        // repaths out.
        state.pi.deviated = true;
        return (0.0, 0.0);
    }

    // No deviation committed and no barge — stay put.
    state.pi.deviated = true;
    (0.0, 0.0)
}

/// Build `RepulsiveLine`s from the level's `LINE_REPULSIVE` grid lines
/// intersecting `box_future` on `layer`.  The force params come from
/// `RADIUS_OBSTACLE_LINE` / `ACTIONRADIUS_OBSTACLE`.
pub fn gather_level_repulsive_lines(
    grid: &FastFindGrid,
    layer: u16,
    box_future: &MapBBox,
) -> Vec<RepulsiveLine> {
    let indices = grid.get_active_repulsive_line_indices(layer, box_future);
    indices
        .into_iter()
        .map(|idx| {
            let g = &grid.level.lines[usize::from(idx)];
            repulsive_line_from_grid(g)
        })
        .collect()
}

fn repulsive_line_from_grid(g: &crate::fast_find_grid::GridLine) -> RepulsiveLine {
    let mut line = RepulsiveLine::new(g.a, g.b, RADIUS_OBSTACLE_LINE, ACTIONRADIUS_OBSTACLE);
    // Repulsive-line normal initialization points AREA-sector boundaries
    // opposite to solid-obstacle boundaries. `GridLine` already retained
    // that authoritative oriented normal when the motion sector was
    // constructed; rebuilding it solely from the endpoints silently turns
    // every AREA line into a non-AREA line.
    line.normal = g.normal;
    let direct = MapVec::new(-line.vector.y, line.vector.x);
    line.is_area = g.normal.x * direct.x + g.normal.y * direct.y > 0.0;
    line
}

/// Build `RepulsivePoint`s from the level's corner / outward-angle
/// repulsive points.  Each point inherits the action field (wedge)
/// from the corner it was generated for.
pub fn gather_level_repulsive_points(
    grid: &FastFindGrid,
    layer: u16,
    box_future: &MapBBox,
) -> Vec<RepulsivePoint> {
    grid.get_level_repulsive_points(layer, box_future)
        .into_iter()
        .map(|p| {
            let mut rp =
                RepulsivePoint::new(p.position, RADIUS_OBSTACLE_LINE, ACTIONRADIUS_OBSTACLE);
            rp.set_action_field(p.limit_left, p.limit_right);
            rp.is_concave = p.is_concave;
            rp
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::map_pt;
    use crate::element::{ActorPc, ElementData};
    use crate::entities::Entities;
    use crate::entity_id::PcId;

    fn pc(x: f32, posture: Posture) -> Entity {
        let mut element = ElementData::from_initial_posture(posture);
        element.active = true;
        element.kind = ElementKind::ActorPc;
        element.set_layer(0);
        element.set_sector(crate::position_interface::SectorHandle::new(1));
        element.set_position_map(map_pt(x, 0.0));
        Entity::Pc(ActorPc {
            element,
            actor: Default::default(),
            human: Default::default(),
            pc: Default::default(),
        })
    }

    fn step(entities: &mut Entities, speed: f32, anti_on: bool) -> (f32, f32) {
        let profiles = ProfileManager::new();
        let (entity, neighbours) = entities.split_owner(PcId(0)).unwrap();
        let mover = CollisionMover::new(PcId(0).into(), entity);
        apply_anti_collision_step(
            &mover,
            CollisionWorld {
                neighbours,
                profiles: &profiles,
            },
            &[],
            None,
            None,
            1.0,
            0.0,
            speed,
            anti_on,
        )
    }

    #[test]
    fn live_candidate_filters_preserve_the_straight_step() {
        let cases: &[(&str, fn(&mut Entity, &mut Entity))] = &[
            ("inactive", |_, other| {
                other.element_data_mut().active = false
            }),
            ("different layer", |_, other| {
                other.element_data_mut().set_layer(1)
            }),
            ("different sector", |_, other| {
                other.element_data_mut().set_sector(None)
            }),
            ("carried", |_, other| *other = pc(8.0, Posture::Carried)),
            ("ignored", |_, other| {
                other
                    .actor_data_mut()
                    .unwrap()
                    .is_ignored_for_anti_collision = true
            }),
            ("outside future box", |_, other| {
                other
                    .element_data_mut()
                    .set_position_map(map_pt(200.0, 0.0))
            }),
            ("behind", |_, other| {
                other.element_data_mut().set_position_map(map_pt(-8.0, 0.0))
            }),
            ("same position", |_, other| {
                other.element_data_mut().set_position_map(map_pt(0.0, 0.0))
            }),
            ("current target", |mover, _| {
                mover
                    .position_iface_mut()
                    .set_target_element(Some(PcId(1).into()))
            }),
            ("swordfighter corpse", |mover, other| {
                mover
                    .human_data_mut()
                    .unwrap()
                    .opponents
                    .push(PcId(1).into());
                *other = pc(8.0, Posture::Dead);
            }),
        ];
        for (name, configure) in cases {
            let mut mover = pc(0.0, Posture::Upright);
            let mut other = pc(8.0, Posture::Upright);
            configure(&mut mover, &mut other);
            let mut entities = Entities::from_legacy_slots(vec![Some(mover), Some(other)]);
            assert_eq!(step(&mut entities, 1.0, true), (1.0, 0.0), "{name}");
        }
    }

    #[test]
    fn candidate_mutations_are_visible_on_the_next_owner_borrow() {
        let mut entities = Entities::from_legacy_slots(vec![
            Some(pc(0.0, Posture::Upright)),
            Some(pc(8.0, Posture::Upright)),
        ]);
        assert_ne!(step(&mut entities, 1.0, true), (1.0, 0.0));
        entities
            .get_mut(PcId(1))
            .unwrap()
            .element_data_mut()
            .set_position_map(map_pt(200.0, 0.0));
        assert_eq!(step(&mut entities, 1.0, true), (1.0, 0.0));
        entities
            .get_mut(PcId(1))
            .unwrap()
            .element_data_mut()
            .set_position_map(map_pt(8.0, 0.0));
        assert_ne!(step(&mut entities, 1.0, true), (1.0, 0.0));
        assert_eq!(step(&mut entities, 1.0, false), (1.0, 0.0));
        entities.get_mut(PcId(0)).unwrap().element_data_mut().active = false;
        assert_eq!(step(&mut entities, 1.0, true), (1.0, 0.0));
    }

    #[test]
    fn split_owner_preserves_candidate_slot_order() {
        let mut entities = Entities::from_legacy_slots(vec![
            Some(pc(10.0, Posture::Upright)),
            None,
            Some(pc(0.0, Posture::Upright)),
            Some(pc(20.0, Posture::Upright)),
        ]);
        let profiles = ProfileManager::new();
        let (owner, neighbours) = entities.split_owner(PcId(2)).unwrap();
        assert_eq!(
            neighbours.occupied().map(|(id, _)| id).collect::<Vec<_>>(),
            vec![EntityId::Pc(PcId(0)), EntityId::Pc(PcId(3))]
        );
        let mover = CollisionMover::new(PcId(2).into(), owner);
        let (points, _) = gather_disturbing(
            &mover,
            CollisionWorld {
                neighbours,
                profiles: &profiles,
            },
            &MapBBox::from_corners(map_pt(-60.0, -60.0), map_pt(60.0, 60.0)),
            MapVec::new(1.0, 0.0),
        );
        assert_eq!(
            points
                .iter()
                .map(|point| point.position)
                .collect::<Vec<_>>(),
            vec![map_pt(10.0, 0.0), map_pt(20.0, 0.0)]
        );
    }

    #[test]
    fn current_motion_target_overrides_stale_seek_target() {
        let mut mover = pc(0.0, Posture::Upright);
        mover.actor_data_mut().unwrap().seek_target = Some(PcId(1).into());
        let mut entities =
            Entities::from_legacy_slots(vec![Some(mover), Some(pc(8.0, Posture::Dead))]);
        assert_ne!(step(&mut entities, 1.0, true), (1.0, 0.0));
        entities
            .get_mut(PcId(0))
            .unwrap()
            .position_iface_mut()
            .set_target_element(Some(PcId(1).into()));
        assert_eq!(step(&mut entities, 1.0, true), (1.0, 0.0));
    }

    #[test]
    fn no_layer_neighbors_are_rejected_before_repulsive_geometry() {
        let mut inactive = pc(8.0, Posture::Upright);
        inactive.element_data_mut().active = false;
        inactive.element_data_mut().clear_layer();
        let mut projectile_element = ElementData::default();
        projectile_element.active = true;
        projectile_element.kind = ElementKind::ObjectProjectile;
        projectile_element.clear_layer();
        let projectile = Entity::Projectile(crate::element::ElementProjectile {
            element: projectile_element,
            object: crate::element::ObjectData {
                object_type: crate::element_kinds::ObjectType::Purse,
                ..Default::default()
            },
            projectile: Default::default(),
        });
        let mut entities = Entities::from_legacy_slots(vec![
            Some(pc(0.0, Posture::Upright)),
            Some(inactive),
            Some(projectile),
        ]);
        assert_eq!(step(&mut entities, 1.0, true), (1.0, 0.0));
    }

    #[test]
    fn dropped_ale_contributes_live_repulsive_geometry() {
        let mut element = ElementData::default();
        element.active = true;
        element.kind = ElementKind::ObjectOther;
        element.set_layer(0);
        element.set_sector(crate::position_interface::SectorHandle::new(1));
        element.set_position_map(map_pt(10.0, 0.0));
        let ale = Entity::Bonus(crate::element::ElementBonus {
            element,
            object: crate::element::ObjectData {
                object_type: crate::element_kinds::ObjectType::Ale,
                ..Default::default()
            },
        });
        let mut entities =
            Entities::from_legacy_slots(vec![Some(pc(0.0, Posture::Upright)), Some(ale)]);
        assert_ne!(step(&mut entities, 1.0, true), (1.0, 0.0));
    }

    #[test]
    fn zero_step_recovers_only_when_repulsive_lists_are_empty() {
        for disturbed in [false, true] {
            let mut entities = Entities::from_legacy_slots(vec![
                Some(pc(0.0, Posture::Upright)),
                disturbed.then(|| pc(8.0, Posture::Upright)),
            ]);
            let profiles = ProfileManager::new();
            let grid = FastFindGrid::default();
            let (entity, neighbours) = entities.split_owner(PcId(0)).unwrap();
            let mover = CollisionMover::new(PcId(0).into(), entity);
            entity.position_iface_mut().deviated = true;
            let mut state = AntiCollisionState {
                pi: entity.position_iface_mut(),
                move_box: Default::default(),
                half_diagonal: MoveBoxHalfDiagonal::new(6.0, 4.0),
                goal_map: map_pt(10.0, 0.0),
            };
            assert_eq!(
                apply_anti_collision_step(
                    &mover,
                    CollisionWorld {
                        neighbours,
                        profiles: &profiles
                    },
                    &[],
                    Some(&grid),
                    Some(&mut state),
                    1.0,
                    0.0,
                    0.0,
                    true,
                ),
                (0.0, 0.0)
            );
            assert_eq!(state.pi.deviated, disturbed);
        }
    }

    #[test]
    fn level_repulsive_lines_preserve_area_oriented_normals() {
        let a = map_pt(0.0, 0.0);
        let b = map_pt(10.0, 0.0);

        let mut area = crate::fast_find_grid::GridLine::new(a, b, true);
        area.initialize_motion_normal(true);
        let area_repulsive = repulsive_line_from_grid(&area);
        assert_eq!(area_repulsive.normal, MapVec::new(0.0, 1.0));
        assert!(area_repulsive.is_area);

        let mut obstacle = crate::fast_find_grid::GridLine::new(a, b, true);
        obstacle.initialize_motion_normal(false);
        let obstacle_repulsive = repulsive_line_from_grid(&obstacle);
        assert_eq!(obstacle_repulsive.normal, MapVec::new(0.0, -1.0));
        assert!(!obstacle_repulsive.is_area);
    }

    #[test]
    fn swordfighter_repulsive_point_matches_original_force() {
        let mut profile_manager = crate::profiles::ProfileManager::new();
        profile_manager
            .characters
            .push(crate::profiles::CharacterProfile {
                hth_weapon_id: 1,
                ..Default::default()
            });
        let mut weapon = crate::profiles::HtHWeaponProfile::default();
        weapon.distance[crate::weapons::WeaponDistance::Maximal as usize] = 50;
        profile_manager.hth_weapons.push(weapon);

        let mut element = {
            let mut initial_element =
                crate::element::ElementData::from_initial_posture(Posture::Upright);
            initial_element.kind = ElementKind::ActorPc;
            initial_element
        };
        element.set_position_map(MapPoint::new(10.0, 20.0));

        let mut human = crate::element::HumanData::default();
        human
            .opponents
            .push(EntityId::Pc(crate::entity_id::PcId(2)));
        let entity = Entity::Pc(crate::element::ActorPc {
            element,
            actor: crate::element::ActorData::default(),
            human,
            pc: crate::element::PcData::default(),
        });

        let point = entity_repulsive_point(&entity, &profile_manager).unwrap();
        assert_eq!(point.position, map_pt(10.0, 20.0));
        assert_eq!(point.radius, RADIUS_SWORDFIGHTING_GUY);
        assert_eq!(point.action_radius, RADIUS_SWORDFIGHTING_GUY + 25.0);
    }

    #[test]
    fn static_repulsive_point_retains_saved_force_and_field_geometry() {
        let a = CollisionMover::new(PcId(0).into(), &pc(0.0, Posture::Upright));
        let saved = StaticRepulsivePoint {
            id: 7,
            position: crate::ai::Position {
                x: 8.0,
                y: 0.0,
                sector: None,
                level: 0,
            },
            radius: 11.0,
            action_radius: 37.0,
            force_a: 0.125,
            force_b: -1.375,
            concave: true,
            limit_left: crate::coordinates::MapVec::new(1.0, 2.0),
            limit_right: crate::coordinates::MapVec::new(3.0, 4.0),
            flags: 1,
        };
        let points = gather_static_repulsive_points(
            &a,
            &[saved],
            &MapBBox::from_corners(MapPoint::new(-1.0, -1.0), MapPoint::new(9.0, 1.0)),
        );

        assert_eq!(points.len(), 1);
        let point = points[0];
        assert_eq!(point.radius, 11.0);
        assert_eq!(point.action_radius, 37.0);
        assert_eq!(point.force_a, 0.125);
        assert_eq!(point.force_b, -1.375);
        assert!(point.is_total);
        assert!(point.is_concave);
        assert_eq!(point.limit_left, crate::coordinates::MapVec::new(1.0, 2.0));
        assert_eq!(point.limit_right, crate::coordinates::MapVec::new(3.0, 4.0));
    }
}
