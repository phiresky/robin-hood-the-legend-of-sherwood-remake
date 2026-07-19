//! Focus manager for UI keyboard/gamepad navigation.
//!
//! Manages keyboard navigation between "groupable" widgets (buttons, menu
//! items) and "focusable" widgets (secondary items like input fields within
//! a row).
//!
//! Arrow keys in the group orientation navigate between groupable widgets;
//! perpendicular arrows navigate between focusable widgets.  Enter
//! selects/activates the focused groupable widget.  Keyboard shortcuts can
//! focus and activate groupable widgets directly.

use robin_engine::coordinates::ScreenPoint;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

use crate::ui::{KeyState as WidgetKeyState, TypeWriter};
use crate::widget::{FrameWnd, Widget, WidgetButton};

// ─── Public types ────────────────────────────────────────────────────

/// Opaque widget identifier.
pub type WidgetId = usize;

/// Orientation of the focus group — determines which arrow keys navigate
/// between groupable widgets vs. focusable widgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupOrientation {
    /// Up/Down navigate groupables, Left/Right navigate focusables.
    Vertical,
    /// Left/Right navigate groupables, Up/Down navigate focusables.
    Horizontal,
}

/// A UI event produced by focus changes, selections, or activations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiEvent {
    pub msg_type: UiEventType,
    pub origin: WidgetId,
}

/// Type of UI event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiEventType {
    /// Frame focus event (from the window system, not from navigation).
    FrameFocus,
    /// A widget gained or lost group focus.
    FocusChanged,
    /// A widget was selected or deselected (e.g. Enter held down).
    SelectionChanged,
    /// A widget was activated (e.g. Enter released on a focused button).
    Activated,
    /// A focusable widget was activated or deactivated.
    FocusableActiveChanged,
}

// ─── Frame-owned button focus ──────────────────────────────────────────────────────────

/// Focus navigation for buttons already owned by a [`FrameWnd`].
///
/// The older [`FocusManager`] owns boxed widget objects, which makes it
/// impossible to use with the live menu frames without cloning each button
/// into parallel state. This adapter stores only widget IDs and applies focus
/// transitions to the buttons in the frame itself. Both mouse input and
/// keyboard focus therefore resolve through the same canonical
/// [`crate::ui::UiEvent`] stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameButtonFocusManager {
    orientation: GroupOrientation,
    group: Vec<FrameButtonEntry>,
    shortcuts: Vec<(KeyCode, crate::widget::WidgetId)>,
    focused_idx: Option<usize>,
    pending_shortcut: Option<KeyCode>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct FrameButtonEntry {
    widget_id: crate::widget::WidgetId,
    navigable: bool,
}

impl FrameButtonFocusManager {
    pub fn new(orientation: GroupOrientation) -> Self {
        Self {
            orientation,
            group: Vec::new(),
            shortcuts: Vec::new(),
            focused_idx: None,
            pending_shortcut: None,
        }
    }

    /// Register a button already owned by `frame`.
    ///
    /// # Panics
    ///
    /// Panics for a missing/non-button widget or a duplicate ID. A focus
    /// registration that points nowhere is a construction error, not an
    /// inactive button.
    pub fn add_button(
        &mut self,
        frame: &FrameWnd,
        widget_id: crate::widget::WidgetId,
        navigable: bool,
    ) {
        assert!(
            matches!(frame.widget(widget_id), Some(Widget::Button(_))),
            "focus button {widget_id} is missing from its frame"
        );
        assert!(
            !self.group.iter().any(|entry| entry.widget_id == widget_id),
            "focus button {widget_id} is already registered"
        );
        self.group.push(FrameButtonEntry {
            widget_id,
            navigable,
        });
    }

    /// Bind a physical key to a registered button.
    ///
    /// Shortcut activation follows the original focus manager's two-edge
    /// behavior: key-down focuses/selects and key-up emits
    /// [`crate::ui::UiMsg::WidgetActivated`].
    pub fn add_shortcut(&mut self, widget_id: crate::widget::WidgetId, key: KeyCode) {
        assert!(
            self.group.iter().any(|entry| entry.widget_id == widget_id),
            "shortcut target {widget_id} is not registered"
        );
        self.shortcuts.retain(|(bound, _)| *bound != key);
        self.shortcuts.push((key, widget_id));
    }

    pub fn focused_button(&self) -> Option<crate::widget::WidgetId> {
        self.focused_idx.map(|idx| self.group[idx].widget_id)
    }

