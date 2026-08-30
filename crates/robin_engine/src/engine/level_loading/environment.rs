//! Environment, audio, and motion-grid mission load stages.

use super::*;

impl EngineInner {
    pub(super) fn begin_mission_level_stage(&mut self) {
        self.scripts.globals.clear();
        self.mission_domain.mission_stat.reset();
        self.mission_domain.achievements =
            crate::achievement::MissionAchievementState::from_mission_start();
        self.mission_domain.short_briefings.clear();
    }

    pub(super) fn load_environment_stage(
        &mut self,
        assets: &mut LevelAssets,
        loaded: &mut crate::level_data::LoadedLevel,
        _script_enabled: bool,
    ) {
        // The SIGHT chunk lists which CHUNK_MATERIAL sectors participate
        // in spatial material queries — only those are registered into
        // the fast-grid's per-block SECTOR_SOUND buckets at layer 0.
        // Material sectors present in CHUNK_MATERIAL but absent from
        // this list exist only as per-obstacle `material_indices`
        // references and are invisible to `GetSectors(SECTOR_SOUND)`
        // callers (footstep lookup, projectile water/hole impact
        // detection).  Filter the raw list so `MaterialSectors::material_at`
        // and `WaterZones` see the same subset.
        //
        // Empty `sight_material_indices` means the level has no SIGHT
        // chunk at all (test fixtures, or broken data) — preserve the
        // original pre-filter behaviour of including every material
        // sector rather than silently blanking material lookup.
        let filtered_material_sectors: Vec<crate::level_data::RawMaterialSector> =
            if loaded.proto.sight_material_indices.is_empty() {
                loaded.proto.material_sectors.clone()
            } else {
                loaded
                    .proto
                    .sight_material_indices
                    .iter()
                    .filter_map(|&idx| {
                        loaded
                            .proto
                            .material_sectors
                            .get(idx as usize)
                            .cloned()
                            .or_else(|| {
                                tracing::error!(
                                    "SIGHT chunk references material sector index {idx} but only \
                                 {} material sectors exist — dropping reference",
                                    loaded.proto.material_sectors.len()
                                );
                                None
                            })
                    })
                    .collect()
            };
        tracing::debug!(
            "SIGHT material-list gate: {} / {} material sectors active for spatial lookup",
            filtered_material_sectors.len(),
            loaded.proto.material_sectors.len()
        );

        // Water/hole zones for projectile splash detection. Material
        // WATER/HOLE sectors used by projectile splash detection go
        // through `GetSectors(SECTOR_SOUND)`, so the filter above applies.
        assets.water_zones =
            crate::water_zones::WaterZones::build_from_raw(&filtered_material_sectors);

        // SECTOR_SOUND registry for footstep material lookup.
        // Used by `set_obstacle_and_material` when no projection-area
        // obstacle is available.
        let default_material_code = loaded
            .proto
            .misc
            .as_ref()
            .map(|m| m.default_material)
            .unwrap_or(0);
        let raw_material_default = crate::element::GameMaterial::from_u32(default_material_code);
        assets.material_sectors = crate::material_sectors::MaterialSectors::build_from_raw(
            &filtered_material_sectors,
            default_material_code,
        );
        assets.all_material_sectors = loaded
            .proto
            .material_sectors
            .iter()
            .map(|raw| crate::material_sectors::MaterialSector::from_raw(raw, raw_material_default))
            .collect();

        // Per-layer SECTOR_SOUND grid registry.
        //
        // `RHFastFindGrid::InitializeFromProtoStream` registers the default
        // sight obstacle's material list at layer 0
        // (`RHfastfindgrid.cpp:5378-5387`) and then loads every obstacle,
        // each of which registers its own material list at that obstacle's
        // `muwLayer` (`RHsightobstacle.cpp:461-471`) — `0xFFFF` when the
        // obstacle is not a projection area (`RHsightobstacle.cpp:412-415`).
        // `SetObstacleAndMaterial`'s no-obstacle arm queries the grid at the
        // actor's own layer, so the registrations must keep their layer.
        assets.material_sectors.registrations.clear();
        for raw in &filtered_material_sectors {
            if let Some(sector) =
                crate::material_sectors::MaterialSector::from_raw(raw, raw_material_default)
            {
                assets
                    .material_sectors
                    .register(Some(crate::position_interface::Layer::ZERO), sector);
            }
        }
        for obstacle in &loaded.proto.sight_obstacles {
            let layer = obstacle
                .projection_area
                .and_then(|(_, layer)| crate::position_interface::Layer::new(layer));
            for &index in &obstacle.material_indices {
                let Some(raw) = loaded.proto.material_sectors.get(usize::from(index)) else {
                    tracing::error!(
                        "SightObstacle references material sector {index} but only {} exist — \
                         dropping SECTOR_SOUND registration",
                        loaded.proto.material_sectors.len()
                    );
                    continue;
                };
                if let Some(sector) =
                    crate::material_sectors::MaterialSector::from_raw(raw, raw_material_default)
                {
                    assets.material_sectors.register(layer, sector);
                }
            }
        }

        // Warn when the mission header's control CRC differs from the
        // proto-level misc chunk's CRC — a cheap sanity check that the
        // mission file matches the proto-level it was authored against.
        if let Some(ref misc) = loaded.proto.misc
            && loaded.mission.header.control_crc != misc.control_crc
        {
            tracing::warn!(
                "Proto/mission CRC mismatch: proto misc control_crc=0x{:08X}, \
                 mission header control_crc=0x{:08X} — proto-level and mission \
                 file may be mismatched",
                misc.control_crc,
                loaded.mission.header.control_crc,
            );
        }

        // Apply mission header
        self.initialize_mission_runtime_features(loaded);
        // The runtime initializer installs the initial view-polygon radius:
        // DAY / ATTACK / CUSTOM_1..4 → 400, FOG / NIGHT → 300.  Without
        // this seed, Fog/Night missions whose StartUp script does not
        // call `SetViewRadius(300)` would run with NPCs whose view
        // radius falls back to DEFAULT_VIEW_RADIUS (400) in the AI
        // vision path, detecting PCs from further away than in the
        // original game.  Script opcodes (engine/script.rs
        // `SetViewRadius`) can still overwrite this.
        self.mission_domain.state.map_name = loaded.mission.header.map_filename.clone();
        assets.scripts.hiking_path_count = loaded.mission.hiking_paths.len();

        // Set building count for script handle validation.
        // Only count actual Building entries, not StandaloneDoors.
        assets.scripts.building_count = loaded
            .proto
            .buildings
            .iter()
            .filter(|e| matches!(e, crate::level_data::RawBuildingEntry::Building { .. }))
            .count();

        // Set script location count and extract positions (points + lines + sectors).
        //
        // Script objects are laid out `[points ...] [lines ...] [sectors ...]`
        // and `GetLocationScript` indexes into the combined array directly.
        // Preserve that layout literally — including the empty lines slab — so
        // the index space matches the original.  `lines` is empty on every
        // shipped mission today (the only line-creation site was a dead branch
        // for an old level format); if a future stream version ever
        // re-introduces lines, the index will shift correctly without a code
        // change here.
        if let Some(ref so) = loaded.mission.script_objects {
            assets.scripts.location_count = so.points.len() + so.lines.len() + so.sectors.len();
            assets.scripts.point_count = so.points.len();
            std::sync::Arc::make_mut(&mut assets.scripts.location_positions).clear();
            std::sync::Arc::make_mut(&mut assets.scripts.location_layers).clear();
            std::sync::Arc::make_mut(&mut assets.scripts.location_sectors).clear();
            // Points come first in the combined index.
            for pt in &so.points {
                std::sync::Arc::make_mut(&mut assets.scripts.location_positions)
                    .push((pt.x as f32, pt.y as f32));
                std::sync::Arc::make_mut(&mut assets.scripts.location_layers).push(pt.layer);
                std::sync::Arc::make_mut(&mut assets.scripts.location_sectors).push(pt.sector);
            }
            // Lines slot into the middle of the index space; midpoint is the
            // natural representative position.  Empty in shipped data — see
            // `RawScriptLine` for the dead-branch rationale.
            for line in &so.lines {
                let mx = (line.x1 as f32 + line.x2 as f32) * 0.5;
                let my = (line.y1 as f32 + line.y2 as f32) * 0.5;
                std::sync::Arc::make_mut(&mut assets.scripts.location_positions).push((mx, my));
                std::sync::Arc::make_mut(&mut assets.scripts.location_layers).push(line.layer);
                std::sync::Arc::make_mut(&mut assets.scripts.location_sectors).push(line.sector);
            }
            // Sectors follow; use polygon centroid as their position.
            for sec in &so.sectors {
                let (cx, cy) = if sec.polygon.points.is_empty() {
                    (0.0, 0.0)
                } else {
                    let n = sec.polygon.points.len() as f32;
                    let sum_x: f32 = sec.polygon.points.iter().map(|p| p.0 as f32).sum();
                    let sum_y: f32 = sec.polygon.points.iter().map(|p| p.1 as f32).sum();
                    (sum_x / n, sum_y / n)
                };
                std::sync::Arc::make_mut(&mut assets.scripts.location_positions).push((cx, cy));
                std::sync::Arc::make_mut(&mut assets.scripts.location_layers).push(sec.layer);
                std::sync::Arc::make_mut(&mut assets.scripts.location_sectors).push(sec.sector_ref);
            }

            // Geometry is registered by `register_script_zone_geometry`
            // after the proto motion grid allocates its layers and blocks.
            // Registering here would leave the global sector/line objects
            // alive while `allocate_layers` silently discards all of their
            // spatial indices.
        }

        // Store hiking paths for patrol route lookups by AI.
        assets.scripts.hiking_path_count = loaded.mission.hiking_paths.len();
        assets.hiking_paths = std::sync::Arc::new(std::mem::take(&mut loaded.mission.hiking_paths));

        // Build the global SeekPoint / AmbushPoint / Archery arrays from
        // raw tactic data: reset the existing lists, then fan out to the
        // per-sub-chunk installers.  Reinforcement doors are handled
        // further below, after the MissionLevelBuilder has created
        // the proto-level doors those entries share a table with.
        self.ai.global.reset_seek_points();
        self.ai.global.reset_ambush_points();
        if let Some(ref tactic) = loaded.mission.tactic_data {
            // Install ambush points.
            // `position_3d` and `id` get fixed up later by the AI-init
            // loop at `engine/ai.rs`.
            for raw in &tactic.ambush_points {
                self.ai.global.ambush_points.push(crate::ai::AmbushPoint {
                    position: crate::ai::Position {
                        x: raw.x as f32,
                        y: raw.y as f32,
                        sector: crate::position_interface::SectorHandle::new(raw.sector),
                        level: raw.level,
                    },
                    direction: 0,
                    position_3d: crate::coordinates::WorldPoint3D::default(),
                    id: 0,
                });
            }
            if !tactic.ambush_points.is_empty() {
                tracing::debug!(
                    "Loaded {} ambush points into AiGlobalState",
                    tactic.ambush_points.len(),
                );
            }

            // Wire archery sectors into AiGlobalState.
            // Archery sectors are populated during InitAI from tactic data.
            self.ai.global.reset_archery_sectors();
            for raw in &tactic.archery_sectors {
                // Resolve the referenced sector through `sector_number_map`
                // so we can read its layer.
                let sector_layer = self
                    .world
                    .fast_grid
                    .level
                    .sector_number_map
                    .get(&crate::sector::SectorNumber::new(raw.sector_ref as i16))
                    .and_then(|&idx| self.world.fast_grid.level.sectors.get(idx))
                    .map(|gs| gs.layer)
                    .unwrap_or(0);
                let mut index_first_shooting: Option<crate::sector::ArcheryPointIdx> = None;
                let mut index_last_shooting: Option<crate::sector::ArcheryPointIdx> = None;
                let mut num_shooting: u16 = 0;
                let n_points = raw.points.len();
                let points: Vec<crate::ai::PointArchery> = raw
                    .points
                    .iter()
                    .enumerate()
                    .map(|(i, rp)| {
                        if rp.is_shooting_point {
                            let idx = crate::sector::ArcheryPointIdx(i as u16);
                            if index_first_shooting.is_none_or(|first| idx < first) {
                                index_first_shooting = Some(idx);
                            }
                            if index_last_shooting.is_none_or(|last| idx > last) {
                                index_last_shooting = Some(idx);
                            }
                            num_shooting += 1;
                            // A shooting-point at either end of the way is
                            // a fatal authoring error.  Surface it as a
                            // loud runtime error so malformed ARCH/NLIP
                            // chunks are caught instead of silently
                            // producing a one-sided way.
                            if n_points >= 2 && (i == 0 || i + 1 == n_points) {
                                tracing::error!(
                                    "Archery sector {}: shooting point at way endpoint \
                                     (index {} of {}) — way is malformed",
                                    raw.sector_ref,
                                    i,
                                    n_points,
                                );
                            }
                        }
                        // Each waypoint carries the layer of the motion-area
                        // sector it references, which can differ from the
                        // archery sector's own layer.  Resolve each point's
                        // sector through `sector_number_map` to get the
                        // right layer.
                        let point_layer = self
                            .world
                            .fast_grid
                            .level
                            .sector_number_map
                            .get(&crate::sector::SectorNumber::new(rp.sector as i16))
                            .and_then(|&idx| self.world.fast_grid.level.sectors.get(idx))
                            .map(|gs| gs.layer)
                            .unwrap_or(sector_layer);
                        crate::ai::PointArchery {
                            position: crate::ai::Position {
                                x: rp.x as f32,
                                y: rp.y as f32,
                                sector: crate::position_interface::SectorHandle::new(rp.sector),
                                level: point_layer,
                            },
                            direction: rp.direction,
                            is_shooting_point: rp.is_shooting_point,
                            sector_index: crate::sector::SectorNumber::new(rp.sector as i16),
                            owner: None,
                        }
                    })
                    .collect();
                // An archery way must have at least 3 points.
                if n_points < 3 {
                    tracing::error!(
                        "Archery sector {}: way has only {} points (need >= 3)",
                        raw.sector_ref,
                        n_points,
                    );
                }
                let polygon: Vec<(f32, f32)> = raw
                    .polygon
                    .points
                    .iter()
                    .map(|&(x, y)| (x as f32, y as f32))
                    .collect();
                self.ai
                    .global
                    .archery_sectors
                    .push(crate::ai::SectorArchery {
                        points,
                        polygon,
                        layer: sector_layer,
                        index_first_shooting_point: index_first_shooting,
                        index_last_shooting_point: index_last_shooting,
                        num_shooting_points: num_shooting,
                        num_owners: 0,
                    });
            }
            if !tactic.archery_sectors.is_empty() {
                tracing::debug!(
                    "Loaded {} archery sectors into AiGlobalState",
                    tactic.archery_sectors.len(),
                );
            }
        }

        // Mobile chariots are instantiated after the ordinary script actors,
        // once sprite banks and hiking paths are available.

        // Convert raw sight obstacles into SightObstacle instances for AI
        // line-of-sight checks.
        //
        // Static (load-time) obstacles live in `LevelAssets::static_sight_obstacles`
        // (Arc-shared so per-frame `EngineInner::clone` is a refcount bump).
        // Engine-owned runtime obstacles remain in the separate
        // `EngineInner::dynamic_sight_obstacles` list.
        //
        // Per-obstacle material sub-sectors index into the **unfiltered**
        // CHUNK_MATERIAL list, which holds every CHUNK_MATERIAL entry
        // regardless of SIGHT-list inclusion.  The SIGHT filter only
        // gates the global SECTOR_SOUND fast-find registry (already
        // applied above for `assets.material_sectors`).
        let all_material_sectors = &loaded.proto.material_sectors;
        let static_obstacles: Vec<crate::sight_obstacle::SightObstacle> = loaded
            .proto
            .sight_obstacles
            .iter()
            .enumerate()
            .map(|(idx, raw)| {
                use crate::sight_obstacle::{
                    ObstaclePoint, SIGHTOBSTACLE_MOUSE, SIGHTOBSTACLE_OPAQUE,
                    SIGHTOBSTACLE_PROJECTION_AREA, SIGHTOBSTACLE_SHOW_SHADOW_POLYGON,
                    SIGHTOBSTACLE_SOLID, SightObstacle,
                };
                let mut flags: u32 = 0;
                if raw.opaque {
                    flags |= SIGHTOBSTACLE_OPAQUE;
                }
                if raw.solid {
                    flags |= SIGHTOBSTACLE_SOLID;
                }
                // MOUSE is only set when SOLID.
                if raw.solid && raw.mouse {
                    flags |= SIGHTOBSTACLE_MOUSE;
                }
                if raw.show_shadow_polygon {
                    flags |= SIGHTOBSTACLE_SHOW_SHADOW_POLYGON;
                }
                if raw.projection_area.is_some() {
                    flags |= SIGHTOBSTACLE_PROJECTION_AREA;
                }

                let mut obs = SightObstacle::new(idx as u32, flags);
                obs.obstacle_points = raw
                    .points
                    .iter()
                    .map(|p| ObstaclePoint {
                        x: p.x,
                        y: p.y,
                        z_top: p.z_top,
                        z_bottom: p.z_bottom,
                    })
                    .collect();
                obs.material = raw.default_material;
                // Build per-obstacle material sub-sectors from the
                // unfiltered CHUNK_MATERIAL list.  Drives projectile
                // material determination on heterogeneous obstacle
                // surfaces (e.g. stone inlay on a wooden platform).
                obs.material_sectors = raw
                    .material_indices
                    .iter()
                    .filter_map(|&mi| {
                        let r = all_material_sectors.get(mi as usize).or_else(|| {
                            tracing::error!(
                                "SightObstacle {idx} references material sector {mi} \
                                 but only {} exist — dropping reference",
                                all_material_sectors.len()
                            );
                            None
                        })?;
                        crate::material_sectors::MaterialSector::from_raw(r, raw_material_default)
                    })
                    .collect();
                if let Some((sector, layer)) = raw.projection_area {
                    let layer = crate::position_interface::Layer::new(layer).unwrap_or_else(|| {
                        panic!("projection obstacle {idx} has reserved layer 0xffff")
                    });
                    let sector = crate::fast_find_grid::SectorIndex::new(u32::from(sector))
                        .unwrap_or_else(|| {
                            panic!("projection obstacle {idx} has reserved sector index")
                        });
                    obs.set_projection_area_ref(layer, sector);
                }
                // Copy each referenced material sector from the global
                // material-sector list onto the obstacle.  We resolve
                // indices into clones of the polygon data so subsequent
                // reads (e.g. branches 2 & 3 of `DetermineWaterHole`)
                // don't need to chase a separate global table at the
                // call site.  Indices that fall outside the proto
                // material-sector array are dropped with a warning
                // rather than panicking — we don't want to crash the
                // renderer over a bad asset reference, but the issue
                // should still surface.
                obs.material_sectors =
                    raw.material_indices
                        .iter()
                        .filter_map(|&idx| {
                            let raw_sector = loaded
                                .proto
                                .material_sectors
                                .get(idx as usize)
                                .or_else(|| {
                                    tracing::warn!(
                                        "Sight obstacle {} references material sector {} but \
                                     only {} material sectors exist — dropping reference",
                                        idx,
                                        idx,
                                        loaded.proto.material_sectors.len()
                                    );
                                    None
                                })?;
                            if raw_sector.polygon.points.len() < 3 {
                                return None;
                            }
                            let points: Vec<MapPoint> = raw_sector
                                .polygon
                                .points
                                .iter()
                                .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                                .collect();
                            let mut bbox = crate::coordinates::MapBBox::new();
                            for &p in &points {
                                bbox.expand_point(p);
                            }
                            // Same material-code → GameMaterial mapping
                            // as `MaterialSectors::build_from_raw` (clamp
                            // out-of-range / LIGHT_SHADOW to default).
                            const N_MATERIALS: u32 = 9;
                            let code = raw_sector.material as u32;
                            let material = if code >= N_MATERIALS {
                                crate::element::GameMaterial::from_u32(default_material_code)
                            } else {
                                crate::element::GameMaterial::from_u32(code)
                            };
                            Some(crate::material_sectors::MaterialSector {
                                points,
                                bounding_box: bbox,
                                material,
                            })
                        })
                        .collect();
                // Capture vertices 0/1/2 as (point3, point1, point2) and
                // seed the top/bottom planes from (point1, point2, point3).
                // Orientation flip is skipped because `compute_plane_z` is
                // symmetric in point order.
                if obs.obstacle_points.len() >= 3 {
                    let p0 = &obs.obstacle_points[0];
                    let p1 = &obs.obstacle_points[1];
                    let p2 = &obs.obstacle_points[2];
                    obs.top_plane_points = [
                        [p1.x, p1.y, p1.z_top],
                        [p2.x, p2.y, p2.z_top],
                        [p0.x, p0.y, p0.z_top],
                    ];
                    obs.bottom_plane_points = [
                        [p1.x, p1.y, p1.z_bottom],
                        [p2.x, p2.y, p2.z_bottom],
                        [p0.x, p0.y, p0.z_bottom],
                    ];
                }
                obs.rebuild_geometry();
                obs
            })
            .collect();
        let n = static_obstacles.len();
        self.world.dynamic_sight_obstacles.clear();
        self.world.static_sight_obstacle_active = vec![true; n];
        assets.static_sight_obstacles = std::sync::Arc::new(static_obstacles);
        tracing::info!("Loaded {} sight obstacles for AI line-of-sight", n);
    }

