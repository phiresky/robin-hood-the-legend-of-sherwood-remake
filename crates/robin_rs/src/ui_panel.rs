//! Bottom UI panel rendering — portraits, minimap frame, and action buttons.
//!
//! The panel is composited from multiple overlapping widget bitmaps; this
//! module renders character portraits loaded from resource files with
//! selection highlighting and health bars.
//!
//! Layout reference:
//! - 5 portrait slots across the bottom, 32px margin on each side
//! - Each portrait: 112px wide, stacked vertically from bottom:
//!   border(3) + bottom_scroll(23) + actions(35) + visage(50) + top_scroll(23)
//! - Minimap button at top-right of panel area
//!
//! The `PANNEL_HEIGHT` used by the engine camera (130px in engine.rs) represents
//! the full UI chrome height including the panel and its transition zone.

use crate::Host;
use std::collections::HashMap;

use crate::element::Entity;
use crate::geo2d::GeoPoint2D;
use crate::gfx_types::Rect;
use crate::profiles::Action;
use crate::renderer::{BLIT_SOURCE_TRANSPARENT, Renderer};
use crate::resource_manager::{ResourceId, ResourceManager};
use crate::widget::requirements::{RequirementSlot, RequirementStatus};
use robin_assets::picture::Picture;
use robin_engine::character_kind::CharacterKind;
use robin_engine::coordinates::ScreenPoint;
use robin_engine::engine::Engine;
use robin_engine::player_command::PlayerId;
use robin_engine::sprite::BBox;

// ─── Layout constants ─────────────────────────────────────────────

/// Number of portrait slots across the panel.
const NUMBER_OF_SLOTS: u16 = 5;

/// Horizontal margin on each side of the portrait area (pixels).
const MARGIN: u16 = 32;

/// Width of a single portrait element (pixels).
const ELEMENT_WIDTH: u16 = 112;

/// Border gap at the very bottom of the screen.
const BORDURE: u16 = 3;

// Vertical heights of portrait sub-elements (open state).
const BOTTOM_SCROLL_HEIGHT: u16 = 23;
const ACTION_HEIGHT: u16 = 35;
const VISAGE_HEIGHT: u16 = 50;
const TOP_SCROLL_HEIGHT: u16 = 23;

/// Total height of a fully open portrait widget.
const PORTRAIT_TOTAL_HEIGHT: u16 =
    BORDURE + BOTTOM_SCROLL_HEIGHT + ACTION_HEIGHT + VISAGE_HEIGHT + TOP_SCROLL_HEIGHT;

// Vertical positions measured from the bottom of the screen.
// Open state (selected PCs) — full layout with action buttons.
const POSITION_BOTTOM_SCROLL: u16 = BORDURE + BOTTOM_SCROLL_HEIGHT;
const POSITION_ACTION: u16 = POSITION_BOTTOM_SCROLL + ACTION_HEIGHT;
const POSITION_VISAGE: u16 = POSITION_ACTION + VISAGE_HEIGHT;
const POSITION_TOP_SCROLL: u16 = POSITION_VISAGE + TOP_SCROLL_HEIGHT;

// Closed state (non-selected PCs) — no action buttons, scrolls compressed.
const CLOSE_POSITION_BOTTOM_SCROLL: u16 = POSITION_BOTTOM_SCROLL;
const CLOSE_POSITION_VISAGE: u16 = CLOSE_POSITION_BOTTOM_SCROLL + VISAGE_HEIGHT;
const CLOSE_POSITION_TOP_SCROLL: u16 = CLOSE_POSITION_VISAGE + TOP_SCROLL_HEIGHT;

// Action button widths (3-button mode).
const ACTION1_WIDTH: u16 = 40;
const ACTION2_WIDTH: u16 = 32;
const ACTION3_WIDTH: u16 = 40;

// Action button widths (2-button mode — peasants whose third action is NoAction).
const ACTIONA_WIDTH: u16 = 56;
const ACTIONB_WIDTH: u16 = 56;

// Quick-action slot icon strip — each icon is 33 px wide, placed 20 px above
// the upper scroll top.
const QA_ICON_WIDTH: u16 = 33;
/// Height of the QA icon strip above the upper scroll.
const QA_ICON_HEIGHT: u16 = 20;
/// Cast of [`crate::macro_store::NUMBER_OF_QA_MEMORY`] for the draw loop.
const NUMBER_OF_QA_MEMORY_U16: u16 = crate::macro_store::NUMBER_OF_QA_MEMORY as u16;

// ─── Colors (RGB565) ───────────────────────────────────────────────

/// Fallback fill for the visage slot when the portrait sprite fails to load.
fn color_visage_fill() -> u16 {
    Renderer::create_color_16(50, 40, 30)
}

