//! In-game rendering passes for the mission loop.
//!
//! Contains the GPU-phase rendering functions: entity sprites, selection
//! outlines, ground marks, and minimap.  The in-game
//! menu rendering lives in [`crate::ingame_menu`] and is driven by
//! [`crate::game_session`].

use crate::gfx_types::Rect;
use crate::host::HostDraw;
use crate::hud_text::{self, HudFonts};
use crate::ingame_menu::layout;
use crate::renderer::{BLIT_SOURCE_TRANSPARENT, OUTLINE_PAD, Renderer, rgb565_to_rgb8};
use crate::titbit_renderer::TitbitRenderer;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::coordinates::{GroundPoint, MapPoint, SpriteAnchor, SpriteFrameOffset};
use robin_engine::element as engine_element;
use robin_engine::element::{ElementKind, Entity, OutlineColorName, Posture, RenderingProperties};
use robin_engine::engine as engine_api;
use robin_engine::engine::{DevState, LevelAssets, PresentationView};
use robin_engine::markers::GroundMark;
use robin_engine::mask as engine_mask;
use robin_engine::position_interface as engine_position_interface;
use robin_engine::sector as engine_sector;
use robin_engine::sight_obstacle as engine_sight_obstacle;
use robin_engine::sprite::BBox;

mod doors;
mod entities;
mod fog;
mod view_cones;
pub(crate) use doors::render_door_overlays;
#[cfg(test)]
use entities::uses_pixel_fog_visibility;
pub(crate) use entities::{
    render_bg_animations_gpu, render_entities_gpu, render_selection_outlines_gpu,
};
use entities::{render_character_masks_clipped, render_text_with_shadow};
pub(crate) use fog::{
    build_vector_fog_mask_rgba, fog_mask_cache_key, render_fog_of_war, vector_fog_mask_dimensions,
};
pub(crate) use view_cones::render_view_cone_overlay;
mod debug;
mod hud;
mod minimap;

pub(crate) use debug::{
    render_debug_animation_lines, render_debug_doors, render_debug_motion_graph,
    render_debug_surfaces_fill, render_debug_surfaces_outline, render_debug_whatsup_overlay,
    render_noise_display, render_shadow_polygon_sphere_debug,
};
pub(crate) use hud::{
    draw_multi_selection_box, prepare_multi_selection_box, render_combat_status_bars,
    render_item_effect_preview, render_listen_ping, render_mission_countdown,
    render_ransom_amulet_overlay, render_trajectory_preview,
};
pub(crate) use minimap::render_minimap;

/// Immutable preferences and ordering shared by the world passes in one frame.
/// Each world pass in a captured or live frame sees one preference snapshot,
/// even if the application profile changes during submission.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct FramePresentationInputs {
    graphic_config: robin_engine::graphic_config::GraphicConfig,
    ambiance: engine_api::Ambiance,
    shadow_color: u16,
    view: MapPoint,
    zoom: f32,
    screen_size: engine_coordinates::ScreenSize,
    draw_order_ids: Vec<engine_element::EntityId>,
}

impl FramePresentationInputs {
    pub(crate) fn prepare(host: &HostDraw<'_>, engine: &PresentationView<'_>) -> Self {
        let graphic_config = host.graphic_config();
        let dynamic = graphic_config.dynamic_ambience_visuals;
        Self {
            ambiance: if dynamic {
                engine.weather().ambiance
            } else {
                engine.initial_mission_ambiance()
            },
            shadow_color: if dynamic {
                engine.weather().night_color
            } else {
                engine.initial_mission_night_color()
            },
            graphic_config,
            view: host.viewport().view_position,
            zoom: host.viewport().zoom_factor,
            screen_size: host.viewport().screen_size,
            draw_order_ids: host.frontend.presentation.draw_order.ids.clone(),
        }
    }
}

/// Original RHSprite::GenerateBlitBox scales both destination edges. Keep
/// sprite, ghost, FX and outline dimensions in the same zoom space as anchors
/// and occlusion masks; texture dimensions remain native world units.
pub(crate) fn zoomed_sprite_rect(x: i32, y: i32, width: u16, height: u16, zoom: f32) -> Rect {
    assert!(zoom.is_finite() && zoom > 0.0, "invalid sprite zoom {zoom}");
    Rect::new(
        x,
        y,
        (width as f32 * zoom).round().max(1.0) as u32,
        (height as f32 * zoom).round().max(1.0) as u32,
    )
}

