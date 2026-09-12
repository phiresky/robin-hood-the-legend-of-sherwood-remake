use super::*;

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
pub fn action_button_tooltip_mt_id(action: robin_engine::profiles::Action) -> Option<usize> {
    use crate::ingame_menu::resources::*;
    use robin_engine::profiles::Action;
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

/// Optional post-port detail appended to the localized Original tooltip.
/// The stable key is exposed with the English fallback so a future extension
/// catalog can translate these strings independently of `MenuText` ids.
pub fn item_action_tooltip_extension(
    action: robin_engine::profiles::Action,
    rules: robin_engine::gameplay_config::ItemGameplayConfig,
    previews: robin_engine::gameplay_config::ItemPreviewConfig,
) -> Option<(&'static str, &'static str)> {
    use robin_engine::profiles::Action;
    match action {
        Action::Apple if previews.apple_effect => Some(if rules.apple_combat_interrupt {
            (
                "item_tooltip.apple.interrupt",
                "60-frame daze; 1500-frame scent; interrupts active combat.",
            )
        } else {
            (
                "item_tooltip.apple.classic",
                "60-frame daze; 1500-frame scent; fighting targets are immune.",
            )
        }),
        Action::Stone if previews.stone_direct_effect || previews.stone_distraction_area => Some(
            match (
                previews.stone_direct_effect,
                previews.stone_distraction_area,
                rules.stone_ground_distraction,
                rules.stone_longer_range,
            ) {
                (true, true, true, true) => (
                    "item_tooltip.stone.direct_and_distraction",
                    "Direct hit: 10 damage + strong concussion; long base range 300. Ground noise radius: 240.",
                ),
                (true, true, true, false) => (
                    "item_tooltip.stone.direct_and_distraction_classic_range",
                    "Direct hit: 10 damage + strong concussion; classic base range 200. Ground noise radius: 240.",
                ),
                (true, _, _, true) => (
                    "item_tooltip.stone.direct",
                    "Direct hit: 10 damage + strong concussion; long base range 300.",
                ),
                (true, _, _, false) => (
                    "item_tooltip.stone.direct_classic_range",
                    "Direct hit: 10 damage + strong concussion; classic base range 200.",
                ),
                (false, true, true, _) => (
                    "item_tooltip.stone.distraction",
                    "Ground noise attracts eligible hostiles within 240.",
                ),
                (false, true, false, _) => (
                    "item_tooltip.stone.distraction_disabled",
                    "Ground distraction is disabled in Gameplay settings.",
                ),
                (false, false, _, _) => unreachable!("tooltip guard checked above"),
            },
        ),
        Action::Net if previews.net_capture_area || previews.net_crumple_prediction => Some(
            match (
                previews.net_capture_area,
                previews.net_crumple_prediction,
                rules.net_selective_immunity,
            ) {
                (true, true, true) => (
                    "item_tooltip.net.capture_and_terrain_crumple",
                    "Captures active people within 40, including allies; VIPs/riders/Net user are skipped; terrain can crumple it.",
                ),
                (true, true, false) => (
                    "item_tooltip.net.capture_and_crumple",
                    "Captures active people within 40, including allies; terrain or people can crumple it.",
                ),
                (true, false, true) => (
                    "item_tooltip.net.selective_capture",
                    "Captures active people within 40, including allies; VIPs, riders, and the Net user are skipped.",
                ),
                (true, false, false) => (
                    "item_tooltip.net.capture",
                    "Captures active people within 40, including allies.",
                ),
                (false, true, true) => (
                    "item_tooltip.net.terrain_crumple",
                    "Terrain can crumple the net; resistant people are skipped.",
                ),
                (false, true, false) => (
                    "item_tooltip.net.crumple",
                    "Terrain and some victim conditions can crumple the net.",
                ),
                (false, false, _) => unreachable!("tooltip guard checked above"),
            },
        ),
        Action::Ale if previews.ale_effect => Some(if rules.ale_reliable_distraction {
            (
                "item_tooltip.ale.reliable",
                "Zero-interest outdoor non-VIP soldiers also accept at potency 20; authored interest and drunk behavior stay unchanged.",
            )
        } else {
            (
                "item_tooltip.ale.classic",
                "Visible outdoor enemies need authored beer interest; drunk enemies accept.",
            )
        }),
        Action::Purse if previews.purse_effect => Some((
            "item_tooltip.purse.effect",
            "Scatters 5 coins worth £50; visible outdoor enemies need money interest.",
        )),
        Action::WaspNest if previews.wasp_area => Some(if rules.wasp_reliable_acquisition {
            (
                "item_tooltip.wasp.reliable",
                "Acquires within 75 (225 if apple-scented); ignores VIPs and active swordfights.",
            )
        } else {
            (
                "item_tooltip.wasp.classic",
                "Acquires within 50 (150 if apple-scented); ignores VIPs and active swordfights.",
            )
        }),
        _ => None,
    }
}

/// Hover-idle tracker for portrait action buttons. These compact controls
/// need much quicker feedback than the large requirements-bar widgets.
#[derive(Default, Clone)]
pub struct PcActionTooltipTracker {
    hovered: Option<(u8, u8)>,
    hover_ticks: u32,
}

pub const PC_ACTION_TOOLTIP_DELAY_TICKS: u32 = 12;

impl PcActionTooltipTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Call once per frame with the hovered `(slot, btn)` pair, or
    /// `None` when the cursor is not over any PC action button.
    pub fn update(&mut self, hovered: Option<(u8, u8)>) {
        if hovered == self.hovered {
            if hovered.is_some() {
                self.hover_ticks = self.hover_ticks.saturating_add(1);
            }
        } else {
            self.hovered = hovered;
            self.hover_ticks = u32::from(hovered.is_some());
        }
    }

    /// `Some((slot, btn))` once the cursor has been idle on the same
    /// button long enough for the tooltip to appear.
    pub fn ready_button(&self) -> Option<(u8, u8)> {
        (self.hover_ticks >= PC_ACTION_TOOLTIP_DELAY_TICKS)
            .then_some(self.hovered)
            .flatten()
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
pub type RequirementsTooltipTracker = HoverTooltipTracker<usize>;

/// Fixed-tick hover delay retaining the caller's target type without slot conversions.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct HoverTooltipTracker<T> {
    hovered_slot: Option<T>,
    /// Ticks accumulated with the cursor on `hovered_slot`.  Saturates
    /// at `u32::MAX` so a very long idle hover can't wrap back below
    /// the threshold.
    pub(super) hover_ticks: u32,
}

impl<T> Default for HoverTooltipTracker<T> {
    fn default() -> Self {
        Self {
            hovered_slot: None,
            hover_ticks: 0,
        }
    }
}

impl<T: Copy + PartialEq> HoverTooltipTracker<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Call once per frame with the slot currently under the cursor.
    /// Resets the tick counter when the target slot changes.
    pub fn update(&mut self, hovered: Option<T>) {
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
    pub fn ready_slot(&self) -> Option<T> {
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
/// drawn without an explicit shadow. `cursor_size` is the current cursor
/// sprite's on-screen size (width, height).
pub fn draw_screen_tooltip(
    renderer: &mut Renderer,
    font: &crate::native_font::Font,
    shadow: Option<&crate::native_font::Font>,
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
        let wrap = layout::wrap_text_for_box_font(font, text, box_w, 3);
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
                layout::render_text_screen_font(renderer, sh_font, &line.text, box_x + 1, ly + 1);
            }
            layout::render_text_screen_font(renderer, font, &line.text, box_x, ly);
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
        layout::render_text_screen_font(renderer, sh_font, text, x + 1, y + 1);
    }
    layout::render_text_screen_font(renderer, font, text, x, y);
}
