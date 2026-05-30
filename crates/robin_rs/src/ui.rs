//! UI framework: keyboard state tracking, mouse cursor, renderers, layout,
//! and the top-level UI manager.
//!
//! SDL / rendering calls are stubbed — actual blitting happens in the
//! widget-specific drawing code (see `widget/` and `game_session`); this
//! module is a serializable state + layout/event skeleton.

use std::collections::{BTreeMap, BTreeSet};

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

use crate::geo2d::{BBox2D, GeoPoint2D, pt};
use crate::ingame_menu::layout::{MenuTransform, TextAlign, VAlign, render_text_in_box_aligned};
use crate::input::KeyboardState;
use crate::native_font::NativeFont;
use crate::renderer::{BLIT_SOURCE_TRANSPARENT, Renderer};
use robin_engine::coordinates::{ScreenBBox, ScreenPoint};
use robin_engine::sprite::BBox;

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
    /// `current_time_ms` is a monotonic timestamp (e.g. `SDL_GetTicks()`).
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

        let mut keys_to_update: BTreeSet<KeyCode> = self.key_state.keys().copied().collect();
        keys_to_update.extend(keyboard_state.keys.iter().copied());
        keys_to_update.extend(self.old_keyboard_state.keys.iter().copied());

        for key in keys_to_update {
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

    pub fn double_press_delay(&self) -> u32 {
        self.double_press_delay
    }

    pub fn set_double_press_delay(&mut self, delay: u32) {
        self.double_press_delay = delay;
    }

    /// Get a reference to the last-seen raw keyboard state.
    pub fn raw_keyboard_state(&self) -> &KeyboardState {
        &self.old_keyboard_state
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
/// until those types are ported. We keep only the owned mouse state
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
    /// are ported).
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

/// Refresh probe code for the smart-refresh system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
pub enum ProbeCode {
    /// No refresh needed.
    #[default]
    NoRefresh = 0,
    /// Minimal (lazy) refresh — call widget's `Refresh`.
    LazyRefresh,
    /// Restore background only (e.g. clearing a tooltip).
    RestoreOnly,
    /// Full background + widget refresh.
    FullRefresh,
}

/// Refresh probe entry for the smart-refresh system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiProbe {
    pub code: ProbeCode,
    pub zone: ScreenBBox,
    pub widget_id: u32,
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

/// Rendering flags.
pub const RENDERER_RESOURCE_MIXING: u32 = 1;

/// Per-frame slide step when alpha is animating upward.
pub const SBRENDERER_ALPHA_INCREMENT: i16 = 9;

/// Per-frame slide step when alpha is animating downward.
pub const SBRENDERER_ALPHA_DECREMENT: i16 = 7;

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

/// Base renderer state shared by all renderer variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RendererBase {
    /// Bounding box where the widget is drawn.
    pub bbox: ScreenBBox,
    /// Current resource ID.
    pub resource_id: ResourceId,
    /// Current sub-resource for rendering.
    pub sub_resource: u8,
    /// Rendering flags.
    pub flags: u32,
    /// Text to display (for text-capable renderers).
    pub text: String,
    /// Double-buffered last-rendered surface IDs.
    pub last_rendered: [u32; 2],
    /// Counter for double-buffer pair selection.
    pub pair_counter: u32,
    /// Opaque handle to the rendering surface.
    pub rendering_surface: u32,
    /// Pixel-alpha mask for hit testing — when present,
    /// `is_real_point` rejects clicks on transparent pixels.
    /// `None` means bbox-only fallback, which is what every Rust
    /// callsite did before pixel-alpha was wired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_mask: Option<AlphaMask>,
}

impl Default for RendererBase {
    fn default() -> Self {
        Self {
            bbox: ScreenBBox::new(),
            resource_id: -1,
            sub_resource: resource_widget_id::NO_RESOURCE,
            flags: 0,
            text: String::new(),
            last_rendered: [u32::MAX; 2],
            pair_counter: 0,
            rendering_surface: u32::MAX,
            alpha_mask: None,
        }
    }
}

impl RendererBase {
    pub fn set_position_bbox(&mut self, bbox: ScreenBBox) {
        self.bbox = bbox;
    }

    pub fn set_position_point(&mut self, point: ScreenPoint) {
        // Actual dimensions are resolved by the concrete widget type
        // (see `widget/picture.rs` etc.), which queries `ResourceManager`
        // and calls `set_position_bbox`. This 1×1 fallback is a
        // safety net for callers that haven't been ported yet.
        self.bbox = ScreenBBox::from_point(point);
    }

    pub fn set_resource(&mut self, id: ResourceId) -> bool {
        self.resource_id = id;
        // Resource loading / reference-counting is done by the
        // concrete widget through `ResourceManager`, not here.
        true
    }

    pub fn dismiss_resource(&mut self) {
        // Concrete widgets release via `ResourceManager`; we only
        // clear our local ID here.
        self.resource_id = -1;
    }

    pub fn set_flags(&mut self, flags: u32) {
        self.flags = flags;
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
    }

    pub fn last_rendered(&self) -> u32 {
        self.last_rendered[(self.pair_counter % 2) as usize]
    }

    pub fn set_counter(&mut self, counter: u32) {
        self.pair_counter = counter;
    }

    pub fn reset_save(&mut self) {
        self.last_rendered = [u32::MAX; 2];
    }

    /// Hit-test a point against the renderer's clickable area.
    ///
    /// Bbox first, then if the renderer was bound to a sprite
    /// (`alpha_mask` populated by `widget_bridge::attach_alpha_masks`
    /// or its peers), reject pixels equal to the surface's color-key.
    /// When no mask is set (text labels, sliders, listboxes — anything
    /// not bound to a pre-loaded sprite pack), the bbox check stands
    /// alone.
    pub fn is_real_point(&self, point: ScreenPoint) -> bool {
        if !self.bbox.contains_point(point) {
            return false;
        }
        let Some(mask) = self.alpha_mask.as_ref() else {
            return true;
        };
        if self.bbox.0.is_none() {
            return true;
        }
        let tl = self.bbox.top_left();
        let lx = (point.x - tl.x).floor() as i32;
        let ly = (point.y - tl.y).floor() as i32;
        if lx < 0 || ly < 0 {
            return false;
        }
        mask.is_opaque(lx as u16, ly as u16)
    }

    /// Attach a pre-built per-pixel opacity mask (e.g. baked from the
    /// renderer's bound sprite). `None` clears any prior mask and
    /// reverts to bbox-only hit testing.
    pub fn set_alpha_mask(&mut self, mask: Option<AlphaMask>) {
        self.alpha_mask = mask;
    }

    /// Surface that would be used for the next render. Legacy hook;
    /// actual drawing goes through the concrete widget's own path.
    pub fn will_be_rendered(&self, _sub_res: u8) -> u32 {
        u32::MAX
    }

    /// Bookkeeping-only `render` — records the sub-resource and
    /// advances the double-buffer counter. The blit itself is done by
    /// the concrete widget against the GPU `Renderer`.
    pub fn render(&mut self, sub_res: u8) -> bool {
        self.sub_resource = sub_res;
        let idx = (self.pair_counter % 2) as usize;
        self.last_rendered[idx] = self.will_be_rendered(sub_res);
        true
    }
}

/// Bitmap renderer — blits a sub-resource directly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RendererBitmap {
    pub base: RendererBase,
}

/// Shadow renderer — blits with alpha-keyed shadow effect.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RendererShadow {
    pub base: RendererBase,
    pub shadow_key: u16,
    pub shadow_intensity: u16,
}

