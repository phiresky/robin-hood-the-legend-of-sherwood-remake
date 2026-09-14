use super::*;

pub(crate) fn ai_position_to_point_3d(
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
    position: Position,
) -> crate::coordinates::WorldPoint3D {
    let z = match position.sector {
        None => 0.0,
        Some(handle) => {
            let sector_number = crate::sector::SectorNumber::new(handle.get() as i16);
            // Navigation requires an exact sector reference here. Schema-16 NPC
            // session-boundary transients predate exact arena identities,
            // though, and can restore number-only seek positions. Recover
            // that omitted pointer from the position, layer, and authored
            // polygons rather than consulting the lossy public-number map:
            // shipped levels may contain duplicate public sector numbers.
            let grid_idx = handle.arena_index().map(usize::from).or_else(|| {
                    let point = MapPoint::new(position.x, position.y);
                    let candidates = fast_grid
                        .level
                        .sectors
                        .iter()
                        .enumerate()
                        .filter(|(_, sector)| sector.sector_number == sector_number)
                        .collect::<Vec<_>>();
                    let matches = candidates
                        .iter()
                        .copied()
                        .filter(|(_, sector)| {
                            sector.layer == position.level && sector.contains_point(point)
                        })
                        .collect::<Vec<_>>();
                    match matches.as_slice() {
                        [(index, _)] => Some(*index),
                        [] => {
                            let same_layer = candidates
                                .iter()
                                .copied()
                                .filter(|(_, sector)| sector.layer == position.level)
                                .collect::<Vec<_>>();
                            match (same_layer.as_slice(), candidates.as_slice()) {
                                ([(index, _)], _) | (_, [(index, _)]) => Some(*index),
                                ([], []) => None,
                                _ => panic!(
                                    "AI position sector {sector_number} layer {} at ({}, {}) is ambiguous in the exact arena",
                                    position.level, position.x, position.y
                                ),
                            }
                        }
                        _ => panic!(
                            "AI position sector {sector_number} layer {} at ({}, {}) has multiple containing exact sectors",
                            position.level, position.x, position.y
                        ),
                    }
                });
            let grid_sector = grid_idx.and_then(|idx| fast_grid.level.sectors.get(idx));
            let is_motion = grid_sector.is_some_and(|sector| sector.sector_type.is_motion());
            if grid_idx.is_some() && !is_motion {
                panic!(
                    "position_to_point_3d: sector {} is not a motion sector",
                    handle.get()
                );
            }

            // Building motion sectors have no projection area of their
            // own. Position conversion walks the sector's gates
            // in order, finds the first door whose inside point is within
            // maximum norm < 20, and samples the outside sector at the exit point.
            let (projection_sector, projection_layer, point) =
                if grid_sector.is_some_and(|sector| sector.sector_type.is_building()) {
                    let sector = grid_sector.expect("building sector disappeared");
                    let door = sector
                    .gate_indices
                    .iter()
                    .filter_map(|index| {
                        fast_grid
                            .level
                            .door_projection_infos
                            .get(usize::from(*index))
                    })
                    .find(|door| {
                        (door.point_in.x - position.x)
                            .abs()
                            .max((door.point_in.y - position.y).abs())
                            < 20.0
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "position_to_point_3d: building sector {} has no door near ({}, {})",
                            handle.get(),
                            position.x,
                            position.y
                        )
                    });
                    (
                        crate::position_interface::SectorHandle::from_number(door.sector_out)
                            .with_arena_index(door.sector_out_index.unwrap_or_else(|| {
                                panic!("building exit door has no outside sector arena identity")
                            })),
                        door.layer_out,
                        door.point_out,
                    )
                } else {
                    (
                        grid_idx.map_or(handle, |index| {
                            handle.with_arena_index(
                                crate::fast_find_grid::SectorIndex::new(index as u32)
                                    .expect("AI projection sector index exceeds the arena range"),
                            )
                        }),
                        position.level,
                        MapPoint::new(position.x, position.y),
                    )
                };
            let projection_layer = crate::position_interface::Layer::new(projection_layer)
                .expect("AI projection lookup cannot use absent layer");
            let projection = crate::sight_obstacle::ProjectionAreaRef {
                layer: projection_layer,
                sector: projection_sector
                    .arena_index()
                    .unwrap_or_else(|| panic!("AI projection lookup lacks exact sector identity")),
            };
            let mut best: Option<(f32, f32)> = None;
            for (_, obstacle) in obstacles.iter_indexed() {
                if obstacle.projection_area_ref() != Some(projection)
                    || !obstacle.box_projection.contains_point(point)
                    || !obstacle.contains_point_projection(point)
                {
                    continue;
                }
                let z_max = obstacle.box_3d_max[2];
                let z = obstacle.compute_top_z_from_projection(point.x, point.y);
                match best {
                    None => best = Some((z_max, z)),
                    Some((prev_z_max, _)) if z_max > prev_z_max => best = Some((z_max, z)),
                    _ => {}
                }
            }

            best.map(|(_, z)| z).unwrap_or(0.0)
        }
    };

    crate::coordinates::WorldPoint3D {
        x: position.x,
        y: position.y + z,
        z,
    }
}

