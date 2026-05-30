//! 3D/2D sound positioning and volume calculations.
//!
//! Handles distance attenuation, panning, fading, and altitude-based
//! volume adjustment for the game's isometric sound system.

use serde::{Deserialize, Serialize};

use crate::coordinates::MapPoint;
use crate::geo2d::{self, GeoPoint2D};

// ─── Constants ──────────────────────────────────────────────────────

use crate::position_interface::INVERSE_ASPECT_RATIO;
/// 1/255 — used to convert byte volumes [0–255] to float [0.0–1.0].
const SOUND_VOLUME_RANGE: f32 = 1.0 / 255.0;

// FX audible distances at different zoom levels
const FX_INNER_DISTANCE_200: f32 = 160.0;
const FX_OUTER_DISTANCE_200: f32 = 400.0;
const FX_INNER_DISTANCE_100: f32 = 240.0;
const FX_OUTER_DISTANCE_100: f32 = 800.0;
const FX_INNER_DISTANCE_050: f32 = 360.0;
const FX_OUTER_DISTANCE_050: f32 = 1200.0;

// Exclamation audible distances at different zoom levels
const EX_INNER_DISTANCE_200: f32 = 160.0;
const EX_OUTER_DISTANCE_200: f32 = 320.0;
const EX_INNER_DISTANCE_100: f32 = 240.0;
const EX_OUTER_DISTANCE_100: f32 = 640.0;
const EX_INNER_DISTANCE_050: f32 = 360.0;
const EX_OUTER_DISTANCE_050: f32 = 960.0;

const MUSIC_HALFRANGE_VOLUME: f32 = 90.0;

/// Sounds below this volume threshold are considered inaudible.
const VOLUME_CUT_THRESHOLD: f32 = 0.01;

// ─── Enums ──────────────────────────────────────────────────────────

/// Sound type — determines how a sound is processed.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum SoundType {
    None,
    /// Positioned sound from a multi-point source geometry.
    Source,
    /// Single-position FX.
    Fx,
    /// Combat FX.
    CombatFx,
    /// Menu FX (UI sounds).
    MenuFx,
    /// Exclamation (character speech bubbles).
    Exclamation,
    /// Streamed music.
    Music,
    /// Streamed dialogue.
    Dialog,
    /// Streamed jingle.
    Jingle,
}

/// Altitude classification for sound sources.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
pub enum SoundSourceAltitude {
    Ground,
    Middle,
    Top,
    /// No altitude adjustment.
    NoAltitude,
}

// ─── Supporting structs ─────────────────────────────────────────────

/// Defines the attenuation range for a sound.
/// Between inner and outer distance, volume interpolates linearly.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SoundRange {
    pub outer_distance: f32,
    pub outer_volume: f32,
    pub inner_distance: f32,
    pub inner_volume: f32,
}

/// Describes a sound source's geometry — either a global source or a
/// shape made of multiple points. This is the data `SoundGeometry`
/// needs from a sound source to compute spatial audio params.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SoundSourceInfo {
    pub is_global: bool,
    pub altitude: SoundSourceAltitude,
    pub inner_distance: u16,
    pub outer_distance: u16,
    pub inner_volume: u16,
    pub outer_volume: u16,
    pub shape: Vec<MapPoint>,
}

/// Sound settings passed to `get_logical_playing_params`.  The union
/// fields are modelled as an enum variant in `SoundSettingsSource`.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SoundSettings {
    pub sound_type: SoundType,
    pub position: MapPoint,
    pub identifier: u32,
    pub source: SoundSettingsSource,
}

/// Discriminated source variant for `SoundSettings`.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub enum SoundSettingsSource {
    /// Sound from a multi-point source.
    SoundSource {
        info: SoundSourceInfo,
        speech_variant: i32,
    },
    /// Single-position sound (FX / Exclamation / etc.).
    Position { material: u8 },
}

/// Logical + final playing parameters.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct PlayingParameters {
    // Logical settings (float, -1..1 for panning/fading, 0..1 for volume)
    pub volume: f32,
    pub panning: f32,
    pub fading: f32,
    pub priority: u16,

    // 2D final settings (0–255 range)
    pub volume_2d: u16,
    pub panning_2d: u16,
    pub fading_2d: u16,

    // 3D final settings
    pub position_3d: [f32; 3],
}