    /// Install tactic seek points after Original's sparse sector topology has
    /// been retained and validated. `load_environment_stage` runs too early:
    /// resolving there can observe stale topology from the previous mission.
    pub(super) fn install_tactic_seek_points_stage(
        &mut self,
        assets: &LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
    ) {
        self.ai.global.reset_seek_points();
        let Some(tactic) = loaded.mission.tactic_data.as_ref() else {
            return;
        };
        for raw in &tactic.seek_points {
            // The tactic stream stores an Original `marraySectors` slot,
            // and `RHSeekPointDirection::InitializeFromMissionStream`
            // resolves it through `GetSector(uwSector)`. Retain that exact
            // arena object rather than interpreting the slot as a displayed
            // sector number.
            let sector = Self::resolve_sparse_position_handle(assets, raw.sector);
            let dir = crate::ai::SeekPointDirection {
                position: crate::ai::Position {
                    x: raw.x as f32,
                    y: raw.y as f32,
                    sector: Some(sector),
                    level: raw.level,
                },
                direction: raw.direction,
            };
            self.ai.global.add_seek_point_direction(&dir);
        }
        tracing::debug!(
            "Loaded {} raw seek-point directions → {} unified seek points",
            tactic.seek_points.len(),
            self.ai.global.seek_points.len(),
        );
    }

