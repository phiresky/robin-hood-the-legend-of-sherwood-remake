//! Client-local camera and touch motion.
use super::*;

/// Host-local viewport state. This is deliberately outside
/// `robin_engine`: screen size, mouse anchoring, render culling, and
/// local scroll/zoom are presentation concerns and may differ on every
/// multiplayer peer.
#[derive(Debug, Clone)]
pub struct ViewportState {
    pub view_position: MapPoint,
    pub old_view_position: MapPoint,
    pub zoom_factor: f32,
    pub old_zoom_factor: f32,
    pub screen_size: ScreenSize,
    pub level_size: MapSize,
    touch_motion: TouchCameraMotion,
}

/// Host-only touch-camera state. Velocities are expressed in screen pixels
/// per second so momentum feels consistent at every zoom level.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct TouchCameraMotion {
    transform_active: bool,
    velocity_x: f32,
    velocity_y: f32,
    last_inertia_ms: u32,
}

impl ViewportState {
    pub fn new(screen_width: f32, screen_height: f32) -> Self {
        Self {
            view_position: MapPoint::ZERO,
            old_view_position: MapPoint::ZERO,
            zoom_factor: 1.0,
            old_zoom_factor: 1.0,
            screen_size: ScreenSize::new(screen_width, screen_height),
            level_size: MapSize::ZERO,
            touch_motion: TouchCameraMotion::default(),
        }
    }

    pub fn set_screen_size(&mut self, width: f32, height: f32) {
        self.screen_size = ScreenSize::new(width, height);
        self.clip_view();
    }

    pub fn set_level_size(&mut self, width: f32, height: f32) {
        self.level_size = MapSize::new(width, height);
        self.clip_view();
    }

    pub fn center_on_point(&mut self, point: MapPoint) {
        self.view_position = MapPoint::new(
            (point.x - self.screen_size.x / (2.0 * self.zoom_factor)).floor(),
            (point.y - self.screen_size.y / (2.0 * self.zoom_factor)).floor(),
        );
        self.clip_view();
    }

    /// Apply scripted motion from the local view. Merely locking input must
    /// not restore the stale shared view left by an earlier cutscene.
    pub fn advance_director_camera(
        &mut self,
        before: engine_api::DirectorCameraFrame,
        after: engine_api::DirectorCameraFrame,
        view_size: ScreenSize,
    ) {
        if !before.owns_view && !after.owns_view {
            return;
        }
        if before.zoom_factor != after.zoom_factor {
            // Script zooms retain the local focal point until a pan or jump
            // explicitly chooses a new one.
            self.zoom_by(after.zoom_factor / self.zoom_factor, None);
        }

        let target = after.slide_target.or_else(|| {
            before
                .slide_target
                .filter(|target| *target == after.view_position)
        });
        if let Some(target) = target {
            if self.zoom_factor != after.zoom_factor {
                self.zoom_by(after.zoom_factor / self.zoom_factor, None);
            }
            let distance = |point: MapPoint| (point.x - target.x).hypot(point.y - target.y);
            let remaining_before = distance(before.view_position);
            let progress = if remaining_before == 0.0 {
                1.0
            } else {
                (1.0 - distance(after.view_position) / remaining_before).clamp(0.0, 1.0)
            };
            // Use the shared pan's progress, but interpolate from the local
            // viewport. This preserves deterministic sequence completion and
            // makes every peer arrive at the scripted destination together.
            // TODO: a shared pan with zero distance has no duration to reuse;
            // presenting a local-only pan then needs a separate visual clock.
            self.old_view_position = self.view_position;
            let target_x =
                target.x + (view_size.x - self.screen_size.x) / (2.0 * after.zoom_factor);
            let target_y =
                target.y + (view_size.y - self.screen_size.y) / (2.0 * after.zoom_factor);
            self.view_position.x += (target_x - self.view_position.x) * progress;
            self.view_position.y += (target_y - self.view_position.y) * progress;
            self.clip_view();
        } else if before.view_position != after.view_position
            && before.zoom_factor == after.zoom_factor
        {
            // An explicit jump (including jump+unlock in one tick), or a
            // follow-camera update, still adopts the scripted framing.
            self.adopt_director_camera(after.view_position, view_size, after.zoom_factor);
        }
    }