/// Alpha-blending renderer with animated alpha transitions.
///
/// Alpha is on a 0–100 scale (not 0–255) to match the renderer's GPU
/// alpha blit and the `SBRENDERER_ALPHA_*` constants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RendererAlphaConstant {
    pub base: RendererBase,
    /// Target alpha (0–100) the slide animates toward.
    pub target_alpha: u16,
    /// Vestigial alpha ceiling — exposed via `SetAlphaLimit`, but
    /// never actually read by the renderer. Kept for API parity only.
    pub alpha_limit: u16,
    /// Internal mixing surface handle. `u32::MAX` when not allocated.
    pub mix_surface: u32,
    /// Animation direction: `true` = increasing, `false` = decreasing.
    pub alpha_direction: bool,
    /// Whether the slide has reached its target.
    pub alpha_reached: bool,
    /// Current animated alpha value, advancing toward `target_alpha`.
    /// Stored as `i16` because the slide can transiently overshoot
    /// before the clamp at the end of `increment_sliding`.
    pub sliding_alpha: i16,
    /// Frame-delay counter; the slide advances when this hits 0.
    pub wait: i16,
    /// Cross-fade alpha during resource transitions (0–100).
    pub mixing_alpha: u16,
    /// Whether a resource cross-fade is in progress.
    pub mixing_in_progress: bool,
    /// Previous resource ID during a cross-fade. `-1` when no fade is
    /// pending.
    pub ancient_resource: ResourceId,
}

impl Default for RendererAlphaConstant {
    fn default() -> Self {
        // Defaults: alpha_reached = true, sliding_alpha = 0,
        // target_alpha = 100, mix surface sentinel 0xFFFFFFFF.
        Self {
            base: RendererBase::default(),
            target_alpha: 100,
            alpha_limit: 100,
            mix_surface: u32::MAX,
            alpha_direction: false,
            alpha_reached: true,
            sliding_alpha: 0,
            wait: 0,
            mixing_alpha: 0,
            mixing_in_progress: false,
            ancient_resource: -1,
        }
    }
}

impl RendererAlphaConstant {
    /// A fully-faded widget (`target_alpha == 0`) cannot receive
    /// clicks. Otherwise defer to the base renderer (bbox +
    /// pixel-alpha).
    pub fn is_real_point(&self, point: ScreenPoint) -> bool {
        if self.target_alpha == 0 {
            return false;
        }
        self.base.is_real_point(point)
    }

    /// Snap the target alpha.
    pub fn set_alpha_level(&mut self, alpha: u16) {
        self.target_alpha = alpha;
    }

    /// Set the cross-fade alpha.
    pub fn set_mix_alpha_level(&mut self, alpha: u16) {
        self.mixing_alpha = alpha;
    }

    /// Set the (vestigial) alpha ceiling.
    pub fn set_alpha_limit(&mut self, limit: u16) {
        self.alpha_limit = limit;
    }

    /// Returns the *target* alpha, not the current animated value.
    /// Use [`sliding_alpha`](Self::sliding_alpha) for the current
    /// frame's blend factor.
    pub fn alpha_level(&self) -> u16 {
        self.target_alpha
    }

    /// Current animated alpha value.
    pub fn sliding_alpha(&self) -> i16 {
        self.sliding_alpha
    }

    /// Configure a sliding alpha animation.
    ///
    /// - `target` — alpha value the slide animates toward.
    /// - `direction` — `true` = increasing, `false` = decreasing.
    /// - `initial` — starting value of `sliding_alpha`.
    /// - `wait` — frames to wait before the first step.
    ///
    /// The per-step delta is hardcoded to `SBRENDERER_ALPHA_INCREMENT`/
    /// `SBRENDERER_ALPHA_DECREMENT`; no caller-supplied step size.
    pub fn set_sliding_alpha_level(
        &mut self,
        target: u16,
        direction: bool,
        initial: i16,
        wait: i16,
    ) {
        self.alpha_reached = false;
        self.sliding_alpha = initial;
        self.target_alpha = target;
        self.alpha_direction = direction;
        self.wait = wait;
    }

    /// Whether an alpha transition has reached its target. Note the
    /// name is inverted relative to its body: `true` means the
    /// animation has *finished*.
    pub fn is_transition_in_progress(&self) -> bool {
        self.alpha_reached
    }

    /// Advance the sliding alpha by one step. Returns the alpha
    /// value (clamped to 0–100) to use for this frame's blit.
    pub fn increment_sliding(&mut self) -> u16 {
        if self.alpha_reached {
            return self.target_alpha;
        }
        if self.wait > 0 {
            self.wait -= 1;
        } else if self.alpha_direction {
            self.sliding_alpha += SBRENDERER_ALPHA_INCREMENT;
            if self.sliding_alpha > self.target_alpha as i16 {
                self.sliding_alpha = self.target_alpha as i16;
                self.alpha_reached = true;
            }
        } else {
            self.sliding_alpha -= SBRENDERER_ALPHA_DECREMENT;
            if self.sliding_alpha < self.target_alpha as i16 {
                self.sliding_alpha = self.target_alpha as i16;
                self.alpha_reached = true;
            }
        }
        self.sliding_alpha.clamp(0, 100) as u16
    }

    /// Stop a running cross-fade. Only flips the
    /// `mixing_in_progress` flag; `ancient_resource` is left untouched
    /// on purpose.
    pub fn clear_mixing(&mut self) {
        self.mixing_in_progress = false;
    }

    /// Set resource with mix transition support. Triggers a cross-fade
    /// only when the flag bit is set, the previous resource was
    /// non-empty, the resource actually changes, and the target alpha
    /// is non-zero.
    pub fn set_resource(&mut self, id: ResourceId) -> bool {
        if id >= 0
            && self.base.resource_id != id
            && self.base.resource_id >= 0
            && (self.base.flags & RENDERER_RESOURCE_MIXING) != 0
            && self.target_alpha > 0
        {
            self.ancient_resource = self.base.resource_id;
            self.mixing_in_progress = true;
            self.mixing_alpha = 100;
        }
        self.base.set_resource(id)
    }

    /// Render this widget with alpha-blending.
    ///
    /// There is no `ResourceManager` indirection that maps
    /// `(resource_id, sub_res) → surface`, so the caller pre-resolves
    /// surfaces and passes them in. `ancient_surface` is consulted
    /// only when [`mixing_in_progress`](Self::mixing_in_progress) is
    /// set.
    ///
    /// Note: this path is reachable today only if a future caller
    /// constructs `WidgetRenderer::Alpha` and routes it here. The
    /// only known caller, the portrait widget, has its
    /// `set_sliding_alpha_level` invocations commented out, and the
    /// only direct subclass is never instantiated.
    pub fn render(
        &mut self,
        renderer: &mut Renderer,
        sub_res: u8,
        current_surface: u32,
        ancient_surface: Option<u32>,
    ) -> bool {
        // Save this one for future use.
        self.base.sub_resource = sub_res;

        // Choose the alpha level.
        let alpha_used = self.increment_sliding();

        if self.base.rendering_surface != 0 {
            tracing::warn!(
                "RendererAlphaConstant::render: GPU alpha path only supports screen destination"
            );
            return false;
        }

        // Compute the dst bbox if a position is set, else blit at (0,0).
        let dst_rect: Option<BBox> = self.base.bbox.0.map(|r| {
            let mn = r.min();
            let mx = r.max();
            BBox::new(pt(mn.x, mn.y), pt(mx.x, mx.y))
        });

        if self.mixing_in_progress {
            let Some(old_surface) = ancient_surface else {
                tracing::warn!(
                    "RendererAlphaConstant::render: mixing in progress but no ancient surface provided"
                );
                return false;
            };

            let current_opacity = (alpha_used as u32)
                .saturating_mul(100u32.saturating_sub(self.mixing_alpha as u32))
                / 100;
            renderer.blit_to_screen_alpha(
                old_surface,
                None,
                dst_rect.as_ref(),
                100u16.saturating_sub(alpha_used),
                BLIT_SOURCE_TRANSPARENT,
            );
            renderer.blit_to_screen_alpha(
                current_surface,
                None,
                dst_rect.as_ref(),
                100u16.saturating_sub(current_opacity as u16),
                BLIT_SOURCE_TRANSPARENT,
            );

            // Stop mixing once the new surface is fully blended in.
            if self.mixing_alpha == 0 {
                self.mixing_in_progress = false;
            }
            return true;
        }

        if current_surface == 0 {
            return true;
        }

        let pair_idx = (self.base.pair_counter % 2) as usize;

        if alpha_used < 100 {
            renderer.blit_to_screen_alpha(
                current_surface,
                None,
                dst_rect.as_ref(),
                100u16.saturating_sub(alpha_used),
                BLIT_SOURCE_TRANSPARENT,
            );
            self.base.last_rendered[pair_idx] = u32::MAX;
        } else {
            renderer.blit_to_screen(
                current_surface,
                None,
                dst_rect.as_ref(),
                BLIT_SOURCE_TRANSPARENT,
            );
            self.base.last_rendered[pair_idx] = current_surface;
        }
        true
    }
}

