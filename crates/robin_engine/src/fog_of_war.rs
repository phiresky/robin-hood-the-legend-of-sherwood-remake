//! Deterministic player visibility and hostile-intelligence state.
//!
//! This is intentionally independent from [`crate::element::ElementData::blipped`].
//! The Original's blip flag is permanent identity discovery; fog visibility is
//! temporary and must never re-arm or otherwise reinterpret that flag.

use geo::{Coord, LineString, MultiPolygon, Polygon};
use i_overlay::core::{fill_rule::FillRule, overlay::Overlay, overlay_rule::OverlayRule};
use i_overlay::i_float::int::point::IntPoint;
use serde::{Deserialize, Serialize};

use crate::coordinates::{MapPoint, MapSize};
use crate::element::EntityId;

/// A seen hostile remains targetable for this many frames to avoid edge flicker.
pub const SPOTTED_HYSTERESIS_FRAMES: u32 = 12;
/// Listen grants a longer temporary intelligence reveal.
pub const LISTEN_REVEAL_FRAMES: u32 = 75;
/// A stationary last-known minimap marker fades over ten seconds.
pub const LAST_KNOWN_FADE_FRAMES: u32 = 250;
/// A temporary intelligence reveal clears enough ground to avoid drawing the
/// revealed actor underneath dark fog. Normal PC sight uses exact polygons.
pub const FOG_INTELLIGENCE_REVEAL_RADIUS: f32 = 12.0;
/// Malformed saves must not be able to allocate unbounded polygon geometry.
pub const MAX_FOG_REGION_VERTICES: usize = 2 * 1024 * 1024;
/// Separate ceiling for decoded entity intelligence. Real missions are many
/// orders of magnitude smaller; the bound prevents a malformed snapshot from
/// admitting an effectively unbounded sorted side table.
pub const MAX_FOG_INTELLIGENCE_ENTRIES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FogCellState {
    Unexplored,
    Explored,
    Visible,
}

/// One serialized component of a fog region in projected map coordinates.
/// Rings omit the duplicate closing vertex used by `geo::LineString`.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct FogPolygon {
    pub exterior: Vec<MapPoint>,
    pub interiors: Vec<Vec<MapPoint>>,
}

/// A renderer-independent polygon union. This is authoritative simulation
/// state; textures are derived presentation caches and are never serialized.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct FogRegion {
    polygons: Vec<FogPolygon>,
}

/// Presentation-geometry cache for an observer whose eye and surrounding
/// obstacle mesh have not changed. It is deliberately absent from saves,
/// rollback snapshots, state hashes, and equality: restored state simply
/// rebuilds the same polygons on its next fog scan.
#[derive(Debug, Clone, Default)]
pub(crate) struct FogScanCache {
    pub(crate) entries: Vec<FogScanCacheEntry>,
}

impl PartialEq for FogScanCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FogScanCacheEntry {
    pub(crate) source_key: [u32; 12],
    pub(crate) obstacle_key: Vec<u32>,
    pub(crate) visible: FogRegion,
    pub(crate) visible_projection: FogRegion,
    pub(crate) prepared_surfaces: std::sync::Arc<crate::engine::fog_of_war::PreparedFogSurfaces>,
}

impl FogRegion {
    pub fn polygons(&self) -> &[FogPolygon] {
        &self.polygons
    }

    pub fn is_empty(&self) -> bool {
        self.polygons.is_empty()
    }

    pub fn contains(&self, point: MapPoint) -> bool {
        point.x.is_finite()
            && point.y.is_finite()
            && self.polygons.iter().any(|polygon| {
                ring_contains(&polygon.exterior, point)
                    && !polygon
                        .interiors
                        .iter()
                        .any(|interior| ring_contains(interior, point))
            })
    }