    /// Append keyboard focus events to the widget events produced by the
    /// frame for this input pass.
    pub fn process_input(
        &mut self,
        frame: &mut FrameWnd,
        mut events: Vec<crate::ui::UiEvent>,
        keyboard: &crate::ui::UiKeyboard,
        mouse_captured: bool,
    ) -> Vec<crate::ui::UiEvent> {
        if mouse_captured || !keyboard.has_changed() {
            return events;
        }

        let mut focus_events = self.process_navigation(frame, keyboard);
        if focus_events.is_empty() {
            focus_events = self.process_shortcuts(frame, keyboard);
        }
        if let Some(origin) = focus_events.first().map(|event| event.origin_widget_id) {
            events.retain(|event| event.origin_widget_id != origin);
        }
        events.extend(focus_events);
        events
    }

    fn process_navigation(
        &mut self,
        frame: &mut FrameWnd,
        keyboard: &crate::ui::UiKeyboard,
    ) -> Vec<crate::ui::UiEvent> {
        let (previous, next) = match self.orientation {
            GroupOrientation::Vertical => (KeyCode::ArrowUp, KeyCode::ArrowDown),
            GroupOrientation::Horizontal => (KeyCode::ArrowLeft, KeyCode::ArrowRight),
        };

        if key_repeats(keyboard, previous) {
            return self.move_focus(frame, false);
        }
        if key_repeats(keyboard, next) {
            return self.move_focus(frame, true);
        }
        if keyboard.get_state_of_key(KeyCode::Enter) == WidgetKeyState::KeyDown
            && keyboard.get_typewriter_state(KeyCode::Enter) == TypeWriter::None
            && let Some(idx) = self.focused_idx
        {
            return button_mut(frame, self.group[idx].widget_id).set_group_selected(true);
        }
        if key_released(keyboard, KeyCode::Enter)
            && let Some(idx) = self.focused_idx
        {
            return self.activate_focused(frame, idx);
        }
        Vec::new()
    }

    fn process_shortcuts(
        &mut self,
        frame: &mut FrameWnd,
        keyboard: &crate::ui::UiKeyboard,
    ) -> Vec<crate::ui::UiEvent> {
        for &(key, widget_id) in &self.shortcuts {
            if keyboard.get_state_of_key(key) == WidgetKeyState::KeyDown
                && keyboard.get_typewriter_state(key) == TypeWriter::None
                && self.focused_idx.is_none()
            {
                self.pending_shortcut = Some(key);
                let mut events = self.focus_button(frame, widget_id);
                events.extend(button_mut(frame, widget_id).set_group_selected(true));
                return events;
            }
        }

        if let Some(key) = self.pending_shortcut
            && key_released(keyboard, key)
        {
            self.pending_shortcut = None;
            let widget_id = self
                .shortcuts
                .iter()
                .find_map(|&(bound, id)| (bound == key).then_some(id))
                .expect("pending shortcut lost its registered button");
            let idx = self
                .focused_idx
                .filter(|&idx| self.group[idx].widget_id == widget_id)
                .expect("pending shortcut lost focus before key release");
            return self.activate_focused(frame, idx);
        }

        Vec::new()
    }

    fn move_focus(&mut self, frame: &mut FrameWnd, forward: bool) -> Vec<crate::ui::UiEvent> {
        let len = self.group.len();
        if len == 0 || !self.group.iter().any(|entry| entry.navigable) {
            return Vec::new();
        }

        let previous_idx = self.focused_idx;
        let mut events = self.clear_focus(frame);
        let start = match previous_idx {
            Some(idx) if forward => (idx + 1) % len,
            Some(0) if !forward => len - 1,
            Some(idx) => idx - 1,
            None if forward => 0,
            None => len - 1,
        };
        let mut idx = start;
        for _ in 0..len {
            let entry = self.group[idx];
            if entry.navigable && button(frame, entry.widget_id).base.enabled {
                self.focused_idx = Some(idx);
                let target = button_mut(frame, entry.widget_id);
                target.hide_focus(false);
                events.extend(target.set_group_focused(true));
                return events;
            }
            idx = if forward {
                (idx + 1) % len
            } else if idx == 0 {
                len - 1
            } else {
                idx - 1
            };
        }
        events
    }

