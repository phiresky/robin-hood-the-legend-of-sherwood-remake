//! Mission HUD and player-feedback rendering helpers.

use super::render_text_with_shadow;
use crate::host::Host;
use crate::hud_text::HudFonts;
use crate::ingame_menu::resources::{IngameMenuResources, MT_STR_AMULETS, MT_STR_RANSOM};
use crate::renderer::Renderer;
use robin_engine::campaign::CampaignValue;
use robin_engine::coordinates as engine_coordinates;
use robin_engine::element::{Entity, ListenPhase};
use robin_engine::engine::{Engine, MULTI_SELECTION_THRESHOLD};

// ─── Combat status bars (red life / blue stamina) ────────────────

/// Draw the red life / blue stamina bars below characters involved in combat.
///
/// - Red bar at offset_y = 8, width = value * 20, 3 rows tall.
/// - Blue bar at offset_y = 12 (civilians skipped — tiredness only
///   exists on NPCs/PCs that fight).
/// - Row 0: black background, full 20-wide.
/// - Row 0 (top pixel), value-width: bright colour `(r, g, b)`.
/// - Rows 1-2, value-width: darker `(r>>1, g>>1, b>>1)`.
///
/// Targets:
/// - NPC set as `host.input.double_status_bar_entity_id` by the bow /
///   stone mouse-hover handlers in
///   [`robin_engine::engine::input::update_mouse`].
/// - Every selected PC currently swordfighting, plus every opponent on
///   that PC's opponents list.
///
/// The `NpcData::display_double_status_bar` flag (set by the soldier
/// hover path, currently un-ported) is also honoured so the feature is
/// ready once that call site lands.
pub(crate) fn render_combat_status_bars(host: &mut Host, engine: &Engine, renderer: &mut Renderer) {
    use robin_engine::element::{Entity, EntityId, Human};
    use std::collections::HashSet;

    let mut targets: HashSet<EntityId> = HashSet::new();

    // Mouse hover target (bow / stone cursor over an NPC).
    if let Some(id) = host.input.double_status_bar_entity_id {
        targets.insert(id);
    }

    // Each selected PC currently swordfighting — bars for PC + all opponents.
    for &pc_id in engine.seat_selection(host.transport.local_seat) {
        let Some(pc) = engine.get_entity(pc_id) else {
            continue;
        };
        let Some(h) = pc.human_data() else { continue };
        if h.opponents.is_empty() {
            continue;
        }
        targets.insert(pc_id);
        for &opp in &h.opponents {
            targets.insert(opp);
        }
    }

    // NPCs that got `display_double_status_bar` set by other code paths
    // (soldier mouse focus, AI, etc.).  The flag is one-shot:
    // consumers elsewhere clear it after rendering.  We only *read* it
    // here to keep this function `&Engine`; the clearing happens in
    // `clear_display_flags`.
    for npc_id in engine.npc_ids() {
        let Some(e) = engine.get_entity(npc_id) else {
            continue;
        };
        if e.npc_data().is_some_and(|n| n.display_double_status_bar) {
            targets.insert(npc_id);
        }
    }

    for id in targets {
        let Some(entity) = engine.get_entity(id) else {
            continue;
        };
        if !entity.is_active() {
            continue;
        }

        // Dispatch to the Human trait for life/max/tiredness.  Non-human
        // entities have no bars.
        let (life, max_life, tiredness, is_civilian) = match entity {
            Entity::Pc(p) => (
                Human::life_points(p),
                Human::max_life_points(p),
                Human::tiredness(p),
                false,
            ),
            Entity::Soldier(s) => (
                Human::life_points(s),
                Human::max_life_points(s),
                Human::tiredness(s),
                false,
            ),
            Entity::Civilian(c) => (
                Human::life_points(c),
                Human::max_life_points(c),
                Human::tiredness(c),
                true,
            ),
            _ => continue,
        };

        let pos = &entity.element_data().position_map();

        if max_life > 0 {
            // min(1, lifepoints / maxLifePoints)
            let frac = (life as f32 / max_life as f32).clamp(0.0, 1.0);
            draw_status_bar(host, renderer, pos.x, pos.y, 8.0, frac, 255, 0, 0);
        }
        if !is_civilian {
            // max(0, 0.01 * (100 - tiredness))
            let t = tiredness.min(100) as f32;
            let frac = ((100.0 - t) * 0.01).max(0.0);
            draw_status_bar(host, renderer, pos.x, pos.y, 12.0, frac, 3, 205, 255);
        }
    }
}

