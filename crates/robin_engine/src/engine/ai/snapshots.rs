//! Detection capture and per-observer optical inputs.
//! Tactical decisions query live state; only inputs with an explicit detection
//! capture lifetime are retained here.

use super::*;
use crate::coordinates::{GroundPoint, MapPoint};
use crate::element::{Camp, Entity, EntityId};
use serde::{Deserialize, Serialize};

/// Enemy archer detection is exactly whether a bow is present.
/// A loaded bow remains a bow even when its normal-shot range is zero.
pub(super) fn is_archer_from_bow(bow: Option<&crate::profiles::BowProfile>) -> bool {
    bow.is_some()
}

/// PC inputs retained until the next PC noise-refresh invalidation.
/// Combat fields are read live at each Think, not copied into this capture.
#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct PcDetectionState {
    pub(super) id: EntityId,
    pub(super) position: MapPoint,
    pub(super) layer: u16,
    pub(super) detection_speed_in_forest: u16,
    pub(super) detection_speed_in_city: u16,
    pub(super) is_vip: bool,
    pub(super) is_robin: bool,
    pub(super) unconscious: bool,
    pub(super) carried: bool,
    /// Persistent bounds can intentionally differ from the produced-noise origin.
    pub(super) hear_noise_box: crate::coordinates::MapBBox,
    pub(super) produced_noise: crate::ai::Noise,
    pub(super) is_swordfighting: bool,
}

/// Per-tick read-only snapshot of a human-typed detection target —
/// shared across the `DetectableType::Body / Friend / MissedFriend /
/// Beggar` per-type passes since all four feed `compute_visibility`
/// against a human target.  Captures the per-target gate inputs each
/// kind needs (e.g. `able_to_help` for Friend, dead/unconscious flags
/// for MissedFriend / Beggar, `is_true_or_false_beggar` for the
/// Beggar cleanup-detectables predicate).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct HumanTarget {
    pub(super) position: MapPoint,
    /// Original-game ground position, i.e. stored world-space X/Y. This is
    /// distinct from projected map position whenever ground Z is non-zero.
    pub(super) ground_position: GroundPoint,
    pub(super) sector: Option<crate::position_interface::SectorHandle>,
    pub(super) layer: u16,
    pub(super) eye_z: f32,
    /// Detection point in world space, taken verbatim as the endpoint
    /// of opaque-reachability queries.
    pub(super) detection_point: crate::coordinates::WorldPoint3D,
    /// Exact ground Z for projected-map to world-horizontal conversion.
    pub(super) ground_z: f32,
    pub(super) posture: crate::element::Posture,
    /// 16-sector facing.  Used for the `LeaningOut` arm of
    /// `compute_detection_point`: the detection point projects
    /// `direction × 40` forward.
    pub(super) direction: i16,
    pub(super) action_state: crate::element::ActionState,
    pub(super) building_sector: Option<crate::position_interface::SectorHandle>,
    /// Canonical human death state. MissedFriend and
    /// Beggar reject dead targets before their per-type cadence decision.
    pub(super) dead: bool,
    pub(super) unconscious: bool,
    pub(super) active: bool,
    pub(super) is_pc: bool,
    /// `is_able_to_help`: alive, conscious, not in a few
    /// state-machine arms that mean "busy with current task".
    /// Used to gate the Friend pass.
    pub(super) able_to_help: bool,
    /// True for a civilian whose profile flags it as a beggar OR
    /// a PC currently in `Posture::SimulatingBeggar`.  Used by
    /// the cleanup-detectables Beggar predicate — entries lose
    /// detectability the moment the target stops being a beggar.
    pub(super) is_true_or_false_beggar: bool,
    /// Whether the target is mid-door-pass.  Used by the
    /// same-building visibility short-circuit.
    pub(super) passing_door: bool,
    /// `pc.guard.is_some()`.  Only meaningful for PCs (false for
    /// soldiers / civilians / non-PC entities).  Used by
    /// predetection handling to suppress shadow events for
    /// already-guarded PCs.
    pub(super) guarded: bool,
    /// The projection-obstacle this human is currently standing
    /// on (e.g. a roof, ledge, balcony, or tree platform).
    /// Threaded into the per-target `compute_view_radius` re-call
    /// inside `run_human_detectable_pass` so detection radius
    /// accounts for the target's elevation in night/fog.
    pub(super) obstacle_idx: Option<crate::position_interface::ObstacleHandle>,
}

