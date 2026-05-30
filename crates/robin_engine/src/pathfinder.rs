//! A* pathfinder operating on a pre-computed waypoint graph.
//!
//! # Architecture
//!
//! The pathfinder uses a **waypoint graph** (not a grid-based A*). The graph
//! is pre-computed from obstacle geometry and loaded from the level's proto
//! stream. Nodes are placed at obstacle corners; links connect nodes across
//! different obstacles.
//!
//! ## Hierarchy
//!
//! The graph is organized as: **Layers → Areas → Obstacles → Nodes**.
//! - **Layer**: a height level in the game world.
//! - **Area**: a connected walkable region within a layer.
//! - **Obstacle**: a collision shape within an area.
//! - **Node**: a corner point of an obstacle.
//!
//! ## Docking places
//!
//! Each node has four potential "docking places" (corners of the unit's
//! bounding box relative to the obstacle corner): `TOP_LEFT`, `TOP_RIGHT`,
//! `BOTTOM_RIGHT`, `BOTTOM_LEFT`. Which docking places are valid depends
//! on the unit's size (half-diagonal index) and the local obstacle geometry.
//!
//! ## State management
//!
//! Obstacles can be toggled on/off (e.g. a drawbridge opening). When the
//! state changes, nodes and links are moved between the active graph and
//! an "alternative" graph.
//!
//! ## Threading
//!
//! Provides a synchronous `find_path()` API. Threading can be layered
//! on top via `std::thread` or `tokio::spawn_blocking`.

use serde::{Deserialize, Serialize};

use crate::coordinates::MapPoint;
use crate::element::EntityId;
use crate::fast_find_grid::FastFindGrid;
use crate::geo2d::{self, BBox2D, GeoPoint2D, Vec2D, pt};
use robin_util::static_arc::StaticArc;

// ─── Geometry helpers ────────────────────────────────────────────

/// Ray-casting point-in-polygon test.
/// Returns `true` if `pt` is inside the polygon defined by `vertices`.
fn point_in_polygon(pt: GeoPoint2D, vertices: &[GeoPoint2D]) -> bool {
    if vertices.len() < 3 {
        return false;
    }
    let mut inside = false;
    let n = vertices.len();
    let mut j = n - 1;
    for i in 0..n {
        let vi = vertices[i];
        let vj = vertices[j];
        if (vi.y > pt.y) != (vj.y > pt.y) {
            let x_intersect = (vj.x - vi.x) * (pt.y - vi.y) / (vj.y - vi.y) + vi.x;
            if pt.x < x_intersect {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

// ─── Docking place bit flags ─────────────────────────────────────

/// Docking places around a graph node (bit flags).
/// These represent the four corners of a unit's bounding box
/// relative to an obstacle corner.
pub const TOP_LEFT: u8 = 1;
pub const TOP_RIGHT: u8 = 2;
pub const BOTTOM_RIGHT: u8 = 4;
pub const BOTTOM_LEFT: u8 = 8;

/// Count the number of set docking places in a bitmask.
#[inline]
pub fn number_of_places(places: u8) -> u8 {
    (places & 0x0F).count_ones() as u8
}

/// Get the next docking place in clockwise (direct=true) or
/// counter-clockwise (direct=false) order.
#[inline]
pub fn next_docking_place(place: u8, direct: bool) -> u8 {
    if direct {
        if place == BOTTOM_LEFT {
            TOP_LEFT
        } else {
            place << 1
        }
    } else if place == TOP_LEFT {
        BOTTOM_LEFT
    } else {
        place >> 1
    }
}

// ─── Enums ───────────────────────────────────────────────────────

/// Pathfinder thread status.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum PathFinderStatus {
    #[default]
    Off,
    Waiting,
    NewRequest,
    Ready,
    Busy,
    Sleep,
}

/// Pathfinder speed / priority.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum PathFinderSpeed {
    Fast = 0,
    Medium = 1,
    #[default]
    Slow = 2,
    VerySlow = 3,
}

// The `PathRequest` struct + its backing queue were deleted during
// the order-queue refactor.  Movement launches now call `find_path`
// synchronously and build orders directly on the Move sequence
// element; the request-object plumbing it replaced had no remaining
// producers.

// ─── Index newtypes ──────────────────────────────────────────────

/// Index into `PathGraph::nodes`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct NodeIdx(pub u32);

/// Index into `PathGraph::links`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct LinkIdx(pub u32);

/// Index into `PathGraph::link_configs`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
)]
pub struct ConfigIdx(pub u32);

// ─── Graph data structures ───────────────────────────────────────

/// Configuration of a link for a specific unit size.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PathGraphLinkConfig {
    /// Combined bitmask of valid start docking places.
    pub start_configurations: u8,
    /// Combined bitmask of valid destination docking places.
    pub destination_configurations: u8,
    /// Per-connection start docking place.
    pub start_config_list: Vec<u8>,
    /// Per-connection destination docking place.
    pub destination_config_list: Vec<u8>,
}

/// A link between two nodes in the pathfinding graph.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PathGraphLink {
    /// Node at the "next" end of this link.
    pub next_node: NodeIdx,
    /// Node at the "previous" end of this link.
    pub prev_node: NodeIdx,
    /// Euclidean distance along this link.
    pub distance: f32,
    /// Bitmask: which obstacle state is required for this link to be active.
    pub required_state: u32,
    /// Per half-diagonal configuration. `None` means invalid for that size.
    pub config_indices: Vec<Option<ConfigIdx>>,
}

/// A node in the pathfinding graph (placed at an obstacle corner).
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PathGraphNode {
    /// World position of this node.
    pub position: GeoPoint2D,

    /// Vector from the obstacle point *before* this node to this node.
    pub vector_to_node: Vec2D,
    /// Vector from this node to the obstacle point *after* this node.
    pub vector_from_node: Vec2D,

    /// Bitmask: which obstacle state is required for this node to be active.
    pub required_state: u32,

    /// Per half-diagonal: bitmask of valid docking places.
    pub configurations: Vec<u8>,

    /// Indices of active links from this node.
    pub link_indices: Vec<LinkIdx>,
    /// Indices of inactive (alternative-state) links from this node.
    pub alternative_link_indices: Vec<LinkIdx>,

    // ── A* working state (reset per search) ──
    /// Whether this node has been visited in the current search.
    pub visited: bool,
    /// Best known distance from source.
    pub distance_from_source: f32,
    /// Estimated distance to goal (heuristic).
    pub distance_to_goal: f32,
    /// f-score = distance_from_source + distance_to_goal.
    pub score: f32,
    /// Link taken to reach this node on the best known path.
    pub previous_link_on_path: Option<LinkIdx>,
    /// Docking places for leaving this node toward the goal.
    pub leave_place: u8,
    /// Docking places for entering this node from the source.
    pub enter_place: u8,
}

/// One motion obstacle within an area — a polygon that blocks movement
/// when the path-graph state has its bit set.
///
/// The `active` flag swaps between active and alternative as
/// `set_state_area` is called; the polygon itself is fixed.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct MotionObstacle {
    /// Required-state bits. The obstacle is active when
    /// `(state_id & current_state) == state_id`.
    pub state_id: u32,
    /// Current active flag — `true` means the obstacle is blocking
    /// movement.
    pub active: bool,
    /// Axis-aligned bounding box of `polygon` for fast intersection.
    pub bounding_box: crate::geo2d::BBox2D,
    /// Polygon vertices defining the obstacle footprint.
    pub polygon: Vec<GeoPoint2D>,
    /// Grid-line indices emitted from this obstacle's perimeter when
    /// the level was loaded. We store the mapping up front so state
    /// transitions can flip the grid's `line_active` flags without a
    /// linear scan over every motion line in the layer.
    pub grid_line_indices: Vec<crate::fast_find_grid::LineIndex>,
}

/// Output payload for `set_state_area_with_appeared` / `toggle_obstacle_state`:
/// a motion obstacle that just transitioned inactive → active, carrying both
/// its axis-aligned bounding box and its polygon vertices.
///
/// Used by the pathfinder-state-change handler to kill actors whose
/// move-box overlaps the newly-active obstacle. The bbox is the cheap
/// pre-filter; the polygon narrows the kill decision to actors whose
/// move box actually overlaps the obstacle footprint (not just its
/// aabb).
#[derive(Debug, Clone)]
pub struct AppearedObstacle {
    /// Bounding box of the obstacle polygon (cheap pre-filter).
    pub bounding_box: crate::geo2d::BBox2D,
    /// Polygon vertices of the obstacle footprint.
    pub polygon: Vec<GeoPoint2D>,
}

/// A motion area within a layer — the walkable polygon and its skeleton.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct MotionArea {
    /// Skeleton segments for fast line-of-sight checks (`is_reachable_fast`).
    pub skeleton: Vec<geo::Line<f32>>,

    /// Polygon vertices defining this motion area's walkable boundary.
    /// Used for point-in-polygon hit-testing (sector hit-test) to
    /// determine which motion area a click falls in.
    pub polygon: Vec<GeoPoint2D>,

    /// Motion obstacles within this area. The total length is fixed at
    /// level load; state transitions flip the `active` flag per entry.
    ///
    /// Stored as a single list with an in-place `active` flag (rather
    /// than two parallel active/alternative lists) so the
    /// sector-conversion and cumulative-count bookkeeping
    /// (`find_sector_at_point`, `build_sector_conversion`) stays stable
    /// across state changes.
    pub motion_obstacles: Vec<MotionObstacle>,
}

/// Sector-to-area conversion entry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SectorToArea {
    pub sector: u16,
    pub area: u16,
}

// ─── Path graph ──────────────────────────────────────────────────

/// Pure-static portion of the pathfinding graph: links, link configs,
/// motion-area polygons / skeletons, half-diagonals, sector→area
/// conversion. All loaded once at level start and never mutated.
/// Wrapped in `StaticArc` on `PathGraph::static_data` so `EngineInner::clone`
/// (per-frame rollback) is a refcount bump and state hashing does not walk
/// load-time geometry.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PathGraphStatic {
    /// All links in the graph.
    pub links: Vec<PathGraphLink>,
    /// All link configurations.
    pub link_configs: Vec<PathGraphLinkConfig>,
    /// Motion areas: `move_layers[layer][area]`.
    pub move_layers: Vec<Vec<MotionArea>>,
    /// Alternative motion areas.
    pub alternative_move_layers: Vec<Vec<MotionArea>>,
    /// Half-diagonal vectors for each unit size.
    pub half_diagonals: Vec<Vec2D>,
    /// Sector-to-area conversion table.
    pub sector_conversion: Vec<SectorToArea>,
}

