//! Procedural combat-gesture guide and post-stroke coaching overlay.
//!
//! No new bitmap assets are required: authored normalized paths are rendered
//! with the existing screen-line primitive and HUD bitmap font.

use crate::host::Host;
use crate::hud_text::HudFonts;
use crate::mouse_way::{
    LEGACY_PATTERNS, MouseWayPattern, display_template, display_template_rotation, pattern_label,
};
use crate::renderer::Renderer;
use robin_engine::coordinates::ScreenVec;
use robin_engine::engine::Engine;
use robin_engine::player_command::CompositeSwordTechnique;

/// Render enabled gesture help after the world scene and mouse trail but
/// before the portrait panel.
pub fn render(host: &mut Host, engine: &Engine, renderer: &mut Renderer, fonts: Option<&HudFonts>) {
    let swordfighting =
        crate::game_input::is_selected_unit_swordfighting(engine, host.transport.local_seat);
    if swordfighting && host.gameplay_config.show_combat_gesture_guide {
        let facing = first_selected_swordfighter_direction(engine, host)
            .unwrap_or(ScreenVec::new(0.0, -1.0));
        render_guide(
            renderer,
            fonts,
            engine.sim_config().more_combat_gestures,
            facing,
        );
    }

    if host.gameplay_config.combat_gesture_coach {
        render_coach(host, renderer, fonts);
    } else {
        host.gesture_coach_feedback = None;
    }
}

fn first_selected_swordfighter_direction(engine: &Engine, host: &Host) -> Option<ScreenVec> {
    engine
        .hero_selection(host.transport.local_seat)
        .iter()
        .chain(engine.tactical_selection(host.transport.local_seat))
        .filter_map(|id| engine.get_entity(*id))
        .find(|entity| {
            entity
                .human_data()
                .is_some_and(|human| !human.opponents.is_empty())
        })
        .map(|entity| {
            let direction =
                crate::shadow_polygon::sector_to_direction(entity.element_data().direction());
            ScreenVec::new(
                direction[0],
                direction[1] * crate::shadow_polygon::ASPECT_RATIO,
            )
        })
}

fn render_guide(
    renderer: &mut Renderer,
    fonts: Option<&HudFonts>,
    show_composites: bool,
    facing: ScreenVec,
) {
    let columns = 3_i32;
    let cell_w = 92_i32;
    let cell_h = 50_i32;
    let rows = if show_composites { 6 } else { 3 };
    let origin_x = (renderer.screen_width() as i32 - columns * cell_w - 8).max(0);
    let origin_y = 8;
    renderer.draw_rect_outline_screen(
        origin_x - 3,
        origin_y - 3,
        origin_x + columns * cell_w + 2,
        origin_y + rows * cell_h + 2,
        0x7BEF,
    );

    let composites = CompositeSwordTechnique::ALL.map(MouseWayPattern::Composite);
    for (index, pattern) in LEGACY_PATTERNS
        .into_iter()
        .chain(
            composites
                .into_iter()
                .take(if show_composites { 9 } else { 0 }),
        )
        .enumerate()
    {
        let column = index as i32 % columns;
        let row = index as i32 / columns;
        let x = origin_x + column * cell_w;
        let y = origin_y + row * cell_h;
        let color = if matches!(pattern, MouseWayPattern::Composite(_)) {
            0x07FF
        } else {
            0xFFE0
        };
        draw_template(
            renderer,
            pattern,
            x + 8,
            y + 4,
            cell_w - 16,
            29,
            color,
            display_template_rotation(pattern, facing),
        );
        draw_label(renderer, fonts, pattern_label(pattern), x + 3, y + 34);
    }
}

