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
mod tests {
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
}
