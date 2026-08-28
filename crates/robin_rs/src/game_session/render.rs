//! Render-frame and screenshot/printscreen drain helpers.
//! Houses `RenderContext` (the bundle of GPU + tooltip resources passed
//! into `render_frame`) and the rewind-icon HUD glyph.

use super::selected_pc_profile_indices;
use super::tick::drain_pending_console_output;
use crate::corner_hud::{self, CornerButtonEnable, CornerHoverState, CornerTooltipTracker};
use crate::game::Game;
use crate::game_render::{
    render_bg_animations_gpu, render_combat_status_bars, render_debug_animation_lines,
    render_debug_doors, render_debug_motion_graph, render_debug_surfaces_fill,
    render_debug_surfaces_outline, render_debug_whatsup_overlay, render_door_overlays,
    render_entities_gpu, render_ground_marks, render_listen_ping, render_minimap,
    render_noise_display, render_ransom_amulet_overlay, render_selection_outlines_gpu,
    render_shadow_polygon_sphere_debug, render_trajectory_preview, render_view_cone_overlay,
};
use crate::host::Host;
use crate::host::PrintScreenRequest;
use crate::ingame_menu::{IngameMenuResources, PauseMenu};
use crate::level_loading_host::EngineLevelLoadExt;
use crate::presentation::{PresentationFrameId, ZoomPresentationUpdate};
use crate::save_file::{THUMB_HEIGHT, THUMB_WIDTH, Thumbnail};
use crate::sherwood_hud::{self, SherwoodButtonEnable, SherwoodTooltipTracker};
use crate::sound::MusicMode;
use crate::stature_hud::{self, StatureEnable, StatureHoverState, StatureTooltipTracker};
use crate::ui_panel::{PortraitHit, PortraitHitArea, PortraitTarget, hit_test_portrait_detailed};
use crate::widget::blazon_bar;
use crate::widget::requirements::{RequirementSlot, build_requirements_state};
use crate::zoom_hud::{self, ZoomButtonEnable, ZoomHoverState, ZoomTooltipTracker};
use robin_engine::ai::{DetachedPatrolPathStatus, PathId, PatrolPath};
use robin_engine::coordinates as engine_coordinates;
use robin_engine::element as engine_element;
use robin_engine::element::Posture;
use robin_engine::engine as engine_api;
use robin_engine::engine::Engine;
use robin_engine::engine::input::MOUSE_OPACITY_DEFAULT;
use robin_engine::engine_manager as engine_manager_api;
use robin_engine::profiles as engine_profiles;
use robin_engine::resource_ids as engine_resource_ids;
use robin_engine::sprite as engine_sprite;
use robin_engine::tactical_control::{CombatStance, TacticalDuty, TacticalFormation};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
struct PatrolRouteOverlay {
    points: Vec<engine_coordinates::MapPoint>,
    active_waypoint: usize,
}

fn authored_patrol_route(
    path: Option<&PatrolPath>,
    detached: &DetachedPatrolPathStatus,
    has_patrol_path: bool,
    hiking_paths: &[robin_engine::level_data::RawHikingPath],
) -> Option<(PathId, PatrolRouteOverlay)> {
    if !has_patrol_path {
        return None;
    }

    let (path_id, current_waypoint) = match path {
        Some(path) => (
            path.hiking_path_index,
            usize::from(path.current_waypoint_index),
        ),
        None => (
            detached.hiking_path_index?,
            usize::from(detached.current_waypoint_index),
        ),
    };
    let raw_path = hiking_paths.get(usize::from(path_id)).unwrap_or_else(|| {
        panic!("selected soldier patrol references missing hiking path {path_id}")
    });
    if raw_path.waypoints.is_empty() {
        return None;
    }
    assert!(
        current_waypoint < raw_path.waypoints.len(),
        "selected soldier patrol path {path_id} has waypoint {current_waypoint}, but only {} waypoints",
        raw_path.waypoints.len()
    );

    Some((
        path_id,
        PatrolRouteOverlay {
            points: raw_path
                .waypoints
                .iter()
                .map(|waypoint| {
                    engine_coordinates::MapPoint::new(f32::from(waypoint.x), f32::from(waypoint.y))
                })
                .collect(),
            active_waypoint: current_waypoint,
        },
    ))
}

fn selected_allied_patrol_routes(
    engine: &Engine,
    assets: &engine_api::LevelAssets,
    seat: robin_engine::player_command::PlayerId,
) -> Vec<PatrolRouteOverlay> {
    let mut routes = Vec::new();
    let mut shown_authored_paths = HashSet::new();

    for &soldier_id in engine.tactical_selection(seat) {
        if let Some(order) = engine.tactical_order(soldier_id)
            && let TacticalDuty::Patrol { points, next } = &order.duty
        {
            routes.push(PatrolRouteOverlay {
                points: points.to_vec(),
                active_waypoint: usize::from(*next),
            });
            continue;
        }

        let entity = engine
            .get_entity(soldier_id)
            .unwrap_or_else(|| panic!("selected allied soldier {soldier_id:?} disappeared"));
        let ai = entity.ai_controller().unwrap_or_else(|| {
            panic!("selected allied soldier {soldier_id:?} has no AI controller")
        });
        let Some((path_id, route)) = authored_patrol_route(
            ai.patrol_path.as_ref(),
            &ai.detached_patrol_path_status,
            ai.has_patrol_path,
            &assets.hiking_paths,
        ) else {
            continue;
        };
        if shown_authored_paths.insert(path_id) {
            routes.push(route);
        }
    }

    routes
}

fn render_selected_allied_patrol_routes(
    host: &Host,
    engine: &Engine,
    assets: &engine_api::LevelAssets,
    seat: robin_engine::player_command::PlayerId,
    renderer: &mut crate::renderer::Renderer,
) {
    const ROUTE_COLOR: u32 = 0x42_CE_72;
    const WAYPOINT_COLOR: u32 = 0x8A_EA_9E;
    const ACTIVE_COLOR: u32 = 0xFF_D2_55;
    const DOT_SPACING: f32 = 9.0;

    let routes = selected_allied_patrol_routes(engine, assets, seat);
    if routes.is_empty() {
        return;
    }

    // Advancing the phase makes the dots flow along the route without adding
    // presentation state to the deterministic simulation.
    let mut phase = (engine.frame_counter() % 18) as f32 * 0.5;
    let route_color = host.draw_manager.pack_color(ROUTE_COLOR);
    let waypoint_color = host.draw_manager.pack_color(WAYPOINT_COLOR);
    let active_color = host.draw_manager.pack_color(ACTIVE_COLOR);
    let pulse_radius = 6 + ((engine.frame_counter() / 4) % 3) as u16;

    for route in routes {
        for segment in route.points.windows(2) {
            host.draw_manager.draw_dotted_line(
                renderer,
                segment[0],
                segment[1],
                &mut phase,
                DOT_SPACING,
                1.25,
                route_color,
            );
        }
        for (index, &point) in route.points.iter().enumerate() {
            if index == route.active_waypoint {
                host.draw_manager
                    .draw_ellipse(renderer, point, pulse_radius, active_color);
            } else {
                host.draw_manager
                    .draw_ellipse(renderer, point, 4, waypoint_color);
            }
        }
    }
}

fn allied_portrait_tooltip(
    engine: &Engine,
    seat: robin_engine::player_command::PlayerId,
    hit: PortraitHit,
) -> String {
    let members: Vec<_> = match hit.target {
        PortraitTarget::AlliedSelection => engine.tactical_selection(seat).to_vec(),
        PortraitTarget::AlliedGroup(group_id) => engine
            .tactical_pinned_groups(seat)
            .iter()
            .find(|group| group.id == group_id)
            .unwrap_or_else(|| panic!("tooltip references missing allied group {group_id}"))
            .members
            .clone(),
        PortraitTarget::Pc(_) => panic!("allied tooltip requested for PC portrait"),
    };
    let order = members
        .first()
        .and_then(|soldier| engine.tactical_order(*soldier));
    match hit.area {
        PortraitHitArea::AlliedAction(0) => {
            let stance = order.map_or(CombatStance::Defensive, |order| order.stance);
            let name = match stance {
                CombatStance::Hold => "Hold position",
                CombatStance::Defensive => "Defensive",
                CombatStance::Aggressive => "Aggressive",
            };
            format!("Stance: {name} - click to change")
        }
        PortraitHitArea::AlliedAction(1) => {
            let active =
                order.is_some_and(|order| matches!(order.duty, TacticalDuty::Patrol { .. }));
            if active {
                "Patrol: active - click, then choose a new route point".to_owned()
            } else {
                "Patrol: click, then choose a route point".to_owned()
            }
        }
        PortraitHitArea::AlliedAction(2) => {
            let formation = order.map_or(TacticalFormation::Line, |order| order.formation);
            let name = match formation {
                TacticalFormation::Line => "Line (officer center, melee front, ranged rear)",
                TacticalFormation::Box => "Box (officer center, ranged protected inside)",
                TacticalFormation::Staggered => "Staggered rows (officer leads)",
                TacticalFormation::Flank => "Flank (officer center, two wings)",
            };
            format!("Formation: {name} - click to change")
        }
        PortraitHitArea::Pin => match hit.target {
            PortraitTarget::AlliedSelection => {
                "Pin this soldier group to the portrait bar".to_owned()
            }
            PortraitTarget::AlliedGroup(_) => {
                "Unpin this soldier group from the portrait bar".to_owned()
            }
            PortraitTarget::Pc(_) => panic!("allied pin tooltip requested for PC portrait"),
        },
        _ => panic!("allied tooltip requested for non-control portrait area"),
    }
}

