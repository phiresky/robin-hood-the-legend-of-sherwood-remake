//! Row-based scrolling for menu lists and wrapped text, independent of selection.
use crate::gfx_types::GameEvent;
use crate::ingame_menu::resources::MenuSurface;
use crate::ingame_menu::{IngameMenuResources, layout::MenuTransform, widget_bridge};
use crate::renderer::Renderer;
use serde::{Deserialize, Serialize};

/// A viewport in virtual menu coordinates. Callers draw only `visible_range()`
/// and retain ownership of content and selection. The scrollbar gutter is
/// reserved even when content fits, so wrapping does not change on overflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrollView {
    bounds: [i32; 4],
    horizontal: bool,
    wheel_step: isize,
    row_height: i32,
    scrollbar_width: i32,
    min_thumb_height: i32,
    total: usize,
    offset: usize,
    #[serde(skip)]
    drag_grab: Option<i32>,
}

impl ScrollView {
    pub fn new(bounds: [i32; 4], row_height: i32, resources: &IngameMenuResources) -> Self {
        let slices = resources.list_scrollbar.map(|slice| {
            slice.expect("scroll view requires the complete listbox scrollbar artwork")
        });
        let view = Self {
            bounds,
            horizontal: false,
            wheel_step: 3,
            row_height,
            scrollbar_width: slices[0].width,
            min_thumb_height: slices[3].height + slices[5].height,
            total: 0,
            offset: 0,
            drag_grab: None,
        };
        assert!(
            row_height > 0 && bounds[3] >= row_height && bounds[3] > 2,
            "scroll view must fit at least one row"
        );
        assert!(
            view.scrollbar_width > 0 && bounds[2] > view.scrollbar_width,
            "scroll view must fit content beside its scrollbar"
        );
        view
    }

