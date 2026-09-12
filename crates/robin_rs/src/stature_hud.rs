//! Stature (stand-up / crouch-down) arrow widgets on the lower panel.
//!
//! The two arrow widgets sit on the lower panel:
//! * Positioned relative to the lower-panel origin
//!   (`(0, height - PANNEL_HEIGHT)`).
//! * Enable/selected state derived from the aggregate `Stature` of the
//!   current selection:
//!   - `None` → both disabled
//!   - `Down` → up-arrow enabled (stand everyone up)
//!   - `Up` → down-arrow enabled (crouch everyone down)
//!   - `Both` → both enabled
//! * Left-clicks issue `PlayerCommand::StandUp` / `PlayerCommand::CrouchDown`,
//!   which go through the same engine dispatch as the keyboard accelerators.
//!
//! Unlike the Sherwood start/quit-mission or corner-HUD buttons, these
//! widgets are driven off the live sim state
//! (`EngineInner::retrieve_stature`) every frame rather than cached
//! host-side — the aggregate stature can shift any frame the selection
//! or posture changes.

use crate::gfx_types::{Point, Rect as ScreenRect};

use robin_engine::engine::{PANNEL_HEIGHT, Stature};
use robin_engine::resource_ids::{RHID_DOWN_ARROW, RHID_UP_ARROW};

use crate::ingame_menu::layout::button_sprite_state;
use crate::renderer::Renderer;
use robin_engine::player_command::PlayerCommand;

/// Which stature arrow widget was hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StatureButton {
    Up,
    Down,
}

impl StatureButton {
    pub fn as_command(self) -> PlayerCommand {
        match self {
            StatureButton::Up => PlayerCommand::StandUp,
            StatureButton::Down => PlayerCommand::CrouchDown,
        }
    }
}

/// Per-frame enable mask derived from [`Stature`] and the focus-latch
/// flags.
///
/// `selected_up` / `selected_down` represent the visually-pressed state
/// during a stature transition: the widget that initiated the
/// transition stays visually pressed, while the opposite arrow is
/// dimmed.  The latch clears via `StatureFocusLatch::maybe_clear` when
/// the aggregate stature shifts.
#[derive(Debug, Clone, Copy, Default)]
pub struct StatureEnable {
    pub up_enabled: bool,
    pub down_enabled: bool,
    pub selected_up: bool,
    pub selected_down: bool,
}

impl StatureEnable {
    pub fn from_stature(s: Stature) -> Self {
        match s {
            Stature::None => Self {
                up_enabled: false,
                down_enabled: false,
                selected_up: false,
                selected_down: false,
            },
            // At least one PC is already crouched → expose the up-arrow
            // to stand everyone up.
            Stature::Down => Self {
                up_enabled: true,
                down_enabled: false,
                selected_up: false,
                selected_down: false,
            },
            // At least one PC is upright → expose down-arrow.
            Stature::Up => Self {
                up_enabled: false,
                down_enabled: true,
                selected_up: false,
                selected_down: false,
            },
            Stature::Both => Self {
                up_enabled: true,
                down_enabled: true,
                selected_up: false,
                selected_down: false,
            },
        }
    }

    /// Overlay the focus-latch onto a stature-derived mask.  While a
    /// stand-up transition is in flight, the up-arrow reads as
    /// enabled + selected and the down-arrow as disabled; the
    /// crouch-down case is symmetric.  The latch takes precedence over
    /// the standard enable/selected state during the transition.
    pub fn with_focus_latch(mut self, latch: StatureFocusLatch) -> Self {
        if latch.focus_standing_up {
            self.up_enabled = true;
            self.selected_up = true;
            self.down_enabled = false;
            self.selected_down = false;
        }
        if latch.focus_crouching_down {
            self.down_enabled = true;
            self.selected_down = true;
            self.up_enabled = false;
            self.selected_up = false;
        }
        self
    }

    fn enabled_for(self, btn: StatureButton) -> bool {
        match btn {
            StatureButton::Up => self.up_enabled,
            StatureButton::Down => self.down_enabled,
        }
    }

    fn selected_for(self, btn: StatureButton) -> bool {
        match btn {
            StatureButton::Up => self.selected_up,
            StatureButton::Down => self.selected_down,
        }
    }
}

/// "Player has pressed a stature-change widget and the sim transition
/// is still in flight" — used to latch the initiating arrow into a
/// visually-pressed state for the duration.
///
/// We don't yet emit a dedicated transition-complete message — instead
/// we snapshot the aggregate `Stature` at the moment the command is
/// issued and auto-clear the latch the first frame the stature changes.
/// The observable behaviour: the arrow stays visually pressed until
/// some PC actually completes its transition.
#[derive(Debug, Clone, Copy, Default)]
pub struct StatureFocusLatch {
    pub focus_standing_up: bool,
    pub focus_crouching_down: bool,
    /// Aggregate `Stature` captured at latch time — used to detect
    /// when the transition completes.
    pub stature_at_latch: Option<Stature>,
}

impl StatureFocusLatch {
    /// Record a stand-up intent.
    pub fn latch_stand_up(&mut self, current: Stature) {
        self.focus_standing_up = true;
        self.focus_crouching_down = false;
        self.stature_at_latch = Some(current);
    }

    /// Record a crouch-down intent.
    pub fn latch_crouch_down(&mut self, current: Stature) {
        self.focus_crouching_down = true;
        self.focus_standing_up = false;
        self.stature_at_latch = Some(current);
    }