/// Update the zoom-HUD presentation area once for the current simulation
/// frame. The renderer retains the frame-addressed immutable snapshot so all
/// render passes in this loop iteration consume identical data.
///
/// Original provenance:
/// - `original-code/sblibng/SBUI.cpp`, `SBUI::Update`: tooltip focus/timer
///   state advances before `SBUI::Display` draws it (lines 382-433).
/// - `original-code/RHgame.cpp`, `RHGame::InitializeInterface`: the Zoom+ and
///   Zoom- widgets receive their localized tooltips (lines 538-545).
fn update_zoom_presentation(
    engine: &Engine,
    display: &engine_api::HostDisplayState,
    host: &Host,
    ctx: &mut RenderContext<'_>,
) {
    let frame_id = PresentationFrameId::new(engine.frame_counter());
    let enable = ZoomButtonEnable::from_engine(engine, display);
    let mouse = ctx.threaded_input.position();
    let hovered = ctx
        .zoom_layout
        .hit_test_geometric(mouse.x as i32, mouse.y as i32);
    let input = ZoomPresentationUpdate::new(enable, hovered, host.input.left_mouse_down);
    ctx.renderer
        .update_zoom_presentation(frame_id, input, &mut *ctx.zoom_tooltip);
}

/// Render a throwaway frame per pending `/screenshot` request, reply
/// with the captured PNG, then clear the offscreen target for the
/// live frame.  No-op when nothing is pending.
///
/// Each screenshot renders against a **clone** of `dev` with its own
/// debug-flag overrides — the live `dev` is never mutated. Tooltip trackers
/// that have not yet moved to immutable presentation data are snapshotted
/// and restored around each render. The zoom HUD is prepared once per engine
/// frame and needs no clone/restore workaround. `host.input` hover state
/// (focused_entity_id etc.) is not restored because the live `render_frame`
/// overwrites it anyway.
pub(super) fn drain_screenshots(
    sim_frame: u32,
    engine: &Engine,
    display: &engine_api::HostDisplayState,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    dev: &engine_api::DevState,
    ctx: &mut RenderContext<'_>,
) {
    // This is the normal frame's update boundary even when there are no HTTP
    // requests. The live draw later in the loop only reads the snapshot.
    update_zoom_presentation(engine, display, host, ctx);

    let pending = crate::http_server::take_pending_screenshots(sim_frame);
    if pending.is_empty() {
        return;
    }
    for ss in pending {
        match render_screenshot_rgba(engine, display, host, assets, dev, ss.request(), ctx) {
            Ok((w, h, rgba)) => ss.respond(w, h, &rgba),
            Err(err) => ss.respond_err(err),
        }

        // Clear the offscreen target so the next render (another
        // screenshot or the live frame) starts from a clean slate.
        ctx.renderer.reset_render_target();
    }
}

/// Render either the current viewport or the complete level according to the
/// same request used by the HTTP screenshot endpoint.
fn render_screenshot_rgba(
    engine: &Engine,
    display: &engine_api::HostDisplayState,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    dev: &engine_api::DevState,
    request: &crate::http_server::ScreenshotRequest,
    ctx: &mut RenderContext<'_>,
) -> Result<(u32, u32, Vec<u8>), String> {
    let mut scratch_dev = dev.clone();
    crate::http_server::apply_screenshot_flags(&mut scratch_dev.debug, &request.flags);

    // Snapshot update-owned presentation state so a throwaway screenshot
    // cannot change the following live render.
    let saved_corner = ctx.corner_tooltip.clone();
    let saved_requirements = ctx.requirements_tooltip.clone();
    let saved_blazon = ctx.blazon_tooltip.clone();
    let saved_stature = ctx.stature_tooltip.clone();
    let saved_sherwood = ctx.sherwood_tooltip.clone();
    let saved_pc_action = ctx.pc_action_tooltip.clone();
    let saved_draw_hud = ctx.draw_hud;
    ctx.draw_hud = !request.hide_ui;

    let captured = if request.full_map {
        capture_wide_map_rgba(engine, display, host, assets, &scratch_dev, ctx)
    } else {
        render_frame(engine, display, host, assets, &scratch_dev, ctx);
        ctx.renderer
            .capture_frame_rgba()
            .ok_or_else(|| "renderer returned no framebuffer".to_owned())
    };

    *ctx.corner_tooltip = saved_corner;
    *ctx.requirements_tooltip = saved_requirements;
    *ctx.blazon_tooltip = saved_blazon;
    *ctx.stature_tooltip = saved_stature;
    *ctx.sherwood_tooltip = saved_sherwood;
    *ctx.pc_action_tooltip = saved_pc_action;
    ctx.draw_hud = saved_draw_hud;
    captured
}

/// Render a dedicated throwaway frame for a save-slot thumbnail and
/// return it downsampled to the configured thumbnail dimensions.
///
/// This mirrors the HTTP screenshot path: render intentionally, read
/// back immediately, then clear the renderer queue so the live frame
/// later in the loop starts clean.
pub(super) fn capture_save_thumbnail(
    engine: &Engine,
    display: &engine_api::HostDisplayState,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    dev: &engine_api::DevState,
    ctx: &mut RenderContext<'_>,
) -> Option<Thumbnail> {
    update_zoom_presentation(engine, display, host, ctx);

    let saved_corner = ctx.corner_tooltip.clone();
    let saved_requirements = ctx.requirements_tooltip.clone();
    let saved_blazon = ctx.blazon_tooltip.clone();
    let saved_stature = ctx.stature_tooltip.clone();
    let saved_sherwood = ctx.sherwood_tooltip.clone();
    let saved_pc_action = ctx.pc_action_tooltip.clone();

    render_frame(engine, display, host, assets, dev, ctx);

    let thumb = match ctx.renderer.capture_frame_rgba() {
        Some((w, h, rgba)) => {
            Thumbnail::from_rgba_downscaled(w, h, &rgba, THUMB_WIDTH, THUMB_HEIGHT)
                .map_err(|err| {
                    tracing::warn!("Save thumbnail capture failed: {err:#}");
                })
                .ok()
        }
        None => {
            tracing::warn!("Save thumbnail capture failed: renderer returned no framebuffer");
            None
        }
    };

    *ctx.corner_tooltip = saved_corner;
    *ctx.requirements_tooltip = saved_requirements;
    *ctx.blazon_tooltip = saved_blazon;
    *ctx.stature_tooltip = saved_stature;
    *ctx.sherwood_tooltip = saved_sherwood;
    *ctx.pc_action_tooltip = saved_pc_action;
    ctx.renderer.reset_render_target();

    thumb
}

/// Capture the composited frame and write it to disk as a PNG.
///
/// Walks `screen000..screen999` and writes to the first free slot.
/// We use PNG instead of the original TGA format so screenshots share the
/// same encoder path as HTTP screenshots.
pub(super) fn drain_print_screen(renderer: &mut crate::renderer::Renderer) {
    let Some((w, h, rgba)) = renderer.capture_frame_rgba() else {
        tracing::warn!("PrintScreen: renderer returned no framebuffer");
        return;
    };
    write_print_screen_png(w, h, rgba);
}

pub(super) fn drain_print_screen_request(
    renderer: &mut crate::renderer::Renderer,
    request: PrintScreenRequest,
) {
    match request {
        PrintScreenRequest::Plain => drain_print_screen(renderer),
        PrintScreenRequest::Median3x3 => {
            let Some((w, h, rgba)) = renderer.capture_frame_rgba() else {
                tracing::warn!("PrintScreen: renderer returned no framebuffer");
                return;
            };
            write_print_screen_png(w, h, median_filter_rgba_3x3(w, h, &rgba));
        }
        PrintScreenRequest::WideSnapshot => {
            tracing::warn!(
                "PrintScreen Ctrl wide snapshot reached viewport drain; saving current viewport"
            );
            drain_print_screen(renderer);
        }
    }
}

pub(super) fn print_screen_request_from_modifiers(
    ctrl_held: bool,
    shift_held: bool,
) -> PrintScreenRequest {
    if ctrl_held {
        PrintScreenRequest::WideSnapshot
    } else if shift_held {
        PrintScreenRequest::Median3x3
    } else {
        PrintScreenRequest::Plain
    }
}

fn write_print_screen_png(w: u32, h: u32, rgba: Vec<u8>) {
    let dir = crate::save_file::default_save_directory();
    if let Err(err) = std::fs::create_dir_all(&dir) {
        tracing::warn!("PrintScreen: failed to create {}: {err:#}", dir.display());
        return;
    }
    let path = (0..1000)
        .map(|idx| dir.join(format!("screen{idx:03}.png")))
        .find(|p| !p.exists());
    let Some(path) = path else {
        tracing::warn!("PrintScreen: all screen000..screen999 slots are taken");
        return;
    };
    match write_rgba_png(&path, w, h, &rgba) {
        Ok(()) => tracing::info!("PrintScreen → {}", path.display()),
        Err(err) => tracing::warn!("PrintScreen: {err}"),
    }
}

