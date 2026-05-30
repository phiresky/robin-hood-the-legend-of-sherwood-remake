//! Push button widget with state machine input handling.
//!
//! The base button and the menu-screen variant share one Rust struct;
//! `is_menu_button` switches between the two state machines.
//!
//! Base state machine (`is_menu_button == false`):
//! ```text
//! DEFAULT ──mouse over──► FOCUSED ──left down──► PUSHED ──left click──► SELECTED
//!    ▲                       │                      │
//!    └──mouse out────────────┘                      │
//!    ▲                                              │
//!    └──────────mouse out + release─────────────────┘
//! ```
//!
//! Menu-button state machine (`is_menu_button == true`):
//! ```text
//! DEFAULT ──hover──► FOCUSED ──LEFT_DOWN──► PUSHED ──LEFT_CLICK──► FOCUSED
//!    ▲                  │                      │ (drag off)
//!    │                  │                      ▼
//!    │                  │                   SELECTED ──drag back──► PUSHED
//!    └──────release outside (LEFT_CLICK)─────┘
//! ```

#[cfg(test)]
use robin_engine::coordinates::ScreenBBox;
use robin_engine::coordinates::ScreenPoint;
use serde::{Deserialize, Serialize};

use crate::focus_manager::WidgetGroupable;
use crate::ui::{
    KeyState, MouseButtons, UiEvent, UiMsg, UiState,
    resource_widget_id::{
        BUTTON_DEFAULT, BUTTON_DISABLED, BUTTON_FOCUSED, BUTTON_SELECTED, NO_RESOURCE,
    },
};

use super::{WidgetBase, WidgetInput};

/// Push button widget.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WidgetButton {
    pub base: WidgetBase,
    /// When set, use the menu-button state machine and renderer
    /// mapping (group focus, drag-off-cancel semantics, hide-focus
    /// branch). Otherwise the base button semantics apply.
    pub is_menu_button: bool,
    /// Group focus/selected state for menu-button group navigation.
    pub group_state: UiState,
    /// Suppress the FOCUSED sprite while a sub-menu drilldown takes
    /// over.
    pub hide_focus: bool,
}

