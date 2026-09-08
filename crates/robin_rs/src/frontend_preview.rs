//! Host-only trajectory cache and its hover/marker lifecycle.
//!
//! Hit validity is deliberately independent from cached arc geometry: several
//! original cursor branches reject a click without erasing the displayed arc.
//! Only the named invalidation transitions below may clear that geometry.

use robin_engine::coordinates::{MapPoint, WorldPoint3D};
use robin_engine::element::TrajectoryPoint;
use robin_engine::engine::{GroundMarkSpriteData, input::TrajectoryPreview};
use robin_engine::markers::GroundMark;
use robin_engine::profiles::Action;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
pub struct FrontendTrajectoryPreview {
    valid: bool,
    shift_held: bool,
    action: Action,
    points: Vec<TrajectoryPoint>,
    start: WorldPoint3D,
    layer: u16,
    crumpled: bool,
    hover_ticks: u32,
    previous_mouse: MapPoint,
    mark_count: u16,
    ground_mark: GroundMark,
}

impl FrontendTrajectoryPreview {
    pub fn is_valid(&self) -> bool {
        self.valid
    }
    pub fn points(&self) -> &[TrajectoryPoint] {
        &self.points
    }
    pub fn start(&self) -> WorldPoint3D {
        self.start
    }
    pub fn layer(&self) -> u16 {
        self.layer
    }
    pub fn crumpled(&self) -> bool {
        self.crumpled
    }
    pub fn hover_ticks(&self) -> u32 {
        self.hover_ticks
    }
    pub fn previous_mouse(&self) -> MapPoint {
        self.previous_mouse
    }
    pub fn mark_count(&self) -> u16 {
        self.mark_count
    }
    pub fn ground_marks(&self) -> &GroundMark {
        &self.ground_mark
    }

    /// Reject a click without discarding the last drawn arc or its tint.
    pub fn reject_hit(&mut self) {
        self.valid = false;
    }

    pub fn apply(&mut self, preview: TrajectoryPreview) {
        match preview {
            TrajectoryPreview::Invalid => {
                self.points.clear();
                self.valid = false;
                self.crumpled = false;
            }
            TrajectoryPreview::HitNoArc => {
                self.points.clear();
                self.valid = true;
                self.crumpled = false;
            }
            TrajectoryPreview::ShowArc {
                points,
                start,
                crumpled,
                layer,
            } => {
                self.valid = true;
                self.points = points;
                self.start = start;
                self.layer = layer;
                self.crumpled = crumpled;
            }
        }
    }

    /// Enhanced net prediction replaces only the tint, not hit eligibility.
    pub fn apply_crumple_prediction(&mut self, predicted: bool) {
        self.crumpled = predicted;
    }

    fn clear_arc(&mut self) {
        self.valid = false;
        self.points.clear();
    }

    /// Preserve ordering: a changed identity resets the timer *before* a
    /// stationary mouse increments it on this same presentation iteration.
    pub fn observe_hover(&mut self, shift_held: bool, action: Action, mouse: MapPoint) {
        if self.shift_held != shift_held || self.action != action {
            self.clear_arc();
            self.hover_ticks = 0;
            self.shift_held = shift_held;
            self.action = action;
        }
        if mouse == self.previous_mouse {
            self.hover_ticks = self.hover_ticks.saturating_add(1);
        } else {
            self.hover_ticks = 0;
            self.clear_arc();
            self.previous_mouse = mouse;
        }
    }

    /// This runs before the current action computes its new result, so the
    /// periodic marker belongs to the previously displayed trajectory.
    pub fn advance_hover_markers(&mut self, display_delay: u32) {
        if self.hover_ticks <= display_delay {
            self.clear_arc();
        }
        if self.valid {
            self.mark_count = self.mark_count.wrapping_add(1);
            if self.mark_count.is_multiple_of(10)
                && let Some(dest) = self.points.last()
            {
                let mark = dest.position.to_map();
                self.ground_mark.add_mark(mark.x, mark.y, self.layer);
            }
        } else {
            self.mark_count = 0;
        }
    }

    pub fn interrupt_hover(&mut self) {
        self.hover_ticks = 0;
    }

    pub fn invalidate_action(&mut self) {
        self.clear_arc();
        self.ground_mark.clear();
        self.mark_count = 0;
        self.crumpled = false;
    }

    /// Save/load reset preserves the old mouse/timer and installed sprite
    /// metadata, matching the original reset scope (not `Default`).
    pub fn reset_after_restore(&mut self) {
        self.invalidate_action();
        self.action = Action::default();
        self.shift_held = false;
    }