/// Draw one status bar at `(x_world, y_world + offset_y)` with the given
/// fill fraction.  Helper for [`render_combat_status_bars`].
///
/// Screen coords are computed manually (matching the other GPU-phase calls
/// in this module) rather than going through
/// [`crate::draw_manager::DrawManager::fill_box`], which has broader
/// gameplay draw-manager semantics than this fixed HUD overlay.
#[allow(clippy::too_many_arguments)]
fn draw_status_bar(
    host: &Host,
    renderer: &mut Renderer,
    x_world: f32,
    y_world: f32,
    offset_y: f32,
    frac: f32,
    r: u8,
    g: u8,
    b: u8,
) {
    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;

    // Cast the origin to integer — truncation in world space, required
    // for pixel-perfect parity.
    let origin_x_world = (x_world - 10.0).floor();
    let origin_y_world = (y_world + offset_y).floor();

    // World → screen (same transform used by `render_entities_gpu`).
    let sx = ((origin_x_world - view.x) * zoom).round() as i32;
    let sy = ((origin_y_world - view.y) * zoom).round() as i32;

    // The bar is 20 × 3 in world units; scale by zoom.  Round so every
    // pixel row/col is covered and there is never a 1-pixel seam.
    let w_full = (20.0 * zoom).round().max(1.0) as i32;
    let w_val = (20.0 * frac * zoom).round().max(0.0) as i32;
    let h_top = zoom.round().max(1.0) as i32;
    let h_full = (3.0 * zoom).round().max(1.0) as i32;
    let h_body = (h_full - h_top).max(0);

    // Case 0: black background, full width, full height.
    renderer.render_gpu_rect(sx, sy, w_full, h_full, 0, 0, 0, 255);
    // Case 1 + 2: value-width bright top row + darker body.
    if w_val > 0 {
        renderer.render_gpu_rect(sx, sy, w_val, h_top, r, g, b, 255);
        if h_body > 0 {
            renderer.render_gpu_rect(sx, sy + h_top, w_val, h_body, r >> 1, g >> 1, b >> 1, 255);
        }
    }
}

// ─── Trajectory preview ──────────────────────────────────────────────

/// Distance between trajectory dots in world units.
const TRAJECTORY_DOT_INTERVAL: f32 = 7.0;

/// Draw trajectory preview dots.
///
/// Draws filled 1-pixel squares at regular intervals along the
/// ballistic arc.
pub(crate) fn render_trajectory_preview(host: &mut Host, renderer: &mut Renderer) {
    if !host.valid_trajectory {
        return;
    }

    let view = host.viewport.view_position;
    let zoom = host.viewport.zoom_factor;
    let screen_w = host.viewport.screen_size.x as i32;
    let screen_h = host.viewport.screen_size.y as i32;

    // Trajectory color: cyan (0,231,191) for a normal arc, pink
    // (255,100,150) when the shot is crumpled / will miss
    // (`net_crumpled` is set).
    let (cr, cg, cb) = if host.net_crumpled {
        (255u8, 100u8, 150u8)
    } else {
        (0u8, 231u8, 191u8)
    };

    /// Render dots along a trajectory from `start` through `points`.
    #[allow(clippy::too_many_arguments)]
    fn render_arc(
        start: engine_coordinates::WorldPoint3D,
        points: &[robin_engine::element::TrajectoryPoint],
        view: engine_coordinates::MapPoint,
        zoom: f32,
        screen_w: i32,
        screen_h: i32,
        cr: u8,
        cg: u8,
        cb: u8,
        renderer: &mut Renderer,
    ) {
        if points.is_empty() {
            return;
        }
        let mut last = start;
        let mut carry = 0.0f32;

        for tp in points {
            let current = tp.position;
            let dx = current.x - last.x;
            let dy = current.y - last.y;
            let dz = current.z - last.z;
            let seg_len = (dx * dx + dy * dy + dz * dz).sqrt();

            if seg_len < 0.001 {
                last = current;
                continue;
            }

            let mut dot_distance = TRAJECTORY_DOT_INTERVAL - carry;
            while dot_distance <= seg_len {
                let ratio = dot_distance / seg_len;
                let walk = engine_coordinates::WorldPoint3D {
                    x: last.x + dx * ratio,
                    y: last.y + dy * ratio,
                    z: last.z + dz * ratio,
                };

                let walk_map = walk.to_map();
                let sx = ((walk_map.x - view.x) * zoom) as i32;
                let sy = ((walk_map.y - view.y) * zoom) as i32;

                if sx >= 0 && sy >= 0 && sx < screen_w && sy < screen_h {
                    renderer.render_gpu_rect(sx, sy, 2, 2, cr, cg, cb, 255);
                }

                dot_distance += TRAJECTORY_DOT_INTERVAL;
            }

            let total = carry + seg_len;
            carry = total - (total / TRAJECTORY_DOT_INTERVAL).floor() * TRAJECTORY_DOT_INTERVAL;
            last = current;
        }
    }

    // Render the hover-preview trajectory (computed by is_valid_trajectory).
    if !host.trajectory_preview_points.is_empty() {
        render_arc(
            host.trajectory_preview_start,
            &host.trajectory_preview_points,
            view,
            zoom,
            screen_w,
            screen_h,
            cr,
            cg,
            cb,
            renderer,
        );
    }
}

