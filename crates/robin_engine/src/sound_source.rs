//! Sound source management.
//!
//! Sound sources are positioned emitters placed in levels — ambient loops,
//! one-shot effects, delayed repetitions, and volatile (play-once-then-delete)
//! sounds. The [`SoundSourceManager`] owns them and provides lifecycle
//! operations.

use serde::{Deserialize, Serialize};

use crate::geo2d::{self, GeoPoint2D};
use crate::position_interface::{ASPECT_RATIO, vector_norm_iso};
use crate::sound_geometry::{SoundSourceAltitude, SoundSourceInfo};

// ---------------------------------------------------------------------------
// SoundSourceKind
// ---------------------------------------------------------------------------

/// How a sound source is played.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum SoundSourceKind {
    /// Played once then stopped.
    Single = 0,
    /// Played continuously without delay.
    Looped = 1,
    /// Played many times with variable delay.
    Delayed = 2,
    /// Played once then deleted from the source manager.
    Volatile = 3,
}

impl SoundSourceKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Single),
            1 => Some(Self::Looped),
            2 => Some(Self::Delayed),
            3 => Some(Self::Volatile),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// SoundSource
// ---------------------------------------------------------------------------

/// A positioned sound emitter in the game world.
///
/// All fields are included in save/load state via serde.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SoundSource {
    /// Ambience bitmask filter — determines which level ambiences include this source.
    pub ambiences: u32,
    /// How this source plays.
    pub source_kind: SoundSourceKind,
    /// Sound sample ID (index into the sound source cache).
    pub id: u32,
    /// If true, this is a global (non-positioned) source — audible everywhere.
    pub is_global: bool,
    /// Inner distance: full-volume zone radius.
    pub inner_distance: u16,
    /// Outer distance: silence zone boundary.
    pub outer_distance: u16,
    /// Noise covering distance — how far this source's noise masks other sounds.
    pub noise_covering_distance: u16,
    /// Volume at inner distance \[0–255\].
    pub inner_volume: u16,
    /// Volume at outer distance \[0–255\].
    pub outer_volume: u16,
    /// Shape points defining the source geometry (polyline).
    pub shape: Vec<GeoPoint2D>,
    /// Altitude classification affecting volume with zoom.
    pub altitude: SoundSourceAltitude,
    /// Minimum delay ticks (for [`SoundSourceKind::Delayed`]).
    pub min_delay: u16,
    /// Maximum delay ticks (for [`SoundSourceKind::Delayed`]).
    pub max_delay: u16,
    /// Delay stepping granularity for random delay computation.
    pub delay_stepping: u16,
    /// Current countdown timer (for [`SoundSourceKind::Delayed`]).
    pub timer: u16,
    /// Whether this source is currently active (playing or ready to play).
    pub active: bool,
}

impl Default for SoundSource {
    fn default() -> Self {
        Self {
            ambiences: 0,
            source_kind: SoundSourceKind::Looped,
            id: u32::MAX,
            is_global: false,
            inner_distance: 0,
            outer_distance: 0,
            noise_covering_distance: 0,
            inner_volume: 0,
            outer_volume: 0,
            shape: Vec::new(),
            altitude: SoundSourceAltitude::Ground,
            min_delay: 0,
            max_delay: 0,
            delay_stepping: 1,
            timer: 0,
            active: false,
        }
    }
}

