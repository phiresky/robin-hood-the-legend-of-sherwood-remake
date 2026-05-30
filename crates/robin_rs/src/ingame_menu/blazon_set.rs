//! Blazon-set widget rendering + hover tooltip resolution.
//!
//! Pairs with [`robin_engine::widget_state::blazon_set`], which does the
//! data-side layout / classification.  This module blits the per-slot
//! sprites through the in-game menu rendering pipeline and hit-tests
//! the cursor against them for tooltip resolution.
//!
//! Used by the pre-mission modal's left-side blazon grid and — via its
//! own tiny path — by the campaign-map hover tooltip.

use robin_engine::widget_state::blazon_set::{
    BlazonSetState, BlazonSlotKind, build_blazon_set_state,
};

use crate::renderer::Renderer;
use robin_engine::campaign as engine_campaign;
use robin_engine::profiles as engine_profiles;
use robin_engine::sprite as engine_sprite;

use super::layout::MenuTransform;
use super::resources::{
    IngameMenuResources, MT_INFOBULLE_BLAZON_TO_WIN, MT_INFOBULLE_BLAZON_TO_WIN_IN_ATTACK,
    MT_INFOBULLE_BLAZON_WON, MenuSurface,
};

/// Build a [`BlazonSetState`] from engine state for the mission
/// described by `mission_idx`.
///
/// `blinking` is the trailing-castle blink count; pass 0 when the
/// one-shot blink latch is inactive.
#[allow(clippy::too_many_arguments)]
pub fn build_for_mission(
    campaign: &engine_campaign::Campaign,
    profiles: &engine_profiles::ProfileManager,
    mission_idx: usize,
    box_x: i32,
    box_y: i32,
    box_w: u32,
    box_h: u32,
    blinking: u32,
) -> BlazonSetState {
    let profile = campaign.missions[mission_idx].profile(profiles);
    let total = profile.number_of_blazons_to_win as u32;
    let to_be_collected = profile.number_of_blazons_to_be_collected as u32;
    let owned = campaign
        .get_value(engine_campaign::CampaignValue::Blazon)
        .max(0) as u32;
    build_blazon_set_state(
        box_x,
        box_y,
        box_w,
        box_h,
        total,
        owned,
        to_be_collected,
        blinking,
    )
}

/// Look up the sprite pack for the current [`BlazonSetState`] in
/// [`IngameMenuResources`].
fn sprites_for(
    state: &BlazonSetState,
    resources: &IngameMenuResources,
) -> [Option<MenuSurface>; 3] {
    if state.huge {
        resources.blazon_huge
    } else {
        resources.blazon_tiny
    }
}

/// Render a blazon-set state through the menu pipeline.
///
/// Draws each slot as the matching sub-picture from `RHID_BLAZON_HUGE`
/// / `RHID_BLAZON_TINY`; slots whose sprite is missing render as a
/// coloured placeholder so the grid remains visible on incomplete data
/// dirs (same fallback path as the campaign-map tooltip — see
/// [`crate::campaign_map`]).
///
/// `virt_origin` is the top-left of the virtual 640×480 menu rectangle
/// that the slot coordinates were computed against; pass the same
/// `(win_x, win_y)` used for the modal background.  Slot coordinates
/// are already expressed relative to the full menu frame (not the
/// modal window), so `virt_origin` is `(0, 0)` for screens that lay
/// out directly in menu space.
pub fn render(
    renderer: &mut Renderer,
    transform: MenuTransform,
    resources: &IngameMenuResources,
    state: &BlazonSetState,
    virt_origin_x: i32,
    virt_origin_y: i32,
) {
    if state.slots.is_empty() {
        return;
    }
    let sprites = sprites_for(state, resources);
    for slot in &state.slots {
        let vx = virt_origin_x + slot.x;
        let vy = virt_origin_y + slot.y;
        match sprites[slot.kind.sprite_sub()] {
            Some(surface) => {
                super::widget_bridge::draw_picture_surface_rect(
                    renderer,
                    transform,
                    surface.id,
                    vx,
                    vy,
                    state.slot_w as i32,
                    state.slot_h as i32,
                    0,
                    0,
                    surface.width,
                    surface.height,
                    true,
                );
            }
            None => {
                // Placeholder — keeps the grid visible when DEFAULT.RES
                // is incomplete.  Colours match the fallback in
                // `crate::campaign_map::draw_tooltip_blazon_strip`.
                let color = match slot.kind {
                    BlazonSlotKind::Normal => Renderer::create_color_16(210, 180, 60),
                    BlazonSlotKind::Castle => Renderer::create_color_16(140, 90, 50),
                    BlazonSlotKind::Empty => Renderer::create_color_16(90, 70, 40),
                };
                let (sx, sy) = transform.to_screen(vx, vy);
                let dst = engine_sprite::BBox::from_coords(
                    sx as f32,
                    sy as f32,
                    (sx + state.slot_w as i32) as f32,
                    (sy + state.slot_h as i32) as f32,
                );
                renderer.fill_screen(Some(&dst), color);
            }
        }
    }
}

