//! Scrollable list box widget.
//!
//! The listbox has a 7-state machine for handling item focus, selection,
//! scrollbar interaction, and knob dragging.
//!
//! State machine:
//! ```text
//! DEFAULT ──mouse over items──► ITEMS_FOCUSED ──left down──► ITEMS_PUSHED
//!    │                              │                            │
//!    │──mouse over scroll──► SCROLL_FOCUSED                     │
//!    │                          │                                │
//!    │                     left down knob                        │
//!    │                          ▼                                │
//!    │                    SCROLL_PUSHED                          │
//!    │                                                           │
//!    └────────────────── ITEMS_SELECTED ◄──mouse outside─────────┘
//! ```

use serde::{Deserialize, Serialize};

use crate::geo2d::BBox2D;
use crate::ui::{MouseButtons, UiEvent, UiEventData, UiMsg};
use robin_engine::coordinates::ScreenPoint;

use super::{WidgetBase, WidgetInput};

/// Internal listbox interaction state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[repr(u8)]
enum ListboxState {
    #[default]
    Default = 0,
    ItemsFocused,
    ItemsPushed,
    ItemsSelected,
    ScrollFocused,
    ScrollPushed,
}

/// A single item in the listbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListboxItem<T: Clone = ()> {
    pub text: String,
    pub data: T,
    pub flags: u32,
}

/// Per-column text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ColumnAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Multi-column layout metadata for pipe-delimited listbox rows.
///
/// Each row is split on `|` into per-column cells, then each cell is
/// rendered inside its column span with the configured alignment.
/// Widths are stored as ratios of the row width (sum to ~1.0) so the
/// same layout works at any list width.
///
/// Empty cells are elided: if cell *i* is empty, the preceding
/// non-empty cell's span is extended to cover it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ColumnLayout {
    pub ratios: Vec<f32>,
    pub aligns: Vec<ColumnAlign>,
}

/// One cell after laying out a row against a [`ColumnLayout`].
#[derive(Debug, Clone, Copy)]
pub struct LayoutCell<'a> {
    pub text: &'a str,
    pub span_x: f32,
    pub span_w: f32,
    pub align: ColumnAlign,
}

impl ColumnLayout {
    /// Build a layout from `(ratio, align)` pairs.
    pub fn new(columns: &[(f32, ColumnAlign)]) -> Self {
        let ratios = columns.iter().map(|(r, _)| *r).collect();
        let aligns = columns.iter().map(|(_, a)| *a).collect();
        Self { ratios, aligns }
    }

    pub fn is_empty(&self) -> bool {
        self.ratios.is_empty()
    }

    pub fn column_count(&self) -> usize {
        self.ratios.len()
    }

    /// Split a pipe-delimited row into per-column cells. Cells beyond
    /// the configured column count are absorbed into the final cell —
    /// only the first `N-1` pipes are split. If no columns are
    /// configured, the whole text is returned as a single cell so
    /// single-column lists work transparently.
    pub fn split_cells<'a>(&self, text: &'a str) -> Vec<&'a str> {
        if self.ratios.is_empty() {
            vec![text]
        } else {
            text.splitn(self.ratios.len(), '|').collect()
        }
    }

    /// Lay out cells against a row spanning `[row_x, row_x + row_width]`.
    ///
    /// Empty cells are skipped; the preceding non-empty cell absorbs
    /// their width.
    pub fn layout_row<'a>(&self, text: &'a str, row_x: f32, row_width: f32) -> Vec<LayoutCell<'a>> {
        let cells = self.split_cells(text);
        if self.ratios.is_empty() {
            return vec![LayoutCell {
                text: cells[0],
                span_x: row_x,
                span_w: row_width,
                align: ColumnAlign::Left,
            }];
        }
        let mut out = Vec::with_capacity(self.ratios.len());
        let mut cursor = row_x;
        let mut i = 0;
        while i < self.ratios.len() {
            let mut span_w = self.ratios[i] * row_width;
            let cell_text = cells.get(i).copied().unwrap_or("");
            // Absorb following empty columns into this span.
            let mut j = i + 1;
            while j < self.ratios.len() {
                let next_text = cells.get(j).copied().unwrap_or("");
                if next_text.is_empty() {
                    span_w += self.ratios[j] * row_width;
                    j += 1;
                } else {
                    break;
                }
            }
            if !cell_text.is_empty() {
                out.push(LayoutCell {
                    text: cell_text,
                    span_x: cursor,
                    span_w,
                    align: self.aligns[i],
                });
            }
            cursor += span_w;
            i = j;
        }
        out
    }
}

