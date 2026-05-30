//! Camera control: director work, zoom, scrolling, resize, coordinate conversion.

use super::*;
use crate::coordinates::{MapPoint, MapSize, MapVec, ScreenSize};
use crate::geo2d;
use crate::messenger::{Message, MessageType, SimpleMessage};

impl EngineInner {
    // ─── Script/director camera ─────────────────────────────────

    /// Read-only view of the shared script/director camera.
    /// Local player viewport projection lives in `robin_rs::Host`.

    /// Record the loaded background's pixel dimensions.
    ///
    /// Called by level loading after the `.map` picture is decoded so
    /// scroll/zoom clamps against the true map extents.
    pub(crate) fn set_level_size(&mut self, width: f32, height: f32) {
        self.cutscene_camera.level_size = MapSize::new(width, height);
    }

    pub(super) fn director_camera_view_size() -> ScreenSize {
        ScreenSize::new(1024.0, 768.0)
    }

    // ─── Display-op update ──────────────────────────────────────

    /// Request a display-op transition, keeping only the highest-priority
    /// code that has been requested this frame.
    ///
    /// The ordering is `NoBackgroundMove < Scroll < InitZoom < InZoom <
    /// Redraw`, so a mid-frame scroll request cannot downgrade an
    /// in-progress zoom or a pending full redraw. Direct assignment of
    /// `display_op` is still used by the frame-end reset in
    /// `display_state.rs` and the end-of-zoom `NoBackgroundMove` write
    /// in `DrawZoom*` — both are intentional unconditional writes
    /// rather than priority-respecting requests.
    pub(crate) fn set_operation(&mut self, display: &mut HostDisplayState, op: DisplayOpCode) {
        if (op as u8) > (display.display_op as u8) {
            display.display_op = op;
        }
    }

    // ─── Director work (camera automation) ─────────────────────

