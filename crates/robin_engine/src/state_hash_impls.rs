//! Manual `StateHash` impls for types we don't own (geo crate, bitflags
//! macro internals). Centralised here so the call sites don't need to
//! know about the orphan-rule workaround.

use robin_util::state_hash::StateHash;
use std::hash::Hasher;

// ─── Bitflags types ───────────────────────────────────────────────
//
// The `bitflags!` macro generates a struct whose body uses an inner
// `_::InternalBitFlags` type — `#[derive(StateHash)]` doesn't work on
// it because the inner type isn't reachable to derive against. Instead
// we hash via `.bits()`, the public accessor that returns the
// underlying integer.

macro_rules! impl_state_hash_bitflags {
    ($($t:path),* $(,)?) => {
        $(
            impl StateHash for $t {
                #[inline]
                fn state_hash<H: Hasher>(&self, state: &mut H) {
                    self.bits().state_hash(state);
                }
            }
        )*
    };
}

impl_state_hash_bitflags! {
    crate::ai::AiLockFlags,
    crate::ai::GotoFlags,
    crate::ai::DutyFlags,
    crate::ai::AlertFlags,
    crate::ai::SpeechFlags,
    crate::ai::RemarkTargetFlags,
    crate::sector::SectorType,
    crate::element_kinds::ExitActionStateFlags,
    crate::element_kinds::EnterActionStateFlags,
    crate::element_kinds::ChangePostureFlags,
    crate::element_kinds::TargetFilter,
    crate::sequence::CascadeFlags,
    crate::sequence::MoveFlags,
    crate::position_interface::IncrementComputed,
    crate::ai_enemy::GetNearestFlags,
    crate::ai_enemy::SeekFlags,
    crate::ai_enemy::ReportUpdateFlags,
    crate::ai_enemy::PrimaryTargetFlags,
    crate::ai_enemy::ConditionFlags,
    crate::combat::SwordDamageResult,
}

// geo-crate impls live in `robin_util::state_hash` (which depends on
// `geo`) so the orphan rule is satisfied.

// fastrand::Rng impl lives in `robin_util::state_hash`.
