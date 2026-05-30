//! Text input field widget with edit mode.
//!
//! Folds together a generic input-field state machine with the menu-
//! specific extras the game needs: noisy-event mapping (focus/activation
//! sounds via [`WidgetInputField::play_noise`]), EditField font lookup
//! on creation, and [`WidgetFocusable`] integration so Tab navigation
//! can enter edit mode. The font is supplied by the caller via
//! `resources.edit_field_font()`.
//!
//! State machine:
//! ```text
//! DEFAULT ──mouse over──► FOCUSED ──left down──► CLICKED
//!    ▲                                              │
//!    │                                     left click
//!    │                                              ▼
//!    └──escape──── SELECTED_EDITABLE ◄──dbl click── SELECTED
//!                        │
//!                   return/tab
//!                        ▼
//!                  emit TextChanged + Activated
//! ```

use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

use crate::ui::{
    KeyState, MouseButtons, ProbeCode, TypeWriter, UiEvent, UiMsg, UiProbe, UiState,
    resource_widget_id::{
        INPUT_FIELD_CLICKED, INPUT_FIELD_DEFAULT, INPUT_FIELD_DISABLED, INPUT_FIELD_FOCUSED,
        INPUT_FIELD_SELECTED,
    },
};

use super::{WidgetBase, WidgetInput};

/// Text input field widget.
///
/// Implements the input-field state machine including caret tracking
/// and keyboard editing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetInputField {
    pub base: WidgetBase,

    /// The text being edited (may differ from base.text during editing).
    pub edit_text: String,
    /// Saved text before editing (for cancel on Escape).
    pub saved_text: String,
    /// Caret position (character index into `edit_text`).
    pub caret_offset: usize,
    /// Whether the caret is currently visible (blink state).
    pub caret_visible: bool,
    /// Maximum allowed text length (0 = unlimited).
    pub max_length: usize,

    // Double-buffered state for probe_refresh.
    old_caret_offset: [usize; 2],
    old_caret_visible: [bool; 2],
    old_text_len: [usize; 2],

    /// ID of the next linked input field (for Tab navigation).
    /// `WIDGET_ID_NONE` if no linked field.
    pub linked_field: super::WidgetId,
}

impl Default for WidgetInputField {
    fn default() -> Self {
        Self {
            base: WidgetBase::default(),
            edit_text: String::new(),
            saved_text: String::new(),
            caret_offset: 0,
            caret_visible: false,
            max_length: 0,
            old_caret_offset: [0; 2],
            old_caret_visible: [false; 2],
            old_text_len: [0; 2],
            linked_field: super::WIDGET_ID_NONE,
        }
    }
}

