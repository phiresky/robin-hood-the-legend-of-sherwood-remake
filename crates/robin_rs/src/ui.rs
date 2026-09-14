//! Keyboard state, UI events, asset selection, and pixel hit masks.
//! Widgets own their geometry and text; the GPU bridge draws them.

use std::collections::BTreeMap;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

use crate::input::KeyboardState;
use robin_engine::coordinates::{ScreenBBox, ScreenPoint};

// ═════════════════════════════════════════════════════════════════════
//  Enums & constants
// ═════════════════════════════════════════════════════════════════════

/// UI message types emitted by widgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum UiMsg {
    WidgetFocused = 0,
    WidgetUnfocused,
    WidgetActivated,
    WidgetDoubleClicked,
    WidgetUnselect,
    WidgetAlreadyUnselected,
    WidgetReAlreadyUnselected,
    WidgetReactivated,
    WidgetEditMode,
    WidgetTextChanged,
    WidgetTextChanging,
    FrameFocus,
    WidgetScrollDown,
    WidgetScrollUp,
    WidgetListFocusChange,
    WidgetListSelectChange,
    WidgetSliderTrack,
    MouseCursorChange,
    WidgetFocusedDisabled,
}

/// Widget interaction state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum UiState {
    #[default]
    Default = 0,
    Focused,
    Selected,
    Pushed,
    Clicked,
    FocusedFirst,
    SelectedFirst,
    PushedFirst,
    FocusedSecond,
    SelectedSecond,
    PushedSecond,
    GlobalSelect,
    GlobalFocus,
    SelectedEditable,
}

// ── Mouse button masks ───────────────────────────────────────────────

bitflags! {
    /// Raw input mouse button bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct RawMouseButtons: u16 {
        const LEFT   = 0x0001;
        const RIGHT  = 0x0002;
        const MIDDLE = 0x0004;
    }
}

bitflags! {
    /// Processed mouse button events (clicks, double-clicks, held).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct MouseButtons: u16 {
        const LEFT_CLICK          = 0x0001;
        const RIGHT_CLICK         = 0x0002;
        const MIDDLE_CLICK        = 0x0004;
        const LEFT_DOUBLE_CLICK   = 0x0008;
        const RIGHT_DOUBLE_CLICK  = 0x0010;
        const MIDDLE_DOUBLE_CLICK = 0x0020;
        const LEFT_DOWN           = 0x0040;
        const RIGHT_DOWN          = 0x0080;
        const MIDDLE_DOWN         = 0x0100;
    }
}

/// Double-click detection window (in frames).
pub const DOUBLE_CLICK_SPEED: u16 = 5;

// ═════════════════════════════════════════════════════════════════════
//  Keyboard wrapper
// ═════════════════════════════════════════════════════════════════════

/// Key state as seen by the UI layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum KeyState {
    /// Key is at rest.
    #[default]
    KeyUp = 0,
    /// Key is held down.
    KeyDown,
    /// Key was released (single press).
    KeyPressed,
    /// Key was double-pressed.
    KeyDouble,
}

/// Typewriter repeat state for a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum TypeWriter {
    #[default]
    None = 0,
    /// First frame the key is held.
    Touch,
    /// Generate a repeat event this frame.
    Repeat,
    /// Waiting for next repeat interval.
    Waiting,
}

/// Delay before the first key repeat (ms).
const REPEAT_FIRST_MS: u32 = 400;
/// Delay between subsequent repeats (ms).
const REPEAT_AFTER_MS: u32 = 50;
/// Default double-press delay (ms).
const DEFAULT_DOUBLE_PRESS_DELAY: u32 = 500;

/// UI-level keyboard state tracker.
///
/// Tracks per-key state transitions (up → down → pressed / double),
/// key repeat (typewriter), and double-press detection.
///
/// [`refresh`](Self::refresh) must be called once per frame with the
/// current raw keyboard state and the current time in milliseconds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiKeyboard {
    initialized: bool,
    changed: bool,
    repeat_delay: u16,
    repeat_loop: u16,
    double_press_delay: u32,

    key_state: BTreeMap<KeyCode, KeyState>,
    old_key_state: BTreeMap<KeyCode, KeyState>,
    repeat_counter: BTreeMap<KeyCode, u16>,
    typewriter: BTreeMap<KeyCode, TypeWriter>,
    last_key_press: BTreeMap<KeyCode, u32>,
    type_press: BTreeMap<KeyCode, u32>,

    old_keyboard_state: KeyboardState,
}