    pub(crate) fn from_geo(region: &MultiPolygon<f32>) -> Self {
        Self {
            polygons: region
                .0
                .iter()
                .filter_map(|polygon| {
                    let exterior = ring_from_geo(polygon.exterior());
                    (exterior.len() >= 3).then(|| FogPolygon {
                        exterior,
                        interiors: polygon.interiors().iter().map(ring_from_geo).collect(),
                    })
                })
                .collect(),
        }
    }

    pub(crate) fn to_geo(&self) -> MultiPolygon<f32> {
        MultiPolygon::new(
            self.polygons
                .iter()
                .map(|polygon| {
                    Polygon::new(
                        ring_to_geo(&polygon.exterior),
                        polygon
                            .interiors
                            .iter()
                            .map(|ring| ring_to_geo(ring))
                            .collect(),
                    )
                })
                .collect(),
        )
    }

    fn union_geo(&mut self, region: &MultiPolygon<f32>) {
        if region.0.is_empty() {
            return;
        }
        // Use the same Boolean engine as geo, but with a stable subpixel
        // coordinate system. Auto-scaled floating overlays followed by f32
        // storage move shared edges on each scan and grow microscopic holes.
        // This is still vector geometry, not a bitmap: precision is 1/256 map
        // pixel, with no area filtering or simplification of real openings.
        const SCALE: f32 = 256.0;
        let point = |x: f32, y: f32| {
            assert!(
                x.is_finite() && y.is_finite() && x.abs() <= 65536.0 && y.abs() <= 65536.0,
                "fog overlay coordinates exceed exact fixed-point storage range: ({x}, {y})"
            );
            IntPoint::new((x * SCALE).round() as i32, (y * SCALE).round() as i32)
        };
        let subject: Vec<Vec<_>> = self
            .polygons
            .iter()
            .flat_map(|p| std::iter::once(&p.exterior).chain(p.interiors.iter()))
            .map(|ring| ring.iter().map(|p| point(p.x, p.y)).collect())
            .collect();
        let clip: Vec<Vec<_>> = region
            .0
            .iter()
            .flat_map(|p| std::iter::once(p.exterior()).chain(p.interiors().iter()))
            .map(|ring| ring.0.iter().map(|p| point(p.x, p.y)).collect())
            .collect();
        let mut overlay = Overlay::with_contours(&subject, &clip);
        overlay.options.ogc = true;
        self.polygons = overlay
            .overlay(OverlayRule::Union, FillRule::EvenOdd)
            .into_iter()
            .map(|shape| {
                let mut rings = shape.into_iter().map(|ring| {
                    ring.into_iter()
                        .map(|p| MapPoint::new(p.x as f32 / SCALE, p.y as f32 / SCALE))
                        .collect()
                });
                FogPolygon {
                    exterior: rings
                        .next()
                        .expect("polygon overlay returned an empty shape"),
                    interiors: rings.collect(),
                }
            })
            .collect();
    }