/// Text renderer — bitmap background + text overlay with four font states.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RendererText {
    pub shadow: RendererShadow,
    /// Font handle for the disabled state.
    pub font_disabled: u32,
    /// Font handle for the default state.
    pub font_default: u32,
    /// Font handle for the focused state.
    pub font_focus: u32,
    /// Font handle for the selected state.
    pub font_selected: u32,
    /// Text alignment flags.
    pub align: u32,
    /// Pixel offset subtracted from the text area.
    pub subtracted: u16,
    /// Text that didn't fit and wasn't rendered.
    pub unrendered_text: String,
    /// Whether to perform a full background refresh.
    pub full_refresh_mode: bool,
}

impl RendererText {
    pub fn set_font(&mut self, disabled: u32, default: u32, focus: u32, selected: u32) {
        self.font_disabled = disabled;
        self.font_default = default;
        self.font_focus = focus;
        self.font_selected = selected;
    }

    pub fn set_alignment(&mut self, align: u32) {
        self.align = align;
    }

    pub fn set_full_refresh_mode(&mut self, full: bool) {
        self.full_refresh_mode = full;
    }

    pub fn set_subtracted(&mut self, value: u16) {
        self.subtracted = value;
    }

    /// Pixel height needed to display the text. Legacy hook; the
    /// concrete widget implementations compute this via
    /// `NativeFont::height` / text layout, so this base stub returns 0.
    pub fn needed_height(&self) -> u32 {
        0
    }
}

/// Listbox item flags.
pub mod listbox_flags {
    /// Indent this item.
    pub const INDENT: u32 = 0x0100;
    /// Alignment mask.
    pub const ALIGN_MASK: u32 = 0x00F0;
    pub const LEFT: u32 = 0x0000;
    pub const RIGHT: u32 = 0x0010;
    pub const CENTER: u32 = 0x0020;
    pub const JUSTIFY: u32 = 0x0030;
    /// State mask.
    pub const STATE_MASK: u32 = 0x000F;
    pub const DEFAULT: u32 = 0x0000;
    pub const FOCUSED: u32 = 0x0001;
    pub const SELECTED: u32 = 0x0002;
    /// Alternate font set.
    pub const ALTERNATE: u32 = 0x0200;
}

/// Alignment for listbox item text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListboxAlignment {
    Left,
    Right,
    Centered,
    Justified,
}

/// Listbox renderer with scrollbar and per-item rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RendererListbox {
    pub base: RendererBase,

    // ── Font sets (default + alternate) ──
    pub font_default: u32,
    pub font_selected: u32,
    pub font_focused: u32,
    pub font_default_alt: u32,
    pub font_selected_alt: u32,
    pub font_focused_alt: u32,

    // ── Font metrics (set when fonts are loaded) ──
    /// Height of one text line in pixels. Must be set via `set_font_height`
    /// after fonts are loaded — needed by all geometry calculations.
    pub font_height: u16,

    // ── Pre-rendered scrollbar surfaces ──
    pub surface_knob: u32,
    pub surface_scrollbar: u32,

    /// Pixel width of the knob (thumb) surface. Set when the knob
    /// surface is created from slider resources.
    pub knob_width: u16,
    /// Pixel width of the scrollbar (track) surface. Set when the
    /// scrollbar surface is created.
    pub scrollbar_track_width: u16,

    // ── Knob (scrollbar thumb) state ──
    pub knob_position: GeoPoint2D,
    pub knob_ratio: f32,
    pub before_ratio: f32,
    pub knob_height: u16,

    // ── List rendering ──
    pub render_position: u16,
    pub number_of_items: u16,
    pub indent_size: u16,
}

impl Default for RendererListbox {
    fn default() -> Self {
        Self::new()
    }
}

impl RendererListbox {
    pub fn new() -> Self {
        Self {
            base: RendererBase::default(),
            font_default: 0,
            font_selected: 0,
            font_focused: 0,
            font_default_alt: 0,
            font_selected_alt: 0,
            font_focused_alt: 0,
            font_height: 0,
            // Sentinels: "surface not yet created".
            surface_knob: u32::MAX,
            surface_scrollbar: u32::MAX,
            knob_width: 0,
            scrollbar_track_width: 0,
            knob_position: GeoPoint2D::default(),
            knob_ratio: 0.0,
            before_ratio: 0.0,
            knob_height: 0,
            render_position: 0,
            number_of_items: 0,
            indent_size: 20,
        }
    }

    /// Set the listbox fonts and validate that the three heights agree.
    /// IDs are assigned first, then heights are checked. On mismatch
    /// the function logs a warning and returns `false`; the IDs
    /// remain assigned regardless.
    ///
    /// Each font argument is a `(font_id, font_height)` tuple — there
    /// is no back-channel from a font ID to its height, so heights
    /// must be supplied by the caller.
    pub fn set_font(
        &mut self,
        default: (u32, u16),
        focused: (u32, u16),
        selected: (u32, u16),
        alternate: bool,
    ) -> bool {
        if alternate {
            self.font_default_alt = default.0;
            self.font_focused_alt = focused.0;
            self.font_selected_alt = selected.0;

            // Each alternate height must match the primary height.
            if default.1 != self.font_height
                || focused.1 != self.font_height
                || selected.1 != self.font_height
            {
                tracing::warn!("Inconsistency in alternate font.");
                return false;
            }
        } else {
            self.font_default = default.0;
            self.font_focused = focused.0;
            self.font_selected = selected.0;
            // Track the primary font height so `font_height` and the
            // alternate-vs-primary check stay in sync.
            self.font_height = default.1;

            // All three primary heights must match.
            if default.1 != focused.1 || focused.1 != selected.1 {
                tracing::warn!("Inconsistency in font.");
                return false;
            }
        }
        true
    }

    /// Set the font line height (in pixels).  Must be called after fonts
    /// are loaded so that geometry calculations work.
    pub fn set_font_height(&mut self, height: u16) {
        self.font_height = height;
    }

    /// Set the knob (thumb) surface width (in pixels). Must be called
    /// after the knob surface is created from slider resources.
    pub fn set_knob_width(&mut self, width: u16) {
        self.knob_width = width;
    }

    /// Set the scrollbar (track) surface width (in pixels). Must be
    /// called after the scrollbar surface is created from slider
    /// resources.
    pub fn set_scrollbar_track_width(&mut self, width: u16) {
        self.scrollbar_track_width = width;
    }

    /// Set scrollbar knob parameters.
    ///
    /// `index` is the first visible item; `total_items` is the total count.
    pub fn set_knob_parameters(&mut self, index: u16, total_items: u16) {
        self.number_of_items = total_items;
        let displayable = self.displayable_item_count();

        if displayable as u16 > total_items {
            // More space than items — full-height knob
            self.knob_ratio = 1.0;
            self.before_ratio = 0.0;
        } else {
            // No guard against `total_items == 0`; with zero items the
            // division produces NaN. Callers are expected to gate empty
            // lists upstream.
            self.knob_ratio = displayable as f32 / total_items as f32;
            self.before_ratio = index as f32 / total_items as f32;
        }
    }

    /// Start a list refresh pass (resets rendering position).
    pub fn start_list_refresh(&mut self) {
        self.render_position = 0;
    }

