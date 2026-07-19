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
