//! Radio button widget (two-state with group exclusion).
//!
//! Radio buttons form groups where only one can be selected at a time.
//! Group management is handled externally by [`FrameWnd`] or the
//! caller — this widget only tracks its own two-state toggle and emits
//! events that the group manager can react to.

use serde::{Deserialize, Serialize};

use crate::focus_manager::WidgetGroupable;
use crate::ui::{
    KeyState, MouseButtons, UiEvent, UiEventData, UiMsg, UiState,
    resource_widget_id::{
        RADIO_EX_DEFAULT1, RADIO_EX_DEFAULT2, RADIO_EX_DISABLED, RADIO_EX_FOCUSED1,
        RADIO_EX_FOCUSED2, RADIO_EX_PUSHED1, RADIO_EX_PUSHED2,
    },
};
use robin_engine::coordinates::ScreenPoint;

use super::{WidgetBase, WidgetId, WidgetInput};

/// Radio button widget.
///
/// Two-state button where group exclusion is managed externally.
/// The `second_state` field tracks selected vs. unselected.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WidgetRadioButton {
    pub base: WidgetBase,
    /// `false` = first/unselected state, `true` = second/selected state.
    pub second_state: bool,
    /// IDs of other radio buttons in the same group.
    /// When this button is selected, the group manager should call
    /// `set_active` on each of these to deselect them.
    pub group_members: Vec<WidgetId>,
    /// Focus-manager: the owning group currently has focus.
    pub group_focused: bool,
    /// Focus-manager: this entry is the selected one in the group.
    pub group_selected: bool,
    /// `HideFocus` side effect — bookkeeping only.
    pub focus_hidden: bool,
}