    /// Render a single list item.  Returns `true` if the next item would
    /// still be inside the bounding box (i.e. there's room for more).
    ///
    /// Decodes alignment from `flags & ALIGN_MASK`, picks the active
    /// font slot from `flags & ALTERNATE | flags & STATE_MASK`, emits
    /// one centred-text draw via `render_text_in_box_aligned`, then
    /// advances the render position and reports whether the next row
    /// still fits.
    ///
    /// `fonts` is a 6-slot table indexed by [`ListboxFontSlot`]:
    /// `[default, focused, selected, default_alt, focused_alt,
    /// selected_alt]`. The `RendererListbox` only stores `u32` font
    /// IDs, so the caller must resolve the IDs to actual glyph data
    /// before calling. When the slot is `None` the row is skipped;
    /// the per-caller "fall back to default font" behaviour is
    /// already handled by
    /// `IngameMenuResources::list_font_native_with_style`.
    pub fn refresh_item(
        &mut self,
        renderer: &mut Renderer,
        transform: MenuTransform,
        fonts: &[Option<&NativeFont>; 6],
        text: &str,
        flags: u32,
    ) -> bool {
        let box_text = self.text_box_for_item(self.render_position, flags);
        let alignment = alignment_for_flags(flags);
        let slot = font_slot_for_flags(flags) as usize;

        if let (Some(rect), Some(font)) = (box_text.0, fonts[slot]) {
            let min = rect.min();
            let max = rect.max();
            let box_x = min.x as i32;
            let box_y = min.y as i32;
            let box_w = (max.x - min.x) as i32;
            let box_h = (max.y - min.y) as i32;
            // Default to vertical centring inside the box.
            let text_align = match alignment {
                ListboxAlignment::Left => TextAlign::Left,
                ListboxAlignment::Right => TextAlign::Right,
                ListboxAlignment::Centered => TextAlign::Center,
                ListboxAlignment::Justified => TextAlign::Justified,
            };
            render_text_in_box_aligned(
                renderer,
                font,
                transform,
                text,
                box_x,
                box_y,
                box_w,
                box_h,
                text_align,
                VAlign::Center,
            );
        }

        self.render_position += 1;
        let next_box = self.text_box_for_item(self.render_position, flags);
        self.base.bbox.contains_bbox(&next_box)
    }

    /// Resolve the active font ID for an item given its flags. Both
    /// `FOCUSED` and `SELECTED` states map to the focused-font slot in
    /// both branches; this aliasing is intentional.
    pub fn font_for_flags(&self, flags: u32) -> u32 {
        if (flags & listbox_flags::ALTERNATE) == 0 {
            match flags & listbox_flags::STATE_MASK {
                listbox_flags::FOCUSED | listbox_flags::SELECTED => self.font_focused,
                _ => self.font_default,
            }
        } else {
            match flags & listbox_flags::STATE_MASK {
                listbox_flags::FOCUSED | listbox_flags::SELECTED => self.font_focused_alt,
                _ => self.font_default_alt,
            }
        }
    }

    /// End the list refresh pass.
    pub fn end_list_refresh(&mut self) -> bool {
        self.render_position = 0;
        true
    }
}

/// Slot indices into the 6-entry font table accepted by
/// [`RendererListbox::refresh_item`]. Order: default, focused,
/// selected, default-alt, focused-alt, selected-alt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ListboxFontSlot {
    Default = 0,
    Focused = 1,
    Selected = 2,
    DefaultAlt = 3,
    FocusedAlt = 4,
    SelectedAlt = 5,
}

/// Decode the active font slot from a listbox item-flags word.
/// `SELECTED` is intentionally aliased to the focused slot in both
/// the primary and alternate paths, so this function never returns
/// the `*Selected` slots — they are preserved here as enum variants
/// for callers that want to populate the table by structural slot.
pub fn font_slot_for_flags(flags: u32) -> ListboxFontSlot {
    if (flags & listbox_flags::ALTERNATE) == 0 {
        match flags & listbox_flags::STATE_MASK {
            listbox_flags::FOCUSED | listbox_flags::SELECTED => ListboxFontSlot::Focused,
            _ => ListboxFontSlot::Default,
        }
    } else {
        match flags & listbox_flags::STATE_MASK {
            listbox_flags::FOCUSED | listbox_flags::SELECTED => ListboxFontSlot::FocusedAlt,
            _ => ListboxFontSlot::DefaultAlt,
        }
    }
}

/// Decode the alignment subfield of a listbox item-flags word.
fn alignment_for_flags(flags: u32) -> ListboxAlignment {
    match flags & listbox_flags::ALIGN_MASK {
        listbox_flags::RIGHT => ListboxAlignment::Right,
        listbox_flags::CENTER => ListboxAlignment::Centered,
        listbox_flags::JUSTIFY => ListboxAlignment::Justified,
        // `LEFT` is the zero value; any unset alignment falls through to it.
        _ => ListboxAlignment::Left,
    }
}

impl RendererListbox {
    /// Calculate how many items can be displayed: bbox height divided
    /// by the default-font line height.
    pub fn displayable_item_count(&self) -> u32 {
        if self.font_height == 0 {
            return 0;
        }
        // The bbox is always expected to be present at this point; a
        // `None` here is a broken lifecycle, not a normal state. Fail
        // loudly rather than fabricate.
        let rect = self
            .base
            .bbox
            .0
            .expect("displayable_item_count called before listbox bbox was set");
        let bbox_height = rect.max().y - rect.min().y;
        (bbox_height / self.font_height as f32) as u32
    }

    /// Get the bounding box of a specific list item.
    ///
    /// ```text
    /// box = (0, fontHeight*index,
    ///        bboxWidth - scrollbarWidth, fontHeight*(index+1))
    /// if INDENT: box.xMin += indentSize
    /// box += bbox.topLeft
    /// ```
    pub fn text_box_for_item(&self, index: u16, flags: u32) -> ScreenBBox {
        // The bbox is always expected to be present at this point; a
        // `None` here is a broken lifecycle, not a normal state. Fail
        // loudly rather than fabricate.
        let rect = self
            .base
            .bbox
            .0
            .expect("text_box_for_item called before listbox bbox was set");
        let min = rect.min();
        let fh = self.font_height as f32;
        let bbox_width = rect.max().x - min.x;

        let mut x_min = 0.0f32;
        let y_min = fh * index as f32;
        // Subtract the scrollbar (track) surface width from the right.
        let x_max = bbox_width - self.scrollbar_track_width as f32;
        let y_max = fh * (index as f32 + 1.0);

        // Apply indent if flagged
        if (flags & listbox_flags::INDENT) != 0 {
            x_min += self.indent_size as f32;
        }

        // Offset by bbox top-left (screen position)
        ScreenBBox::from_coords(x_min + min.x, y_min + min.y, x_max + min.x, y_max + min.y)
    }

    /// Get the current scrollbar knob bounding box.
    ///
    /// ```text
    /// knob_y_start = (height - 2) * beforeRatio
    /// knob_y_end   = (height - 2) * (beforeRatio + knobRatio)
    /// knob_x       = bbox.right - 1 - scrollbarWidth
    /// offset by (knob_x, bbox.top + 1)
    /// ```
    pub fn knob_bbox(&self) -> ScreenBBox {
        // The bbox is always expected to be present at this point; a
        // `None` here is a broken lifecycle, not a normal state. Fail
        // loudly rather than fabricate.
        let rect = self
            .base
            .bbox
            .0
            .expect("knob_bbox called before listbox bbox was set");
        let min = rect.min();
        let max = rect.max();
        let height = max.y - min.y;
        // Use the knob (thumb) surface width here.
        let kw = self.knob_width as f32;

        let y_start = (height - 2.0) * self.before_ratio;
        let y_end = (height - 2.0) * (self.before_ratio + self.knob_ratio);
        let x_start = max.x - 1.0 - kw;

        ScreenBBox::from_coords(
            x_start,
            y_start + min.y + 1.0,
            x_start + kw,
            y_end + min.y + 1.0,
        )
    }

    /// Get the full scrollbar track bounding box.
    ///
    /// ```text
    /// box = (0, 0, scrollbarWidth, bboxHeight)
    /// offset by (bbox.right - scrollbarWidth, bbox.top)
    /// ```
    pub fn scrollbar_bbox(&self) -> ScreenBBox {
        if self.surface_scrollbar == u32::MAX {
            // Return a degenerate `(0,0,0,0)` box here, not a "no box"
            // sentinel — callers expect a real box.
            return ScreenBBox::from_coords(0.0, 0.0, 0.0, 0.0);
        }
        // The bbox is always expected to be present at this point — a
        // `None` here means the listbox lifecycle is broken (refresh
        // ran before geometry was set). Fail loudly.
        let rect = self
            .base
            .bbox
            .0
            .expect("scrollbar_bbox called before listbox bbox was set");
        let min = rect.min();
        let max = rect.max();
        let sw = self.scrollbar_track_width as f32;

        ScreenBBox::from_coords(max.x - sw, min.y, max.x, max.y)
    }

