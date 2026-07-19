//! Developer rendering overlays.

use super::render_text_with_shadow;
use crate::host::Host;
use crate::hud_text::HudFonts;
use crate::renderer::{Renderer, rgb565_to_rgb8};
use robin_engine::coordinates as engine_coordinates;
use robin_engine::coordinates::MapPoint;
use robin_engine::element as engine_element;
use robin_engine::element::{Entity, GameMaterial};
use robin_engine::engine as engine_api;
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::pathfinder as engine_pathfinder;

/// Render the developer shadow-polygon sphere debug overlay when the
/// `shadow_polygon_sphere` cheat is active.
///
/// Reduced to the ground-ring slice — the 100-slice vertical stack is
/// omitted because it visualises viewer Z which is always 0 in the
/// current camera model.  A white ground ellipse of radius
/// `params.radius` is sufficient to match the gameplay-relevant debug
/// cue.
pub(crate) fn render_shadow_polygon_sphere_debug(
    host: &Host,
    engine: &Engine,
    selected_view_element: Option<engine_element::EntityId>,
    dev: &engine_api::DevState,
    renderer: &mut Renderer,
) {
    if !dev.debug.shadow_polygon_sphere {
        return;
    }
    let Some((viewer, params, _tint)) = engine.selected_view_cone_params(selected_view_element)
    else {
        return;
    };
    host.draw_manager.draw_ellipse(
        renderer,
        engine_coordinates::MapPoint::new(viewer.x, viewer.y),
        params.radius as u16,
        0xFFFF,
    );
}

// ─── Debug overlays ──────────────────────────────────────────────────

/// Render debug door gizmos: gate endpoint markers + connecting lines.
///
/// Dispatched when the `door_display` debug flag is set.  For every
/// gate-that-is-a-door, the pass draws:
///
/// * Green (`0x00FF`) line(s) from `point_out` → (`point_mid` →)
///   `point_in`.
/// * Cyan-ish (`0x0CC0`) 4-px box at `point_out`.
/// * White (`0xFFFF`) 4-px box at `point_mid` (non-default door types
///   only).
/// * Red (`0xFA00`) 4-px box at `point_in`.
///
/// Drawn through GPU helpers, so zoom scaling is applied per-axis
/// (same architectural shift the `draw_status_bar` docstring calls
/// out).
pub(crate) fn render_debug_doors(
    host: &Host,
    engine: &Engine,
    dev: &engine_api::DevState,
    renderer: &mut Renderer,
) {
    use robin_engine::gate::DoorType;

    if !dev.debug.door_display {
        return;
    }
    if engine.mission_script().is_none() {
        return;
    }

    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;

    // 4-px endpoint box (`point ± (2,2)`) in world units; scale by
    // zoom so the gizmo stays the same pixel size regardless of zoom.
    let half = (2.0 * zoom).round().max(1.0) as i32;
    let side = (half * 2).max(1);

    let (line_r, line_g, line_b) = rgb565_to_rgb8(0x00FF);
    let (out_r, out_g, out_b) = rgb565_to_rgb8(0x0CC0);
    let (mid_r, mid_g, mid_b) = rgb565_to_rgb8(0xFFFF);
    let (in_r, in_g, in_b) = rgb565_to_rgb8(0xFA00);

    let world_to_screen = |p: engine_coordinates::MapPoint| -> (i32, i32) {
        let sx = ((p.x - view.x) * zoom).round() as i32;
        let sy = ((p.y - view.y) * zoom).round() as i32;
        (sx, sy)
    };
    let box_at = |renderer: &mut Renderer, (x, y): (i32, i32), (r, g, b): (u8, u8, u8)| {
        renderer.render_gpu_rect(x - half, y - half, side, side, r, g, b, 255);
    };

    for door in engine.doors() {
        if !door.is_door() {
            continue;
        }

        let out_screen = world_to_screen(door.point_out);
        let in_screen = world_to_screen(door.point_in);

        if matches!(door.door_type, DoorType::Default) {
            // 2-point branch.
            renderer.render_gpu_line(
                out_screen.0,
                out_screen.1,
                in_screen.0,
                in_screen.1,
                line_r,
                line_g,
                line_b,
            );
            box_at(renderer, out_screen, (out_r, out_g, out_b));
            box_at(renderer, in_screen, (in_r, in_g, in_b));
        } else {
            // 3-point branch.
            let mid_screen = world_to_screen(door.point_mid);
            renderer.render_gpu_line(
                out_screen.0,
                out_screen.1,
                mid_screen.0,
                mid_screen.1,
                line_r,
                line_g,
                line_b,
            );
            renderer.render_gpu_line(
                mid_screen.0,
                mid_screen.1,
                in_screen.0,
                in_screen.1,
                line_r,
                line_g,
                line_b,
            );
            box_at(renderer, out_screen, (out_r, out_g, out_b));
            box_at(renderer, mid_screen, (mid_r, mid_g, mid_b));
            box_at(renderer, in_screen, (in_r, in_g, in_b));
        }
    }
}

