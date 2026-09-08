//! Player-character vision and temporary hostile intelligence.

use super::*;
use crate::coordinates::{GroundBBox, GroundPoint, MapPoint, WorldPoint3D};
use crate::element::{Camp, Entity, Posture};
use crate::fog_of_war::{
    FogCellState, FogOfWarState, LISTEN_REVEAL_FRAMES, SPOTTED_HYSTERESIS_FRAMES,
};
use crate::shadow_polygon::{CHARACTER_HEIGHT, VisibilitySurface, visible_region_on_surfaces};
use geo::orient::{Direction, Orient};
use geo::{
    Area, BooleanOps, ConvexHull, Coord, Intersects, IsConvex, LineString, MultiPoint,
    MultiPolygon, Point, Polygon, TriangulateEarcut,
};
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use std::collections::BTreeMap;

/// Fog view distance relative to the mission's standard vision radius.
/// This is intentionally shorter than the original game's blip-visibility factor (1.5).
const FOG_VIEW_DISTANCE_FACTOR: f32 = 1.2;
// Preserve the Original's additional perched-view multiplier.
const ON_SHOULDERS_FACTOR: f32 = 1.3;
// A small upward allowance keeps the camera-projected upper pixels of a
// directly faced wall clear without turning the sight volume into unlimited
// vertical look-up. The allowance is measured at the far edge, so it becomes
// proportionally smaller on a nearer first-hit facade.
const FOG_VERTICAL_AIM_MARGIN: f32 = 20.0;

#[derive(Clone, Copy)]
struct VisionSource {
    eye: WorldPoint3D,
    map_position: MapPoint,
    plane: Option<crate::position_interface::PlaneZCoeffs>,
    eye_height: f32,
    standard_radius: f32,
    super_factor: f32,
}

impl EngineInner {
    pub fn fog_of_war_enabled(&self) -> bool {
        // A 0x0 level is a legitimate construction/preflight state used
        // before mission assets are installed. Fog becomes gameplay-active
        // only once there is an actual grid to query and composite.
        self.control.sim_config.fog_of_war && self.players.fog_of_war.is_initialized()
    }

    pub fn fog_of_war(&self) -> &FogOfWarState {
        &self.players.fog_of_war
    }

    pub fn fog_cell_state(&self, position: MapPoint) -> FogCellState {
        if !self.fog_of_war_enabled() {
            FogCellState::Visible
        } else {
            self.players.fog_of_war.cell_state(position)
        }
    }

    /// Whether presentation and input may expose an entity at this frame.
    /// Allied humans remain known. Every other entity is exposed when its
    /// current map position is inside the visible region or temporary intelligence reveals it.
    /// This keeps actors and the visibly clear ground beneath them in sync.
    pub fn fog_entity_visible(&self, entity_id: EntityId) -> bool {
        if !self.fog_of_war_enabled() {
            return true;
        }
        let entity = self
            .get_entity(entity_id)
            .unwrap_or_else(|| panic!("fog visibility queried missing entity {entity_id:?}"));
        if self
            .players
            .fog_of_war
            .is_spotted(entity_id, self.control.frame_counter)
        {
            return true;
        }
        let player_camps = self.player_camps();
        if self.is_allied_to_player(entity, &player_camps) {
            return true;
        }
        self.players
            .fog_of_war
            .cell_state(entity.element_data().position_map())
            == FogCellState::Visible
    }

    pub fn fog_entity_is_hostile(&self, entity_id: EntityId) -> bool {
        let entity = self
            .get_entity(entity_id)
            .unwrap_or_else(|| panic!("fog hostility queried missing entity {entity_id:?}"));
        self.is_hostile_to_player(entity, &self.player_camps())
    }

    pub(crate) fn refresh_fog_of_war(&mut self, assets: &LevelAssets, force: bool) {
        if !self.control.sim_config.fog_of_war {
            return;
        }
        let scan_started = web_time::Instant::now();

        let level_size = self.feedback.cutscene_camera.level_size;
        if level_size.x <= 0.0 || level_size.y <= 0.0 {
            tracing::warn!("fog visibility scan skipped before level dimensions were loaded");
            return;
        }

        // Initialise before looking for observers. A mission with no live
        // player-aligned actor must render as unseen, not accidentally fall
        // through the renderer's uninitialised-state guard and expose the
        // whole level.
        let mut fog = std::mem::take(&mut self.players.fog_of_war);
        if !fog.is_initialized() || fog.level_size() != level_size {
            fog.initialize(level_size);
        }
        let previous_visible = fog.begin_scan();

        let player_camps = self.player_camps();
        if player_camps.is_empty() {
            tracing::debug!(
                "fog visibility has no player-camp PC; retaining exploration with no live sight"
            );
            fog.finish_scan(previous_visible);
            fog.retain_entities(|entity_id| self.world.entities.get(entity_id).is_some());
            self.players.fog_of_war = fog;
            return;
        }

        let sources = self.fog_vision_sources();

        let obstacles = self.sight_obstacles(assets);
        let previous_cache = if force {
            crate::fog_of_war::FogScanCache::default()
        } else {
            std::mem::take(&mut fog.scan_cache)
        };
        let compute_source = |source: &VisionSource| {
            let source_started = web_time::Instant::now();
            let ground_radius = fog_horizontal_radius(source.standard_radius, source.super_factor);
            let min_map = MapPoint::new(
                (source.map_position.x - ground_radius).max(0.0),
                (source.map_position.y - ground_radius).max(0.0),
            );
            let max_map = MapPoint::new(
                (source.map_position.x + ground_radius).min(level_size.x),
                (source.map_position.y + ground_radius).min(level_size.y),
            );
            let candidate_obstacles =
                fog_obstacle_candidates(&self.world.fast_grid, source, min_map, max_map);
            let source_key = fog_source_key(source);
            let obstacle_key = fog_obstacle_key(obstacles, &candidate_obstacles);
            if let Some(cached) = previous_cache
                .entries
                .iter()
                .find(|entry| entry.source_key == source_key && entry.obstacle_key == obstacle_key)
            {
                return (
                    cached.visible.to_geo(),
                    cached.visible_projection.to_geo(),
                    source_key,
                    obstacle_key,
                    cached.prepared_surfaces.clone(),
                );
            }
            let prepared_surfaces = previous_cache
                .entries
                .iter()
                .find(|entry| entry.obstacle_key == obstacle_key)
                .map(|entry| entry.prepared_surfaces.clone())
                .unwrap_or_else(|| {
                    std::sync::Arc::new(prepare_camera_surfaces(obstacles, &candidate_obstacles))
                });
            let prepared_us = source_started.elapsed().as_micros();
            let ground_started = web_time::Instant::now();
            let candidate_refs: Vec<_> = candidate_obstacles
                .iter()
                .filter_map(|&index| {
                    let index = usize::from(index);
                    if !obstacles.is_active(index) {
                        return None;
                    }
                    obstacles.get(index).or_else(|| {
                        tracing::warn!(
                            obstacle_index = index,
                            "fog references missing sight obstacle"
                        );
                        None
                    })
                })
                .chain(
                    obstacles
                        .dynamic_obstacles
                        .iter()
                        .filter(|obstacle| obstacle.is_opaque()),
                )
                .collect();
            let map_domains = fog_map_domains(source.map_position, ground_radius, level_size);
            let mut source_visible_polygons = Vec::new();
            for map_domain in &map_domains {
                let ground_domain = map_polygon_to_ground(map_domain, source.plane);
                let visible_ground = visible_region_on_surfaces(
                    &ground_domain,
                    [source.eye.x, source.eye.y, source.eye.z],
                    &[
                        VisibilitySurface {
                            plane: source.plane,
                            height: 0.0,
                        },
                        VisibilitySurface {
                            plane: source.plane,
                            height: CHARACTER_HEIGHT,
                        },
                    ],
                    &candidate_refs,
                    ground_radius * 8.0,
                );
                let visible_map = ground_region_to_map(&visible_ground, source.plane);
                source_visible_polygons.extend(visible_map.0.iter().cloned());
            }
            let source_visible = if source_visible_polygons.is_empty() {
                MultiPolygon::new(Vec::new())
            } else {
                unary_union(&source_visible_polygons)
            };
            let ground_us = ground_started.elapsed().as_micros();
            let surfaces_started = web_time::Instant::now();
            let visible_projection = visible_camera_surfaces(
                source,
                obstacles,
                &prepared_surfaces,
                &source_visible,
                &map_domains,
            );
            tracing::trace!(target: "fog_perf", prepared_us, ground_us,
                surfaces_us = surfaces_started.elapsed().as_micros(),
                faces = prepared_surfaces.faces.len(), obstacles = candidate_obstacles.len(),
                "fog source");
            (
                source_visible,
                visible_projection,
                source_key,
                obstacle_key,
                prepared_surfaces,
            )
        };
        #[cfg(not(target_arch = "wasm32"))]
        let source_regions: Vec<_> = sources.par_iter().map(compute_source).collect();
        #[cfg(target_arch = "wasm32")]
        let source_regions: Vec<_> = sources.iter().map(compute_source).collect();
        let sources_us = scan_started.elapsed().as_micros();
        fog.scan_cache.entries = source_regions
            .iter()
            .map(
                |(
                    visible_ground,
                    visible_projection,
                    source_key,
                    obstacle_key,
                    prepared_surfaces,
                )| {
                    crate::fog_of_war::FogScanCacheEntry {
                        source_key: *source_key,
                        obstacle_key: obstacle_key.clone(),
                        visible: crate::fog_of_war::FogRegion::from_geo(visible_ground),
                        visible_projection: crate::fog_of_war::FogRegion::from_geo(
                            visible_projection,
                        ),
                        prepared_surfaces: prepared_surfaces.clone(),
                    }
                },
            )
            .collect();
        for (visible_ground, visible_projection, _, _, _) in source_regions {
            fog.add_visible_region(&visible_ground);
            fog.add_visible_projection_region(&visible_projection);
        }

        let frame = self.control.frame_counter;
        for (entity_id, entity) in self
            .world
            .entities
            .humans()
            .map(|(id, entity)| (id.into(), entity))
        {
            if !entity.is_active()
                || entity.element_data().hidden_in_building
                || !self.is_hostile_to_player(entity, &player_camps)
            {
                continue;
            }
            let position = entity.element_data().position_map();
            let directly_visible = fog.cell_state(position) == FogCellState::Visible;
            if directly_visible {
                fog.remember_visible(entity_id, position, frame, SPOTTED_HYSTERESIS_FRAMES, true);
            }
            if fog.is_spotted(entity_id, frame) {
                // Hysteresis and Listen expose the live actor. Keep its
                // ground around the live actor clear while intelligence is active,
                // without updating the stationary last-known position.
                mark_position_visible(&mut fog, entity.element_data().position_map());
            }
        }
        // Listen can expose blipped non-human objects as well as hostile
        // humans. Preserve ground around their live position for the reveal
        // interval; the intelligence record, not permanent `blipped`, owns
        // this temporary presentation state.
        let spotted_entities: Vec<_> = fog.spotted_entity_ids(frame).collect();
        for entity_id in spotted_entities {
            let Some(entity) = self.world.entities.get(entity_id) else {
                continue;
            };
            if entity.is_active() && !entity.element_data().hidden_in_building {
                mark_position_visible(&mut fog, entity.element_data().position_map());
            }
        }
        let intelligence_us = scan_started.elapsed().as_micros() - sources_us;
        fog.finish_scan(previous_visible);
        fog.retain_entities(|entity_id| self.world.entities.get(entity_id).is_some());
        tracing::trace!(target: "fog_perf", sources_us, intelligence_us,
            total_us = scan_started.elapsed().as_micros(), "fog scan");
        self.players.fog_of_war = fog;
    }