impl Default for PlayingParameters {
    fn default() -> Self {
        Self {
            volume: 0.0,
            panning: 0.0,
            fading: 0.0,
            priority: 0,
            volume_2d: 0,
            panning_2d: 128,
            fading_2d: 128,
            position_3d: [0.0, 10000.0, 0.0],
        }
    }
}

// ─── SoundGeometry ──────────────────────────────────────────────────

/// Main sound geometry processor.
/// Computes volume, panning, and fading based on listener position,
/// zoom level, and sound source geometry.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct SoundGeometry {
    /// Current listener position in level coordinates.
    listen_point: MapPoint,
    /// Current zoom level (0.5 = zoomed out, 2.0 = zoomed in).
    zoom_factor: f32,

    // Per-category volume multipliers (0.0–1.0)
    exclamation_volume: f32,
    fx_volume: f32,
    music_volume: f32,
    jingle_volume: f32,
    dialogue_volume: f32,

    // Zoom-dependent FX audible range
    fx_outer_distance: f32,
    fx_inner_distance: f32,

    // Zoom-dependent exclamation audible range
    ex_outer_distance: f32,
    ex_inner_distance: f32,
}

struct GeometryScratch {
    decrunched_geometry: Vec<GeoPoint2D>,
    point_distances: Vec<f32>,
    segment_distances: Vec<f32>,
    segment_intersections: Vec<GeoPoint2D>,
    closest_point: GeoPoint2D,
    source_distance: f32,
}

impl Default for SoundGeometry {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundGeometry {
    // Zero every member up front.  The zoom-dependent distances and the
    // per-source transient arrays would otherwise rely on
    // `set_zoom_factor` / `update_geometry_for_listen_pos` running
    // before any read; defensive zeroing is strictly safer with the
    // same observable behaviour for any well-ordered call sequence.
    pub fn new() -> Self {
        Self {
            listen_point: MapPoint::ZERO,
            zoom_factor: 0.0,
            exclamation_volume: 0.0,
            fx_volume: 0.0,
            music_volume: 0.0,
            jingle_volume: 0.0,
            dialogue_volume: 0.0,
            fx_outer_distance: 0.0,
            fx_inner_distance: 0.0,
            ex_outer_distance: 0.0,
            ex_inner_distance: 0.0,
        }
    }

    // ── Accessors ──

    pub fn listen_point(&self) -> MapPoint {
        self.listen_point
    }

    pub fn set_listen_point(&mut self, point: MapPoint) {
        self.listen_point = point;
    }

    pub fn zoom_factor(&self) -> f32 {
        self.zoom_factor
    }

    pub fn set_exclamation_volume(&mut self, v: f32) {
        self.exclamation_volume = v;
    }

    pub fn set_fx_volume(&mut self, v: f32) {
        self.fx_volume = v;
    }

    pub fn set_music_volume(&mut self, v: f32) {
        self.music_volume = v;
    }

    pub fn set_dialogue_volume(&mut self, v: f32) {
        self.dialogue_volume = v;
    }

    pub fn fx_volume_byte(&self) -> u16 {
        (self.fx_volume * 255.0) as u16
    }

    // ── Zoom factor ──

