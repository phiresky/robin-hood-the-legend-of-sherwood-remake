//! Minimap rendering state and logic.
//!
//! The minimap is a UI widget that shows an overview map with dots for
//! actors, items, and highlighted elements.  This module captures the
//! serializable state and pure logic (coordinate conversion, dot
//! classification, display transitions); rendering is handled by
//! `game_render::render_minimap` using the GPU `Renderer`.

use serde::{Deserialize, Serialize};

use crate::coordinates::{MapPoint, MapSize, MinimapSize, ScreenBBox, ScreenPoint, ScreenSize};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Constants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Transition speed factor when closing the map.
const MINUS_VALUE: f32 = 0.1;

/// Number of frames between delayed highlight reveals.
const DELAYED_REFRESH_TIMEOUT: u32 = 25;

/// Dead zone around the minimap edges that is not part of the usable map area.
pub const NON_MAP_AREA: ScreenSize = ScreenSize { x: 14.0, y: 24.0 };

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// CustomDot — script-overridable dot appearance
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Custom minimap dot type that can be set on elements via script.
///
/// When set to anything other than `NotCustomized`, the element's minimap dot
/// is forced to the specified appearance regardless of the element's class.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
#[repr(u16)]
pub enum CustomDot {
    /// Element is invisible on the minimap.
    Invisible = 0,
    /// No override — use the default classification logic.
    #[default]
    NotCustomized = 1,

    Pc = 100,
    PcLying = 101,
    PcDead = 102,
    /// PC with multi-state (alive/unconscious/dead resolved at render time).
    PcMulti = 111,

    Villain = 200,
    VillainLying = 201,
    VillainDead = 202,
    VillainMulti = 222,

    Civilian = 300,
    CivilianLying = 301,
    CivilianDead = 302,
    CivilianMulti = 333,

    Vip = 400,
    VipLying = 401,
    VipDead = 402,
    VipMulti = 444,

    Item = 500,

    Animal = 666,
    // CUSTOM_DOT_HORSE = 777 was defined originally but no `RefreshElement`
    // arm handles it — the default-case assert would fire.  No shipped
    // script emits it, so we omit the variant entirely; `from_u16(777)`
    // panics.
}

impl CustomDot {
    pub fn from_u16(v: u16) -> Self {
        match v {
            0 => Self::Invisible,
            1 => Self::NotCustomized,
            100 => Self::Pc,
            101 => Self::PcLying,
            102 => Self::PcDead,
            111 => Self::PcMulti,
            200 => Self::Villain,
            201 => Self::VillainLying,
            202 => Self::VillainDead,
            222 => Self::VillainMulti,
            300 => Self::Civilian,
            301 => Self::CivilianLying,
            302 => Self::CivilianDead,
            333 => Self::CivilianMulti,
            400 => Self::Vip,
            401 => Self::VipLying,
            402 => Self::VipDead,
            444 => Self::VipMulti,
            500 => Self::Item,
            666 => Self::Animal,
            _ => panic!("undefined custom minimap dot ID: {v}"),
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DotType — the actual sprite index used for rendering
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The visual dot type rendered on the minimap.
///
/// Each variant maps to a sprite in the `RHMAP_ITEMS` resource.
/// The `as u16` value is the sprite frame index.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, robin_state_hash_derive::StateHash,
)]
#[repr(u16)]
pub enum DotType {
    Hero = 0,
    DeadHero = 1,
    StunnedHero = 2,
    Blip = 3,
    Enemy = 4,
    DeadEnemy = 5,
    StunnedEnemy = 6,
    Vip = 7,
    DeadVip = 8,
    StunnedVip = 9,
    Ally = 10,
    DeadAlly = 11,
    StunnedAlly = 12,
    Civilian = 13,
    DeadCivilian = 14,
    StunnedCivilian = 15,
    Animal = 16,
    Item = 17,
    Highlighted = 18,
    Projectile = 19,
    Scroll = 20,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UIState
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Mouse-interaction state of the minimap button.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    robin_state_hash_derive::StateHash,
)]
pub enum UIState {
    #[default]
    Default,
    Focused,
    Selected,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Camp — for determining ally vs enemy
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Camp / faction affiliation used to determine dot colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Camp {
    Lacklandists,
    Other,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// HighlightedElement — delayed-reveal element on the minimap
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// An element queued for delayed highlight reveal on the minimap.
///
/// When a script calls `SetHighlighted`, the element is added here and
/// revealed after a countdown, one at a time.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct HighlightedElement {
    /// Element index.
    pub element_index: u32,
    /// Whether this element's highlight has been revealed.
    pub refresh: bool,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// HitMask — pixel-level transparency bitmask
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Pre-computed bitmask of non-transparent pixels for a minimap surface.
///
/// We pre-compute the mask at load time so click-hit checks don't have to
/// lock the surface and read a pixel each frame.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct HitMask {
    width: u16,
    height: u16,
    /// One bool per pixel — `true` = opaque (hit), `false` = transparent (miss).
    opaque: Vec<bool>,
}

impl HitMask {
    /// Build a hit mask from raw 16-bit pixel data.
    pub fn from_pixels_u16(
        width: u16,
        height: u16,
        pixels: &[u16],
        transparent_color: u16,
    ) -> Self {
        let opaque = pixels.iter().map(|&px| px != transparent_color).collect();
        Self {
            width,
            height,
            opaque,
        }
    }