// ─── Ground marks ──────────────────────────────────────────────────

/// Render every active destination marker.
///
/// For each active mark, check on-screen and blit. Engine-owned command
/// marks advance inside `perform_hourglass`; host-owned trajectory-
/// preview marks advance on the same hourglass cadence without entering
/// sim state.
pub(crate) fn render_ground_marks(
    host: &HostDraw<'_>,
    engine: &PresentationView<'_>,
    renderer: &mut Renderer,
) {
    if host
        .frontend
        .resources
        .mission_surfaces
        .ground_marks()
        .is_empty()
    {
        return;
    }

    render_ground_mark_set(host, engine.ground_mark(), engine, renderer);
    render_ground_mark_set(
        host,
        host.frontend.trajectory_preview().ground_marks(),
        engine,
        renderer,
    );
}

fn render_ground_mark_set(
    host: &HostDraw<'_>,
    ground_mark: &GroundMark,
    engine: &PresentationView<'_>,
    renderer: &mut Renderer,
) {
    if ground_mark.is_empty() {
        return;
    }
    let zoom = host.viewport().zoom_factor;
    let screen_w = host.viewport().screen_size.x as i32;
    let screen_h = host.viewport().screen_size.y as i32;

    // The same shadow rendering used for entity shadows.
    let shadow_level = host.frontend.resources.frame_holder().global_shadow();

    let view_pos = host.viewport().view_position;

    let per_frame_offsets = ground_mark.per_frame_offsets();

    for mark in &ground_mark.marks {
        // Read `render_frame` (snapshot taken at the start of `tick`
        // before advancing) so we draw the pre-retire frame on the tick
        // where the animation ends.
        let frame_idx = mark.render_frame as usize;
        let (surf_id, fw, fh) = match host
            .frontend
            .resources
            .mission_surfaces
            .ground_marks()
            .get(frame_idx)
        {
            Some(Some(frame)) => frame.parts(),
            _ => continue,
        };

        // World→screen. `mark.x`/`mark.y` is already the sprite top-left
        // (half-diagonal was subtracted at `add_mark` time), so the
        // on-screen destination is the direct affine transform — no
        // additional half-width offset.
        let screen_x = (mark.x - view_pos.x) * zoom;
        let screen_y = (mark.y - view_pos.y) * zoom;

        let scaled_w = (fw as f32 * zoom).round() as i32;
        let scaled_h = (fh as f32 * zoom).round() as i32;
        if scaled_w <= 0 || scaled_h <= 0 {
            continue;
        }

        let dst_x = screen_x.round() as i32;
        let dst_y = screen_y.round() as i32;

        // Per-frame offset — added to the sprite top-left before
        // computing the cull AABB.  We still blit the uncropped surface
        // (transparent border absorbs the offset visually), but the
        // cull tracks the offset when it's non-zero.
        let (ox, oy) = per_frame_offsets.get(frame_idx).copied().unwrap_or((0, 0));
        let cull_x = dst_x + (ox as f32 * zoom).round() as i32;
        let cull_y = dst_y + (oy as f32 * zoom).round() as i32;

        let on_screen = cull_x + scaled_w > 0
            && cull_y + scaled_h > 0
            && cull_x < screen_w
            && cull_y < screen_h;
        if !on_screen {
            continue;
        }

        let src_box = BBox::from_coords(0.0, 0.0, fw as f32, fh as f32);
        let dst_box = BBox::from_coords(
            dst_x as f32,
            dst_y as f32,
            (dst_x + scaled_w) as f32,
            (dst_y + scaled_h) as f32,
        );

        let draw_checkpoint = renderer.draw_queue_checkpoint();
        renderer
            .draw_surface_with_shadow(
                surf_id,
                Some(&src_box),
                Some(&dst_box),
                shadow_level,
                BLIT_SOURCE_TRANSPARENT,
            )
            .expect("mission ground mark must belong to the live renderer");

        let mark_world_bbox = engine_coordinates::MapBBox::from_coords(
            mark.x + ox as f32,
            mark.y + oy as f32,
            mark.x + ox as f32 + fw as f32,
            mark.y + oy as f32 + fh as f32,
        );
        let mark_position = MapPoint::new(
            mark.x + ox as f32 + fw as f32 * 0.5,
            mark.y + oy as f32 + fh as f32 * 0.5,
        );
        let mark_rect = Rect::new(dst_x, dst_y, scaled_w as u32, scaled_h as u32);
        render_character_masks_clipped(
            engine,
            renderer,
            mark.layer,
            &mark_world_bbox,
            mark_position,
            mark_rect,
            draw_checkpoint,
            view_pos,
            zoom,
        );
    }
}

