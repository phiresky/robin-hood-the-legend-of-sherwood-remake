//! Top-of-panel HUD buttons for non-Sherwood missions: Clock (record QA),
//! Sight (view-cone lock), QuickStart (launch all QAs).
//!
//! Click handlers forward messenger events:
//!
//! * Clock (single + double click): gated on a PC being selected — if not
//!   recording, picks an empty QA slot and forwards
//!   `MSG_START_RECORDING_MACRO`; if recording, rotates to the next slot
//!   via `MSG_CHANGE_QA_MEMORY`.  Right-click (WIDGETUNSELECT) forwards
//!   `MSG_DELETE_MACRO(all, 0)`.
//! * Sight: left-click forwards `MSG_LOCK_ALT`; right-click
//!   (WIDGETUNSELECT) forwards `MSG_UNLOCK_ALT` + clears the selected
//!   view element.
//! * QuickStart: left-click (disabled during recording) forwards
//!   `MSG_START_MACRO(all, 0)`; right-click forwards
//!   `MSG_DELETE_MACRO(all, 0)`.
//!
//! Enable state:
//!   - Clock: always visually present; click gate on a PC being selected.
//!   - Sight: always enabled.
//!   - QuickStart: enabled when any PC has slot 0 populated AND we're not
//!     currently recording.

use crate::gfx_types::{Point, Rect as SdlRect};
use robin_engine::coordinates::ScreenBBox;
use robin_engine::engine as engine_api;
use robin_engine::engine::PANNEL_HEIGHT;
use robin_engine::sprite as engine_sprite;

use crate::ingame_menu::layout::{
    BTN_STATE_DISABLED, BTN_STATE_HOVER, BTN_STATE_NORMAL, BTN_STATE_PRESSED, button_sprite_state,
};
use crate::renderer::{BLIT_SOURCE_TRANSPARENT, Renderer};
use crate::resource_ids::{RHID_CLOCK, RHID_QUICKSTART, RHID_SIGHT};
use crate::resource_manager::ResourceManager;

fn screen_rect_to_sprite_bbox(rect: SdlRect) -> engine_sprite::BBox {
    let bbox = ScreenBBox::from_coords(
        rect.x() as f32,
        rect.y() as f32,
        (rect.x() + rect.width() as i32) as f32,
        (rect.y() + rect.height() as i32) as f32,
    );
    engine_sprite::BBox::new(bbox.top_left(), bbox.bottom_right())
}

/// Logical id for the three corner HUD buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CornerButton {
    /// Record / cycle QA memory slot.
    Clock,
    /// Lock the alt (view-cone) hover modifier.
    Sight,
    /// Launch all PCs' slot-0 macros.
    QuickStart,
}

impl CornerButton {
    fn index(self) -> usize {
        match self {
            CornerButton::Clock => 0,
            CornerButton::Sight => 1,
            CornerButton::QuickStart => 2,
        }
    }
}

/// Per-button enable / pressed state for this frame.
///
/// - Clock: always drawable; click is gated on a PC being selected.
/// - Sight: always enabled.  `pressed` follows `is_lock_alt` so the
///   widget reads as latched while the lock is held.
/// - QuickStart: disabled when recording is in progress or no PC has a
///   slot-0 macro.
#[derive(Debug, Clone, Copy, Default)]
pub struct CornerButtonEnable {
    pub clock: bool,
    pub clock_recording: bool,
    /// True when the Clock (save-QA horn) should dim to semi-transparent.
    /// The widget renders faintly when there's no PC available to record
    /// a macro for — it stays on screen so the player knows where it'd
    /// appear, but reads as "inactive".
    pub clock_dim: bool,
    pub sight: bool,
    pub sight_locked: bool,
    pub quickstart: bool,
    /// Same dimming convention as `clock_dim` — QuickStart renders
    /// semi-transparent when no PC has a recorded macro in slot 0.
    pub quickstart_dim: bool,
}

impl CornerButtonEnable {
    /// Snapshot the enable / selected mask from the engine + game state.
    pub fn from_engine(engine: &engine_api::Engine) -> Self {
        // Clock and QuickStart are always drawn, but dim to
        // semi-transparent when their click gate isn't satisfied:
        //   - Clock dims when no PC is selected.
        //   - QuickStart dims when no PC has a macro recorded in any
        //     QA slot (nested `slot × PC` loop, enabled when *any* PC
        //     has a macro in *any* slot).
        let any_pc_selected = !engine.selected_pc_ids().is_empty();
        let any_qa = (0..crate::macro_store::NUMBER_OF_QA_MEMORY as u8).any(|slot| {
            engine
                .pc_ids()
                .iter()
                .any(|&id| engine.has_quick_action(id, slot))
        });
        Self {
            clock: true,
            clock_recording: engine.is_recording_macro(),
            clock_dim: !any_pc_selected,
            sight: true,
            sight_locked: engine.is_lock_alt(),
            quickstart: true,
            quickstart_dim: !any_qa,
        }
    }