    pub(crate) fn reveal_entity_from_listen(&mut self, entity_id: EntityId) {
        if !self.control.sim_config.fog_of_war {
            return;
        }
        let entity =
            self.world.entities.get(entity_id).unwrap_or_else(|| {
                panic!("Listen fog reveal references missing entity {entity_id:?}")
            });
        if !entity.is_active() || entity.element_data().hidden_in_building {
            return;
        }
        let position = entity.element_data().position_map();
        let track_last_known =
            entity.is_human() && self.is_hostile_to_player(entity, &self.player_camps());
        self.players.fog_of_war.remember_visible(
            entity_id,
            position,
            self.control.frame_counter,
            LISTEN_REVEAL_FRAMES,
            track_last_known,
        );
        if !self.players.fog_of_war.reveal_position_now(position) {
            tracing::warn!(
                ?position,
                ?entity_id,
                "Listen revealed an entity outside the initialized fog region"
            );
        }
    }

    fn player_camps(&self) -> Vec<Camp> {
        let mut camps: Vec<_> = self
            .world
            .entities
            .pcs()
            .filter(|(_, pc)| pc.element.active && pc.pc.playable)
            .map(|(_, pc)| pc.pc.cached_camp)
            .filter(|camp| camp.allegiance_id().is_some())
            .collect();
        camps.sort_unstable();
        camps.dedup();
        camps
    }

    fn fog_vision_sources(&self) -> Vec<VisionSource> {
        let standard_radius = if self.ai.standard_view_polygon_radius > 0 {
            self.ai.standard_view_polygon_radius as f32
        } else {
            crate::ai_vision::DEFAULT_VIEW_RADIUS as f32
        };
        self.world
            .entities
            .humans()
            .filter_map(|(_, entity)| {
                let Entity::Pc(pc) = entity else {
                    return None;
                };
                let element = &pc.element;
                if !pc.pc.playable
                    || !element.active
                    || element.hidden_in_building
                    || entity.is_dead()
                    || pc.human.unconscious
                {
                    return None;
                }
                let eye = entity.compute_eyes_point(None)?;
                let posture_factor = if element.posture == Posture::OnShoulders {
                    ON_SHOULDERS_FACTOR
                } else {
                    1.0
                };
                // Ladder/wall obstacles carry the vertical plane used to move
                // the actor along the climb. It is not terrain on which a
                // visibility domain can be evaluated: projecting a map-space
                // circle through that near-vertical plane stretches it across
                // the level. Use the horizontal slice through the actor's
                // feet while preserving the real eye and obstacle volumes.
                let plane = if matches!(
                    element.posture,
                    Posture::OnLadder | Posture::OnWall | Posture::Flying
                ) {
                    Some(crate::position_interface::PlaneZCoeffs {
                        az: 0.0,
                        bz: 0.0,
                        dz: element.position().z,
                    })
                } else {
                    entity.position_iface().get_plane().copied()
                };
                Some(VisionSource {
                    eye,
                    map_position: element.position_map(),
                    plane,
                    eye_height: eye.z - element.position().z,
                    standard_radius,
                    // `blip_detection_range_percent` controls the Original's
                    // hidden-object discovery mechanic. Applying it here made
                    // Legendary fog only 40% as wide as the configured PC
                    // view, despite the fog option promising a multiple of
                    // the mission's standard radius.
                    super_factor: FOG_VIEW_DISTANCE_FACTOR * posture_factor,
                })
            })
            .collect()
    }

    fn is_allied_to_player(&self, entity: &Entity, player_camps: &[Camp]) -> bool {
        self.relationship_to_player_anchors(entity.camp(), player_camps)
            == Some(crate::diplomacy::Relationship::Allied)
    }

    fn is_hostile_to_player(&self, entity: &Entity, player_camps: &[Camp]) -> bool {
        self.relationship_to_player_anchors(entity.camp(), player_camps)
            == Some(crate::diplomacy::Relationship::Hostile)
    }

    fn relationship_to_player_anchors(
        &self,
        camp: Camp,
        player_camps: &[Camp],
    ) -> Option<crate::diplomacy::Relationship> {
        if camp.allegiance_id().is_none() || player_camps.is_empty() {
            return None;
        }
        let mut saw_neutral = false;
        for &player_camp in player_camps {
            if player_camp.allegiance_id().is_none() {
                continue;
            }
            match self
                .mission_domain
                .diplomacy
                .relationship(camp, player_camp)
            {
                crate::diplomacy::Relationship::Hostile => {
                    return Some(crate::diplomacy::Relationship::Hostile);
                }
                crate::diplomacy::Relationship::Neutral => saw_neutral = true,
                crate::diplomacy::Relationship::Allied => {}
            }
        }
        Some(if saw_neutral {
            crate::diplomacy::Relationship::Neutral
        } else {
            crate::diplomacy::Relationship::Allied
        })
    }
}

fn mark_position_visible(fog: &mut FogOfWarState, position: MapPoint) {
    if !fog.mark_position_visible(position) {
        tracing::warn!(?position, "cannot mark invalid fog position visible");
    }
}

fn fog_target_eye(source: &VisionSource, target: MapPoint) -> WorldPoint3D {
    // TODO(fog-stacked-surfaces): The fog bitmap is still a single 2D field.
    // A map with independently visible stacked floors needs per-surface fog;
    // until then, sample the observer's current projection plane consistently.
    let surface_z = source
        .plane
        .map_or(0.0, |plane| plane.compute_z(target.x, target.y));
    let ground = GroundPoint::from_map_and_z(target, surface_z);
    WorldPoint3D::new(ground.x, ground.y, surface_z + source.eye_height.max(0.0))
}

fn fog_obstacle_candidates(
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    source: &VisionSource,
    min_map: MapPoint,
    max_map: MapPoint,
) -> Vec<crate::sight_obstacle::SightObstacleIndex> {
    let corners = [
        min_map,
        MapPoint::new(min_map.x, max_map.y),
        MapPoint::new(max_map.x, min_map.y),
        max_map,
    ];
    let mut min_x = source.eye.x;
    let mut min_y = source.eye.y;
    let mut max_x = source.eye.x;
    let mut max_y = source.eye.y;
    for corner in corners {
        let world = fog_target_eye(source, corner);
        min_x = min_x.min(world.x);
        min_y = min_y.min(world.y);
        max_x = max_x.max(world.x);
        max_y = max_y.max(world.y);
    }
    let bbox = GroundBBox::from_corners(
        GroundPoint::new(min_x, min_y),
        GroundPoint::new(max_x, max_y),
    );
    let mut candidates = Vec::new();
    for layer in 0..=fast_grid.level.special_layer {
        for index in fast_grid.get_obstacle_indices(layer, &bbox) {
            if !candidates.contains(&index) {
                candidates.push(index);
            }
        }
    }
    candidates
}

fn fog_map_domains(
    center: MapPoint,
    radius: f32,
    level_size: crate::coordinates::MapSize,
) -> Vec<Polygon<f32>> {
    const CIRCLE_SEGMENTS: usize = 128;
    let circle = Polygon::new(
        closed_line_string((0..CIRCLE_SEGMENTS).map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / CIRCLE_SEGMENTS as f32;
            Coord {
                x: center.x + radius * angle.cos(),
                y: center.y + radius * angle.sin(),
            }
        })),
        Vec::new(),
    );
    let level = Polygon::new(
        closed_line_string([
            Coord { x: 0.0, y: 0.0 },
            Coord {
                x: level_size.x,
                y: 0.0,
            },
            Coord {
                x: level_size.x,
                y: level_size.y,
            },
            Coord {
                x: 0.0,
                y: level_size.y,
            },
        ]),
        Vec::new(),
    );
    circle.intersection(&level).0
}

fn closed_line_string(points: impl IntoIterator<Item = Coord<f32>>) -> LineString<f32> {
    let mut points: Vec<_> = points.into_iter().collect();
    if let Some(&first) = points.first() {
        points.push(first);
    }
    LineString::new(points)
}

/// Projection can reverse a face's winding. Geo's bulk union requires all
/// exterior rings to have the same winding, otherwise overlapping front/back
/// faces cancel and leave holes in camera coverage (notably bridge tops).
fn unary_union(polygons: &[Polygon<f32>]) -> MultiPolygon<f32> {
    let oriented: Vec<_> = polygons
        .iter()
        .map(|polygon| polygon.orient(Direction::Default))
        .collect();
    geo::algorithm::unary_union(&oriented)
}

fn map_polygon_to_ground(
    polygon: &Polygon<f32>,
    plane: Option<crate::position_interface::PlaneZCoeffs>,
) -> Polygon<f32> {
    Polygon::new(
        map_ring_to_ground(polygon.exterior(), plane),
        polygon
            .interiors()
            .iter()
            .map(|ring| map_ring_to_ground(ring, plane))
            .collect(),
    )
}

fn map_ring_to_ground(
    ring: &LineString<f32>,
    plane: Option<crate::position_interface::PlaneZCoeffs>,
) -> LineString<f32> {
    LineString::new(
        ring.0
            .iter()
            .map(|coord| {
                let map = MapPoint::new(coord.x, coord.y);
                let z = plane.map_or(0.0, |plane| plane.compute_z(map.x, map.y));
                let ground = GroundPoint::from_map_and_z(map, z);
                Coord {
                    x: ground.x,
                    y: ground.y,
                }
            })
            .collect(),
    )
}

fn ground_region_to_map(
    region: &MultiPolygon<f32>,
    plane: Option<crate::position_interface::PlaneZCoeffs>,
) -> MultiPolygon<f32> {
    MultiPolygon::new(
        region
            .0
            .iter()
            .map(|polygon| {
                Polygon::new(
                    ground_ring_to_map(polygon.exterior(), plane),
                    polygon
                        .interiors()
                        .iter()
                        .map(|ring| ground_ring_to_map(ring, plane))
                        .collect(),
                )
            })
            .collect(),
    )
}

fn ground_ring_to_map(
    ring: &LineString<f32>,
    plane: Option<crate::position_interface::PlaneZCoeffs>,
) -> LineString<f32> {
    LineString::new(
        ring.0
            .iter()
            .map(|coord| {
                let z = plane.map_or(0.0, |plane| plane.compute_world_z(coord.x, coord.y));
                Coord {
                    x: coord.x,
                    y: coord.y - z,
                }
            })
            .collect(),
    )
}

fn fog_horizontal_radius(standard_radius: f32, super_factor: f32) -> f32 {
    standard_radius * super_factor
}

