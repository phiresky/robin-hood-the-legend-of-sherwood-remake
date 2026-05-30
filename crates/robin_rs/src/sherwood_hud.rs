//! Sherwood-mission HUD buttons (DisplayCampaignMap, GoToExit,
//! StartMission, QuitMission).
//!
//! Only the logical layout + hit-test + enabled-state machine is
//! implemented here; visual polish (proper sprite backgrounds, tooltips,
//! blink animations) is a separate pass driven by the widget module.
//!
//! The buttons drive the Sherwood flow defined in `run_mission`:
//!
//! - `DisplayCampaignMap` re-raises the campaign-map overlay (the
//!   same modal shown on Sherwood entry).  Enabled while the player
//!   hasn't yet committed to a next mission.
//! - `GoToExit` centers the camera on the current mission's exit
//!   gate.  Enabled once a mission has been committed via the
//!   overlay.
//! - `StartMission` re-triggers the confirm dialog for the already
//!   selected next mission — without having to re-open the map.
//!   Enabled once a mission has been committed.
//! - `QuitMission` opens the `REALLY_RETURN_TO_MAP` confirm dialog
//!   to bail back to the campaign-map overlay.  Always enabled in
//!   Sherwood once the player has picked a mission (so they can
//!   back out).

use crate::gfx_types::{Point, Rect as SdlRect};

use crate::ingame_menu::layout::{
    BTN_STATE_DISABLED, BTN_STATE_HOVER, BTN_STATE_NORMAL, BTN_STATE_PRESSED, button_sprite_state,
};
use crate::native_font::NativeFont;
use crate::renderer::{BLIT_SOURCE_TRANSPARENT, Renderer};
use crate::resource_ids::{
    RHID_DISPLAY_CAMPAIGN_MAP, RHID_FLOATING_CANCEL, RHID_FLOATING_OK, RHID_GO_TO_EXIT,
};
use crate::resource_manager::ResourceManager;

/// Logical Sherwood button id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SherwoodButton {
    /// Re-raise the campaign map.
    DisplayCampaignMap,
    /// Centre the camera on the next mission's exit gate.
    GoToExit,
    /// Confirm the committed mission selection.
    StartMission,
    /// Return to the campaign map.
    QuitMission,
}

impl SherwoodButton {
    fn index(self) -> usize {
        match self {
            SherwoodButton::DisplayCampaignMap => 0,
            SherwoodButton::GoToExit => 1,
            SherwoodButton::StartMission => 2,
            SherwoodButton::QuitMission => 3,
        }
    }
}

/// Which Sherwood buttons are interactable this frame.
///
/// The enable toggles are derived from initialization (GoToExit
/// default-off), commit transitions (DisplayCampaignMap disabled after
/// commit), and per-frame mission-team refreshes (Start gated on
/// requirements).
#[derive(Debug, Clone, Copy)]
pub struct SherwoodButtonEnable {
    pub display_campaign_map: bool,
    pub go_to_exit: bool,
    pub start_mission: bool,
    pub quit_mission: bool,
}

impl SherwoodButtonEnable {
    /// Default state at Sherwood entry, before the player has
    /// committed to a next mission via the map overlay.  GoToExit is
    /// disabled, Start/Quit disabled until `DisplayCampaignMap`
    /// completes.
    pub fn pre_commit() -> Self {
        Self {
            display_campaign_map: true,
            go_to_exit: false,
            start_mission: false,
            quit_mission: false,
        }
    }

    /// State right after the player commits a mission via the overlay,
    /// before the live requirements gate has been applied to Start.
    /// DisplayCampaignMap is disabled, GoToExit and Quit are enabled.
    /// `start_mission` is left `false` here — the caller should
    /// immediately follow up with [`Self::apply_update_mission_team`]
    /// to reflect the live requirements / blazon-conversion gates.
    pub fn post_commit() -> Self {
        Self {
            display_campaign_map: false,
            go_to_exit: true,
            start_mission: false,
            quit_mission: true,
        }
    }