    fn validate(&self, name: &str, level_size: MapSize) -> Result<usize, String> {
        let mut vertices = 0usize;
        for (polygon_index, polygon) in self.polygons.iter().enumerate() {
            validate_ring(
                name,
                polygon_index,
                "exterior",
                &polygon.exterior,
                level_size,
            )?;
            vertices = vertices
                .checked_add(polygon.exterior.len())
                .ok_or_else(|| format!("{name} fog vertex count overflowed"))?;
            for (interior_index, interior) in polygon.interiors.iter().enumerate() {
                validate_ring(
                    name,
                    polygon_index,
                    &format!("interior {interior_index}"),
                    interior,
                    level_size,
                )?;
                vertices = vertices
                    .checked_add(interior.len())
                    .ok_or_else(|| format!("{name} fog vertex count overflowed"))?;
            }
            if vertices > MAX_FOG_REGION_VERTICES {
                return Err(format!(
                    "{name} fog region has {vertices} vertices, exceeding the {MAX_FOG_REGION_VERTICES} limit"
                ));
            }
        }
        Ok(vertices)
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct EntityIntelligence {
    pub entity_id: EntityId,
    pub last_known_position: MapPoint,
    pub last_seen_frame: u32,
    /// Exclusive visibility deadline. A reveal at frame `f` for `n` frames is
    /// visible for exactly `f..f+n`.
    pub visible_until_frame: u32,
    /// Whether this sighting leaves a fading hostile minimap marker. Listen
    /// may temporarily reveal non-human objects without inventing one.
    pub track_last_known: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LastKnownMarker {
    pub entity_id: EntityId,
    pub position: MapPoint,
    /// 0 is transparent and 255 is fully opaque.
    pub alpha: u8,
}

/// Three-state world/minimap fog plus temporary hostile intelligence.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct FogOfWarState {
    level_size: MapSize,
    explored: FogRegion,
    visible: FogRegion,
    explored_projection: FogRegion,
    visible_projection: FogRegion,
    intelligence: Vec<EntityIntelligence>,
    generation: u32,
    #[bitcode(skip)]
    #[state_hash(skip)]
    pub(crate) scan_cache: FogScanCache,
}

impl serde::Serialize for FogOfWarState {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PersistedFogOfWarState::capture(self).serialize(serializer)
    }
}
impl<'de> serde::Deserialize<'de> for FogOfWarState {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(PersistedFogOfWarState::deserialize(deserializer)?.into_runtime())
    }
}

/// Explicit save-owned projection; process-local state is reconstructed here,
/// independently of raw rollback cloning and the native wire codec.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct PersistedFogOfWarState {
    level_size: MapSize,

    explored: FogRegion,

    visible: FogRegion,

    explored_projection: FogRegion,

    visible_projection: FogRegion,

    intelligence: Vec<EntityIntelligence>,

    generation: u32,
}

impl PersistedFogOfWarState {
    pub(crate) fn capture(value: &FogOfWarState) -> Self {
        let FogOfWarState {
            level_size: _,
            explored: _,
            visible: _,
            explored_projection: _,
            visible_projection: _,
            intelligence: _,
            generation: _,
            scan_cache: _,
        } = value;
        Self {
            level_size: value.level_size,
            explored: value.explored.clone(),
            visible: value.visible.clone(),
            explored_projection: value.explored_projection.clone(),
            visible_projection: value.visible_projection.clone(),
            intelligence: value.intelligence.clone(),
            generation: value.generation,
        }
    }

    pub(crate) fn into_runtime(self) -> FogOfWarState {
        FogOfWarState {
            level_size: self.level_size,
            explored: self.explored,
            visible: self.visible,
            explored_projection: self.explored_projection,
            visible_projection: self.visible_projection,
            intelligence: self.intelligence,
            generation: self.generation,
            scan_cache: FogScanCache::default(),
        }
    }
}

impl FogOfWarState {
    pub fn is_initialized(&self) -> bool {
        self.level_size.x > 0.0 && self.level_size.y > 0.0
    }

    pub fn level_size(&self) -> MapSize {
        self.level_size
    }

    pub fn explored_region(&self) -> &FogRegion {
        &self.explored
    }

    pub fn visible_region(&self) -> &FogRegion {
        &self.visible
    }

    /// Projected world pixels which presentation may expose. This includes
    /// visible terrain plus the front faces of occluding architecture; entity
    /// disclosure deliberately continues to query [`Self::visible_region`].
    pub fn explored_projection_region(&self) -> &FogRegion {
        &self.explored_projection
    }