/// The complete pathfinding graph for all layers.
///
/// Owns all nodes, links, and configurations in flat arena-style vectors.
/// The hierarchy is represented by nested `Vec`s of `NodeIdx`.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PathGraph {
    /// Pure-static load-time data, shared via `StaticArc` so
    /// `EngineInner::clone` is a refcount bump and rollback hashing skips
    /// the immutable geometry.
    pub static_data: StaticArc<PathGraphStatic>,

    /// All nodes in the graph.
    pub nodes: Vec<PathGraphNode>,

    /// Hierarchy: `layers[layer][area][obstacle]` → `Vec<NodeIdx>`.
    pub layers: Vec<Vec<Vec<Vec<NodeIdx>>>>,
    /// Alternative (inactive) hierarchy (same structure).
    pub alternative_layers: Vec<Vec<Vec<Vec<NodeIdx>>>>,

    /// State bitmask per area: `states[layer][area]`.
    pub states: Vec<Vec<u32>>,
}

impl Default for PathGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl PathGraph {
    pub fn new() -> Self {
        Self {
            static_data: StaticArc::new(PathGraphStatic::default()),
            nodes: Vec::new(),
            layers: Vec::new(),
            alternative_layers: Vec::new(),
            states: Vec::new(),
        }
    }

    /// Mutable access to the static-data Arc via `Arc::make_mut`.
    /// Use only during level loading / builder code that's known to
    /// hold the only reference.
    #[inline]
    pub fn static_mut(&mut self) -> &mut PathGraphStatic {
        StaticArc::make_mut(&mut self.static_data)
    }

    /// Convert a sector number to an area index.
    pub fn convert_sector(&self, sector: u16) -> u16 {
        for entry in &self.static_data.sector_conversion {
            if entry.sector == sector {
                return entry.area;
            }
        }
        panic!(
            "ConvertSector failed: sector {} not found in conversion table",
            sector
        );
    }

    /// Try to convert a sector number to an area index.
    /// Returns `None` if the sector isn't in the conversion table
    /// (e.g. entity in a building or on a special layer).
    pub fn try_convert_sector(&self, sector: u16) -> Option<u16> {
        self.static_data
            .sector_conversion
            .iter()
            .find(|e| e.sector == sector)
            .map(|e| e.area)
    }

    /// Get the number of half-diagonals (unit sizes).
    pub fn num_half_diagonals(&self) -> usize {
        self.static_data.half_diagonals.len()
    }

    /// Find the sector number for a point on a given layer.
    ///
    /// Returns `Some(sector_number)` if the point is inside a motion
    /// area polygon. The sector number, when passed to `convert_sector`,
    /// returns the area index for the given (layer, point).
    ///
    /// The sector_conversion table is built sequentially across layers
    /// and areas, with each area consuming `num_obstacles + 1` sector
    /// numbers (one for the polygon itself, one per obstacle).
    pub fn find_sector_at_point(&self, layer: usize, point: MapPoint) -> Option<u16> {
        let area_idx = self.find_area_at_point(layer, point)?;
        // Cumulative sector numbers contributed by all layers BEFORE the target.
        let mut cumulative: u16 = 0;
        for (l, layer_areas) in self.static_data.move_layers.iter().enumerate() {
            if l == layer {
                // Within this layer, walk to the target area, summing contributions.
                for (a, area) in layer_areas.iter().enumerate() {
                    if a == area_idx {
                        return Some(cumulative);
                    }
                    cumulative += area.motion_obstacles.len() as u16 + 1;
                }
                return None;
            }
            for area in layer_areas {
                cumulative += area.motion_obstacles.len() as u16 + 1;
            }
        }
        None
    }

    /// Find which motion area on a given layer contains `point`.
    ///
    /// Returns `Some(area_index)` if the point is inside a motion area
    /// polygon, `None` if it's outside all of them (e.g. in a moat or
    /// on a non-walkable surface).
    ///
    /// Uses the standard ray-casting point-in-polygon test on each
    /// area's stored polygon.
    pub fn find_area_at_point(&self, layer: usize, point: MapPoint) -> Option<usize> {
        let point = point.to_geo();
        let areas = self.static_data.move_layers.get(layer)?;
        for (area_idx, area) in areas.iter().enumerate() {
            if point_in_polygon(point, &area.polygon) {
                return Some(area_idx);
            }
        }
        None
    }

    /// Get all active node indices in an area.
    pub fn area_node_indices(
        &self,
        layer: usize,
        area: usize,
    ) -> impl Iterator<Item = NodeIdx> + '_ {
        self.layers[layer][area]
            .iter()
            .flat_map(|obstacle| obstacle.iter().copied())
    }

    /// Read just the move-box half-diagonal table from a proto stream
    /// into `grid` and this graph's static copy.  Returns the byte
    /// offset at which the half-diagonal section ends, so a later
    /// full load can fast-forward past it.
    ///
    /// The pathfinder's full load (`load_from_proto_stream`) runs after
    /// the background bitmap decodes because it registers grid sectors
    /// that need the level bbox.  Soldier spawn, however, needs the
    /// half-diagonal table up front to build `move_box` from the
    /// soldier profile's `pathfinder_index` — without it every NPC
    /// falls back to a `(-1,-1,1,1)` box and `TestIfPathIsFine` /
    /// anti-collision break.  This prepass reads only the first
    /// section of the proto stream (leading `u16` count + `count` ×
    /// `(f32, f32)`) so the tables are populated before any actor
    /// spawns.
    pub fn preload_half_diagonals_from_proto(
        &mut self,
        grid: &mut FastFindGrid,
        data: &[u8],
    ) -> Result<usize, String> {
        if data.len() < 2 {
            return Err("proto stream shorter than u16 count".into());
        }
        // Idempotent: calling twice (prepass + full load) is safe.
        if !self.static_data.half_diagonals.is_empty() {
            let count = u16::from_le_bytes([data[0], data[1]]) as usize;
            return Ok(2 + count * 8);
        }
        let mut pos = 0usize;
        let count = u16::from_le_bytes([data[0], data[1]]) as usize;
        pos += 2;
        let needed = count * 8;
        if data.len() < pos + needed {
            return Err("proto stream truncated in half-diagonal section".into());
        }
        for _ in 0..count {
            let x = f32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
            let y =
                f32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
            pos += 8;
            self.static_mut().half_diagonals.push(pt(x, y));
            grid.add_move_box_half_diagonal(pt(x, y));
        }
        Ok(pos)
    }

    /// Load the pathfinding graph from a binary proto stream.
    ///
    /// The format is:
    /// 1. Half-diagonal list (unit sizes)
    /// 2. Graph hierarchy (layers → areas → obstacles → nodes)
    /// 3. Links (with node references as layer/area/obstacle/node tuples)
    /// 4. Link configurations
    /// 5. Fix-up pass to resolve index references
    /// 6. Build sector conversion table
    pub fn load_from_proto_stream(
        &mut self,
        grid: &mut FastFindGrid,
        data: &[u8],
    ) -> Result<(), String> {
        let mut pos = 0usize;

        let read_u16 = |data: &[u8], pos: &mut usize| -> Result<u16, String> {
            if *pos + 2 > data.len() {
                return Err("unexpected end of proto stream".into());
            }
            let val = u16::from_le_bytes([data[*pos], data[*pos + 1]]);
            *pos += 2;
            Ok(val)
        };

        let read_u8 = |data: &[u8], pos: &mut usize| -> Result<u8, String> {
            if *pos + 1 > data.len() {
                return Err("unexpected end of proto stream".into());
            }
            let val = data[*pos];
            *pos += 1;
            Ok(val)
        };

        let read_i16 = |data: &[u8], pos: &mut usize| -> Result<i16, String> {
            if *pos + 2 > data.len() {
                return Err("unexpected end of proto stream".into());
            }
            let val = i16::from_le_bytes([data[*pos], data[*pos + 1]]);
            *pos += 2;
            Ok(val)
        };

        let read_f32 = |data: &[u8], pos: &mut usize| -> Result<f32, String> {
            if *pos + 4 > data.len() {
                return Err("unexpected end of proto stream".into());
            }
            let val =
                f32::from_le_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            Ok(val)
        };

        let read_u32 = |data: &[u8], pos: &mut usize| -> Result<u32, String> {
            if *pos + 4 > data.len() {
                return Err("unexpected end of proto stream".into());
            }
            let val =
                u32::from_le_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            Ok(val)
        };

        // 1. Load half-diagonals (unit sizes).  May already have been
        // pre-populated by `preload_half_diagonals_from_proto` during
        // `initialize_from_mission` (before soldier spawn) — in that
        // case, just fast-forward past the section.
        let num_sizes = read_u16(data, &mut pos)?;
        let already_loaded = !self.static_data.half_diagonals.is_empty();
        for _ in 0..num_sizes {
            let x = read_f32(data, &mut pos)?;
            let y = read_f32(data, &mut pos)?;
            if !already_loaded {
                self.static_mut().half_diagonals.push(pt(x, y));
                grid.add_move_box_half_diagonal(pt(x, y));
            }
        }

        // 2. Load graph hierarchy
        let num_layers = read_u16(data, &mut pos)?;
        // Track node address → NodeIdx mapping for link resolution
        // Address is (layer, area, obstacle, node_within_obstacle)
        let mut node_address_to_idx: Vec<Vec<Vec<Vec<NodeIdx>>>> = Vec::new();

        for layer_idx in 0..num_layers {
            let num_areas = read_u16(data, &mut pos)?;

            let mut layer_nodes = Vec::new();
            let mut alt_layer_nodes = Vec::new();
            self.states.push(Vec::new());

            let mut layer_addresses = Vec::new();

            for _area_idx in 0..num_areas {
                let num_obstacles = read_u16(data, &mut pos)?;

                let mut area_nodes = Vec::new();
                let mut alt_area_nodes = Vec::new();
                self.states[layer_idx as usize].push(0);

                let mut area_addresses = Vec::new();

                for _obs_idx in 0..num_obstacles {
                    let num_nodes = read_u16(data, &mut pos)?;

                    let mut obstacle_nodes = Vec::new();
                    let mut obstacle_addresses = Vec::new();

                    for _node_idx in 0..num_nodes {
                        let global_idx = NodeIdx(self.nodes.len() as u32);

                        // Read configurations per half-diagonal
                        let num_configs = read_u16(data, &mut pos)?;
                        let mut configs = Vec::with_capacity(num_configs as usize);
                        for _ in 0..num_configs {
                            configs.push(read_u8(data, &mut pos)?);
                        }

                        // Node position
                        let px = read_i16(data, &mut pos)? as f32;
                        let py = read_i16(data, &mut pos)? as f32;

                        // Node 'From' vector
                        let fx = read_i16(data, &mut pos)? as f32;
                        let fy = read_i16(data, &mut pos)? as f32;

                        // Node 'To' vector
                        let tx = read_i16(data, &mut pos)? as f32;
                        let ty = read_i16(data, &mut pos)? as f32;

                        // Required state
                        let required_state = read_u32(data, &mut pos)?;

                        // Link indices (wrapped in `LinkIdx` typed-id).
                        let num_links = read_u16(data, &mut pos)?;
                        let mut link_indices = Vec::with_capacity(num_links as usize);
                        for _ in 0..num_links {
                            let link_idx = read_u16(data, &mut pos)?;
                            link_indices.push(LinkIdx(link_idx as u32));
                        }

                        self.nodes.push(PathGraphNode {
                            position: pt(px, py),
                            vector_to_node: pt(tx, ty),
                            vector_from_node: pt(fx, fy),
                            required_state,
                            configurations: configs,
                            link_indices,
                            alternative_link_indices: Vec::new(),
                            visited: false,
                            distance_from_source: 1e10,
                            distance_to_goal: 1e10,
                            score: 2e10,
                            previous_link_on_path: None,
                            leave_place: 0,
                            enter_place: 0,
                        });

                        obstacle_nodes.push(global_idx);
                        obstacle_addresses.push(global_idx);
                    }

                    area_nodes.push(obstacle_nodes);
                    alt_area_nodes.push(Vec::new()); // Empty alternative initially
                    area_addresses.push(obstacle_addresses);
                }

                layer_nodes.push(area_nodes);
                alt_layer_nodes.push(alt_area_nodes);
                layer_addresses.push(area_addresses);
            }

            self.layers.push(layer_nodes);
            self.alternative_layers.push(alt_layer_nodes);
            node_address_to_idx.push(layer_addresses);
        }

        // 3. Load links
        let num_links = read_u16(data, &mut pos)?;
        for _ in 0..num_links {
            // Next node address
            let next_layer = read_u16(data, &mut pos)? as usize;
            let next_area = read_u16(data, &mut pos)? as usize;
            let next_obs = read_u16(data, &mut pos)? as usize;
            let next_node = read_u16(data, &mut pos)? as usize;

            // Previous node address
            let prev_layer = read_u16(data, &mut pos)? as usize;
            let prev_area = read_u16(data, &mut pos)? as usize;
            let prev_obs = read_u16(data, &mut pos)? as usize;
            let prev_node = read_u16(data, &mut pos)? as usize;

            let next_node_idx = node_address_to_idx[next_layer][next_area][next_obs][next_node];
            let prev_node_idx = node_address_to_idx[prev_layer][prev_area][prev_obs][prev_node];

            // Distance
            let distance = read_f32(data, &mut pos)?;

            // Required state
            let required_state = read_u32(data, &mut pos)?;

            // Configuration indices (wrapped in `ConfigIdx`; validity is
            // checked in step 5 below, which clears entries whose target
            // config has the 255 sentinel).
            let num_configs = read_u16(data, &mut pos)?;
            let mut raw_config_indices = Vec::with_capacity(num_configs as usize);
            for _ in 0..num_configs {
                let config_idx = read_u16(data, &mut pos)?;
                raw_config_indices.push(config_idx);
            }

            self.static_mut().links.push(PathGraphLink {
                next_node: next_node_idx,
                prev_node: prev_node_idx,
                distance,
                required_state,
                // Store raw indices temporarily; resolved in step 5
                config_indices: raw_config_indices
                    .iter()
                    .map(|&i| Some(ConfigIdx(i as u32)))
                    .collect(),
            });
        }

        // 4. Load link configurations
        let num_configs = read_u16(data, &mut pos)?;
        for _ in 0..num_configs {
            let start_cfg = read_u8(data, &mut pos)?;

            if start_cfg == 255 {
                // Sentinel: invalid configuration
                self.static_mut().link_configs.push(PathGraphLinkConfig {
                    start_configurations: 255,
                    destination_configurations: 0,
                    start_config_list: Vec::new(),
                    destination_config_list: Vec::new(),
                });
            } else {
                let dest_cfg = read_u8(data, &mut pos)?;

                let num_start = read_u16(data, &mut pos)?;
                let mut start_list = Vec::with_capacity(num_start as usize);
                for _ in 0..num_start {
                    start_list.push(read_u8(data, &mut pos)?);
                }

                let num_dest = read_u16(data, &mut pos)?;
                let mut dest_list = Vec::with_capacity(num_dest as usize);
                for _ in 0..num_dest {
                    dest_list.push(read_u8(data, &mut pos)?);
                }

                self.static_mut().link_configs.push(PathGraphLinkConfig {
                    start_configurations: start_cfg,
                    destination_configurations: dest_cfg,
                    start_config_list: start_list,
                    destination_config_list: dest_list,
                });
            }
        }

        // 5. Resolve link configuration indices
        let static_data = self.static_mut();
        let n_configs = static_data.link_configs.len();
        let starts: Vec<u8> = static_data
            .link_configs
            .iter()
            .map(|c| c.start_configurations)
            .collect();
        for link in &mut static_data.links {
            for config_ref in &mut link.config_indices {
                if let Some(ConfigIdx(raw)) = *config_ref {
                    let idx = raw as usize;
                    if idx >= n_configs || starts[idx] == 255 {
                        *config_ref = None;
                    }
                }
            }
        }

        // 6. Resolve node link indices (nodes stored raw link indices during load)
        // The link indices in nodes were stored as raw u16 values cast to LinkIdx
        // They are already correct indices into self.static_data.links, so no resolution needed.

        // 7. Build sector conversion table
        // (This needs the move_layers which are loaded separately by the engine.
        //  The conversion table will be built when move_layers are set.)

        Ok(())
    }

    /// Build the sector conversion table from the move layers.
    /// Call this after both the graph and move layers are loaded.
    pub fn build_sector_conversion(&mut self) {
        // Snapshot the per-area obstacle counts before mutating the
        // static-data Arc.
        let counts: Vec<Vec<u16>> = self
            .static_data
            .move_layers
            .iter()
            .map(|layer| {
                layer
                    .iter()
                    .map(|area| area.motion_obstacles.len() as u16)
                    .collect()
            })
            .collect();
        let static_data = self.static_mut();
        static_data.sector_conversion.clear();
        let mut obstacle_count: u16 = 0;
        for layer_counts in counts {
            for (area_idx, n_obstacles) in layer_counts.iter().enumerate() {
                static_data.sector_conversion.push(SectorToArea {
                    sector: obstacle_count,
                    area: area_idx as u16,
                });
                obstacle_count += n_obstacles + 1;
            }
        }
    }
}