    fn focus_button(
        &mut self,
        frame: &mut FrameWnd,
        widget_id: crate::widget::WidgetId,
    ) -> Vec<crate::ui::UiEvent> {
        let mut events = self.clear_focus(frame);
        let idx = self
            .group
            .iter()
            .position(|entry| entry.widget_id == widget_id)
            .expect("focus target is not registered");
        self.focused_idx = Some(idx);
        let target = button_mut(frame, widget_id);
        target.hide_focus(false);
        events.extend(target.set_group_focused(true));
        events
    }

    fn clear_focus(&mut self, frame: &mut FrameWnd) -> Vec<crate::ui::UiEvent> {
        let Some(idx) = self.focused_idx.take() else {
            return Vec::new();
        };
        let target = button_mut(frame, self.group[idx].widget_id);
        let mut events = target.set_group_focused(false);
        events.extend(target.set_group_selected(false));
        events
    }

    fn activate_focused(&mut self, frame: &mut FrameWnd, idx: usize) -> Vec<crate::ui::UiEvent> {
        let widget_id = self.group[idx].widget_id;
        self.pending_shortcut = None;
        let mut events = self.clear_focus(frame);
        events.extend(button_mut(frame, widget_id).activate());
        events
    }
}

fn key_repeats(keyboard: &crate::ui::UiKeyboard, key: KeyCode) -> bool {
    keyboard.get_state_of_key(key) == WidgetKeyState::KeyDown
        && matches!(
            keyboard.get_typewriter_state(key),
            TypeWriter::None | TypeWriter::Repeat
        )
}

fn key_released(keyboard: &crate::ui::UiKeyboard, key: KeyCode) -> bool {
    matches!(
        keyboard.get_state_of_key(key),
        WidgetKeyState::KeyPressed | WidgetKeyState::KeyDouble
    ) && keyboard.has_key_changed(key)
}

fn button(frame: &FrameWnd, widget_id: crate::widget::WidgetId) -> &WidgetButton {
    match frame.widget(widget_id) {
        Some(Widget::Button(button)) => button,
        Some(_) => panic!("focus target {widget_id} is not a button"),
        None => panic!("focus target {widget_id} is missing from its frame"),
    }
}

fn button_mut(frame: &mut FrameWnd, widget_id: crate::widget::WidgetId) -> &mut WidgetButton {
    match frame.widget_mut(widget_id) {
        Some(Widget::Button(button)) => button,
        Some(_) => panic!("focus target {widget_id} is not a button"),
        None => panic!("focus target {widget_id} is missing from its frame"),
    }
}

// ─── Keyboard input types ────────────────────────────────────────────

/// Per-key press state for a single frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum KeyPressState {
    /// Key state did not change this frame.
    #[default]
    Unchanged,
    /// Key was pressed down this frame.
    Down,
    /// Key was released this frame.
    Up,
}

/// Typewriter (auto-repeat) state for a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TypewriterState {
    /// No typewriter event (first press or not repeating).
    #[default]
    None,
    /// Key is auto-repeating.
    Repeat,
    /// Some other typewriter state (key held but not yet repeating, etc.).
    Other,
}

/// State of a single keyboard key for one frame.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct KeyInfo {
    pub press_state: KeyPressState,
    pub has_changed: bool,
    pub typewriter: TypewriterState,
}

/// Full keyboard state for one frame.
#[derive(Debug, Clone, Default)]
pub struct KeyboardState {
    pub keys: HashMap<KeyCode, KeyInfo>,
    pub has_changed: bool,
}

/// Combined UI input state for one frame.
#[derive(Debug, Clone)]
pub struct UiInput {
    pub mouse_position: ScreenPoint,
    pub keyboard: KeyboardState,
}

// ─── Widget traits ───────────────────────────────────────────────────

/// Interface for widgets that participate in group focus navigation
/// (buttons, menu items, etc.).
pub trait WidgetGroupable {
    /// Unique identifier for this widget.
    fn widget_id(&self) -> WidgetId;
    /// Whether the widget is enabled (can receive focus).
    fn is_enabled(&self) -> bool;
    /// Whether the widget is sleeping (inactive).
    fn is_sleeping(&self) -> bool;
    /// Whether the given point is inside this widget's bounds.
    fn is_mouse_inside(&self, point: ScreenPoint) -> bool;
    /// Show or hide the focus indicator.
    fn hide_focus(&mut self, hide: bool);
    /// Set group-focus state; returns resulting UI events.
    fn set_group_focused(&mut self, focused: bool) -> Vec<UiEvent>;
    /// Set group-selected state (e.g. Enter held); returns resulting UI events.
    fn set_group_selected(&mut self, selected: bool) -> Vec<UiEvent>;
    /// Activate the widget (e.g. Enter released); returns resulting UI events.
    fn activate(&mut self) -> Vec<UiEvent>;
    /// IDs of mutually exclusive group peers. Radio buttons override this so
    /// focus-manager activation can mirror the frame-window group walk.
    fn group_members(&self) -> Vec<WidgetId> {
        Vec::new()
    }
    /// Deselect this widget because another member of its group became active.
    fn set_active_other(&mut self) {}
}