    pub fn visible_projection_region(&self) -> &FogRegion {
        &self.visible_projection
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    pub fn initialize(&mut self, level_size: MapSize) {
        validate_level_size(level_size)
            .unwrap_or_else(|error| panic!("cannot initialize polygon fog: {error}"));
        *self = Self {
            level_size,
            explored: FogRegion::default(),
            visible: FogRegion::default(),
            explored_projection: FogRegion::default(),
            visible_projection: FogRegion::default(),
            intelligence: Vec::new(),
            generation: self.generation.wrapping_add(1),
            scan_cache: FogScanCache::default(),
        };
    }

    pub(crate) fn begin_scan(&mut self) -> (FogRegion, FogRegion) {
        (
            std::mem::take(&mut self.visible),
            std::mem::take(&mut self.visible_projection),
        )
    }

    pub(crate) fn add_visible_region(&mut self, region: &MultiPolygon<f32>) {
        self.visible.union_geo(region);
        // Ground visibility controls actors and the minimap. The world mask
        // must only receive camera-owned surfaces: ground seen through an
        // opening can project underneath an unseen wall or roof.
    }

    pub(crate) fn add_visible_projection_region(&mut self, region: &MultiPolygon<f32>) {
        self.visible_projection.union_geo(region);
    }

    pub(crate) fn finish_scan(
        &mut self,
        (previous_visible, previous_projection): (FogRegion, FogRegion),
    ) {
        // The preceding scan already inserted its visible regions into history.
        // Unchanged views cannot reveal anything new; avoid overlaying the whole
        // explored map again (and avoid needless renderer texture uploads).
        let ground_changed = previous_visible != self.visible;
        let projection_changed = previous_projection != self.visible_projection;
        if ground_changed {
            self.explored.union_geo(&self.visible.to_geo());
        }
        if projection_changed {
            self.explored_projection
                .union_geo(&self.visible_projection.to_geo());
        }
        if ground_changed || projection_changed {
            self.generation = self.generation.wrapping_add(1);
        }
    }

    pub fn cell_state(&self, position: MapPoint) -> FogCellState {
        if !self.position_in_level(position) {
            FogCellState::Unexplored
        } else if self.visible.contains(position) {
            FogCellState::Visible
        } else if self.explored.contains(position) {
            FogCellState::Explored
        } else {
            FogCellState::Unexplored
        }
    }

    pub(crate) fn mark_position_visible(&mut self, position: MapPoint) -> bool {
        if !self.position_in_level(position) {
            return false;
        }
        self.visible
            .union_geo(&circle_region(position, FOG_INTELLIGENCE_REVEAL_RADIUS));
        self.visible_projection
            .union_geo(&circle_region(position, FOG_INTELLIGENCE_REVEAL_RADIUS));
        true
    }

    /// Expose a position outside the periodic scan (currently Listen's
    /// one-shot reveal) and immediately preserve it as explored.
    pub(crate) fn reveal_position_now(&mut self, position: MapPoint) -> bool {
        if !self.position_in_level(position) {
            return false;
        }
        let region = circle_region(position, FOG_INTELLIGENCE_REVEAL_RADIUS);
        self.visible.union_geo(&region);
        self.explored.union_geo(&region);
        self.visible_projection.union_geo(&region);
        self.explored_projection.union_geo(&region);
        self.generation = self.generation.wrapping_add(1);
        true
    }

    pub(crate) fn remember_visible(
        &mut self,
        entity_id: EntityId,
        position: MapPoint,
        frame: u32,
        reveal_frames: u32,
        track_last_known: bool,
    ) {
        let deadline = frame.saturating_add(reveal_frames);
        match self
            .intelligence
            .binary_search_by_key(&entity_id, |entry| entry.entity_id)
        {
            Ok(index) => {
                let entry = &mut self.intelligence[index];
                entry.last_known_position = position;
                entry.last_seen_frame = frame;
                entry.visible_until_frame = entry.visible_until_frame.max(deadline);
                entry.track_last_known |= track_last_known;
            }
            Err(index) => self.intelligence.insert(
                index,
                EntityIntelligence {
                    entity_id,
                    last_known_position: position,
                    last_seen_frame: frame,
                    visible_until_frame: deadline,
                    track_last_known,
                },
            ),
        }
    }

    pub fn is_spotted(&self, entity_id: EntityId, frame: u32) -> bool {
        self.intelligence
            .binary_search_by_key(&entity_id, |entry| entry.entity_id)
            .ok()
            .is_some_and(|index| frame < self.intelligence[index].visible_until_frame)
    }

    pub(crate) fn spotted_entity_ids(&self, frame: u32) -> impl Iterator<Item = EntityId> + '_ {
        self.intelligence
            .iter()
            .filter(move |entry| frame < entry.visible_until_frame)
            .map(|entry| entry.entity_id)
    }

