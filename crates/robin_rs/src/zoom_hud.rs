//! Zoom HUD buttons (zoom-in / zoom-out).
//!
//! The two zoom widgets sit on the lower panel.  Their enable mask
//! comes from `Engine::is_zoom_up_possible` / `is_zoom_down_possible`
//! and an in-flight transition pins the active widget to
//! visually-pressed for the duration of the zoom animation.  Every
//! one of those states is derived directly from the engine queries
//! each frame.
//!
//! Button sprites come from the `RHID_ZOOM_UP` / `RHID_ZOOM_DOWN`
//! BTTN resources.  The four sub-ids encode interaction state:
//! 0 = disabled, 1 = normal, 2 = focused/selected, 3 = pressed; we
//! fall back to the normal frame when a specific state frame is
//! missing in the resource pack.

use crate::gfx_types::Rect as SdlRect;

use crate::ingame_menu::layout::{
    BTN_STATE_DISABLED, BTN_STATE_HOVER, BTN_STATE_NORMAL, BTN_STATE_PRESSED, button_sprite_state,
};
use crate::native_font::NativeFont;
use crate::renderer::{BLIT_SOURCE_TRANSPARENT, Renderer};
use crate::resource_ids::{RHID_ZOOM_DOWN, RHID_ZOOM_UP};
use crate::resource_manager::ResourceManager;

/// Logical zoom button id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomButton {
    /// Zoom in (increase zoom factor).
    ZoomUp,
    /// Zoom out (decrease zoom factor).
    ZoomDown,
}

impl ZoomButton {
    /// Stable slot index used by the tooltip tracker.
    fn index(self) -> usize {
        match self {
            ZoomButton::ZoomUp => 0,
            ZoomButton::ZoomDown => 1,
        }
    }
}

/// Which zoom buttons are interactable this frame.
///
/// Derived from `Engine::is_zoom_possible`, `is_zoom_up_possible`
/// and `is_zoom_down_possible`. `selected_*` is set while a zoom
/// transition is in flight — the active widget reads enabled +
/// selected for the duration of the animation.
#[derive(Debug, Clone, Copy, Default)]
pub struct ZoomButtonEnable {
    pub zoom_up: bool,
    pub zoom_down: bool,
    pub selected_up: bool,
    pub selected_down: bool,
}

impl ZoomButtonEnable {
    /// Snapshot the current enable mask from the engine's zoom-state
    /// queries.  A transition-in-progress clears both buttons via
    /// `is_zoom_possible`; we then re-open the specific direction
    /// that's active so its widget stays visually "pressed" for the
    /// duration.
    pub fn from_engine(
        engine: &robin_engine::engine::Engine,
        display: &robin_engine::engine::HostDisplayState,
    ) -> Self {
        let gated = engine.is_zoom_possible(display);
        let zoom_up_in_progress = engine.is_zoom_up_in_progress(display);
        let zoom_down_in_progress = engine.is_zoom_down_in_progress(display);
        Self {
            zoom_up: (gated && engine.is_zoom_up_possible()) || zoom_up_in_progress,
            zoom_down: (gated && engine.is_zoom_down_possible()) || zoom_down_in_progress,
            selected_up: zoom_up_in_progress,
            selected_down: zoom_down_in_progress,
        }
    }

    fn for_button(self, btn: ZoomButton) -> (bool, bool) {
        match btn {
            ZoomButton::ZoomUp => (self.zoom_up, self.selected_up),
            ZoomButton::ZoomDown => (self.zoom_down, self.selected_down),
        }
    }
}

/// Screen-space bounding boxes for the two zoom buttons.
#[derive(Debug, Clone, Copy)]
pub struct ZoomHudLayout {
    pub zoom_up: SdlRect,
    pub zoom_down: SdlRect,
}

