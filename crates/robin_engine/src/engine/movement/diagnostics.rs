//! Read-only movement boundary diagnostic selection. No simulation capability
//! is accepted here: filters cannot draw RNG or mutate the observed owner.

use crate::element::EntityId;

#[inline]
pub(super) fn debug_post_seek_handoff_enabled() -> bool {
    super::super::diagnostics::config().post_seek_handoff
}

pub(super) fn movement_pop_goal_owner_debug_matches(frame: u32, owner: EntityId) -> bool {
    super::super::diagnostics::config().goal_owner_matches(frame, owner)
}