    /// Update zoom factor and recompute distance thresholds.
    /// Zoom range: 0.5 (far) → 1.0 (default) → 2.0 (close).
    pub fn set_zoom_factor(&mut self, zoom: f32) {
        self.zoom_factor = zoom;

        if zoom < 1.0 {
            // Interpolate between 050 (far) and 100 (default)
            let t = 1.0 - zoom * 2.0; // t: 0.0 at zoom=0.5, -1.0 at zoom=1.0...
            // Formula: (DIST_050 - DIST_100) * (1 - zoom*2) + DIST_050
            // At zoom=0.5: t=0 → DIST_050
            // At zoom=1.0: t=-1 → 2*DIST_100 - DIST_050
            self.fx_outer_distance =
                (FX_OUTER_DISTANCE_050 - FX_OUTER_DISTANCE_100) * t + FX_OUTER_DISTANCE_050;
            self.fx_inner_distance =
                (FX_INNER_DISTANCE_050 - FX_INNER_DISTANCE_100) * t + FX_INNER_DISTANCE_050;
            self.ex_outer_distance =
                (EX_OUTER_DISTANCE_050 - EX_OUTER_DISTANCE_100) * t + EX_OUTER_DISTANCE_050;
            self.ex_inner_distance =
                (EX_INNER_DISTANCE_050 - EX_INNER_DISTANCE_100) * t + EX_INNER_DISTANCE_050;
        } else {
            // Interpolate between 100 (default) and 200 (close)
            let t = 1.0 - zoom; // t: 0.0 at zoom=1.0, -1.0 at zoom=2.0
            self.fx_outer_distance =
                (FX_OUTER_DISTANCE_100 - FX_OUTER_DISTANCE_200) * t + FX_OUTER_DISTANCE_100;
            self.fx_inner_distance =
                (FX_INNER_DISTANCE_100 - FX_INNER_DISTANCE_200) * t + FX_INNER_DISTANCE_100;
            self.ex_outer_distance =
                (EX_OUTER_DISTANCE_100 - EX_OUTER_DISTANCE_200) * t + EX_OUTER_DISTANCE_100;
            self.ex_inner_distance =
                (EX_INNER_DISTANCE_100 - EX_INNER_DISTANCE_200) * t + EX_INNER_DISTANCE_100;
        }
    }

    // ── Private geometry methods ──

    /// Linear interpolation of volume based on distance within a SoundRange.
    fn volume_for_distance(distance: f32, range: &SoundRange) -> f32 {
        if distance > range.outer_distance {
            return range.outer_volume;
        }
        if distance <= range.inner_distance {
            return range.inner_volume;
        }
        // Linear interpolation between inner and outer
        range.inner_volume
            + (distance - range.inner_distance) / (range.outer_distance - range.inner_distance)
                * (range.outer_volume - range.inner_volume)
    }

    /// Compute panning for a single point relative to the listener.
    /// Returns -1.0 (full left) to 1.0 (full right), 0.0 = centered.
    fn panning_for_point(position: GeoPoint2D, range: &SoundRange) -> f32 {
        let x_abs = position.x.abs();

        if x_abs < range.inner_distance {
            return 0.0;
        }
        if x_abs >= range.outer_distance {
            return if position.x >= 0.0 { 1.0 } else { -1.0 };
        }

        let ratio = (x_abs - range.inner_distance) / (range.outer_distance - range.inner_distance);
        if position.x >= 0.0 { ratio } else { -ratio }
    }

    /// Compute panning for a multi-point sound source.
    /// Weighted average of per-point pannings, weighted by volume.
    fn panning_for_sound_source(scratch: &GeometryScratch, range: &SoundRange) -> f32 {
        if scratch.decrunched_geometry.len() == 1 {
            return Self::panning_for_point(scratch.closest_point, range);
        }

        let mut panning = 0.0_f32;
        let mut count = 0_u16;

        for (i, dist) in scratch.point_distances.iter().copied().enumerate() {
            if dist < range.outer_distance {
                if dist < range.inner_distance {
                    return 0.0;
                }
                panning += Self::panning_for_point(scratch.decrunched_geometry[i], range)
                    * Self::volume_for_distance(dist, range);
                count += 1;
            }
        }

        // Segment intersection contributions
        for (i, dist) in scratch.segment_distances.iter().copied().enumerate() {
            if dist >= 0.0 && dist < range.outer_distance {
                if dist < range.inner_distance {
                    return 0.0;
                }
                panning += Self::panning_for_point(scratch.segment_intersections[i], range)
                    * Self::volume_for_distance(dist, range);
                count += 1;
            }
        }

        if count != 0 {
            panning / count as f32
        } else {
            0.0
        }
    }