/// Interface for widgets in the secondary focusable chain
/// (e.g. sub-items within a row).
pub trait WidgetFocusable {
    /// Unique identifier for this widget.
    fn widget_id(&self) -> WidgetId;
    /// Set the active/focused state; returns resulting UI events.
    fn set_focusable_active(&mut self, active: bool) -> Vec<UiEvent>;
    /// Whether the focus manager should suppress its own navigation and
    /// shortcut handling while this widget is active.
    ///
    /// Used by input-field widgets that want to capture all keyboard
    /// input while the user is typing — they return `true` here so
    /// arrow keys and shortcut bindings don't fight with their own
    /// edit-mode handling. Default is `false` so most focusable widgets
    /// keep navigation responsive.
    fn suppresses_navigation_while_active(&self) -> bool {
        false
    }
}

// ─── Internal types ──────────────────────────────────────────────────

/// Navigation key classification extracted from keyboard state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    None,
    LeftArrow,
    UpArrow,
    RightArrow,
    DownArrow,
    ReturnDown,
    ReturnUp,
}

/// Navigation direction indices into `navigation_keys`.
const GROUPABLE_PREVIOUS: usize = 0;
const GROUPABLE_NEXT: usize = 1;
const FOCUSABLE_PREVIOUS: usize = 2;
const FOCUSABLE_NEXT: usize = 3;

/// A groupable widget entry with its navigability flag.
struct GroupableEntry {
    widget: Box<dyn WidgetGroupable>,
    navigable: bool,
}

// ─── FocusManager ────────────────────────────────────────────────────

/// Manages keyboard/gamepad focus navigation between UI widgets.
///
/// Widgets are organized in two dimensions:
/// - **Groupable** widgets form the main navigation chain (e.g. a vertical
///   list of buttons). Arrow keys in the group orientation move between them.
/// - **Focusable** widgets form a secondary chain (e.g. sub-items within a
///   row). Arrow keys perpendicular to the group orientation move between them.
///
/// Focusable navigation blocks groupable navigation — you must navigate past
/// all focusable widgets before groupable arrow keys work again.
pub struct FocusManager {
    /// Maps `[GroupablePrevious, GroupableNext, FocusablePrevious, FocusableNext]`
    /// to the corresponding [`Key`] based on group orientation.
    navigation_keys: [Key; 4],
    group: Vec<GroupableEntry>,
    focusable_widgets: Vec<Box<dyn WidgetFocusable>>,
    ignored_widget_ids: Vec<WidgetId>,
    focused_groupable_idx: Option<usize>,
    focused_focusable_idx: Option<usize>,
    navigation_enabled: bool,
    shortcuts_enabled: bool,
    /// Maps physical key → widget ID for keyboard shortcuts.
    shortcuts: HashMap<KeyCode, WidgetId>,
    /// Physical keys currently held down for shortcut activation.
    pending_shortcuts: Vec<KeyCode>,
    old_mouse_pos: ScreenPoint,
}

impl FocusManager {
    // ── Construction ─────────────────────────────────────────────────

    pub fn new(orientation: GroupOrientation) -> Self {
        let navigation_keys = match orientation {
            GroupOrientation::Vertical => [
                Key::UpArrow,    // GroupablePrevious
                Key::DownArrow,  // GroupableNext
                Key::LeftArrow,  // FocusablePrevious
                Key::RightArrow, // FocusableNext
            ],
            GroupOrientation::Horizontal => [
                Key::LeftArrow,  // GroupablePrevious
                Key::RightArrow, // GroupableNext
                Key::UpArrow,    // FocusablePrevious
                Key::DownArrow,  // FocusableNext
            ],
        };

        Self {
            navigation_keys,
            group: Vec::new(),
            focusable_widgets: Vec::new(),
            ignored_widget_ids: Vec::new(),
            focused_groupable_idx: None,
            focused_focusable_idx: None,
            navigation_enabled: true,
            shortcuts_enabled: true,
            shortcuts: HashMap::new(),
            pending_shortcuts: Vec::new(),
            old_mouse_pos: ScreenPoint::new(-1.0, -1.0),
        }
    }