/// Resolve the entry point used when enemy approach reconsideration's final target
/// position belongs to a lift.
///
/// Original-game lift-sector initialization identifies the high and
/// low doors by minimum/maximum exit-point Y, irrespective of their authored
/// door-type tags. Enemy approach reconsideration then
/// uses the high entry only when its outside layer equals the attacker's
/// current layer; every other layer uses the low entry
/// during enemy-approach reconsideration.
///
/// The outer option distinguishes an ordinary sector from a lift. Stairs are
/// lifts, but deliberately return `Some(None)` because they suppress charging
/// without taking the ladder-entry detour.
pub(crate) fn enemy_lift_approach_for_position(
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    target: Position,
    attacker_layer: Option<u16>,
) -> Option<Option<Position>> {
    let target_sector = target.sector?;
    let sector_number = crate::sector::SectorNumber::new(target_sector.get() as i16);
    let target_has_exact_sector = matches!(
        target_sector.reference(),
        crate::position_interface::SectorReference::Exact { .. }
    );
    // Follow the target's exact sector reference. Shipped levels can
    // contain duplicate public sector numbers, so consult the retained
    // arena identity first and use the legacy number map only for
    // explicitly number-only synthetic/compatibility positions.
    let grid_index = match target_sector.reference() {
        crate::position_interface::SectorReference::NumberOnly(_) => *fast_grid
            .level
            .sector_number_map
            .get(&sector_number)
            .unwrap_or_else(|| {
                panic!("primary target sector {sector_number} is absent from the grid")
            }),
        crate::position_interface::SectorReference::Exact { index, .. } => usize::from(index),
    };
    let sector = fast_grid.level.sectors.get(grid_index).unwrap_or_else(|| {
        panic!("primary target sector {sector_number} maps to missing grid index {grid_index}")
    });
    assert_eq!(
        sector.sector_number, sector_number,
        "primary target exact arena index {grid_index} conflicts with public sector {sector_number}"
    );
    if !sector.sector_type.is_lift() && sector.lift_type.is_none() {
        return None;
    }
    let lift_type = sector.lift_type.unwrap_or_else(|| {
        panic!("lift sector {sector_number} has no lift type during enemy approach")
    });
    if lift_type == crate::sector::LiftType::Stairs {
        return Some(None);
    }

    // A lift sector's own doors have the lift as `sector_in`, hence their
    // outside endpoint differs from the lift sector. Gate indices can also
    // include a door whose outside is this lift; exclude that reverse edge.
    let mut endpoints = sector.gate_indices.iter().filter_map(|index| {
        let door = fast_grid
            .level
            .door_projection_infos
            .get(usize::from(*index))
            .unwrap_or_else(|| {
                panic!(
                    "lift sector {sector_number} references missing door projection {}",
                    index.0
                )
            });
        (door.sector_out != sector_number).then_some((*index, door))
    });
    let first = endpoints.next().unwrap_or_else(|| {
        panic!("non-stairs lift sector {sector_number} has no authored entry doors")
    });
    let (mut high, mut low) = (first, first);
    for endpoint in endpoints {
        if endpoint.1.point_out.y < high.1.point_out.y {
            high = endpoint;
        }
        if endpoint.1.point_out.y > low.1.point_out.y {
            low = endpoint;
        }
    }
    assert!(
        high.0 != low.0,
        "non-stairs lift sector {sector_number} has fewer than two distinct authored entry doors"
    );

    let attacker_layer = attacker_layer.unwrap_or_else(|| {
        panic!("enemy lift approach for sector {sector_number} has no live attacker layer")
    });
    let selected = if high.1.layer_out == attacker_layer {
        high.1
    } else {
        low.1
    };
    let mut selected_sector =
        crate::position_interface::SectorHandle::new(u16::from(selected.sector_out));
    match selected.sector_out_index {
        Some(index) => {
            selected_sector = selected_sector.map(|sector| sector.with_arena_index(index));
        }
        None if target_has_exact_sector => {
            panic!(
                "exact lift sector {sector_number} selected an endpoint without an exact outside sector"
            );
        }
        None => {}
    }
    Some(Some(Position {
        x: selected.point_out.x,
        y: selected.point_out.y,
        sector: selected_sector,
        level: selected.layer_out,
    }))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod hiking_waypoint_identity_tests;

/// Lightweight view of an entity other than the evaluating NPC, used
/// by AI stimulus handlers. All fields come from the live entity at
/// the moment the stimulus is dispatched.
#[derive(Debug, Clone, Default)]
pub struct AntagonistInfo {
    /// The antagonist's map position.
    pub position: Position,
    /// The antagonist's camp.
    pub camp: crate::element::Camp,
    /// True when the antagonist is in a sword-fighting action state.
    pub is_swordfighting: bool,
    /// True when the antagonist is a player character.
    pub is_pc: bool,
    /// True when the antagonist is the Robin Hood PC. Civilian reactions
    /// (`CivAdmiresRobin`) special-case this.
    pub is_robin: bool,
    /// True when the antagonist is a VIP civilian / VIP soldier.
    pub is_vip: bool,
    /// True when the antagonist is inside a building sector.
    pub in_building: bool,
}

// ---------------------------------------------------------------------------
// ReinforcementDoorInfo — cached door data for forest retreats
// ---------------------------------------------------------------------------

/// Cached info for a reinforcement door, used by forest retreats
/// to find the nearest map exit and animate running to its exit point.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ReinforcementDoorInfo {
    /// Inner position of the door (where the NPC walks *to*).
    pub position_in: Position,
    /// Index into the canonical interactable door array.
    pub door_index: crate::gate::DoorIndex,
    /// Outer point of the door (where the NPC exits the map).
    pub point_out: MapPoint,
    /// Mid-point of the door interior. Used by `AlertSoldiers` to
    /// compute the door-out vector for the indoor officer formation
    /// sweep.
    pub point_mid: MapPoint,
    /// Layer index of the outer (outside) end of the door. Used by
    /// indoor formation paths to place gather slots on the outside
    /// layer.
    pub layer_out: u16,
    /// Sector handle of the outer (outside) end of the door.
    pub sector_out: Option<crate::position_interface::SectorHandle>,
    /// Inner door point as raw coordinates. `position_in` already
    /// carries this with layer/sector tagging, but the raw f32 pair is
    /// convenient for the door-vector math.
    pub point_in: MapPoint,
}