    pub fn last_known_markers(&self, frame: u32) -> impl Iterator<Item = LastKnownMarker> + '_ {
        self.intelligence.iter().filter_map(move |entry| {
            if !entry.track_last_known || frame < entry.visible_until_frame {
                return None;
            }
            let age = frame.saturating_sub(entry.visible_until_frame);
            if age >= LAST_KNOWN_FADE_FRAMES {
                return None;
            }
            let alpha = ((LAST_KNOWN_FADE_FRAMES - age) * 255 / LAST_KNOWN_FADE_FRAMES) as u8;
            Some(LastKnownMarker {
                entity_id: entry.entity_id,
                position: entry.last_known_position,
                alpha,
            })
        })
    }

    pub(crate) fn retain_entities(&mut self, mut exists: impl FnMut(EntityId) -> bool) {
        self.intelligence.retain(|entry| exists(entry.entity_id));
    }

    /// Reject malformed decoded state before it reaches geometry operations.
    /// Native snapshots intentionally have no old-layout adapter.
    pub fn validate(&self) -> Result<(), String> {
        if !self.is_initialized() {
            if self.level_size == MapSize::new(0.0, 0.0)
                && self.explored.is_empty()
                && self.visible.is_empty()
                && self.explored_projection.is_empty()
                && self.visible_projection.is_empty()
                && self.intelligence.is_empty()
            {
                return Ok(());
            }
            return Err("uninitialized fog state must be the exact empty 0x0 state".to_owned());
        }

        validate_level_size(self.level_size)?;
        self.explored.validate("explored", self.level_size)?;
        self.visible.validate("visible", self.level_size)?;
        self.explored_projection
            .validate("explored projection", self.level_size)?;
        self.visible_projection
            .validate("visible projection", self.level_size)?;
        if self.intelligence.len() > MAX_FOG_INTELLIGENCE_ENTRIES {
            return Err(format!(
                "fog intelligence has {} entries, exceeding the {MAX_FOG_INTELLIGENCE_ENTRIES} limit",
                self.intelligence.len()
            ));
        }
        let mut previous = None;
        for entry in &self.intelligence {
            if previous.is_some_and(|id| id >= entry.entity_id) {
                return Err("fog intelligence must be strictly sorted and unique".to_owned());
            }
            if !entry.last_known_position.x.is_finite() || !entry.last_known_position.y.is_finite()
            {
                return Err(format!(
                    "fog intelligence {:?} has a non-finite position",
                    entry.entity_id
                ));
            }
            if entry.visible_until_frame < entry.last_seen_frame {
                return Err(format!(
                    "fog intelligence {:?} has a deadline before its sighting",
                    entry.entity_id
                ));
            }
            previous = Some(entry.entity_id);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn corrupt_visible_region_for_test(&mut self) {
        self.visible.polygons.push(FogPolygon {
            exterior: vec![MapPoint::new(f32::NAN, 0.0); 3],
            interiors: Vec::new(),
        });
    }

    fn position_in_level(&self, position: MapPoint) -> bool {
        self.is_initialized()
            && position.x.is_finite()
            && position.y.is_finite()
            && position.x >= 0.0
            && position.y >= 0.0
            && position.x <= self.level_size.x
            && position.y <= self.level_size.y
    }
}

fn validate_level_size(level_size: MapSize) -> Result<(), String> {
    if !level_size.x.is_finite()
        || !level_size.y.is_finite()
        || level_size.x <= 0.0
        || level_size.y <= 0.0
    {
        return Err(format!(
            "polygon fog requires finite positive level dimensions, got {level_size:?}"
        ));
    }
    Ok(())
}

fn validate_ring(
    region_name: &str,
    polygon_index: usize,
    ring_name: &str,
    ring: &[MapPoint],
    level_size: MapSize,
) -> Result<(), String> {
    if ring.len() < 3 {
        return Err(format!(
            "{region_name} fog polygon {polygon_index} {ring_name} has fewer than three vertices"
        ));
    }
    for point in ring {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(format!(
                "{region_name} fog polygon {polygon_index} {ring_name} has a non-finite vertex"
            ));
        }
        // Shadow projection may produce tiny floating-point excursions. A
        // generous finite bound still rejects hostile decoded geometry.
        if point.x.abs() > level_size.x * 4.0 || point.y.abs() > level_size.y * 4.0 {
            return Err(format!(
                "{region_name} fog polygon {polygon_index} {ring_name} has an out-of-range vertex"
            ));
        }
    }
    Ok(())
}

