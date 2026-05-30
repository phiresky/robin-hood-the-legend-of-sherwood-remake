//! Widget system: interactive UI elements with state-machine input handling.

mod button;
mod frame_wnd;
mod input_field;
mod label;
mod listbox;
mod picture;
mod radio;
mod slider;
mod toggle;

// Sim-side widget state (campaign-derived derived values) lives in
// `robin_engine::widget_state` now (Decision 7B). Re-export for
// compatibility.
pub use robin_engine::widget_state::blazon_bar;
pub use robin_engine::widget_state::requirements;

pub use button::WidgetButton;
pub use frame_wnd::FrameWnd;
pub use input_field::{TextFromCaretSide, WidgetInputField};
pub use label::WidgetLabel;
pub use listbox::{ColumnAlign, ColumnLayout, LayoutCell, WidgetListbox};
pub use picture::{WidgetMultiPicture, WidgetPicture};
pub use radio::WidgetRadioButton;
pub use slider::WidgetSlider;
pub use toggle::WidgetToggleButton;

use serde::{Deserialize, Serialize};

use crate::geo2d::BBox2D;
use crate::ui::{
    MouseButtons, ProbeCode, RendererAlphaConstant, RendererBase, RendererBitmap, RendererListbox,
    RendererShadow, RendererText, ResourceId, UiEvent, UiEventData, UiMsg, UiProbe, UiState,
    resource_widget_id,
};
use robin_engine::coordinates::ScreenPoint;

// ─── Widget ID ──────────────────────────────────────────────────────

/// Unique identifier for a widget within a frame window.
pub type WidgetId = u32;

/// Sentinel value for "no widget".
pub const WIDGET_ID_NONE: WidgetId = u32::MAX;

// ─── Input context ──────────────────────────────────────────────────

/// Interior-mutable slot that `process_input` writes into to request
/// mouse-capture (re)assignment. A widget in a pushed state calls
/// `set(id)` so the owning UI keeps routing input to it while the
/// mouse wanders outside its bbox, and `clear()` on click-release.
///
/// FrameWnd currently dispatches to every widget every frame regardless
/// of capture, so this is informational — callers that care (e.g. a
/// modal loop that wants to freeze sibling widgets while one has a
/// drag-lock) can read `get()` after `process_input`.
#[derive(Debug, Default)]
pub struct CaptureSlot(std::cell::Cell<Option<WidgetId>>);

impl CaptureSlot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self) -> Option<WidgetId> {
        self.0.get()
    }

    pub fn set(&self, id: WidgetId) {
        self.0.set(Some(id));
    }

    pub fn clear(&self) {
        self.0.set(None);
    }
}

/// Input state passed to widgets during `process_input`.
///
/// Unlike the serializable `ui::UiInput`, this carries references
/// for use within a single frame's input processing pass.
///
/// `text_input` carries UTF-8 characters produced by the SDL3
/// `SDL_EVENT_TEXT_INPUT` stream (via [`crate::gfx_types::GameEvent::TextInput`])
/// since the previous frame. SDL delivers only the committed text from
/// the platform IME — dead-key composition, non-Latin layouts, and IME
/// candidate selection all resolve before the characters reach us, so
/// editable widgets can insert them directly at the caret without
/// re-implementing layout decoding.
///
/// `capture` is the optional slot widgets write into for mouse capture;
/// see [`CaptureSlot`]. Callers that don't care about capture pass
/// `None`.
pub struct WidgetInput<'a> {
    pub mouse_position: ScreenPoint,
    pub mouse_z: i16,
    pub mouse_button: MouseButtons,
    pub keyboard: &'a crate::ui::UiKeyboard,
    pub text_input: &'a str,
    pub capture: Option<&'a CaptureSlot>,
}

// ─── Renderer wrapper ───────────────────────────────────────────────

/// Renderer variant attached to a widget.
///
/// Implemented as an enum to allow heterogeneous widget storage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum WidgetRenderer {
    #[default]
    None,
    Bitmap(RendererBitmap),
    Shadow(RendererShadow),
    Alpha(RendererAlphaConstant),
    Text(RendererText),
    Listbox(RendererListbox),
}

impl WidgetRenderer {
    /// Get a reference to the underlying [`RendererBase`].
    pub fn base(&self) -> Option<&RendererBase> {
        match self {
            Self::None => None,
            Self::Bitmap(r) => Some(&r.base),
            Self::Shadow(r) => Some(&r.base),
            Self::Alpha(r) => Some(&r.base),
            Self::Text(r) => Some(&r.shadow.base),
            Self::Listbox(r) => Some(&r.base),
        }
    }

