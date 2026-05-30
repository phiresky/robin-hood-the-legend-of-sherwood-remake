//! Slider widget.
//!
//! A pseudo-slider built on radio-button sub-widgets — one per tick —
//! plus a sound hookup whose dispatch lives caller-side in
//! [`crate::ingame_menu::widget_bridge`].
//!
//! Three-state machine (DEFAULT → FOCUSED → SELECTED):
//! - DEFAULT: cursor outside.  Hover-in with no button held transitions
//!   to FOCUSED and emits `WidgetFocused`.
//! - FOCUSED: cursor inside, no drag.  Clicking the left button
//!   transitions to SELECTED *silently* (no event on this edge).
//!   Leaving the bbox returns to DEFAULT with `WidgetUnfocused`.
//! - SELECTED: drag is captured.  Mouse position is clamped to the
//!   bbox so the sub-buttons keep reacting even when the user drags
//!   outside.  On tracking change emits `WidgetSliderTrack`; on
//!   `LEFT_CLICK` (release) emits `WidgetActivated` and drops to
//!   FOCUSED/DEFAULT depending on whether the release was inside.

use robin_engine::coordinates::ScreenBBox;
#[cfg(test)]
use robin_engine::coordinates::ScreenPoint;
use serde::{Deserialize, Serialize};

use crate::ui::{
    MouseButtons, UiEvent, UiEventData, UiMsg, UiState,
    resource_widget_id::{BUTTON_DEFAULT, BUTTON_DISABLED, BUTTON_FOCUSED, BUTTON_SELECTED},
};

use super::{WidgetBase, WidgetInput, WidgetRadioButton};

/// Composite pseudo-slider widget.
///
/// Owns an array of `WidgetRadioButton` sub-widgets — one per tick — so
/// per-tick hit-testing and per-tick sprite rendering work uniformly.
/// `tracking` is the currently-selected tick index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetSlider {
    pub base: WidgetBase,
    /// Minimum value (logical).
    pub min: f32,
    /// Maximum value (logical).
    pub max: f32,
    /// Current tracking index, in `0..=max(0, step_count() - 1)`.
    pub tracking: u32,
    /// Whether the drag is active (capture flag).
    pub dragging: bool,
    /// Sub-buttons — one per tick.  Empty when `step_count == 0`
    /// (continuous mode, no visible sub-widgets).
    pub buttons: Vec<WidgetRadioButton>,
    /// Last tracking index at which `WidgetSliderTrack` was emitted.
    /// Compared against the fresh `tracking` after each drag update;
    /// the event only fires when they differ.
    #[serde(default = "default_last_tracked")]
    last_tracked: Option<u32>,
}

fn default_last_tracked() -> Option<u32> {
    None
}

impl Default for WidgetSlider {
    fn default() -> Self {
        Self {
            base: WidgetBase::default(),
            min: 0.0,
            max: 1.0,
            tracking: 0,
            dragging: false,
            buttons: Vec::new(),
            last_tracked: None,
        }
    }
}