    /// Refresh the `start_mission` gate based on the live campaign
    /// state:
    ///
    /// - In men-to-blazon conversion mode, enable Start only when the
    ///   campaign has enough peasants to convert.
    /// - Otherwise, enable Start when the selected mission's
    ///   requirements are fulfilled by the current mission team
    ///   (`Campaign::mission_requirements_met`).
    ///
    /// Once post-commit, `QuitMission` is always live so the player can
    /// back out.
    ///
    /// `start_disabled_temp` / `quit_disabled_temp` are the transient
    /// disable flags driven by the hourglass tick at
    /// `Game::perform_hourglass_inner`.  Without them the PC-guarded
    /// transient suppression does not visually disable the Sherwood
    /// buttons.  We fold the temp flags directly into the Sherwood
    /// enable mask rather than maintaining a parallel
    /// `enable_quit_mission` flag for Sherwood — the persistent
    /// post-commit state is tracked by [`Self::post_commit`].
    pub fn apply_update_mission_team(
        &mut self,
        men_to_blazon: bool,
        can_convert_merry_men: bool,
        requirements_met: bool,
        start_disabled_temp: bool,
        quit_disabled_temp: bool,
    ) {
        let base_start = if men_to_blazon {
            can_convert_merry_men
        } else {
            requirements_met
        };
        self.start_mission = base_start && !start_disabled_temp;
        self.quit_mission = !quit_disabled_temp;
    }

    /// Refresh the `go_to_exit` gate based on the live portrait /
    /// mission state.  Runs every frame, equivalent to the predicate:
    ///
    /// ```text
    /// go_to_exit =
    ///     portrait_count != 0
    ///     && has_next_mission
    ///     && ( portrait_count <= number_of_beam_mes
    ///          || men_to_blazon )
    ///     && !selected_pc_in_mission_team
    /// ```
    ///
    /// - `portrait_count` is the Sherwood portrait-bar size — the
    ///   number of PC entities the HUD draws
    ///   (`engine.pc_ids().len()`), priority-sorted at level load.
    /// - `number_of_beam_mes` comes from the upcoming mission's
    ///   profile (`CharacterProfile::number_of_beam_mes`).  The guard
    ///   stops the player from exiting with more Merry Men than the
    ///   mission can physically beam in.
    /// - `selected_pc_in_mission_team` comes from
    ///   [`EngineInner::are_selected_pc_in_mission_team`] — GoToExit
    ///   goes dark once every selected PC is already committed to the
    ///   upcoming mission, so the button only lights up when the
    ///   player has something meaningful to exit with.
    ///
    /// Callers that still have no committed next mission (`post_commit`
    /// hasn't run) keep `go_to_exit = false` from [`Self::pre_commit`];
    /// this helper leaves the flag alone in that case so pre-commit
    /// state is preserved.
    pub fn apply_update_portraits_delayed(
        &mut self,
        has_next_mission: bool,
        portrait_count: usize,
        number_of_beam_mes: u16,
        men_to_blazon: bool,
        selected_pc_in_mission_team: bool,
    ) {
        if !has_next_mission {
            // No refresh when the player hasn't picked a target yet.
            return;
        }
        let fits_beam_slots = portrait_count <= usize::from(number_of_beam_mes) || men_to_blazon;
        self.go_to_exit = portrait_count != 0 && fits_beam_slots && !selected_pc_in_mission_team;
    }
}

/// Screen-space bounding boxes for the four Sherwood HUD buttons.
#[derive(Debug, Clone, Copy)]
pub struct SherwoodHudLayout {
    pub display_campaign_map: SdlRect,
    pub go_to_exit: SdlRect,
    pub start_mission: SdlRect,
    pub quit_mission: SdlRect,
}