// ─── PathFinder ──────────────────────────────────────────────────

/// The pathfinding engine. Holds the graph and performs A* searches.
///
/// Plain struct with synchronous methods.
#[derive(Debug, Clone)]
pub struct PathFinderRuntime {
    /// The pathfinding graph.
    pub graph: PathGraph,

    /// Number of attempts for the A* (higher = more likely to find shortest path).
    pub number_of_attempts: u16,

    // ── Search state (reset per search) ──
    /// Sorted open node list (ascending by score).
    open_nodes: Vec<NodeIdx>,

    /// Best f-score found so far for a complete path.
    shortest_distance_found: f32,

    /// Current layer being searched.
    current_layer: u16,

    /// Current half-diagonal index being used.
    current_half_diagonal_idx: u16,

    /// Current half-diagonal vector.
    current_half_diagonal: Vec2D,

    /// Current motion area indices (layer, area).
    current_motion_area: (usize, usize),

    /// Current path graph area indices (layer, area).
    current_graph_area: (usize, usize),
}

/// Serializable pathfinder simulation state.
///
/// The graph geometry is level data. Pending requests live in the
/// deterministic engine movement queues, so the remaining pathfinder
/// snapshot is just the attempt count plus per-area obstacle state
/// table.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PathFinder {
    pub number_of_attempts: u16,
    pub states: Vec<Vec<u32>>,
}

impl Default for PathFinder {
    fn default() -> Self {
        Self::new()
    }
}

impl PathFinder {
    pub fn new() -> Self {
        Self {
            number_of_attempts: 1,
            states: Vec::new(),
        }
    }

    fn runtime_from_graph(&self, graph: &PathGraph) -> PathFinderRuntime {
        let mut runtime = PathFinderRuntime::new();
        runtime.number_of_attempts = self.number_of_attempts;
        runtime.graph = graph.clone();
        if self.states.len() != runtime.graph.states.len() {
            panic!(
                "pathfinder state layer count {} does not match level graph layer count {}",
                self.states.len(),
                runtime.graph.states.len()
            );
        }
        for (layer_idx, (state_layer, graph_layer)) in
            self.states.iter().zip(&runtime.graph.states).enumerate()
        {
            if state_layer.len() != graph_layer.len() {
                panic!(
                    "pathfinder state area count for layer {} is {} but level graph has {}",
                    layer_idx,
                    state_layer.len(),
                    graph_layer.len()
                );
            }
        }
        runtime.graph.states = self.states.clone();
        runtime.set_states_all();
        runtime
    }

    pub fn initialize_from_graph(&mut self, graph: &PathGraph) {
        let mut runtime = PathFinderRuntime::new();
        runtime.graph = graph.clone();
        runtime.initialize();
        self.states = runtime.graph.states;
        self.number_of_attempts = runtime.number_of_attempts;
    }

    pub fn try_convert_sector(&self, graph: &PathGraph, sector: u16) -> Option<u16> {
        graph.try_convert_sector(sector)
    }

    pub fn cancel_requests_for(&mut self, _actor_id: EntityId) {}

    pub fn toggle_obstacle_state(
        &mut self,
        graph: &PathGraph,
        layer: usize,
        area: usize,
        changing_obstacle: u16,
        appeared: &mut Vec<AppearedObstacle>,
        line_toggles: &mut Vec<(crate::fast_find_grid::LineIndex, bool)>,
    ) -> bool {
        let mut runtime = self.runtime_from_graph(graph);
        let changed =
            runtime.toggle_obstacle_state(layer, area, changing_obstacle, appeared, line_toggles);
        self.states = runtime.graph.states;
        changed
    }

    #[allow(clippy::too_many_arguments)]
    pub fn find_path(
        &self,
        graph: &PathGraph,
        grid: &FastFindGrid,
        layer: u16,
        sector: u16,
        half_diagonal_idx: u16,
        source: MapPoint,
        goal: MapPoint,
        use_first_point: bool,
    ) -> Option<Vec<MapPoint>> {
        let mut runtime = self.runtime_from_graph(graph);
        runtime.find_path(
            grid,
            layer,
            sector,
            half_diagonal_idx,
            source,
            goal,
            use_first_point,
        )
    }

    pub fn draw_graph<F>(&self, graph: &PathGraph, view: BBox2D, half_diagonal_idx: u16, draw: F)
    where
        F: FnMut(GeoPoint2D, GeoPoint2D, u16),
    {
        self.runtime_from_graph(graph)
            .draw_graph(view, half_diagonal_idx, draw);
    }

    pub fn draw_nodes<F>(&self, graph: &PathGraph, view: BBox2D, half_diagonal_idx: u16, draw: F)
    where
        F: FnMut(GeoPoint2D, GeoPoint2D, u16),
    {
        self.runtime_from_graph(graph)
            .draw_nodes(view, half_diagonal_idx, draw);
    }
}

impl Default for PathFinderRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl PathFinderRuntime {
    pub fn new() -> Self {
        Self {
            graph: PathGraph::new(),
            number_of_attempts: 1,
            open_nodes: Vec::new(),
            shortest_distance_found: 2e10,
            current_layer: 0,
            current_half_diagonal_idx: 0,
            current_half_diagonal: pt(0.0, 0.0),
            current_motion_area: (0, 0),
            current_graph_area: (0, 0),
        }
    }