    /// Construct with theme metrics when the caller owns a borrowed scrollbar skin.
    pub fn with_geometry(
        bounds: [i32; 4],
        row_height: i32,
        width: i32,
        min_thumb: i32,
        horizontal: bool,
    ) -> Self {
        let length = if horizontal { bounds[2] } else { bounds[3] };
        assert!(row_height > 0 && length >= row_height && length > 2);
        assert!(width > 0 && min_thumb > 0);
        Self {
            bounds,
            row_height,
            scrollbar_width: width,
            min_thumb_height: min_thumb,
            horizontal,
            wheel_step: 1,
            total: 0,
            offset: 0,
            drag_grab: None,
        }
    }
    pub fn offset(&self) -> usize {
        self.offset
    }
    pub fn set_offset(&mut self, offset: usize) {
        self.offset = offset.min(self.max_offset());
    }
    pub fn scroll_by(&mut self, step: isize) {
        self.offset = self
            .offset
            .saturating_add_signed(step)
            .min(self.max_offset());
    }
    pub fn set_wheel_step(&mut self, step: isize) {
        assert!(step > 0);
        self.wheel_step = step;
    }
    fn length(&self) -> i32 {
        if self.horizontal {
            self.bounds[2]
        } else {
            self.bounds[3]
        }
    }
    fn origin(&self) -> i32 {
        if self.horizontal {
            self.bounds[0]
        } else {
            self.bounds[1]
        }
    }
    fn axis(&self, x: i32, y: i32) -> i32 {
        if self.horizontal { x } else { y }
    }
    fn in_track(&self, x: i32, y: i32) -> bool {
        self.contains(x, y)
            && if self.horizontal {
                y >= self.bounds[1] + self.bounds[3] - self.scrollbar_width
            } else {
                x >= self.bounds[0] + self.content_width()
            }
    }
    pub fn visible_count(&self) -> usize {
        (self.length() / self.row_height) as usize
    }
    pub fn visible_range(&self) -> std::ops::Range<usize> {
        self.offset..(self.offset + self.visible_count()).min(self.total)
    }
    pub fn content_width(&self) -> i32 {
        self.bounds[2]
            - if self.horizontal {
                0
            } else {
                self.scrollbar_width
            }
    }
    pub fn row_y(&self, row: usize) -> i32 {
        assert!(
            self.visible_range().contains(&row),
            "row is outside the viewport"
        );
        self.origin() + (row - self.offset) as i32 * self.row_height
    }
    pub fn set_total(&mut self, total: usize) {
        if self.total != total {
            self.drag_grab = None;
        }
        self.total = total;
        self.offset = self.offset.min(self.max_offset());
    }
    pub fn reset(&mut self) {
        self.offset = 0;
        self.drag_grab = None;
    }
    pub fn reveal(&mut self, row: usize) {
        assert!(row < self.total, "cannot reveal a missing row");
        if row < self.offset {
            self.offset = row;
        } else if row >= self.offset + self.visible_count() {
            self.offset = row + 1 - self.visible_count();
        }
        self.drag_grab = None;
    }
    fn max_offset(&self) -> usize {
        self.total.saturating_sub(self.visible_count())
    }
    fn contains(&self, x: i32, y: i32) -> bool {
        (self.bounds[0]..self.bounds[0] + self.bounds[2]).contains(&x)
            && (self.bounds[1]..self.bounds[1] + self.bounds[3]).contains(&y)
    }
    /// Hit-test content only; scrollbar clicks and the partial bottom row
    /// must never select a list item.
    pub fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.contains(x, y) || self.in_track(x, y) {
            return None;
        }
        let row = self.offset + ((self.axis(x, y) - self.origin()) / self.row_height) as usize;
        self.visible_range().contains(&row).then_some(row)
    }
    fn drag_to(&mut self, y: i32, grab: i32) {
        let usable = (self.length() - 2) as usize;
        self.offset = (((y - self.origin() - 1 - grab).max(0) as usize * self.total + usable / 2)
            / usable)
            .min(self.max_offset());
    }
    /// Returns true when scrolling consumed the event. Pass the current mouse
    /// position in virtual coordinates for wheel events, which carry no position.
    pub fn handle_event(
        &mut self,
        event: &GameEvent,
        transform: MenuTransform,
        pointer: (i32, i32),
    ) -> bool {
        match *event {
            GameEvent::MouseWheel(delta) if self.contains(pointer.0, pointer.1) => {
                self.offset = self
                    .offset
                    .saturating_add_signed(-(delta as isize) * self.wheel_step)
                    .min(self.max_offset());
                true
            }
            GameEvent::MouseDown(x, y, 1, _) => {
                let (x, y) = transform.from_screen(x, y);
                if self.max_offset() == 0 || !self.in_track(x, y) {
                    return false;
                }
                let (top, height) = widget_bridge::listbox_scrollbar_thumb(
                    self.length(),
                    self.offset,
                    self.visible_count(),
                    self.total,
                    self.min_thumb_height,
                );
                let relative = self.axis(x, y) - self.origin();
                let grab = if (top..top + height).contains(&relative) {
                    relative - top
                } else {
                    height / 2
                };
                self.drag_grab = Some(grab);
                self.drag_to(self.axis(x, y), grab);
                true
            }
            GameEvent::MouseMove { x, y, .. } if self.drag_grab.is_some() => {
                let (x, y) = transform.from_screen(x, y);
                self.drag_to(
                    self.axis(x, y),
                    self.drag_grab.expect("active scrollbar drag"),
                );
                true
            }
            GameEvent::MouseUp(_, _, 1) => self.drag_grab.take().is_some(),
            GameEvent::PointerCancel => {
                self.drag_grab = None;
                false
            }
            _ => false,
        }
    }
    pub fn draw_scrollbar(
        &self,
        renderer: &mut Renderer,
        transform: MenuTransform,
        resources: &IngameMenuResources,
    ) {
        self.draw_skin(renderer, transform, &resources.list_scrollbar);
    }
    pub fn draw_skin(
        &self,
        renderer: &mut Renderer,
        transform: MenuTransform,
        skin: &[Option<MenuSurface>; 6],
    ) {
        if self.max_offset() > 0 {
            let (x, y) = if self.horizontal {
                (
                    self.bounds[0],
                    self.bounds[1] + self.bounds[3] - self.scrollbar_width,
                )
            } else {
                (self.bounds[0] + self.content_width(), self.bounds[1])
            };
            widget_bridge::draw_scrollbar_slices(
                renderer,
                transform,
                skin,
                x,
                y,
                self.scrollbar_width,
                self.length(),
                self.offset,
                self.visible_count(),
                self.total,
                self.horizontal,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn view() -> ScrollView {
        ScrollView {
            bounds: [10, 20, 200, 103],
            horizontal: false,
            wheel_step: 3,
            row_height: 20,
            scrollbar_width: 16,
            min_thumb_height: 16,
            total: 100,
            offset: 0,
            drag_grab: None,
        }
    }
    #[test]
    fn horizontal_view_uses_x_for_dragging_and_a_bottom_scrollbar() {
        let mut v = ScrollView::with_geometry([30, 40, 400, 100], 100, 16, 16, true);
        v.set_total(20);
        let t = MenuTransform::centered(640, 480);
        assert_eq!(v.visible_count(), 4);
        assert_eq!(v.row_at(135, 50), Some(1));
        assert_eq!(v.row_at(135, 130), None);
        let (x, y) = t.to_screen(40, 130);
        assert!(v.handle_event(&GameEvent::MouseDown(x, y, 1, 1), t, (40, 130)));
        let (x, y) = t.to_screen(800, 130);
        assert!(v.handle_event(
            &GameEvent::MouseMove {
                x,
                y,
                xrel: 0,
                yrel: 0
            },
            t,
            (800, 130)
        ));
        assert_eq!(v.visible_range(), 16..20);
    }

    #[test]
    fn wheel_is_scoped_to_hovered_view_and_does_not_reveal_selection() {
        let mut left = view();
        let mut right = view();
        right.bounds[0] = 230;
        let t = MenuTransform::centered(640, 480);
        assert!(left.handle_event(&GameEvent::MouseWheel(-2), t, (30, 30)));
        assert!(!right.handle_event(&GameEvent::MouseWheel(-2), t, (30, 30)));
        assert_eq!(left.visible_range(), 6..11);
        assert_eq!(right.visible_range(), 0..5);
        left.reveal(0);
        assert_eq!(left.visible_range(), 0..5);
    }
    #[test]
    fn content_hit_test_excludes_scrollbar_and_partial_rows() {
        let v = view();
        assert_eq!(v.row_at(20, 21), Some(0));
        assert_eq!(v.row_at(20, 119), Some(4));
        assert_eq!(v.row_at(200, 21), None);
        assert_eq!(v.row_at(20, 121), None);
        assert_eq!(v.row_at(9, 21), None);
    }
    #[test]
    fn drag_matches_shared_artwork_mapping_and_clamps_at_both_ends() {
        let mut v = view();
        v.offset = 50;
        let (top, _) = widget_bridge::listbox_scrollbar_thumb(103, 50, 5, 100, 16);
        v.drag_to(v.bounds[1] + top + 8, 8);
        assert_eq!(v.offset, 50);
        v.drag_to(-100, 8);
        assert_eq!(v.offset, 0);
        v.drag_to(1000, 8);
        assert_eq!(v.offset, 95);
        v.set_total(2);
        assert_eq!(v.visible_range(), 0..2);
    }
    #[test]
    fn dragging_outside_view_consumes_release_and_cancellation_stops_drag() {
        let mut v = view();
        let t = MenuTransform::centered(640, 480);
        let (x, y) = t.to_screen(200, 25);
        assert!(v.handle_event(&GameEvent::MouseDown(x, y, 1, 1), t, (200, 25)));
        assert!(v.handle_event(&GameEvent::MouseUp(500, 500, 1), t, (500, 500)));
        assert!(!v.handle_event(&GameEvent::MouseUp(x, y, 1), t, (200, 25)));
        v.drag_grab = Some(3);
        v.handle_event(&GameEvent::PointerCancel, t, (200, 25));
        assert_eq!(v.drag_grab, None);
    }
}