/// Fallback fill for an action button slot when its icon sprite is missing.
fn color_action_fill() -> u16 {
    Renderer::create_color_16(40, 50, 35)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionButtonVisual {
    Disabled,
    Normal,
    Hover,
    Pressed,
    HoverPressed,
}

const ACTION_SUB_ID_DISABLED: usize = 0;
const ACTION_SUB_ID_UNSELECTED: usize = 1;
const ACTION_SUB_ID_FOCUSED: usize = 2;
const ACTION_SUB_ID_SELECTED: usize = 3;
const ACTION_SUB_ID_FOCUSED_SELECTED: usize = 4;

fn action_button_visual(
    is_active: bool,
    is_disabled: bool,
    is_hovered: bool,
) -> ActionButtonVisual {
    if is_disabled {
        ActionButtonVisual::Disabled
    } else if is_active && is_hovered {
        ActionButtonVisual::HoverPressed
    } else if is_active {
        ActionButtonVisual::Pressed
    } else if is_hovered {
        ActionButtonVisual::Hover
    } else {
        ActionButtonVisual::Normal
    }
}

use crate::resource_ids;

// ─── Scroll decoration resource IDs ───────────────────────────────
// These are generic parchment frame bitmaps shared by all portrait widgets.

/// Top scroll parchment banner (character name area).
const RHID_TOP_SCROLL: ResourceId = resource_ids::RHID_TOP_SCROLL;
/// Top scroll alternate (HP gauge overlay).
const RHID_TOP_SCROLL_ALTERNATE: ResourceId = resource_ids::RHID_TOP_SCROLL_ALTERNATE;
/// Bottom scroll parchment banner (ammo count area).
const RHID_BOTTOM_SCROLL: ResourceId = resource_ids::RHID_BOTTOM_SCROLL;

// ─── Panel border frame resource IDs ─────────────────────────────
// These form the ornamental frame around the bottom panel area.

const RHID_TOP_LEFT_CORNER: ResourceId = resource_ids::RHID_TOP_LEFT_CORNER;
const RHID_TOP_RIGHT_CORNER: ResourceId = resource_ids::RHID_TOP_RIGHT_CORNER;
const RHID_BOTTOM_LEFT_CORNER: ResourceId = resource_ids::RHID_BOTTOM_LEFT_CORNER;
const RHID_BOTTOM_RIGHT_CORNER: ResourceId = resource_ids::RHID_BOTTOM_RIGHT_CORNER;
const RHID_MIDDLE_800: ResourceId = resource_ids::RHID_MIDDLE_800;
const RHID_MIDDLE_1024: ResourceId = resource_ids::RHID_MIDDLE_1024;

// Border piece dimensions are derived from the bitmap surface sizes at runtime;
// the renderer auto-fits its bounding box to the resource size.

// Portrait resource IDs are pulled directly from `resource_ids` below.

// ─── Action button resource IDs ─────────────────────────────────

// ─── Localized name string resource IDs ────────────────────────

/// Resource ID for the menu text string table (campaign version).
pub(crate) const MENU_TEXT_TABLE_ID: ResourceId = 1000507;
/// Alternate menu text table IDs for demo versions.
pub(crate) const MENU_TEXT_TABLE_ID_DEMO: ResourceId = 1000040;
pub(crate) const MENU_TEXT_TABLE_ID_DEMO2: ResourceId = 1000034;

// ─── Portrait cache ───────────────────────────────────────────────

/// Pre-loaded portrait renderer surfaces and action button icons, keyed by [`CharacterKind`].
///
/// Loaded once at mission start from `Data/Interface/DEFAULT.RES`, then
/// passed to [`draw_panel`] each frame.  The per-character arrays are
/// indexed via `CharacterKind::as_index()` (`CharacterKind::COUNT`
/// slots).
pub struct PortraitCache {
    /// Renderer surface id for each character's face portrait.
    surfaces: [Option<u32>; CharacterKind::COUNT],
    /// `[action1, action2, action3]` renderer surface ids per character
    /// (disabled state, sub_id 0).
    action_disabled_surfaces: [Option<[Option<u32>; 3]>; CharacterKind::COUNT],
    /// `[action1, action2, action3]` renderer surface ids per character
    /// (normal state, sub_id 1).
    action_surfaces: [Option<[Option<u32>; 3]>; CharacterKind::COUNT],
    /// `[action1, action2, action3]` renderer surface ids per character
    /// (focused/hover state, sub_id 2).
    action_hover_surfaces: [Option<[Option<u32>; 3]>; CharacterKind::COUNT],
    /// `[action1, action2, action3]` renderer surface ids per character
    /// (pressed/selected state, sub_id 3).  Used to highlight the
    /// currently active action button.
    action_pressed_surfaces: [Option<[Option<u32>; 3]>; CharacterKind::COUNT],
    /// `[action1, action2, action3]` renderer surface ids per character
    /// (focused selected state, sub_id 4).
    action_hover_pressed_surfaces: [Option<[Option<u32>; 3]>; CharacterKind::COUNT],
    /// Localized display name per character.
    localized_names: [Option<String>; CharacterKind::COUNT],
    /// Generic scroll decoration surfaces (shared by all portraits).
    top_scroll_surface: Option<u32>,
    top_scroll_alt_surface: Option<u32>,
    bottom_scroll_surface: Option<u32>,
    /// Panel border frame pieces.
    border_top_left: Option<u32>,
    border_top_right: Option<u32>,
    border_bottom_left: Option<u32>,
    border_bottom_right: Option<u32>,
    border_middle: Option<u32>,
    /// Fighting sword overlay surface per character.
    fighting_surfaces: [Option<u32>; CharacterKind::COUNT],
    /// Guard indicator surface (RHID_GUARD=209).
    guard_surface: Option<u32>,
    /// Trumpet/reinforcement indicator surface (RHID_TRUMPET=224).
    trumpet_surface: Option<u32>,
    /// Amulet/clover indicator surface (RHID_CLOVER=165).
    /// Shown in burned state when PC is NOT guarded (player can click to revive).
    amulet_surface: Option<u32>,
    /// Pixel-level hit mask for the top scroll surface.
    /// Used to reject clicks on transparent curved parchment edges.
    top_scroll_hit_mask: Option<crate::minimap::HitMask>,
    /// Quick-action slot icon (RHID_QUICKACTION, shared by all QA slots).
    qa_icon_surface: Option<u32>,
    /// Quick-action slot icon while recording (RHID_QUICKACTION_IN_PROGRESS).
    qa_icon_recording_surface: Option<u32>,
    /// PC-info popup backgrounds (RHID_INFO_POPUP_BKGND_{TINY,HUGE}).
    info_popup_bg_tiny: Option<u32>,
    info_popup_bg_huge: Option<u32>,
    /// PC-info popup pip sprites (RHID_INFO_POPUP_SWORD / BOW).  We blit
    /// the "on" pip for the first `n` slots and skip the rest.
    info_popup_sword: Option<u32>,
    info_popup_bow: Option<u32>,
    /// Blazon bar icon strip — sub_ids: 0 = empty, 1 = normal (won),
    /// 2 = castle (to-collect).  We load the tiny set (used when the bar
    /// is the thin top strip).
    blazon_tiny_empty: Option<u32>,
    blazon_tiny_normal: Option<u32>,
    blazon_tiny_castle: Option<u32>,
    /// Per-(resource, sub_id) surface cache for resources that carry a
    /// table of sub-pictures indexed by character profile / action
    /// (requirements bar per-slot icons).  Pre-loaded at level load so the
    /// HUD can blit any `(res_id, sub_id)` without holding a
    /// `ResourceManager` borrow across the draw path.
    sub_pictures: HashMap<(ResourceId, usize), u32>,
    /// `RHID_YES_NO` status overlay — sub_id 0 = yes (green tick),
    /// sub_id 1 = no (red cross).
    req_yes: Option<u32>,
    req_no: Option<u32>,
    /// `RHID_SELECTED_ACTION` overlay marker used to highlight the
    /// currently-selected slot on the requirements bar.
    req_selected: Option<u32>,
}

impl Default for PortraitCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PortraitCache {
    pub fn new() -> Self {
        Self {
            surfaces: [None; CharacterKind::COUNT],
            action_disabled_surfaces: [None; CharacterKind::COUNT],
            action_surfaces: [None; CharacterKind::COUNT],
            action_hover_surfaces: [None; CharacterKind::COUNT],
            action_pressed_surfaces: [None; CharacterKind::COUNT],
            action_hover_pressed_surfaces: [None; CharacterKind::COUNT],
            localized_names: [const { None }; CharacterKind::COUNT],
            top_scroll_surface: None,
            top_scroll_alt_surface: None,
            bottom_scroll_surface: None,
            border_top_left: None,
            border_top_right: None,
            border_bottom_left: None,
            border_bottom_right: None,
            border_middle: None,
            fighting_surfaces: [None; CharacterKind::COUNT],
            guard_surface: None,
            trumpet_surface: None,
            amulet_surface: None,
            top_scroll_hit_mask: None,
            qa_icon_surface: None,
            qa_icon_recording_surface: None,
            info_popup_bg_tiny: None,
            info_popup_bg_huge: None,
            info_popup_sword: None,
            info_popup_bow: None,
            blazon_tiny_empty: None,
            blazon_tiny_normal: None,
            blazon_tiny_castle: None,
            sub_pictures: HashMap::new(),
            req_yes: None,
            req_no: None,
            req_selected: None,
        }
    }

    /// Load portrait pictures for all known characters.
    ///
    /// Reads each portrait resource from the resource manager, converts
    /// to a renderer surface. Missing resources are logged and skipped.
    pub fn load(&mut self, res: &mut ResourceManager, renderer: &mut Renderer) {
        for kind in CharacterKind::VARIANTS {
            let slot = kind.as_index();
            let res_id = kind.portrait_resource();

            // Portrait resources are BTTN type with bitmask 0b1110;
            // sub_id 0 is absent, sub_id 1 is the default portrait.
            match res.get_picture(res_id, 1) {
                Ok(pic) => {
                    let surface_id = pic_to_surface(renderer, pic);
                    tracing::info!(
                        "Loaded portrait for {:?}: resource {res_id}, surface {surface_id} ({}x{})",
                        kind,
                        pic.width,
                        pic.height,
                    );
                    self.surfaces[slot] = Some(surface_id);
                }
                Err(e) => {
                    tracing::warn!("Failed to load portrait for {kind:?} (resource {res_id}): {e}",);
                }
            }
        }

        tracing::info!(
            "Portrait cache: {} surfaces loaded",
            self.surfaces.iter().filter(|s| s.is_some()).count(),
        );

        // ── Load scroll decoration surfaces (generic, shared by all portraits) ──
        for (res_id, field, label) in [
            (
                RHID_TOP_SCROLL,
                &mut self.top_scroll_surface as &mut Option<u32>,
                "top scroll",
            ),
            (
                RHID_TOP_SCROLL_ALTERNATE,
                &mut self.top_scroll_alt_surface,
                "top scroll alt",
            ),
            (
                RHID_BOTTOM_SCROLL,
                &mut self.bottom_scroll_surface,
                "bottom scroll",
            ),
        ] {
            match res.get_picture(res_id, 1) {
                Ok(pic) => {
                    // Build pixel-level hit mask for the top scroll so clicks on
                    // transparent curved parchment edges fall through.
                    if res_id == RHID_TOP_SCROLL {
                        let pixels: Vec<u16> = pic
                            .data
                            .chunks_exact(2)
                            .map(|c| u16::from_le_bytes([c[0], c[1]]))
                            .collect();
                        let tc = crate::renderer::TRANSPARENT_COLOR_KEY_16;
                        self.top_scroll_hit_mask = Some(crate::minimap::HitMask::from_pixels_u16(
                            pic.width, pic.height, &pixels, tc,
                        ));
                        tracing::info!("Built top scroll hit mask ({}x{})", pic.width, pic.height);
                    }

                    let sid = pic_to_surface(renderer, pic);
                    tracing::info!(
                        "Loaded {label}: resource {res_id}, surface {sid} ({}x{})",
                        pic.width,
                        pic.height,
                    );
                    *field = Some(sid);
                }
                Err(e) => {
                    tracing::warn!("Failed to load {label} (resource {res_id}): {e}");
                }
            }
        }

        // ── Load panel border frame pieces ──
        // Choose the center piece based on screen width (800 vs 1024).
        let middle_id = if renderer.screen_width() >= 1024 {
            RHID_MIDDLE_1024
        } else {
            RHID_MIDDLE_800
        };
        for (res_id, field, label) in [
            (
                RHID_TOP_LEFT_CORNER,
                &mut self.border_top_left as &mut Option<u32>,
                "border top-left",
            ),
            (
                RHID_TOP_RIGHT_CORNER,
                &mut self.border_top_right,
                "border top-right",
            ),
            (
                RHID_BOTTOM_LEFT_CORNER,
                &mut self.border_bottom_left,
                "border bottom-left",
            ),
            (
                RHID_BOTTOM_RIGHT_CORNER,
                &mut self.border_bottom_right,
                "border bottom-right",
            ),
            (middle_id, &mut self.border_middle, "border middle"),
        ] {
            match res.get_picture(res_id, 0) {
                Ok(pic) => {
                    let sid = pic_to_surface(renderer, pic);
                    tracing::info!(
                        "Loaded {label}: resource {res_id}, surface {sid} ({}x{})",
                        pic.width,
                        pic.height,
                    );
                    *field = Some(sid);
                }
                Err(e) => {
                    tracing::warn!("Failed to load {label} (resource {res_id}): {e}");
                }
            }
        }

        // ── Load action button icons (normal + focused + pressed states) ──
        for kind in CharacterKind::VARIANTS {
            let slot = kind.as_index();
            let action_res_ids = kind.action_resources();
            let mut disabled = [None; 3];
            let mut surfaces = [None; 3];
            let mut hover = [None; 3];
            let mut pressed = [None; 3];
            let mut hover_pressed = [None; 3];
            for (i, opt_id) in action_res_ids.iter().enumerate() {
                if let Some(res_id) = opt_id {
                    // Action button BTTN resources follow SBResourceWidgetID:
                    // radio 0 = disabled, 1 = unselected, 2 = focused,
                    // 3 = selected, 4 = focused selected.
                    match res.get_picture(*res_id, ACTION_SUB_ID_DISABLED) {
                        Ok(pic) => {
                            disabled[i] = Some(pic_to_surface(renderer, pic));
                        }
                        Err(_) => {
                            // Fallback: disabled surface unavailable, will use normal.
                        }
                    }
                    match res.get_picture(*res_id, ACTION_SUB_ID_UNSELECTED) {
                        Ok(pic) => {
                            let surface_id = pic_to_surface(renderer, pic);
                            tracing::info!(
                                "Loaded action icon for {kind:?} action {i}: resource {res_id}, surface {surface_id} ({}x{})",
                                pic.width,
                                pic.height,
                            );
                            surfaces[i] = Some(surface_id);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to load action icon for {kind:?} action {i} (resource {res_id}): {e}"
                            );
                        }
                    }
                    // Focused/hover state (sub_id 2).
                    match res.get_picture(*res_id, ACTION_SUB_ID_FOCUSED) {
                        Ok(pic) => {
                            hover[i] = Some(pic_to_surface(renderer, pic));
                        }
                        Err(_) => {
                            // Fallback: hover surface unavailable, will use normal.
                        }
                    }
                    // Pressed/selected state (sub_id 3).
                    match res.get_picture(*res_id, ACTION_SUB_ID_SELECTED) {
                        Ok(pic) => {
                            pressed[i] = Some(pic_to_surface(renderer, pic));
                        }
                        Err(_) => {
                            // Fallback: pressed surface unavailable, will use normal
                        }
                    }
                    // Focused selected state. C++ portrait actions are
                    // `RHWidgetRadioButton`, which uses classic radio state
                    // ids even when the backing resource contains seven RDO
                    // frames.
                    match res.get_picture(*res_id, ACTION_SUB_ID_FOCUSED_SELECTED) {
                        Ok(pic) => {
                            hover_pressed[i] = Some(pic_to_surface(renderer, pic));
                        }
                        Err(_) => {
                            // Fallback: focused selected surface unavailable.
                        }
                    }
                }
            }
            self.action_disabled_surfaces[slot] = Some(disabled);
            self.action_surfaces[slot] = Some(surfaces);
            self.action_hover_surfaces[slot] = Some(hover);
            self.action_pressed_surfaces[slot] = Some(pressed);
            self.action_hover_pressed_surfaces[slot] = Some(hover_pressed);
        }
        tracing::info!(
            "Portrait cache: {} action icon sets loaded",
            self.action_surfaces.iter().filter(|s| s.is_some()).count(),
        );

        // ── Load fighting sword overlay surfaces (per character) ──
        for kind in CharacterKind::VARIANTS {
            let slot = kind.as_index();
            let res_id = kind.fighting_resource();
            // Fighting overlays are PICT type; sub_id 0 is the default picture.
            match res.get_picture(res_id, 0) {
                Ok(pic) => {
                    let sid = pic_to_surface(renderer, pic);
                    tracing::info!(
                        "Loaded fighting overlay for {kind:?}: resource {res_id}, surface {sid} ({}x{})",
                        pic.width,
                        pic.height,
                    );
                    self.fighting_surfaces[slot] = Some(sid);
                }
                Err(_) => {
                    // Try sub_id 1 as fallback (some resources use BTTN layout)
                    if let Ok(pic) = res.get_picture(res_id, 1) {
                        let sid = pic_to_surface(renderer, pic);
                        self.fighting_surfaces[slot] = Some(sid);
                    }
                }
            }
        }
        tracing::info!(
            "Portrait cache: {} fighting overlays loaded",
            self.fighting_surfaces
                .iter()
                .filter(|s| s.is_some())
                .count(),
        );

        // ── Load guard and trumpet indicator surfaces ──
        for (res_id, field, label) in [
            (
                resource_ids::RHID_GUARD,
                &mut self.guard_surface as &mut Option<u32>,
                "guard indicator",
            ),
            (
                resource_ids::RHID_TRUMPET,
                &mut self.trumpet_surface,
                "trumpet indicator",
            ),
            (
                resource_ids::RHID_CLOVER,
                &mut self.amulet_surface,
                "amulet/clover indicator",
            ),
        ] {
            // Try sub_id 0 first, then sub_id 1
            let pic = match res.get_picture(res_id, 0) {
                Ok(p) => Ok(p),
                Err(_) => res.get_picture(res_id, 1),
            };
            match pic {
                Ok(pic) => {
                    let sid = pic_to_surface(renderer, pic);
                    tracing::info!(
                        "Loaded {label}: resource {res_id}, surface {sid} ({}x{})",
                        pic.width,
                        pic.height,
                    );
                    *field = Some(sid);
                }
                Err(e) => {
                    tracing::warn!("Failed to load {label} (resource {res_id}): {e}");
                }
            }
        }

        // ── Load QA icon surfaces (RHID_QUICKACTION / _IN_PROGRESS) ──
        // RHID_QUICKACTION is the normal icon and RHID_QUICKACTION_IN_PROGRESS
        // is the recording-alternate.  Shared across all PCs and all three slots.
        for (res_id, field, label) in [
            (
                resource_ids::RHID_QUICKACTION,
                &mut self.qa_icon_surface as &mut Option<u32>,
                "QA icon",
            ),
            (
                resource_ids::RHID_QUICKACTION_IN_PROGRESS,
                &mut self.qa_icon_recording_surface,
                "QA icon (recording)",
            ),
        ] {
            let pic = match res.get_picture(res_id, 1) {
                Ok(p) => Ok(p),
                Err(_) => res.get_picture(res_id, 0),
            };
            match pic {
                Ok(pic) => {
                    let sid = pic_to_surface(renderer, pic);
                    tracing::info!(
                        "Loaded {label}: resource {res_id}, surface {sid} ({}x{})",
                        pic.width,
                        pic.height,
                    );
                    *field = Some(sid);
                }
                Err(e) => {
                    tracing::warn!("Failed to load {label} (resource {res_id}): {e}");
                }
            }
        }

        // ── Load PC-info popup resources (backgrounds + pips) ──
        // Backgrounds and pip rows both live at sub_id 0.  We blit one pip
        // per lit slot rather than maintaining widget visibility flags.
        for (res_id, field, label) in [
            (
                resource_ids::RHID_INFO_POPUP_BKGND_TINY,
                &mut self.info_popup_bg_tiny as &mut Option<u32>,
                "info popup bg (tiny)",
            ),
            (
                resource_ids::RHID_INFO_POPUP_BKGND_HUGE,
                &mut self.info_popup_bg_huge,
                "info popup bg (huge)",
            ),
            (
                resource_ids::RHID_INFO_POPUP_SWORD,
                &mut self.info_popup_sword,
                "info popup sword pip",
            ),
            (
                resource_ids::RHID_INFO_POPUP_BOW,
                &mut self.info_popup_bow,
                "info popup bow pip",
            ),
        ] {
            let pic = match res.get_picture(res_id, 0) {
                Ok(p) => Ok(p),
                Err(_) => res.get_picture(res_id, 1),
            };
            match pic {
                Ok(pic) => {
                    let sid = pic_to_surface(renderer, pic);
                    tracing::info!(
                        "Loaded {label}: resource {res_id}, surface {sid} ({}x{})",
                        pic.width,
                        pic.height,
                    );
                    *field = Some(sid);
                }
                Err(e) => {
                    tracing::warn!("Failed to load {label} (resource {res_id}): {e}");
                }
            }
        }

        // ── Load blazon-bar icons (tiny set) ──
        // `RHID_BLAZON_TINY` carries 3 sub-pictures: 0 = empty, 1 = normal
        // (won), 2 = castle (to-collect).  The tiny set is the default
        // layout on 800+ width panels.
        for (sub_id, field, label) in [
            (
                0usize,
                &mut self.blazon_tiny_empty as &mut Option<u32>,
                "blazon tiny empty",
            ),
            (1, &mut self.blazon_tiny_normal, "blazon tiny normal"),
            (2, &mut self.blazon_tiny_castle, "blazon tiny castle"),
        ] {
            match res.get_picture(resource_ids::RHID_BLAZON_TINY, sub_id) {
                Ok(pic) => {
                    let sid = pic_to_surface(renderer, pic);
                    tracing::info!(
                        "Loaded {label}: resource {} sub {sub_id}, surface {sid} ({}x{})",
                        resource_ids::RHID_BLAZON_TINY,
                        pic.width,
                        pic.height,
                    );
                    *field = Some(sid);
                }
                Err(e) => {
                    tracing::warn!("Failed to load {label}: {e}");
                }
            }
        }

        // ── Load requirements-bar status overlays (yes/no, selected) ──
        // `RHID_YES_NO` has sub 0 = yes tick, sub 1 = no cross.
        for (res_id, sub_id, field, label) in [
            (
                resource_ids::RHID_YES_NO,
                0usize,
                &mut self.req_yes as &mut Option<u32>,
                "requirements yes overlay",
            ),
            (
                resource_ids::RHID_YES_NO,
                1,
                &mut self.req_no,
                "requirements no overlay",
            ),
            (
                resource_ids::RHID_SELECTED_ACTION,
                0,
                &mut self.req_selected,
                "requirements selected overlay",
            ),
        ] {
            match res.get_picture(res_id, sub_id) {
                Ok(pic) => {
                    let sid = pic_to_surface(renderer, pic);
                    tracing::info!(
                        "Loaded {label}: resource {res_id} sub {sub_id}, surface {sid} ({}x{})",
                        pic.width,
                        pic.height,
                    );
                    *field = Some(sid);
                }
                Err(e) => {
                    tracing::warn!("Failed to load {label}: {e}");
                }
            }
        }

        // ── Pre-load all per-slot sub-pictures of the requirements-bar
        //    icon tables.  Each resource carries one sub-picture per
        //    character-profile or per-action enum value.  Loading the full
        //    table here lets `draw_requirements_bar` blit `(res_id, sub_id)`
        //    without ever re-borrowing the `ResourceManager` at render time.
        for res_id in [
            resource_ids::RHID_REQUIRED_PC,
            resource_ids::RHID_REQUIRED_ACTION,
            resource_ids::RHID_OPTIONAL_PC,
        ] {
            // Collect first to avoid re-borrowing `res` inside the loop.
            let subs: Vec<(usize, u16, u16, Vec<u16>)> = match res.get_pictures(res_id) {
                Ok(pics) => pics
                    .iter()
                    .enumerate()
                    .filter_map(|(i, opt)| {
                        opt.as_ref().map(|pic| {
                            let pixels: Vec<u16> = pic
                                .data
                                .chunks_exact(2)
                                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                                .collect();
                            (i, pic.width, pic.height, pixels)
                        })
                    })
                    .collect(),
                Err(e) => {
                    tracing::warn!("Failed to load sub-pictures for resource {res_id}: {e}");
                    continue;
                }
            };
            for (sub_id, w, h, pixels) in subs {
                let surface_id = renderer
                    .create_surface_from_rgb565(w, h, &pixels)
                    .expect("requirements sub-picture dimensions must match RGB565 payload");
                tracing::debug!(
                    "Loaded requirements sub-picture: res {res_id} sub {sub_id}, surface {surface_id} ({w}x{h})"
                );
                self.sub_pictures.insert((res_id, sub_id), surface_id);
            }
        }
    }

    /// Install a pre-loaded localized-name map.  Read at render time
    /// via [`Self::get_localized_name`] and by the peasant-name
    /// generator.  [`load_localized_character_names`] builds the map
    /// from `Level.res`.
    pub fn install_localized_names(&mut self, names: [Option<String>; CharacterKind::COUNT]) {
        self.localized_names = names;
    }

    //
    // Determinism: the resulting names are persisted to
    // `campaign.peasant_names` via `RegisterPeasantName`, which is
    // part of the rollback / replay hash.  Must NOT use ambient
    // `rand::rng()` (seeded from OS entropy) — that causes a
    // different peasant-name set every run, producing a frame-0
    // replay desync.  Instead seed a throwaway `fastrand::Rng` from
    // the engine's current seed so both recording and replay pick
    // the same names without advancing the sim RNG.
    pub fn generate_peasant_names(
        &mut self,
        res: &mut ResourceManager,
        engine: &mut robin_engine::engine::Engine,
        display: &mut robin_engine::engine::HostDisplayState,
        input: &mut robin_engine::engine::InputState,
        assets: &robin_engine::engine::LevelAssets,
    ) {
        const FIRSTNAME_BASE: usize = 100;
        const SURNAME_BASE: usize = 122;
        const NAME_COUNT: usize = 22;
        const MAX_ATTEMPTS: usize = 10;

        let table_ids = [
            MENU_TEXT_TABLE_ID,
            MENU_TEXT_TABLE_ID_DEMO,
            MENU_TEXT_TABLE_ID_DEMO2,
        ];
        let peasants = [
            CharacterKind::MerryManA,
            CharacterKind::MerryManB,
            CharacterKind::MerryManC,
        ];

        // Helper: fetch a string from the first available table.
        let get_str = |res: &mut ResourceManager, sub_id: usize| -> Option<String> {
            for &tid in &table_ids {
                if let Ok(s) = res.get_string(tid, sub_id) {
                    return Some(s.to_string());
                }
            }
            None
        };

        // Pre-load all available first/last names.
        let firstnames: Vec<String> = (0..NAME_COUNT)
            .filter_map(|i| get_str(res, FIRSTNAME_BASE + i))
            .collect();
        let surnames: Vec<String> = (0..NAME_COUNT)
            .filter_map(|i| get_str(res, SURNAME_BASE + i))
            .collect();

        if firstnames.is_empty() || surnames.is_empty() {
            tracing::warn!(
                "Peasant name generation: no firstname/surname strings found ({}/{})",
                firstnames.len(),
                surnames.len(),
            );
            return;
        }

        #[allow(clippy::disallowed_methods)]
        let mut rng = fastrand::Rng::with_seed(engine.rng_seed());
        for kind in peasants {
            let slot = kind.as_index();
            if self.localized_names[slot].is_some() {
                continue;
            }

            let mut generated = None;
            for _ in 0..MAX_ATTEMPTS {
                let first = &firstnames[rng.usize(0..firstnames.len())];
                let last = &surnames[rng.usize(0..surnames.len())];
                let full = format!("{first} {last}");

                if !engine.is_peasant_name_registered(&full) {
                    engine.apply_command(
                        display,
                        input,
                        assets,
                        &robin_engine::player_command::PlayerCommand::RegisterPeasantName {
                            name: full.clone(),
                        },
                    );
                    generated = Some(full);
                    break;
                }
            }

            let display_name = generated.unwrap_or_else(|| "Misteryman".to_string());
            tracing::info!("Peasant {kind:?} → {display_name:?}");
            self.localized_names[slot] = Some(display_name);
        }
    }

    /// Look up the renderer surface for a character's face portrait.
    pub fn get_surface(&self, kind: CharacterKind) -> Option<u32> {
        self.surfaces[kind.as_index()]
    }

    /// Look up the action button surfaces for a character.
    pub fn get_action_disabled_surfaces(&self, kind: CharacterKind) -> Option<&[Option<u32>; 3]> {
        self.action_disabled_surfaces[kind.as_index()].as_ref()
    }

    /// Look up the action button surfaces for a character.
    pub fn get_action_surfaces(&self, kind: CharacterKind) -> Option<&[Option<u32>; 3]> {
        self.action_surfaces[kind.as_index()].as_ref()
    }

    /// Look up the focused/hover action button surfaces for a character.
    pub fn get_action_hover_surfaces(&self, kind: CharacterKind) -> Option<&[Option<u32>; 3]> {
        self.action_hover_surfaces[kind.as_index()].as_ref()
    }

    /// Look up the pressed/selected action button surfaces for a character.
    pub fn get_action_pressed_surfaces(&self, kind: CharacterKind) -> Option<&[Option<u32>; 3]> {
        self.action_pressed_surfaces[kind.as_index()].as_ref()
    }

    /// Look up the focused selected action button surfaces for a character.
    pub fn get_action_hover_pressed_surfaces(
        &self,
        kind: CharacterKind,
    ) -> Option<&[Option<u32>; 3]> {
        self.action_hover_pressed_surfaces[kind.as_index()].as_ref()
    }

    /// Look up the fighting sword overlay surface for a character.
    pub fn get_fighting_surface(&self, kind: CharacterKind) -> Option<u32> {
        self.fighting_surfaces[kind.as_index()]
    }

    /// Look up the localized display name for a character.
    pub fn get_localized_name(&self, kind: CharacterKind) -> Option<&str> {
        self.localized_names[kind.as_index()].as_deref()
    }

    /// True if at least one portrait has been loaded.
    pub fn is_loaded(&self) -> bool {
        self.surfaces.iter().any(|s| s.is_some())
    }

    /// Look up a pre-loaded `(resource_id, sub_id)` surface.
    ///
    /// Populated at [`PortraitCache::load`] time for the requirements-bar
    /// icon tables (`RHID_REQUIRED_PC` / `RHID_REQUIRED_ACTION` /
    /// `RHID_OPTIONAL_PC`).
    pub fn get_sub_picture(&self, res_id: ResourceId, sub_id: usize) -> Option<u32> {
        self.sub_pictures.get(&(res_id, sub_id)).copied()
    }
}