    // ── Request queue (retained for cancel-only after refactor) ───
    //
    // `add_request` / `process_requests` + the `pending_requests`
    // queue were deleted during the order-queue refactor.  Movement
    // now takes the synchronous `find_path` path via Move sequence
    // elements (see `engine::tick::perform_hourglass_inner`'s Move
    // dispatch), so nothing ever enqueues.  `cancel_requests_for` is
    // kept as a no-op for call-site compatibility (the former callers
    // tore down any in-flight request on actor-state resets;
    // post-refactor the teardown is handled by sequence-element
    // interruption instead).
    pub fn cancel_requests_for(&mut self, _actor_id: EntityId) {
        // no-op: no queue to drain
    }

    // `make_fast` / `make_slow` / `make_upright` / `make_crouched`
    // / `process_requests` were deleted along with the
    // `pending_requests` queue — they only rewrote in-flight queued
    // requests, and no producer remains.  Mid-path speed/posture
    // changes now rewrite the Move element's orders directly (see
    // `posture_transitions.rs::reapply_drunken_deviation`).

    // ── Core A* ──────────────────────────────────────────────────

    /// Find a path from `source` to `goal` within a given layer/area
    /// for a unit with the given half-diagonal index.
    ///
    /// Returns the path as a sequence of waypoints, or `None` if no path exists.
    #[allow(clippy::too_many_arguments)]
    pub fn find_path(
        &mut self,
        grid: &FastFindGrid,
        layer: u16,
        area: u16,
        half_diagonal_idx: u16,
        source: MapPoint,
        goal: MapPoint,
        use_first_point: bool,
    ) -> Option<Vec<MapPoint>> {
        let source_geo = source.to_geo();
        let goal_geo = goal.to_geo();
        tracing::trace!(
            ?source,
            ?goal,
            layer,
            area,
            half_diagonal_idx,
            use_first_point,
            "find_path: entry",
        );
        self.current_layer = layer;
        let right_area = self.graph.convert_sector(area) as usize;

        self.current_graph_area = (layer as usize, right_area);
        self.current_motion_area = (layer as usize, right_area);

        self.current_half_diagonal_idx = half_diagonal_idx;
        self.current_half_diagonal =
            self.graph.static_data.half_diagonals[half_diagonal_idx as usize];

        self.reset_graph();

        // Check if goal position is valid
        if !self.object_position_authorized_geo(grid, goal_geo) {
            tracing::trace!(
                ?source,
                ?goal,
                layer,
                area,
                right_area,
                half_diagonal_idx,
                "find_path: goal position not authorized — returning None",
            );
            return None;
        }

        // Check if direct path is possible. Gated on `use_first_point`:
        // the fast-return is only attempted when the request was flagged
        // as needing the source kept (i.e. the source is already known
        // authorized). With `use_first_point == false` we must fall
        // through to A* and return a node-routed path that starts at
        // `source`.
        let fast_ok = self.is_reachable_fast_geo(source_geo, goal_geo);
        let grid_ok = fast_ok && self.is_reachable_grid_geo(grid, source_geo, goal_geo);
        if use_first_point && fast_ok && grid_ok {
            tracing::trace!(
                ?source,
                ?goal,
                layer,
                area,
                right_area,
                "find_path: direct path — returning goal",
            );
            return Some(vec![goal]);
        }

        // Link source to nearby graph nodes
        self.link_source(grid, source_geo, goal_geo);

        // Run A* to find node-based path
        let end_node = self.find_path_nodes(grid, goal_geo);

        if end_node.is_none() {
            tracing::trace!(
                ?source,
                ?goal,
                layer,
                area,
                right_area,
                half_diagonal_idx,
                fast_ok,
                grid_ok,
                "find_path: A* found no path — returning None",
            );
        }

        if let Some(end_node_idx) = end_node {
            let mut path = Vec::new();
            path.push(goal_geo);

            let mut current_node = end_node_idx;
            let mut leave_places = self.graph.nodes[current_node.0 as usize].leave_place;
            let mut current_link = self.graph.nodes[current_node.0 as usize].previous_link_on_path;

            // Trace back through the path, building waypoints around each node.
            // Loop until the node has no previous link, then fall through to
            // `pass_around_last_node` on the source-side node. The zero-link
            // case (end_node is itself the source-side node) must still call
            // `pass_around_last_node`, so the call is hoisted OUT of the loop.
            while let Some(link_idx) = current_link {
                leave_places = self.pass_around_node(link_idx, leave_places, &mut path);
                current_node = self.graph.static_data.links[link_idx.0 as usize].prev_node;
                current_link = self.graph.nodes[current_node.0 as usize].previous_link_on_path;
            }

            let enter_places = self.graph.nodes[current_node.0 as usize].enter_place;
            self.pass_around_last_node(current_node, enter_places, leave_places, &mut path);

            path.push(source_geo);
            path.reverse();

            // Post-processing: remove redundant waypoints
            self.smooth_path(grid, &mut path);

            Some(path.into_iter().map(MapPoint::from_geo).collect())
        } else {
            None
        }
    }

    /// A* search on graph nodes. Returns the last node of the best path found.
    fn find_path_nodes(&mut self, grid: &FastFindGrid, goal: GeoPoint2D) -> Option<NodeIdx> {
        let mut best_node: Option<NodeIdx> = None;
        let mut attempts_left = self.number_of_attempts;

        while !self.open_nodes.is_empty() {
            // Pop the node with the lowest score
            let current_idx = self.open_nodes.remove(0);
            let current_pos = self.graph.nodes[current_idx.0 as usize].position;
            let node_config = self.graph.nodes[current_idx.0 as usize]
                .configurations
                .get(self.current_half_diagonal_idx as usize)
                .copied()
                .unwrap_or(0);

            // Check if the goal is directly reachable from this node
            let mut end_place: u8 = 0;
            if self.is_reachable_fast_geo(goal, current_pos) {
                let mut dp = TOP_LEFT;
                while dp < 16 {
                    if (dp & node_config) != 0 {
                        let is_good_direct = self.is_good_docking_place(
                            goal,
                            current_idx,
                            dp,
                            self.current_half_diagonal,
                            true,
                        );
                        let is_good_indirect = self.is_good_docking_place(
                            goal,
                            current_idx,
                            dp,
                            self.current_half_diagonal,
                            false,
                        );

                        if is_good_direct || is_good_indirect {
                            // Skip if node has opposing-diagonal-only config (5 or 10)
                            if node_config != 5 && node_config != 10 {
                                let dock_pt = self.docking_point_geo(
                                    current_idx,
                                    dp,
                                    self.current_half_diagonal,
                                );
                                if self.is_reachable_grid_geo(grid, goal, dock_pt) {
                                    end_place |= dp;
                                }
                            }
                        }
                    }
                    dp <<= 1;
                }
            }

            if end_place != 0 {
                let current_score = self.graph.nodes[current_idx.0 as usize].score;
                if self.shortest_distance_found > current_score {
                    self.shortest_distance_found = current_score;
                    self.graph.nodes[current_idx.0 as usize].leave_place = end_place;
                    best_node = Some(current_idx);
                    attempts_left -= 1;
                }

                if attempts_left == 0 {
                    return best_node;
                }
            }

            // Expand neighbors
            let num_links = self.graph.nodes[current_idx.0 as usize].link_indices.len();
            for link_i in 0..num_links {
                let link_idx = self.graph.nodes[current_idx.0 as usize].link_indices[link_i];
                let link = &self.graph.static_data.links[link_idx.0 as usize];
                let next_node_idx = link.next_node;

                let config = link
                    .config_indices
                    .get(self.current_half_diagonal_idx as usize)
                    .copied()
                    .flatten();

                if config.is_some() {
                    // Only expand if node config allows free passage between docking places
                    if node_config != 5 && node_config != 10 {
                        let new_dist = self.graph.nodes[current_idx.0 as usize]
                            .distance_from_source
                            + link.distance;

                        if new_dist
                            < self.graph.nodes[next_node_idx.0 as usize].distance_from_source
                            && new_dist < self.shortest_distance_found
                        {
                            let next = &mut self.graph.nodes[next_node_idx.0 as usize];
                            next.previous_link_on_path = Some(link_idx);
                            next.distance_from_source = new_dist;

                            if !next.visited {
                                next.distance_to_goal =
                                    Self::estimate_distance(next.position, goal);
                            }

                            next.score = new_dist + next.distance_to_goal;
                            next.visited = true;

                            self.add_to_open_nodes(next_node_idx);
                        }
                    }
                }
            }
        }

        best_node
    }

    /// Add a node to the sorted open list.
    /// Maintains ascending order by score. Skips if already present.
    fn add_to_open_nodes(&mut self, node: NodeIdx) {
        let score = self.graph.nodes[node.0 as usize].score;

        if self.open_nodes.is_empty() {
            self.open_nodes.push(node);
            return;
        }

        let mut insert_pos = 0;
        while insert_pos < self.open_nodes.len() {
            let existing = self.open_nodes[insert_pos];
            // Dedupe only fires for advanced positions (insert_pos > 0):
            // the equality check is inside the loop after the first
            // increment, so a duplicate at index 0 is tolerated. This
            // skew matters because the open-list ordering feeds A*
            // tiebreaks deterministically.
            if insert_pos > 0 && existing == node {
                return; // Already in list
            }
            if self.graph.nodes[existing.0 as usize].score >= score {
                break;
            }
            insert_pos += 1;
        }

        self.open_nodes.insert(insert_pos, node);
    }