/// Scrollable list box widget.
///
/// Generic over item data type `T`. Defaults to `()` for text-only lists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetListbox<T: Clone = ()> {
    pub base: WidgetBase,

    /// All items in the list.
    pub items: Vec<ListboxItem<T>>,
    /// Index of the currently selected item, or `None`.
    pub selected: Option<usize>,
    /// Index of the currently focused (hovered) item, or `None`.
    pub focused: Option<usize>,
    /// Index of the first visible item (scroll position).
    pub first_visible: usize,
    /// Number of items that fit in the visible area.
    pub visible_count: usize,

    /// Internal interaction state.
    state: ListboxState,

    // ── Scrollbar geometry (set by layout/renderer) ──
    /// Bounding box of the item area (excluding scrollbar).
    pub items_bbox: BBox2D,
    /// Bounding box of the scrollbar track.
    pub scrollbar_bbox: BBox2D,
    /// Bounding box of the scrollbar knob (thumb).
    pub knob_bbox: BBox2D,
    /// Height of one item in pixels.
    pub item_height: f32,

    /// Mouse position saved for knob dragging.
    drag_start_y: f32,
    /// First_visible at drag start, for computing drag delta.
    drag_start_first: usize,

    /// Double-buffered state tracking for probe_refresh.
    remember: [[u16; 6]; 2],
    force_refresh: [bool; 2],

    /// Column layout for multi-column rendering. Empty by default,
    /// meaning the list renders single-column. Populated via
    /// [`WidgetListbox::set_columns`] for pipe-delimited multi-column
    /// rows (save/load picker, key-binding list, …).
    pub column_layout: ColumnLayout,
}

impl<T: Clone> Default for WidgetListbox<T> {
    fn default() -> Self {
        Self {
            base: WidgetBase::default(),
            items: Vec::new(),
            selected: None,
            focused: None,
            first_visible: 0,
            visible_count: 0,
            state: ListboxState::Default,
            items_bbox: BBox2D::new(),
            scrollbar_bbox: BBox2D::new(),
            knob_bbox: BBox2D::new(),
            item_height: 16.0,
            drag_start_y: 0.0,
            drag_start_first: 0,
            remember: [[0; 6]; 2],
            force_refresh: [false; 2],
            column_layout: ColumnLayout::default(),
        }
    }
}

