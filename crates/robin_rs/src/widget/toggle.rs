//! Two-state toggle button widget.
//!
//! Implements menu-specific state-machine behaviour plus group-focus
//! support. Nearly every toggle button in the game uses these menu
//! semantics, so that's what's implemented here — a toggle that wanted
//! the simpler raw behaviour could add a flag later.
//!
//! State machine highlights:
//! - `SelectedFirst` / `SelectedSecond` hover → transition to the
//!   matching `Focused*` state and emit `WidgetFocused`.
//! - `FocusedFirst` / `FocusedSecond` mouse-leave → go back to the
//!   matching `Selected*` state (not `Default`).
//! - `PushedFirst` click / double-click both emit `WidgetActivated`
//!   (not `Reactivated`) and flip to `SelectedSecond`.
//! - `PushedSecond` single-click → `SelectedFirst` with
//!   `WidgetReactivated`. Double-click → stays `SelectedSecond` with
//!   `WidgetReactivated` plus a trailing `WidgetDoubleClicked` event.
//! - `Activate()` is the keyboard/focus-manager activation entry point.
//!   Keyboard accelerator keys run first at the top of `process_input`.

use serde::{Deserialize, Serialize};

use crate::focus_manager::WidgetGroupable;
use crate::ui::{
    KeyState, MouseButtons, UiEvent, UiEventData, UiMsg, UiState,
    resource_widget_id::{
        BUTTON_FOCUSED, BUTTON_SELECTED, NO_RESOURCE, TOGGLE_DISABLED, TOGGLE_FOCUSED_ONE,
        TOGGLE_FOCUSED_TWO, TOGGLE_SELECTED_ONE, TOGGLE_SELECTED_TWO,
    },
};
use robin_engine::coordinates::ScreenPoint;

use super::{WidgetBase, WidgetInput};

/// Two-state toggle button.
///
/// `second_state` tracks which visual/logical state the toggle is in.
/// Each activation flips the state. `group_focused` / `group_selected`
/// come from the [`WidgetGroupable`] mixin and let an owning focus
/// manager override the rendered sprite to the menu BUTTON_FOCUSED /
/// BUTTON_SELECTED styles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetToggleButton {
    pub base: WidgetBase,
    /// `false` = first state, `true` = second state.
    pub second_state: bool,
    /// Focus-manager: the owning group currently has focus.
    pub group_focused: bool,
    /// Focus-manager: this entry is the selected one in the group.
    pub group_selected: bool,
    /// HideFocus side effect — bookkeeping only. The renderer ignores
    /// this flag today, but we track it so a future renderer can
    /// consume it.
    pub focus_hidden: bool,
}

impl Default for WidgetToggleButton {
    fn default() -> Self {
        Self {
            base: WidgetBase {
                // Seed with `SelectedFirst` so the state-machine
                // dispatch reaches a valid arm on the first frame.
                state: UiState::SelectedFirst,
                ..Default::default()
            },
            second_state: false,
            group_focused: false,
            group_selected: false,
            focus_hidden: false,
        }
    }
}