    /// Link the source point to reachable graph nodes, populating the open list.
    fn link_source(&mut self, grid: &FastFindGrid, source: GeoPoint2D, goal: GeoPoint2D) {
        // Build a bounding box to limit which nodes we consider
        let movement = pt(goal.x - source.x, goal.y - source.y);
        let link_margin = pt(400.0, 400.0);

        let box_link = if movement.x > 0.0 {
            if movement.y > 0.0 {
                BBox2D::from_corners(
                    pt(source.x - link_margin.x, source.y - link_margin.y),
                    pt(goal.x + link_margin.x, goal.y + link_margin.y),
                )
            } else {
                BBox2D::from_corners(
                    pt(source.x - link_margin.x, goal.y - link_margin.y),
                    pt(goal.x + link_margin.x, source.y + link_margin.y),
                )
            }
        } else if movement.y > 0.0 {
            BBox2D::from_corners(
                pt(goal.x - link_margin.x, source.y - link_margin.y),
                pt(source.x + link_margin.x, goal.y + link_margin.y),
            )
        } else {
            BBox2D::from_corners(
                pt(goal.x - link_margin.x, goal.y - link_margin.y),
                pt(source.x + link_margin.x, source.y + link_margin.y),
            )
        };

        // Single pass over candidate nodes.  Returns the number of
        // nodes actually linked.  Factored out so we can retry with a
        // relaxed docking-check when the strict pass produces nothing
        // (see `relax_grid` below).
        let linked = self.try_link_nodes(grid, source, goal, &box_link, false);

        if linked == 0 {
            // Fallback: the thick-corridor grid check rejected every
            // candidate docking point.  This happens when the actor
            // is in a "narrow pocket" — its 1-px-shrunk bbox fits
            // between nearby motion lines (so `object_position_
            // authorized` passes and the actor legitimately stands
            // here), but every full-corridor sweep to a nearby graph
            // node clips a line because the corridor is ~hd wider
            // than the bbox.  Without this fallback the symptom is a
            // PC that stalls at an interrupted-walk midpoint:
            // `find_path` returns None and the Move element ages out
            // to `IMPOSSIBLE` after the 100-frame timeout, so the
            // actor is frozen for a full second of in-game time even
            // though it legitimately stands at an authorized spot.
            //
            // The unauthorized-source case is handled upstream:
            // `engine::movement::try_dispatch_move_path` pre-snaps any
            // unauthorized actor bbox via `find_authorized_position`
            // before submitting, then sets `use_first_point = true` so
            // the snapped point becomes the first waypoint.  This
            // branch therefore only handles the *narrow-pocket* case
            // below.
            //
            // Relaxed retry: only if the source position itself is
            // authorized (so we're not papering over an actor that
            // actually clipped out of bounds — `try_dispatch_move_path`
            // already extracted those before calling `find_path`), drop
            // the `is_reachable_grid` check at the docking-point step.
            // The skeleton (`is_reachable_fast`) check and the
            // `is_good_docking_place` geometry test still gate which
            // nodes get linked, so the pathfinder still respects
            // coarse sector topology.  Per-frame motion-line
            // collision during the actual walk (handled in
            // `engine/movement.rs`) will clamp any detail that the
            // relaxed check glossed over.
            if self.object_position_authorized_geo(grid, source) {
                let relaxed_linked = self.try_link_nodes(grid, source, goal, &box_link, true);
                tracing::trace!(
                    ?source,
                    ?goal,
                    relaxed_linked,
                    "link_source: strict pass empty; fell back to relaxed grid check",
                );
            } else {
                tracing::trace!(
                    ?source,
                    ?goal,
                    "link_source: strict pass empty and source not authorized; no fallback",
                );
            }
        }
    }

    /// Body of `link_source`.  When `relax_grid` is true the
    /// `is_reachable_grid` check on the source→docking-point corridor is
    /// skipped — used as a fallback when the strict pass finds nothing
    /// (see `link_source` for rationale).  Returns the number of nodes
    /// successfully added to the open list.
    fn try_link_nodes(
        &mut self,
        grid: &FastFindGrid,
        source: GeoPoint2D,
        goal: GeoPoint2D,
        box_link: &BBox2D,
        relax_grid: bool,
    ) -> u32 {
        let (layer, area) = self.current_graph_area;
        let hd_idx = self.current_half_diagonal_idx as usize;
        let hd = self.current_half_diagonal;

        let mut cnt_linked: u32 = 0;

        let num_obstacles = self.graph.layers[layer][area].len();
        for obs_idx in 0..num_obstacles {
            let num_nodes = self.graph.layers[layer][area][obs_idx].len();
            for node_i in 0..num_nodes {
                let node_idx = self.graph.layers[layer][area][obs_idx][node_i];
                let node = &self.graph.nodes[node_idx.0 as usize];
                let node_config = node.configurations.get(hd_idx).copied().unwrap_or(0);

                if node_config == 0 {
                    continue;
                }

                // Check if node is in range, useful, and reachable
                if !box_link.contains_point(node.position)
                    || !self.is_useful_link(source, node_idx)
                    || !self.is_reachable_fast_geo(source, node.position)
                {
                    continue;
                }

                let mut start_config: u8 = 0;
                let mut dp = TOP_LEFT;
                while dp < 16 {
                    if (dp & node_config) != 0 {
                        let good_direct =
                            self.is_good_docking_place(source, node_idx, dp, hd, true);
                        let good_indirect =
                            self.is_good_docking_place(source, node_idx, dp, hd, false);

                        if good_direct || good_indirect {
                            let grid_ok = relax_grid
                                || self.is_reachable_grid_geo(
                                    grid,
                                    source,
                                    self.docking_point_geo(node_idx, dp, hd),
                                );
                            if grid_ok {
                                start_config |= dp;
                            }
                        }
                    }
                    dp <<= 1;
                }

                if start_config != 0 {
                    let node = &mut self.graph.nodes[node_idx.0 as usize];
                    node.enter_place = start_config;
                    node.distance_from_source =
                        geo2d::length(pt(node.position.x - source.x, node.position.y - source.y));
                    node.distance_to_goal = Self::estimate_distance(node.position, goal);
                    node.score = node.distance_from_source + node.distance_to_goal;
                    node.previous_link_on_path = None;
                    node.visited = true;

                    self.add_to_open_nodes(node_idx);
                    cnt_linked += 1;
                }
            }
        }

        cnt_linked
    }

    /// Reset the graph for a new A* search.
    fn reset_graph(&mut self) {
        self.open_nodes.clear();
        self.shortest_distance_found = 2e10;

        let (layer, area) = self.current_graph_area;
        for obstacle in &self.graph.layers[layer][area] {
            for &node_idx in obstacle {
                let node = &mut self.graph.nodes[node_idx.0 as usize];
                node.visited = false;
                node.distance_from_source = 1e10;
                node.distance_to_goal = 1e10;
                node.score = 2e10;
                node.previous_link_on_path = None;
            }
        }
    }

    // ── Path construction ────────────────────────────────────────

    /// Compute the world position of a docking point.
    #[inline]
    pub fn docking_point(&self, node: NodeIdx, place: u8, half_diagonal: Vec2D) -> MapPoint {
        MapPoint::from_geo(self.docking_point_geo(node, place, half_diagonal))
    }

    #[inline]
    fn docking_point_geo(&self, node: NodeIdx, place: u8, half_diagonal: Vec2D) -> GeoPoint2D {
        let pos = self.graph.nodes[node.0 as usize].position;
        match place {
            TOP_LEFT => pt(pos.x - half_diagonal.x, pos.y - half_diagonal.y),
            TOP_RIGHT => pt(pos.x + half_diagonal.x, pos.y - half_diagonal.y),
            BOTTOM_LEFT => pt(pos.x - half_diagonal.x, pos.y + half_diagonal.y),
            BOTTOM_RIGHT => pt(pos.x + half_diagonal.x, pos.y + half_diagonal.y),
            _ => pos,
        }
    }

    /// Check if a docking place is appropriate for a unit at `point`.
    fn is_good_docking_place(
        &self,
        point: GeoPoint2D,
        node: NodeIdx,
        docking_place: u8,
        half_diagonal: Vec2D,
        direct: bool,
    ) -> bool {
        let dock_pt = self.docking_point_geo(node, docking_place, half_diagonal);
        let test_vec = pt(point.x - dock_pt.x, point.y - dock_pt.y);

        let (v1, v2) = if direct {
            match docking_place {
                TOP_LEFT => (pt(-1.0, 0.0), pt(0.0, 1.0)),
                TOP_RIGHT => (pt(0.0, -1.0), pt(-1.0, 0.0)),
                BOTTOM_LEFT => (pt(0.0, 1.0), pt(1.0, 0.0)),
                BOTTOM_RIGHT => (pt(1.0, 0.0), pt(0.0, -1.0)),
                _ => return false,
            }
        } else {
            match docking_place {
                TOP_LEFT => (pt(1.0, 0.0), pt(0.0, -1.0)),
                TOP_RIGHT => (pt(0.0, 1.0), pt(1.0, 0.0)),
                BOTTOM_LEFT => (pt(0.0, -1.0), pt(-1.0, 0.0)),
                BOTTOM_RIGHT => (pt(-1.0, 0.0), pt(0.0, 1.0)),
                _ => return false,
            }
        };

        if direct {
            geo2d::cross(v1, test_vec) < 0.0 && geo2d::cross(v2, test_vec) >= 0.0
        } else {
            geo2d::cross(v1, test_vec) <= 0.0 && geo2d::cross(v2, test_vec) > 0.0
        }
    }

    /// Build the path segment that passes around a node.
    /// Returns the enter places for the previous node on the path.
    fn pass_around_node(&self, link: LinkIdx, leave_places: u8, path: &mut Vec<GeoPoint2D>) -> u8 {
        if leave_places == 0 {
            return 0;
        }

        let link_data = &self.graph.static_data.links[link.0 as usize];
        let current_node = link_data.next_node;
        let hd_idx = self.current_half_diagonal_idx as usize;
        let hd = self.current_half_diagonal;

        let config_idx = link_data.config_indices.get(hd_idx).copied().flatten();
        let link_config = match config_idx {
            Some(ci) => &self.graph.static_data.link_configs[ci.0 as usize],
            None => return 0,
        };

        let enter_places = link_config.destination_configurations;
        let common = enter_places & leave_places;

        // Fast path: exactly one common docking place
        if common != 0 && number_of_places(common) == 1 {
            path.push(self.docking_point_geo(current_node, common, hd));
            return self.collect_start_places(link_config, common);
        }

        // Find best route around the node
        let (best_enter, best_leave, best_direct) =
            self.find_best_route_around(current_node, leave_places, enter_places, hd_idx);

        // Emit waypoints along the route
        let mut current_place = best_leave;
        while current_place != best_enter {
            path.push(self.docking_point_geo(current_node, current_place, hd));
            current_place = next_docking_place(current_place, best_direct);
        }
        path.push(self.docking_point_geo(current_node, current_place, hd));

        self.collect_start_places(link_config, best_enter)
    }

    /// Build the path segment that passes around the last (source-side) node.
    fn pass_around_last_node(
        &self,
        node: NodeIdx,
        enter_places: u8,
        leave_places: u8,
        path: &mut Vec<GeoPoint2D>,
    ) {
        if leave_places == 0 {
            return;
        }

        let hd_idx = self.current_half_diagonal_idx as usize;
        let hd = self.current_half_diagonal;
        let common = enter_places & leave_places;

        // Fast path: exactly one common docking place
        if common != 0 && number_of_places(common) == 1 {
            path.push(self.docking_point_geo(node, common, hd));
            return;
        }

        // Find best route
        let (best_enter, best_leave, best_direct) =
            self.find_best_route_around(node, leave_places, enter_places, hd_idx);

        // Emit waypoints
        let mut current_place = best_leave;
        while current_place != best_enter {
            path.push(self.docking_point_geo(node, current_place, hd));
            current_place = next_docking_place(current_place, best_direct);
        }
        path.push(self.docking_point_geo(node, current_place, hd));
    }