    /// Register mission script sectors only after the proto motion pass has
    /// allocated the fast-grid layer and block arrays.
    ///
    /// Original loads its spatial grid before mission script geometry. Rust's
    /// environment metadata pass runs earlier, so doing this work there left
    /// the global sector/line objects alive but lost every per-layer and
    /// per-block index when `FastFindGrid::allocate_layers` replaced those
    /// arrays.
    fn register_script_zone_geometry(
        &mut self,
        assets: &mut LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
    ) {
        std::sync::Arc::make_mut(&mut assets.scripts.zone_grid_indices).clear();
        self.script_domains.zones.scripts.clear();

        let Some(script_objects) = loaded.mission.script_objects.as_ref() else {
            return;
        };
        let script_enabled = self.control.sim_config.script_enabled;
        for sec in &script_objects.sectors {
            // Original nudges the polygon away from integer-aligned actor
            // positions before building the script sector.
            let pts: Vec<MapPoint> = sec
                .polygon
                .points
                .iter()
                .map(|&(x, y)| MapPoint::new(x as f32, y as f32 + 0.000348367))
                .collect();
            let mut bbox = MapBBox::new();
            for &point in &pts {
                bbox.expand_point(point);
            }

            let mut script_data = crate::sector::ScriptSectorData::new();
            script_data.owning_motion_sector =
                crate::sector::SectorNumber::new(sec.sector_ref as i16);
            script_data.script_associated = sec.script_class.is_some() && script_enabled;
            script_data.script_class_name = sec.script_class.clone();

            let mut sector_type = crate::sector::SectorType::SCRIPT;
            if script_data.script_associated {
                sector_type |= crate::sector::SectorType::CROSS;
            }
            if pts.len() < 3 {
                tracing::error!(
                    "Script sector (class {:?}, layer {}) has only {} polygon points — \
                     containment tests will never match (need >= 3)",
                    sec.script_class,
                    sec.layer,
                    pts.len(),
                );
            }

            let grid_idx = self.world.fast_grid_mut().add_sector(
                crate::fast_find_grid::GridSector {
                    points: pts,
                    bounding_box: bbox,
                    sector_type,
                    layer: sec.layer,
                    // Script polygons do not own a motion-sector number. The
                    // SCRP owner lives on ScriptSectorData so this registration
                    // cannot replace the real motion sector in number_map.
                    sector_number: crate::sector::SectorNumber::new(-1),
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
                },
                sec.layer,
            );
            std::sync::Arc::make_mut(&mut assets.scripts.zone_grid_indices).push(grid_idx);
            let zone_idx = self.script_domains.zones.scripts.len();
            self.script_domains.zones.scripts.push(script_data);

            let zone_idx_u16 = u16::try_from(zone_idx).unwrap_or_else(|_| {
                panic!("script zone index {zone_idx} exceeds the u16 Original index domain")
            });
            self.world.fast_grid_mut().add_sector_lines_for_script(
                grid_idx,
                sec.layer,
                zone_idx_u16,
                true,
            );
        }

        if !assets.scripts.zone_grid_indices.is_empty() {
            tracing::info!(
                "Registered {} script zone sectors after motion-grid allocation",
                assets.scripts.zone_grid_indices.len()
            );
        }
    }

