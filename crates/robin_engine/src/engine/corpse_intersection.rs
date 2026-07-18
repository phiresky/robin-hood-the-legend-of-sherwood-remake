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
//! setter doesn't see `EngineInner`, so we cannot react synchronously
//! to every posture change.  Instead,
//! [`HumanData::last_is_lying_for_corpse_intersection`] records the
//! previously-observed lying state, and every tick
//! [`EngineInner::process_corpse_intersection_updates`] scans all
//! humans, compares `posture.is_lying()` against that record, and
//! fires [`EngineInner::update_intersecting_corpses`] for each
//! transition it finds.  The scan runs once per hourglass, right
//! after the animation phase has had a chance to change postures.
//!
//! The deferred model is slightly looser than a synchronous hook:
//! a posture that toggles lying→upright→lying inside a single tick
//! won't trigger any update (the end-of-tick observation matches the
//! start).  In practice those transient flips don't happen — posture
//! changes are driven by animations that take multiple frames — and
//! the visible symptom (corpse hitboxes) is only sampled during
//! movement, which runs once per tick.

use crate::coordinates::MapPoint;
use crate::element::EntityId;

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
                }
                Some(prev) if prev != is_lying => {
                    human.last_is_lying_for_corpse_intersection = Some(is_lying);
                    transitions.push((entity_id.into(), is_lying));
                }
                _ => {}
            }
        }

        for (id, b_added) in transitions {
            self.update_intersecting_corpses(id, b_added);
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
    pub(crate) fn update_intersecting_corpses(&mut self, corpse: EntityId, b_added: bool) {
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
            let neighbours = self.find_intersecting_corpses(
                corpse,
                corpse_sector,
                corpse_layer,
                corpse_pos,
                /* candidate_small_flag */ true,
            );

            for id in neighbours {
                self.update_intersecting_corpses(id, true);
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
    ) -> Vec<EntityId> {
        let mut out = Vec::new();
        for (id, actor) in self.world.entities.humans() {
            if id == corpse {
                continue;
            }
            let Some(human) = actor.human_data() else {
                continue;
            };
            if human.small_repulsive_radius != candidate_small_flag {
                continue;
            }
            let ed = actor.element_data();
            if !ed.posture.is_lying() {
                continue;
            }
            if ed.layer() != corpse_layer {
                continue;
            }
            if ed.sector() != corpse_sector {
                continue;
            }
            let dx = ed.position_map().x - corpse_pos.x;
            let dy = ed.position_map().y - corpse_pos.y;
            if dx * dx + dy * dy < INTERSECT_SQ_DIST {
                out.push(id.into());
            }
        }
        out
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
        if let Some(_host) = self.mission_script.as_ref().and_then(|s| s.game_host()) {
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
}
