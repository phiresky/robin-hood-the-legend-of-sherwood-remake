//! Keyboard navigation for frame-owned menu buttons.

use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

use crate::ui::{KeyState as WidgetKeyState, TypeWriter};
use crate::widget::{FrameWnd, Widget, WidgetButton};

// ─── Public types ────────────────────────────────────────────────────

/// Orientation of the focus group — determines which arrow keys navigate
/// between groupable widgets vs. focusable widgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupOrientation {
    /// Up/Down navigate groupables, Left/Right navigate focusables.
    Vertical,
    /// Left/Right navigate groupables, Up/Down navigate focusables.
    Horizontal,
}

// ─── Frame-owned button focus ──────────────────────────────────────────────────────────

/// Focus navigation for buttons already owned by a [`FrameWnd`].
///
/// Stores widget IDs rather than parallel widget state, applying focus
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
