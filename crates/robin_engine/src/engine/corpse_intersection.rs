//! Immediate corpse-intersection updates at human posture transitions.

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
    /// Apply a guarded posture request and publish its spatial effects immediately.
    ///
    /// The human callback compares the previous actual posture with the requested
    /// posture, even when the dead-body guard rejects the underlying write.
    pub(crate) fn set_entity_posture(&mut self, id: EntityId, posture: crate::element::Posture) {
        let entity = self
            .get_entity_mut(id)
            .expect("posture transition owner disappeared");
        let was_lying = entity.posture().is_lying();
        let is_human = entity.is_human();
        entity.set_posture(posture);
        if is_human && was_lying != posture.is_lying() {
            self.update_intersecting_corpses(id, posture.is_lying());
        }
    }

    /// Publish an executing order's logical posture while retaining the sprite
    /// position posture and synchronously updating the human spatial state.
    pub(crate) fn publish_entity_order_posture(
        &mut self,
        id: EntityId,
        posture: crate::element::Posture,
    ) {
        let entity = self
            .get_entity_mut(id)
            .expect("order posture owner disappeared");
        let was_lying = entity.posture().is_lying();
        let is_human = entity.is_human();
        entity.element_data_mut().publish_order_posture(posture);
        if is_human && was_lying != posture.is_lying() {
            self.update_intersecting_corpses(id, posture.is_lying());
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

        // Opt-in trace for the anti-collision repulsive-radius frontier:
        // reports every lying-transition callback with the spatial keys the
        // intersect test uses.  Stderr only, outside serialized state.
        if super::diagnostics::config().corpse_intersection {
            eprintln!(
                "[CORPSE frame={} corpse={corpse:?} added={b_added} sector={corpse_sector:?} building={} layer={corpse_layer} pos={corpse_pos:?}]",
                self.control.frame_counter,
                self.sector_is_building(corpse_sector),
            );
        }

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

            let actor_count = self.world.actor_registry_ids.len();
            for index in 0..actor_count {
                let id = self.world.actor_registry_ids[index];
                if self.is_intersecting_corpse_candidate(
                    id,
                    corpse,
                    corpse_sector,
                    corpse_layer,
                    corpse_pos,
                    /* candidate_small_flag */ false,
                ) {
                    if let Some(h) = self.get_entity_mut(id).and_then(|e| e.human_data_mut()) {
                        h.small_repulsive_radius = true;
                    }
                    if let Some(h) = self.get_entity_mut(corpse).and_then(|e| e.human_data_mut()) {
                        h.small_repulsive_radius = true;
                    }
                }
            }
        } else {
            // Removing a corpse.
            // A recursive addition
            // call can change the small-radius flag of an actor that the
            // outer walk has not reached yet, and the later actor must be
            // tested against that new flag. These updates do not change actor
            // membership, so each short lookup follows the canonical registry.
            let actor_count = self.world.actor_registry_ids.len();
            for index in 0..actor_count {
                let id = self.world.actor_registry_ids[index];
                if self.is_intersecting_corpse_candidate(
                    id,
                    corpse,
                    corpse_sector,
                    corpse_layer,
                    corpse_pos,
                    /* candidate_small_flag */ true,
                ) {
                    self.update_intersecting_corpses(id, true);
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

    fn is_intersecting_corpse_candidate(
        &self,
        candidate: EntityId,
        corpse: EntityId,
        corpse_sector: Option<crate::position_interface::SectorHandle>,
        corpse_layer: u16,
        corpse_pos: MapPoint,
        candidate_small_flag: bool,
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
        if !ed.posture().is_lying() || ed.layer() != corpse_layer || ed.sector() != corpse_sector {
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
        ActorCivilian, ActorData, ActorPc, ActorSoldier, CivilianData, ElementData, ElementKind,
        Entity, HumanData, NpcData, PcData, Posture, SoldierData,
    };
    use crate::engine::EngineInner;

    fn civilian_at(x: f32, y: f32, posture: Posture, sector: u16) -> ActorCivilian {
        let mut element = {
            let mut initial_element = ElementData::from_initial_posture(posture);
            initial_element.kind = ElementKind::ActorCivilian;
            initial_element
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

    #[test]
    fn rejected_dead_idle_posture_request_still_rechecks_intersecting_corpses() {
        let mut engine = EngineInner::new();
        let mut corpse_element = {
            let mut initial_element = ElementData::from_initial_posture(Posture::DeadBack);
            initial_element.kind = ElementKind::ActorSoldier;
            initial_element
        };
        corpse_element.set_position_map(crate::coordinates::MapPoint::new(100.0, 100.0));
        corpse_element.set_sector(crate::position_interface::SectorHandle::new(1));
        let corpse_human = HumanData {
            small_repulsive_radius: true,
            ..Default::default()
        };
        let corpse = engine.add_test_entity(Entity::Soldier(ActorSoldier {
            element: corpse_element,
            actor: ActorData::default(),
            human: corpse_human,
            npc: NpcData::default(),
            soldier: SoldierData {
                cached_camp: crate::element::Camp::Lacklandists,
                ..Default::default()
            },
        }));

        let mut pc_element = {
            let mut initial_element = ElementData::from_initial_posture(Posture::Lying);
            initial_element.kind = ElementKind::ActorPc;
            initial_element
        };
        pc_element.set_position_map(crate::coordinates::MapPoint::new(110.0, 100.0));
        pc_element.set_sector(crate::position_interface::SectorHandle::new(1));
        let pc_human = HumanData {
            unconscious: true,
            small_repulsive_radius: true,
            ..Default::default()
        };
        let pc = engine.add_test_entity(Entity::Pc(ActorPc {
            element: pc_element,
            actor: ActorData::default(),
            human: pc_human,
            pc: PcData::default(),
        }));

        engine.set_entity_posture(corpse, Posture::Upright);
        assert_eq!(
            engine.get_entity(corpse).unwrap().element_data().posture(),
            Posture::DeadBack,
            "the base element rejects the requested upright posture"
        );

        for id in [corpse, pc] {
            assert!(
                !engine
                    .get_entity(id)
                    .unwrap()
                    .human_data()
                    .unwrap()
                    .small_repulsive_radius,
                "the outer removal and recursive re-add leave both bodies at full radius"
            );
        }
        assert_eq!(
            engine.get_entity(corpse).unwrap().element_data().posture(),
            Posture::DeadBack
        );
    }

    /// Stand up a default `EngineInner`, add two lying civilians within one
    /// corpse-radius of each other, and invoke the hook as if the
    /// first one just became lying.  Both should end up flagged.
    #[test]
    fn two_intersecting_lying_humans_get_small_radius() {
        let mut engine = EngineInner::new();
        let a = engine.add_test_entity(Entity::Civilian(civilian_at(
            100.0,
            100.0,
            Posture::Lying,
            1,
        )));
        let b = engine.add_test_entity(Entity::Civilian(civilian_at(
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
        let a = engine.add_test_entity(Entity::Civilian(civilian_at(
            100.0,
            100.0,
            Posture::Lying,
            1,
        )));
        let b = engine.add_test_entity(Entity::Civilian(civilian_at(
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
        let a = engine.add_test_entity(Entity::Civilian(civilian_at(
            100.0,
            100.0,
            Posture::Lying,
            1,
        )));
        let b = engine.add_test_entity(Entity::Civilian(civilian_at(
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
        let a = engine.add_test_entity(Entity::Civilian(civilian_at(
            100.0,
            100.0,
            Posture::Lying,
            1,
        )));
        let b = engine.add_test_entity(Entity::Civilian(civilian_at(
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
        let a = engine.add_test_entity(Entity::Civilian(civilian_at(
            100.0,
            100.0,
            Posture::Lying,
            1,
        )));
        let b = engine.add_test_entity(Entity::Civilian(civilian_at(
            110.0,
            100.0,
            Posture::Lying,
            1,
        )));
        let c = engine.add_test_entity(Entity::Civilian(civilian_at(
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
        let a = engine.add_test_entity(Entity::Civilian(a));
        let removed = engine.add_test_entity(Entity::Civilian(removed));
        let later = engine.add_test_entity(Entity::Civilian(later));

        engine.set_entity_posture(removed, Posture::Upright);

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

    /// Assigning the same lying posture preserves an adopted radius flag.
    #[test]
    fn unchanged_lying_posture_preserves_radius() {
        let mut engine = EngineInner::new();
        let mut civ = civilian_at(100.0, 100.0, Posture::Lying, 1);
        civ.human.small_repulsive_radius = true;
        let a = engine.add_test_entity(Entity::Civilian(civ));

        engine.set_entity_posture(a, Posture::Lying);

        assert!(
            engine
                .get_entity(a)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );
    }

    #[test]
    fn consecutive_falls_publish_intersections_immediately() {
        let mut engine = EngineInner::new();
        let first = civilian_at(100.0, 100.0, Posture::Upright, 1);
        let second = civilian_at(110.0, 100.0, Posture::Upright, 1);
        let first = engine.add_test_entity(Entity::Civilian(first));
        let second = engine.add_test_entity(Entity::Civilian(second));

        engine.set_entity_posture(first, Posture::Dead);
        assert!(
            !engine
                .get_entity(first)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius,
            "the first fall has no lying neighbour yet"
        );
        engine.set_entity_posture(second, Posture::Dead);

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
            "the later fall must immediately publish its own radius"
        );
    }

    #[test]
    fn standing_up_clears_door_ignore_immediately() {
        let mut engine = EngineInner::new();
        let mut standing = civilian_at(100.0, 100.0, Posture::Lying, 1);
        standing.actor.is_ignored_for_anti_collision = true;
        let standing = engine.add_test_entity(Entity::Civilian(standing));

        engine.set_entity_posture(standing, Posture::Upright);

        let entity = engine.get_entity(standing).unwrap();
        assert_eq!(entity.element_data().posture(), Posture::Upright);
        assert!(
            !entity.actor_data().unwrap().is_ignored_for_anti_collision,
            "a later actor's anti-collision gather must see the stood-up body"
        );
    }
}