    /// Resolve tactic archery topology after motion-sector numbers exist.
    ///
    /// The mission metadata pass preserves the raw archery topology before
    /// motion loading, but cannot resolve a sector number to its authored
    /// layer or exact arena identity at that point. Leaving those temporary
    /// values in place made every elevated archery way behave as layer 0 and
    /// made cross-sector AI routes mix an exact source with a number-only
    /// destination, which can never match in the identity-aware gate graph.
    pub(super) fn resolve_archery_topology_after_motion(
        &mut self,
        loaded: &crate::level_data::LoadedLevel,
    ) -> Result<(), EngineError> {
        let Some(tactic) = loaded.mission.tactic_data.as_ref() else {
            return Ok(());
        };
        if self.ai.global.archery_sectors.len() != tactic.archery_sectors.len() {
            return Err(EngineError::MissionLevelStage {
                stage: "archery tactic layers",
                reason: format!(
                    "runtime archery sector count {} differs from authored count {}",
                    self.ai.global.archery_sectors.len(),
                    tactic.archery_sectors.len()
                ),
            });
        }

        let resolve_topology = |sector_number: u16| -> Result<
            (crate::fast_find_grid::SectorIndex, u16),
            EngineError,
        > {
            let number = crate::sector::SectorNumber::new(sector_number as i16);
            let sector_idx = self
                .world
                .fast_grid
                .level
                .sector_number_map
                .get(&number)
                .copied()
                .ok_or_else(|| EngineError::MissionLevelStage {
                    stage: "archery tactic layers",
                    reason: format!(
                        "authored archery point references missing motion sector {sector_number}"
                    ),
                })?;
            let layer = self
                .world
                .fast_grid
                .level
                .sectors
                .get(sector_idx)
                .map(|sector| sector.layer)
                .ok_or_else(|| EngineError::MissionLevelStage {
                    stage: "archery tactic layers",
                    reason: format!(
                        "motion sector {sector_number} maps to missing grid index {sector_idx}"
                    ),
                })?;
            let sector_idx = crate::fast_find_grid::SectorIndex::new(sector_idx as u32)
                .ok_or_else(|| EngineError::MissionLevelStage {
                    stage: "archery tactic layers",
                    reason: format!(
                        "motion sector {sector_number} maps to invalid grid index {sector_idx}"
                    ),
                })?;
            Ok((sector_idx, layer))
        };

        for (runtime, raw) in self
            .ai
            .global
            .archery_sectors
            .iter_mut()
            .zip(&tactic.archery_sectors)
        {
            if runtime.points.len() != raw.points.len() {
                return Err(EngineError::MissionLevelStage {
                    stage: "archery tactic layers",
                    reason: format!(
                        "archery sector {} runtime point count {} differs from authored count {}",
                        raw.sector_ref,
                        runtime.points.len(),
                        raw.points.len()
                    ),
                });
            }
            runtime.layer = resolve_topology(raw.sector_ref)?.1;
            for (point, raw_point) in runtime.points.iter_mut().zip(&raw.points) {
                let (sector_idx, layer) = resolve_topology(raw_point.sector)?;
                let sector = crate::position_interface::SectorHandle::new(raw_point.sector)
                    .ok_or_else(|| EngineError::MissionLevelStage {
                        stage: "archery tactic layers",
                        reason: format!(
                            "authored archery point has invalid motion sector {}",
                            raw_point.sector
                        ),
                    })?;
                point.position.sector = Some(sector.with_arena_index(sector_idx));
                point.position.level = layer;
            }
        }
        Ok(())
    }

