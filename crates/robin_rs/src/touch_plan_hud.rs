//! Sticky touch control for automatic quick-action planning.

use crate::renderer::Renderer;

pub const WIDTH: i32 = 118;
pub const HEIGHT: i32 = 34;

pub const fn platform_has_touch_planning_hud() -> bool {
    cfg!(any(
        target_os = "android",
        target_os = "ios",
        target_arch = "wasm32"
    ))
}

pub fn rect(screen_width: u16) -> (i32, i32, i32, i32) {
    let right = i32::from(screen_width).saturating_sub(10);
    (right - WIDTH, 46, right, 46 + HEIGHT)
}

pub fn hit_test(screen_width: u16, x: i32, y: i32) -> bool {
    let (left, top, right, bottom) = rect(screen_width);
    (left..=right).contains(&x) && (top..=bottom).contains(&y)
}

pub fn render(renderer: &mut Renderer, fonts: Option<&crate::hud_text::HudFonts>, active: bool) {
    if !platform_has_touch_planning_hud() {
        return;
    }
    let (left, top, right, bottom) = rect(renderer.screen_width());
    let border = if active {
        Renderer::create_color_16(238, 192, 55)
    } else {
        Renderer::create_color_16(224, 211, 157)
    };
    renderer.draw_rect_outline_screen(left, top, right, bottom, border);
    renderer.draw_rect_outline_screen(left + 2, top + 2, right - 2, bottom - 2, border);
    if let Some(fonts) = fonts {
        let label = if active {
            "CANCEL PLAN"
        } else {
            "PLAN ACTIONS"
        };
        let width = fonts.tooltip_font.text_width(label);
        crate::ingame_menu::layout::render_text_screen_font(
            renderer,
            &fonts.tooltip_font,
            label,
            left + (WIDTH - width) / 2,
            top + (HEIGHT - fonts.tooltip_font.height() as i32) / 2,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_box_tracks_right_edge_and_includes_boundary() {
        let (left, top, right, bottom) = rect(1024);
        assert!(hit_test(1024, left, top));
        assert!(hit_test(1024, right, bottom));
        assert!(!hit_test(1024, left - 1, top));
    }
}
