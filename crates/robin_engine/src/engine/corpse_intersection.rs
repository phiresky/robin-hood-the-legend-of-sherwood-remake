//! Corpse-intersection repulsion hook.
//!
//! **Why this exists.** Bodies lying on the ground carry a repulsion
//! field that shoves other actors out of their hitbox.  When two
//! corpses collapse on top of each other the normal radius is too
//! large — they'd repel each other forever and never settle.  The
//! fix is to shrink both corpses' radii
//! ([`HumanData::small_repulsive_radius`]) so intersecting bodies
//! sit still instead of self-launching.
//!
//! **How we wire it.** Many callers mutate `posture` directly through
//! [`Entity::human_and_posture_mut`] (e.g. [`crate::combat::tie_up`])
//! and the
//! [`ElementData::set_posture`](crate::element::ElementData::set_posture)
//! setter doesn't see `EngineInner`, so it cannot react by itself.
//! Instead,
//! [`HumanData::last_is_lying_for_corpse_intersection`] records the
//! previously-observed lying state. The live actor walk calls
//! [`EngineInner::process_corpse_intersection_update_for`] at every owner
//! boundary, before the next creation slot can sample repulsive state.
//! [`EngineInner::process_corpse_intersection_updates`] retains a full scan
//! after the actor walk as a fallback for posture writes outside those owner
//! envelopes.
//!
//! This closes the source-visible cross-owner boundary while retaining the
//! centralized tracker needed by direct posture mutations.

use crate::coordinates::MapPoint;
use crate::element::EntityId;
use std::collections::HashSet;

use super::EngineInner;

/// Corpse physics radius.
const RADIUS_CORPSE: f32 = 10.0;
/// Corpse action radius.
const ACTIONRADIUS_CORPSE: f32 = 15.0;

/// Squared distance threshold for "intersecting" corpses:
/// `4 * (RADIUS_CORPSE + ACTIONRADIUS_CORPSE)^2`.
const INTERSECT_SQ_DIST: f32 =
    4.0 * (RADIUS_CORPSE + ACTIONRADIUS_CORPSE) * (RADIUS_CORPSE + ACTIONRADIUS_CORPSE);

impl EngineInner {
    /// Close one human owner's `SetPosture` corpse-intersection boundary.
    ///
    /// Original performs this work synchronously from
    /// `RHElementActorHuman::SetPosture`, so a later creation slot must see
    /// the updated anti-collision flag and repulsive radius. The full drain
    /// remains as a fallback for posture writes outside the live actor walk.
    pub(crate) fn process_corpse_intersection_update_for(&mut self, id: EntityId) {
        let transition = {
            let Some(entity) = self.world.entities.get_mut(id) else {
                return;
            };
            let is_lying = entity.element_data().posture.is_lying();
            let Some(human) = entity.human_data_mut() else {
                return;
            };
            match human.last_is_lying_for_corpse_intersection {
                None => {
                    human.last_is_lying_for_corpse_intersection = Some(is_lying);
                    None
                }
                Some(previous) if previous != is_lying => {
                    human.last_is_lying_for_corpse_intersection = Some(is_lying);
                    Some(is_lying)
                }
                Some(_) => None,
            }
        };
        if let Some(b_added) = transition {
            self.update_intersecting_corpses_with_snapshot(id, b_added, None);
        }
    }

