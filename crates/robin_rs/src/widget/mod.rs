//! Widget system: interactive UI elements with state-machine input handling.

mod button;
mod frame_wnd;
mod input_field;
mod label;
mod listbox;
mod picture;
mod radio;
mod slider;
#[cfg(test)]
mod test_support;
mod toggle;

// Sim-side widget state (campaign-derived derived values) lives in
// `robin_engine::widget_state` now (Decision 7B). Re-export for
// compatibility.
pub use engine_widget_state::blazon_bar;
pub use engine_widget_state::requirements;
use robin_engine::widget_state as engine_widget_state;

pub use button::WidgetButton;
pub use frame_wnd::FrameWnd;
pub use input_field::WidgetInputField;
pub use label::WidgetLabel;
pub use listbox::{ColumnAlign, ColumnLayout, LayoutCell, WidgetListbox};
pub use picture::{WidgetMultiPicture, WidgetPicture};
pub use radio::WidgetRadioButton;
pub use slider::WidgetSlider;
pub use toggle::WidgetToggleButton;

use serde::{Deserialize, Serialize};

use crate::ui::{
    MouseButtons, ResourceId, UiEvent, UiEventData, UiMsg, UiState, WidgetAppearance,
    resource_widget_id,
};
use robin_engine::coordinates::{ScreenBBox, ScreenPoint};

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
/// `text_input` carries UTF-8 characters produced by winit IME commit events
/// (via [`crate::gfx_types::GameEvent::TextInput`]) since the previous frame.
/// It contains only committed text from
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
    /// Accelerator physical key.
    pub fast_key: Option<winit::keyboard::KeyCode>,
    /// Creation flags.
    pub flags: u32,
    /// Widget text (button label, input text, etc.).
    pub text: String,
    /// Tooltip text (empty = no tooltip).
    pub tooltip_text: String,
    /// Position and size in screen coordinates.
    pub bbox: ScreenBBox,
    /// Current interaction state.
    pub state: UiState,
    /// Asset selection and pixel hit mask; the GPU bridge owns drawing.
    pub appearance: Option<WidgetAppearance>,
}

impl Default for WidgetBase {
    fn default() -> Self {
        Self {
            id: WIDGET_ID_NONE,
            enabled: true,
            with_focus: true,
            with_default: true,
            created: false,
            fast_key: None,
            flags: 0,
            text: String::new(),
            tooltip_text: String::new(),
            bbox: ScreenBBox::new(),
            state: UiState::Default,
            appearance: None,
        }
    }
}

impl WidgetBase {
    /// Initialize the widget.
    pub fn create(&mut self, text: &str, bbox: ScreenBBox, flags: u32) {
        text.clone_into(&mut self.text);
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
        bbox: ScreenBBox,
        flags: u32,
        resource_id: ResourceId,
    ) {
        self.create(text, bbox, flags);
        self.appearance
            .get_or_insert_with(WidgetAppearance::default)
            .resource_id = resource_id;
    }

    pub fn set_text(&mut self, text: &str) {
        text.clone_into(&mut self.text);
    }

    pub fn set_tooltip_text(&mut self, text: &str) {
        text.clone_into(&mut self.tooltip_text);
    }

    pub fn has_tooltip(&self) -> bool {
        !self.tooltip_text.is_empty()
    }

    pub fn set_position(&mut self, bbox: ScreenBBox) {
        self.bbox = bbox;
    }

    pub fn set_enable(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Check if a screen point is inside the widget's clickable area.
    ///
    /// Tests bounding box first, then checks the asset's alpha mask for
    /// pixel-perfect hit testing (e.g. transparency check). Uses the
    /// half-open `is_boxed_point` so adjacent widgets never both claim
    /// a shared right/bottom edge column.
    pub fn is_inside(&self, point: ScreenPoint) -> bool {
        self.bbox.is_boxed_point(point)
            && self
                .appearance
                .as_ref()
                .is_some_and(|appearance| appearance.is_real_point(self.bbox, point))
    }

    /// Clear the widget's asset selection.
    pub fn dismiss_resource(&mut self) {
        if let Some(appearance) = self.appearance.as_mut() {
            appearance.resource_id = -1;
        }
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