impl WidgetToggleButton {
    pub fn new(id: super::WidgetId) -> Self {
        Self {
            base: WidgetBase {
                id,
                state: UiState::SelectedFirst,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Get the current toggle state (0 = first, 1 = second).
    pub fn state_index(&self) -> u32 {
        if self.second_state { 1 } else { 0 }
    }

    /// Group-focus flag setter. Emits `WidgetReactivated` when the
    /// group gains focus on an enabled widget.
    pub fn set_group_focused(&mut self, focused: bool) -> Vec<UiEvent> {
        self.group_focused = focused;
        if self.base.enabled && focused {
            vec![self.event_with_state(UiMsg::WidgetReactivated)]
        } else {
            Vec::new()
        }
    }

    /// Group-selected flag setter. Silently swallows the call on a
    /// disabled widget (no flag change, no event).
    pub fn set_group_selected(&mut self, selected: bool) -> Vec<UiEvent> {
        if self.base.enabled {
            self.group_selected = selected;
        }
        Vec::new()
    }

    /// Programmatic activation (keyboard accelerator or focus-manager
    /// enter press). Toggling out of `SelectedSecond` emits
    /// `WidgetReactivated`; anything else emits `WidgetActivated` and
    /// lands in `SelectedSecond`.
    pub fn activate(&mut self) -> Vec<UiEvent> {
        if !self.base.enabled {
            return Vec::new();
        }
        let msg = if self.base.state == UiState::SelectedSecond {
            self.second_state = false;
            self.base.state = UiState::SelectedFirst;
            UiMsg::WidgetReactivated
        } else {
            self.second_state = true;
            self.base.state = UiState::SelectedSecond;
            UiMsg::WidgetActivated
        };
        // Programmatic activation deliberately drops the
        // `second_state` payload that mouse-driven activation carries —
        // emit a plain event with no data.
        vec![self.base.make_event(msg)]
    }

    /// Focus-visibility toggle. Stashes the flag so downstream code
    /// can read it; the built-in renderer ignores it.
    pub fn hide_focus(&mut self, hide: bool) {
        self.focus_hidden = hide;
    }

    /// Whether this toggle is currently in its second (upper) state.
    pub fn is_upper_state(&self) -> bool {
        self.second_state
    }

    /// Whether this toggle is currently in a focused arm.
    pub fn is_focused(&self) -> bool {
        matches!(
            self.base.state,
            UiState::FocusedFirst | UiState::FocusedSecond
        )
    }

    /// Force the toggle into a focused arm without emitting events.
    /// Picks the first/second half based on the current `second_state`.
    pub fn set_focused(&mut self) {
        self.base.state = if self.second_state {
            UiState::FocusedSecond
        } else {
            UiState::FocusedFirst
        };
    }

    /// Whether the toggle is "sleeping" (not eligible for auto-focus).
    /// Always `true` for menu toggles — the focus manager skips
    /// sleeping widgets during navigation auto-advance.
    pub fn is_sleeping(&self) -> bool {
        true
    }

    /// Hit-test for the focus manager: bbox plus per-pixel sample
    /// against any attached alpha mask.
    ///
    /// `WidgetBase::is_inside` does both halves: bbox via
    /// `BBox2D::is_boxed_point`, then `RendererBase::is_real_point`
    /// rejects clicks on transparent pixels when an `AlphaMask` is
    /// attached (the wiring layer — `widget_bridge::attach_alpha_masks` —
    /// bakes one for any widget whose `resource_id` resolves to a known
    /// sprite pack). Toggle widgets without a baked mask gracefully
    /// fall back to bbox-only.
    pub fn is_mouse_inside(&self, point: ScreenPoint) -> bool {
        self.base.is_inside(point)
    }

    /// Build an activation/focus event carrying `second_state` as
    /// `ListIndex` data (0 for first state, 1 for second).
    fn event_with_state(&self, msg: UiMsg) -> UiEvent {
        self.base
            .make_event_with_data(msg, UiEventData::ListIndex(self.state_index()))
    }

    /// Map current state to renderer sub-resource ID. Group flags are
    /// checked first, then disabled, then the per-state mapping.
    pub fn transform_state_into_id(&self) -> u8 {
        // Group-highlight sprites take priority over disabled handling:
        // a disabled widget that still has a group flag set keeps its
        // group-highlight sprite.
        if self.group_selected {
            return BUTTON_SELECTED;
        }
        if self.group_focused {
            return BUTTON_FOCUSED;
        }
        if !self.base.enabled {
            return TOGGLE_DISABLED;
        }

        let id = match self.base.state {
            UiState::FocusedFirst => {
                if self.base.with_focus {
                    TOGGLE_FOCUSED_ONE
                } else {
                    TOGGLE_SELECTED_ONE
                }
            }
            UiState::SelectedFirst | UiState::PushedFirst => TOGGLE_SELECTED_ONE,
            UiState::FocusedSecond => {
                if self.base.with_focus {
                    TOGGLE_FOCUSED_TWO
                } else {
                    TOGGLE_SELECTED_TWO
                }
            }
            UiState::SelectedSecond | UiState::PushedSecond => TOGGLE_SELECTED_TWO,
            UiState::Default => {
                if self.second_state {
                    TOGGLE_SELECTED_TWO
                } else {
                    TOGGLE_SELECTED_ONE
                }
            }
            _ => {
                if self.second_state {
                    TOGGLE_SELECTED_TWO
                } else {
                    TOGGLE_SELECTED_ONE
                }
            }
        };

        // `with_default == false` hides the neutral selected sprites
        // (the widget renders nothing in its resting state).
        if !self.base.with_default && (id == TOGGLE_SELECTED_ONE || id == TOGGLE_SELECTED_TWO) {
            NO_RESOURCE
        } else {
            id
        }
    }

    /// Set enable state. Only touches the enabled flag and leaves the
    /// state machine alone (unlike the push-button version which
    /// resets to Default).
    pub fn set_enable(&mut self, enabled: bool) -> Option<UiEvent> {
        self.base.enabled = enabled;
        None
    }

    /// Process input for one frame. Dispatches by state to the six
    /// per-state arms. An accelerator-key path runs first so keyboard
    /// shortcuts still work even when the mouse-driven dispatch
    /// wouldn't react.
    pub fn process_input(&mut self, input: &WidgetInput) -> Vec<UiEvent> {
        if !self.base.enabled {
            return self.base.tooltip_event_if_disabled().into_iter().collect();
        }

        // Accelerator key handling.
        if let Some(fast_key) = self.base.fast_key {
            let key_state = input.keyboard.get_state_of_key(fast_key);
            match key_state {
                KeyState::KeyDown => {
                    self.base.state = if self.second_state {
                        UiState::PushedSecond
                    } else {
                        UiState::PushedFirst
                    };
                    return vec![self.event_with_state(UiMsg::WidgetFocused)];
                }
                KeyState::KeyPressed | KeyState::KeyDouble => {
                    self.second_state = !self.second_state;
                    self.base.state = if self.second_state {
                        UiState::SelectedSecond
                    } else {
                        UiState::SelectedFirst
                    };
                    return vec![self.event_with_state(UiMsg::WidgetActivated)];
                }
                _ => {}
            }
        }

        let inside = self.base.is_inside(input.mouse_position);
        let buttons = input.mouse_button;
        let capture = input.capture;

        match self.base.state {
            UiState::SelectedFirst => self.process_input_selected_first(inside, buttons),
            UiState::FocusedFirst => self.process_input_focused_first(inside, buttons, capture),
            UiState::PushedFirst => self.process_input_pushed_first(inside, buttons, capture),
            UiState::SelectedSecond => self.process_input_selected_second(inside, buttons),
            UiState::FocusedSecond => self.process_input_focused_second(inside, buttons, capture),
            UiState::PushedSecond => self.process_input_pushed_second(inside, buttons, capture),
            _ => {
                // Not a menu-toggle state (e.g. freshly deserialized at
                // `Default`). Re-sync to the Selected arm matching
                // `second_state` so the state machine can progress on
                // the next frame.
                self.base.state = if self.second_state {
                    UiState::SelectedSecond
                } else {
                    UiState::SelectedFirst
                };
                Vec::new()
            }
        }
    }

    // ── Per-state arms ─────────────────────────────────────────────

    /// `SelectedFirst` arm: hover transitions to `FocusedFirst`.
    fn process_input_selected_first(
        &mut self,
        inside: bool,
        buttons: MouseButtons,
    ) -> Vec<UiEvent> {
        if inside && !buttons.contains(MouseButtons::LEFT_DOWN) {
            self.base.state = UiState::FocusedFirst;
            return vec![self.event_with_state(UiMsg::WidgetFocused)];
        }
        Vec::new()
    }

    /// `FocusedFirst` arm: mouse-down → `PushedFirst` (with capture);
    /// leaving with no button held → `SelectedFirst`.
    fn process_input_focused_first(
        &mut self,
        inside: bool,
        buttons: MouseButtons,
        capture: Option<&super::CaptureSlot>,
    ) -> Vec<UiEvent> {
        if inside {
            if buttons.contains(MouseButtons::LEFT_DOWN) {
                self.base.state = UiState::PushedFirst;
                if let Some(slot) = capture {
                    slot.set(self.base.id);
                }
            }
        } else {
            if !buttons.contains(MouseButtons::LEFT_DOWN) {
                self.base.state = UiState::SelectedFirst;
                self.second_state = false;
            }
            if (buttons.contains(MouseButtons::LEFT_CLICK)
                || buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK))
                && let Some(slot) = capture
            {
                slot.clear();
            }
        }
        Vec::new()
    }

    /// `PushedFirst` arm: click/double-click flips to
    /// `SelectedSecond` and emits `WidgetActivated`.
    fn process_input_pushed_first(
        &mut self,
        inside: bool,
        buttons: MouseButtons,
        capture: Option<&super::CaptureSlot>,
    ) -> Vec<UiEvent> {
        if !inside {
            if buttons.contains(MouseButtons::LEFT_DOWN) {
                self.base.state = UiState::FocusedFirst;
                self.second_state = false;
            }
            return Vec::new();
        }
        if buttons.contains(MouseButtons::LEFT_CLICK)
            || buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK)
        {
            self.base.state = UiState::SelectedSecond;
            self.second_state = true;
            if let Some(slot) = capture {
                slot.clear();
            }
            return vec![self.event_with_state(UiMsg::WidgetActivated)];
        }
        Vec::new()
    }

    /// `SelectedSecond` arm: hover transitions to `FocusedSecond`.
    fn process_input_selected_second(
        &mut self,
        inside: bool,
        buttons: MouseButtons,
    ) -> Vec<UiEvent> {
        if inside && !buttons.contains(MouseButtons::LEFT_DOWN) {
            self.base.state = UiState::FocusedSecond;
            return vec![self.event_with_state(UiMsg::WidgetFocused)];
        }
        Vec::new()
    }

    /// `FocusedSecond` arm: mouse-down → `PushedSecond` (with
    /// capture); leaving with no button held → `SelectedSecond`.
    fn process_input_focused_second(
        &mut self,
        inside: bool,
        buttons: MouseButtons,
        capture: Option<&super::CaptureSlot>,
    ) -> Vec<UiEvent> {
        if !inside {
            if !buttons.contains(MouseButtons::LEFT_DOWN) {
                self.base.state = UiState::SelectedSecond;
                self.second_state = true;
            }
            if (buttons.contains(MouseButtons::LEFT_CLICK)
                || buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK))
                && let Some(slot) = capture
            {
                slot.clear();
            }
        } else if buttons.contains(MouseButtons::LEFT_DOWN) {
            self.base.state = UiState::PushedSecond;
            if let Some(slot) = capture {
                slot.set(self.base.id);
            }
        }
        Vec::new()
    }

