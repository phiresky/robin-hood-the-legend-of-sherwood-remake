//! Host-only input lifecycle. Profile policy, sticky planning and paired pointer
//! events are not simulation state and never enter saves or replay commands.
use crate::gfx_types::GameEvent;
use serde::{Deserialize, Serialize};

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
}

impl FrontendPointerCapture {
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
    /// Drain the captured pointer sequence without consuming keyboard, window,
    /// or other-button events. A press and release may share one input batch.
    pub fn filter_touch_plan_events(&mut self, events: &mut Vec<GameEvent>) {
        if !self.touch_plan_captured {
            return;
        }
        let released = events
            .iter()
            .any(|event| matches!(event, GameEvent::MouseUp(_, _, 1)));
        events.retain(|event| {
            !matches!(
                event,
                GameEvent::MouseDown(_, _, 1, _)
                    | GameEvent::MouseUp(_, _, 1)
                    | GameEvent::MouseMove { .. }
            )
        });
        if released {
            self.touch_plan_captured = false;
        }
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
        capture.capture_touch_plan();
        let mut events = vec![
            GameEvent::MouseDown(10, 20, 1, 1),
            GameEvent::MouseMove {
                x: 11,
                y: 20,
                xrel: 1,
                yrel: 0,
            },
            GameEvent::MouseUp(11, 20, 1),
            GameEvent::MouseDown(11, 20, 3, 1),
            GameEvent::Quit,
        ];
        capture.filter_touch_plan_events(&mut events);
        assert!(!capture.touch_plan_captured());
        assert!(matches!(
            events.as_slice(),
            [GameEvent::MouseDown(_, _, 3, _), GameEvent::Quit]
        ));
        events.push(GameEvent::MouseUp(11, 20, 1));
        capture.filter_touch_plan_events(&mut events);
        assert_eq!(events.len(), 3, "the next sequence is not captured");
    }

    #[test]
    fn capture_survives_batches_until_matching_release() {
        let mut capture = FrontendPointerCapture::default();
        capture.capture_touch_plan();
        let mut events = vec![GameEvent::MouseUp(0, 0, 3)];
        capture.filter_touch_plan_events(&mut events);
        assert!(capture.touch_plan_captured());
        assert_eq!(events.len(), 1);
        events.push(GameEvent::MouseUp(0, 0, 1));
        capture.filter_touch_plan_events(&mut events);
        assert!(!capture.touch_plan_captured());
        assert_eq!(events.len(), 1);
    }
}
