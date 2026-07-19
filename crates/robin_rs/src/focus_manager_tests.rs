use super::*;
use std::cell::Cell;
use std::rc::Rc;

// ── Mock widgets ────────────────────────────────────────────────

struct MockGroupable {
    id: WidgetId,
    enabled: bool,
    sleeping: bool,
    mouse_rect: Option<(f32, f32, f32, f32)>, // (x1, y1, x2, y2)
    group_members: Vec<WidgetId>,
    deselected_by_peer: Option<Rc<Cell<bool>>>,
}

impl MockGroupable {
    fn new(id: WidgetId) -> Self {
        Self {
            id,
            enabled: true,
            sleeping: false,
            mouse_rect: None,
            group_members: Vec::new(),
            deselected_by_peer: None,
        }
    }

    fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    fn radio_group(mut self, group_members: Vec<WidgetId>) -> Self {
        self.group_members = group_members;
        self
    }

    fn deselect_flag(mut self, flag: Rc<Cell<bool>>) -> Self {
        self.deselected_by_peer = Some(flag);
        self
    }
}

impl WidgetGroupable for MockGroupable {
    fn widget_id(&self) -> WidgetId {
        self.id
    }
    fn is_enabled(&self) -> bool {
        self.enabled
    }
    fn is_sleeping(&self) -> bool {
        self.sleeping
    }
    fn is_mouse_inside(&self, point: ScreenPoint) -> bool {
        if let Some((x1, y1, x2, y2)) = self.mouse_rect {
            point.x >= x1 && point.x <= x2 && point.y >= y1 && point.y <= y2
        } else {
            false
        }
    }
    fn hide_focus(&mut self, _hide: bool) {}
    fn set_group_focused(&mut self, _focused: bool) -> Vec<UiEvent> {
        vec![UiEvent {
            msg_type: UiEventType::FocusChanged,
            origin: self.id,
        }]
    }
    fn set_group_selected(&mut self, _selected: bool) -> Vec<UiEvent> {
        vec![UiEvent {
            msg_type: UiEventType::SelectionChanged,
            origin: self.id,
        }]
    }
    fn activate(&mut self) -> Vec<UiEvent> {
        vec![UiEvent {
            msg_type: UiEventType::Activated,
            origin: self.id,
        }]
    }
    fn group_members(&self) -> Vec<WidgetId> {
        self.group_members.clone()
    }
    fn set_active_other(&mut self) {
        if let Some(flag) = &self.deselected_by_peer {
            flag.set(true);
        }
    }
}

struct MockFocusable {
    id: WidgetId,
}

impl MockFocusable {
    fn new(id: WidgetId) -> Self {
        Self { id }
    }
}

impl WidgetFocusable for MockFocusable {
    fn widget_id(&self) -> WidgetId {
        self.id
    }
    fn set_focusable_active(&mut self, _active: bool) -> Vec<UiEvent> {
        vec![UiEvent {
            msg_type: UiEventType::FocusableActiveChanged,
            origin: self.id,
        }]
    }
}

// ── Helpers ─────────────────────────────────────────────────────

fn keyboard_with_key_down(key: KeyCode) -> KeyboardState {
    let mut state = KeyboardState {
        has_changed: true,
        ..Default::default()
    };
    state.keys.insert(
        key,
        KeyInfo {
            press_state: KeyPressState::Down,
            has_changed: true,
            typewriter: TypewriterState::None,
        },
    );
    state
}

fn keyboard_with_key_up(key: KeyCode) -> KeyboardState {
    let mut state = KeyboardState {
        has_changed: true,
        ..Default::default()
    };
    state.keys.insert(
        key,
        KeyInfo {
            press_state: KeyPressState::Up,
            has_changed: true,
            typewriter: TypewriterState::None,
        },
    );
    state
}

fn input_with_key_down(key: KeyCode) -> UiInput {
    UiInput {
        mouse_position: ScreenPoint::new(0.0, 0.0),
        keyboard: keyboard_with_key_down(key),
    }
}