    /// Script-driven camera changes: follow-cam, camera slide, zoom dispatch.
    ///
    /// Called at the start of each `draw()` frame.
    pub(super) fn perform_director_work(&mut self, display: &mut HostDisplayState) {
        // ── Locker follow-cam ────────────────────────────────────
        // If following an NPC with locker, cancel if it dies or is
        // knocked unconscious.
        if let Some(follow_id) = self.seats[0].follow_element
            && self.seats[0].locker_active
        {
            let should_cancel = self.get_entity(follow_id).is_none_or(|e| {
                e.is_npc() && (e.is_dead() || e.human_data().is_some_and(|h| h.unconscious))
            });
            if should_cancel {
                self.seats[0].follow_element = None;
                self.seats[0].locker_active = false;
            }
        }
        if let Some(follow_id) = self.seats[0].follow_element
            && self.seats[0].locker_active
        {
            // Each frame we compute the displacement that would
            // re-anchor the followed actor at the saved screen
            // position, factoring in whether the actor is about to
            // move on its own.
            const RH_CAMERA_COUNTER: u16 = 15;

            // Snapshot what we need from the followed entity.
            let (pos_map, increment_map, increment_computed, average_speed) = {
                let entity = self
                    .get_entity(follow_id)
                    .expect("follow_element must exist while locker_active");
                let pi_opt = Some(entity.position_iface());
                let pos = entity.element_data().position_map();
                let (inc, inc_ok) = match pi_opt {
                    Some(pi) if pi.is_increment_map_computed() => (pi.get_increment_map(), true),
                    _ => (crate::coordinates::MapVec::ZERO, false),
                };
                let avg = entity.sprite().current_average_speed();
                (pos, inc, inc_ok, avg)
            };

            let view = self.cutscene_camera.view_position;
            let zoom = self.cutscene_camera.zoom_factor;
            let saved = self.cutscene_camera.position_saved;

            if self.cutscene_camera.displacement_counter == 0 {
                // `displacement = (pos - view) * zoom - saved`
                let mut displacement = MapVec::new(
                    (pos_map.x - view.x) * zoom - saved.x,
                    (pos_map.y - view.y) * zoom - saved.y,
                );
                let post = geo2d::pt(
                    (pos_map.x + increment_map.x - view.x) * zoom - saved.x,
                    (pos_map.y + increment_map.y - view.y) * zoom - saved.y,
                );

                let inc_sq = increment_map.x * increment_map.x + increment_map.y * increment_map.y;

                if !increment_computed || inc_sq == 0.0 {
                    // Stationary target — spread the catch-up over
                    // `2 * RHCAMERA_COUNTER` frames.
                    let scale = 1.0 / (2.0 * RH_CAMERA_COUNTER as f32);
                    displacement.x *= scale;
                    displacement.y *= scale;
                    self.cutscene_camera.displacement_counter = RH_CAMERA_COUNTER;
                } else {
                    // If the character's own motion already reduces
                    // the gap on an axis, zero the camera's
                    // displacement on that axis.
                    if displacement.x.abs() > post.x.abs() {
                        displacement.x = 0.0;
                    }
                    if displacement.y.abs() > post.y.abs() {
                        displacement.y = 0.0;
                    }

                    if displacement.x.abs() >= 1.0 || displacement.y.abs() >= 1.0 {
                        self.cutscene_camera.displacement_counter = RH_CAMERA_COUNTER;

                        if displacement.x.abs() > 1.0 {
                            displacement.x = average_speed * displacement.x / displacement.x.abs();
                        }
                        if displacement.y.abs() > 1.0 {
                            displacement.y = average_speed * displacement.y / displacement.y.abs();
                        }
                    }
                }

                displacement.x = displacement.x.floor();
                displacement.y = displacement.y.floor();
                self.cutscene_camera.displacement = displacement;
            }

            if self.cutscene_camera.displacement_counter > 0 {
                // Apply displacement, but snap any axis where the
                // character is already at the saved anchor.
                let point_pos = geo2d::pt((pos_map.x - view.x) * zoom, (pos_map.y - view.y) * zoom);
                self.cutscene_camera.displacement_counter -= 1;
                let mut scroll = self.cutscene_camera.displacement;
                if saved.x.floor() == point_pos.x.floor() {
                    scroll.x = 0.0;
                }
                if saved.y.floor() == point_pos.y.floor() {
                    scroll.y = 0.0;
                }
                display.background_transform.scrolling_vector = scroll;

                if self.perform_check_scroll(display)
                    || display.background_transform.scrolling_vector.x != 0.0
                    || display.background_transform.scrolling_vector.y != 0.0
                {
                    self.set_operation(display, DisplayOpCode::Scroll);
                    return;
                }
            } else {
                self.set_operation(display, DisplayOpCode::NoBackgroundMove);
                return;
            }
        }
        // ── Desired zoom factor dispatch ─────────────────────────
        // If a script requested a specific zoom factor, dispatch zoom messages.
        if self.cutscene_camera.desired_zoom_factor > 0.0
            && (self.cutscene_camera.desired_zoom_factor - self.cutscene_camera.zoom_factor).abs()
                < f32::EPSILON
        {
            self.cutscene_camera.desired_zoom_factor = -1.0;
            // Zoom reached target, release the latched ZoomLevel
            // sequence element.
            if let Some(r) = self.cutscene_camera.sequence_element.take() {
                self.sequence_manager
                    .element_terminated(r.sequence_id, r.element_index);
            }
        }

        if self.cutscene_camera.desired_zoom_factor > 0.0
            && (self.cutscene_camera.desired_zoom_factor - self.cutscene_camera.zoom_factor).abs()
                > f32::EPSILON
            && !self.cutscene_camera.zoom_init_done
        {
            if self.cutscene_camera.desired_zoom_factor > self.cutscene_camera.zoom_factor {
                if self.is_zoom_up_possible() {
                    self.cutscene_camera.mechanized_zoom = true;
                    self.messenger.send(Message::with_value(
                        MessageType::Simple(SimpleMessage::ZoomUp),
                        1,
                    ));
                } else {
                    self.cutscene_camera.desired_zoom_factor = self.cutscene_camera.zoom_factor;
                }
            } else if self.is_zoom_down_possible() {
                self.cutscene_camera.mechanized_zoom = true;
                self.messenger.send(Message::with_value(
                    MessageType::Simple(SimpleMessage::ZoomDown),
                    1,
                ));
            } else {
                self.cutscene_camera.desired_zoom_factor = self.cutscene_camera.zoom_factor;
            }
        }

        // ── Delayed zoom requests ────────────────────────────────
        if display.background_transform.required_zoom_down {
            display.background_transform.required_zoom_down = false;
            self.messenger
                .send(Message::new(MessageType::Simple(SimpleMessage::ZoomDown)));
        }
        if display.background_transform.required_zoom_up {
            display.background_transform.required_zoom_up = false;
            self.messenger
                .send(Message::new(MessageType::Simple(SimpleMessage::ZoomUp)));
        }

        // ── Camera slide animation ───────────────────────────────
        if self.cutscene_camera.is_sliding() {
            if self.cutscene_camera.camera_slide != self.cutscene_camera.view_position {
                let approach = geo2d::pt(
                    self.cutscene_camera.camera_slide.x - self.cutscene_camera.view_position.x,
                    self.cutscene_camera.camera_slide.y - self.cutscene_camera.view_position.y,
                );
                let approach_sq = approach.x * approach.x + approach.y * approach.y;
                let approach_len = approach_sq.sqrt();

                // Compute scroll vector along approach direction
                let slide_speed = if self.cutscene_camera.fixed_camera_speed == 0 {
                    self.speed
                } else {
                    self.cutscene_camera.fixed_camera_speed as f32
                };
                let mut scroll = if approach_len > 0.0 {
                    geo2d::pt(
                        approach.x / approach_len * slide_speed,
                        approach.y / approach_len * slide_speed,
                    )
                } else {
                    geo2d::pt(0.0, 0.0)
                };

                // Don't overshoot
                let scroll_sq = scroll.x * scroll.x + scroll.y * scroll.y;
                if scroll_sq > approach_sq {
                    scroll = approach;
                }

                // Truncate to integer pixels via truncate-toward-zero
                // (not floor). For slides with negative fractional
                // components this matters: `scroll = (-0.8, 0)` must
                // stay `(0, 0)`, not round to `(-1, 0)`.
                display.background_transform.scrolling_vector =
                    MapVec::new(scroll.x.trunc(), scroll.y.trunc());

                let valid = self.perform_check_scroll(display);

                if !valid {
                    // Can't scroll further — cancel slide
                    self.cutscene_camera.stop_slide();
                    self.speed = 1.0;
                    self.pending_side_effects.invalidate_background = true;
                    display.background_transform.scrolling_vector = MapVec::ZERO;
                    // Slide clipped at level edge, release the latched
                    // CameraGoto element.
                    if let Some(r) = self.cutscene_camera.sequence_element.take() {
                        self.sequence_manager
                            .element_terminated(r.sequence_id, r.element_index);
                    }
                } else {
                    // Accelerate slide speed
                    if self.speed == 1.0 {
                        self.speed_int = 0;
                    } else {
                        self.speed_int = (self.speed_int + 1).min(31);
                    }
                    self.speed =
                        display.background_transform.y_scrolling_values[self.speed_int as usize];

                    if display.background_transform.scrolling_vector.x != 0.0
                        || display.background_transform.scrolling_vector.y != 0.0
                    {
                        self.set_operation(display, DisplayOpCode::Scroll);
                    }
                }
            } else {
                // Already at target
                self.cutscene_camera.stop_slide();
                self.speed = 1.0;
                self.pending_side_effects.invalidate_background = true;
                display.background_transform.scrolling_vector = MapVec::ZERO;
                // Slide reached target, release the latched
                // CameraGoto element.
                if let Some(r) = self.cutscene_camera.sequence_element.take() {
                    self.sequence_manager
                        .element_terminated(r.sequence_id, r.element_index);
                }
            }
        }
    }