// ─── Requirements-bar per-slot sub-picture mapping ────────────────

/// Sub-picture index within `RHID_REQUIRED_ACTION` for a required action.
///
/// Returns the `UnknownAction=0` fallback for actions the widget does
/// not visualise.
pub(crate) fn required_action_sub_id(action: crate::profiles::Action) -> usize {
    // Sub-id mapping: UnknownAction=0, Bow=1, Carry=2, Climb=3, Jump=4,
    //                 Lever=5, Lockpick=6, Stun=7, Tie=8, Eat=9, Search=10.
    use crate::profiles::Action;
    match action {
        Action::Bow => 1,
        Action::LittleJohnCarry | Action::FarmerCarry => 2,
        Action::Climb => 3,
        Action::Jump => 4,
        Action::Lever => 5,
        Action::Lockpick => 6,
        Action::Hit | Action::HitHard => 7,
        Action::Tie => 8,
        Action::Eat | Action::Guzzle => 9,
        Action::Search => 10,
        _ => 0,
    }
}

/// Upload a 16-bit picture into a new renderer surface.
pub(crate) fn pic_to_surface(renderer: &mut Renderer, pic: &Picture) -> u32 {
    let pixels: Vec<u16> = pic
        .data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    renderer
        .create_surface_from_rgb565(pic.width, pic.height, &pixels)
        .expect("pic_to_surface: decoded picture dimensions must match RGB565 payload")
}

/// The [`CharacterKind`] for a PC entity, if it is a PC whose profile
/// matched one of the 10 known characters at level-load time.
fn pc_character_kind(entity: &Entity) -> Option<CharacterKind> {
    match entity {
        Entity::Pc(pc) => pc.pc.kind,
        _ => None,
    }
}

// ─── Helpers ───────────────────────────────────────────────────────

/// Compute the pixel width of each portrait slot.
fn slot_width(screen_width: u16) -> u16 {
    (screen_width - 2 * MARGIN) / NUMBER_OF_SLOTS
}

/// Compute the left X of a portrait element within its slot.
fn slot_left_x(screen_width: u16, slot_index: u16) -> u16 {
    let sw = slot_width(screen_width);
    let position_in_slot = MARGIN + (sw - ELEMENT_WIDTH) / 2;
    slot_index * sw + position_in_slot
}

fn bbox(x1: u16, y1: u16, x2: u16, y2: u16) -> BBox {
    BBox::new(
        GeoPoint2D {
            x: x1 as f32,
            y: y1 as f32,
        },
        GeoPoint2D {
            x: x2 as f32,
            y: y2 as f32,
        },
    )
}

fn blit_to_screen_widget(
    renderer: &mut Renderer,
    surface_id: u32,
    src: Option<&BBox>,
    dst: Option<&BBox>,
    flags: u32,
) {
    let src_box = src.copied().unwrap_or_else(|| {
        BBox::new(
            GeoPoint2D { x: 0.0, y: 0.0 },
            GeoPoint2D {
                x: renderer.surface_width(surface_id) as f32,
                y: renderer.surface_height(surface_id) as f32,
            },
        )
    });
    let dst_box = dst.copied().unwrap_or(src_box);
    crate::ingame_menu::widget_bridge::draw_picture_surface_rect(
        renderer,
        crate::ingame_menu::layout::MenuTransform {
            origin_x: 0,
            origin_y: 0,
        },
        surface_id,
        dst_box.min.x as i32,
        dst_box.min.y as i32,
        dst_box.width() as i32,
        dst_box.height() as i32,
        src_box.min.x as i32,
        src_box.min.y as i32,
        src_box.width() as i32,
        src_box.height() as i32,
        flags & BLIT_SOURCE_TRANSPARENT != 0,
    );
}

/// Check if a PC is in coma state (amulet death-save, still alive but burned).
///
/// Coma PCs have `in_coma=true` in their campaign PcStatus and
/// life_points=5 (set by GetWounded). They render as burned portraits
/// with the health gauge visible. Fully dead PCs have life_points<=0
/// and are NOT in coma — their scrolls are hidden entirely.
fn is_pc_in_coma(engine: &Engine, entity: &Entity) -> bool {
    let profile_idx = match entity.pc_data() {
        Some(pc) => pc.profile_index,
        None => return false,
    };
    engine
        .campaign()
        .and_then(|c| c.characters.get(usize::from(profile_idx)))
        .map(|desc| desc.status.in_coma)
        .unwrap_or(false)
}

/// Determine which action button index (0..=2) is currently active for a PC.
///
/// Compares the PC's `current_action` against the profile's `actions[]` array.
/// Returns `None` if `current_action == NoAction` or doesn't match any slot.
fn active_action_index(
    engine: &Engine,
    profiles: &robin_engine::profiles::ProfileManager,
    entity: &Entity,
) -> Option<u8> {
    use crate::profiles::Action;
    let pc = entity.pc_data()?;
    if pc.current_action == Action::NoAction {
        return None;
    }
    engine.campaign()?;
    let profile = profiles.get_character(pc.profile_index)?;
    profile
        .actions
        .iter()
        .position(|a| *a == pc.current_action)
        .map(|i| i as u8)
}

// ─── Public API ────────────────────────────────────────────────────

/// Load the 7-hero localized display name map from `Level.res`.
///
/// Tries the campaign menu text table first, then demo variants in
/// order.  The map is fed to [`PortraitCache::install_localized_names`]
/// and consulted at render time by the HUD's `entity_display_name`
/// (PC branch) and by the peasant-name generator (to avoid colliding
/// with hero names).
pub fn load_localized_character_names(
    text_res: &mut ResourceManager,
) -> [Option<String>; CharacterKind::COUNT] {
    let table_ids = [
        MENU_TEXT_TABLE_ID,
        MENU_TEXT_TABLE_ID_DEMO,
        MENU_TEXT_TABLE_ID_DEMO2,
    ];
    let mut out: [Option<String>; CharacterKind::COUNT] = [const { None }; CharacterKind::COUNT];
    let mut loaded = 0usize;
    for kind in CharacterKind::VARIANTS {
        let str_id = match kind.localized_name_string_id() {
            Some(id) => id,
            None => continue,
        };
        for &table_id in &table_ids {
            if let Ok(localized) = text_res.get_string(table_id, str_id) {
                tracing::info!(
                    "Localized name for {kind:?}: {localized:?} (table {table_id}, sub {str_id})"
                );
                out[kind.as_index()] = Some(localized.to_string());
                loaded += 1;
                break;
            }
        }
    }
    tracing::info!("Loaded {loaded} localized character names");
    out
}