fn input_with_key_up(key: KeyCode) -> UiInput {
    UiInput {
        mouse_position: ScreenPoint::new(0.0, 0.0),
        keyboard: keyboard_with_key_up(key),
    }
}

// ── Construction tests ──────────────────────────────────────────

#[test]
fn vertical_orientation_keys() {
    let fm = FocusManager::new(GroupOrientation::Vertical);
    assert_eq!(fm.navigation_keys[GROUPABLE_PREVIOUS], Key::UpArrow);
    assert_eq!(fm.navigation_keys[GROUPABLE_NEXT], Key::DownArrow);
    assert_eq!(fm.navigation_keys[FOCUSABLE_PREVIOUS], Key::LeftArrow);
    assert_eq!(fm.navigation_keys[FOCUSABLE_NEXT], Key::RightArrow);
    assert!(fm.navigation_enabled);
    assert!(fm.shortcuts_enabled);
}

#[test]
fn horizontal_orientation_keys() {
    let fm = FocusManager::new(GroupOrientation::Horizontal);
    assert_eq!(fm.navigation_keys[GROUPABLE_PREVIOUS], Key::LeftArrow);
    assert_eq!(fm.navigation_keys[GROUPABLE_NEXT], Key::RightArrow);
    assert_eq!(fm.navigation_keys[FOCUSABLE_PREVIOUS], Key::UpArrow);
    assert_eq!(fm.navigation_keys[FOCUSABLE_NEXT], Key::DownArrow);
}

// ── Registration tests ──────────────────────────────────────────

#[test]
fn add_groupable_widgets() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_groupable(Box::new(MockGroupable::new(2)), false);
    assert_eq!(fm.group.len(), 2);
    assert!(fm.group[0].navigable);
    assert!(!fm.group[1].navigable);
}

#[test]
#[should_panic(expected = "groupable widget 1 already registered")]
fn add_duplicate_groupable_panics() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
}

#[test]
fn add_focusable_widgets() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_focusable(Box::new(MockFocusable::new(10)));
    fm.add_focusable(Box::new(MockFocusable::new(11)));
    assert_eq!(fm.focusable_widgets.len(), 2);
}

#[test]
#[should_panic(expected = "focusable widget 10 already registered")]
fn add_duplicate_focusable_panics() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_focusable(Box::new(MockFocusable::new(10)));
    fm.add_focusable(Box::new(MockFocusable::new(10)));
}

// ── Key extraction tests ────────────────────────────────────────

#[test]
fn get_key_arrow_keys() {
    assert_eq!(
        FocusManager::get_key(&keyboard_with_key_down(KeyCode::ArrowLeft)),
        Key::LeftArrow
    );
    assert_eq!(
        FocusManager::get_key(&keyboard_with_key_down(KeyCode::ArrowUp)),
        Key::UpArrow
    );
    assert_eq!(
        FocusManager::get_key(&keyboard_with_key_down(KeyCode::ArrowRight)),
        Key::RightArrow
    );
    assert_eq!(
        FocusManager::get_key(&keyboard_with_key_down(KeyCode::ArrowDown)),
        Key::DownArrow
    );
}

#[test]
fn get_key_return() {
    assert_eq!(
        FocusManager::get_key(&keyboard_with_key_down(KeyCode::Enter)),
        Key::ReturnDown
    );
    assert_eq!(
        FocusManager::get_key(&keyboard_with_key_up(KeyCode::Enter)),
        Key::ReturnUp
    );
}

#[test]
fn get_key_none_when_no_key_pressed() {
    assert_eq!(FocusManager::get_key(&KeyboardState::default()), Key::None);
}

#[test]
fn get_key_with_typewriter_repeat() {
    let mut state = KeyboardState::default();
    state.keys.insert(
        KeyCode::ArrowDown,
        KeyInfo {
            press_state: KeyPressState::Down,
            has_changed: true,
            typewriter: TypewriterState::Repeat,
        },
    );
    assert_eq!(FocusManager::get_key(&state), Key::DownArrow);
}