    /// Mirror the shared script/director camera for an explicit placement.
    ///
    /// The director camera is deterministic shared state, so the engine
    /// frames its focal point inside a fixed virtual view (`view_size`)
    /// rather than any peer's canvas. Shift the top-left by half the size
    /// difference so that same focal point lands in the centre of this
    /// host's (possibly widescreen) view. The shift is exactly zero when
    /// both sizes agree, which keeps the classic 1024x768 path bit-identical.
    pub fn adopt_director_camera(
        &mut self,
        view_position: MapPoint,
        view_size: ScreenSize,
        zoom_factor: f32,
    ) {
        assert!(
            zoom_factor.is_finite() && zoom_factor > 0.0,
            "director camera supplied invalid zoom factor {zoom_factor}"
        );
        self.old_view_position = self.view_position;
        self.old_zoom_factor = self.zoom_factor;
        self.zoom_factor = zoom_factor;
        let shift_x = (view_size.x - self.screen_size.x) / (2.0 * zoom_factor);
        let shift_y = (view_size.y - self.screen_size.y) / (2.0 * zoom_factor);
        self.view_position = MapPoint::new(view_position.x + shift_x, view_position.y + shift_y);
        self.clip_view();
    }

    pub fn sound_listen_point(&self) -> MapPoint {
        MapPoint::new(
            self.view_position.x + self.screen_size.x * 0.5 / self.zoom_factor,
            self.view_position.y + (self.screen_size.y - PANNEL_HEIGHT) * 0.5 / self.zoom_factor,
        )
    }

    pub fn scroll_by(&mut self, delta: ScreenVec) {
        self.old_view_position = self.view_position;
        self.view_position.x += delta.x / self.zoom_factor;
        self.view_position.y += delta.y / self.zoom_factor;
        self.clip_view();
    }

    pub fn zoom_by(&mut self, factor: f32, mouse_screen: Option<ScreenPoint>) {
        let next = (self.zoom_factor * factor).clamp(0.5, 2.0);
        if (next - self.zoom_factor).abs() < f32::EPSILON {
            return;
        }
        let anchor = mouse_screen.unwrap_or_else(|| {
            ScreenPoint::new(self.screen_size.x * 0.5, self.screen_size.y * 0.5)
        });
        let before = self.screen_to_map_unchecked(anchor);
        self.old_zoom_factor = self.zoom_factor;
        self.zoom_factor = next;
        self.view_position = MapPoint::new(
            before.x - anchor.x / self.zoom_factor,
            before.y - anchor.y / self.zoom_factor,
        );
        self.clip_view();
    }

    /// Begin a two-finger camera transform after the gameplay layer has
    /// decided whether both fingers originated in the world viewport.
    pub fn begin_touch_transform(&mut self, accepted: bool) {
        self.touch_motion = TouchCameraMotion {
            transform_active: accepted,
            ..TouchCameraMotion::default()
        };
    }

    /// Atomically apply centroid translation and pinch scaling. The map point
    /// beneath the previous centroid remains beneath the current centroid,
    /// avoiding the order-dependent wobble caused by separate pan/zoom calls.
    pub fn apply_touch_transform(
        &mut self,
        centroid: ScreenPoint,
        pan: ScreenVec,
        scale: f32,
    ) -> bool {
        if !self.touch_motion.transform_active {
            return false;
        }
        if !scale.is_finite()
            || scale <= 0.0
            || !centroid.x.is_finite()
            || !centroid.y.is_finite()
            || !pan.x.is_finite()
            || !pan.y.is_finite()
        {
            tracing::warn!(?centroid, ?pan, scale, "ignored non-finite touch transform");
            return false;
        }

        let previous_centroid = ScreenPoint::new(centroid.x - pan.x, centroid.y - pan.y);
        let anchor = self.screen_to_map_unchecked(previous_centroid);
        self.old_view_position = self.view_position;
        self.old_zoom_factor = self.zoom_factor;
        self.zoom_factor = (self.zoom_factor * scale).clamp(0.5, 2.0);
        self.view_position = MapPoint::new(
            anchor.x - centroid.x / self.zoom_factor,
            anchor.y - centroid.y / self.zoom_factor,
        );
        self.clip_view();
        true
    }