/// Render the pathfinder motion-graph debug overlay: graph edges plus
/// per-node corner stubs.
///
/// Dispatched when the motion-graph-display cheat is set, drawing both
/// edges and nodes.  The half-diagonal index is the first PC's
/// pathfinder index, so the rendered overlay reflects what A* would
/// actually consider for that unit's body size.
///
/// Both passes use GPU line draws (the same architectural shift that
/// `render_debug_doors` documents); per-segment clipping is delegated
/// to the GPU framebuffer rather than explicit clipping against the
/// view rect.  World→screen transform is
/// `(point - view_rect.top_left) * zoom`.
pub(crate) fn render_debug_motion_graph(
    host: &Host,
    engine: &Engine,
    assets: &engine_api::LevelAssets,
    dev: &engine_api::DevState,
    renderer: &mut Renderer,
) {
    if !dev.debug.motion_graph_display {
        return;
    }

    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;
    let screen_size = host.viewport.screen_size;
    if zoom <= 0.0 || screen_size.x <= 0.0 || screen_size.y <= 0.0 {
        return;
    }

    // The bounding box is the camera viewport in world coords: origin
    // at `view_position`, dimensions `screen_size / zoom_factor`.
    let view_rect = engine_coordinates::MapBBox::from_point_size(
        view,
        screen_size.x / zoom,
        screen_size.y / zoom,
    );

    // PC[0] is the first portrait-order player character.
    let half_diagonal_idx = engine
        .pc_ids()
        .first()
        .and_then(|id| engine.get_entity(*id))
        .map(|e| e.sprite().position_iface.get_pathfinder_index())
        .unwrap_or(0);

    let world_to_screen = |p: engine_coordinates::MapPoint| -> (i32, i32) {
        let sx = ((p.x - view.x) * zoom).round() as i32;
        let sy = ((p.y - view.y) * zoom).round() as i32;
        (sx, sy)
    };

    let pathfinder = engine.pathfinder();

    pathfinder.draw_graph(
        assets.pathfinder_graph.as_ref(),
        view_rect,
        half_diagonal_idx,
        |a, b, color| {
            let (r, g, blu) = rgb565_to_rgb8(color);
            let (x1, y1) = world_to_screen(a);
            let (x2, y2) = world_to_screen(b);
            renderer.render_gpu_line(x1, y1, x2, y2, r, g, blu);
        },
    );

    pathfinder.draw_nodes(
        assets.pathfinder_graph.as_ref(),
        view_rect,
        half_diagonal_idx,
        |a, b, color| {
            let (r, g, blu) = rgb565_to_rgb8(color);
            let (x1, y1) = world_to_screen(a);
            let (x2, y2) = world_to_screen(b);
            renderer.render_gpu_line(x1, y1, x2, y2, r, g, blu);
        },
    );
}