    /// `PushedSecond` arm: single-click flips to `SelectedFirst` with
    /// `WidgetReactivated`; double-click stays in `SelectedSecond`
    /// with `WidgetReactivated` plus a trailing `WidgetDoubleClicked`.
    fn process_input_pushed_second(
        &mut self,
        inside: bool,
        buttons: MouseButtons,
        capture: Option<&super::CaptureSlot>,
    ) -> Vec<UiEvent> {
        if !inside {
            if buttons.contains(MouseButtons::LEFT_DOWN) {
                self.base.state = UiState::FocusedSecond;
            }
            return Vec::new();
        }
        let clicked = buttons.contains(MouseButtons::LEFT_CLICK);
        let double_clicked = buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK);
        if !clicked && !double_clicked {
            return Vec::new();
        }

        // LEFT_CLICK takes priority over LEFT_DOUBLE_CLICK, even
        // though both bits can be set the same frame.
        if clicked {
            self.base.state = UiState::SelectedFirst;
            self.second_state = false;
        } else {
            self.base.state = UiState::SelectedSecond;
            self.second_state = true;
        }
        if let Some(slot) = capture {
            slot.clear();
        }

        let mut events = vec![self.event_with_state(UiMsg::WidgetReactivated)];
        if double_clicked {
            events.push(self.event_with_state(UiMsg::WidgetDoubleClicked));
        }
        events
    }
}

