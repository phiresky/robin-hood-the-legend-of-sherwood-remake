//! Mission minimap rendering.

use crate::host::Host;
use crate::renderer::{BLIT_SOURCE_TRANSPARENT, Renderer};
use robin_engine::coordinates as engine_coordinates;
use robin_engine::engine as engine_api;
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::minimap as engine_minimap;
use robin_engine::minimap::UIState;
use robin_engine::sprite::BBox;

// ─── Minimap rendering ─────────────────────────────────────────────

/// Blits the minimap bitmap at its current position, the viewport
/// indicator rectangle for the current camera view, and a dot per
/// active entity coloured by kind + state.
pub(crate) fn render_minimap(
    host: &mut Host,
    display: &engine_api::HostDisplayState,
    engine: &Engine,
    assets: &LevelAssets,
    renderer: &mut Renderer,
) {
    if host.map_surface == 0 {
        return; // no minimap loaded
    }

    let mm = display.minimap();

    // When map is closed and no transition is active, render the corner
    // button — blit it at the button-box position.
    if !mm.is_displayed() && mm.transition_counter() == 0.0 {
        if mm.button_box().is_somewhere() && !host.minimap_corner_surfaces.is_empty() {
            let state_idx = match mm.ui_state() {
                UIState::Default => 0,
                UIState::Focused => 1,
                UIState::Selected => 2,
            };
            let surface = host
                .minimap_corner_surfaces
                .get(state_idx)
                .or(host.minimap_corner_surfaces.first())
                .copied()
                .unwrap_or(0);
            if surface != 0 {
                let tl = mm.button_box().top_left();
                let br = mm.button_box().bottom_right();
                let src = BBox::from_coords(0.0, 0.0, br.x - tl.x, br.y - tl.y);
                let dst = BBox::from_coords(tl.x, tl.y, br.x, br.y);
                renderer.blit_to_screen(surface, Some(&src), Some(&dst), BLIT_SOURCE_TRANSPARENT);
            }
        }
        return;
    }

    if !mm.is_displayed() {
        return; // transitioning — don't draw full map yet
    }

    if !mm.map_box().is_somewhere() {
        return;
    }

    let map_box = mm.map_box();
    let map_tl = map_box.top_left();
    let map_size = mm.map_size();
    let map_w = map_size.x;
    let map_h = map_size.y;

    // Blit the minimap bitmap to the screen
    let src_box = BBox::from_coords(0.0, 0.0, map_w, map_h);
    let dst_box = BBox::from_coords(map_tl.x, map_tl.y, map_tl.x + map_w, map_tl.y + map_h);

    renderer.blit_to_screen(
        host.map_surface,
        Some(&src_box),
        Some(&dst_box),
        BLIT_SOURCE_TRANSPARENT,
    );

    // Draw viewport indicator rectangle.
    let camera_pos = host.viewport.view_position;
    let screen_size = host.viewport.screen_size;
    let zoom = host.viewport.zoom_factor;
    let level_size = host.viewport.level_size;

    // The visible area in world coordinates (accounting for zoom and
    // panel height).  Divide by zoom first, then subtract
    // PANNEL_HEIGHT.  This diverges from the camera-position clamp
    // formula (which subtracts before dividing); the original may
    // itself be a bug, but the parity contract wins.
    let view_br = engine_coordinates::MapPoint::new(
        camera_pos.x + screen_size.x / zoom,
        camera_pos.y + screen_size.y / zoom - 80.0, // PANNEL_HEIGHT = 80
    );

    // Convert camera corners to minimap pixel coordinates
    if let (Some(tl), Some(br)) = (
        mm.real_to_map(camera_pos, level_size),
        mm.real_to_map(view_br, level_size),
    ) {
        let x1 = tl.x.floor() as i32;
        let y1 = tl.y.floor() as i32;
        let x2 = br.x.floor() as i32;
        let y2 = br.y.floor() as i32;

        // Black rectangle outline (color 0x0000).
        renderer.draw_rect_outline_screen(x1, y1, x2, y2, 0x0000);
    }

    // ── Element dots ──
    // Sort for minimap, draw each active non-highlighted element's
    // dot, then draw delayed highlights.
    if host.minimap_dot_surfaces.is_empty() {
        return;
    }

    let widget_box = if mm.map_box().is_somewhere() {
        *mm.map_box()
    } else {
        return;
    };

    let sorted = engine.sort_for_minimap();
    for id in sorted {
        if mm.is_element_highlighted(id.index()) {
            continue;
        }
        let mut info = match engine.minimap_dot_info(id, assets) {
            Some(i) => i,
            None => continue,
        };
        if !host.diplomacy_visuals {
            info.camp = info.legacy_camp;
        }
        if !info.is_active {
            continue;
        }
        let dot_type = match engine_minimap::classify_element_dot(&info) {
            Some(d) => d,
            None => continue,
        };
        let entity = match engine.get_entity(id) {
            Some(e) => e,
            None => continue,
        };
        refresh_dot(
            host,
            mm,
            level_size,
            entity.element_data().position_map(),
            dot_type,
            &widget_box,
            renderer,
        );
    }

    // Delayed-reveal highlighted elements (scroll reveal etc.).
    for h in mm.highlighted_elements() {
        if !h.refresh {
            continue;
        }
        let Some(entity_id) = engine.entity_id_for_index(h.element_index) else {
            continue;
        };
        let entity = match engine.get_entity(entity_id) {
            Some(e) => e,
            None => continue,
        };
        refresh_dot(
            host,
            mm,
            level_size,
            entity.element_data().position_map(),
            engine_minimap::DotType::Highlighted,
            &widget_box,
            renderer,
        );
    }
}