/// Render the health gauge (two-parchment composite) at the given position.
///
/// Splits the top scroll into a "live" left portion (normal parchment)
/// and a "dead" right portion (darkened parchment) at `ratio × width`.
fn render_health_gauge(
    renderer: &mut Renderer,
    portraits: &PortraitCache,
    entity: Option<&Entity>,
    x: u16,
    top: u16,
) {
    let Some(normal_sid) = portraits.top_scroll_surface else {
        return;
    };
    let w = renderer.surface_width(normal_sid);
    let h = renderer.surface_height(normal_sid);

    let ratio = match entity {
        Some(Entity::Pc(pc)) => (pc.pc.life_points.max(0) as f32 / 100.0).clamp(0.0, 1.0),
        _ => 1.0,
    };
    let split_x = (w as f32 * ratio) as u16;

    // Dead portion (right side — darkened parchment)
    if split_x < w
        && let Some(alt_sid) = portraits.top_scroll_alt_surface
    {
        let src = bbox(split_x, 0, w, h);
        let dst = bbox(x + split_x, top, x + w, top + h);
        blit_to_screen_widget(
            renderer,
            alt_sid,
            Some(&src),
            Some(&dst),
            BLIT_SOURCE_TRANSPARENT,
        );
    }
    // Live portion (left side — normal parchment)
    if split_x > 0 {
        let src = bbox(0, 0, split_x, h);
        let dst = bbox(x, top, x + split_x, top + h);
        blit_to_screen_widget(
            renderer,
            normal_sid,
            Some(&src),
            Some(&dst),
            BLIT_SOURCE_TRANSPARENT,
        );
    }
}

/// Blit a surface centered within the vertical region between two scrolls.
///
/// Centers the indicator widget within the reference box spanning from
/// upper scroll top to lower scroll bottom.
fn blit_centered_between_scrolls(
    renderer: &mut Renderer,
    surface: Option<u32>,
    x: u16,
    ref_top: u16,
    ref_bot: u16,
) {
    let Some(sid) = surface else { return };
    let iw = renderer.surface_width(sid);
    let ih = renderer.surface_height(sid);
    let ref_h = ref_bot.saturating_sub(ref_top);
    let ix = x + (ELEMENT_WIDTH.saturating_sub(iw)) / 2;
    let iy = ref_top + (ref_h.saturating_sub(ih)) / 2;
    let dst = bbox(ix, iy, ix + iw, iy + ih);
    blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
}

/// Draw the bottom UI panel to the screen surface.
///
/// This renders, in order:
/// 1. Ornamental border frame pieces (corners + middle strip) around the
///    bottom panel area.
/// 2. Portrait slots for each displayed PC — top/bottom scrolls, visage,
///    action buttons, fighting/guard/trumpet/amulet overlays, QA strip,
///    and burned-state variants for dead/coma PCs.
///
/// The minimap and its frame are drawn separately by `render_minimap`
/// (the `RHMAP_CORNER` sprite covers the slot in non-Sherwood missions).
///
/// Should be called after entity rendering and before `renderer.flip()`.
#[allow(clippy::too_many_arguments)]
pub fn draw_panel(
    host: &mut Host,
    engine: &Engine,
    local_seat: PlayerId,
    profiles: &robin_engine::profiles::ProfileManager,
    renderer: &mut Renderer,
    portraits: &PortraitCache,
    mouse_x: f32,
    mouse_y: f32,
    titbit_renderer: Option<&mut crate::titbit_renderer::TitbitRenderer>,
) {
    let sw = renderer.screen_width();
    let sh = renderer.screen_height();

    if sw == 0 || sh == 0 {
        return;
    }

    // ── Panel border frame (ornamental frame around the bottom panel) ──
    // Rendered BEFORE portrait widgets, in absolute screen coordinates.
    // Blit using source surface dimensions to avoid size mismatch issues.
    if let Some(sid) = portraits.border_top_left {
        let w = renderer.surface_width(sid);
        let h = renderer.surface_height(sid);
        let dst = bbox(0, 0, w, h);
        blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
    }
    if let Some(sid) = portraits.border_bottom_left {
        let w = renderer.surface_width(sid);
        let h = renderer.surface_height(sid);
        let dst = bbox(0, sh - h, w, sh);
        blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
    }
    if let Some(sid) = portraits.border_bottom_right {
        let w = renderer.surface_width(sid);
        let h = renderer.surface_height(sid);
        let dst = bbox(sw - w, sh - h, sw, sh);
        blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
    }
    // Center border piece — only at 800+ width (disabled at 640).
    if sw > 640
        && let Some(sid) = portraits.border_middle
    {
        let w = renderer.surface_width(sid);
        let h = renderer.surface_height(sid);
        let bx = renderer.surface_width(portraits.border_bottom_left.unwrap_or(0));
        let dst = bbox(bx, sh - h, bx + w, sh);
        blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
    }

    // ── Portrait slots (one per PC in the mission) ──
    // Hidden-interface PCs don't consume a slot, so filter on
    // `pc.interface_hidden` via `engine.displayed_pc_ids()` rather than
    // walking `pc_ids` directly.
    let displayed_pcs = engine.displayed_pc_ids();
    let num_portraits = displayed_pcs.len().min(NUMBER_OF_SLOTS as usize) as u16;
    let frame = engine.frame_counter();
    let hovered_action =
        hit_test_portrait_detailed(engine, local_seat, portraits, sw, sh, mouse_x, mouse_y)
            .and_then(|hit| match hit.area {
                PortraitHitArea::ActionButton(btn) => Some((hit.slot, btn)),
                _ => None,
            });

    let mut titbit_renderer_opt = titbit_renderer;

    for slot in 0..num_portraits {
        let x = slot_left_x(sw, slot);
        let x2 = x + ELEMENT_WIDTH;

        let pc_id = displayed_pcs[slot as usize];
        let entity = engine.get_entity(pc_id);
        let is_selected = engine.seat_selection(local_seat).contains(&pc_id);

        // ── Extract PC-specific state for overlay rendering ──
        let (is_dead, is_coma, is_sword_fighting, is_guarded, has_trumpet) = match entity {
            Some(Entity::Pc(pc)) => (
                pc.pc.life_points <= 0,
                is_pc_in_coma(engine, entity.unwrap()),
                pc.actor.action_state.is_sword(),
                pc.pc.guard.is_some(),
                pc.pc.trumpet_enabled,
            ),
            _ => (false, false, false, false, false),
        };
        // Burned = dead OR in coma (the burn path covers both).
        let is_burned = is_dead || is_coma;

        if is_burned {
            // ── BURNED STATE ──
            // Visage and action buttons hidden. Upper scroll repositioned
            // directly above lower scroll.
            // Coma PCs show health gauge + enabled scrolls;
            // fully dead PCs hide scrolls entirely.
            let burned_upper_top = sh - POSITION_BOTTOM_SCROLL - BOTTOM_SCROLL_HEIGHT;

            if is_coma {
                // Coma: scrolls enabled, health gauge visible.
                render_health_gauge(renderer, portraits, entity, x, burned_upper_top);

                if let Some(sid) = portraits.bottom_scroll_surface {
                    let w = renderer.surface_width(sid);
                    let h = renderer.surface_height(sid);
                    let top = sh - POSITION_BOTTOM_SCROLL;
                    let dst = bbox(x, top, x + w, top + h);
                    blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
                }

                // Guard indicator (centered between scrolls).
                if is_guarded {
                    let guard_visible = if engine.mission().mission_won {
                        (frame / 25).is_multiple_of(2)
                    } else {
                        true
                    };
                    if guard_visible {
                        blit_centered_between_scrolls(
                            renderer,
                            portraits.guard_surface,
                            x,
                            burned_upper_top,
                            sh - BORDURE,
                        );
                    }
                }
                // Amulet/clover indicator when NOT guarded.
                if !is_guarded {
                    blit_centered_between_scrolls(
                        renderer,
                        portraits.amulet_surface,
                        x,
                        burned_upper_top,
                        sh - BORDURE,
                    );
                }
            }
            // Fully dead (not coma): scrolls disabled, nothing rendered.
            //
            // Trumpet indicator: the trumpet only appears on dead PCs (not
            // coma). `melee.rs:3208` sets `trumpet_enabled = true` when the
            // killed PC has a non-VIP replacement available. Drawn centered
            // in the same between-scrolls region the coma amulet/guard uses.
            if has_trumpet {
                blit_centered_between_scrolls(
                    renderer,
                    portraits.trumpet_surface,
                    x,
                    burned_upper_top,
                    sh - BORDURE,
                );
            }
        } else {
            // ── NORMAL STATE ──
            let pos_top_scroll = if is_selected {
                POSITION_TOP_SCROLL
            } else {
                CLOSE_POSITION_TOP_SCROLL
            };
            let pos_visage = if is_selected {
                POSITION_VISAGE
            } else {
                CLOSE_POSITION_VISAGE
            };

            // Top scroll (health gauge)
            render_health_gauge(renderer, portraits, entity, x, sh - pos_top_scroll);

            // Bottom scroll
            if let Some(sid) = portraits.bottom_scroll_surface {
                let w = renderer.surface_width(sid);
                let h = renderer.surface_height(sid);
                let top = sh - POSITION_BOTTOM_SCROLL;
                let dst = bbox(x, top, x + w, top + h);
                blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
            }

            // Visage (face) — 1:1 blit at native surface dimensions
            let vis_top = sh - pos_visage;
            let vis_bot = if is_selected {
                sh - POSITION_ACTION
            } else {
                sh - CLOSE_POSITION_BOTTOM_SCROLL
            };

            let mut portrait_drawn = false;
            if let Some(ent) = entity
                && let Some(kind) = pc_character_kind(ent)
                && let Some(surface_id) = portraits.get_surface(kind)
            {
                let src_w = renderer.surface_width(surface_id);
                let src_h = renderer.surface_height(surface_id);
                if src_w > 0 && src_h > 0 {
                    let dst = bbox(x, vis_top, x + src_w, vis_top + src_h);
                    blit_to_screen_widget(
                        renderer,
                        surface_id,
                        None,
                        Some(&dst),
                        BLIT_SOURCE_TRANSPARENT,
                    );
                    portrait_drawn = true;
                }
            }
            if !portrait_drawn {
                renderer.fill_screen(Some(&bbox(x, vis_top, x2, vis_bot)), color_visage_fill());
            }

            // Fighting sword overlay (period=10 frames).
            // Positioned at visage top-left, per-character bitmap (91×44 px).
            // When selected the sword is always visible; otherwise it blinks
            // on the odd half of each 10-frame cycle.
            if is_sword_fighting {
                let fighting_visible = is_selected || (frame / 10).is_multiple_of(2);
                if fighting_visible
                    && let Some(ent) = entity
                    && let Some(kind) = pc_character_kind(ent)
                    && let Some(sid) = portraits.get_fighting_surface(kind)
                {
                    let fw = renderer.surface_width(sid);
                    let fh = renderer.surface_height(sid);
                    let dst = bbox(x, vis_top, x + fw, vis_top + fh);
                    blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
                }
            }

            // Action buttons — only when selected/open.
            // Switches between 3-button (40+32+40) and 2-button (56+56)
            // layout based on whether the profile's third action is NoAction.
            // We detect this from action_icons[2] being None.
            if is_selected {
                let act_top = sh - POSITION_ACTION;
                let act_bot = sh - POSITION_BOTTOM_SCROLL;

                let kind = entity.and_then(pc_character_kind);
                let action_icons = kind.and_then(|k| portraits.get_action_surfaces(k).cloned());
                let action_disabled =
                    kind.and_then(|k| portraits.get_action_disabled_surfaces(k).cloned());
                let action_hover =
                    kind.and_then(|k| portraits.get_action_hover_surfaces(k).cloned());
                let action_pressed =
                    kind.and_then(|k| portraits.get_action_pressed_surfaces(k).cloned());
                let action_hover_pressed =
                    kind.and_then(|k| portraits.get_action_hover_pressed_surfaces(k).cloned());

                let two_button_mode = action_icons
                    .as_ref()
                    .is_some_and(|icons| icons[2].is_none());

                let (btn_lefts, btn_rights, num_buttons) = if two_button_mode {
                    let a_right = x + ACTIONA_WIDTH;
                    let b_right = a_right + ACTIONB_WIDTH;
                    ([x, a_right, 0], [a_right, b_right, 0], 2)
                } else {
                    let a1_right = x + ACTION1_WIDTH;
                    let a2_right = a1_right + ACTION2_WIDTH;
                    let a3_right = a2_right + ACTION3_WIDTH;
                    ([x, a1_right, a2_right], [a1_right, a2_right, a3_right], 3)
                };

                // Determine active action button index and disabled state.
                let active_idx = entity.and_then(|e| active_action_index(engine, profiles, e));

                for i in 0..num_buttons {
                    let is_active = active_idx == Some(i as u8);
                    let is_disabled = entity.and_then(|e| e.pc_data()).is_some_and(|pc| {
                        pc.disabled_actions.get(i).copied().unwrap_or(false)
                            || pc.disabled_actions_temp.get(i).copied().unwrap_or(false)
                    });
                    let is_hovered = hovered_action == Some((slot as u8, i as u8));

                    let mut icon_drawn = false;

                    let visual = action_button_visual(is_active, is_disabled, is_hovered);
                    let visual_surface = match visual {
                        ActionButtonVisual::Disabled => action_disabled
                            .as_ref()
                            .and_then(|disabled| disabled[i])
                            .or_else(|| action_icons.as_ref().and_then(|icons| icons[i])),
                        ActionButtonVisual::HoverPressed => action_hover_pressed
                            .as_ref()
                            .and_then(|hover_pressed| hover_pressed[i])
                            .or_else(|| action_pressed.as_ref().and_then(|pressed| pressed[i])),
                        ActionButtonVisual::Pressed => {
                            action_pressed.as_ref().and_then(|pressed| pressed[i])
                        }
                        ActionButtonVisual::Hover => {
                            action_hover.as_ref().and_then(|hover| hover[i])
                        }
                        ActionButtonVisual::Normal => None,
                    }
                    .or_else(|| action_icons.as_ref().and_then(|icons| icons[i]));

                    if let Some(surface_id) = visual_surface {
                        let dst = bbox(btn_lefts[i], act_top, btn_rights[i], act_bot);
                        blit_to_screen_widget(
                            renderer,
                            surface_id,
                            None,
                            Some(&dst),
                            BLIT_SOURCE_TRANSPARENT,
                        );
                        icon_drawn = true;
                    }
                    if !icon_drawn {
                        renderer.fill_screen(
                            Some(&bbox(
                                btn_lefts[i] + 1,
                                act_top + 1,
                                btn_rights[i] - 1,
                                act_bot - 1,
                            )),
                            color_action_fill(),
                        );
                    }

                    // Disabled-state BTTN resources are already authored as
                    // grayed-out sprites.  Do not add a synthetic stipple on
                    // top: it produces visible horizontal artifacts through
                    // portrait action icons.
                }
            }

            // ── Quick-action icon strip ──
            // Positioned 20 px above the top scroll, 33 px wide each.
            // Shared icon per slot; alternate sprite while this slot is the
            // active recording target.
            let upper_top = sh - pos_top_scroll;
            let qa_strip_y = upper_top.saturating_sub(QA_ICON_HEIGHT);
            let recording_slot = if engine.is_qa_recording_for(pc_id) {
                engine
                    .macro_store()
                    .get(pc_id)
                    .and_then(|m| m.recording_slot())
            } else {
                None
            };
            for slot_idx in 0..NUMBER_OF_QA_MEMORY_U16 {
                let has_macro = engine
                    .macro_store()
                    .get(pc_id)
                    .map(|m| m.has_macro(slot_idx as usize))
                    .unwrap_or(false);
                let is_recording_slot = recording_slot == Some(slot_idx as u8);
                if !has_macro && !is_recording_slot {
                    continue;
                }

                let sid_opt = if is_recording_slot {
                    portraits
                        .qa_icon_recording_surface
                        .or(portraits.qa_icon_surface)
                } else {
                    portraits.qa_icon_surface
                };
                let Some(sid) = sid_opt else { continue };

                let icon_x = x + slot_idx * QA_ICON_WIDTH;
                let iw = renderer.surface_width(sid);
                let ih = renderer.surface_height(sid);
                let dst = bbox(icon_x, qa_strip_y, icon_x + iw, qa_strip_y + ih);
                blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);

                // Single-frame titbit sprite overlay.  Looks up the slot's
                // titbit id in the per-PC slot table and resolves to the one
                // `RHID_QUICKACTION_TITBITS` sub-frame via the titbit
                // manager's phase lookup.
                //
                // The per-slot titbit id is registered at
                // `record_macro_step_for` (`engine/commands.rs`) on the
                // first committed PlayerCommand of a recording.
                //
                // Layered on top: the falling-button refresh animation —
                // each slot tracks `shift_phase` (px) that re-arms to
                // `SHIFT_STEP` whenever the step count changes and decays
                // by `SHIFT_FALL_PER_REFRESH` each draw.  The titbit icon is
                // offset by `shift_phase` along +X to produce the slide.
                let slot_idx_usz = slot_idx as usize;
                let shift_phase = host.engine_display.macro_shift_phase(pc_id, slot_idx_usz);
                // Fizzle-blink visibility: the QA strobe toggles the per-slot
                // titbit on/off after a macro fizzles.  When blink-hidden,
                // skip the titbit blit.
                let blink_hidden = host
                    .engine_display
                    .macro_titbit_blink_hidden(pc_id, slot_idx_usz);
                if has_macro && !blink_hidden {
                    // Per-step titbit overlay: draw the `RHID_QUICKACTION_TITBITS`
                    // sub-frame for the slot's most recent step, resolved via
                    // `action_to_qa_frame(step.action)`.  Driven directly off
                    // the recorded step's `Action` rather than the transient
                    // titbit manager entry — so the overlay survives a titbit
                    // expiring or a ground-target step that never produced an
                    // `add_titbit` entry (walk/run).
                    //
                    // When the last step is an action with no dedicated
                    // `RHQUICK_*` icon (e.g. `Jump`, `Search`) we fall back
                    // to the slot's titbit phase if one is still live, so
                    // interact-only flows (`LaunchInteraction`) keep their
                    // RHQUICK_INTERRACT_PC/NPC fallback from `commands.rs`.
                    let frame_from_last_step = engine
                        .macro_store()
                        .get(pc_id)
                        .and_then(|m| m.slot(slot_idx as usize))
                        .and_then(|s| s.steps.last())
                        .and_then(|step| crate::macro_store::action_to_qa_frame(step.action));
                    let phase_from_slot_titbit = || {
                        engine
                            .macro_store()
                            .get(pc_id)
                            .and_then(|m| m.get_slot_titbit(slot_idx as usize))
                            .map(|id| engine.titbit_manager().get_phase(id))
                            .filter(|&p| p != 0xFFFF)
                    };
                    let frame = frame_from_last_step.or_else(phase_from_slot_titbit);
                    // The per-slot `run` flag carries through into the
                    // shifting-titbit renderer, which then draws a second
                    // copy of the sprite offset by `(3, 0)`.  The flag is
                    // driven by `is_running_for_qa(...)` on the slot's
                    // titbit id.
                    let run = engine
                        .macro_store()
                        .get(pc_id)
                        .and_then(|m| m.get_slot_titbit(slot_idx as usize))
                        .map(|id| engine.titbit_manager().is_running_for_qa(id))
                        .unwrap_or(false);
                    if let (Some(tbr), Some(frame)) = (titbit_renderer_opt.as_mut(), frame) {
                        let shift_px = shift_phase.round() as i32;
                        tbr.blit_ui_frame(
                            renderer,
                            crate::titbit::SpriteRow::QuickActionTitbits,
                            frame,
                            Rect::new(
                                icon_x as i32 + shift_px,
                                qa_strip_y as i32,
                                iw as u32,
                                ih as u32,
                            ),
                            run,
                        );
                    }
                }
            }

            // The trumpet widget is only enabled on death, so it never
            // appears on a living PC — nothing to draw in this branch.
        }
    }
}