    /// Compute fading (front-to-back panning) for a multi-point source.
    /// Same as panning but with X and Y swapped.
    fn fading_for_sound_source(scratch: &GeometryScratch, range: &SoundRange) -> f32 {
        if scratch.decrunched_geometry.len() == 1 {
            let swapped = geo2d::pt(scratch.closest_point.y, scratch.closest_point.x);
            return Self::panning_for_point(swapped, range);
        }

        let mut fading = 0.0_f32;
        let mut count = 0_u16;

        for (i, dist) in scratch.point_distances.iter().copied().enumerate() {
            if dist < range.outer_distance {
                if dist < range.inner_distance {
                    return 0.0;
                }
                let p = scratch.decrunched_geometry[i];
                let swapped = geo2d::pt(p.y, p.x);
                fading += Self::panning_for_point(swapped, range)
                    * Self::volume_for_distance(dist, range);
                count += 1;
            }
        }

        for (i, dist) in scratch.segment_distances.iter().copied().enumerate() {
            if dist >= 0.0 && dist < range.outer_distance {
                if dist < range.inner_distance {
                    return 0.0;
                }
                let p = scratch.segment_intersections[i];
                let swapped = geo2d::pt(p.y, p.x);
                fading += Self::panning_for_point(swapped, range)
                    * Self::volume_for_distance(dist, range);
                count += 1;
            }
        }

        if count != 0 {
            fading / count as f32
        } else {
            0.0
        }
    }

    /// Pre-calculate geometry data for all points of a sound source
    /// relative to the current listener position.
    ///
    /// Transforms source points into listener-relative coordinates
    /// (with isometric Y scaling), computes per-point distances, and
    /// finds closest-point-on-segment projections for each edge.
    fn geometry_for_listen_pos(&self, source: &SoundSourceInfo) -> GeometryScratch {
        let n = source.shape.len();

        let mut scratch = GeometryScratch {
            decrunched_geometry: vec![geo2d::pt(0.0, 0.0); n],
            point_distances: vec![0.0; n],
            segment_distances: vec![-1.0; n.saturating_sub(1)],
            segment_intersections: vec![geo2d::pt(0.0, 0.0); n.saturating_sub(1)],
            closest_point: geo2d::pt(0.0, 0.0),
            source_distance: 1_000_000.0,
        };

        // Transform each point to listener-relative coords with aspect correction
        for i in 0..n {
            let raw = source.shape[i];
            let point = geo2d::pt(
                raw.x - self.listen_point.x,
                (raw.y - self.listen_point.y) * INVERSE_ASPECT_RATIO,
            );
            scratch.decrunched_geometry[i] = point;

            let dist = (point.x * point.x + point.y * point.y).sqrt();
            if dist < scratch.source_distance {
                scratch.source_distance = dist;
                scratch.closest_point = point;
            }
            scratch.point_distances[i] = dist;
        }

        if n < 2 {
            return scratch;
        }

        // For each segment, find the perpendicular projection of the
        // origin (listener) onto the segment.  Equivalent to creating a
        // line through the origin along the segment's normal and
        // intersecting it with the segment.
        let mut point_a = scratch.decrunched_geometry[0];

        for i in 1..n {
            let point_b = scratch.decrunched_geometry[i];
            let seg_idx = i - 1;

            // Project origin onto line defined by segment [point_a, point_b]
            let ab = geo2d::pt(point_b.x - point_a.x, point_b.y - point_a.y);
            let len_sq = ab.x * ab.x + ab.y * ab.y;

            if len_sq > geo2d::PRECISION * geo2d::PRECISION {
                // t = dot(origin - point_a, ab) / len_sq = dot(-point_a, ab) / len_sq
                let t = (-point_a.x * ab.x + -point_a.y * ab.y) / len_sq;

                if (0.0..=1.0).contains(&t) {
                    // Projection falls within the segment
                    let proj = geo2d::pt(point_a.x + t * ab.x, point_a.y + t * ab.y);
                    let dist = (proj.x * proj.x + proj.y * proj.y).sqrt();

                    if dist < scratch.source_distance {
                        scratch.source_distance = dist;
                        scratch.closest_point = proj;
                    }

                    scratch.segment_distances[seg_idx] = dist;
                    scratch.segment_intersections[seg_idx] = proj;
                } else {
                    scratch.segment_distances[seg_idx] = -1.0;
                }
            } else {
                scratch.segment_distances[seg_idx] = -1.0;
            }

            point_a = point_b;
        }

        scratch
    }

