//! Row-based scrolling for menu lists and wrapped text, independent of selection.
use crate::gfx_types::{GameEvent, Keycode};
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
        self.scroll_distance(step.unsigned_abs(), step < 0);
    }
    fn scroll_distance(&mut self, distance: usize, backwards: bool) {
        self.offset = if backwards {
            self.offset.saturating_sub(distance)
        } else {
            self.offset.saturating_add(distance)
        }
        .min(self.max_offset());
    }
    /// Scroll-only navigation. Selectable lists may handle arrows themselves
    /// and call `reveal` after changing selection.
    pub fn navigate(&mut self, key: Keycode) -> bool {
        match key {
            Keycode::Up => self.scroll_by(-1),
            Keycode::Down => self.scroll_by(1),
            Keycode::PageUp => self.scroll_by(-(self.visible_count() as isize)),
            Keycode::PageDown => self.scroll_by(self.visible_count() as isize),
            Keycode::Home => self.reset(),
            Keycode::End => self.set_offset(usize::MAX),
            _ => return false,
        }
        true
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
    fn track_origin(&self) -> (i32, i32) {
        if self.horizontal {
            (
                self.bounds[0],
                self.bounds[1] + self.bounds[3] - self.scrollbar_width,
            )
        } else {
            (self.bounds[0] + self.content_width(), self.bounds[1])
        }
    }
    fn in_track(&self, x: i32, y: i32) -> bool {
        let (track_x, track_y) = self.track_origin();
        self.contains(x, y) && x >= track_x && y >= track_y
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
        if self.max_offset() == 0 {
            return;
        }
        let (_, height) = widget_bridge::listbox_scrollbar_thumb(
            self.length(),
            self.offset,
            self.visible_count(),
            self.total,
            self.min_thumb_height,
        );
        let travel = (self.length() - 2 - height).max(0) as usize;
        if travel == 0 {
            return;
        }
        let relative = i64::from(y) - i64::from(self.origin()) - 1 - i64::from(grab);
        let scaled = relative.max(0) as u128 * self.max_offset() as u128 + travel as u128 / 2;
        self.offset = (scaled / travel as u128).min(self.max_offset() as u128) as usize;
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
                let distance =
                    (delta.unsigned_abs() as usize).saturating_mul(self.wheel_step as usize);
                self.scroll_distance(distance, delta > 0);
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
                if !(top..top + height).contains(&relative) {
                    self.drag_to(self.axis(x, y), grab);
                }
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
            let (x, y) = self.track_origin();
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
    fn track_origin_and_hit_testing_share_horizontal_and_vertical_boundaries() {
        for horizontal in [false, true] {
            for width in [1, 16, 23] {
                let mut v =
                    ScrollView::with_geometry([-10, 20, 200, 103], 20, width, 16, horizontal);
                let expected_origin = if horizontal {
                    (-10, 123 - width)
                } else {
                    (190 - width, 20)
                };
                assert_eq!(v.track_origin(), expected_origin);
                for total in [0, 1, 100] {
                    v.set_total(total);
                    for x in [-11, -10, 189 - width, 190 - width, 189, 190] {
                        for y in [19, 20, 122 - width, 123 - width, 122, 123] {
                            let inside = (-10..190).contains(&x) && (20..123).contains(&y);
                            let expected = inside
                                && if horizontal {
                                    y >= 123 - width
                                } else {
                                    x >= 190 - width
                                };
                            assert_eq!(
                                v.in_track(x, y),
                                expected,
                                "{horizontal}, {width}, {total}, ({x}, {y})"
                            );
                            if expected {
                                assert_eq!(v.row_at(x, y), None);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn minimum_thumb_stays_inside_track_and_reaches_last_row() {
        let mut v = view();
        v.set_total(10_000);
        v.set_offset(usize::MAX);
        let (top, height) = widget_bridge::listbox_scrollbar_thumb(
            v.length(),
            v.offset,
            v.visible_count(),
            v.total,
            v.min_thumb_height,
        );
        assert_eq!(height, 16);
        assert_eq!(top + height, v.length() - 1);
        v.drag_to(v.origin() + top, 0);
        assert_eq!(v.offset, 9995);
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
    fn wheel_and_programmatic_scrolling_share_direction_and_clamping() {
        let transform = MenuTransform::centered(640, 480);
        for start in [0, 2, 50, 94, 95] {
            for delta in [-40, -2, -1, 0, 1, 2, 40] {
                let mut wheel = view();
                wheel.set_offset(start);
                let mut direct = wheel.clone();
                assert!(wheel.handle_event(&GameEvent::MouseWheel(delta), transform, (30, 30)));
                direct.scroll_by(-(delta as isize) * 3);
                assert_eq!(wheel.offset(), direct.offset());
            }
        }
    }

    #[test]
    fn extreme_scroll_distances_clamp_without_signed_overflow() {
        let transform = MenuTransform::centered(640, 480);
        let mut v = view();
        v.set_wheel_step(isize::MAX);
        for delta in [i32::MIN, -2, -1, 1, 2, i32::MAX] {
            v.set_offset(50);
            assert!(v.handle_event(&GameEvent::MouseWheel(delta), transform, (30, 30)));
            assert_eq!(v.offset(), if delta < 0 { 95 } else { 0 });
        }
        v.set_offset(50);
        v.scroll_by(isize::MIN);
        assert_eq!(v.offset(), 0);
        v.scroll_by(isize::MAX);
        assert_eq!(v.offset(), 95);

        // A saturated wheel distance can cover even a usize-sized logical list.
        v.set_total(usize::MAX);
        v.handle_event(&GameEvent::MouseWheel(i32::MIN), transform, (30, 30));
        assert_eq!(v.offset(), v.max_offset());
        v.handle_event(&GameEvent::MouseWheel(i32::MAX), transform, (30, 30));
        assert_eq!(v.offset(), 0);
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
    fn drag_scaling_uses_actual_thumb_travel_for_ordinary_lists() {
        for horizontal in [false, true] {
            for total in [0, 1, 5, 6, 100, 10_000] {
                let mut v = view();
                v.horizontal = horizontal;
                v.set_total(total);
                for grab in [0, 1, 8, 16] {
                    for position in -20..250 {
                        let height = if total == 0 {
                            v.length() - 2
                        } else {
                            widget_bridge::listbox_scrollbar_thumb(
                                v.length(),
                                v.offset(),
                                v.visible_count(),
                                total,
                                v.min_thumb_height,
                            )
                            .1
                        };
                        let travel = (v.length() - 2 - height).max(0) as usize;
                        let expected = if travel == 0 {
                            v.offset()
                        } else {
                            (((position - v.origin() - 1 - grab).max(0) as usize * v.max_offset()
                                + travel / 2)
                                / travel)
                                .min(v.max_offset())
                        };
                        v.drag_to(position, grab);
                        assert_eq!(v.offset(), expected);
                    }
                }
            }
        }
    }

    #[test]
    fn drag_scaling_handles_extreme_coordinates_and_large_content_counts() {
        let mut v = view();
        v.set_total(usize::MAX);
        v.drag_to(i32::MIN, 8);
        assert_eq!(v.offset(), 0);
        v.drag_to(i32::MAX, 8);
        assert_eq!(v.offset(), v.max_offset());
        // The minimum 16-pixel thumb leaves 85 pixels of travel.
        v.drag_to(v.origin() + 2, 0);
        assert_eq!(v.offset(), ((v.max_offset() as u128 + 42) / 85) as usize);

        v.bounds[1] = i32::MIN;
        v.drag_to(i32::MAX, 0);
        assert_eq!(v.offset(), v.max_offset());
        v.bounds[1] = i32::MAX - v.bounds[3];
        v.drag_to(i32::MIN, 8);
        assert_eq!(v.offset(), 0);
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