    /// Auto-clear the latch once the aggregate stature changes.
    pub fn maybe_clear(&mut self, current: Stature) {
        if let Some(snap) = self.stature_at_latch
            && snap != current
        {
            self.focus_standing_up = false;
            self.focus_crouching_down = false;
            self.stature_at_latch = None;
        }
    }
}

/// Screen-space bounding boxes for the two stature buttons.
#[derive(Debug, Clone, Copy)]
pub struct StatureHudLayout {
    pub up: ScreenRect,
    pub down: ScreenRect,
}

impl StatureHudLayout {
    /// Derive rects from the current screen resolution. Both arrows
    /// are positioned relative to the lower-panel origin
    /// `(0, height - PANNEL_HEIGHT)` (see
    /// `corner_hud::CornerHudLayout::for_resolution` for the same
    /// derivation). Missing sprites fall back to a 32x32 hit box; drawing
    /// still skips the missing sprite.
    pub fn for_resolution(_screen_w: u32, screen_h: u32, sprites: &StatureSprites) -> Self {
        const FALLBACK_W: u32 = 32;
        const FALLBACK_H: u32 = 32;

        let frame_origin_y = screen_h as i32 - PANNEL_HEIGHT as i32;

        let (up_w, up_h) = sprites
            .size(StatureButton::Up)
            .unwrap_or((FALLBACK_W as u16, FALLBACK_H as u16));
        let (down_w, down_h) = sprites
            .size(StatureButton::Down)
            .unwrap_or((FALLBACK_W as u16, FALLBACK_H as u16));

        // Up arrow at (1, -27) from the panel origin, down arrow at (0, 33).
        Self {
            up: ScreenRect::new(1, frame_origin_y - 27, up_w as u32, up_h as u32),
            down: ScreenRect::new(0, frame_origin_y + 33, down_w as u32, down_h as u32),
        }
    }

    pub fn hit_test(&self, x: i32, y: i32, enable: StatureEnable) -> Option<StatureButton> {
        let pt = Point::new(x, y);
        if enable.up_enabled && self.up.contains_point(pt) {
            return Some(StatureButton::Up);
        }
        if enable.down_enabled && self.down.contains_point(pt) {
            return Some(StatureButton::Down);
        }
        None
    }

    /// Purely geometric hit-test — ignores the enable mask.  Used by
    /// the hover tracker so the tooltip still shows when the arrow is
    /// disabled (tooltips are tied to the widget rect, not its enable
    /// state).
    pub fn hit_test_geometric(&self, x: i32, y: i32) -> Option<StatureButton> {
        self.hit_test(
            x,
            y,
            StatureEnable {
                up_enabled: true,
                down_enabled: true,
                ..Default::default()
            },
        )
    }
}

pub type StatureSprites = crate::hud_sprite::ButtonSprites<StatureButton, 2>;
pub type StatureHoverState = crate::hud_sprite::HoverState<StatureButton>;

impl crate::hud_sprite::HudButton<2> for StatureButton {
    const ALL: [Self; 2] = [Self::Up, Self::Down];
    fn index(self) -> usize {
        self as usize
    }
    fn resource(self) -> (i32, &'static str) {
        match self {
            Self::Up => (RHID_UP_ARROW, "StatureUp"),
            Self::Down => (RHID_DOWN_ARROW, "StatureDown"),
        }
    }
}

pub fn draw_with_sprites(
    renderer: &mut Renderer,
    layout: &StatureHudLayout,
    enable: StatureEnable,
    hover: StatureHoverState,
    sprites: &StatureSprites,
) {
    for (rect, btn) in [
        (&layout.up, StatureButton::Up),
        (&layout.down, StatureButton::Down),
    ] {
        if !enable.enabled_for(btn) {
            continue;
        }
        let hovered = hover.hovered == Some(btn);
        let selected = enable.selected_for(btn);
        // Latched `selected` (transition-in-progress) overrides hover
        // and pressed.
        let pressed = selected || (hovered && hover.mouse_pressed);
        let state = button_sprite_state(true, hovered || selected, pressed);

        sprites.draw(renderer, btn, *rect, state, 0);
    }
}

pub type StatureTooltipTracker = crate::hud_sprite::ButtonTooltipTracker<StatureButton>;

/// Menu-text id for the tooltip attached to the given stature button.
pub fn stature_button_tooltip_mt_id(btn: StatureButton) -> usize {
    use crate::ingame_menu::resources::{MT_INFOBULLE_CROUCH, MT_INFOBULLE_STANDUP};
    match btn {
        StatureButton::Up => MT_INFOBULLE_STANDUP,
        StatureButton::Down => MT_INFOBULLE_CROUCH,
    }
}

pub use crate::hud_sprite::{TooltipPlacement, draw_tooltip};

#[cfg(test)]
mod hit_order_tests {
    use super::*;

    #[test]
    fn geometric_hits_ignore_enable_state_but_preserve_overlap_priority() {
        let rect = ScreenRect::new(0, 0, 10, 10);
        let layout = StatureHudLayout {
            up: rect,
            down: rect,
        };
        assert_eq!(layout.hit_test_geometric(1, 1), Some(StatureButton::Up));
        assert_eq!(layout.hit_test(1, 1, StatureEnable::default()), None);
        let last_only = StatureEnable {
            down_enabled: true,
            ..Default::default()
        };
        assert_eq!(layout.hit_test(1, 1, last_only), Some(StatureButton::Down));
        assert_eq!(layout.hit_test_geometric(10, 1), None);
        assert_eq!(layout.hit_test_geometric(1, 10), None);
    }
}