impl<T: Clone> WidgetListbox<T> {
    pub fn new(id: super::WidgetId) -> Self {
        Self {
            base: WidgetBase {
                id,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Configure the column layout for pipe-delimited rows. Columns are
    /// always set up as a known list at construction, so they're passed
    /// as a single slice.
    pub fn set_columns(&mut self, columns: &[(f32, ColumnAlign)]) {
        self.column_layout = ColumnLayout::new(columns);
    }

    // ── Item management ────────────────────────────────────────────

    /// Add an item at the end of the list.
    pub fn add_item(&mut self, text: &str, data: T, flags: u32) {
        self.items.push(ListboxItem {
            text: text.to_string(),
            data,
            flags,
        });
        self.update_visible_count();
    }

    /// Insert an item at the given index.
    pub fn insert_item(&mut self, index: usize, text: &str, data: T, flags: u32) {
        self.items.insert(
            index.min(self.items.len()),
            ListboxItem {
                text: text.to_string(),
                data,
                flags,
            },
        );
        self.update_visible_count();
    }

    /// Remove the item at the given index.
    pub fn delete_item(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
            // Adjust selection and focus.
            if self.selected == Some(index) {
                self.selected = None;
            } else if let Some(sel) = self.selected
                && sel > index
            {
                self.selected = Some(sel - 1);
            }
            if self.focused == Some(index) {
                self.focused = None;
            }
            self.update_visible_count();
        }
    }

    /// Remove all items.
    pub fn delete_all_items(&mut self) {
        self.items.clear();
        self.selected = None;
        self.focused = None;
        self.first_visible = 0;
    }

    /// Get the number of items.
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Get the text of an item.
    pub fn item_text(&self, index: usize) -> Option<&str> {
        self.items.get(index).map(|i| i.text.as_str())
    }

    /// Set the text of an item.
    pub fn set_item_text(&mut self, index: usize, text: &str) {
        if let Some(item) = self.items.get_mut(index) {
            item.text = text.to_string();
        }
    }

    /// Get the data of the selected item.
    pub fn selected_item(&self) -> Option<&ListboxItem<T>> {
        self.selected.and_then(|i| self.items.get(i))
    }

    /// Get the selected item index.
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// Set the selected item index programmatically.
    pub fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index;
        // Ensure selected item is visible.
        if let Some(idx) = index {
            if idx < self.first_visible {
                self.first_visible = idx;
            } else if idx >= self.first_visible + self.visible_count {
                self.first_visible = idx.saturating_sub(self.visible_count - 1);
            }
        }
    }

    /// Force a full refresh on both frames.
    pub fn force_refresh(&mut self) {
        self.force_refresh = [true; 2];
    }

    /// Recalculate how many items fit in the visible area.
    fn update_visible_count(&mut self) {
        if self.item_height > 0.0
            && let Some(rect) = self.items_bbox.0
        {
            let height = rect.max().y - rect.min().y;
            self.visible_count = (height / self.item_height) as usize;
        }
    }

    // ── Scrolling ──────────────────────────────────────────────────

    /// Scroll up by one item.
    pub fn scroll_up(&mut self) -> bool {
        if self.first_visible > 0 {
            self.first_visible -= 1;
            true
        } else {
            false
        }
    }

    /// Scroll down by one item.
    pub fn scroll_down(&mut self) -> bool {
        let max_first = self.items.len().saturating_sub(self.visible_count);
        if self.first_visible < max_first {
            self.first_visible += 1;
            true
        } else {
            false
        }
    }

    /// Scroll so that the given item index is visible.
    pub fn ensure_visible(&mut self, index: usize) {
        if index < self.first_visible {
            self.first_visible = index;
        } else if index >= self.first_visible + self.visible_count {
            self.first_visible = index.saturating_sub(self.visible_count - 1);
        }
    }

    // ── Hit testing ──────────────────────────────────���─────────────

    /// Get the item index at a screen point, or None if not over an item.
    fn item_at_point(&self, point: ScreenPoint) -> Option<usize> {
        if !self.items_bbox.is_boxed_point(point.to_geo()) {
            return None;
        }
        if let Some(rect) = self.items_bbox.0 {
            let relative_y = point.y - rect.min().y;
            if relative_y < 0.0 || self.item_height <= 0.0 {
                return None;
            }
            let item_offset = (relative_y / self.item_height) as usize;
            let index = self.first_visible + item_offset;
            if index < self.items.len() {
                Some(index)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Check if a point is over the scrollbar area.
    fn is_over_scrollbar(&self, point: ScreenPoint) -> bool {
        self.scrollbar_bbox.is_boxed_point(point.to_geo())
    }

    /// Check if a point is over the scrollbar knob.
    fn is_over_knob(&self, point: ScreenPoint) -> bool {
        self.knob_bbox.is_boxed_point(point.to_geo())
    }

    // ── Input processing ───────────────────────────────────────────

    /// Process input for one frame. Drives the 7-state machine.
    pub fn process_input(&mut self, input: &WidgetInput) -> Vec<UiEvent> {
        if !self.base.enabled {
            return self.base.tooltip_event_if_disabled().into_iter().collect();
        }

        let buttons = input.mouse_button;
        let mouse = input.mouse_position;

        match self.state {
            ListboxState::Default => self.process_default(mouse, buttons, input.mouse_z),
            ListboxState::ItemsFocused => self.process_items_focused(mouse, buttons, input.mouse_z),
            ListboxState::ItemsPushed => self.process_items_pushed(mouse, buttons, input.mouse_z),
            ListboxState::ItemsSelected => {
                self.process_items_selected(mouse, buttons, input.mouse_z)
            }
            ListboxState::ScrollFocused => {
                self.process_scroll_focused(mouse, buttons, input.mouse_z)
            }
            ListboxState::ScrollPushed => self.process_scroll_pushed(mouse, buttons),
        }
    }

    // ── State handlers ─────────────────────────────────────────────

    fn process_default(
        &mut self,
        mouse: ScreenPoint,
        _buttons: MouseButtons,
        _mouse_z: i16,
    ) -> Vec<UiEvent> {
        if let Some(item_idx) = self.item_at_point(mouse) {
            self.state = ListboxState::ItemsFocused;
            self.focused = Some(item_idx);
            return vec![self.base.make_event_with_data(
                UiMsg::WidgetListFocusChange,
                UiEventData::ListIndex(item_idx as u32),
            )];
        }
        if self.is_over_scrollbar(mouse) {
            self.state = ListboxState::ScrollFocused;
        }
        Vec::new()
    }

    fn process_items_focused(
        &mut self,
        mouse: ScreenPoint,
        buttons: MouseButtons,
        mouse_z: i16,
    ) -> Vec<UiEvent> {
        let mut events = Vec::new();

        // Mouse wheel scrolling.
        if mouse_z != 0 {
            let scrolled = if mouse_z > 0 {
                self.scroll_up()
            } else {
                self.scroll_down()
            };
            if scrolled {
                let msg = if mouse_z > 0 {
                    UiMsg::WidgetScrollUp
                } else {
                    UiMsg::WidgetScrollDown
                };
                events.push(self.base.make_event(msg));
            }
        }

        // Double-click activates.
        if buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK)
            && let Some(item_idx) = self.item_at_point(mouse)
        {
            self.selected = Some(item_idx);
            events.push(self.base.make_event(UiMsg::WidgetActivated));
            return events;
        }

        // Right-click deselects.
        if buttons.contains(MouseButtons::RIGHT_CLICK) {
            self.selected = None;
            events.push(self.base.make_event(UiMsg::WidgetUnselect));
            return events;
        }

        // Left-down starts push (capture).
        if buttons.contains(MouseButtons::LEFT_DOWN) && self.item_at_point(mouse).is_some() {
            self.state = ListboxState::ItemsPushed;
            return events;
        }

        // Update focused item on mouse move.
        if let Some(item_idx) = self.item_at_point(mouse) {
            if self.focused != Some(item_idx) {
                self.focused = Some(item_idx);
                events.push(self.base.make_event_with_data(
                    UiMsg::WidgetListFocusChange,
                    UiEventData::ListIndex(item_idx as u32),
                ));
            }
        } else if self.is_over_scrollbar(mouse) {
            self.state = ListboxState::ScrollFocused;
            self.focused = None;
        } else {
            // Mouse left both items and scrollbar.
            self.state = ListboxState::Default;
            self.focused = None;
        }

        events
    }

    fn process_items_pushed(
        &mut self,
        mouse: ScreenPoint,
        buttons: MouseButtons,
        mouse_z: i16,
    ) -> Vec<UiEvent> {
        let mut events = Vec::new();

        // Mouse wheel.
        if mouse_z != 0 {
            let scrolled = if mouse_z > 0 {
                self.scroll_up()
            } else {
                self.scroll_down()
            };
            if scrolled {
                let msg = if mouse_z > 0 {
                    UiMsg::WidgetScrollUp
                } else {
                    UiMsg::WidgetScrollDown
                };
                events.push(self.base.make_event(msg));
            }
        }

        // Click inside items → select.
        if buttons.contains(MouseButtons::LEFT_CLICK) {
            if let Some(item_idx) = self.item_at_point(mouse) {
                self.selected = Some(item_idx);
                self.state = ListboxState::ItemsFocused;
                events.push(self.base.make_event_with_data(
                    UiMsg::WidgetListSelectChange,
                    UiEventData::ListIndex(item_idx as u32),
                ));
            } else {
                // Click outside items area.
                self.state = ListboxState::ItemsSelected;
            }
            return events;
        }

        // Double-click → activate.
        if buttons.contains(MouseButtons::LEFT_DOUBLE_CLICK) {
            if let Some(item_idx) = self.item_at_point(mouse) {
                self.selected = Some(item_idx);
                self.state = ListboxState::ItemsFocused;
                events.push(self.base.make_event(UiMsg::WidgetActivated));
            }
            return events;
        }

        // Mouse outside items while held → items selected (captured).
        if self.item_at_point(mouse).is_none() && !buttons.contains(MouseButtons::LEFT_DOWN) {
            self.state = ListboxState::ItemsSelected;
        }

        events
    }

    fn process_items_selected(
        &mut self,
        mouse: ScreenPoint,
        buttons: MouseButtons,
        mouse_z: i16,
    ) -> Vec<UiEvent> {
        let mut events = Vec::new();

        // Mouse wheel.
        if mouse_z != 0 {
            let scrolled = if mouse_z > 0 {
                self.scroll_up()
            } else {
                self.scroll_down()
            };
            if scrolled {
                let msg = if mouse_z > 0 {
                    UiMsg::WidgetScrollUp
                } else {
                    UiMsg::WidgetScrollDown
                };
                events.push(self.base.make_event(msg));
            }
        }

        // Click to transition out.
        if buttons.contains(MouseButtons::LEFT_CLICK) {
            if self.item_at_point(mouse).is_some() {
                self.state = ListboxState::ItemsPushed;
            } else if self.is_over_scrollbar(mouse) {
                self.state = ListboxState::ScrollFocused;
            } else {
                self.state = ListboxState::Default;
            }
        }

        events
    }

    fn process_scroll_focused(
        &mut self,
        mouse: ScreenPoint,
        buttons: MouseButtons,
        mouse_z: i16,
    ) -> Vec<UiEvent> {
        let mut events = Vec::new();

        // Mouse wheel.
        if mouse_z != 0 {
            let scrolled = if mouse_z > 0 {
                self.scroll_up()
            } else {
                self.scroll_down()
            };
            if scrolled {
                let msg = if mouse_z > 0 {
                    UiMsg::WidgetScrollUp
                } else {
                    UiMsg::WidgetScrollDown
                };
                events.push(self.base.make_event(msg));
            }
            return events;
        }

        // Click on scrollbar track (not knob) → page scroll.
        if buttons.contains(MouseButtons::LEFT_CLICK)
            && self.is_over_scrollbar(mouse)
            && !self.is_over_knob(mouse)
        {
            // Click above knob → scroll up, below → scroll down.
            if let Some(knob_rect) = self.knob_bbox.0 {
                let scrolled = if mouse.y < knob_rect.min().y {
                    // Page up.
                    for _ in 0..self.visible_count {
                        if !self.scroll_up() {
                            break;
                        }
                    }
                    true
                } else {
                    // Page down.
                    for _ in 0..self.visible_count {
                        if !self.scroll_down() {
                            break;
                        }
                    }
                    true
                };
                if scrolled {
                    events.push(self.base.make_event(UiMsg::WidgetScrollDown));
                }
            }
            return events;
        }

        // Left down on knob → start drag.
        if buttons.contains(MouseButtons::LEFT_DOWN) && self.is_over_knob(mouse) {
            self.state = ListboxState::ScrollPushed;
            self.drag_start_y = mouse.y;
            self.drag_start_first = self.first_visible;
            return events;
        }

        // Mouse moved to items area.
        if self.item_at_point(mouse).is_some() {
            self.state = ListboxState::ItemsFocused;
            return events;
        }

        // Mouse left scrollbar.
        if !self.is_over_scrollbar(mouse) && self.item_at_point(mouse).is_none() {
            self.state = ListboxState::Default;
        }

        events
    }

    fn process_scroll_pushed(&mut self, mouse: ScreenPoint, buttons: MouseButtons) -> Vec<UiEvent> {
        let mut events = Vec::new();

        if buttons.contains(MouseButtons::LEFT_DOWN) {
            // Dragging the knob.
            let delta_y = mouse.y - self.drag_start_y;
            if let Some(scroll_rect) = self.scrollbar_bbox.0 {
                let track_height = scroll_rect.max().y - scroll_rect.min().y;
                if track_height > 0.0 && !self.items.is_empty() {
                    let items_per_pixel = self.items.len() as f32 / track_height;
                    let item_delta = (delta_y * items_per_pixel) as isize;
                    let new_first = (self.drag_start_first as isize + item_delta).max(0) as usize;
                    let max_first = self.items.len().saturating_sub(self.visible_count);
                    let new_first = new_first.min(max_first);

                    if new_first != self.first_visible {
                        let msg = if new_first > self.first_visible {
                            UiMsg::WidgetScrollDown
                        } else {
                            UiMsg::WidgetScrollUp
                        };
                        self.first_visible = new_first;
                        events.push(self.base.make_event(msg));
                    }
                }
            }
        } else {
            // Released — transition based on mouse position.
            if self.item_at_point(mouse).is_some() {
                self.state = ListboxState::ItemsFocused;
            } else if self.is_over_scrollbar(mouse) {
                self.state = ListboxState::ScrollFocused;
            } else {
                self.state = ListboxState::Default;
            }
        }

        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_layout_empty_renders_single_cell() {
        let cl = ColumnLayout::default();
        let cells = cl.layout_row("only-text", 0.0, 100.0);
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].text, "only-text");
        assert_eq!(cells[0].span_x, 0.0);
        assert_eq!(cells[0].span_w, 100.0);
    }

    #[test]
    fn column_layout_three_full_cells() {
        let cl = ColumnLayout::new(&[
            (0.25, ColumnAlign::Left),
            (0.5, ColumnAlign::Center),
            (0.25, ColumnAlign::Right),
        ]);
        let cells = cl.layout_row("a|b|c", 10.0, 100.0);
        assert_eq!(cells.len(), 3);
        assert_eq!(
            (cells[0].text, cells[0].span_x, cells[0].span_w),
            ("a", 10.0, 25.0)
        );
        assert_eq!(
            (cells[1].text, cells[1].span_x, cells[1].span_w),
            ("b", 35.0, 50.0)
        );
        assert_eq!(
            (cells[2].text, cells[2].span_x, cells[2].span_w),
            ("c", 85.0, 25.0)
        );
        assert_eq!(cells[1].align, ColumnAlign::Center);
        assert_eq!(cells[2].align, ColumnAlign::Right);
    }

    #[test]
    fn column_layout_absorbs_trailing_empty_cells() {
        let cl = ColumnLayout::new(&[
            (0.3, ColumnAlign::Left),
            (0.3, ColumnAlign::Left),
            (0.4, ColumnAlign::Left),
        ]);
        // Single-cell text → first column spans the full row width.
        let cells = cl.layout_row("< New Save >", 0.0, 100.0);
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].text, "< New Save >");
        assert_eq!(cells[0].span_x, 0.0);
        assert!((cells[0].span_w - 100.0).abs() < 1e-4);
    }

