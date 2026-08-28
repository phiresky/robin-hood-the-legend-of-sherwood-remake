//! Main-menu "Show Credits" entry.
//!
//! Shows the credits background (`RHID_BK_CREDITS`) with the credits
//! roll (`RHID_CREDITS_PICTURE`) scrolling bottom-to-top on top of it.
//! Exits on Escape, Space, Return, or a left click.  Stops advancing
//! once the bottom of the roll lines up with the middle of the screen
//! (`offset + screen_h + ((768 - screen_h) / 2) < credit_height - 1`).
//!
//! Cursor: the main-menu's `CursorRenderer::init` already hides the OS
//! cursor at start-up, and the outer-loop `ModalCursor` stops rendering
//! for the duration of `show_credits`, so nothing draws over the scroll.

use crate::gfx_types::Keycode;
use robin_engine::sprite::BBox;

use crate::gfx_types::GameEvent;
use crate::host::ApplicationContext;
use crate::main_entry::picture_to_surface;
use crate::renderer::{BLIT_SOURCE_TRANSPARENT, Renderer};
use robin_assets::resource_manager::ResourceManager;
use robin_engine::resource_ids;

/// Show the credits scroll.  Returns once the player dismisses it.
pub(crate) async fn show_credits(
    application_context: &ApplicationContext,
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
) {
    // The original wraps the entire credits flow in a `sound_enabled`
    // guard — the sound-manager suspend/resume hooks around the roll
    // were removed from the shipping build but the guard stayed.
    // Faithfully reproduce it: `-NOSOUND` skips credits entirely.
    if !application_context.options().sound_enabled {
        tracing::debug!("Credits: sound_enabled is false (-NOSOUND) — skipping credits roll");
        return;
    }

    let shipping = application_context
        .shipping()
        .unwrap_or_else(|error| panic!("Credits lost its ApplicationContext: {error}"));
    let mut res = ResourceManager::new();
    if let Err(e) = res.attach_or_from_shipping("Data/Interface/DEFAULT.RES", shipping) {
        tracing::warn!("Credits: DEFAULT.RES unavailable ({e}) — skipping");
        return;
    }

    // Credits and background surfaces. The background would normally
    // be centered inside a screen-sized surface filled with black, but
    // since our renderer screen-blits already letterbox to the logical
    // menu size, a plain `picture_to_surface` is equivalent.
    let credits_surface = match res.get_picture(resource_ids::RHID_CREDITS_PICTURE, 0) {
        Ok(pic) => picture_to_surface(renderer, pic),
        Err(e) => {
            tracing::warn!("Credits: RHID_CREDITS_PICTURE unavailable ({e}) — skipping");
            return;
        }
    };
    let credit_width = renderer.surface_width(credits_surface) as i32;
    let credit_height = renderer.surface_height(credits_surface) as i32;

    let bg_surface = match res.get_picture(resource_ids::RHID_BK_CREDITS, 0) {
        Ok(pic) => Some(picture_to_surface(renderer, pic)),
        Err(e) => {
            tracing::info!("Credits: RHID_BK_CREDITS unavailable ({e}) — using plain black");
            None
        }
    };
    let bg_dims = bg_surface.map(|sid| {
        (
            renderer.surface_width(sid) as i32,
            renderer.surface_height(sid) as i32,
        )
    });

    let screen_w = renderer.screen_width() as i32;
    let screen_h = renderer.screen_height() as i32;
    let margin_x = ((screen_w - credit_width) / 2).max(0);

    // Start the offset at `-screen_h` so the roll enters from the
    // bottom of the screen, then increment by 1 per tick while the
    // guard below holds.
    let mut offset: i32 = -screen_h;
    let mut last_scroll_sample = web_time::Instant::now();
    let mut scroll_accumulated_us = 0_u64;

    loop {
        for event in event_pump.poll_events() {
            match event {
                // The original dismisses only on left-click or Escape.
                // `Quit` is treated as an implicit ESC since the
                // original game had no window-close path.
                GameEvent::Quit
                | GameEvent::KeyDown {
                    keycode: Keycode::Escape,
                    ..
                }
                | GameEvent::MouseDown(_, _, 1, _) => {
                    return;
                }
                _ => {}
            }
        }

        // ── Render ──
        // Background: fill with black, then blit the centered texture.
        renderer.begin_gpu_frame_clear();
        if let Some(bg) = bg_surface {
            let (bw, bh) = bg_dims.unwrap();
            let bx = (screen_w - bw) / 2;
            let by = (screen_h - bh) / 2;
            let src = BBox::from_coords(0.0, 0.0, bw as f32, bh as f32);
            let dst = BBox::from_coords(bx as f32, by as f32, (bx + bw) as f32, (by + bh) as f32);
            renderer.blit_to_screen(bg, Some(&src), Some(&dst), 0);
        }

        // Credits roll — three phases of the scroll: entering from
        // below, fully visible, and clipping off the top.
        if offset < 0 {
            // Entering the screen from the bottom.
            let dst_top = -offset;
            let dst_bottom = screen_h;
            let src_top = 0;
            let src_bottom = screen_h + offset; // = credit visible height so far
            if src_bottom > 0 {
                let src =
                    BBox::from_coords(0.0, src_top as f32, credit_width as f32, src_bottom as f32);
                let dst = BBox::from_coords(
                    margin_x as f32,
                    dst_top as f32,
                    (margin_x + credit_width) as f32,
                    dst_bottom as f32,
                );
                renderer.blit_with_shadow(
                    credits_surface,
                    Some(&src),
                    0,
                    Some(&dst),
                    0x1f,
                    50,
                    BLIT_SOURCE_TRANSPARENT,
                );
            }
        } else if offset + screen_h < credit_height {
            // Fully scrolling.
            let src = BBox::from_coords(
                0.0,
                offset as f32,
                credit_width as f32,
                (offset + screen_h) as f32,
            );
            let dst = BBox::from_coords(
                margin_x as f32,
                0.0,
                (margin_x + credit_width) as f32,
                screen_h as f32,
            );
            renderer.blit_with_shadow(
                credits_surface,
                Some(&src),
                0,
                Some(&dst),
                0x1f,
                50,
                BLIT_SOURCE_TRANSPARENT,
            );
        } else {
            // Tail — the bottom of the roll is within the screen.
            let remaining = credit_height - offset;
            if remaining > 0 {
                let src = BBox::from_coords(
                    0.0,
                    offset as f32,
                    credit_width as f32,
                    credit_height as f32,
                );
                let dst = BBox::from_coords(
                    margin_x as f32,
                    0.0,
                    (margin_x + credit_width) as f32,
                    remaining as f32,
                );
                renderer.blit_with_shadow(
                    credits_surface,
                    Some(&src),
                    0,
                    Some(&dst),
                    0x1f,
                    50,
                    BLIT_SOURCE_TRANSPARENT,
                );
            }
        }

        // Stop guard: keep advancing only until the roll's centred end
        // clears the midpoint of a 768-tall target surface.  The 768
        // literal is preserved so other resolutions hit the same scroll
        // stop point.
        if offset + screen_h + ((768 - screen_h) / 2) < credit_height - 1 {
            let now = web_time::Instant::now();
            scroll_accumulated_us = scroll_accumulated_us
                .saturating_add(now.duration_since(last_scroll_sample).as_micros() as u64);
            last_scroll_sample = now;
            let pixels = (scroll_accumulated_us / 20_000).min(i32::MAX as u64) as i32;
            scroll_accumulated_us %= 20_000;
            offset = offset.saturating_add(pixels);
        } else {
            last_scroll_sample = web_time::Instant::now();
            scroll_accumulated_us = 0;
        }

        renderer.flip();
        // Presentation follows the configured display cadence; scroll motion
        // above remains at the original 50 pixels/second wall-clock rate.
        crate::window::sleep_ui_frame().await;
    }
}