// ── WidgetGroupable trait impl ─────────────────────────────────────
//
// Glue for `FocusManager` so a toggle button can be dropped into the
// group navigation chain via `add_groupable`.
//
// `focus_manager` owns a compact navigation-event type, so widget events
// are adapted at this boundary and keep payload-heavy UI dispatch out of
// the keyboard/gamepad navigation layer.

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

impl WidgetGroupable for WidgetToggleButton {
    fn widget_id(&self) -> crate::focus_manager::WidgetId {
        self.base.id as crate::focus_manager::WidgetId
    }

    fn is_enabled(&self) -> bool {
        self.base.enabled
    }

    fn is_sleeping(&self) -> bool {
        WidgetToggleButton::is_sleeping(self)
    }

    fn is_mouse_inside(&self, point: ScreenPoint) -> bool {
        WidgetToggleButton::is_mouse_inside(self, point)
    }

    fn hide_focus(&mut self, hide: bool) {
        WidgetToggleButton::hide_focus(self, hide);
    }

    fn set_group_focused(&mut self, focused: bool) -> Vec<crate::focus_manager::UiEvent> {
        translate(WidgetToggleButton::set_group_focused(self, focused))
    }

    fn set_group_selected(&mut self, selected: bool) -> Vec<crate::focus_manager::UiEvent> {
        translate(WidgetToggleButton::set_group_selected(self, selected))
    }