    /// Apply deceleration to scrolling when no input is active.
    /// Matches the scroll deceleration logic at the top of Draw().
    pub(super) fn decelerate_scrolling(&mut self, display: &mut HostDisplayState) {
        let already = display.frame_scrolled;
        let zoom = self.cutscene_camera.zoom_factor;
        let mut scroll_requested = false;
        let bg = &mut display.background_transform;

        // X-axis deceleration
        if !already[ScrollDirection::Left as usize]
            && !already[ScrollDirection::Right as usize]
            && bg.current_x_scrolling_level != 0
        {
            bg.current_x_scrolling_level -= 1;
            scroll_requested = true;
            let idx = bg.current_x_scrolling_level as usize;
            let speed = bg.x_scrolling_values[idx] / zoom;
            bg.scrolling_vector.x = if bg.scroll_to_left { -speed } else { speed };
        }

        // Y-axis deceleration
        if !already[ScrollDirection::Up as usize]
            && !already[ScrollDirection::Down as usize]
            && bg.current_y_scrolling_level != 0
        {
            bg.current_y_scrolling_level -= 1;
            scroll_requested = true;
            let idx = bg.current_y_scrolling_level as usize;
            let speed = bg.y_scrolling_values[idx] / zoom;
            bg.scrolling_vector.y = if bg.scroll_to_up { -speed } else { speed };
        }

        if scroll_requested {
            self.set_operation(display, DisplayOpCode::Scroll);
        }
    }