impl WidgetRadioButton {
    pub fn new(id: WidgetId) -> Self {
        Self {
            base: WidgetBase {
                id,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Whether this radio button is in its selected (second) state.
    pub fn is_pushed(&self) -> bool {
        self.second_state
    }

    /// Called by the group manager when another radio button in the
    /// group becomes active. Deselects this button.
    ///
    /// Only siblings currently in `SelectedSecond` are mutated, and
    /// they land at `SelectedFirst` (not `Default`). Bystanders sitting
    /// in `Focused*`, `Pushed*`, `SelectedFirst`, or `Default` keep
    /// their transient state.
    pub fn set_active_other(&mut self) {
        if self.second_state || matches!(self.base.state, UiState::SelectedSecond) {
            self.second_state = false;
            self.base.state = UiState::SelectedFirst;
        }
    }

    /// Programmatically select this radio button.
    pub fn set_selected(&mut self, selected: bool) {
        self.second_state = selected;
        if selected {
            self.base.state = UiState::SelectedSecond;
        } else {
            self.base.state = UiState::Default;
        }
    }

    /// Map current state to renderer sub-resource ID.
    ///
    /// Uses the extended radio resource IDs (RADIO_EX_*).
    pub fn transform_state_into_id(&self) -> u8 {
        if !self.base.enabled {
            return RADIO_EX_DISABLED;
        }
        match self.base.state {
            UiState::Default => {
                if self.second_state {
                    RADIO_EX_DEFAULT2
                } else {
                    RADIO_EX_DEFAULT1
                }
            }
            UiState::FocusedFirst => RADIO_EX_FOCUSED1,
            UiState::FocusedSecond => RADIO_EX_FOCUSED2,
            UiState::PushedFirst => RADIO_EX_PUSHED1,
            UiState::PushedSecond => RADIO_EX_PUSHED2,
            UiState::SelectedFirst => RADIO_EX_DEFAULT1,
            UiState::SelectedSecond => RADIO_EX_DEFAULT2,
            _ => {
                if self.second_state {
                    RADIO_EX_DEFAULT2
                } else {
                    RADIO_EX_DEFAULT1
                }
            }
        }
    }

    /// Process input for one frame.
    pub fn process_input(&mut self, input: &WidgetInput) -> Vec<UiEvent> {
        if !self.base.enabled {
            return self.base.tooltip_event_if_disabled().into_iter().collect();
        }

        let inside = self.base.is_inside(input.mouse_position);
        let buttons = input.mouse_button;

        // Check accelerator key.
        if self.base.fast_key != 0 {
            let key_state = input.keyboard.get_state_of_key(self.base.fast_key);
            match key_state {
                KeyState::KeyDown => {
                    return self.process_pushed();
                }
                KeyState::KeyPressed | KeyState::KeyDouble => {
                    return self.process_select_toggle();
                }
                _ => {}
            }
        }

        match self.base.state {
            UiState::Default => {
                if inside {
                    if buttons.contains(MouseButtons::LEFT_DOWN) {
                        return self.process_pushed();
                    }
                    if buttons.contains(MouseButtons::LEFT_CLICK)
                        || buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK)
                    {
                        return self.process_select_toggle();
                    }
                    if buttons.contains(MouseButtons::RIGHT_CLICK) {
                        return self.process_unselect();
                    }
                    if self.base.with_focus {
                        return self.process_focus();
                    }
                }
                Vec::new()
            }

            UiState::FocusedFirst | UiState::FocusedSecond => {
                if inside {
                    if buttons.contains(MouseButtons::LEFT_DOWN) {
                        return self.process_pushed();
                    }
                    if buttons.contains(MouseButtons::LEFT_CLICK)
                        || buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK)
                    {
                        return self.process_select_toggle();
                    }
                    if buttons.contains(MouseButtons::RIGHT_CLICK) {
                        return self.process_unselect();
                    }
                    Vec::new()
                } else {
                    self.process_default()
                }
            }

            UiState::PushedFirst | UiState::PushedSecond => {
                if inside {
                    if buttons.contains(MouseButtons::LEFT_CLICK)
                        || buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK)
                    {
                        return self.process_select_toggle();
                    }
                    if !buttons.contains(MouseButtons::LEFT_DOWN) {
                        return self.process_focus();
                    }
                    Vec::new()
                } else {
                    self.process_default()
                }
            }

            UiState::SelectedFirst | UiState::SelectedSecond => {
                if inside {
                    if buttons.contains(MouseButtons::LEFT_CLICK)
                        || buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK)
                    {
                        return self.process_select_toggle();
                    }
                    if buttons.contains(MouseButtons::RIGHT_CLICK) {
                        return self.process_unselect();
                    }
                    Vec::new()
                } else {
                    self.process_default()
                }
            }

            _ => Vec::new(),
        }
    }

    // ── State transition helpers ──────��────────────────────────���────

    fn process_default(&mut self) -> Vec<UiEvent> {
        self.base.state = UiState::Default;
        vec![self.base.make_event(UiMsg::WidgetUnfocused)]
    }

    fn process_focus(&mut self) -> Vec<UiEvent> {
        self.base.state = if self.second_state {
            UiState::FocusedSecond
        } else {
            UiState::FocusedFirst
        };
        vec![self.base.make_event(UiMsg::WidgetFocused)]
    }

    fn process_pushed(&mut self) -> Vec<UiEvent> {
        self.base.state = if self.second_state {
            UiState::PushedSecond
        } else {
            UiState::PushedFirst
        };
        vec![self.base.make_event(UiMsg::WidgetFocused)]
    }

    /// Toggle selection and emit the appropriate event.
    fn process_select_toggle(&mut self) -> Vec<UiEvent> {
        if self.second_state {
            // Already selected — this is a re-select or already-unselected event.
            self.base.state = UiState::SelectedSecond;
            vec![self.base.make_event(UiMsg::WidgetReactivated)]
        } else {
            // Selecting for the first time.
            self.second_state = true;
            self.base.state = UiState::SelectedSecond;
            vec![
                self.base
                    .make_event_with_data(UiMsg::WidgetActivated, UiEventData::ListIndex(1)),
            ]
        }
    }

    fn process_unselect(&mut self) -> Vec<UiEvent> {
        if self.second_state {
            self.base.state = UiState::Default;
            vec![self.base.make_event(UiMsg::WidgetUnselect)]
        } else {
            vec![self.base.make_event(UiMsg::WidgetAlreadyUnselected)]
        }
    }

    /// Programmatic activation entry point used by the focus manager
    /// (keyboard Enter / shortcut release). Gated on
    /// `enabled && state == SelectedFirst`; transitions to
    /// `SelectedSecond` and emits `WidgetActivated`. Disabled or
    /// non-`SelectedFirst` calls are no-ops.
    ///
    /// Group exclusion is applied by callers that own the sibling list:
    /// `FrameWnd::process_input` for mouse dispatch and
    /// `FocusManager::apply_group_activation` for keyboard/shortcut
    /// dispatch.
    pub fn activate(&mut self) -> Vec<UiEvent> {
        if !self.base.enabled || self.base.state != UiState::SelectedFirst {
            return Vec::new();
        }
        self.second_state = true;
        self.base.state = UiState::SelectedSecond;
        vec![
            self.base
                .make_event_with_data(UiMsg::WidgetActivated, UiEventData::ListIndex(1)),
        ]
    }

    /// Group-focus flag setter. Emits `WidgetReactivated` when the
    /// group gains focus on an enabled widget.
    pub fn set_group_focused(&mut self, focused: bool) -> Vec<UiEvent> {
        self.group_focused = focused;
        if self.base.enabled && focused {
            vec![self.base.make_event_with_data(
                UiMsg::WidgetReactivated,
                UiEventData::ListIndex(if self.second_state { 1 } else { 0 }),
            )]
        } else {
            Vec::new()
        }
    }

    /// Group-selected flag setter. Disabled widgets swallow the call
    /// without changing state.
    pub fn set_group_selected(&mut self, selected: bool) -> Vec<UiEvent> {
        if self.base.enabled {
            self.group_selected = selected;
        }
        Vec::new()
    }

    /// Stash the hide-focus flag for downstream renderers.
    pub fn hide_focus(&mut self, hide: bool) {
        self.focus_hidden = hide;
    }

    /// Always returns `true`.
    pub fn is_sleeping(&self) -> bool {
        true
    }

    /// Hit-test for the focus manager: bbox + per-pixel transparency
    /// via `WidgetBase::is_inside` → `RendererBase::is_real_point`
    /// (samples an `AlphaMask` baked from the bound sprite if the
    /// wiring layer attached one — see
    /// `widget_bridge::attach_alpha_masks`). Falls back to bbox-only
    /// when no mask is attached.
    pub fn is_mouse_inside(&self, point: ScreenPoint) -> bool {
        self.base.is_inside(point)
    }
}