// ─── Debug surfaces overlay ──────────────────────────────────────────

/// Hash a `(layer, area)` pair to a stable RGB color.  Uses a Wang-style
/// integer hash to spread adjacent indices across the hue circle so
/// neighbouring areas get visually distinct colors.
fn surface_color(layer: usize, area: usize) -> (u8, u8, u8) {
    let mut h = (layer as u32).wrapping_mul(0x9E3779B1) ^ (area as u32).wrapping_mul(0x85EBCA77);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB352D);
    h ^= h >> 15;
    let hue = (h & 0xFF) as f32 / 255.0;
    // HSV → RGB with fixed S=0.7, V=0.9.
    let (s, v) = (0.7_f32, 0.9_f32);
    let i = (hue * 6.0).floor() as i32;
    let f = hue * 6.0 - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i.rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// Locate the `(layer, area_idx)` of a world-space point across every
/// layer.  Both indices are vec positions in
/// `move_layers[layer][area]` — matches `PathGraph::find_area_at_point`,
/// which is the canonical lookup.
fn locate_surface(graph: &engine_pathfinder::PathGraph, pt: MapPoint) -> Option<(usize, usize)> {
    (0..graph.static_data.move_layers.len())
        .find_map(|l| graph.find_area_at_point(l, pt).map(|a| (l, a)))
}

/// Draw a closed polyline outline on the GPU layer.
fn draw_polygon_outline_map(
    renderer: &mut Renderer,
    verts: &[MapPoint],
    map_to_screen: &dyn Fn(MapPoint) -> (i32, i32),
    r: u8,
    g: u8,
    b: u8,
) {
    if verts.len() < 2 {
        return;
    }
    for i in 0..verts.len() {
        let a = verts[i];
        let bp = verts[(i + 1) % verts.len()];
        let (x1, y1) = map_to_screen(a);
        let (x2, y2) = map_to_screen(bp);
        renderer.render_gpu_line(x1, y1, x2, y2, r, g, b);
    }
}

/// Fill a (possibly concave) polygon via ear-clipping triangulation.
///
/// Falls back silently on degenerate polygons (earcutr returns an empty
/// index list).  Fan triangulation isn't sufficient here — `MotionArea`
/// boundaries are routinely concave (e.g. ground areas wrapping around
/// buildings), and a fan from vertex 0 produces giant bowtie triangles
/// that fan across empty space.
fn fill_polygon_map(
    renderer: &mut Renderer,
    verts: &[MapPoint],
    map_to_screen: &dyn Fn(MapPoint) -> (f32, f32),
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) {
    if verts.len() < 3 {
        return;
    }
    let mut flat: Vec<f64> = Vec::with_capacity(verts.len() * 2);
    for v in verts {
        flat.push(v.x as f64);
        flat.push(v.y as f64);
    }
    let indices = match earcutr::earcut(&flat, &[], 2) {
        Ok(ix) => ix,
        Err(_) => return,
    };
    for tri in indices.as_chunks::<3>().0 {
        let p0 = map_to_screen(verts[tri[0]]);
        let p1 = map_to_screen(verts[tri[1]]);
        let p2 = map_to_screen(verts[tri[2]]);
        renderer.render_gpu_triangle([p0, p1, p2], r, g, b, a);
    }
}

/// Find the `(layer, area_idx)` the selected character is standing on,
/// using the canonical `PathGraph::find_area_at_point` lookup.
fn selected_surface(
    host: &Host,
    engine: &Engine,
    graph: &engine_pathfinder::PathGraph,
) -> Option<(usize, usize)> {
    let pc_id = engine
        .seat_selection(host.transport.local_seat)
        .first()
        .copied()?;
    let entity = engine.get_entity(pc_id)?;
    let ed = entity.element_data();
    let layer = ed.layer() as usize;
    let pos = ed.position_map();
    let area = graph.find_area_at_point(layer, pos)?;
    Some((layer, area))
}