    fn for_button(self, btn: CornerButton) -> (bool, bool) {
        match btn {
            CornerButton::Clock => (self.clock, self.clock_recording),
            CornerButton::Sight => (self.sight, self.sight_locked),
            CornerButton::QuickStart => (self.quickstart, false),
        }
    }

    fn is_dim(self, btn: CornerButton) -> bool {
        match btn {
            CornerButton::Clock => self.clock_dim,
            CornerButton::Sight => false,
            CornerButton::QuickStart => self.quickstart_dim,
        }
    }
}

/// Screen-space bounding boxes for the three corner HUD buttons.
#[derive(Debug, Clone, Copy)]
pub struct CornerHudLayout {
    pub clock: SdlRect,
    pub sight: SdlRect,
    pub quickstart: SdlRect,
}

impl CornerHudLayout {
    /// Derive button rects from the current screen resolution and loaded
    /// sprite sizes.
    ///
    /// The lower-panel frame origin is `(0, height - PANNEL_HEIGHT)` —
    /// `PANNEL_HEIGHT = 80` is the *interactive* panel height, not the
    /// 165-pixel decorative corner sprite height.  Clock/QuickStart
    /// offsets are relative to that frame origin; Sight/Zoom are
    /// screen-absolute.
    pub fn for_resolution(screen_w: u32, screen_h: u32, sprites: &CornerButtonSprites) -> Self {
        const FALLBACK_W: u32 = 24;
        const FALLBACK_H: u32 = 24;

        let sw = screen_w as i32;
        let sh = screen_h as i32;
        // Lower-panel frame origin: `(0, height - PANNEL_HEIGHT)` where
        // `PANNEL_HEIGHT = 80`.
        let frame_origin_y = sh - PANNEL_HEIGHT as i32;

        let (clock_w, clock_h) = sprites
            .clock_size()
            .unwrap_or((FALLBACK_W as u16, FALLBACK_H as u16));
        let (sight_w, sight_h) = sprites
            .sight_size()
            .unwrap_or((FALLBACK_W as u16, FALLBACK_H as u16));
        let (qs_w, qs_h) = sprites
            .quickstart_size()
            .unwrap_or((FALLBACK_W as u16, FALLBACK_H as u16));

        Self {
            // Clock — `(width-60, 13)` relative to the panel frame origin.
            clock: SdlRect::new(sw - 60, frame_origin_y + 13, clock_w as u32, clock_h as u32),
            // Sight — `(width-100, 0)` screen-absolute (top-right), not
            // panel-relative.
            sight: SdlRect::new(sw - 100, 0, sight_w as u32, sight_h as u32),
            // QuickStart — `(width-29, -15)` relative to the panel frame
            // origin (so it sits *above* the panel proper).
            quickstart: SdlRect::new(sw - 29, frame_origin_y - 15, qs_w as u32, qs_h as u32),
        }
    }

    /// Hit-test a screen-space click.  Returns the first matching
    /// button that is currently enabled, or `None`.
    pub fn hit_test(&self, x: i32, y: i32, enable: CornerButtonEnable) -> Option<CornerButton> {
        let pt = Point::new(x, y);
        if enable.clock && self.clock.contains_point(pt) {
            return Some(CornerButton::Clock);
        }
        if enable.sight && self.sight.contains_point(pt) {
            return Some(CornerButton::Sight);
        }
        if enable.quickstart && self.quickstart.contains_point(pt) {
            return Some(CornerButton::QuickStart);
        }
        None
    }

    /// Geometric hit-test that ignores the enable mask — used so the
    /// tooltip still surfaces when the button is disabled.
    pub fn hit_test_geometric(&self, x: i32, y: i32) -> Option<CornerButton> {
        let pt = Point::new(x, y);
        if self.clock.contains_point(pt) {
            return Some(CornerButton::Clock);
        }
        if self.sight.contains_point(pt) {
            return Some(CornerButton::Sight);
        }
        if self.quickstart.contains_point(pt) {
            return Some(CornerButton::QuickStart);
        }
        None
    }
}

/// One loaded BTTN sprite frame: surface id plus native pixel size.
type SpriteFrame = (u32, u16, u16);

