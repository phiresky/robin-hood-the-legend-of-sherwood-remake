//! Frame window container that owns widgets and routes input/refresh.
//!
//! A frame window holds a collection of widgets, routes input to them,
//! collects events, and manages refresh probes. Widgets are positioned
//! relative to the frame's origin.

#[cfg(test)]
use robin_engine::coordinates as engine_coordinates;
use serde::{Deserialize, Serialize};

use crate::ui::{UiEvent, UiMsg, UiProbe};
use robin_engine::coordinates::{ScreenBBox, ScreenPoint};

use super::{Widget, WidgetId, WidgetInput};

/// Frame window — a container that owns and routes input to widgets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameWnd {
    /// Window title.
    pub title: String,
    /// Position and size in screen coordinates.
    pub bbox: ScreenBBox,
    /// Creation flags.
    pub flags: u32,
    /// Whether this frame window is enabled.
    pub enabled: bool,
    /// Whether input processing is enabled.
    pub input_enabled: bool,
    /// Tooltip text (empty = no tooltip).
    pub tooltip_text: String,
    /// Explicit tooltip flag.
    ///
    /// Set by `set_tooltip_text` regardless of the text's emptiness —
    /// callers can flag a tooltip as present even when the text is empty.
    pub tooltip_set: bool,

    /// All widgets owned by this frame.
    widgets: Vec<Widget>,
    /// Widget IDs that are excluded from input/refresh processing.
    excluded: Vec<WidgetId>,

    /// Opaque handle to the rendering surface.
    rendering_surface: u32,

    /// Internal event for FrameFocus.
    frame_id: u32,
}

impl Default for FrameWnd {
    fn default() -> Self {
        Self {
            title: String::new(),
            bbox: ScreenBBox::new(),
            flags: 0,
            enabled: true,
            input_enabled: true,
            tooltip_text: String::new(),
            tooltip_set: false,
            widgets: Vec::new(),
            excluded: Vec::new(),
            rendering_surface: u32::MAX,
            frame_id: 0,
        }
    }
}

impl FrameWnd {
    /// Create a new frame window.
    pub fn new(title: &str, bbox: ScreenBBox, flags: u32) -> Self {
        Self {
            title: title.to_string(),
            bbox,
            flags,
            ..Default::default()
        }
    }

    /// Set a unique frame ID (used for FrameFocus events).
    pub fn set_frame_id(&mut self, id: u32) {
        self.frame_id = id;
    }

    /// Attach all widgets to a rendering surface.
    pub fn attach_to_display(&mut self, surface: u32) {
        self.rendering_surface = surface;
        for widget in &mut self.widgets {
            widget.attach_to_display(surface);
        }
    }

    // ── Widget management ──────────────────────────────────────────

    /// Add a widget to the frame.
    ///
    /// The widget's position is adjusted relative to the frame's origin
    /// — unconditionally adds the frame's top-left to the widget's
    /// position. Frame/widget bboxes that are `None` are treated as
    /// having a (0, 0) origin so this stays side-effect-free for
    /// unsized frames.
    pub fn add_widget(&mut self, mut widget: Widget) {
        let frame_origin = self
            .bbox
            .0
            .map(|r| ScreenPoint::new(r.min().x, r.min().y))
            .unwrap_or(ScreenPoint::ZERO);
        if let Some(widget_rect) = widget.base().bbox.0 {
            let adjusted = ScreenBBox::from_coords(
                widget_rect.min().x + frame_origin.x,
                widget_rect.min().y + frame_origin.y,
                widget_rect.max().x + frame_origin.x,
                widget_rect.max().y + frame_origin.y,
            );
            widget.base_mut().set_position(adjusted);
        }

        // Attach to rendering surface if we already have one.
        if self.rendering_surface != u32::MAX {
            widget.attach_to_display(self.rendering_surface);
        }

        self.widgets.push(widget);
    }

    /// Add a widget without adjusting its position (already in screen coords).
    pub fn add_widget_absolute(&mut self, mut widget: Widget) {
        if self.rendering_surface != u32::MAX {
            widget.attach_to_display(self.rendering_surface);
        }
        self.widgets.push(widget);
    }

    /// Remove a widget by ID.
    ///
    /// The exclusion list is **not** cleaned up here — stale entries
    /// for the removed id remain. Callers must call
    /// [`include_widget`](Self::include_widget) explicitly if they care.
    ///
    /// Uses `swap_remove` (overwrite slot with last element, shrink),
    /// which perturbs subsequent iteration order.
    pub fn remove_widget(&mut self, id: WidgetId) -> Option<Widget> {
        let idx = self.widgets.iter().position(|w| w.id() == id)?;
        Some(self.widgets.swap_remove(idx))
    }

    /// Remove *all* widgets from the frame.
    ///
    /// Clears the widget tree; the exclusion list is intentionally left
    /// alone.
    pub fn clear_widgets(&mut self) {
        self.widgets.clear();
    }

    /// Get a reference to a widget by ID.
    pub fn widget(&self, id: WidgetId) -> Option<&Widget> {
        self.widgets.iter().find(|w| w.id() == id)
    }

    /// Get a mutable reference to a widget by ID.
    pub fn widget_mut(&mut self, id: WidgetId) -> Option<&mut Widget> {
        self.widgets.iter_mut().find(|w| w.id() == id)
    }

    /// Get a reference to a widget by index.
    pub fn widget_at(&self, index: usize) -> Option<&Widget> {
        self.widgets.get(index)
    }

    /// Number of widgets in this frame.
    pub fn widget_count(&self) -> usize {
        self.widgets.len()
    }

    /// Iterate over all widgets.
    pub fn widgets(&self) -> &[Widget] {
        &self.widgets
    }