fn write_rgba_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    if rgba.len() != w as usize * h as usize * 4 {
        return Err(format!(
            "invalid RGBA buffer for {}x{} PNG: got {} bytes",
            w,
            h,
            rgba.len()
        ));
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err:#}", parent.display()))?;
    }
    let file = std::fs::File::create(path)
        .map_err(|err| format!("failed to create {}: {err:#}", path.display()))?;
    let mut writer = std::io::BufWriter::new(file);
    let mut enc = png::Encoder::new(&mut writer, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut png_writer| png_writer.write_image_data(rgba))
        .map_err(|err| format!("failed to encode {}: {err:#}", path.display()))
}

pub(super) fn drain_wide_print_screen(
    engine: &Engine,
    display: &engine_api::HostDisplayState,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    dev: &engine_api::DevState,
    ctx: &mut RenderContext<'_>,
) -> bool {
    match capture_wide_map_rgba(engine, display, host, assets, dev, ctx) {
        Ok((w, h, rgba)) => {
            write_print_screen_png(w, h, rgba);
            true
        }
        Err(err) => {
            tracing::warn!("PrintScreen Ctrl wide snapshot: {err}");
            false
        }
    }
}

/// Render a screenshot request and write its full-resolution pixels to `path`.
///
/// The temporary render target includes the bottom panel so the normal
/// frame renderer observes its usual geometry; the returned PNG is cropped
/// to the level bounds and therefore contains the map scene only.
pub(super) fn capture_screenshot_to_path(
    engine: &Engine,
    display: &engine_api::HostDisplayState,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    dev: &engine_api::DevState,
    ctx: &mut RenderContext<'_>,
    request: &crate::http_server::ScreenshotRequest,
    path: &std::path::Path,
) -> Result<(), String> {
    update_zoom_presentation(engine, display, host, ctx);
    let (w, h, rgba) = render_screenshot_rgba(engine, display, host, assets, dev, request, ctx)?;
    write_rgba_png(path, w, h, &rgba)
}

fn capture_wide_map_rgba(
    engine: &Engine,
    display: &engine_api::HostDisplayState,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    dev: &engine_api::DevState,
    ctx: &mut RenderContext<'_>,
) -> Result<(u32, u32, Vec<u8>), String> {
    let level_w = host.viewport.level_size.x.ceil() as u32;
    let level_h = host.viewport.level_size.y.ceil() as u32;
    if level_w == 0 || level_h == 0 {
        return Err("level size is empty".to_owned());
    }
    if level_w > u16::MAX as u32
        || level_h.saturating_add(engine_api::PANNEL_HEIGHT as u32) > u16::MAX as u32
    {
        return Err(format!("level {level_w}x{level_h} exceeds renderer limits"));
    }

    let saved_view = host.viewport.view_position;
    let saved_old_view = host.viewport.old_view_position;
    let saved_zoom = host.viewport.zoom_factor;
    let saved_old_zoom = host.viewport.old_zoom_factor;
    let saved_screen = host.viewport.screen_size;
    let saved_renderer_w = ctx.renderer.screen_width();
    let saved_renderer_h = ctx.renderer.screen_height();

    let render_h = level_h + engine_api::PANNEL_HEIGHT as u32;
    host.viewport.view_position = engine_coordinates::MapPoint::ZERO;
    host.viewport.old_view_position = host.viewport.view_position;
    host.viewport.zoom_factor = 1.0;
    host.viewport.old_zoom_factor = 1.0;
    host.viewport
        .set_screen_size(level_w as f32, render_h as f32);
    ctx.renderer.resize(level_w as u16, render_h as u16);

    render_frame(engine, display, host, assets, dev, ctx);
    let captured = ctx.renderer.capture_frame_rgba();

    ctx.renderer.resize(saved_renderer_w, saved_renderer_h);
    host.viewport.view_position = saved_view;
    host.viewport.old_view_position = saved_old_view;
    host.viewport.zoom_factor = saved_zoom;
    host.viewport.old_zoom_factor = saved_old_zoom;
    host.viewport
        .set_screen_size(saved_screen.x, saved_screen.y);

    let Some((w, h, rgba)) = captured else {
        return Err("renderer returned no framebuffer".to_owned());
    };
    if w != level_w || h < level_h {
        return Err(format!(
            "captured unexpected frame {w}x{h}, expected at least {level_w}x{level_h}"
        ));
    }

    let row_bytes = w as usize * 4;
    let crop_bytes = level_h as usize * row_bytes;
    Ok((level_w, level_h, rgba[..crop_bytes].to_vec()))
}

fn median_filter_rgba_3x3(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let width = w as usize;
    let height = h as usize;
    if width == 0 || height == 0 || rgba.len() != width * height * 4 {
        tracing::warn!(
            "PrintScreen median filter: invalid frame {}x{} with {} bytes",
            w,
            h,
            rgba.len()
        );
        return rgba.to_vec();
    }

    let mut out = rgba.to_vec();
    let mut samples = [0u8; 9];
    for y in 0..height {
        for x in 0..width {
            for channel in 0..3 {
                let mut n = 0;
                for dy in -1isize..=1 {
                    let sy = (y as isize + dy).clamp(0, height as isize - 1) as usize;
                    for dx in -1isize..=1 {
                        let sx = (x as isize + dx).clamp(0, width as isize - 1) as usize;
                        samples[n] = rgba[(sy * width + sx) * 4 + channel];
                        n += 1;
                    }
                }
                samples.sort_unstable();
                out[(y * width + x) * 4 + channel] = samples[4];
            }
            out[(y * width + x) * 4 + 3] = rgba[(y * width + x) * 4 + 3];
        }
    }
    out
}

fn render_display_info_overlay(
    host: &mut Host,
    renderer: &mut crate::renderer::Renderer,
    fonts: &crate::hud_text::HudFonts,
    elapsed_secs: u32,
) {
    debug_assert!(
        renderer.is_gpu_phase(),
        "render_display_info_overlay runs after flush_base_layer"
    );

    let now = crate::window::process_uptime_ms();
    let frame_ms = if host.display_info_last_tick_ms == 0 {
        engine_api::FRAME_TIME_MS
    } else {
        now.saturating_sub(host.display_info_last_tick_ms).max(1)
    };
    host.display_info_last_tick_ms = now;
    let cursor = host.display_info_sample_cursor % host.display_info_frame_samples.len();
    host.display_info_frame_samples[cursor] = frame_ms;
    host.display_info_sample_cursor = (cursor + 1) % host.display_info_frame_samples.len();
    let sample_sum: u32 = host.display_info_frame_samples.iter().copied().sum();
    let avg_ms = (sample_sum / host.display_info_frame_samples.len() as u32).max(1);
    let fps = 1000 / avg_ms;

    let sw = renderer.screen_width() as i32;
    let sh = renderer.screen_height() as i32;
    let font = &fonts.tooltip_font;
    let shadow = fonts.shadow_font.as_ref();
    let text = |renderer: &mut crate::renderer::Renderer, line: &str, x: i32, y: i32| {
        crate::hud_text::render_text_background(font, shadow, line, x, y, |f, t, fx, fy| {
            crate::ingame_menu::layout::render_text_screen_font(renderer, f, t, fx, fy);
        });
    };

    let opts = host.application_context.options();
    let version = format!(
        "v{}.{}.{:03} ({})",
        opts.major_version, opts.minor_version, opts.build_number, opts.release_name
    );
    text(renderer, &version, (sw - 150).max(0), (sh - 32).max(0));

    let minutes = elapsed_secs / 60;
    let seconds = elapsed_secs % 60;
    text(
        renderer,
        &format!("{minutes:02}:{seconds:02}"),
        (sw - 200).max(0),
        8,
    );
    text(
        renderer,
        &format!("Time {avg_ms:03} -> FPS {fps:02}"),
        (sw - 200).max(0),
        16,
    );

    let left = (sw - 160).max(0);
    let top = (sh - 200).max(42);
    text(renderer, "Music mode", left, top - 12);
    renderer.draw_rect_outline_screen(left, top, left + 129, top + 33, 0xffff);

    let quiet = host.audio.sound.quiet_mode_weight().min(256);
    let alert = host.audio.sound.alert_mode_weight().min(256);
    let fight = host.audio.sound.fight_mode_weight().min(256);
    fill_display_bar(renderer, left + 1, top + 3, quiet, 0x97cc);
    fill_display_bar(renderer, left + 1, top + 13, alert, 0xfe40);
    fill_display_bar(renderer, left + 1, top + 23, fight, 0xfa80);

    let mode_color = if host.audio.sound.is_new_music_starting() {
        0x03ef
    } else {
        match host.audio.sound.music_mode() {
            MusicMode::Quiet => 0x07ef,
            MusicMode::Alert => 0xfbe0,
            MusicMode::Fight => 0xf80f,
        }
    };
    text(
        renderer,
        &format!("{}%", host.audio.sound.stream_relative_position()),
        left + 96,
        top - 12,
    );
    fill_rect(renderer, left + 84, top - 8, 12, 4, mode_color);

    host.display_info_max_pending_sounds = host
        .display_info_max_pending_sounds
        .max(host.audio.sound.num_pending_sounds());
    fill_rect(renderer, left - 24, top + 48, 180, 12, 0x2408);
    text(
        renderer,
        &format!(
            "PS: {:4} MAX: {:4}",
            host.audio.sound.num_pending_sounds(),
            host.display_info_max_pending_sounds
        ),
        left - 24,
        top + 48,
    );

    let stats = host.audio.sound.sound_cache.get_cache_stats();
    fill_rect(renderer, left - 24, top + 88, 190, 42, 0x7bd4);
    for (idx, label) in ["FX", "SR", "SP", "GL"].iter().enumerate() {
        let stat = &stats[idx];
        text(
            renderer,
            &format!(
                "H {:06} M {:06} S: {:05} Kb {label}",
                stat.hits,
                stat.misses,
                stat.data_size >> 10
            ),
            left - 24,
            top + 88 + idx as i32 * 10,
        );
    }
}