    // ── Widget registration ─────────────────────────────────────────

    /// Add a focusable widget to the focus chain.
    ///
    /// # Panics
    /// Panics if a widget with the same ID is already registered.
    pub fn add_focusable(&mut self, widget: Box<dyn WidgetFocusable>) {
        // The manager toggles nav/shortcuts on enter-/exit-edit by
        // checking `WidgetFocusable::suppresses_navigation_while_active()`
        // whenever it activates/deactivates a focusable — see
        // `set_focusable_active_with_suppression` below.
        assert!(
            !self.has_focusable_widget(widget.widget_id()),
            "focusable widget {} already registered",
            widget.widget_id()
        );
        self.focusable_widgets.push(widget);
    }

    /// Add a groupable widget to the focus group.
    ///
    /// When `navigable` is `false`, the widget can be focused via shortcuts
    /// or mouse but will be skipped by arrow-key navigation.
    ///
    /// # Panics
    /// Panics if a widget with the same ID is already registered.
    pub fn add_groupable(&mut self, widget: Box<dyn WidgetGroupable>, navigable: bool) {
        assert!(
            !self.has_groupable_widget(widget.widget_id()),
            "groupable widget {} already registered",
            widget.widget_id()
        );
        self.group.push(GroupableEntry { widget, navigable });
    }

    /// Register a keyboard shortcut that focuses and activates a groupable widget.
    ///
    /// The widget must already be registered via [`add_groupable`](Self::add_groupable).
    /// On key-down the widget is focused and selected; on key-up it is activated.
    pub fn add_shortcut(&mut self, widget_id: WidgetId, key: KeyCode) {
        self.shortcuts.insert(key, widget_id);
    }

    /// Mark a widget ID as ignored — its events won't trigger mouse-based
    /// focus resets.
    pub fn add_widget_to_ignore(&mut self, widget_id: WidgetId) {
        self.ignored_widget_ids.push(widget_id);
    }

    // ── Configuration ───────────────────────────────────────────────

    pub fn set_navigation_enabled(&mut self, enabled: bool) {
        self.navigation_enabled = enabled;
    }

    pub fn set_shortcuts_enabled(&mut self, enabled: bool) {
        self.shortcuts_enabled = enabled;
    }

    // ── Queries ─────────────────────────────────────────────────────

    /// Access a groupable widget by index.
    pub fn groupable(&self, index: usize) -> &dyn WidgetGroupable {
        &*self.group[index].widget
    }

    /// Access a focusable widget by index.
    pub fn focusable(&self, index: usize) -> &dyn WidgetFocusable {
        &*self.focusable_widgets[index]
    }

    // ── Main input processing ───────────────────────────────────────

    /// Process input for this frame.
    ///
    /// Takes existing UI events (from the window system), the current input
    /// state, and whether the mouse is captured by the UI.  Returns the
    /// updated event list with focus navigation events appended or replacing
    /// existing events as appropriate.
    pub fn process_input(
        &mut self,
        mut events: Vec<UiEvent>,
        input: &UiInput,
        mouse_captured: bool,
    ) -> Vec<UiEvent> {
        if mouse_captured {
            return events;
        }

        // If the mouse moved and there's a non-FrameFocus, non-ignored event,
        // reset keyboard focus (mouse takes priority).
        if self.old_mouse_pos != input.mouse_position {
            let should_reset = events.iter().any(|e| {
                e.msg_type != UiEventType::FrameFocus && !self.is_widget_to_ignore(e.origin)
            });
            if should_reset {
                // Return value intentionally discarded.
                let _ = self.reset_focused_groupable_widget();
                self.old_mouse_pos = input.mouse_position;
            }
        }

        if input.keyboard.has_changed {
            let nav_events = self.process_input_for_navigation(input);

            if !nav_events.is_empty() {
                // Replace existing events from the same widget that navigation
                // just produced events for.
                let nav_origin = nav_events[0].origin;
                events.retain(|e| e.origin != nav_origin);
                events.extend(nav_events);
            } else {
                // No navigation happened — try shortcuts.
                events.extend(self.process_input_for_shortcuts(input));
            }
        }

        events
    }