    /// Check if the pixel at `(x, y)` is opaque (non-transparent).
    pub fn is_opaque(&self, x: u16, y: u16) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        self.opaque[y as usize * self.width as usize + x as usize]
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// MinimapState — all serializable + runtime minimap state
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The minimap's complete state (save-game + runtime).
///
/// All mutating fields are crate-private — host code drives the
/// minimap exclusively through `PlayerCommand::Minimap*` variants via
/// [`EngineInner::apply_command`].  Read access is provided by `pub fn`
/// accessors (`is_displayed`, `map_box`, `button_box`, …).
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct MinimapState {
    // ── Serialized (save-game state) ──
    /// Whether the map should be opening (`true`) or closing (`false`).
    pub(crate) go_in: bool,

    /// Whether the full map is currently displayed.
    pub(crate) map_displayed: bool,

    /// Transition animation progress (0.0 = fully open/closed, >0 = animating).
    pub(crate) transition_counter: f32,

    /// Countdown timer for revealing the next highlighted element.
    pub(crate) highlight_refresh: u32,

    /// Whether the map should auto-close after all highlights are shown.
    pub(crate) close_after_highlight: bool,

    /// Whether the minimap position should be restored after highlight display.
    pub(crate) restore: bool,

    /// Saved map box to restore after highlight-driven centering.
    pub(crate) memory_box: ScreenBBox,

    /// Queue of elements with delayed highlight reveal.
    pub(crate) highlighted_elements: Vec<HighlightedElement>,

    // ── Host display interaction state ──
    /// Whether a drag operation is in progress.
    pub(crate) drag_start: bool,

    /// Whether the mouse has moved enough to count as a drag.
    pub(crate) dragged: bool,

    /// Whether the mouse entered the widget cleanly (not mid-drag).
    pub(crate) entered_nicely: bool,

    /// Whether the minimap has captured mouse input.
    pub(crate) capture: bool,

    /// Mouse position at drag start.
    pub(crate) dragging_point: ScreenPoint,

    /// Map box position before a drag started.
    pub(crate) position_before_dragging: ScreenPoint,

    /// Size of the map bitmap in pixels.
    pub(crate) map_size: MinimapSize,

    /// Current bounding box of the deployed map.
    pub(crate) map_box: ScreenBBox,

    /// Bounding box of the minimap button (collapsed state).
    pub(crate) button_box: ScreenBBox,

    /// Current UI interaction state.
    pub(crate) ui_state: UIState,

    /// Pixel-level hit mask for the deployed map bitmap.
    pub(crate) map_hit_mask: Option<HitMask>,

    /// Pixel-level hit mask for the collapsed button bitmap (RHMAP_CORNER frame 1).
    pub(crate) button_hit_mask: Option<HitMask>,

    /// Set whenever `set_minimap_position` accepts a new position.  The
    /// engine drains this into `SideEffects::pending_minimap_position`
    /// after each command-apply so the host can persist the new top-left
    /// to the active player profile.
    pub(crate) position_dirty: bool,
}

impl Default for MinimapState {
    fn default() -> Self {
        Self {
            go_in: false,
            map_displayed: false,
            transition_counter: 0.0,
            highlight_refresh: 0,
            close_after_highlight: false,
            restore: false,
            memory_box: ScreenBBox::new(),
            highlighted_elements: Vec::new(),
            drag_start: false,
            dragged: false,
            entered_nicely: false,
            capture: false,
            dragging_point: ScreenPoint::ZERO,
            position_before_dragging: ScreenPoint::ZERO,
            map_size: MinimapSize::ZERO,
            map_box: ScreenBBox::new(),
            button_box: ScreenBBox::new(),
            ui_state: UIState::Default,
            map_hit_mask: None,
            button_hit_mask: None,
            position_dirty: false,
        }
    }
}

impl MinimapState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_displayed(&self) -> bool {
        self.map_displayed
    }

    // ── Read-only field accessors for external renderers / hit-testers ──

    /// Current bounding box of the deployed map (`is_somewhere() == false`
    /// until the map has been positioned).
    pub fn map_box(&self) -> &ScreenBBox {
        &self.map_box
    }

    /// Bounding box of the collapsed minimap button.
    pub fn button_box(&self) -> &ScreenBBox {
        &self.button_box
    }

    /// Size of the map bitmap in pixels (zero vector until loaded).
    pub fn map_size(&self) -> MinimapSize {
        self.map_size
    }

    /// Open/close transition progress — 0.0 when fully open or closed.
    pub fn transition_counter(&self) -> f32 {
        self.transition_counter
    }

    /// Current mouse-interaction UI state (for rendering the corner
    /// button sprite frame).
    pub fn ui_state(&self) -> UIState {
        self.ui_state
    }

    /// Whether a drag is currently in progress (used by the host to
    /// suppress edge-scrolling and related input while dragging).
    pub fn drag_start(&self) -> bool {
        self.drag_start
    }