    /// Get a mutable reference to the underlying [`RendererBase`].
    pub fn base_mut(&mut self) -> Option<&mut RendererBase> {
        match self {
            Self::None => None,
            Self::Bitmap(r) => Some(&mut r.base),
            Self::Shadow(r) => Some(&mut r.base),
            Self::Alpha(r) => Some(&mut r.base),
            Self::Text(r) => Some(&mut r.shadow.base),
            Self::Listbox(r) => Some(&mut r.base),
        }
    }

    /// Hit-test a point against the renderer's area.
    ///
    /// For an `Alpha` renderer this short-circuits to `false` whenever
    /// the current alpha level is 0 — a fully-faded widget cannot
    /// receive clicks. Otherwise routes through
    /// `RendererBase::is_real_point`, which performs a per-pixel
    /// transparency test against the widget's surface (honouring an
    /// attached `AlphaMask` if the wiring layer baked one from the
    /// bound sprite — see `widget_bridge::attach_alpha_masks`).
    pub fn is_real_point(&self, point: ScreenPoint) -> bool {
        match self {
            Self::Alpha(r) => r.is_real_point(point),
            _ => self.base().is_some_and(|b| b.is_real_point(point)),
        }
    }

    /// Set the bounding box on the underlying renderer.
    pub fn set_position(&mut self, bbox: BBox2D) {
        if let Some(b) = self.base_mut() {
            b.set_position_bbox(bbox);
        }
    }

    /// Set the resource ID on the underlying renderer.
    pub fn set_resource(&mut self, id: ResourceId) -> bool {
        self.base_mut().is_some_and(|b| b.set_resource(id))
    }

    /// Set text on the underlying renderer.
    pub fn set_text(&mut self, text: &str) {
        if let Some(b) = self.base_mut() {
            b.set_text(text);
        }
    }

    /// Dismiss (release) the current resource.
    pub fn dismiss_resource(&mut self) {
        if let Some(b) = self.base_mut() {
            b.dismiss_resource();
        }
    }

    /// Render with the given sub-resource ID.
    pub fn render(&mut self, sub_res: u8) -> bool {
        self.base_mut().is_some_and(|b| b.render(sub_res))
    }

    /// Attach to a rendering surface.
    pub fn attach_to_display(&mut self, surface: u32) {
        if let Some(b) = self.base_mut() {
            b.rendering_surface = surface;
        }
    }

    /// Set the double-buffer pair counter.
    pub fn set_counter(&mut self, counter: u32) {
        if let Some(b) = self.base_mut() {
            b.set_counter(counter);
        }
    }

    /// Reset saved state for full refresh.
    pub fn reset_save(&mut self) {
        if let Some(b) = self.base_mut() {
            b.reset_save();
        }
    }
}

// ─── Widget base ────────────────────────────────────────────────────

/// Common widget state shared by all widget types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetBase {
    /// Unique identifier within the owning frame window.
    pub id: WidgetId,
    /// Whether the widget accepts input.
    pub enabled: bool,
    /// Whether the widget can display a focus indicator.
    pub with_focus: bool,
    /// Whether the widget renders in its default (unfocused) state.
    pub with_default: bool,
    /// Whether `create()` has been called.
    pub created: bool,
    /// Accelerator key scancode (0 = none).
    pub fast_key: u16,
    /// Creation flags.
    pub flags: u32,
    /// Widget text (button label, input text, etc.).
    pub text: String,
    /// Tooltip text (empty = no tooltip).
    pub tooltip_text: String,
    /// Position and size in screen coordinates.
    pub bbox: BBox2D,
    /// Current interaction state.
    pub state: UiState,
    /// Renderer for visual output.
    pub renderer: WidgetRenderer,
    /// Opaque handle to the rendering surface.
    pub rendering_surface: u32,
}

impl Default for WidgetBase {
    fn default() -> Self {
        Self {
            id: WIDGET_ID_NONE,
            enabled: true,
            with_focus: true,
            with_default: true,
            created: false,
            fast_key: 0,
            flags: 0,
            text: String::new(),
            tooltip_text: String::new(),
            bbox: BBox2D::new(),
            state: UiState::Default,
            renderer: WidgetRenderer::None,
            rendering_surface: u32::MAX,
        }
    }
}

impl WidgetBase {
    /// Initialize the widget.
    pub fn create(&mut self, text: &str, bbox: BBox2D, flags: u32) {
        self.text = text.to_string();
        self.bbox = bbox;
        self.flags = flags;
        self.created = true;
        // `create` deliberately does **not** touch `state` — re-creating
        // a widget preserves any focused/selected/pushed state.
        // `Default` initialises `state` to `UiState::Default`, so the
        // first-call path is unaffected.
    }

