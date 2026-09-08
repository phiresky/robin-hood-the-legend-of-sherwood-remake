use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::*;

const PATCH_DATA: &str = include_str!("legendary_enemy_patches.json");

#[derive(Debug, Serialize, Deserialize)]
struct LegendaryPatchManifest {
    version: u32,
    preset: String,
    missions: Vec<LegendaryMissionPatch>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LegendaryMissionPatch {
    mission: String,
    authored_soldier_count: usize,
    selected_proposal_ids: Vec<String>,
    additions: Vec<LegendarySoldierAddition>,
    existing_officer_links: Vec<ExistingOfficerLink>,
    new_patrol_groups: Vec<NewPatrolGroup>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LegendarySoldierAddition {
    source_index: usize,
    position_x: u16,
    position_y: u16,
    layer: u16,
    direction: u32,
    path_behavior: AddedPathBehavior,
    proposal_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AddedPathBehavior {
    Inherit,
    Stationary,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExistingOfficerLink {
    officer_index: usize,
    addition_indices: Vec<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NewPatrolGroup {
    addition_indices: Vec<usize>,
}

fn patch_manifest() -> &'static LegendaryPatchManifest {
    static MANIFEST: OnceLock<LegendaryPatchManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        serde_json::from_str(PATCH_DATA)
            .expect("embedded Legendary enemy patch manifest must be valid JSON")
    })
}

impl EngineInner {
    pub(super) fn apply_legendary_reinforcements(
        &mut self,
        assets: &LevelAssets,
        loaded: &mut crate::level_data::LoadedLevel,
        mission_name: &str,
        config: SimConfig,
    ) -> Result<Vec<crate::level_data::RawSoldier>, EngineError> {
        if config.difficulty != crate::player_profile::DifficultyLevel::Legendary {
            return Ok(Vec::new());
        }
        let manifest = patch_manifest();
        if manifest.version != 1 || manifest.preset != "legendary" {
            return Err(EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!(
                    "unsupported embedded patch manifest version/preset: {}/{}",
                    manifest.version, manifest.preset
                ),
            });
        }
        let Some(patch) = manifest
            .missions
            .iter()
            .find(|patch| patch.mission.eq_ignore_ascii_case(mission_name))
        else {
            tracing::debug!(mission = mission_name, "no reviewed Legendary enemy patch");
            return Ok(Vec::new());
        };
        if loaded.mission.soldiers.len() != patch.authored_soldier_count {
            return Err(EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!(
                    "mission {mission_name} has {} authored soldiers, but reviewed patch expects {}",
                    loaded.mission.soldiers.len(),
                    patch.authored_soldier_count
                ),
            });
        }
        let authored_count = loaded.mission.soldiers.len();
        let final_count = authored_count
            .checked_add(patch.additions.len())
            .ok_or_else(|| EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!("mission {mission_name} soldier count overflow"),
            })?;
        if final_count > usize::from(u16::MAX) {
            return Err(EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!("mission {mission_name} would exceed the u16 soldier-index limit"),
            });
        }

        let authored = loaded.mission.soldiers.clone();
        let mut additions = Vec::with_capacity(patch.additions.len());
        for addition in &patch.additions {
            let mut raw = authored
                .get(addition.source_index)
                .cloned()
                .ok_or_else(|| EngineError::MissionLevelStage {
                    stage: "Legendary enemy reinforcements",
                    reason: format!(
                        "mission {mission_name} proposal {} references missing source soldier {}",
                        addition.proposal_id, addition.source_index
                    ),
                })?;
            let point = MapPoint::new(
                f32::from(addition.position_x),
                f32::from(addition.position_y),
            );
            raw.position_x = addition.position_x;
            raw.position_y = addition.position_y;
            raw.layer = addition.layer;
            raw.direction = addition.direction & 15;
            raw.sector = self.legendary_sparse_sector_for_point(
                assets,
                point,
                addition.layer,
                mission_name,
                &addition.proposal_id,
            )?;
            raw.obstacle_index = u16::MAX;
            raw.money = 0;
            raw.script_class = None;
            raw.subordinate_ids.clear();
            if matches!(addition.path_behavior, AddedPathBehavior::Stationary) {
                raw.path_id = crate::level_data::NO_HIKING_PATH_ID;
                raw.alert_path_id = crate::level_data::NO_HIKING_PATH_ID;
            }
            additions.push(raw);
        }

        let global_index = |local_index: usize| -> Result<u16, EngineError> {
            let index = authored_count.checked_add(local_index).ok_or_else(|| {
                EngineError::MissionLevelStage {
                    stage: "Legendary enemy reinforcements",
                    reason: format!("mission {mission_name} addition index overflow"),
                }
            })?;
            u16::try_from(index).map_err(|_| EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!("mission {mission_name} addition index {index} exceeds u16"),
            })
        };
        for link in &patch.existing_officer_links {
            if link.officer_index >= authored_count {
                return Err(EngineError::MissionLevelStage {
                    stage: "Legendary enemy reinforcements",
                    reason: format!(
                        "mission {mission_name} references missing officer {}",
                        link.officer_index
                    ),
                });
            }
            let subordinate_ids = link
                .addition_indices
                .iter()
                .map(|&index| global_index(index))
                .collect::<Result<Vec<_>, _>>()?;
            loaded.mission.soldiers[link.officer_index]
                .subordinate_ids
                .extend(subordinate_ids);
        }
        for group in &patch.new_patrol_groups {
            let (&commander_local, followers) =
                group.addition_indices.split_first().ok_or_else(|| {
                    EngineError::MissionLevelStage {
                        stage: "Legendary enemy reinforcements",
                        reason: format!(
                            "mission {mission_name} contains an empty added patrol group"
                        ),
                    }
                })?;
            let commander_global = usize::from(global_index(commander_local)?);
            let subordinate_ids = followers
                .iter()
                .map(|&index| global_index(index))
                .collect::<Result<Vec<_>, _>>()?;
            additions[commander_global - authored_count].subordinate_ids = subordinate_ids;
        }

        tracing::info!(
            mission = mission_name,
            additions = patch.additions.len(),
            proposals = ?patch.selected_proposal_ids,
            "applied reviewed Legendary enemy reinforcements"
        );
        Ok(additions)
    }

    fn legendary_sparse_sector_for_point(
        &self,
        assets: &LevelAssets,
        point: MapPoint,
        layer: u16,
        mission_name: &str,
        proposal_id: &str,
    ) -> Result<u16, EngineError> {
        if !self.world.fast_grid.is_inside_grid_point(point) {
            return Err(EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!(
                    "mission {mission_name} proposal {proposal_id} places a soldier outside walkable motion geometry at ({}, {}) layer {layer}",
                    point.x, point.y
                ),
            });
        }
        // `get_sector` deliberately prioritizes doors, lifts, patches and jump
        // sectors for cursor/movement queries. A spawn position instead needs
        // the underlying motion AREA identity that Original serialized on the
        // soldier, even when a special sector overlaps it.
        let block = self.world.fast_grid.get_block_index(point, layer);
        let mut motion_area = None;
        let mut blocked = false;
        for (index, sector) in self
            .world
            .fast_grid
            .get_sectors_at_block(block, crate::sector::SectorType::MOTION)
        {
            if !sector.contains_point(point) {
                continue;
            }
            if sector.sector_type.is_area() {
                motion_area = crate::fast_find_grid::SectorIndex::new(index);
            } else {
                blocked = true;
            }
        }
        let Some(sector_idx) = motion_area.filter(|_| !blocked) else {
            return Err(EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!(
                    "mission {mission_name} proposal {proposal_id} places a soldier outside walkable motion geometry at ({}, {}) layer {layer}",
                    point.x, point.y
                ),
            });
        };
        let topology =
            assets
                .legacy_grid_topology
                .as_ref()
                .ok_or_else(|| EngineError::MissionLevelStage {
                    stage: "Legendary enemy reinforcements",
                    reason: format!(
                        "mission {mission_name} has no retained sparse sector topology"
                    ),
                })?;
        topology
            .position_sector_indices
            .iter()
            .position(|candidate| *candidate == Some(sector_idx))
            .and_then(|slot| u16::try_from(slot).ok())
            .ok_or_else(|| EngineError::MissionLevelStage {
                stage: "Legendary enemy reinforcements",
                reason: format!(
                    "mission {mission_name} proposal {proposal_id} resolved to an unmappable runtime sector"
                ),
            })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn embedded_manifest_has_valid_unique_indices() {
        let manifest = patch_manifest();
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.preset, "legendary");
        let mut missions = BTreeSet::new();
        let mut patrol_groups = 0;
        for patch in &manifest.missions {
            assert!(missions.insert(patch.mission.to_ascii_lowercase()));
            assert!(!patch.additions.is_empty());
            assert_eq!(
                patch
                    .selected_proposal_ids
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len(),
                patch.selected_proposal_ids.len()
            );
            for addition in &patch.additions {
                assert!(addition.source_index < patch.authored_soldier_count);
                assert!(addition.direction < 16);
            }
            for link in &patch.existing_officer_links {
                assert!(link.officer_index < patch.authored_soldier_count);
                assert!(!link.addition_indices.is_empty());
                assert!(
                    link.addition_indices
                        .iter()
                        .all(|&index| index < patch.additions.len())
                );
            }
            for group in &patch.new_patrol_groups {
                patrol_groups += 1;
                assert!(group.addition_indices.len() >= 2);
                assert!(
                    group
                        .addition_indices
                        .iter()
                        .all(|&index| index < patch.additions.len())
                );
            }
        }
        assert!(
            patrol_groups >= 2,
            "reviewed manifest should exercise cloned patrol ownership"
        );
    }
}