fn fill_display_bar(
    renderer: &mut crate::renderer::Renderer,
    x: i32,
    y: i32,
    weight: u32,
    color: u16,
) {
    let width = ((weight * 128) / 256) as i32;
    if width > 0 {
        fill_rect(renderer, x, y, width, 8, color);
    }
}

fn fill_rect(renderer: &mut crate::renderer::Renderer, x: i32, y: i32, w: i32, h: i32, color: u16) {
    if w <= 0 || h <= 0 {
        return;
    }
    let rect = engine_sprite::BBox::from_coords(x as f32, y as f32, (x + w) as f32, (y + h) as f32);
    renderer.fill_screen(Some(&rect), color);
}

/// Per-frame mouse/cursor update hoisted out of `render_frame` so that
/// pass can observe an immutable `&Engine`.
///
/// `host_mouse::update_mouse` updates host-side per-frame state
/// (`focused_entity_id`, `selected_sector_idx`, cursor shadow/opacity,
/// etc.). The cursor texture upload lives here too since it reads the
/// same `new_cursor` id. Sim mutations such as
/// `PlayerCommand::PerformOrientation` must run earlier in the frame,
/// before rollback/replay command logging commits the tick.
#[allow(clippy::too_many_arguments)]
pub(super) fn update_mouse_and_cursor(
    manager: &mut engine_manager_api::EngineManager,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    dev: &engine_api::DevState,
    external_actions: &mut Vec<engine_api::ExternalAction>,
    renderer: &mut crate::renderer::Renderer,
    cursor_res: &mut robin_assets::resource_manager::ResourceManager,
    cursor_renderer: &mut crate::cursor::CursorRenderer,
    threaded_input: &crate::input::ThreadedInput,
    portrait_cache: &crate::ui_panel::PortraitCache,
    shift_held: bool,
    last_cursor_id: &mut i32,
) {
    let mouse_screen = threaded_input.position();
    let Some(mouse_map) = host.viewport.screen_to_map(mouse_screen) else {
        return;
    };
    // `is_alt_effective()` so the permanent-alt toggle affects
    // cursor/view-cone hover the same way the physical key does.
    let alt_for_cursor = manager.engine.is_alt_effective(&host.input);
    let mut new_cursor = crate::host_mouse::update_mouse(
        &mut manager.engine,
        host,
        assets,
        dev,
        external_actions,
        mouse_map,
        alt_for_cursor,
        shift_held,
    );

    // The Yes/No cursor for armed portrait actions is keyed off the
    // portrait's own attached PC, not the world cell occluded by the
    // portrait bar.  When the pointer is over a portrait while a
    // Heal/Shield/BigShield action is armed, override the cursor
    // computed by `update_mouse` (which queries `find_focusable_*`
    // against the world `mouse_map`) so the cursor reflects whether
    // the portrait's PC is a valid target.
    let local_seat = host.transport.local_seat;
    let armed = manager.engine.selected_action_for_seat(local_seat);
    if matches!(
        armed,
        engine_profiles::Action::Heal
            | engine_profiles::Action::Shield
            | engine_profiles::Action::BigShield
    ) && let Some(hit) = hit_test_portrait_detailed(
        &manager.engine,
        local_seat,
        portrait_cache,
        renderer.screen_width(),
        renderer.screen_height(),
        mouse_screen.x,
        mouse_screen.y,
    ) && !hit.is_burned
    {
        let pc_id = hit.pc_id;
        let life = manager
            .engine
            .get_entity(pc_id)
            .and_then(|e| e.pc_data())
            .map(|pc| pc.life_points)
            .unwrap_or(0);
        let override_cursor = match armed {
            engine_profiles::Action::Heal => {
                // Same predicate as the portrait Heal commit (alive +
                // injured).
                if life > 0 && life < 100 {
                    Some(engine_resource_ids::RHMOUSE_HEAL_YES)
                } else {
                    Some(engine_resource_ids::RHMOUSE_HEAL_NO)
                }
            }
            engine_profiles::Action::Shield => {
                if life > 0 {
                    Some(engine_resource_ids::RHMOUSE_SHIELD_YES)
                } else {
                    Some(engine_resource_ids::RHMOUSE_SHIELD_NO)
                }
            }
            engine_profiles::Action::BigShield => {
                if life > 0 {
                    Some(engine_resource_ids::RHMOUSE_BIG_SHIELD_YES)
                } else {
                    Some(engine_resource_ids::RHMOUSE_BIG_SHIELD_NO)
                }
            }
            _ => None,
        };
        if let Some(c) = override_cursor {
            new_cursor = c;
        }
    }

    if new_cursor != *last_cursor_id {
        cursor_renderer.load_cursor(new_cursor, cursor_res, renderer);
        *last_cursor_id = new_cursor;
    }
}

/// Bundle of render-only state threaded through [`render_frame`] —
/// mutable GPU/render resources, mutable per-frame UI trackers,
/// immutable resource tables, and a handful of outer-loop inputs
/// (game, pause menu, shift_held).  Short-lived (`'a`) borrows from
/// the [`run_mission`] stack frame.
///
/// The struct exists so the screenshot path can call `render_frame`
/// with a one-liner:
/// `render_frame(&engine, &mut host, &assets, &scratch_dev, &mut ctx)`
/// instead of threading ~25 arguments through the HTTP plumbing.
pub struct RenderContext<'a> {
    // Mutable GPU / render resources.
    pub renderer: &'a mut crate::renderer::Renderer,
    pub cursor_renderer: &'a mut crate::cursor::CursorRenderer,
    pub selection_mark_renderer: &'a mut crate::markers::SelectionMarkRenderer,
    pub titbit_renderer: &'a mut crate::titbit_renderer::TitbitRenderer,
    pub console_overlay: &'a mut crate::console_overlay::ConsoleOverlay,

    // Update-owned per-frame UI trackers (tooltip hover timers). The zoom
    // tracker is only mutated by `update_zoom_presentation`; remaining
    // trackers still await migration out of the draw pass.
    pub zoom_tooltip: &'a mut ZoomTooltipTracker,
    pub corner_tooltip: &'a mut CornerTooltipTracker,
    pub requirements_tooltip: &'a mut crate::ui_panel::RequirementsTooltipTracker,
    pub blazon_tooltip: &'a mut crate::ui_panel::BlazonTooltipTracker,
    pub stature_tooltip: &'a mut StatureTooltipTracker,
    pub sherwood_tooltip: &'a mut SherwoodTooltipTracker,
    pub pc_action_tooltip: &'a mut crate::ui_panel::PcActionTooltipTracker,

    // Immutable resources.
    pub mouse_trail_renderer: Option<&'a crate::mouse_trail::MouseTrailRenderer>,
    pub portrait_cache: &'a crate::ui_panel::PortraitCache,
    pub menu_resources: Option<&'a IngameMenuResources>,
    pub hud_fonts: Option<&'a crate::hud_text::HudFonts>,
    pub short_briefing_strings: &'a std::collections::HashMap<u32, String>,
    pub sherwood_layout: &'a sherwood_hud::SherwoodHudLayout,
    pub sherwood_sprites: &'a sherwood_hud::SherwoodButtonSprites,
    pub zoom_layout: &'a zoom_hud::ZoomHudLayout,
    pub zoom_sprites: &'a zoom_hud::ZoomButtonSprites,
    pub corner_layout: &'a corner_hud::CornerHudLayout,
    pub corner_sprites: &'a corner_hud::CornerButtonSprites,
    pub stature_layout: &'a stature_hud::StatureHudLayout,
    pub stature_sprites: &'a stature_hud::StatureSprites,
    pub threaded_input: &'a crate::input::ThreadedInput,
    pub game: &'a Game,
    pub pause_menu: Option<&'a PauseMenu>,

    // Copy values threaded through from the outer loop.
    pub sherwood_enable: SherwoodButtonEnable,
    pub shift_held: bool,
    pub rewind_active: bool,
    pub display_info_elapsed_secs: u32,

    /// Draw screen-space gameplay UI after the map scene. Mission-map exports
    /// disable this while retaining the normal renderer/viewport dimensions.
    pub draw_hud: bool,
}

impl RenderContext<'_> {
    pub(super) fn present(&mut self) {
        self.renderer.present();
    }
}