/// Fill pass for the surface debug overlay.  Drawn before sprite
/// rendering so the highlight tint sits *under* characters and
/// non-static obstacle sprites.  Only the selected character's
/// MotionArea is filled — outlining every area is left to the post-
/// sprite pass so the sprite art reads cleanly.
pub(crate) fn render_debug_surfaces_fill(
    host: &Host,
    engine: &Engine,
    assets: &LevelAssets,
    dev: &engine_api::DevState,
    renderer: &mut Renderer,
) {
    if !dev.debug.surface_display {
        return;
    }
    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;
    let screen_size = host.viewport.screen_size;
    if zoom <= 0.0 || screen_size.x <= 0.0 || screen_size.y <= 0.0 {
        return;
    }
    let to_screen_f =
        move |p: MapPoint| -> (f32, f32) { ((p.x - view.x) * zoom, (p.y - view.y) * zoom) };
    let graph = assets.pathfinder_graph.as_ref();
    let Some((sel_layer, sel_area)) = selected_surface(host, engine, graph) else {
        return;
    };
    let Some(area) = graph
        .static_data
        .move_layers
        .get(sel_layer)
        .and_then(|areas| areas.get(sel_area))
    else {
        return;
    };
    fill_polygon_map(renderer, &area.polygon, &to_screen_f, 255, 255, 0, 80);
}