/// Hit-test the mouse against the blazon set in virtual menu space.
/// Returns the slot index under the cursor, or `None`.
///
/// The slots are mouse-only targets — they don't participate in
/// tab-focus, just tooltip display.
pub fn hit_test(
    state: &BlazonSetState,
    virt_origin_x: i32,
    virt_origin_y: i32,
    mouse_virt_x: i32,
    mouse_virt_y: i32,
) -> Option<usize> {
    for (i, slot) in state.slots.iter().enumerate() {
        let vx = virt_origin_x + slot.x;
        let vy = virt_origin_y + slot.y;
        if mouse_virt_x >= vx
            && mouse_virt_x < vx + state.slot_w as i32
            && mouse_virt_y >= vy
            && mouse_virt_y < vy + state.slot_h as i32
        {
            return Some(i);
        }
    }
    None
}

/// Menu-text ID for the per-slot tooltip.
pub fn tooltip_mt_id(kind: BlazonSlotKind) -> usize {
    match kind {
        BlazonSlotKind::Normal => MT_INFOBULLE_BLAZON_WON,
        BlazonSlotKind::Empty => MT_INFOBULLE_BLAZON_TO_WIN,
        BlazonSlotKind::Castle => MT_INFOBULLE_BLAZON_TO_WIN_IN_ATTACK,
    }
}

/// Wall-clock hover tracker for the blazon set, modelled on
/// [`super::layout::TooltipState`].  The main modal's `TooltipState`
/// only tracks `FrameWnd` widgets, but the blazon set uses
/// immediate-mode slots that aren't registered as widgets, so it needs
/// its own tracker to reproduce the standard hover-delay tooltip.
#[derive(Default)]
pub struct BlazonTooltipTracker {
    hover_slot: Option<(usize, BlazonSlotKind)>,
    hover_since: Option<web_time::Instant>,
}

impl BlazonTooltipTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Re-scan the state for the slot under the cursor.  Resets the
    /// idle timer on target change.
    pub fn update(
        &mut self,
        state: &BlazonSetState,
        virt_origin_x: i32,
        virt_origin_y: i32,
        mouse_virt_x: i32,
        mouse_virt_y: i32,
    ) {
        let hit = hit_test(
            state,
            virt_origin_x,
            virt_origin_y,
            mouse_virt_x,
            mouse_virt_y,
        );
        let now = hit.map(|i| (i, state.slots[i].kind));
        if now != self.hover_slot {
            self.hover_slot = now;
            self.hover_since = now.map(|_| web_time::Instant::now());
        }
    }

    /// Paint the tooltip once the hover has been idle past
    /// [`super::layout::TOOLTIP_HOVER_DELAY`].
    pub fn draw(
        &self,
        renderer: &mut Renderer,
        font: &crate::native_font::NativeFont,
        transform: MenuTransform,
        resources: &IngameMenuResources,
        mouse_virt_x: i32,
        mouse_virt_y: i32,
    ) {
        let Some((_, kind)) = self.hover_slot else {
            return;
        };
        let Some(started) = self.hover_since else {
            return;
        };
        if started.elapsed() < super::layout::TOOLTIP_HOVER_DELAY {
            return;
        }
        let text = resources.menu_text.get(tooltip_mt_id(kind));
        super::layout::draw_tooltip(renderer, font, transform, &text, mouse_virt_x, mouse_virt_y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use robin_engine::widget_state::blazon_set::{
        BlazonSlotState, HUGE_BLAZON_HEIGHT, HUGE_BLAZON_WIDTH,
    };

    #[test]
    fn hit_test_finds_slot_under_cursor() {
        let state = BlazonSetState {
            huge: true,
            slot_w: HUGE_BLAZON_WIDTH,
            slot_h: HUGE_BLAZON_HEIGHT,
            slots: vec![
                BlazonSlotState {
                    kind: BlazonSlotKind::Normal,
                    x: 10,
                    y: 20,
                },
                BlazonSlotState {
                    kind: BlazonSlotKind::Castle,
                    x: 50,
                    y: 20,
                },
            ],
        };
        assert_eq!(hit_test(&state, 0, 0, 15, 25), Some(0));
        assert_eq!(hit_test(&state, 0, 0, 55, 25), Some(1));
        // Gap between slot 0 (ends at x=42) and slot 1 (starts at x=50).
        assert_eq!(hit_test(&state, 0, 0, 45, 25), None);
        // Below the slot strip (slot 0 y=20..62).
        assert_eq!(hit_test(&state, 0, 0, 15, 70), None);
        // With a non-zero origin, coordinates shift.
        assert_eq!(hit_test(&state, 100, 0, 115, 25), Some(0));
    }

    #[test]
    fn tooltip_mt_id_per_slot_kind() {
        assert_eq!(
            tooltip_mt_id(BlazonSlotKind::Normal),
            MT_INFOBULLE_BLAZON_WON
        );
        assert_eq!(
            tooltip_mt_id(BlazonSlotKind::Empty),
            MT_INFOBULLE_BLAZON_TO_WIN
        );
        assert_eq!(
            tooltip_mt_id(BlazonSlotKind::Castle),
            MT_INFOBULLE_BLAZON_TO_WIN_IN_ATTACK
        );
    }
}