/// Per-tick read-only snapshot of an object target — anything that may
/// appear in an NPC's `DetectableType::Object` list (coins, ales,
/// money bags, etc.).  Captures the data the object-visibility
/// computation reads.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct ObjectTarget {
    pub(super) position: MapPoint,
    /// Original-game ground position used by the outer detection-refresh box.
    pub(super) ground_position: GroundPoint,
    /// Original-game object point after the detection path raises Z by one.
    pub(super) world_position: crate::coordinates::WorldPoint3D,
    pub(super) layer: u16,
    pub(super) belongs_to_beggar: bool,
    pub(super) active: bool,
}

fn object_detection_world_position(
    position: crate::coordinates::WorldPoint3D,
) -> crate::coordinates::WorldPoint3D {
    crate::coordinates::WorldPoint3D::new(position.x, position.y, position.z + 1.0)
}

/// Inputs retained across NPC detection refreshes until a PC publishes noise.
/// This is not a tactical world projection: each Think builds its live inputs.
#[derive(Debug, Clone, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub(super) struct DetectionFrameState {
    pub(super) pcs: Vec<PcDetectionState>,
    /// Optical target selection uses this captured occupancy; tactical target
    /// selectors use Original's separately ordered 16-bit scratch counters.
    pub(super) detection_target_multiplicity: std::collections::BTreeMap<EntityId, u32>,
    pub(super) unconscious_soldiers: Vec<(EntityId, Camp, bool)>,
}

impl EngineInner {
    /// Preserve the established capture/invalidation cadence while retaining
    /// only data consumed by detection and its queued sleeping-enemy lists.
    pub(super) fn capture_detection_frame_state(
        &mut self,
        assets: &LevelAssets,
    ) -> DetectionFrameState {
        let _detail = super::super::tick::entity_system_detail_guard(
            super::super::tick::EntitySystemDetail::BuildWorldView,
        );
        let pcs = self.capture_pc_detection_state(assets);
        let detection_target_multiplicity = self.tick_enemy_ai_build_primary_target_multiplicity();
        self.refresh_archer_shield_links();
        let unconscious_soldiers = self.tick_enemy_ai_build_unconscious_soldiers();
        DetectionFrameState {
            pcs,
            detection_target_multiplicity,
            unconscious_soldiers,
        }
    }