impl Default for UiKeyboard {
    fn default() -> Self {
        Self::new(DEFAULT_DOUBLE_PRESS_DELAY)
    }
}

impl UiKeyboard {
    pub fn new(double_press_delay: u32) -> Self {
        Self {
            initialized: false,
            changed: false,
            repeat_delay: 5,
            repeat_loop: 2,
            double_press_delay,
            key_state: BTreeMap::new(),
            old_key_state: BTreeMap::new(),
            repeat_counter: BTreeMap::new(),
            typewriter: BTreeMap::new(),
            last_key_press: BTreeMap::new(),
            type_press: BTreeMap::new(),
            old_keyboard_state: KeyboardState::default(),
        }
    }

    /// Refresh key states from the current raw keyboard state.
    ///
    /// `current_time_ms` is a monotonic timestamp.
    /// Returns `true` if the keyboard was already initialized (i.e. this
    /// is not the very first call).
    pub fn refresh(&mut self, keyboard_state: &KeyboardState, current_time_ms: u32) -> bool {
        // On the very first call, just memorize the state.
        if !self.initialized {
            self.old_keyboard_state = keyboard_state.clone();
            self.initialized = true;
            return false;
        }

        // Save previous key states for change detection.
        self.old_key_state = self.key_state.clone();

        self.changed = false;

        // The snapshot contains every tracked key. Visit raw-only keys separately,
        // excluding overlaps so held keys advance their typewriter only once.
        let keys_to_update = self.old_key_state.keys().chain(
            keyboard_state
                .keys
                .union(&self.old_keyboard_state.keys)
                .filter(|key| !self.old_key_state.contains_key(key)),
        );

        for &key in keys_to_update {
            let cur = keyboard_state.keys.contains(&key);
            let old = self.old_keyboard_state.keys.contains(&key);

            if cur != old {
                // ── Key state changed this frame ──
                self.changed = true;

                if cur {
                    // Key just went down.
                    self.key_state.insert(key, KeyState::KeyDown);
                    self.type_press.insert(key, current_time_ms);
                    self.typewriter.insert(key, TypeWriter::None);
                } else {
                    // Key just went up.
                    self.repeat_counter.insert(key, 0);

                    // Only handle a previous `KeyDown` here; other previous
                    // states (`KeyPressed`, `KeyDouble`, `KeyUp`) are a no-op
                    // and leave `last_key_press` untouched.
                    if self.key_state.get(&key).copied() == Some(KeyState::KeyDown) {
                        let last_key_press = *self.last_key_press.get(&key).unwrap_or(&0);
                        if current_time_ms.wrapping_sub(last_key_press) <= self.double_press_delay {
                            self.key_state.insert(key, KeyState::KeyDouble);
                        } else {
                            self.key_state.insert(key, KeyState::KeyPressed);
                            self.last_key_press.insert(key, current_time_ms);
                        }
                    }
                }
            } else {
                // ── Key state unchanged ──

                if cur {
                    // Key is still held — advance the typewriter.
                    match self
                        .typewriter
                        .get(&key)
                        .copied()
                        .unwrap_or(TypeWriter::None)
                    {
                        TypeWriter::None => {
                            self.typewriter.insert(key, TypeWriter::Touch);
                        }
                        TypeWriter::Touch => {
                            let type_press = *self.type_press.get(&key).unwrap_or(&0);
                            if current_time_ms.wrapping_sub(type_press) > REPEAT_FIRST_MS {
                                self.typewriter.insert(key, TypeWriter::Repeat);
                                self.type_press.insert(key, current_time_ms);
                            }
                        }
                        TypeWriter::Repeat => {
                            self.typewriter.insert(key, TypeWriter::Waiting);
                        }
                        TypeWriter::Waiting => {
                            let type_press = *self.type_press.get(&key).unwrap_or(&0);
                            if current_time_ms.wrapping_sub(type_press) > REPEAT_AFTER_MS {
                                self.typewriter.insert(key, TypeWriter::Repeat);
                                self.type_press.insert(key, current_time_ms);
                            }
                        }
                    }
                } else {
                    self.typewriter.insert(key, TypeWriter::None);
                }

                // Clean up transient states.
                match self.key_state.get(&key).copied().unwrap_or(KeyState::KeyUp) {
                    KeyState::KeyDouble | KeyState::KeyPressed => {
                        self.changed = true;
                        self.key_state.insert(key, KeyState::KeyUp);
                    }
                    _ => {}
                }
            }
        }

        self.old_keyboard_state = keyboard_state.clone();
        true
    }

