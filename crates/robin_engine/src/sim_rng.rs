// This module is the only sanctioned authoritative caller of
// `fastrand::*` / `rand::*`.
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
//! - Non-simulation code (audio jitter, menus, loading screens) may still use
//!   ambient RNG; those must not feed back into simulation state. See
//!   `sound.rs` / `ingame_menu/*` for examples.
//! - Authoritative auxiliary randomness that intentionally does not advance
//!   the serialized stream must use a reviewed [`AuxiliaryRngSite`].
//! - Code that runs *outside* a tick (e.g. tests, tools) can call
//!   [`with_seed`] to get a temporary scope.

use std::cell::RefCell;
use std::ops::RangeBounds;

/// Reviewed authoritative gameplay RNG entry points.
///
/// Every production draw must name one of these sites.  The source-audit
/// test at the bottom of this module compares their structural use against
/// `docs/RNG_AUDIT.md`, so adding or moving gameplay randomness requires an
/// explicit review rather than an unlabelled call to a generic RNG helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::EnumIter)]
pub enum RngSite {
    LuaMathRandom,
    SwordDamageProtection,
    SwordStrikeSelection,
    SequenceRecordingBuildingExitWait,
    ScriptRand,
    SpriteBoredStart,
    SpriteSnakeStart,
    LevelBonusInitialFrame,
    MobileWaypointProbability,
    SherwoodProductionBonusFrame,
    SherwoodRelicFrame,
    ScrollInitialFrame,
    ScrollRevealFrame,
    CivilianBeggarSpeechGate,
    CivilianBeggarSpeechChoice,
    CivilianFirstLookTimer,
    CivilianPanicDirection,
    BowAccuracy,
    ArrowFallingFrame,
    SoldierBrawlCooldown,
    SoldierNoiseCooldown,
    CampaignForcedMission,
    CampaignAccessChance,
    CampaignReinforcementPeasant,
    CampaignReservistReturn,
    CampaignNewPeasantType,
    BuildingExitGate,
    SeekPointDirectionPattern,
    NearSeekPoint,
    AiRandomValueRectangle,
    AiRandomValueGaussHigh,
    AiRandomValueGauss,
    MacroRand,
    CheckForLookDirection,
    AiPanic,
    SpecialActionRemark,
    DefaultPostLook,
    VipIdleRemark,
    BattleCourage,
    BattleProvoke,
    BattlePanicRemark,
    SeekPointSelection,
    SeekPointAcceptance,
    ArcherForestTarget,
    PhalanxAdvance,
    DrunkCombatFreeze,
    CombatReposition,
    CombatObserveSideStep,
    EnemyWonderingLook,
    EnemySeekDirectionShuffle,
    EnemySeekLook,
    TooProudLook,
    ShieldAdvance,
    CharlySorrow,
    OfficerSearchLook,
    SherwoodBeamMeShuffle,
    ArrowPiercingProtection,
    StonePiercingProtection,
    MeleeInitiative,
    SmalltalkStrikeSide,
    ReinforcementDoor,
    ReinforcementJitter,
    SherwoodReturningPcPlacement,
    MeleeProvoke,
    HeroSpeech,
    PrincipalOpponent,
    MeleeDegenerateDirection,
    MeleePrincipalReshuffle,
    MeleeNonMutualGate,
    MeleeStepBack,
    RuntimeBuildingExitWait,
    PeasantReservistSurvival,
    SoldierFreedRotation,
    DoorFightDispersion,
    DoorFightTarget,
    PurseCoinScatter,
    WaspDirectionTimer,
    WaspStingTimer,
    WaspMovement,
    WriggleDirection,
    BoredAnimationChoice,
    NetWriggleGate,
    DelayedSoundTimer,
    DrunkenPathDeviation,
    TitbitUpdate,
}

/// Reviewed deterministic randomness whose results enter authoritative state
/// without consuming the serialized Engine-owned draw stream.
///
/// These generators are separately seeded and ephemeral. Their resulting
/// state, rather than their temporary RNG, must be covered by snapshots and
/// state hashes. The source-inventory test guards every production use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::EnumIter)]
pub enum AuxiliaryRngSite {
    PeasantNames,
}