    /// Initialize with a resource.
    pub fn create_with_resource(
        &mut self,
        text: &str,
        bbox: BBox2D,
        flags: u32,
        resource_id: ResourceId,
    ) {
        self.create(text, bbox, flags);
        self.renderer.set_resource(resource_id);
        self.renderer.set_position(bbox);
        self.renderer.set_text(text);
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.renderer.set_text(text);
    }

    pub fn set_tooltip_text(&mut self, text: &str) {
        self.tooltip_text = text.to_string();
    }

    pub fn has_tooltip(&self) -> bool {
        !self.tooltip_text.is_empty()
    }

    pub fn set_accelerator(&mut self, key: u16) {
        self.fast_key = key;
    }

    pub fn set_position(&mut self, bbox: BBox2D) {
        self.bbox = bbox;
        self.renderer.set_position(bbox);
    }

    pub fn set_position_point(&mut self, point: ScreenPoint) {
        // Translate the existing bounding box so its top-left sits at
        // `point`, preserving width/height. If the widget has no bbox
        // yet (hyperspace), fall back to a 1×1 at `point` so we stay
        // compatible with callers that set the position before sizing.
        self.bbox = match self.bbox.0 {
            Some(_) => {
                BBox2D::from_point_size(point.to_geo(), self.bbox.width(), self.bbox.height())
            }
            None => BBox2D::from_point(point.to_geo()),
        };
        self.renderer.set_position(self.bbox);
    }

    pub fn set_enable(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn set_focus_enabled(&mut self, enabled: bool) {
        self.with_focus = enabled;
    }

    pub fn set_default_enabled(&mut self, enabled: bool) {
        self.with_default = enabled;
    }

    /// Check if a screen point is inside the widget's clickable area.
    ///
    /// Tests bounding box first, then delegates to the renderer for
    /// pixel-perfect hit testing (e.g. transparency check). Uses the
    /// half-open `is_boxed_point` so adjacent widgets never both claim
    /// a shared right/bottom edge column.
    pub fn is_inside(&self, point: ScreenPoint) -> bool {
        self.bbox.is_boxed_point(point.to_geo()) && self.renderer.is_real_point(point)
    }

    /// Attach the widget (and its renderer) to a rendering surface.
    pub fn attach_to_display(&mut self, surface: u32) {
        self.rendering_surface = surface;
        self.renderer.attach_to_display(surface);
    }

    /// Dismiss the renderer's resource.
    pub fn dismiss_resource(&mut self) {
        self.renderer.dismiss_resource();
    }

    /// Render the widget with the given sub-resource.
    pub fn refresh(&mut self, sub_res: u8) {
        self.renderer.render(sub_res);
    }

    /// Build a [`UiEvent`] from this widget.
    pub fn make_event(&self, msg: UiMsg) -> UiEvent {
        UiEvent {
            msg_type: msg,
            origin_widget_id: self.id,
            data: None,
        }
    }

    /// Build a [`UiEvent`] with associated data.
    pub fn make_event_with_data(&self, msg: UiMsg, data: UiEventData) -> UiEvent {
        UiEvent {
            msg_type: msg,
            origin_widget_id: self.id,
            data: Some(data),
        }
    }

    /// Build a tooltip event if the widget has tooltip text and is disabled.
    pub fn tooltip_event_if_disabled(&self) -> Option<UiEvent> {
        if !self.enabled && self.has_tooltip() {
            Some(self.make_event(UiMsg::WidgetFocusedDisabled))
        } else {
            None
        }
    }

    /// Build a [`UiProbe`] for this widget.
    pub fn make_probe(&self, code: ProbeCode) -> UiProbe {
        UiProbe {
            code,
            zone: self.bbox,
            widget_id: self.id,
        }
    }
}

// ─── Widget enum ────────────────────────────────────────────────────

/// A concrete widget instance, wrapping one of the supported widget types.
///
/// This enum enables heterogeneous storage in [`FrameWnd`] without
/// trait objects, via static dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Widget {
    Button(WidgetButton),
    ToggleButton(WidgetToggleButton),
    RadioButton(WidgetRadioButton),
    Label(WidgetLabel),
    Picture(WidgetPicture),
    MultiPicture(WidgetMultiPicture),
    InputField(WidgetInputField),
    Slider(WidgetSlider),
    Listbox(WidgetListbox),
}

