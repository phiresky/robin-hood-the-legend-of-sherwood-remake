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
use robin_assets::shipping_datadir as assets_shipping_datadir;
use robin_engine::engine::GlobalOptions;
use robin_engine::sprite::BBox;

use crate::geo2d;
use crate::gfx_types::GameEvent;
use crate::main_entry::picture_to_surface;
use crate::renderer::{BLIT_SOURCE_TRANSPARENT, Renderer};
use crate::resource_ids;
use crate::resource_manager::ResourceManager;

/// Show the credits scroll.  Returns once the player dismisses it.
pub(crate) async fn show_credits(
    event_pump: &mut crate::window::GameWindow,
    renderer: &mut Renderer,
    shipping: Option<&assets_shipping_datadir::ShippingDatadir>,
) {
    // The original wraps the entire credits flow in a `sound_enabled`
    // guard — the sound-manager suspend/resume hooks around the roll
    // were removed from the shipping build but the guard stayed.
    // Faithfully reproduce it: `-NOSOUND` skips credits entirely.
    if !GlobalOptions::global()
        .as_ref()
        .is_none_or(|opts| opts.sound_enabled)
    {
        tracing::debug!("Credits: sound_enabled is false (-NOSOUND) — skipping credits roll");
        return;
    }

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

    loop {
        for event in event_pump.poll_events() {
            match event {
                // The original dismisses only on left-click or Escape.
                // SDL `Quit` is treated as an implicit ESC since the
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
            let src = BBox::new(geo2d::pt(0.0, 0.0), geo2d::pt(bw as f32, bh as f32));
            let dst = BBox::new(
                geo2d::pt(bx as f32, by as f32),
                geo2d::pt((bx + bw) as f32, (by + bh) as f32),
            );
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
                let src = BBox::new(
                    geo2d::pt(0.0, src_top as f32),
                    geo2d::pt(credit_width as f32, src_bottom as f32),
                );
                let dst = BBox::new(
                    geo2d::pt(margin_x as f32, dst_top as f32),
                    geo2d::pt((margin_x + credit_width) as f32, dst_bottom as f32),
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
            let src = BBox::new(
                geo2d::pt(0.0, offset as f32),
                geo2d::pt(credit_width as f32, (offset + screen_h) as f32),
            );
            let dst = BBox::new(
                geo2d::pt(margin_x as f32, 0.0),
                geo2d::pt((margin_x + credit_width) as f32, screen_h as f32),
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
                let src = BBox::new(
                    geo2d::pt(0.0, offset as f32),
                    geo2d::pt(credit_width as f32, credit_height as f32),
                );
                let dst = BBox::new(
                    geo2d::pt(margin_x as f32, 0.0),
                    geo2d::pt((margin_x + credit_width) as f32, remaining as f32),
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
            offset += 1;
        }

        renderer.flip();
        // 20 ms per frame keeps the reel speed in parity with the
        // original busy-wait.
        crate::window::sleep_ms(20).await;
    }
}
