//! Per-target interactive helpers: focus / cursor / filter / command /
//! titbit ladders.
//!
//! Translates the target's action-filter bitmask (plus a few PC-ability
//! gates) into either a [`Command`], a cursor id, or a quick-action
//! hint frame. These helpers live here rather than as
//! methods on `TargetData` so they can combine cleanly with PC state
//! read from `EngineInner` / `LevelAssets` at the dispatching call
//! sites in `commands.rs` and `input.rs`.
//!
//! Priority ordering for the three ladders is kept in lockstep so a
//! focused target shows the cursor that matches the command the click
//! will fire.

use crate::element::{Focus, TargetFilter};
use crate::element_kinds::Command;
use crate::resource_ids::*;
use crate::titbit::QuickAction;

/// Return the filter bit a focus type requires on a target.
///
/// `Focus::Use` has its own overload below; every other focus type
/// that doesn't map to a specific filter bit returns an empty bitset
/// so the caller rejects the focus.
pub fn focus_to_target_filter(focus: Focus) -> TargetFilter {
    match focus {
        Focus::Bow => TargetFilter::ARROW,
        Focus::Apple => TargetFilter::APPLE,
        Focus::Stone => TargetFilter::STONE,
        Focus::Heal => TargetFilter::HEAL,
        Focus::Lever => TargetFilter::LEVER,
        _ => TargetFilter::empty(),
    }
}

/// Mask of filter bits a PC can match when `Focus::Use` asks for the
/// generic interactive filter.
///
/// CUT | HANDLE | TAKE are always on; SEARCH / LEVER / MONEY are added
/// conditionally on PC abilities.
pub fn action_filter_for_pc(
    pc_has_search: bool,
    pc_has_lever: bool,
    pc_is_vip: bool,
) -> TargetFilter {
    let mut f = TargetFilter::CUT | TargetFilter::HANDLE | TargetFilter::TAKE;
    if pc_has_search {
        f |= TargetFilter::SEARCH;
    }
    if pc_has_lever {
        f |= TargetFilter::LEVER;
    }
    if pc_is_vip {
        f |= TargetFilter::MONEY;
    }
    f
}

/// Pick the `Command` a `Focus::Use` click on this target should
/// dispatch.  `None` means the click should do nothing.
///
/// Priority ladder: SEARCH beats LEVER beats CUT beats HANDLE beats
/// TAKE beats MONEY.  SEARCH / LEVER / MONEY require the PC to carry
/// the matching contextual action or VIP flag on top of the target
/// filter bit.
pub fn target_use_command(
    action_filter: TargetFilter,
    pc_has_search: bool,
    pc_has_lever: bool,
    pc_is_vip: bool,
) -> Option<Command> {
    if action_filter.contains(TargetFilter::SEARCH) && pc_has_search {
        Some(Command::SearchCmd)
    } else if action_filter.contains(TargetFilter::LEVER) && pc_has_lever {
        Some(Command::UseLever)
    } else if action_filter.contains(TargetFilter::CUT) {
        Some(Command::HitTarget)
    } else if action_filter.contains(TargetFilter::HANDLE) {
        Some(Command::HandleTarget)
    } else if action_filter.contains(TargetFilter::TAKE) {
        Some(Command::TakeTarget)
    } else if action_filter.contains(TargetFilter::MONEY) && pc_is_vip {
        Some(Command::Pay)
    } else {
        None
    }
}

/// Pick the cursor for a `Focus::Use` hover on this
/// target.  `0` means "no target-specific cursor — let the caller
/// fall back to the default arrow".
///
/// Priority is identical to `target_use_command`; `LEVER` splits on
/// VIP (raises vs. kicks the lever), `MONEY` gates on VIP only.
pub fn target_mouse_cursor(
    action_filter: TargetFilter,
    pc_has_search: bool,
    pc_has_lever: bool,
    pc_is_vip: bool,
) -> i32 {
    if action_filter.contains(TargetFilter::SEARCH) && pc_has_search {
        RHMOUSE_SEARCH
    } else if action_filter.contains(TargetFilter::LEVER) && pc_has_lever {
        if pc_is_vip {
            RHMOUSE_LEVER_YES
        } else {
            RHMOUSE_LEVER_FOOT_YES
        }
    } else if action_filter.contains(TargetFilter::CUT) {
        RHMOUSE_TARGET_CUT_YES
    } else if action_filter.contains(TargetFilter::HANDLE) {
        RHMOUSE_TARGET_HANDLE_YES
    } else if action_filter.contains(TargetFilter::TAKE) {
        RHMOUSE_GET_YES
    } else if action_filter.contains(TargetFilter::MONEY) && pc_is_vip {
        RHMOUSE_PAY_YES
    } else {
        0
    }
}