    /// Adjust volume based on sound source altitude and current zoom.
    fn volume_for_altitude(&self, volume: f32, altitude: SoundSourceAltitude) -> f32 {
        match altitude {
            SoundSourceAltitude::Ground => 0.5 * self.zoom_factor * volume,
            SoundSourceAltitude::NoAltitude => volume,
            SoundSourceAltitude::Middle => {
                if self.zoom_factor > 1.0 {
                    volume * (1.0 - (self.zoom_factor - 1.0) * 0.5)
                } else {
                    volume * self.zoom_factor
                }
            }
            SoundSourceAltitude::Top => {
                if self.zoom_factor > 1.0 {
                    volume * (0.5 - (self.zoom_factor - 1.0) * 0.25)
                } else {
                    volume * (0.5 / self.zoom_factor)
                }
            }
        }
    }

    // ── Public methods ──

    /// Compute logical playing parameters for given sound settings.
    /// Returns `Some(params)` if audible, `None` if below threshold.
    pub fn get_logical_playing_params(
        &mut self,
        settings: &SoundSettings,
        low_priority: bool,
    ) -> Option<PlayingParameters> {
        let mut params = PlayingParameters::default();

        if settings.sound_type == SoundType::Source {
            let SoundSettingsSource::SoundSource { info, .. } = &settings.source else {
                panic!("SoundType::Source requires SoundSettingsSource::SoundSource");
            };

            if info.is_global {
                params.volume = self.volume_for_altitude(1.0, info.altitude) * self.fx_volume;
                params.panning = 0.0;
                params.fading = 0.0;
            } else {
                let source_range = SoundRange {
                    outer_distance: info.outer_distance as f32,
                    outer_volume: info.outer_volume as f32 * SOUND_VOLUME_RANGE,
                    inner_distance: info.inner_distance as f32,
                    inner_volume: info.inner_volume as f32 * SOUND_VOLUME_RANGE,
                };

                let scratch = self.geometry_for_listen_pos(info);

                params.volume = self.volume_for_altitude(
                    Self::volume_for_distance(scratch.source_distance, &source_range),
                    info.altitude,
                ) * self.fx_volume;

                if params.volume < VOLUME_CUT_THRESHOLD {
                    return None;
                }

                params.panning = Self::panning_for_sound_source(&scratch, &source_range);
                params.fading = Self::fading_for_sound_source(&scratch, &source_range);
                params.priority = (params.volume * 63.0) as u16 + 64;
            }
        } else {
            // Single-position sound (FX / Exclamation / CombatFx / MenuFx)
            let (outer_dist, inner_dist) = if settings.sound_type == SoundType::Exclamation {
                (self.ex_outer_distance, self.ex_inner_distance)
            } else {
                (self.fx_outer_distance, self.fx_inner_distance)
            };

            let location_range = SoundRange {
                outer_distance: outer_dist,
                outer_volume: 0.0,
                inner_distance: inner_dist,
                inner_volume: 1.0,
            };

            let rel = geo2d::pt(
                settings.position.x - self.listen_point.x,
                (settings.position.y - self.listen_point.y) * INVERSE_ASPECT_RATIO,
            );
            let distance = (rel.x * rel.x + rel.y * rel.y).sqrt();

            let mut volume = self.volume_for_altitude(
                Self::volume_for_distance(distance, &location_range),
                SoundSourceAltitude::Ground,
            );

            match settings.sound_type {
                SoundType::Exclamation => volume *= self.exclamation_volume,
                SoundType::Fx | SoundType::MenuFx | SoundType::CombatFx => {
                    volume *= self.fx_volume;
                }
                _ => panic!(
                    "Unexpected sound type {:?} for position-based sound",
                    settings.sound_type
                ),
            }

            if volume < VOLUME_CUT_THRESHOLD {
                return None;
            }

            params.volume = volume;
            params.panning = Self::panning_for_point(rel, &location_range);
            params.fading = Self::panning_for_point(rel, &location_range);

            if low_priority {
                params.priority = (params.volume * 63.0) as u16;
            } else {
                params.priority = (params.volume * 126.0) as u16 + 128;
            }
        }

        Some(params)
    }

