//! Mission-to-v48 topology adapters.
//!
//! Original v48 saves omit the sizes of several mission-created arrays.  The
//! decoder must recover those sizes from the already initialized mission,
//! following the same construction order as the Original.  This module only
//! exposes mappings for facts the Rust engine currently retains exactly.
//! Facts discarded during level loading fail with a named, typed error rather
//! than being reconstructed heuristically.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    engine::{EngineInner, LevelAssets},
    level_data::WaypointCommand,
};

use super::{
    payload_context::LegacyMissionPayloadMetadata,
    post_grid::LegacyGridTopology,
    post_hiking::{LegacyHikingGuideTopology, LegacyHikingPathTopology, LegacyWaypointTopology},
    post_tail::LegacyPostTailTopology,
};

/// A non-self-describing v48 fact which the current Rust mission model does
/// not retain with enough identity/order information to reproduce exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyMissingTopologyFact {
    /// `RHElement::mulCreationOrder` for each live static element.
    ///
    /// Entity slots preserve element order, but not hidden constructor draws
    /// from `gulCreationCounter` (notably each engine-owned mobile master).
    ElementCreationOrders,
    /// Full `RHFastFindGrid::marrayGates`, including byte-less jump gates.
    ///
    /// Rust keeps doors and jump-line geometry in separate collections and
    /// discards their interleaving in the Original gate array.
    GridGateOrder,
    /// Sparse `RHFastFindGrid::marraySectors`, including null holes and the
    /// separately appended out-of-map sector.
    GridSparseSectorOrder,
}

/// Strict failure returned while deriving omitted v48 save topology.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LegacyTopologyAdapterError {
    MissingRetainedFact {
        fact: LegacyMissingTopologyFact,
        original_owner: &'static str,
        detail: &'static str,
    },
    MissionAttachmentMismatch {
        fact: &'static str,
        engine_value: String,
        asset_value: String,
    },
}

impl fmt::Display for LegacyTopologyAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRetainedFact {
                fact,
                original_owner,
                detail,
            } => write!(
                formatter,
                "cannot derive {fact:?}: Original owns it in {original_owner}; {detail}"
            ),
            Self::MissionAttachmentMismatch {
                fact,
                engine_value,
                asset_value,
            } => write!(
                formatter,
                "cannot derive {fact}: initialized engine value {engine_value} does not match attached level asset value {asset_value}"
            ),
        }
    }
}

impl std::error::Error for LegacyTopologyAdapterError {}

/// Derive phase-two element metadata.
///
/// This intentionally fails until Original creation orders are retained
/// explicitly.  `entity_id.index() + 31` is not a general solution: the
/// engine-owned `RHElementMobile` masters consume creation orders without
/// entering `RHEngine::marrayElements`.
pub fn derive_element_payload_metadata(
    _engine: &EngineInner,
    _assets: &LevelAssets,
) -> Result<LegacyMissionPayloadMetadata, LegacyTopologyAdapterError> {
    Err(LegacyTopologyAdapterError::MissingRetainedFact {
        fact: LegacyMissingTopologyFact::ElementCreationOrders,
        original_owner: "RHElement::gulCreationCounter / RHElement::mulCreationOrder",
        detail: "Rust retains entity slots but not every non-array constructor that advances the counter",
    })
}

/// Derive the exact `RHFastFindGrid::Serialize` walk topology.
///
/// Patch order and serializing door order can be recovered, but the requested
/// result represents the *full* arrays. Returning a wire-equivalent topology
/// with jump gates omitted would hide a mission-identity mismatch, so the
/// adapter remains strict.
pub fn derive_grid_topology(
    _engine: &EngineInner,
    _assets: &LevelAssets,
) -> Result<LegacyGridTopology, LegacyTopologyAdapterError> {
    Err(LegacyTopologyAdapterError::MissingRetainedFact {
        fact: LegacyMissingTopologyFact::GridGateOrder,
        original_owner: "RHFastFindGrid::marrayGates",
        detail:
            "Rust separates doors from jump gates and does not retain their shared insertion order",
    })
}