impl SoundSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if this source is part of the given ambience.

    /// Convert to [`SoundSourceInfo`] for use by the geometry engine.
    pub fn to_source_info(&self) -> SoundSourceInfo {
        SoundSourceInfo {
            is_global: self.is_global,
            altitude: self.altitude,
            inner_distance: self.inner_distance,
            outer_distance: self.outer_distance,
            inner_volume: self.inner_volume,
            outer_volume: self.outer_volume,
            shape: self.shape.clone(),
        }
    }

    // ── Geometry helpers ──────────────────────────────────────────────

    /// Distance between two points with isometric Y scaling.
    fn distance_for_point(origin: GeoPoint2D, position: GeoPoint2D) -> f32 {
        vector_norm_iso(position.x - origin.x, position.y - origin.y)
    }

    /// Perpendicular distance from `position` to segment `[seg_a, seg_b]`,
    /// using the isometric-corrected normal. Returns `None` if the
    /// perpendicular projection doesn't fall within the segment bounds.
    fn distance_to_segment(
        seg_a: GeoPoint2D,
        seg_b: GeoPoint2D,
        position: GeoPoint2D,
    ) -> Option<f32> {
        // Segment direction
        let dir = geo2d::pt(seg_b.x - seg_a.x, seg_b.y - seg_a.y);

        // Normal (perpendicular) of direction vector
        let normal_raw = geo2d::pt(-dir.y, dir.x);
        // Apply isometric Y correction, then normalize
        let corrected = geo2d::pt(normal_raw.x, normal_raw.y * ASPECT_RATIO);
        let len = (corrected.x * corrected.x + corrected.y * corrected.y).sqrt();
        if len < 1e-9 {
            return None;
        }
        let norm = geo2d::pt(corrected.x / len, corrected.y / len);

        // Line through `position` along the corrected normal
        let line_b = geo2d::pt(position.x + norm.x, position.y + norm.y);

        // Intersect segment [seg_a, seg_b] with line [position, line_b]
        let d1 = geo2d::pt(seg_b.x - seg_a.x, seg_b.y - seg_a.y);
        let d2 = geo2d::pt(line_b.x - position.x, line_b.y - position.y);

        let cross = d1.x * d2.y - d1.y * d2.x;
        if cross.abs() < 1e-9 {
            return None; // Parallel
        }

        let dp = geo2d::pt(position.x - seg_a.x, position.y - seg_a.y);
        let t = (dp.x * d2.y - dp.y * d2.x) / cross;

        if !(0.0..=1.0).contains(&t) {
            return None; // Intersection outside segment
        }

        let intersection = geo2d::pt(seg_a.x + t * d1.x, seg_a.y + t * d1.y);
        Some(Self::distance_for_point(intersection, position))
    }

    // ── Noise covering ───────────────────────────────────────────────

    /// Get noise covering volume for a given distance from this source.
    pub fn noise_covering_volume_for_distance(&self, distance: u16) -> u16 {
        let volume = self.noise_covering_distance;
        if self.is_global {
            return volume;
        }
        volume.saturating_sub(distance)
    }

    /// Get the noise covering volume at a given 3D world position.
    ///
    /// The Z coordinate is subtracted from Y for isometric projection.
    pub fn noise_covering_volume_for_3d(&self, x: f32, y: f32, z: f32) -> u16 {
        if !self.active {
            return 0;
        }

        let position = geo2d::pt(x, y - z);
        let num_points = self.shape.len();

        match num_points {
            0 => 0,
            1 => {
                let dist = Self::distance_for_point(self.shape[0], position) as u16;
                self.noise_covering_volume_for_distance(dist)
            }
            _ => {
                // Minimum distance to any shape point
                let mut min_dist: f32 = 1_000_000.0;
                for &pt in &self.shape {
                    let d = Self::distance_for_point(pt, position);
                    if d < min_dist {
                        min_dist = d;
                    }
                }

                // Check segment projection distances
                let mut prev = self.shape[0];
                for i in 1..num_points {
                    let curr = self.shape[i];
                    if let Some(d) = Self::distance_to_segment(prev, curr, position)
                        && d < min_dist
                    {
                        min_dist = d;
                    }
                    prev = curr;
                }

                self.noise_covering_volume_for_distance(min_dist as u16)
            }
        }
    }

    // ── Proto stream loading ─────────────────────────────────────────

    /// Parse a sound source from the binary proto-level stream.
    ///
    /// `new_levels` should be `true` for the full-game data format (non-demo),
    /// which includes extra padding bytes.
    pub fn from_proto_stream(data: &[u8], pos: &mut usize, new_levels: bool) -> Self {
        let mut source = SoundSource::new();

        source.id = read_u32_le(data, pos);
        source.active = read_bool(data, pos);

        let kind_byte = read_u8(data, pos);
        source.source_kind = SoundSourceKind::from_u8(kind_byte)
            .unwrap_or_else(|| panic!("Invalid sound source kind: {kind_byte}"));

        if source.source_kind == SoundSourceKind::Delayed {
            source.min_delay = read_u16_le(data, pos);
            source.max_delay = read_u16_le(data, pos);
            source.delay_stepping = read_u16_le(data, pos);
            // Pre-increment so callers can mod by `delay_stepping` directly.
            source.delay_stepping += 1;
        }

        source.is_global = read_bool(data, pos);

        if !source.is_global {
            source.inner_distance = read_u16_le(data, pos);
            source.outer_distance = read_u16_le(data, pos);

            if new_levels {
                let _dummy = read_u8(data, pos);
            }

            let num_points = read_u16_le(data, pos);
            source.shape.reserve(num_points as usize);
            for _ in 0..num_points {
                let x = read_i16_le(data, pos) as f32;
                let y = read_i16_le(data, pos) as f32;
                source.shape.push(geo2d::pt(x, y));
            }

            if new_levels {
                let _dummy = read_u8(data, pos);
            }

            // Inner volume: 0–100 range → 0–255
            let mut inner_vol = read_u16_le(data, pos);
            if inner_vol > 100 {
                inner_vol = 100;
            }
            source.inner_volume = (inner_vol as f32 * 2.55) as u16;

            // Outer volume: 0–100 range → 0–255
            let mut outer_vol = read_u16_le(data, pos);
            if outer_vol > 100 {
                outer_vol = 100;
            }
            source.outer_volume = (outer_vol as f32 * 2.55) as u16;

            source.noise_covering_distance = read_u16_le(data, pos);
        }

        let altitude_byte = read_u8(data, pos);
        source.altitude = match altitude_byte {
            0 => SoundSourceAltitude::Ground,
            1 => SoundSourceAltitude::Middle,
            2 => SoundSourceAltitude::Top,
            3 => SoundSourceAltitude::NoAltitude,
            _ => panic!("Invalid sound source altitude: {altitude_byte}"),
        };

        source.ambiences = read_u32_le(data, pos);

        source
    }
}

