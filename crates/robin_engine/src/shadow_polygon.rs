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
    // Keep the literal table from SBGeoVector2D.cpp. Re-evaluating sin/cos
    // produces values a few ULPs away from the Original constants; patrol
    // formation multiplies these offsets by 20, and the resulting error can
    // flip the exact dot-product test in IsGoalReached.
    const SIN_PI_EIGHTH: f32 = 0.382_683_432_365_09;
    const COS_PI_EIGHTH: f32 = 0.923_879_532_511_28;
    const HALF_SQRT_TWO: f32 = 0.707_106_781_186_54;
    const X: [f32; 16] = [
        0.0,
        SIN_PI_EIGHTH,
        HALF_SQRT_TWO,
        COS_PI_EIGHTH,
        1.0,
        COS_PI_EIGHTH,
        HALF_SQRT_TWO,
        SIN_PI_EIGHTH,
        0.0,
        -SIN_PI_EIGHTH,
        -HALF_SQRT_TWO,
        -COS_PI_EIGHTH,
        -1.0,
        -COS_PI_EIGHTH,
        -HALF_SQRT_TWO,
        -SIN_PI_EIGHTH,
    ];
    const Y: [f32; 16] = [
        -1.0,
        -COS_PI_EIGHTH,
        -HALF_SQRT_TWO,
        -SIN_PI_EIGHTH,
        0.0,
        SIN_PI_EIGHTH,
        HALF_SQRT_TWO,
        COS_PI_EIGHTH,
        1.0,
        COS_PI_EIGHTH,
        HALF_SQRT_TWO,
        SIN_PI_EIGHTH,
        0.0,
        -SIN_PI_EIGHTH,
        -HALF_SQRT_TWO,
        -COS_PI_EIGHTH,
    ];
    let index = sector.rem_euclid(16) as usize;
    [X[index], Y[index]]
}

#[cfg(test)]
mod tests {
    use super::sector_to_direction;

    #[test]
    fn sector_directions_match_original_literal_table_bits() {
        let expected_x = [
            0x0000_0000,
            0x3ec3_ef15,
            0x3f35_04f3,
            0x3f6c_835e,
            0x3f80_0000,
            0x3f6c_835e,
            0x3f35_04f3,
            0x3ec3_ef15,
            0x0000_0000,
            0xbec3_ef15,
            0xbf35_04f3,
            0xbf6c_835e,
            0xbf80_0000,
            0xbf6c_835e,
            0xbf35_04f3,
            0xbec3_ef15,
        ];
        let expected_y = [
            0xbf80_0000,
            0xbf6c_835e,
            0xbf35_04f3,
            0xbec3_ef15,
            0x0000_0000,
            0x3ec3_ef15,
            0x3f35_04f3,
            0x3f6c_835e,
            0x3f80_0000,
            0x3f6c_835e,
            0x3f35_04f3,
            0x3ec3_ef15,
            0x0000_0000,
            0xbec3_ef15,
            0xbf35_04f3,
            0xbf6c_835e,
        ];

        for sector in 0..16 {
            let [x, y] = sector_to_direction(sector);
            assert_eq!(x.to_bits(), expected_x[sector as usize]);
            assert_eq!(y.to_bits(), expected_y[sector as usize]);
        }
        assert_eq!(sector_to_direction(-1), sector_to_direction(15));
        assert_eq!(sector_to_direction(16), sector_to_direction(0));
    }
}