    // ── Hit testing ──

    /// Check if a screen position is "really" over the minimap widget.
    ///
    /// First checks the active bounding box (map when deployed, button when
    /// collapsed), then pixel-tests against the corresponding [`HitMask`].
    pub fn is_over_widget(&self, screen_pos: ScreenPoint) -> bool {
        let (active_box, mask) = if self.map_displayed {
            (&self.map_box, &self.map_hit_mask)
        } else {
            (&self.button_box, &self.button_hit_mask)
        };

        // Points exactly on the minimap's four edges register as outside,
        // matching the original `!IsOnBoundary && IsInside` check.
        if !active_box.contains_point(screen_pos) || active_box.is_on_boundary(screen_pos) {
            return false;
        }

        let local_x = screen_pos.x - active_box.top_left().x;
        let local_y = screen_pos.y - active_box.top_left().y;

        if local_x < 0.0 || local_y < 0.0 {
            return false;
        }

        match mask {
            Some(m) => m.is_opaque(local_x as u16, local_y as u16),
            None => true, // No mask loaded → bbox test already passed
        }
    }

    // ── Display transitions ──

    /// Open or close the map with optional quick (instant) mode.
    pub(crate) fn display_map(&mut self, on: bool, quick: bool) {
        if on && !self.map_displayed && self.transition_counter == 0.0 {
            if quick {
                self.transition_counter = 0.0;
                self.map_displayed = true;
            } else {
                self.transition_counter = 1.0;
                self.go_in = true;
            }
        }

        if !on && self.map_displayed && self.transition_counter == 0.0 {
            if quick {
                self.map_displayed = false;
                self.transition_counter = 0.0;
            } else {
                self.go_in = false;
                self.transition_counter = MINUS_VALUE;
            }
        }
    }

    /// Advance the open/close transition by one frame.
    ///
    /// Returns `true` if the map is fully deployed and ready for element
    /// rendering, `false` if still animating or collapsed.
    pub(crate) fn tick_transition(&mut self) -> bool {
        if self.transition_counter != 0.0 {
            if self.go_in {
                self.transition_counter /= 2.0;
                if self.transition_counter < 0.05 {
                    self.transition_counter = 0.0;
                    self.map_displayed = true;
                }
            } else {
                self.transition_counter *= 2.0;
                if self.transition_counter >= 1.0 {
                    self.transition_counter = 0.8;
                }

                if self.transition_counter >= 0.8 && !self.go_in {
                    self.transition_counter = 0.0;
                    self.map_displayed = false;

                    if self.restore {
                        self.map_box = self.memory_box;
                        self.restore = false;
                    }
                }
            }
            false
        } else {
            self.map_displayed
        }
    }

    // ── Highlighted elements ──

    /// Queue an element for delayed highlight on the minimap.  Called
    /// from [`EngineInner::reveal_scroll`] for each revealed scroll; the
    /// render loop queries `is_element_highlighted` for each entity
    /// and switches its dot to [`DotType::Highlighted`] when the
    /// delayed-refresh timer exposes it.
    ///
    /// [`EngineInner::reveal_scroll`]: crate::engine::EngineInner::reveal_scroll
    pub(crate) fn set_highlighted(&mut self, element_index: u32) {
        if !self.is_element_highlighted(element_index) {
            self.highlighted_elements.push(HighlightedElement {
                element_index,
                refresh: false,
            });
            self.highlight_refresh = DELAYED_REFRESH_TIMEOUT;
        }
    }

    /// Check if an element is already in the highlight queue.
    pub fn is_element_highlighted(&self, element_index: u32) -> bool {
        self.highlighted_elements
            .iter()
            .any(|h| h.element_index == element_index)
    }

    /// Read-only view of the delayed-reveal queue.  Each entry exposes
    /// its target `element_index` and a `refresh` flag — `true` once
    /// the reveal timer has surfaced it so the renderer should draw the
    /// highlighted dot.
    pub fn highlighted_elements(&self) -> &[HighlightedElement] {
        &self.highlighted_elements
    }

    /// Open the map for delayed element display.
    /// Centers the map if it was closed, and flags for auto-close
    /// afterwards.  Called by [`EngineInner::reveal_scrolls`] once the
    /// beggar's current scroll set has been queued via
    /// [`Self::set_highlighted`].
    ///
    /// [`EngineInner::reveal_scrolls`]: crate::engine::EngineInner::reveal_scrolls
    pub(crate) fn display_for_delayed_elements(&mut self, screen_width: f32, screen_height: f32) {
        self.close_after_highlight = !self.map_displayed;

        if self.close_after_highlight && self.map_box.is_somewhere() {
            self.memory_box = self.map_box;
            self.restore = true;

            let map_w = self.map_box.width();
            let map_h = self.map_box.height();
            let new_x = (screen_width - map_w) * 0.5;
            let new_y = (screen_height - map_h) * 0.5;
            self.map_box = ScreenBBox::from_coords(new_x, new_y, new_x + map_w, new_y + map_h);
        }

        self.display_map(true, false);
    }