    /// Validate and clamp the current scroll vector to level bounds.
    ///
    /// Returns `true` if the scroll was entirely within bounds,
    /// `false` if clamping was needed.
    pub(super) fn perform_check_scroll(&mut self, display: &mut HostDisplayState) -> bool {
        let mut valid = true;

        let view_x = self.cutscene_camera.view_position.x;
        let view_y = self.cutscene_camera.view_position.y;
        let screen = Self::director_camera_view_size();
        let screen_x = screen.x;
        let screen_y = screen.y;
        let level_x = self.cutscene_camera.level_size.x;
        let level_y = self.cutscene_camera.level_size.y;
        let zoom = self.cutscene_camera.zoom_factor;

        let bg = &mut display.background_transform;

        // Right boundary
        if view_x + bg.scrolling_vector.x + (screen_x / zoom) > level_x {
            valid = false;
            bg.current_x_scrolling_level = 0;
            bg.scrolling_vector.x = level_x - view_x - (screen_x / zoom);
        }

        // Left boundary
        if view_x + bg.scrolling_vector.x < 0.0 {
            valid = false;
            bg.current_x_scrolling_level = 0;
            bg.scrolling_vector.x = -view_x;
        }

        // Bottom boundary
        if view_y + bg.scrolling_vector.y + ((screen_y - PANNEL_HEIGHT) / zoom) > level_y {
            valid = false;
            bg.current_y_scrolling_level = 0;
            bg.scrolling_vector.y = level_y - ((screen_y - PANNEL_HEIGHT) / zoom) - view_y;
        }

        // Top boundary
        if view_y + bg.scrolling_vector.y < 0.0 {
            valid = false;
            bg.current_y_scrolling_level = 0;
            bg.scrolling_vector.y = -view_y;
        }

        valid
    }

    // ─── Sound ───────────────────────────────────────────────────

    /// Update the sound listener position from the shared script/director camera.
    ///
    /// The listen point is the center of the visible game area (excluding the
    /// bottom UI panel).
    pub(super) fn update_sound_listener_position(&mut self) {
        let listen_point = MapPoint::new(
            self.cutscene_camera.view_position.x
                + Self::director_camera_view_size().x * 0.5 / self.cutscene_camera.zoom_factor,
            self.cutscene_camera.view_position.y
                + (Self::director_camera_view_size().y - PANNEL_HEIGHT) * 0.5
                    / self.cutscene_camera.zoom_factor,
        );
        self.pending_side_effects
            .sounds
            .push(super::SoundCommand::SetListenPoint {
                position: listen_point,
                zoom: self.cutscene_camera.zoom_factor,
            });
    }

    // ─── Camera ──────────────────────────────────────────────────