    /// Capture the PC registry in its insertion order, independently of portrait
    /// sorting. Noise was already produced by the PC's human-update tail.
    pub(super) fn capture_pc_detection_state(&self, assets: &LevelAssets) -> Vec<PcDetectionState> {
        self.world
            .original_pc_registry()
            .iter()
            .map(|&id| {
                let Entity::Pc(pc) = self.expect_entity(id, "PC detection registry") else {
                    panic!("non-PC {id:?} in PC detection registry");
                };
                let character = assets
                    .profile_manager
                    .get_character(pc.pc.profile_index)
                    .unwrap_or_else(|| {
                        panic!(
                            "PC {} requires missing character profile {}",
                            id.index(),
                            u32::from(pc.pc.profile_index)
                        )
                    });
                let mut position = pc.element.position_map();
                // Preserve the captured sleeping-candidate geometry. Optical
                // detection separately reads the body's stored world coordinates.
                if pc.element.posture() == crate::element::Posture::LeaningOut {
                    let (dx, dy) = crate::element::direction_vector_16(pc.element.direction());
                    position.x += 40.0 * dx;
                    position.y += 40.0 * dy;
                }
                PcDetectionState {
                    id,
                    position,
                    layer: pc.element.layer(),
                    detection_speed_in_forest: character.detection_speed_in_forest,
                    detection_speed_in_city: character.detection_speed_in_city,
                    is_vip: character.vip,
                    is_robin: pc.pc.robin,
                    unconscious: pc.human.unconscious,
                    carried: pc.human.carrier.is_some(),
                    hear_noise_box: pc.actor.hear_noise_box,
                    produced_noise: pc.actor.produced_noise.unwrap_or_else(|| {
                        panic!("PC {} has no initialized produced-noise record", id.index())
                    }),
                    is_swordfighting: !pc.human.opponents.is_empty(),
                }
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn ai_pc_snapshot_ids_for_test(&mut self, assets: &LevelAssets) -> Vec<EntityId> {
        self.capture_pc_detection_state(assets)
            .into_iter()
            .map(|snapshot| snapshot.id)
            .collect()
    }

    /// Build the live-derived target multiplicity used by optical detection.
    /// This is deliberately separate from Original's nonserialized human
    /// scratch counters, which start at zero after load and are mutated only
    /// by the source AI routines that reset/increment them.
    pub(super) fn tick_enemy_ai_build_primary_target_multiplicity(
        &self,
    ) -> std::collections::BTreeMap<EntityId, u32> {
        let mut primary_target_multiplicity = std::collections::BTreeMap::new();
        for (_, soldier) in self.world.entities.soldiers() {
            if let Some(ai) = soldier.npc.ai_brain.base()
                && let Some(primary_target) = ai.primary_target
                && ai.current_substate.is_any_swordfight()
                && let Some(target_id) = self.world.entities.id_at_legacy_slot(primary_target.get())
            {
                let count = primary_target_multiplicity
                    .entry(target_id)
                    .or_insert(0_u32);
                *count = u32::from((*count as u16).wrapping_add(1));
            }
        }
        primary_target_multiplicity
    }

    /// Publish reverse archer links at the established detection-capture point.
    /// Read every claimant before writing: inactive reciprocal links refer to
    /// the pre-refresh state, and the last eligible claimant in registry order wins.
    fn refresh_archer_shield_links(&mut self) {
        let mut reverse = std::collections::HashMap::new();
        let mut active_owners = Vec::new();
        for (archer_id, archer) in self.world.entities.soldiers() {
            if archer.human.unconscious {
                continue;
            }
            let ai = archer.npc.ai_brain.enemy().unwrap_or_else(|| {
                panic!("conscious soldier {archer_id:?} has no EnemyAi during shield-link refresh")
            });
            if archer.element.active {
                active_owners.push(EntityId::from(archer_id));
            }
            let Some(shield_handle) = ai.shield_bearer_before_me else {
                continue;
            };
            let shield = self
                .world
                .entities
                .id_at_legacy_slot(shield_handle.get())
                .and_then(|id| self.world.entities.get(id));
            let Some(Entity::Soldier(shield)) = shield.filter(|entity| !entity.is_unconscious())
            else {
                tracing::warn!(
                    archer = EntityId::from(archer_id).index(),
                    shield_bearer = shield_handle.get(),
                    "shield-bearer relationship points outside the conscious soldier registry"
                );
                continue;
            };
            let stored_archer = shield
                .npc
                .ai_brain
                .enemy()
                .expect("conscious shield bearer requires EnemyAi")
                .archer_behind_me;
            let archer_handle = crate::ai::AiEntityHandle::new(EntityId::from(archer_id).index());
            if archer.element.active || stored_archer == Some(archer_handle) {
                reverse.insert(shield_handle, archer_handle);
            } else {
                tracing::warn!(
                    archer = archer_handle.get(),
                    shield_bearer = shield_handle.get(),
                    ?stored_archer,
                    "ignoring stale one-sided inactive archer relationship"
                );
            }
        }
        for id in active_owners {
            self.world
                .entities
                .expect_entity_mut(id, format_args!("shield-link owner"))
                .enemy_ai_mut()
                .expect("active conscious soldier requires EnemyAi")
                .archer_behind_me = reverse.remove(&crate::ai::AiEntityHandle::new(id.index()));
        }
    }

    /// Capture active, unconscious-but-alive soldiers for money-fight scans.
    /// The `knocked_out_in_money_fight` flag rides along
    /// because only the victim scan filters on it; the morale scan
    /// merely classifies with it.
    pub(super) fn tick_enemy_ai_build_unconscious_soldiers(&self) -> Vec<(EntityId, Camp, bool)> {
        let mut unconscious_soldiers: Vec<(EntityId, Camp, bool)> =
            Vec::with_capacity(self.world.entities.soldiers().count());
        for (npc_id, s) in self.world.entities.soldiers() {
            if !s.element.active {
                continue;
            }
            if s.npc.life_points <= 0 {
                continue;
            }
            if !s.human.unconscious {
                continue;
            }
            let knocked_out_in_money_fight = s
                .npc
                .ai_brain
                .base()
                .map(|ai| ai.knocked_out_in_money_fight)
                .unwrap_or(false);
            unconscious_soldiers.push((
                npc_id.into(),
                s.soldier.cached_camp,
                knocked_out_in_money_fight,
            ));
        }
        unconscious_soldiers
    }

    /// Snapshot every potential human + object target referenced by one
    /// NPC's per-type detectable lists at that NPC's creation-order boundary.
    /// The resulting maps let the body / friend / missed-friend / beggar /
    /// object passes run without re-borrowing `self.world.entities` for each lookup.
    ///
    /// Captures the targets the per-type detection-refresh loop
    /// dereferences from each detectable list.  Hashing by
    /// `EntityId` means a non-trivial detectable list size doesn't
    /// blow up to a linear scan per lookup.
    ///
    /// Body / Friend / MissedFriend / Beggar share the `HumanTarget`
    /// shape — all four feed `compute_visibility` against a human
    /// target, so the per-target metadata is identical (position /
    /// posture / unconscious / etc) plus a couple of per-kind
    /// predicate fields (`able_to_help` for Friend,
    /// `is_true_or_false_beggar` for the Beggar cleanup-detectables
    /// predicate).  Object stays in its own snapshot map because it
    /// uses `compute_object_visibility`, which has a different query
    /// shape.
    pub(super) fn tick_enemy_ai_build_human_object_targets_for_npc(
        &self,
        npc_id: EntityId,
    ) -> (
        std::collections::HashMap<EntityId, HumanTarget>,
        std::collections::HashMap<EntityId, ObjectTarget>,
    ) {
        use crate::element::DetectableType;

        let npc = self.world.entities.expect_ai_actor_data(
            npc_id,
            format_args!("creation-ordered NPC before its live detection target snapshot"),
        );
        let mut human_ids: std::collections::HashSet<EntityId> = std::collections::HashSet::new();
        let mut object_ids: std::collections::HashSet<EntityId> = std::collections::HashSet::new();
        for kind in [
            DetectableType::Body,
            DetectableType::Friend,
            DetectableType::MissedFriend,
            DetectableType::Beggar,
        ] {
            for detectable in &npc.detectable_lists[kind as usize] {
                if let Some(id) = detectable.element {
                    human_ids.insert(id);
                }
            }
        }
        for detectable in &npc.detectable_lists[DetectableType::Object as usize] {
            if let Some(id) = detectable.element {
                object_ids.insert(id);
            }
        }

        let mut human_targets: std::collections::HashMap<EntityId, HumanTarget> =
            std::collections::HashMap::with_capacity(human_ids.len());
        for id in human_ids {
            let Some(entity) = self.world.entities.get(id) else {
                continue;
            };
            // The original game's corpse-carrying movement synchronizes the carried human
            // immediately after the carrier's motion processing
            // during the carrying action. The carrier therefore
            // decides whether the body has moved at this actor boundary, but
            // the position bytes still belong to the body: Original copies
            // the carrier's map point, then computes the body's 3-D point on
            // the body's own obstacle. Using the carrier's world point here
            // incorrectly gives a ground-level corpse the carrier's stair
            // elevation.
            let human = entity.human_data().unwrap_or_else(|| {
                panic!("human detectable target {} has no human data", id.index())
            });
            let stored_map = (entity.element_data()).position_map();
            let stored_world = (entity.element_data()).position();
            let position = stored_map;
            let layer = entity.element_data().layer();
            let posture = entity.element_data().posture();
            // These IDs came from a human-only detectable list.
            // A non-human entry is corrupt data, not
            // a waiting/conscious human.
            let actor = entity.actor_data().unwrap_or_else(|| {
                panic!("human detectable target {} has no actor data", id.index())
            });
            let action_state = actor.action_state;
            let dead = entity.is_dead();
            let unconscious = human.unconscious;
            let active = entity.element_data().active;
            let passing_door = actor.active_door_pass.is_some();
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            // HumanTarget's `eye_z` is consumed as the *detection*
            // point Z by `VisibilityQuery::target_eye_z` (the target
            // side of `compute_detection_point`), so use
            // `detection_z_for_posture` not `eye_z_for_posture` —
            // they differ for Lying (+2 vs +5) and Carried (+25 vs
            // default).
            let is_rider = matches!(entity, Entity::Soldier(s) if s.soldier.rider);
            let ground_z = entity.element_data().position().z;
            let ground_position = GroundPoint::from_map_and_z(position, ground_z);
            let eye_z = ground_z + crate::stealth::detection_z_for_posture(posture, is_rider);
            let direction = entity.element_data().direction();
            let detection_point =
                crate::stealth::detection_point_world(stored_world, posture, direction, is_rider);
            let is_pc = matches!(entity, Entity::Pc(_));
            // Only PCs carry a guard; everything else is unguarded
            // by definition.
            let guarded = if let Entity::Pc(p) = entity {
                p.pc.guard.is_some()
            } else {
                false
            };

            // Soldier `is_able_to_help`.
            let able_to_help = if let Entity::Soldier(s) = entity {
                // This HumanTarget is a detection/visibility projection, not
                // AlertSoldiers' camp population. Preserve its independent
                // active visibility gate here.
                let able_to_fight = active && !s.human.unconscious && s.npc.life_points > 0;
                crate::ai_enemy::soldier_is_able_to_help_state(
                    able_to_fight,
                    s.npc.ai_state(),
                    s.npc.ai_substate(),
                )
            } else {
                // Civilians and PCs are never able to help — the
                // predicate is soldier-only.
                false
            };

            // True for a civilian whose profile is a beggar, or any
            // human in `Posture::SimulatingBeggar`.
            let is_true_or_false_beggar = if let Entity::Civilian(c) = entity {
                c.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar
            } else {
                posture == crate::element::Posture::SimulatingBeggar
            };

            let obstacle_idx = entity.element_data().obstacle_index();
            human_targets.insert(
                id,
                HumanTarget {
                    position,
                    ground_position,
                    sector: entity.element_data().sector(),
                    layer,
                    eye_z,
                    detection_point,
                    ground_z,
                    posture,
                    direction,
                    action_state,
                    building_sector,
                    dead,
                    unconscious,
                    active,
                    is_pc,
                    able_to_help,
                    is_true_or_false_beggar,
                    passing_door,
                    guarded,
                    obstacle_idx,
                },
            );
        }

        let mut object_targets: std::collections::HashMap<EntityId, ObjectTarget> =
            std::collections::HashMap::with_capacity(object_ids.len());
        for id in object_ids {
            let Some(entity) = self.world.entities.get(id) else {
                continue;
            };
            let stored_map = (entity.element_data()).position_map();
            let stored_world = (entity.element_data()).position();
            let position = stored_map;
            let world_position = object_detection_world_position(stored_world);
            let ground_position = GroundPoint::new(world_position.x, world_position.y);
            let layer = entity.element_data().layer();
            let active = entity.element_data().active;
            // The original game's detection refresh treats detectable-object entries as
            // an object-element reference before reading beggar ownership.
            let belongs_to_beggar = entity
                .object_data()
                .unwrap_or_else(|| {
                    panic!("object detectable target {} has no object data", id.index())
                })
                .belongs_to_beggar;
            object_targets.insert(
                id,
                ObjectTarget {
                    position,
                    ground_position,
                    world_position,
                    layer,
                    belongs_to_beggar,
                    active,
                },
            );
        }

        (human_targets, object_targets)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_archer_from_bow, object_detection_world_position};

    #[test]
    fn shield_link_refresh_preserves_claim_order_and_inactive_reciprocity() {
        use crate::ai::AiEntityHandle;
        use crate::element::Camp;
        use crate::engine::test_support::actors::make_test_ai_soldier;

        // (first active, last active, last unconscious, shield active,
        //  shield unconscious, stored claimant, expected claimant).
        let cases = [
            (
                "last active claimant wins",
                (true, true, false, true, false, None, Some(1)),
            ),
            (
                "reciprocal inactive claimant wins",
                (true, false, false, true, false, Some(1), Some(1)),
            ),
            (
                "stale inactive claimant cannot displace first",
                (true, false, false, true, false, Some(0), Some(0)),
            ),
            (
                "inactive one-sided links are cleared",
                (false, false, false, true, false, None, None),
            ),
            (
                "inactive shield is not overwritten",
                (true, true, false, false, false, Some(0), Some(0)),
            ),
            (
                "unconscious claimant is excluded",
                (true, true, true, true, false, Some(1), Some(0)),
            ),
            (
                "unconscious shield is not overwritten",
                (true, true, false, true, true, Some(0), Some(0)),
            ),
        ];
        for (
            name,
            (
                first_active,
                last_active,
                last_unconscious,
                shield_active,
                shield_unconscious,
                stored,
                expected,
            ),
        ) in cases
        {
            let mut engine = crate::engine::EngineInner::new();
            let shield = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
            let first = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
            let last = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
            let handles = [
                AiEntityHandle::new(first.index()),
                AiEntityHandle::new(last.index()),
            ];
            for (id, active, unconscious) in [
                (shield, shield_active, shield_unconscious),
                (first, first_active, false),
                (last, last_active, last_unconscious),
            ] {
                let entity = engine.get_entity_mut(id).unwrap();
                entity.element_data_mut().active = active;
                entity.human_data_mut().unwrap().unconscious = unconscious;
            }
            for id in [first, last] {
                engine
                    .get_entity_mut(id)
                    .unwrap()
                    .enemy_ai_mut()
                    .unwrap()
                    .shield_bearer_before_me = Some(AiEntityHandle::new(shield.index()));
            }
            engine
                .get_entity_mut(shield)
                .unwrap()
                .enemy_ai_mut()
                .unwrap()
                .archer_behind_me = stored.map(|index: usize| handles[index]);

            engine.refresh_archer_shield_links();

            assert_eq!(
                engine
                    .get_entity(shield)
                    .unwrap()
                    .enemy_ai()
                    .unwrap()
                    .archer_behind_me,
                expected.map(|index| handles[index]),
                "{name}"
            );
            assert_eq!(
                engine
                    .get_entity(first)
                    .unwrap()
                    .enemy_ai()
                    .unwrap()
                    .shield_bearer_before_me,
                Some(AiEntityHandle::new(shield.index())),
                "refresh must preserve forward links: {name}"
            );
        }
    }

    #[test]
    fn owner_boundary_ai_position_recovers_duplicate_public_sector_identity() {
        use crate::coordinates::{MapBBox, MapPoint};
        use crate::fast_find_grid::{GridSector, SectorIndex};
        use crate::sector::{SectorNumber, SectorType};

        let grid_sector = |min, max| GridSector {
            points: vec![
                MapPoint::new(min, min),
                MapPoint::new(max, min),
                MapPoint::new(max, max),
                MapPoint::new(min, max),
            ],
            bounding_box: MapBBox::from_coords(min, min, max, max),
            sector_type: SectorType::MOTION | SectorType::AREA,
            layer: 2,
            sector_number: SectorNumber::new(88),
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
        };

        let mut engine = crate::engine::EngineInner::new();
        engine.world.fast_grid_mut().size_map(8, 8);
        engine.world.fast_grid_mut().allocate_layers(3);
        let wrong = engine
            .world
            .fast_grid_mut()
            .add_sector(grid_sector(300.0, 350.0), 2);
        let exact = engine
            .world
            .fast_grid_mut()
            .add_sector(grid_sector(100.0, 200.0), 2);
        assert_ne!(wrong, exact);

        let target = engine.add_test_entity(crate::element::Entity::Pc(crate::element::ActorPc {
            element: {
                let mut initial_element = crate::element::ElementData::from_initial_posture(
                    crate::element::Posture::Upright,
                );
                initial_element.kind = crate::element::ElementKind::ActorPc;
                initial_element
            },
            actor: Default::default(),
            human: Default::default(),
            pc: Default::default(),
        }));
        let element = engine
            .get_entity_mut(target)
            .expect("test PC exists")
            .element_data_mut();
        element.set_position_map(MapPoint::new(150.0, 150.0));
        element.set_layer(2);
        element.set_sector(crate::position_interface::SectorHandle::new(88));

        let position = super::super::build_entity_views_without_forecast(&engine)
            .get(&target.index())
            .expect("target requires live AI view")
            .position;
        assert_eq!(
            position.sector.and_then(|sector| sector.arena_index()),
            SectorIndex::new(exact)
        );
    }

    #[test]
    fn bow_presence_defines_archer_even_with_zero_normal_range() {
        let bow = crate::profiles::BowProfile::default();
        assert_eq!(bow.normal_shoot.range, 0);
        assert!(is_archer_from_bow(Some(&bow)));
        assert!(!is_archer_from_bow(None));
    }

    #[test]
    fn object_detection_raises_the_ray_above_the_stored_position() {
        assert_eq!(
            object_detection_world_position(crate::coordinates::WorldPoint3D::new(10.0, 25.0, 7.0)),
            crate::coordinates::WorldPoint3D::new(10.0, 25.0, 8.0)
        );
    }
}

impl EngineInner {
    pub(super) fn soldier_profile_facts<'a>(
        &self,
        assets: &'a LevelAssets,
        s: &crate::element::ActorSoldier,
        id: EntityId,
    ) -> (
        &'a crate::profiles::SoldierProfile,
        u16,
        Option<&'a crate::profiles::BowProfile>,
    ) {
        let soldier_profile = assets
            .profile_manager
            .get_soldier(s.soldier.soldier_profile_index)
            .unwrap_or_else(|| {
                panic!(
                    "soldier {} requires missing soldier profile {}",
                    id.index(),
                    u32::from(s.soldier.soldier_profile_index)
                )
            });
        let fighting_ability = {
            let base = soldier_profile.fighting;
            if self.is_hostile_to_player_camp(s.soldier.cached_camp) {
                let diff = self.control.sim_config.difficulty;
                diff.rules().enemy_fighting(base, 100)
            } else {
                base
            }
        };
        // Enemy archer detection is exactly
        // the actor having a bow. Weapon initialization creates the
        // bow whenever the one-based shooting-weapon id is non-zero;
        // the bow profile's ranges do not participate in identity.
        let bow_profile = if soldier_profile.shooting_weapon_id == 0 {
            None
        } else {
            Some(
                assets
                    .profile_manager
                    .get_bow(soldier_profile.shooting_weapon_id)
                    .unwrap_or_else(|| {
                        panic!(
                            "soldier {} requires missing bow profile {}",
                            id.index(),
                            soldier_profile.shooting_weapon_id
                        )
                    }),
            )
        };

        (soldier_profile, fighting_ability, bow_profile)
    }
}