    /// Find the best route around a node from `leave_places` to `target_places`.
    /// Returns (enter_place, leave_place, is_direct).
    fn find_best_route_around(
        &self,
        node: NodeIdx,
        leave_places: u8,
        target_places: u8,
        hd_idx: usize,
    ) -> (u8, u8, bool) {
        let node_config = self.graph.nodes[node.0 as usize]
            .configurations
            .get(hd_idx)
            .copied()
            .unwrap_or(0);

        let num_leave = number_of_places(leave_places);

        let mut best_score = 5u8;
        let mut best_enter = TOP_LEFT;
        let mut best_leave_place = TOP_LEFT;
        let mut best_direct = true;

        let mut current_place = TOP_LEFT;

        for _ in 0..num_leave {
            // Advance to next set leave place
            while (current_place & leave_places) == 0 && current_place < 16 {
                current_place <<= 1;
            }
            if current_place >= 16 {
                break;
            }

            // Try direct (clockwise) and indirect (counter-clockwise)
            for &direct in &[true, false] {
                let mut probe = current_place;
                let mut score = 0u8;
                let mut valid = true;

                loop {
                    probe = next_docking_place(probe, direct);
                    if (probe & node_config) == 0 {
                        valid = false;
                        break;
                    }
                    if (probe & target_places) != 0 {
                        break;
                    }
                    score += 1;
                    if score > 4 {
                        valid = false;
                        break;
                    }
                }

                // Tie-break rules differ by leave-count:
                //  * Multi-leave branch uses `<` with direct tested
                //    first, so direct wins ties (indirect's
                //    `score < best_score` fails after direct updated
                //    best_score in the same inner iter).
                //  * 1-leave branch has no scoring — assignments are
                //    unconditional and indirect runs last, so indirect
                //    wins ties within an iteration.
                // Encode both: use `<=` for indirect in the 1-leave case.
                let wins = if num_leave == 1 && !direct {
                    score <= best_score
                } else {
                    score < best_score
                };
                if valid && wins {
                    best_score = score;
                    best_enter = probe;
                    best_leave_place = current_place;
                    best_direct = direct;
                }
            }

            current_place <<= 1;
        }

        (best_enter, best_leave_place, best_direct)
    }

    /// Collect the start places that correspond to a given destination place.
    fn collect_start_places(&self, config: &PathGraphLinkConfig, dest_place: u8) -> u8 {
        let mut result: u8 = 0;
        for (i, &dest) in config.destination_config_list.iter().enumerate() {
            if dest == dest_place
                && let Some(&start) = config.start_config_list.get(i)
            {
                result |= start;
            }
        }
        result
    }

    /// Post-process a path to remove redundant waypoints.
    /// If three consecutive points can be shortcut (middle one removed),
    /// do so.
    fn smooth_path(&self, grid: &FastFindGrid, path: &mut Vec<GeoPoint2D>) {
        if path.len() <= 3 {
            return;
        }

        let mut i = 0;
        while i + 2 < path.len() {
            let first = path[i];
            let last = path[i + 2];

            let small_vec = pt(0.5e-4 * (last.x - first.x), 0.5e-4 * (last.y - first.y));
            let p1 = pt(first.x + small_vec.x, first.y + small_vec.y);
            let p2 = pt(last.x - small_vec.x, last.y - small_vec.y);

            if self.is_reachable_grid_geo(grid, p1, p2) {
                path.remove(i + 1);
            } else {
                i += 1;
            }
        }
    }

    // ── Reachability checks ──────────────────────────────────────

    /// Fast line-of-sight check using skeleton segments.
    /// Returns true if the segment from `p1` to `p2` does not cross any
    /// skeleton line.
    pub fn is_reachable_fast(&self, p1: MapPoint, p2: MapPoint) -> bool {
        self.is_reachable_fast_geo(p1.to_geo(), p2.to_geo())
    }

    fn is_reachable_fast_geo(&self, p1: GeoPoint2D, p2: GeoPoint2D) -> bool {
        let move_seg = geo2d::segment(p1, p2);
        let (layer, area) = self.current_motion_area;
        // Callers must call `set_current_motion_area` before any
        // reachability probe — assert strictly rather than falling back
        // defensively, since returning `true` would silently mark
        // unreachable corridors as clear (per CLAUDE.md "no fake data"
        // rule).
        assert!(
            layer < self.graph.static_data.move_layers.len()
                && area < self.graph.static_data.move_layers[layer].len(),
            "is_reachable_fast called before set_current_motion_area: layer={layer} area={area}"
        );
        let motion_area = &self.graph.static_data.move_layers[layer][area];

        for skel_seg in &motion_area.skeleton {
            if geo2d::segments_intersect(move_seg, *skel_seg) {
                return false;
            }
        }
        true
    }

    /// Grid-based thick reachability check using motion lines.
    /// Builds a movement corridor and checks for intersecting motion lines.
    pub fn is_reachable_grid(&self, grid: &FastFindGrid, p1: MapPoint, p2: MapPoint) -> bool {
        self.is_reachable_grid_geo(grid, p1.to_geo(), p2.to_geo())
    }

    fn is_reachable_grid_geo(&self, grid: &FastFindGrid, p1: GeoPoint2D, p2: GeoPoint2D) -> bool {
        let hd = self.current_half_diagonal;
        let corridor = match FastFindGrid::build_thick_move_corridor(p1, p2, hd) {
            Some(c) => c,
            None => return true, // Zero movement
        };

        let line_indices = grid.get_active_motion_lines_for_segments(
            self.current_layer,
            corridor.seg1,
            corridor.seg2,
            &corridor.bbox,
        );

        if line_indices.is_empty() {
            return true;
        }

        // Check segment intersections
        for &idx in &line_indices {
            let line = &grid.level.lines[usize::from(idx)];
            if line.intersects_segment(corridor.seg1) || line.intersects_segment(corridor.seg2) {
                return false;
            }
        }

        // Check if any line endpoint lies inside the corridor
        for &idx in &line_indices {
            let p = grid.level.lines[usize::from(idx)].a;
            if corridor.point_inside(p.to_geo()) {
                return false;
            }
        }

        true
    }

    /// Check if a unit at `point` does not collide with any motion line.
    pub fn object_position_authorized(&self, grid: &FastFindGrid, point: MapPoint) -> bool {
        self.object_position_authorized_geo(grid, point.to_geo())
    }

    fn object_position_authorized_geo(&self, grid: &FastFindGrid, point: GeoPoint2D) -> bool {
        let hd = pt(
            self.current_half_diagonal.x - 1.0,
            self.current_half_diagonal.y - 1.0,
        );
        let bbox = BBox2D::from_corners(
            pt(point.x - hd.x, point.y - hd.y),
            pt(point.x + hd.x, point.y + hd.y),
        );

        // Bounds check
        if let Some(rect) = bbox.0 {
            let x_min = (rect.min().x as i32) >> 6;
            let y_min = (rect.min().y as i32) >> 6;
            let x_max = (rect.max().x as i32) >> 6;
            let y_max = (rect.max().y as i32) >> 6;

            if x_min < 0
                || y_min < 0
                || x_max >= grid.level.grid_width as i32
                || y_max >= grid.level.grid_height as i32
            {
                return false;
            }
        }

        grid.is_position_authorized(&bbox, self.current_layer)
    }

    /// Check if it is useful to visit a node from a given point.
    /// Tests whether any docking point is "visible" from the node's perspective.
    fn is_useful_link(&self, point: GeoPoint2D, node: NodeIdx) -> bool {
        let n = &self.graph.nodes[node.0 as usize];
        let hd = self.current_half_diagonal;

        // Test all four docking positions
        let offsets = [
            pt(-hd.x, -hd.y), // TOP_LEFT
            pt(hd.x, -hd.y),  // TOP_RIGHT
            pt(hd.x, hd.y),   // BOTTOM_RIGHT
            pt(-hd.x, hd.y),  // BOTTOM_LEFT
        ];

        for offset in &offsets {
            let v = pt(
                point.x + offset.x - n.position.x,
                point.y + offset.y - n.position.y,
            );
            if geo2d::cross(n.vector_to_node, v) > 0.0 || geo2d::cross(n.vector_from_node, v) > 0.0
            {
                return true;
            }
        }

        false
    }

    /// Estimate the distance between two points (straight-line heuristic).
    #[inline]
    fn estimate_distance(p1: GeoPoint2D, p2: GeoPoint2D) -> f32 {
        geo2d::length(pt(p2.x - p1.x, p2.y - p1.y))
    }

    // ── State management ─────────────────────────────────────────

    /// Change the state of an area, activating/deactivating nodes and links
    /// based on the new state bitmask.
    ///
    /// Returns `true` if any motion obstacles changed activation.
    ///
    /// The appeared-obstacles list and grid-line toggle list are
    /// exposed through [`Self::set_state_area_with_appeared`]; this
    /// wrapper discards them.
    pub fn set_state_area(&mut self, layer: usize, area: usize, new_state: u32) -> bool {
        let mut appeared = Vec::new();
        let mut line_toggles = Vec::new();
        self.set_state_area_with_appeared(layer, area, new_state, &mut appeared, &mut line_toggles)
    }

    /// Like [`Self::set_state_area`] but pushes the bounding boxes of
    /// motion obstacles that went from inactive → active into
    /// `appeared` (used downstream to kill actors standing on cells
    /// that just became blocked) and the grid-line
    /// `(LineIndex, active)` toggles implied by each obstacle's state
    /// change into `line_toggles` (so the grid's `line_active` flags
    /// stay in sync with the motion-obstacle active list).
    pub fn set_state_area_with_appeared(
        &mut self,
        layer: usize,
        area: usize,
        new_state: u32,
        appeared: &mut Vec<AppearedObstacle>,
        line_toggles: &mut Vec<(crate::fast_find_grid::LineIndex, bool)>,
    ) -> bool {
        self.graph.states[layer][area] = new_state;
        let mut changed = false;

        // Move nodes that don't match new state: active → alternative.
        // Reverse iteration + `Vec::remove(i)` preserves surviving
        // element order so deterministic A* tiebreaks stay stable
        // across state changes.
        let num_obstacles = self.graph.layers[layer][area].len();
        for obs_idx in 0..num_obstacles {
            for node_i in (0..self.graph.layers[layer][area][obs_idx].len()).rev() {
                let node_idx = self.graph.layers[layer][area][obs_idx][node_i];
                let required = self.graph.nodes[node_idx.0 as usize].required_state;

                if (required & new_state) != required {
                    // Move to alternative
                    self.graph.layers[layer][area][obs_idx].remove(node_i);
                    self.graph.alternative_layers[layer][area][obs_idx].push(node_idx);
                    // Move non-matching links to alternative
                    Self::update_node_links_for_state_static(
                        &mut self.graph.nodes,
                        &self.graph.static_data.links,
                        node_idx,
                        new_state,
                    );
                } else {
                    // Keep active, update links
                    Self::update_node_links_for_state_static(
                        &mut self.graph.nodes,
                        &self.graph.static_data.links,
                        node_idx,
                        new_state,
                    );
                }
            }
        }

        // Move nodes that now match: alternative → active.
        // Same reverse-iter + `Vec::remove(i)` pattern as the
        // active→alternative pass.
        // Note: neither node-move branch touches `changed`; that flag
        // tracks only motion-obstacle transitions further down.
        let num_alt_obstacles = self.graph.alternative_layers[layer][area].len();
        for obs_idx in 0..num_alt_obstacles {
            for node_i in (0..self.graph.alternative_layers[layer][area][obs_idx].len()).rev() {
                let node_idx = self.graph.alternative_layers[layer][area][obs_idx][node_i];
                let required = self.graph.nodes[node_idx.0 as usize].required_state;

                if (required & new_state) == required {
                    // Move to active
                    self.graph.alternative_layers[layer][area][obs_idx].remove(node_i);
                    self.graph.layers[layer][area][obs_idx].push(node_idx);
                    Self::update_node_links_for_state_static(
                        &mut self.graph.nodes,
                        &self.graph.static_data.links,
                        node_idx,
                        new_state,
                    );
                }
            }
        }

        // Motion-obstacle activation swap: flip each obstacle's
        // `active` flag and record freshly-active obstacles into
        // `appeared`.
        if let Some(area_ref) = self
            .graph
            .static_mut()
            .move_layers
            .get_mut(layer)
            .and_then(|l| l.get_mut(area))
        {
            for obs in &mut area_ref.motion_obstacles {
                let should_be_active = (obs.state_id & new_state) == obs.state_id;
                if obs.active != should_be_active {
                    obs.active = should_be_active;
                    changed = true;
                    if should_be_active {
                        appeared.push(AppearedObstacle {
                            bounding_box: obs.bounding_box,
                            polygon: obs.polygon.clone(),
                        });
                    }
                    // Grid-line sync: keep the fast-find grid's
                    // `line_active` flags in sync with the obstacle's
                    // active state.
                    for &line_idx in &obs.grid_line_indices {
                        line_toggles.push((line_idx, should_be_active));
                    }
                }
            }
        }

        changed
    }