fn fog_source_key(source: &VisionSource) -> [u32; 12] {
    let (plane_present, plane_az, plane_bz, plane_dz) =
        source.plane.map_or((0, 0.0, 0.0, 0.0), |plane| {
            (1, plane.az, plane.bz, plane.dz)
        });
    [
        source.eye.x.to_bits(),
        source.eye.y.to_bits(),
        source.eye.z.to_bits(),
        source.map_position.x.to_bits(),
        source.map_position.y.to_bits(),
        plane_present,
        plane_az.to_bits(),
        plane_bz.to_bits(),
        plane_dz.to_bits(),
        source.eye_height.to_bits(),
        source.standard_radius.to_bits(),
        source.super_factor.to_bits(),
    ]
}

fn fog_obstacle_key(
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
    candidates: &[crate::sight_obstacle::SightObstacleIndex],
) -> Vec<u32> {
    let mut key = Vec::new();
    for &candidate in candidates {
        let index = usize::from(candidate);
        key.push(index as u32);
        key.push(u32::from(obstacles.is_active(index)));
        if let Some(obstacle) = obstacles.get(index) {
            key.push(obstacle.id);
            key.push(obstacle.obstacle_type);
            for point in &obstacle.obstacle_points {
                key.push(point.x.to_bits());
                key.push(point.y.to_bits());
                key.push(point.z_top.to_bits());
                key.push(point.z_bottom.to_bits());
            }
        } else {
            // A missing indexed obstacle is diagnosed by the visibility pass;
            // keep it distinct here so it can never reuse a valid cache entry.
            key.push(u32::MAX);
        }
    }
    key.push(obstacles.dynamic_obstacles.len() as u32);
    for obstacle in obstacles.dynamic_obstacles {
        key.push(obstacle.id);
        key.push(obstacle.obstacle_type);
        for point in &obstacle.obstacle_points {
            key.push(point.x.to_bits());
            key.push(point.y.to_bits());
            key.push(point.z_top.to_bits());
            key.push(point.z_bottom.to_bits());
        }
    }
    key
}

#[derive(Debug, Clone)]
struct FogMeshTriangle {
    owner: usize,
    face: usize,
    world: [[f32; 3]; 3],
    projected: Polygon<f32>,
    projected_bbox: [f32; 4],
}

#[derive(Debug, Clone, Copy)]
struct TrianglePlane {
    origin: [f32; 3],
    u: [f32; 3],
    v: [f32; 3],
    normal: [f32; 3],
}

#[derive(Debug, Clone)]
struct PreparedFogFace {
    key: (usize, usize),
    plane: TrianglePlane,
    camera_owned: MultiPolygon<f32>,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedFogSurfaces {
    candidate_indices: Vec<usize>,
    coverage: MultiPolygon<f32>,
    faces: Vec<PreparedFogFace>,
}

impl Default for PreparedFogSurfaces {
    fn default() -> Self {
        Self {
            candidate_indices: Vec::new(),
            coverage: MultiPolygon::new(Vec::new()),
            faces: Vec::new(),
        }
    }
}

/// Return exactly the authored mesh surfaces visible both to the player and
/// to the fixed isometric camera.
///
/// Sight obstacles are closed meshes: a top cap plus two triangles for every
/// side quad. Visibility is solved on each triangle's own 3D plane. The final
/// composition is a vector depth buffer: pairwise affine depth comparisons
/// retain only the camera-nearest projected face at every point. This prevents
/// a visible courtyard or roof behind a wall from clearing the wall's pixels.
fn prepare_camera_surfaces(
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
    candidates: &[crate::sight_obstacle::SightObstacleIndex],
) -> PreparedFogSurfaces {
    let candidate_indices: Vec<usize> =
        candidates.iter().map(|&index| usize::from(index)).collect();
    let mut triangles = Vec::new();
    for &candidate in candidates {
        let obstacle_index = usize::from(candidate);
        if !obstacles.is_active(obstacle_index) {
            continue;
        }
        let Some(obstacle) = obstacles.get(obstacle_index) else {
            tracing::warn!(
                obstacle_index,
                "fog surface references missing sight obstacle"
            );
            continue;
        };
        triangles.extend(obstacle_mesh_triangles(obstacle_index, obstacle));
    }

    let raw_surfaces: Vec<_> = triangles
        .into_iter()
        .filter_map(|triangle| {
            let Some(plane) = TrianglePlane::from_triangle(triangle.world) else {
                tracing::warn!(
                    obstacle_index = triangle.owner,
                    "fog skipped degenerate sight-obstacle mesh triangle"
                );
                return None;
            };
            Some((triangle, plane))
        })
        .collect();

    let coverage_polygons: Vec<_> = raw_surfaces
        .iter()
        .map(|(triangle, _)| triangle.projected.clone())
        .collect();
    let coverage = if coverage_polygons.is_empty() {
        MultiPolygon::new(Vec::new())
    } else {
        unary_union(&coverage_polygons)
    };
    #[cfg(not(target_arch = "wasm32"))]
    let camera_owned: Vec<_> = (0..raw_surfaces.len())
        .into_par_iter()
        .map(|index| camera_owned_surface(index, &raw_surfaces))
        .collect();
    #[cfg(target_arch = "wasm32")]
    let camera_owned: Vec<_> = (0..raw_surfaces.len())
        .map(|index| camera_owned_surface(index, &raw_surfaces))
        .collect();
    let owned_surfaces: Vec<_> = raw_surfaces
        .into_iter()
        .zip(camera_owned)
        .filter_map(|((triangle, _), mut camera_owned)| {
            retain_finite_polygons(&mut camera_owned);
            (!camera_owned.0.is_empty()).then_some((triangle, camera_owned))
        })
        .collect();
    let mut face_parts: BTreeMap<_, (TrianglePlane, Vec<Polygon<f32>>)> = BTreeMap::new();
    for (triangle, camera_owned) in &owned_surfaces {
        let key = (triangle.owner, triangle.face);
        let plane = TrianglePlane::from_triangle(triangle.world)
            .expect("prepared fog surface lost its non-degenerate plane");
        let entry = face_parts.entry(key).or_insert_with(|| (plane, Vec::new()));
        // Match the uncached exact implementation's BTreeMap collection:
        // authored top caps may be triangulated but not perfectly coplanar,
        // and duplicate face keys deliberately retain the last triangle's
        // projection plane. Keeping the first plane instead can project an
        // owning obstacle's shadow across its PC-facing wall.
        entry.0 = plane;
        entry.1.extend(camera_owned.0.iter().cloned());
    }
    let faces = face_parts
        .into_iter()
        .filter_map(|(key, (plane, polygons))| {
            if polygons.is_empty() {
                return None;
            }
            let mut camera_owned = unary_union(&polygons);
            retain_finite_polygons(&mut camera_owned);
            if camera_owned.0.is_empty() {
                return None;
            }
            Some(PreparedFogFace {
                key,
                plane,
                camera_owned,
            })
        })
        .collect();
    PreparedFogSurfaces {
        candidate_indices,
        coverage,
        faces,
    }
}

fn retain_finite_polygons(polygons: &mut MultiPolygon<f32>) {
    polygons.0.retain(|polygon| {
        polygon.exterior().0.len() >= 4
            && polygon
                .exterior()
                .0
                .iter()
                .all(|point| point.x.is_finite() && point.y.is_finite())
            && polygon.unsigned_area().is_finite()
            && polygon.unsigned_area() > 0.001
    });
}

fn visible_camera_surfaces(
    source: &VisionSource,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
    prepared: &PreparedFogSurfaces,
    visible_ground: &MultiPolygon<f32>,
    map_domains: &[Polygon<f32>],
) -> MultiPolygon<f32> {
    let mut opaque_obstacles: Vec<PreparedFogOccluder> = prepared
        .candidate_indices
        .iter()
        .filter_map(|&index| {
            obstacles
                .is_active(index)
                .then(|| obstacles.get(index))
                .flatten()
                .filter(|obstacle| obstacle.is_opaque())
                .and_then(|obstacle| PreparedFogOccluder::new(index, obstacle))
        })
        .collect();
    opaque_obstacles.extend(
        obstacles
            .dynamic_obstacles
            .iter()
            .enumerate()
            .filter(|(_, obstacle)| obstacle.is_opaque())
            .filter_map(|(index, obstacle)| {
                PreparedFogOccluder::new(obstacles.static_obstacles.len() + index, obstacle)
            }),
    );

    // Camera ownership and mesh coverage do not depend on the observer. They
    // remain exact while being reused across sub-pixel movement until the
    // surrounding obstacle key changes.
    let mut visible_parts = visible_ground.difference(&prepared.coverage).0;
    #[cfg(not(target_arch = "wasm32"))]
    let face_parts: Vec<_> = prepared
        .faces
        .par_iter()
        .filter_map(|face| visible_prepared_face(source, face, &opaque_obstacles, map_domains))
        .collect();
    #[cfg(target_arch = "wasm32")]
    let face_parts: Vec<_> = prepared
        .faces
        .iter()
        .filter_map(|face| visible_prepared_face(source, face, &opaque_obstacles, map_domains))
        .collect();
    visible_parts.extend(face_parts.into_iter().flatten());
    if visible_parts.is_empty() {
        MultiPolygon::new(Vec::new())
    } else {
        unary_union(&visible_parts)
    }
}

fn obstacle_mesh_triangles(
    owner: usize,
    obstacle: &crate::sight_obstacle::SightObstacle,
) -> Vec<FogMeshTriangle> {
    let points = &obstacle.obstacle_points;
    if points.len() < 3 {
        tracing::warn!(
            obstacle_id = obstacle.id,
            "fog mesh has fewer than three points"
        );
        return Vec::new();
    }
    let bottom = |index: usize| {
        let point = points[index];
        [point.x, point.y, point.z_bottom]
    };
    let top = |index: usize| {
        let point = points[index];
        [point.x, point.y, point.z_top]
    };
    let mut world_triangles = Vec::with_capacity(3 * points.len() - 2);
    let footprint = Polygon::new(
        closed_line_string(points.iter().map(|point| Coord {
            x: point.x,
            y: point.y,
        })),
        Vec::new(),
    );
    let convex = footprint.unsigned_area() + 0.001 >= footprint.convex_hull().unsigned_area();
    if convex {
        // Preserve authored ordering for convex, occasionally non-planar caps.
        for index in 1..points.len() - 1 {
            world_triangles.push((0, [top(0), top(index), top(index + 1)]));
        }
    } else {
        // A vertex-zero fan is not a triangulation of a concave footprint:
        // it leaves missing camera-depth coverage (or invents coverage across
        // an opening), allowing a visible surface behind a wall to clear the
        // wall's pixels. Use the same exact cap decomposition as sight rays.
        let triangulation = footprint.earcut_triangles_raw();
        for triangle in triangulation.triangle_indices.chunks_exact(3) {
            world_triangles.push((0, [top(triangle[0]), top(triangle[1]), top(triangle[2])]));
        }
    }
    for index in 0..points.len() {
        let next = (index + 1) % points.len();
        let first = [bottom(index), bottom(next), top(next)];
        let second = [bottom(index), top(next), top(index)];
        let shared_face = TrianglePlane::from_triangle(first)
            .is_some_and(|plane| plane.signed_distance(top(index)).abs() <= 0.01);
        world_triangles.push((1 + index * 2, first));
        world_triangles.push((1 + index * 2 + usize::from(!shared_face), second));
    }
    world_triangles
        .into_iter()
        .filter_map(|(face, world)| {
            let projected = Polygon::new(
                closed_line_string(world.map(|point| Coord {
                    x: point[0],
                    y: point[1] - point[2],
                })),
                Vec::new(),
            );
            (projected.unsigned_area() > 0.001).then(|| FogMeshTriangle {
                owner,
                face,
                world,
                projected_bbox: polygon_bbox(&projected),
                projected,
            })
        })
        .collect()
}

impl TrianglePlane {
    fn from_triangle(points: [[f32; 3]; 3]) -> Option<Self> {
        let u = normalize3(sub3(points[1], points[0]))?;
        let normal = normalize3(cross3(
            sub3(points[1], points[0]),
            sub3(points[2], points[0]),
        ))?;
        let v = cross3(normal, u);
        Some(Self {
            origin: points[0],
            u,
            v,
            normal,
        })
    }