// ---------------------------------------------------------------------------
// Global AI state
// ---------------------------------------------------------------------------

/// A building interior known to the AI.
///
/// Populated during AI initialization by collecting every sector whose
/// building classification is true. Houses carry their occupant list so AI
/// code can ask "who's inside?" without scanning all entities, and
/// their door indices so pursuers / investigators can pick the right
/// gate to enter / exit through.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct House {
    /// Sector index (into `FastFindGrid::sectors`) of the building's
    /// interior motion area.
    pub sector_index: u32,
    /// Building index (into canonical `BuildingState`) if this sector is
    /// linked to one. The same index addresses the tenant list. `None` when the
    /// sector isn't proto-linked to a building (e.g. script-synthesised
    /// portals).
    pub building_index: Option<crate::sector::BuildingIdx>,
    /// Doors that connect this building to the outside.  Indices into
    /// the canonical interactable door table.
    pub door_indices: Vec<u32>,
    /// Entities currently inside the building.  Kept live by the
    /// `PassDoor` Enter / Leave hooks in `engine::door_pass`.
    pub occupant_ids: Vec<crate::element::EntityId>,
    /// Whether this building carries an arrow reserve. Populated from
    /// the GUYS/CAVE tenant chunk.
    pub arrow_reserve: bool,
}