    /// Pixel height to move the knob for one item.
    ///
    /// No zero guard — calling with `number_of_items == 0` would
    /// divide by zero. The bbox must also be set.
    pub fn knob_height_for_one_item(&self) -> u32 {
        debug_assert!(
            self.number_of_items > 0,
            "RendererListbox::knob_height_for_one_item called with number_of_items == 0",
        );
        let rect = self
            .base
            .bbox
            .0
            .expect("RendererListbox::knob_height_for_one_item called before bbox set");
        let height = rect.max().y - rect.min().y;
        ((height - 2.0) / self.number_of_items as f32) as u32
    }
}

// ═════════════════════════════════════════════════════════════════════
//  Layout system
// ═════════════════════════════════════════════════════════════════════

/// Horizontal axis orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HorizontalOrientation {
    #[default]
    LeftToRight,
    RightToLeft,
}

/// Vertical axis orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VerticalOrientation {
    #[default]
    TopDown,
    BottomUp,
}

/// Coordinate mapping direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapType {
    /// Map from physical to logical coordinates.
    Logical,
    /// Map from logical to physical coordinates.
    Physical,
}

/// A 2D layout coordinate system with configurable axis orientation.
///
/// Converts between physical screen coordinates and logical layout
/// coordinates based on an origin point and axis orientations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layout {
    h_orientation: HorizontalOrientation,
    v_orientation: VerticalOrientation,
    physical_origin: GeoPoint2D,
    /// Logical bounding box (stored as raw start/end in logical coords).
    bbox_start: LayoutPoint,
    bbox_end: LayoutPoint,
}

impl Layout {
    /// Create a layout from a physical bounding box and origin.
    pub fn new(
        bbox: &BBox2D,
        origin: GeoPoint2D,
        h_orientation: HorizontalOrientation,
        v_orientation: VerticalOrientation,
    ) -> Self {
        let mut layout = Self {
            h_orientation,
            v_orientation,
            physical_origin: origin,
            bbox_start: LayoutPoint { x: 0.0, y: 0.0 },
            bbox_end: LayoutPoint { x: 0.0, y: 0.0 },
        };
        // Convert the physical bbox corners to logical coordinates.
        if let Some(rect) = bbox.0 {
            let min = rect.min();
            let max = rect.max();
            let mut tl = LayoutPoint::from_physical(&layout, min);
            let mut br = LayoutPoint::from_physical(&layout, max);
            // Ensure start ≤ end.
            if tl.x > br.x {
                std::mem::swap(&mut tl.x, &mut br.x);
            }
            if tl.y > br.y {
                std::mem::swap(&mut tl.y, &mut br.y);
            }
            layout.bbox_start = tl;
            layout.bbox_end = br;
        }
        layout
    }

    /// Map a horizontal coordinate between physical ↔ logical.
    pub fn horizontal_map(&self, value: f32, map_type: MapType) -> f32 {
        match map_type {
            MapType::Logical => match self.h_orientation {
                HorizontalOrientation::LeftToRight => value - self.physical_origin.x,
                HorizontalOrientation::RightToLeft => self.physical_origin.x - value,
            },
            MapType::Physical => match self.h_orientation {
                HorizontalOrientation::LeftToRight => value + self.physical_origin.x,
                HorizontalOrientation::RightToLeft => self.physical_origin.x - value,
            },
        }
    }

    /// Map a vertical coordinate between physical ↔ logical.
    pub fn vertical_map(&self, value: f32, map_type: MapType) -> f32 {
        match map_type {
            MapType::Logical => match self.v_orientation {
                VerticalOrientation::TopDown => value - self.physical_origin.y,
                VerticalOrientation::BottomUp => self.physical_origin.y - value,
            },
            MapType::Physical => match self.v_orientation {
                VerticalOrientation::TopDown => value + self.physical_origin.y,
                VerticalOrientation::BottomUp => self.physical_origin.y - value,
            },
        }
    }

    pub fn width(&self) -> u32 {
        (self.bbox_end.x - self.bbox_start.x) as u32
    }

    pub fn height(&self) -> u32 {
        (self.bbox_end.y - self.bbox_start.y) as u32
    }

    pub fn set_width(&mut self, w: u32) {
        self.bbox_end.x = self.bbox_start.x + w as f32;
    }

    pub fn set_height(&mut self, h: u32) {
        self.bbox_end.y = self.bbox_start.y + h as f32;
    }

    pub fn origin(&self) -> GeoPoint2D {
        self.physical_origin
    }

    pub fn set_origin(&mut self, origin: GeoPoint2D) {
        self.physical_origin = origin;
    }

    pub fn h_orientation(&self) -> HorizontalOrientation {
        self.h_orientation
    }

    pub fn set_h_orientation(&mut self, o: HorizontalOrientation) {
        self.h_orientation = o;
    }

    pub fn v_orientation(&self) -> VerticalOrientation {
        self.v_orientation
    }

    pub fn set_v_orientation(&mut self, o: VerticalOrientation) {
        self.v_orientation = o;
    }

    /// Convert a physical bounding box to a logical bbox within this layout.
    pub fn set_bounding_box_physical(&mut self, bbox: &BBox2D) {
        if let Some(rect) = bbox.0 {
            let min = rect.min();
            let max = rect.max();
            let mut tl = LayoutPoint::from_physical(self, min);
            let mut br = LayoutPoint::from_physical(self, max);
            if tl.x > br.x {
                std::mem::swap(&mut tl.x, &mut br.x);
            }
            if tl.y > br.y {
                std::mem::swap(&mut tl.y, &mut br.y);
            }
            self.bbox_start = tl;
            self.bbox_end = br;
        }
    }

    /// Set bounding box from logical coordinates directly.
    pub fn set_bounding_box_logical(&mut self, start: LayoutPoint, end: LayoutPoint) {
        self.bbox_start = start;
        self.bbox_end = end;
    }

    /// Convert the logical bounding box to a physical `BBox2D`.
    pub fn to_physical_bbox(&self) -> BBox2D {
        let start = self.bbox_start.to_physical(self);
        let end = self.bbox_end.to_physical(self);
        let x0 = start.x.min(end.x);
        let y0 = start.y.min(end.y);
        let x1 = start.x.max(end.x);
        let y1 = start.y.max(end.y);
        BBox2D::from_coords(x0, y0, x1, y1)
    }
}

/// A point in logical layout coordinates.
///
/// This does NOT store a pointer to its parent `Layout`; conversions
/// require passing a `&Layout` reference explicitly.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct LayoutPoint {
    pub x: f32,
    pub y: f32,
}

impl LayoutPoint {
    /// Create a point in logical coordinates directly.
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Convert a physical point to logical coordinates within a layout.
    pub fn from_physical(layout: &Layout, phys: GeoPoint2D) -> Self {
        Self {
            x: layout.horizontal_map(phys.x, MapType::Logical),
            y: layout.vertical_map(phys.y, MapType::Logical),
        }
    }

    /// Convert this logical point back to physical coordinates.
    pub fn to_physical(&self, layout: &Layout) -> GeoPoint2D {
        pt(
            layout.horizontal_map(self.x, MapType::Physical),
            layout.vertical_map(self.y, MapType::Physical),
        )
    }

    /// Offset by a vector.
    pub fn offset(&mut self, dx: f32, dy: f32) {
        self.x += dx;
        self.y += dy;
    }

    /// Test if a point has strictly greater coordinates than another.
    pub fn is_greater_than(&self, other: &LayoutPoint) -> bool {
        self.x > other.x && self.y > other.y
    }

    /// Test if this point falls inside a layout's bounding box.
    pub fn is_inside_layout(&self, layout: &Layout) -> bool {
        let phys = self.to_physical(layout);
        layout.to_physical_bbox().contains_point(phys)
    }
}

/// A rectangle in logical layout coordinates.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct LayoutBox {
    pub start: LayoutPoint,
    pub end: LayoutPoint,
}

impl LayoutBox {
    /// Create from two logical corner points, normalizing order.
    pub fn new(mut start: LayoutPoint, mut end: LayoutPoint) -> Self {
        if start.x > end.x {
            std::mem::swap(&mut start.x, &mut end.x);
        }
        if start.y > end.y {
            std::mem::swap(&mut start.y, &mut end.y);
        }
        Self { start, end }
    }

