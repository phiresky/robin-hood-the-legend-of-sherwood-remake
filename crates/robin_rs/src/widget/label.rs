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
        let mut widget = Self::default();
        widget.base.id = id;
        widget
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

#[test]
fn repeated_label_text_updates_reuse_storage_and_refresh_both_buffers() {
    use super::WidgetRenderer;
    use crate::ui::{RendererBase, RendererBitmap};
    let mut label = WidgetLabel::new(1);
    label.base.text = String::with_capacity(64);
    label.base.tooltip_text = String::with_capacity(64);
    label.base.renderer = WidgetRenderer::Bitmap(RendererBitmap {
        base: RendererBase {
            text: String::with_capacity(64),
            ..Default::default()
        },
    });
    let text_pointer = label.base.text.as_ptr();
    let tooltip_pointer = label.base.tooltip_text.as_ptr();
    let renderer_pointer = label.base.renderer.base().unwrap().text.as_ptr();
    for text in ["Robin Hood", "罗宾", "", ""] {
        label.set_text(text);
        label.base.set_tooltip_text(text);
        assert_eq!(label.base.text, text);
        assert_eq!(label.base.tooltip_text, text);
        assert_eq!(label.base.renderer.base().unwrap().text, text);
        assert_eq!(label.base.text.as_ptr(), text_pointer);
        assert_eq!(label.base.tooltip_text.as_ptr(), tooltip_pointer);
        assert_eq!(
            label.base.renderer.base().unwrap().text.as_ptr(),
            renderer_pointer
        );
        assert!(label.probe_refresh(0).is_some());
        assert!(label.probe_refresh(0).is_none());
        assert!(label.probe_refresh(1).is_some());
        assert!(label.probe_refresh(1).is_none());
    }
}

#[test]
fn noninteractive_widget_constructors_preserve_all_defaults_except_id() {
    macro_rules! check {
        ($widget:ty) => {
            for id in [0, 1, u32::MAX] {
                let actual = <$widget>::new(id);
                let mut expected = <$widget>::default();
                expected.base.id = id;
                assert!(!actual.base.with_focus);
                assert_eq!(
                    serde_json::to_value(&actual).unwrap(),
                    serde_json::to_value(&expected).unwrap()
                );
            }
        };
    }
    check!(WidgetLabel);
    check!(super::WidgetPicture);
    check!(super::WidgetMultiPicture);
}