    /// Per-tick drain: detect lying↔non-lying posture transitions on
    /// every human and fire [`EngineInner::update_intersecting_corpses`]
    /// for each.
    ///
    /// Seeds [`HumanData::last_is_lying_for_corpse_intersection`] on
    /// the first observation (post-load or post-spawn) without
    /// firing an update, so serialized `small_repulsive_radius`
    /// values carry over untouched.
    pub(crate) fn process_corpse_intersection_updates(&mut self) {
        let mut transitions: Vec<(EntityId, bool)> = Vec::new();
        // Reconstruct the lying population that existed at the start of the
        // actor pass. Original calls UpdateIntersectingCorpses synchronously
        // from each SetPosture, so a later actor's new lying posture is not
        // visible to an earlier actor's callback. The Rust scan observes all
        // final postures at once; this staged set restores creation-order
        // visibility while retaining the centralized posture tracker.
        let mut logically_lying: HashSet<EntityId> = HashSet::new();

        for (entity_id, entity) in self.world.entities.humans_mut() {
            let is_lying = entity.element_data().posture.is_lying();
            let Some(human) = entity.human_data_mut() else {
                continue;
            };
            match human.last_is_lying_for_corpse_intersection {
                None => {
                    // First observation — seed the tracker without
                    // triggering an update.  Serialized flags stay
                    // authoritative across save/load.
                    human.last_is_lying_for_corpse_intersection = Some(is_lying);
                    if is_lying {
                        logically_lying.insert(entity_id.into());
                    }
                }
                Some(prev) if prev != is_lying => {
                    if prev {
                        logically_lying.insert(entity_id.into());
                    }
                    human.last_is_lying_for_corpse_intersection = Some(is_lying);
                    transitions.push((entity_id.into(), is_lying));
                }
                Some(true) => {
                    logically_lying.insert(entity_id.into());
                }
                Some(false) => {}
            }
        }

        for (id, b_added) in transitions {
            if b_added {
                logically_lying.insert(id);
            } else {
                logically_lying.remove(&id);
            }
            self.update_intersecting_corpses_with_snapshot(id, b_added, Some(&logically_lying));
        }
    }

    /// Update `small_repulsive_radius` on `corpse` and its neighbours
    /// in response to a lying↔non-lying transition.
    ///
    /// `b_added = true` — `corpse` just became lying.  If any nearby
    /// lying human's hitbox intersects ours, mark both sides with the
    /// small radius so the repulsive field stops shoving them apart.
    /// Also gates the body against door-blocking via
    /// [`EngineInner::disable_anticollision_iff_blocking_door`].
    ///
    /// `b_added = false` — `corpse` just stood up (or was carried
    /// away).  Clear our own flags and re-evaluate each nearby lying
    /// human that was flagged against us: if they still overlap some
    /// *other* corpse they stay small; otherwise the recursive
    /// `update_intersecting_corpses(_, true)` call on each of them
    /// restores the normal radius.
    #[cfg(test)]
    pub(crate) fn update_intersecting_corpses(&mut self, corpse: EntityId, b_added: bool) {
        self.update_intersecting_corpses_with_snapshot(corpse, b_added, None);
    }