    pub(super) fn load_sound_sources_stage(
        &mut self,
        assets: &mut LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
    ) -> Result<(), EngineError> {
        // ── Sound sources ──
        // Convert raw sound sources from the proto level into the SoundManager's
        // source manager, filtering by the current ambiance bitmask.
        {
            use crate::sound_geometry::SoundSourceAltitude;
            use crate::sound_source::{SoundSource, SoundSourceKind};
            use std::collections::BTreeSet;

            let ambiance_mask = self.world.weather.ambiance.to_bitmask();
            let runtime_switchable = self
                .mission_domain
                .state
                .runtime_features
                .ambience_schedule
                .is_some();
            let mut required_ids = BTreeSet::new();

            for raw in &loaded.proto.sound_sources {
                if !runtime_switchable && raw.ambience_filter & ambiance_mask == 0 {
                    // Preserve Original source-handle alignment for ordinary
                    // fixed-ambience levels without loading unused samples.
                    self.feedback.sound_sim.sources.sources_push_none();
                    continue;
                }
                let source_kind = SoundSourceKind::from_u8(raw.source_kind).ok_or_else(|| {
                    EngineError::MissionLevelStage {
                        stage: "sound sources",
                        reason: format!("source {} has invalid kind {}", raw.id, raw.source_kind),
                    }
                })?;

                let (min_delay, max_delay, delay_stepping) =
                    if let Some((min, max, step)) = raw.delayed_params {
                        (min, max, step + 1) // delay_stepping is pre-incremented
                    } else {
                        (0, 0, 1)
                    };

                let shape: Vec<MapPoint> = raw
                    .polyline
                    .as_ref()
                    .map(|pts| {
                        pts.iter()
                            .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                            .collect()
                    })
                    .unwrap_or_default();

                // Scale volumes from 0–100 range to 0–255
                let inner_volume = raw
                    .inner_volume
                    .map(|v| {
                        let clamped = v.min(100);
                        (clamped as f32 * 2.55) as u16
                    })
                    .unwrap_or(0);
                let outer_volume = raw
                    .outer_volume
                    .map(|v| {
                        let clamped = v.min(100);
                        (clamped as f32 * 2.55) as u16
                    })
                    .unwrap_or(0);

                let altitude = match raw.altitude {
                    0 => SoundSourceAltitude::Ground,
                    1 => SoundSourceAltitude::Middle,
                    2 => SoundSourceAltitude::Top,
                    3 => SoundSourceAltitude::NoAltitude,
                    _ => {
                        return Err(EngineError::MissionLevelStage {
                            stage: "sound sources",
                            reason: format!(
                                "source {} has invalid altitude {}",
                                raw.id, raw.altitude
                            ),
                        });
                    }
                };

                let source = SoundSource {
                    ambiences: raw.ambience_filter,
                    source_kind,
                    id: raw.id as u32,
                    is_global: raw.global,
                    inner_distance: raw.inner_distance.unwrap_or(0),
                    outer_distance: raw.outer_distance.unwrap_or(0),
                    noise_covering_distance: raw.noise_covering_distance.unwrap_or(0),
                    inner_volume,
                    outer_volume,
                    shape,
                    altitude,
                    min_delay,
                    max_delay,
                    delay_stepping,
                    timer: 0,
                    active: raw.active,
                    ambience_enabled: (raw.ambience_filter & ambiance_mask) != 0,
                };

                required_ids.insert(source.id);
                self.feedback.sound_sim.sources.sources_push_some(source);
            }

            tracing::info!(
                "Loaded {} sound sources ({} active, {} required samples)",
                loaded.proto.sound_sources.len(),
                self.feedback.sound_sim.sources.iter_active().count(),
                required_ids.len(),
            );

            // Store on level assets for host-side `setup_mission_audio`
            // to populate the sound-cache source map.
            assets.sound_source_required_ids = required_ids;
        }
        Ok(())
    }