impl ZoomHudLayout {
    /// Derive button rects from the current screen resolution and the
    /// loaded sprite dimensions.
    ///
    /// Both widgets sit at `x = width - 26`, with zoom-in at screen
    /// `y = 0` and zoom-out at `y = 46`.  Coordinates are
    /// screen-absolute — the zoom buttons live on the top-right
    /// parchment scroll, not the lower panel.  Hit-box sizes follow
    /// the BTTN sprite dimensions when available; when the resource is
    /// missing we fall back to a 24x24 hit box; drawing still skips the
    /// missing sprite.
    pub fn for_resolution(screen_w: u32, _screen_h: u32, sprites: &ZoomButtonSprites) -> Self {
        const FALLBACK_W: u32 = 24;
        const FALLBACK_H: u32 = 24;

        let sw = screen_w as i32;

        let x = sw - 26;
        let zoom_up_y = 0;
        let zoom_down_y = 46;

        let (up_w, up_h) = sprites
            .zoom_up_size()
            .unwrap_or((FALLBACK_W as u16, FALLBACK_H as u16));
        let (down_w, down_h) = sprites
            .zoom_down_size()
            .unwrap_or((FALLBACK_W as u16, FALLBACK_H as u16));

        Self {
            zoom_up: SdlRect::new(x, zoom_up_y, up_w as u32, up_h as u32),
            zoom_down: SdlRect::new(x, zoom_down_y, down_w as u32, down_h as u32),
        }
    }

    /// Hit-test a screen-space click.  Returns the first matching
    /// button that is currently enabled, or `None`.
    pub fn hit_test(&self, x: i32, y: i32, enable: ZoomButtonEnable) -> Option<ZoomButton> {
        let pt = crate::gfx_types::Point::new(x, y);
        if enable.zoom_up && self.zoom_up.contains_point(pt) {
            return Some(ZoomButton::ZoomUp);
        }
        if enable.zoom_down && self.zoom_down.contains_point(pt) {
            return Some(ZoomButton::ZoomDown);
        }
        None
    }

    /// Purely geometric hit-test — ignores the enable mask.  Used by
    /// the hover tracker so the tooltip still shows when the button is
    /// disabled (widgets own their tooltip independently of their
    /// enable state).
    pub fn hit_test_geometric(&self, x: i32, y: i32) -> Option<ZoomButton> {
        let pt = crate::gfx_types::Point::new(x, y);
        if self.zoom_up.contains_point(pt) {
            return Some(ZoomButton::ZoomUp);
        }
        if self.zoom_down.contains_point(pt) {
            return Some(ZoomButton::ZoomDown);
        }
        None
    }
}

/// One loaded BTTN sprite frame: surface id plus native pixel size.
type SpriteFrame = (u32, u16, u16);

/// Cached sprite surface ids for the two zoom HUD buttons.
///
/// Each button owns up to four sub-ids matching the
/// `BTN_STATE_DISABLED / NORMAL / HOVER / PRESSED` indices.  Missing
/// sub-ids fall back to the normal frame at draw time; if even normal
/// is absent `draw_with_sprites` skips the button entirely (no
/// fallback rect — see the note in `draw_with_sprites`).
#[derive(Debug, Default)]
pub struct ZoomButtonSprites {
    pub zoom_up: [Option<SpriteFrame>; 4],
    pub zoom_down: [Option<SpriteFrame>; 4],
}

impl ZoomButtonSprites {
    /// Load button sprites from the attached DEFAULT.RES.  Walks
    /// sub-ids 0..=3 per resource; a missing sub-id is stored as
    /// `None` and recovered via [`ZoomButtonSprites::frame`].
    pub fn load(res: &mut ResourceManager, renderer: &mut Renderer) -> Self {
        fn fetch_frame(
            res: &mut ResourceManager,
            renderer: &mut Renderer,
            id: i32,
            sub: usize,
        ) -> Option<SpriteFrame> {
            let pic = res.get_picture(id, sub).ok()?;
            let w = pic.width;
            let h = pic.height;
            let surface = crate::ui_panel::pic_to_surface(renderer, pic);
            Some((surface, w, h))
        }
        fn fetch_all(
            res: &mut ResourceManager,
            renderer: &mut Renderer,
            id: i32,
        ) -> [Option<SpriteFrame>; 4] {
            [
                fetch_frame(res, renderer, id, 0),
                fetch_frame(res, renderer, id, 1),
                fetch_frame(res, renderer, id, 2),
                fetch_frame(res, renderer, id, 3),
            ]
        }

        Self {
            zoom_up: fetch_all(res, renderer, RHID_ZOOM_UP),
            zoom_down: fetch_all(res, renderer, RHID_ZOOM_DOWN),
        }
    }

    fn frames(&self, btn: ZoomButton) -> &[Option<SpriteFrame>; 4] {
        match btn {
            ZoomButton::ZoomUp => &self.zoom_up,
            ZoomButton::ZoomDown => &self.zoom_down,
        }
    }

