//! Static text label widget.
//!
//! Labels are non-interactive — `process_input` always returns nothing.
//! They track whether they need a refresh via double-buffered flags.

use serde::{Deserialize, Serialize};

use crate::ui::{
    ProbeCode, UiProbe,
    resource_widget_id::{BUTTON_DEFAULT, NO_RESOURCE},
};

use super::WidgetBase;

/// Static text label widget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetLabel {
    pub base: WidgetBase,
    /// Double-buffered refresh flags.
    /// When text changes, both frames need a redraw.
    refresh_needed: [bool; 2],
}

impl Default for WidgetLabel {
    fn default() -> Self {
        Self {
            base: WidgetBase {
                with_focus: false,
                ..Default::default()
            },
            refresh_needed: [false; 2],
        }
    }
}

impl WidgetLabel {
    pub fn new(id: super::WidgetId) -> Self {
        Self {
            base: WidgetBase {
                id,
                with_focus: false,
                ..Default::default()
            },
            refresh_needed: [false; 2],
        }
    }

    /// Override set_text to mark both buffers as needing refresh.
    pub fn set_text(&mut self, text: &str) {
        self.base.set_text(text);
        self.refresh_needed = [true; 2];
    }

    /// Map state to renderer sub-resource ID.
    pub fn transform_state_into_id(&self) -> u8 {
        if self.base.enabled {
            BUTTON_DEFAULT
        } else {
            NO_RESOURCE
        }
    }

    /// Probe whether a refresh is needed.
    ///
    /// Only returns a probe if the current buffer frame needs refresh.
    pub fn probe_refresh(&mut self, counter: u32) -> Option<UiProbe> {
        self.base.renderer.set_counter(counter);
        let idx = (counter % 2) as usize;
        if self.refresh_needed[idx] {
            self.refresh_needed[idx] = false;
            Some(self.base.make_probe(ProbeCode::LazyRefresh))
        } else {
            None
        }
    }
}