impl WidgetButton {
    pub fn new(id: super::WidgetId) -> Self {
        Self {
            base: WidgetBase {
                id,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Map the current widget state to a renderer sub-resource ID.
    pub fn transform_state_into_id(&self) -> u8 {
        if !self.base.enabled {
            return BUTTON_DISABLED;
        }
        if self.is_menu_button {
            // Pick the higher of state / group_state (enum order
            // DEFAULT < FOCUSED < SELECTED < PUSHED).
            let state = max_state(self.base.state, self.group_state);
            return match state {
                UiState::Pushed => BUTTON_SELECTED,
                UiState::Selected => BUTTON_FOCUSED,
                UiState::Focused => {
                    if self.hide_focus {
                        BUTTON_DEFAULT
                    } else {
                        BUTTON_FOCUSED
                    }
                }
                _ => BUTTON_DEFAULT,
            };
        }
        match self.base.state {
            UiState::Selected | UiState::Pushed => BUTTON_SELECTED,
            UiState::Focused => BUTTON_FOCUSED,
            UiState::Default => {
                if self.base.with_default {
                    BUTTON_DEFAULT
                } else {
                    NO_RESOURCE
                }
            }
            _ => BUTTON_DEFAULT,
        }
    }

    /// Set enable state with side effects.
    ///
    /// - Enabling resets state to Default
    /// - Disabling generates an Unselect event
    pub fn set_enable(&mut self, enabled: bool) -> Option<UiEvent> {
        let was_enabled = self.base.enabled;
        self.base.enabled = enabled;

        if enabled {
            self.base.state = UiState::Default;
            None
        } else if was_enabled {
            // Was enabled, now disabled → emit unselect
            self.base.state = UiState::Default;
            Some(self.base.make_event(UiMsg::WidgetUnselect))
        } else {
            None
        }
    }

    /// Group-focus flag setter. Writes `group_state`, then on enabled
    /// widgets emits `WidgetFocused` when gaining focus.
    pub fn set_group_focused(&mut self, focused: bool) -> Vec<UiEvent> {
        self.group_state = if focused {
            UiState::Focused
        } else {
            UiState::Default
        };
        if self.base.enabled && focused {
            vec![self.base.make_event(UiMsg::WidgetFocused)]
        } else {
            Vec::new()
        }
    }

    /// Group-selected flag setter. Writes `group_state`
    /// unconditionally — unlike the toggle-button variant there is no
    /// enabled-state gate.
    pub fn set_group_selected(&mut self, selected: bool) -> Vec<UiEvent> {
        self.group_state = if selected {
            UiState::Pushed
        } else {
            UiState::Default
        };
        Vec::new()
    }

    /// Focus-visibility toggle.
    pub fn hide_focus(&mut self, hide: bool) {
        self.hide_focus = hide;
    }

    /// Programmatic activation (focus-manager Enter press): emit
    /// `WidgetActivated` on enabled widgets.
    pub fn activate(&mut self) -> Vec<UiEvent> {
        if self.base.enabled {
            vec![self.base.make_event(UiMsg::WidgetActivated)]
        } else {
            Vec::new()
        }
    }

    /// Whether the widget is "sleeping" (DEFAULT state).
    pub fn is_sleeping(&self) -> bool {
        self.base.state == UiState::Default
    }

    /// Process input for one frame.
    ///
    /// Dispatches to either the base button state machine or the
    /// menu-button variant depending on `is_menu_button`.
    pub fn process_input(&mut self, input: &WidgetInput) -> Vec<UiEvent> {
        // Disabled: only return tooltip event if applicable.
        if !self.base.enabled {
            return self.base.tooltip_event_if_disabled().into_iter().collect();
        }

        if self.is_menu_button {
            return self.process_input_menu_button(input);
        }

        let inside = self.base.is_inside(input.mouse_position);
        let buttons = input.mouse_button;

        // Check accelerator key.
        if let Some(fast_key) = self.base.fast_key {
            let key_state = input.keyboard.get_state_of_key(fast_key);
            match key_state {
                KeyState::KeyDown => {
                    return self.process_pushed();
                }
                KeyState::KeyPressed | KeyState::KeyDouble => {
                    return self.process_select(key_state == KeyState::KeyDouble);
                }
                _ => {}
            }
        }

        // Mouse-based state machine.
        match self.base.state {
            UiState::Default => {
                if inside {
                    if buttons.contains(MouseButtons::LEFT_DOWN) {
                        return self.process_pushed();
                    }
                    if buttons.contains(MouseButtons::LEFT_CLICK) {
                        return self.process_select(false);
                    }
                    if buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK) {
                        return self.process_select(true);
                    }
                    if buttons.contains(MouseButtons::RIGHT_CLICK) {
                        return self.process_unselect();
                    }
                    if self.base.with_focus {
                        return self.process_focus();
                    }
                }
                // Already default, nothing to do.
                Vec::new()
            }

            UiState::Focused => {
                if inside {
                    if buttons.contains(MouseButtons::LEFT_DOWN) {
                        return self.process_pushed();
                    }
                    if buttons.contains(MouseButtons::LEFT_CLICK) {
                        return self.process_select(false);
                    }
                    if buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK) {
                        return self.process_select(true);
                    }
                    if buttons.contains(MouseButtons::RIGHT_CLICK) {
                        return self.process_unselect();
                    }
                    // Stay focused.
                    Vec::new()
                } else {
                    self.process_default()
                }
            }

            UiState::Pushed => {
                if inside {
                    if buttons.contains(MouseButtons::LEFT_CLICK) {
                        return self.process_select(false);
                    }
                    if buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK) {
                        return self.process_select(true);
                    }
                    if !buttons.contains(MouseButtons::LEFT_DOWN) {
                        // Mouse released inside without a click event — stay focused.
                        return self.process_focus();
                    }
                    // Still held down inside.
                    Vec::new()
                } else {
                    // Mouse moved outside while held — go to default.
                    self.process_default()
                }
            }

            UiState::Selected => {
                if inside {
                    if buttons.contains(MouseButtons::LEFT_CLICK) {
                        return self.process_select(false);
                    }
                    if buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK) {
                        return self.process_select(true);
                    }
                    if buttons.contains(MouseButtons::RIGHT_CLICK) {
                        return self.process_unselect();
                    }
                    // Stay selected.
                    Vec::new()
                } else {
                    self.process_default()
                }
            }

            _ => Vec::new(),
        }
    }

    /// Menu-button state machine. Differs from the base machine in
    /// three load-bearing ways:
    ///   1. clicks only register from `PUSHED`, not `DEFAULT`/`FOCUSED`,
    ///   2. the press-and-drag-off cycle uses `SELECTED` as a pending-
    ///      cancel state that can re-arm by dragging back in,
    ///   3. focus emits `WidgetFocused` every frame while hovered, not
    ///      once on entry — drives per-frame menu hover sounds.
    fn process_input_menu_button(&mut self, input: &WidgetInput) -> Vec<UiEvent> {
        let inside = self.base.is_inside(input.mouse_position);
        let buttons = input.mouse_button;
        let capture = input.capture;

        match self.base.state {
            UiState::Default => self.process_input_menu_default(inside, buttons),
            UiState::Focused => self.process_input_menu_focused(inside, buttons, capture),
            UiState::Pushed => self.process_input_menu_pushed(inside, buttons, capture),
            UiState::Selected => self.process_input_menu_selected(inside, buttons, capture),
            _ => Vec::new(),
        }
    }

    /// Menu-button DEFAULT-state input handler.
    fn process_input_menu_default(&mut self, inside: bool, buttons: MouseButtons) -> Vec<UiEvent> {
        // Inside + LEFT_DOWN-not-set → silently transition to FOCUSED.
        if inside && !buttons.contains(MouseButtons::LEFT_DOWN) {
            self.base.state = UiState::Focused;
        }
        Vec::new()
    }

    /// Menu-button FOCUSED-state input handler.
    fn process_input_menu_focused(
        &mut self,
        inside: bool,
        buttons: MouseButtons,
        capture: Option<&super::CaptureSlot>,
    ) -> Vec<UiEvent> {
        if inside {
            if buttons.contains(MouseButtons::LEFT_DOWN) {
                // Focused → Pushed, set capture, no event.
                self.base.state = UiState::Pushed;
                if let Some(slot) = capture {
                    slot.set(self.base.id);
                }
                Vec::new()
            } else {
                // Re-emit WidgetFocused every frame while hovered —
                // drives per-frame menu hover sounds (the focus event
                // maps to a continuously-firing noisy event).
                vec![self.base.make_event(UiMsg::WidgetFocused)]
            }
        } else {
            // Mouse out → DEFAULT silently (no event).
            self.base.state = UiState::Default;
            Vec::new()
        }
    }

    /// Menu-button PUSHED-state input handler.
    fn process_input_menu_pushed(
        &mut self,
        inside: bool,
        buttons: MouseButtons,
        capture: Option<&super::CaptureSlot>,
    ) -> Vec<UiEvent> {
        if inside {
            if buttons.contains(MouseButtons::LEFT_CLICK)
                || buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK)
            {
                // Click: release capture, go to FOCUSED (not SELECTED),
                // emit WidgetActivated.
                if let Some(slot) = capture {
                    slot.clear();
                }
                self.base.state = UiState::Focused;
                return vec![self.base.make_event(UiMsg::WidgetActivated)];
            }
            // Still held inside: stay PUSHED, no event.
            Vec::new()
        } else {
            // Drag-off while held: go to SELECTED (pending-cancel).
            self.base.state = UiState::Selected;
            Vec::new()
        }
    }

    /// Menu-button SELECTED-state input handler.
    fn process_input_menu_selected(
        &mut self,
        inside: bool,
        buttons: MouseButtons,
        capture: Option<&super::CaptureSlot>,
    ) -> Vec<UiEvent> {
        if inside {
            // Drag back in: re-arm to PUSHED silently.
            self.base.state = UiState::Pushed;
        } else if buttons.contains(MouseButtons::LEFT_CLICK) {
            // Release outside: cancel, release capture, → DEFAULT.
            if let Some(slot) = capture {
                slot.clear();
            }
            self.base.state = UiState::Default;
        }
        Vec::new()
    }

    // ── State transition helpers (base button) ────────────────────────

    fn process_default(&mut self) -> Vec<UiEvent> {
        self.base.state = UiState::Default;
        vec![self.base.make_event(UiMsg::WidgetUnfocused)]
    }

    fn process_focus(&mut self) -> Vec<UiEvent> {
        self.base.state = UiState::Focused;
        vec![self.base.make_event(UiMsg::WidgetFocused)]
    }

    fn process_pushed(&mut self) -> Vec<UiEvent> {
        self.base.state = UiState::Pushed;
        vec![self.base.make_event(UiMsg::WidgetFocused)]
    }

    fn process_select(&mut self, double_click: bool) -> Vec<UiEvent> {
        self.base.state = UiState::Selected;
        let msg = if double_click {
            UiMsg::WidgetDoubleClicked
        } else {
            UiMsg::WidgetActivated
        };
        vec![self.base.make_event(msg)]
    }

    fn process_unselect(&mut self) -> Vec<UiEvent> {
        self.base.state = UiState::Default;
        vec![self.base.make_event(UiMsg::WidgetUnselect)]
    }

    /// Hit-test for the focus manager. Combines the bbox test with the
    /// renderer's per-pixel transparency check.
    pub fn is_mouse_inside(&self, point: ScreenPoint) -> bool {
        self.base.is_inside(point)
    }
}

