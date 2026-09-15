//! Read-only movement boundary diagnostic selection. No simulation capability
//! is accepted here: filters cannot draw RNG or mutate the observed owner.

#[inline]
pub(super) fn debug_post_seek_handoff_enabled() -> bool {
    super::super::diagnostics::config().post_seek_handoff
}
