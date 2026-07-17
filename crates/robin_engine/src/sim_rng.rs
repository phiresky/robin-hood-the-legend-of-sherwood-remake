// This module is the only sanctioned caller of `fastrand::*` / `rand::*`.
#![allow(clippy::disallowed_methods)]

//! Deterministic simulation RNG.
//!
//! Rollback multiplayer requires that all gameplay-affecting randomness is
//! reproducible: given the same tick history, every client must compute the
//! same result. This module owns the *only* RNG the simulation layer is
//! allowed to use.
//!
//! ## Design
//!
//! The authoritative state is [`EngineInner::rng`]. At the start of every tick the
//! engine installs that RNG into a `thread_local` via [`install`], runs the
//! tick logic (which calls free functions like [`u32`] / [`usize`] / etc.),
//! and then takes it back via [`uninstall`] so the updated state persists in
//! the engine's owned field and participates in snapshots/clone.
//!
//! Using a thread-local rather than threading `&mut fastrand::Rng` through
//! every helper keeps call sites terse and avoids churning dozens of
//! signatures — rollback determinism only requires that *every* call funnels
//! through this module, not that the RNG is passed by reference.
//!
//! **Rules:**
//! - Gameplay code must call `sim_rng::{u32, usize, u8, bool, choose, …}` —
//!   never `rand::*` or `fastrand::*` globals directly.
//! - Non-simulation code (UI flavour, audio jitter, menus, loading screens)
//!   may still use ambient RNG; those must not feed back into simulation
//!   state. See `sound.rs` / `ingame_menu/*` for examples.
//! - Code that runs *outside* a tick (e.g. tests, tools) can call
//!   [`with_seed`] to get a temporary scope.

use std::cell::RefCell;
use std::ops::RangeBounds;

thread_local! {
    /// The installed simulation RNG for the current tick, if any.
    static SIM_RNG: RefCell<Option<fastrand::Rng>> = const { RefCell::new(None) };
}

/// Install `rng` as the active simulation RNG for this thread. Panics if an
/// RNG is already installed (nested tick execution is not supported).
pub fn install(rng: fastrand::Rng) {
    SIM_RNG.with(|cell| {
        let mut slot = cell.borrow_mut();
        assert!(
            slot.is_none(),
            "sim_rng::install called while an RNG is already installed"
        );
        *slot = Some(rng);
    });
}

/// Take the active simulation RNG back out. Panics if none was installed.
pub fn uninstall() -> fastrand::Rng {
    SIM_RNG.with(|cell| {
        cell.borrow_mut()
            .take()
            .expect("sim_rng::uninstall called without an installed RNG")
    })
}

/// Run `f` with a freshly seeded RNG installed. Used by tests and tools that
/// want determinism without going through `EngineInner::perform_hourglass`.
pub fn with_seed<R>(seed: u64, f: impl FnOnce() -> R) -> R {
    install(fastrand::Rng::with_seed(seed));
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = SIM_RNG.with(|cell| cell.borrow_mut().take());
        }
    }
    let _g = Guard;
    f()
}

fn with_rng<R>(f: impl FnOnce(&mut fastrand::Rng) -> R) -> R {
    SIM_RNG.with(|cell| {
        let mut slot = cell.borrow_mut();
        let rng = slot
            .as_mut()
            .expect("sim_rng used outside of an installed scope");
        f(rng)
    })
}

// ─── Range helpers (mirror fastrand's API) ───────────────────────────

pub fn u32(range: impl RangeBounds<u32>) -> u32 {
    with_rng(|rng| rng.u32(range))
}

pub fn i32(range: impl RangeBounds<i32>) -> i32 {
    with_rng(|rng| rng.i32(range))
}

pub fn u16(range: impl RangeBounds<u16>) -> u16 {
    with_rng(|rng| rng.u16(range))
}

pub fn u8(range: impl RangeBounds<u8>) -> u8 {
    with_rng(|rng| rng.u8(range))
}

pub fn i16(range: impl RangeBounds<i16>) -> i16 {
    with_rng(|rng| rng.i16(range))
}

pub fn usize(range: impl RangeBounds<usize>) -> usize {
    with_rng(|rng| rng.usize(range))
}

pub fn bool() -> bool {
    with_rng(|rng| rng.bool())
}

pub fn f32() -> f32 {
    with_rng(|rng| rng.f32())
}

/// Shuffle a slice in-place using the simulation RNG.
pub fn shuffle<T>(slice: &mut [T]) {
    with_rng(|rng| rng.shuffle(slice));
}

/// `serde` adapters for `fastrand::Rng`.
///
/// Use with `#[serde(with = "crate::sim_rng::serde_rng")]` on any
/// `fastrand::Rng` field. The RNG is serialized as a single `u64` via
/// [`fastrand::Rng::get_seed`] / [`fastrand::Rng::with_seed`], which
/// preserves the full internal state (fastrand's PRNG state IS the seed).
///
/// Used by `EngineInner::rng`, so save files, rollback snapshots, network
/// state-sync, and desync dumps preserve the exact next simulation roll.
pub mod serde_rng {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(rng: &fastrand::Rng, ser: S) -> Result<S::Ok, S::Error> {
        rng.get_seed().serialize(ser)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<fastrand::Rng, D::Error> {
        #[allow(clippy::disallowed_methods)]
        u64::deserialize(de).map(fastrand::Rng::with_seed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determinism() {
        let a = with_seed(42, || (0..10).map(|_| u32(..)).collect::<Vec<_>>());
        let b = with_seed(42, || (0..10).map(|_| u32(..)).collect::<Vec<_>>());
        assert_eq!(a, b);
    }

    #[test]
    fn serde_rng_roundtrip_preserves_state() {
        // Advance to a non-trivial state, serialize, deserialize, pull the
        // same u32 — must match.
        install(fastrand::Rng::with_seed(0xABCD_EF01));
        let _ = u32(..);
        let _ = u32(..);
        let rng = uninstall();

        let seed = rng.get_seed();
        let mut restored = fastrand::Rng::with_seed(seed);
        let mut original = rng;

        assert_eq!(original.u32(..), restored.u32(..));
        assert_eq!(original.u32(..), restored.u32(..));
    }

    #[test]
    fn install_uninstall_roundtrip() {
        install(fastrand::Rng::with_seed(7));
        let _ = u32(..);
        let rng = uninstall();
        // Install again and verify the returned RNG continues state forward.
        install(rng);
        let x1 = u32(..);
        let _advanced = uninstall();
        install(fastrand::Rng::with_seed(7));
        let _ = u32(..);
        let x2 = u32(..);
        assert_eq!(x1, x2);
        let _ = uninstall();
    }
}