// ─── Blazon bar & requirements bar icon strips ────────────────────

/// The blazon set uses three sprites per slot (normal/empty/castle).
/// Classifying a slot up-front lets the draw and tooltip paths share
/// layout + semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlazonSlotKind {
    /// Already-owned blazon.
    Normal,
    /// Un-owned slot that will be earned via Sherwood buy/convert.
    Empty,
    /// Un-owned slot that must be collected inside the mission itself.
    /// Flashes to `Normal` while the blink latch is armed.
    Castle,
}

const BLAZON_BAR_TINY_W: u16 = 9;
const BLAZON_BAR_TINY_H: u16 = 14;
const BLAZON_BAR_SPACING: u16 = 5;
const BLAZON_BAR_Y: u16 = 2;

/// Classify each blazon-bar slot:
///
/// - slots `0..owned` → `Normal`
/// - if `owned + to_be_collected < total`: middle gap → `Empty`,
///   trailing `to_be_collected` slots → `Castle`
/// - otherwise: `owned..total` → `Castle`
///
/// The `blinking` suffix flips the trailing N `Castle` slots back to
/// `Normal`.
pub fn blazon_bar_slot_kinds(
    state: &crate::widget::blazon_bar::BlazonBarState,
) -> Vec<BlazonSlotKind> {
    let owned = state.current.saturating_add(state.additional);
    let slots = state.required.max(owned);
    if slots == 0 {
        return Vec::new();
    }
    let slots_u = slots as usize;
    let owned_clamped = owned.min(slots) as usize;
    let to_be_collected = state.to_be_collected.min(slots) as usize;
    let castle_start = slots_u - to_be_collected;
    let blink_start = slots_u - (state.blinking.min(slots) as usize);

    let mut kinds = Vec::with_capacity(slots_u);
    for i in 0..slots_u {
        let kind = if i < owned_clamped {
            BlazonSlotKind::Normal
        } else if i < castle_start {
            BlazonSlotKind::Empty
        } else if state.blinking > 0 && i >= blink_start {
            BlazonSlotKind::Normal
        } else {
            BlazonSlotKind::Castle
        };
        kinds.push(kind);
    }
    kinds
}

/// Start-X of the centered blazon-bar strip and its per-slot step.
/// Exposed so hit-testing shares the same layout as the draw.
fn blazon_bar_start_x(screen_width: u16, slot_count: u16) -> u16 {
    if slot_count == 0 {
        return 0;
    }
    let total_w =
        slot_count * BLAZON_BAR_TINY_W + slot_count.saturating_sub(1) * BLAZON_BAR_SPACING;
    screen_width.saturating_sub(total_w) / 2
}

/// Hit-test the blazon bar against a screen-space mouse position.
/// Returns the slot index under the cursor, or `None` when the cursor
/// is outside every icon rect.
pub fn hit_test_blazon_bar(
    screen_width: u16,
    state: &crate::widget::blazon_bar::BlazonBarState,
    mouse_x: i32,
    mouse_y: i32,
) -> Option<usize> {
    let owned = state.current.saturating_add(state.additional);
    let slots: u16 = state.required.max(owned).min(u16::MAX as u32) as u16;
    if slots == 0 {
        return None;
    }
    let start_x = blazon_bar_start_x(screen_width, slots) as i32;
    let step = (BLAZON_BAR_TINY_W + BLAZON_BAR_SPACING) as i32;
    let y0 = BLAZON_BAR_Y as i32;
    let y1 = y0 + BLAZON_BAR_TINY_H as i32;
    if mouse_y < y0 || mouse_y >= y1 {
        return None;
    }
    for i in 0..slots {
        let x0 = start_x + (i as i32) * step;
        let x1 = x0 + BLAZON_BAR_TINY_W as i32;
        if mouse_x >= x0 && mouse_x < x1 {
            return Some(i as usize);
        }
    }
    None
}

/// Draw the blazon-bar icon strip across the top of the screen.
///
/// Tiny-variant layout: the bar is centred across the screen width at
/// `y = 2` (the blazon bar sits in a 0..150 band but the actual icon row
/// is top-justified).  Reads the per-frame state from
/// [`crate::widget::blazon_bar::build_blazon_bar_state`].
///
/// Slot colouring is delegated to [`blazon_bar_slot_kinds`] which
/// implements the three-sprite split (normal / empty / castle) plus the
/// one-shot blink latch.
pub fn draw_blazon_bar(
    renderer: &mut Renderer,
    portraits: &PortraitCache,
    state: &crate::widget::blazon_bar::BlazonBarState,
) {
    let (Some(normal), Some(castle), Some(empty)) = (
        portraits.blazon_tiny_normal,
        portraits.blazon_tiny_castle,
        portraits.blazon_tiny_empty,
    ) else {
        return;
    };
    let kinds = blazon_bar_slot_kinds(state);
    if kinds.is_empty() {
        return;
    }
    let sw = renderer.screen_width();
    let start_x = blazon_bar_start_x(sw, kinds.len() as u16);
    for (i, kind) in kinds.iter().enumerate() {
        let sid = match kind {
            BlazonSlotKind::Normal => normal,
            BlazonSlotKind::Empty => empty,
            BlazonSlotKind::Castle => castle,
        };
        let x = start_x + (i as u16) * (BLAZON_BAR_TINY_W + BLAZON_BAR_SPACING);
        let dst = bbox(
            x,
            BLAZON_BAR_Y,
            x + BLAZON_BAR_TINY_W,
            BLAZON_BAR_Y + BLAZON_BAR_TINY_H,
        );
        blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
    }
}

//
// Layout constants:
//   ICON_WIDTH              40
//   ICON_HEIGHT             50
//   ICON_MARGIN             10
//   DIFFERENCE_X_YES_NO     25
//   DIFFERENCE_Y_YES_NO     30
//   DIFFERENCE_X_SELECTED   -1
//   DIFFERENCE_Y_SELECTED    2
const REQ_BAR_ICON_W: u16 = 40;
const REQ_BAR_ICON_H: u16 = 50;
const REQ_BAR_ICON_MARGIN: u16 = 10;
const REQ_BAR_Y: u16 = 2;
// The box is `(40, 2, screen_w - 40, 2)`.
const REQ_BAR_BOX_INSET: u16 = 40;
const REQ_BAR_DIFFERENCE_X_YES_NO: i32 = 25;
const REQ_BAR_DIFFERENCE_Y_YES_NO: i32 = 30;
const REQ_BAR_DIFFERENCE_X_SELECTED: i32 = -1;
const REQ_BAR_DIFFERENCE_Y_SELECTED: i32 = 2;

/// Starting-X for the centered requirements strip.  The box is
/// `(40, 2, screen_w - 40, 2)` so box_width = `screen_w - 80`; the strip
/// is centered by offsetting the box's top-left by `(box_w - needed_w) / 2`.
/// `needed_w = n * (ICON_W + MARGIN) - MARGIN`.
/// Returns `None` when the strip is wider than the box (no slots fit).
fn requirements_bar_start_x(screen_width: u16, slot_count: usize) -> Option<i32> {
    if slot_count == 0 {
        return None;
    }
    let step = (REQ_BAR_ICON_W + REQ_BAR_ICON_MARGIN) as i32;
    let needed = slot_count as i32 * step - REQ_BAR_ICON_MARGIN as i32;
    let box_left = REQ_BAR_BOX_INSET as i32;
    let box_w = (screen_width as i32) - 2 * (REQ_BAR_BOX_INSET as i32);
    if box_w <= 0 {
        return None;
    }
    Some(box_left + (box_w - needed) / 2)
}