#[test]
fn get_key_return_ignored_on_repeat() {
    let mut state = KeyboardState::default();
    state.keys.insert(
        KeyCode::Enter,
        KeyInfo {
            press_state: KeyPressState::Down,
            has_changed: true,
            typewriter: TypewriterState::Repeat,
        },
    );
    // Return is only recognized on first press (TypewriterState::None),
    // not on repeat.
    assert_eq!(FocusManager::get_key(&state), Key::None);
}

// ── Group navigation tests ──────────────────────────────────────

#[test]
fn navigate_down_focuses_first() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_groupable(Box::new(MockGroupable::new(2)), true);

    let events = fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, Some(0));
    assert!(
        events
            .iter()
            .any(|e| e.origin == 1 && e.msg_type == UiEventType::FocusChanged)
    );
}

#[test]
fn navigate_down_wraps_around() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_groupable(Box::new(MockGroupable::new(2)), true);

    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, Some(0));

    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, Some(1));

    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, Some(0));
}

#[test]
fn navigate_up_focuses_last() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_groupable(Box::new(MockGroupable::new(2)), true);

    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowUp), false);
    assert_eq!(fm.focused_groupable_idx, Some(1));
}

#[test]
fn navigate_up_wraps_around() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_groupable(Box::new(MockGroupable::new(2)), true);
    fm.add_groupable(Box::new(MockGroupable::new(3)), true);

    // First up → last widget (index 2)
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowUp), false);
    assert_eq!(fm.focused_groupable_idx, Some(2));

    // Second up → index 1
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowUp), false);
    assert_eq!(fm.focused_groupable_idx, Some(1));

    // Third up → index 0
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowUp), false);
    assert_eq!(fm.focused_groupable_idx, Some(0));

    // Fourth up → wraps to index 2
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowUp), false);
    assert_eq!(fm.focused_groupable_idx, Some(2));
}

#[test]
fn navigate_skips_disabled() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_groupable(Box::new(MockGroupable::new(2).disabled()), true);
    fm.add_groupable(Box::new(MockGroupable::new(3)), true);

    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, Some(0));

    // Skips disabled widget 2 → goes to widget 3
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, Some(2));
}

#[test]
fn navigate_skips_non_navigable() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_groupable(Box::new(MockGroupable::new(2)), false);
    fm.add_groupable(Box::new(MockGroupable::new(3)), true);

    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, Some(0));

    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, Some(2));
}

#[test]
fn all_disabled_no_focus_change() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1).disabled()), true);
    fm.add_groupable(Box::new(MockGroupable::new(2).disabled()), true);

    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, None);
}

// ── Enter/activate tests ────────────────────────────────────────

#[test]
fn enter_selects_then_activates() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);

    // Focus widget 1
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, Some(0));

    // Enter down → select
    let events = fm.process_input(vec![], &input_with_key_down(KeyCode::Enter), false);
    assert!(
        events
            .iter()
            .any(|e| e.msg_type == UiEventType::SelectionChanged)
    );
    assert_eq!(fm.focused_groupable_idx, Some(0));

    // Enter up → activate and clear focus
    let events = fm.process_input(vec![], &input_with_key_up(KeyCode::Enter), false);
    assert!(
        events
            .iter()
            .any(|e| e.msg_type == UiEventType::Activated && e.origin == 1)
    );
    assert_eq!(fm.focused_groupable_idx, None);
}