    /// Toggle a specific obstacle's state bit.
    ///
    /// Pushes bounding boxes of motion obstacles that transitioned from
    /// inactive → active into `appeared` (used downstream for actor
    /// kills on cells that just became blocked).
    pub fn toggle_obstacle_state(
        &mut self,
        layer: usize,
        area: usize,
        changing_obstacle: u16,
        appeared: &mut Vec<AppearedObstacle>,
        line_toggles: &mut Vec<(crate::fast_find_grid::LineIndex, bool)>,
    ) -> bool {
        let bit = 1u32 << ((changing_obstacle as u32) << 1);
        let current = self.graph.states[layer][area];
        let new_state = if (current & bit) != 0 {
            current + bit
        } else {
            current.wrapping_sub(bit)
        };
        self.set_state_area_with_appeared(layer, area, new_state, appeared, line_toggles)
    }

    /// Initialize all area states to the given value.
    pub fn initialize_all_states(&mut self, state: u32) {
        for layer_states in &mut self.graph.states {
            for area_state in layer_states.iter_mut() {
                *area_state = state;
            }
        }
    }

    /// Apply all current states (move nodes/links accordingly).
    pub fn set_states_all(&mut self) {
        let num_layers = self.graph.layers.len();
        for layer in 0..num_layers {
            let num_areas = self.graph.layers[layer].len();
            for area in 0..num_areas {
                let state = self.graph.states[layer][area];
                self.set_state_area(layer, area, state);
            }
        }
    }

    /// Initialize to the default state (alternating bits = 0x55555555).
    pub fn initialize(&mut self) {
        self.initialize_all_states(0x5555_5555); // 1010101010... in binary
        self.set_states_all();
    }

    /// Merge all alternative nodes back into the active graph.
    pub fn merge_states(&mut self) {
        self.initialize_all_states(0);

        for layer in 0..self.graph.alternative_layers.len() {
            for area in 0..self.graph.alternative_layers[layer].len() {
                for obs in 0..self.graph.alternative_layers[layer][area].len() {
                    // Move all alternative nodes back to active
                    while let Some(node_idx) = self.graph.alternative_layers[layer][area][obs].pop()
                    {
                        self.graph.layers[layer][area][obs].push(node_idx);

                        // Move all alternative links back to active
                        let node = &mut self.graph.nodes[node_idx.0 as usize];
                        node.link_indices.append(&mut node.alternative_link_indices);
                    }
                }
            }
        }
    }

    // ── Debug visualisation ──────────────────────────────────────