impl SherwoodHudLayout {
    /// Derive button rects from the current screen resolution.
    ///
    /// Sizes are rough estimates matching the sprite dimensions from
    /// DEFAULT.RES (confirmed by eye — proper sprite-driven sizing
    /// lands with the widget port).
    pub fn for_resolution(screen_w: u32, _screen_h: u32, sprites: &SherwoodButtonSprites) -> Self {
        // Fallback sizes for missing sprites (keeps the hit-rect
        // usable during development against incomplete DEFAULT.RES).
        const FALLBACK_WIDE_W: u16 = 80;
        const FALLBACK_WIDE_H: u16 = 32;
        const FALLBACK_TALL_W: u16 = 40;
        const FALLBACK_TALL_H: u16 = 40;

        let sw = screen_w as i32;

        // DisplayCampaignMap / GoToExit share the top-right corner of
        // the screen at `(width - 100, 0)` (no viewport-origin offset)
        // — they sit on the parchment-scroll decoration, not the lower
        // panel.  Sherwood missions show DisplayCampaignMap; non-Sherwood
        // would show Sight instead at the same coordinates.
        let wide_x = sw - 100;
        let wide_y = 0;

        // StartMission / QuitMission live at absolute (sw-45, 105) and
        // (sw-45, 150) — see derivation in the parent module.
        let tall_x = sw - 45;
        let start_y = 105;
        let quit_y = 150;

        let (dcm_w, dcm_h) = sprites
            .size(SherwoodButton::DisplayCampaignMap)
            .unwrap_or((FALLBACK_WIDE_W, FALLBACK_WIDE_H));
        let (gte_w, gte_h) = sprites
            .size(SherwoodButton::GoToExit)
            .unwrap_or((FALLBACK_WIDE_W, FALLBACK_WIDE_H));
        let (sm_w, sm_h) = sprites
            .size(SherwoodButton::StartMission)
            .unwrap_or((FALLBACK_TALL_W, FALLBACK_TALL_H));
        let (qm_w, qm_h) = sprites
            .size(SherwoodButton::QuitMission)
            .unwrap_or((FALLBACK_TALL_W, FALLBACK_TALL_H));

        Self {
            display_campaign_map: SdlRect::new(wide_x, wide_y, dcm_w as u32, dcm_h as u32),
            go_to_exit: SdlRect::new(wide_x, wide_y, gte_w as u32, gte_h as u32),
            start_mission: SdlRect::new(tall_x, start_y, sm_w as u32, sm_h as u32),
            quit_mission: SdlRect::new(tall_x, quit_y, qm_w as u32, qm_h as u32),
        }
    }

    /// Hit-test a screen-space click.  Returns the first matching
    /// button that is currently enabled, or `None`.  Respects the
    /// layout where `DisplayCampaignMap` and `GoToExit` overlap —
    /// whichever is enabled wins.
    pub fn hit_test(&self, x: i32, y: i32, enable: SherwoodButtonEnable) -> Option<SherwoodButton> {
        let pt = Point::new(x, y);
        if enable.display_campaign_map && self.display_campaign_map.contains_point(pt) {
            return Some(SherwoodButton::DisplayCampaignMap);
        }
        if enable.go_to_exit && self.go_to_exit.contains_point(pt) {
            return Some(SherwoodButton::GoToExit);
        }
        if enable.start_mission && self.start_mission.contains_point(pt) {
            return Some(SherwoodButton::StartMission);
        }
        if enable.quit_mission && self.quit_mission.contains_point(pt) {
            return Some(SherwoodButton::QuitMission);
        }
        None
    }

    /// Purely geometric hit-test — reports which visible button the
    /// cursor is over, filtered only by the `enable` mask so the
    /// DisplayCampaignMap / GoToExit overlap still resolves
    /// consistently (disabled widgets don't claim hover, but
    /// overlapping-enabled ones do).  Used to drive the tooltip hover
    /// tracker, which needs to know when the cursor is over a specific
    /// live button (Start/Quit tooltip text depends on mode and
    /// switches with it).
    pub fn hit_test_geometric(
        &self,
        x: i32,
        y: i32,
        enable: SherwoodButtonEnable,
    ) -> Option<SherwoodButton> {
        self.hit_test(x, y, enable)
    }
}

type SpriteFrame = (u32, u16, u16);