// ─── Listen / Whistle ability radar ping ─────────────────────────────

/// Draw the expanding radar-ping circle at a PC's feet during the
/// last `TIME_LISTEN` (5) frames of the Listen / Whistle countdown.
///
/// Draws an ellipse with a radius growing from 0 → `DISTANCE_LISTEN`
/// (Listen) or `NOISE_VOLUME_PFIIIT` (Whistle) over `TIME_LISTEN`
/// frames.
pub(crate) fn render_listen_ping(host: &mut Host, engine: &Engine, renderer: &mut Renderer) {
    const TIME_LISTEN: u32 = 5;
    const DISTANCE_LISTEN: f32 = 750.0;
    const NOISE_VOLUME_PFIIIT: f32 = 400.0;
    const LISTEN_STEP_RADIUS: f32 = DISTANCE_LISTEN / TIME_LISTEN as f32;
    const WHISTLE_STEP_RADIUS: f32 = NOISE_VOLUME_PFIIIT / TIME_LISTEN as f32;

    for entity in engine.entities_iter() {
        // Guard: `wait_time != 0 && anim ∈ {Listening, Whistling}`
        // and `wait_time < TIME_LISTEN`.  Listen and Whistle are
        // tracked on separate fields (`listen_wait_time` /
        // `whistle_wait_time`) — only one ability can be active at a
        // time so they never collide.
        let (position, radius) = match entity {
            Entity::Pc(pc) => {
                let listen_active = pc.actor.listen_phase == ListenPhase::CountingDown
                    && pc.actor.listen_wait_time != 0
                    && pc.actor.listen_wait_time < TIME_LISTEN;
                let whistle_active = matches!(
                    pc.actor.active_ability.kind,
                    Some(robin_engine::movement::AbilityKind::Whistle)
                ) && pc.actor.whistle_wait_time != 0
                    && pc.actor.whistle_wait_time < TIME_LISTEN;

                let (wait_time, step) = if listen_active {
                    (pc.actor.listen_wait_time, LISTEN_STEP_RADIUS)
                } else if whistle_active {
                    (pc.actor.whistle_wait_time, WHISTLE_STEP_RADIUS)
                } else {
                    continue;
                };
                // radius = (TIME_LISTEN - wait_time) * STEP
                let frames_in = TIME_LISTEN - wait_time;
                let radius = (frames_in as f32 * step) as u16;
                (pc.element.position_map(), radius)
            }
            _ => continue,
        };
        host.draw_manager.draw_circle(
            renderer, position, radius, 0xFFFF, // white
        );
    }
}

// ─── Ransom/amulet text overlay ──────────────────────────────────────

/// Render the ransom and amulet counters in the top-left corner.
///
/// Renders both values via `render_text_background`, which draws the
/// text with a drop shadow using the shadow font at ±1 offsets, then
/// the main font on top.
pub(crate) fn render_ransom_amulet_overlay(
    engine: &Engine,
    renderer: &mut Renderer,
    fonts: &HudFonts,
    menu_resources: Option<&IngameMenuResources>,
) {
    let campaign = engine.campaign();

    let ransom = campaign.get_value(CampaignValue::Ransom);
    let amulets = campaign.get_value(CampaignValue::Amulets);

    // Use localized menu-text format strings with `%d`.  The demo
    // data's English strings are "Money: £%d" and "Clover: %d"; fall
    // back to hard-coded English "Ransom: %d" / "Amulets: %d" when the
    // table is unavailable.
    let (ransom_tpl, amulet_tpl) = if let Some(res) = menu_resources {
        (
            res.menu_text.get(MT_STR_RANSOM),
            res.menu_text.get(MT_STR_AMULETS),
        )
    } else {
        ("Ransom: %d".into(), "Amulets: %d".into())
    };
    let ransom_text = substitute_int(&ransom_tpl, ransom);
    let amulet_text = substitute_int(&amulet_tpl, amulets);

    // Positions: (0, 0) for ransom, (0, 15) for amulets.  The text
    // renderer's left-anchored point overload insets the glyph anchor
    // by `kerning_margin = 2` on the X axis.
    const KERNING_MARGIN: i32 = 2;
    render_text_with_shadow(renderer, fonts, &ransom_text, KERNING_MARGIN, 0);
    render_text_with_shadow(renderer, fonts, &amulet_text, KERNING_MARGIN, 15);
}