/// Pick the higher of two `UiState` values (enum order
/// `DEFAULT < FOCUSED < SELECTED < PUSHED`). The `repr(u8)`
/// discriminants match this ordering for the four base values, so a
/// numeric compare suffices.
fn max_state(a: UiState, b: UiState) -> UiState {
    if (a as u8) >= (b as u8) { a } else { b }
}

// ── WidgetGroupable trait impl ─────────────────────────────────────
//
// Glue for `FocusManager` so a menu button can be dropped into the group
// navigation chain via `add_groupable`. Parallels `WidgetToggleButton`'s
// impl (see `widget/toggle.rs`).

fn ui_event_to_focus_event(event: crate::ui::UiEvent) -> Option<crate::focus_manager::UiEvent> {
    use crate::focus_manager::{UiEvent as FmEvent, UiEventType as FmType};
    let msg_type = match event.msg_type {
        UiMsg::WidgetFocused | UiMsg::WidgetUnfocused => FmType::FocusChanged,
        UiMsg::WidgetActivated => FmType::Activated,
        UiMsg::WidgetReactivated => FmType::SelectionChanged,
        _ => return None,
    };
    Some(FmEvent {
        msg_type,
        origin: event.origin_widget_id as crate::focus_manager::WidgetId,
    })
}

fn translate(events: Vec<UiEvent>) -> Vec<crate::focus_manager::UiEvent> {
    events
        .into_iter()
        .filter_map(ui_event_to_focus_event)
        .collect()
}