thread_local! {
    /// The installed simulation RNG for the current tick, if any.
    static SIM_RNG: RefCell<Option<fastrand::Rng>> = const { RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    static DRAW_TRACE: RefCell<Option<Vec<RngSite>>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_draw_trace<R>(f: impl FnOnce() -> R) -> (R, Vec<RngSite>) {
    DRAW_TRACE.with(|trace| {
        assert!(trace.borrow().is_none(), "nested RNG draw trace");
        *trace.borrow_mut() = Some(Vec::new());
    });
    let result = f();
    let trace = DRAW_TRACE.with(|trace| trace.borrow_mut().take().expect("draw trace missing"));
    (result, trace)
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

/// Run one reviewed authoritative auxiliary generator from a deterministic
/// seed without installing or advancing the serialized simulation stream.
pub fn with_auxiliary_seed<R>(
    site: AuxiliaryRngSite,
    seed: u64,
    f: impl FnOnce(&mut fastrand::Rng) -> R,
) -> R {
    let _ = site;
    f(&mut fastrand::Rng::with_seed(seed))
}

fn with_rng<R>(site: RngSite, f: impl FnOnce(&mut fastrand::Rng) -> R) -> R {
    #[cfg(test)]
    DRAW_TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.push(site);
        }
    });
    #[cfg(not(test))]
    let _ = site;
    SIM_RNG.with(|cell| {
        let mut slot = cell.borrow_mut();
        let rng = slot
            .as_mut()
            .expect("sim_rng used outside of an installed scope");
        f(rng)
    })
}

// ─── Range helpers (mirror fastrand's API) ───────────────────────────

pub fn u32(site: RngSite, range: impl RangeBounds<u32>) -> u32 {
    with_rng(site, |rng| rng.u32(range))
}

pub fn i32(site: RngSite, range: impl RangeBounds<i32>) -> i32 {
    with_rng(site, |rng| rng.i32(range))
}

pub fn u16(site: RngSite, range: impl RangeBounds<u16>) -> u16 {
    with_rng(site, |rng| rng.u16(range))
}

pub fn u8(site: RngSite, range: impl RangeBounds<u8>) -> u8 {
    with_rng(site, |rng| rng.u8(range))
}

pub fn i16(site: RngSite, range: impl RangeBounds<i16>) -> i16 {
    with_rng(site, |rng| rng.i16(range))
}

pub fn usize(site: RngSite, range: impl RangeBounds<usize>) -> usize {
    with_rng(site, |rng| rng.usize(range))
}

pub fn bool(site: RngSite) -> bool {
    with_rng(site, |rng| rng.bool())
}

pub fn f32(site: RngSite) -> f32 {
    with_rng(site, |rng| rng.f32())
}

/// Shuffle a slice in-place using the simulation RNG.
pub fn shuffle<T>(site: RngSite, slice: &mut [T]) {
    with_rng(site, |rng| rng.shuffle(slice));
}

/// Original's MSVC-era `rand() / RAND_MAX` fraction, including both 0 and 1.
///
/// The shipped code assumes `RAND_MAX == 32767` at the two authoritative
/// floating-point call sites.  We retain those range semantics without
/// attempting to reproduce the libc output sequence.
pub fn c_rand_unit_inclusive(site: RngSite) -> f32 {
    u16(site, 0..=32767) as f32 / 32767.0
}

/// Script `Rand(max)`: exactly one draw in `[0, max)` for a positive bound.
///
/// Original executes `rand() % iMaximum`; zero is a fatal divide-by-zero and
/// the documented contract requires a positive maximum.  Reject non-positive
/// bounds loudly instead of fabricating the value zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ScriptRandError {
    #[error("script Rand(max) requires max > 0, got {0}")]
    NonPositiveMaximum(i32),
}