    /// Reset focus state for both groupable and focusable widgets.
    pub fn reset_focused_widgets(&mut self) -> Vec<UiEvent> {
        let mut events = self.reset_focused_focusable_widget();
        events.extend(self.reset_focused_groupable_widget());
        events
    }

    // ── Private: key extraction ─────────────────────────────────────

    /// Extract the first navigation-relevant key from the keyboard state.
    ///
    /// Returns [`Key::None`] if no relevant key is pressed/released.
    fn get_key(keyboard: &KeyboardState) -> Key {
        for (key, info) in &keyboard.keys {
            if info.press_state == KeyPressState::Down
                && (info.typewriter == TypewriterState::Repeat
                    || info.typewriter == TypewriterState::None)
            {
                match key {
                    KeyCode::ArrowLeft => return Key::LeftArrow,
                    KeyCode::ArrowUp => return Key::UpArrow,
                    KeyCode::ArrowRight => return Key::RightArrow,
                    KeyCode::ArrowDown => return Key::DownArrow,
                    KeyCode::Enter if info.typewriter == TypewriterState::None => {
                        return Key::ReturnDown;
                    }
                    _ => {}
                }
            } else if info.press_state == KeyPressState::Up
                && info.has_changed
                && *key == KeyCode::Enter
            {
                return Key::ReturnUp;
            }
        }
        Key::None
    }

    // ── Private: queries ────────────────────────────────────────────

    fn has_focusable_widget(&self, widget_id: WidgetId) -> bool {
        self.focusable_widgets
            .iter()
            .any(|f| f.widget_id() == widget_id)
    }

    fn has_groupable_widget(&self, widget_id: WidgetId) -> bool {
        self.group.iter().any(|g| g.widget.widget_id() == widget_id)
    }

    fn is_widget_to_ignore(&self, widget_id: WidgetId) -> bool {
        self.ignored_widget_ids.contains(&widget_id)
    }

    fn hide_groupable_focus(&mut self, hide: bool) {
        for entry in &mut self.group {
            entry.widget.hide_focus(hide);
        }
    }

    fn apply_group_activation(&mut self, idx: usize) -> Vec<UiEvent> {
        let events = self.group[idx].widget.activate();
        let activated = events.iter().any(|e| e.msg_type == UiEventType::Activated);
        if activated {
            let active_id = self.group[idx].widget.widget_id();
            let group_members = self.group[idx].widget.group_members();
            for member_id in group_members {
                if member_id == active_id {
                    continue;
                }
                if let Some(peer) = self
                    .group
                    .iter_mut()
                    .find(|entry| entry.widget.widget_id() == member_id)
                {
                    peer.widget.set_active_other();
                }
            }
        }
        events
    }

    // ── Private: group focus movement ───────────────────────────────

    /// Move focus to the next groupable widget (wrapping around).
    fn move_group_focus_next(&mut self) -> Vec<UiEvent> {
        let mut events = Vec::new();
        let len = self.group.len();
        if len == 0 {
            return events;
        }

        // Determine start index and unfocus current widget.
        let start = match self.focused_groupable_idx {
            None => 0,
            Some(idx) => {
                self.group[idx].widget.hide_focus(true);
                events.extend(self.group[idx].widget.set_group_focused(false));
                events.extend(self.group[idx].widget.set_group_selected(false));
                (idx + 1) % len
            }
        };

        // Search forward (wrapping) for an enabled, navigable widget.
        let mut candidate = start;
        for _ in 0..len {
            if self.group[candidate].widget.is_enabled() && self.group[candidate].navigable {
                self.focused_groupable_idx = Some(candidate);
                self.group[candidate].widget.hide_focus(false);
                events.extend(self.group[candidate].widget.set_group_focused(true));
                return events;
            }
            candidate = (candidate + 1) % len;
        }

        events
    }

    /// Move focus to the previous groupable widget (wrapping around).
    fn move_group_focus_previous(&mut self) -> Vec<UiEvent> {
        let mut events = Vec::new();
        let len = self.group.len();
        if len == 0 {
            return events;
        }

        // Determine start index.
        let start = match self.focused_groupable_idx {
            None | Some(0) => len - 1,
            Some(idx) => idx - 1,
        };

        // Unfocus current widget if any.
        if let Some(idx) = self.focused_groupable_idx {
            self.group[idx].widget.hide_focus(true);
            events.extend(self.group[idx].widget.set_group_focused(false));
            events.extend(self.group[idx].widget.set_group_selected(false));
        }

        // Search backward (wrapping) for an enabled, navigable widget.
        let mut candidate = start;
        for _ in 0..len {
            if self.group[candidate].widget.is_enabled() && self.group[candidate].navigable {
                self.focused_groupable_idx = Some(candidate);
                self.group[candidate].widget.hide_focus(false);
                events.extend(self.group[candidate].widget.set_group_focused(true));
                return events;
            }
            if candidate == 0 {
                candidate = len - 1;
            } else {
                candidate -= 1;
            }
        }

        events
    }