    /// Convert logical parameters to 2D hardware values (0–255 range).
    pub fn get_2d_playing_params(params: &mut PlayingParameters) {
        if params.volume < VOLUME_CUT_THRESHOLD + 0.01 {
            params.volume_2d = 0;
            params.panning_2d = 128;
            params.fading_2d = 128;
        } else {
            params.volume_2d = (params.volume * 255.0) as u16;
            params.panning_2d = ((params.panning + 1.0) * 127.5) as u16;
            params.fading_2d = ((params.fading + 1.0) * 127.5) as u16;
        }
    }

    /// Convert logical parameters to 3D hardware values.
    /// Also computes 2D params as a side effect.
    pub fn get_3d_playing_params(params: &mut PlayingParameters) {
        Self::get_2d_playing_params(params);

        if params.volume < VOLUME_CUT_THRESHOLD + 0.01 {
            params.position_3d = [0.0, 10000.0, 0.0];
        } else {
            params.position_3d[0] = -params.panning;
            params.position_3d[1] = 1.0 - (params.panning.powi(2) + params.fading.powi(2)).sqrt();
            params.position_3d[2] = params.fading;
        }
    }

    /// Get music volume as a 0–255 value (actually 0–127 due to >>1).
    pub fn get_volume_for_music(&self, zoom_dependent: bool) -> u16 {
        let raw = if zoom_dependent {
            (255.0 - MUSIC_HALFRANGE_VOLUME * (self.zoom_factor - 0.5)) * self.music_volume
        } else {
            255.0 * self.music_volume
        };
        (raw as u16) >> 1
    }

    /// Get jingle volume as a 0–255 value.
    /// Sets `jingle_volume = fx_volume` as a side effect.
    pub fn get_volume_for_jingle(&mut self) -> u16 {
        self.jingle_volume = self.fx_volume;
        (255.0 * self.jingle_volume) as u16
    }

    /// Get dialogue volume as a 0–255 value.
    pub fn get_volume_for_dialogue(&self) -> u16 {
        (255.0 * self.dialogue_volume) as u16
    }
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_geometry() -> SoundGeometry {
        let mut sg = SoundGeometry::new();
        sg.set_zoom_factor(1.0);
        sg.set_fx_volume(1.0);
        sg.set_exclamation_volume(1.0);
        sg.set_music_volume(0.5);
        sg.set_dialogue_volume(0.8);
        sg
    }

    #[test]
    fn volume_for_distance_clamping() {
        let range = SoundRange {
            inner_distance: 100.0,
            inner_volume: 1.0,
            outer_distance: 400.0,
            outer_volume: 0.0,
        };

        // Inside inner → max volume
        assert_eq!(SoundGeometry::volume_for_distance(50.0, &range), 1.0);
        assert_eq!(SoundGeometry::volume_for_distance(100.0, &range), 1.0);

        // Beyond outer → min volume
        assert_eq!(SoundGeometry::volume_for_distance(500.0, &range), 0.0);

        // Midpoint → linear interpolation (halfway between inner and outer)
        let mid = SoundGeometry::volume_for_distance(250.0, &range);
        assert!((mid - 0.5).abs() < 1e-6, "mid = {}", mid);
    }

    #[test]
    fn panning_for_point_centered() {
        let range = SoundRange {
            inner_distance: 100.0,
            inner_volume: 1.0,
            outer_distance: 400.0,
            outer_volume: 0.0,
        };

        // Inside inner distance → centered
        let p = SoundGeometry::panning_for_point(geo2d::pt(50.0, 0.0), &range);
        assert_eq!(p, 0.0);
    }