impl Widget {
    /// Get a reference to the common [`WidgetBase`].
    pub fn base(&self) -> &WidgetBase {
        match self {
            Self::Button(w) => &w.base,
            Self::ToggleButton(w) => &w.base,
            Self::RadioButton(w) => &w.base,
            Self::Label(w) => &w.base,
            Self::Picture(w) => &w.base,
            Self::MultiPicture(w) => &w.base,
            Self::InputField(w) => &w.base,
            Self::Slider(w) => &w.base,
            Self::Listbox(w) => &w.base,
        }
    }

    /// Get a mutable reference to the common [`WidgetBase`].
    pub fn base_mut(&mut self) -> &mut WidgetBase {
        match self {
            Self::Button(w) => &mut w.base,
            Self::ToggleButton(w) => &mut w.base,
            Self::RadioButton(w) => &mut w.base,
            Self::Label(w) => &mut w.base,
            Self::Picture(w) => &mut w.base,
            Self::MultiPicture(w) => &mut w.base,
            Self::InputField(w) => &mut w.base,
            Self::Slider(w) => &mut w.base,
            Self::Listbox(w) => &mut w.base,
        }
    }

    /// Widget ID.
    pub fn id(&self) -> WidgetId {
        self.base().id
    }

    /// Whether the widget is enabled for input.
    pub fn is_enabled(&self) -> bool {
        self.base().enabled
    }

    /// Process input for this widget, returning any generated events.
    ///
    /// Dispatches to the widget-specific state machine.
    pub fn process_input(&mut self, input: &WidgetInput) -> Vec<UiEvent> {
        match self {
            Self::Button(w) => w.process_input(input),
            Self::ToggleButton(w) => w.process_input(input),
            Self::RadioButton(w) => w.process_input(input),
            Self::Label(_) => Vec::new(), // labels are non-interactive
            Self::Picture(w) => w.process_input(input),
            Self::MultiPicture(_) => Vec::new(),
            Self::InputField(w) => w.process_input(input),
            Self::Slider(w) => w.process_input(input),
            Self::Listbox(w) => w.process_input(input),
        }
    }

    /// Map the current state to a renderer sub-resource ID.
    pub fn transform_state_into_id(&self) -> u8 {
        match self {
            Self::Button(w) => w.transform_state_into_id(),
            Self::ToggleButton(w) => w.transform_state_into_id(),
            Self::RadioButton(w) => w.transform_state_into_id(),
            Self::Label(w) => w.transform_state_into_id(),
            Self::Picture(w) => w.transform_state_into_id(),
            Self::MultiPicture(w) => w.transform_state_into_id(),
            Self::InputField(w) => w.transform_state_into_id(),
            Self::Slider(w) => w.transform_state_into_id(),
            Self::Listbox(_) => resource_widget_id::BUTTON_DEFAULT,
        }
    }

    /// Probe whether the widget needs a refresh.
    pub fn probe_refresh(&mut self, counter: u32) -> Option<UiProbe> {
        match self {
            Self::Label(w) => w.probe_refresh(counter),
            Self::InputField(w) => w.probe_refresh(counter),
            _ => {
                // Default probe: check if the rendered sub-resource changed.
                let sub_res = self.transform_state_into_id();
                let base = self.base_mut();
                base.renderer.set_counter(counter);
                let will_render = base
                    .renderer
                    .base()
                    .map_or(u32::MAX, |b| b.will_be_rendered(sub_res));
                let last = base.renderer.base().map_or(u32::MAX, |b| b.last_rendered());
                if will_render != last {
                    Some(base.make_probe(ProbeCode::FullRefresh))
                } else {
                    None
                }
            }
        }
    }

    /// Render the widget.
    pub fn refresh(&mut self) {
        let sub_res = self.transform_state_into_id();
        if sub_res != resource_widget_id::NO_RESOURCE {
            self.base_mut().refresh(sub_res);
        }
    }

    /// Restore the widget's renderer state.
    pub fn restore(&mut self) {
        self.base_mut().renderer.reset_save();
    }

    /// Attach to a rendering surface.
    pub fn attach_to_display(&mut self, surface: u32) {
        self.base_mut().attach_to_display(surface);
    }

    /// Set enable state, with widget-specific side effects.
    pub fn set_enable(&mut self, enabled: bool) {
        match self {
            Self::Button(w) => {
                w.set_enable(enabled);
            }
            Self::ToggleButton(w) => {
                w.set_enable(enabled);
            }
            _ => self.base_mut().set_enable(enabled),
        }
    }
}