    pub fn end_touch_transform(&mut self, velocity: ScreenVec, cancelled: bool, now_ms: u32) {
        const MAX_INERTIA_SPEED: f32 = 5_000.0;

        if !self.touch_motion.transform_active {
            self.touch_motion = TouchCameraMotion::default();
            return;
        }
        self.touch_motion.transform_active = false;
        if cancelled || !velocity.x.is_finite() || !velocity.y.is_finite() {
            self.touch_motion.velocity_x = 0.0;
            self.touch_motion.velocity_y = 0.0;
        } else {
            self.touch_motion.velocity_x = velocity.x;
            self.touch_motion.velocity_y = velocity.y;
            let speed = velocity.x.hypot(velocity.y);
            if speed > MAX_INERTIA_SPEED {
                let scale = MAX_INERTIA_SPEED / speed;
                self.touch_motion.velocity_x *= scale;
                self.touch_motion.velocity_y *= scale;
            }
        }
        self.touch_motion.last_inertia_ms = now_ms;
    }

    pub fn cancel_touch_motion(&mut self) {
        self.touch_motion = TouchCameraMotion::default();
    }

    /// Advance hard-clamped pan inertia using wall time. Returns whether the
    /// camera moved, allowing future display-rate render loops to skip static
    /// recomposition without coupling momentum to the 25 Hz simulation.
    pub fn advance_touch_inertia(&mut self, now_ms: u32) -> bool {
        const DECAY_PER_SECOND: f32 = 6.5;
        const STOP_SPEED: f32 = 18.0;
        const MAX_STEP_SECONDS: f32 = 0.050;

        if self.touch_motion.transform_active {
            self.touch_motion.last_inertia_ms = now_ms;
            return false;
        }
        let speed = self
            .touch_motion
            .velocity_x
            .hypot(self.touch_motion.velocity_y);
        if speed < STOP_SPEED {
            self.touch_motion.velocity_x = 0.0;
            self.touch_motion.velocity_y = 0.0;
            self.touch_motion.last_inertia_ms = now_ms;
            return false;
        }
        let elapsed = now_ms.wrapping_sub(self.touch_motion.last_inertia_ms) as f32 / 1000.0;
        let dt = elapsed.min(MAX_STEP_SECONDS);
        self.touch_motion.last_inertia_ms = now_ms;
        if dt <= 0.0 {
            return false;
        }

        let before = self.view_position;
        self.scroll_by(ScreenVec::new(
            -self.touch_motion.velocity_x * dt,
            -self.touch_motion.velocity_y * dt,
        ));
        if (self.view_position.x - before.x).abs() < f32::EPSILON {
            self.touch_motion.velocity_x = 0.0;
        }
        if (self.view_position.y - before.y).abs() < f32::EPSILON {
            self.touch_motion.velocity_y = 0.0;
        }
        let decay = (-DECAY_PER_SECOND * dt).exp();
        self.touch_motion.velocity_x *= decay;
        self.touch_motion.velocity_y *= decay;
        self.view_position != before
    }

    pub fn screen_to_map(&self, screen_pt: ScreenPoint) -> Option<MapPoint> {
        let map_pt = self.screen_to_map_unchecked(screen_pt);
        if map_pt.x > 0.0
            && map_pt.y > 0.0
            && map_pt.x <= self.level_size.x
            && map_pt.y <= self.level_size.y
        {
            Some(map_pt)
        } else {
            None
        }
    }

    pub fn screen_to_map_unchecked(&self, screen_pt: ScreenPoint) -> MapPoint {
        MapPoint::new(
            self.view_position.x + screen_pt.x / self.zoom_factor,
            self.view_position.y + screen_pt.y / self.zoom_factor,
        )
    }

    pub fn map_to_screen(&self, map_pt: MapPoint) -> Option<ScreenPoint> {
        let screen_pt = self.map_to_screen_unclamped(map_pt);
        if screen_pt.x >= 0.0
            && screen_pt.y >= 0.0
            && screen_pt.x <= self.screen_size.x
            && screen_pt.y <= self.screen_size.y
        {
            Some(screen_pt)
        } else {
            None
        }
    }

    pub fn map_to_screen_unclamped(&self, map_pt: MapPoint) -> ScreenPoint {
        ScreenPoint::new(
            (map_pt.x - self.view_position.x) * self.zoom_factor,
            (map_pt.y - self.view_position.y) * self.zoom_factor,
        )
    }