    /// Advance the highlight reveal logic by one frame.
    ///
    /// After this runs, revealed highlights can be queried via
    /// [`MinimapState::is_element_highlighted`]; the `refresh` flag on
    /// each entry records whether its dot is currently being drawn.
    pub(crate) fn tick_highlights(&mut self) {
        if self.highlight_refresh == 0 && !self.highlighted_elements.is_empty() {
            // Find the next unrevealed element.
            let next_unrevealed = self.highlighted_elements.iter().position(|h| !h.refresh);

            if let Some(idx) = next_unrevealed {
                let remaining = self.highlighted_elements.len() - idx - 1;
                self.highlighted_elements[idx].refresh = true;

                if remaining >= 1 {
                    self.highlight_refresh = DELAYED_REFRESH_TIMEOUT;
                } else {
                    // After the last highlight is revealed, arm the 2×
                    // grace period before clean-up runs on the next tick.
                    self.highlight_refresh = 2 * DELAYED_REFRESH_TIMEOUT;
                }
            } else {
                // All highlights revealed — clean up.
                self.highlighted_elements.clear();
                if self.close_after_highlight {
                    self.close_after_highlight = false;
                    self.display_map(false, false);
                }
            }
        }

        if self.highlight_refresh != 0 {
            self.highlight_refresh -= 1;
        }
    }

    // ── Coordinate conversion ──

    /// Convert a world (engine) position to a minimap pixel position.
    ///
    /// `level_size` is the total projected level dimensions in map units.
    pub fn real_to_map(&self, engine_pos: MapPoint, level_size: MapSize) -> Option<ScreenPoint> {
        if !self.map_box.is_somewhere() {
            return None;
        }

        let usable = ScreenBBox::from_corners(
            ScreenPoint::new(
                self.map_box.top_left().x + NON_MAP_AREA.x,
                self.map_box.top_left().y + NON_MAP_AREA.y,
            ),
            ScreenPoint::new(
                self.map_box.bottom_right().x - NON_MAP_AREA.x,
                self.map_box.bottom_right().y - NON_MAP_AREA.y,
            ),
        );

        if level_size.x == 0.0 || level_size.y == 0.0 {
            return None;
        }

        let fx = engine_pos.x / level_size.x;
        let fy = engine_pos.y / level_size.y;

        Some(ScreenPoint::new(
            (usable.top_left().x + usable.width() * fx).floor(),
            (usable.top_left().y + usable.height() * fy).floor(),
        ))
    }

    /// Convert a minimap pixel position back to a world (engine) position.
    ///
    /// `level_size` is the total projected level dimensions in map units.
    pub fn map_to_real(&self, map_pos: ScreenPoint, level_size: MapSize) -> Option<MapPoint> {
        if !self.map_box.is_somewhere() {
            return None;
        }

        let usable = ScreenBBox::from_corners(
            ScreenPoint::new(
                self.map_box.top_left().x + NON_MAP_AREA.x,
                self.map_box.top_left().y + NON_MAP_AREA.y,
            ),
            ScreenPoint::new(
                self.map_box.bottom_right().x - NON_MAP_AREA.x,
                self.map_box.bottom_right().y - NON_MAP_AREA.y,
            ),
        );

        let w = usable.width();
        let h = usable.height();
        if w == 0.0 || h == 0.0 {
            return None;
        }

        let local = ScreenPoint::new(
            map_pos.x - usable.top_left().x,
            map_pos.y - usable.top_left().y,
        );
        let fx = local.x / w;
        let fy = local.y / h;

        Some(MapPoint::new(level_size.x * fx, level_size.y * fy))
    }

    /// Set the widget base position and re-derive button and map boxes.
    ///
    /// Called during init and on resolution change with
    /// `(screen_width - 83, 38)`.  `corner_size` is the pixel size of the
    /// RHMAP_CORNER resource.
    pub(crate) fn set_widget_position(
        &mut self,
        base: ScreenPoint,
        corner_size: ScreenSize,
        screen_width: f32,
        screen_height: f32,
    ) {
        // button_box = base position sized to corner sprite
        self.button_box = ScreenBBox::from_coords(
            base.x,
            base.y,
            base.x + corner_size.x,
            base.y + corner_size.y,
        );

        // Re-validate map position (derives default from button if off-screen).
        // We always call `set_minimap_position` here even if the bitmap is
        // not yet loaded; the function early-returns when `map_size` is not
        // yet populated, so the call is safe before `setup_minimap_map` has
        // run.
        let map_tl = if self.map_box.is_somewhere() {
            self.map_box.top_left()
        } else {
            // Pre-init re-entry: feed the sentinel so the function takes
            // the default-fallback path once `map_size` has been set.
            ScreenPoint::new(65536.0, 65536.0)
        };
        self.set_minimap_position(map_tl, screen_width, screen_height);
    }