    fn signed_distance(self, point: [f32; 3]) -> f32 {
        dot3(sub3(point, self.origin), self.normal)
    }

    fn local(self, point: [f32; 3]) -> Coord<f32> {
        let relative = sub3(point, self.origin);
        Coord {
            x: dot3(relative, self.u),
            y: dot3(relative, self.v),
        }
    }

    fn world(self, point: Coord<f32>) -> [f32; 3] {
        add3(
            self.origin,
            add3(scale3(self.u, point.x), scale3(self.v, point.y)),
        )
    }

    /// Camera-ray depth at projected `(x, y)`. A projected point corresponds
    /// to world `(x, y + z, z)`, so the plane intersection is affine in map
    /// coordinates and larger Z is nearer the fixed camera.
    fn camera_depth(self, point: Coord<f32>) -> f32 {
        let denominator = self.normal[1] + self.normal[2];
        assert!(
            denominator.abs() > 1.0e-6,
            "non-degenerate projected triangle must intersect the camera ray"
        );
        -(self.normal[0] * (point.x - self.origin[0]) + self.normal[1] * (point.y - self.origin[1])
            - self.normal[2] * self.origin[2])
            / denominator
    }
}

fn projected_triangle_in_front_of(
    triangle: &FogMeshTriangle,
    plane: TrianglePlane,
    target_plane: TrianglePlane,
) -> MultiPolygon<f32> {
    const DEPTH_EPSILON: f32 = 1.0e-4;
    let difference = |point: Coord<f32>| {
        plane.camera_depth(point) - target_plane.camera_depth(point) - DEPTH_EPSILON
    };
    let vertices = &triangle.projected.exterior().0[..3];
    let min = vertices
        .iter()
        .copied()
        .map(difference)
        .fold(f32::INFINITY, f32::min);
    let max = vertices
        .iter()
        .copied()
        .map(difference)
        .fold(f32::NEG_INFINITY, f32::max);
    if max <= 0.0 {
        return MultiPolygon::new(Vec::new());
    }
    if min >= 0.0 {
        return MultiPolygon::new(vec![triangle.projected.clone()]);
    }
    let clipped = clip_projected_ring(vertices, difference);
    if clipped.len() < 3 {
        MultiPolygon::new(Vec::new())
    } else {
        MultiPolygon::new(vec![Polygon::new(closed_line_string(clipped), Vec::new())])
    }
}

fn clip_projected_ring(
    input: &[Coord<f32>],
    signed_distance: impl Fn(Coord<f32>) -> f32,
) -> Vec<Coord<f32>> {
    let Some(mut previous) = input.last().copied() else {
        return Vec::new();
    };
    let mut previous_distance = signed_distance(previous);
    let mut previous_inside = previous_distance >= 0.0;
    let mut output = Vec::new();
    for &current in input {
        let current_distance = signed_distance(current);
        let current_inside = current_distance >= 0.0;
        if current_inside != previous_inside {
            let fraction = previous_distance / (previous_distance - current_distance);
            output.push(Coord {
                x: previous.x + (current.x - previous.x) * fraction,
                y: previous.y + (current.y - previous.y) * fraction,
            });
        }
        if current_inside {
            output.push(current);
        }
        previous = current;
        previous_distance = current_distance;
        previous_inside = current_inside;
    }
    output
}

fn polygon_bbox(polygon: &Polygon<f32>) -> [f32; 4] {
    polygon.exterior().0.iter().fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ],
        |mut bbox, point| {
            bbox[0] = bbox[0].min(point.x);
            bbox[1] = bbox[1].min(point.y);
            bbox[2] = bbox[2].max(point.x);
            bbox[3] = bbox[3].max(point.y);
            bbox
        },
    )
}

fn bboxes_intersect(left: [f32; 4], right: [f32; 4]) -> bool {
    left[0] <= right[2] && left[2] >= right[0] && left[1] <= right[3] && left[3] >= right[1]
}

fn camera_owned_surface(
    index: usize,
    surfaces: &[(FogMeshTriangle, TrianglePlane)],
) -> MultiPolygon<f32> {
    let (triangle, plane) = &surfaces[index];
    let mut owned = MultiPolygon::new(vec![triangle.projected.clone()]);
    for (other_index, (other, other_plane)) in surfaces.iter().enumerate() {
        if index == other_index || !bboxes_intersect(triangle.projected_bbox, other.projected_bbox)
        {
            continue;
        }
        let front = projected_triangle_in_front_of(other, *other_plane, *plane);
        if !front.0.is_empty() {
            owned = owned.difference(&front);
            if owned.0.is_empty() {
                return owned;
            }
        }
    }
    owned
}

fn visible_prepared_face(
    source: &VisionSource,
    face: &PreparedFogFace,
    opaque_obstacles: &[PreparedFogOccluder],
    map_domains: &[Polygon<f32>],
) -> Option<Vec<Polygon<f32>>> {
    // The original visibility test looks from the actor's eye towards the
    // feet/head surfaces inside the view radius. A mesh face is therefore a
    // possible first hit only where that finite bundle of rays crosses it.
    // Treating every camera-facing mesh pixel as an independent sight target
    // gives the actor unrestricted vertical look-up: a ray through a doorway
    // then incorrectly clears the lintel and wall above it.
    let view_volume = view_volume_on_plane(source, face.plane, map_domains);
    if view_volume.0.is_empty() {
        return None;
    }
    let target = face.camera_owned.intersection(&view_volume);
    if target.0.is_empty() {
        return None;
    }
    // Clip before projecting obstacle shadows. Besides enforcing the correct
    // vertical field, this rejects most distant/elevated faces and greatly
    // reduces the exact face-by-obstacle work performed while walking.
    let shadows = projected_shadows_on_plane(source, face, &target, opaque_obstacles);
    Some(target.difference(&shadows).0)
}

fn projected_shadows_on_plane(
    source: &VisionSource,
    target: &PreparedFogFace,
    target_region: &MultiPolygon<f32>,
    opaque_obstacles: &[PreparedFogOccluder],
) -> MultiPolygon<f32> {
    let viewer = [source.eye.x, source.eye.y, source.eye.z];
    let local_bounds = projection_region_local_bbox(target_region, target.plane);
    let corners = [
        Point::new(local_bounds[0], local_bounds[1]),
        Point::new(local_bounds[2], local_bounds[1]),
        Point::new(local_bounds[2], local_bounds[3]),
        Point::new(local_bounds[0], local_bounds[3]),
    ];
    // Every sight segment lies in the AABB of the eye and target face.
    // Reject disjoint volumes in world space before perspective projection;
    // in particular overhead obstacles cannot shadow rays entirely below them.
    let mut ray_min = viewer;
    let mut ray_max = viewer;
    for corner in corners {
        let world = target.plane.world(corner.0);
        for axis in 0..3 {
            ray_min[axis] = ray_min[axis].min(world[axis]);
            ray_max[axis] = ray_max[axis].max(world[axis]);
        }
    }
    let mut shadows = Vec::new();
    for obstacle in opaque_obstacles {
        if (0..3).any(|axis| {
            obstacle.bounds_max[axis] < ray_min[axis] - 0.001
                || obstacle.bounds_min[axis] > ray_max[axis] + 0.001
        }) {
            continue;
        }
        let Some(shadow) = obstacle_shadow_on_plane(
            viewer,
            target.plane,
            obstacle,
            obstacle.owner == target.key.0,
            local_bounds,
        ) else {
            continue;
        };
        // For a convex shadow component (a clipped mesh triangle or
        // a convex obstacle envelope), covering all four corners of the
        // target bounds, the whole face is hidden. Avoid constructing and
        // unioning every remaining obstacle shadow for that invisible face.
        if shadow.exact.iter().any(|polygon| {
            corners.iter().all(|corner| polygon.intersects(corner))
                && polygon.exterior().is_convex()
        }) {
            return target_region.clone();
        }
        shadows.extend(shadow.exact);
    }
    if shadows.is_empty() {
        return MultiPolygon::new(Vec::new());
    }
    map_local_region_to_projection(&unary_union(&shadows), target.plane)
}

/// Intersect the finite feet/head sight-ray volume with one authored mesh
/// plane and return that cross-section in camera projection coordinates.
fn view_volume_on_plane(
    source: &VisionSource,
    target_plane: TrianglePlane,
    map_domains: &[Polygon<f32>],
) -> MultiPolygon<f32> {
    let viewer = [source.eye.x, source.eye.y, source.eye.z];
    let viewer_distance = target_plane.signed_distance(viewer);
    if viewer_distance.abs() <= 0.001 {
        // TODO(fog-eye-coplanar-view-volume): construct the in-plane radial
        // section for a facade passing exactly through the observer.
        return MultiPolygon::new(Vec::new());
    }

    let mut sections = Vec::new();
    for domain in map_domains {
        let map_ring = &domain.exterior().0;
        if map_ring.len() < 4 {
            continue;
        }
        let mut projected_points = Vec::new();
        let upper_height = CHARACTER_HEIGHT.max(source.eye_height + FOG_VERTICAL_AIM_MARGIN);
        for height in [0.0, upper_height] {
            let endpoints: Vec<_> = map_ring[..map_ring.len() - 1]
                .iter()
                .map(|coord| {
                    let map = MapPoint::new(coord.x, coord.y);
                    let z = source
                        .plane
                        .map_or(0.0, |plane| plane.compute_z(map.x, map.y));
                    let ground = GroundPoint::from_map_and_z(map, z);
                    [ground.x, ground.y, z + height]
                })
                .collect();
            // Retain only endpoints beyond the target plane. Their finite
            // viewer-to-endpoint segments actually cross this mesh plane;
            // same-side endpoints would only hit it after the view radius.
            let crossing_endpoints = clip_world_face(
                &endpoints,
                |point| target_plane.signed_distance(point) / viewer_distance,
                0.0,
                false,
            );
            projected_points.extend(crossing_endpoints.into_iter().filter_map(|point| {
                let point_distance = target_plane.signed_distance(point);
                let denominator = viewer_distance - point_distance;
                if denominator.abs() <= f32::EPSILON {
                    return None;
                }
                let fraction = viewer_distance / denominator;
                let intersection = add3(viewer, scale3(sub3(point, viewer), fraction));
                Some(Point::from(target_plane.local(intersection)))
            }));
        }
        if projected_points.len() < 3 {
            continue;
        }
        let section = MultiPoint::new(projected_points).convex_hull();
        if section.unsigned_area() > 0.001 {
            sections.push(section);
        }
    }
    if sections.is_empty() {
        return MultiPolygon::new(Vec::new());
    }
    map_local_region_to_projection(&unary_union(&sections), target_plane)
}