    #[test]
    fn column_layout_absorbs_middle_empty_cell() {
        let cl = ColumnLayout::new(&[
            (0.3, ColumnAlign::Left),
            (0.3, ColumnAlign::Left),
            (0.4, ColumnAlign::Left),
        ]);
        // Middle cell empty → first column spans cols 0+1, third column
        // renders at its own offset.
        let cells = cl.layout_row("a||c", 0.0, 100.0);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].text, "a");
        assert!((cells[0].span_x - 0.0).abs() < 1e-3);
        assert!((cells[0].span_w - 60.0).abs() < 1e-3);
        assert_eq!(cells[1].text, "c");
        assert!((cells[1].span_x - 60.0).abs() < 1e-3);
        assert!((cells[1].span_w - 40.0).abs() < 1e-3);
    }

    #[test]
    fn column_layout_too_many_pipes_absorbed_into_last_cell() {
        let cl = ColumnLayout::new(&[(0.5, ColumnAlign::Left), (0.5, ColumnAlign::Left)]);
        // Extra pipes past the column count stay in the trailing cell —
        // the file name column shouldn't swallow a pipe buried in a save
        // display name.
        let cells = cl.layout_row("a|b|c", 0.0, 100.0);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].text, "a");
        assert_eq!(cells[1].text, "b|c");
    }
}
