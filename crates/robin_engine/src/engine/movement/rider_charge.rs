//! Rider-charge geometry and animation decision helpers.

use crate::coordinates::MapPoint;

pub(super) fn rider_charge_point_in_quad(point: MapPoint, quad: [(f32, f32); 4]) -> bool {
    fn cross(ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
        ax * by - ay * bx
    }
    let mut positive = false;
    let mut negative = false;
    for index in 0..4 {
        let (x1, y1) = quad[index];
        let (x2, y2) = quad[(index + 1) % 4];
        let value = cross(x2 - x1, y2 - y1, point.x - x1, point.y - y1);
        positive |= value > 0.0;
        negative |= value < 0.0;
    }
    !(positive && negative)
}

pub(super) fn is_galopp_decision_frame(current_frame: u16, frame_count: u16) -> bool {
    assert!(
        frame_count > 0,
        "selected RunningUpright rider-charge animation has no frames"
    );
    // Original compares WORD values in an arithmetic expression, so C++
    // promotes them to signed int. In particular, a one-frame animation's
    // midpoint expression is -1 rather than an unsigned underflow.
    let current_frame = i32::from(current_frame);
    let frame_count = i32::from(frame_count);
    current_frame == frame_count / 2 - 1 || current_frame == frame_count - 1
}