    fn clip_view(&mut self) {
        if self.view_position.x < 0.0 {
            self.view_position.x = 0.0;
        }
        if self.view_position.y < 0.0 {
            self.view_position.y = 0.0;
        }
        if self.level_size.x > 0.0 {
            let max_x = (self.level_size.x - self.screen_size.x / self.zoom_factor).max(0.0);
            self.view_position.x = self.view_position.x.min(max_x);
        }
        if self.level_size.y > 0.0 {
            let max_y = (self.level_size.y
                - (self.screen_size.y - PANNEL_HEIGHT) / self.zoom_factor)
                .max(0.0);
            self.view_position.y = self.view_position.y.min(max_y);
        }
    }
}

impl Default for ViewportState {
    fn default() -> Self {
        Self::new(1024.0, 768.0)
    }
}

#[cfg(test)]
mod viewport_touch_tests {
    use super::*;

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.001,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn combined_touch_transform_preserves_anchor() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(500.0, 400.0);
        let previous_centroid = ScreenPoint::new(300.0, 250.0);
        let anchor = viewport.screen_to_map_unchecked(previous_centroid);

        viewport.begin_touch_transform(true);
        viewport.apply_touch_transform(
            ScreenPoint::new(340.0, 270.0),
            ScreenVec::new(40.0, 20.0),
            1.5,
        );

