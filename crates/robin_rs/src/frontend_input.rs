//! Host-only input lifecycle. Profile policy, sticky planning and paired pointer
//! events are not simulation state and never enter saves or replay commands.
use crate::gfx_types::GameEvent;
use serde::{Deserialize, Serialize};

/// One owner for paired pointer metadata and the gesture sampled by that
/// sequence. Sampled engine input stays separate, but lifecycle operations
/// require it so cancellation cannot leave a live drag behind the capture.
#[derive(Debug, Default, Serialize)]
pub(crate) struct FrontendPointerSequence {
    capture: FrontendPointerCapture,
    mouse_way: crate::mouse_way::MouseWay,
}

impl<'de> Deserialize<'de> for FrontendPointerSequence {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "live pointer sequences cannot be restored",
        ))
    }
}

impl FrontendPointerSequence {
    pub(crate) fn capture(&self) -> &FrontendPointerCapture {
        &self.capture
    }
    pub(crate) fn mouse_way(&self) -> &crate::mouse_way::MouseWay {
        &self.mouse_way
    }
    pub(crate) fn add_point(&mut self, point: robin_engine::coordinates::ScreenPoint) {
        self.mouse_way.add_point(point);
    }
    pub(crate) fn advance_trail(&mut self, trail: &crate::mouse_trail::MouseTrailRenderer) {
        trail.advance(&mut self.mouse_way);
    }
    pub(crate) fn clear_gesture(&mut self) {
        self.mouse_way.clear();
    }
    pub(crate) fn begin_left(
        &mut self,
        input: &mut robin_engine::engine::InputState,
        point: robin_engine::coordinates::ScreenPoint,
        clicks: u8,
    ) {
        input.press_left_pointer(point, clicks);
        self.mouse_way.clear();
    }
    pub(crate) fn release_left(&mut self, input: &mut robin_engine::engine::InputState) -> bool {
        // Keep the path until release dispatch has evaluated the gesture.
        input.release_left_pointer()
    }
    pub(crate) fn begin_right(&mut self, input: &mut robin_engine::engine::InputState, clicks: u8) {
        input.right_mouse_down = true;
        self.capture.right_button_down(clicks);
    }
    pub(crate) fn release_right(&mut self, input: &mut robin_engine::engine::InputState) -> bool {
        input.right_mouse_down = false;
        self.capture.take_right_double_click()
    }
    pub(crate) fn cancel_left(&mut self, input: &mut robin_engine::engine::InputState) {
        input.cancel_left_pointer();
        self.capture.touch_plan_captured = false;
        if self.capture.hud_button == Some(1) {
            self.capture.hud_button = None;
        }
        self.capture.end_minimap_drag();
        self.mouse_way.clear();
    }
    pub(crate) fn reset(&mut self, input: &mut robin_engine::engine::InputState) {
        input.reset_pointer_sequence();
        self.capture.cancel_sequence();
        self.mouse_way.clear();
    }
    pub(crate) fn reset_modal(&mut self, input: &mut robin_engine::engine::InputState) {
        // Preserve the engine's held-without-drag/suppression semantics.
        input.reset_modal_input();
        self.capture.cancel_sequence();
        self.mouse_way.clear();
    }
    pub(crate) fn begin_minimap_drag(&mut self, camera: bool) {
        self.capture.begin_minimap_drag(camera);
    }
    pub(crate) fn end_minimap_drag(&mut self) {
        self.capture.end_minimap_drag();
    }
    pub(crate) fn route_hud_event(&mut self, event: &GameEvent, hit: bool) -> bool {
        self.capture.route_hud_event(event, hit)
    }
    pub(crate) fn route_touch_plan_event(
        &mut self,
        planning: &mut FrontendPlanning,
        event: &GameEvent,
        admit_touch: bool,
        hit_test: impl FnOnce(i32, i32) -> bool,
    ) -> TouchPlanRoute {
        self.capture
            .route_touch_plan_event(planning, event, admit_touch, hit_test)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FrontendPlanning {
    preference_enabled: bool,
    forced_off: bool,
    touch_latched: bool,
}

impl FrontendPlanning {
    pub fn new(preference_enabled: bool) -> Self {
        Self {
            preference_enabled,
            ..Self::default()
        }
    }

    pub fn enabled(&self) -> bool {
        self.preference_enabled && !self.forced_off
    }

    pub fn touch_latched(&self) -> bool {
        self.enabled() && self.touch_latched
    }

    /// Options cannot override replay/parity session policy.
    pub fn update_preference(&mut self, enabled: bool) {
        self.preference_enabled = enabled;
        if !self.enabled() {
            self.cancel_touch();
        }
    }

    pub fn force_off_for_session(&mut self) {
        self.forced_off = true;
        self.cancel_touch();
    }

    pub fn toggle_touch(&mut self) {
        assert!(
            self.enabled(),
            "touch planning requires enabled session policy"
        );
        self.touch_latched = !self.touch_latched;
    }

    pub fn cancel_touch(&mut self) {
        self.touch_latched = false;
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FrontendPointerCapture {
    right_double_click_pending: bool,
    touch_plan_captured: bool,
    #[serde(default)]
    hud_button: Option<u8>,
    #[serde(default)]
    minimap_camera_drag: Option<bool>,
}

/// One chronological routing decision; only the caller dispatches commands.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TouchPlanRoute {
    Forward,
    Captured,
    Toggled { cancel_planned: bool },
}

impl FrontendPointerCapture {
    pub fn begin_minimap_drag(&mut self, camera: bool) {
        self.minimap_camera_drag = Some(camera);
    }

    pub fn minimap_drag_active(&self) -> bool {
        self.minimap_camera_drag.is_some()
    }

    pub fn minimap_camera_drag_active(&self) -> bool {
        self.minimap_camera_drag == Some(true)
    }

    pub fn end_minimap_drag(&mut self) {
        self.minimap_camera_drag = None;
    }

    /// Capture a HUD press through its release, even outside the widget.
    pub fn route_hud_event(&mut self, event: &GameEvent, hit: bool) -> bool {
        if let Some(button) = self.hud_button {
            match *event {
                GameEvent::PointerCancel => self.hud_button = None,
                GameEvent::MouseUp(_, _, released) if released == button => {
                    self.hud_button = None;
                    return true;
                }
                GameEvent::MouseDown(_, _, pressed, _) if pressed == button => return true,
                GameEvent::MouseMove { .. } => return true,
                _ => {}
            }
        }
        if hit && let GameEvent::MouseDown(_, _, button, _) = *event {
            self.hud_button = Some(button);
            return true;
        }
        false
    }

    pub fn right_button_down(&mut self, clicks: u8) {
        self.right_double_click_pending = clicks >= 2;
    }

    pub fn take_right_double_click(&mut self) -> bool {
        std::mem::take(&mut self.right_double_click_pending)
    }

    pub fn capture_touch_plan(&mut self) {
        self.touch_plan_captured = true;
    }
    pub fn touch_plan_captured(&self) -> bool {
        self.touch_plan_captured
    }
    /// Admit presses only when enabled, but retire an already captured sequence
    /// even if policy changes before its release. Never inspect a future event:
    /// world input on either side of a HUD tap belongs to the world.
    pub fn route_touch_plan_event(
        &mut self,
        planning: &mut FrontendPlanning,
        event: &GameEvent,
        admit_touch: bool,
        hit_test: impl FnOnce(i32, i32) -> bool,
    ) -> TouchPlanRoute {
        if self.touch_plan_captured {
            match event {
                GameEvent::PointerCancel => {
                    self.touch_plan_captured = false;
                    return TouchPlanRoute::Forward;
                }
                GameEvent::MouseUp(_, _, 1) => {
                    self.touch_plan_captured = false;
                    return TouchPlanRoute::Captured;
                }
                GameEvent::MouseDown(_, _, 1, _) | GameEvent::MouseMove { .. } => {
                    return TouchPlanRoute::Captured;
                }
                _ => return TouchPlanRoute::Forward,
            }
        }
        if let GameEvent::MouseDown(x, y, 1, _) = *event
            && admit_touch
            && planning.enabled()
            && hit_test(x, y)
        {
            self.capture_touch_plan();
            planning.toggle_touch();
            return TouchPlanRoute::Toggled {
                cancel_planned: !planning.touch_latched(),
            };
        }
        TouchPlanRoute::Forward
    }

    /// Modal close, engine cancellation and snapshot restoration all retire
    /// the complete pointer sequence, including deferred release metadata.
    pub fn cancel_sequence(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_preserves_gesture_for_dispatch_and_next_press_retires_it() {
        let mut sequence = FrontendPointerSequence::default();
        let mut input = robin_engine::engine::InputState::default();
        sequence.begin_left(&mut input, Default::default(), 2);
        sequence.add_point(Default::default());
        assert!(sequence.release_left(&mut input));
        assert!(!input.left_mouse_down());
        assert!(!sequence.mouse_way().is_empty());
        assert!(!sequence.release_left(&mut input));
        sequence.begin_left(&mut input, Default::default(), 1);
        assert!(sequence.mouse_way().is_empty());
    }

    #[test]
    fn touch_takeover_cancels_left_sequence_without_consuming_right_release() {
        let mut sequence = FrontendPointerSequence::default();
        let mut input = robin_engine::engine::InputState::default();
        sequence.begin_left(&mut input, Default::default(), 2);
        sequence.begin_right(&mut input, 2);
        sequence.begin_minimap_drag(true);
        sequence.add_point(Default::default());
        sequence.cancel_left(&mut input);
        assert!(!input.left_mouse_down());
        assert!(!sequence.capture().minimap_drag_active());
        assert!(sequence.mouse_way().is_empty());
        assert!(!sequence.release_left(&mut input));
        assert!(input.right_mouse_down);
        assert!(sequence.release_right(&mut input));
        assert!(!sequence.release_right(&mut input));
    }

    #[test]
    fn pointer_diagnostic_cannot_recreate_live_sequence() {
        let sequence = FrontendPointerSequence::default();
        let bytes = serde_json::to_vec(&sequence).unwrap();
        assert!(serde_json::from_slice::<FrontendPointerSequence>(&bytes).is_err());
    }

    #[test]
    fn hud_press_captures_drag_and_release_across_frames() {
        let mut capture = FrontendPointerCapture::default();
        assert!(capture.route_hud_event(&GameEvent::MouseDown(10, 10, 1, 1), true));
        assert!(capture.route_hud_event(
            &GameEvent::MouseMove {
                x: 100,
                y: 100,
                xrel: 90,
                yrel: 90
            },
            false
        ));
        assert!(capture.route_hud_event(&GameEvent::MouseUp(100, 100, 1), false));
        assert!(!capture.route_hud_event(&GameEvent::MouseDown(100, 100, 1, 1), false));
        assert!(!capture.route_hud_event(&GameEvent::MouseUp(100, 100, 1), false));
    }

    #[test]
    fn hud_capture_cancels_and_does_not_consume_other_buttons() {
        let mut capture = FrontendPointerCapture::default();
        assert!(capture.route_hud_event(&GameEvent::MouseDown(10, 10, 3, 1), true));
        assert!(!capture.route_hud_event(&GameEvent::MouseUp(10, 10, 1), false));
        assert!(!capture.route_hud_event(&GameEvent::PointerCancel, false));
        assert!(!capture.route_hud_event(&GameEvent::MouseUp(10, 10, 3), false));
    }

    #[test]
    fn minimap_gesture_is_captured_before_queued_commands_run() {
        let mut capture = FrontendPointerCapture::default();
        capture.begin_minimap_drag(true);
        assert!(capture.minimap_drag_active());
        assert!(capture.minimap_camera_drag_active());
        capture.end_minimap_drag();
        assert!(!capture.minimap_drag_active());
        capture.begin_minimap_drag(false);
        assert!(capture.minimap_drag_active());
        assert!(!capture.minimap_camera_drag_active());
        capture.cancel_sequence();
        assert!(!capture.minimap_drag_active());
    }

    #[test]
    fn session_policy_survives_preference_updates() {
        let mut planning = FrontendPlanning::new(true);
        planning.toggle_touch();
        planning.force_off_for_session();
        planning.update_preference(true);
        assert!(!planning.enabled());
        assert!(!planning.touch_latched());
    }

    #[test]
    fn disabling_preference_retires_sticky_planning() {
        let mut planning = FrontendPlanning::new(true);
        planning.toggle_touch();
        planning.update_preference(false);
        planning.update_preference(true);
        assert!(!planning.touch_latched());
    }

    #[test]
    fn pointer_metadata_is_one_shot_and_resettable() {
        let mut capture = FrontendPointerCapture::default();
        capture.right_button_down(2);
        assert!(capture.take_right_double_click());
        assert!(!capture.take_right_double_click());
        capture.right_button_down(2);
        capture.capture_touch_plan();
        capture.cancel_sequence();
        assert!(!capture.take_right_double_click());
        assert!(!capture.touch_plan_captured());
    }

    #[test]
    fn captured_press_release_batch_preserves_unrelated_events() {
        let mut capture = FrontendPointerCapture::default();
        let mut planning = FrontendPlanning::new(true);
        let events = vec![
            GameEvent::MouseDown(0, 0, 1, 1),
            GameEvent::MouseUp(0, 0, 1),
            GameEvent::MouseDown(10, 20, 1, 1),
            GameEvent::MouseMove {
                x: 11,
                y: 20,
                xrel: 1,
                yrel: 0,
            },
            GameEvent::MouseUp(11, 20, 1),
            GameEvent::MouseDown(0, 0, 1, 1),
            GameEvent::MouseUp(0, 0, 1),
            GameEvent::MouseDown(11, 20, 3, 1),
            GameEvent::Quit,
        ];
        let decisions: Vec<_> = events
            .iter()
            .map(|event| capture.route_touch_plan_event(&mut planning, event, true, |x, _| x == 10))
            .collect();
        assert!(!capture.touch_plan_captured());
        use TouchPlanRoute::*;
        assert_eq!(
            decisions,
            [
                Forward,
                Forward,
                Toggled {
                    cancel_planned: false
                },
                Captured,
                Captured,
                Forward,
                Forward,
                Forward,
                Forward
            ]
        );
    }

    #[test]
    fn capture_survives_batches_until_matching_release() {
        let mut capture = FrontendPointerCapture::default();
        let mut planning = FrontendPlanning::new(true);
        let route = |capture: &mut FrontendPointerCapture,
                     planning: &mut FrontendPlanning,
                     event,
                     admit| {
            capture.route_touch_plan_event(planning, &event, admit, |_, _| true)
        };
        assert_eq!(
            route(
                &mut capture,
                &mut planning,
                GameEvent::MouseDown(0, 0, 1, 1),
                true
            ),
            TouchPlanRoute::Toggled {
                cancel_planned: false
            }
        );
        assert_eq!(
            route(
                &mut capture,
                &mut planning,
                GameEvent::MouseUp(0, 0, 3),
                false
            ),
            TouchPlanRoute::Forward
        );
        assert!(capture.touch_plan_captured());
        planning.update_preference(false);
        assert_eq!(
            route(
                &mut capture,
                &mut planning,
                GameEvent::MouseUp(0, 0, 1),
                false
            ),
            TouchPlanRoute::Captured
        );
        assert!(!capture.touch_plan_captured());
        assert_eq!(
            route(
                &mut capture,
                &mut planning,
                GameEvent::MouseDown(0, 0, 1, 1),
                true
            ),
            TouchPlanRoute::Forward
        );
        planning.update_preference(true);
        assert_eq!(
            route(
                &mut capture,
                &mut planning,
                GameEvent::MouseDown(0, 0, 1, 1),
                false
            ),
            TouchPlanRoute::Forward
        );
    }

    #[test]
    fn two_taps_emit_ordered_toggle_and_cancellation_and_preserve_keys() {
        use TouchPlanRoute::*;
        let mut capture = FrontendPointerCapture::default();
        let mut planning = FrontendPlanning::new(true);
        let events = [
            GameEvent::MouseDown(0, 0, 1, 1),
            GameEvent::KeyDown {
                keycode: crate::gfx_types::Keycode::Space,
                physical_key: Some(winit::keyboard::KeyCode::Space),
            },
            GameEvent::MouseUp(0, 0, 3),
            GameEvent::MouseUp(0, 0, 1),
            GameEvent::MouseDown(0, 0, 1, 1),
            GameEvent::MouseUp(0, 0, 1),
        ];
        let decisions: Vec<_> = events
            .iter()
            .map(|event| capture.route_touch_plan_event(&mut planning, event, true, |_, _| true))
            .collect();
        assert_eq!(
            decisions,
            [
                Toggled {
                    cancel_planned: false
                },
                Forward,
                Forward,
                Captured,
                Toggled {
                    cancel_planned: true
                },
                Captured
            ]
        );
        assert!(!planning.touch_latched());
        assert!(!capture.touch_plan_captured());

        capture.route_touch_plan_event(&mut planning, &events[0], true, |_, _| true);
        assert_eq!(
            capture.route_touch_plan_event(
                &mut planning,
                &GameEvent::PointerCancel,
                true,
                |_, _| true
            ),
            Forward
        );
        assert!(!capture.touch_plan_captured());
        assert_eq!(
            capture.route_touch_plan_event(&mut planning, &events[3], true, |_, _| true),
            Forward
        );

        capture.route_touch_plan_event(&mut planning, &events[0], true, |_, _| true);
        capture.cancel_sequence();
        planning.cancel_touch();
        assert_eq!(
            capture.route_touch_plan_event(&mut planning, &events[3], true, |_, _| true),
            Forward
        );
        assert!(!planning.touch_latched());
    }
}
