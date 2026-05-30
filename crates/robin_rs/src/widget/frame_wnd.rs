//! Frame window container that owns widgets and routes input/refresh.
//!
//! A frame window holds a collection of widgets, routes input to them,
//! collects events, and manages refresh probes. Widgets are positioned
//! relative to the frame's origin.

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
            .map(|r| ScreenPoint::from_geo(r.min()))
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
            .map(|r| ScreenPoint::from_geo(r.min()))
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
mod tests {
    use super::*;
    use crate::ui::{MouseButtons, UiKeyboard, UiMsg};
    use crate::widget::{WidgetButton, WidgetRadioButton, WidgetRenderer};

    fn make_keyboard() -> &'static UiKeyboard {
        Box::leak(Box::new(UiKeyboard::default()))
    }

    fn make_input(x: f32, y: f32, buttons: MouseButtons) -> WidgetInput<'static> {
        WidgetInput {
            mouse_position: robin_engine::coordinates::ScreenPoint::new(x, y),
            mouse_z: 0,
            mouse_button: buttons,
            keyboard: make_keyboard(),
            text_input: "",
            capture: None,
        }
    }

    fn make_button_widget(id: WidgetId, x: f32, y: f32, w: f32, h: f32) -> Widget {
        let mut btn = WidgetButton::new(id);
        let bbox = ScreenBBox::from_coords(x, y, x + w, y + h);
        btn.base.create("Test", bbox, 0);
        btn.base.renderer = WidgetRenderer::Bitmap(crate::ui::RendererBitmap {
            base: crate::ui::RendererBase {
                bbox,
                ..Default::default()
            },
        });
        Widget::Button(btn)
    }

    #[test]
    fn add_widget_adjusts_position() {
        let mut frame = FrameWnd::new(
            "Test",
            ScreenBBox::from_coords(100.0, 50.0, 400.0, 300.0),
            0,
        );
        // Widget at (10, 10) relative to frame.
        let mut btn = WidgetButton::new(1);
        btn.base
            .create("Btn", ScreenBBox::from_coords(10.0, 10.0, 80.0, 30.0), 0);
        frame.add_widget(Widget::Button(btn));

        let widget_bbox = frame.widget(1).unwrap().base().bbox;
        // Should be adjusted by frame origin (100, 50).
        let rect = widget_bbox.0.unwrap();
        assert!((rect.min().x - 110.0).abs() < 0.01);
        assert!((rect.min().y - 60.0).abs() < 0.01);
    }

    #[test]
    fn process_input_routes_to_widgets() {
        let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
        frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));

        // Hover over button.
        let input = make_input(50.0, 20.0, MouseButtons::empty());
        let events = frame.process_input(&input);
        assert!(events.iter().any(|e| e.msg_type == UiMsg::FrameFocus));
        assert!(events.iter().any(|e| e.msg_type == UiMsg::WidgetFocused));
    }

    #[test]
    fn excluded_widget_skipped() {
        let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
        frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));
        frame.exclude_widget(1);

        let input = make_input(50.0, 20.0, MouseButtons::empty());
        let events = frame.process_input(&input);
        // Only FrameFocus, no widget events.
        assert!(!events.iter().any(|e| e.msg_type == UiMsg::WidgetFocused));
    }

    #[test]
    fn remove_widget_works() {
        let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
        frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));
        assert_eq!(frame.widget_count(), 1);

        let removed = frame.remove_widget(1);
        assert!(removed.is_some());
        assert_eq!(frame.widget_count(), 0);
    }

    #[test]
    fn disabled_frame_returns_no_events() {
        let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
        frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));
        frame.set_enable(false);

        let input = make_input(50.0, 20.0, MouseButtons::LEFT_CLICK);
        let events = frame.process_input(&input);
        assert!(events.is_empty());
    }

    #[test]
    fn disabled_frame_refresh_skips_children() {
        use crate::ui::resource_widget_id::{BUTTON_DEFAULT, NO_RESOURCE};

        let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
        frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));

        assert_eq!(
            frame
                .widget(1)
                .unwrap()
                .base()
                .renderer
                .base()
                .unwrap()
                .sub_resource,
            NO_RESOURCE,
        );

        frame.set_enable(false);
        frame.refresh();
        assert_eq!(
            frame
                .widget(1)
                .unwrap()
                .base()
                .renderer
                .base()
                .unwrap()
                .sub_resource,
            NO_RESOURCE,
            "refresh() on disabled frame must not render children",
        );

        frame.set_enable(true);
        frame.refresh();
        assert_eq!(
            frame
                .widget(1)
                .unwrap()
                .base()
                .renderer
                .base()
                .unwrap()
                .sub_resource,
            BUTTON_DEFAULT,
            "refresh() on enabled frame must render children",
        );
    }

    #[test]
    fn disabled_frame_restore_and_probe_skip_children() {
        let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
        frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));
        // Dirty last_rendered so we can detect whether restore() cleared it.
        frame
            .widget_mut(1)
            .unwrap()
            .base_mut()
            .renderer
            .base_mut()
            .unwrap()
            .last_rendered = [42, 42];

        frame.set_enable(false);
        frame.restore();
        assert_eq!(
            frame
                .widget(1)
                .unwrap()
                .base()
                .renderer
                .base()
                .unwrap()
                .last_rendered,
            [42, 42],
            "restore() on disabled frame must not touch children",
        );

        let region = ScreenBBox::from_coords(0.0, 0.0, 100.0, 100.0);
        frame.restore_region(&region);
        assert_eq!(
            frame
                .widget(1)
                .unwrap()
                .base()
                .renderer
                .base()
                .unwrap()
                .last_rendered,
            [42, 42],
            "restore_region() on disabled frame must not touch children",
        );

        let probes = frame.probe_refresh(0);
        assert!(
            probes.is_empty(),
            "probe_refresh() on disabled frame must return no probes",
        );
    }

    #[test]
    fn restore_region_calls_both_restore_and_refresh() {
        use crate::ui::resource_widget_id::{BUTTON_DEFAULT, NO_RESOURCE};

        let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
        frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));
        // Sentinel for restore() detection: reset_save will reset this to [MAX; 2].
        frame
            .widget_mut(1)
            .unwrap()
            .base_mut()
            .renderer
            .base_mut()
            .unwrap()
            .last_rendered = [42, 42];

        assert_eq!(
            frame
                .widget(1)
                .unwrap()
                .base()
                .renderer
                .base()
                .unwrap()
                .sub_resource,
            NO_RESOURCE,
        );

        let region = ScreenBBox::from_coords(0.0, 0.0, 100.0, 100.0);
        frame.restore_region(&region);

        let rbase = frame.widget(1).unwrap().base().renderer.base().unwrap();
        assert_eq!(
            rbase.last_rendered,
            [u32::MAX; 2],
            "restore() must have cleared last_rendered",
        );
        assert_eq!(
            rbase.sub_resource, BUTTON_DEFAULT,
            "refresh() must have run after restore() for intersecting widget",
        );
    }

    #[test]
    fn exclude_widget_requires_membership() {
        let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
        frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));

        // Unknown widget id must not be excluded.
        assert!(!frame.exclude_widget(999));
        assert!(!frame.is_excluded(999));

        // First exclusion of a known widget succeeds.
        assert!(frame.exclude_widget(1));
        assert!(frame.is_excluded(1));

        // Duplicate exclusion is a no-op.
        assert!(!frame.exclude_widget(1));
    }

    #[test]
    fn remove_widget_leaves_exclusion_list() {
        let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
        frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));
        assert!(frame.exclude_widget(1));
        assert!(frame.is_excluded(1));

        // remove_widget intentionally leaves the exclusion list untouched.
        let removed = frame.remove_widget(1);
        assert!(removed.is_some());
        assert!(
            frame.is_excluded(1),
            "remove_widget must not prune the exclusion list",
        );
    }

    #[test]
    fn clear_widgets_empties_tree() {
        let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
        frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));
        frame.add_widget_absolute(make_button_widget(2, 10.0, 50.0, 80.0, 30.0));
        assert_eq!(frame.widget_count(), 2);

        frame.clear_widgets();
        assert_eq!(frame.widget_count(), 0);
    }

    #[test]
    fn has_tooltip_reflects_set_call_not_text() {
        let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
        assert!(!frame.has_tooltip());

        // setting empty text still flags the tooltip present.
        frame.set_tooltip_text("");
        assert!(frame.has_tooltip());

        frame.set_tooltip_text("hello");
        assert!(frame.has_tooltip());
        assert_eq!(frame.tooltip_text, "hello");
    }

    #[test]
    fn add_widget_adjusts_even_without_frame_bbox() {
        // Frame with no bbox — origin defaults to (0, 0); widget position
        // should stay unchanged.
        let mut frame = FrameWnd::new("Test", ScreenBBox::new(), 0);
        let mut btn = WidgetButton::new(1);
        btn.base
            .create("Btn", ScreenBBox::from_coords(10.0, 10.0, 80.0, 30.0), 0);
        frame.add_widget(Widget::Button(btn));

        let rect = frame.widget(1).unwrap().base().bbox.0.unwrap();
        assert!((rect.min().x - 10.0).abs() < 0.01);
        assert!((rect.min().y - 10.0).abs() < 0.01);
    }

    fn make_radio_widget(id: WidgetId, x: f32, y: f32, w: f32, h: f32) -> WidgetRadioButton {
        let mut rb = WidgetRadioButton::new(id);
        let bbox = ScreenBBox::from_coords(x, y, x + w, y + h);
        rb.base.create("Radio", bbox, 0);
        rb.base.renderer = WidgetRenderer::Bitmap(crate::ui::RendererBitmap {
            base: crate::ui::RendererBase {
                bbox,
                ..Default::default()
            },
        });
        rb
    }

    #[test]
    fn radio_group_exclusion_deselects_siblings() {
        // Three radio buttons linked as a group — clicking one must
        // deselect the others.
        let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 400.0, 400.0), 0);
        let mut rb0 = make_radio_widget(10, 10.0, 10.0, 80.0, 20.0);
        let mut rb1 = make_radio_widget(11, 10.0, 40.0, 80.0, 20.0);
        let mut rb2 = make_radio_widget(12, 10.0, 70.0, 80.0, 20.0);
        rb0.group_members = vec![10, 11, 12];
        rb1.group_members = vec![10, 11, 12];
        rb2.group_members = vec![10, 11, 12];
        // Pre-select rb0 so we can confirm it gets kicked.
        rb0.set_selected(true);
        frame.add_widget_absolute(Widget::RadioButton(rb0));
        frame.add_widget_absolute(Widget::RadioButton(rb1));
        frame.add_widget_absolute(Widget::RadioButton(rb2));

        // Click inside rb1 (center of its bbox).
        let input = make_input(50.0, 50.0, MouseButtons::LEFT_CLICK);
        let events = frame.process_input(&input);
        assert!(events.iter().any(|e| e.msg_type == UiMsg::WidgetActivated));

        let get_second_state = |f: &FrameWnd, id: WidgetId| -> bool {
            match f.widget(id).unwrap() {
                Widget::RadioButton(rb) => rb.is_pushed(),
                _ => panic!("expected radio button"),
            }
        };
        assert!(
            !get_second_state(&frame, 10),
            "rb0 must be deselected after rb1 activation"
        );
        assert!(get_second_state(&frame, 11), "rb1 must stay selected");
        assert!(!get_second_state(&frame, 12), "rb2 must remain deselected");
    }

    #[test]
    fn radio_activation_without_group_does_not_touch_others() {
        // Radio buttons with empty group_members must not interfere with
        // each other — matches the slider sub-button case where exclusion
        // is managed by the slider, not the frame.
        let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 400.0, 400.0), 0);
        let rb0 = make_radio_widget(10, 10.0, 10.0, 80.0, 20.0);
        let mut rb1 = make_radio_widget(11, 10.0, 40.0, 80.0, 20.0);
        rb1.set_selected(true);
        frame.add_widget_absolute(Widget::RadioButton(rb0));
        frame.add_widget_absolute(Widget::RadioButton(rb1));

        // Click rb0 to emit Activated with no group_members wired — rb1
        // must stay selected because nothing walks the chain.
        let input = make_input(50.0, 20.0, MouseButtons::LEFT_CLICK);
        let events = frame.process_input(&input);
        assert!(events.iter().any(|e| e.msg_type == UiMsg::WidgetActivated));

        let is_pushed = |f: &FrameWnd, id: WidgetId| -> bool {
            match f.widget(id).unwrap() {
                Widget::RadioButton(rb) => rb.is_pushed(),
                _ => panic!(),
            }
        };
        assert!(
            is_pushed(&frame, 10),
            "rb0 must be selected after being clicked",
        );
        assert!(
            is_pushed(&frame, 11),
            "rb1 must remain selected when rb0 has no group_members",
        );
    }

    #[test]
    fn restore_region_skips_non_intersecting() {
        use crate::ui::resource_widget_id::NO_RESOURCE;

        let mut frame = FrameWnd::new("Test", ScreenBBox::from_coords(0.0, 0.0, 200.0, 200.0), 0);
        frame.add_widget_absolute(make_button_widget(1, 10.0, 10.0, 80.0, 30.0));

        // Region far outside the widget's bbox.
        let region = ScreenBBox::from_coords(150.0, 150.0, 200.0, 200.0);
        frame.restore_region(&region);

        assert_eq!(
            frame
                .widget(1)
                .unwrap()
                .base()
                .renderer
                .base()
                .unwrap()
                .sub_resource,
            NO_RESOURCE,
            "non-intersecting widget must not be refreshed",
        );
    }
}
