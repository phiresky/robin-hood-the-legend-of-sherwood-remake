//! Picture and multi-picture display widgets.
//!
//! Pictures are mostly non-interactive but can optionally respond to
//! mouse hover (focus) and clicks (select).

use serde::{Deserialize, Serialize};

use crate::ui::{
    MouseButtons, UiEvent, UiMsg, UiState,
    resource_widget_id::{NO_RESOURCE, PICTURE_DEFAULT},
};

use super::{WidgetBase, WidgetInput};

/// Single-image picture widget.
///
/// Supports optional focus (hover) and click activation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetPicture {
    pub base: WidgetBase,
    /// Runtime-generated surface blitted in place of the resource-mapped
    /// picture. `None` falls back to the sub-resource returned by
    /// [`transform_state_into_id`].
    ///
    /// Surface IDs are live handles that don't survive serialization —
    /// callers must re-set them after reload, which is why this field is
    /// skipped.
    #[serde(skip)]
    alternate_picture: Option<u32>,
}

impl Default for WidgetPicture {
    fn default() -> Self {
        Self {
            base: WidgetBase {
                with_focus: false,
                ..Default::default()
            },
            alternate_picture: None,
        }
    }
}

impl WidgetPicture {
    pub fn new(id: super::WidgetId) -> Self {
        Self {
            base: WidgetBase {
                id,
                with_focus: false,
                ..Default::default()
            },
            alternate_picture: None,
        }
    }

    /// Map state to renderer sub-resource ID.
    ///
    /// Only consulted when no alternate picture is set — callers that
    /// blit the widget should check [`alternate_picture`] first.
    pub fn transform_state_into_id(&self) -> u8 {
        if self.base.enabled {
            PICTURE_DEFAULT
        } else {
            NO_RESOURCE
        }
    }

    /// Set the runtime surface to blit in place of the resource-mapped
    /// picture.
    pub fn set_alternate_picture(&mut self, surface_id: u32) {
        self.alternate_picture = Some(surface_id);
    }

    /// Clear the alternate surface, reverting to the resource-mapped
    /// picture.
    pub fn reset_alternate_picture(&mut self) {
        self.alternate_picture = None;
    }

    /// Current alternate surface handle, if any.
    pub fn alternate_picture(&self) -> Option<u32> {
        self.alternate_picture
    }

    /// Process input for one frame.
    pub fn process_input(&mut self, input: &WidgetInput) -> Vec<UiEvent> {
        if !self.base.enabled {
            return Vec::new();
        }

        let inside = self.base.is_inside(input.mouse_position);

        match self.base.state {
            UiState::Default => {
                if inside {
                    if input.mouse_button.contains(MouseButtons::LEFT_CLICK)
                        || input.mouse_button.contains(MouseButtons::LEFT_DOUBLE_CLICK)
                    {
                        return self.process_select();
                    }
                    if self.base.with_focus {
                        return self.process_focus();
                    }
                }
                Vec::new()
            }
            UiState::Focused => {
                if inside {
                    if input.mouse_button.contains(MouseButtons::LEFT_CLICK)
                        || input.mouse_button.contains(MouseButtons::LEFT_DOUBLE_CLICK)
                    {
                        return self.process_select();
                    }
                    Vec::new()
                } else {
                    self.process_default()
                }
            }
            UiState::Selected => {
                if !inside {
                    self.process_default()
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    fn process_default(&mut self) -> Vec<UiEvent> {
        self.base.state = UiState::Default;
        vec![self.base.make_event(UiMsg::WidgetUnfocused)]
    }

    fn process_focus(&mut self) -> Vec<UiEvent> {
        self.base.state = UiState::Focused;
        vec![self.base.make_event(UiMsg::WidgetFocused)]
    }

    fn process_select(&mut self) -> Vec<UiEvent> {
        self.base.state = UiState::Selected;
        vec![self.base.make_event(UiMsg::WidgetActivated)]
    }
}

/// Multi-frame picture widget.
///
/// Displays one of several sub-pictures, selected by index.
/// Non-interactive (input always ignored).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetMultiPicture {
    pub base: WidgetBase,
    /// Currently displayed sub-picture index.
    pub sub_picture: u32,
}

impl Default for WidgetMultiPicture {
    fn default() -> Self {
        Self {
            base: WidgetBase {
                with_focus: false,
                ..Default::default()
            },
            sub_picture: 0,
        }
    }
}

impl WidgetMultiPicture {
    pub fn new(id: super::WidgetId) -> Self {
        Self {
            base: WidgetBase {
                id,
                with_focus: false,
                ..Default::default()
            },
            sub_picture: 0,
        }
    }

    /// Select which sub-picture to display.
    pub fn select_picture(&mut self, index: u32) {
        self.sub_picture = index;
    }

    /// Map state to renderer sub-resource ID.
    ///
    /// Returns the sub-picture index directly as the resource ID.
    pub fn transform_state_into_id(&self) -> u8 {
        self.sub_picture as u8
    }
}
