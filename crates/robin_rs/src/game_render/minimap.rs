//! Mission minimap rendering.

use crate::host::HostPresentation;
use crate::renderer::{BLIT_SOURCE_TRANSPARENT, Renderer, SurfaceHandle};
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
    host: &mut HostPresentation<'_>,
    display: &engine_api::HostDisplayState,
    engine: &Engine,
    assets: &LevelAssets,
    renderer: &mut Renderer,
) {
    let Some(map_surface) = host.frontend.mission_surfaces.map() else {
        return; // no minimap loaded
    };

    let mm = display.minimap();

    // When map is closed and no transition is active, render the corner
    // button — blit it at the button-box position.
    if !mm.is_displayed() && mm.transition_counter() == 0.0 {
        if mm.button_box().is_somewhere() {
            let state_idx = match mm.ui_state() {
                UIState::Default => 0,
                UIState::Focused => 1,
                UIState::Selected => 2,
            };
            if let Some(surface) = host.frontend.mission_surfaces.corner(state_idx) {
                let tl = mm.button_box().top_left();
                let br = mm.button_box().bottom_right();
                let src = BBox::from_coords(0.0, 0.0, br.x - tl.x, br.y - tl.y);
                let dst = BBox::from_coords(tl.x, tl.y, br.x, br.y);
                renderer
                    .draw_surface(surface, Some(&src), Some(&dst), BLIT_SOURCE_TRANSPARENT)
                    .expect("mission corner must belong to the live renderer");
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

    renderer
        .draw_surface(
            map_surface,
            Some(&src_box),
            Some(&dst_box),
            BLIT_SOURCE_TRANSPARENT,
        )
        .expect("mission minimap must belong to the live renderer");

    render_minimap_fog(engine, mm, level_size_for(host), renderer);

    // Draw viewport indicator rectangle.
    let camera_pos = host.frontend.viewport.view_position;
    let screen_size = host.frontend.viewport.screen_size;
    let zoom = host.frontend.viewport.zoom_factor;
    let level_size = host.frontend.viewport.level_size;

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
    if host.frontend.mission_surfaces.dots().is_empty() {
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
        if !host.frontend.diplomacy_visuals {
            info.camp = info.legacy_camp;
        }
        if !info.is_active {
            continue;
        }
        if !engine.fog_entity_visible(id) {
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

    // Hidden hostiles leave a stationary, fading last-known marker. The
    // marker is deliberately not updated while the entity is outside sight.
    if engine.fog_of_war_enabled() {
        for marker in engine
            .fog_of_war()
            .last_known_markers(engine.frame_counter())
        {
            let Some(entity) = engine.get_entity(marker.entity_id) else {
                continue;
            };
            if !entity.is_active() || !engine.fog_entity_is_hostile(marker.entity_id) {
                continue;
            }
            refresh_dot_alpha(
                host,
                mm,
                level_size,
                marker.position,
                engine_minimap::DotType::Enemy,
                &widget_box,
                marker.alpha,
                renderer,
            );
        }
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
    host: &HostPresentation<'_>,
    mm: &engine_minimap::MinimapState,
    level_size: engine_coordinates::MapSize,
    world_pos: engine_coordinates::MapPoint,
    dot_type: engine_minimap::DotType,
    widget_box: &engine_coordinates::ScreenBBox,
    renderer: &mut Renderer,
) {
    let Some((surface, src_box, dst_box)) =
        clipped_dot_blit(host, mm, level_size, world_pos, dot_type, widget_box)
    else {
        return;
    };
    // Preserve the Original-compatible draw path exactly when no fade is
    // requested; disabled fog must not reroute ordinary minimap dots through
    // an alpha-specific renderer path.
    renderer
        .draw_surface(
            surface,
            Some(&src_box),
            Some(&dst_box),
            BLIT_SOURCE_TRANSPARENT,
        )
        .expect("mission dot must belong to the live renderer");
}

#[allow(clippy::too_many_arguments)]
fn refresh_dot_alpha(
    host: &HostPresentation<'_>,
    mm: &engine_minimap::MinimapState,
    level_size: engine_coordinates::MapSize,
    world_pos: engine_coordinates::MapPoint,
    dot_type: engine_minimap::DotType,
    widget_box: &engine_coordinates::ScreenBBox,
    opacity: u8,
    renderer: &mut Renderer,
) {
    let Some((surface, src_box, dst_box)) =
        clipped_dot_blit(host, mm, level_size, world_pos, dot_type, widget_box)
    else {
        return;
    };
    let transparency = 100u16.saturating_sub(u16::from(opacity) * 100 / 255);
    renderer
        .draw_surface_alpha(
            surface,
            Some(&src_box),
            Some(&dst_box),
            transparency,
            BLIT_SOURCE_TRANSPARENT,
        )
        .expect("mission fading dot must belong to the live renderer");
}

fn clipped_dot_blit(
    host: &HostPresentation<'_>,
    mm: &engine_minimap::MinimapState,
    level_size: engine_coordinates::MapSize,
    world_pos: engine_coordinates::MapPoint,
    dot_type: engine_minimap::DotType,
    widget_box: &engine_coordinates::ScreenBBox,
) -> Option<(SurfaceHandle, BBox, BBox)> {
    let idx = dot_type as usize;
    let (surface, dot_w, dot_h) = match host.frontend.mission_surfaces.dots().get(idx) {
        Some(Some(frame)) => frame.parts(),
        _ => return None,
    };

    let map_pos = match mm.real_to_map(world_pos, level_size) {
        Some(p) => p,
        None => return None,
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
        return None;
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
        return None;
    }

    let src_box = BBox::from_coords(
        src_x_min,
        src_y_min,
        src_x_min + (dst_x_max - dst_x_min),
        src_y_min + (dst_y_max - dst_y_min),
    );
    let dst_box = BBox::from_coords(dst_x_min, dst_y_min, dst_x_max, dst_y_max);
    Some((surface, src_box, dst_box))
}

fn level_size_for(host: &HostPresentation<'_>) -> engine_coordinates::MapSize {
    host.frontend.viewport.level_size
}

fn render_minimap_fog(
    engine: &Engine,
    mm: &engine_minimap::MinimapState,
    level_size: engine_coordinates::MapSize,
    renderer: &mut Renderer,
) {
    if !engine.fog_of_war_enabled() {
        return;
    }
    let fog = engine.fog_of_war();
    let map_box = mm.map_box();
    let top_left = map_box.top_left();
    let map_size = mm.map_size();
    let (mask_width, mask_height) = super::vector_fog_mask_dimensions(fog.level_size());
    let cache_key = super::fog_mask_cache_key(fog);
    renderer.render_fog_mask(
        mask_width,
        mask_height,
        cache_key,
        top_left.x.round() as i32,
        top_left.y.round() as i32,
        map_size.x.round() as i32,
        map_size.y.round() as i32,
        [
            0.0,
            0.0,
            level_size.x / fog.level_size().x,
            level_size.y / fog.level_size().y,
        ],
        || super::build_vector_fog_mask_rgba(fog),
    );
}
