use super::*;

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

pub(super) const BLAZON_BAR_TINY_W: u16 = 9;
pub(super) const BLAZON_BAR_TINY_H: u16 = 14;
pub(super) const BLAZON_BAR_SPACING: u16 = 5;
pub(super) const BLAZON_BAR_Y: u16 = 2;

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
pub(super) fn blazon_bar_start_x(screen_width: u16, slot_count: u16) -> u16 {
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