    /// Validate and set the minimap position when deployed.
    ///
    /// Returns `true` if the position was valid and accepted.
    pub(crate) fn set_minimap_position(
        &mut self,
        position: ScreenPoint,
        screen_width: f32,
        screen_height: f32,
    ) -> bool {
        // Allow callers to fire before the minimap bitmap is loaded
        // (e.g. window resize before `setup_minimap_map`); without
        // `map_size` the test/full boxes would be degenerate and the
        // default-fallback would snap to a zero-area box at the button
        // corner.
        if self.map_size.x <= 0.0 || self.map_size.y <= 0.0 {
            return false;
        }

        let screen_box = ScreenBBox::from_coords(0.0, 0.0, screen_width, screen_height);

        let map_w = self.map_size.x;
        let map_h = self.map_size.y;

        // Test box inset by the non-map dead zone.
        let test_box = ScreenBBox::from_corners(
            ScreenPoint::new(position.x + NON_MAP_AREA.x, position.y + NON_MAP_AREA.y),
            ScreenPoint::new(
                position.x + map_w - NON_MAP_AREA.x,
                position.y + map_h - NON_MAP_AREA.y,
            ),
        );

        if screen_box.intersects_bbox(&test_box) {
            self.map_box = ScreenBBox::from_coords(
                position.x,
                position.y,
                position.x + map_w,
                position.y + map_h,
            );
            // The new top-left should be written back to the active player
            // profile.  The engine doesn't carry a profile reference; flag
            // dirty and let the host drain via
            // `SideEffects::pending_minimap_position`.
            self.position_dirty = true;
            return true;
        }

        // Initial or completely off-screen: snap to default position.
        let sentinel = ScreenPoint::new(65536.0, 65536.0);
        let full_map_box = ScreenBBox::from_coords(
            position.x,
            position.y,
            position.x + map_w,
            position.y + map_h,
        );
        let is_sentinel =
            (position.x - sentinel.x).abs() < 1.0 && (position.y - sentinel.y).abs() < 1.0;
        if (is_sentinel || !screen_box.intersects_bbox(&full_map_box))
            && self.button_box.is_somewhere()
        {
            let default_x = self.button_box.x_min() - map_w;
            let default_y = self.button_box.y_max();
            self.map_box =
                ScreenBBox::from_coords(default_x, default_y, default_x + map_w, default_y + map_h);
        }

        false
    }

    /// Take the dirty-position flag, returning the current map top-left
    /// when it was set.  Used by the engine's command-apply path to
    /// propagate drag-induced position changes into
    /// [`SideEffects::pending_minimap_position`].
    pub(crate) fn take_pending_position(&mut self) -> Option<ScreenPoint> {
        if self.position_dirty && self.map_box.is_somewhere() {
            self.position_dirty = false;
            Some(self.map_box.top_left())
        } else {
            self.position_dirty = false;
            None
        }
    }

    /// Force-start the open animation, bypassing the
    /// `transition_counter == 0.0` guard in [`Self::display_map`].
    /// Force-start the open animation: an in-flight close immediately
    /// reverses regardless of any transition guard.
    pub(crate) fn force_open_animation(&mut self) {
        self.transition_counter = 1.0;
        self.go_in = true;
    }

    /// Force-start the close animation, bypassing the
    /// `transition_counter == 0.0` guard in [`Self::display_map`].
    /// Used by the fast-key close shortcut and the right-click arm.
    pub(crate) fn force_close_animation(&mut self) {
        self.ui_state = UIState::Selected;
        self.go_in = false;
        self.transition_counter = MINUS_VALUE;
    }

    // ── Dragging ──

    /// Begin or continue a drag operation.
    ///
    /// The first call (while `drag_start` is false) records the starting
    /// position; subsequent calls move the minimap window once the
    /// displacement exceeds 15 px.
    pub(crate) fn manage_dragging(
        &mut self,
        mouse_pos: ScreenPoint,
        screen_width: f32,
        screen_height: f32,
    ) {
        if !self.map_displayed {
            return;
        }

        if !self.drag_start {
            if self.map_box.is_somewhere() {
                self.position_before_dragging = self.map_box.top_left();
            }
            self.dragging_point = mouse_pos;
            self.drag_start = true;
            self.restore = false;
        } else {
            // Edge-scrolling suppression while dragging is handled by the
            // caller, which checks `drag_start` before issuing scroll
            // actions.
            let dx = mouse_pos.x - self.dragging_point.x;
            let dy = mouse_pos.y - self.dragging_point.y;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq > 15.0 * 15.0 {
                let new_pos = ScreenPoint::new(
                    self.position_before_dragging.x + dx,
                    self.position_before_dragging.y + dy,
                );
                self.set_minimap_position(new_pos, screen_width, screen_height);
                self.dragged = true;
            }
        }

        self.close_after_highlight = false;
    }