    /// The sprite actually rendered for a given interaction state,
    /// with a fallback to the normal frame if the requested state
    /// frame is absent.
    fn frame(&self, btn: ZoomButton, state: usize) -> Option<SpriteFrame> {
        let frames = self.frames(btn);
        frames[state].or(frames[BTN_STATE_NORMAL])
    }

    /// Native size of the zoom-up button's normal frame, used to size
    /// the hit rect.
    pub fn zoom_up_size(&self) -> Option<(u16, u16)> {
        Self::size_of(&self.zoom_up)
    }

    /// Companion to [`Self::zoom_up_size`].
    pub fn zoom_down_size(&self) -> Option<(u16, u16)> {
        Self::size_of(&self.zoom_down)
    }

    fn size_of(frames: &[Option<SpriteFrame>; 4]) -> Option<(u16, u16)> {
        frames[BTN_STATE_NORMAL]
            .or(frames[BTN_STATE_HOVER])
            .or(frames[BTN_STATE_PRESSED])
            .or(frames[BTN_STATE_DISABLED])
            .map(|(_, w, h)| (w, h))
    }
}

/// Transient per-frame input snapshot consumed by the draw routine —
/// which button is under the cursor, whether the left button is held.
#[derive(Debug, Clone, Copy, Default)]
pub struct ZoomHoverState {
    pub hovered: Option<ZoomButton>,
    pub mouse_pressed: bool,
}

/// Draw the zoom HUD buttons with state-aware sprite selection.  When
/// the resource pack is missing a frame entirely the button is simply
/// skipped — see the inline note below for why there's no fallback
/// rect.
pub fn draw_with_sprites(
    renderer: &mut Renderer,
    layout: &ZoomHudLayout,
    enable: ZoomButtonEnable,
    hover: ZoomHoverState,
    sprites: &ZoomButtonSprites,
) {
    let buttons = [
        (&layout.zoom_up, ZoomButton::ZoomUp),
        (&layout.zoom_down, ZoomButton::ZoomDown),
    ];

    for (rect, btn) in buttons {
        let (enabled, selected) = enable.for_button(btn);
        let hovered = hover.hovered == Some(btn);
        // Selected (zoom-in-progress) takes priority over pressed so
        // the widget reads visually as locked-down for the full
        // transition.
        let pressed = selected || (hovered && hover.mouse_pressed && enabled);
        let state = button_sprite_state(enabled, hovered || selected, pressed);

        if let Some((sid, _sw, _sh)) = sprites.frame(btn, state) {
            let dst = robin_engine::sprite::BBox::new(
                crate::geo2d::Point2D {
                    x: rect.x() as f32,
                    y: rect.y() as f32,
                },
                crate::geo2d::Point2D {
                    x: (rect.x() + rect.width() as i32) as f32,
                    y: (rect.y() + rect.height() as i32) as f32,
                },
            );
            // C++ zoom widgets use SBUIRendererBitmap, not the shadow
            // renderer, so shadow-key pixels are treated by the normal
            // transparent blit path.
            renderer.blit_to_screen(sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
        }
        // No placeholder-rect fallback — if a zoom sprite is missing
        // from DEFAULT.RES we simply don't draw the button.  The old
        // fallback painted a dark rectangle with a "Z+" / "Z-" label
        // that surfaced as a visible "black box" in release builds.
    }
}

/// Hover tracker for the zoom button tooltips.  Thin wrapper around the
/// shared `RequirementsTooltipTracker` that keys on a `ZoomButton` slot
/// index (0 = up, 1 = down).  See `RequirementsTooltipTracker` docs
/// for the delay semantics.
#[derive(Default, Clone)]
pub struct ZoomTooltipTracker {
    inner: crate::ui_panel::RequirementsTooltipTracker,
}

impl ZoomTooltipTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, hovered: Option<ZoomButton>) {
        self.inner.update(hovered.map(ZoomButton::index));
    }

    /// Returns the zoom button whose tooltip is currently ready to
    /// paint, or `None` when the hover hasn't crossed the idle
    /// threshold yet.
    pub fn ready_button(&self) -> Option<ZoomButton> {
        self.inner.ready_slot().and_then(|i| match i {
            0 => Some(ZoomButton::ZoomUp),
            1 => Some(ZoomButton::ZoomDown),
            _ => None,
        })
    }
}