#[test]
fn enter_activation_deselects_radio_group_peers() {
    let rb2_deselected = Rc::new(Cell::new(false));
    let rb3_deselected = Rc::new(Cell::new(false));
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(
        Box::new(MockGroupable::new(1).radio_group(vec![1, 2, 3])),
        true,
    );
    fm.add_groupable(
        Box::new(MockGroupable::new(2).deselect_flag(rb2_deselected.clone())),
        true,
    );
    fm.add_groupable(
        Box::new(MockGroupable::new(3).deselect_flag(rb3_deselected.clone())),
        true,
    );

    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    let events = fm.process_input(vec![], &input_with_key_up(KeyCode::Enter), false);

    assert!(
        events
            .iter()
            .any(|e| e.msg_type == UiEventType::Activated && e.origin == 1)
    );
    assert!(rb2_deselected.get());
    assert!(rb3_deselected.get());
}

#[test]
fn enter_with_no_focus_does_nothing() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);

    let events = fm.process_input(vec![], &input_with_key_down(KeyCode::Enter), false);
    assert!(events.is_empty());
    assert_eq!(fm.focused_groupable_idx, None);
}

// ── Focusable navigation tests ──────────────────────────────────

#[test]
fn focusable_previous_enters_from_end() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_focusable(Box::new(MockFocusable::new(10)));
    fm.add_focusable(Box::new(MockFocusable::new(11)));

    // Left → focus last focusable (entering from end)
    let events = fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowLeft), false);
    assert_eq!(fm.focused_focusable_idx, Some(1));
    assert!(events.iter().any(|e| e.origin == 11));
}

#[test]
fn focusable_navigation_cycle() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_focusable(Box::new(MockFocusable::new(10)));
    fm.add_focusable(Box::new(MockFocusable::new(11)));

    // Left → focus last focusable (11)
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowLeft), false);
    assert_eq!(fm.focused_focusable_idx, Some(1));

    // Left again → focus previous focusable (10)
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowLeft), false);
    assert_eq!(fm.focused_focusable_idx, Some(0));

    // Right → focus next focusable (11)
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowRight), false);
    assert_eq!(fm.focused_focusable_idx, Some(1));

    // Right → past end, no focused focusable
    let events = fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowRight), false);
    assert_eq!(fm.focused_focusable_idx, None);
    assert!(events.iter().any(|e| e.origin == 11));
}

#[test]
fn focusable_blocks_groupable_navigation() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_groupable(Box::new(MockGroupable::new(2)), true);
    fm.add_focusable(Box::new(MockFocusable::new(10)));

    // Focus a focusable via Left
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowLeft), false);
    assert_eq!(fm.focused_focusable_idx, Some(0));

    // Down arrow should NOT move groupable focus while focusable is active
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, None);
}

// ── Horizontal orientation tests ────────────────────────────────

#[test]
fn horizontal_navigation() {
    let mut fm = FocusManager::new(GroupOrientation::Horizontal);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_groupable(Box::new(MockGroupable::new(2)), true);

    // Right → next groupable
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowRight), false);
    assert_eq!(fm.focused_groupable_idx, Some(0));

    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowRight), false);
    assert_eq!(fm.focused_groupable_idx, Some(1));

    // Left → previous groupable
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowLeft), false);
    assert_eq!(fm.focused_groupable_idx, Some(0));
}

// ── Shortcut tests ──────────────────────────────────────────────

#[test]
fn shortcut_focus_and_activate() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_shortcut(1, KeyCode::KeyA); // arbitrary shortcut key

    // Key down → focus + select
    let events = fm.process_input(vec![], &input_with_key_down(KeyCode::KeyA), false);
    assert_eq!(fm.focused_groupable_idx, Some(0));
    assert!(
        events
            .iter()
            .any(|e| e.msg_type == UiEventType::FocusChanged)
    );
    assert!(
        events
            .iter()
            .any(|e| e.msg_type == UiEventType::SelectionChanged)
    );

    // Key up → activate and clear focus
    let events = fm.process_input(vec![], &input_with_key_up(KeyCode::KeyA), false);
    assert!(events.iter().any(|e| e.msg_type == UiEventType::Activated));
    assert_eq!(fm.focused_groupable_idx, None);
}