/// Outline + path pass for the surface debug overlay.  Drawn after
/// sprite rendering so polygon outlines, obstacle outlines, the
/// highlighted-surface outline, and the committed-path polyline all
/// sit on top of the world and remain readable.
pub(crate) fn render_debug_surfaces_outline(
    host: &Host,
    engine: &Engine,
    assets: &LevelAssets,
    dev: &engine_api::DevState,
    renderer: &mut Renderer,
) {
    if !dev.debug.surface_display {
        return;
    }

    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;
    let screen_size = host.viewport.screen_size;
    if zoom <= 0.0 || screen_size.x <= 0.0 || screen_size.y <= 0.0 {
        return;
    }

    let to_screen_i = move |p: MapPoint| -> (i32, i32) {
        let (sx, sy) = ((p.x - view.x) * zoom, (p.y - view.y) * zoom);
        (sx.round() as i32, sy.round() as i32)
    };

    let graph = assets.pathfinder_graph.as_ref();
    let move_layers = &graph.static_data.move_layers;
    let selected_layer_area = selected_surface(host, engine, graph);
    let selected_id = engine
        .seat_selection(host.transport.local_seat)
        .first()
        .copied();

    // Pass 1: outline every walkable area, plus active obstacles within.
    for (layer_idx, areas) in move_layers.iter().enumerate() {
        for (area_idx, area) in areas.iter().enumerate() {
            let (r, g, b) = surface_color(layer_idx, area_idx);
            draw_polygon_outline_map(renderer, &area.polygon, &to_screen_i, r, g, b);
            for obstacle in &area.motion_obstacles {
                if !obstacle.active {
                    continue;
                }
                draw_polygon_outline_map(renderer, &obstacle.polygon, &to_screen_i, 200, 40, 40);
            }
        }
    }

    // Pass 2: bright outline on the selected character's surface
    // (the fill is drawn earlier, beneath sprites).
    if let Some((sel_layer, sel_area)) = selected_layer_area
        && let Some(area) = move_layers
            .get(sel_layer)
            .and_then(|areas| areas.get(sel_area))
    {
        draw_polygon_outline_map(renderer, &area.polygon, &to_screen_i, 255, 255, 0);
    }

    // Pass 3: committed path polyline, colored per segment by the
    // destination waypoint's (layer, area).  An X marker at each
    // waypoint highlights surface transitions.
    if let Some(pc_id) = selected_id
        && let Some(waypoints) = engine.actor_path_waypoints(pc_id)
        && !waypoints.is_empty()
    {
        let start = engine
            .get_entity(pc_id)
            .map(|e| e.element_data().position_map())
            .unwrap_or(waypoints[0]);
        let mut prev = start;
        for &wp in &waypoints {
            let (r, g, b) = match locate_surface(graph, wp) {
                Some((l, a)) => surface_color(l, a),
                None => (255, 255, 255),
            };
            let (x1, y1) = to_screen_i(prev);
            let (x2, y2) = to_screen_i(wp);
            renderer.render_gpu_line(x1, y1, x2, y2, r, g, b);
            const M: i32 = 4;
            renderer.render_gpu_line(x2 - M, y2 - M, x2 + M, y2 + M, r, g, b);
            renderer.render_gpu_line(x2 - M, y2 + M, x2 + M, y2 - M, r, g, b);
            prev = wp;
        }
    }

    // Pass 4: 3D-position anchor for the selected character.
    // Vertical drop from where the sprite renders (iso-projected
    // (x, y - z)) down to the z = 0 ground projection (x, y), plus a
    // small flat ellipse "shadow footprint" at the bottom.  Makes the
    // entity's height immediately visible — useful when debugging
    // movement on rooftops, ladders, or during jumps/falls.
    if let Some(pc_id) = selected_id
        && let Some(entity) = engine.get_entity(pc_id)
    {
        let pos = entity.element_data().position();
        // Top: where the sprite is drawn.  Bottom: same map (x, y)
        // but at z = 0.
        let top_map = pos.to_map();
        let top_x_w = top_map.x;
        let top_y_w = top_map.y;
        let bot_x_w = pos.x;
        let bot_y_w = pos.y;
        let top = (
            ((top_x_w - view.x) * zoom).round() as i32,
            ((top_y_w - view.y) * zoom).round() as i32,
        );
        let bot = (
            ((bot_x_w - view.x) * zoom).round() as i32,
            ((bot_y_w - view.y) * zoom).round() as i32,
        );
        // Vertical drop line.
        renderer.render_gpu_line(top.0, top.1, bot.0, bot.1, 255, 255, 255);
        // Footprint ellipse: 16 segments around an ellipse with
        // world-unit radii (rx, ry) — flattened to suggest the
        // ground plane.  Drawn in screen space directly.
        const RX_W: f32 = 8.0;
        const RY_W: f32 = 3.0;
        const SEGMENTS: u32 = 16;
        let cx = bot.0 as f32;
        let cy = bot.1 as f32;
        let rx_s = RX_W * zoom;
        let ry_s = RY_W * zoom;
        let mut prev_pt = (cx + rx_s, cy);
        for i in 1..=SEGMENTS {
            let t = (i as f32) * std::f32::consts::TAU / (SEGMENTS as f32);
            let p = (cx + rx_s * t.cos(), cy + ry_s * t.sin());
            renderer.render_gpu_line(
                prev_pt.0.round() as i32,
                prev_pt.1.round() as i32,
                p.0.round() as i32,
                p.1.round() as i32,
                255,
                255,
                255,
            );
            prev_pt = p;
        }
    }
}