// ---------------------------------------------------------------------------
// SoundSourceManager
// ---------------------------------------------------------------------------

/// Manages a collection of [`SoundSource`]s.
///
/// Sound sources are stored in a flat vector. Deleted sources become `None`
/// slots so existing indices stay stable.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SoundSourceManager {
    sources: Vec<Option<SoundSource>>,
}

impl SoundSourceManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn num_sources(&self) -> usize {
        self.sources.len()
    }

    /// Get a sound source by index.
    ///
    /// Out-of-bounds indices panic; a deleted (`None`) slot returns `None`
    /// silently.
    pub fn get(&self, index: usize) -> Option<&SoundSource> {
        self.sources[index].as_ref()
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut SoundSource> {
        self.sources[index].as_mut()
    }

    /// Add a sound source (runtime creation path). Sets it to inactive.
    /// Returns its index.
    ///
    /// Pointer-identity double-registration is impossible by construction
    /// here: the source is moved in by value, so each call produces a
    /// fresh slot. Two sources sharing the same sample `id` are legal
    /// (e.g. two delayed sources of the same sample), so this
    /// deliberately does **not** dedupe by `id`.
    pub fn add(&mut self, mut source: SoundSource) -> usize {
        source.active = false;
        let idx = self.sources.len();
        self.sources.push(Some(source));
        idx
    }

    /// Push a `Some(source)` slot, preserving the source's active state.
    /// Used during proto-level loading where active state comes from the data.
    pub fn sources_push_some(&mut self, source: SoundSource) {
        self.sources.push(Some(source));
    }

    /// Push a `None` slot to preserve index alignment (for filtered-out sources).
    pub fn sources_push_none(&mut self) {
        self.sources.push(None);
    }

    /// Delete a sound source by index, returning it if present.
    /// The slot is set to `None` (not removed) to preserve indices.
    pub fn delete(&mut self, index: usize) -> Option<SoundSource> {
        if index < self.sources.len() {
            self.sources[index].take()
        } else {
            None
        }
    }

    /// Find the index of the first sound source whose sample `id` matches.
    ///
    /// **Not** an identity-based reverse lookup. Two delayed sources can
    /// legally share an `id`, so this returns the first match, not a
    /// specific instance. If you need to identify a specific registered
    /// source, remember the slot index returned by `add` at registration
    /// time or add a dedicated identity-based lookup.
    pub fn find_by_sample_id(&self, id: u32) -> Option<usize> {
        self.sources
            .iter()
            .position(|s| s.as_ref().is_some_and(|src| src.id == id))
    }

    /// Clear all sound sources.
    pub fn clear(&mut self) {
        self.sources.clear();
    }

    /// Maximum covering volume from all active sources at a 3D point.
    /// Walks every active source and returns the largest noise-covering
    /// volume at `position`. `0` when no source covers this point.
    pub fn max_noise_covering_volume_for_3d(&self, x: f32, y: f32, z: f32) -> u16 {
        let mut max = 0u16;
        for src in self.sources.iter().flatten() {
            let v = src.noise_covering_volume_for_3d(x, y, z);
            if v > max {
                max = v;
            }
        }
        max
    }

    /// Get the loop flags for all active sources, for configuring cache entries.
    /// Returns `(sample_id, should_loop)` pairs.
    pub fn get_loop_flags(&self) -> Vec<(u32, bool)> {
        self.sources
            .iter()
            .flatten()
            .map(|s| (s.id, s.source_kind == SoundSourceKind::Looped))
            .collect()
    }

    /// Iterate over all active (non-None) sources with their indices.
    pub fn iter_active(&self) -> impl Iterator<Item = (usize, &SoundSource)> {
        self.sources
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|src| (i, src)))
    }
}

