use super::*;

//
// Layout constants:
//   ICON_WIDTH              40
//   ICON_HEIGHT             50
//   ICON_MARGIN             10
//   DIFFERENCE_X_YES_NO     25
//   DIFFERENCE_Y_YES_NO     30
//   DIFFERENCE_X_SELECTED   -1
//   DIFFERENCE_Y_SELECTED    2
pub(super) const REQ_BAR_ICON_W: u16 = 40;
pub(super) const REQ_BAR_ICON_H: u16 = 50;
pub(super) const REQ_BAR_ICON_MARGIN: u16 = 10;
pub(super) const REQ_BAR_Y: u16 = 2;
// The box is `(40, 2, screen_w - 40, 2)`.
pub(super) const REQ_BAR_BOX_INSET: u16 = 40;
pub(super) const REQ_BAR_DIFFERENCE_X_YES_NO: i32 = 25;
pub(super) const REQ_BAR_DIFFERENCE_Y_YES_NO: i32 = 30;
pub(super) const REQ_BAR_DIFFERENCE_X_SELECTED: i32 = -1;
pub(super) const REQ_BAR_DIFFERENCE_Y_SELECTED: i32 = 2;

/// Starting-X for the centered requirements strip.  The box is
/// `(40, 2, screen_w - 40, 2)` so box_width = `screen_w - 80`; the strip
/// is centered by offsetting the box's top-left by `(box_w - needed_w) / 2`.
/// `needed_w = n * (ICON_W + MARGIN) - MARGIN`.
/// Returns `None` when the strip is wider than the box (no slots fit).
pub(super) fn requirements_bar_start_x(screen_width: u16, slot_count: usize) -> Option<i32> {
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
    profiles: &engine_profiles::ProfileManager,
    state: &crate::widget::requirements::RequirementsState,
) {
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
                let Some(sub_id) = profiles
                    .get_character(*character_profile_idx)
                    .and_then(|p| CharacterKind::from_profile_name(&p.profile_name))
                    .map(|k| k.required_pc_sub_id())
                else {
                    tracing::warn!(
                        character_profile_idx,
                        "required character has no portrait mapping"
                    );
                    continue;
                };
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
                let (w, h) = portrait_surface_dimensions(renderer, sid);
                let (w, h) = (w as i32, h as i32);
                let bx = icon_x + REQ_BAR_DIFFERENCE_X_YES_NO;
                let by = icon_y + REQ_BAR_DIFFERENCE_Y_YES_NO;
                let badge = bbox_i32(bx, by, bx + w, by + h);
                blit_to_screen_widget(renderer, sid, None, Some(&badge), BLIT_SOURCE_TRANSPARENT);
            }
        }
        // Selected-ring overlay at (-1, +2) from the icon origin.
        if selected && let Some(sid) = portraits.req_selected {
            let (w, h) = portrait_surface_dimensions(renderer, sid);
            let (w, h) = (w as i32, h as i32);
            let rx = icon_x + REQ_BAR_DIFFERENCE_X_SELECTED;
            let ry = icon_y + REQ_BAR_DIFFERENCE_Y_SELECTED;
            let ring = bbox_i32(rx, ry, rx + w, ry + h);
            blit_to_screen_widget(renderer, sid, None, Some(&ring), BLIT_SOURCE_TRANSPARENT);
        }
    }
}