    /// Activate or deactivate a focusable widget while honouring its
    /// `suppresses_navigation_while_active` preference.
    fn set_focusable_active_with_suppression(&mut self, idx: usize, active: bool) -> Vec<UiEvent> {
        let suppresses = self.focusable_widgets[idx].suppresses_navigation_while_active();
        let events = self.focusable_widgets[idx].set_focusable_active(active);
        if suppresses {
            // Nav/shortcuts off while editing, back on when leaving edit mode.
            let enabled = !active;
            self.navigation_enabled = enabled;
            self.shortcuts_enabled = enabled;
        }
        events
    }

    // ── Private: focusable focus movement ───────────────────────────

    /// Move focus to the next focusable widget. When the end is reached,
    /// focus is cleared (moves past the last widget).
    fn move_focusable_focus_next(&mut self) -> Vec<UiEvent> {
        let mut events = Vec::new();

        if self.focusable_widgets.is_empty() {
            return events;
        }

        if let Some(idx) = self.focused_focusable_idx {
            events.extend(self.set_focusable_active_with_suppression(idx, false));
            let next = idx + 1;
            if next < self.focusable_widgets.len() {
                self.focused_focusable_idx = Some(next);
                events.extend(self.set_focusable_active_with_suppression(next, true));
            } else {
                self.focused_focusable_idx = None;
            }
        }

        events
    }

    /// Move focus to the previous focusable widget. When called with no
    /// focused focusable, resets groupable focus and focuses the last
    /// focusable widget.
    fn move_focusable_focus_previous(&mut self) -> Vec<UiEvent> {
        let mut events = Vec::new();

        if self.focusable_widgets.is_empty() {
            return events;
        }

        match self.focused_focusable_idx {
            None => {
                // No focused focusable — enter the focusable chain from the end.
                events.extend(self.reset_focused_groupable_widget());
                let last = self.focusable_widgets.len() - 1;
                self.focused_focusable_idx = Some(last);
                events.extend(self.set_focusable_active_with_suppression(last, true));
            }
            Some(idx) if idx > 0 => {
                events.extend(self.set_focusable_active_with_suppression(idx, false));
                let prev = idx - 1;
                self.focused_focusable_idx = Some(prev);
                events.extend(self.set_focusable_active_with_suppression(prev, true));
            }
            Some(_) => {
                // Already at the first focusable — do nothing.
            }
        }

        events
    }

    // ── Private: focus management ───────────────────────────────────

    /// Focus a specific groupable widget by ID, unfocusing any currently
    /// focused widget first.
    fn focus_groupable_by_id(&mut self, widget_id: WidgetId) -> Vec<UiEvent> {
        let mut events = Vec::new();

        // Unfocus current widget if any.
        if let Some(idx) = self.focused_groupable_idx {
            self.group[idx].widget.hide_focus(true);
            events.extend(self.group[idx].widget.set_group_focused(false));
            events.extend(self.group[idx].widget.set_group_selected(false));
        }

        // Find and focus the target widget.
        let target_idx = self
            .group
            .iter()
            .position(|g| g.widget.widget_id() == widget_id);

        if let Some(idx) = target_idx {
            self.focused_groupable_idx = Some(idx);
            self.group[idx].widget.hide_focus(false);
            events.extend(self.group[idx].widget.set_group_focused(true));
        }

        events
    }

    fn reset_focused_groupable_widget(&mut self) -> Vec<UiEvent> {
        let mut events = Vec::new();

        // Reset focus-hidden state on all groupable widgets.
        self.hide_groupable_focus(false);

        if let Some(idx) = self.focused_groupable_idx {
            events.extend(self.group[idx].widget.set_group_focused(false));
            events.extend(self.group[idx].widget.set_group_selected(false));
            self.focused_groupable_idx = None;
        }

        self.pending_shortcuts.clear();
        events
    }

