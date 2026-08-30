//! Per-tick helpers for actor-vs-actor anti-collision.
//!
//! The pure math lives in [`crate::position_interface::compute_deviated_future`]
//! / [`crate::repulsive`]; this module glues it to the engine's entity
//! iteration and gather filters for the disturbing-element loop.  The
//! Mission chariots contribute their translated motion-sector perimeter as
//! repulsive lines and thick-corridor blockers.

use crate::ai::RepulsivePoint as StaticRepulsivePoint;
use crate::coordinates::{MapBBox, MapPoint, MapVec, MoveBox, MoveBoxHalfDiagonal};
use crate::element::{Entity, EntityId};
use crate::element_kinds::{ElementKind, Posture};
use crate::entities::{Entities, EntitySlots};
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
    if std::env::var_os("PARITY_DEBUG_GOAL_OWNER_HANDOFF").is_none() {
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
    std::env::var_os("PARITY_DEBUG_GOAL_OWNER_HANDOFF")?;
    let frame = GOAL_OWNER_ANTI_FRAME.with(std::cell::Cell::get)?;
    let expected_frame = std::env::var("PARITY_DEBUG_GOAL_OWNER_FRAME")
        .unwrap_or_else(|_| {
            panic!("PARITY_DEBUG_GOAL_OWNER_HANDOFF requires PARITY_DEBUG_GOAL_OWNER_FRAME=FRAME")
        })
        .parse::<u32>()
        .unwrap_or_else(|error| panic!("invalid PARITY_DEBUG_GOAL_OWNER_FRAME: {error}"));
    if frame != expected_frame {
        return None;
    }
    let filter = std::env::var("PARITY_DEBUG_GOAL_OWNER").unwrap_or_else(|_| {
        panic!(
            "PARITY_DEBUG_GOAL_OWNER_HANDOFF requires PARITY_DEBUG_GOAL_OWNER=pc|soldier|civilian:INDEX"
        )
    });
    let (kind, index) = filter.split_once(':').unwrap_or_else(|| {
        panic!("PARITY_DEBUG_GOAL_OWNER must look like pc|soldier|civilian:INDEX")
    });
    let index = index
        .parse::<u32>()
        .unwrap_or_else(|error| panic!("invalid PARITY_DEBUG_GOAL_OWNER={filter:?}: {error}"));
    let kind_matches = matches!(
        (kind, mover),
        ("pc", EntityId::Pc(_))
            | ("soldier", EntityId::Soldier(_))
            | ("civilian", EntityId::Civilian(_))
    );
    if kind_matches && mover.index() == index {
        Some(frame)
    } else if matches!(kind, "pc" | "soldier" | "civilian") {
        None
    } else {
        panic!("PARITY_DEBUG_GOAL_OWNER has unsupported kind {kind:?}")
    }
}

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

