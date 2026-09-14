//! Static text label widget.
//!
//! Labels are non-interactive — `process_input` always returns nothing.

use serde::{Deserialize, Serialize};

use crate::ui::resource_widget_id::{BUTTON_DEFAULT, NO_RESOURCE};

use super::WidgetBase;

/// Static text label widget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetLabel {
    pub base: WidgetBase,
}

impl Default for WidgetLabel {
    fn default() -> Self {
        Self {
            base: WidgetBase {
                with_focus: false,
                ..Default::default()
            },
        }
    }
}

impl WidgetLabel {
    pub fn new(id: super::WidgetId) -> Self {
        let mut widget = Self::default();
        widget.base.id = id;
        widget
    }

    /// Update the label text.
    pub fn set_text(&mut self, text: &str) {
        self.base.set_text(text);
    }

    /// Map state to renderer sub-resource ID.
    pub fn transform_state_into_id(&self) -> u8 {
        if self.base.enabled {
            BUTTON_DEFAULT
        } else {
            NO_RESOURCE
        }
    }
}

#[test]
fn repeated_label_text_updates_reuse_storage() {
    let mut label = WidgetLabel::new(1);
    label.base.text = String::with_capacity(64);
    label.base.tooltip_text = String::with_capacity(64);
    let text_pointer = label.base.text.as_ptr();
    let tooltip_pointer = label.base.tooltip_text.as_ptr();
    for text in ["Robin Hood", "罗宾", "", ""] {
        label.set_text(text);
        label.base.set_tooltip_text(text);
        assert_eq!(label.base.text, text);
        assert_eq!(label.base.tooltip_text, text);
        assert_eq!(label.base.text.as_ptr(), text_pointer);
        assert_eq!(label.base.tooltip_text.as_ptr(), tooltip_pointer);
    }
}