/// Hit-test the requirements bar against a screen-space mouse position.
/// Returns the slot index under the cursor, or `None` when the cursor
/// is outside every icon rect.  Layout mirrors [`draw_requirements_bar`].
pub fn hit_test_requirements_bar(
    screen_width: u16,
    state: &crate::widget::requirements::RequirementsState,
    mouse: ScreenPoint,
) -> Option<usize> {
    let mouse_x = mouse.x as i32;
    let mouse_y = mouse.y as i32;
    let start_x = requirements_bar_start_x(screen_width, state.slots.len())?;
    let step = (REQ_BAR_ICON_MARGIN + REQ_BAR_ICON_W) as i32;
    let y0 = REQ_BAR_Y as i32;
    let y1 = (REQ_BAR_Y + REQ_BAR_ICON_H) as i32;
    if mouse_y < y0 || mouse_y >= y1 {
        return None;
    }
    for i in 0..state.slots.len() {
        let x0 = start_x + (i as i32) * step;
        let x1 = x0 + REQ_BAR_ICON_W as i32;
        if mouse_x >= x0 && mouse_x < x1 {
            return Some(i);
        }
    }
    None
}

pub fn draw_requirements_bar(
    renderer: &mut Renderer,
    portraits: &PortraitCache,
    campaign: &crate::campaign::Campaign,
    profiles: &robin_engine::profiles::ProfileManager,
    state: &crate::widget::requirements::RequirementsState,
) {
    let _ = campaign;
    let sw = renderer.screen_width();
    let Some(start_x) = requirements_bar_start_x(sw, state.slots.len()) else {
        return;
    };
    let step = (REQ_BAR_ICON_MARGIN + REQ_BAR_ICON_W) as i32;
    for (i, slot) in state.slots.iter().enumerate() {
        let icon_x = start_x + (i as i32) * step;
        let icon_y = REQ_BAR_Y as i32;
        let dst = bbox_i32(
            icon_x,
            icon_y,
            icon_x + REQ_BAR_ICON_W as i32,
            icon_y + REQ_BAR_ICON_H as i32,
        );
        let (icon_sid, status, selected) = match slot {
            RequirementSlot::RequiredCharacter {
                character_profile_idx,
                status,
                selected,
            } => {
                let sub_id = profiles
                    .get_character(*character_profile_idx)
                    .and_then(|p| CharacterKind::from_profile_name(&p.profile_name))
                    .map(|k| k.required_pc_sub_id())
                    .unwrap_or(0);
                (
                    portraits.get_sub_picture(resource_ids::RHID_REQUIRED_PC, sub_id),
                    Some(*status),
                    *selected,
                )
            }
            RequirementSlot::RequiredAction {
                action,
                status,
                selected,
            } => {
                let sub_id = required_action_sub_id(*action);
                (
                    portraits.get_sub_picture(resource_ids::RHID_REQUIRED_ACTION, sub_id),
                    Some(*status),
                    *selected,
                )
            }
            RequirementSlot::OptionalCharacter {
                character_profile_idx,
            } => {
                let slot_kind = character_profile_idx
                    .and_then(|idx| profiles.get_character(idx))
                    .and_then(|p| CharacterKind::from_profile_name(&p.profile_name));
                let sub_id = CharacterKind::optional_pc_sub_id(slot_kind);
                (
                    portraits.get_sub_picture(resource_ids::RHID_OPTIONAL_PC, sub_id),
                    None,
                    false,
                )
            }
        };
        if let Some(sid) = icon_sid {
            blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
        }
        // Status badge (yes tick / no cross) — small corner overlay at
        // (+25, +30) from the icon origin, not stretched across the icon.
        // Its size comes from the `RHID_YES_NO` resource's native dimensions.
        if let Some(st) = status {
            let overlay = match st {
                RequirementStatus::Fulfilled => portraits.req_yes,
                RequirementStatus::Missing => portraits.req_no,
            };
            if let Some(sid) = overlay {
                let w = renderer.surface_width(sid) as i32;
                let h = renderer.surface_height(sid) as i32;
                let bx = icon_x + REQ_BAR_DIFFERENCE_X_YES_NO;
                let by = icon_y + REQ_BAR_DIFFERENCE_Y_YES_NO;
                let badge = bbox_i32(bx, by, bx + w, by + h);
                blit_to_screen_widget(renderer, sid, None, Some(&badge), BLIT_SOURCE_TRANSPARENT);
            }
        }
        // Selected-ring overlay at (-1, +2) from the icon origin.
        if selected && let Some(sid) = portraits.req_selected {
            let w = renderer.surface_width(sid) as i32;
            let h = renderer.surface_height(sid) as i32;
            let rx = icon_x + REQ_BAR_DIFFERENCE_X_SELECTED;
            let ry = icon_y + REQ_BAR_DIFFERENCE_Y_SELECTED;
            let ring = bbox_i32(rx, ry, rx + w, ry + h);
            blit_to_screen_widget(renderer, sid, None, Some(&ring), BLIT_SOURCE_TRANSPARENT);
        }
    }
}

/// Build a `BBox` from signed i32 screen coordinates (helper for layouts
/// that compute positions in signed space — e.g. the `-1` offset of the
/// requirements-bar selected ring).
fn bbox_i32(x0: i32, y0: i32, x1: i32, y1: i32) -> BBox {
    BBox::new(
        robin_engine::geo2d::pt(x0 as f32, y0 as f32),
        robin_engine::geo2d::pt(x1 as f32, y1 as f32),
    )
}

/// Menu-text id for the static tooltip attached to a given requirements-bar
/// slot:
/// - `MT_INFOBULLE_QG_NEEDED_PC` for `RequiredCharacter`
/// - `MT_INFOBULLE_QG_NEEDED_ACTION` for `RequiredAction`
/// - `MT_INFOBULLE_QG_OTHER_PC` for `OptionalCharacter`
pub fn requirements_slot_tooltip_mt_id(
    slot: &crate::widget::requirements::RequirementSlot,
) -> usize {
    use crate::ingame_menu::resources::{
        MT_INFOBULLE_QG_NEEDED_ACTION, MT_INFOBULLE_QG_NEEDED_PC, MT_INFOBULLE_QG_OTHER_PC,
    };
    match slot {
        RequirementSlot::RequiredCharacter { .. } => MT_INFOBULLE_QG_NEEDED_PC,
        RequirementSlot::RequiredAction { .. } => MT_INFOBULLE_QG_NEEDED_ACTION,
        RequirementSlot::OptionalCharacter { .. } => MT_INFOBULLE_QG_OTHER_PC,
    }
}

/// Menu-text id for the static tooltip attached to a given blazon-bar
/// slot:
/// - `MT_INFOBULLE_BLAZON_WON` for `Normal`
/// - `MT_INFOBULLE_BLAZON_TO_WIN` for `Empty`
/// - `MT_INFOBULLE_BLAZON_TO_WIN_IN_ATTACK` for `Castle`
pub fn blazon_slot_tooltip_mt_id(kind: BlazonSlotKind) -> usize {
    use crate::ingame_menu::resources::{
        MT_INFOBULLE_BLAZON_TO_WIN, MT_INFOBULLE_BLAZON_TO_WIN_IN_ATTACK, MT_INFOBULLE_BLAZON_WON,
    };
    match kind {
        BlazonSlotKind::Normal => MT_INFOBULLE_BLAZON_WON,
        BlazonSlotKind::Empty => MT_INFOBULLE_BLAZON_TO_WIN,
        BlazonSlotKind::Castle => MT_INFOBULLE_BLAZON_TO_WIN_IN_ATTACK,
    }
}

/// The blazon bar and the requirements bar share the same tooltip
/// idle-timer pipeline.  Rather than duplicate the tracker, the two
/// bars use the same struct — a slot index is a slot index.
pub type BlazonTooltipTracker = RequirementsTooltipTracker;

/// Menu-text id for the tooltip attached to a PC action button.
/// Actions that are not in the switch (e.g. contextual-only actions
/// or `NoAction`) get `None`, which renders as no tooltip.
pub fn action_button_tooltip_mt_id(action: crate::profiles::Action) -> Option<usize> {
    use crate::ingame_menu::resources::*;
    use crate::profiles::Action;
    Some(match action {
        Action::Bow => MT_INFOBULLE_ACTION_BOW,
        Action::Hit | Action::HitHard => MT_INFOBULLE_ACTION_FIST,
        Action::Purse => MT_INFOBULLE_ACTION_PURSE,
        Action::Stone => MT_INFOBULLE_ACTION_STONE,
        Action::Shield | Action::BigShield => MT_INFOBULLE_ACTION_SHIELD,
        Action::Strangle => MT_INFOBULLE_ACTION_STRANGLER,
        Action::HelpToClimb => MT_INFOBULLE_ACTION_COURTE_ECHELLE,
        Action::Apple => MT_INFOBULLE_ACTION_APPLE,
        Action::Eat | Action::Guzzle => MT_INFOBULLE_ACTION_GIGOT,
        Action::Listen => MT_INFOBULLE_ACTION_SPY,
        Action::Heal => MT_INFOBULLE_ACTION_HERBS,
        Action::Net => MT_INFOBULLE_ACTION_NET,
        Action::Beggar => MT_INFOBULLE_ACTION_SIMULER_MENDIANT,
        Action::WaspNest => MT_INFOBULLE_ACTION_WASP,
        Action::Ale => MT_INFOBULLE_ACTION_BEER,
        Action::Whistle => MT_INFOBULLE_ACTION_SIFFLER,
        _ => return None,
    })
}

/// Hover-idle tracker for the three PC portrait action buttons.
/// Wraps [`RequirementsTooltipTracker`] keyed on the `(portrait slot,
/// button index)` pair encoded as `slot * 4 + btn` — each portrait has
/// at most 3 buttons so the 4x stride leaves a unique slot id per
/// button.  The shared 75-tick idle-hover delay is inherited from
/// `RequirementsTooltipTracker`.
#[derive(Default, Clone)]
pub struct PcActionTooltipTracker {
    inner: RequirementsTooltipTracker,
}

impl PcActionTooltipTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Encode `(slot, btn)` into the shared slot-index space.
    fn encode(slot: u8, btn: u8) -> usize {
        (slot as usize) * 4 + (btn as usize)
    }

    fn decode(encoded: usize) -> (u8, u8) {
        ((encoded / 4) as u8, (encoded % 4) as u8)
    }

    /// Call once per frame with the hovered `(slot, btn)` pair, or
    /// `None` when the cursor is not over any PC action button.
    pub fn update(&mut self, hovered: Option<(u8, u8)>) {
        self.inner.update(hovered.map(|(s, b)| Self::encode(s, b)));
    }

    /// `Some((slot, btn))` once the cursor has been idle on the same
    /// button long enough for the tooltip to appear.
    pub fn ready_button(&self) -> Option<(u8, u8)> {
        self.inner.ready_slot().map(Self::decode)
    }
}

/// Number of `update()` ticks the cursor must idle on the same slot
/// before the tooltip appears (one tick per game frame).
pub const REQUIREMENTS_TOOLTIP_DELAY_TICKS: u32 = 75;

/// Hover tracker for the requirements-bar tooltip.  Increments a
/// per-tick counter while the cursor stays on the same slot, resets
/// when the target slot changes, and fires the tooltip once the
/// counter crosses the threshold.
///
/// The bar is drawn in immediate mode with no backing widget list, so
/// we key the tracker on the slot index returned by
/// [`hit_test_requirements_bar`] rather than on a `WidgetId`.  The
/// counter is frame-count-based (not wall-clock), so pausing the frame
/// loop pauses the delay too.
#[derive(Default, Clone)]
pub struct RequirementsTooltipTracker {
    hovered_slot: Option<usize>,
    /// Ticks accumulated with the cursor on `hovered_slot`.  Saturates
    /// at `u32::MAX` so a very long idle hover can't wrap back below
    /// the threshold.
    hover_ticks: u32,
}

impl RequirementsTooltipTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Call once per frame with the slot currently under the cursor.
    /// Resets the tick counter when the target slot changes.
    pub fn update(&mut self, hovered: Option<usize>) {
        if hovered != self.hovered_slot {
            self.hovered_slot = hovered;
            self.hover_ticks = 0;
        } else if hovered.is_some() {
            self.hover_ticks = self.hover_ticks.saturating_add(1);
        }
    }

    /// Returns `Some(slot_idx)` when the cursor has been idle over the
    /// same slot long enough for the tooltip to appear.  Strictly
    /// greater-than the threshold.
    pub fn ready_slot(&self) -> Option<usize> {
        let idx = self.hovered_slot?;
        if self.hover_ticks > REQUIREMENTS_TOOLTIP_DELAY_TICKS {
            Some(idx)
        } else {
            None
        }
    }
}