#[cfg(test)]
mod fog_render_tests {
    use super::*;
    use robin_engine::coordinates::MapSize;
    use robin_engine::element::{ElementData, ElementFx, FxData};

    #[test]
    fn sprite_destinations_scale_with_the_map() {
        for (zoom, width, height) in [(0.5, 20, 40), (1.0, 40, 80), (2.0, 80, 160)] {
            let rect = zoomed_sprite_rect(120, 90, 40, 80, zoom);
            assert_eq!((rect.x, rect.y, rect.w, rect.h), (120, 90, width, height));
        }
    }

    #[test]
    fn cache_key_tracks_polygon_generation() {
        let mut fog = robin_engine::fog_of_war::FogOfWarState::default();
        fog.initialize(MapSize::new(96.0, 96.0));
        let before = fog_mask_cache_key(&fog);
        fog.initialize(MapSize::new(96.0, 96.0));
        assert_ne!(before, fog_mask_cache_key(&fog));
    }

    #[test]
    fn patch_fx_defers_visibility_to_the_pixel_fog_composite() {
        let ordinary = Entity::Fx(ElementFx {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::Fx;
                initial_element
            },
            fx: FxData::default(),
        });
        assert!(!uses_pixel_fog_visibility(&ordinary));

        let patch = Entity::Fx(ElementFx {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::Fx;
                initial_element
            },
            fx: FxData {
                patch_index: robin_engine::patch::PatchIndex::new(0),
                ..FxData::default()
            },
        });
        assert!(uses_pixel_fog_visibility(&patch));
    }
}

/// Resolve the active profile and validate its frame without choosing a pass's fallback.
fn current_render_frame(
    sprite: &robin_engine::sprite::Sprite,
) -> Option<(&robin_engine::sprite_script::SpriteScript, u16, u32)> {
    let script = sprite
        .current_scripts_opt()?
        .get(usize::from(sprite.current_row))?;
    let frame = sprite.current_frame;
    let bank_id = *script.frame_ids.get(usize::from(frame))?;
    Some((script, frame, bank_id))
}