    fn update_intersecting_corpses_with_snapshot(
        &mut self,
        corpse: EntityId,
        b_added: bool,
        logically_lying: Option<&HashSet<EntityId>>,
    ) {
        // Snapshot the corpse's spatial keys; also the sector so we
        // can skip the whole operation inside buildings.
        let Some(entity) = self.get_entity(corpse) else {
            return;
        };
        let corpse_sector = entity.element_data().sector();
        let corpse_layer = entity.element_data().layer();
        let corpse_pos = entity.element_data().position_map();

        if self.sector_is_building(corpse_sector) {
            return;
        }

        if b_added {
            // Adding a corpse.
            self.disable_anticollision_iff_blocking_door(corpse);

            // Clear self first, then re-set to `true` only if we find
            // an intersecting neighbour below.
            if let Some(h) = self.get_entity_mut(corpse).and_then(|e| e.human_data_mut()) {
                h.small_repulsive_radius = false;
            }

            let victims = self.find_intersecting_corpses(
                corpse,
                corpse_sector,
                corpse_layer,
                corpse_pos,
                /* candidate_small_flag */ false,
                logically_lying,
            );

            if !victims.is_empty() {
                if let Some(h) = self.get_entity_mut(corpse).and_then(|e| e.human_data_mut()) {
                    h.small_repulsive_radius = true;
                }
                for id in victims {
                    if let Some(h) = self.get_entity_mut(id).and_then(|e| e.human_data_mut()) {
                        h.small_repulsive_radius = true;
                    }
                }
            }
        } else {
            // Removing a corpse.
            // Original walks marrayActors live here. A recursive `bAdded`
            // call can change the small-radius flag of an actor that the
            // outer walk has not reached yet, and the later actor must be
            // tested against that new flag. Snapshot only the stable actor
            // identities so recursive mutation is borrow-safe, then perform
            // every predicate check against live state.
            let mut human_ids: Vec<EntityId> = self
                .world
                .entities
                .humans()
                .map(|(id, _)| id.into())
                .collect();
            human_ids.sort_by_key(|&id| self.world.original_creation_order(id));

            for id in human_ids {
                if self.is_intersecting_corpse_candidate(
                    id,
                    corpse,
                    corpse_sector,
                    corpse_layer,
                    corpse_pos,
                    /* candidate_small_flag */ true,
                    logically_lying,
                ) {
                    self.update_intersecting_corpses_with_snapshot(id, true, logically_lying);
                }
            }

            if let Some(entity) = self.get_entity_mut(corpse) {
                if let Some(h) = entity.human_data_mut() {
                    h.small_repulsive_radius = false;
                }
                if let Some(a) = entity.actor_data_mut() {
                    a.is_ignored_for_anti_collision = false;
                }
            }
        }
    }