/// Cached sprite surface ids for the three corner HUD buttons.
///
/// Each button owns up to four sub-ids — disabled, normal, focused, and
/// pressed.  Missing sub-ids fall back to the normal frame; if that's
/// also missing the button is simply not drawn (matches `zoom_hud`).
#[derive(Debug, Default)]
pub struct CornerButtonSprites {
    pub clock: [Option<SpriteFrame>; 4],
    pub sight: [Option<SpriteFrame>; 4],
    pub quickstart: [Option<SpriteFrame>; 4],
}

impl CornerButtonSprites {
    /// Load button sprites from the attached DEFAULT.RES.  Walks
    /// sub-ids 0..=3 per resource; missing sub-ids stay `None`.
    pub fn load(res: &mut ResourceManager, renderer: &mut Renderer) -> Self {
        fn fetch_frame(
            res: &mut ResourceManager,
            renderer: &mut Renderer,
            id: i32,
            sub: usize,
            label: &str,
        ) -> Option<SpriteFrame> {
            match res.get_picture(id, sub) {
                Ok(pic) => {
                    let w = pic.width;
                    let h = pic.height;
                    let surface = crate::ui_panel::pic_to_surface(renderer, pic);
                    tracing::info!(
                        "corner_hud: {label} sub{sub} → resource {id}, surface {surface} ({w}x{h})"
                    );
                    Some((surface, w, h))
                }
                Err(e) => {
                    tracing::debug!("corner_hud: {label} sub{sub} missing (resource {id}): {e}");
                    None
                }
            }
        }
        fn fetch_all(
            res: &mut ResourceManager,
            renderer: &mut Renderer,
            id: i32,
            label: &str,
        ) -> [Option<SpriteFrame>; 4] {
            [
                fetch_frame(res, renderer, id, 0, label),
                fetch_frame(res, renderer, id, 1, label),
                fetch_frame(res, renderer, id, 2, label),
                fetch_frame(res, renderer, id, 3, label),
            ]
        }

        Self {
            clock: fetch_all(res, renderer, RHID_CLOCK, "Clock"),
            sight: fetch_all(res, renderer, RHID_SIGHT, "Sight"),
            quickstart: fetch_all(res, renderer, RHID_QUICKSTART, "QuickStart"),
        }
    }

    fn frames(&self, btn: CornerButton) -> &[Option<SpriteFrame>; 4] {
        match btn {
            CornerButton::Clock => &self.clock,
            CornerButton::Sight => &self.sight,
            CornerButton::QuickStart => &self.quickstart,
        }
    }

    /// The sprite actually rendered for a given interaction state,
    /// with a fallback to the normal frame if the requested state is
    /// missing — matches `zoom_hud.rs`.
    fn frame(&self, btn: CornerButton, state: usize) -> Option<SpriteFrame> {
        let frames = self.frames(btn);
        frames[state].or(frames[BTN_STATE_NORMAL])
    }

    pub fn clock_size(&self) -> Option<(u16, u16)> {
        Self::size_of(&self.clock)
    }

    pub fn sight_size(&self) -> Option<(u16, u16)> {
        Self::size_of(&self.sight)
    }

    pub fn quickstart_size(&self) -> Option<(u16, u16)> {
        Self::size_of(&self.quickstart)
    }

    fn size_of(frames: &[Option<SpriteFrame>; 4]) -> Option<(u16, u16)> {
        frames[BTN_STATE_NORMAL]
            .or(frames[BTN_STATE_HOVER])
            .or(frames[BTN_STATE_PRESSED])
            .or(frames[BTN_STATE_DISABLED])
            .map(|(_, w, h)| (w, h))
    }
}

/// Transient per-frame hover snapshot used by the draw routine.
#[derive(Debug, Clone, Copy, Default)]
pub struct CornerHoverState {
    pub hovered: Option<CornerButton>,
    pub mouse_pressed: bool,
}