#[test]
fn render_frame_lookup_preserves_active_rows_and_rejects_missing_frames() {
    use robin_engine::sprite::Sprite;
    use robin_engine::sprite_script::SpriteScript;
    use std::sync::Arc;
    let mut sprite = Sprite::new(
        Arc::new(vec![SpriteScript {
            frame_ids: vec![17, 23],
            ..Default::default()
        }]),
        Arc::new(Vec::new()),
    );
    assert_eq!(
        current_render_frame(&sprite).map(|(_, frame, bank)| (frame, bank)),
        Some((0, 17))
    );
    sprite.current_frame = 1;
    assert_eq!(
        current_render_frame(&sprite).map(|(_, frame, bank)| (frame, bank)),
        Some((1, 23))
    );
    sprite.current_frame = 2;
    assert!(current_render_frame(&sprite).is_none());
    sprite.current_frame = 0;
    sprite.current_row = 1;
    assert!(current_render_frame(&sprite).is_none());
    sprite.current_row = 0;
    sprite.use_alternate_profile = true;
    assert!(current_render_frame(&sprite).is_none());
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct SpritePlacement {
    world_origin: MapPoint,
    screen_origin: (i32, i32),
}

/// Floor the sprite anchor in world space before zoom, then truncate screen pixels.
fn sprite_placement(
    position: MapPoint,
    center: SpriteAnchor,
    offset: SpriteFrameOffset,
    view: MapPoint,
    zoom: f32,
) -> SpritePlacement {
    let x = (position.x - center.x).floor() + offset.x;
    let y = (position.y - center.y).floor() + offset.y;
    SpritePlacement {
        world_origin: MapPoint::new(x, y),
        screen_origin: (((x - view.x) * zoom) as i32, ((y - view.y) * zoom) as i32),
    }
}

#[test]
fn sprite_origin_preserves_floor_before_zoom_and_signed_truncation() {
    for (zoom, expected) in [
        (0.5, (4, 0)),
        (1.0, (9, -1)),
        (1.5, (14, -2)),
        (2.0, (19, -3)),
    ] {
        assert_eq!(
            sprite_placement(
                MapPoint::new(10.75, -2.25),
                SpriteAnchor::new(0.5, 0.5),
                SpriteFrameOffset::new(0.25, 0.75),
                MapPoint::new(0.5, -0.5),
                zoom,
            )
            .screen_origin,
            expected,
        );
    }
    assert_eq!(
        sprite_placement(
            MapPoint::ZERO,
            SpriteAnchor::new(0.25, 0.25),
            SpriteFrameOffset::ZERO,
            MapPoint::ZERO,
            0.5,
        )
        .screen_origin,
        (0, 0),
    );
}

/// Keep anchors on the margin boundary visible, matching the legacy sprite passes.
fn outside_sprite_cull_margin((x, y): (i32, i32), (width, height): (i32, i32), zoom: f32) -> bool {
    let margin = (256.0 * zoom).ceil() as i32;
    x < -margin || y < -margin || x > width + margin || y > height + margin
}

#[test]
fn sprite_culling_preserves_inclusive_zoom_scaled_edges() {
    for (zoom, margin) in [(0.25, 64), (0.501, 129), (1.0, 256), (1.5, 384)] {
        for point in [
            (-margin, 0),
            (0, -margin),
            (100 + margin, 0),
            (0, 80 + margin),
            (0, 0),
            (100, 80),
        ] {
            assert!(!outside_sprite_cull_margin(point, (100, 80), zoom));
        }
        for point in [
            (-margin - 1, 0),
            (0, -margin - 1),
            (101 + margin, 0),
            (0, 81 + margin),
        ] {
            assert!(outside_sprite_cull_margin(point, (100, 80), zoom));
        }
    }
}

#[test]
fn fog_rasterization_preserves_holes_clipping_and_ring_orientation() {
    use robin_engine::coordinates::MapSize;
    use robin_engine::fog_of_war::{FogPolygon, FogRegion};

    let rectangle = |left, top, right, bottom| {
        vec![
            MapPoint::new(left, top),
            MapPoint::new(right, top),
            MapPoint::new(right, bottom),
            MapPoint::new(left, bottom),
        ]
    };
    let mut polygons = vec![
        FogPolygon {
            exterior: rectangle(0.0, 0.0, 4.0, 4.0),
            interiors: vec![rectangle(1.0, 1.0, 3.0, 3.0)],
        },
        FogPolygon {
            exterior: rectangle(6.0, 0.0, 9.0, 2.0),
            interiors: Vec::new(),
        },
        FogPolygon {
            exterior: rectangle(-2.0, 5.0, 2.0, 7.0),
            interiors: Vec::new(),
        },
        FogPolygon {
            exterior: Vec::new(),
            interiors: vec![Vec::new()],
        },
    ];
    let expected = [
        "####..##", "#..#..##", "#..#....", "####....", "........", "##......",
    ];
    for close_and_reverse in [false, true] {
        if close_and_reverse {
            for polygon in &mut polygons {
                for ring in std::iter::once(&mut polygon.exterior).chain(&mut polygon.interiors) {
                    ring.reverse();
                    if let Some(first) = ring.first().copied() {
                        ring.push(first);
                    }
                }
            }
        }
        let region: FogRegion =
            serde_json::from_value(serde_json::json!({"polygons": polygons})).unwrap();
        let mut alpha = vec![255; 8 * 6];
        for value in [165, 0] {
            rasterize_fog_region(&mut alpha, 8, 6, MapSize::new(8.0, 6.0), &region, value);
            for (actual, row) in alpha.chunks_exact(8).zip(expected) {
                let expected: Vec<u8> = row
                    .bytes()
                    .map(|byte| if byte == b'#' { value } else { 255 })
                    .collect();
                assert_eq!(actual, expected);
            }
        }
    }
}