/// Render debug animation lines: polylines for all FX entities.
///
/// Active FX are drawn in white (0xFFFF), inactive in dark gray
/// (0xFA00).
/// Render the `noise_display` cheat overlay.
///
/// 0. Outline every sound-sector polygon on the visible map in dark
///    teal (0x00AF).  These are the material sectors that feed
///    footstep material lookups + water/hole detection.
/// 1. For every PC, print the current-floor material name above the
///    PC and draw expanding isometric rings sized by the PC's
///    currently-produced footstep noise volume.  The start radius
///    scrolls via `dev.noise_display_start_radius`, which the engine
///    advances each tick so the rings animate outward.
/// 2. For every punctual noise active in `dev.displayed_noises`
///    (populated by `broadcast_noise`), draw concentric rings from
///    `start_radius` up to the effective volume; those entries retire
///    on the sim side once the ring has outgrown the volume.
/// 3. For the currently view-selected NPC (or the first NPC if none
///    is selected), draw a black ring at its `cover_noise_deafness`
///    radius — the "can't hear inside this circle" envelope.
pub(crate) fn render_noise_display(
    host: &mut Host,
    engine: &Engine,
    assets: &LevelAssets,
    dev: &engine_api::DevState,
    fonts: Option<&HudFonts>,
    selected_view_element: Option<engine_element::EntityId>,
    renderer: &mut Renderer,
) {
    if !dev.debug.noise_display {
        return;
    }

    const CIRCLE_DISTANCE: u16 = 20;
    const HEARING_FACTOR: f32 = 1.0;

    // ── (0) Sound-sector polygon outlines ────────────────────────
    // Iterate material sectors registered as sound sectors and draw
    // each polygon outline in dark teal.
    for sector in &assets.material_sectors.sectors {
        if sector.points.len() < 2 {
            continue;
        }
        // `draw_polyline` draws segments between consecutive points —
        // append the first point so the polygon closes.
        let mut closed = sector.points.clone();
        closed.push(sector.points[0]);
        host.draw_manager.draw_polyline(renderer, &closed, 0x00AF);
    }

    // ── (1) Per-PC footstep rings + material label ────────────────
    let start_radius = dev.noise_display_start_radius;
    for entity in engine.entities_iter() {
        let Entity::Pc(pc) = entity else {
            continue;
        };
        let position = pc.element.position_map();
        let origin = position;

        // Material name text.  Use the PC's cached position-interface
        // material (the same value `pc_noise_volume` reads) so the
        // label stays consistent with the ring size.
        if let Some(fonts) = fonts {
            let label = match pc.element.material() {
                GameMaterial::Ground => "ground",
                GameMaterial::Wood => "wood",
                GameMaterial::Stone => "stone",
                GameMaterial::Grass => "grass",
                GameMaterial::Leaves => "leaves",
                GameMaterial::Water => "water",
                GameMaterial::Bush => "bush",
                GameMaterial::Ice => "ice",
                GameMaterial::Hole => "hole",
                GameMaterial::LightShadow => "shadow",
            };
            // Offset (+10, -40) from the centre, in screen space.
            let screen = host.draw_manager.map_to_screen(origin);
            render_text_with_shadow(
                renderer,
                fonts,
                label,
                screen.x as i32 + 10,
                screen.y as i32 - 40,
            );
        }

        let volume = pc.actor.last_noise_volume;
        if volume == 0 {
            continue;
        }
        let effective = (volume as f32 * HEARING_FACTOR) as u16;
        let mut r = start_radius;
        while r < effective {
            host.draw_manager.draw_ellipse(renderer, origin, r, 0xFFFF);
            r = r.saturating_add(CIRCLE_DISTANCE);
            if r == 0 {
                break;
            }
        }
    }

    // ── (2) Punctual noises ──────────────────────────────────────
    for displayed in &dev.displayed_noises {
        let noise = &displayed.noise;
        let origin = engine_coordinates::MapPoint::new(noise.origin.x, noise.origin.y);
        let effective = (noise.volume as f32 * HEARING_FACTOR) as u16;
        let mut r = displayed.start_radius;
        while r < effective {
            host.draw_manager.draw_ellipse(renderer, origin, r, 0xFFFF);
            r = r.saturating_add(CIRCLE_DISTANCE);
            if r == 0 {
                break;
            }
        }
        // Height slices — stack dim ellipses offset vertically by
        // ±height to hint at 3D noise volume.
        let mut sw_height = effective.saturating_sub(1) as i32;
        let min_h = -(effective as i32) + 1;
        while sw_height > min_h {
            if sw_height <= -(noise.elevation as i32) {
                break;
            }
            let r2 = effective as f32 * effective as f32 - (sw_height * sw_height) as f32;
            if r2 > 0.0 {
                let radius = r2.sqrt() as u16;
                host.draw_manager.draw_ellipse(
                    renderer,
                    engine_coordinates::MapPoint::new(origin.x, origin.y - sw_height as f32),
                    radius,
                    0x000A,
                );
            }
            sw_height -= CIRCLE_DISTANCE as i32;
        }
    }

    // ── (3) Selected NPC deafness ring ───────────────────────────
    // Pick the selected view element if it's an NPC, else the first
    // NPC.
    let picked_npc: Option<engine_element::EntityId> = selected_view_element
        .filter(|id| engine.get_entity(*id).map(|e| e.is_npc()).unwrap_or(false))
        .or_else(|| engine.npc_ids().first().copied());
    if let Some(npc_id) = picked_npc
        && let Some(entity) = engine.get_entity(npc_id)
        && let Some(npc) = entity.npc_data()
    {
        // Read the stored deafness — decay was already applied this
        // tick in the sim path.  `get_deafness` mutates for lazy
        // decay, but during rendering we only need the snapshot.
        let radius = npc.old_cover_noise_deafness;
        if radius > 0 {
            let pos = entity.element_data().position_map();
            host.draw_manager
                .draw_ellipse(renderer, pos, radius, 0x0000);
        }
    }
}

