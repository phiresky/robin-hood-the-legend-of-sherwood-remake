//! Zoom HUD buttons (zoom-in / zoom-out).
//!
//! The two zoom widgets sit on the top-right parchment scroll.  Their enable mask
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

use crate::gfx_types::{Point, Rect as ScreenRect};
use robin_engine::engine as engine_api;
#[cfg(test)]
use robin_engine::player_command::PlayerCommand;

use crate::ingame_menu::layout::button_sprite_state;
use crate::renderer::Renderer;
use robin_engine::resource_ids::{RHID_ZOOM_DOWN, RHID_ZOOM_UP};

/// Logical zoom button id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomButton {
    /// Zoom in (increase zoom factor).
    ZoomUp,
    /// Zoom out (decrease zoom factor).
    ZoomDown,
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
    pub fn from_engine(engine: &engine_api::PresentationView<'_>) -> Self {
        let gated = engine.is_zoom_possible();
        let zoom_up_in_progress = engine.is_zoom_up_in_progress();
        let zoom_down_in_progress = engine.is_zoom_down_in_progress();
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
    pub zoom_up: ScreenRect,
    pub zoom_down: ScreenRect,
}

impl ZoomHudLayout {
    /// Derive button rects from the current screen width and the
    /// loaded sprite dimensions.
    ///
    /// Both widgets sit at `x = width - 26`, with zoom-in at screen
    /// `y = 0` and zoom-out at `y = 46`.  Coordinates are
    /// screen-absolute — the zoom buttons live on the top-right
    /// parchment scroll, not the lower panel.  Hit-box sizes follow
    /// the BTTN sprite dimensions when available; when the resource is
    /// missing we fall back to a 24x24 hit box; drawing still skips the
    /// missing sprite.
    pub fn for_screen_width(screen_w: u32, sprites: &ZoomButtonSprites) -> Self {
        const FALLBACK_W: u32 = 24;
        const FALLBACK_H: u32 = 24;

        let sw = screen_w as i32;

        let x = sw - 26;
        let zoom_up_y = 0;
        let zoom_down_y = 46;

        let (up_w, up_h) = sprites
            .size(ZoomButton::ZoomUp)
            .unwrap_or((FALLBACK_W as u16, FALLBACK_H as u16));
        let (down_w, down_h) = sprites
            .size(ZoomButton::ZoomDown)
            .unwrap_or((FALLBACK_W as u16, FALLBACK_H as u16));

        Self {
            zoom_up: ScreenRect::new(x, zoom_up_y, up_w as u32, up_h as u32),
            zoom_down: ScreenRect::new(x, zoom_down_y, down_w as u32, down_h as u32),
        }
    }

    /// Hit-test a screen-space click.  Returns the first matching
    /// button that is currently enabled, or `None`.
    pub fn hit_test(&self, x: i32, y: i32, enable: ZoomButtonEnable) -> Option<ZoomButton> {
        let pt = Point::new(x, y);
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
        self.hit_test(
            x,
            y,
            ZoomButtonEnable {
                zoom_up: true,
                zoom_down: true,
                ..Default::default()
            },
        )
    }
}

/// One loaded BTTN sprite frame: surface id plus native pixel size.
pub type ZoomButtonSprites = crate::hud_sprite::ButtonSprites<ZoomButton, 2>;
pub type ZoomHoverState = crate::hud_sprite::HoverState<ZoomButton>;

impl crate::hud_sprite::HudButton<2> for ZoomButton {
    const ALL: [Self; 2] = [Self::ZoomUp, Self::ZoomDown];
    fn index(self) -> usize {
        self as usize
    }
    fn resource(self) -> (i32, &'static str) {
        match self {
            Self::ZoomUp => (RHID_ZOOM_UP, "ZoomUp"),
            Self::ZoomDown => (RHID_ZOOM_DOWN, "ZoomDown"),
        }
    }
}

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

        sprites.draw(renderer, btn, *rect, state, 0);
    }
}

pub type ZoomTooltipTracker = crate::hud_sprite::ButtonTooltipTracker<ZoomButton>;

/// Menu-text id for the tooltip attached to the given zoom button.
pub fn zoom_button_tooltip_mt_id(btn: ZoomButton) -> usize {
    use crate::ingame_menu::resources::{MT_INFOBULLE_ZOOMIN, MT_INFOBULLE_ZOOMOUT};
    match btn {
        ZoomButton::ZoomUp => MT_INFOBULLE_ZOOMIN,
        ZoomButton::ZoomDown => MT_INFOBULLE_ZOOMOUT,
    }
}

pub use crate::hud_sprite::{TooltipPlacement, draw_tooltip};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_top_right_of_screen() {
        let sprites = ZoomButtonSprites::default();
        let layout = ZoomHudLayout::for_screen_width(800, &sprites);
        assert_eq!(layout.zoom_up.x(), 800 - 26);
        assert_eq!(layout.zoom_down.x(), 800 - 26);
        assert_eq!(layout.zoom_up.y(), 0);
        assert_eq!(layout.zoom_down.y(), 46);
    }

    #[test]
    fn hit_test_respects_enable() {
        let sprites = ZoomButtonSprites::default();
        let layout = ZoomHudLayout::for_screen_width(800, &sprites);
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
        use robin_engine::campaign::Campaign;
        use robin_engine::engine::{EngineStateRequest, LevelAssets};

        let mut assets = LevelAssets::new();
        let mut engine = engine_api::Engine::new_for_test_with_level_size(
            1024.0,
            768.0,
            Campaign::default(),
            &mut assets,
            4096.0,
            4096.0,
        )
        .expect("engine");

        // Idle state: both directions available at zoom_factor = 1.0.
        let mask = ZoomButtonEnable::from_engine(&engine.presentation_view());
        assert!(mask.zoom_up);
        assert!(mask.zoom_down);
        assert!(!mask.selected_up);
        assert!(!mask.selected_down);

        // Kick off a zoom-up transition — `is_zoom_possible` flips
        // false for the duration. The active direction stays enabled
        // + latched to selected; the inactive direction disables.
        engine
            .advance_frame(
                &assets,
                engine_api::SimulationFrameInput::new(vec![
                    PlayerCommand::ChangeState(EngineStateRequest::ZoomingUp).into(),
                ])
                .with_hourglass(false),
            )
            .expect("zoom command admission");
        let mask = ZoomButtonEnable::from_engine(&engine.presentation_view());
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

#[cfg(test)]
mod hit_order_tests {
    use super::*;

    #[test]
    fn geometric_hits_ignore_enable_state_but_preserve_overlap_priority() {
        let rect = ScreenRect::new(0, 0, 10, 10);
        let layout = ZoomHudLayout {
            zoom_up: rect,
            zoom_down: rect,
        };
        assert_eq!(layout.hit_test_geometric(1, 1), Some(ZoomButton::ZoomUp));
        assert_eq!(layout.hit_test(1, 1, ZoomButtonEnable::default()), None);
        let last_only = ZoomButtonEnable {
            zoom_down: true,
            ..Default::default()
        };
        assert_eq!(layout.hit_test(1, 1, last_only), Some(ZoomButton::ZoomDown));
        assert_eq!(layout.hit_test_geometric(10, 1), None);
        assert_eq!(layout.hit_test_geometric(1, 10), None);
    }
}