    /// Set the shared script/director camera view position (with clamping).
    pub(crate) fn set_view_position_for_seat(&mut self, _seat: usize, pos: MapPoint) {
        self.cutscene_camera.view_position = pos;
        self.cutscene_camera.clip_view();
    }

    /// Given a raw script point in map coordinates, compute the camera
    /// **top-left** that centers the point on screen, clamps it inside
    /// the level, and applies the script-side effects in one go:
    ///   - center on both axes with the **full** director view size
    ///     divided by `2 * zoom_factor` (PANNEL_HEIGHT only enters the
    ///     bottom-clip check, never the centering itself);
    ///   - integer-truncate (toward zero) the centered point before
    ///     clamping;
    ///   - on **double-axis** over-clip (level smaller than the
    ///     zoomed-out viewport on either axis-pair), reset
    ///     `zoom_factor` to 1.0 and return `(0, 0)`;
    ///   - at `zoom_factor == 0.5`, decrement odd output coords so the
    ///     blitter stays on even pixels.
    ///
    /// Single source of truth for every script-driven camera
    /// placement: `ScrollCameraTo`, `ScrollCameraSlowlyTo`,
    /// `JumpCameraTo`, the sequence `CameraGoto` / `CameraJumpTo`
    /// commands, and `Resize` re-deriving the slide target from the
    /// stored raw `camera_wanted` script point.
    pub(crate) fn check_location_is_valid_for_camera(&mut self, point: MapPoint) -> MapPoint {
        let screen = Self::director_camera_view_size();
        let half_w = screen.x / (2.0 * self.cutscene_camera.zoom_factor);
        let half_h = screen.y / (2.0 * self.cutscene_camera.zoom_factor);

        // Truncate toward zero (not floor).
        let mut x = ((point.x - half_w) as i32) as f32;
        let mut y = ((point.y - half_h) as i32) as f32;

        let mut clipped_h = false;
        let mut clipped_v = false;

        if x < 0.0 {
            x = 0.0;
            clipped_h = true;
        }
        if y < 0.0 {
            y = 0.0;
            clipped_v = true;
        }

        let view_w = screen.x / self.cutscene_camera.zoom_factor;
        if x + view_w > self.cutscene_camera.level_size.x {
            if clipped_h {
                self.cutscene_camera.zoom_factor = 1.0;
                return MapPoint::ZERO;
            }
            x = self.cutscene_camera.level_size.x - view_w;
        }

        let view_h = (screen.y - PANNEL_HEIGHT) / self.cutscene_camera.zoom_factor;
        if y + view_h > self.cutscene_camera.level_size.y {
            if clipped_v {
                self.cutscene_camera.zoom_factor = 1.0;
                return MapPoint::ZERO;
            }
            y = self.cutscene_camera.level_size.y - view_h;
        }

        if self.cutscene_camera.zoom_factor == 0.5 {
            if (x as i32) & 1 != 0 {
                x -= 1.0;
            }
            if (y as i32) & 1 != 0 {
                y -= 1.0;
            }
            debug_assert!(x >= 0.0 && y >= 0.0);
        }

        MapPoint::new(x, y)
    }

    /// Center the shared script/director camera on a map point.
    ///
    /// Both axes of the half-screen offset use the raw director view size
    /// divided by `2 * zoom` — the bottom-panel exclusion only kicks in
    /// for the clamp (handled in `CameraState::clip_view`), not the
    /// centering itself.  The result is integer-floored before
    /// assignment.  Side effects: the per-axis scroll stepper is reset
    /// and the background is invalidated.
    ///
    /// Side effects also queue a rubber-band cancel via
    /// `pending_side_effects.cancel_multi_selection`.  Those flags live
    /// on `InputState` which the host owns; `apply_side_effects` clears
    /// them.
    pub(crate) fn center_on_point(&mut self, seat: usize, point: MapPoint) {
        let half_screen = geo2d::pt(
            Self::director_camera_view_size().x / (2.0 * self.cutscene_camera.zoom_factor),
            Self::director_camera_view_size().y / (2.0 * self.cutscene_camera.zoom_factor),
        );
        let target = MapPoint::new(
            (point.x - half_screen.x).floor(),
            (point.y - half_screen.y).floor(),
        );
        self.set_view_position_for_seat(seat, target);
        self.pending_side_effects.invalidate_background = true;
        self.pending_side_effects.cancel_multi_selection = true;
    }