/// Substitute the first `%d` or `%i` token in a C-style format string.
/// C's `swprintf` accepts either for an integer; the stock English demo
/// data uses `%i` ("Money: £%i") while other locales use `%d`.
fn substitute_int(template: &str, value: i32) -> String {
    let d = template.find("%d");
    let i = template.find("%i");
    let pos = match (d, i) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    match pos {
        Some(p) => format!("{}{}{}", &template[..p], value, &template[p + 2..]),
        None => template.to_string(),
    }
}

// ─── Multi-selection rubber-band rectangle ────────────────────────

/// Draw the multi-selection rubber-band box when dragging a selection
/// rectangle on the map.
///
/// Rules:
/// * If any selected controllable unit is swordfighting, cancel both the selection
///   and unselection drags and skip rendering.
/// * Otherwise, once the drag exceeds `MULTI_SELECTION_THRESHOLD`
///   squared distance, latch `draw_multi_selection = true` so
///   subsequent frames paint the box even if the pointer briefly
///   shrinks the rect below the threshold.
/// * When latched, paint the four edges in the select/unselect color.
pub(crate) fn draw_multi_selection_box(
    host: &mut Host,
    engine: &Engine,
    renderer: &mut Renderer,
    advance_transients: bool,
) {
    // ── Swordfighting cancel ──
    if crate::game_input::is_selected_unit_swordfighting(engine, host.transport.local_seat) {
        if advance_transients {
            host.input.multi_selection_active = false;
            host.input.multi_unselection_active = false;
        }
        return;
    }

    if !host.input.multi_selection_active && !host.input.multi_unselection_active {
        return;
    }

    let p1 = host.input.multi_selection_pt1;
    let p2 = host.input.multi_selection_pt2;

    // ── Latch draw_multi_selection once the drag clears the
    //    threshold.  The square norm is in map units; compared to
    //    `MULTI_SELECTION_THRESHOLD` (1600). ──
    let mut draw_multi_selection = host.input.draw_multi_selection;
    if !draw_multi_selection {
        let dx = p1.x - p2.x;
        let dy = p1.y - p2.y;
        if dx * dx + dy * dy > MULTI_SELECTION_THRESHOLD {
            draw_multi_selection = true;
            if advance_transients {
                host.input.draw_multi_selection = true;
            }
        }
    }

    if !draw_multi_selection {
        return;
    }

    // ── Colors: 0x737 for select, 0x373 for unselect — written
    //    directly as RGB565 pixel values. ──
    let color: u16 = if host.input.multi_selection_active {
        0x0737
    } else {
        0x0373
    };

    // ── Compute screen-space corners via the unclamped transform;
    //    the GPU line renderer clips off-screen pieces. ──
    let a = host
        .viewport
        .map_to_screen_unclamped(engine_coordinates::MapPoint::new(
            p1.x.min(p2.x),
            p1.y.min(p2.y),
        ));
    let b = host
        .viewport
        .map_to_screen_unclamped(engine_coordinates::MapPoint::new(
            p1.x.max(p2.x),
            p1.y.min(p2.y),
        ));
    let c = host
        .viewport
        .map_to_screen_unclamped(engine_coordinates::MapPoint::new(
            p1.x.max(p2.x),
            p1.y.max(p2.y),
        ));
    let d = host
        .viewport
        .map_to_screen_unclamped(engine_coordinates::MapPoint::new(
            p1.x.min(p2.x),
            p1.y.max(p2.y),
        ));

    renderer.draw_line_screen(a.x as i32, a.y as i32, b.x as i32, b.y as i32, color);
    renderer.draw_line_screen(b.x as i32, b.y as i32, c.x as i32, c.y as i32, color);
    renderer.draw_line_screen(c.x as i32, c.y as i32, d.x as i32, d.y as i32, color);
    renderer.draw_line_screen(d.x as i32, d.y as i32, a.x as i32, a.y as i32, color);
}
