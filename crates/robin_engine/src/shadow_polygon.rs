//! Shadow polygon — sim-side state and constants.
//!
//! The host-side rasteriser (which uses `Renderer`) lives in
//! `robin_rs::shadow_polygon`. EngineInner code only needs the sim-state
//! struct, view parameters, sector-direction helper, and a few constants
//! consumed by AI/render glue.

use serde::{Deserialize, Serialize};

// ── Constants ─────────────────────────────────────────────────────
/// Re-export of [`crate::position_interface::ASPECT_RATIO`].
pub use crate::position_interface::ASPECT_RATIO;
pub const RADIUS_DAY: f32 = 400.0;
pub const RADIUS_NIGHT: f32 = 300.0;
pub const ALPHA_DAY: u8 = 192;
pub const ALPHA_NIGHT: u8 = 120;
pub const NORMAL_HALF_APERTURE: f32 = 0.5;
/// Eye-level offset used by the obstacle-usefulness filter.
/// This is the offset from a character's feet to the eye plane — not the
/// full stature — and is used to decide which obstacles can contribute to
/// the visibility polygon given the viewer's Z.
pub const CHARACTER_HEIGHT: f32 = 40.0;

// ── ViewParameters ────────────────────────────────────────────────
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ViewParameters {
    pub direction: [f32; 2],
    pub half_aperture: f32,
    pub radius: f32,
    pub alpha: u8,
    pub lean_out: bool,
    /// Viewer eye altitude. Used by the obstacle-usefulness filter in
    /// `compute_visibility_polygon`.
    pub viewer_z: f32,
    /// Projection plane for debug rendering. The original
    /// `RHShadowPolygon::SetScreenCoords` projects rendered polygon
    /// vertices with `screen_y = y - plane.ComputeZ(x, y)`.
    #[serde(default)]
    pub projection_plane: Option<crate::position_interface::PlaneZCoeffs>,
    /// Current projection-area obstacle used by the display path. The
    /// original `RHShadowPolygon` renders one slice per projection area
    /// and clips the slice to that area's projected polygon.
    #[serde(skip)]
    pub projection_obstacle: Option<crate::position_interface::ObstacleHandle>,
}

impl Default for ViewParameters {
    fn default() -> Self {
        Self {
            direction: [1.0, 0.0],
            half_aperture: NORMAL_HALF_APERTURE,
            radius: RADIUS_DAY,
            alpha: ALPHA_DAY,
            lean_out: false,
            viewer_z: 0.0,
            projection_plane: None,
            projection_obstacle: None,
        }
    }
}

/// Convert a 16-sector cardinal direction index to a unit (x, y) vector.
/// Sector 0 = north = -Y; sectors increase clockwise.
pub fn sector_to_direction(sector: i16) -> [f32; 2] {
    let sector = sector.rem_euclid(16);
    let angle = sector as f32 * std::f32::consts::TAU / 16.0;
    let x = angle.sin();
    let y = -angle.cos();
    [x, y]
}