    fn activate(&mut self) -> Vec<crate::focus_manager::UiEvent> {
        translate(WidgetToggleButton::activate(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::UiKeyboard;
    use robin_engine::coordinates::ScreenBBox;

    fn test_widget() -> WidgetToggleButton {
        let mut w = WidgetToggleButton::new(1);
        w.base.bbox = ScreenBBox::from_coords(0.0, 0.0, 10.0, 10.0);
        // No renderer attached → `is_real_point` would return false and
        // `is_inside` would fail. Give it a bitmap renderer with the
        // same bbox so hit-testing succeeds.
        w.base.renderer = crate::widget::WidgetRenderer::Bitmap(crate::ui::RendererBitmap {
            base: crate::ui::RendererBase {
                bbox: w.base.bbox,
                ..Default::default()
            },
        });
        w
    }

    fn input<'a>(pos: ScreenPoint, buttons: MouseButtons, kb: &'a UiKeyboard) -> WidgetInput<'a> {
        WidgetInput {
            mouse_position: pos,
            mouse_z: 0,
            mouse_button: buttons,
            keyboard: kb,
            text_input: "",
            capture: None,
        }
    }

    fn input_with_capture<'a>(
        pos: ScreenPoint,
        buttons: MouseButtons,
        kb: &'a UiKeyboard,
        capture: &'a super::super::CaptureSlot,
    ) -> WidgetInput<'a> {
        WidgetInput {
            mouse_position: pos,
            mouse_z: 0,
            mouse_button: buttons,
            keyboard: kb,
            text_input: "",
            capture: Some(capture),
        }
    }

    #[test]
    fn selected_first_hover_focuses() {
        let mut w = test_widget();
        let kb = UiKeyboard::default();
        let events = w.process_input(&input(
            ScreenPoint::new(5.0, 5.0),
            MouseButtons::empty(),
            &kb,
        ));
        assert_eq!(w.base.state, UiState::FocusedFirst);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].msg_type, UiMsg::WidgetFocused);
    }

    #[test]
    fn pushed_first_click_activates_and_flips() {
        let mut w = test_widget();
        let kb = UiKeyboard::default();
        w.base.state = UiState::PushedFirst;

        let events = w.process_input(&input(
            ScreenPoint::new(5.0, 5.0),
            MouseButtons::LEFT_CLICK,
            &kb,
        ));
        assert_eq!(w.base.state, UiState::SelectedSecond);
        assert!(w.second_state);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].msg_type, UiMsg::WidgetActivated);
    }

    #[test]
    fn pushed_first_double_click_also_emits_activated() {
        let mut w = test_widget();
        let kb = UiKeyboard::default();
        w.base.state = UiState::PushedFirst;

        let events = w.process_input(&input(
            ScreenPoint::new(5.0, 5.0),
            MouseButtons::LEFT_DOUBLE_CLICK,
            &kb,
        ));
        assert_eq!(events[0].msg_type, UiMsg::WidgetActivated);
        assert!(w.second_state);
    }

    #[test]
    fn pushed_first_leave_with_button_held_goes_focused_first() {
        let mut w = test_widget();
        let kb = UiKeyboard::default();
        w.base.state = UiState::PushedFirst;
        w.second_state = false;

        let events = w.process_input(&input(
            ScreenPoint::new(100.0, 100.0),
            MouseButtons::LEFT_DOWN,
            &kb,
        ));
        assert_eq!(w.base.state, UiState::FocusedFirst);
        assert!(!w.second_state);
        assert!(events.is_empty());
    }

    #[test]
    fn focused_first_leave_goes_selected_first() {
        let mut w = test_widget();
        let kb = UiKeyboard::default();
        w.base.state = UiState::FocusedFirst;

        let events = w.process_input(&input(
            ScreenPoint::new(100.0, 100.0),
            MouseButtons::empty(),
            &kb,
        ));
        assert_eq!(w.base.state, UiState::SelectedFirst);
        assert!(events.is_empty());
    }

    #[test]
    fn focused_second_leave_goes_selected_second() {
        let mut w = test_widget();
        let kb = UiKeyboard::default();
        w.second_state = true;
        w.base.state = UiState::FocusedSecond;

        let events = w.process_input(&input(
            ScreenPoint::new(100.0, 100.0),
            MouseButtons::empty(),
            &kb,
        ));
        assert_eq!(w.base.state, UiState::SelectedSecond);
        assert!(w.second_state);
        assert!(events.is_empty());
    }

    #[test]
    fn pushed_second_single_click_reactivates_and_goes_first() {
        let mut w = test_widget();
        let kb = UiKeyboard::default();
        w.second_state = true;
        w.base.state = UiState::PushedSecond;

        let events = w.process_input(&input(
            ScreenPoint::new(5.0, 5.0),
            MouseButtons::LEFT_CLICK,
            &kb,
        ));
        assert_eq!(w.base.state, UiState::SelectedFirst);
        assert!(!w.second_state);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].msg_type, UiMsg::WidgetReactivated);
    }

    #[test]
    fn pushed_second_double_click_stays_second_and_emits_two_events() {
        let mut w = test_widget();
        let kb = UiKeyboard::default();
        w.second_state = true;
        w.base.state = UiState::PushedSecond;

        let events = w.process_input(&input(
            ScreenPoint::new(5.0, 5.0),
            MouseButtons::LEFT_DOUBLE_CLICK,
            &kb,
        ));
        assert_eq!(w.base.state, UiState::SelectedSecond);
        assert!(w.second_state);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].msg_type, UiMsg::WidgetReactivated);
        assert_eq!(events[1].msg_type, UiMsg::WidgetDoubleClicked);
    }

    #[test]
    fn activate_from_first_goes_second_with_activated() {
        let mut w = test_widget();
        w.second_state = false;
        w.base.state = UiState::SelectedFirst;

        let events = w.activate();
        assert_eq!(w.base.state, UiState::SelectedSecond);
        assert!(w.second_state);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].msg_type, UiMsg::WidgetActivated);
    }

    #[test]
    fn activate_from_second_goes_first_with_reactivated() {
        let mut w = test_widget();
        w.second_state = true;
        w.base.state = UiState::SelectedSecond;

        let events = w.activate();
        assert_eq!(w.base.state, UiState::SelectedFirst);
        assert!(!w.second_state);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].msg_type, UiMsg::WidgetReactivated);
    }

    #[test]
    fn group_selected_preempts_sprite() {
        let mut w = test_widget();
        w.group_selected = true;
        assert_eq!(w.transform_state_into_id(), BUTTON_SELECTED);
    }

    #[test]
    fn group_focused_preempts_sprite_when_not_selected() {
        let mut w = test_widget();
        w.group_focused = true;
        assert_eq!(w.transform_state_into_id(), BUTTON_FOCUSED);
    }

    #[test]
    fn set_group_focused_enabled_emits_reactivated() {
        let mut w = test_widget();
        let events = w.set_group_focused(true);
        assert!(w.group_focused);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].msg_type, UiMsg::WidgetReactivated);
    }

    #[test]
    fn set_group_focused_disabled_is_silent() {
        let mut w = test_widget();
        w.base.enabled = false;
        let events = w.set_group_focused(true);
        assert!(w.group_focused); // flag still flips
        assert!(events.is_empty());
    }

    #[test]
    fn set_group_selected_disabled_is_noop() {
        let mut w = test_widget();
        w.base.enabled = false;
        let events = w.set_group_selected(true);
        assert!(!w.group_selected);
        assert!(events.is_empty());
    }

    #[test]
    fn activation_events_carry_list_index() {
        let mut w = test_widget();
        w.base.state = UiState::PushedFirst;
        let kb = UiKeyboard::default();
        let events = w.process_input(&input(
            ScreenPoint::new(5.0, 5.0),
            MouseButtons::LEFT_CLICK,
            &kb,
        ));
        match events[0].data {
            Some(UiEventData::ListIndex(1)) => {}
            ref other => panic!("expected ListIndex(1), got {other:?}"),
        }
    }

    #[test]
    fn push_captures_release_clears() {
        let mut w = test_widget();
        let kb = UiKeyboard::default();
        let capture = super::super::CaptureSlot::new();

        // Focused → Pushed sets capture.
        w.base.state = UiState::FocusedFirst;
        w.process_input(&input_with_capture(
            ScreenPoint::new(5.0, 5.0),
            MouseButtons::LEFT_DOWN,
            &kb,
            &capture,
        ));
        assert_eq!(capture.get(), Some(1));

        // Pushed → Selected (click) clears capture.
        w.process_input(&input_with_capture(
            ScreenPoint::new(5.0, 5.0),
            MouseButtons::LEFT_CLICK,
            &kb,
            &capture,
        ));
        assert_eq!(capture.get(), None);
    }

    #[test]
    fn hide_focus_sets_flag() {
        let mut w = test_widget();
        w.hide_focus(true);
        assert!(w.focus_hidden);
        w.hide_focus(false);
        assert!(!w.focus_hidden);
    }

    #[test]
    fn is_upper_state_tracks_second_state() {
        let mut w = test_widget();
        assert!(!w.is_upper_state());
        w.second_state = true;
        assert!(w.is_upper_state());
    }

    #[test]
    fn set_focused_picks_half_from_second_state() {
        let mut w = test_widget();
        w.set_focused();
        assert_eq!(w.base.state, UiState::FocusedFirst);
        w.second_state = true;
        w.set_focused();
        assert_eq!(w.base.state, UiState::FocusedSecond);
    }

    #[test]
    fn is_focused_true_only_on_focused_arms() {
        let mut w = test_widget();
        assert!(!w.is_focused());
        w.base.state = UiState::FocusedFirst;
        assert!(w.is_focused());
        w.base.state = UiState::FocusedSecond;
        assert!(w.is_focused());
        w.base.state = UiState::PushedFirst;
        assert!(!w.is_focused());
    }

    #[test]
    fn trait_impl_routes_to_inherent_methods() {
        use crate::focus_manager::UiEventType;
        let mut w = test_widget();
        let g: &mut dyn WidgetGroupable = &mut w;
        assert_eq!(g.widget_id(), 1);
        assert!(g.is_enabled());
        assert!(g.is_sleeping());
        let events = g.activate();
        assert_eq!(events[0].msg_type, UiEventType::Activated);
        assert_eq!(events[0].origin, 1);
    }
}