impl WidgetInputField {
    pub fn new(id: super::WidgetId) -> Self {
        Self {
            base: WidgetBase {
                id,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Map state to renderer sub-resource ID.
    pub fn transform_state_into_id(&self) -> u8 {
        if !self.base.enabled {
            return INPUT_FIELD_DISABLED;
        }
        match self.base.state {
            UiState::Default => INPUT_FIELD_DEFAULT,
            UiState::Focused => INPUT_FIELD_FOCUSED,
            UiState::Clicked => INPUT_FIELD_CLICKED,
            UiState::Selected => INPUT_FIELD_SELECTED,
            UiState::SelectedEditable => INPUT_FIELD_SELECTED,
            _ => INPUT_FIELD_DEFAULT,
        }
    }

    /// Probe whether a refresh is needed.
    ///
    /// Tracks caret position, visibility, and text length changes
    /// across double-buffered frames.
    pub fn probe_refresh(&mut self, counter: u32) -> Option<UiProbe> {
        self.base.renderer.set_counter(counter);
        let idx = (counter % 2) as usize;

        let caret_changed = self.old_caret_offset[idx] != self.caret_offset
            || self.old_caret_visible[idx] != self.caret_visible;
        let text_changed = self.old_text_len[idx] != self.edit_text.len();

        self.old_caret_offset[idx] = self.caret_offset;
        self.old_caret_visible[idx] = self.caret_visible;
        self.old_text_len[idx] = self.edit_text.len();

        if caret_changed {
            return Some(self.base.make_probe(ProbeCode::LazyRefresh));
        }

        if text_changed {
            return Some(self.base.make_probe(ProbeCode::FullRefresh));
        }

        // Check renderer state change (same as default widget probe).
        let sub_res = self.transform_state_into_id();
        let will_render = self
            .base
            .renderer
            .base()
            .map_or(u32::MAX, |b| b.will_be_rendered(sub_res));
        let last = self
            .base
            .renderer
            .base()
            .map_or(u32::MAX, |b| b.last_rendered());
        if will_render != last {
            Some(self.base.make_probe(ProbeCode::FullRefresh))
        } else {
            None
        }
    }

    /// Process input for one frame.
    pub fn process_input(&mut self, input: &WidgetInput) -> Vec<UiEvent> {
        if !self.base.enabled {
            return self.base.tooltip_event_if_disabled().into_iter().collect();
        }

        match self.base.state {
            UiState::Default => self.process_input_default(input),
            UiState::Focused => self.process_input_focused(input),
            UiState::Clicked => self.process_input_clicked(input),
            UiState::Selected => self.process_input_selected(input),
            UiState::SelectedEditable => self.process_input_editable(input),
            _ => Vec::new(),
        }
    }

    // ── State-specific input handlers ──────────────────────────────

    /// DEFAULT state: check if mouse enters the widget.
    fn process_input_default(&mut self, input: &WidgetInput) -> Vec<UiEvent> {
        let inside = self.base.is_inside(input.mouse_position);
        if inside && self.base.with_focus {
            self.base.state = UiState::Focused;
            vec![self.base.make_event(UiMsg::WidgetFocused)]
        } else {
            Vec::new()
        }
    }

    /// FOCUSED state: check for mouse down to enter clicked state.
    fn process_input_focused(&mut self, input: &WidgetInput) -> Vec<UiEvent> {
        let inside = self.base.is_inside(input.mouse_position);
        if !inside {
            self.base.state = UiState::Default;
            return vec![self.base.make_event(UiMsg::WidgetUnfocused)];
        }
        if input.mouse_button.contains(MouseButtons::LEFT_DOWN) {
            self.base.state = UiState::Clicked;
        }
        Vec::new()
    }

    /// CLICKED state: wait for click release to enter selected state.
    fn process_input_clicked(&mut self, input: &WidgetInput) -> Vec<UiEvent> {
        let inside = self.base.is_inside(input.mouse_position);
        if input.mouse_button.contains(MouseButtons::LEFT_CLICK) {
            if inside {
                self.base.state = UiState::Selected;
                return vec![self.base.make_event(UiMsg::WidgetActivated)];
            } else {
                self.base.state = UiState::Default;
                return vec![self.base.make_event(UiMsg::WidgetUnfocused)];
            }
        }
        // Escape cancels.
        if input.keyboard.get_state_of_key(KeyCode::Escape) == KeyState::KeyPressed {
            self.base.state = UiState::Default;
            return vec![self.base.make_event(UiMsg::WidgetUnfocused)];
        }
        Vec::new()
    }

    /// SELECTED state: check for double-click to enter edit mode.
    fn process_input_selected(&mut self, input: &WidgetInput) -> Vec<UiEvent> {
        let inside = self.base.is_inside(input.mouse_position);

        if input.mouse_button.contains(MouseButtons::LEFT_DOUBLE_CLICK) && inside {
            self.enter_edit_mode();
            return vec![self.base.make_event(UiMsg::WidgetEditMode)];
        }

        if input.mouse_button.contains(MouseButtons::LEFT_CLICK) && !inside {
            self.base.state = UiState::Default;
            return vec![self.base.make_event(UiMsg::WidgetUnfocused)];
        }

        Vec::new()
    }

    /// SELECTED_EDITABLE state: full keyboard input handling.
    ///
    /// Character input comes from `input.text_input`, not from physical
    /// key codes — that path lets
    /// the platform IME handle dead keys, layouts, and composition.
    fn process_input_editable(&mut self, input: &WidgetInput) -> Vec<UiEvent> {
        // Toggle caret blink.
        self.caret_visible = !self.caret_visible;

        // Insert any text that arrived this frame via SDL text input.
        // Limit to chars allowed by `max_length` (0 = unlimited). The
        // length comparison includes a leading dummy slot, so the max
        // actual char count is `max_length - 1` (callers passing
        // `MAX_PLAYER_NAME_LENGTH = 30` get 29 typeable chars).
        let mut text_inserted = false;
        for ch in input.text_input.chars() {
            if ch == '\0' {
                continue;
            }
            if self.max_length > 0 && self.edit_text.chars().count() >= self.max_length - 1 {
                break;
            }
            let byte_pos = byte_offset_for_char_index(&self.edit_text, self.caret_offset);
            self.edit_text.insert(byte_pos, ch);
            self.caret_offset += 1;
            text_inserted = true;
        }
        if text_inserted {
            self.caret_visible = true;
            return vec![self.base.make_event(UiMsg::WidgetTextChanging)];
        }

        let kb = input.keyboard;

        for key in [
            KeyCode::ArrowLeft,
            KeyCode::ArrowRight,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Backspace,
            KeyCode::Delete,
            KeyCode::Enter,
            KeyCode::Escape,
            KeyCode::Tab,
            KeyCode::ArrowUp,
            KeyCode::ArrowDown,
        ] {
            let key_state = kb.get_state_of_key(key);
            let tw = kb.get_typewriter_state(key);

            // Only process on key-pressed, key-double, or typewriter repeat.
            let should_process = matches!(key_state, KeyState::KeyPressed | KeyState::KeyDouble)
                || tw == TypeWriter::Repeat;

            if !should_process {
                continue;
            }

            match key {
                KeyCode::ArrowLeft => {
                    if self.caret_offset > 0 {
                        self.caret_offset -= 1;
                    }
                    self.caret_visible = true;
                    return Vec::new();
                }
                KeyCode::ArrowRight => {
                    if self.caret_offset < self.edit_text.chars().count() {
                        self.caret_offset += 1;
                    }
                    self.caret_visible = true;
                    return Vec::new();
                }
                KeyCode::Home => {
                    self.caret_offset = 0;
                    self.caret_visible = true;
                    return Vec::new();
                }
                KeyCode::End => {
                    self.caret_offset = self.edit_text.chars().count();
                    self.caret_visible = true;
                    return Vec::new();
                }
                KeyCode::Backspace => {
                    if self.caret_offset > 0 {
                        self.caret_offset -= 1;
                        let byte_pos =
                            byte_offset_for_char_index(&self.edit_text, self.caret_offset);
                        self.edit_text.remove(byte_pos);
                        return vec![self.base.make_event(UiMsg::WidgetTextChanging)];
                    }
                    return Vec::new();
                }
                KeyCode::Delete => {
                    if self.caret_offset < self.edit_text.chars().count() {
                        let byte_pos =
                            byte_offset_for_char_index(&self.edit_text, self.caret_offset);
                        self.edit_text.remove(byte_pos);
                        return vec![self.base.make_event(UiMsg::WidgetTextChanging)];
                    }
                    return Vec::new();
                }
                KeyCode::Enter => {
                    return self.validate_and_exit();
                }
                KeyCode::Escape => {
                    return self.cancel_and_exit();
                }
                KeyCode::Tab => {
                    // Validate current field and move to linked field.
                    let mut events = self.validate_and_exit();
                    // The frame window / menu will handle focusing the linked field.
                    events.push(self.base.make_event(UiMsg::WidgetActivated));
                    return events;
                }
                KeyCode::ArrowUp | KeyCode::ArrowDown => {
                    // Up/Down exit edit mode (for navigation to other fields).
                    return self.validate_and_exit();
                }
                _ => {
                    // Printable characters arrive via `input.text_input`,
                    // handled above. Physical keys here are just the
                    // special-key cases.
                }
            }
        }

        Vec::new()
    }

    // ── Edit mode helpers ──────────────────────────────────────────

    /// Enter edit mode: save current text, position caret at end.
    ///
    /// Public so callers can force-enter edit mode — the Save modal
    /// uses this to start editing immediately rather than requiring
    /// the user to double-click the field.
    ///
    /// Gated on `state != SelectedEditable`: re-entering while already
    /// in edit mode would clobber `saved_text` (defeating Esc-undo) and
    /// snap the caret to end-of-text.
    pub fn enter_edit_mode(&mut self) {
        if self.base.state == UiState::SelectedEditable {
            return;
        }
        self.base.state = UiState::SelectedEditable;
        self.saved_text = self.edit_text.clone();
        self.caret_offset = self.edit_text.chars().count();
        self.caret_visible = true;
    }

    /// Validate the edit and exit edit mode.
    fn validate_and_exit(&mut self) -> Vec<UiEvent> {
        self.base.state = UiState::Selected;
        self.base.text = self.edit_text.clone();
        // Hiding the caret snaps it to end of buffer.
        self.caret_offset = self.edit_text.chars().count();
        self.caret_visible = false;
        vec![
            self.base.make_event(UiMsg::WidgetTextChanged),
            self.base.make_event(UiMsg::WidgetActivated),
        ]
    }

    /// Cancel the edit and restore the saved text.
    fn cancel_and_exit(&mut self) -> Vec<UiEvent> {
        self.edit_text = self.saved_text.clone();
        self.base.text = self.saved_text.clone();
        self.base.state = UiState::Default;
        // Snap caret to end of the *restored* buffer so `caret_offset`
        // doesn't dangle past the new end.
        self.caret_offset = self.edit_text.chars().count();
        self.caret_visible = false;
        vec![self.base.make_event(UiMsg::WidgetUnfocused)]
    }

    /// Set text and sync the edit buffer. Unconditionally resets the
    /// caret to position 0.
    pub fn set_text(&mut self, text: &str) {
        self.base.set_text(text);
        self.edit_text = text.to_string();
        self.caret_offset = 0;
    }

    /// Set the max allowed character count (0 = unlimited).
    ///
    /// The length comparison includes a leading dummy slot, so the max
    /// actual char count is `max_length - 1`.
    pub fn set_max_length(&mut self, max_length: usize) {
        self.max_length = max_length;
        if max_length > 0 {
            let max_actual = max_length.saturating_sub(1);
            let cur_chars = self.edit_text.chars().count();
            if cur_chars > max_actual {
                let truncated: String = self.edit_text.chars().take(max_actual).collect();
                self.edit_text = truncated.clone();
                self.base.text = truncated;
                self.caret_offset = self.caret_offset.min(max_actual);
            }
        }
    }

    // ── Caller-driven edit helpers ─────────────────────────────────
    //
    // Modal loops that don't drive a full `UiKeyboard` each frame (the
    // Save picker, for example) still want Backspace / Delete / caret
    // movement to route through the widget so its edit buffer remains
    // the single source of truth. These wrap the same body the
    // `process_input_editable` special-key arms do, exposed as direct
    // calls for callers holding a key event.

    /// Remove the character before the caret (Backspace).
    pub fn backspace(&mut self) -> Option<UiEvent> {
        if self.caret_offset == 0 {
            return None;
        }
        self.caret_offset -= 1;
        let byte_pos = byte_offset_for_char_index(&self.edit_text, self.caret_offset);
        self.edit_text.remove(byte_pos);
        self.caret_visible = true;
        Some(self.base.make_event(UiMsg::WidgetTextChanging))
    }

    /// Remove the character at the caret (Delete).
    pub fn delete_char(&mut self) -> Option<UiEvent> {
        if self.caret_offset >= self.edit_text.chars().count() {
            return None;
        }
        let byte_pos = byte_offset_for_char_index(&self.edit_text, self.caret_offset);
        self.edit_text.remove(byte_pos);
        self.caret_visible = true;
        Some(self.base.make_event(UiMsg::WidgetTextChanging))
    }

    /// Move caret one char left. No-op at position 0.
    pub fn move_caret_left(&mut self) {
        if self.caret_offset > 0 {
            self.caret_offset -= 1;
            self.caret_visible = true;
        }
    }

    /// Move caret one char right. No-op at end of buffer.
    pub fn move_caret_right(&mut self) {
        if self.caret_offset < self.edit_text.chars().count() {
            self.caret_offset += 1;
            self.caret_visible = true;
        }
    }

    /// Jump caret to start of buffer.
    pub fn move_caret_home(&mut self) {
        self.caret_offset = 0;
        self.caret_visible = true;
    }

    /// Jump caret to end of buffer.
    pub fn move_caret_end(&mut self) {
        self.caret_offset = self.edit_text.chars().count();
        self.caret_visible = true;
    }

    /// `true` iff the widget is currently in `SelectedEditable`.
    /// Callers holding a `FocusManager` use this to gate the re-enable
    /// of shortcuts/navigation while editing.
    pub fn is_editing(&self) -> bool {
        self.base.state == UiState::SelectedEditable
    }

    /// Whether the caret is logically shown — i.e. whether the widget is
    /// in edit mode. `caret_visible` is overloaded as both that flag and
    /// the per-frame blink toggle, so callers that want the edit-mode
    /// flag (independent of blink state) should use this getter.
    pub fn is_caret_shown(&self) -> bool {
        self.is_editing()
    }

    /// Insert a single character at the current caret position.
    ///
    /// Rejects only `\0`, gates on `max_length` (counting the dummy
    /// head slot), advances the caret one position past the insert.
    /// Returns `true` on insert, `false` on full/zero. Exposed for
    /// callers (e.g. modal special-key paths) that drive insertion
    /// outside `process_input`.
    pub fn insert_character(&mut self, ch: char) -> bool {
        if ch == '\0' {
            return false;
        }
        if self.max_length > 0 && self.edit_text.chars().count() >= self.max_length - 1 {
            return false;
        }
        let byte_pos = byte_offset_for_char_index(&self.edit_text, self.caret_offset);
        self.edit_text.insert(byte_pos, ch);
        self.caret_offset += 1;
        self.caret_visible = true;
        true
    }

    /// Compute the visible substring from the caret outward, bounded by
    /// a pixel-width budget.
    ///
    /// Walks left or right from the caret, accumulating per-char pixel
    /// widths via `char_width_fn` (which should return the character's
    /// rendered width including any extra spacing) and stops when the
    /// running total would exceed `width`. Used by the renderer to
    /// compute a horizontal scroll window so long text scrolls under
    /// the caret rather than overflowing the field.
    ///
    /// Font-agnostic at the widget level: callers wire their own font
    /// (NativeFont, TrueTypeFont, …) through the closure.
    pub fn get_text_from_caret(
        &self,
        side: TextFromCaretSide,
        width: u32,
        char_width_fn: impl Fn(char) -> u32,
    ) -> String {
        let chars: Vec<char> = self.edit_text.chars().collect();
        let n = chars.len();
        let mut result = String::new();
        let mut cur_width: u32 = 0;

        match side {
            TextFromCaretSide::Left => {
                // Walk leftward from `caret_offset - 1` (the char just
                // before the caret), prepending to the result.
                if self.caret_offset == 0 {
                    return result;
                }
                let mut i = self.caret_offset;
                while i > 0 {
                    let ch = chars[i - 1];
                    let w = char_width_fn(ch);
                    if cur_width + w > width {
                        break;
                    }
                    cur_width += w;
                    result.insert(0, ch);
                    i -= 1;
                }
            }
            TextFromCaretSide::Right => {
                // Walk rightward from `caret_offset` (the char at the
                // caret position).
                let mut i = self.caret_offset;
                while i < n {
                    let ch = chars[i];
                    let w = char_width_fn(ch);
                    if cur_width + w > width {
                        break;
                    }
                    cur_width += w;
                    result.push(ch);
                    i += 1;
                }
            }
        }

        result
    }

    /// Commit the edit and leave edit mode (as if Enter were pressed).
    pub fn commit_edit(&mut self) -> Vec<UiEvent> {
        if self.base.state != UiState::SelectedEditable {
            return Vec::new();
        }
        self.validate_and_exit()
    }

    /// Revert to the saved text and leave edit mode (as if Esc were
    /// pressed).
    pub fn cancel_edit(&mut self) -> Vec<UiEvent> {
        if self.base.state != UiState::SelectedEditable {
            return Vec::new();
        }
        self.cancel_and_exit()
    }

    /// Play the menu sound for any focus/activation events emitted this
    /// frame. Should be invoked right after the state-machine dispatch,
    /// using `WIDGET_NOISY_INPUTFIELD` as the widget variety.
    pub fn play_noise(
        events: &[crate::ui::UiEvent],
        sound: &mut crate::sound::SoundManager,
        backend: Option<&mut dyn crate::sound::AudioBackend>,
        loader: &robin_engine::sound_cache::SampleLoader,
    ) {
        crate::ingame_menu::widget_bridge::play_widget_noise(
            events,
            crate::ingame_menu::widget_bridge::WIDGET_NOISY_INPUTFIELD,
            sound,
            backend,
            loader,
        );
    }
}

// ── WidgetFocusable trait impl ─────────────────────────────────────
//
// Glue for `FocusManager` so an input field can be dropped into the
// focusable chain via `add_focusable`. The `active` argument is
// ignored: `set_focusable_active` always calls `enter_edit_mode()`,
// which is itself gated on `state != SelectedEditable`. Truth table:
// already in edit mode → no-op; otherwise → enter edit mode.

impl crate::focus_manager::WidgetFocusable for WidgetInputField {
    fn widget_id(&self) -> crate::focus_manager::WidgetId {
        self.base.id as crate::focus_manager::WidgetId
    }

    fn set_focusable_active(&mut self, _active: bool) -> Vec<crate::focus_manager::UiEvent> {
        self.enter_edit_mode();
        Vec::new()
    }

    /// Entering edit mode disables the focus manager's shortcuts and
    /// navigation; leaving re-enables them. Routed through the trait
    /// so the focus manager owns the toggle without needing a
    /// back-pointer.
    fn suppresses_navigation_while_active(&self) -> bool {
        true
    }
}

/// Side selector for [`WidgetInputField::get_text_from_caret`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFromCaretSide {
    Left,
    Right,
}

/// Convert a character index into a byte offset within a UTF-8 string.
///
/// `char_index == s.chars().count()` maps to `s.len()` (one past the end),
/// mirroring how caret offsets can sit at the end of the buffer.
fn byte_offset_for_char_index(s: &str, char_index: usize) -> usize {
    s.char_indices()
        .nth(char_index)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{MouseButtons, UiKeyboard};
    use robin_engine::coordinates::ScreenBBox;

    fn make_editable_field() -> WidgetInputField {
        let mut f = WidgetInputField::new(1);
        f.base
            .create("", ScreenBBox::from_coords(0.0, 0.0, 100.0, 20.0), 0);
        f.base.renderer = crate::widget::WidgetRenderer::Bitmap(crate::ui::RendererBitmap {
            base: crate::ui::RendererBase {
                bbox: ScreenBBox::from_coords(0.0, 0.0, 100.0, 20.0),
                ..Default::default()
            },
        });
        f.base.state = UiState::SelectedEditable;
        f
    }

    fn make_input<'a>(kb: &'a UiKeyboard, text: &'a str) -> WidgetInput<'a> {
        WidgetInput {
            mouse_position: robin_engine::coordinates::ScreenPoint::new(-1.0, -1.0),
            mouse_z: 0,
            mouse_button: MouseButtons::empty(),
            keyboard: kb,
            text_input: text,
            capture: None,
        }
    }

    #[test]
    fn text_input_inserts_at_caret() {
        let mut f = make_editable_field();
        let kb = UiKeyboard::default();
        let events = f.process_input(&make_input(&kb, "hi"));
        assert_eq!(f.edit_text, "hi");
        assert_eq!(f.caret_offset, 2);
        assert!(
            events
                .iter()
                .any(|e| e.msg_type == UiMsg::WidgetTextChanging)
        );
    }

    #[test]
    fn text_input_respects_max_length() {
        // The dummy-head slot counts toward `max_length`, so
        // max_length=3 caps actual chars at 2.
        let mut f = make_editable_field();
        f.max_length = 3;
        let kb = UiKeyboard::default();
        f.process_input(&make_input(&kb, "abcde"));
        assert_eq!(f.edit_text, "ab");
        assert_eq!(f.caret_offset, 2);
    }

    #[test]
    fn text_input_inserts_tab_and_newline() {
        // `insert_character` rejects only `\0`; TAB and NEWLINE go
        // through. SDL_EVENT_TEXT_INPUT excludes them in practice,
        // but the filter is literal.
        let mut f = make_editable_field();
        let kb = UiKeyboard::default();
        f.process_input(&make_input(&kb, "a\tb\nc"));
        assert_eq!(f.edit_text, "a\tb\nc");
    }

    #[test]
    fn text_input_inserts_at_caret_position() {
        let mut f = make_editable_field();
        f.edit_text = "ab".to_string();
        f.caret_offset = 1;
        let kb = UiKeyboard::default();
        f.process_input(&make_input(&kb, "X"));
        assert_eq!(f.edit_text, "aXb");
        assert_eq!(f.caret_offset, 2);
    }

    #[test]
    fn multibyte_text_inserts_and_caret_tracks_chars() {
        let mut f = make_editable_field();
        let kb = UiKeyboard::default();
        // "café" has 4 chars but 5 bytes (é is 2 bytes in UTF-8).
        f.process_input(&make_input(&kb, "café"));
        assert_eq!(f.edit_text, "café");
        assert_eq!(f.caret_offset, 4);
    }

    #[test]
    fn byte_offset_helper_handles_multibyte() {
        // "é" = 2 bytes, "f" = 1 byte, "é" = 2 bytes  → total 5 bytes, 3 chars
        let s = "éfé";
        assert_eq!(byte_offset_for_char_index(s, 0), 0);
        assert_eq!(byte_offset_for_char_index(s, 1), 2);
        assert_eq!(byte_offset_for_char_index(s, 2), 3);
        assert_eq!(byte_offset_for_char_index(s, 3), 5);
        // One past the end → string length.
        assert_eq!(byte_offset_for_char_index(s, 4), 5);
    }

    // ── Menu wrapper behaviors ──────────────────────────────────────

    #[test]
    fn enter_edit_mode_public_seeds_saved_text_and_caret() {
        let mut f = WidgetInputField::new(7);
        f.set_text("hello");
        f.enter_edit_mode();
        assert_eq!(f.base.state, UiState::SelectedEditable);
        assert_eq!(f.saved_text, "hello");
        assert_eq!(f.caret_offset, 5);
        assert!(f.caret_visible);
    }

    #[test]
    fn backspace_removes_char_before_caret() {
        let mut f = make_editable_field();
        f.edit_text = "abc".to_string();
        f.caret_offset = 2;
        let ev = f.backspace();
        assert_eq!(f.edit_text, "ac");
        assert_eq!(f.caret_offset, 1);
        assert!(ev.is_some());
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut f = make_editable_field();
        f.edit_text = "abc".to_string();
        f.caret_offset = 0;
        assert!(f.backspace().is_none());
        assert_eq!(f.edit_text, "abc");
    }

    #[test]
    fn backspace_handles_multibyte() {
        let mut f = make_editable_field();
        f.edit_text = "café".to_string();
        f.caret_offset = 4;
        f.backspace();
        assert_eq!(f.edit_text, "caf");
        assert_eq!(f.caret_offset, 3);
    }

    #[test]
    fn delete_char_removes_at_caret() {
        let mut f = make_editable_field();
        f.edit_text = "abc".to_string();
        f.caret_offset = 1;
        let ev = f.delete_char();
        assert_eq!(f.edit_text, "ac");
        assert_eq!(f.caret_offset, 1);
        assert!(ev.is_some());
    }

    #[test]
    fn commit_edit_returns_changed_and_activated() {
        let mut f = make_editable_field();
        f.edit_text = "hi".to_string();
        let events = f.commit_edit();
        assert_eq!(f.base.state, UiState::Selected);
        assert_eq!(f.base.text, "hi");
        assert!(
            events
                .iter()
                .any(|e| e.msg_type == UiMsg::WidgetTextChanged)
        );
        assert!(events.iter().any(|e| e.msg_type == UiMsg::WidgetActivated));
    }

    #[test]
    fn cancel_edit_restores_saved_text() {
        let mut f = make_editable_field();
        f.saved_text = "orig".to_string();
        f.edit_text = "edited".to_string();
        let events = f.cancel_edit();
        assert_eq!(f.edit_text, "orig");
        assert_eq!(f.base.text, "orig");
        assert_eq!(f.base.state, UiState::Default);
        assert!(events.iter().any(|e| e.msg_type == UiMsg::WidgetUnfocused));
    }

    #[test]
    fn set_max_length_truncates_existing_text() {
        // max_length=3 includes the dummy-head slot, so max actual chars=2.
        let mut f = WidgetInputField::new(1);
        f.set_text("abcdef");
        f.set_max_length(3);
        assert_eq!(f.edit_text, "ab");
        assert_eq!(f.base.text, "ab");
    }

    #[test]
    fn focusable_active_enters_edit_mode() {
        use crate::focus_manager::WidgetFocusable;
        let mut f = WidgetInputField::new(42);
        f.set_text("start");
        let events = f.set_focusable_active(true);
        assert!(events.is_empty());
        assert_eq!(f.base.state, UiState::SelectedEditable);
        assert_eq!(f.saved_text, "start");
        // widget_id passes through.
        let fcs: &dyn WidgetFocusable = &f;
        assert_eq!(fcs.widget_id(), 42);
    }

    #[test]
    fn focusable_active_while_editing_is_noop() {
        // Re-entering edit mode while already editing must not clobber
        // saved_text or snap the caret.
        use crate::focus_manager::WidgetFocusable;
        let mut f = make_editable_field();
        f.saved_text = "orig".to_string();
        f.edit_text = "typed".to_string();
        f.caret_offset = 2;
        f.set_focusable_active(true);
        assert_eq!(f.saved_text, "orig");
        assert_eq!(f.caret_offset, 2);
        assert_eq!(f.base.state, UiState::SelectedEditable);
    }

    #[test]
    fn focusable_inactive_outside_edit_mode_enters_edit() {
        // `set_focusable_active` ignores its argument, so passing false
        // while in a non-editable state must still enter edit mode.
        use crate::focus_manager::WidgetFocusable;
        let mut f = WidgetInputField::new(5);
        f.set_text("hello");
        f.set_focusable_active(false);
        assert_eq!(f.base.state, UiState::SelectedEditable);
        assert_eq!(f.saved_text, "hello");
    }

    #[test]
    fn get_text_from_caret_left_and_right_with_pixel_budget() {
        // With every char 10 pixels wide, a width budget of 25 fits
        // exactly two chars on either side of the caret (cumulative
        // 20 ≤ 25, next would push to 30 > 25).
        let mut f = WidgetInputField::new(1);
        f.edit_text = "abcdef".to_string();
        f.caret_offset = 3; // between 'c' and 'd'
        let cw = |_ch: char| -> u32 { 10 };
        let left = f.get_text_from_caret(TextFromCaretSide::Left, 25, cw);
        assert_eq!(left, "bc");
        let right = f.get_text_from_caret(TextFromCaretSide::Right, 25, cw);
        assert_eq!(right, "de");
    }

    #[test]
    fn get_text_from_caret_zero_width_returns_empty() {
        // Returns empty when the budget can't fit the first char.
        let mut f = WidgetInputField::new(1);
        f.edit_text = "abc".to_string();
        f.caret_offset = 1;
        let cw = |_ch: char| -> u32 { 10 };
        assert_eq!(f.get_text_from_caret(TextFromCaretSide::Left, 5, cw), "");
        assert_eq!(f.get_text_from_caret(TextFromCaretSide::Right, 5, cw), "");
    }

    #[test]
    fn validate_and_cancel_snap_caret_to_end() {
        // Hiding the caret on exit snaps it to end of buffer.
        let mut f = make_editable_field();
        f.edit_text = "hello".to_string();
        f.caret_offset = 2;
        f.commit_edit();
        assert_eq!(f.caret_offset, 5);

        let mut f2 = make_editable_field();
        f2.saved_text = "abc".to_string();
        f2.edit_text = "longer text".to_string();
        f2.caret_offset = 9;
        f2.cancel_edit();
        // Caret should snap to end of the *restored* buffer ("abc"), not
        // dangle past it.
        assert_eq!(f2.edit_text, "abc");
        assert_eq!(f2.caret_offset, 3);
    }

    #[test]
    fn set_text_resets_caret_to_zero() {
        // `set_text` unconditionally resets the caret to position 0.
        let mut f = WidgetInputField::new(1);
        f.caret_offset = 4;
        f.set_text("hello");
        assert_eq!(f.caret_offset, 0);
    }

    #[test]
    fn is_caret_shown_tracks_edit_mode_not_blink() {
        let mut f = WidgetInputField::new(1);
        f.set_text("hi");
        assert!(!f.is_caret_shown());
        f.enter_edit_mode();
        assert!(f.is_caret_shown());
        // Blink toggle must not flip the edit-mode-active semantic.
        f.caret_visible = false;
        assert!(f.is_caret_shown());
    }

    #[test]
    fn insert_character_rejects_null_and_respects_max_length() {
        let mut f = make_editable_field();
        f.max_length = 3; // dummy-head slot counts → max actual chars = 2.
        assert!(!f.insert_character('\0'));
        assert!(f.insert_character('a'));
        assert!(f.insert_character('b'));
        assert!(!f.insert_character('c'));
        assert_eq!(f.edit_text, "ab");
        assert_eq!(f.caret_offset, 2);
    }

    #[test]
    fn focusable_inactive_while_editing_is_noop() {
        // The field stays in edit mode; an earlier impl had an
        // implicit-commit-on-focus-loss which has since been removed.
        use crate::focus_manager::WidgetFocusable;
        let mut f = make_editable_field();
        f.base.id = 5;
        f.edit_text = "typed".to_string();
        f.set_focusable_active(false);
        assert_eq!(f.base.state, UiState::SelectedEditable);
        assert_eq!(f.base.text, "");
    }
}