pub fn script_rand(site: RngSite, max: i32) -> Result<i32, ScriptRandError> {
    if max <= 0 {
        return Err(ScriptRandError::NonPositiveMaximum(max));
    }
    Ok(i32(site, 0..max))
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
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};
    use strum::IntoEnumIterator;
    use syn::visit::Visit;

    const REVIEWED_SITE_USES: &[(&str, usize)] = &[
        ("AiPanic", 7),
        ("AiRandomValueGauss", 3),
        ("AiRandomValueGaussHigh", 3),
        ("AiRandomValueRectangle", 1),
        ("ArcherForestTarget", 1),
        ("ArrowFallingFrame", 1),
        ("ArrowPiercingProtection", 1),
        ("BattleCourage", 1),
        ("BattlePanicRemark", 3),
        ("BattleProvoke", 1),
        ("BoredAnimationChoice", 1),
        ("BowAccuracy", 4),
        ("BuildingExitGate", 1),
        ("CampaignAccessChance", 2),
        ("CampaignForcedMission", 1),
        ("CampaignNewPeasantType", 1),
        ("CampaignReinforcementPeasant", 1),
        ("CampaignReservistReturn", 2),
        ("CharlySorrow", 2),
        ("CheckForLookDirection", 1),
        ("CivilianBeggarSpeechChoice", 1),
        ("CivilianBeggarSpeechGate", 1),
        ("CivilianFirstLookTimer", 1),
        ("CivilianPanicDirection", 1),
        ("CombatObserveSideStep", 1),
        ("CombatReposition", 1),
        ("DefaultPostLook", 1),
        ("DelayedSoundTimer", 1),
        ("DoorFightDispersion", 2),
        ("DoorFightTarget", 1),
        ("DrunkCombatFreeze", 2),
        ("DrunkenPathDeviation", 2),
        ("EnemySeekDirectionShuffle", 1),
        ("EnemySeekLook", 2),
        ("EnemyWonderingLook", 5),
        ("HeroSpeech", 1),
        ("LevelBonusInitialFrame", 1),
        ("MobileWaypointProbability", 1),
        ("LuaMathRandom", 3),
        ("MacroRand", 2),
        ("MeleeDegenerateDirection", 1),
        ("MeleeInitiative", 1),
        ("MeleeNonMutualGate", 1),
        ("MeleePrincipalReshuffle", 1),
        ("MeleeProvoke", 1),
        ("MeleeStepBack", 1),
        ("NearSeekPoint", 1),
        ("NetWriggleGate", 1),
        ("OfficerSearchLook", 1),
        ("PeasantReservistSurvival", 1),
        ("PhalanxAdvance", 1),
        ("PrincipalOpponent", 1),
        ("PurseCoinScatter", 2),
        ("ReinforcementDoor", 1),
        ("ReinforcementJitter", 2),
        ("RuntimeBuildingExitWait", 4),
        ("ScriptRand", 1),
        ("ScrollInitialFrame", 1),
        ("ScrollRevealFrame", 1),
        ("SeekPointAcceptance", 1),
        ("SeekPointDirectionPattern", 1),
        ("SeekPointSelection", 3),
        ("SequenceRecordingBuildingExitWait", 2),
        ("SherwoodBeamMeShuffle", 2),
        ("SherwoodProductionBonusFrame", 1),
        ("SherwoodRelicFrame", 1),
        ("SherwoodReturningPcPlacement", 2),
        ("ShieldAdvance", 1),
        ("SmalltalkStrikeSide", 1),
        ("SoldierBrawlCooldown", 1),
        ("SoldierFreedRotation", 1),
        ("SoldierNoiseCooldown", 1),
        ("SpecialActionRemark", 1),
        ("SpriteBoredStart", 1),
        ("SpriteSnakeStart", 1),
        ("StonePiercingProtection", 1),
        ("SwordDamageProtection", 2),
        ("SwordStrikeSelection", 2),
        ("TitbitUpdate", 1),
        ("TooProudLook", 1),
        ("VipIdleRemark", 1),
        ("WaspDirectionTimer", 1),
        ("WaspMovement", 3),
        ("WaspStingTimer", 1),
        ("WriggleDirection", 1),
    ];

    const REVIEWED_AUXILIARY_SITE_USES: &[(&str, usize)] = &[("PeasantNames", 1)];

    const REVIEWED_PUBLIC_ENTRY_POINTS: &[&str] = &[
        "bool",
        "c_rand_unit_inclusive",
        "f32",
        "i16",
        "i32",
        "install",
        "script_rand",
        "shuffle",
        "u16",
        "u32",
        "u8",
        "uninstall",
        "usize",
        "with_auxiliary_seed",
        "with_seed",
    ];

    const REVIEWED_AMBIENT_RNG_USES: &[(&str, usize)] = &[
        (
            "crates/robin_engine/src/engine/types.rs|fastrand::Rng::with_seed",
            3,
        ),
        (
            "crates/robin_rs/src/game_session/mod.rs|fastrand::Rng::new",
            1,
        ),
        (
            "crates/robin_rs/src/game_session/multiplayer.rs|fastrand::Rng::new",
            1,
        ),
        (
            "crates/robin_rs/src/ingame_menu/dialogue.rs|fastrand::Rng::new",
            2,
        ),
        ("crates/robin_rs/src/multiplayer/lobby.rs|fastrand::u64", 1),
    ];

    #[test]
    fn determinism() {
        let a = with_seed(42, || {
            (0..10)
                .map(|_| u32(RngSite::TitbitUpdate, ..))
                .collect::<Vec<_>>()
        });
        let b = with_seed(42, || {
            (0..10)
                .map(|_| u32(RngSite::TitbitUpdate, ..))
                .collect::<Vec<_>>()
        });
        assert_eq!(a, b);
    }

    #[test]
    fn serde_rng_roundtrip_preserves_state() {
        // Advance to a non-trivial state, serialize, deserialize, pull the
        // same u32 — must match.
        install(fastrand::Rng::with_seed(0xABCD_EF01));
        let _ = u32(RngSite::TitbitUpdate, ..);
        let _ = u32(RngSite::TitbitUpdate, ..);
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
        let _ = u32(RngSite::TitbitUpdate, ..);
        let rng = uninstall();
        // Install again and verify the returned RNG continues state forward.
        install(rng);
        let x1 = u32(RngSite::TitbitUpdate, ..);
        let _advanced = uninstall();
        install(fastrand::Rng::with_seed(7));
        let _ = u32(RngSite::TitbitUpdate, ..);
        let x2 = u32(RngSite::TitbitUpdate, ..);
        assert_eq!(x1, x2);
        let _ = uninstall();
    }

    #[test]
    fn script_rand_range_and_invalid_bounds() {
        with_seed(0xA036, || {
            assert_eq!(script_rand(RngSite::ScriptRand, 1), Ok(0));
            for _ in 0..4096 {
                let value = script_rand(RngSite::ScriptRand, 7).expect("positive script bound");
                assert!((0..7).contains(&value));
            }
        });

        for invalid in [0, -1, i32::MIN] {
            let (result, trace) = with_seed(1, || {
                with_draw_trace(|| script_rand(RngSite::ScriptRand, invalid))
            });
            assert_eq!(result, Err(ScriptRandError::NonPositiveMaximum(invalid)));
            assert!(trace.is_empty(), "invalid Rand must not consume a draw");
        }
    }

    #[test]
    fn integer_and_float_helpers_preserve_reviewed_range_shapes() {
        with_seed(0x3600, || {
            let mut saw_inclusive_min = false;
            let mut saw_inclusive_max = false;
            for _ in 0..4096 {
                let half_open = i32(RngSite::SoldierFreedRotation, -8..9);
                assert!((-8..9).contains(&half_open));

                let inclusive = u16(RngSite::SwordDamageProtection, 1..=3);
                assert!((1..=3).contains(&inclusive));
                saw_inclusive_min |= inclusive == 1;
                saw_inclusive_max |= inclusive == 3;

                let unit = f32(RngSite::LuaMathRandom);
                assert!((0.0..1.0).contains(&unit));

                let c_unit = c_rand_unit_inclusive(RngSite::ReinforcementJitter);
                assert!((0.0..=1.0).contains(&c_unit));
            }
            assert!(saw_inclusive_min && saw_inclusive_max);
        });
    }

    #[test]
    fn original_unit_fraction_includes_both_endpoints() {
        assert_eq!(0u16 as f32 / 32767.0, 0.0);
        assert_eq!(32767u16 as f32 / 32767.0, 1.0);
    }

    #[test]
    fn authoritative_auxiliary_rng_is_seed_derived_and_stream_independent() {
        let generate = || {
            with_auxiliary_seed(AuxiliaryRngSite::PeasantNames, 0xA036, |rng| {
                (0..8)
                    .map(|_| (rng.usize(0..22), rng.usize(0..22)))
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(generate(), generate());

        install(fastrand::Rng::with_seed(0xA036));
        let _ = generate();
        let actual_next = u32(RngSite::TitbitUpdate, ..);
        let _ = uninstall();

        install(fastrand::Rng::with_seed(0xA036));
        let expected_next = u32(RngSite::TitbitUpdate, ..);
        let _ = uninstall();
        assert_eq!(actual_next, expected_next);
    }

    fn is_test_only(attrs: &[syn::Attribute]) -> bool {
        attrs.iter().any(|attr| {
            attr.path().is_ident("test")
                || (attr.path().is_ident("cfg")
                    && matches!(&attr.meta, syn::Meta::List(list) if list.tokens.to_string().contains("test")))
        })
    }

    struct RngSourceVisitor<'a> {
        file: &'a Path,
        sites: BTreeMap<String, usize>,
        auxiliary_sites: BTreeMap<String, usize>,
        unlabelled_calls: Vec<String>,
        ambient_rng: Vec<String>,
        macro_rng: Vec<String>,
    }

    impl RngSourceVisitor<'_> {
        fn path_text(path: &syn::Path) -> String {
            path.segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>()
                .join("::")
        }

        fn site_name(expr: &syn::Expr, enum_name: &str) -> Option<String> {
            let syn::Expr::Path(path) = expr else {
                return None;
            };
            let segments = path.path.segments.iter().collect::<Vec<_>>();
            (segments.len() >= 2 && segments[segments.len() - 2].ident == enum_name)
                .then(|| segments.last().expect("checked length").ident.to_string())
        }
    }

    impl<'ast> Visit<'ast> for RngSourceVisitor<'_> {
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            let tokens = node.tokens.to_string();
            if tokens.contains("sim_rng ::")
                || tokens.contains("fastrand ::")
                || tokens.contains("rand ::")
            {
                self.macro_rng.push(Self::path_text(&node.path));
            }
            syn::visit::visit_macro(self, node);
        }

        fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
            if !is_test_only(&node.attrs) {
                syn::visit::visit_item_mod(self, node);
            }
        }

        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            if !is_test_only(&node.attrs) {
                syn::visit::visit_item_fn(self, node);
            }
        }

        fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
            if !is_test_only(&node.attrs) {
                syn::visit::visit_impl_item_fn(self, node);
            }
        }

        fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
            let segments = node.path.segments.iter().collect::<Vec<_>>();
            if segments.len() >= 2 && segments[segments.len() - 2].ident == "RngSite" {
                *self
                    .sites
                    .entry(segments.last().expect("checked length").ident.to_string())
                    .or_default() += 1;
            }
            if segments.len() >= 2 && segments[segments.len() - 2].ident == "AuxiliaryRngSite" {
                *self
                    .auxiliary_sites
                    .entry(segments.last().expect("checked length").ident.to_string())
                    .or_default() += 1;
            }
            if segments
                .first()
                .is_some_and(|segment| segment.ident == "fastrand" || segment.ident == "rand")
            {
                self.ambient_rng.push(Self::path_text(&node.path));
            }
            syn::visit::visit_expr_path(self, node);
        }

        fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
            let syn::Expr::Path(function) = node.func.as_ref() else {
                syn::visit::visit_expr_call(self, node);
                return;
            };
            let path = Self::path_text(&function.path);
            let helper = function
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string())
                .unwrap_or_default();
            let is_draw = path.contains("sim_rng")
                && matches!(
                    helper.as_str(),
                    "u32"
                        | "i32"
                        | "u16"
                        | "u8"
                        | "i16"
                        | "usize"
                        | "bool"
                        | "f32"
                        | "shuffle"
                        | "c_rand_unit_inclusive"
                        | "script_rand"
                );
            if is_draw {
                let labelled = node
                    .args
                    .first()
                    .and_then(|expr| Self::site_name(expr, "RngSite"))
                    .is_some();
                let forwarded_sprite_site = self.file.ends_with("sprite.rs")
                    && helper == "u16"
                    && node.args.first().is_some_and(
                        |arg| matches!(arg, syn::Expr::Path(path) if path.path.is_ident("site")),
                    );
                if !labelled && !forwarded_sprite_site {
                    self.unlabelled_calls.push(path.clone());
                }
            }
            if path.contains("sim_rng") && helper == "with_auxiliary_seed" {
                let labelled = node
                    .args
                    .first()
                    .and_then(|expr| Self::site_name(expr, "AuxiliaryRngSite"))
                    .is_some();
                if !labelled {
                    self.unlabelled_calls.push(path);
                }
            }
            syn::visit::visit_expr_call(self, node);
        }
    }

    fn rust_sources(root: &Path) -> Vec<PathBuf> {
        let mut pending = vec![root.to_owned()];
        let mut result = Vec::new();
        while let Some(path) = pending.pop() {
            for entry in std::fs::read_dir(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
            {
                let path = entry.expect("read source entry").path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    result.push(path);
                }
            }
        }
        result.sort();
        result
    }

    #[test]
    fn authoritative_rng_source_inventory_is_reviewed() {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repository = manifest
            .join("../..")
            .canonicalize()
            .expect("resolve repository root");
        let roots = [
            manifest.join("src"),
            manifest.join("../robin_lua/src"),
            manifest.join("../robin_rs/src"),
        ];
        let mut actual = BTreeMap::<String, usize>::new();
        let mut actual_auxiliary = BTreeMap::<String, usize>::new();
        let mut actual_ambient = BTreeMap::<String, usize>::new();
        let mut violations = Vec::new();

        let sim_rng_source =
            std::fs::read_to_string(manifest.join("src/sim_rng.rs")).expect("read sim_rng.rs");
        let sim_rng_syntax = syn::parse_file(&sim_rng_source).expect("parse sim_rng.rs");
        let public_entry_points = sim_rng_syntax
            .items
            .iter()
            .filter_map(|item| {
                let syn::Item::Fn(function) = item else {
                    return None;
                };
                matches!(function.vis, syn::Visibility::Public(_))
                    .then(|| function.sig.ident.to_string())
            })
            .collect::<BTreeSet<_>>();
        let expected_entry_points = REVIEWED_PUBLIC_ENTRY_POINTS
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            public_entry_points, expected_entry_points,
            "update the reviewed public sim_rng entry-point inventory"
        );

        for file in roots.iter().flat_map(|root| rust_sources(root)) {
            if file.ends_with("sim_rng.rs") || file.ends_with("engine/tests.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&file)
                .unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
            let syntax = syn::parse_file(&source)
                .unwrap_or_else(|error| panic!("parse {}: {error}", file.display()));
            let mut visitor = RngSourceVisitor {
                file: &file,
                sites: BTreeMap::new(),
                auxiliary_sites: BTreeMap::new(),
                unlabelled_calls: Vec::new(),
                ambient_rng: Vec::new(),
                macro_rng: Vec::new(),
            };
            visitor.visit_file(&syntax);
            for (site, count) in visitor.sites {
                *actual.entry(site).or_default() += count;
            }
            for (site, count) in visitor.auxiliary_sites {
                *actual_auxiliary.entry(site).or_default() += count;
            }
            for call in visitor.unlabelled_calls {
                violations.push(format!("{}: unlabelled {call}", file.display()));
            }
            for macro_path in visitor.macro_rng {
                violations.push(format!(
                    "{}: RNG call hidden inside {macro_path}! macro",
                    file.display()
                ));
            }
            let relative = file
                .canonicalize()
                .expect("resolve scanned source")
                .strip_prefix(&repository)
                .expect("source must be inside repository")
                .to_owned();
            for call in visitor.ambient_rng {
                *actual_ambient
                    .entry(format!("{}|{call}", relative.display()))
                    .or_default() += 1;
            }
        }

        assert!(violations.is_empty(), "{}", violations.join("\n"));
        let expected = REVIEWED_SITE_USES
            .iter()
            .map(|&(site, count)| (site.to_owned(), count))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(actual, expected, "update the reviewed RNG inventory");

        let expected_auxiliary = REVIEWED_AUXILIARY_SITE_USES
            .iter()
            .map(|&(site, count)| (site.to_owned(), count))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            actual_auxiliary, expected_auxiliary,
            "update the reviewed auxiliary RNG inventory"
        );

        let expected_ambient = REVIEWED_AMBIENT_RNG_USES
            .iter()
            .map(|&(site, count)| (site.to_owned(), count))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            actual_ambient, expected_ambient,
            "update the reviewed ambient RNG exception inventory"
        );

        let enum_sites = RngSite::iter()
            .map(|site| format!("{site:?}"))
            .collect::<BTreeSet<_>>();
        let expected_sites = expected.keys().cloned().collect::<BTreeSet<_>>();
        assert_eq!(enum_sites, expected_sites, "RngSite and inventory differ");

        let auxiliary_enum_sites = AuxiliaryRngSite::iter()
            .map(|site| format!("{site:?}"))
            .collect::<BTreeSet<_>>();
        let expected_auxiliary_sites = expected_auxiliary.keys().cloned().collect::<BTreeSet<_>>();
        assert_eq!(
            auxiliary_enum_sites, expected_auxiliary_sites,
            "AuxiliaryRngSite and inventory differ"
        );

        let docs = std::fs::read_to_string(manifest.join("../../docs/RNG_AUDIT.md"))
            .expect("docs/RNG_AUDIT.md must exist");
        for site in enum_sites {
            assert!(
                docs.contains(&format!("| `{site}` |")),
                "docs/RNG_AUDIT.md is missing {site}"
            );
        }
        for site in auxiliary_enum_sites {
            assert!(
                docs.contains(&format!("| `AuxiliaryRngSite::{site}` |")),
                "docs/RNG_AUDIT.md is missing AuxiliaryRngSite::{site}"
            );
        }
    }
}