    /// Create from a start point and dimensions.
    pub fn from_point_size(start: LayoutPoint, width: f32, height: f32) -> Self {
        let end = LayoutPoint::new(start.x + width, start.y + height);
        Self::new(start, end)
    }

    /// Create from four physical scalars, mapping each corner into
    /// logical coords and normalizing per-axis.
    pub fn from_physical_rect(
        layout: &Layout,
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
    ) -> Self {
        let start = LayoutPoint::from_physical(layout, pt(left, top));
        let end = LayoutPoint::from_physical(layout, pt(right, bottom));
        Self::new(start, end)
    }

    /// Create from a physical bounding box, converting to logical coords.
    pub fn from_physical_bbox(layout: &Layout, bbox: &BBox2D) -> Self {
        if let Some(rect) = bbox.0 {
            let start = LayoutPoint::from_physical(layout, rect.min());
            let end = LayoutPoint::from_physical(layout, rect.max());
            Self::new(start, end)
        } else {
            Self::default()
        }
    }

    /// Convert to physical `BBox2D`.
    pub fn to_physical_bbox(&self, layout: &Layout) -> BBox2D {
        let start = self.start.to_physical(layout);
        let end = self.end.to_physical(layout);
        let x0 = start.x.min(end.x);
        let y0 = start.y.min(end.y);
        let x1 = start.x.max(end.x);
        let y1 = start.y.max(end.y);
        BBox2D::from_coords(x0, y0, x1, y1)
    }

    pub fn width(&self) -> u32 {
        (self.end.x - self.start.x) as u32
    }

    pub fn height(&self) -> u32 {
        (self.end.y - self.start.y) as u32
    }

    pub fn set_width(&mut self, w: u32) {
        self.end.x = self.start.x + w as f32;
    }

    pub fn set_height(&mut self, h: u32) {
        self.end.y = self.start.y + h as f32;
    }

    /// Clip (intersect) with another layout box.
    pub fn clip(&self, other: &LayoutBox) -> LayoutBox {
        LayoutBox {
            start: LayoutPoint {
                x: self.start.x.max(other.start.x),
                y: self.start.y.max(other.start.y),
            },
            end: LayoutPoint {
                x: self.end.x.min(other.end.x),
                y: self.end.y.min(other.end.y),
            },
        }
    }

    /// Offset by a vector.
    pub fn offset(&mut self, dx: f32, dy: f32) {
        self.start.offset(dx, dy);
        self.end.offset(dx, dy);
    }

    /// Test if both corners lie inside the layout's bounding box.
    pub fn is_inside_layout(&self, layout: &Layout) -> bool {
        self.start.is_inside_layout(layout) && self.end.is_inside_layout(layout)
    }
}

