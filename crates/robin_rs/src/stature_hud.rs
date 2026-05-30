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

use crate::gfx_types::Rect as SdlRect;

use robin_engine::engine::{PANNEL_HEIGHT, Stature};
use robin_engine::resource_ids::{RHID_DOWN_ARROW, RHID_UP_ARROW};

use crate::ingame_menu::layout::{
    BTN_STATE_DISABLED, BTN_STATE_HOVER, BTN_STATE_NORMAL, BTN_STATE_PRESSED, button_sprite_state,
};
use crate::native_font::NativeFont;
use crate::player_command::PlayerCommand;
use crate::renderer::{BLIT_SOURCE_TRANSPARENT, Renderer};
use crate::resource_manager::ResourceManager;

/// Which stature arrow widget was hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    fn index(self) -> usize {
        match self {
            StatureButton::Up => 0,
            StatureButton::Down => 1,
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
    pub up: SdlRect,
    pub down: SdlRect,
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
            .up_size()
            .unwrap_or((FALLBACK_W as u16, FALLBACK_H as u16));
        let (down_w, down_h) = sprites
            .down_size()
            .unwrap_or((FALLBACK_W as u16, FALLBACK_H as u16));

        // Up arrow at (1, -27) from the panel origin, down arrow at (0, 33).
        Self {
            up: SdlRect::new(1, frame_origin_y - 27, up_w as u32, up_h as u32),
            down: SdlRect::new(0, frame_origin_y + 33, down_w as u32, down_h as u32),
        }
    }

    pub fn hit_test(&self, x: i32, y: i32, enable: StatureEnable) -> Option<StatureButton> {
        let pt = crate::gfx_types::Point::new(x, y);
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
        let pt = crate::gfx_types::Point::new(x, y);
        if self.up.contains_point(pt) {
            return Some(StatureButton::Up);
        }
        if self.down.contains_point(pt) {
            return Some(StatureButton::Down);
        }
        None
    }
}

type SpriteFrame = (u32, u16, u16);

#[derive(Debug, Default)]
pub struct StatureSprites {
    pub up: [Option<SpriteFrame>; 4],
    pub down: [Option<SpriteFrame>; 4],
}

impl StatureSprites {
    /// Walk the four button-state sub-ids for each arrow resource.
    /// Same loader pattern as `CornerButtonSprites::load`.
    pub fn load(res: &mut ResourceManager, renderer: &mut Renderer) -> Self {
        fn fetch(
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
        fn all(
            res: &mut ResourceManager,
            renderer: &mut Renderer,
            id: i32,
        ) -> [Option<SpriteFrame>; 4] {
            [
                fetch(res, renderer, id, 0),
                fetch(res, renderer, id, 1),
                fetch(res, renderer, id, 2),
                fetch(res, renderer, id, 3),
            ]
        }
        Self {
            up: all(res, renderer, RHID_UP_ARROW),
            down: all(res, renderer, RHID_DOWN_ARROW),
        }
    }

    fn frames(&self, btn: StatureButton) -> &[Option<SpriteFrame>; 4] {
        match btn {
            StatureButton::Up => &self.up,
            StatureButton::Down => &self.down,
        }
    }

    fn frame(&self, btn: StatureButton, state: usize) -> Option<SpriteFrame> {
        let f = self.frames(btn);
        f[state].or(f[BTN_STATE_NORMAL])
    }

    pub fn up_size(&self) -> Option<(u16, u16)> {
        Self::size_of(&self.up)
    }

    pub fn down_size(&self) -> Option<(u16, u16)> {
        Self::size_of(&self.down)
    }

    fn size_of(frames: &[Option<SpriteFrame>; 4]) -> Option<(u16, u16)> {
        frames[BTN_STATE_NORMAL]
            .or(frames[BTN_STATE_HOVER])
            .or(frames[BTN_STATE_PRESSED])
            .or(frames[BTN_STATE_DISABLED])
            .map(|(_, w, h)| (w, h))
    }
}

/// Per-frame hover state passed to the draw routine.
#[derive(Debug, Clone, Copy, Default)]
pub struct StatureHoverState {
    pub hovered: Option<StatureButton>,
    pub mouse_pressed: bool,
}

/// Draw the two stature arrows.  Disabled arrows are not rendered.
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

        let Some((sid, _, _)) = sprites.frame(btn, state) else {
            continue;
        };
        let dst = robin_engine::sprite::BBox::new(
            crate::geo2d::GeoPoint2D {
                x: rect.x() as f32,
                y: rect.y() as f32,
            },
            crate::geo2d::GeoPoint2D {
                x: (rect.x() + rect.width() as i32) as f32,
                y: (rect.y() + rect.height() as i32) as f32,
            },
        );
        // C++ stature arrows are SBWidgetToggleButton<SBUIRendererBitmap>,
        // so they use the regular transparent bitmap path.
        renderer.blit_to_screen(sid, None, Some(&dst), BLIT_SOURCE_TRANSPARENT);
    }
}

/// Hover tracker for the stature arrow tooltips.  Mirrors
/// `ZoomTooltipTracker` exactly — shared HUD hover delay.
#[derive(Default, Clone)]
pub struct StatureTooltipTracker {
    inner: crate::ui_panel::RequirementsTooltipTracker,
}

impl StatureTooltipTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, hovered: Option<StatureButton>) {
        self.inner.update(hovered.map(StatureButton::index));
    }

    pub fn ready_button(&self) -> Option<StatureButton> {
        self.inner.ready_slot().and_then(|i| match i {
            0 => Some(StatureButton::Up),
            1 => Some(StatureButton::Down),
            _ => None,
        })
    }
}

/// Menu-text id for the tooltip attached to the given stature button.
pub fn stature_button_tooltip_mt_id(btn: StatureButton) -> usize {
    use crate::ingame_menu::resources::{MT_INFOBULLE_CROUCH, MT_INFOBULLE_STANDUP};
    match btn {
        StatureButton::Up => MT_INFOBULLE_STANDUP,
        StatureButton::Down => MT_INFOBULLE_CROUCH,
    }
}

/// Draw the hover tooltip for the stature HUD buttons.  Does nothing
/// when no tooltip is ready or the font stack isn't available.
#[allow(clippy::too_many_arguments)]
pub fn draw_tooltip(
    renderer: &mut Renderer,
    tracker: &StatureTooltipTracker,
    tooltip_text: impl Fn(StatureButton) -> String,
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