// ── WidgetGroupable trait impl ─────────────────────────────────────
//
// Glue for `FocusManager` so a radio button can be dropped into the
// group navigation chain.
//
// `focus_manager` uses its own `UiEvent` / `UiEventType` pair (a
// slimmer subset without data payload) rather than `ui::UiEvent`; we
// translate via the `ui_event_to_focus_event` shape below.

fn ui_event_to_focus_event(event: UiEvent) -> Option<crate::focus_manager::UiEvent> {
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

impl WidgetGroupable for WidgetRadioButton {
    fn widget_id(&self) -> crate::focus_manager::WidgetId {
        self.base.id as crate::focus_manager::WidgetId
    }

    fn is_enabled(&self) -> bool {
        self.base.enabled
    }

    fn is_sleeping(&self) -> bool {
        WidgetRadioButton::is_sleeping(self)
    }

    fn is_mouse_inside(&self, point: ScreenPoint) -> bool {
        WidgetRadioButton::is_mouse_inside(self, point)
    }

    fn hide_focus(&mut self, hide: bool) {
        WidgetRadioButton::hide_focus(self, hide);
    }

    fn set_group_focused(&mut self, focused: bool) -> Vec<crate::focus_manager::UiEvent> {
        translate(WidgetRadioButton::set_group_focused(self, focused))
    }

    fn set_group_selected(&mut self, selected: bool) -> Vec<crate::focus_manager::UiEvent> {
        translate(WidgetRadioButton::set_group_selected(self, selected))
    }

    fn activate(&mut self) -> Vec<crate::focus_manager::UiEvent> {
        translate(WidgetRadioButton::activate(self))
    }

    fn group_members(&self) -> Vec<crate::focus_manager::WidgetId> {
        self.group_members
            .iter()
            .map(|&id| id as crate::focus_manager::WidgetId)
            .collect()
    }

    fn set_active_other(&mut self) {
        WidgetRadioButton::set_active_other(self);
    }
}