struct ObstacleShadow {
    /// One polygon for convex obstacles; mesh-face projections otherwise.
    exact: Vec<Polygon<f32>>,
}

struct PreparedFogOccluder {
    owner: usize,
    convex: bool,
    vertices: Vec<[f32; 3]>,
    faces: Vec<Vec<[f32; 3]>>,
    bounds_min: [f32; 3],
    bounds_max: [f32; 3],
}

impl PreparedFogOccluder {
    fn new(owner: usize, obstacle: &crate::sight_obstacle::SightObstacle) -> Option<Self> {
        let points = &obstacle.obstacle_points;
        if points.len() < 3 {
            return None;
        }
        let bottom = |index: usize| {
            let point = points[index];
            [point.x, point.y, point.z_bottom]
        };
        let top = |index: usize| {
            let point = points[index];
            [point.x, point.y, point.z_top]
        };
        let footprint = Polygon::new(
            closed_line_string(points.iter().map(|point| Coord {
                x: point.x,
                y: point.y,
            })),
            Vec::new(),
        );
        let convex = footprint.unsigned_area() + 0.001 >= footprint.convex_hull().unsigned_area();
        let faces = if convex {
            let mut faces = Vec::with_capacity(points.len() + 2);
            faces.push((0..points.len()).map(top).collect());
            faces.push((0..points.len()).rev().map(bottom).collect());
            for index in 0..points.len() {
                let next = (index + 1) % points.len();
                faces.push(vec![bottom(index), bottom(next), top(next), top(index)]);
            }
            faces
        } else {
            let triangulation = footprint.earcut_triangles_raw();
            let mut faces =
                Vec::with_capacity(triangulation.triangle_indices.len() * 2 / 3 + points.len() * 2);
            for triangle in triangulation.triangle_indices.chunks_exact(3) {
                faces.push(vec![top(triangle[0]), top(triangle[1]), top(triangle[2])]);
                faces.push(vec![
                    bottom(triangle[2]),
                    bottom(triangle[1]),
                    bottom(triangle[0]),
                ]);
            }
            for index in 0..points.len() {
                let next = (index + 1) % points.len();
                faces.push(vec![bottom(index), bottom(next), top(next)]);
                faces.push(vec![bottom(index), top(next), top(index)]);
            }
            faces
        };
        let vertices: Vec<_> = (0..points.len())
            .flat_map(|index| [bottom(index), top(index)])
            .collect();
        let mut bounds_min = [f32::INFINITY; 3];
        let mut bounds_max = [f32::NEG_INFINITY; 3];
        for vertex in &vertices {
            for axis in 0..3 {
                bounds_min[axis] = bounds_min[axis].min(vertex[axis]);
                bounds_max[axis] = bounds_max[axis].max(vertex[axis]);
            }
        }
        Some(Self {
            owner,
            convex,
            vertices,
            faces,
            bounds_min,
            bounds_max,
        })
    }
}

fn obstacle_shadow_on_plane(
    viewer: [f32; 3],
    plane: TrianglePlane,
    obstacle: &PreparedFogOccluder,
    owns_target: bool,
    local_bounds: [f32; 4],
) -> Option<ObstacleShadow> {
    const HORIZON_CLIP_FRACTION: f32 = 1.0 - 1.0e-3;
    let viewer_distance = plane.signed_distance(viewer);
    if viewer_distance.abs() <= 0.001 {
        // TODO(fog-eye-coplanar-face): split the obstacle by the target plane
        // and extrude its cross-section when a viewer lies exactly on a
        // vertical target face. Shipped walkable surfaces keep the eye away
        // from their planes; returning no shadow is preferable to inventing
        // a black strip at the projective horizon.
        return None;
    }
    // The target face is the ray endpoint, not an occluder. Move the owning
    // volume's near clip infinitesimally toward the viewer, retaining its
    // nearer walls while excluding vertices that lie on the target plane.
    let near_fraction = if owns_target { 1.0e-5 } else { 0.0 };
    // Convex prisms retain the old single-polygon fast path: for a convex
    // volume the hull of all projected, clipped faces is its exact shadow.
    // Preserve concave authored footprints by projecting their mesh faces.
    // Projecting every vertex and then
    // taking a convex hull fills courtyards and bends in castle outlines,
    // causing those nonexistent volumes to black out camera-facing walls.
    // Triangulation also keeps clipping well-defined when a concave cap is
    // split into multiple pieces by the target plane.
    // A convex volume only has a convex finite shadow while it stays on one
    // side of the projective horizon. If it crosses the viewer-parallel
    // plane, its finite projection consists of separate unbounded pieces;
    // taking their hull invents a strip between them.
    let ratios: Vec<_> = obstacle
        .vertices
        .iter()
        .map(|&point| plane.signed_distance(point) / viewer_distance)
        .collect();
    let min_ratio = ratios.iter().copied().fold(f32::INFINITY, f32::min);
    let max_ratio = ratios.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if max_ratio < near_fraction || min_ratio > HORIZON_CLIP_FRACTION {
        return None;
    }
    let use_convex_envelope = obstacle.convex && max_ratio < HORIZON_CLIP_FRACTION;

    // Fully finite obstacle projections have a conservative local bounding
    // box. Rejecting disjoint boxes here avoids clipping/unioning every mesh
    // face for the overwhelming majority of face-obstacle pairs.
    if min_ratio >= near_fraction && max_ratio <= HORIZON_CLIP_FRACTION {
        let mut projected_bounds = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for &point in &obstacle.vertices {
            let point_distance = plane.signed_distance(point);
            let denominator = viewer_distance - point_distance;
            if denominator.abs() <= f32::EPSILON {
                continue;
            }
            let fraction = viewer_distance / denominator;
            let local = plane.local(add3(viewer, scale3(sub3(point, viewer), fraction)));
            projected_bounds[0] = projected_bounds[0].min(local.x);
            projected_bounds[1] = projected_bounds[1].min(local.y);
            projected_bounds[2] = projected_bounds[2].max(local.x);
            projected_bounds[3] = projected_bounds[3].max(local.y);
        }
        if !bboxes_intersect(projected_bounds, local_bounds) {
            return None;
        }
    }

    let mut shadows = Vec::new();
    let mut envelope_points = Vec::new();
    for face in &obstacle.faces {
        let face = clip_world_face(
            face,
            |point| plane.signed_distance(point) / viewer_distance,
            near_fraction,
            true,
        );
        // The viewer-parallel plane is a projective horizon: points exactly
        // on it have no finite intersection with the target plane. Floating
        // roundoff at a threshold of 1.0 can flip those points across the
        // horizon and create enormous polygons which black out unrelated
        // facade pixels. Stay infinitesimally on the finite, forward-ray side;
        // the cap remains far beyond any bounded target mesh.
        let face = clip_world_face(
            &face,
            |point| plane.signed_distance(point) / viewer_distance,
            HORIZON_CLIP_FRACTION,
            false,
        );
        let mut projected: Vec<_> = face
            .into_iter()
            .filter_map(|point| {
                let point_distance = plane.signed_distance(point);
                let denominator = viewer_distance - point_distance;
                if denominator.abs() <= f32::EPSILON {
                    return None;
                }
                let fraction = viewer_distance / denominator;
                let intersection = add3(viewer, scale3(sub3(point, viewer), fraction));
                Some(plane.local(intersection))
            })
            .collect();
        projected = clip_projected_ring(&projected, |point| point.x - local_bounds[0]);
        projected = clip_projected_ring(&projected, |point| local_bounds[2] - point.x);
        projected = clip_projected_ring(&projected, |point| point.y - local_bounds[1]);
        projected = clip_projected_ring(&projected, |point| local_bounds[3] - point.y);
        if projected.len() < 3 {
            continue;
        }
        envelope_points.extend(projected.iter().copied().map(Point::from));
        let shadow =
            Polygon::new(closed_line_string(projected), Vec::new()).orient(Direction::Default);
        if shadow.unsigned_area() > 0.001 {
            shadows.push(shadow);
        }
    }
    if envelope_points.len() < 3 || shadows.is_empty() {
        return None;
    }
    let envelope = MultiPoint::new(envelope_points).convex_hull();
    if use_convex_envelope {
        Some(ObstacleShadow {
            exact: vec![envelope],
        })
    } else {
        Some(ObstacleShadow { exact: shadows })
    }
}

fn projection_region_local_bbox(region: &MultiPolygon<f32>, plane: TrianglePlane) -> [f32; 4] {
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for projected in region
        .0
        .iter()
        .flat_map(|polygon| polygon.exterior().0.iter())
    {
        let z = plane.camera_depth(*projected);
        let local = plane.local([projected.x, projected.y + z, z]);
        bounds[0] = bounds[0].min(local.x);
        bounds[1] = bounds[1].min(local.y);
        bounds[2] = bounds[2].max(local.x);
        bounds[3] = bounds[3].max(local.y);
    }
    assert!(
        bounds.iter().all(|value| value.is_finite()),
        "prepared fog target has no finite local bounds"
    );
    const PADDING: f32 = 0.01;
    [
        bounds[0] - PADDING,
        bounds[1] - PADDING,
        bounds[2] + PADDING,
        bounds[3] + PADDING,
    ]
}

fn clip_world_face(
    input: &[[f32; 3]],
    value: impl Fn([f32; 3]) -> f32,
    threshold: f32,
    keep_above: bool,
) -> Vec<[f32; 3]> {
    let Some(mut previous) = input.last().copied() else {
        return Vec::new();
    };
    let mut previous_value = value(previous);
    let mut previous_inside = if keep_above {
        previous_value >= threshold
    } else {
        previous_value <= threshold
    };
    let mut output = Vec::new();
    for &current in input {
        let current_value = value(current);
        let current_inside = if keep_above {
            current_value >= threshold
        } else {
            current_value <= threshold
        };
        if current_inside != previous_inside {
            let denominator = current_value - previous_value;
            if denominator.abs() > f32::EPSILON {
                let fraction = (threshold - previous_value) / denominator;
                output.push(add3(previous, scale3(sub3(current, previous), fraction)));
            }
        }
        if current_inside {
            output.push(current);
        }
        previous = current;
        previous_value = current_value;
        previous_inside = current_inside;
    }
    output
}

fn map_local_region_to_projection(
    region: &MultiPolygon<f32>,
    plane: TrianglePlane,
) -> MultiPolygon<f32> {
    let ring = |input: &LineString<f32>| {
        LineString::new(
            input
                .0
                .iter()
                .map(|&point| {
                    let world = plane.world(point);
                    Coord {
                        x: world[0],
                        y: world[1] - world[2],
                    }
                })
                .collect(),
        )
    };
    MultiPolygon::new(
        region
            .0
            .iter()
            .map(|polygon| {
                Polygon::new(
                    ring(polygon.exterior()),
                    polygon.interiors().iter().map(ring).collect(),
                )
            })
            .collect(),
    )
}