/// Pick the quick-action hint frame for a macro-recorded interaction
/// with this target.  Returned as the ordinal of
/// [`QuickAction`] so it can feed straight into the titbit manager.
///
/// Priority here diverges slightly from the command / cursor ladder:
/// SEARCH → CUT → HANDLE → TAKE → MONEY → ARROW → LEVER → STONE →
/// APPLE → HEAL.  LEVER still splits on VIP (raise vs. kick).  Per
/// CLAUDE.md "no fake data" we log a warning on the fallback branch
/// and return `InteractNpc` so a mis-authored target is visible
/// rather than crashing.
pub fn target_qa_titbit(action_filter: TargetFilter, pc_has_search: bool, pc_is_vip: bool) -> u16 {
    let frame = if action_filter.contains(TargetFilter::SEARCH) && pc_has_search {
        QuickAction::Search
    } else if action_filter.contains(TargetFilter::CUT) {
        QuickAction::TargetHit
    } else if action_filter.contains(TargetFilter::HANDLE) {
        QuickAction::TargetHandled
    } else if action_filter.contains(TargetFilter::TAKE) {
        QuickAction::Take
    } else if action_filter.contains(TargetFilter::MONEY) && pc_is_vip {
        QuickAction::GiveMoney
    } else if action_filter.contains(TargetFilter::ARROW) {
        QuickAction::BowOk
    } else if action_filter.contains(TargetFilter::LEVER) {
        if pc_is_vip {
            QuickAction::Lever
        } else {
            QuickAction::TargetFoot
        }
    } else if action_filter.contains(TargetFilter::STONE) {
        QuickAction::Stone
    } else if action_filter.contains(TargetFilter::APPLE) {
        QuickAction::Apple
    } else if action_filter.contains(TargetFilter::HEAL) {
        QuickAction::Heal
    } else {
        tracing::warn!(
            ?action_filter,
            "target_qa_titbit: target has no actionable filter bits; \
             falling back to InteractNpc",
        );
        QuickAction::InteractNpc
    };
    frame as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_to_filter_covers_five_arms() {
        assert_eq!(focus_to_target_filter(Focus::Bow), TargetFilter::ARROW);
        assert_eq!(focus_to_target_filter(Focus::Apple), TargetFilter::APPLE);
        assert_eq!(focus_to_target_filter(Focus::Stone), TargetFilter::STONE);
        assert_eq!(focus_to_target_filter(Focus::Heal), TargetFilter::HEAL);
        assert_eq!(focus_to_target_filter(Focus::Lever), TargetFilter::LEVER);
        assert_eq!(focus_to_target_filter(Focus::Use), TargetFilter::empty());
        assert_eq!(focus_to_target_filter(Focus::Sword), TargetFilter::empty());
    }

    #[test]
    fn pc_filter_mask_builds_from_abilities() {
        assert_eq!(
            action_filter_for_pc(false, false, false),
            TargetFilter::CUT | TargetFilter::HANDLE | TargetFilter::TAKE,
        );
        assert!(action_filter_for_pc(true, false, false).contains(TargetFilter::SEARCH));
        assert!(action_filter_for_pc(false, true, false).contains(TargetFilter::LEVER));
        assert!(action_filter_for_pc(false, false, true).contains(TargetFilter::MONEY));
    }

    #[test]
    fn command_priority_ladder() {
        // SEARCH beats everything when the PC has the search action.
        let mixed = TargetFilter::SEARCH | TargetFilter::CUT | TargetFilter::HANDLE;
        assert_eq!(
            target_use_command(mixed, true, false, false),
            Some(Command::SearchCmd),
        );
        // Without the PC ability, SEARCH falls through to CUT.
        assert_eq!(
            target_use_command(mixed, false, false, false),
            Some(Command::HitTarget),
        );
        // LEVER (with ability) beats CUT.
        assert_eq!(
            target_use_command(TargetFilter::LEVER | TargetFilter::CUT, false, true, false),
            Some(Command::UseLever),
        );
        // MONEY requires VIP.
        assert_eq!(
            target_use_command(TargetFilter::MONEY, false, false, false),
            None,
        );
        assert_eq!(
            target_use_command(TargetFilter::MONEY, false, false, true),
            Some(Command::Pay),
        );
        // Empty filter → no command.
        assert_eq!(
            target_use_command(TargetFilter::empty(), true, true, true),
            None,
        );
    }

    #[test]
    fn cursor_lever_splits_on_vip() {
        let f = TargetFilter::LEVER;
        assert_eq!(
            target_mouse_cursor(f, false, true, false),
            RHMOUSE_LEVER_FOOT_YES,
        );
        assert_eq!(target_mouse_cursor(f, false, true, true), RHMOUSE_LEVER_YES,);
    }

    #[test]
    fn qa_titbit_priority_ladder() {
        // SEARCH wins over CUT when the PC has search.
        let mixed = TargetFilter::SEARCH | TargetFilter::CUT;
        assert_eq!(
            target_qa_titbit(mixed, true, false),
            QuickAction::Search as u16,
        );
        // Without search, CUT wins.
        assert_eq!(
            target_qa_titbit(mixed, false, false),
            QuickAction::TargetHit as u16,
        );
        // LEVER splits on VIP even when the PC lacks the lever action
        // — the QA titbit ladder doesn't gate LEVER on pc_has_lever
        // for the recorded icon.
        assert_eq!(
            target_qa_titbit(TargetFilter::LEVER, false, false),
            QuickAction::TargetFoot as u16,
        );
        assert_eq!(
            target_qa_titbit(TargetFilter::LEVER, false, true),
            QuickAction::Lever as u16,
        );
    }
}