// ---------------------------------------------------------------------------
// Binary reading helpers (for proto stream parsing)
// ---------------------------------------------------------------------------

fn read_u8(data: &[u8], pos: &mut usize) -> u8 {
    assert!(
        *pos < data.len(),
        "Unexpected end of data reading u8 at offset {pos}",
        pos = *pos
    );
    let v = data[*pos];
    *pos += 1;
    v
}

fn read_bool(data: &[u8], pos: &mut usize) -> bool {
    read_u8(data, pos) != 0
}

fn read_u16_le(data: &[u8], pos: &mut usize) -> u16 {
    assert!(
        *pos + 2 <= data.len(),
        "Unexpected end of data reading u16 at offset {pos}",
        pos = *pos
    );
    let v = u16::from_le_bytes([data[*pos], data[*pos + 1]]);
    *pos += 2;
    v
}

fn read_i16_le(data: &[u8], pos: &mut usize) -> i16 {
    read_u16_le(data, pos) as i16
}

fn read_u32_le(data: &[u8], pos: &mut usize) -> u32 {
    assert!(
        *pos + 4 <= data.len(),
        "Unexpected end of data reading u32 at offset {pos}",
        pos = *pos
    );
    let v = u32::from_le_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
    *pos += 4;
    v
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position_interface::INVERSE_ASPECT_RATIO;

    #[test]
    fn sound_source_default() {
        let src = SoundSource::new();
        assert_eq!(src.source_kind, SoundSourceKind::Looped);
        assert_eq!(src.id, u32::MAX);
        assert!(!src.active);
        assert!(!src.is_global);
    }

    #[test]
    fn sound_source_kind_roundtrip() {
        for i in 0..4u8 {
            let kind = SoundSourceKind::from_u8(i).unwrap();
            assert_eq!(kind as u8, i);
        }
        assert!(SoundSourceKind::from_u8(4).is_none());
    }

    #[test]
    fn noise_covering_global() {
        let mut src = SoundSource::new();
        src.is_global = true;
        src.noise_covering_distance = 100;
        // Global sources return full volume regardless of distance
        assert_eq!(src.noise_covering_volume_for_distance(50), 100);
        assert_eq!(src.noise_covering_volume_for_distance(200), 100);
    }

    #[test]
    fn noise_covering_local() {
        let mut src = SoundSource::new();
        src.is_global = false;
        src.noise_covering_distance = 100;
        assert_eq!(src.noise_covering_volume_for_distance(0), 100);
        assert_eq!(src.noise_covering_volume_for_distance(50), 50);
        assert_eq!(src.noise_covering_volume_for_distance(100), 0);
        assert_eq!(src.noise_covering_volume_for_distance(150), 0);
    }

    #[test]
    fn noise_covering_inactive() {
        let src = SoundSource::new(); // active=false by default
        assert_eq!(src.noise_covering_volume_for_3d(0.0, 0.0, 0.0), 0);
    }

    #[test]
    fn noise_covering_single_point() {
        let mut src = SoundSource::new();
        src.active = true;
        src.noise_covering_distance = 200;
        src.shape.push(geo2d::pt(100.0, 100.0));

        // At the source point, distance ~0 → full volume
        let vol = src.noise_covering_volume_for_3d(100.0, 100.0, 0.0);
        assert_eq!(vol, 200);

        // Far away → zero
        let vol = src.noise_covering_volume_for_3d(1000.0, 1000.0, 0.0);
        assert_eq!(vol, 0);
    }

    #[test]
    fn noise_covering_with_z() {
        let mut src = SoundSource::new();
        src.active = true;
        src.noise_covering_distance = 200;
        src.shape.push(geo2d::pt(100.0, 50.0));

        // Position (100, 100, 50) → projected to (100, 50) → at source
        let vol = src.noise_covering_volume_for_3d(100.0, 100.0, 50.0);
        assert_eq!(vol, 200);
    }

    #[test]
    fn distance_for_point_basic() {
        let a = geo2d::pt(0.0, 0.0);
        let b = geo2d::pt(100.0, 0.0);
        let dist = SoundSource::distance_for_point(a, b);
        assert!((dist - 100.0).abs() < 0.01);
    }

    #[test]
    fn distance_for_point_isometric() {
        let a = geo2d::pt(0.0, 0.0);
        let b = geo2d::pt(0.0, 100.0);
        // Y distance is scaled by INVERSE_ASPECT_RATIO
        let dist = SoundSource::distance_for_point(a, b);
        assert!((dist - 100.0 * INVERSE_ASPECT_RATIO).abs() < 0.1);
    }

    #[test]
    fn segment_distance_perpendicular() {
        let a = geo2d::pt(0.0, 0.0);
        let b = geo2d::pt(100.0, 0.0);
        // Point directly above the midpoint
        let pos = geo2d::pt(50.0, 10.0);
        let dist = SoundSource::distance_to_segment(a, b, pos);
        assert!(dist.is_some());
        // Distance should be roughly 10 * INVERSE_ASPECT_RATIO (isometric Y)
        let d = dist.unwrap();
        assert!((d - 10.0 * INVERSE_ASPECT_RATIO).abs() < 1.0);
    }

    #[test]
    fn segment_distance_outside() {
        let a = geo2d::pt(0.0, 0.0);
        let b = geo2d::pt(100.0, 0.0);
        // Point far beyond the segment endpoint — perpendicular misses
        let pos = geo2d::pt(200.0, 10.0);
        let dist = SoundSource::distance_to_segment(a, b, pos);
        assert!(dist.is_none());
    }

    #[test]
    fn source_manager_add_delete() {
        let mut mgr = SoundSourceManager::new();
        assert_eq!(mgr.num_sources(), 0);

        let mut src = SoundSource::new();
        src.id = 42;
        src.active = true;
        let idx = mgr.add(src);
        assert_eq!(idx, 0);
        assert_eq!(mgr.num_sources(), 1);
        // add() sets active to false
        assert!(!mgr.get(0).unwrap().active);
        assert_eq!(mgr.get(0).unwrap().id, 42);

        let deleted = mgr.delete(0);
        assert!(deleted.is_some());
        assert!(mgr.get(0).is_none());
        // Slot remains, but is None
        assert_eq!(mgr.num_sources(), 1);
    }

    #[test]
    fn source_manager_find_by_sample_id() {
        let mut mgr = SoundSourceManager::new();
        let mut src1 = SoundSource::new();
        src1.id = 10;
        let mut src2 = SoundSource::new();
        src2.id = 20;
        mgr.add(src1);
        mgr.add(src2);

        assert_eq!(mgr.find_by_sample_id(10), Some(0));
        assert_eq!(mgr.find_by_sample_id(20), Some(1));
        assert_eq!(mgr.find_by_sample_id(30), None);
    }

    #[test]
    fn source_manager_iter_active() {
        let mut mgr = SoundSourceManager::new();
        let mut s0 = SoundSource::new();
        s0.id = 1;
        let mut s1 = SoundSource::new();
        s1.id = 2;
        mgr.add(s0);
        mgr.add(s1);
        mgr.delete(0);

        let active: Vec<_> = mgr.iter_active().collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].0, 1); // index 1
        assert_eq!(active[0].1.id, 2);
    }

    #[test]
    fn proto_stream_roundtrip() {
        // Build a minimal proto stream for a global looped source
        let mut data = Vec::new();
        // id (u32)
        data.extend_from_slice(&42u32.to_le_bytes());
        // active (bool/u8)
        data.push(1);
        // kind (u8) = Looped
        data.push(1);
        // is_global (bool/u8)
        data.push(1);
        // altitude (u8) = Ground
        data.push(0);
        // ambiences (u32)
        data.extend_from_slice(&0xFFu32.to_le_bytes());

        let mut pos = 0;
        let src = SoundSource::from_proto_stream(&data, &mut pos, false);
        assert_eq!(src.id, 42);
        assert!(src.active);
        assert_eq!(src.source_kind, SoundSourceKind::Looped);
        assert!(src.is_global);
        assert_eq!(src.altitude, SoundSourceAltitude::Ground);
        assert_eq!(src.ambiences, 0xFF);
        assert_eq!(pos, data.len());
    }

    #[test]
    fn proto_stream_local_source() {
        // Build a proto stream for a local single-shot source (non-demo format)
        let mut data = Vec::new();
        // id
        data.extend_from_slice(&7u32.to_le_bytes());
        // active
        data.push(0);
        // kind = Single
        data.push(0);
        // is_global = false
        data.push(0);
        // inner_distance
        data.extend_from_slice(&50u16.to_le_bytes());
        // outer_distance
        data.extend_from_slice(&200u16.to_le_bytes());
        // _NEW_LEVELS dummy byte
        data.push(0);
        // num_points = 1
        data.extend_from_slice(&1u16.to_le_bytes());
        // point (10, 20) as i16
        data.extend_from_slice(&10i16.to_le_bytes());
        data.extend_from_slice(&20i16.to_le_bytes());
        // _NEW_LEVELS dummy byte
        data.push(0);
        // inner_volume (percent) = 80
        data.extend_from_slice(&80u16.to_le_bytes());
        // outer_volume (percent) = 20
        data.extend_from_slice(&20u16.to_le_bytes());
        // noise_covering_distance
        data.extend_from_slice(&150u16.to_le_bytes());
        // altitude = Middle
        data.push(1);
        // ambiences
        data.extend_from_slice(&3u32.to_le_bytes());

        let mut pos = 0;
        let src = SoundSource::from_proto_stream(&data, &mut pos, true);
        assert_eq!(src.id, 7);
        assert!(!src.active);
        assert_eq!(src.source_kind, SoundSourceKind::Single);
        assert!(!src.is_global);
        assert_eq!(src.inner_distance, 50);
        assert_eq!(src.outer_distance, 200);
        assert_eq!(src.shape.len(), 1);
        assert!((src.shape[0].x - 10.0).abs() < 0.01);
        assert!((src.shape[0].y - 20.0).abs() < 0.01);
        // 80 * 2.55 = 204
        assert_eq!(src.inner_volume, 204);
        // 20 * 2.55 = 51
        assert_eq!(src.outer_volume, 51);
        assert_eq!(src.noise_covering_distance, 150);
        assert_eq!(src.altitude, SoundSourceAltitude::Middle);
        assert_eq!(src.ambiences, 3);
        assert_eq!(pos, data.len());
    }

    #[test]
    fn serde_roundtrip() {
        let mut src = SoundSource::new();
        src.id = 7;
        src.source_kind = SoundSourceKind::Delayed;
        src.min_delay = 10;
        src.max_delay = 50;
        src.active = true;
        src.shape.push(geo2d::pt(1.0, 2.0));

        let json = serde_json::to_string(&src).unwrap();
        let restored: SoundSource = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, 7);
        assert_eq!(restored.source_kind, SoundSourceKind::Delayed);
        assert!(restored.active);
        assert_eq!(restored.shape.len(), 1);
    }

    #[test]
    fn source_manager_serde() {
        let mut mgr = SoundSourceManager::new();
        let mut s = SoundSource::new();
        s.id = 99;
        mgr.add(s);

        let json = serde_json::to_string(&mgr).unwrap();
        let restored: SoundSourceManager = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.num_sources(), 1);
        assert_eq!(restored.get(0).unwrap().id, 99);
    }
}