pub(crate) fn render_debug_animation_lines(
    host: &mut Host,
    engine: &Engine,
    dev: &engine_api::DevState,
    renderer: &mut Renderer,
) {
    if !dev.debug.display_animation_lines {
        return;
    }

    for entity in engine.entities_iter() {
        if !entity.is_fx() {
            continue;
        }
        let polyline = entity.display_polyline();
        if polyline.is_empty() {
            continue;
        }
        let color: u16 = if entity.is_active() { 0xFFFF } else { 0xFA00 };

        host.draw_manager.draw_polyline(renderer, polyline, color);
    }
}

/// Render the per-NPC "whatsup" debug overlay: a red suspect bar and a
/// white outline rectangle centred 55 world units above each active NPC.
///
/// Gated on `GlobalOptions::whatsup` so the overlay stays off for
/// normal runs.  The reference visibility bar is commented out in the
/// source and is therefore intentionally not ported.
///
/// The red fill occupies the top half of the outline (y in
/// `[-HALFHEIGHT, 0]`) and scales its width by `max_suspect * 0.001`,
/// i.e. `min(1000, max(maximal_detection_suspect, sorrow_level)) /
/// 1000`.  The outline is always the full 40×4 rectangle centred on
/// `position_map + (0, -55)`.
///
/// Architectural note: drawn through the GPU path (matching
/// `render_debug_doors`, `draw_status_bar`, etc.) so we just
/// world→screen transform and let the framebuffer clip.  Empty rects
/// are skipped.
pub(crate) fn render_debug_whatsup_overlay(host: &Host, engine: &Engine, renderer: &mut Renderer) {
    let enabled = host.application_context.options().whatsup;
    if !enabled {
        return;
    }

    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;
    if zoom <= 0.0 {
        return;
    }

    const HALF_WIDTH: f32 = 20.0;
    const HALF_HEIGHT: f32 = 2.0;

    let to_screen = |wx: f32, wy: f32| -> (i32, i32) {
        let sx = ((wx - view.x) * zoom).round() as i32;
        let sy = ((wy - view.y) * zoom).round() as i32;
        (sx, sy)
    };

    for npc_id in engine.npc_ids() {
        let Some(entity) = engine.get_entity(npc_id) else {
            continue;
        };
        if !entity.is_active() || entity.is_dead() {
            continue;
        }
        let Some(ai) = entity.ai_controller() else {
            continue;
        };
        let Some(npc) = entity.npc_data() else {
            continue;
        };

        // `max_suspect` = min(1000, max(maximal_detection_suspect,
        // sorrow_level)) — see `alert_colors.rs:61` for the
        // definition.
        let max_suspect = npc.maximal_detection_suspect.max(ai.sorrow_level).min(1000);

        let pos = entity.element_data().position_map();
        // Centre = position_map + (0, -55).
        let cx = pos.x;
        let cy = pos.y - 55.0;

        // ── Red suspect fill bar (b3D=false) ──
        // full_box = (centre - (HALFWIDTH, HALFHEIGHT),
        //            centre + (-HALFWIDTH + 2*HALFWIDTH*filled, 0))
        let filled = max_suspect as f32 * 0.001;
        let full_min_x = cx - HALF_WIDTH;
        let full_min_y = cy - HALF_HEIGHT;
        let full_max_x = cx + (-HALF_WIDTH + 2.0 * HALF_WIDTH * filled);
        let full_max_y = cy;
        if full_max_x > full_min_x && full_max_y > full_min_y {
            let (sx1, sy1) = to_screen(full_min_x, full_min_y);
            let (sx2, sy2) = to_screen(full_max_x, full_max_y);
            fill_box_whatsup(renderer, sx1, sy1, sx2 - sx1, sy2 - sy1, 255, 0, 0, false);
        }

        // ── White outline around the full bar extent (b3D=false, distance=0) ──
        let (ex1, ey1) = to_screen(cx - HALF_WIDTH, cy - HALF_HEIGHT);
        let (ex2, ey2) = to_screen(cx + HALF_WIDTH, cy + HALF_HEIGHT);
        if ex2 > ex1 && ey2 > ey1 {
            // 4-line rectangle — `b3D=false` bounding-box branch.
            renderer.render_gpu_line(ex1, ey1, ex2, ey1, 255, 255, 255);
            renderer.render_gpu_line(ex2, ey1, ex2, ey2, 255, 255, 255);
            renderer.render_gpu_line(ex2, ey2, ex1, ey2, 255, 255, 255);
            renderer.render_gpu_line(ex1, ey2, ex1, ey1, 255, 255, 255);
        }
    }
}