/// Derive `RHHikingGuide::marrayHikingPathes` in its exact stored order.
///
/// Original provenance:
/// `original-code/RHhikingguide.cpp::RHHikingGuide::Serialize` walks paths
/// then waypoints without serializing either count. `RHWaypoint::Serialize`
/// serializes members only when global scripting and `bCommandIsScript` are
/// both true.
pub fn derive_hiking_guide_topology(
    engine: &EngineInner,
    assets: &LevelAssets,
) -> Result<LegacyHikingGuideTopology, LegacyTopologyAdapterError> {
    let script_enabled = engine.scripts.mission.is_some();
    let asset_script_enabled = assets.scripts.mission_name.is_some();
    if script_enabled != asset_script_enabled {
        return Err(LegacyTopologyAdapterError::MissionAttachmentMismatch {
            fact: "global script enablement",
            engine_value: script_enabled.to_string(),
            asset_value: asset_script_enabled.to_string(),
        });
    }

    Ok(map_hiking_paths(&assets.hiking_paths, script_enabled))
}

fn map_hiking_paths(
    paths: &[crate::level_data::RawHikingPath],
    script_enabled: bool,
) -> LegacyHikingGuideTopology {
    LegacyHikingGuideTopology {
        paths: paths
            .iter()
            .map(|path| LegacyHikingPathTopology {
                waypoints: path
                    .waypoints
                    .iter()
                    .map(|waypoint| LegacyWaypointTopology {
                        script_class: match &waypoint.command {
                            WaypointCommand::Script(class) if script_enabled => Some(class.clone()),
                            WaypointCommand::None
                            | WaypointCommand::Macro(_)
                            | WaypointCommand::Script(_) => None,
                        },
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Derive the mission-sized arrays consumed after `RHTitbits::Serialize`.
///
/// The global engine VM is always the SCB `StartUp` class in the Rust port,
/// matching `MissionScript::from_manager`. Seek/archery arrays are read from
/// the initialized AI runtime because construction may merge authored seek
/// directions. Pathfinder counts come from the runtime state matrix that the
/// save bytes restore, and are checked against the attached static graph.
pub fn derive_post_tail_topology(
    engine: &EngineInner,
    assets: &LevelAssets,
    eof_offset: u64,
) -> Result<LegacyPostTailTopology, LegacyTopologyAdapterError> {
    let global_script_class = match (
        engine.scripts.mission.as_ref(),
        assets.scripts.mission_name.as_deref(),
    ) {
        (None, None) => None,
        (Some(script), Some(asset_name)) if script.script_name == asset_name => {
            Some("StartUp".to_owned())
        }
        (engine_script, asset_script) => {
            return Err(LegacyTopologyAdapterError::MissionAttachmentMismatch {
                fact: "mission script identity",
                engine_value: engine_script.map_or_else(
                    || "disabled".to_owned(),
                    |script| script.script_name.clone(),
                ),
                asset_value: asset_script.unwrap_or("disabled").to_owned(),
            });
        }
    };

    let runtime_area_counts: Vec<usize> = engine
        .world
        .pathfinder
        .states
        .iter()
        .map(Vec::len)
        .collect();
    let asset_area_counts: Vec<usize> = assets
        .pathfinder_graph
        .layers
        .iter()
        .map(Vec::len)
        .collect();
    if runtime_area_counts != asset_area_counts {
        return Err(LegacyTopologyAdapterError::MissionAttachmentMismatch {
            fact: "pathfinder layer/area topology",
            engine_value: format!("{runtime_area_counts:?}"),
            asset_value: format!("{asset_area_counts:?}"),
        });
    }

    Ok(LegacyPostTailTopology {
        global_script_class,
        seek_point_count: engine.ai.global.seek_points.len(),
        archery_sector_point_counts: engine
            .ai
            .global
            .archery_sectors
            .iter()
            .map(|sector| sector.points.len())
            .collect(),
        path_graph_area_counts: runtime_area_counts,
        eof_offset,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{
        ai::{PointArchery, Position, SectorArchery, SeekPoint},
        engine::EngineInner,
        level_data::{RawHikingPath, RawWaypoint},
        pathfinder::PathGraph,
        sector::SectorNumber,
    };

    use super::*;

    #[test]
    fn hiking_mapping_preserves_path_waypoint_order_and_script_gate() {
        let engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        assets.hiking_paths = Arc::new(vec![
            RawHikingPath {
                waypoints: vec![
                    waypoint(WaypointCommand::Macro(vec![1, 2])),
                    waypoint(WaypointCommand::Script("PatrolTurn".to_owned())),
                ],
            },
            RawHikingPath {
                waypoints: vec![waypoint(WaypointCommand::None)],
            },
        ]);

        let topology = derive_hiking_guide_topology(&engine, &assets).unwrap();
        assert_eq!(topology.paths.len(), 2);
        assert_eq!(
            topology.paths[0]
                .waypoints
                .iter()
                .map(|waypoint| waypoint.script_class.as_deref())
                .collect::<Vec<_>>(),
            vec![None, None],
        );
        assert_eq!(topology.paths[1].waypoints[0].script_class, None);

        let scripted = map_hiking_paths(&assets.hiking_paths, true);
        assert_eq!(
            scripted.paths[0].waypoints[1].script_class.as_deref(),
            Some("PatrolTurn"),
        );
    }

    #[test]
    fn hiking_rejects_engine_asset_script_enablement_mismatch() {
        let engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        assets.scripts.mission_name = Some("mission".to_owned());

        assert!(matches!(
            derive_hiking_guide_topology(&engine, &assets),
            Err(LegacyTopologyAdapterError::MissionAttachmentMismatch {
                fact: "global script enablement",
                ..
            })
        ));
    }

    #[test]
    fn post_tail_uses_runtime_ai_order_and_checked_pathfinder_shape() {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();

        engine.ai.global.seek_points = (0..3).map(seek_point).collect();
        engine.ai.global.archery_sectors = vec![archery_sector(2), archery_sector(4)];
        engine.world.pathfinder.states = vec![vec![0; 2], vec![0; 1]];

        let mut graph = PathGraph::new();
        graph.layers = vec![vec![Vec::new(); 2], vec![Vec::new(); 1]];
        assets.pathfinder_graph = Arc::new(graph);

        let topology = derive_post_tail_topology(&engine, &assets, 9_999).unwrap();
        assert_eq!(topology.global_script_class, None);
        assert_eq!(topology.seek_point_count, 3);
        assert_eq!(topology.archery_sector_point_counts, vec![2, 4]);
        assert_eq!(topology.path_graph_area_counts, vec![2, 1]);
        assert_eq!(topology.eof_offset, 9_999);
    }

    #[test]
    fn post_tail_rejects_detached_pathfinder_shape() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        engine.world.pathfinder.states = vec![vec![0]];

        assert!(matches!(
            derive_post_tail_topology(&engine, &assets, 0),
            Err(LegacyTopologyAdapterError::MissionAttachmentMismatch {
                fact: "pathfinder layer/area topology",
                ..
            })
        ));
    }

    #[test]
    fn unsupported_exact_topologies_name_the_discarded_fact() {
        let engine = EngineInner::new();
        let assets = LevelAssets::new();
        assert!(matches!(
            derive_element_payload_metadata(&engine, &assets),
            Err(LegacyTopologyAdapterError::MissingRetainedFact {
                fact: LegacyMissingTopologyFact::ElementCreationOrders,
                ..
            })
        ));
        assert!(matches!(
            derive_grid_topology(&engine, &assets),
            Err(LegacyTopologyAdapterError::MissingRetainedFact {
                fact: LegacyMissingTopologyFact::GridGateOrder,
                ..
            })
        ));
    }

    fn waypoint(command: WaypointCommand) -> RawWaypoint {
        RawWaypoint {
            x: 0,
            y: 0,
            sector: 0,
            level: 0,
            command,
        }
    }

    fn archery_sector(point_count: usize) -> SectorArchery {
        SectorArchery {
            points: (0..point_count)
                .map(|_| PointArchery {
                    position: Position::default(),
                    direction: 0,
                    is_shooting_point: false,
                    sector_index: SectorNumber::new(0),
                    owner: None,
                })
                .collect(),
            polygon: vec![(0.0, 0.0)],
            layer: 0,
            index_first_shooting_point: None,
            index_last_shooting_point: None,
            num_shooting_points: 0,
            num_owners: 0,
        }
    }

    fn seek_point(id: u16) -> SeekPoint {
        SeekPoint {
            position: Position::default(),
            frame_when_full_interest: 0,
            directions: Vec::new(),
            last_calculated_interest: 100,
            locked: false,
            id,
        }
    }
}