/// Build a snapshot array indexed by entity slot. Slots without an actor or
/// object are filled with `None`.
///
/// `profile_manager` is used to look up per-entity sword / rider
/// overrides so the right force parameters end up on each snapshot.
pub fn snapshot_all(
    entities: &Entities,
    profile_manager: &ProfileManager,
) -> EntitySlots<Option<ActorSnapshot>> {
    let mut snapshots = EntitySlots::filled(entities.len(), None);
    for (snapshot_id, entity) in entities.occupied() {
        if entity.actor_data().is_none() && !entity.is_object() {
            continue;
        }
        let elem = entity.element_data();
        // Original's disturbing-element loop tests IsActive() before it
        // reads GetLayer(), GetSector(), or any repulsive geometry. Loaded
        // replaced-PC corpses can be inactive and retain the serialized
        // no-layer sentinel, so do not eagerly inspect state the original
        // short-circuit never reaches.
        if !elem.active {
            continue;
        }
        let is_actor = entity.is_actor();
        let actor = entity.actor_data();
        // RHsprite::PerformMotion copies pOrderCurrent->pAntagonist into
        // PositionInterface::mpTargetElement whenever a motion order starts.
        // Intermediate path orders deliberately have no antagonist, even
        // though the owning Seek element and ActorData still retain their
        // eventual target. Using either persistent value here makes actors
        // ignore that target for the entire path instead of only on the final
        // approach order.
        let target_element = entity.position_iface().target_element();
        snapshots[snapshot_id] = Some(ActorSnapshot {
            id: snapshot_id,
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

/// Update a cached actor snapshot after its movement step is committed.
/// Later actors in the same serial movement pass must see the moved
/// footprint, including animal offset points and body lines.
pub fn sync_snapshot_after_move(
    snapshot: &mut ActorSnapshot,
    new_position: MapPoint,
    movement: MapVec,
) {
    snapshot.position_map = new_position;
    if let Some(point) = snapshot.repulsive_point.as_mut() {
        point.position = new_position;
    }
    for point in &mut snapshot.extra_repulsive_points {
        point.position.x += movement.x;
        point.position.y += movement.y;
    }
    for line in &mut snapshot.repulsive_lines {
        line.a.x += movement.x;
        line.a.y += movement.y;
        line.b.x += movement.x;
        line.b.y += movement.y;
    }
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
            // RHFastFindGrid deserializes static points into a default
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
    // `RHElement::GetDirectionVector` calls
    // `SetSector0to15(direction, ASPECT_RATIO)`: the compass table is
    // Euclidean in X, but its Y component is compressed for the isometric
    // map. This matters for the offset repulsive center of sitting actors.
    let (x, y) = crate::element_kinds::direction_vector_16(dir as i16);
    (x, y * crate::position_interface::ASPECT_RATIO)
}

/// Gather the disturbing-actor filter for the anti-collision loop. Mobile
/// perimeter lines are supplied separately to [`apply_anti_collision_step`]
/// because their master elements do not occupy entity slots.
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
    box_future: &MapBBox,
    increment: MapVec,
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
        if !box_future.contains_point(other.position_map) {
            continue;
        }
        if !is_object {
            let rel = MapVec::new(
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
#[allow(clippy::too_many_arguments)]
pub fn apply_anti_collision_step(
    mover: &ActorSnapshot,
    neighbours: &[Option<ActorSnapshot>],
    static_points: &[StaticRepulsivePoint],
    mobile_points: &[RepulsivePoint],
    mobile_lines: &[crate::fast_find_grid::GridLine],
    mobile_polygons: &[Vec<MapPoint>],
    grid: Option<&FastFindGrid>,
    mut state: Option<&mut AntiCollisionState>,
    nx: f32,
    ny: f32,
    speed: f32,
    anti_collision_on: bool,
) -> (f32, f32) {
    let naive = (nx * speed, ny * speed);
    // RHSprite::PerformMotion only calls UpdatePositionAntiCollision when the
    // owning actor is active. Inactive actors still execute scripted motion,
    // but commit the naive step without touching persistent deviation state.
    if !anti_collision_on || !mover.active {
        return naive;
    }
    if mover.repulsive_point.is_none() && !mover.is_actor {
        // Non-actors don't have their own repulsive footprint —
        // they just stomp through.
        return naive;
    }

    // RHPositionInterface::UpdatePositionAntiCollision derives boxFuture's
    // half diagonal from the *current* mfRadius. Repeated blocked moves can
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
    let (mut points, mut lines) = gather_disturbing(mover, neighbours, &box_future, increment);
    // FastFindGrid::GetMobileRepulsiveObjects first rejects a mobile whose
    // complete motion sector misses boxFuture, then contributes all of that
    // mobile's repulsive objects. Released missions contain one mobile per
    // level, so a single intersection gate exactly preserves that grouping.
    let mobile_interferes = mobile_polygons
        .iter()
        .any(|polygon| crate::mobile::MobileElement::polygon_intersects_bbox(polygon, &box_future));
    if mobile_interferes {
        points.extend(mobile_points.iter().copied());
        lines.extend(
            mobile_lines
                .iter()
                .map(|line| RepulsiveLine::new(line.a, line.b, 0.0, 15.0)),
        );
    }
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
        let relevant_neighbours = neighbours
            .iter()
            .flatten()
            .filter(|candidate| box_future.contains_point(candidate.position_map))
            .collect::<Vec<_>>();
        eprintln!(
            "[GOAL_OWNER frame={frame} owner={:?} stage=anti_gather origin_bits={:08x},{:08x} increment_bits={:08x},{:08x} speed_bits={:08x} future_bits={:08x},{:08x} goal_bits={:08x},{:08x} layer={} radius_bits={:08x} was_deviated={} blocked_count={} mobile_interferes={} neighbours={relevant_neighbours:?} points={points:?} lines={lines:?}]",
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
            mobile_interferes,
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
            if mobile_interferes
                && let Some(grid) = grid
                && is_blocked_by_mobile(
                    grid,
                    mover.position_map,
                    future,
                    mover.layer,
                    state
                        .as_deref()
                        .expect("deviated state disappeared")
                        .half_diagonal,
                    &state
                        .as_deref()
                        .expect("deviated state disappeared")
                        .move_box,
                    mobile_lines,
                    mobile_polygons,
                )
            {
                return (0.0, 0.0);
            }
            if let Some(s) = state.as_deref_mut() {
                s.pi.deviated = false;
            }
            return naive;
        }
        // Deviated && !reachable: fall through.
    }

    // RHPositionInterface::UpdatePositionAntiCollision gathers repulsive
    // objects first, but returns immediately when that gathered pass has a
    // zero movement vector.  In particular it does not run the later
    // no-new-deviation recovery that clears `IsDeviated` and rebuilds the
    // cached increment.  Keep this after the empty-list arm above: Original
    // tries to recover the old trajectory first when there are no repulsive
    // objects, even if the requested step itself is zero.
    if future == mover.position_map {
        return (0.0, 0.0);
    }

    // Original forwards the animation's requested `fDistance` unchanged to
    // every repulsive object's `ComputeDeviation`. Recomputing the norm from
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

    // Original keeps `ptFuture = position + increment * distance` and assigns
    // that point directly when none of the gathered repulsive objects
    // actually deflects the actor. Returning `ptFuture - position` here made
    // the caller add the rounded delta a second time. At map coordinates in
    // the thousands that cancellation can move the result by several ULPs,
    // eventually changing exact IsGoalReached branches and patrol history.
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
            if mobile_interferes
                && is_blocked_by_mobile(
                    grid.expect("reachable mobile recovery requires a grid"),
                    mover.position_map,
                    deviated_future,
                    mover.layer,
                    state.half_diagonal,
                    &state.move_box,
                    mobile_lines,
                    mobile_polygons,
                )
            {
                return (0.0, 0.0);
            }
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
    let blocked_by_mobile = mobile_interferes
        && is_blocked_by_mobile(
            grid,
            mover.position_map,
            deviated_future,
            mover.layer,
            state.half_diagonal,
            &state.move_box,
            mobile_lines,
            mobile_polygons,
        );
    let reachable_to_goal = grid.is_reachable_thick(
        deviated_future.to_geo().into(),
        state.goal_map.to_geo().into(),
        mover.layer,
        state.half_diagonal,
    );
    let can_commit = straight_authorized && !blocked_by_mobile && reachable_to_goal;

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
        // The original checks the already-computed candidate against mobile
        // geometry before its break-through/barge escape. A cart is never
        // barged through, even when static anti-collision has been blocked
        // long enough to trigger the escape hatch.
        if mobile_interferes
            && is_blocked_by_mobile(
                grid,
                mover.position_map,
                deviated_future,
                mover.layer,
                state.half_diagonal,
                &state.move_box,
                mobile_lines,
                mobile_polygons,
            )
        {
            return (0.0, 0.0);
        }
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

/// C++ `RHPositionInterface::IsBlockedByMobile`: the thick movement
/// corridor must avoid both the cart's repulsive perimeter lines and a
/// destination move-box overlap with its full motion polygon.
#[allow(clippy::too_many_arguments)]
fn is_blocked_by_mobile(
    grid: &FastFindGrid,
    start: MapPoint,
    goal: MapPoint,
    layer: u16,
    half_diagonal: MoveBoxHalfDiagonal,
    move_box: &MoveBox,
    mobile_lines: &[crate::fast_find_grid::GridLine],
    mobile_polygons: &[Vec<MapPoint>],
) -> bool {
    if !grid.is_reachable_thick_mobile(start, goal, layer, half_diagonal, mobile_lines) {
        return true;
    }
    let goal_box = move_box.translated(goal);
    mobile_polygons
        .iter()
        .any(|polygon| crate::mobile::MobileElement::polygon_intersects_bbox(polygon, &goal_box))
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
    // `RHRepulsiveLine::InitializeNormal` points AREA-sector boundaries
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
    fn snapshot_all_skips_inactive_no_layer_actor_before_layer_access() {
        use crate::element::{ActorData, ActorPc, ElementData, HumanData, PcData};
        use crate::entity_id::PcId;

        let entity = Entity::Pc(ActorPc {
            element: ElementData {
                active: false,
                kind: ElementKind::ActorPc,
                ..Default::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        });
        let entities = Entities::from_legacy_slots(vec![Some(entity)]);

        let snapshots = snapshot_all(&entities, &ProfileManager::new());
        assert!(snapshots[PcId(0)].is_none());
    }

    fn snapshot_mover_and_corpse(
        order_antagonist: Option<EntityId>,
    ) -> (ActorSnapshot, ActorSnapshot) {
        use crate::element::{
            ActorData, ActorPc, ActorSoldier, ElementData, HumanData, NpcData, PcData, SoldierData,
        };
        use crate::entity_id::{PcId, SoldierId};
        use crate::movement::ActiveMovement;
        use crate::sequence::SequenceId;

        let corpse_id = EntityId::Soldier(SoldierId(1));

        let mut mover_element = ElementData {
            active: true,
            kind: ElementKind::ActorPc,
            posture: Posture::Upright,
            ..Default::default()
        };
        mover_element.set_position_map(map_pt(0.0, 0.0));
        mover_element.set_sector(crate::position_interface::SectorHandle::new(1));
        mover_element
            .sprite
            .position_iface
            .set_target_element(order_antagonist);
        let mover_actor = ActorData {
            // Deliberately stale: the preceding Seek targeted this soldier,
            // but the current sprite order is authoritative for anti-collision.
            seek_target: Some(corpse_id),
            // Friday cleanup can remove an interrupted movement sequence
            // before this derived Rust latch is reconciled. Original's
            // anti-collision reads RHPositionInterface::mpTargetElement and
            // never dereferences the old sequence pointer here.
            active_movement: ActiveMovement::new(SequenceId(999), 5),
            ..Default::default()
        };

        let mut corpse_element = ElementData {
            active: true,
            kind: ElementKind::ActorSoldier,
            posture: Posture::Dead,
            ..Default::default()
        };
        corpse_element.set_position_map(map_pt(8.0, 0.0));
        corpse_element.set_sector(crate::position_interface::SectorHandle::new(1));

        let entities = Entities::from_legacy_slots(vec![
            Some(Entity::Pc(ActorPc {
                element: mover_element,
                actor: mover_actor,
                human: HumanData::default(),
                pc: PcData::default(),
            })),
            Some(Entity::Soldier(ActorSoldier {
                element: corpse_element,
                actor: ActorData::default(),
                human: HumanData::default(),
                npc: NpcData::default(),
                soldier: SoldierData::default(),
            })),
        ]);
        let snapshots = snapshot_all(&entities, &ProfileManager::new());
        (
            snapshots[PcId(0)].clone().expect("mover snapshot"),
            snapshots[SoldierId(1)].clone().expect("corpse snapshot"),
        )
    }

    #[test]
    fn snapshot_all_includes_dropped_ale_as_a_repulsive_object() {
        use crate::element::{ElementBonus, ElementData, ObjectData};
        use crate::element_kinds::ObjectType;
        use crate::entity_id::BonusId;

        let mut element = ElementData {
            active: true,
            kind: ElementKind::ObjectOther,
            ..Default::default()
        };
        element.set_position_map(map_pt(557.0, 1184.0));
        element.set_sector(crate::position_interface::SectorHandle::new(0));
        let entities = Entities::from_legacy_slots(vec![Some(Entity::Bonus(ElementBonus {
            element,
            object: ObjectData {
                object_type: ObjectType::Ale,
                ..Default::default()
            },
        }))]);

        let snapshots = snapshot_all(&entities, &ProfileManager::new());
        let ale = snapshots[BonusId(0)]
            .as_ref()
            .expect("dropped ale snapshot");
        assert!(!ale.is_actor);
        assert_eq!(ale.position_map, map_pt(557.0, 1184.0));
        assert_eq!(
            ale.repulsive_point,
            Some(RepulsivePoint::new(map_pt(557.0, 1184.0), 5.0, 10.0))
        );

        let mut mover = mk_snapshot(1, 545.0, 1184.0);
        mover.sector = crate::position_interface::SectorHandle::new(0);
        let movement = apply_anti_collision_step(
            &mover,
            snapshots.as_slice(),
            &[],
            &[],
            &[],
            &[],
            None,
            None,
            1.0,
            0.0,
            1.0,
            true,
        );
        assert_ne!(movement, (1.0, 0.0));
    }

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
    fn inactive_mover_bypasses_anti_collision() {
        let mut mover = mk_snapshot(0, 0.0, 0.0);
        mover.active = false;
        let neighbour = mk_snapshot(1, 8.0, 0.0);
        let snapshots = vec![Some(mover.clone()), Some(neighbour)];

        let movement = apply_anti_collision_step(
            &mover,
            &snapshots,
            &[],
            &[],
            &[],
            &[],
            None,
            None,
            1.0,
            0.0,
            1.0,
            true,
        );

        assert_eq!(movement, (1.0, 0.0));
    }

    #[test]
    fn zero_step_with_repulsive_object_preserves_deviation() {
        let mover = mk_snapshot(0, 0.0, 0.0);
        let mut object = mk_snapshot(1, 10.0, 0.0);
        object.is_actor = false;
        object.is_human = false;
        object.repulsive_point = Some(RepulsivePoint::new(map_pt(10.0, 0.0), 5.0, 10.0));
        let snapshots = vec![Some(mover.clone()), Some(object)];

        let mut pi = crate::position_interface::PositionInterface::default();
        pi.deviated = true;
        let mut state = AntiCollisionState {
            pi: &mut pi,
            move_box: Default::default(),
            half_diagonal: MoveBoxHalfDiagonal::new(6.0, 4.0),
            goal_map: map_pt(10.0, 0.0),
        };
        let grid = FastFindGrid::default();

        let movement = apply_anti_collision_step(
            &mover,
            &snapshots,
            &[],
            &[],
            &[],
            &[],
            Some(&grid),
            Some(&mut state),
            0.0,
            0.0,
            1.8,
            true,
        );

        assert_eq!(movement, (0.0, 0.0));
        assert!(state.pi.deviated);
    }

    #[test]
    fn zero_step_with_empty_repulsive_lists_recovers_deviation_first() {
        let mover = mk_snapshot(0, 0.0, 0.0);
        let snapshots = vec![Some(mover.clone())];

        let mut pi = crate::position_interface::PositionInterface::default();
        pi.deviated = true;
        let mut state = AntiCollisionState {
            pi: &mut pi,
            move_box: Default::default(),
            half_diagonal: MoveBoxHalfDiagonal::new(6.0, 4.0),
            goal_map: map_pt(10.0, 0.0),
        };
        let grid = FastFindGrid::default();

        let movement = apply_anti_collision_step(
            &mover,
            &snapshots,
            &[],
            &[],
            &[],
            &[],
            Some(&grid),
            Some(&mut state),
            0.0,
            0.0,
            1.8,
            true,
        );

        assert_eq!(movement, (0.0, 0.0));
        assert!(!state.pi.deviated);
    }

    #[test]
    fn two_actors_head_on_are_pushed_apart() {
        // A at (0,0) walking +X toward B at (8,0) — within Upright's
        // RADIUS_GUY (4) + ACTIONRADIUS_GUY (12) → deviation required.
        let a = mk_snapshot(0, 0.0, 0.0);
        let b = mk_snapshot(1, 8.0, 0.0);
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &[],
            &[],
            &[],
            &[],
            None,
            None,
            1.0,
            0.0,
            1.0,
            true,
        );
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
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &[],
            &[],
            &[],
            &[],
            None,
            None,
            -1.0,
            0.0,
            1.0,
            true,
        );
        assert!((dx - -1.0).abs() < 1e-4);
        assert!(dy.abs() < 1e-4);
    }

    #[test]
    fn disabled_anti_collision_skips_deviation() {
        let a = mk_snapshot(0, 0.0, 0.0);
        let b = mk_snapshot(1, 8.0, 0.0);
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        // anti_collision_on = false ⇒ naive step.
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &[],
            &[],
            &[],
            &[],
            None,
            None,
            1.0,
            0.0,
            1.0,
            false,
        );
        assert!((dx - 1.0).abs() < 1e-4);
        assert!(dy.abs() < 1e-4);
    }

    #[test]
    fn different_layer_neighbour_is_ignored() {
        let a = mk_snapshot(0, 0.0, 0.0);
        let mut b = mk_snapshot(1, 8.0, 0.0);
        b.layer = 1;
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &[],
            &[],
            &[],
            &[],
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
    fn ignored_for_anti_collision_neighbour_is_skipped() {
        let a = mk_snapshot(0, 0.0, 0.0);
        let mut b = mk_snapshot(1, 8.0, 0.0);
        b.is_ignored_for_anti_collision = true;
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &[],
            &[],
            &[],
            &[],
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
    fn far_neighbour_is_outside_box_future() {
        // Neighbour at x=200 — outside MAX_REPULSIVE_DISTANCE + radius
        // around the future position.
        let a = mk_snapshot(0, 0.0, 0.0);
        let b = mk_snapshot(1, 200.0, 0.0);
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &[],
            &[],
            &[],
            &[],
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
    fn target_element_is_skipped() {
        // Mover's seek_target == B's id → B contributes no push.
        let mut a = mk_snapshot(0, 0.0, 0.0);
        a.target_element = Some(EntityId::Pc(crate::entity_id::PcId(1)));
        let b = mk_snapshot(1, 8.0, 0.0);
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &[],
            &[],
            &[],
            &[],
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
    fn stale_seek_target_does_not_hide_corpse_from_targetless_current_order() {
        let (mover, corpse) = snapshot_mover_and_corpse(None);
        assert_eq!(mover.target_element, None);

        let neighbours = vec![Some(mover.clone()), Some(corpse)];
        let (dx, dy) = apply_anti_collision_step(
            &mover,
            &neighbours,
            &[],
            &[],
            &[],
            &[],
            None,
            None,
            1.0,
            0.0,
            1.0,
            true,
        );
        assert!(
            dy.abs() > 0.01 || (dx - 1.0).abs() > 0.01,
            "the current targetless Move must be repelled by the corpse, got dx={dx} dy={dy}"
        );
    }

    #[test]
    fn current_order_antagonist_is_hidden_from_anti_collision() {
        let corpse_id = EntityId::Soldier(crate::entity_id::SoldierId(1));
        let (mover, corpse) = snapshot_mover_and_corpse(Some(corpse_id));
        assert_eq!(mover.target_element, Some(corpse_id));

        let neighbours = vec![Some(mover.clone()), Some(corpse)];
        let (dx, dy) = apply_anti_collision_step(
            &mover,
            &neighbours,
            &[],
            &[],
            &[],
            &[],
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
    fn swordfighter_skips_corpses() {
        let mut a = mk_snapshot(0, 0.0, 0.0);
        a.is_swordfighting = true;
        let mut b = mk_snapshot(1, 8.0, 0.0);
        b.posture = Posture::Dead;
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &[],
            &[],
            &[],
            &[],
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
    fn sectorless_mover_rejects_sectored_neighbour() {
        // Strict sector equality — sectorless vs. Some(1) should skip.
        let mut a = mk_snapshot(0, 0.0, 0.0);
        a.sector = None;
        let b = mk_snapshot(1, 8.0, 0.0);
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &[],
            &[],
            &[],
            &[],
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
    fn carried_neighbour_is_skipped() {
        let a = mk_snapshot(0, 0.0, 0.0);
        let mut b = mk_snapshot(1, 8.0, 0.0);
        b.posture = Posture::Carried;
        let snapshots = vec![Some(a.clone()), Some(b.clone())];
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &[],
            &[],
            &[],
            &[],
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
            action_radius: RADIUS_GUY + ACTIONRADIUS_GUY,
            force_a: 1.0 / ACTIONRADIUS_GUY,
            force_b: -RADIUS_GUY / ACTIONRADIUS_GUY,
            concave: false,
            limit_left: crate::coordinates::MapVec::ZERO,
            limit_right: crate::coordinates::MapVec::ZERO,
            flags: 1,
        }];
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &static_points,
            &[],
            &[],
            &[],
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
    fn static_repulsive_point_retains_saved_force_and_field_geometry() {
        let a = mk_snapshot(0, 0.0, 0.0);
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
            action_radius: RADIUS_GUY + ACTIONRADIUS_GUY,
            force_a: 1.0 / ACTIONRADIUS_GUY,
            force_b: -RADIUS_GUY / ACTIONRADIUS_GUY,
            concave: false,
            limit_left: crate::coordinates::MapVec::ZERO,
            limit_right: crate::coordinates::MapVec::ZERO,
            flags: 2,
        }];
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &static_points,
            &[],
            &[],
            &[],
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
            action_radius: RADIUS_GUY + ACTIONRADIUS_GUY,
            force_a: 1.0 / ACTIONRADIUS_GUY,
            force_b: -RADIUS_GUY / ACTIONRADIUS_GUY,
            concave: false,
            limit_left: crate::coordinates::MapVec::ZERO,
            limit_right: crate::coordinates::MapVec::ZERO,
            flags: 1,
        }];
        let (dx, dy) = apply_anti_collision_step(
            &a,
            &snapshots,
            &static_points,
            &[],
            &[],
            &[],
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