    /// Iterate every drawable graph link segment for the motion-graph
    /// debug overlay. Walks layers→areas→obstacles→nodes, and for each
    /// outbound link iterates its link-configuration's
    /// start/destination docking pairs at `half_diagonal_idx`. The
    /// closure receives world-space endpoints plus a colour: red
    /// (`0xFA00`) when the link requires a non-zero obstacle state,
    /// dim blue (`0x4BBA`) otherwise. `view_rect` is used only as a
    /// cheap pre-filter; final clipping is the caller's job (typically
    /// the GPU framebuffer).
    pub fn draw_graph<F: FnMut(GeoPoint2D, GeoPoint2D, u16)>(
        &self,
        view_rect: BBox2D,
        half_diagonal_idx: u16,
        mut emit: F,
    ) {
        let static_data = &*self.graph.static_data;
        let hd_idx = half_diagonal_idx as usize;
        let Some(&half_diagonal) = static_data.half_diagonals.get(hd_idx) else {
            return;
        };

        for layer in &self.graph.layers {
            for area in layer {
                for obstacle in area {
                    for &node_idx in obstacle {
                        let node = &self.graph.nodes[node_idx.0 as usize];
                        for &link_idx in &node.link_indices {
                            let link = &static_data.links[link_idx.0 as usize];
                            // Skip dead-config sentinel
                            // (`start_configurations == 255`), stored
                            // as `None` here.
                            let Some(cfg_idx) = link.config_indices.get(hd_idx).copied().flatten()
                            else {
                                continue;
                            };
                            let cfg = &static_data.link_configs[cfg_idx.0 as usize];

                            let color: u16 = if link.required_state != 0 {
                                0xFA00
                            } else {
                                0x4BBA
                            };

                            for (&start_place, &dest_place) in cfg
                                .start_config_list
                                .iter()
                                .zip(cfg.destination_config_list.iter())
                            {
                                let p1 =
                                    self.docking_point_geo(node_idx, start_place, half_diagonal);
                                let p2 = self.docking_point_geo(
                                    link.next_node,
                                    dest_place,
                                    half_diagonal,
                                );
                                if !view_rect.intersects_segment(geo2d::segment(p1, p2)) {
                                    continue;
                                }
                                emit(p1, p2, color);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Iterate every drawable node-corner stub for the motion-graph
    /// debug overlay. Walks layers→areas→obstacles→nodes and, for
    /// each node whose position lies inside `view_rect`, emits a
    /// 10-unit diagonal stub for every set bit in
    /// `configurations[half_diagonal_idx]` — `TOP_LEFT`→(-10,-10),
    /// `TOP_RIGHT`→(10,-10), `BOTTOM_LEFT`→(-10,10),
    /// `BOTTOM_RIGHT`→(10,10). All stubs are white (`0xFFFF`).
    pub fn draw_nodes<F: FnMut(GeoPoint2D, GeoPoint2D, u16)>(
        &self,
        view_rect: BBox2D,
        half_diagonal_idx: u16,
        mut emit: F,
    ) {
        let hd_idx = half_diagonal_idx as usize;
        const STUBS: [(u8, f32, f32); 4] = [
            (TOP_LEFT, -10.0, -10.0),
            (TOP_RIGHT, 10.0, -10.0),
            (BOTTOM_LEFT, -10.0, 10.0),
            (BOTTOM_RIGHT, 10.0, 10.0),
        ];

        for layer in &self.graph.layers {
            for area in layer {
                for obstacle in area {
                    for &node_idx in obstacle {
                        let node = &self.graph.nodes[node_idx.0 as usize];
                        if !view_rect.contains_point(node.position) {
                            continue;
                        }
                        let Some(&config) = node.configurations.get(hd_idx) else {
                            continue;
                        };
                        for &(bit, dx, dy) in &STUBS {
                            if (config & bit) != 0 {
                                let q = pt(node.position.x + dx, node.position.y + dy);
                                emit(node.position, q, 0xFFFF);
                            }
                        }
                    }
                }
            }
        }
    }

    // ── Link state helpers ───────────────────────────────────────

    /// Update a node's links: move matching alternatives to active and vice versa.
    /// Static version to avoid borrow checker issues when graph layers are also borrowed.
    ///
    /// Uses `Vec::remove(i)` (not `swap_remove(i)`) so surviving link
    /// order is preserved. The open-list heap still picks shortest
    /// paths, but deterministic A* tiebreaks between equal-cost edges
    /// depend on stable per-node link iteration order.
    fn update_node_links_for_state_static(
        nodes: &mut [PathGraphNode],
        links: &[PathGraphLink],
        node_idx: NodeIdx,
        state: u32,
    ) {
        let node = &mut nodes[node_idx.0 as usize];

        // Active → alternative
        let mut i = 0;
        while i < node.link_indices.len() {
            let link_idx = node.link_indices[i];
            let link = &links[link_idx.0 as usize];
            if (link.required_state & state) != link.required_state {
                node.alternative_link_indices.push(link_idx);
                node.link_indices.remove(i);
            } else {
                i += 1;
            }
        }

        // Alternative → active
        let mut i = 0;
        while i < node.alternative_link_indices.len() {
            let link_idx = node.alternative_link_indices[i];
            let link = &links[link_idx.0 as usize];
            if (link.required_state & state) == link.required_state {
                node.link_indices.push(link_idx);
                node.alternative_link_indices.remove(i);
            } else {
                i += 1;
            }
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_docking_place_count() {
        assert_eq!(number_of_places(0), 0);
        assert_eq!(number_of_places(TOP_LEFT), 1);
        assert_eq!(number_of_places(TOP_LEFT | TOP_RIGHT), 2);
        assert_eq!(number_of_places(TOP_LEFT | BOTTOM_RIGHT), 2);
        assert_eq!(number_of_places(TOP_LEFT | TOP_RIGHT | BOTTOM_LEFT), 3);
        assert_eq!(
            number_of_places(TOP_LEFT | TOP_RIGHT | BOTTOM_LEFT | BOTTOM_RIGHT),
            4
        );
    }

    #[test]
    fn test_next_docking_place() {
        // Clockwise: TL → TR → BR → BL → TL
        assert_eq!(next_docking_place(TOP_LEFT, true), TOP_RIGHT);
        assert_eq!(next_docking_place(TOP_RIGHT, true), BOTTOM_RIGHT);
        assert_eq!(next_docking_place(BOTTOM_RIGHT, true), BOTTOM_LEFT);
        assert_eq!(next_docking_place(BOTTOM_LEFT, true), TOP_LEFT);

        // Counter-clockwise: TL → BL → BR → TR → TL
        assert_eq!(next_docking_place(TOP_LEFT, false), BOTTOM_LEFT);
        assert_eq!(next_docking_place(BOTTOM_LEFT, false), BOTTOM_RIGHT);
        assert_eq!(next_docking_place(BOTTOM_RIGHT, false), TOP_RIGHT);
        assert_eq!(next_docking_place(TOP_RIGHT, false), TOP_LEFT);
    }

    #[test]
    fn test_docking_point_positions() {
        let mut runtime = PathFinderRuntime::new();
        runtime.graph.nodes.push(PathGraphNode {
            position: pt(100.0, 100.0),
            vector_to_node: pt(0.0, 0.0),
            vector_from_node: pt(0.0, 0.0),
            required_state: 0,
            configurations: vec![15], // All four places
            link_indices: Vec::new(),
            alternative_link_indices: Vec::new(),
            visited: false,
            distance_from_source: 0.0,
            distance_to_goal: 0.0,
            score: 0.0,
            previous_link_on_path: None,
            leave_place: 0,
            enter_place: 0,
        });

        let node = NodeIdx(0);
        let hd = pt(10.0, 8.0);

        assert_eq!(
            runtime.docking_point(node, TOP_LEFT, hd),
            MapPoint::new(90.0, 92.0)
        );
        assert_eq!(
            runtime.docking_point(node, TOP_RIGHT, hd),
            MapPoint::new(110.0, 92.0)
        );
        assert_eq!(
            runtime.docking_point(node, BOTTOM_LEFT, hd),
            MapPoint::new(90.0, 108.0)
        );
        assert_eq!(
            runtime.docking_point(node, BOTTOM_RIGHT, hd),
            MapPoint::new(110.0, 108.0)
        );
    }

    #[test]
    fn test_estimate_distance() {
        let d = PathFinderRuntime::estimate_distance(pt(0.0, 0.0), pt(3.0, 4.0));
        assert!((d - 5.0).abs() < 1e-6);
    }

    #[test]
    fn test_is_reachable_fast_no_skeleton() {
        let mut runtime = PathFinderRuntime::new();
        // Empty motion area — everything is reachable
        runtime
            .graph
            .static_mut()
            .move_layers
            .push(vec![MotionArea {
                polygon: Vec::new(),
                skeleton: Vec::new(),
                motion_obstacles: Vec::new(),
            }]);
        runtime.current_motion_area = (0, 0);

        assert!(runtime.is_reachable_fast(MapPoint::new(0.0, 0.0), MapPoint::new(100.0, 100.0)));
    }

    #[test]
    fn test_is_reachable_fast_with_skeleton() {
        let mut runtime = PathFinderRuntime::new();
        // Horizontal skeleton line at y=50
        runtime
            .graph
            .static_mut()
            .move_layers
            .push(vec![MotionArea {
                polygon: Vec::new(),
                skeleton: vec![geo2d::segment(pt(0.0, 50.0), pt(200.0, 50.0))],
                motion_obstacles: Vec::new(),
            }]);
        runtime.current_motion_area = (0, 0);

        // Movement that crosses the skeleton
        assert!(!runtime.is_reachable_fast(MapPoint::new(50.0, 30.0), MapPoint::new(50.0, 70.0)));

        // Movement that doesn't cross
        assert!(runtime.is_reachable_fast(MapPoint::new(50.0, 30.0), MapPoint::new(100.0, 30.0)));
    }

    #[test]
    fn test_sector_conversion() {
        let mut graph = PathGraph::new();
        let static_data = graph.static_mut();
        static_data
            .sector_conversion
            .push(SectorToArea { sector: 0, area: 0 });
        static_data
            .sector_conversion
            .push(SectorToArea { sector: 5, area: 1 });
        static_data.sector_conversion.push(SectorToArea {
            sector: 10,
            area: 2,
        });

        assert_eq!(graph.convert_sector(0), 0);
        assert_eq!(graph.convert_sector(5), 1);
        assert_eq!(graph.convert_sector(10), 2);
    }

    #[test]
    #[should_panic(expected = "ConvertSector failed")]
    fn test_sector_conversion_missing() {
        let graph = PathGraph::new();
        graph.convert_sector(42); // Should panic
    }

    #[test]
    #[should_panic(expected = "ConvertSector failed")]
    fn test_find_path_missing_sector_conversion_panics() {
        let pf = PathFinder::new();
        let graph = PathGraph::new();
        pf.find_path(
            &graph,
            &FastFindGrid::new(),
            0,
            42,
            0,
            MapPoint::new(0.0, 0.0),
            MapPoint::new(1.0, 1.0),
            false,
        );
    }

    /// Regression for the set-state-sector fix: the cumulative
    /// obstacle count (sector) must be run through `convert_sector`
    /// before being used as an area index. On a multi-area, multi-
    /// obstacle layout the mapping is non-identity, so feeding the raw
    /// sector as an area would toggle the wrong graph area.
    #[test]
    fn test_set_state_sector_converts_via_build_sector_conversion() {
        let mut pf = PathFinder::new();
        let mut graph = PathGraph::new();

        // One layer, three areas. Obstacle counts per area: 2, 1, 0.
        // `build_sector_conversion` walks areas in order and emits
        // `(sector = cumulative count, area = vec position)`:
        //   area 0 → sector 0 (count += 2 + 1 = 3)
        //   area 1 → sector 3 (count += 1 + 1 = 5)
        //   area 2 → sector 5
        // So the raw sector values differ from the area indices.
        let make_obstacle = || MotionObstacle {
            state_id: 0,
            active: false,
            bounding_box: crate::geo2d::BBox2D::default(),
            polygon: Vec::new(),
            grid_line_indices: Vec::new(),
        };
        let make_area = |n_obstacles: usize| MotionArea {
            polygon: Vec::new(),
            skeleton: Vec::new(),
            motion_obstacles: (0..n_obstacles).map(|_| make_obstacle()).collect(),
        };

        graph
            .static_mut()
            .move_layers
            .push(vec![make_area(2), make_area(1), make_area(0)]);

        graph.build_sector_conversion();

        // Conversion table is the heart of the fix: sector IDs (cumulative
        // obstacle counts) must map to the correct area indices.
        assert_eq!(graph.try_convert_sector(0), Some(0));
        assert_eq!(graph.try_convert_sector(3), Some(1));
        assert_eq!(graph.try_convert_sector(5), Some(2));
        // Raw cumulative values between anchors don't identify any area.
        assert_eq!(graph.try_convert_sector(1), None);
        assert_eq!(graph.try_convert_sector(4), None);

        // Set up minimal per-area state storage so `toggle_obstacle_state`
        // can record the flip; layers/alternative_layers must have one
        // slot per obstacle inside each area.
        graph.layers = vec![vec![
            vec![Vec::new(), Vec::new()], // area 0: 2 obstacles
            vec![Vec::new()],             // area 1: 1 obstacle
            vec![],                       // area 2: 0 obstacles
        ]];
        graph.alternative_layers =
            vec![vec![vec![Vec::new(), Vec::new()], vec![Vec::new()], vec![]]];
        graph.states = vec![vec![0, 0, 0]];
        pf.states = graph.states.clone();

        // A patch whose stream-deserialised `pathfinder_sector` is 3
        // targets area 1. If `toggle_obstacle_state` were called with
        // the raw sector (3), it would index past the end of the layer
        // (only 3 areas) or corrupt area 2; with the conversion it
        // flips the correct state for area 1.
        let sector = 3u16;
        let area = graph.try_convert_sector(sector).unwrap() as usize;
        assert_eq!(area, 1);

        let mut appeared = Vec::new();
        let mut line_toggles = Vec::new();
        pf.toggle_obstacle_state(&graph, 0, area, 0, &mut appeared, &mut line_toggles);
        assert_ne!(
            pf.states[0][1], 0,
            "toggling obstacle 0 in converted area 1 must mutate its state"
        );
        assert_eq!(
            pf.states[0][0], 0,
            "area 0 must not be touched when toggling area 1"
        );
        assert_eq!(
            pf.states[0][2], 0,
            "area 2 must not be touched when toggling area 1"
        );
    }

    #[test]
    fn test_state_management_basics() {
        let mut runtime = PathFinderRuntime::new();

        // Set up a minimal graph with one layer, one area, one obstacle, two nodes
        runtime.graph.nodes.push(PathGraphNode {
            position: pt(10.0, 10.0),
            vector_to_node: pt(0.0, 0.0),
            vector_from_node: pt(0.0, 0.0),
            required_state: 0, // Always active (matches any state)
            configurations: vec![15],
            link_indices: Vec::new(),
            alternative_link_indices: Vec::new(),
            visited: false,
            distance_from_source: 0.0,
            distance_to_goal: 0.0,
            score: 0.0,
            previous_link_on_path: None,
            leave_place: 0,
            enter_place: 0,
        });
        runtime.graph.nodes.push(PathGraphNode {
            position: pt(50.0, 50.0),
            vector_to_node: pt(0.0, 0.0),
            vector_from_node: pt(0.0, 0.0),
            required_state: 1, // Only active when bit 0 is set
            configurations: vec![15],
            link_indices: Vec::new(),
            alternative_link_indices: Vec::new(),
            visited: false,
            distance_from_source: 0.0,
            distance_to_goal: 0.0,
            score: 0.0,
            previous_link_on_path: None,
            leave_place: 0,
            enter_place: 0,
        });

        runtime.graph.layers = vec![vec![vec![vec![NodeIdx(0), NodeIdx(1)]]]];
        runtime.graph.alternative_layers = vec![vec![vec![Vec::new()]]];
        runtime.graph.states = vec![vec![1]]; // State = 1 (bit 0 set)

        // Both nodes should be active with state=1
        assert_eq!(runtime.graph.layers[0][0][0].len(), 2);

        // Change state to 0 — node 1 (required_state=1) should move to alternative
        runtime.set_state_area(0, 0, 0);
        assert_eq!(runtime.graph.layers[0][0][0].len(), 1);
        assert_eq!(runtime.graph.alternative_layers[0][0][0].len(), 1);

        // Change state back to 1 — node 1 should return
        runtime.set_state_area(0, 0, 1);
        assert_eq!(runtime.graph.layers[0][0][0].len(), 2);
        assert_eq!(runtime.graph.alternative_layers[0][0][0].len(), 0);
    }

    #[test]
    fn test_find_path_direct() {
        // Test that when source and goal are directly reachable, we get a single-point path
        let mut runtime = PathFinderRuntime::new();
        let mut grid = FastFindGrid::new();
        grid.size_map(4, 4);
        grid.allocate_layers(1);

        // Set up minimal motion area with no skeleton (everything reachable)
        runtime
            .graph
            .static_mut()
            .move_layers
            .push(vec![MotionArea {
                polygon: Vec::new(),
                skeleton: Vec::new(),
                motion_obstacles: Vec::new(),
            }]);

        runtime.graph.static_mut().half_diagonals.push(pt(5.0, 5.0));
        runtime.graph.layers.push(vec![Vec::new()]); // One layer, one area, no obstacles
        runtime.graph.alternative_layers.push(vec![Vec::new()]);
        runtime.graph.states.push(vec![0]);
        runtime
            .graph
            .static_mut()
            .sector_conversion
            .push(SectorToArea { sector: 0, area: 0 });

        let path = runtime.find_path(
            &grid,
            0,
            0,
            0,
            MapPoint::new(50.0, 50.0),
            MapPoint::new(100.0, 50.0),
            true,
        );
        assert!(path.is_some());
        let path = path.unwrap();
        // Direct path: just source and goal
        assert_eq!(path.len(), 1); // Only goal (source is at front after reverse, but direct path returns just goal)
    }
}