/// Cached sprite surface ids for the four Sherwood HUD buttons.
///
/// Resource IDs: `RHID_DISPLAY_CAMPAIGN_MAP` (251), `RHID_GO_TO_EXIT`
/// (241), `RHID_FLOATING_OK` (281 — Start), `RHID_FLOATING_CANCEL`
/// (282 — Quit).  Each resource is a multi-sub-id BTTN strip:
/// disabled, normal, focused, selected.
#[derive(Debug, Default, Clone, Copy)]
pub struct SherwoodButtonSprites {
    pub display_campaign_map: [Option<SpriteFrame>; 4],
    pub go_to_exit: [Option<SpriteFrame>; 4],
    pub start_mission: [Option<SpriteFrame>; 4],
    pub quit_mission: [Option<SpriteFrame>; 4],
}

impl SherwoodButtonSprites {
    /// Load button sprites from the attached DEFAULT.RES.  Missing
    /// resources fall back to `None`; `draw_with_sprites` then skips
    /// the button entirely (no fallback rect — see `draw_with_sprites`).
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
                        "sherwood_hud: {label} sub{sub} -> resource {id}, surface {surface} ({w}x{h})"
                    );
                    Some((surface, w, h))
                }
                Err(e) => {
                    tracing::debug!("sherwood_hud: {label} sub{sub} missing (resource {id}): {e}");
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
            display_campaign_map: fetch_all(
                res,
                renderer,
                RHID_DISPLAY_CAMPAIGN_MAP,
                "DisplayCampaignMap",
            ),
            go_to_exit: fetch_all(res, renderer, RHID_GO_TO_EXIT, "GoToExit"),
            start_mission: fetch_all(res, renderer, RHID_FLOATING_OK, "StartMission"),
            quit_mission: fetch_all(res, renderer, RHID_FLOATING_CANCEL, "QuitMission"),
        }
    }

    fn frames(&self, btn: SherwoodButton) -> &[Option<SpriteFrame>; 4] {
        match btn {
            SherwoodButton::DisplayCampaignMap => &self.display_campaign_map,
            SherwoodButton::GoToExit => &self.go_to_exit,
            SherwoodButton::StartMission => &self.start_mission,
            SherwoodButton::QuitMission => &self.quit_mission,
        }
    }

    fn frame(&self, btn: SherwoodButton, state: usize, frame_counter: u32) -> Option<SpriteFrame> {
        let frames = self.frames(btn);
        let state = if btn == SherwoodButton::DisplayCampaignMap && state == BTN_STATE_NORMAL {
            // C++ calls SetBlinking(DEFAULT, DEFAULT, FOCUSED, 25):
            // 25 frames normal, 25 frames focused, repeating.
            if (frame_counter / 25) & 1 == 1 {
                BTN_STATE_HOVER
            } else {
                BTN_STATE_NORMAL
            }
        } else {
            state
        };
        frames[state].or(frames[BTN_STATE_NORMAL])
    }

    fn size(&self, btn: SherwoodButton) -> Option<(u16, u16)> {
        let frames = self.frames(btn);
        frames[BTN_STATE_NORMAL]
            .or(frames[BTN_STATE_HOVER])
            .or(frames[BTN_STATE_PRESSED])
            .or(frames[BTN_STATE_DISABLED])
            .map(|(_, w, h)| (w, h))
    }
}

/// Transient per-frame hover snapshot used by the draw routine.
#[derive(Debug, Clone, Copy, Default)]
pub struct SherwoodHoverState {
    pub hovered: Option<SherwoodButton>,
    pub mouse_pressed: bool,
}