/// Render one frame: draws the background, then walks every GPU overlay
/// (selection circles, ground marks, view cone, doors, entities, status
/// bars, HUD, minimap, Sherwood/zoom buttons, tooltips, pause overlay,
/// console, cursor, rewind icon, fade-to-black).
///
/// **Engine is read-only.** The `dev` argument is also read-only —
/// pass a clone with overrides applied (e.g. `&scratch_dev`) if you
/// want the frame to render with alternate debug flags without
/// touching the live sim state.
///
/// The caller is responsible for:
/// - running `pre_render_engine_setup` before this function (drain
///   deferred bg blits, sort display order);
/// - preparing the zoom presentation through the screenshot/thumbnail/wide
///   update boundary before drawing;
/// - calling `renderer.present()` after this function returns;
/// - running `post_render_engine_cleanup` to clear one-shot NPC flags;
/// - skipping the whole trio in fast-forward (`host.skip_render`).
pub(super) fn render_frame(
    engine: &Engine,
    display: &engine_api::HostDisplayState,
    host: &mut Host,
    assets: &engine_api::LevelAssets,
    dev: &engine_api::DevState,
    ctx: &mut RenderContext<'_>,
) {
    // Rendering only reads the zoom presentation prepared at the update
    // boundary. A missing or stale snapshot is an ordering error, never a
    // reason to invent default button state.
    let zoom_frame_id = PresentationFrameId::new(engine.frame_counter());
    let zoom_mouse = ctx.threaded_input.position();
    let zoom_presentation = *ctx
        .renderer
        .zoom_presentation(zoom_frame_id)
        .unwrap_or_else(|err| panic!("render_frame requires prepared zoom presentation: {err}"));

    // Unpack once — the function body is long and every deref is
    // noisy.  All fields are `&'a mut T` / `&'a T`, so this is a
    // reborrow, not a move.
    let renderer = &mut *ctx.renderer;
    renderer.set_shader_frame_count(Some(engine.frame_counter() as usize));
    let cursor_renderer = &mut *ctx.cursor_renderer;
    let selection_mark_renderer = &mut *ctx.selection_mark_renderer;
    let titbit_renderer = &mut *ctx.titbit_renderer;
    let console_overlay = &mut *ctx.console_overlay;
    let corner_tooltip = &mut *ctx.corner_tooltip;
    let requirements_tooltip = &mut *ctx.requirements_tooltip;
    let blazon_tooltip = &mut *ctx.blazon_tooltip;
    let stature_tooltip = &mut *ctx.stature_tooltip;
    let sherwood_tooltip = &mut *ctx.sherwood_tooltip;
    let pc_action_tooltip = &mut *ctx.pc_action_tooltip;
    let mouse_trail_renderer = ctx.mouse_trail_renderer;
    let portrait_cache = ctx.portrait_cache;
    let menu_resources = ctx.menu_resources;
    let hud_fonts = ctx.hud_fonts;
    let short_briefing_strings = ctx.short_briefing_strings;
    let threaded_input = ctx.threaded_input;
    let sherwood_layout = ctx.sherwood_layout;
    let sherwood_enable = ctx.sherwood_enable;
    let sherwood_sprites = ctx.sherwood_sprites;
    let zoom_layout = ctx.zoom_layout;
    let zoom_sprites = ctx.zoom_sprites;
    let corner_layout = ctx.corner_layout;
    let corner_sprites = ctx.corner_sprites;
    let stature_layout = ctx.stature_layout;
    let stature_sprites = ctx.stature_sprites;
    let pause_menu = ctx.pause_menu;
    let game = ctx.game;
    let shift_held = ctx.shift_held;
    let rewind_active = ctx.rewind_active;
    let display_info_elapsed_secs = ctx.display_info_elapsed_secs;
    let draw_hud = ctx.draw_hud;
    let local_seat = host.transport.local_seat;
    // Queue the GPU background texture for the current camera view.
    // Engine-mutating pre-render bookkeeping (bg blits, SortDisplayOrder)
    // is hoisted to the main loop so `render_frame` itself observes an
    // immutable `&Engine` / `&DevState` — this lets
    // the `/screenshot` HTTP endpoint render with dev-flag overrides
    // without disturbing the live sim state.
    // Drop any modal snapshot once gameplay owns the frame again.  While the
    // non-blocking pause menu is open we keep the original gameplay snapshot
    // alive across frames; otherwise the dim pass would either no-op or start
    // tinting the previous pause frame.
    if pause_menu.is_none() {
        renderer.clear_frozen_scene();
    }

    engine.draw_background(host, renderer);
    crate::blit_to_map::render_background_decals(host, renderer);

    // ═══════════════════════════════════════════════════════════
    //  FLUSH: enter GPU overlay phase.  Everything after this point
    //  renders as GPU textures / overlays on top.
    // ═══════════════════════════════════════════════════════════
    renderer.flush_base_layer();

    // C++ parity: RHengine::PerformRefreshAllElements refreshes elevation-zero
    // background animations before ShowDetectionPolygon. Elevated patch FX
    // stay in the normal sorted entity pass.
    render_bg_animations_gpu(engine, host, assets, renderer);

    // Darken the map inside the selected view element's vision cone (if
    // any). C++ draws this immediately after background animations and
    // before door overlays, selection marks, ground marks, and elements.
    render_view_cone_overlay(
        host,
        engine,
        assets,
        host.selected_view_element,
        dev,
        renderer,
    );
    render_shadow_polygon_sphere_debug(host, engine, host.selected_view_element, dev, renderer);

    // ── GPU phase: door / jump zone alpha overlays ──
    // Includes the shift-held `DisplayAllDoorsAndJumpZones` path and
    // the patch-FX overlay.
    render_door_overlays(host, engine, assets, renderer, shift_held);

    // Draw rotating selection circles BELOW the characters' feet for
    // every selected hero and directly controlled ally. C++ draws selection marks after
    // ShowDetectionPolygon and before ground marks/entities.  Skipped when
    // the PC is inside a building or in POSTURE_FLYING.
    for &pc_id in engine
        .hero_selection(local_seat)
        .iter()
        .chain(engine.tactical_selection(local_seat))
    {
        if !engine.pc_draws_selection_mark(pc_id) {
            continue;
        }
        let entity = match engine.get_entity(pc_id) {
            Some(e) => e,
            None => continue,
        };
        let elem = entity.element_data();
        let pos = &elem.position_map();
        let mut map_pt = *pos;
        // Offset +(0, -50) when the PC is on shoulders.
        if elem.posture == Posture::OnShoulders {
            map_pt.y -= 50.0;
        }
        let Some(screen_pt) = host.viewport.map_to_screen(map_pt) else {
            continue;
        };
        // Swordfighting iff the PC has any opponents.
        let in_combat = entity.human_data().is_some_and(|h| !h.opponents.is_empty());
        selection_mark_renderer.draw(
            renderer,
            host.selection_mark.animation_frame(),
            in_combat,
            screen_pt.x as i32,
            screen_pt.y as i32,
        );
    }

    // Draw the destination markers (ground marks).  Drawn AFTER the
    // selection marks but BEFORE entity rendering, so ground marks
    // render on top of selection circles but behind characters.
    render_ground_marks(host, engine, assets, renderer);

    // ── GPU phase: entity sprites (cached as ARGB textures) ──
    // Display-order sort is hoisted to the main loop so it runs
    // before this immutable-render pass — see
    // `pre_render_engine_setup`.  Titbit cursor is reset at the start
    // of the entity pass so the per-human-entity interleave inside
    // `render_entities_gpu` starts at titbit 0 and walks
    // monotonically forward across the entity list.
    // ── GPU phase: surface fill (under sprites) ──
    // Translucent yellow tint for the selected character's `MotionArea`,
    // drawn before sprites so characters / non-static obstacle sprites
    // sit on top.  Outlines + path are drawn after sprites.
    render_debug_surfaces_fill(host, engine, assets, dev, renderer);

    titbit_renderer.begin_frame();
    render_entities_gpu(host, engine, assets, dev, renderer, titbit_renderer);

    // ── Host-side Mark() contributions ──
    // Currently the requirements-bar hover (`MarkPCWithAction`) and
    // the portrait guard-swap hover.  The sim-side call site
    // (mission-team add) has already pushed into
    // `host.input.marked_pc_ids` via `EngineCommand::MarkPc` →
    // `SideEffects::pending_mark_pc_ids` → `apply_side_effects`.  All
    // contributions accumulate into the shared list and the render
    // pass drains it below.
    {
        let mp = threaded_input.position();
        let sw = renderer.screen_width();
        let sh = renderer.screen_height();

        // Requirements-bar hover (Sherwood only).
        if game.is_sherwood
            && let Some(next_idx) = engine.campaign().next_mission_idx
        {
            let campaign = engine.campaign();
            let mission_team = campaign.mission_team_profile_indices();
            let selected = selected_pc_profile_indices(engine, local_seat);
            if let Some(req) = build_requirements_state(
                campaign,
                &assets.profile_manager,
                next_idx,
                &mission_team,
                &selected,
            ) && let Some(slot_idx) = crate::ui_panel::hit_test_requirements_bar(sw, &req, mp)
                && let Some(RequirementSlot::RequiredAction { action, .. }) =
                    req.slots.get(slot_idx)
            {
                engine.collect_pcs_with_action(assets, *action, &mut host.input.marked_pc_ids);
            }
        }

        // Portrait guard-swap hover: when hovering the guard
        // indicator on a burned PC's portrait, flash the PC's
        // guard NPC.
        if let Some(hit) =
            hit_test_portrait_detailed(engine, local_seat, portrait_cache, sw, sh, mp.x, mp.y)
            && hit.is_burned
            && hit.area == PortraitHitArea::Guard
            && let Some(engine_element::Entity::Pc(pc)) = engine.get_entity(hit.pc_id)
            && let Some(guard_id) = pc.pc.guard
        {
            host.input.marked_pc_ids.push(guard_id);
        }
    }

    // ── GPU phase: selection / hover outlines ──
    // Draws coloured outline masks for selected PCs and the hovered
    // entity (focused by the cursor).
    render_selection_outlines_gpu(host, engine, assets, renderer);
    // One-frame Mark() consumption: clear after the outline pass
    // has observed the list — the mark is drained right after the
    // forced outline is drawn.
    host.input.marked_pc_ids.clear();

    // ── GPU phase: combat status bars ─────────────────────────
    // Red life + blue stamina bars below swordfighting PCs, their
    // opponents, and any NPC flagged by bow/stone hover or
    // `MarkToDisplayDoubleStatusBar`.
    render_combat_status_bars(host, engine, renderer);
    // The one-shot "display double status bar" NPC flag is cleared in
    // `post_render_engine_cleanup` (main loop) — `render_frame` is
    // read-only on Engine.

    // ── GPU phase: trajectory preview ──
    // Draws dots along projectile arcs every 7 world units.
    render_trajectory_preview(host, renderer);

    // ── GPU phase: Listen ability radar ping ──
    // Draws an expanding white circle at the PC's feet during the
    // final TIME_LISTEN (5) frames of the Listen countdown.
    render_listen_ping(host, engine, renderer);

    // ── GPU phase: debug animation lines ──
    // Draws polylines for all FX entities when the cheat flag is on.
    render_debug_animation_lines(host, engine, dev, renderer);

    // ── GPU phase: debug door gizmos ──
    // Dispatched when the door-display debug flag is set; draws each
    // gate's endpoint markers + connecting line.
    render_debug_doors(host, engine, dev, renderer);

    // ── GPU phase: pathfinder motion-graph overlay ──
    // Dispatched when the motion-graph debug flag is set (toggle:
    // console cheat "euler"). Draws graph edges + node corner stubs
    // at PC[0]'s pathfinder/half-diagonal index.
    render_debug_motion_graph(host, engine, assets, dev, renderer);

    // ── GPU phase: surface debug outlines + path (above sprites) ──
    // Companion to `render_debug_surfaces_fill` — outlines every
    // `MotionArea`, draws active obstacle outlines, the bright outline
    // on the selected character's surface, and the committed path
    // polyline.  All on top of sprites; only the highlight tint is
    // drawn beneath them.
    render_debug_surfaces_outline(host, engine, assets, dev, renderer);

    // ── GPU phase: per-NPC "whatsup" debug overlay ──
    // Gated on `GlobalOptions::whatsup` so it is off by default.
    render_debug_whatsup_overlay(host, engine, renderer);

    // ── GPU phase: noise-display debug overlay ──
    // Dispatched when the noise-display debug flag is set via the
    // console `NOISE` cheat.  Draws the SECTOR_SOUND polygon
    // outlines, per-PC footstep rings + material labels,
    // broadcast-noise rings animated in from `dev.displayed_noises`,
    // and the selected NPC's cover-noise deafness envelope.
    render_noise_display(
        host,
        engine,
        assets,
        dev,
        hud_fonts,
        host.selected_view_element,
        renderer,
    );

    // ── GPU phase: flush remaining (in-front) titbits ──
    // Interleaved titbits that sit behind each entity are already
    // drawn from inside `render_entities_gpu`.  This flushes every
    // titbit whose display_order is still ahead of the last entity
    // drawn (stars/counters/etc. that belong in front of every
    // actor).
    titbit_renderer.render_up_to(host, engine, assets, renderer, f32::INFINITY);

    // Scene-only captures deliberately stop at the last world-space pass.
    // Keeping this boundary after titbits preserves entity status effects and
    // other mission-authored world visuals, while excluding the panel,
    // minimap, information bars, buttons, tooltips, console, and cursor.
    if !draw_hud {
        return;
    }

    // ── GPU phase: multi-selection rubber band box ──
    crate::game_render::draw_multi_selection_box(host, engine, renderer);

    // ── GPU phase: swordfight mouse-trail ──
    // While dragging during a swordfight, draw the recorded polyline
    // as a fading orange streak and decay its alpha.  Gated on
    // `is_dragging`, not `left_mouse_down`, so the portrait re-arm
    // edge case lines up with the dragging-state semantics.
    if let Some(trail) = mouse_trail_renderer
        && host.input.is_dragging
        && crate::game_input::is_selected_unit_swordfighting(engine, local_seat)
        && !host.mouse_way.is_empty()
    {
        trail.render(&mut host.mouse_way, renderer);
    }

    // ── GPU phase: per-PC macro dotted chains (world space) ──
    // Walks each PC's recorded macro slots and draws a dotted
    // polyline from the PC through its titbit waypoints.  Advances
    // the persistent dotted-line phase stored on `PcMacroState`.
    // Allied patrol routes share this foreground, floating-chain layer.
    render_selected_allied_patrol_routes(host, engine, assets, local_seat, renderer);
    crate::ui_panel::render_macro_dotted_chains(host, engine, renderer);

    // ── GPU phase: UI panel, minimap ──
    let panel_mouse = threaded_input.position();
    crate::ui_panel::draw_panel(
        host,
        engine,
        local_seat,
        &assets.profile_manager,
        renderer,
        portrait_cache,
        panel_mouse.x,
        panel_mouse.y,
        Some(titbit_renderer),
        shift_held,
    );

    // ── GPU phase: blazon-bar / requirements icon strips ──
    // Top-of-screen icon strips rebuilt each frame from campaign
    // state.
    //
    // Default the requirements tooltip to "no hover" each frame so
    // leaving Sherwood (or losing the requirements strip mid-frame)
    // clears the idle timer; the block below re-arms it when the
    // cursor is actually over a slot.
    requirements_tooltip.update(None);
    blazon_tooltip.update(None);
    {
        let campaign = engine.campaign();
        let men_to_blazon = engine.is_men_to_blazon_conversion_mode();
        let blinking = engine.active_blinking_blazons();
        if let Some(bb) = blazon_bar::build_blazon_bar_state(
            campaign,
            &assets.profile_manager,
            men_to_blazon,
            blinking,
        ) {
            crate::ui_panel::draw_blazon_bar(renderer, portrait_cache, &bb);

            // Per-slot hover tooltip with the standard hover timer.
            let mp = threaded_input.position();
            let sw = renderer.screen_width();
            let hovered_slot =
                crate::ui_panel::hit_test_blazon_bar(sw, &bb, mp.x as i32, mp.y as i32);
            blazon_tooltip.update(hovered_slot);
            if let Some(slot_idx) = blazon_tooltip.ready_slot()
                && let Some(kind) = crate::ui_panel::blazon_bar_slot_kinds(&bb)
                    .get(slot_idx)
                    .copied()
                && let (Some(resources), Some(fonts)) = (menu_resources, hud_fonts)
            {
                let mt_id = crate::ui_panel::blazon_slot_tooltip_mt_id(kind);
                let text = resources.menu_text.get(mt_id);
                let (cw, ch) = cursor_renderer.current_frame_size();
                crate::ui_panel::draw_screen_tooltip(
                    renderer,
                    &fonts.tooltip_font,
                    fonts.shadow_font.as_ref(),
                    &text,
                    mp.x as i32,
                    mp.y as i32,
                    (cw as i32, ch as i32),
                );
            }
        }
        let mission_team = campaign.mission_team_profile_indices();
        let selected = selected_pc_profile_indices(engine, local_seat);
        // Original parity: `RHGame::UpdateInformationBars` creates the
        // requirements widget only under `mbSherwood`. Outside Sherwood the
        // top information bar may contain blazons, but never mission-team
        // character/action requirements.
        if game.is_sherwood
            && let Some(next_idx) = campaign.next_mission_idx
            && let Some(req) = build_requirements_state(
                campaign,
                &assets.profile_manager,
                next_idx,
                &mission_team,
                &selected,
            )
        {
            crate::ui_panel::draw_requirements_bar(
                renderer,
                portrait_cache,
                campaign,
                &assets.profile_manager,
                &req,
            );

            // Hover-tooltip per slot type.  The hover pipeline keys
            // the delay on which widget owns the mouse; we reproduce
            // that with a slot-index tracker and paint once it
            // crosses the idle threshold.
            let mp = threaded_input.position();
            let sw = renderer.screen_width();
            let hovered_slot = crate::ui_panel::hit_test_requirements_bar(sw, &req, mp);
            requirements_tooltip.update(hovered_slot);
            if let Some(slot_idx) = requirements_tooltip.ready_slot()
                && let Some(slot) = req.slots.get(slot_idx)
                && let (Some(resources), Some(fonts)) = (menu_resources, hud_fonts)
            {
                let mt_id = crate::ui_panel::requirements_slot_tooltip_mt_id(slot);
                let text = resources.menu_text.get(mt_id);
                let (cw, ch) = cursor_renderer.current_frame_size();
                crate::ui_panel::draw_screen_tooltip(
                    renderer,
                    &fonts.tooltip_font,
                    fonts.shadow_font.as_ref(),
                    &text,
                    mp.x as i32,
                    mp.y as i32,
                    (cw as i32, ch as i32),
                );
            }
        }
    }

    // Minimap is only created in non-Sherwood missions.  In Sherwood
    // the top-right scroll slot is replaced by the campaign-map /
    // go-to-exit widgets, so skip the corner-button blit entirely.
    if !game.is_sherwood {
        render_minimap(host, display, engine, assets, renderer);
    }

    // ── Sherwood HUD buttons ──
    // The DisplayCampaignMap / GoToExit / StartMission / QuitMission
    // widgets on the Sherwood lower panel.  Uses `SherwoodHudLayout`
    // for resolution-dependent positioning and the `sherwood_enable`
    // mask to gate widget state.
    if game.is_sherwood {
        let mp = threaded_input.position();
        let hovered_btn =
            sherwood_layout.hit_test_geometric(mp.x as i32, mp.y as i32, sherwood_enable);
        let hover = sherwood_hud::SherwoodHoverState {
            hovered: hovered_btn,
            mouse_pressed: host.input.left_mouse_down,
        };
        sherwood_hud::draw_with_sprites(
            renderer,
            sherwood_layout,
            sherwood_enable,
            hover,
            sherwood_sprites,
            engine.frame_counter(),
        );

        // Per-button hover tooltip (Start/Quit mission).  The actual
        // text swaps with mode (Sherwood vs in-mission, regular vs
        // men-to-blazon) — `sherwood_button_tooltip_mt_id` owns that
        // 3-way switch.
        sherwood_tooltip.update(hovered_btn);
        if let (Some(resources), Some(fonts)) = (menu_resources, hud_fonts) {
            let (cw, ch) = cursor_renderer.current_frame_size();
            let is_sherwood = game.is_sherwood;
            let men_to_blazon = game.is_men_to_blazon_conversion();
            sherwood_hud::draw_tooltip(
                renderer,
                sherwood_tooltip,
                |btn| {
                    sherwood_hud::sherwood_button_tooltip_mt_id(btn, is_sherwood, men_to_blazon)
                        .map(|mt_id| resources.menu_text.get(mt_id))
                },
                &fonts.tooltip_font,
                fonts.shadow_font.as_ref(),
                mp.x as i32,
                mp.y as i32,
                (cw as i32, ch as i32),
            );
        }
    } else {
        // Outside Sherwood, the Start/Quit widgets are used as
        // Mission-Finish / Mission-Abandon dialogs.  We don't draw
        // those in this HUD pass — they're routed through the
        // pause-menu flow — but we still clear the tooltip tracker
        // so a hover accrued in Sherwood doesn't leak across a mode
        // change.
        sherwood_tooltip.update(None);
    }

    // Zoom HUD buttons (ZoomUp / ZoomDown) on the lower panel. All button
    // and tooltip decisions were prepared in the update phase above; this
    // complete area now only consumes immutable presentation data.
    {
        let hover = ZoomHoverState {
            hovered: zoom_presentation.hovered.map(Into::into),
            mouse_pressed: zoom_presentation.mouse_pressed,
        };
        zoom_hud::draw_with_sprites(
            renderer,
            zoom_layout,
            zoom_presentation.button_enable(),
            hover,
            zoom_sprites,
        );

        if let (Some(tooltip), Some(resources), Some(fonts)) =
            (zoom_presentation.ready_tooltip, menu_resources, hud_fonts)
        {
            let btn = tooltip.into();
            let text = resources
                .menu_text
                .get(zoom_hud::zoom_button_tooltip_mt_id(btn));
            if !text.is_empty() {
                let (cw, ch) = cursor_renderer.current_frame_size();
                crate::ui_panel::draw_screen_tooltip(
                    renderer,
                    &fonts.tooltip_font,
                    fonts.shadow_font.as_ref(),
                    &text,
                    zoom_mouse.x as i32,
                    zoom_mouse.y as i32,
                    (cw as i32, ch as i32),
                );
            }
        }
    }

    // Corner HUD buttons (Clock / Sight / QuickStart) — added to the
    // panel in non-Sherwood missions only.  Hidden entirely during
    // Sherwood, where the Sherwood HUD owns this real-estate.
    if !game.is_sherwood {
        let corner_enable = CornerButtonEnable::from_engine(engine);
        let mp = threaded_input.position();
        let hovered_btn = corner_layout.hit_test_geometric(mp.x as i32, mp.y as i32);
        let hover = CornerHoverState {
            hovered: hovered_btn,
            mouse_pressed: host.input.left_mouse_down,
        };
        corner_hud::draw_with_sprites(
            renderer,
            corner_layout,
            corner_enable,
            hover,
            corner_sprites,
        );

        // Stature (up/down arrow) widgets on the lower panel.  Driven
        // live off `EngineInner::retrieve_stature(None)` — we poll the
        // sim directly each frame.
        //
        // The focus-latch overlay (`with_focus_latch`) keeps the
        // initiating arrow visually pressed while the sim's stature
        // transition is running, and dims the opposite arrow.  The
        // latch is set when the player issues StandUp/CrouchDown
        // (keyboard or widget click) — see
        // `input_dispatch_stature_commands` below — and auto-clears
        // when the aggregate stature shifts.
        let stature = engine.retrieve_stature(None);
        let stature_enable =
            StatureEnable::from_stature(stature).with_focus_latch(game.stature_focus);
        let stature_geom_hovered = stature_layout.hit_test_geometric(mp.x as i32, mp.y as i32);
        let stature_hovered = stature_layout.hit_test(mp.x as i32, mp.y as i32, stature_enable);
        let stature_hover = StatureHoverState {
            hovered: stature_hovered,
            mouse_pressed: host.input.left_mouse_down,
        };
        stature_hud::draw_with_sprites(
            renderer,
            stature_layout,
            stature_enable,
            stature_hover,
            stature_sprites,
        );

        // Hover tooltip for the arrow widgets ("Crouch"/"Stand up").
        // Uses the geometric hit-test so the tooltip still appears
        // when the arrow is disabled (hover is tied to the widget
        // rect, not its enable state).
        stature_tooltip.update(stature_geom_hovered);
        if let (Some(resources), Some(fonts)) = (menu_resources, hud_fonts) {
            let (cw, ch) = cursor_renderer.current_frame_size();
            stature_hud::draw_tooltip(
                renderer,
                stature_tooltip,
                |btn| {
                    let mt_id = stature_hud::stature_button_tooltip_mt_id(btn);
                    resources.menu_text.get(mt_id)
                },
                &fonts.tooltip_font,
                fonts.shadow_font.as_ref(),
                mp.x as i32,
                mp.y as i32,
                (cw as i32, ch as i32),
            );
        }

        corner_tooltip.update(hovered_btn);
        if let (Some(resources), Some(fonts)) = (menu_resources, hud_fonts)
            && let Some(btn) = corner_tooltip.ready_button()
        {
            let mt_id = corner_hud::corner_button_tooltip_mt_id(btn);
            let text = resources.menu_text.get(mt_id);
            if !text.is_empty() {
                let (cw, ch) = cursor_renderer.current_frame_size();
                crate::ui_panel::draw_screen_tooltip(
                    renderer,
                    &fonts.tooltip_font,
                    fonts.shadow_font.as_ref(),
                    &text,
                    mp.x as i32,
                    mp.y as i32,
                    (cw as i32, ch as i32),
                );
            }
        }
    }

    // ── GPU phase: hovered-PC info popup (RHInfoPopup) ──
    {
        let mouse_pos = threaded_input.position();
        crate::ui_panel::draw_pc_info_overlay(
            host,
            engine,
            &assets.profile_manager,
            renderer,
            portrait_cache,
            mouse_pos,
        );
    }

    // ── Portrait action-button hover tooltip ──
    // Hero actions use localized game strings; allied controls describe
    // both the action and its current state/target. Only the selected
    // portrait shows action buttons, so
    // `hit_test_portrait_detailed` already gates on that.
    {
        let mp = threaded_input.position();
        let sw = renderer.screen_width();
        let sh = renderer.screen_height();
        let hovered_hit =
            hit_test_portrait_detailed(engine, local_seat, portrait_cache, sw, sh, mp.x, mp.y);
        let hovered_action_btn = hovered_hit.and_then(|hit| match hit.area {
            PortraitHitArea::ActionButton(btn) | PortraitHitArea::AlliedAction(btn) => {
                Some((hit.slot, btn))
            }
            PortraitHitArea::Pin if !matches!(hit.target, PortraitTarget::Pc(_)) => {
                Some((hit.slot, 3))
            }
            _ => None,
        });
        pc_action_tooltip.update(hovered_action_btn);
        if pc_action_tooltip.ready_button().is_some()
            && let (Some(hit), Some(fonts)) = (hovered_hit, hud_fonts)
        {
            let text = match hit.area {
                PortraitHitArea::AlliedAction(_) | PortraitHitArea::Pin => {
                    allied_portrait_tooltip(engine, local_seat, hit)
                }
                PortraitHitArea::ActionButton(btn) => {
                    let pc_id = match hit.target {
                        PortraitTarget::Pc(pc_id) => Some(pc_id),
                        _ => None,
                    };
                    pc_id
                        .and_then(|pc_id| engine.get_entity(pc_id))
                        .and_then(engine_element::Entity::pc_data)
                        .and_then(|pc| assets.profile_manager.get_character(pc.profile_index))
                        .and_then(|profile| profile.actions.get(btn as usize))
                        .and_then(|action| crate::ui_panel::action_button_tooltip_mt_id(*action))
                        .and_then(|mt_id| {
                            menu_resources.map(|resources| resources.menu_text.get(mt_id))
                        })
                        .unwrap_or_default()
                }
                _ => String::new(),
            };
            if !text.is_empty() {
                let (cw, ch) = cursor_renderer.current_frame_size();
                crate::ui_panel::draw_screen_tooltip(
                    renderer,
                    &fonts.tooltip_font,
                    fonts.shadow_font.as_ref(),
                    &text,
                    mp.x as i32,
                    mp.y as i32,
                    (cw as i32, ch as i32),
                );
            }
        }
    }

    // ── GPU phase: HUD text ──
    if let Some(fonts) = hud_fonts {
        crate::hud_text::render_hud_text(
            engine,
            local_seat,
            &host.viewport,
            assets,
            &host.draw_order.ids,
            portrait_cache,
            renderer,
            fonts,
        );

        // ── GPU phase: ransom / amulet counters ──
        // Renders ransom and amulet values in the top-left corner
        // with a drop-shadow background font.
        render_ransom_amulet_overlay(engine, renderer, fonts, menu_resources);

        // Dev-only EntityId overlay — draws each entity's ID under its
        // feet.  Driven by the `/screenshot?entity_ids` HTTP flag.
        if dev.debug.entity_ids {
            crate::hud_text::render_entity_id_overlay(engine, &host.viewport, renderer, fonts);
        }

        // Dev-only AI speech-log overlay — draws recent accepted
        // remarks as `(prefix) Remark` lines in a top-centred band.
        // Gated on `host.info_displayed`.
        if host.info_displayed {
            render_display_info_overlay(host, renderer, fonts, display_info_elapsed_secs);
            crate::hud_text::render_screen_remarks(engine, renderer, fonts);
        }

        // AI log dump for the selected NPC.  Logged via
        // `tracing::trace!` rather than rendered on-screen as titbits.
        engine.display_ai_log_for_selected(host.selected_view_element);

        // Transient centered-banner message driven by
        // `DisplayMessage` / `message_delay`.  Renders while the
        // delay is non-zero; main loop decrements after render.
        if ctx.game.message_delay > 0 && !ctx.game.message_text.is_empty() {
            crate::hud_text::render_transient_message(renderer, fonts, &ctx.game.message_text);
        }
    }

    // ── GPU phase: pause overlay ──
    if let (Some(menu), Some(resources)) = (pause_menu, menu_resources) {
        renderer.freeze_scene_for_modal();
        let briefings = Some(engine.short_briefings());
        // Look up each short briefing's localized string from the
        // briefing-text table that we pre-resolved into
        // `short_briefing_strings` after Level.res was attached.
        let text_lookup = |id: u32| -> Option<String> { short_briefing_strings.get(&id).cloned() };
        menu.render(renderer, resources, briefings, &text_lookup);
    }

    // ── GPU phase: console overlay ──
    // Drawn after the pause menu so the cheat console is reachable
    // even mid-pause — the console captures input independent of
    // the pause overlay.
    // Pump host-side deferred console output into the overlay's history
    // so those lines surface in the scrollback even though they
    // originate outside the dispatcher.
    drain_pending_console_output(console_overlay, host);
    console_overlay.tick_animation();
    if console_overlay.is_visible() {
        let console_font = menu_resources.and_then(|r| r.label_font_any());
        console_overlay.render(renderer, console_font);
    }

    // Mouse cursor selection + `PerformOrientation` dispatch is hoisted
    // into `update_mouse_and_cursor` (main loop, pre-render) so this
    // pass keeps `&Engine` immutable.  The cursor texture has already
    // been loaded by that point; `last_cursor_id` is the live id.

    // ── GPU phase: cursor on top of everything ──
    // The cursor blits first and advances its animation afterwards,
    // so the displayed frame this tick is the one chosen by the
    // previous tick's animation step.
    let (cursor_opacity, cursor_shadow_color) = if pause_menu.is_some() {
        (MOUSE_OPACITY_DEFAULT, 0)
    } else {
        (host.input.mouse_opacity, host.input.mouse_shadow_color)
    };
    cursor_renderer.render(
        renderer,
        threaded_input.position().x,
        threaded_input.position().y,
        cursor_opacity,
        cursor_shadow_color,
    );

    // The increment-cursor flag gates animation advancement.
    if host.input.increment_cursor_animation {
        cursor_renderer.advance_animation();
    }

    // ── GPU phase: rewind indicator ──
    // Transparent "◀◀" glyph in the top-right corner while
    // BACKSPACE is held, to make it obvious that the engine
    // isn't just glitching.
    if rewind_active {
        let sw = renderer.screen_width() as i32;
        draw_rewind_icon(renderer, sw - 100, 30, 40);
    }

    // ── Pixel-level fade (script opcode `FADE_TO_BLACK`) ──
    // Draw a full-screen black rect with alpha ramping up then
    // back down. The GPU alpha-blend
    // matches the channel × scale math closely enough for the
    // ellipsis effect used by cutscenes. Advancement happens only
    // after the live `present()`; this function is also used for
    // throwaway screenshot and thumbnail renders.
    if let Some(fade) = host.fade_to_black {
        let alpha = fade.current_alpha();
        if alpha > 0 {
            let sw = renderer.screen_width() as i32;
            let sh = renderer.screen_height() as i32;
            renderer.render_gpu_rect(0, 0, sw, sh, 0, 0, 0, alpha);
        }
    }

    // `renderer.present()` is called by the caller (`run_mission`) after
    // this function returns, so a `/screenshot` HTTP request can read
    // pixels from the composed offscreen target before `present()`
    // clears it.
}