fn sub3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn add3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn scale3(vector: [f32; 3], scale: f32) -> [f32; 3] {
    [vector[0] * scale, vector[1] * scale, vector[2] * scale]
}

fn dot3(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn normalize3(vector: [f32; 3]) -> Option<[f32; 3]> {
    let length = dot3(vector, vector).sqrt();
    (length > 1.0e-6).then(|| scale3(vector, 1.0 / length))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{
        ActorData, ActorPc, ActorSoldier, ElementBonus, ElementData, ElementKind, HumanData,
        NpcData, ObjectData, PcData, SoldierData,
    };
    use crate::sight_obstacle::{ObstaclePoint, SightObstacle, SightObstacleIndex};
    use geo::{Contains, Point};

    fn positioned_element(kind: ElementKind, x: f32, y: f32) -> ElementData {
        let mut element = ElementData {
            kind,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position(WorldPoint3D::new(x, y, 0.0));
        element
    }

    fn pc_at(x: f32, y: f32) -> Entity {
        Entity::Pc(ActorPc {
            element: positioned_element(ElementKind::ActorPc, x, y),
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })
    }

    fn soldier_at(x: f32, y: f32, camp: Camp) -> Entity {
        Entity::Soldier(ActorSoldier {
            element: positioned_element(ElementKind::ActorSoldier, x, y),
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData {
                cached_camp: camp,
                ..SoldierData::default()
            },
        })
    }

    fn bonus_at(x: f32, y: f32) -> Entity {
        Entity::Bonus(ElementBonus {
            element: positioned_element(ElementKind::ObjectBonus, x, y),
            object: ObjectData::default(),
        })
    }

    fn pc_vision_fixture() -> (EngineInner, EntityId, EntityId) {
        let mut engine = EngineInner::new();
        engine.control.sim_config.fog_of_war = true;
        engine.set_level_size(1_400.0, 400.0);
        // Keep the target inside the configured 1.2x fog range while
        // leaving enough separation for an authored wall between them.
        engine.ai.standard_view_polygon_radius = 180;
        let pc = engine.add_entity(pc_at(800.0, 100.0));
        let enemy = engine.add_entity(soldier_at(1_000.0, 100.0, Camp::Lacklandists));
        engine.refresh_fog_of_war(&LevelAssets::default(), true);
        (engine, pc, enemy)
    }

    fn opaque_wall_between_pc_and_enemy() -> SightObstacle {
        let mut wall = SightObstacle::new_default(1);
        wall.obstacle_points = vec![
            ObstaclePoint {
                x: 945.0,
                y: 50.0,
                z_bottom: 0.0,
                z_top: 100.0,
            },
            ObstaclePoint {
                x: 955.0,
                y: 50.0,
                z_bottom: 0.0,
                z_top: 100.0,
            },
            ObstaclePoint {
                x: 955.0,
                y: 150.0,
                z_bottom: 0.0,
                z_top: 100.0,
            },
            ObstaclePoint {
                x: 945.0,
                y: 150.0,
                z_bottom: 0.0,
                z_top: 100.0,
            },
        ];
        wall.top_plane_points = [
            [945.0, 50.0, 100.0],
            [955.0, 50.0, 100.0],
            [955.0, 150.0, 100.0],
        ];
        wall.bottom_plane_points = [[945.0, 50.0, 0.0], [955.0, 50.0, 0.0], [955.0, 150.0, 0.0]];
        wall.rebuild_geometry();
        wall
    }

    #[test]
    fn prepared_non_planar_face_keeps_uncached_projection_plane() {
        let mut obstacle = SightObstacle::new_default(7);
        obstacle.obstacle_points = vec![
            ObstaclePoint {
                x: 100.0,
                y: 100.0,
                z_bottom: 0.0,
                z_top: 10.0,
            },
            ObstaclePoint {
                x: 200.0,
                y: 100.0,
                z_bottom: 0.0,
                z_top: 20.0,
            },
            ObstaclePoint {
                x: 200.0,
                y: 200.0,
                z_bottom: 0.0,
                z_top: 40.0,
            },
            ObstaclePoint {
                x: 100.0,
                y: 200.0,
                z_bottom: 0.0,
                z_top: 80.0,
            },
        ];
        obstacle.rebuild_geometry();
        let storage = [obstacle];
        let obstacles = crate::sight_obstacle::ObstacleList::from_slice_all_active(&storage);
        let index = SightObstacleIndex::new(0).expect("fixture obstacle index");
        let prepared = prepare_camera_surfaces(obstacles, &[index]);
        let face = prepared
            .faces
            .iter()
            .find(|face| face.key == (0, 0))
            .expect("prepared top face");
        let expected = TrianglePlane::from_triangle([
            [100.0, 100.0, 10.0],
            [200.0, 200.0, 40.0],
            [100.0, 200.0, 80.0],
        ])
        .expect("last authored top triangle plane");

        assert_eq!(face.plane.origin, expected.origin);
        assert_eq!(face.plane.u, expected.u);
        assert_eq!(face.plane.v, expected.v);
        assert_eq!(face.plane.normal, expected.normal);
    }

    #[test]
    fn obstacle_shadow_preserves_concave_opening() {
        let mut obstacle = SightObstacle::new_default(11);
        obstacle.obstacle_points = [
            (-4.0, -4.0),
            (4.0, -4.0),
            (4.0, 4.0),
            (1.0, 4.0),
            (1.0, -1.0),
            (-1.0, -1.0),
            (-1.0, 4.0),
            (-4.0, 4.0),
        ]
        .into_iter()
        .map(|(x, y)| ObstaclePoint {
            x,
            y,
            z_bottom: 5.0,
            z_top: 10.0,
        })
        .collect();
        obstacle.rebuild_geometry();
        let ground =
            TrianglePlane::from_triangle([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]])
                .expect("ground plane");
        let obstacle = PreparedFogOccluder::new(0, &obstacle).expect("prepared obstacle");

        let pieces = obstacle_shadow_on_plane(
            [0.0, 0.0, 20.0],
            ground,
            &obstacle,
            false,
            [-100.0, -100.0, 100.0, 100.0],
        )
        .expect("concave obstacle shadow");
        let shadow = unary_union(&pieces.exact);

        assert!(
            !shadow.contains(&Point::new(0.0, 3.0)),
            "the open middle of a concave obstacle must not become an occluder"
        );
        assert!(
            shadow.contains(&Point::new(3.0, 3.0)),
            "an actual arm of the concave obstacle must still cast a shadow"
        );
    }

    #[test]
    fn camera_mesh_preserves_concave_top_cap() {
        let mut obstacle = SightObstacle::new_default(13);
        obstacle.obstacle_points = [
            (-4.0, -4.0),
            (4.0, -4.0),
            (4.0, 4.0),
            (1.0, 4.0),
            (1.0, -1.0),
            (-1.0, -1.0),
            (-1.0, 4.0),
            (-4.0, 4.0),
        ]
        .into_iter()
        .map(|(x, y)| ObstaclePoint {
            x,
            y,
            z_bottom: 0.0,
            z_top: 10.0,
        })
        .collect();
        obstacle.rebuild_geometry();

        let top_parts: Vec<_> = obstacle_mesh_triangles(0, &obstacle)
            .into_iter()
            .filter(|triangle| triangle.face == 0)
            .map(|triangle| triangle.projected)
            .collect();
        let actual = unary_union(&top_parts);
        let expected = Polygon::new(
            closed_line_string(obstacle.obstacle_points.iter().map(|point| Coord {
                x: point.x,
                y: point.y - point.z_top,
            })),
            Vec::new(),
        );
        let expected = MultiPolygon::new(vec![expected]);
        let mismatch = actual.difference(&expected).unsigned_area()
            + expected.difference(&actual).unsigned_area();
        assert!(
            mismatch < 0.01,
            "camera depth coverage must exactly retain a concave cap"
        );
    }

    #[test]
    fn obstacle_crossing_projection_horizon_does_not_shadow_opposite_ray() {
        let mut obstacle = SightObstacle::new_default(12);
        obstacle.obstacle_points = [(-5.0, -2.0), (5.0, -2.0), (5.0, 2.0), (-5.0, 2.0)]
            .into_iter()
            .map(|(x, y)| ObstaclePoint {
                x,
                y,
                z_bottom: 0.0,
                z_top: 5.0,
            })
            .collect();
        obstacle.rebuild_geometry();
        let target_plane =
            TrianglePlane::from_triangle([[10.0, -1.0, 0.0], [10.0, 1.0, 0.0], [10.0, -1.0, 1.0]])
                .expect("vertical target plane");
        let obstacle = PreparedFogOccluder::new(0, &obstacle).expect("prepared obstacle");
        let shadow = obstacle_shadow_on_plane(
            [0.0, 0.0, 10.0],
            target_plane,
            &obstacle,
            false,
            [-100.0, -100.0, 100.0, 100.0],
        )
        .expect("horizon-crossing obstacle shadow");
        assert!(
            shadow.exact.len() > 1,
            "a horizon-crossing volume must retain separate projected faces"
        );
        let shadow = unary_union(&shadow.exact);

        assert!(
            !shadow.contains(&Point::from(target_plane.local([10.0, 0.0, 15.0]))),
            "a target ray rising away from a lower obstacle must remain clear"
        );
    }

    #[test]
    fn no_player_camp_initializes_to_unseen_instead_of_exposing_the_map() {
        let mut engine = EngineInner::new();
        engine.control.sim_config.fog_of_war = true;
        engine.set_level_size(192.0, 144.0);
        engine.refresh_fog_of_war(&LevelAssets::default(), true);

        assert!(engine.fog_of_war().is_initialized());
        assert_eq!(
            engine.fog_cell_state(MapPoint::new(50.0, 50.0)),
            FogCellState::Unexplored
        );
    }

    #[test]
    fn visible_polygon_is_centered_on_the_pc_and_uses_configured_range() {
        let mut engine = EngineInner::new();
        engine.control.sim_config.fog_of_war = true;
        engine.set_level_size(1_000.0, 1_000.0);
        engine.ai.standard_view_polygon_radius = 100;
        engine.add_entity(pc_at(492.0, 492.0));
        engine.refresh_fog_of_war(&LevelAssets::default(), true);

        for point in [
            MapPoint::new(396.0, 492.0),
            MapPoint::new(588.0, 492.0),
            MapPoint::new(492.0, 396.0),
            MapPoint::new(492.0, 588.0),
        ] {
            assert_eq!(
                engine.fog_cell_state(point),
                FogCellState::Visible,
                "the reveal must be circular and symmetric around the PC"
            );
        }
        assert_eq!(
            engine.fog_cell_state(MapPoint::new(684.0, 492.0)),
            FogCellState::Unexplored,
            "absolute PC elevation must not turn the bounded fog radius into unlimited sight"
        );
    }

    #[test]
    fn pc_fog_range_is_not_reduced_by_blip_discovery_difficulty() {
        let mut engine = EngineInner::new();
        engine.control.sim_config.fog_of_war = true;
        engine.control.sim_config.difficulty = crate::player_profile::DifficultyLevel::Legendary;
        engine.set_level_size(1_200.0, 1_200.0);
        engine.ai.standard_view_polygon_radius = 400;
        engine.add_entity(pc_at(600.0, 600.0));
        engine.refresh_fog_of_war(&LevelAssets::default(), true);

        assert_eq!(
            engine.fog_cell_state(MapPoint::new(1_050.0, 600.0)),
            FogCellState::Visible,
            "1.2x the 400-unit standard radius must reach 450 units even though Legendary blip discovery is 40%"
        );
    }

    #[test]
    fn ladder_vision_uses_a_horizontal_slice_instead_of_the_vertical_move_plane() {
        let mut engine = EngineInner::new();
        engine.control.sim_config.fog_of_war = true;
        engine.set_level_size(1_000.0, 1_000.0);
        let pc = engine.add_entity(pc_at(500.0, 500.0));
        let entity = engine.get_entity_mut(pc).expect("ladder PC fixture");
        entity.element_data_mut().posture = Posture::OnLadder;
        entity
            .position_iface_mut()
            .set_position(WorldPoint3D::new(500.0, 700.0, 200.0));
        entity.position_iface_mut().set_obstacle(
            None,
            Some(crate::position_interface::PlaneZCoeffs {
                az: -1.2,
                bz: -5.0,
                dz: 10_000.0,
            }),
        );
        let expected_height = entity.element_data().position().z;

        let source = engine
            .fog_vision_sources()
            .into_iter()
            .next()
            .expect("ladder vision source");
        assert_eq!(
            source.plane,
            Some(crate::position_interface::PlaneZCoeffs {
                az: 0.0,
                bz: 0.0,
                dz: expected_height,
            })
        );
    }

    #[test]
    fn camp_classification_requires_a_live_playable_pc_anchor() {
        let mut engine = EngineInner::new();
        let soldier_id = engine.add_entity(soldier_at(50.0, 50.0, Camp::Royalists));
        let soldier = engine.get_entity(soldier_id).expect("soldier fixture");
        let camps = engine.player_camps();
        assert!(camps.is_empty());
        assert!(!engine.is_allied_to_player(soldier, &camps));
        assert!(!engine.is_hostile_to_player(soldier, &camps));
    }

    #[test]
    fn invalid_and_neutral_camps_are_not_misclassified() {
        let mut engine = EngineInner::new();
        engine.add_entity(pc_at(10.0, 10.0));
        engine.mission_domain.diplomacy.set_enabled(true);
        engine
            .mission_domain
            .diplomacy
            .set_relationship(
                Camp::Royalists,
                Camp::Custom(2),
                crate::diplomacy::Relationship::Neutral,
            )
            .expect("valid neutral relationship");
        let neutral_id = engine.add_entity(soldier_at(20.0, 20.0, Camp::Custom(2)));
        let invalid_id = engine.add_entity(soldier_at(30.0, 30.0, Camp::Error));
        let camps = engine.player_camps();

        for id in [neutral_id, invalid_id] {
            let entity = engine.get_entity(id).expect("classification fixture");
            assert!(!engine.is_allied_to_player(entity, &camps));
            assert!(!engine.is_hostile_to_player(entity, &camps));
        }
    }

    #[test]
    fn allied_npcs_do_not_reveal_fog() {
        let mut engine = EngineInner::new();
        engine.control.sim_config.fog_of_war = true;
        engine.set_level_size(1_400.0, 400.0);
        engine.ai.standard_view_polygon_radius = 100;
        engine.add_entity(pc_at(100.0, 100.0));
        engine.mission_domain.diplomacy.set_enabled(true);
        engine
            .mission_domain
            .diplomacy
            .set_relationship(
                Camp::Royalists,
                Camp::Custom(2),
                crate::diplomacy::Relationship::Allied,
            )
            .expect("valid allied relationship");
        engine.add_entity(soldier_at(900.0, 100.0, Camp::Custom(2)));
        let enemy = engine.add_entity(soldier_at(1_000.0, 100.0, Camp::Lacklandists));
        engine.refresh_fog_of_war(&LevelAssets::default(), true);
        assert!(!engine.fog_entity_visible(enemy));
        assert_eq!(
            engine.fog_cell_state(MapPoint::new(900.0, 100.0)),
            FogCellState::Unexplored,
            "an allied mission NPC must not create a reveal circle"
        );
    }

    #[test]
    fn zero_sized_pre_level_state_keeps_fog_dormant() {
        let mut engine = EngineInner::new();
        engine.control.sim_config.fog_of_war = true;
        engine.set_level_size(0.0, 0.0);

        assert!(engine.control.sim_config.fog_of_war);
        assert!(!engine.fog_of_war_enabled());
        assert!(!engine.fog_of_war().is_initialized());
    }

    #[test]
    fn fog_setting_is_host_owned_and_original_parity_cannot_enable_it() {
        let mut engine = EngineInner::new();
        engine.set_level_size(192.0, 144.0);
        engine.control.sim_config.fog_of_war = false;
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::default();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        engine.apply_commands(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &[crate::player_command::PlayerInput::new(
                crate::player_command::PlayerId(1),
                crate::player_command::PlayerCommand::SetFogOfWar { enabled: true },
            )],
        );
        assert!(!engine.control.sim_config.fog_of_war);

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &crate::player_command::PlayerCommand::SetFogOfWar { enabled: true },
        );
        assert!(engine.control.sim_config.fog_of_war);

        engine.control.sim_config.fog_of_war = false;
        engine.control.rng = SimulationRng::with_original_replay(Vec::new());
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &crate::player_command::PlayerCommand::SetFogOfWar { enabled: true },
        );
        assert!(!engine.control.sim_config.fog_of_war);
    }

    #[test]
    fn disabling_fog_restores_unfiltered_original_visibility() {
        let (mut engine, pc, enemy) = pc_vision_fixture();
        engine
            .get_entity_mut(pc)
            .expect("PC observer")
            .element_data_mut()
            .active = false;
        engine.control.frame_counter = SPOTTED_HYSTERESIS_FRAMES + 1;
        engine.refresh_fog_of_war(&LevelAssets::default(), true);
        assert!(!engine.fog_entity_visible(enemy));

        engine.control.sim_config.fog_of_war = false;
        assert!(!engine.fog_of_war_enabled());
        assert!(engine.fog_entity_visible(enemy));
        assert_eq!(
            engine.fog_cell_state(MapPoint::new(1_000.0, 100.0)),
            FogCellState::Visible
        );
    }

    #[test]
    fn pc_sight_and_hidden_enemy_leave_stationary_intelligence() {
        let (mut engine, pc, enemy) = pc_vision_fixture();
        assert!(engine.fog_entity_visible(enemy));

        engine
            .get_entity_mut(pc)
            .expect("PC observer")
            .element_data_mut()
            .active = false;
        engine.control.frame_counter = SPOTTED_HYSTERESIS_FRAMES + 1;
        engine.refresh_fog_of_war(&LevelAssets::default(), true);

        assert!(!engine.fog_entity_visible(enemy));
        let marker = engine
            .fog_of_war()
            .last_known_markers(engine.control.frame_counter)
            .find(|marker| marker.entity_id == enemy)
            .expect("hidden enemy retains fading last-known intelligence");
        assert_eq!(marker.position, MapPoint::new(1_000.0, 100.0));

        engine
            .get_entity_mut(enemy)
            .expect("hidden enemy")
            .element_data_mut()
            .set_position(WorldPoint3D::new(1_200.0, 100.0, 0.0));
        let marker_after_move = engine
            .fog_of_war()
            .last_known_markers(engine.control.frame_counter)
            .find(|marker| marker.entity_id == enemy)
            .expect("last-known marker remains while enemy moves unseen");
        assert_eq!(marker_after_move.position, marker.position);
    }

    #[test]
    fn opaque_authored_geometry_blocks_pc_vision() {
        let (mut engine, _, enemy) = pc_vision_fixture();
        let mut assets = LevelAssets::default();
        let wall = opaque_wall_between_pc_and_enemy();
        let wall_bbox = wall.box_ground;
        assets.static_sight_obstacles = std::sync::Arc::new(vec![wall]);
        engine.world.static_sight_obstacle_active = vec![true];
        let grid = std::sync::Arc::make_mut(&mut engine.world.fast_grid);
        grid.size_map(32, 16);
        grid.allocate_layers(1);
        grid.add_obstacle_index(
            SightObstacleIndex::new(0).expect("fixture obstacle index"),
            None,
            &wall_bbox,
        );

        // Clear the fixture's prior unobstructed sighting and advance beyond
        // its hysteresis before rescanning against the wall.
        engine.control.frame_counter = SPOTTED_HYSTERESIS_FRAMES + 1;
        engine.refresh_fog_of_war(&assets, true);

        assert!(!engine.fog_entity_visible(enemy));
        assert_eq!(
            engine.fog_cell_state(MapPoint::new(1_000.0, 100.0)),
            FogCellState::Explored,
            "the previously seen position remains explored while its hostile is hidden"
        );
    }

    #[test]
    fn visible_wall_facade_does_not_reveal_the_ground_or_actors_behind_it() {
        let mut engine = EngineInner::new();
        engine.control.sim_config.fog_of_war = true;
        engine.set_level_size(600.0, 600.0);
        engine.ai.standard_view_polygon_radius = 250;
        engine.add_entity(pc_at(100.0, 300.0));

        // A north-facing wall: its front artwork projects from ground Y 210
        // upward to map Y 110, directly over the ground shadow behind it.
        let mut wall = SightObstacle::new_default(1);
        wall.obstacle_points = vec![
            ObstaclePoint {
                x: 50.0,
                y: 200.0,
                z_bottom: 0.0,
                z_top: 100.0,
            },
            ObstaclePoint {
                x: 150.0,
                y: 200.0,
                z_bottom: 0.0,
                z_top: 100.0,
            },
            ObstaclePoint {
                x: 150.0,
                y: 210.0,
                z_bottom: 0.0,
                z_top: 100.0,
            },
            ObstaclePoint {
                x: 50.0,
                y: 210.0,
                z_bottom: 0.0,
                z_top: 100.0,
            },
        ];
        wall.top_plane_points = [
            [50.0, 200.0, 100.0],
            [150.0, 200.0, 100.0],
            [150.0, 210.0, 100.0],
        ];
        wall.bottom_plane_points = [[50.0, 200.0, 0.0], [150.0, 200.0, 0.0], [150.0, 210.0, 0.0]];
        wall.rebuild_geometry();
        let wall_bbox = wall.box_ground;
        let mut assets = LevelAssets::default();
        assets.static_sight_obstacles = std::sync::Arc::new(vec![wall]);
        engine.world.static_sight_obstacle_active = vec![true];
        let grid = std::sync::Arc::make_mut(&mut engine.world.fast_grid);
        grid.size_map(32, 32);
        grid.allocate_layers(1);
        grid.add_obstacle_index(
            SightObstacleIndex::new(0).expect("fixture obstacle index"),
            None,
            &wall_bbox,
        );

        engine.refresh_fog_of_war(&assets, true);

        let facade_pixel = MapPoint::new(100.0, 160.0);
        assert_eq!(
            engine.fog_cell_state(facade_pixel),
            FogCellState::Unexplored,
            "the ground and any actor behind the wall must stay hidden"
        );
        assert!(
            engine
                .fog_of_war()
                .visible_projection_region()
                .contains(facade_pixel),
            "the directly visible wall artwork must remain clear"
        );
        assert!(
            !engine
                .fog_of_war()
                .visible_projection_region()
                .contains(MapPoint::new(100.0, 105.0)),
            "a roof reached only through the nearer wall must remain hidden from an eye below it"
        );
    }

    #[test]
    fn ground_seen_under_bridge_does_not_clear_bridge_artwork() {
        let mut engine = EngineInner::new();
        engine.control.sim_config.fog_of_war = true;
        engine.set_level_size(600.0, 600.0);
        engine.ai.standard_view_polygon_radius = 250;
        engine.add_entity(pc_at(100.0, 300.0));

        // The camera sees an overhead slab at this pixel, while Robin can
        // see the ground underneath and beyond it through the open arch.
        let mut bridge = SightObstacle::new_default(1);
        bridge.obstacle_points = [(50.0, 200.0), (150.0, 200.0), (150.0, 220.0), (50.0, 220.0)]
            .into_iter()
            .map(|(x, y)| ObstaclePoint {
                x,
                y,
                z_bottom: 100.0,
                z_top: 120.0,
            })
            .collect();
        bridge.top_plane_points = [
            [50.0, 200.0, 120.0],
            [150.0, 200.0, 120.0],
            [150.0, 220.0, 120.0],
        ];
        bridge.bottom_plane_points = [
            [50.0, 200.0, 100.0],
            [150.0, 200.0, 100.0],
            [150.0, 220.0, 100.0],
        ];
        bridge.rebuild_geometry();
        let bbox = bridge.box_ground;
        let mut assets = LevelAssets::default();
        assets.static_sight_obstacles = std::sync::Arc::new(vec![bridge]);
        engine.world.static_sight_obstacle_active = vec![true];
        let grid = std::sync::Arc::make_mut(&mut engine.world.fast_grid);
        grid.size_map(32, 32);
        grid.allocate_layers(1);
        grid.add_obstacle_index(SightObstacleIndex::new(0).unwrap(), None, &bbox);

        let pixel = MapPoint::new(100.0, 90.0);
        for force in [true, false] {
            engine.refresh_fog_of_war(&assets, force);
            let fog = engine.fog_of_war();
            let cached = &fog.scan_cache.entries[0];
            assert!(
                cached
                    .prepared_surfaces
                    .coverage
                    .contains(&Point::new(pixel.x, pixel.y)),
                "bridge must own the test pixel; candidates {:?}, coverage {:?}",
                cached.prepared_surfaces.candidate_indices,
                cached.prepared_surfaces.coverage
            );
            assert!(
                !cached.visible_projection.contains(pixel),
                "computed camera mask must hide the top; eye {:?}",
                engine.fog_vision_sources()[0].eye
            );
            assert_eq!(
                fog.cell_state(pixel),
                FogCellState::Visible,
                "ground through the arch"
            );
            assert!(
                !fog.visible_projection_region().contains(pixel),
                "unseen bridge top must occlude visible ground"
            );
            assert!(
                !fog.explored_projection_region().contains(pixel),
                "occluded artwork must not become explored"
            );
        }
    }

    #[test]
    fn large_mesh_surface_is_clipped_by_exact_view_distance() {
        let mut wall = SightObstacle::new_default(1);
        wall.obstacle_points = vec![
            ObstaclePoint {
                x: 0.0,
                y: 0.0,
                z_bottom: 0.0,
                z_top: 100.0,
            },
            ObstaclePoint {
                x: 1_000.0,
                y: 0.0,
                z_bottom: 0.0,
                z_top: 100.0,
            },
            ObstaclePoint {
                x: 1_000.0,
                y: 500.0,
                z_bottom: 0.0,
                z_top: 100.0,
            },
            ObstaclePoint {
                x: 0.0,
                y: 500.0,
                z_bottom: 0.0,
                z_top: 100.0,
            },
        ];
        wall.rebuild_geometry();
        let source = VisionSource {
            eye: WorldPoint3D::new(50.0, 600.0, CHARACTER_HEIGHT),
            map_position: MapPoint::new(50.0, 600.0),
            plane: None,
            eye_height: CHARACTER_HEIGHT,
            standard_radius: 1_000.0,
            super_factor: 1.0,
        };
        let visible_ground = MultiPolygon::new(vec![Polygon::new(
            closed_line_string([
                Coord { x: -10.0, y: 490.0 },
                Coord { x: 110.0, y: 490.0 },
                Coord { x: 110.0, y: 620.0 },
                Coord { x: -10.0, y: 620.0 },
            ]),
            Vec::new(),
        )]);
        let obstacle_index = SightObstacleIndex::new(0).expect("fixture obstacle index");
        let obstacle_storage = [wall];
        let obstacles =
            crate::sight_obstacle::ObstacleList::from_slice_all_active(&obstacle_storage);
        let prepared = prepare_camera_surfaces(obstacles, &[obstacle_index]);
        let map_domains = fog_map_domains(
            source.map_position,
            180.0,
            crate::coordinates::MapSize::new(1_200.0, 800.0),
        );
        let visible =
            visible_camera_surfaces(&source, obstacles, &prepared, &visible_ground, &map_domains);

        assert!(
            visible.contains(&Point::new(50.0, 450.0)),
            "the visible portion of the wall face must remain clear"
        );
        assert!(
            !visible.contains(&Point::new(500.0, 450.0)),
            "one nearby mesh portion must not reveal a distant portion of the same face"
        );
        assert!(
            !visible.contains(&Point::new(50.0, 350.0)),
            "the authored top must obey the same exact view-distance boundary as ground"
        );
    }

    #[test]
    fn hostile_in_the_visible_fog_region_is_rendered() {
        let (mut engine, pc, enemy) = pc_vision_fixture();
        engine
            .get_entity_mut(pc)
            .expect("PC observer")
            .element_data_mut()
            .active = false;
        engine.control.frame_counter = SPOTTED_HYSTERESIS_FRAMES + 1;
        engine.refresh_fog_of_war(&LevelAssets::default(), true);
        assert!(!engine.fog_entity_visible(enemy));

        let enemy_position = engine
            .get_entity(enemy)
            .expect("hostile fixture")
            .element_data()
            .position_map();
        assert!(
            engine
                .players
                .fog_of_war
                .mark_position_visible(enemy_position)
        );
        assert!(
            engine.fog_entity_visible(enemy),
            "a hostile must not disappear over ground the fog renders as currently visible"
        );
    }

    #[test]
    fn elevated_pc_tests_walls_in_world_space_instead_of_projected_map_space() {
        let (mut engine, pc, enemy) = pc_vision_fixture();
        engine
            .get_entity_mut(enemy)
            .expect("fixture enemy")
            .element_data_mut()
            .active = false;
        engine
            .get_entity_mut(pc)
            .expect("PC observer")
            .position_iface_mut()
            .set_obstacle(
                None,
                Some(crate::position_interface::PlaneZCoeffs {
                    az: 0.0,
                    bz: 0.0,
                    dz: 220.0,
                }),
            );
        engine.players.fog_of_war = FogOfWarState::default();

        let mut wall = opaque_wall_between_pc_and_enemy();
        wall.translate_2d(0.0, 220.0);
        for point in &mut wall.obstacle_points {
            point.z_bottom += 220.0;
            point.z_top += 220.0;
        }
        for point in &mut wall.top_plane_points {
            point[2] += 220.0;
        }
        for point in &mut wall.bottom_plane_points {
            point[2] += 220.0;
        }
        wall.rebuild_geometry();
        let wall_bbox = wall.box_ground;
        let mut assets = LevelAssets::default();
        assets.static_sight_obstacles = std::sync::Arc::new(vec![wall]);
        engine.world.static_sight_obstacle_active = vec![true];
        let grid = std::sync::Arc::make_mut(&mut engine.world.fast_grid);
        grid.size_map(32, 16);
        grid.allocate_layers(1);
        grid.add_obstacle_index(
            SightObstacleIndex::new(0).expect("fixture obstacle index"),
            None,
            &wall_bbox,
        );

        engine.refresh_fog_of_war(&assets, true);

        assert_eq!(
            engine.fog_cell_state(MapPoint::new(1_000.0, 100.0)),
            FogCellState::Unexplored,
            "an elevated wall north of the PC must block the reconstructed 3D fog ray"
        );
    }

    #[test]
    fn identical_visibility_scans_produce_identical_serialized_state_and_hash() {
        let (first, _, _) = pc_vision_fixture();
        let (second, _, _) = pc_vision_fixture();

        assert_eq!(first.fog_of_war(), second.fog_of_war());
        assert_eq!(
            bitcode::encode(first.fog_of_war()),
            bitcode::encode(second.fog_of_war())
        );
        assert_eq!(
            crate::replay::state_hash(&first),
            crate::replay::state_hash(&second)
        );
    }

    #[test]
    fn listen_reveals_world_position_temporarily_without_touching_blip_identity() {
        let (mut engine, pc, enemy) = pc_vision_fixture();
        engine
            .get_entity_mut(pc)
            .expect("PC observer")
            .element_data_mut()
            .active = false;
        engine.control.frame_counter = SPOTTED_HYSTERESIS_FRAMES + 1;
        engine.refresh_fog_of_war(&LevelAssets::default(), true);
        assert!(!engine.fog_entity_visible(enemy));

        let was_blipped = engine
            .get_entity(enemy)
            .expect("listen target")
            .element_data()
            .blipped;
        engine.reveal_entity_from_listen(enemy);
        assert!(engine.fog_entity_visible(enemy));
        assert_eq!(
            engine.fog_cell_state(MapPoint::new(1_000.0, 100.0)),
            FogCellState::Visible
        );
        assert_eq!(
            engine
                .get_entity(enemy)
                .expect("listen target after reveal")
                .element_data()
                .blipped,
            was_blipped,
            "temporary intelligence must not mutate Original blip identity"
        );

        engine.control.frame_counter += LISTEN_REVEAL_FRAMES;
        engine.refresh_fog_of_war(&LevelAssets::default(), true);
        assert!(!engine.fog_entity_visible(enemy));
    }

    #[test]
    fn listen_temporarily_reveals_a_nonhuman_object_without_a_last_known_marker() {
        let (mut engine, _, _) = pc_vision_fixture();
        let object = engine.add_entity(bonus_at(1_300.0, 100.0));
        engine.refresh_fog_of_war(&LevelAssets::default(), false);
        assert!(!engine.fog_entity_visible(object));

        engine.reveal_entity_from_listen(object);
        assert!(engine.fog_entity_visible(object));
        assert!(
            engine
                .fog_of_war()
                .last_known_markers(engine.control.frame_counter)
                .all(|marker| marker.entity_id != object)
        );

        engine.control.frame_counter += LISTEN_REVEAL_FRAMES;
        engine.refresh_fog_of_war(&LevelAssets::default(), false);
        assert!(!engine.fog_entity_visible(object));
    }
}