    /// Handle a click on the minimap.
    pub(crate) fn manage_click(&mut self) {
        self.drag_start = false;
        if !self.dragged {
            if !self.map_displayed {
                self.ui_state = UIState::Selected;
                self.transition_counter = 1.0;
                self.go_in = true;
            }
            // If map is displayed, the caller should handle camera centering
            // by calling map_to_real with the click position.
        } else {
            self.dragged = false;
        }
        self.close_after_highlight = false;
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Element classification for minimap dots
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Compute the usable map area (excluding the dead zone border).
///
/// Click hit-testing insets [`NON_MAP_AREA`] from the full map bounding
/// box; this helper produces that inset rectangle.
pub fn usable_area(map_box: &ScreenBBox) -> ScreenBBox {
    if !map_box.is_somewhere() {
        return ScreenBBox::new();
    }
    ScreenBBox::from_corners(
        ScreenPoint::new(
            map_box.top_left().x + NON_MAP_AREA.x,
            map_box.top_left().y + NON_MAP_AREA.y,
        ),
        ScreenPoint::new(
            map_box.bottom_right().x - NON_MAP_AREA.x,
            map_box.bottom_right().y - NON_MAP_AREA.y,
        ),
    )
}

/// Properties of an element needed to determine its minimap dot type.
///
/// Decouples the dot classification logic from the element hierarchy.
/// The caller fills this in from the element's properties.
#[derive(Debug, Clone)]
pub struct ElementDotInfo {
    pub custom_dot: CustomDot,
    pub is_active: bool,
    pub is_human: bool,
    pub is_object: bool,
    pub is_projectile: bool,
    pub is_scroll: bool,
    pub is_pc: bool,
    pub is_soldier: bool,
    pub is_civilian: bool,
    pub is_civilian_vip: bool,
    pub is_vip: bool,
    pub is_blipped: bool,
    pub is_dead: bool,
    pub is_unconscious: bool,
    pub posture_lying: bool,
    pub camp: Camp,
}

/// Determine the minimap dot type for an element.
///
/// Returns `None` if the element should not be displayed (invisible or
/// inactive).
pub fn classify_element_dot(info: &ElementDotInfo) -> Option<DotType> {
    if !info.is_active {
        return None;
    }

    match info.custom_dot {
        CustomDot::Invisible => None,

        CustomDot::NotCustomized => classify_default(info),

        CustomDot::Pc => Some(DotType::Hero),
        CustomDot::PcLying => Some(DotType::StunnedHero),
        CustomDot::PcDead => Some(DotType::DeadHero),
        CustomDot::PcMulti => {
            classify_multi_state(info, DotType::Hero, DotType::StunnedHero, DotType::DeadHero)
        }

        CustomDot::Villain => Some(DotType::Enemy),
        CustomDot::VillainLying => Some(DotType::StunnedEnemy),
        CustomDot::VillainDead => Some(DotType::DeadEnemy),
        CustomDot::VillainMulti => classify_multi_state(
            info,
            DotType::Enemy,
            DotType::StunnedEnemy,
            DotType::DeadEnemy,
        ),

        CustomDot::Civilian => Some(DotType::Civilian),
        CustomDot::CivilianLying => Some(DotType::StunnedCivilian),
        CustomDot::CivilianDead => Some(DotType::DeadCivilian),
        CustomDot::CivilianMulti => classify_multi_state(
            info,
            DotType::Civilian,
            DotType::StunnedCivilian,
            DotType::DeadCivilian,
        ),

        CustomDot::Vip => Some(DotType::Vip),
        CustomDot::VipLying => Some(DotType::StunnedVip),
        CustomDot::VipDead => Some(DotType::DeadVip),
        CustomDot::VipMulti => {
            classify_multi_state(info, DotType::Vip, DotType::StunnedVip, DotType::DeadVip)
        }

        CustomDot::Animal => Some(DotType::Animal),
        CustomDot::Item => Some(DotType::Item),
    }
}

/// Classify a "multi" custom dot based on alive/unconscious/dead state.
fn classify_multi_state(
    info: &ElementDotInfo,
    alive: DotType,
    stunned: DotType,
    dead: DotType,
) -> Option<DotType> {
    if !info.is_human {
        return None;
    }
    if info.is_dead {
        Some(dead)
    } else if info.is_unconscious {
        Some(stunned)
    } else {
        Some(alive)
    }
}

/// Default dot classification when no custom dot override is set.
fn classify_default(info: &ElementDotInfo) -> Option<DotType> {
    if info.is_human {
        if info.is_pc {
            if info.is_dead || (info.posture_lying && !info.is_unconscious) {
                Some(DotType::DeadHero)
            } else if info.is_unconscious {
                Some(DotType::StunnedHero)
            } else {
                Some(DotType::Hero)
            }
        } else if info.is_soldier {
            classify_npc_soldier(info)
        } else if info.is_civilian {
            classify_npc_civilian(info)
        } else {
            None
        }
    } else if info.is_object {
        // The legacy minimap logic had an animal arm here that
        // mapped to `DotType::Animal`.  The animal actor subsystem was
        // intentionally deleted (see element.rs:9), so no shipped mission
        // spawns an animal element and the arm is dead in practice; not
        // ported.
        if info.is_scroll {
            Some(DotType::Scroll)
        } else if info.is_projectile {
            Some(DotType::Projectile)
        } else {
            Some(DotType::Item)
        }
    } else {
        None
    }
}

fn classify_npc_soldier(info: &ElementDotInfo) -> Option<DotType> {
    if info.is_blipped {
        return Some(DotType::Blip);
    }
    if info.is_vip {
        return if info.is_dead {
            Some(DotType::DeadVip)
        } else if info.is_unconscious {
            Some(DotType::StunnedVip)
        } else {
            Some(DotType::Vip)
        };
    }
    let is_enemy = info.camp == Camp::Lacklandists;
    if info.is_dead {
        Some(if is_enemy {
            DotType::DeadEnemy
        } else {
            DotType::DeadAlly
        })
    } else if info.is_unconscious {
        Some(if is_enemy {
            DotType::StunnedEnemy
        } else {
            DotType::StunnedAlly
        })
    } else {
        Some(if is_enemy {
            DotType::Enemy
        } else {
            DotType::Ally
        })
    }
}

fn classify_npc_civilian(info: &ElementDotInfo) -> Option<DotType> {
    if info.is_blipped {
        return Some(DotType::Blip);
    }
    if info.is_civilian_vip {
        return if info.is_dead {
            Some(DotType::DeadVip)
        } else if info.is_unconscious {
            Some(DotType::StunnedVip)
        } else {
            Some(DotType::Vip)
        };
    }
    if info.is_dead {
        Some(DotType::DeadCivilian)
    } else if info.is_unconscious {
        Some(DotType::StunnedCivilian)
    } else {
        Some(DotType::Civilian)
    }
}

/// Compute element display priority for minimap sorting.
///
/// Higher priority elements are drawn later (on top).
///
/// The original priority code packed an `(IsAnimal() << 11)` bit; that
/// bit is intentionally omitted here because the animal actor subsystem
/// was deleted (see `element.rs:9`).  Animals never spawn in shipped
/// missions, so the `IsAnimal()` bit is always 0 and unreachable.
pub fn element_priority(is_object: bool, is_pc: bool, is_soldier: bool) -> u32 {
    ((is_object as u32) << 16)
        | ((is_pc as u32) << 15)
        | ((is_soldier as u32) << 14)
        | ((is_soldier as u32) << 13)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    fn default_info() -> ElementDotInfo {
        ElementDotInfo {
            custom_dot: CustomDot::NotCustomized,
            is_active: true,
            is_human: false,
            is_object: false,
            is_projectile: false,
            is_scroll: false,
            is_pc: false,
            is_soldier: false,
            is_civilian: false,
            is_civilian_vip: false,
            is_vip: false,
            is_blipped: false,
            is_dead: false,
            is_unconscious: false,
            posture_lying: false,
            camp: Camp::Other,
        }
    }

    #[test]
    fn classify_hero_alive() {
        let info = ElementDotInfo {
            is_human: true,
            is_pc: true,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), Some(DotType::Hero));
    }

    #[test]
    fn classify_hero_dead() {
        let info = ElementDotInfo {
            is_human: true,
            is_pc: true,
            is_dead: true,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), Some(DotType::DeadHero));
    }