fn ring_from_geo(ring: &LineString<f32>) -> Vec<MapPoint> {
    let mut points: Vec<_> = ring
        .0
        .iter()
        .map(|coord| MapPoint::new(coord.x, coord.y))
        .collect();
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    points
}

fn ring_to_geo(ring: &[MapPoint]) -> LineString<f32> {
    let mut coords: Vec<_> = ring
        .iter()
        .map(|point| Coord {
            x: point.x,
            y: point.y,
        })
        .collect();
    if let Some(&first) = coords.first() {
        coords.push(first);
    }
    LineString::new(coords)
}

fn ring_contains(ring: &[MapPoint], point: MapPoint) -> bool {
    let mut inside = false;
    for index in 0..ring.len() {
        let a = ring[index];
        let b = ring[(index + 1) % ring.len()];
        let cross = (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x);
        if cross.abs() <= 1.0e-4
            && point.x >= a.x.min(b.x) - 1.0e-4
            && point.x <= a.x.max(b.x) + 1.0e-4
            && point.y >= a.y.min(b.y) - 1.0e-4
            && point.y <= a.y.max(b.y) + 1.0e-4
        {
            return true;
        }
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
    }
    inside
}

fn circle_region(center: MapPoint, radius: f32) -> MultiPolygon<f32> {
    const SEGMENTS: usize = 32;
    let mut ring: Vec<_> = (0..SEGMENTS)
        .map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / SEGMENTS as f32;
            Coord {
                x: center.x + radius * angle.cos(),
                y: center.y + radius * angle.sin(),
            }
        })
        .collect();
    ring.push(ring[0]);
    MultiPolygon::new(vec![Polygon::new(LineString::new(ring), Vec::new())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_id::SoldierId;

    #[test]
    fn accumulating_overlapping_subpixel_views_does_not_fragment_history() {
        let mut region = FogRegion::default();
        for frame in 0..200 {
            let center = MapPoint::new(1937.0 + frame as f32 * 0.37, 1384.0);
            region.union_geo(&circle_region(center, 320.0));
        }
        assert_eq!(region.polygons.len(), 1);
        assert!(region.polygons[0].interiors.is_empty());
        assert!(region.contains(MapPoint::new(1937.0, 1384.0)));
        assert!(region.contains(MapPoint::new(2010.0, 1384.0)));
    }

    #[test]
    fn stable_overlay_preserves_subpixel_holes_and_stationary_history() {
        let ring = |x0, y0, x1, y1| {
            ring_to_geo(&[
                MapPoint::new(x0, y0),
                MapPoint::new(x1, y0),
                MapPoint::new(x1, y1),
                MapPoint::new(x0, y1),
            ])
        };
        let region = MultiPolygon::new(vec![Polygon::new(
            ring(1900.0, 1300.0, 2100.0, 1500.0),
            vec![ring(1999.875, 1390.0, 2000.125, 1410.0)],
        )]);
        let mut fog = FogOfWarState::default();
        fog.initialize(MapSize::new(3000.0, 2200.0));
        let previous = fog.begin_scan();
        fog.add_visible_projection_region(&region);
        fog.finish_scan(previous);
        let expected = fog.clone();
        for _ in 0..100 {
            let previous = fog.begin_scan();
            fog.add_visible_projection_region(&region);
            fog.finish_scan(previous);
        }
        assert_eq!(fog, expected);
        assert!(
            !fog.explored_projection
                .contains(MapPoint::new(2000.0, 1400.0))
        );
        assert!(
            fog.explored_projection
                .contains(MapPoint::new(1999.5, 1400.0))
        );
    }

    #[test]
    fn polygon_region_progresses_from_unexplored_to_visible_to_explored() {
        let mut fog = FogOfWarState::default();
        fog.initialize(MapSize::new(96.0, 96.0));
        let point = MapPoint::new(36.0, 36.0);
        assert_eq!(fog.cell_state(point), FogCellState::Unexplored);
        let previous = fog.begin_scan();
        assert!(fog.mark_position_visible(point));
        fog.finish_scan(previous);
        assert_eq!(fog.cell_state(point), FogCellState::Visible);
        let previous = fog.begin_scan();
        fog.finish_scan(previous);
        assert_eq!(fog.cell_state(point), FogCellState::Explored);
        assert_eq!(fog.generation(), 3);
    }

    #[test]
    fn last_known_marker_is_stationary_and_fades_after_hysteresis() {
        let mut fog = FogOfWarState::default();
        let id = EntityId::Soldier(SoldierId(4));
        fog.remember_visible(id, MapPoint::new(12.0, 34.0), 10, 12, true);
        assert!(fog.is_spotted(id, 21));
        assert!(!fog.is_spotted(id, 22));
        let marker = fog.last_known_markers(22).next().expect("fading marker");
        assert_eq!(marker.position, MapPoint::new(12.0, 34.0));
        assert!(marker.alpha > 0);
        assert!(
            fog.last_known_markers(22 + LAST_KNOWN_FADE_FRAMES)
                .next()
                .is_none()
        );
    }

    #[test]
    fn exact_level_boundary_is_queryable() {
        let mut fog = FogOfWarState::default();
        fog.initialize(MapSize::new(96.0, 96.0));
        let previous = fog.begin_scan();
        assert!(fog.mark_position_visible(MapPoint::new(96.0, 96.0)));
        fog.finish_scan(previous);
        assert_eq!(
            fog.cell_state(MapPoint::new(96.0, 96.0)),
            FogCellState::Visible
        );
        assert_eq!(
            fog.cell_state(MapPoint::new(96.01, 96.0)),
            FogCellState::Unexplored
        );
    }

    #[test]
    fn deterministic_state_round_trips_through_json_and_bitcode() {
        let mut fog = FogOfWarState::default();
        fog.initialize(MapSize::new(192.0, 144.0));
        let previous = fog.begin_scan();
        assert!(fog.mark_position_visible(MapPoint::new(60.0, 70.0)));
        fog.finish_scan(previous);
        fog.remember_visible(
            EntityId::Soldier(SoldierId(9)),
            MapPoint::new(80.0, 90.0),
            123,
            SPOTTED_HYSTERESIS_FRAMES,
            true,
        );
        fog.remember_visible(
            EntityId::Soldier(SoldierId(2)),
            MapPoint::new(20.0, 30.0),
            123,
            LISTEN_REVEAL_FRAMES,
            false,
        );
        fog.scan_cache.entries.push(FogScanCacheEntry {
            source_key: [7; 12],
            obstacle_key: vec![9],
            visible: FogRegion::default(),
            visible_projection: FogRegion::default(),
            prepared_surfaces: std::sync::Arc::new(
                crate::engine::fog_of_war::PreparedFogSurfaces::default(),
            ),
        });

        let json = serde_json::to_string(&fog).expect("serialize fog state");
        assert!(!json.contains("scan_cache"));
        let from_json: FogOfWarState = serde_json::from_str(&json).expect("deserialize fog state");
        assert_eq!(from_json, fog);
        assert!(from_json.scan_cache.entries.is_empty());

        let bytes = bitcode::encode(&fog);
        let from_bitcode: FogOfWarState = bitcode::decode(&bytes).expect("decode fog state");
        assert_eq!(from_bitcode, fog);
        assert!(from_bitcode.scan_cache.entries.is_empty());
        assert_eq!(bitcode::encode(&from_bitcode), bytes);
    }

    #[test]
    fn reveal_deadlines_are_exclusive_and_exact() {
        let id = EntityId::Soldier(SoldierId(1));
        let mut fog = FogOfWarState::default();
        fog.remember_visible(
            id,
            MapPoint::new(1.0, 2.0),
            100,
            SPOTTED_HYSTERESIS_FRAMES,
            true,
        );
        assert!(fog.is_spotted(id, 100));
        assert!(fog.is_spotted(id, 100 + SPOTTED_HYSTERESIS_FRAMES - 1));
        assert!(!fog.is_spotted(id, 100 + SPOTTED_HYSTERESIS_FRAMES));

        fog.remember_visible(id, MapPoint::new(3.0, 4.0), 200, LISTEN_REVEAL_FRAMES, true);
        assert!(fog.is_spotted(id, 200 + LISTEN_REVEAL_FRAMES - 1));
        assert!(!fog.is_spotted(id, 200 + LISTEN_REVEAL_FRAMES));
    }

    #[test]
    fn validate_rejects_malformed_regions_and_intelligence() {
        let mut malformed_region = FogOfWarState::default();
        malformed_region.initialize(MapSize::new(96.0, 96.0));
        malformed_region.visible.polygons.push(FogPolygon {
            exterior: vec![MapPoint::new(f32::NAN, 0.0); 3],
            interiors: Vec::new(),
        });
        assert!(
            malformed_region
                .validate()
                .unwrap_err()
                .contains("non-finite vertex")
        );

        let first = EntityId::Soldier(SoldierId(1));
        let second = EntityId::Soldier(SoldierId(2));
        let mut bad_intelligence = FogOfWarState::default();
        bad_intelligence.initialize(MapSize::new(96.0, 96.0));
        bad_intelligence.remember_visible(first, MapPoint::new(1.0, 1.0), 5, 1, true);
        bad_intelligence.remember_visible(second, MapPoint::new(2.0, 2.0), 5, 1, true);
        bad_intelligence.intelligence.swap(0, 1);
        assert!(
            bad_intelligence
                .validate()
                .unwrap_err()
                .contains("strictly sorted")
        );

        bad_intelligence
            .intelligence
            .sort_by_key(|entry| entry.entity_id);
        bad_intelligence.intelligence[0].last_known_position.x = f32::NAN;
        assert!(
            bad_intelligence
                .validate()
                .unwrap_err()
                .contains("non-finite position")
        );

        bad_intelligence.intelligence[0].last_known_position.x = 1.0;
        bad_intelligence.intelligence[0].visible_until_frame = 4;
        assert!(
            bad_intelligence
                .validate()
                .unwrap_err()
                .contains("deadline before")
        );
    }

    #[test]
    fn level_size_rejects_non_finite_and_empty_dimensions() {
        assert!(validate_level_size(MapSize::new(f32::NAN, 1.0)).is_err());
        assert!(validate_level_size(MapSize::new(0.0, 1.0)).is_err());
    }
}