/// Draw the rewind HUD indicator: two left-pointing triangles forming
/// a "◀◀" glyph, with a subtle dark backdrop rectangle behind them so
/// the icon reads against any scene.
///
/// `size` is the edge length of each triangle in pixels; the full
/// icon spans `2 × size` horizontally.
pub(super) fn draw_rewind_icon(
    renderer: &mut crate::renderer::Renderer,
    x: i32,
    y: i32,
    size: i32,
) {
    // Each chevron is narrower than it is tall — a slim triangle
    // reads as a playback-style "rewind" glyph more clearly than a
    // 1:1 equilateral.
    let width = (size as f32 * 0.65).round();
    let sz = size as f32;
    let total_width = (2.0 * width) as i32;
    // Semi-opaque dark backdrop for contrast.
    renderer.render_gpu_rect(x - 6, y - 4, total_width + 12, size + 8, 0, 0, 0, 140);
    for triangle_idx in 0..2 {
        let tri_x = x as f32 + triangle_idx as f32 * width;
        let ty = y as f32;
        renderer.render_gpu_triangle(
            [
                (tri_x, ty + sz / 2.0),   // apex (left, mid)
                (tri_x + width, ty),      // base top-right
                (tri_x + width, ty + sz), // base bottom-right
            ],
            255,
            255,
            255,
            220,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    #[test]
    fn print_screen_modifier_request_priority_matches_reference() {
        assert_eq!(
            print_screen_request_from_modifiers(false, false),
            PrintScreenRequest::Plain
        );
        assert_eq!(
            print_screen_request_from_modifiers(false, true),
            PrintScreenRequest::Median3x3
        );
        assert_eq!(
            print_screen_request_from_modifiers(true, false),
            PrintScreenRequest::WideSnapshot
        );
        assert_eq!(
            print_screen_request_from_modifiers(true, true),
            PrintScreenRequest::WideSnapshot
        );
    }

    #[test]
    fn median_filter_preserves_alpha_and_uses_channel_median() {
        let rgba = vec![
            0, 0, 0, 1, 10, 10, 10, 2, 20, 20, 20, 3, 30, 30, 30, 4, 250, 250, 250, 5, 50, 50, 50,
            6, 60, 60, 60, 7, 70, 70, 70, 8, 80, 80, 80, 9,
        ];
        let out = median_filter_rgba_3x3(3, 3, &rgba);
        let center = (3 + 1) * 4;
        assert_eq!(&out[center..center + 4], &[50, 50, 50, 5]);
    }

    #[test]
    fn authored_patrol_overlay_uses_runtime_waypoint_cursor() {
        let paths = vec![RawHikingPath {
            waypoints: vec![
                RawWaypoint {
                    x: 10,
                    y: 20,
                    sector: 1,
                    level: 0,
                    command: WaypointCommand::None,
                },
                RawWaypoint {
                    x: 30,
                    y: 40,
                    sector: 1,
                    level: 0,
                    command: WaypointCommand::None,
                },
            ],
        }];
        let path_id = PathId::new(0).unwrap();
        let mut patrol = PatrolPath::new(path_id, &paths).unwrap();
        patrol.current_waypoint_index = 1;

        let (resolved_id, route) = authored_patrol_route(
            Some(&patrol),
            &DetachedPatrolPathStatus::default(),
            true,
            &paths,
        )
        .unwrap();

        assert_eq!(resolved_id, path_id);
        assert_eq!(route.active_waypoint, 1);
        assert_eq!(
            route.points,
            vec![
                engine_coordinates::MapPoint::new(10.0, 20.0),
                engine_coordinates::MapPoint::new(30.0, 40.0),
            ]
        );
    }

    #[test]
    fn authored_patrol_overlay_supports_detached_mission_path_state() {
        let paths = vec![RawHikingPath {
            waypoints: vec![RawWaypoint {
                x: 50,
                y: 60,
                sector: 2,
                level: 1,
                command: WaypointCommand::None,
            }],
        }];
        let path_id = PathId::new(0).unwrap();
        let detached = DetachedPatrolPathStatus {
            hiking_path_index: Some(path_id),
            ..Default::default()
        };

        let (_, route) = authored_patrol_route(None, &detached, true, &paths).unwrap();

        assert_eq!(
            route.points[0],
            engine_coordinates::MapPoint::new(50.0, 60.0)
        );
        assert_eq!(route.active_waypoint, 0);
    }
}