impl WidgetGroupable for WidgetButton {
    fn widget_id(&self) -> crate::focus_manager::WidgetId {
        self.base.id as crate::focus_manager::WidgetId
    }

    fn is_enabled(&self) -> bool {
        self.base.enabled
    }

    fn is_sleeping(&self) -> bool {
        WidgetButton::is_sleeping(self)
    }

    fn is_mouse_inside(&self, point: ScreenPoint) -> bool {
        WidgetButton::is_mouse_inside(self, point)
    }

    fn hide_focus(&mut self, hide: bool) {
        WidgetButton::hide_focus(self, hide);
    }

    fn set_group_focused(&mut self, focused: bool) -> Vec<crate::focus_manager::UiEvent> {
        translate(WidgetButton::set_group_focused(self, focused))
    }

    fn set_group_selected(&mut self, selected: bool) -> Vec<crate::focus_manager::UiEvent> {
        translate(WidgetButton::set_group_selected(self, selected))
    }

    fn activate(&mut self) -> Vec<crate::focus_manager::UiEvent> {
        translate(WidgetButton::activate(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::UiKeyboard;

    fn make_input(mouse_x: f32, mouse_y: f32, buttons: MouseButtons) -> WidgetInput<'static> {
        // Leak a keyboard for test convenience (tests are short-lived).
        let kb = Box::leak(Box::new(UiKeyboard::default()));
        WidgetInput {
            mouse_position: ScreenPoint::new(mouse_x, mouse_y),
            mouse_z: 0,
            mouse_button: buttons,
            keyboard: kb,
            text_input: "",
            capture: None,
        }
    }

    fn make_input_with_capture<'a>(
        mouse_x: f32,
        mouse_y: f32,
        buttons: MouseButtons,
        capture: &'a super::super::CaptureSlot,
    ) -> WidgetInput<'a> {
        let kb = Box::leak(Box::new(UiKeyboard::default()));
        WidgetInput {
            mouse_position: ScreenPoint::new(mouse_x, mouse_y),
            mouse_z: 0,
            mouse_button: buttons,
            keyboard: kb,
            text_input: "",
            capture: Some(capture),
        }
    }