impl WidgetSlider {
    pub fn new(id: super::WidgetId) -> Self {
        Self {
            base: WidgetBase {
                id,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Set the slider range.
    pub fn set_range(&mut self, min: f32, max: f32) {
        self.min = min;
        self.max = max;
    }

    /// Configure discrete tick snapping.  The slider allocates
    /// `step_count` radio-button children laid out evenly across the
    /// bbox.  Pass `0` to stay continuous (no sub-buttons).
    ///
    /// Call after [`WidgetBase::bbox`] is set — sub-buttons are sized
    /// relative to the current bbox.
    pub fn set_step_count(&mut self, step_count: u32) {
        self.buttons = if step_count >= 2 {
            self.rebuild_buttons(step_count)
        } else {
            Vec::new()
        };
        self.tracking = self.tracking.min(step_count.saturating_sub(1));
        self.sync_button_selection();
    }

    fn rebuild_buttons(&self, step_count: u32) -> Vec<WidgetRadioButton> {
        let Some(rect) = self.base.bbox.0 else {
            // No bbox yet — fall back to zero-sized children; the
            // caller is expected to call `set_position` before input.
            return (0..step_count).map(WidgetRadioButton::new).collect();
        };
        let left = rect.min().x;
        let top = rect.min().y;
        let bottom = rect.max().y;
        let total_w = rect.max().x - left;
        let count_f = step_count as f32;
        (0..step_count)
            .map(|i| {
                let x0 = left + (i as f32 / count_f) * total_w;
                let x1 = left + ((i + 1) as f32 / count_f) * total_w;
                let mut btn = WidgetRadioButton::new(i);
                btn.base.bbox = ScreenBBox::from_coords(x0, top, x1, bottom);
                btn
            })
            .collect()
    }

    /// Number of discrete ticks.
    pub fn step_count(&self) -> u32 {
        self.buttons.len() as u32
    }

    /// Set the slider value, clamped to [min, max] and snapped to the
    /// nearest tick (if step_count >= 2).
    pub fn set_value(&mut self, value: f32) {
        let clamped = value.clamp(self.min, self.max);
        self.tracking = if self.step_count() < 2 || self.max <= self.min {
            0
        } else {
            let span = self.max - self.min;
            let t = ((clamped - self.min) / span).clamp(0.0, 1.0);
            (t * (self.step_count() - 1) as f32).round() as u32
        };
        self.sync_button_selection();
    }

    /// Current tick index.
    pub fn tick_index(&self) -> u32 {
        self.tracking
    }

    /// Current snapped logical value derived from [`tick_index`].
    pub fn value(&self) -> f32 {
        if self.step_count() < 2 || self.max <= self.min {
            return self.min;
        }
        let span = self.max - self.min;
        self.min + (self.tracking as f32 / (self.step_count() - 1) as f32) * span
    }

    /// Map state to renderer sub-resource ID.
    pub fn transform_state_into_id(&self) -> u8 {
        if !self.base.enabled {
            return BUTTON_DISABLED;
        }
        match self.base.state {
            UiState::Selected | UiState::Pushed => BUTTON_SELECTED,
            UiState::Focused => BUTTON_FOCUSED,
            _ => BUTTON_DEFAULT,
        }
    }

    /// Sync each sub-button's `second_state` to the `tracking` index
    /// — the active tick is the selected one, rest deselect.
    fn sync_button_selection(&mut self) {
        for (i, btn) in self.buttons.iter_mut().enumerate() {
            btn.set_selected(i as u32 == self.tracking);
        }
    }

    /// Update the tracking index from the raw mouse position.
    fn update_tracking(&mut self, mouse_x: f32) {
        let Some(rect) = self.base.bbox.0 else { return };
        let count = self.step_count();
        if count == 0 {
            return;
        }
        let width = rect.max().x - rect.min().x;
        if width <= 0.0 {
            return;
        }
        // Integer-truncate `(mouse_x - bbox.left) / radio_width` so
        // tick boundaries land at the visual button edges.
        let cell_w = width / count as f32;
        let raw = ((mouse_x - rect.min().x) / cell_w).floor();
        let idx = raw.clamp(0.0, (count - 1) as f32) as u32;
        self.tracking = idx;
        self.sync_button_selection();
    }

    /// Clamp the mouse X to the bbox so sub-buttons keep reacting even
    /// when the user drags outside.
    fn clamp_mouse_x(&self, mouse_x: f32) -> f32 {
        match self.base.bbox.0 {
            Some(rect) => mouse_x.clamp(rect.min().x, rect.max().x),
            None => mouse_x,
        }
    }

    /// Process input for one frame.  Implements the three-state
    /// DEFAULT / FOCUSED / SELECTED machine documented at the top of
    /// this module; maps to the Rust `UiState` values DEFAULT / FOCUSED
    /// / PUSHED respectively.
    pub fn process_input(&mut self, input: &WidgetInput) -> Vec<UiEvent> {
        if !self.base.enabled {
            return self.base.tooltip_event_if_disabled().into_iter().collect();
        }

        let inside = self.base.is_inside(input.mouse_position);
        let buttons = input.mouse_button;

        match self.base.state {
            UiState::Default => self.process_default(input, inside, buttons),
            UiState::Focused => self.process_focused(input, inside, buttons),
            UiState::Pushed => self.process_selected(input, inside, buttons),
            _ => Vec::new(),
        }
    }

    /// DEFAULT state: hover-in transitions to FOCUSED and emits
    /// `WidgetFocused`.  Tracking is primed here too.
    fn process_default(
        &mut self,
        input: &WidgetInput,
        inside: bool,
        buttons: MouseButtons,
    ) -> Vec<UiEvent> {
        if inside && !buttons.contains(MouseButtons::LEFT_DOWN) && self.base.with_focus {
            self.base.state = UiState::Focused;
            self.update_tracking(input.mouse_position.x);
            return vec![self.base.make_event(UiMsg::WidgetFocused)];
        }
        Vec::new()
    }

    /// FOCUSED state: mouse down transitions to SELECTED silently (no
    /// event on this edge); mouse-out returns to DEFAULT.  Tracking is
    /// still resampled here so the visible thumb moves as the cursor
    /// hovers across the widget — if it crosses a tick, emit
    /// `WidgetSliderTrack`.
    fn process_focused(
        &mut self,
        input: &WidgetInput,
        inside: bool,
        buttons: MouseButtons,
    ) -> Vec<UiEvent> {
        let mut events = Vec::new();
        if inside {
            if buttons.contains(MouseButtons::LEFT_DOWN) {
                self.base.state = UiState::Pushed;
                self.dragging = true;
            }
            let old = self.tracking;
            self.update_tracking(input.mouse_position.x);
            if self.tracking != old {
                events.extend(self.track_event_if_changed());
            } else if self.last_tracked.is_none() {
                // Prime the last-tracked marker on entering FOCUSED
                // so the next tick move is the first one to emit.
                self.last_tracked = Some(self.tracking);
            }
        } else {
            self.base.state = UiState::Default;
            events.push(self.base.make_event(UiMsg::WidgetUnfocused));
        }
        events
    }

    /// SELECTED state: drag is captured.  Mouse is clamped to the
    /// bbox; tracking is resampled; `WidgetSliderTrack` fires on tick
    /// change.  On `LEFT_CLICK` (release) transition back to FOCUSED
    /// (if still inside) or DEFAULT with `WidgetActivated`.
    fn process_selected(
        &mut self,
        input: &WidgetInput,
        inside: bool,
        buttons: MouseButtons,
    ) -> Vec<UiEvent> {
        let mut events = Vec::new();

        // LEFT_CLICK is the one-shot "just released" bit — LEFT_DOWN
        // going away by itself is not the release edge.
        let released = buttons.contains(MouseButtons::LEFT_CLICK);

        if released {
            self.dragging = false;
            self.base.state = if inside {
                UiState::Focused
            } else {
                UiState::Default
            };
            events.push(self.base.make_event(UiMsg::WidgetActivated));
        }

        // Always resample tracking with the mouse clamped to the
        // bbox so outside-drag keeps the end-stop button selected.
        let clamped_x = self.clamp_mouse_x(input.mouse_position.x);
        let old = self.tracking;
        self.update_tracking(clamped_x);
        if self.tracking != old {
            events.extend(self.track_event_if_changed());
        }

        events
    }

    /// Emit `WidgetSliderTrack` when `tracking` differs from the last
    /// value an emission was produced for.  Updates the cached value
    /// so a subsequent no-op call stays silent.
    fn track_event_if_changed(&mut self) -> Vec<UiEvent> {
        if self.last_tracked == Some(self.tracking) {
            return Vec::new();
        }
        self.last_tracked = Some(self.tracking);
        vec![self.base.make_event_with_data(
            UiMsg::WidgetSliderTrack,
            UiEventData::SliderPosition(self.value()),
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{RendererBase, RendererBitmap, UiKeyboard};
    use crate::widget::WidgetRenderer;

    fn make_slider(step_count: u32) -> WidgetSlider {
        let mut slider = WidgetSlider::new(7);
        let bbox = ScreenBBox::from_coords(0.0, 0.0, 100.0, 10.0);
        slider.base.bbox = bbox;
        slider.base.renderer = WidgetRenderer::Bitmap(RendererBitmap {
            base: RendererBase {
                bbox,
                ..Default::default()
            },
        });
        slider.set_range(0.0, 9.0);
        slider.set_step_count(step_count);
        slider
    }

    fn input_at(x: f32, buttons: MouseButtons, keyboard: &UiKeyboard) -> WidgetInput<'_> {
        WidgetInput {
            mouse_position: ScreenPoint::new(x, 5.0),
            mouse_z: 0,
            mouse_button: buttons,
            keyboard,
            text_input: "",
            capture: None,
        }
    }

    #[test]
    fn step_count_builds_subbuttons() {
        let slider = make_slider(10);
        assert_eq!(slider.step_count(), 10);
        assert_eq!(slider.buttons.len(), 10);
        // First sub-button covers x=[0,10); last covers x=[90,100).
        let first_bbox = slider.buttons[0].base.bbox.0.unwrap();
        assert_eq!(first_bbox.min().x, 0.0);
        assert_eq!(first_bbox.max().x, 10.0);
        let last_bbox = slider.buttons[9].base.bbox.0.unwrap();
        assert_eq!(last_bbox.min().x, 90.0);
        assert_eq!(last_bbox.max().x, 100.0);
    }

    #[test]
    fn set_value_snaps_tracking() {
        let mut slider = make_slider(10);
        slider.set_value(3.1);
        assert_eq!(slider.tracking, 3);
        assert_eq!(slider.value(), 3.0);
        slider.set_value(8.9);
        assert_eq!(slider.tracking, 9);
        assert_eq!(slider.value(), 9.0);
    }

    #[test]
    fn hover_emits_focused_silent_on_click_down() {
        let kb = UiKeyboard::default();
        let mut slider = make_slider(10);
        // Hover inside, no button → DEFAULT→FOCUSED, WidgetFocused.
        let events = slider.process_input(&input_at(30.0, MouseButtons::empty(), &kb));
        assert_eq!(slider.base.state, UiState::Focused);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].msg_type, UiMsg::WidgetFocused);
        // Click down → FOCUSED→PUSHED silently (no event on this edge).
        let events = slider.process_input(&input_at(30.0, MouseButtons::LEFT_DOWN, &kb));
        assert_eq!(slider.base.state, UiState::Pushed);
        // Tracking already 3 from the hover, so no track event either.
        assert!(events.is_empty(), "click-down edge should be silent");
    }

    #[test]
    fn track_fires_on_tick_change_silent_within_cell() {
        let kb = UiKeyboard::default();
        let mut slider = make_slider(10);
        // Hover to FOCUSED at x=5 (tick 0).
        slider.process_input(&input_at(5.0, MouseButtons::empty(), &kb));
        // Start drag at x=5 (still tick 0) — silent.
        let events = slider.process_input(&input_at(5.0, MouseButtons::LEFT_DOWN, &kb));
        assert!(events.is_empty());
        // Drag across to tick 2 — fires once.
        let events = slider.process_input(&input_at(25.0, MouseButtons::LEFT_DOWN, &kb));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].msg_type, UiMsg::WidgetSliderTrack);
        assert_eq!(slider.tracking, 2);
        // Jiggle inside tick 2 — silent.
        let events = slider.process_input(&input_at(27.0, MouseButtons::LEFT_DOWN, &kb));
        assert!(events.is_empty());
    }

    #[test]
    fn drag_outside_bbox_clamps_to_end() {
        let kb = UiKeyboard::default();
        let mut slider = make_slider(10);
        slider.process_input(&input_at(50.0, MouseButtons::empty(), &kb));
        slider.process_input(&input_at(50.0, MouseButtons::LEFT_DOWN, &kb));
        // Drag to x=500 (way outside) — tracking clamps to last tick.
        let events = slider.process_input(&input_at(500.0, MouseButtons::LEFT_DOWN, &kb));
        assert_eq!(slider.tracking, 9);
        assert!(
            events
                .iter()
                .any(|e| e.msg_type == UiMsg::WidgetSliderTrack)
        );
    }

    #[test]
    fn release_fires_activated_only_on_left_click() {
        let kb = UiKeyboard::default();
        let mut slider = make_slider(10);
        slider.process_input(&input_at(50.0, MouseButtons::empty(), &kb));
        slider.process_input(&input_at(50.0, MouseButtons::LEFT_DOWN, &kb));
        assert!(slider.dragging);
        // LEFT_DOWN going away alone — not the release edge (no LEFT_CLICK
        // one-shot), so no WidgetActivated should fire.
        let events = slider.process_input(&input_at(50.0, MouseButtons::empty(), &kb));
        assert!(
            !events.iter().any(|e| e.msg_type == UiMsg::WidgetActivated),
            "release only emits on LEFT_CLICK, not on LEFT_DOWN going absent"
        );
        // Now the real release — LEFT_CLICK flag on top of not-held.
        let events = slider.process_input(&input_at(50.0, MouseButtons::LEFT_CLICK, &kb));
        assert!(!slider.dragging);
        assert!(events.iter().any(|e| e.msg_type == UiMsg::WidgetActivated));
    }

    #[test]
    fn sub_buttons_track_selected_tick() {
        let mut slider = make_slider(10);
        slider.set_value(4.0);
        assert!(slider.buttons[4].is_pushed());
        for (i, b) in slider.buttons.iter().enumerate() {
            if i != 4 {
                assert!(!b.is_pushed(), "tick {i} should not be selected");
            }
        }
    }
}