    // ─── Zoom ────────────────────────────────────────────────────

    /// Whether a zoom operation is currently possible (no zoom in
    /// progress) for the host seat.  Single-player + UI gating
    /// callers use this; per-seat dispatch uses
    /// [`Self::is_zoom_possible_for_seat`].
    pub fn is_zoom_possible(&self, _display: &HostDisplayState) -> bool {
        self.is_camera_zoom_possible_for_seat(0)
    }

    /// Per-seat variant of [`Self::is_zoom_possible`].
    pub fn is_zoom_possible_for_seat(&self, display: &HostDisplayState, _seat: usize) -> bool {
        !self.cutscene_camera.zoom_init_done
            && display.display_op != DisplayOpCode::InZoom
            && !display.background_transform.zoom_to_up
            && !display.background_transform.zoom_to_down
    }

    fn is_camera_zoom_possible_for_seat(&self, seat: usize) -> bool {
        let display = self.cutscene_camera.display.to_host_display_state();
        self.is_zoom_possible_for_seat(&display, seat)
    }

    /// Whether a zoom is currently in progress.
    pub fn is_zooming(&self, _display: &HostDisplayState) -> bool {
        !self.is_camera_zoom_possible_for_seat(0)
    }

    /// Whether a zoom-up transition is currently in flight.  Set when
    /// `MSG_ZOOM_UP_START` fires and cleared at `MSG_ZOOM_UP_END`.  Used
    /// by HUD code to pin the zoom+ widget to selected for the duration
    /// of the transition.
    pub fn is_zoom_up_in_progress(&self, _display: &HostDisplayState) -> bool {
        self.cutscene_camera.display.background_transform.zoom_to_up
    }

    /// Companion to [`Self::is_zoom_up_in_progress`] for zoom-out
    /// transitions.
    pub fn is_zoom_down_in_progress(&self, _display: &HostDisplayState) -> bool {
        self.cutscene_camera
            .display
            .background_transform
            .zoom_to_down
    }

    /// Whether zooming in (2x) is possible for the host seat.
    pub fn is_zoom_up_possible(&self) -> bool {
        self.is_zoom_up_possible_for_seat(0)
    }

    /// Per-seat variant of [`Self::is_zoom_up_possible`].
    pub fn is_zoom_up_possible_for_seat(&self, _seat: usize) -> bool {
        self.cutscene_camera.zoom_factor < 2.0
    }

    /// Whether zooming out (0.5x) is possible for the host seat.
    pub fn is_zoom_down_possible(&self) -> bool {
        self.is_zoom_down_possible_for_seat(0)
    }

    /// Per-seat variant of [`Self::is_zoom_down_possible`].
    pub fn is_zoom_down_possible_for_seat(&self, _seat: usize) -> bool {
        if self.cutscene_camera.zoom_factor <= 0.5 {
            return false;
        }
        let factor = 2.0 / self.cutscene_camera.zoom_factor;
        let screen = Self::director_camera_view_size();
        screen.x * factor <= self.cutscene_camera.level_size.x
            && (screen.y - PANNEL_HEIGHT) * factor <= self.cutscene_camera.level_size.y
    }