    #[test]
    fn panning_for_point_extremes() {
        let range = SoundRange {
            inner_distance: 100.0,
            inner_volume: 1.0,
            outer_distance: 400.0,
            outer_volume: 0.0,
        };

        // Beyond outer distance → full left/right
        assert_eq!(
            SoundGeometry::panning_for_point(geo2d::pt(500.0, 0.0), &range),
            1.0
        );
        assert_eq!(
            SoundGeometry::panning_for_point(geo2d::pt(-500.0, 0.0), &range),
            -1.0
        );

        // Midpoint → proportional
        let p = SoundGeometry::panning_for_point(geo2d::pt(250.0, 0.0), &range);
        assert!((p - 0.5).abs() < 1e-6, "p = {}", p);

        let p = SoundGeometry::panning_for_point(geo2d::pt(-250.0, 0.0), &range);
        assert!((p - -0.5).abs() < 1e-6, "p = {}", p);
    }

    #[test]
    fn zoom_factor_distances() {
        let mut sg = SoundGeometry::new();

        // At zoom=1.0 (default)
        sg.set_zoom_factor(1.0);
        assert_eq!(sg.fx_outer_distance, FX_OUTER_DISTANCE_100);
        assert_eq!(sg.fx_inner_distance, FX_INNER_DISTANCE_100);

        // At zoom=2.0 (close)
        sg.set_zoom_factor(2.0);
        assert_eq!(sg.fx_outer_distance, FX_OUTER_DISTANCE_200);
        assert_eq!(sg.fx_inner_distance, FX_INNER_DISTANCE_200);

        // At zoom=0.5 (far): t = 1 - 0.5*2 = 0, so the formula
        // (050-100)*(1-0.5*2) + 050 collapses to 050.
        sg.set_zoom_factor(0.5);
        assert_eq!(sg.fx_outer_distance, FX_OUTER_DISTANCE_050);
        assert_eq!(sg.fx_inner_distance, FX_INNER_DISTANCE_050);
    }

    #[test]
    fn altitude_ground_scales_with_zoom() {
        let mut sg = make_geometry();
        sg.set_zoom_factor(1.0);
        let v = sg.volume_for_altitude(1.0, SoundSourceAltitude::Ground);
        assert!((v - 0.5).abs() < 1e-6);

        sg.set_zoom_factor(2.0);
        let v = sg.volume_for_altitude(1.0, SoundSourceAltitude::Ground);
        assert!((v - 1.0).abs() < 1e-6);
    }

    #[test]
    fn altitude_none_ignores_zoom() {
        let mut sg = make_geometry();
        sg.set_zoom_factor(0.5);
        let v = sg.volume_for_altitude(0.7, SoundSourceAltitude::NoAltitude);
        assert!((v - 0.7).abs() < 1e-6);
    }

    #[test]
    fn get_2d_params_silent() {
        let mut params = PlayingParameters {
            volume: 0.005, // below threshold + 0.01
            ..Default::default()
        };
        SoundGeometry::get_2d_playing_params(&mut params);
        assert_eq!(params.volume_2d, 0);
        assert_eq!(params.panning_2d, 128);
        assert_eq!(params.fading_2d, 128);
    }

    #[test]
    fn get_2d_params_full_volume_centered() {
        let mut params = PlayingParameters {
            volume: 1.0,
            panning: 0.0,
            fading: 0.0,
            ..Default::default()
        };
        SoundGeometry::get_2d_playing_params(&mut params);
        assert_eq!(params.volume_2d, 255);
        // panning 0.0 → (0+1)*127.5 = 127
        assert_eq!(params.panning_2d, 127);
        assert_eq!(params.fading_2d, 127);
    }

    #[test]
    fn get_3d_params_silent_far_away() {
        let mut params = PlayingParameters {
            volume: 0.005,
            ..Default::default()
        };
        SoundGeometry::get_3d_playing_params(&mut params);
        assert_eq!(params.position_3d[1], 10000.0);
    }

    #[test]
    fn music_volume() {
        let sg = make_geometry();
        // Non-zoom: (255 * 0.5) >> 1 = 127 >> 1 = 63
        assert_eq!(sg.get_volume_for_music(false), 63);
    }

    #[test]
    fn dialogue_volume() {
        let sg = make_geometry();
        // 255 * 0.8 = 204
        assert_eq!(sg.get_volume_for_dialogue(), 204);
    }

    #[test]
    fn jingle_uses_fx_volume() {
        let mut sg = make_geometry();
        sg.set_fx_volume(0.6);
        let v = sg.get_volume_for_jingle();
        assert_eq!(v, (255.0 * 0.6) as u16);
        assert!((sg.jingle_volume - 0.6).abs() < 1e-6);
    }