#[test]
fn shortcut_ignored_when_widget_focused() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_groupable(Box::new(MockGroupable::new(2)), true);
    fm.add_shortcut(2, KeyCode::KeyA);

    // Focus widget 1 via arrow key
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, Some(0));

    // Shortcut should not fire when a widget is already focused
    fm.process_input(vec![], &input_with_key_down(KeyCode::KeyA), false);
    // Focus didn't change to widget 2
    assert_eq!(fm.focused_groupable_idx, Some(0));
}

#[test]
fn shortcuts_disabled() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_shortcut(1, KeyCode::KeyA);
    fm.set_shortcuts_enabled(false);

    fm.process_input(vec![], &input_with_key_down(KeyCode::KeyA), false);
    assert_eq!(fm.focused_groupable_idx, None);
}

// ── Reset tests ─────────────────────────────────────────────────

#[test]
fn reset_focused_widgets_clears_both() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_focusable(Box::new(MockFocusable::new(10)));

    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, Some(0));

    let events = fm.reset_focused_widgets();
    assert_eq!(fm.focused_groupable_idx, None);
    assert_eq!(fm.focused_focusable_idx, None);
    assert!(!events.is_empty());
}

// ── Mouse interaction tests ─────────────────────────────────────

#[test]
fn mouse_move_resets_keyboard_focus() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);

    // Focus via keyboard
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, Some(0));

    // Mouse moves, with a non-FrameFocus event present → resets focus
    let mouse_input = UiInput {
        mouse_position: ScreenPoint::new(100.0, 200.0),
        keyboard: KeyboardState::default(),
    };
    let existing = vec![UiEvent {
        msg_type: UiEventType::FocusChanged,
        origin: 999,
    }];
    fm.process_input(existing, &mouse_input, false);
    assert_eq!(fm.focused_groupable_idx, None);
}

#[test]
fn mouse_move_with_ignored_widget_no_reset() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.add_widget_to_ignore(999);

    // Focus via keyboard
    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, Some(0));

    // Mouse moves, but the only event is from an ignored widget
    let mouse_input = UiInput {
        mouse_position: ScreenPoint::new(100.0, 200.0),
        keyboard: KeyboardState::default(),
    };
    let existing = vec![UiEvent {
        msg_type: UiEventType::FocusChanged,
        origin: 999,
    }];
    fm.process_input(existing, &mouse_input, false);
    // Focus should NOT be reset because the event origin is ignored
    assert_eq!(fm.focused_groupable_idx, Some(0));
}

#[test]
fn mouse_captured_skips_processing() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);

    let events = fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), true);
    assert_eq!(fm.focused_groupable_idx, None);
    assert!(events.is_empty());
}

// ── Configuration tests ─────────────────────────────────────────

#[test]
fn navigation_disabled() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);
    fm.set_navigation_enabled(false);

    fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert_eq!(fm.focused_groupable_idx, None);
}

#[test]
fn empty_group_no_crash() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    let events = fm.process_input(vec![], &input_with_key_down(KeyCode::ArrowDown), false);
    assert!(events.is_empty());
    assert_eq!(fm.focused_groupable_idx, None);
}

// ── Event replacement tests ─────────────────────────────────────

#[test]
fn navigation_replaces_existing_events_from_same_origin() {
    let mut fm = FocusManager::new(GroupOrientation::Vertical);
    fm.add_groupable(Box::new(MockGroupable::new(1)), true);

    let existing = vec![
        UiEvent {
            msg_type: UiEventType::FocusChanged,
            origin: 1,
        },
        UiEvent {
            msg_type: UiEventType::FocusChanged,
            origin: 2,
        },
    ];

    let events = fm.process_input(existing, &input_with_key_down(KeyCode::ArrowDown), false);

    // Old event for origin 1 should be removed, event for origin 2 kept,
    // and new navigation events for origin 1 appended.
    assert!(events.iter().any(|e| e.origin == 2));
    assert!(
        events
            .iter()
            .any(|e| e.origin == 1 && e.msg_type == UiEventType::FocusChanged)
    );
}