/// Blit a single minimap dot sprite centred on a converted world
/// position.
fn refresh_dot(
    host: &Host,
    mm: &engine_minimap::MinimapState,
    level_size: engine_coordinates::MapSize,
    world_pos: engine_coordinates::MapPoint,
    dot_type: engine_minimap::DotType,
    widget_box: &engine_coordinates::ScreenBBox,
    renderer: &mut Renderer,
) {
    let idx = dot_type as usize;
    let (surface, dot_w, dot_h) = match host.minimap_dot_surfaces.get(idx) {
        Some(&(s, w, h)) if s != 0 => (s, w, h),
        _ => return,
    };

    let map_pos = match mm.real_to_map(world_pos, level_size) {
        Some(p) => p,
        None => return,
    };

    // Centre the sprite on the converted position.
    let top_left = engine_coordinates::ScreenPoint::new(
        map_pos.x - (dot_w as f32) * 0.5,
        map_pos.y - (dot_h as f32) * 0.5,
    );

    // The top-left (already shifted by half-size) must lie inside the
    // full widget box.  Dots that spill out get clipped below; dots
    // whose anchor is entirely outside are skipped.
    if !widget_box.contains_point(top_left) {
        return;
    }

    // Clip the destination rect to the widget box before the final
    // blit.
    let mut dst_x_min = top_left.x;
    let mut dst_y_min = top_left.y;
    let mut dst_x_max = top_left.x + dot_w as f32;
    let mut dst_y_max = top_left.y + dot_h as f32;

    let mut src_x_min = 0.0f32;
    let mut src_y_min = 0.0f32;

    if dst_x_min < widget_box.top_left().x {
        src_x_min += widget_box.top_left().x - dst_x_min;
        dst_x_min = widget_box.top_left().x;
    }
    if dst_y_min < widget_box.top_left().y {
        src_y_min += widget_box.top_left().y - dst_y_min;
        dst_y_min = widget_box.top_left().y;
    }
    if dst_x_max > widget_box.bottom_right().x {
        dst_x_max = widget_box.bottom_right().x;
    }
    if dst_y_max > widget_box.bottom_right().y {
        dst_y_max = widget_box.bottom_right().y;
    }

    if dst_x_max <= dst_x_min || dst_y_max <= dst_y_min {
        return;
    }

    let src_box = BBox::from_coords(
        src_x_min,
        src_y_min,
        src_x_min + (dst_x_max - dst_x_min),
        src_y_min + (dst_y_max - dst_y_min),
    );
    let dst_box = BBox::from_coords(dst_x_min, dst_y_min, dst_x_max, dst_y_max);

    renderer.blit_to_screen(
        surface,
        Some(&src_box),
        Some(&dst_box),
        BLIT_SOURCE_TRANSPARENT,
    );
}