    #[test]
    fn classify_hero_stunned() {
        let info = ElementDotInfo {
            is_human: true,
            is_pc: true,
            is_unconscious: true,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), Some(DotType::StunnedHero));
    }

    #[test]
    fn classify_hero_lying_conscious() {
        // PC that is lying but not unconscious shows as dead (hiding).
        let info = ElementDotInfo {
            is_human: true,
            is_pc: true,
            posture_lying: true,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), Some(DotType::DeadHero));
    }

    #[test]
    fn classify_enemy_soldier() {
        let info = ElementDotInfo {
            is_human: true,
            is_soldier: true,
            camp: Camp::Lacklandists,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), Some(DotType::Enemy));
    }

    #[test]
    fn classify_ally_soldier() {
        let info = ElementDotInfo {
            is_human: true,
            is_soldier: true,
            camp: Camp::Other,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), Some(DotType::Ally));
    }

    #[test]
    fn classify_vip_soldier() {
        let info = ElementDotInfo {
            is_human: true,
            is_soldier: true,
            is_vip: true,
            camp: Camp::Lacklandists,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), Some(DotType::Vip));
    }

    #[test]
    fn classify_blipped_soldier() {
        let info = ElementDotInfo {
            is_human: true,
            is_soldier: true,
            is_blipped: true,
            camp: Camp::Lacklandists,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), Some(DotType::Blip));
    }

    #[test]
    fn classify_civilian() {
        let info = ElementDotInfo {
            is_human: true,
            is_civilian: true,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), Some(DotType::Civilian));
    }

    #[test]
    fn classify_civilian_vip() {
        let info = ElementDotInfo {
            is_human: true,
            is_civilian: true,
            is_civilian_vip: true,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), Some(DotType::Vip));
    }

    #[test]
    fn classify_object_item() {
        let info = ElementDotInfo {
            is_object: true,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), Some(DotType::Item));
    }

    #[test]
    fn classify_scroll() {
        let info = ElementDotInfo {
            is_object: true,
            is_scroll: true,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), Some(DotType::Scroll));
    }

    #[test]
    fn classify_projectile() {
        let info = ElementDotInfo {
            is_object: true,
            is_projectile: true,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), Some(DotType::Projectile));
    }

    #[test]
    fn classify_invisible() {
        let info = ElementDotInfo {
            custom_dot: CustomDot::Invisible,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), None);
    }

    #[test]
    fn classify_inactive() {
        let info = ElementDotInfo {
            is_active: false,
            is_human: true,
            is_pc: true,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), None);
    }

    #[test]
    fn classify_custom_villain_multi_dead() {
        let info = ElementDotInfo {
            custom_dot: CustomDot::VillainMulti,
            is_human: true,
            is_dead: true,
            ..default_info()
        };
        assert_eq!(classify_element_dot(&info), Some(DotType::DeadEnemy));
    }

    #[test]
    fn custom_dot_from_u16() {
        assert_eq!(CustomDot::from_u16(0), CustomDot::Invisible);
        assert_eq!(CustomDot::from_u16(1), CustomDot::NotCustomized);
        assert_eq!(CustomDot::from_u16(100), CustomDot::Pc);
        assert_eq!(CustomDot::from_u16(222), CustomDot::VillainMulti);
        assert_eq!(CustomDot::from_u16(666), CustomDot::Animal);
    }

    #[test]
    fn minimap_display_transition() {
        let mut mm = MinimapState::new();
        assert!(!mm.is_displayed());

        // Open quickly.
        mm.display_map(true, true);
        assert!(mm.is_displayed());

        // Close with animation.
        mm.display_map(false, false);
        assert_eq!(mm.transition_counter, MINUS_VALUE);
        assert!(!mm.go_in);

        // Tick until closed.
        for _ in 0..20 {
            mm.tick_transition();
            if !mm.is_displayed() && mm.transition_counter == 0.0 {
                break;
            }
        }
        assert!(!mm.is_displayed());
    }

    #[test]
    fn minimap_open_animation() {
        let mut mm = MinimapState::new();
        mm.display_map(true, false);
        assert_eq!(mm.transition_counter, 1.0);
        assert!(mm.go_in);

        // Tick until open.
        for _ in 0..50 {
            let ready = mm.tick_transition();
            if ready {
                break;
            }
        }
        assert!(mm.is_displayed());
    }

    #[test]
    fn minimap_highlight_flow() {
        let mut mm = MinimapState::new();
        mm.set_highlighted(10);
        mm.set_highlighted(20);
        assert_eq!(mm.highlighted_elements.len(), 2);
        assert_eq!(mm.highlight_refresh, DELAYED_REFRESH_TIMEOUT);

        // Duplicates are ignored.
        mm.set_highlighted(10);
        assert_eq!(mm.highlighted_elements.len(), 2);

        // Tick down the timer (25 ticks to reach 0, then 1 more to trigger reveal).
        for _ in 0..=DELAYED_REFRESH_TIMEOUT {
            mm.tick_highlights();
        }

        // First element should be revealed now.
        assert!(mm.highlighted_elements[0].refresh);
        assert!(!mm.highlighted_elements[1].refresh);
    }

    #[test]
    fn real_to_map_conversion() {
        let mut mm = MinimapState::new();
        mm.map_box = ScreenBBox::from_coords(100.0, 100.0, 300.0, 300.0);

        let level_size = MapSize::new(1000.0, 1000.0);

        // Top-left corner of the world → near top-left of usable area.
        let result = mm.real_to_map(MapPoint::new(0.0, 0.0), level_size).unwrap();
        let usable_tl_x = 100.0 + NON_MAP_AREA.x;
        let usable_tl_y = 100.0 + NON_MAP_AREA.y;
        assert!((result.x - usable_tl_x).abs() < 1.0);
        assert!((result.y - usable_tl_y).abs() < 1.0);

        // Center of the world → center of usable area.
        let result = mm
            .real_to_map(MapPoint::new(500.0, 500.0), level_size)
            .unwrap();
        let usable_cx = usable_tl_x + (300.0 - 100.0 - 2.0 * NON_MAP_AREA.x) * 0.5;
        let usable_cy = usable_tl_y + (300.0 - 100.0 - 2.0 * NON_MAP_AREA.y) * 0.5;
        assert!((result.x - usable_cx).abs() < 1.0);
        assert!((result.y - usable_cy).abs() < 1.0);
    }

    #[test]
    fn element_priority_ordering() {
        let prio_pc = element_priority(false, true, false);
        let prio_soldier = element_priority(false, false, true);
        let prio_object = element_priority(true, false, false);

        // Objects have the lowest priority (drawn first, behind everything).
        assert!(prio_object > prio_pc);
        // PC > soldier.
        assert!(prio_pc > prio_soldier);
    }

    #[test]
    fn minimap_serde_roundtrip() {
        let mut mm = MinimapState::new();
        mm.go_in = true;
        mm.map_displayed = true;
        mm.transition_counter = 0.5;
        mm.close_after_highlight = true;
        mm.set_highlighted(42);
        mm.highlight_refresh = 10;

        let json = serde_json::to_string(&mm).unwrap();
        let de: MinimapState = serde_json::from_str(&json).unwrap();

        assert!(de.go_in);
        assert!(de.map_displayed);
        assert!((de.transition_counter - 0.5).abs() < 1e-6);
        assert_eq!(de.highlight_refresh, 10);
        assert!(de.close_after_highlight);
        assert_eq!(de.highlighted_elements.len(), 1);
        assert_eq!(de.highlighted_elements[0].element_index, 42);

        // Runtime fields reset to defaults.
        assert!(!de.drag_start);
    }
}
