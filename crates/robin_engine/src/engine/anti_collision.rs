//! Per-tick helpers for actor-vs-actor anti-collision.
//!
//! The pure math lives in [`crate::position_interface::compute_deviated_future`]
//! / [`crate::repulsive`]; this module glues it to the engine's entity
//! iteration and gather filters for the disturbing-element loop.  The
//! mobile-element arm of that loop is omitted — shipped RH missions
//! don't use mobile elements (no trains, platforms, etc.), so there's
//! no live call site.

use crate::ai::RepulsivePoint as StaticRepulsivePoint;
use crate::coordinates::MapPoint;
use crate::element::{Entity, EntityId};
use crate::element_kinds::{ElementKind, Posture};
use crate::entities::Entities;
use crate::fast_find_grid::FastFindGrid;
use crate::geo2d::{self, BBox2D, GeoPoint2D};
use crate::position_interface::{RADIUS_GUY, compute_deviated_future};
use crate::profiles::ProfileManager;
use crate::repulsive::{RepulsiveLine, RepulsivePoint};
use crate::sequence::{SequenceElementData, SequenceManager};

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

/// Snapshot of everything the anti-collision pre-pass needs from a
/// neighbour actor.  Captured once per tick — neighbour positions are
/// not re-read as the mutable loop walks entities, matching the
/// deterministic start-of-tick view the replay system relies on.
#[derive(Debug, Clone)]
pub struct ActorSnapshot {
    pub id: EntityId,
    pub active: bool,
    pub is_actor: bool,
    pub is_human: bool,
    pub is_ignored_for_anti_collision: bool,
    /// Map-space position captured at the start of the anti-collision pass.
    pub position_map: MapPoint,
    pub layer: u16,
    pub sector: Option<crate::position_interface::SectorHandle>,
    pub posture: Posture,
    /// Element kind — used to filter static repulsive points by
    /// their `affects_*` flags.
    pub element_kind: ElementKind,
    /// Mover's current movement target / antagonist.  The mover
    /// never treats its target as disturbing.
    pub target_element: Option<EntityId>,
    /// True when the mover is actively swordfighting — drives the
    /// corpse-skip filter.  Only meaningful when `is_human == true`.
    pub is_swordfighting: bool,
    /// The primary repulsive point this actor contributes when
    /// disturbed, or `None` if the actor's posture produces no
    /// repulsive zone.
    pub repulsive_point: Option<RepulsivePoint>,
    /// Additional points (animal front/back).  Empty for humans and
    /// objects — they only contribute the primary point.
    pub extra_repulsive_points: Vec<RepulsivePoint>,
    /// Repulsive lines (animal body-line).  Empty for everything
    /// except upright animals.
    pub repulsive_lines: Vec<crate::repulsive::RepulsiveLine>,
}

/// Build a snapshot array indexed by entity slot.  Slots without an
/// entity (or without actor data) are filled with `None`.
///
/// `profile_manager` is used to look up per-entity sword / rider
/// overrides so the right force parameters end up on each snapshot.
pub fn snapshot_all(
    entities: &Entities,
    sequence_manager: &SequenceManager,
    profile_manager: &ProfileManager,
) -> Vec<Option<ActorSnapshot>> {
    let mut snapshots = vec![None; entities.len()];
    for (entity_id, entity) in entities.actors() {
        let entity_id = EntityId::from(entity_id);
        let elem = entity.element_data();
        let is_actor = true;
        let actor = entity.actor_data();
        let target_element = actor.and_then(|a| {
            a.seek_target.or_else(|| {
                let seq_id = a.active_movement.sequence_id?;
                let elem = sequence_manager.get_element(seq_id, a.active_movement.element_index)?;
                match &elem.data {
                    SequenceElementData::Movement { element, .. } => *element,
                    _ => None,
                }
            })
        });
        snapshots[entity_id.index() as usize] = Some(ActorSnapshot {
            id: entity_id,
            active: elem.active,
            is_actor,
            is_human: entity.is_human(),
            is_ignored_for_anti_collision: actor
                .map(|a| a.is_ignored_for_anti_collision)
                .unwrap_or(false),
            position_map: elem.position_map(),
            layer: elem.layer(),
            sector: elem.sector(),
            posture: elem.posture,
            element_kind: elem.kind,
            // Prefer the live seek target, then fall back to the
            // active movement element's antagonist/target field.
            // For combat / pickup movements this is the opponent
            // / item the actor is closing on — the "don't repel
            // my target" rule applies to it.
            target_element,
            is_swordfighting: entity
                .human_data()
                .map(|h| !h.opponents.is_empty())
                .unwrap_or(false),
            repulsive_point: entity_repulsive_point(entity, profile_manager),
            extra_repulsive_points: entity_extra_repulsive_points(entity),
            repulsive_lines: entity_repulsive_lines(entity),
        });
    }
    snapshots
}

