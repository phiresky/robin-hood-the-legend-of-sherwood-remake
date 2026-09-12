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
    let files = match application_context.preparation_files() {
        Ok(files) => files.clone(),
        Err(error) => {
            tracing::error!(%error, "Credits unavailable without resource preparation authority");
            return;
        }
    };
    let mut res = ResourceManager::with_files(files);
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
    let (credit_width, credit_height) = renderer
        .surface_dimensions(credits_surface.handle())
        .expect("live credits upload");
    let (credit_width, credit_height) = (i32::from(credit_width), i32::from(credit_height));

    let bg_surface = match res.get_picture(resource_ids::RHID_BK_CREDITS, 0) {
        Ok(pic) => {
            let surface = picture_to_surface(renderer, pic);
            let (w, h) = renderer
                .surface_dimensions(surface.handle())
                .expect("live credits background");
            Some((surface, (i32::from(w), i32::from(h))))
        }
        Err(e) => {
            tracing::info!("Credits: RHID_BK_CREDITS unavailable ({e}) — using plain black");
            None
        }
    };
    // Both uploads own their GPU data; the source archive and decoded pictures
    // are no longer needed during the potentially long-running scroll.
    drop(res);

    let mut state = CreditsModalState::new(renderer.screen_height() as i32);
    while !state.tick(
        event_pump,
        renderer,
        &credits_surface,
        bg_surface.as_ref(),
        (credit_width, credit_height),
    ) {
        crate::window::sleep_ui_frame().await;
    }
    retire_uploads(
        renderer,
        credits_surface,
        bg_surface.map(|(surface, _)| surface),
    );
}

/// Runtime clock and scroll position belong to this one credits presentation.
struct CreditsModalState {
    offset: i32,
    last_scroll_sample: web_time::Instant,
    scroll_accumulated_us: u64,
}

impl CreditsModalState {
    fn new(screen_height: i32) -> Self {
        Self {
            offset: -screen_height,
            last_scroll_sample: web_time::Instant::now(),
            scroll_accumulated_us: 0,
        }
    }

    fn tick(
        &mut self,
        event_pump: &mut crate::window::GameWindow,
        renderer: &mut Renderer,
        credits_surface: &crate::renderer::OwnedSurface,
        bg_surface: Option<&(crate::renderer::OwnedSurface, (i32, i32))>,
        (credit_width, credit_height): (i32, i32),
    ) -> bool {
        let events = event_pump.poll_events();
        renderer.sync_window_size(event_pump);
        for event in events {
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
                    return true;
                }
                _ => {}
            }
        }

        let screen_w = renderer.screen_width() as i32;
        let screen_h = renderer.screen_height() as i32;

        // ── Render ──
        // Background: fill with black, then blit the centered texture.
        renderer.begin_gpu_frame_clear();
        renderer.begin_ui_only_frame();
        if let Some((bg, dimensions)) = bg_surface {
            let (bw, bh) = *dimensions;
            let bx = (screen_w - bw) / 2;
            let by = (screen_h - bh) / 2;
            let src = BBox::from_coords(0.0, 0.0, bw as f32, bh as f32);
            let dst = BBox::from_coords(bx as f32, by as f32, (bx + bw) as f32, (by + bh) as f32);
            renderer
                .draw_surface(bg.handle(), Some(&src), Some(&dst), 0)
                .expect("live credits background");
        }

        if let Some((src, dst)) = credits_rectangles(
            self.offset,
            (credit_width, credit_height),
            (screen_w, screen_h),
        ) {
            renderer
                .draw_surface_with_shadow(
                    credits_surface.handle(),
                    Some(&src),
                    Some(&dst),
                    50,
                    BLIT_SOURCE_TRANSPARENT,
                )
                .expect("live credits upload");
        }

        self.advance_scroll(screen_h, credit_height, web_time::Instant::now());

        renderer.flip();
        // Presentation follows the configured display cadence; scroll motion
        // above remains at the original 50 pixels/second wall-clock rate.
        false
    }

    fn advance_scroll(&mut self, screen_h: i32, credit_height: i32, now: web_time::Instant) {
        // Stop guard: keep advancing only until the roll's centred end
        // clears the midpoint of a 768-tall target surface.  The 768
        // literal is preserved so other resolutions hit the same scroll
        // stop point.
        if self.offset + screen_h + ((768 - screen_h) / 2) < credit_height - 1 {
            self.scroll_accumulated_us = self
                .scroll_accumulated_us
                .saturating_add(now.duration_since(self.last_scroll_sample).as_micros() as u64);
            self.last_scroll_sample = now;
            let pixels = (self.scroll_accumulated_us / 20_000).min(i32::MAX as u64) as i32;
            self.scroll_accumulated_us %= 20_000;
            self.offset = self.offset.saturating_add(pixels);
        } else {
            self.last_scroll_sample = now;
            self.scroll_accumulated_us = 0;
        }
    }
}