/// Draw the Sherwood HUD buttons using loaded sprite surfaces.  A
/// button with no loaded sprite is simply skipped (no fallback rect).
pub fn draw_with_sprites(
    renderer: &mut Renderer,
    layout: &SherwoodHudLayout,
    enable: SherwoodButtonEnable,
    hover: SherwoodHoverState,
    sprites: &SherwoodButtonSprites,
    frame_counter: u32,
) {
    let buttons = [
        (
            &layout.display_campaign_map,
            enable.display_campaign_map,
            SherwoodButton::DisplayCampaignMap,
        ),
        (
            &layout.go_to_exit,
            enable.go_to_exit,
            SherwoodButton::GoToExit,
        ),
        (
            &layout.start_mission,
            enable.start_mission,
            SherwoodButton::StartMission,
        ),
        (
            &layout.quit_mission,
            enable.quit_mission,
            SherwoodButton::QuitMission,
        ),
    ];

    for (rect, enabled, btn) in buttons {
        let is_hovered = hover.hovered == Some(btn);
        let pressed = is_hovered && hover.mouse_pressed && enabled;
        let state = button_sprite_state(enabled, is_hovered, pressed);
        if let Some((sid, _sw, _sh)) = sprites.frame(btn, state, frame_counter) {
            // Blit centred inside the hit-test rect.  The sprite is
            // typically authored at the button's native size, but we
            // blit into the logical rect anyway so the visuals track
            // our layout.
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
            if matches!(
                btn,
                SherwoodButton::StartMission | SherwoodButton::QuitMission
            ) {
                renderer.blit_with_shadow(
                    sid,
                    None,
                    0, // screen
                    Some(&dst),
                    0,  // shadow_color (unused in the MMX-parity path)
                    50, // SBUIRendererShadow default intensity
                    BLIT_SOURCE_TRANSPARENT,
                );
            } else {
                renderer.blit_to_screen(sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
            }
        }
        // No placeholder-rect fallback — a missing sprite simply means
        // the button isn't drawn.  The old "Go" / "Back" / "Map" label
        // rectangles showed through in release builds when a resource
        // lookup quietly failed.
    }
}

/// Hover tracker for the four Sherwood button tooltips.  Thin wrapper
/// around the shared `RequirementsTooltipTracker` keyed on
/// [`SherwoodButton::index`], matching [`crate::zoom_hud::ZoomTooltipTracker`].
#[derive(Default, Clone)]
pub struct SherwoodTooltipTracker {
    inner: crate::ui_panel::RequirementsTooltipTracker,
}

impl SherwoodTooltipTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, hovered: Option<SherwoodButton>) {
        self.inner.update(hovered.map(SherwoodButton::index));
    }

    pub fn ready_button(&self) -> Option<SherwoodButton> {
        self.inner.ready_slot().and_then(|i| match i {
            0 => Some(SherwoodButton::DisplayCampaignMap),
            1 => Some(SherwoodButton::GoToExit),
            2 => Some(SherwoodButton::StartMission),
            3 => Some(SherwoodButton::QuitMission),
            _ => None,
        })
    }
}

/// Menu-text id for the Start/Quit mission button tooltip in the
/// current mode: Sherwood + men-to-blazon → Farmers-to-blazon /
/// BackToMap; Sherwood + regular → BeginMission / BackToMap;
/// in-mission → MissionFinish / MissionAbandon.
///
/// Returns `None` for the `DisplayCampaignMap` / `GoToExit` buttons —
/// their tooltips are static and don't switch with mode.  Callers
/// handle those via the static IDs in [`sherwood_static_tooltip_mt_id`].
pub fn start_quit_tooltip_mt_ids(is_sherwood: bool, men_to_blazon: bool) -> (usize, usize) {
    use crate::ingame_menu::resources::{
        MT_INFOBULLE_BUTTON_FARMERS_TO_BLAZON, MT_INFOBULLE_MISSION_ABANDON,
        MT_INFOBULLE_MISSION_FINISH, MT_INFOBULLE_QG_BACKTOMAP, MT_INFOBULLE_QG_BEGIN_MISSION,
    };
    if is_sherwood {
        if men_to_blazon {
            (
                MT_INFOBULLE_BUTTON_FARMERS_TO_BLAZON,
                MT_INFOBULLE_QG_BACKTOMAP,
            )
        } else {
            (MT_INFOBULLE_QG_BEGIN_MISSION, MT_INFOBULLE_QG_BACKTOMAP)
        }
    } else {
        (MT_INFOBULLE_MISSION_FINISH, MT_INFOBULLE_MISSION_ABANDON)
    }
}