fn render_coach(host: &mut Host, renderer: &mut Renderer, fonts: Option<&HudFonts>) {
    let Some(feedback) = host.gesture_coach_feedback else {
        return;
    };
    let now = crate::window::process_uptime_ms();
    if now.wrapping_sub(feedback.created_at_ms) > 1_800 {
        host.gesture_coach_feedback = None;
        return;
    }

    let (lo, hi) = feedback.bounds;
    let quality = feedback.quality.permille();
    let label = format!("{}  {}%", pattern_label(feedback.pattern), quality / 10);
    let label_width = fonts
        .map(|fonts| fonts.tooltip_font.text_width(&label) + 8)
        .unwrap_or(44);
    let max_width = (renderer.screen_width() as i32 - 4).max(1);
    let width = ((hi.x - lo.x).abs().round() as i32)
        .max(label_width)
        .max(44)
        .min(max_width);
    let height = ((hi.y - lo.y).abs().round() as i32).clamp(44, 160);
    let x = (lo.x.round() as i32).clamp(2, (renderer.screen_width() as i32 - width - 2).max(2));
    let y =
        (lo.y.round() as i32 - 18).clamp(2, (renderer.screen_height() as i32 - height - 20).max(2));
    let color = match quality {
        750..=1000 => 0x07E0,
        500..=749 => 0xFFE0,
        _ => 0xF800,
    };
    renderer.draw_rect_outline_screen(x, y, x + width, y + height + 16, color);
    draw_template(
        renderer,
        feedback.pattern,
        x + 5,
        y + 16,
        width - 10,
        height - 5,
        color,
        feedback.template_rotation,
    );
    draw_label(renderer, fonts, &label, x + 4, y + 2);
}

fn draw_template(
    renderer: &mut Renderer,
    pattern: MouseWayPattern,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    color: u16,
    rotation: f32,
) {
    let Some(points) = display_template(pattern) else {
        return;
    };
    let (sin, cos) = rotation.sin_cos();
    // Preserve the authored aspect ratio. The compact guide cells are much
    // wider than they are tall; independent X/Y stretching would teach a
    // shape that the recognizer correctly rejects.
    let extent = width.min(height) as f32 * 0.5;
    let center_x = x as f32 + width as f32 * 0.5;
    let center_y = y as f32 + height as f32 * 0.5;
    let transform = |point: (f32, f32)| {
        let point = (point.0 * cos - point.1 * sin, point.0 * sin + point.1 * cos);
        (
            (center_x + point.0 * extent).round() as i32,
            (center_y + point.1 * extent).round() as i32,
        )
    };
    for pair in points.windows(2) {
        let a = transform(pair[0]);
        let b = transform(pair[1]);
        renderer.draw_line_screen(a.0, a.1, b.0, b.1, color);
    }
    if let Some(pair) = points.windows(2).last() {
        let a = transform(pair[0]);
        let b = transform(pair[1]);
        let dx = (b.0 - a.0) as f32;
        let dy = (b.1 - a.1) as f32;
        let length = dx.hypot(dy);
        if length > 0.5 {
            let ux = dx / length;
            let uy = dy / length;
            let left = (
                b.0 - (ux * 7.0 - uy * 4.0).round() as i32,
                b.1 - (uy * 7.0 + ux * 4.0).round() as i32,
            );
            let right = (
                b.0 - (ux * 7.0 + uy * 4.0).round() as i32,
                b.1 - (uy * 7.0 - ux * 4.0).round() as i32,
            );
            renderer.draw_line_screen(b.0, b.1, left.0, left.1, color);
            renderer.draw_line_screen(b.0, b.1, right.0, right.1, color);
        }
    }
}

fn draw_label(renderer: &mut Renderer, fonts: Option<&HudFonts>, label: &str, x: i32, y: i32) {
    let Some(fonts) = fonts else { return };
    crate::hud_text::render_text_background(
        &fonts.tooltip_font,
        fonts.shadow_font.as_ref(),
        label,
        x,
        y,
        |font, text, tx, ty| {
            crate::ingame_menu::layout::render_text_screen_font(renderer, font, text, tx, ty)
        },
    );
}