    /// Iterate mutably over all widgets.
    pub fn widgets_mut(&mut self) -> &mut [Widget] {
        &mut self.widgets
    }

    // ── Exclusion ──────────────────────────────────────────────────

    /// Exclude a widget from input/refresh processing.
    ///
    /// Only excludes widgets that are actually owned by this frame, and
    /// refuses to add duplicate entries. Returns `true` if the widget
    /// was added to the exclusion list, `false` if it was unknown or
    /// already excluded.
    pub fn exclude_widget(&mut self, id: WidgetId) -> bool {
        if !self.widgets.iter().any(|w| w.id() == id) {
            return false;
        }
        if self.excluded.contains(&id) {
            return false;
        }
        self.excluded.push(id);
        true
    }

    /// Check if a widget is excluded.
    pub fn is_excluded(&self, id: WidgetId) -> bool {
        self.excluded.contains(&id)
    }

    // ── Position ───────────────────────────────────────────────────

    /// Get the frame's origin (top-left corner).
    pub fn origin(&self) -> ScreenPoint {
        self.bbox
            .0
            .map(|r| ScreenPoint::new(r.min().x, r.min().y))
            .unwrap_or(ScreenPoint::ZERO)
    }

    /// Move the frame and all its widgets by a delta.
    pub fn set_position(&mut self, new_bbox: ScreenBBox) {
        if let (Some(old_rect), Some(new_rect)) = (self.bbox.0, new_bbox.0) {
            let dx = new_rect.min().x - old_rect.min().x;
            let dy = new_rect.min().y - old_rect.min().y;

            for widget in &mut self.widgets {
                if let Some(wrect) = widget.base().bbox.0 {
                    let adjusted = ScreenBBox::from_coords(
                        wrect.min().x + dx,
                        wrect.min().y + dy,
                        wrect.max().x + dx,
                        wrect.max().y + dy,
                    );
                    widget.base_mut().set_position(adjusted);
                }
            }
        }
        self.bbox = new_bbox;
    }

    // ── Enable / disable ───────────────────────────────────────────

    pub fn set_enable(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn set_tooltip_text(&mut self, text: &str) {
        self.tooltip_text = text.to_string();
        self.tooltip_set = true;
    }

    pub fn has_tooltip(&self) -> bool {
        self.tooltip_set
    }

    // ── Input processing ───────────────────────────────────────────

    /// Process input for all widgets in this frame.
    ///
    /// Returns a list of events generated by the widgets, plus a
    /// `FrameFocus` event if the mouse is over the frame.
    pub fn process_input(&mut self, input: &WidgetInput) -> Vec<UiEvent> {
        let mut events = Vec::new();

        if !self.enabled || !self.input_enabled {
            return events;
        }

        // Check if mouse is over the frame → emit FrameFocus.
        let mouse_in_frame = self.bbox.contains_point(input.mouse_position);
        if mouse_in_frame {
            events.push(UiEvent {
                msg_type: UiMsg::FrameFocus,
                origin_widget_id: self.frame_id,
                data: None,
            });
        }

        // Route input to each non-excluded widget. Radio-button group
        // exclusion is handled here: when an activated radio button is
        // in a group, its siblings need to be deselected. The sibling
        // list lives on the widget as `group_members: Vec<WidgetId>`,
        // and the frame is the only place with access to siblings.
        for idx in 0..self.widgets.len() {
            let widget_id = self.widgets[idx].id();
            if self.excluded.contains(&widget_id) {
                continue;
            }
            let widget_events = self.widgets[idx].process_input(input);

            let activated = widget_events
                .iter()
                .any(|e| e.msg_type == UiMsg::WidgetActivated);
            if activated {
                let group: Option<Vec<WidgetId>> = match &self.widgets[idx] {
                    Widget::RadioButton(rb) if !rb.group_members.is_empty() => {
                        Some(rb.group_members.clone())
                    }
                    _ => None,
                };
                if let Some(group) = group {
                    for other_id in group {
                        if other_id == widget_id {
                            continue;
                        }
                        if let Some(Widget::RadioButton(other)) =
                            self.widgets.iter_mut().find(|w| w.id() == other_id)
                        {
                            other.set_active_other();
                        }
                    }
                }
            }

            events.extend(widget_events);
        }

        events
    }

    // ── Refresh ────────────────────────────────────────────────────

    /// Probe all widgets for refresh needs.
    pub fn probe_refresh(&mut self, counter: u32) -> Vec<UiProbe> {
        let mut probes = Vec::new();

        if !self.enabled {
            return probes;
        }

        for widget in &mut self.widgets {
            if self.excluded.contains(&widget.id()) {
                continue;
            }
            if let Some(probe) = widget.probe_refresh(counter) {
                probes.push(probe);
            }
        }

        probes
    }

    /// Refresh (render) all widgets.
    pub fn refresh(&mut self) {
        if !self.enabled {
            return;
        }
        for widget in &mut self.widgets {
            if self.excluded.contains(&widget.id()) {
                continue;
            }
            widget.refresh();
        }
    }

    /// Restore all widgets' renderer state.
    pub fn restore(&mut self) {
        if !self.enabled {
            return;
        }
        for widget in &mut self.widgets {
            if self.excluded.contains(&widget.id()) {
                continue;
            }
            widget.restore();
        }
    }

    /// Restore only widgets that overlap with the given region.
    pub fn restore_region(&mut self, region: &ScreenBBox) {
        if !self.enabled {
            return;
        }
        for widget in &mut self.widgets {
            if self.excluded.contains(&widget.id()) {
                continue;
            }
            if widget.base().bbox.intersects_bbox(region) {
                widget.restore();
                widget.refresh();
            }
        }
    }
}

#[cfg(test)]
#[path = "frame_wnd_tests.rs"]
mod tests;