/// Menu-text id for the tooltip attached to a Sherwood button, given
/// the current mode.  Uses [`start_quit_tooltip_mt_ids`] for Start/Quit
/// and the fixed static IDs for the Sherwood-only DisplayCampaignMap /
/// GoToExit buttons.
pub fn sherwood_button_tooltip_mt_id(
    btn: SherwoodButton,
    is_sherwood: bool,
    men_to_blazon: bool,
) -> Option<usize> {
    let (start_id, quit_id) = start_quit_tooltip_mt_ids(is_sherwood, men_to_blazon);
    match btn {
        SherwoodButton::StartMission => Some(start_id),
        SherwoodButton::QuitMission => Some(quit_id),
        // DisplayCampaignMap / GoToExit use their own fixed menu-text
        // IDs (`MT_INFOBULLE_QG_SELECT_MISSION` = 299 and
        // `MT_INFOBULLE_QG_DEPLOY` = 296), set once at initialization.
        // They aren't wired through the start/quit refresh path, so
        // they're outside this chunk's scope — when a future pass wires
        // those static tooltips in, plug them in here.
        SherwoodButton::DisplayCampaignMap | SherwoodButton::GoToExit => None,
    }
}

/// Draw the hover tooltip for the Sherwood HUD buttons.  Does nothing
/// when no tooltip is ready or the font stack isn't available.
#[allow(clippy::too_many_arguments)]
pub fn draw_tooltip(
    renderer: &mut Renderer,
    tracker: &SherwoodTooltipTracker,
    tooltip_text: impl Fn(SherwoodButton) -> Option<String>,
    font: &NativeFont,
    shadow: Option<&NativeFont>,
    mouse_x: i32,
    mouse_y: i32,
    cursor_size: (i32, i32),
) {
    if let Some(btn) = tracker.ready_button()
        && let Some(text) = tooltip_text(btn)
        && !text.is_empty()
    {
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
    fn layout_fits_screen() {
        let sprites = SherwoodButtonSprites::default();
        let layout = SherwoodHudLayout::for_resolution(800, 600, &sprites);
        // Display / GoToExit sit at the top-right (on the parchment
        // scroll decoration), NOT the lower panel — screen-absolute
        // positioning.
        assert_eq!(layout.display_campaign_map.y(), 0);
        assert!(
            layout.display_campaign_map.x() + layout.display_campaign_map.width() as i32 <= 800
        );
        // Start / Quit near the top-right.
        assert_eq!(layout.start_mission.x(), 800 - 45);
        assert_eq!(layout.start_mission.y(), 105);
        assert_eq!(layout.quit_mission.y(), 150);
    }

    #[test]
    fn hit_test_prefers_enabled() {
        let sprites = SherwoodButtonSprites::default();
        let layout = SherwoodHudLayout::for_resolution(800, 600, &sprites);
        let pt_map = (
            layout.display_campaign_map.x() + 1,
            layout.display_campaign_map.y() + 1,
        );
        // DisplayCampaignMap and GoToExit overlap; enabling only
        // GoToExit should return GoToExit.
        let post = SherwoodButtonEnable::post_commit();
        assert_eq!(
            layout.hit_test(pt_map.0, pt_map.1, post),
            Some(SherwoodButton::GoToExit)
        );
        let pre = SherwoodButtonEnable::pre_commit();
        assert_eq!(
            layout.hit_test(pt_map.0, pt_map.1, pre),
            Some(SherwoodButton::DisplayCampaignMap)
        );
    }

    #[test]
    fn hit_test_misses_outside_rects() {
        let sprites = SherwoodButtonSprites::default();
        let layout = SherwoodHudLayout::for_resolution(800, 600, &sprites);
        assert!(
            layout
                .hit_test(0, 0, SherwoodButtonEnable::post_commit())
                .is_none()
        );
    }
}