    fn reset_focused_focusable_widget(&mut self) -> Vec<UiEvent> {
        let mut events = Vec::new();

        if let Some(idx) = self.focused_focusable_idx {
            events.extend(self.set_focusable_active_with_suppression(idx, false));
            self.focused_focusable_idx = None;
        }

        events
    }

    /// If no groupable is currently focused and the mouse is over a
    /// groupable widget, focus that widget.
    fn synchronize_groupable_with_mouse(&mut self, mouse_pos: ScreenPoint) {
        if self.focused_groupable_idx.is_some() {
            return;
        }

        let target_id = self
            .group
            .iter()
            .find(|g| g.widget.is_mouse_inside(mouse_pos))
            .map(|g| g.widget.widget_id());

        if let Some(widget_id) = target_id {
            let _ = self.reset_focused_groupable_widget();
            let _ = self.focus_groupable_by_id(widget_id);
        }
    }

    // ── Private: input processing ───────────────────────────────────

    fn process_input_for_navigation(&mut self, input: &UiInput) -> Vec<UiEvent> {
        // Navigation (including focusable navigation) requires at least
        // one groupable widget — the empty-group early-out gates both.
        if !self.navigation_enabled || self.group.is_empty() {
            return Vec::new();
        }

        let key = Self::get_key(&input.keyboard);
        if key == Key::None {
            return Vec::new();
        }

        if key == self.navigation_keys[GROUPABLE_NEXT] {
            // Move to next groupable, but only if no focusable is focused.
            if self.focused_focusable_idx.is_none() {
                self.synchronize_groupable_with_mouse(input.mouse_position);
                return self.move_group_focus_next();
            }
        } else if key == self.navigation_keys[GROUPABLE_PREVIOUS] {
            if self.focused_focusable_idx.is_none() {
                self.synchronize_groupable_with_mouse(input.mouse_position);
                return self.move_group_focus_previous();
            }
        } else if key == self.navigation_keys[FOCUSABLE_PREVIOUS] {
            return self.move_focusable_focus_previous();
        } else if key == self.navigation_keys[FOCUSABLE_NEXT] {
            return self.move_focusable_focus_next();
        } else if key == Key::ReturnUp {
            // Enter released — activate the focused widget.
            if let Some(idx) = self.focused_groupable_idx {
                let mut events = Vec::new();
                events.extend(self.group[idx].widget.set_group_focused(false));
                events.extend(self.group[idx].widget.set_group_selected(false));
                events.extend(self.apply_group_activation(idx));
                self.focused_groupable_idx = None;
                return events;
            }
        } else if key == Key::ReturnDown {
            // Enter pressed — select (highlight) the focused widget.
            if let Some(idx) = self.focused_groupable_idx {
                return self.group[idx].widget.set_group_selected(true);
            }
        }

        Vec::new()
    }

    fn process_input_for_shortcuts(&mut self, input: &UiInput) -> Vec<UiEvent> {
        let mut events = Vec::new();

        if !self.shortcuts_enabled {
            return events;
        }

        for (key, info) in &input.keyboard.keys {
            // Key down: focus the shortcut's widget and select it.
            if info.press_state == KeyPressState::Down
                && info.typewriter == TypewriterState::None
                && self.focused_groupable_idx.is_none()
                && self.focused_focusable_idx.is_none()
            {
                if let Some(&widget_id) = self.shortcuts.get(key) {
                    self.pending_shortcuts.push(*key);
                    events.extend(self.focus_groupable_by_id(widget_id));
                    if let Some(idx) = self.focused_groupable_idx {
                        events.extend(self.group[idx].widget.set_group_selected(true));
                    }
                    break;
                }
            }
            // Key up: release pending shortcut and activate the widget.
            else if info.press_state == KeyPressState::Up
                && info.has_changed
                && let Some(pos) = self.pending_shortcuts.iter().position(|&s| s == *key)
            {
                self.pending_shortcuts.remove(pos);

                if let Some(&widget_id) = self.shortcuts.get(key)
                    && let Some(idx) = self.focused_groupable_idx
                    && self.group[idx].widget.widget_id() == widget_id
                {
                    events.extend(self.group[idx].widget.set_group_focused(false));
                    events.extend(self.group[idx].widget.set_group_selected(false));
                    events.extend(self.apply_group_activation(idx));
                    self.focused_groupable_idx = None;
                }
            }
        }

        events
    }
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "focus_manager_tests.rs"]
mod tests;