/// Draw a tooltip string at a screen-space position: shadowed text (the
/// Background font rendered at `+1, +1` then the Tooltips font on top),
/// anchored at `mouse + cursor_size - (0, font_height)` so the tooltip
/// sits to the right of the cursor with its bottom aligned with the
/// cursor's bottom.  When it would overflow the right edge the tooltip
/// flips to the left of the cursor; if the left fallback also doesn't
/// fit, falls back to a multi-line text box anchored at the cursor,
/// clipped to the right screen edge and at most three lines tall.
/// Shifts up when it would overflow the bottom edge.  No background
/// fill — relies on the shadow font for contrast against the scene.
///
/// `shadow` is the optional "Background" font; when `None`, the text is
/// drawn without an explicit shadow (the NativeFont rasterizer still
/// bakes a subtle halo into ARGB glyphs).  `cursor_size` is the current
/// cursor sprite's on-screen size (width, height).
pub fn draw_screen_tooltip(
    renderer: &mut Renderer,
    font: &crate::native_font::NativeFont,
    shadow: Option<&crate::native_font::NativeFont>,
    text: &str,
    mouse_x: i32,
    mouse_y: i32,
    cursor_size: (i32, i32),
) {
    if text.is_empty() {
        return;
    }
    let tw = font.text_width(text);
    let th = font.height() as i32;
    if tw <= 0 || th <= 0 {
        return;
    }

    let sw = renderer.screen_width() as i32;
    let sh = renderer.screen_height() as i32;
    let (cursor_w, cursor_h) = cursor_size;

    // Default anchor: to the right of the cursor, bottom-aligned with
    // the cursor's bottom edge (`mouse + cursor_size - (0, font_h)`).
    let default_x = mouse_x + cursor_w;
    let default_y = mouse_y + cursor_h - th;

    let right_overflow = default_x + tw > sw;
    let left_fits = mouse_x - tw > 0;

    if right_overflow && !left_fits {
        // Three-way fallback: neither right nor left fits — wrap the text
        // into a multi-line box anchored at the cursor, with width clipped
        // to the right screen edge and height capped at `3 * font.height()`.
        let box_x = mouse_x.max(0);
        let box_y = default_y.max(0);
        let box_w = (sw - box_x).max(1);
        let wrap = crate::ingame_menu::layout::wrap_text(font, text, box_w, 3);
        // Clamp vertically if the wrapped box overflows the bottom.
        let total_h = (wrap.lines.len() as i32) * th;
        let y_top = if box_y + total_h > sh {
            (sh - total_h).max(0)
        } else {
            box_y
        };
        for (i, line) in wrap.lines.iter().enumerate() {
            let ly = y_top + (i as i32) * th;
            if let Some(sh_font) = shadow {
                renderer.render_text_argb(sh_font, line, box_x + 1, ly + 1);
            }
            renderer.render_text_argb(font, line, box_x, ly);
        }
        return;
    }

    let (mut x, mut y) = if right_overflow {
        // Overflow right: flip to the left of the cursor, same y.
        (mouse_x - tw, default_y)
    } else {
        (default_x, default_y)
    };

    // Overflow bottom: shift up so the tooltip stays on screen.
    if y + th > sh {
        y = sh - th;
    }
    if y < 0 {
        y = 0;
    }
    if x < 0 {
        x = 0;
    }

    if let Some(sh_font) = shadow {
        renderer.render_text_argb(sh_font, text, x + 1, y + 1);
    }
    renderer.render_text_argb(font, text, x, y);
}

// ─── PC info popup overlay ────────────────────────────────────────

/// Render the hovered-PC info popup.
///
/// Resolves the hovered PC's sword/bow capacity from the campaign
/// `HumanStatus`, re-computes the pip layout each frame (skills are
/// re-read on every show), then blits the background + lit pip sprites.
/// Does nothing when the overlay is not visible.
///
/// `mouse` is the current mouse cursor position — the overlay clamps
/// itself to the screen bounds each frame.
pub fn draw_pc_info_overlay(
    host: &mut Host,
    engine: &Engine,
    profiles: &robin_engine::profiles::ProfileManager,
    renderer: &mut Renderer,
    portraits: &PortraitCache,
    mouse: ScreenPoint,
) {
    use crate::pc_info_overlay::{LEVEL_NUMBER, PcInfoOverlay};

    let ov = &host.pc_info_overlay;
    if !ov.visible {
        return;
    }
    let Some(pc_id) = ov.pc_id else { return };
    let Some(Entity::Pc(pc)) = engine.get_entity(pc_id) else {
        return;
    };

    // Pull sword / bow capacity from the campaign character descriptor.
    let Some(campaign) = engine.campaign() else {
        return;
    };
    let Some(desc) = campaign.characters.get(usize::from(pc.pc.profile_index)) else {
        return;
    };
    let sword_cap = desc.status.human_status.hand_to_hand.capacity;
    let bow_cap = desc.status.human_status.bow.capacity;

    // Archer iff the PC's profile lists a Bow action.
    let _ = campaign;
    let is_archer = profiles
        .get_character(pc.pc.profile_index)
        .map(|p| p.actions.contains(&Action::Bow))
        .unwrap_or(false);

    let sw = renderer.screen_width() as i32;
    let sh = renderer.screen_height() as i32;

    // Compute positions + pip counts for this frame (recomputed every
    // show, since skills can change between displays).
    let mut frame_ov = PcInfoOverlay::default();
    frame_ov.show(pc_id, mouse, (sw, sh), is_archer, sword_cap, bow_cap);

    // ── Background ──
    let bg_sid = if is_archer {
        portraits.info_popup_bg_huge
    } else {
        portraits.info_popup_bg_tiny
    };
    if let Some(sid) = bg_sid {
        let w = renderer.surface_width(sid);
        let h = renderer.surface_height(sid);
        let (x, y) = (frame_ov.position.0 as u16, frame_ov.position.1 as u16);
        let dst = bbox(x, y, x + w, y + h);
        blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
    }

    // ── Sword pips ──
    if let Some(sid) = portraits.info_popup_sword {
        let w = renderer.surface_width(sid);
        let h = renderer.surface_height(sid);
        for i in 0..frame_ov.sword_pips.min(LEVEL_NUMBER) {
            let (px, py) = frame_ov.sword_pip_position(i);
            let dst = bbox(px as u16, py as u16, px as u16 + w, py as u16 + h);
            blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
        }
    }

    // ── Bow pips (archer only) ──
    if is_archer && let Some(sid) = portraits.info_popup_bow {
        let w = renderer.surface_width(sid);
        let h = renderer.surface_height(sid);
        for i in 0..frame_ov.bow_pips.min(LEVEL_NUMBER) {
            let (px, py) = frame_ov.bow_pip_position(i);
            let dst = bbox(px as u16, py as u16, px as u16 + w, py as u16 + h);
            blit_to_screen_widget(renderer, sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
        }
    }
}

// ─── Dotted-chain rendering (world space) ─────────────────────────

/// Render the per-PC macro dotted chains.
///
/// For every PC with at least one non-empty macro slot, walks the
/// recorded steps and calls
/// `DrawManager::draw_dotted_line(… DISTANCE_DOT, 1, 0x0000 …)` for each
/// segment starting at the PC's map position.  The dot phase is a
/// single field (`TitbitManager::dotted_start`) shared across all PCs.
pub fn render_macro_dotted_chains(host: &mut Host, engine: &Engine, renderer: &mut Renderer) {
    use crate::macro_store::DISTANCE_DOT;

    // Snapshot the PC positions first — the draw call borrows engine.host
    // mutably for the draw_manager and its phase store, so we can't
    // still be iterating `engine.pc_ids()` while calling it.
    let mut per_pc: Vec<(
        crate::element::EntityId,
        robin_engine::coordinates::MapPoint,
    )> = Vec::with_capacity(engine.pc_ids().len());
    for &pc_id in engine.pc_ids() {
        if let Some(ent) = engine.get_entity(pc_id) {
            let pos = ent.element_data().position_map();
            per_pc.push((pc_id, pos));
        }
    }

    // The dotted-phase is chained across every segment draw within a
    // frame; since the engine-owned phase is advanced once per tick
    // (`TitbitManager::prepare_refresh`), the renderer reads the current
    // phase and chains locally across segments.  Not writing back
    // preserves the mutation-free invariant — next frame's tick will
    // re-advance the canonical phase.
    let mut phase = engine.titbit_dotted_start();
    for (pc_id, pc_pos) in per_pc {
        let Some(state) = engine.macro_store().get(pc_id) else {
            continue;
        };
        if state.slots().iter().all(|s| s.is_empty()) {
            continue;
        }

        // Gather every QA-memory titbit into a single list and walk it
        // once with `from` carrying forward across slots — the polyline
        // is `PC → slot0 → slot1 → slot2`, not three separate fans from
        // the PC.
        let mut from = pc_pos;
        for slot in state.slots() {
            for step in &slot.steps {
                let to = step.position;
                host.draw_manager.draw_dotted_line(
                    renderer,
                    from.to_geo(),
                    to.to_geo(),
                    &mut phase,
                    DISTANCE_DOT,
                    1.0,
                    0x0000,
                );
                from = to;
            }
        }
    }
}

// ─── Portrait hit-testing ─────────────────────────────────────────

/// Which sub-area of a portrait was clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortraitHitArea {
    /// Top scroll (health gauge / parchment).
    TopScroll,
    /// Face / visage area.
    Visage,
    /// One of the action buttons (0-based index).
    ActionButton(u8),
    /// One of the quick-action macro icons (0-based QA slot).
    QuickAction(u8),
    /// Bottom scroll (ammo count area).
    BottomScroll,
    /// Guard indicator (burned state only).
    Guard,
    /// Amulet/clover indicator (burned state, not guarded).
    Amulet,
    /// Reinforcement trumpet indicator (dead state, when a replacement is
    /// still available).
    Trumpet,
}

/// Result of a portrait hit-test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortraitHit {
    /// Portrait slot index (0-based into `engine.displayed_pc_ids()`).
    /// Hidden-interface PCs are not part of the displayed list, so this
    /// index is *not* a valid index into `engine.pc_ids()` whenever any
    /// PC has `interface_hidden = true` — use `pc_id` instead of
    /// re-indexing.
    pub slot: u8,
    /// Resolved PC entity id at this slot — saves the caller from
    /// re-walking `displayed_pc_ids()`.
    pub pc_id: crate::element::EntityId,
    /// Which sub-area was clicked.
    pub area: PortraitHitArea,
    /// Whether this portrait's PC is burned (coma/dead).
    pub is_burned: bool,
}

/// Hit-test a screen-space click against the portrait slots (simple version).
///
/// Returns the index of the clicked portrait slot (0-based into `engine.pc_ids()`),
/// or `None` if the click is outside all portrait areas.
pub fn hit_test_portrait(
    screen_width: u16,
    screen_height: u16,
    click_x: f32,
    click_y: f32,
    num_pcs: usize,
) -> Option<u8> {
    let num_slots = num_pcs.min(NUMBER_OF_SLOTS as usize);
    let sh = screen_height;

    let panel_top = sh - PORTRAIT_TOTAL_HEIGHT;
    let panel_bot = sh - BORDURE;

    // Quick reject: not in panel area
    if click_y < panel_top as f32 || click_y > panel_bot as f32 {
        return None;
    }

    for slot in 0..num_slots {
        let x = slot_left_x(screen_width, slot as u16) as f32;
        let x2 = x + ELEMENT_WIDTH as f32;

        if click_x >= x && click_x <= x2 {
            return Some(slot as u8);
        }
    }

    None
}