    /// Scan for humans that are lying, in the same layer+sector as
    /// `corpse`, within `INTERSECT_SQ_DIST` of `corpse_pos`, and whose
    /// `small_repulsive_radius` flag matches `candidate_small_flag`.
    fn find_intersecting_corpses(
        &self,
        corpse: EntityId,
        corpse_sector: Option<crate::position_interface::SectorHandle>,
        corpse_layer: u16,
        corpse_pos: MapPoint,
        candidate_small_flag: bool,
        logically_lying: Option<&HashSet<EntityId>>,
    ) -> Vec<EntityId> {
        let mut out = Vec::new();
        for (id, _) in self.world.entities.humans() {
            let candidate_id = EntityId::from(id);
            if self.is_intersecting_corpse_candidate(
                candidate_id,
                corpse,
                corpse_sector,
                corpse_layer,
                corpse_pos,
                candidate_small_flag,
                logically_lying,
            ) {
                out.push(candidate_id);
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn is_intersecting_corpse_candidate(
        &self,
        candidate: EntityId,
        corpse: EntityId,
        corpse_sector: Option<crate::position_interface::SectorHandle>,
        corpse_layer: u16,
        corpse_pos: MapPoint,
        candidate_small_flag: bool,
        logically_lying: Option<&HashSet<EntityId>>,
    ) -> bool {
        if candidate == corpse {
            return false;
        }
        let Some(actor) = self.get_entity(candidate) else {
            return false;
        };
        let Some(human) = actor.human_data() else {
            return false;
        };
        if human.small_repulsive_radius != candidate_small_flag {
            return false;
        }
        let ed = actor.element_data();
        let is_logically_lying = logically_lying
            .map(|lying| lying.contains(&candidate))
            .unwrap_or_else(|| ed.posture.is_lying());
        if !is_logically_lying || ed.layer() != corpse_layer || ed.sector() != corpse_sector {
            return false;
        }
        let dx = ed.position_map().x - corpse_pos.x;
        let dy = ed.position_map().y - corpse_pos.y;
        dx * dx + dy * dy < INTERSECT_SQ_DIST
    }

    /// If `corpse` isn't already flagged
    /// ([`ActorData::is_ignored_for_anti_collision`]) and its body
    /// would block any door, flag it.  Otherwise (when
    /// [`update_intersecting_corpses`](EngineInner::update_intersecting_corpses)
    /// is the caller) leave the flag as-is — the `false` clearing
    /// lives on the corpse-removal branch.
    ///
    /// The door iteration reads from the live `self.script_domains.interactables.doors` table.
    fn disable_anticollision_iff_blocking_door(&mut self, corpse: EntityId) {
        let Some(entity) = self.get_entity(corpse) else {
            return;
        };
        let already_ignored = entity
            .actor_data()
            .map(|a| a.is_ignored_for_anti_collision)
            .unwrap_or(false);
        if already_ignored {
            // Don't override an anticollision that's already disabled
            // for a different reason.
            return;
        }
        let pos = entity.element_data().position_map();
        let Some(body_sector) = entity.element_data().sector() else {
            // No sector info — can't decide.  Bodies are assumed to
            // always have a valid sector.
            return;
        };

        let mut blocks = false;
        if self.scripts.mission.is_some() {
            for door in &self.script_domains.interactables.doors {
                if !door.is_door() {
                    continue;
                }
                let sq_in = {
                    let dx = door.point_in.x - pos.x;
                    let dy = door.point_in.y - pos.y;
                    dx * dx + dy * dy
                };
                let sq_out = {
                    let dx = door.point_out.x - pos.x;
                    let dy = door.point_out.y - pos.y;
                    dx * dx + dy * dy
                };
                if door.body_would_block(
                    crate::sector::SectorNumber::new(u16::from(body_sector) as i16),
                    door.sector_in,
                    door.sector_out,
                    sq_in,
                    sq_out,
                ) {
                    blocks = true;
                    break;
                }
            }
        }

        if let Some(actor) = self.get_entity_mut(corpse).and_then(|e| e.actor_data_mut()) {
            actor.is_ignored_for_anti_collision = blocks;
        }
    }

    /// Sector building-ness via [`FastFindGrid`].  `None`-sector
    /// corpses are treated as "not in a building" — sectorless actors
    /// fall through the test (in practice every actor has a sector).
    pub(crate) fn sector_is_building(
        &self,
        sector: Option<crate::position_interface::SectorHandle>,
    ) -> bool {
        let Some(sn) = sector else { return false };
        self.world
            .fast_grid
            .level
            .sector_number_map
            .get(&crate::sector::SectorNumber::new(u16::from(sn) as i16))
            .and_then(|&idx| self.world.fast_grid.level.sectors.get(idx))
            .map(|gs| gs.sector_type.is_building())
            .unwrap_or(false)
    }

    /// Same shape as [`Self::sector_is_building`], but for lift sectors.
    /// Used by `try_dispatch_move_path` to gate the inner Upright
    /// animation normalisations: lift sectors take a different
    /// action-state path.
    pub(crate) fn sector_is_lift(
        &self,
        sector: Option<crate::position_interface::SectorHandle>,
    ) -> bool {
        let Some(sn) = sector else { return false };
        self.world
            .fast_grid
            .level
            .sector_number_map
            .get(&crate::sector::SectorNumber::new(u16::from(sn) as i16))
            .and_then(|&idx| self.world.fast_grid.level.sectors.get(idx))
            .map(|gs| gs.sector_type.is_lift())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use crate::element::{
        ActorCivilian, ActorData, CivilianData, ElementData, ElementKind, Entity, HumanData,
        NpcData, Posture,
    };
    use crate::engine::EngineInner;

    fn civilian_at(x: f32, y: f32, posture: Posture, sector: u16) -> ActorCivilian {
        let mut element = ElementData {
            kind: ElementKind::ActorCivilian,
            posture,
            ..ElementData::default()
        };
        element.set_position_map(crate::coordinates::MapPoint { x, y });
        element.set_layer(0);
        element.set_sector(crate::position_interface::SectorHandle::new(sector));
        ActorCivilian {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            civilian: CivilianData::default(),
        }
    }

    /// Stand up a default `EngineInner`, add two lying civilians within one
    /// corpse-radius of each other, and invoke the hook as if the
    /// first one just became lying.  Both should end up flagged.
    #[test]
    fn two_intersecting_lying_humans_get_small_radius() {
        let mut engine = EngineInner::new();
        let a = engine.add_entity(Entity::Civilian(civilian_at(
            100.0,
            100.0,
            Posture::Lying,
            1,
        )));
        let b = engine.add_entity(Entity::Civilian(civilian_at(
            110.0,
            100.0,
            Posture::Lying,
            1,
        )));
        engine.update_intersecting_corpses(a, true);

        assert!(
            engine
                .get_entity(a)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );
        assert!(
            engine
                .get_entity(b)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );
    }

    /// Distance > 2 * (R + AR) = 50 → no intersection, flags stay clear.
    #[test]
    fn distant_lying_humans_stay_large_radius() {
        let mut engine = EngineInner::new();
        let a = engine.add_entity(Entity::Civilian(civilian_at(
            100.0,
            100.0,
            Posture::Lying,
            1,
        )));
        let b = engine.add_entity(Entity::Civilian(civilian_at(
            1000.0,
            100.0,
            Posture::Lying,
            1,
        )));
        engine.update_intersecting_corpses(a, true);

        assert!(
            !engine
                .get_entity(a)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );
        assert!(
            !engine
                .get_entity(b)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );
    }

    /// Standing humans are filtered out — only lying neighbours
    /// contribute to the intersection count.
    #[test]
    fn standing_human_is_skipped() {
        let mut engine = EngineInner::new();
        let a = engine.add_entity(Entity::Civilian(civilian_at(
            100.0,
            100.0,
            Posture::Lying,
            1,
        )));
        let b = engine.add_entity(Entity::Civilian(civilian_at(
            110.0,
            100.0,
            Posture::Upright,
            1,
        )));
        engine.update_intersecting_corpses(a, true);

        assert!(
            !engine
                .get_entity(a)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );
        assert!(
            !engine
                .get_entity(b)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );
    }

    /// A lying human in a different sector never counts as intersecting.
    #[test]
    fn different_sector_is_skipped() {
        let mut engine = EngineInner::new();
        let a = engine.add_entity(Entity::Civilian(civilian_at(
            100.0,
            100.0,
            Posture::Lying,
            1,
        )));
        let b = engine.add_entity(Entity::Civilian(civilian_at(
            110.0,
            100.0,
            Posture::Lying,
            2,
        )));
        engine.update_intersecting_corpses(a, true);

        assert!(
            !engine
                .get_entity(a)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );
        assert!(
            !engine
                .get_entity(b)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );
    }

    /// Three corpses in a line pairwise-intersecting.  Removing the
    /// middle one triggers the recursive re-evaluation: the outer two
    /// still intersect each other (Δ = 20 < 50), so their flags stay
    /// set.
    #[test]
    fn removing_corpse_rechecks_neighbours() {
        let mut engine = EngineInner::new();
        let a = engine.add_entity(Entity::Civilian(civilian_at(
            100.0,
            100.0,
            Posture::Lying,
            1,
        )));
        let b = engine.add_entity(Entity::Civilian(civilian_at(
            110.0,
            100.0,
            Posture::Lying,
            1,
        )));
        let c = engine.add_entity(Entity::Civilian(civilian_at(
            120.0,
            100.0,
            Posture::Lying,
            1,
        )));
        engine.update_intersecting_corpses(a, true);
        engine.update_intersecting_corpses(b, true);
        engine.update_intersecting_corpses(c, true);

        engine.update_intersecting_corpses(b, false);

        assert!(
            !engine
                .get_entity(b)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );
        assert!(
            engine
                .get_entity(a)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );
        assert!(
            engine
                .get_entity(c)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );
    }

    /// The Original removal walk tests actors against their current flag,
    /// not a neighbour list captured before recursion. Re-evaluating `a`
    /// makes `c` small, so the still-running outer walk must subsequently
    /// visit `c`, re-evaluate it, and restore its large radius.
    #[test]
    fn removing_corpse_rechecks_later_actor_after_recursive_mutation() {
        let mut engine = EngineInner::new();
        let mut a = civilian_at(1616.4663, 1607.3628, Posture::Tied, 1);
        a.human.small_repulsive_radius = true;
        let mut removed = civilian_at(1599.2805, 1602.0388, Posture::Tied, 1);
        removed.human.small_repulsive_radius = true;
        let mut later = civilian_at(1602.0326, 1623.9482, Posture::Tied, 1);
        later.human.small_repulsive_radius = false;
        let a = engine.add_entity(Entity::Civilian(a));
        let removed = engine.add_entity(Entity::Civilian(removed));
        let later = engine.add_entity(Entity::Civilian(later));

        engine
            .get_entity_mut(removed)
            .unwrap()
            .set_posture(Posture::Upright);
        engine.update_intersecting_corpses(removed, false);

        assert!(
            engine
                .get_entity(a)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );
        assert!(
            !engine
                .get_entity(removed)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );
        assert!(
            !engine
                .get_entity(later)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius,
            "the live outer walk must revisit a later actor changed by recursion"
        );
    }

    /// A lying corpse with a *serialized* `small_repulsive_radius = true`
    /// (simulating a savegame load) must not have that flag cleared by
    /// the very first drain — the tracker is seeded, no transition is
    /// emitted.
    #[test]
    fn process_drain_seeds_without_firing() {
        let mut engine = EngineInner::new();
        let mut civ = civilian_at(100.0, 100.0, Posture::Lying, 1);
        civ.human.small_repulsive_radius = true;
        let a = engine.add_entity(Entity::Civilian(civ));

        engine.process_corpse_intersection_updates();

        assert!(
            engine
                .get_entity(a)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );
        assert_eq!(
            engine
                .get_entity(a)
                .unwrap()
                .human_data()
                .unwrap()
                .last_is_lying_for_corpse_intersection,
            Some(true)
        );
    }

    #[test]
    fn simultaneous_falls_preserve_owner_order_intersection_visibility() {
        let mut engine = EngineInner::new();
        let mut first = civilian_at(100.0, 100.0, Posture::Upright, 1);
        first.human.last_is_lying_for_corpse_intersection = Some(false);
        let mut second = civilian_at(110.0, 100.0, Posture::Upright, 1);
        second.human.last_is_lying_for_corpse_intersection = Some(false);
        let first = engine.add_entity(Entity::Civilian(first));
        let second = engine.add_entity(Entity::Civilian(second));

        engine
            .get_entity_mut(first)
            .unwrap()
            .set_posture(Posture::Dead);
        engine
            .get_entity_mut(second)
            .unwrap()
            .set_posture(Posture::Dead);
        engine.process_corpse_intersection_updates();

        assert!(
            engine
                .get_entity(first)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius,
            "the later fall must discover the already-processed earlier corpse"
        );
        assert!(
            engine
                .get_entity(second)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius,
            "replaying both final postures at once must not clear the later corpse"
        );
    }

    #[test]
    fn owner_boundary_clears_door_ignore_before_later_actor_moves() {
        let mut engine = EngineInner::new();
        let mut standing = civilian_at(100.0, 100.0, Posture::Lying, 1);
        standing.human.last_is_lying_for_corpse_intersection = Some(true);
        standing.actor.is_ignored_for_anti_collision = true;
        let standing = engine.add_entity(Entity::Civilian(standing));

        engine
            .get_entity_mut(standing)
            .unwrap()
            .set_posture(Posture::Upright);
        engine.process_corpse_intersection_update_for(standing);

        let entity = engine.get_entity(standing).unwrap();
        assert_eq!(
            entity
                .human_data()
                .unwrap()
                .last_is_lying_for_corpse_intersection,
            Some(false)
        );
        assert!(
            !entity.actor_data().unwrap().is_ignored_for_anti_collision,
            "a later actor's anti-collision gather must see the stood-up body"
        );
    }
}