    /// Execute one step of the zoom animation and finalize when complete.
    pub(super) fn perform_zoom_step(&mut self, display: &mut HostDisplayState) {
        display.background_transform.zoom_count += 1;
        let count = display.background_transform.zoom_count;
        let steps = display.background_transform.number_of_zoom_steps;

        if count >= steps {
            // Zoom animation complete — snap to target and finalize.
            let zoom_up = self.is_zoom_up_possible_for_seat(0) as u32;
            let zoom_down = self.is_zoom_down_possible_for_seat(0) as u32;

            self.cutscene_camera.zoom_factor = display.background_transform.zoom_to;
            let target = display.background_transform.view_to;
            self.set_view_position_for_seat(0, target);

            display.display_op = DisplayOpCode::NoBackgroundMove;
            self.cutscene_camera.zoom_init_done = false;

            if display.background_transform.zoom_to_up {
                self.messenger.send(Message::with_value(
                    MessageType::Simple(SimpleMessage::ZoomUpEnd),
                    (zoom_up << 16) | zoom_down,
                ));
                display.background_transform.zoom_to_up = false;
            } else {
                self.messenger.send(Message::with_value(
                    MessageType::Simple(SimpleMessage::ZoomDownEnd),
                    (zoom_up << 16) | zoom_down,
                ));
                display.background_transform.zoom_to_down = false;
                self.pending_side_effects.invalidate_background = true;
            }

            self.cutscene_camera.old_view_position = self.cutscene_camera.view_position;
            self.cutscene_camera.old_zoom_factor = self.cutscene_camera.zoom_factor;
        } else {
            // Interpolate zoom / view between the endpoints captured in
            // `InitZoom`.  The Rust renderer re-composes live each
            // frame, so we just drive the camera to the interpolated
            // state and let the normal `draw_background` + entity
            // pipeline do the work.
            let t = count as f32 / steps as f32;
            let zoom_from = display.background_transform.zoom_from;
            let zoom_to = display.background_transform.zoom_to;
            let view_from = display.background_transform.view_from;
            let view_to = display.background_transform.view_to;

            // Linear interpolation in zoom-factor space is uniform in
            // apparent size per step; indistinguishable from a stretch-
            // blit ramp at 8 frames.
            self.cutscene_camera.zoom_factor = zoom_from + (zoom_to - zoom_from) * t;
            let interp = MapPoint::new(
                view_from.x + (view_to.x - view_from.x) * t,
                view_from.y + (view_to.y - view_from.y) * t,
            );
            self.set_view_position_for_seat(0, interp);

            self.set_operation(display, DisplayOpCode::InZoom);
        }
    }

    // ─── Resize ──────────────────────────────────────────────────

    /// Handle a window/screen resize.
    #[cfg(test)]
    pub(crate) fn resize(
        &mut self,
        display: &mut HostDisplayState,
        new_width: f32,
        new_height: f32,
    ) {
        let _ = (new_width, new_height);
        // The Rust engine doesn't own a cache surface — the host
        // renderer does, and `invalidate_background` below signals it
        // to drop and rebuild on the next frame.

        // Recalculate camera slide target if one is active.  Re-runs
        // `check_location_is_valid_for_camera(camera_wanted)` against
        // the new screen size so the slide target re-centers on the
        // originally-requested raw script point — possible only because
        // `camera_wanted` is stored as the raw point (not the clipped
        // top-left).
        if self.cutscene_camera.is_sliding() {
            let wanted = self.cutscene_camera.camera_wanted;
            self.cutscene_camera.camera_slide = self.check_location_is_valid_for_camera(wanted);
        }

        // Abort any in-progress zoom
        if display.display_op == DisplayOpCode::InZoom {
            self.cutscene_camera.zoom_init_done = false;
            // Finalize the zoom abruptly.
            if display.background_transform.zoom_to_up {
                display.background_transform.zoom_to_up = false;
            } else {
                display.background_transform.zoom_to_down = false;
            }
            self.cutscene_camera.old_view_position = self.cutscene_camera.view_position;
            self.cutscene_camera.old_zoom_factor = self.cutscene_camera.zoom_factor;
        }

        self.pending_side_effects.invalidate_background = true;

        // If at 0.5x zoom and can't zoom down anymore, snap to 1x
        if self.cutscene_camera.zoom_factor == 0.5 && !self.is_zoom_down_possible() {
            self.cutscene_camera.zoom_factor = 1.0;
            display.background_transform.current_zoom_level += 1;
        }

        // Re-center and clamp camera
        let center = MapPoint::new(
            self.cutscene_camera.view_position.x + new_width * 0.5,
            self.cutscene_camera.view_position.y + new_height * 0.5,
        );
        self.center_on_point(0, center);
    }
}