/// Detailed hit-test returning which sub-area of which portrait was clicked.
///
/// Uses engine state to determine burned/selected per slot, and maps
/// the click Y to the appropriate sub-area.
pub fn hit_test_portrait_detailed(
    engine: &Engine,
    local_seat: PlayerId,
    portraits: &PortraitCache,
    screen_width: u16,
    screen_height: u16,
    click_x: f32,
    click_y: f32,
) -> Option<PortraitHit> {
    let displayed_pcs = engine.displayed_pc_ids();
    let num_slots = displayed_pcs.len().min(NUMBER_OF_SLOTS as usize);
    let sh = screen_height;
    let cy = click_y;

    let panel_top = (sh - PORTRAIT_TOTAL_HEIGHT - QA_ICON_HEIGHT) as f32;
    let panel_bot = (sh - BORDURE) as f32;

    if cy < panel_top || cy > panel_bot {
        return None;
    }

    for (slot, &pc_id) in displayed_pcs.iter().enumerate().take(num_slots) {
        let x = slot_left_x(screen_width, slot as u16) as f32;
        let x2 = x + ELEMENT_WIDTH as f32;

        if click_x < x || click_x > x2 {
            continue;
        }

        let entity = engine.get_entity(pc_id);
        let is_selected = engine.seat_selection(local_seat).contains(&pc_id);

        let is_coma = entity.map(|e| is_pc_in_coma(engine, e)).unwrap_or(false);
        if cy < (sh - PORTRAIT_TOTAL_HEIGHT) as f32 && (!is_selected || is_coma) {
            continue;
        }
        if is_selected && !is_coma {
            let qa_strip_y = (sh - POSITION_TOP_SCROLL - QA_ICON_HEIGHT) as f32;
            let qa_strip_bot = qa_strip_y + QA_ICON_HEIGHT as f32;
            if cy >= qa_strip_y && cy < qa_strip_bot {
                let rel_x = click_x - x;
                if rel_x >= 0.0 {
                    let slot_idx = (rel_x / QA_ICON_WIDTH as f32).floor() as u8;
                    if usize::from(slot_idx) < crate::macro_store::NUMBER_OF_QA_MEMORY {
                        return Some(PortraitHit {
                            slot: slot as u8,
                            pc_id,
                            area: PortraitHitArea::QuickAction(slot_idx),
                            is_burned: false,
                        });
                    }
                }
            }
        }
        if cy < (sh - PORTRAIT_TOTAL_HEIGHT) as f32 {
            continue;
        }

        let is_dead = match entity {
            Some(Entity::Pc(pc)) => pc.pc.life_points <= 0,
            _ => false,
        };
        let is_burned = is_dead || is_coma;
        let is_guarded = match entity {
            Some(Entity::Pc(pc)) => pc.pc.guard.is_some(),
            _ => false,
        };
        let has_trumpet = match entity {
            Some(Entity::Pc(pc)) => pc.pc.trumpet_enabled,
            _ => false,
        };

        if is_burned {
            // Burned layout: upper scroll repositioned above lower scroll.
            // Guard/amulet/trumpet indicator is between the two scrolls.
            let bottom_scroll_top = (sh - POSITION_BOTTOM_SCROLL) as f32;
            let upper_scroll_top = (sh - POSITION_BOTTOM_SCROLL - BOTTOM_SCROLL_HEIGHT) as f32;
            let upper_scroll_bot = upper_scroll_top + TOP_SCROLL_HEIGHT as f32;

            let area = if cy >= upper_scroll_top && cy < upper_scroll_bot {
                PortraitHitArea::TopScroll
            } else if cy >= upper_scroll_bot && cy < bottom_scroll_top {
                // Between scrolls — trumpet (dead only) takes priority over
                // guard/amulet (coma only).  The trumpet is only enabled on
                // dead PCs, so the two indicator families never overlap in
                // practice.
                if has_trumpet && is_dead && !is_coma {
                    PortraitHitArea::Trumpet
                } else if is_guarded {
                    PortraitHitArea::Guard
                } else {
                    PortraitHitArea::Amulet
                }
            } else {
                PortraitHitArea::BottomScroll
            };

            return Some(PortraitHit {
                slot: slot as u8,
                pc_id,
                area,
                is_burned,
            });
        }

        // Normal (non-burned) layout
        let top_scroll_top = if is_selected {
            (sh - POSITION_TOP_SCROLL) as f32
        } else {
            (sh - CLOSE_POSITION_TOP_SCROLL) as f32
        };
        let visage_top = if is_selected {
            (sh - POSITION_VISAGE) as f32
        } else {
            (sh - CLOSE_POSITION_VISAGE) as f32
        };
        let action_top = (sh - POSITION_ACTION) as f32;
        let bottom_scroll_top = (sh - POSITION_BOTTOM_SCROLL) as f32;

        let area = if cy >= top_scroll_top && cy < visage_top {
            // Check pixel transparency on the curved scroll edges.
            // If the pixel is transparent, reject the hit so the click falls through.
            if let Some(ref mask) = portraits.top_scroll_hit_mask {
                let rel_x = (click_x - x) as u16;
                let rel_y = (cy - top_scroll_top) as u16;
                if !mask.is_opaque(rel_x, rel_y) {
                    return None;
                }
            }
            PortraitHitArea::TopScroll
        } else if cy >= visage_top && cy < action_top && is_selected {
            PortraitHitArea::Visage
        } else if cy >= visage_top && !is_selected {
            // Closed state: visage extends down to bottom scroll
            PortraitHitArea::Visage
        } else if cy >= action_top && cy < bottom_scroll_top && is_selected {
            // Determine which action button based on X.
            // Check two-button mode
            let action_icons = entity
                .and_then(pc_character_kind)
                .map(|k| k.action_resources());
            let two_btn = action_icons
                .as_ref()
                .is_some_and(|icons| icons[2].is_none());
            let rel_x = click_x - x;

            let btn_idx = if two_btn {
                if rel_x < ACTIONA_WIDTH as f32 { 0 } else { 1 }
            } else if rel_x < ACTION1_WIDTH as f32 {
                0
            } else if rel_x < (ACTION1_WIDTH + ACTION2_WIDTH) as f32 {
                1
            } else {
                2
            };
            PortraitHitArea::ActionButton(btn_idx)
        } else {
            PortraitHitArea::BottomScroll
        };

        return Some(PortraitHit {
            slot: slot as u8,
            pc_id,
            area,
            is_burned,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_width_at_800() {
        // screen_width=800, margin=32 each side → 736px / 5 = 147px per slot
        assert_eq!(slot_width(800), 147);
    }

    #[test]
    fn slot_left_positions_at_800() {
        // Each slot center should contain the 112px element
        let sw = 800u16;
        for slot in 0..5 {
            let left = slot_left_x(sw, slot);
            let right = left + ELEMENT_WIDTH;
            assert!(
                left >= MARGIN || slot == 0,
                "slot {} starts before margin",
                slot
            );
            assert!(
                right <= sw - MARGIN || slot == 4,
                "slot {} extends past margin",
                slot
            );
        }
    }

    #[test]
    fn portrait_total_height() {
        // 3 + 23 + 35 + 50 + 23 = 134
        assert_eq!(PORTRAIT_TOTAL_HEIGHT, 134);
    }

    #[test]
    fn position_stack() {
        // Verify the position constants stack correctly from bottom
        assert_eq!(POSITION_BOTTOM_SCROLL, 26); // 3 + 23
        assert_eq!(POSITION_ACTION, 61); // 26 + 35
        assert_eq!(POSITION_VISAGE, 111); // 61 + 50
        assert_eq!(POSITION_TOP_SCROLL, 134); // 111 + 23
    }

    #[test]
    fn bbox_construction() {
        let b = bbox(10, 20, 30, 40);
        assert_eq!(b.min.x, 10.0);
        assert_eq!(b.min.y, 20.0);
        assert_eq!(b.max.x, 30.0);
        assert_eq!(b.max.y, 40.0);
    }

    #[test]
    fn action_button_visual_matches_widget_priority() {
        assert_eq!(
            action_button_visual(false, false, false),
            ActionButtonVisual::Normal
        );
        assert_eq!(
            action_button_visual(false, false, true),
            ActionButtonVisual::Hover
        );
        assert_eq!(
            action_button_visual(false, true, true),
            ActionButtonVisual::Disabled
        );
        assert_eq!(
            action_button_visual(true, false, true),
            ActionButtonVisual::HoverPressed
        );
        assert_eq!(
            action_button_visual(true, true, true),
            ActionButtonVisual::Disabled
        );
    }

    #[test]
    fn action_button_sub_ids_cover_classic_radio_and_rdo_hover() {
        assert_eq!(ACTION_SUB_ID_DISABLED, 0);
        assert_eq!(ACTION_SUB_ID_UNSELECTED, 1);
        assert_eq!(ACTION_SUB_ID_FOCUSED, 2);
        assert_eq!(ACTION_SUB_ID_SELECTED, 3);
        assert_eq!(ACTION_SUB_ID_FOCUSED_SELECTED, 4);
    }

    // The CharacterKind resource-lookup and sub-id tests live in
    // `robin_engine::character_kind`; the UI-panel side just delegates to
    // those methods, so no duplicate tests are needed here.

    #[test]
    fn hit_test_outside_panel() {
        // Click above the panel area
        assert_eq!(hit_test_portrait(800, 600, 400.0, 100.0, 3), None);
    }

    #[test]
    fn hit_test_on_portrait_slot() {
        // screen 800x600, slot 0 starts at slot_left_x(800, 0)
        let x = slot_left_x(800, 0) as f32 + 10.0;
        let y = 600.0 - 50.0; // within the panel area
        assert_eq!(hit_test_portrait(800, 600, x, y, 3), Some(0));
    }

    #[test]
    fn hit_test_empty_slots() {
        // No PCs means no hits even inside the panel
        let x = slot_left_x(800, 0) as f32 + 10.0;
        let y = 600.0 - 50.0;
        assert_eq!(hit_test_portrait(800, 600, x, y, 0), None);
    }

    #[test]
    fn hit_test_between_slots() {
        // Click between slot boundaries (in the gap)
        let x0_right = slot_left_x(800, 0) as f32 + ELEMENT_WIDTH as f32 + 5.0;
        let y = 600.0 - 50.0;
        let x1_left = slot_left_x(800, 1) as f32;
        // Only a gap hit if the click is truly between elements
        if x0_right < x1_left {
            assert_eq!(hit_test_portrait(800, 600, x0_right, y, 3), None);
        }
    }

    #[test]
    fn hit_test_requirements_bar_maps_screen_coords_to_slots() {
        use crate::profiles::Action;
        use crate::widget::requirements::{RequirementSlot, RequirementStatus, RequirementsState};
        let state = RequirementsState {
            slots: vec![
                RequirementSlot::RequiredCharacter {
                    character_profile_idx: robin_engine::profiles::CharacterProfileIdx(1),
                    status: RequirementStatus::Fulfilled,
                    selected: false,
                },
                RequirementSlot::RequiredAction {
                    action: Action::Bow,
                    status: RequirementStatus::Fulfilled,
                    selected: false,
                },
            ],
            all_fulfilled: true,
        };
        // Strip is centered in the box (40, screen_w - 40).  For 800px:
        // box_w = 720, needed_w = 2*(40+10) - 10 = 90, start_x =
        // 40 + (720 - 90)/2 = 355.  Step = 50px between slot origins.
        let step = (REQ_BAR_ICON_MARGIN + REQ_BAR_ICON_W) as i32;
        let start_x = requirements_bar_start_x(800, state.slots.len()).unwrap();
        let y_in = (REQ_BAR_Y + REQ_BAR_ICON_H / 2) as i32;
        let slot0_cx = start_x + REQ_BAR_ICON_W as i32 / 2;
        let slot1_cx = slot0_cx + step;
        assert_eq!(start_x, 355);
        assert_eq!(
            hit_test_requirements_bar(800, &state, ScreenPoint::new(slot0_cx as f32, y_in as f32)),
            Some(0)
        );
        assert_eq!(
            hit_test_requirements_bar(800, &state, ScreenPoint::new(slot1_cx as f32, y_in as f32)),
            Some(1)
        );
        // In the margin between slot 0 and slot 1.
        let gap_x = start_x + REQ_BAR_ICON_W as i32 + 1;
        assert_eq!(
            hit_test_requirements_bar(800, &state, ScreenPoint::new(gap_x as f32, y_in as f32)),
            None
        );
        // Below the bar.
        assert_eq!(
            hit_test_requirements_bar(800, &state, ScreenPoint::new(slot0_cx as f32, 200.0)),
            None
        );
    }

    #[test]
    fn requirements_bar_centered_in_box() {
        // The strip is centered inside the (40, w-40) box.  A 2-slot
        // strip on a 1024px screen: box_w = 944, needed_w = 90, start_x
        // = 40 + (944 - 90)/2 = 467.
        assert_eq!(requirements_bar_start_x(1024, 2), Some(467));
        // 0 slots = no strip to center.
        assert_eq!(requirements_bar_start_x(1024, 0), None);
        // Narrow screen with negative box_w falls back to None.
        assert_eq!(requirements_bar_start_x(40, 2), None);
    }

    #[test]
    fn requirements_slot_tooltip_mt_id_matches_table() {
        use crate::ingame_menu::resources::{
            MT_INFOBULLE_QG_NEEDED_ACTION, MT_INFOBULLE_QG_NEEDED_PC, MT_INFOBULLE_QG_OTHER_PC,
        };
        use crate::profiles::Action;
        use crate::widget::requirements::{RequirementSlot, RequirementStatus};
        assert_eq!(
            requirements_slot_tooltip_mt_id(&RequirementSlot::RequiredCharacter {
                character_profile_idx: robin_engine::profiles::CharacterProfileIdx(1),
                status: RequirementStatus::Fulfilled,
                selected: false,
            }),
            MT_INFOBULLE_QG_NEEDED_PC
        );
        assert_eq!(
            requirements_slot_tooltip_mt_id(&RequirementSlot::RequiredAction {
                action: Action::Bow,
                status: RequirementStatus::Fulfilled,
                selected: false,
            }),
            MT_INFOBULLE_QG_NEEDED_ACTION
        );
        assert_eq!(
            requirements_slot_tooltip_mt_id(&RequirementSlot::OptionalCharacter {
                character_profile_idx: Some(robin_engine::profiles::CharacterProfileIdx(2)),
            }),
            MT_INFOBULLE_QG_OTHER_PC
        );
        assert_eq!(
            requirements_slot_tooltip_mt_id(&RequirementSlot::OptionalCharacter {
                character_profile_idx: None,
            }),
            MT_INFOBULLE_QG_OTHER_PC
        );
    }

    #[test]
    fn requirements_tooltip_tracker_counts_ticks() {
        let mut t = RequirementsTooltipTracker::new();
        assert!(t.ready_slot().is_none());

        // First update arms the counter at 0 (timer resets when the
        // focus changes).  Not yet ready.
        t.update(Some(0));
        assert!(t.ready_slot().is_none());

        // Bump up to (but not past) the threshold — the comparison is
        // strictly greater-than, so 75 ticks still means "not ready".
        for _ in 0..REQUIREMENTS_TOOLTIP_DELAY_TICKS {
            t.update(Some(0));
        }
        assert_eq!(t.ready_slot(), None);

        // One more tick crosses the threshold.
        t.update(Some(0));
        assert_eq!(t.ready_slot(), Some(0));

        // Switching slots resets the counter.
        t.update(Some(1));
        assert!(t.ready_slot().is_none());

        // Leaving the bar clears the tracker entirely.
        t.update(None);
        assert!(t.ready_slot().is_none());
    }

    #[test]
    fn portrait_cache_empty() {
        let cache = PortraitCache::new();
        assert!(!cache.is_loaded());
        let robin = CharacterKind::RobinHood { is_town: false };
        assert_eq!(cache.get_surface(robin), None);
        assert!(cache.get_action_surfaces(robin).is_none());
        assert!(cache.get_localized_name(robin).is_none());
        assert_eq!(
            cache.get_sub_picture(resource_ids::RHID_REQUIRED_PC, 1),
            None,
        );
    }

    #[test]
    fn required_action_sub_ids_match_table() {
        use crate::profiles::Action;
        // Sub-id table:
        // UnknownAction=0, Bow=1, Carry=2, Climb=3, Jump=4, Lever=5,
        // Lockpick=6, Stun=7, Tie=8, Eat=9, Search=10.
        assert_eq!(required_action_sub_id(Action::Bow), 1);
        assert_eq!(required_action_sub_id(Action::LittleJohnCarry), 2);
        assert_eq!(required_action_sub_id(Action::FarmerCarry), 2);
        assert_eq!(required_action_sub_id(Action::Climb), 3);
        assert_eq!(required_action_sub_id(Action::Jump), 4);
        assert_eq!(required_action_sub_id(Action::Lever), 5);
        assert_eq!(required_action_sub_id(Action::Lockpick), 6);
        assert_eq!(required_action_sub_id(Action::Hit), 7);
        assert_eq!(required_action_sub_id(Action::HitHard), 7);
        assert_eq!(required_action_sub_id(Action::Tie), 8);
        assert_eq!(required_action_sub_id(Action::Eat), 9);
        assert_eq!(required_action_sub_id(Action::Guzzle), 9);
        assert_eq!(required_action_sub_id(Action::Search), 10);
    }
}