    pub(super) fn load_motion_stage(
        &mut self,
        assets: &mut LevelAssets,
        staging: &mut LevelLoadStaging,
        loaded: &mut crate::level_data::LoadedLevel,
        bg_pixel_dims: (f32, f32),
    ) -> Result<(), EngineError> {
        // Rewire building-door sector_in/layer_in to point at the empty
        // BUILDING grid sectors that `initialize_motion_from_level_data` will
        // create later. This has to run before the MissionLevelBuilder
        // so the `self.script_domains.interactables.doors` list stores the rewritten values.  The matching
        // grid sectors are registered later, in the motion-init pass, using
        // the sector numbers we stash on constructor-local pending data.
        self.rewire_building_doors(
            staging,
            &mut loaded.proto.buildings,
            loaded.proto.motion_data.as_ref(),
        )?;

        // Store motion data for processing when the background bitmap
        // is applied (grid sector registration needs map dimensions).
        // Keep the parsed source descriptor on `LoadedLevel` until
        // `retain_legacy_grid_topology` has reconstructed Original's exact
        // constructor walk. The runtime staging copy is consumed below.
        staging.motion.motion_data = loaded.proto.motion_data.clone();

        // Pre-load *only* the move-box half-diagonal table from the
        // motion-data proto stream, so the soldier / civilian / PC
        // spawn blocks below can size each actor's `move_box` from
        // the real pathfinder profile instead of the `(-1,-1,1,1)`
        // fallback.  The rest of the pathfinder graph (sectors,
        // obstacles, links) still loads later in
        // `initialize_motion_from_level_data` because sector
        // registration needs `map_bbox`, which isn't known until the
        // background bitmap has been decoded.
        // `load_from_proto_stream` detects the already-populated table
        // and skips re-pushing.
        if let Some(ref motion_data) = staging.motion.motion_data
            && !motion_data.graph_bytes.is_empty()
            && let Err(e) = std::sync::Arc::make_mut(&mut assets.pathfinder_graph)
                .preload_half_diagonals_from_proto(
                    self.world.fast_grid_mut(),
                    &motion_data.graph_bytes,
                )
        {
            tracing::error!(
                "Failed to pre-load pathfinder half-diagonals (soldier move_boxes will fall back): {e}"
            );
        }
        // Stash raw masks; converted into RuntimeMask and pushed into the
        // fast grid once layers are allocated in
        // `initialize_motion_from_level_data`.
        staging.motion.masks = std::mem::take(&mut loaded.proto.masks);
        // Stash lift proto data alongside motion data for sector fixup.
        // Clone rather than take — the lift stage still needs them.
        staging.motion.lifts = loaded.proto.lifts.clone();
        // Stash elevation (bond) lines so `initialize_motion_from_level_data`
        // can register them into the fast grid once layers are allocated.
        staging.motion.elevation_lines = std::mem::take(&mut loaded.proto.elevation_lines);
        // Stash jump-zone + jump-line-pair data for post-sector processing
        // in `load_jump_lines_from_proto`.
        // These counts/order also define Original's sparse sector and mixed
        // gate arrays, so do not erase them before topology retention.
        staging.motion.jump_zones = loaded.proto.jump_zones.clone();
        staging.motion.jump_line_pairs = loaded.proto.jump_line_pairs.clone();
        // Stash light/shadow sectors so `initialize_motion_from_level_data`
        // can register them into the grid once layers are allocated and
        // sector numbers have been assigned to the motion / lift / building
        // sectors.
        // Light-sector constructors participate in the same sparse Original
        // sector numbering retained after the motion stage.
        staging.motion.light_sectors = loaded.proto.light_sectors.clone();

        // Load order (ProtoStream → MissionStream): size the grid and
        // register every motion sector / lift / mask / elevation-line
        // BEFORE any mission entity spawns.  The beam-me / soldier /
        // civilian sector validations all assume the fast-grid sector
        // lookup is populated at this point.  Deferring sector
        // registration to after PC spawn would make every beam-me
        // sector check return "no sector".
        self.set_level_size(bg_pixel_dims.0, bg_pixel_dims.1);
        self.build_motion_stage(assets, staging);
        self.register_sound_material_lines(assets, loaded);
        self.register_script_zone_geometry(assets, loaded);
        self.resolve_archery_topology_after_motion(loaded)?;

        // Set forest_level from proto misc — must happen before entity
        // spawning uses it to decide CHARACTER vs CHARACTER_BLIPPED.
        self.world.weather.is_forest_level =
            loaded.proto.misc.as_ref().is_some_and(|m| m.forest_level);

        Ok(())
    }