/// Fill a screen-space rectangle and optionally draw a Windows-button
/// bevel around it.
///
/// The base colour is `(r, g, b)` (handled directly by
/// `render_gpu_rect`, so a depth-aware color helper is not needed);
/// the bevel uses `min(255, c * 1.5)` for the top-left highlight and
/// `c * 0.7` for the bottom-right shadow.  The only caller is
/// `render_debug_whatsup_overlay`, which passes `b3D=false` for both
/// of its boxes, so the bevel arm is wired up for completeness but
/// exercised only when a future caller needs it.
#[allow(clippy::too_many_arguments)]
fn fill_box_whatsup(
    renderer: &mut Renderer,
    sx: i32,
    sy: i32,
    w: i32,
    h: i32,
    r: u8,
    g: u8,
    b: u8,
    b3d: bool,
) {
    if w <= 0 || h <= 0 {
        return;
    }
    renderer.render_gpu_rect(sx, sy, w, h, r, g, b, 255);
    if !b3d {
        return;
    }

    // `(c * 1.5)` clamped to 255.
    let br = ((r as u16 * 3) / 2).min(255) as u8;
    let bg = ((g as u16 * 3) / 2).min(255) as u8;
    let bb = ((b as u16 * 3) / 2).min(255) as u8;
    // `(c * 0.7)` — float truncation to integer.
    let dr = (r as f32 * 0.7) as u8;
    let dg = (g as f32 * 0.7) as u8;
    let db = (b as f32 * 0.7) as u8;

    let x1 = sx;
    let y1 = sy;
    let x2 = sx + w;
    let y2 = sy + h;

    // Shadow: bottom-left → bottom-right, bottom-right → top-right.
    renderer.render_gpu_line(x1, y2, x2, y2, dr, dg, db);
    renderer.render_gpu_line(x2, y2, x2, y1, dr, dg, db);
    // Highlight: bottom-left → top-left, top-left → top-right.
    renderer.render_gpu_line(x1, y2, x1, y1, br, bg, bb);
    renderer.render_gpu_line(x1, y1, x2, y1, br, bg, bb);
}