impl House {
    /// Number of actors currently inside the building.
    #[inline]
    pub fn occupant_count(&self) -> usize {
        self.occupant_ids.len()
    }

    /// Match original-game building-sector authorization.
    ///
    /// Original-game prototype initialization sets
    /// maximal occupant count to `0xFFFF`, and the proto loader does not
    /// overwrite it. The occupant count is nevertheless tested live on each
    /// authorization call.
    #[inline]
    pub fn is_authorized(&self) -> bool {
        self.occupant_count() < usize::from(u16::MAX)
    }

    /// Whether the given entity is currently an occupant.
    #[inline]
    pub fn contains_occupant(&self, eid: crate::element::EntityId) -> bool {
        self.occupant_ids.contains(&eid)
    }
}

// ─── On the actor-handle vs EntityId dual ─────────────────────────
//
// Building occupancy is tracked in two parallel data structures:
//
//   * `ai::House::occupant_ids: Vec<EntityId>` — the AI-facing view.
//     Populated at `EngineInner::initialize_buildings` and maintained
//     live by the `execute_pass_door` Enter / Leave hooks.  New AI
//     code should query this.
//
//   * `ScriptDomains::buildings` — the script-facing view, indexed by
//     `building_index` with actor
//     script handles. Kept in sync by the same hooks so script
//     natives (`GetNumberOfOccupants`, `GetOccupant`, etc.) see the
//     same occupancy that AI code does.
//
// Both are kept consistent; the dual exists because script identity
// (`i32` handle) and AI identity (`EntityId`) co-exist across the
// codebase and neither can be dropped independently.  Long-term
// consolidation would either migrate script natives to `EntityId`
// or delete `building_occupants` once all natives query via a
// a canonical `occupants_of(building_index) -> &[i32]` helper that
// derives from the House list on demand.
//

/// A rally point positioned just outside a building door.
///
/// Where NPCs regroup after exiting a building before resuming patrol.
/// Built during AI initialization at a fixed `AI_DOOR_RALLY_POINT_DISTANCE` from
/// each building door's exit point.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct DoorRallyPoint {
    /// World position (outside the door).
    pub position: Position,
    /// Door index in the canonical interactable door table.
    pub door_index: crate::gate::DoorIndex,
    /// Radius around `position` within which NPCs are "at" the rally
    /// point.
    pub radius: f32,
}

/// Distance offset (from the exit point) at which door rally points are
/// anchored.
pub const AI_DOOR_RALLY_POINT_DISTANCE: f32 = 100.0;