/// Menu-text id for the tooltip attached to the given zoom button.
pub fn zoom_button_tooltip_mt_id(btn: ZoomButton) -> usize {
    use crate::ingame_menu::resources::{MT_INFOBULLE_ZOOMIN, MT_INFOBULLE_ZOOMOUT};
    match btn {
        ZoomButton::ZoomUp => MT_INFOBULLE_ZOOMIN,
        ZoomButton::ZoomDown => MT_INFOBULLE_ZOOMOUT,
    }
}

/// Draw the hover tooltip for the zoom HUD buttons.  Does nothing when
/// no tooltip is ready.  Uses the shared HUD tooltip font + the
/// anchor pipeline via `ui_panel::draw_screen_tooltip`.
#[allow(clippy::too_many_arguments)]
pub fn draw_tooltip(
    renderer: &mut Renderer,
    tracker: &ZoomTooltipTracker,
    tooltip_text: impl Fn(ZoomButton) -> String,
    font: &NativeFont,
    shadow: Option<&NativeFont>,
    mouse_x: i32,
    mouse_y: i32,
    cursor_size: (i32, i32),
) {
    if let Some(btn) = tracker.ready_button() {
        let text = tooltip_text(btn);
        if text.is_empty() {
            return;
        }
        crate::ui_panel::draw_screen_tooltip(
            renderer,
            font,
            shadow,
            &text,
            mouse_x,
            mouse_y,
            cursor_size,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_top_right_of_screen() {
        let sprites = ZoomButtonSprites::default();
        let layout = ZoomHudLayout::for_resolution(800, 600, &sprites);
        assert_eq!(layout.zoom_up.x(), 800 - 26);
        assert_eq!(layout.zoom_down.x(), 800 - 26);
        assert_eq!(layout.zoom_up.y(), 0);
        assert_eq!(layout.zoom_down.y(), 46);
    }

    #[test]
    fn hit_test_respects_enable() {
        let sprites = ZoomButtonSprites::default();
        let layout = ZoomHudLayout::for_resolution(800, 600, &sprites);
        let pt = (layout.zoom_up.x() + 1, layout.zoom_up.y() + 1);
        let both = ZoomButtonEnable {
            zoom_up: true,
            zoom_down: true,
            selected_up: false,
            selected_down: false,
        };
        assert_eq!(layout.hit_test(pt.0, pt.1, both), Some(ZoomButton::ZoomUp));
        let neither = ZoomButtonEnable::default();
        assert_eq!(layout.hit_test(pt.0, pt.1, neither), None);
    }

    #[test]
    fn enable_mask_gates_on_is_zoom_possible() {
        use crate::campaign::Campaign;
        use robin_engine::engine::{EngineStateRequest, HostDisplayState, InputState, LevelAssets};
        use robin_engine::player_command::PlayerCommand;

        let mut assets = LevelAssets::new();
        let mut engine = robin_engine::engine::Engine::new_for_test_with_level_size(
            1024.0,
            768.0,
            Campaign::default(),
            &mut assets,
            4096.0,
            4096.0,
        )
        .expect("engine");
        let mut display = HostDisplayState::default();

        // Idle state: both directions available at zoom_factor = 1.0.
        let mask = ZoomButtonEnable::from_engine(&engine, &display);
        assert!(mask.zoom_up);
        assert!(mask.zoom_down);
        assert!(!mask.selected_up);
        assert!(!mask.selected_down);

        // Kick off a zoom-up transition — `is_zoom_possible` flips
        // false for the duration. The active direction stays enabled
        // + latched to selected; the inactive direction disables.
        let mut input = InputState::default();
        engine.apply_command(
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::ChangeState(EngineStateRequest::ZoomingUp),
        );
        let mask = ZoomButtonEnable::from_engine(&engine, &display);
        assert!(mask.zoom_up);
        assert!(mask.selected_up);
        assert!(!mask.zoom_down);
        assert!(!mask.selected_down);
    }

    #[test]
    fn tooltip_tracker_round_trips_buttons() {
        let mut t = ZoomTooltipTracker::new();
        // Hover needs to outlast the idle threshold before the
        // tooltip is ready — 76 frames (delay is "strictly greater").
        for _ in 0..80 {
            t.update(Some(ZoomButton::ZoomUp));
        }
        assert_eq!(t.ready_button(), Some(ZoomButton::ZoomUp));
        // Switching targets resets the timer.
        t.update(Some(ZoomButton::ZoomDown));
        assert_eq!(t.ready_button(), None);
    }
}