    fn make_button() -> WidgetButton {
        let mut btn = WidgetButton::new(1);
        btn.base
            .create("Test", ScreenBBox::from_coords(0.0, 0.0, 100.0, 30.0), 0);
        // Use a bitmap renderer with a matching bbox for hit testing.
        btn.base.renderer = super::super::WidgetRenderer::Bitmap(crate::ui::RendererBitmap {
            base: crate::ui::RendererBase {
                bbox: ScreenBBox::from_coords(0.0, 0.0, 100.0, 30.0),
                ..Default::default()
            },
        });
        btn
    }

    fn make_menu_button() -> WidgetButton {
        let mut btn = make_button();
        btn.is_menu_button = true;
        btn
    }

    #[test]
    fn default_to_focused_on_hover() {
        let mut btn = make_button();
        let input = make_input(50.0, 15.0, MouseButtons::empty());
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Focused);
        assert!(events.iter().any(|e| e.msg_type == UiMsg::WidgetFocused));
    }

    #[test]
    fn focused_to_pushed_on_left_down() {
        let mut btn = make_button();
        btn.base.state = UiState::Focused;
        let input = make_input(50.0, 15.0, MouseButtons::LEFT_DOWN);
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Pushed);
        assert!(events.iter().any(|e| e.msg_type == UiMsg::WidgetFocused));
    }

    #[test]
    fn pushed_to_selected_on_click() {
        let mut btn = make_button();
        btn.base.state = UiState::Pushed;
        let input = make_input(50.0, 15.0, MouseButtons::LEFT_CLICK);
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Selected);
        assert!(events.iter().any(|e| e.msg_type == UiMsg::WidgetActivated));
    }

    #[test]
    fn double_click_emits_double_clicked() {
        let mut btn = make_button();
        btn.base.state = UiState::Pushed;
        let input = make_input(50.0, 15.0, MouseButtons::LEFT_DOUBLE_CLICK);
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Selected);
        assert!(
            events
                .iter()
                .any(|e| e.msg_type == UiMsg::WidgetDoubleClicked)
        );
    }

    #[test]
    fn focused_to_default_on_mouse_out() {
        let mut btn = make_button();
        btn.base.state = UiState::Focused;
        let input = make_input(200.0, 200.0, MouseButtons::empty());
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Default);
        assert!(events.iter().any(|e| e.msg_type == UiMsg::WidgetUnfocused));
    }

    #[test]
    fn disabled_returns_no_events() {
        let mut btn = make_button();
        btn.base.enabled = false;
        let input = make_input(50.0, 15.0, MouseButtons::LEFT_CLICK);
        let events = btn.process_input(&input);
        assert!(events.is_empty());
    }

    #[test]
    fn disabled_with_tooltip_returns_tooltip_event() {
        let mut btn = make_button();
        btn.base.enabled = false;
        btn.base.set_tooltip_text("disabled hint");
        let input = make_input(50.0, 15.0, MouseButtons::LEFT_CLICK);
        let events = btn.process_input(&input);
        assert!(
            events
                .iter()
                .any(|e| e.msg_type == UiMsg::WidgetFocusedDisabled)
        );
    }

    #[test]
    fn right_click_unselects() {
        let mut btn = make_button();
        btn.base.state = UiState::Focused;
        let input = make_input(50.0, 15.0, MouseButtons::RIGHT_CLICK);
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Default);
        assert!(events.iter().any(|e| e.msg_type == UiMsg::WidgetUnselect));
    }

    #[test]
    fn transform_state_disabled() {
        let mut btn = make_button();
        btn.base.enabled = false;
        assert_eq!(btn.transform_state_into_id(), BUTTON_DISABLED);
    }

    #[test]
    fn transform_state_selected() {
        let mut btn = make_button();
        btn.base.state = UiState::Selected;
        assert_eq!(btn.transform_state_into_id(), BUTTON_SELECTED);
    }

    #[test]
    fn transform_state_focused() {
        let mut btn = make_button();
        btn.base.state = UiState::Focused;
        assert_eq!(btn.transform_state_into_id(), BUTTON_FOCUSED);
    }

    #[test]
    fn transform_state_default_with_default() {
        let btn = make_button();
        assert_eq!(btn.transform_state_into_id(), BUTTON_DEFAULT);
    }

    #[test]
    fn transform_state_default_without_default() {
        let mut btn = make_button();
        btn.base.with_default = false;
        assert_eq!(btn.transform_state_into_id(), NO_RESOURCE);
    }

    #[test]
    fn enable_resets_state() {
        let mut btn = make_button();
        btn.base.state = UiState::Selected;
        btn.set_enable(true);
        assert_eq!(btn.base.state, UiState::Default);
    }

    #[test]
    fn disable_emits_unselect() {
        let mut btn = make_button();
        btn.base.enabled = true;
        let event = btn.set_enable(false);
        assert!(event.is_some());
        assert_eq!(event.unwrap().msg_type, UiMsg::WidgetUnselect);
    }

    // ── Menu-button state-machine tests ──────────────────────────────

    #[test]
    fn menu_default_to_focused_on_hover_silent() {
        let mut btn = make_menu_button();
        let input = make_input(50.0, 15.0, MouseButtons::empty());
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Focused);
        // DEFAULT-state input handler never emits an event.
        assert!(events.is_empty());
    }

    #[test]
    fn menu_default_left_down_stays_default() {
        let mut btn = make_menu_button();
        let input = make_input(50.0, 15.0, MouseButtons::LEFT_DOWN);
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Default);
        assert!(events.is_empty());
    }

    #[test]
    fn menu_default_click_alone_focuses_no_event() {
        // The DEFAULT-state handler's only condition is
        // `inside && !LEFT_DOWN`. A `LEFT_CLICK` bit without `LEFT_DOWN`
        // also satisfies it, so the click frame transitions DEFAULT →
        // FOCUSED silently — the click itself is dropped (no event).
        let mut btn = make_menu_button();
        let input = make_input(50.0, 15.0, MouseButtons::LEFT_CLICK);
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Focused);
        assert!(events.is_empty());
    }

    #[test]
    fn menu_focused_left_down_pushes_silently_and_captures() {
        let mut btn = make_menu_button();
        btn.base.state = UiState::Focused;
        let capture = super::super::CaptureSlot::new();
        let input = make_input_with_capture(50.0, 15.0, MouseButtons::LEFT_DOWN, &capture);
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Pushed);
        assert!(events.is_empty());
        assert_eq!(capture.get(), Some(1));
    }

    #[test]
    fn menu_focused_hover_emits_focused_each_frame() {
        let mut btn = make_menu_button();
        btn.base.state = UiState::Focused;
        let input = make_input(50.0, 15.0, MouseButtons::empty());
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Focused);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].msg_type, UiMsg::WidgetFocused);
    }

    #[test]
    fn menu_focused_mouse_out_silent_default() {
        let mut btn = make_menu_button();
        btn.base.state = UiState::Focused;
        let input = make_input(200.0, 200.0, MouseButtons::empty());
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Default);
        // No WidgetUnfocused event in the menu-button machine.
        assert!(events.is_empty());
    }

    #[test]
    fn menu_pushed_click_inside_goes_focused_with_activated() {
        let mut btn = make_menu_button();
        btn.base.state = UiState::Pushed;
        let capture = super::super::CaptureSlot::new();
        capture.set(1);
        let input = make_input_with_capture(50.0, 15.0, MouseButtons::LEFT_CLICK, &capture);
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Focused);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].msg_type, UiMsg::WidgetActivated);
        assert_eq!(capture.get(), None);
    }

    #[test]
    fn menu_pushed_drag_off_goes_selected() {
        let mut btn = make_menu_button();
        btn.base.state = UiState::Pushed;
        let input = make_input(200.0, 200.0, MouseButtons::LEFT_DOWN);
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Selected);
        assert!(events.is_empty());
    }

    #[test]
    fn menu_selected_drag_back_re_arms_pushed() {
        let mut btn = make_menu_button();
        btn.base.state = UiState::Selected;
        let input = make_input(50.0, 15.0, MouseButtons::LEFT_DOWN);
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Pushed);
        assert!(events.is_empty());
    }

    #[test]
    fn menu_selected_release_outside_cancels_to_default() {
        let mut btn = make_menu_button();
        btn.base.state = UiState::Selected;
        let capture = super::super::CaptureSlot::new();
        capture.set(1);
        let input = make_input_with_capture(200.0, 200.0, MouseButtons::LEFT_CLICK, &capture);
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Default);
        assert!(events.is_empty());
        assert_eq!(capture.get(), None);
    }

    #[test]
    fn menu_selected_outside_no_click_stays_selected() {
        let mut btn = make_menu_button();
        btn.base.state = UiState::Selected;
        let input = make_input(200.0, 200.0, MouseButtons::LEFT_DOWN);
        let events = btn.process_input(&input);
        assert_eq!(btn.base.state, UiState::Selected);
        assert!(events.is_empty());
    }

    // ── Menu-button transform_state_into_id tests ────────────────────

    #[test]
    fn menu_transform_default_renders_default() {
        let btn = make_menu_button();
        assert_eq!(btn.transform_state_into_id(), BUTTON_DEFAULT);
    }

    #[test]
    fn menu_transform_focused_renders_focused() {
        let mut btn = make_menu_button();
        btn.base.state = UiState::Focused;
        assert_eq!(btn.transform_state_into_id(), BUTTON_FOCUSED);
    }

    #[test]
    fn menu_transform_focused_with_hide_focus_renders_default() {
        let mut btn = make_menu_button();
        btn.base.state = UiState::Focused;
        btn.hide_focus = true;
        assert_eq!(btn.transform_state_into_id(), BUTTON_DEFAULT);
    }

    #[test]
    fn menu_transform_pushed_renders_selected() {
        let mut btn = make_menu_button();
        btn.base.state = UiState::Pushed;
        assert_eq!(btn.transform_state_into_id(), BUTTON_SELECTED);
    }

    #[test]
    fn menu_transform_selected_renders_focused() {
        let mut btn = make_menu_button();
        btn.base.state = UiState::Selected;
        // Menu-button override: SELECTED state (drag-off-while-held)
        // renders the FOCUSED sprite.
        assert_eq!(btn.transform_state_into_id(), BUTTON_FOCUSED);
    }

    #[test]
    fn menu_transform_group_state_aggregates() {
        let mut btn = make_menu_button();
        btn.base.state = UiState::Default;
        btn.group_state = UiState::Focused;
        // max(DEFAULT, FOCUSED) = FOCUSED → BUTTON_FOCUSED.
        assert_eq!(btn.transform_state_into_id(), BUTTON_FOCUSED);
    }

    #[test]
    fn menu_transform_group_pushed_overrides_default() {
        let mut btn = make_menu_button();
        btn.base.state = UiState::Default;
        btn.group_state = UiState::Pushed;
        assert_eq!(btn.transform_state_into_id(), BUTTON_SELECTED);
    }

    // ── set_group_focused / set_group_selected tests ─────────────────

    #[test]
    fn set_group_focused_enabled_emits_focused() {
        let mut btn = make_menu_button();
        let events = btn.set_group_focused(true);
        assert_eq!(btn.group_state, UiState::Focused);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].msg_type, UiMsg::WidgetFocused);
    }

    #[test]
    fn set_group_focused_disabled_silent() {
        let mut btn = make_menu_button();
        btn.base.enabled = false;
        let events = btn.set_group_focused(true);
        assert_eq!(btn.group_state, UiState::Focused);
        assert!(events.is_empty());
    }

    #[test]
    fn set_group_unfocused_silent() {
        let mut btn = make_menu_button();
        let events = btn.set_group_focused(false);
        assert_eq!(btn.group_state, UiState::Default);
        assert!(events.is_empty());
    }

    #[test]
    fn set_group_selected_writes_pushed_no_event() {
        let mut btn = make_menu_button();
        let events = btn.set_group_selected(true);
        assert_eq!(btn.group_state, UiState::Pushed);
        assert!(events.is_empty());
    }

    #[test]
    fn set_group_selected_disabled_still_writes() {
        // Unlike WidgetToggleButton, set_group_selected on a menu
        // button has no enabled-state gate.
        let mut btn = make_menu_button();
        btn.base.enabled = false;
        let events = btn.set_group_selected(true);
        assert_eq!(btn.group_state, UiState::Pushed);
        assert!(events.is_empty());
    }

    #[test]
    fn hide_focus_sets_flag() {
        let mut btn = make_menu_button();
        btn.hide_focus(true);
        assert!(btn.hide_focus);
        btn.hide_focus(false);
        assert!(!btn.hide_focus);
    }

    #[test]
    fn activate_enabled_emits_activated() {
        let mut btn = make_menu_button();
        let events = btn.activate();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].msg_type, UiMsg::WidgetActivated);
    }

    #[test]
    fn activate_disabled_silent() {
        let mut btn = make_menu_button();
        btn.base.enabled = false;
        let events = btn.activate();
        assert!(events.is_empty());
    }

    #[test]
    fn is_sleeping_true_in_default() {
        let btn = make_menu_button();
        assert!(btn.is_sleeping());
    }

    #[test]
    fn is_sleeping_false_in_focused() {
        let mut btn = make_menu_button();
        btn.base.state = UiState::Focused;
        assert!(!btn.is_sleeping());
    }

    #[test]
    fn groupable_trait_routes() {
        use crate::focus_manager::{UiEventType, WidgetGroupable};
        let mut btn = make_menu_button();
        let g: &mut dyn WidgetGroupable = &mut btn;
        assert_eq!(g.widget_id(), 1);
        assert!(g.is_enabled());
        assert!(g.is_sleeping());
        let events = g.activate();
        assert_eq!(events[0].msg_type, UiEventType::Activated);
        assert_eq!(events[0].origin, 1);
    }
}