/// Global / shared AI state, conceptually module-static.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    smart_default::SmartDefault,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct AiGlobalState {
    pub green_alert_soldiers: u16,
    pub yellow_alert_soldiers: u16,
    pub red_alert_soldiers: u16,

    /// Valid soldier allegiances sampled during mission AI initialization.
    /// Like Original's camp-presence flags, this is not recomputed after
    /// deaths or camp changes. The set also supports multi-team hostility.
    pub soldier_camps: std::collections::BTreeSet<crate::element_kinds::Camp>,

    pub stupid_soldiers_cheat: bool,
    pub freeze: bool,

    pub overall_alert_status: AlertLevel,
    pub overall_villain_alert_status: AlertLevel,

    /// Ambush points in the current mission.
    pub ambush_points: Vec<AmbushPoint>,
    /// Seek points shared between all NPCs.
    pub seek_points: Vec<SeekPoint>,
    /// Archery sectors in the current mission.
    pub archery_sectors: Vec<SectorArchery>,

    /// Saved random seed value for deterministic replay.
    pub saved_random_seed: i64,

    /// Per-remark forbidden-until-frame table.
    pub remarks_forbidden_till_frame: Vec<u32>,
    /// Active forbidden remarks.
    pub forbidden_remarks: Vec<ForbiddenRemark>,
    /// Screen remarks to display.
    pub screen_remarks: Vec<ScreenRemark>,

    // Display toggles (not serialized, debug only)
    pub attribute_display: bool,
    pub speech_display: bool,
    pub golden_eye_mode: bool,
    /// `DIES IRAE` cheat — "thunder of God" toggle. While active the
    /// vengeance path (Ezekiel 25:17 reference) is applied, killing
    /// targets chosen by the selected-view-element overlay. Consumers
    /// that still need porting should read this flag on
    /// `AiGlobalState`.
    pub ezekiel_2517: bool,

    pub current_speech_variant: u16,

    /// Repulsive points: NPCs avoid these areas during pathfinding.
    /// Scripts add/remove them by integer ID.
    pub repulsive_points: Vec<RepulsivePoint>,

    /// Next auto-incrementing ID for repulsive points. IDs start at 1.
    #[default(1)]
    pub next_repulsive_point_id: i32,

    /// Cached door geometry for finding a door an enemy could be behind.
    /// Populated at level load and kept in the save snapshot so any
    /// door-state-dependent authorization cache survives restore.
    pub door_seek_infos: Vec<DoorSeekInfo>,

    /// Reinforcement doors: (inner position, door index, exit point).
    /// Used by forest retreats to find the nearest map exit
    /// and animate the NPC running to the door's exit point.
    /// Populated at level load.
    pub reinforcement_doors: Vec<ReinforcementDoorInfo>,

    /// Buildings the AI knows about — populated during AI initialization from
    /// every sector whose `sector_type.is_building()` is true, with
    /// each house's occupant list and doors filled in.
    pub houses: Vec<House>,

    /// Rally points anchored just outside each building door, created in
    /// AI initialization per house gate.
    pub door_rally_points: Vec<DoorRallyPoint>,

    /// Soldier load-order index → entity-handle (slot) mapping. Scripts
    /// and waypoint commands address NPCs by their soldier register
    /// index (the position in the all-soldiers list at level load), not
    /// by their entity slot. Cloned out of
    /// `LevelAssets::all_soldier_entity_ids` once at level load so the
    /// AI tick can resolve a friend ID without re-borrowing the engine.
    pub all_soldier_handles: std::sync::Arc<Vec<u32>>,

    /// Owner-ordered mirror of human primary-target multiplicity.
    /// Original-game AI resets and increments these 16-bit counters directly
    /// on target humans, so later owners in the same actor pass observe the
    /// exact serial mutation history. Original explicitly does not serialize
    /// this scratch field, so a loaded session starts it empty and then
    /// preserves owner-ordered mutations.
    #[serde(skip)]
    #[state_hash(skip)]
    #[bitcode(skip)]
    pub primary_target_multiplicity_scratch: std::collections::BTreeMap<HumanHandle, u32>,
    #[serde(skip)]
    #[state_hash(skip)]
    #[bitcode(skip)]
    pub primary_target_multiplicity_initialized: bool,
}