/// Draw the three corner HUD buttons with state-aware sprite selection.
/// If a sprite is missing from DEFAULT.RES the button is silently
/// skipped — no placeholder-rect fallback (matches `zoom_hud`, which
/// dropped its fallback to avoid visible black boxes in release).
pub fn draw_with_sprites(
    renderer: &mut Renderer,
    layout: &CornerHudLayout,
    enable: CornerButtonEnable,
    hover: CornerHoverState,
    sprites: &CornerButtonSprites,
) {
    let buttons = [
        (&layout.clock, CornerButton::Clock),
        (&layout.sight, CornerButton::Sight),
        (&layout.quickstart, CornerButton::QuickStart),
    ];

    for (rect, btn) in buttons {
        let (enabled, selected) = enable.for_button(btn);
        let hovered = hover.hovered == Some(btn);
        let pressed = selected || (hovered && hover.mouse_pressed && enabled);
        let state = button_sprite_state(enabled, hovered || selected, pressed);

        // Hide disabled buttons entirely.  Matches the original game
        // screenshots: Clock is hidden until a PC is selected, QuickStart
        // is hidden until at least one macro is recorded, Sight stays
        // visible whenever the game is in play.  The DISABLED sub-frames
        // are visually identical to the normal frames (or absent), so
        // skipping the draw entirely matches what the player expects.
        if !enabled {
            continue;
        }

        // No placeholder-rect fallback — if the sprite is missing from
        // DEFAULT.RES we simply don't draw the button (matches `zoom_hud`).
        if let Some((sid, _sw, _sh)) = sprites.frame(btn, state) {
            let dst = screen_rect_to_sprite_bbox(*rect);
            tracing::trace!(
                "corner_hud blit: {:?} state {state} dim={} surface {sid} at ({},{}) {}x{}",
                btn,
                enable.is_dim(btn),
                rect.x(),
                rect.y(),
                rect.width(),
                rect.height()
            );
            if enable.is_dim(btn) {
                // ~40% alpha — matches the "inactive but still visible"
                // read the user expects for the blowing-horn / QA-start
                // castle when their click conditions aren't satisfied.
                renderer.blit_to_screen_alpha(sid, None, Some(&dst), 40, BLIT_SOURCE_TRANSPARENT);
            } else if btn == CornerButton::QuickStart {
                renderer.blit_with_shadow(
                    sid,
                    None,
                    0, // screen
                    Some(&dst),
                    0,  // shadow_color unused
                    50, // SBUIRendererShadow default intensity
                    BLIT_SOURCE_TRANSPARENT,
                );
            } else {
                // Clock and Sight are SBUIRendererBitmap widgets in C++.
                renderer.blit_to_screen(sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
            }
        }
    }
}

/// Hover tracker for the three button tooltips, mirroring
/// `ZoomTooltipTracker`.
#[derive(Default, Clone)]
pub struct CornerTooltipTracker {
    inner: crate::ui_panel::RequirementsTooltipTracker,
}

impl CornerTooltipTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, hovered: Option<CornerButton>) {
        self.inner.update(hovered.map(CornerButton::index));
    }

    pub fn ready_button(&self) -> Option<CornerButton> {
        self.inner.ready_slot().and_then(|i| match i {
            0 => Some(CornerButton::Clock),
            1 => Some(CornerButton::Sight),
            2 => Some(CornerButton::QuickStart),
            _ => None,
        })
    }
}

/// Menu-text id for the given corner button's tooltip.
pub fn corner_button_tooltip_mt_id(btn: CornerButton) -> usize {
    use crate::ingame_menu::resources::{
        MT_INFOBULLE_LAUNCHQA_ALL, MT_INFOBULLE_SAVEQA, MT_INFOBULLE_VIEWCONE,
    };
    match btn {
        CornerButton::Clock => MT_INFOBULLE_SAVEQA,
        CornerButton::Sight => MT_INFOBULLE_VIEWCONE,
        CornerButton::QuickStart => MT_INFOBULLE_LAUNCHQA_ALL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_places_buttons_right_of_screen() {
        let sprites = CornerButtonSprites::default();
        let layout = CornerHudLayout::for_resolution(800, 600, &sprites);
        // All three buttons cluster near the right edge.
        assert!(layout.clock.x() > 600);
        assert!(layout.sight.x() > 600);
        assert!(layout.quickstart.x() > 600);
        // Sight is to the left of Clock (which is to the left of QuickStart).
        assert!(layout.sight.x() < layout.clock.x());
    }

    #[test]
    fn hit_test_respects_enable() {
        let sprites = CornerButtonSprites::default();
        let layout = CornerHudLayout::for_resolution(800, 600, &sprites);
        let pt = (layout.clock.x() + 1, layout.clock.y() + 1);
        let all = CornerButtonEnable {
            clock: true,
            clock_recording: false,
            clock_dim: false,
            sight: true,
            sight_locked: false,
            quickstart: true,
            quickstart_dim: false,
        };
        assert_eq!(layout.hit_test(pt.0, pt.1, all), Some(CornerButton::Clock));
        let none = CornerButtonEnable::default();
        assert_eq!(layout.hit_test(pt.0, pt.1, none), None);
    }
}