/// Filter static (Lua-authored) repulsive points by the mover's
/// layer, element kind, and bounding box.  The point list lives on
/// `EngineInner::ai_global.repulsive_points` since the Lua
/// `AddRepulsivePoint` native stores there (natives/mod.rs:4280).
///
/// `flags` bit layout:
/// bit 0 = affects PCs, bit 1 = soldiers, bit 2 = civilians, bit 3 = animals.
pub fn gather_static_repulsive_points(
    mover: &ActorSnapshot,
    static_points: &[StaticRepulsivePoint],
    box_future: &BBox2D,
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
        if !box_future.contains_point(p.to_geo()) {
            continue;
        }
        out.push(RepulsivePoint::new(p, sp.radius, sp.action_radius));
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
/// via [`entity_repulsive_lines`].
pub fn entity_repulsive_point(
    entity: &Entity,
    profile_manager: &ProfileManager,
) -> Option<RepulsivePoint> {
    let elem = entity.element_data();
    let pos = elem.position_map();
    let posture = elem.posture;

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

/// Animals only ever emit a single point — there's no secondary
/// front/back or body line in any code path, even though the engine
/// has fields for them.  Empty for every entity.
pub fn entity_repulsive_lines(_entity: &Entity) -> Vec<crate::repulsive::RepulsiveLine> {
    Vec::new()
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
    // Sector → angle: 0 = -Y (north), 4 = +X (east), 8 = +Y (south), 12 = -X (west).
    let angle = std::f32::consts::PI * (dir as f32) / 8.0 - std::f32::consts::FRAC_PI_2;
    (angle.cos(), angle.sin())
}

/// Gather the disturbing-actor filter for the anti-collision loop.
/// The mobile-element arm is omitted — shipped RH missions don't use
/// mobile elements.
///
/// `mover` is a snapshot of the actor that's about to move;
/// `neighbours` is the full snapshot array.  `box_future` is the
/// axis-aligned bounding box around the mover's prospective future
/// position — neighbours outside are rejected.
///
/// Movement direction is passed in via `increment` (the unit vector
/// the mover is currently heading along).  The "dot product ≥ 5"
/// prefilter rejects neighbours that are fully behind the mover's
/// direction of travel.
pub fn gather_disturbing(
    mover: &ActorSnapshot,
    neighbours: &[Option<ActorSnapshot>],
    box_future: &BBox2D,
    increment: geo2d::Vec2D,
) -> (Vec<RepulsivePoint>, Vec<crate::repulsive::RepulsiveLine>) {
    let mut points = Vec::new();
    let mut lines = Vec::new();
    for slot in neighbours {
        let other = match slot {
            Some(o) => o,
            None => continue,
        };
        if other.id == mover.id {
            continue;
        }
        if !other.active {
            continue;
        }
        if other.layer != mover.layer {
            continue;
        }
        // Strict sector equality — sector handles compare directly,
        // so a sectorless mover rejects sectored neighbours and vice
        // versa.
        if other.sector != mover.sector {
            continue;
        }
        // Target-element filter: mover never treats its own target
        // as disturbing.  Actors walking up to a horse they'll
        // mount, carrying onto a corpse they'll pick up, etc. need
        // to pass through without deviation.
        if let Some(tgt) = mover.target_element
            && tgt == other.id
        {
            continue;
        }
        // Objects and actors share the ignored-for-anti-collision
        // check.
        if other.is_ignored_for_anti_collision {
            continue;
        }
        let is_object = !other.is_actor;
        if !is_object {
            // Actor-specific filters.
            if other.position_map.x == mover.position_map.x
                && other.position_map.y == mover.position_map.y
            {
                continue;
            }
            if other.is_human && other.posture == Posture::Carried {
                continue;
            }
            // Swordfighters close on downed opponents without being
            // repelled by them — skip Lying / Dead / StuckUnderNet
            // postures when the mover is a swordfighting human.
            // DeadBack is *not* in the skip set — that looks like a
            // bug in the original game, but we preserve it so
            // behaviour matches.
            if mover.is_human
                && mover.is_swordfighting
                && matches!(
                    other.posture,
                    Posture::Lying | Posture::Dead | Posture::StuckUnderNet
                )
            {
                continue;
            }
        }
        if !box_future.contains_point(other.position_map.to_geo()) {
            continue;
        }
        if !is_object {
            let rel = geo2d::pt(
                other.position_map.x - mover.position_map.x,
                other.position_map.y - mover.position_map.y,
            );
            let dot = increment.x * rel.x + increment.y * rel.y;
            if dot < 5.0 {
                continue;
            }
        }
        if let Some(pt) = other.repulsive_point {
            points.push(pt);
        }
        points.extend(other.extra_repulsive_points.iter().copied());
        lines.extend(other.repulsive_lines.iter().copied());
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
    pub move_box: BBox2D,
    /// Half-diagonal used by `is_reachable_thick`.
    pub half_diagonal: geo2d::Vec2D,
    /// Current movement goal (for the break-through barge).
    pub goal_map: MapPoint,
}

impl AntiCollisionState<'_> {
    fn update_box_blocked(&mut self, point: MapPoint) -> bool {
        let p = &mut *self.pi;
        if p.box_blocked.is_somewhere() && p.box_blocked.contains_point(point.to_geo()) {
            p.blocked_count = p.blocked_count.saturating_add(1);
            if p.radius > 1.0 {
                p.radius -= 0.2;
            }
            false
        } else {
            let half = 0.49;
            p.box_blocked
                .expand_point(geo2d::pt(point.x + half, point.y + half));
            p.box_blocked
                .expand_point(geo2d::pt(point.x - half, point.y - half));
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
#[allow(clippy::too_many_arguments)]
pub fn apply_anti_collision_step(
    mover: &ActorSnapshot,
    neighbours: &[Option<ActorSnapshot>],
    static_points: &[StaticRepulsivePoint],
    grid: Option<&FastFindGrid>,
    mut state: Option<&mut AntiCollisionState>,
    nx: f32,
    ny: f32,
    speed: f32,
    anti_collision_on: bool,
) -> (f32, f32) {
    let naive = (nx * speed, ny * speed);
    if !anti_collision_on {
        return naive;
    }
    if mover.repulsive_point.is_none() && !mover.is_actor {
        // Non-actors don't have their own repulsive footprint —
        // they just stomp through.
        return naive;
    }

    let future = MapPoint::new(
        mover.position_map.x + naive.0,
        mover.position_map.y + naive.1,
    );
    let half = MAX_REPULSIVE_DISTANCE + RADIUS_GUY;
    let box_future = BBox2D::from_corners(
        geo2d::pt(future.x - half, future.y - half),
        geo2d::pt(future.x + half, future.y + half),
    );

    let increment = geo2d::pt(nx, ny);
    let (mut points, mut lines) = gather_disturbing(mover, neighbours, &box_future, increment);
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
                let rel = geo2d::pt(
                    mover.position_map.x - p.position.x,
                    mover.position_map.y - p.position.y,
                );
                let dist = geo2d::length(rel);
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
                let rel = geo2d::pt(mover.position_map.x - l.a.x, mover.position_map.y - l.a.y);
                let dist = rel.x * l.normal.x + rel.y * l.normal.y;
                if dist <= l.action_radius + l.radius {
                    lines.push(l);
                }
            }
        }
    }

    // The actor's effective radius may have shrunk if it's been
    // blocked (the blocked-count branch below shrinks it by 0.2 per
    // hit).  Honour that here so the sort + deviation math uses the
    // current radius.
    let actor_radius = state.as_deref().map(|s| s.pi.radius).unwrap_or(RADIUS_GUY);

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

    let (deviated_future, deviated) =
        compute_deviated_future(mover.position_map, future, actor_radius, points, lines);

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
            return (
                deviated_future.x - mover.position_map.x,
                deviated_future.y - mover.position_map.y,
            );
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

    let can_commit = grid.is_straight_movement_authorized(
        mover.position_map.to_geo().into(),
        deviated_future.to_geo().into(),
        mover.layer,
        &state.move_box,
    ) && grid.is_reachable_thick(
        deviated_future.to_geo().into(),
        state.goal_map.to_geo().into(),
        mover.layer,
        state.half_diagonal,
    );

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
        let to_goal = geo2d::pt(
            state.goal_map.x - mover.position_map.x,
            state.goal_map.y - mover.position_map.y,
        );
        let n = geo2d::normalize(to_goal);
        let mut barge = geo2d::pt(n.x * speed, n.y * speed);
        let mut barge_future = geo2d::pt(
            mover.position_map.x + barge.x,
            mover.position_map.y + barge.y,
        );

        // Inset the move box by 1 unit.
        let box_inset = if let Some(r) = state.move_box.0 {
            BBox2D(Some(geo::Rect::new(
                geo2d::pt(r.min().x + 1.0, r.min().y + 1.0),
                geo2d::pt(r.max().x - 1.0, r.max().y - 1.0),
            )))
        } else {
            BBox2D::new()
        };

        let offset = |b: &BBox2D, p: GeoPoint2D| -> BBox2D {
            if let Some(r) = b.0 {
                BBox2D(Some(geo::Rect::new(
                    geo2d::pt(r.min().x + p.x, r.min().y + p.y),
                    geo2d::pt(r.max().x + p.x, r.max().y + p.y),
                )))
            } else {
                BBox2D::new()
            }
        };

        if grid.is_position_authorized(&offset(&box_inset, barge_future), mover.layer) {
            state.pi.deviated = true;
            return (barge.x, barge.y);
        }

        let mut slower = speed;
        while slower > 0.1 {
            if grid.is_position_authorized(&offset(&box_inset, barge_future), mover.layer) {
                state.pi.deviated = true;
                return (barge.x, barge.y);
            }
            slower *= 0.8;
            barge = geo2d::pt(barge.x * 0.8, barge.y * 0.8);
            barge_future = geo2d::pt(
                mover.position_map.x + barge.x,
                mover.position_map.y + barge.y,
            );
        }

        // Widen the box a touch and hand it to the grid's
        // nearest-authorised-position search.  Success teleports the
        // actor to the found cell's centre.
        let mut widened = offset(&state.move_box, barge_future);
        if let Some(r) = widened.0 {
            widened = BBox2D(Some(geo::Rect::new(
                geo2d::pt(r.min().x - 0.2, r.min().y - 0.2),
                geo2d::pt(r.max().x + 0.2, r.max().y + 0.2),
            )));
        }
        if grid.find_authorized_position(&mut widened, mover.layer) {
            let c = widened.center();
            state.pi.deviated = true;
            return (c.x - mover.position_map.x, c.y - mover.position_map.y);
        }

        // No barge possible — stay put.  The original asserts here
        // in debug; release builds silently leave the sprite stuck.
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
    box_future: &BBox2D,
) -> Vec<RepulsiveLine> {
    let indices = grid.get_active_repulsive_line_indices(layer, box_future);
    indices
        .into_iter()
        .map(|idx| {
            let g = &grid.level.lines[usize::from(idx)];
            RepulsiveLine::new(
                g.a.into(),
                g.b.into(),
                RADIUS_OBSTACLE_LINE,
                ACTIONRADIUS_OBSTACLE,
            )
        })
        .collect()
}

/// Build `RepulsivePoint`s from the level's corner / outward-angle
/// repulsive points.  Each point inherits the action field (wedge)
/// from the corner it was generated for.
pub fn gather_level_repulsive_points(
    grid: &FastFindGrid,
    layer: u16,
    box_future: &BBox2D,
) -> Vec<RepulsivePoint> {
    grid.get_level_repulsive_points(layer, box_future)
        .into_iter()
        .map(|p| {
            let mut rp = RepulsivePoint::new(
                p.position.into(),
                RADIUS_OBSTACLE_LINE,
                ACTIONRADIUS_OBSTACLE,
            );
            rp.set_action_field(p.limit_left.into(), p.limit_right.into());
            rp.is_concave = p.is_concave;
            rp
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::map_pt;

    fn mk_snapshot(id: u32, x: f32, y: f32) -> ActorSnapshot {
        ActorSnapshot {
            id: EntityId::Pc(crate::entity_id::PcId(id)),
            active: true,
            is_actor: true,
            is_human: true,
            is_ignored_for_anti_collision: false,
            position_map: map_pt(x, y),
            layer: 0,
            sector: crate::position_interface::SectorHandle::new(1),
            posture: Posture::Upright,
            element_kind: ElementKind::ActorPc,
            target_element: None,
            is_swordfighting: false,
            repulsive_point: Some(RepulsivePoint::new(
                map_pt(x, y),
                RADIUS_GUY,
                ACTIONRADIUS_GUY,
            )),
            extra_repulsive_points: Vec::new(),
            repulsive_lines: Vec::new(),
        }
    }

    #[test]
    fn two_actors_head_on_are_pushed_apart() {
        // A at (0,0) walking +X toward B at (8,0) — within Upright's
        // RADIUS_GUY (4) + ACTIONRADIUS_GUY (12) → deviation required.
        let a = mk_snapshot(0, 0.0, 0.0);
        let b = mk_snapshot(1, 8.0, 0.0);
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) =
            apply_anti_collision_step(&a, &snapshots, &[], None, None, 1.0, 0.0, 1.0, true);
        // The step should be pushed sideways (|dy| > 0) and shortened
        // or redirected from the naive (1.0, 0.0).
        assert!(
            dy.abs() > 0.01,
            "expected sideways push, got dx={dx} dy={dy}"
        );
    }

    #[test]
    fn two_actors_back_to_back_are_not_affected() {
        // A walks -X, B is behind A at +X — the `increment · rel >= 5`
        // prefilter rejects neighbours behind the mover.
        let a = mk_snapshot(0, 0.0, 0.0);
        let b = mk_snapshot(1, 8.0, 0.0);
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) =
            apply_anti_collision_step(&a, &snapshots, &[], None, None, -1.0, 0.0, 1.0, true);
        assert!((dx - -1.0).abs() < 1e-4);
        assert!(dy.abs() < 1e-4);
    }

    #[test]
    fn disabled_anti_collision_skips_deviation() {
        let a = mk_snapshot(0, 0.0, 0.0);
        let b = mk_snapshot(1, 8.0, 0.0);
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        // anti_collision_on = false ⇒ naive step.
        let (dx, dy) =
            apply_anti_collision_step(&a, &snapshots, &[], None, None, 1.0, 0.0, 1.0, false);
        assert!((dx - 1.0).abs() < 1e-4);
        assert!(dy.abs() < 1e-4);
    }

    #[test]
    fn different_layer_neighbour_is_ignored() {
        let a = mk_snapshot(0, 0.0, 0.0);
        let mut b = mk_snapshot(1, 8.0, 0.0);
        b.layer = 1;
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) =
            apply_anti_collision_step(&a, &snapshots, &[], None, None, 1.0, 0.0, 1.0, true);
        assert!((dx - 1.0).abs() < 1e-4);
        assert!(dy.abs() < 1e-4);
    }

    #[test]
    fn ignored_for_anti_collision_neighbour_is_skipped() {
        let a = mk_snapshot(0, 0.0, 0.0);
        let mut b = mk_snapshot(1, 8.0, 0.0);
        b.is_ignored_for_anti_collision = true;
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) =
            apply_anti_collision_step(&a, &snapshots, &[], None, None, 1.0, 0.0, 1.0, true);
        assert!((dx - 1.0).abs() < 1e-4);
        assert!(dy.abs() < 1e-4);
    }

    #[test]
    fn far_neighbour_is_outside_box_future() {
        // Neighbour at x=200 — outside MAX_REPULSIVE_DISTANCE + radius
        // around the future position.
        let a = mk_snapshot(0, 0.0, 0.0);
        let b = mk_snapshot(1, 200.0, 0.0);
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) =
            apply_anti_collision_step(&a, &snapshots, &[], None, None, 1.0, 0.0, 1.0, true);
        assert!((dx - 1.0).abs() < 1e-4);
        assert!(dy.abs() < 1e-4);
    }

    #[test]
    fn target_element_is_skipped() {
        // Mover's seek_target == B's id → B contributes no push.
        let mut a = mk_snapshot(0, 0.0, 0.0);
        a.target_element = Some(EntityId::Pc(crate::entity_id::PcId(1)));
        let b = mk_snapshot(1, 8.0, 0.0);
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) =
            apply_anti_collision_step(&a, &snapshots, &[], None, None, 1.0, 0.0, 1.0, true);
        assert!((dx - 1.0).abs() < 1e-4);
        assert!(dy.abs() < 1e-4);
    }

    #[test]
    fn swordfighter_skips_corpses() {
        let mut a = mk_snapshot(0, 0.0, 0.0);
        a.is_swordfighting = true;
        let mut b = mk_snapshot(1, 8.0, 0.0);
        b.posture = Posture::Dead;
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) =
            apply_anti_collision_step(&a, &snapshots, &[], None, None, 1.0, 0.0, 1.0, true);
        assert!((dx - 1.0).abs() < 1e-4);
        assert!(dy.abs() < 1e-4);
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

        let mut element = crate::element::ElementData {
            kind: ElementKind::ActorPc,
            posture: Posture::Upright,
            ..Default::default()
        };
        element.set_position_map(geo2d::pt(10.0, 20.0).into());

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
    fn sectorless_mover_rejects_sectored_neighbour() {
        // Strict sector equality — sectorless vs. Some(1) should skip.
        let mut a = mk_snapshot(0, 0.0, 0.0);
        a.sector = None;
        let b = mk_snapshot(1, 8.0, 0.0);
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) =
            apply_anti_collision_step(&a, &snapshots, &[], None, None, 1.0, 0.0, 1.0, true);
        assert!((dx - 1.0).abs() < 1e-4);
        assert!(dy.abs() < 1e-4);
    }

    #[test]
    fn carried_neighbour_is_skipped() {
        let a = mk_snapshot(0, 0.0, 0.0);
        let mut b = mk_snapshot(1, 8.0, 0.0);
        b.posture = Posture::Carried;
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) =
            apply_anti_collision_step(&a, &snapshots, &[], None, None, 1.0, 0.0, 1.0, true);
        assert!((dx - 1.0).abs() < 1e-4);
        assert!(dy.abs() < 1e-4);
    }

    #[test]
    fn static_repulsive_point_with_matching_flag_deflects_pc() {
        // Static point at (8, 0) with flags = 1 (affects PCs).  A PC
        // walking +X should be deflected by the static point alone.
        let a = mk_snapshot(0, 0.0, 0.0);
        let snapshots = vec![Some(a.clone())];
        let static_points = vec![StaticRepulsivePoint {
            id: 1,
            position: crate::ai::Position {
                x: 8.0,
                y: 0.0,
                sector: None,
                level: 0,
            },
            radius: RADIUS_GUY,
            action_radius: ACTIONRADIUS_GUY,
            flags: 1,
        }];
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &static_points,
            None,
            None,
            1.0,
            0.0,
            1.0,
            true,
        );
        assert!(
            dy.abs() > 0.01,
            "expected static-point push, got dx={dx} dy={dy}"
        );
    }

    #[test]
    fn static_repulsive_point_with_wrong_flag_skipped_for_pc() {
        let a = mk_snapshot(0, 0.0, 0.0);
        let snapshots = vec![Some(a.clone())];
        // flags = 2 → affects soldiers only, not PCs.
        let static_points = vec![StaticRepulsivePoint {
            id: 1,
            position: crate::ai::Position {
                x: 8.0,
                y: 0.0,
                sector: None,
                level: 0,
            },
            radius: RADIUS_GUY,
            action_radius: ACTIONRADIUS_GUY,
            flags: 2,
        }];
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &static_points,
            None,
            None,
            1.0,
            0.0,
            1.0,
            true,
        );
        assert!((dx - 1.0).abs() < 1e-4);
        assert!(dy.abs() < 1e-4);
    }

    #[test]
    fn static_repulsive_point_different_layer_ignored() {
        let a = mk_snapshot(0, 0.0, 0.0);
        let snapshots = vec![Some(a.clone())];
        let static_points = vec![StaticRepulsivePoint {
            id: 1,
            position: crate::ai::Position {
                x: 8.0,
                y: 0.0,
                sector: None,
                level: 99,
            },
            radius: RADIUS_GUY,
            action_radius: ACTIONRADIUS_GUY,
            flags: 1,
        }];
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &static_points,
            None,
            None,
            1.0,
            0.0,
            1.0,
            true,
        );
        assert!((dx - 1.0).abs() < 1e-4);
        assert!(dy.abs() < 1e-4);
    }
}