        close(viewport.zoom_factor, 1.5);
        let transformed_anchor = viewport.screen_to_map_unchecked(ScreenPoint::new(340.0, 270.0));
        close(transformed_anchor.x, anchor.x);
        close(transformed_anchor.y, anchor.y);
    }

    #[test]
    fn director_pan_starts_at_local_view_and_finishes_at_script_target() {
        let mut viewport = ViewportState::new(1280.0, 720.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(2200.0, 1800.0);
        let view_size = ScreenSize::new(1024.0, 768.0);
        let idle = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(100.0, 200.0),
            zoom_factor: 1.0,
            slide_target: None,
            owns_view: false,
        };

        // LockUser and a timer before CameraGoto must leave the local view
        // where the player put it, even though the director is far away.
        let locked = engine_api::DirectorCameraFrame {
            owns_view: true,
            ..idle
        };
        viewport.advance_director_camera(idle, locked, view_size);
        assert_eq!(viewport.view_position, MapPoint::new(2200.0, 1800.0));
        let start = engine_api::DirectorCameraFrame {
            slide_target: Some(MapPoint::new(1100.0, 1200.0)),
            ..locked
        };
        viewport.advance_director_camera(locked, start, view_size);
        assert_eq!(viewport.view_position, MapPoint::new(2200.0, 1800.0));

        let halfway = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(600.0, 700.0),
            ..start
        };
        viewport.advance_director_camera(start, halfway, view_size);
        close(viewport.view_position.x, 1586.0);
        close(viewport.view_position.y, 1512.0);

        // Completion can clear the slide in the same tick that it arrives.
        let end = engine_api::DirectorCameraFrame {
            view_position: start.slide_target.unwrap(),
            slide_target: None,
            ..start
        };
        viewport.advance_director_camera(halfway, end, view_size);
        assert_eq!(viewport.view_position, MapPoint::new(972.0, 1224.0));
        viewport.advance_director_camera(end, end, view_size);
        assert_eq!(viewport.view_position, MapPoint::new(972.0, 1224.0));
    }

    #[test]
    fn director_jump_interrupts_pan_and_adopts_destination() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(2200.0, 1800.0);
        let before = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(100.0, 200.0),
            zoom_factor: 1.0,
            slide_target: Some(MapPoint::new(1100.0, 1200.0)),
            owns_view: true,
        };
        let after = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(500.0, 600.0),
            slide_target: None,
            owns_view: false,
            ..before
        };
        viewport.advance_director_camera(before, after, viewport.screen_size);
        assert_eq!(viewport.view_position, after.view_position);
    }

    #[test]
    fn director_zoom_keeps_local_focal_point() {
        let mut viewport = ViewportState::new(1280.0, 720.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(2200.0, 1800.0);
        let center = ScreenPoint::new(640.0, 360.0);
        let focal_point = viewport.screen_to_map_unchecked(center);
        let before = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(100.0, 200.0),
            zoom_factor: 1.0,
            slide_target: None,
            owns_view: true,
        };
        let after = engine_api::DirectorCameraFrame {
            view_position: MapPoint::new(356.0, 392.0),
            zoom_factor: 2.0,
            ..before
        };
        viewport.advance_director_camera(before, after, ScreenSize::new(1024.0, 768.0));
        assert_eq!(viewport.zoom_factor, 2.0);
        assert_eq!(viewport.screen_to_map_unchecked(center), focal_point);
    }

    #[test]
    fn director_camera_is_identity_on_matching_canvas() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.adopt_director_camera(
            MapPoint::new(1234.5678, 987.6543),
            ScreenSize::new(1024.0, 768.0),
            0.5,
        );
        assert_eq!(viewport.view_position, MapPoint::new(1234.5678, 987.6543));
        assert_eq!(viewport.zoom_factor, 0.5);
    }

    #[test]
    fn director_camera_recentres_focal_point_on_widescreen_canvas() {
        // The engine framed focal point (1512, 1384) as top-left (1000, 1000)
        // in its 1024x768 virtual view. A 1280x720 host must show that same
        // point in the middle of its own canvas.
        let mut viewport = ViewportState::new(1280.0, 720.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.adopt_director_camera(
            MapPoint::new(1000.0, 1000.0),
            ScreenSize::new(1024.0, 768.0),
            1.0,
        );
        assert_eq!(viewport.view_position, MapPoint::new(872.0, 1024.0));
        let centre = viewport.screen_to_map_unchecked(ScreenPoint::new(640.0, 360.0));
        assert_eq!(centre, MapPoint::new(1512.0, 1384.0));

        // Zoomed out, the shift is measured in map pixels, so it doubles.
        viewport.adopt_director_camera(
            MapPoint::new(1000.0, 1000.0),
            ScreenSize::new(1024.0, 768.0),
            0.5,
        );
        assert_eq!(viewport.view_position, MapPoint::new(744.0, 1048.0));
    }

    #[test]
    fn rejected_touch_transform_does_not_move_camera() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(500.0, 400.0);
        viewport.begin_touch_transform(false);
        viewport.apply_touch_transform(
            ScreenPoint::new(400.0, 300.0),
            ScreenVec::new(50.0, 20.0),
            1.2,
        );
        assert_eq!(viewport.view_position, MapPoint::new(500.0, 400.0));
        assert_eq!(viewport.zoom_factor, 1.0);
    }

    #[test]
    fn touch_zoom_and_inertia_hard_clamp_to_map() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(1200.0, 900.0);
        viewport.begin_touch_transform(true);
        viewport.apply_touch_transform(
            ScreenPoint::new(0.0, 0.0),
            ScreenVec::new(1000.0, 1000.0),
            0.01,
        );
        assert_eq!(viewport.zoom_factor, 0.5);
        assert_eq!(viewport.view_position, MapPoint::ZERO);

        viewport.end_touch_transform(ScreenVec::new(2000.0, 1200.0), false, 100);
        assert!(!viewport.advance_touch_inertia(116));
        assert_eq!(viewport.view_position, MapPoint::ZERO);
        assert!(!viewport.advance_touch_inertia(132));
    }

    #[test]
    fn cancelling_transform_disables_momentum() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(1000.0, 1000.0);
        viewport.begin_touch_transform(true);
        viewport.end_touch_transform(ScreenVec::new(1000.0, 0.0), true, 100);
        assert!(!viewport.advance_touch_inertia(150));
        assert_eq!(viewport.view_position, MapPoint::new(1000.0, 1000.0));
    }

    #[test]
    fn touch_inertia_clamps_implausible_release_velocity() {
        let mut viewport = ViewportState::new(1024.0, 768.0);
        viewport.set_level_size(5000.0, 5000.0);
        viewport.view_position = MapPoint::new(1000.0, 1000.0);
        viewport.begin_touch_transform(true);
        viewport.end_touch_transform(ScreenVec::new(1_000_000.0, 0.0), false, 100);

        assert!(viewport.advance_touch_inertia(116));
        close(viewport.view_position.x, 920.0);
        close(viewport.view_position.y, 1000.0);
    }
}