// ═════════════════════════════════════════════════════════════════════
//  Tests
// ═════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── UiKeyboard tests ──

    fn make_keys(pressed: &[KeyCode]) -> KeyboardState {
        let mut ks = KeyboardState::default();
        for &key in pressed {
            ks.keys.insert(key);
        }
        ks
    }

    #[test]
    fn keyboard_first_refresh_initializes() {
        let mut kb = UiKeyboard::default();
        let ks = make_keys(&[KeyCode::KeyA]);
        assert!(!kb.refresh(&ks, 0));
        // Not initialized until second call.
        assert!(!kb.has_changed());
    }

    #[test]
    fn keyboard_key_down_detected() {
        let mut kb = UiKeyboard::default();
        kb.refresh(&make_keys(&[]), 0);

        kb.refresh(&make_keys(&[KeyCode::Backspace]), 100);
        assert!(kb.has_changed());
        assert_eq!(kb.get_state_of_key(KeyCode::Backspace), KeyState::KeyDown);
    }

    #[test]
    fn keyboard_key_pressed_on_release() {
        let mut kb = UiKeyboard::default();
        kb.refresh(&make_keys(&[]), 0);

        // Press key
        kb.refresh(&make_keys(&[KeyCode::Backspace]), 100);
        assert_eq!(kb.get_state_of_key(KeyCode::Backspace), KeyState::KeyDown);

        // Release key → KeyPressed
        kb.refresh(&make_keys(&[]), 700);
        assert_eq!(
            kb.get_state_of_key(KeyCode::Backspace),
            KeyState::KeyPressed
        );

        // Next frame → KeyUp (transient state cleaned up)
        kb.refresh(&make_keys(&[]), 800);
        assert_eq!(kb.get_state_of_key(KeyCode::Backspace), KeyState::KeyUp);
    }

    #[test]
    fn keyboard_double_press_within_delay() {
        let mut kb = UiKeyboard::new(500); // 500ms double-press window
        // Use timestamps well past 0 so the initial last_key_press (0) is
        // outside the double-press window — matches real SDL_GetTicks() usage.
        kb.refresh(&make_keys(&[]), 10_000);

        // First press + release
        kb.refresh(&make_keys(&[KeyCode::KeyA]), 10_100);
        kb.refresh(&make_keys(&[]), 10_200);
        assert_eq!(kb.get_state_of_key(KeyCode::KeyA), KeyState::KeyPressed);

        // Consume the pressed state
        kb.refresh(&make_keys(&[]), 10_250);

        // Second press + release within 500ms of first release
        kb.refresh(&make_keys(&[KeyCode::KeyA]), 10_300);
        kb.refresh(&make_keys(&[]), 10_400);
        assert_eq!(kb.get_state_of_key(KeyCode::KeyA), KeyState::KeyDouble);
    }

    #[test]
    fn keyboard_no_double_press_outside_delay() {
        let mut kb = UiKeyboard::new(500);
        kb.refresh(&make_keys(&[]), 10_000);

        // First press + release
        kb.refresh(&make_keys(&[KeyCode::KeyA]), 10_100);
        kb.refresh(&make_keys(&[]), 10_200);
        kb.refresh(&make_keys(&[]), 10_250);

        // Second press + release AFTER 500ms from first release
        kb.refresh(&make_keys(&[KeyCode::KeyA]), 10_800);
        kb.refresh(&make_keys(&[]), 10_900);
        assert_eq!(kb.get_state_of_key(KeyCode::KeyA), KeyState::KeyPressed);
    }

    #[test]
    fn keyboard_typewriter_repeat() {
        let mut kb = UiKeyboard::default();
        kb.refresh(&make_keys(&[]), 0);

        // Press key
        kb.refresh(&make_keys(&[KeyCode::KeyB]), 100);
        assert_eq!(kb.get_typewriter_state(KeyCode::KeyB), TypeWriter::None);

        // Hold — transitions to Touch
        kb.refresh(&make_keys(&[KeyCode::KeyB]), 200);
        assert_eq!(kb.get_typewriter_state(KeyCode::KeyB), TypeWriter::Touch);

        // Hold past REPEAT_FIRST (400ms) → Repeat
        kb.refresh(&make_keys(&[KeyCode::KeyB]), 550);
        assert_eq!(kb.get_typewriter_state(KeyCode::KeyB), TypeWriter::Repeat);

        // Next frame → Waiting
        kb.refresh(&make_keys(&[KeyCode::KeyB]), 560);
        assert_eq!(kb.get_typewriter_state(KeyCode::KeyB), TypeWriter::Waiting);

        // Wait past REPEAT_AFTER (50ms) → Repeat again
        kb.refresh(&make_keys(&[KeyCode::KeyB]), 620);
        assert_eq!(kb.get_typewriter_state(KeyCode::KeyB), TypeWriter::Repeat);
    }

    #[test]
    fn keyboard_has_key_changed() {
        let mut kb = UiKeyboard::default();
        kb.refresh(&make_keys(&[]), 0);

        kb.refresh(&make_keys(&[KeyCode::Digit5]), 100);
        assert!(kb.has_key_changed(KeyCode::Digit5));
        assert!(!kb.has_key_changed(KeyCode::Digit6));
    }

    #[test]
    fn keyboard_reset() {
        let mut kb = UiKeyboard::default();
        kb.refresh(&make_keys(&[]), 0);
        kb.refresh(&make_keys(&[KeyCode::KeyA]), 100);
        kb.reset();
        assert!(kb.has_changed()); // reset sets changed = true
    }

    // ── Layout tests ──

    #[test]
    fn layout_left_to_right_top_down() {
        let bbox = BBox2D::from_coords(0.0, 0.0, 100.0, 100.0);
        let layout = Layout::new(
            &bbox,
            pt(10.0, 20.0),
            HorizontalOrientation::LeftToRight,
            VerticalOrientation::TopDown,
        );

        // Physical 50 → logical 50 - 10 = 40
        assert_eq!(layout.horizontal_map(50.0, MapType::Logical), 40.0);
        // Logical 40 → physical 40 + 10 = 50
        assert_eq!(layout.horizontal_map(40.0, MapType::Physical), 50.0);

        // Physical 60 → logical 60 - 20 = 40
        assert_eq!(layout.vertical_map(60.0, MapType::Logical), 40.0);
        // Logical 40 → physical 40 + 20 = 60
        assert_eq!(layout.vertical_map(40.0, MapType::Physical), 60.0);
    }

    #[test]
    fn layout_right_to_left() {
        let bbox = BBox2D::from_coords(0.0, 0.0, 100.0, 100.0);
        let layout = Layout::new(
            &bbox,
            pt(100.0, 0.0),
            HorizontalOrientation::RightToLeft,
            VerticalOrientation::TopDown,
        );

        // Physical 80 → logical 100 - 80 = 20
        assert_eq!(layout.horizontal_map(80.0, MapType::Logical), 20.0);
        // Logical 20 → physical 100 - 20 = 80
        assert_eq!(layout.horizontal_map(20.0, MapType::Physical), 80.0);
    }

    #[test]
    fn layout_bottom_up() {
        let bbox = BBox2D::from_coords(0.0, 0.0, 100.0, 100.0);
        let layout = Layout::new(
            &bbox,
            pt(0.0, 100.0),
            HorizontalOrientation::LeftToRight,
            VerticalOrientation::BottomUp,
        );

        // Physical 30 → logical 100 - 30 = 70
        assert_eq!(layout.vertical_map(30.0, MapType::Logical), 70.0);
        // Logical 70 → physical 100 - 70 = 30
        assert_eq!(layout.vertical_map(70.0, MapType::Physical), 30.0);
    }

    #[test]
    fn layout_point_roundtrip() {
        let bbox = BBox2D::from_coords(0.0, 0.0, 200.0, 200.0);
        let layout = Layout::new(
            &bbox,
            pt(50.0, 50.0),
            HorizontalOrientation::LeftToRight,
            VerticalOrientation::TopDown,
        );

        let phys = pt(120.0, 80.0);
        let logical = LayoutPoint::from_physical(&layout, phys);
        let back = logical.to_physical(&layout);
        assert!((back.x - phys.x).abs() < 1e-6);
        assert!((back.y - phys.y).abs() < 1e-6);
    }

    #[test]
    fn layout_point_roundtrip_rtl_bu() {
        let bbox = BBox2D::from_coords(0.0, 0.0, 200.0, 200.0);
        let layout = Layout::new(
            &bbox,
            pt(200.0, 200.0),
            HorizontalOrientation::RightToLeft,
            VerticalOrientation::BottomUp,
        );

        let phys = pt(50.0, 80.0);
        let logical = LayoutPoint::from_physical(&layout, phys);
        let back = logical.to_physical(&layout);
        assert!((back.x - phys.x).abs() < 1e-6);
        assert!((back.y - phys.y).abs() < 1e-6);
    }

    #[test]
    fn layout_box_clip() {
        let a = LayoutBox::new(LayoutPoint::new(10.0, 10.0), LayoutPoint::new(50.0, 50.0));
        let b = LayoutBox::new(LayoutPoint::new(30.0, 30.0), LayoutPoint::new(70.0, 70.0));
        let clipped = a.clip(&b);
        assert_eq!(clipped.start.x, 30.0);
        assert_eq!(clipped.start.y, 30.0);
        assert_eq!(clipped.end.x, 50.0);
        assert_eq!(clipped.end.y, 50.0);
    }

    #[test]
    fn layout_box_dimensions() {
        let b = LayoutBox::new(LayoutPoint::new(10.0, 20.0), LayoutPoint::new(110.0, 120.0));
        assert_eq!(b.width(), 100);
        assert_eq!(b.height(), 100);
    }

    // ── Renderer tests ──

    #[test]
    fn renderer_base_defaults() {
        let r = RendererBase::default();
        assert_eq!(r.resource_id, -1);
        assert_eq!(r.last_rendered, [u32::MAX; 2]);
        assert!(r.alpha_mask.is_none());
    }

    #[test]
    fn renderer_base_is_real_point_bbox_only() {
        let mut r = RendererBase::default();
        r.set_position_bbox(ScreenBBox::from_coords(10.0, 10.0, 30.0, 30.0));
        assert!(r.is_real_point(ScreenPoint::new(15.0, 15.0)));
        assert!(!r.is_real_point(ScreenPoint::new(5.0, 5.0)));
        // Without a mask, every in-bbox pixel is opaque.
        assert!(r.is_real_point(ScreenPoint::new(10.0, 10.0)));
    }

    #[test]
    fn renderer_base_is_real_point_with_mask() {
        // 4x4 surface, color-key = 0x07C0; pixel (1,1) is opaque,
        // everything else is keyed transparent.
        const KEY: u16 = 0x07C0;
        let mut pixels = vec![KEY; 16];
        pixels[5] = 0x1234;
        let mask = AlphaMask::from_pixels(4, 4, 4, &pixels, KEY);

        let mut r = RendererBase::default();
        r.set_position_bbox(ScreenBBox::from_coords(10.0, 20.0, 14.0, 24.0));
        r.set_alpha_mask(Some(mask));

        // bbox top-left = (10, 20); only local (1, 1) is opaque.
        assert!(r.is_real_point(ScreenPoint::new(11.0, 21.0)));
        assert!(!r.is_real_point(ScreenPoint::new(10.0, 20.0)));
        assert!(!r.is_real_point(ScreenPoint::new(13.0, 23.0)));
        // Outside the bbox: rejected before the mask check.
        assert!(!r.is_real_point(ScreenPoint::new(50.0, 50.0)));
    }

    #[test]
    fn renderer_alpha_constant_is_real_point_short_circuits() {
        let mut r = RendererAlphaConstant::default();
        r.base
            .set_position_bbox(ScreenBBox::from_coords(0.0, 0.0, 10.0, 10.0));
        assert!(r.is_real_point(ScreenPoint::new(5.0, 5.0)));
        r.set_alpha_level(0);
        assert!(!r.is_real_point(ScreenPoint::new(5.0, 5.0)));
    }

    #[test]
    fn renderer_alpha_sliding_up() {
        // Contract: target=100, increasing from initial 0.
        // Per-step delta is the hardcoded SBRENDERER_ALPHA_INCREMENT (9).
        let mut r = RendererAlphaConstant::default();
        r.set_sliding_alpha_level(100, true, 0, 0);

        assert_eq!(r.increment_sliding(), 9);
        assert_eq!(r.increment_sliding(), 18);
        // Drive to completion: 11 steps of +9 from 0 = 99, 12th hits 108
        // which clamps to target=100 and flips alpha_reached.
        for _ in 0..10 {
            r.increment_sliding();
        }
        assert!(r.alpha_reached);
        assert_eq!(r.sliding_alpha, 100);
        // After alpha_reached, increment_sliding returns the target unchanged.
        let prev = r.sliding_alpha;
        let v = r.increment_sliding();
        assert_eq!(v, r.target_alpha);
        assert_eq!(r.sliding_alpha, prev);
    }

    #[test]
    fn renderer_alpha_sliding_down_to_nonzero_target() {
        // Decrement target need not be 0; the slide stops at sliding < target.
        let mut r = RendererAlphaConstant::default();
        r.set_sliding_alpha_level(50, false, 100, 0);

        // Per-step delta is SBRENDERER_ALPHA_DECREMENT (7).
        assert_eq!(r.increment_sliding(), 93);
        assert_eq!(r.increment_sliding(), 86);
        for _ in 0..10 {
            r.increment_sliding();
        }
        assert!(r.alpha_reached);
        assert_eq!(r.sliding_alpha, 50);
    }

    #[test]
    fn renderer_alpha_sliding_wait() {
        // wait counter holds the slide for `wait` calls before it advances.
        let mut r = RendererAlphaConstant::default();
        r.set_sliding_alpha_level(100, true, 0, 2);

        // Two ticks decrement wait; sliding_alpha stays at 0.
        assert_eq!(r.increment_sliding(), 0);
        assert_eq!(r.increment_sliding(), 0);
        // Third tick advances by SBRENDERER_ALPHA_INCREMENT.
        assert_eq!(r.increment_sliding(), 9);
    }

    #[test]
    fn renderer_alpha_default_state() {
        // Default: target=100, alpha_reached=true, sliding_alpha=0.
        let r = RendererAlphaConstant::default();
        assert_eq!(r.target_alpha, 100);
        assert!(r.alpha_reached);
        assert_eq!(r.sliding_alpha, 0);
        assert_eq!(r.mix_surface, u32::MAX);
        assert_eq!(r.ancient_resource, -1);
    }

    #[test]
    fn renderer_alpha_mixing() {
        let mut r = RendererAlphaConstant::default();
        r.base.flags = RENDERER_RESOURCE_MIXING;
        r.base.set_resource(10);

        // Changing resource with RESOURCE_MIXING flag triggers mixing
        r.set_resource(20);
        assert!(r.mixing_in_progress);
        assert_eq!(r.ancient_resource, 10);

        r.clear_mixing();
        assert!(!r.mixing_in_progress);
    }

    #[test]
    fn renderer_listbox_defaults() {
        let lb = RendererListbox::new();
        assert_eq!(lb.indent_size, 20);
        assert_eq!(lb.font_height, 0);
        assert_eq!(lb.knob_width, 0);
        assert_eq!(lb.scrollbar_track_width, 0);
        // Sentinels: "surface not yet created".
        assert_eq!(lb.surface_knob, u32::MAX);
        assert_eq!(lb.surface_scrollbar, u32::MAX);
    }

    #[test]
    fn renderer_listbox_displayable_items() {
        let mut lb = RendererListbox::new();
        lb.base.bbox = ScreenBBox::from_coords(0.0, 0.0, 200.0, 100.0);
        lb.set_font_height(20);
        assert_eq!(lb.displayable_item_count(), 5); // 100 / 20

        lb.set_font_height(0);
        assert_eq!(lb.displayable_item_count(), 0); // guard against zero
    }

    #[test]
    fn renderer_listbox_knob_params() {
        let mut lb = RendererListbox::new();
        lb.base.bbox = ScreenBBox::from_coords(0.0, 0.0, 200.0, 100.0);
        lb.set_font_height(20);
        lb.set_scrollbar_track_width(16);

        // 50 items, starting at index 10
        lb.set_knob_parameters(10, 50);
        assert_eq!(lb.number_of_items, 50);
        // knob_ratio = 5/50 = 0.1, before_ratio = 10/50 = 0.2
        assert!((lb.knob_ratio - 0.1).abs() < 1e-6);
        assert!((lb.before_ratio - 0.2).abs() < 1e-6);
    }

    #[test]
    fn renderer_listbox_knob_params_few_items() {
        let mut lb = RendererListbox::new();
        lb.base.bbox = ScreenBBox::from_coords(0.0, 0.0, 200.0, 100.0);
        lb.set_font_height(20);
        // Only 3 items, but can display 5 → full knob
        lb.set_knob_parameters(0, 3);
        assert!((lb.knob_ratio - 1.0).abs() < 1e-6);
        assert!((lb.before_ratio - 0.0).abs() < 1e-6);
    }

    #[test]
    fn renderer_listbox_text_box() {
        let mut lb = RendererListbox::new();
        lb.base.bbox = ScreenBBox::from_coords(10.0, 20.0, 210.0, 120.0);
        lb.set_font_height(15);
        lb.set_scrollbar_track_width(16);

        // Item 0, no flags
        let b0 = lb.text_box_for_item(0, 0);
        let r0 = b0.0.unwrap();
        assert!((r0.min().x - 10.0).abs() < 1e-6); // bbox left
        assert!((r0.min().y - 20.0).abs() < 1e-6); // bbox top
        assert!((r0.max().x - (10.0 + 200.0 - 16.0)).abs() < 1e-6); // bbox left + width - scrollbar
        assert!((r0.max().y - 35.0).abs() < 1e-6); // bbox top + fontHeight

        // Item 2, with indent
        let b2 = lb.text_box_for_item(2, listbox_flags::INDENT);
        let r2 = b2.0.unwrap();
        assert!((r2.min().x - (10.0 + 20.0)).abs() < 1e-6); // indented by 20
        assert!((r2.min().y - (20.0 + 30.0)).abs() < 1e-6); // 2 * 15 offset
    }

    #[test]
    fn renderer_listbox_scrollbar_bbox() {
        let mut lb = RendererListbox::new();
        lb.base.bbox = ScreenBBox::from_coords(0.0, 0.0, 200.0, 100.0);
        lb.set_scrollbar_track_width(16);
        lb.surface_scrollbar = 0; // not 0xFFFFFFFF

        let sb = lb.scrollbar_bbox();
        let r = sb.0.unwrap();
        assert!((r.min().x - 184.0).abs() < 1e-6); // 200 - 16
        assert!((r.min().y - 0.0).abs() < 1e-6);
        assert!((r.max().x - 200.0).abs() < 1e-6);
        assert!((r.max().y - 100.0).abs() < 1e-6);
    }

    #[test]
    fn renderer_listbox_scrollbar_uninitialized() {
        let lb = RendererListbox::new();
        let sb = lb.scrollbar_bbox();
        // Returns degenerate `(0,0,0,0)` here, not "no box".
        let r = sb.0.expect("expected degenerate (0,0,0,0) box");
        assert_eq!(r.min().x, 0.0);
        assert_eq!(r.min().y, 0.0);
        assert_eq!(r.max().x, 0.0);
        assert_eq!(r.max().y, 0.0);
    }

    #[test]
    fn renderer_listbox_knob_bbox() {
        let mut lb = RendererListbox::new();
        lb.base.bbox = ScreenBBox::from_coords(0.0, 0.0, 200.0, 100.0);
        lb.set_knob_width(16);
        lb.before_ratio = 0.0;
        lb.knob_ratio = 0.5;

        let kb = lb.knob_bbox();
        let r = kb.0.unwrap();
        // x: 200 - 1 - 16 = 183
        assert!((r.min().x - 183.0).abs() < 1e-6);
        // y_start: (100-2)*0.0 + 0 + 1 = 1
        assert!((r.min().y - 1.0).abs() < 1e-6);
        // y_end: (100-2)*0.5 + 0 + 1 = 50
        assert!((r.max().y - 50.0).abs() < 1e-6);
    }

    #[test]
    fn renderer_listbox_knob_height_for_one_item() {
        let mut lb = RendererListbox::new();
        lb.base.bbox = ScreenBBox::from_coords(0.0, 0.0, 200.0, 102.0);
        lb.number_of_items = 10;
        // (102 - 2) / 10 = 10
        assert_eq!(lb.knob_height_for_one_item(), 10);
    }

    // ── Serde roundtrip tests ──

    #[test]
    fn serde_ui_msg_roundtrip() {
        let msg = UiMsg::WidgetDoubleClicked;
        let json = serde_json::to_string(&msg).unwrap();
        let back: UiMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn serde_ui_event_roundtrip() {
        let ev = UiEvent {
            msg_type: UiMsg::WidgetActivated,
            origin_widget_id: 42,
            data: Some(UiEventData::SliderPosition(0.75)),
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: UiEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.msg_type, UiMsg::WidgetActivated);
        assert_eq!(back.origin_widget_id, 42);
    }

    #[test]
    fn serde_layout_roundtrip() {
        let bbox = BBox2D::from_coords(0.0, 0.0, 100.0, 100.0);
        let layout = Layout::new(
            &bbox,
            pt(10.0, 20.0),
            HorizontalOrientation::LeftToRight,
            VerticalOrientation::TopDown,
        );
        let json = serde_json::to_string(&layout).unwrap();
        let back: Layout = serde_json::from_str(&json).unwrap();
        assert_eq!(back.width(), layout.width());
        assert_eq!(back.height(), layout.height());
    }
}