impl AiGlobalState {
    pub fn npcs_can_be_enemies(&self, diplomacy: &crate::diplomacy::DiplomacyState) -> bool {
        if !diplomacy.npc_faction_wars() {
            return false;
        }
        self.soldier_camps.iter().enumerate().any(|(index, camp)| {
            self.soldier_camps
                .iter()
                .skip(index + 1)
                .any(|other| diplomacy.is_hostile(*camp, *other))
        })
    }

    pub fn overall_villain_alert(&self) -> AlertLevel {
        if self.red_alert_soldiers > 0 {
            AlertLevel::Red
        } else if self.yellow_alert_soldiers > 0 {
            AlertLevel::Yellow
        } else {
            AlertLevel::Green
        }
    }

    pub fn reset_seek_points(&mut self) {
        self.seek_points.clear();
    }

    pub fn reset_ambush_points(&mut self) {
        self.ambush_points.clear();
    }

    pub fn reset_archery_sectors(&mut self) {
        self.archery_sectors.clear();
    }

    pub fn init_green_yellow_red_alert_soldiers(&mut self) {
        self.green_alert_soldiers = 0;
        self.yellow_alert_soldiers = 0;
        self.red_alert_soldiers = 0;
    }

    /// Add a seek-point direction, either merging it into an existing
    /// nearby seek point or creating a new one.
    pub fn add_seek_point_direction(&mut self, dir: &SeekPointDirection) {
        // Check all existing seek points in reverse order
        for sp in self.seek_points.iter_mut().rev() {
            if sp.add_if_near(dir) {
                return;
            }
        }
        // No nearby point found — create a new one
        let mut new_sp = SeekPoint::from_direction(dir);
        new_sp.id = self.seek_points.len() as u16;
        self.seek_points.push(new_sp);
    }

    /// Snap `pos` onto a nearby seek point, chosen at random from those
    /// within the maximum-axis distance from self to `pos`, scaled by
    /// `distance_factor` and increased by `abs_distance`
    /// (with a +100 penalty for level changes). Returns `true` if a
    /// candidate was found and `pos` was overwritten.
    pub fn set_pos_on_near_seek_point(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        me_pos: Position,
        pos: &mut Position,
        distance_factor: f32,
        abs_distance: u16,
    ) -> bool {
        let base_dx = (me_pos.x - pos.x).abs();
        let base_dy = (me_pos.y - pos.y).abs();
        // The original game narrows the computed floating-point limit to 16 bits before it
        // examines candidates. This truncation is observable for ordinary
        // fractional actor positions, not merely at overflow boundaries.
        let distance_limit = (base_dx.max(base_dy) * distance_factor + abs_distance as f32) as u16;

        let mut candidates: Vec<usize> = Vec::new();
        for (idx, sp) in self.seek_points.iter().enumerate() {
            let dx = (sp.position.x - pos.x).abs();
            let dy = (sp.position.y - pos.y).abs();
            let mut distance = dx.max(dy) as u16;
            if sp.position.level != pos.level {
                distance = distance.wrapping_add(100);
            }
            if distance < distance_limit {
                candidates.push(idx);
            }
        }

        if candidates.is_empty() {
            return false;
        }
        let pick = crate::sim_rng::usize(
            sim,
            crate::sim_rng::RngSite::NearSeekPoint,
            0..candidates.len(),
        );
        *pos = self.seek_points[candidates[pick]].position;
        true
    }

    /// Post-process seek points near building doors: teleport them inside.
    pub fn teleport_seek_points_inside_doors(&mut self) {
        for sp in &mut self.seek_points {
            for door_info in &self.door_seek_infos {
                if door_info.door_type == crate::gate::DoorType::Building {
                    let dx = sp.position.x - door_info.point_out.x;
                    let dy = sp.position.y - door_info.point_out.y;
                    let max_norm = dx.abs().max(dy.abs());
                    if max_norm <= 5.0 {
                        sp.position = door_info.position_in;
                        // First matching door wins: the point has moved
                        // inside, so later doors must not re-teleport it.
                        break;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
