//! Short briefings list rendering.
//!
//! Shows primary and secondary mission objectives on the left half of
//! the pause menu. The data model already lives in
//! [`robin_engine::short_briefings`]; this file owns the visual layout.

use crate::renderer::Renderer;
use robin_engine::short_briefings::ShortBriefings;

use super::layout::{
    MenuRect, MenuTransform, TextAlign, draw_background, measure_text_height_in_box,
    render_text_in_box,
};
use super::resources::IngameMenuResources;

/// Vertical gap between successive briefing entries.
const SPACING: i32 = 3;
/// Left indent reserved for the completion check-mark.
const MARGIN: i32 = 15;

/// Render the short briefings list inside a virtual bounding box.
///
/// Draws each briefing with the active or inactive font depending on
/// its `done` flag, inserting a separator between primary and secondary
/// sections when both are present.
///
/// Briefing text is resolved from the level's short-briefing text-table
/// id via the supplied lookup.  When the lookup returns `None`
/// (no level resources loaded) the briefing id is shown as a placeholder
/// so the layout stays visible in development builds.
pub fn draw_short_briefings(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    transform: MenuTransform,
    rect: &MenuRect,
    briefings: &ShortBriefings,
    lookup: &dyn Fn(u32) -> Option<String>,
) {
    let mut cursor_y = rect.y;

    if briefings.count(true) > 0 {
        cursor_y = draw_section(
            renderer, resources, transform, rect, cursor_y, briefings, true, lookup,
        );
    }

    // Separator between primary and secondary (`RHID_SEPARATOR`).
    // The separator bitmap is a required asset; if it's missing we log
    // a warning and skip the advance entirely rather than emit a
    // half-positioned 10 px stub.
    if briefings.count(true) > 0 && briefings.count(false) > 0 {
        if let Some(sep) = resources.separator {
            let sep_w = sep.width.min(rect.w);
            let sep_h = sep.height;
            let sep_x = rect.x + (rect.w - sep_w) / 2;
            cursor_y += 10;
            draw_background(renderer, transform, &sep, sep_x, cursor_y, sep_w, sep_h);
            cursor_y += sep_h + 10;
        } else {
            tracing::warn!("short briefings: separator asset (RHID_SEPARATOR) missing; skipping");
        }
    }

    if briefings.count(false) > 0 {
        draw_section(
            renderer, resources, transform, rect, cursor_y, briefings, false, lookup,
        );
    }
}

/// Render a single primary/secondary briefing section and return the new
/// cursor Y after the last entry.
#[allow(clippy::too_many_arguments)]
fn draw_section(
    renderer: &mut Renderer,
    resources: &IngameMenuResources,
    transform: MenuTransform,
    rect: &MenuRect,
    start_y: i32,
    briefings: &ShortBriefings,
    primary: bool,
    lookup: &dyn Fn(u32) -> Option<String>,
) -> i32 {
    let count = briefings.count(primary);
    let mut y = start_y;
    for i in 0..count {
        let done = briefings.is_entry_done(primary, i).unwrap_or(false);
        let id = match briefings.get_id(primary, i) {
            Some(id) => id,
            None => continue,
        };
        // Fixed placeholder when the string-table lookup fails.
        let text = lookup(id).unwrap_or_else(|| "Invalid short briefing ID...".to_string());

        let font = if done {
            resources.inactive_briefing_font()
        } else {
            resources.active_briefing_font()
        };
        let Some(font) = font else { continue };

        // Check mark is drawn in the left margin for completed entries.
        if done && let Some(check) = resources.check_mark {
            let cx = rect.x;
            let cy = y;
            draw_background(
                renderer,
                transform,
                &check,
                cx,
                cy,
                check.width,
                check.height,
            );
        }

        // Body text is indented to make room for the checkmark.
        let text_x = rect.x + MARGIN;
        let text_w = (rect.w - MARGIN).max(0);
        // Keep rendering even when entries fall outside `position` and
        // let the widget renderer clip; pass the full remaining_h
        // budget but don't break the loop early.
        let remaining_h = (rect.h - (y - rect.y)).max(font.height() as i32);
        // Both this measurement and `render_text_in_box` below route
        // through the same `wrap_text` call with the same
        // `max_lines = remaining_h / line_h`, so the advance can't
        // drift from the rasterised line count.
        let needed_h =
            measure_text_height_in_box(font, &text, text_w, remaining_h).max(font.height() as i32);
        let _ = render_text_in_box(
            renderer,
            font,
            transform,
            &text,
            text_x,
            y,
            text_w,
            remaining_h,
            TextAlign::Justified,
        );
        y += needed_h + SPACING;
    }
    y
}