    #[test]
    fn single_point_source_panning() {
        let mut sg = make_geometry();
        sg.set_listen_point(MapPoint::ZERO);
        sg.set_zoom_factor(1.0);

        // Place a sound source 300 units to the right (within FX range at zoom=1.0)
        let info = SoundSourceInfo {
            is_global: false,
            altitude: SoundSourceAltitude::NoAltitude,
            inner_distance: 100,
            outer_distance: 500,
            inner_volume: 255,
            outer_volume: 0,
            shape: vec![MapPoint::new(300.0, 0.0)],
        };

        let settings = SoundSettings {
            sound_type: SoundType::Source,
            position: MapPoint::ZERO,
            identifier: 0,
            source: SoundSettingsSource::SoundSource {
                info,
                speech_variant: 0,
            },
        };

        let params = sg.get_logical_playing_params(&settings, false).unwrap();
        // Sound is to the right → positive panning
        assert!(params.panning > 0.0, "panning = {}", params.panning);
    }

    #[test]
    fn global_source_centered() {
        let mut sg = make_geometry();
        sg.set_zoom_factor(1.0);

        let info = SoundSourceInfo {
            is_global: true,
            altitude: SoundSourceAltitude::NoAltitude,
            inner_distance: 100,
            outer_distance: 500,
            inner_volume: 255,
            outer_volume: 0,
            shape: vec![],
        };

        let settings = SoundSettings {
            sound_type: SoundType::Source,
            position: MapPoint::ZERO,
            identifier: 0,
            source: SoundSettingsSource::SoundSource {
                info,
                speech_variant: 0,
            },
        };

        let params = sg.get_logical_playing_params(&settings, false).unwrap();
        assert_eq!(params.panning, 0.0);
        assert_eq!(params.fading, 0.0);
        assert!((params.volume - 1.0).abs() < 1e-6);
    }

    #[test]
    fn position_fx_too_far_returns_none() {
        let mut sg = make_geometry();
        sg.set_listen_point(MapPoint::ZERO);

        let settings = SoundSettings {
            sound_type: SoundType::Fx,
            position: MapPoint::new(10000.0, 10000.0),
            identifier: 0,
            source: SoundSettingsSource::Position { material: 0 },
        };

        assert!(sg.get_logical_playing_params(&settings, false).is_none());
    }

    #[test]
    fn multi_point_source_geometry() {
        let mut sg = make_geometry();
        sg.set_listen_point(MapPoint::new(150.0, 0.0));
        sg.set_zoom_factor(1.0);

        // A horizontal line segment from (100,0) to (200,0)
        // Listener at (150,0) is right on the midpoint
        let info = SoundSourceInfo {
            is_global: false,
            altitude: SoundSourceAltitude::NoAltitude,
            inner_distance: 50,
            outer_distance: 500,
            inner_volume: 255,
            outer_volume: 0,
            shape: vec![MapPoint::new(100.0, 0.0), MapPoint::new(200.0, 0.0)],
        };

        let settings = SoundSettings {
            sound_type: SoundType::Source,
            position: MapPoint::ZERO,
            identifier: 0,
            source: SoundSettingsSource::SoundSource {
                info,
                speech_variant: 0,
            },
        };

        let params = sg.get_logical_playing_params(&settings, false).unwrap();
        // Listener is on the segment → should be very close, high volume
        assert!(params.volume > 0.5, "volume = {}", params.volume);
    }

    #[test]
    fn serde_roundtrip() {
        let mut sg = make_geometry();
        sg.set_listen_point(MapPoint::new(100.0, 200.0));
        sg.set_zoom_factor(1.5);

        let json = serde_json::to_string(&sg).unwrap();
        let sg2: SoundGeometry = serde_json::from_str(&json).unwrap();

        assert_eq!(sg2.listen_point.x, 100.0);
        assert_eq!(sg2.listen_point.y, 200.0);
        assert_eq!(sg2.zoom_factor, 1.5);
        assert_eq!(sg2.fx_volume, 1.0);
    }
}