    /// Register every Original LINE_SOUND edge after the fast grid has
    /// allocated its block/layer arrays.
    ///
    /// The SIGHT chunk first associates a list of material sectors with
    /// the default ground projection at layer 0, then each sight obstacle
    /// associates its own material list with that obstacle's projection
    /// layer. Original registers both sets independently and each line
    /// retains its owning `RHSectorMaterial`.
    fn register_sound_material_lines(
        &mut self,
        assets: &LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
    ) {
        let mut registered_polygons = 0usize;
        let mut register =
            |grid: &mut crate::fast_find_grid::FastFindGrid, raw_index: u16, layer: u16| {
                let Some(sector) = assets
                    .all_material_sectors
                    .get(usize::from(raw_index))
                    .and_then(Option::as_ref)
                else {
                    tracing::error!(
                        "LINE_SOUND references missing or degenerate material sector {raw_index}"
                    );
                    return;
                };
                grid.add_sector_lines_for_sound(layer, &sector.points, raw_index, true);
                registered_polygons += 1;
            };

        for &raw_index in &loaded.proto.sight_material_indices {
            register(self.world.fast_grid_mut(), raw_index, 0);
        }

        for (obstacle_index, obstacle) in loaded.proto.sight_obstacles.iter().enumerate() {
            if obstacle.material_indices.is_empty() {
                continue;
            }
            let Some((_, layer)) = obstacle.projection_area else {
                tracing::error!(
                    "sight obstacle {obstacle_index} has material sectors but no projection layer"
                );
                continue;
            };
            for &raw_index in &obstacle.material_indices {
                register(self.world.fast_grid_mut(), raw_index, layer);
            }
        }

        tracing::debug!(
            registered_polygons,
            "Registered source-associated LINE_SOUND material polygons"
        );
    }
}