    pub fn install_mark_sprite(&mut self, data: &GroundMarkSpriteData) {
        self.ground_mark.set_sprite_data(
            data.half_w,
            data.half_h,
            data.frame_sizes.clone(),
            data.per_frame_offsets.clone(),
        );
    }

    pub fn tick_marks(
        &mut self,
        view: geo::Coord<f32>,
        zoom: f32,
        width: i32,
        height: i32,
        frame: u32,
    ) {
        self.ground_mark.tick(view, zoom, width, height, frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arc() -> TrajectoryPreview {
        TrajectoryPreview::ShowArc {
            points: vec![TrajectoryPoint {
                position: WorldPoint3D::new(30.0, 40.0, 5.0),
                time: 3,
            }],
            start: WorldPoint3D::new(10.0, 20.0, 0.0),
            layer: 4,
            crumpled: true,
        }
    }

    #[test]
    fn rejection_and_result_replacement_have_distinct_geometry_lifetimes() {
        let mut preview = FrontendTrajectoryPreview::default();
        preview.apply(arc());
        preview.reject_hit();
        assert!(!preview.is_valid());
        assert_eq!(preview.points().len(), 1);
        assert!(preview.crumpled());
        preview.apply(TrajectoryPreview::HitNoArc);
        assert!(preview.is_valid());
        assert!(preview.points().is_empty());
        assert!(!preview.crumpled());
        // The original keeps shooter coordinates when an arc disappears.
        assert_eq!(preview.start(), WorldPoint3D::new(10.0, 20.0, 0.0));
        assert_eq!(preview.layer(), 4);
        preview.apply(TrajectoryPreview::Invalid);
        assert!(!preview.is_valid());
    }

    #[test]
    fn stationary_identity_change_increments_after_reset_and_motion_clears_arc() {
        let mut preview = FrontendTrajectoryPreview::default();
        preview.apply(arc());
        preview.observe_hover(true, Action::default(), MapPoint::ZERO);
        assert_eq!(preview.hover_ticks(), 1);
        assert!(preview.points().is_empty());
        assert!(preview.crumpled(), "hover invalidation does not reset tint");
        preview.observe_hover(true, Action::default(), MapPoint::ZERO);
        assert_eq!(preview.hover_ticks(), 2);
        preview.apply(arc());
        preview.observe_hover(true, Action::default(), MapPoint::new(1.0, 2.0));
        assert_eq!(preview.hover_ticks(), 0);
        assert!(!preview.is_valid());
        assert!(preview.points().is_empty());
        preview.hover_ticks = u32::MAX;
        preview.observe_hover(true, Action::default(), MapPoint::new(1.0, 2.0));
        assert_eq!(preview.hover_ticks(), u32::MAX);
    }

    #[test]
    fn marker_cadence_uses_previous_arc_and_preserves_wrapping_counter() {
        let mut preview = FrontendTrajectoryPreview::default();
        preview
            .ground_mark
            .set_sprite_data(0.0, 0.0, vec![(1, 1)], vec![(0, 0)]);
        preview.hover_ticks = 2;
        preview.apply(arc());
        for _ in 0..9 {
            preview.advance_hover_markers(1);
        }
        assert!(preview.ground_marks().marks.is_empty());
        preview.advance_hover_markers(1);
        let mark = &preview.ground_marks().marks[0];
        assert_eq!((mark.x, mark.y, mark.layer), (30.0, 35.0, 4));
        preview.mark_count = u16::MAX;
        preview.advance_hover_markers(1);
        assert_eq!(preview.mark_count(), 0);
        assert_eq!(preview.ground_marks().marks.len(), 2);
        preview.reject_hit();
        preview.advance_hover_markers(1);
        assert_eq!(preview.mark_count(), 0);
        preview.hover_ticks = 1;
        preview.apply(arc());
        preview.advance_hover_markers(1);
        assert!(!preview.is_valid());
        assert!(preview.points().is_empty());
        assert!(preview.crumpled());
    }

    #[test]
    fn restore_reset_clears_identity_and_marks_but_not_hover_observation() {
        let mut preview = FrontendTrajectoryPreview::default();
        preview.observe_hover(true, Action::default(), MapPoint::new(9.0, 8.0));
        preview.observe_hover(true, Action::default(), MapPoint::new(9.0, 8.0));
        preview.apply(arc());
        preview.reset_after_restore();
        assert_eq!(preview.hover_ticks(), 1);
        assert_eq!(preview.previous_mouse(), MapPoint::new(9.0, 8.0));
        assert!(!preview.shift_held);
        assert!(!preview.is_valid());
        assert!(preview.points().is_empty());
        assert!(!preview.crumpled());
        assert!(preview.ground_marks().marks.is_empty());
        preview.interrupt_hover();
        assert_eq!(preview.hover_ticks(), 0);
    }
}