    /// Whether any key changed during the last [`refresh`](Self::refresh).
    pub fn has_changed(&self) -> bool {
        self.changed
    }

    /// Whether a specific key changed state during the last refresh.
    pub fn has_key_changed(&self, key: KeyCode) -> bool {
        self.old_key_state
            .get(&key)
            .copied()
            .unwrap_or(KeyState::KeyUp)
            != self.key_state.get(&key).copied().unwrap_or(KeyState::KeyUp)
    }

    /// Current state of a key.
    pub fn get_state_of_key(&self, key: KeyCode) -> KeyState {
        self.key_state.get(&key).copied().unwrap_or(KeyState::KeyUp)
    }

    /// Typewriter repeat state of a key.
    pub fn get_typewriter_state(&self, key: KeyCode) -> TypeWriter {
        self.typewriter
            .get(&key)
            .copied()
            .unwrap_or(TypeWriter::None)
    }

    /// Reset all key states and counters.
    pub fn reset(&mut self) {
        self.changed = true;
        self.key_state.clear();
        self.old_key_state.clear();
        self.repeat_counter.clear();
        self.typewriter.clear();
        self.last_key_press.clear();
        self.type_press.clear();
        self.old_keyboard_state.keys.clear();
    }
}

// ═════════════════════════════════════════════════════════════════════
//  UI structures
// ═════════════════════════════════════════════════════════════════════

/// Input context passed to widgets during event processing.
///
/// Widget and UI manager references are represented as opaque handles
/// until those types are implemented. We keep only the owned mouse state
/// here because the widget event paths (`widget/*.rs`) pass the
/// keyboard / ui as explicit arguments rather than bundling them into
/// the input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiInput {
    pub mouse_position: ScreenPoint,
    pub mouse_z: i16,
    pub mouse_button: u16,
}

/// A UI event produced by widget input processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiEvent {
    pub msg_type: UiMsg,
    /// Opaque widget handle (to be replaced with a typed ID when widgets
    /// are implemented).
    pub origin_widget_id: u32,
    /// Optional associated data.
    pub data: Option<UiEventData>,
}

/// Typed event data payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiEventData {
    SliderPosition(f32),
    ScrollDelta(i32),
    Text(String),
    CursorId(u16),
    ListIndex(u32),
}

// ═════════════════════════════════════════════════════════════════════
//  Resource widget sub-IDs
// ═════════════════════════════════════════════════════════════════════

/// Sub-resource identifiers for widget rendering states.
///
/// These are `u8` constants rather than an enum because different widget
/// types intentionally share the same numeric values (e.g. all widgets use
/// 0 for "disabled", 1 for "default", etc.).
pub mod resource_widget_id {
    // General
    pub const PICTURE_DEFAULT: u8 = 0;

    // Push buttons
    pub const BUTTON_DISABLED: u8 = 0;
    pub const BUTTON_DEFAULT: u8 = 1;
    pub const BUTTON_FOCUSED: u8 = 2;
    pub const BUTTON_SELECTED: u8 = 3;

    // Check boxes
    pub const CHECKBOX_DISABLED: u8 = 0;
    pub const CHECKBOX_UNSELECTED: u8 = 1;
    pub const CHECKBOX_FOCUS: u8 = 2;
    pub const CHECKBOX_SELECTED: u8 = 3;

    // Radio buttons
    pub const RADIO_DISABLED: u8 = 0;
    pub const RADIO_UNSELECTED: u8 = 1;
    pub const RADIO_FOCUS: u8 = 2;
    pub const RADIO_SELECTED: u8 = 3;
    pub const RADIO_FOCUS_SELECTED: u8 = 4;
    pub const RADIO_FOCUS_UNSELECTED: u8 = 5;

    // Extended radio
    pub const RADIO_EX_DISABLED: u8 = 0;
    pub const RADIO_EX_DEFAULT1: u8 = 1;
    pub const RADIO_EX_FOCUSED1: u8 = 2;
    pub const RADIO_EX_PUSHED1: u8 = 3;
    pub const RADIO_EX_DEFAULT2: u8 = 4;
    pub const RADIO_EX_FOCUSED2: u8 = 5;
    pub const RADIO_EX_PUSHED2: u8 = 6;