#[test]
fn credits_scroll_state_retains_fractional_time_and_discards_stopped_time() {
    let mut state = CreditsModalState::new(480);
    let start = state.last_scroll_sample;
    let sample = |micros| start + std::time::Duration::from_micros(micros);
    state.advance_scroll(480, 1000, sample(19_999));
    assert_eq!(state.offset, -480);
    state.advance_scroll(480, 1000, sample(40_001));
    assert_eq!(state.offset, -478);
    assert_eq!(state.scroll_accumulated_us, 1);

    // The original 768-high centering guard stops this roll at offset 375.
    state.offset = 375;
    state.advance_scroll(480, 1000, sample(1_000_000));
    assert_eq!(state.offset, 375);
    assert_eq!(state.scroll_accumulated_us, 0);
    // If a resize permits scrolling again, stopped wall time is not replayed.
    state.advance_scroll(400, 1000, sample(1_020_000));
    assert_eq!(state.offset, 376);
    assert_eq!(state.scroll_accumulated_us, 0);
}

/// Preserve the entering, full-screen, and trailing phases of the legacy roll.
fn credits_rectangles(
    offset: i32,
    (credit_width, credit_height): (i32, i32),
    (screen_width, screen_height): (i32, i32),
) -> Option<(BBox, BBox)> {
    let margin_x = ((screen_width - credit_width) / 2).max(0);
    let (src_top, src_bottom, dst_top, dst_bottom) = if offset < 0 {
        let visible = screen_height + offset;
        if visible <= 0 {
            return None;
        }
        (0, visible, -offset, screen_height)
    } else if offset + screen_height < credit_height {
        (offset, offset + screen_height, 0, screen_height)
    } else {
        let remaining = credit_height - offset;
        if remaining <= 0 {
            return None;
        }
        (offset, credit_height, 0, remaining)
    };
    Some((
        BBox::from_coords(0.0, src_top as f32, credit_width as f32, src_bottom as f32),
        BBox::from_coords(
            margin_x as f32,
            dst_top as f32,
            (margin_x + credit_width) as f32,
            dst_bottom as f32,
        ),
    ))
}

#[test]
fn credits_rectangles_preserve_scroll_phase_boundaries() {
    let coords = |rect: BBox| [rect.min.x, rect.min.y, rect.max.x, rect.max.y];
    assert!(credits_rectangles(-481, (200, 1000), (640, 480)).is_none());
    assert!(credits_rectangles(-480, (200, 1000), (640, 480)).is_none());
    for (offset, source_y, destination_y) in [
        (-479, [0.0, 1.0], [479.0, 480.0]),
        (-1, [0.0, 479.0], [1.0, 480.0]),
        (0, [0.0, 480.0], [0.0, 480.0]),
        (519, [519.0, 999.0], [0.0, 480.0]),
        (520, [520.0, 1000.0], [0.0, 480.0]),
        (999, [999.0, 1000.0], [0.0, 1.0]),
    ] {
        let (src, dst) = credits_rectangles(offset, (200, 1000), (640, 480)).unwrap();
        assert_eq!(coords(src), [0.0, source_y[0], 200.0, source_y[1]]);
        assert_eq!(
            coords(dst),
            [220.0, destination_y[0], 420.0, destination_y[1]]
        );
    }
    assert!(credits_rectangles(1000, (200, 1000), (640, 480)).is_none());
    let (src, dst) = credits_rectangles(0, (200, 100), (100, 480)).unwrap();
    assert_eq!(coords(src), [0.0, 0.0, 200.0, 100.0]);
    assert_eq!(coords(dst), [0.0, 0.0, 200.0, 100.0]);
}

fn retire_uploads(
    renderer: &mut Renderer,
    credits: crate::renderer::OwnedSurface,
    background: Option<crate::renderer::OwnedSurface>,
) {
    renderer.retire_surface(credits);
    if let Some(background) = background {
        renderer.retire_surface(background);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn verify_gpu_retirement(renderer: &mut Renderer) {
    for with_background in [false, true] {
        let credits = renderer.upload_rgb565(1, 1, &[0xffff]).unwrap();
        let handle = credits.handle();
        let background = with_background.then(|| renderer.upload_rgb565(1, 1, &[0xffff]).unwrap());
        let background_handle = background.as_ref().map(|upload| upload.handle());
        renderer
            .draw_surface_with_shadow(handle, None, None, 50, BLIT_SOURCE_TRANSPARENT)
            .unwrap();
        retire_uploads(renderer, credits, background);
        assert!(renderer.surface_dimensions(handle).is_err());
        if let Some(handle) = background_handle {
            assert!(renderer.surface_dimensions(handle).is_err());
        }
        assert_eq!(
            &renderer.try_capture_frame_rgba().unwrap().2[..4],
            &[248, 252, 248, 255]
        );
    }
}