    // Toggle buttons
    pub const TOGGLE_DISABLED: u8 = 0;
    pub const TOGGLE_SELECTED_ONE: u8 = 1;
    pub const TOGGLE_FOCUSED_ONE: u8 = 2;
    pub const TOGGLE_SELECTED_TWO: u8 = 3;
    pub const TOGGLE_FOCUSED_TWO: u8 = 4;

    // Input fields
    pub const INPUT_FIELD_DISABLED: u8 = 0;
    pub const INPUT_FIELD_DEFAULT: u8 = 1;
    pub const INPUT_FIELD_FOCUSED: u8 = 2;
    pub const INPUT_FIELD_SELECTED: u8 = 3;
    pub const INPUT_FIELD_CLICKED: u8 = 4;
    pub const INPUT_FIELD_CARET: u8 = 5;

    // Slider
    pub const SLIDER_BACK_START: u8 = 0;
    pub const SLIDER_BACK_FILL: u8 = 1;
    pub const SLIDER_BACK_END: u8 = 2;
    pub const SLIDER_THUMB_START: u8 = 3;
    pub const SLIDER_THUMB_FILL: u8 = 4;
    pub const SLIDER_THUMB_END: u8 = 5;

    pub const NO_RESOURCE: u8 = 255;
}

// ═════════════════════════════════════════════════════════════════════
//  Renderers
// ═════════════════════════════════════════════════════════════════════

/// Resource ID type.
pub type ResourceId = i32;

/// Bit-packed opacity mask for the renderer's underlying sprite.
///
/// Hit-testing needs to reject clicks on transparent pixels of a
/// sprite, which conceptually requires sampling the surface and
/// comparing each pixel to its color-key. Rather than re-locking and
/// resampling per click, we pre-bake the `pixel != color_key` answer
/// into one bit per pixel; the wiring layer attaches a mask whenever
/// a widget is bound to a known sprite (see
/// `widget_bridge::attach_alpha_masks`). Mask is expressed in
/// renderer-local coords (0,0 = `bbox.top_left`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AlphaMask {
    pub width: u16,
    pub height: u16,
    /// Row-major, 1 bit per pixel, `(x + y*stride*8)` indexed.
    /// `bits[byte] & (1 << bit)` non-zero means the pixel is opaque.
    pub bits: Vec<u8>,
}

impl AlphaMask {
    /// Create a mask of the given size from a flat RGB565 pixel buffer.
    /// `pixels.len()` must be at least `pitch_words * height`. Pixels
    /// equal to `color_key` are flagged transparent.
    pub fn from_pixels(
        width: u16,
        height: u16,
        pitch_words: u32,
        pixels: &[u16],
        color_key: u16,
    ) -> Self {
        let stride_bytes = (width as usize).div_ceil(8);
        let mut bits = vec![0u8; stride_bytes * height as usize];
        for y in 0..height as usize {
            let row_off = y * pitch_words as usize;
            let bit_row = y * stride_bytes;
            for x in 0..width as usize {
                if pixels[row_off + x] != color_key {
                    bits[bit_row + (x >> 3)] |= 1 << (x & 7);
                }
            }
        }
        Self {
            width,
            height,
            bits,
        }
    }

    #[inline]
    pub fn is_opaque(&self, x: u16, y: u16) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        let stride_bytes = (self.width as usize).div_ceil(8);
        let byte = self.bits[y as usize * stride_bytes + (x as usize >> 3)];
        (byte & (1 << (x as usize & 7))) != 0
    }
}

/// Asset selection and optional pixel mask shared by GPU widget drawing and input.
/// Geometry and text belong to the widget itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetAppearance {
    pub resource_id: ResourceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_mask: Option<AlphaMask>,
}

impl Default for WidgetAppearance {
    fn default() -> Self {
        Self {
            resource_id: -1,
            alpha_mask: None,
        }
    }
}

impl WidgetAppearance {
    pub fn is_real_point(&self, bbox: ScreenBBox, point: ScreenPoint) -> bool {
        if !bbox.contains_point(point) {
            return false;
        }
        let Some(mask) = self.alpha_mask.as_ref() else {
            return true;
        };
        let tl = bbox.top_left();
        let lx = (point.x - tl.x).floor() as i32;
        let ly = (point.y - tl.y).floor() as i32;
        lx >= 0 && ly >= 0 && mask.is_opaque(lx as u16, ly as u16)
    }
}

#[cfg(test)]
mod tests;
