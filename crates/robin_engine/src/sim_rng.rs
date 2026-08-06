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
//! The authoritative state is the `SimulationRng` owned by one engine's
//! `SimulationControl`. Gameplay, AI, scripts, and level setup receive an
//! explicit [`SimulationContext`] handle to that exact allocation. There is no
//! ambient RNG scope: a caller that lacks a context cannot draw.
//!
//! **Rules:**
//! - Gameplay code must call `sim_rng::{u32, usize, u8, bool, …}` with its
//!   explicit context —
//!   never `rand::*` or `fastrand::*` globals directly.
//! - Non-simulation code (audio jitter, menus, loading screens) may still use
//!   ambient RNG; those must not feed back into simulation state. See
//!   `sound.rs` / `ingame_menu/*` for examples.
//! - Authoritative auxiliary randomness that intentionally does not advance
//!   the serialized stream must use a reviewed [`AuxiliaryRngSite`].
//! - Focused tests and tools construct a standalone [`SimulationContext`] with
//!   [`SimulationContext::with_seed`].

use std::ops::{Bound, RangeBounds};
use std::sync::{Arc, Mutex};

#[cfg(test)]
use std::cell::RefCell;

/// Explicit capability for authoritative simulation randomness.
///
/// The engine only lends this non-`Clone`, non-serializable capability through
/// synchronous call boundaries. It cannot be detached from the owning
/// `SimulationRng`; engine snapshot cloning and serialization operate on that
/// owner, never on the capability.
pub struct SimulationContext {
    rng: Arc<Mutex<fastrand::Rng>>,
    original_replay: Option<Arc<Mutex<OriginalRngReplay>>>,
    config: crate::engine::SimConfig,
}

impl SimulationContext {
    pub(crate) fn new(
        rng: Arc<Mutex<fastrand::Rng>>,
        original_replay: Option<Arc<Mutex<OriginalRngReplay>>>,
        config: crate::engine::SimConfig,
    ) -> Self {
        Self {
            rng,
            original_replay,
            config,
        }
    }

    #[allow(clippy::disallowed_methods)]
    pub fn with_seed(seed: u64) -> Self {
        Self::with_seed_and_config(seed, crate::engine::SimConfig::default())
    }

    #[allow(clippy::disallowed_methods)]
    pub fn with_seed_and_config(seed: u64, config: crate::engine::SimConfig) -> Self {
        Self {
            rng: Arc::new(Mutex::new(fastrand::Rng::with_seed(seed))),
            original_replay: None,
            config,
        }
    }

    pub fn config(&self) -> crate::engine::SimConfig {
        self.config
    }

    pub fn seed(&self) -> u64 {
        self.rng
            .lock()
            .expect("simulation RNG mutex poisoned")
            .get_seed()
    }
}

/// Raw libc `rand()` values supplied by an original-game parity trace.
///
/// This is a diagnostic execution mode, not a replacement saved-game RNG.
/// Every Rust authoritative draw consumes exactly one value in global order;
/// the parity runner checks the cursor at every original frame boundary.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OriginalRngReplay {
    draws: Vec<u32>,
    cursor: usize,
    sites: Vec<RngSite>,
    script_rand_contexts: Vec<Option<ScriptVmDiagnosticContext>>,
    script_zone_queries: Vec<ScriptZoneQueryDiagnostic>,
}

/// Non-authoritative provenance for script-native parity diagnostics.
///
/// This deliberately uses display strings instead of engine-owned VM types so
/// the simulation RNG remains independent of the script driver's internals.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScriptVmDiagnosticContext {
    pub vm_key: String,
    pub class_name: String,
    pub method_name: String,
    pub native_max: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScriptZoneQueryDiagnostic {
    /// RNG cursor at the instant the query ran. This associates a query with
    /// the ScriptRand draw it can conditionally enable without consuming RNG.
    pub rng_cursor: usize,
    pub vm: ScriptVmDiagnosticContext,
    pub location_handle: i32,
    pub occupant_handles: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OriginalRngDiagnostics {
    pub script_rand_contexts: Vec<Option<ScriptVmDiagnosticContext>>,
    pub script_zone_queries: Vec<ScriptZoneQueryDiagnostic>,
}

impl OriginalRngReplay {
    pub fn new(draws: Vec<u32>) -> Self {
        Self {
            draws,
            cursor: 0,
            sites: Vec::new(),
            script_rand_contexts: Vec::new(),
            script_zone_queries: Vec::new(),
        }
    }

    pub fn append(&mut self, draws: impl IntoIterator<Item = u32>) {
        self.draws.extend(draws);
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn sites(&self, range: std::ops::Range<usize>) -> Vec<RngSite> {
        self.sites
            .get(range.clone())
            .unwrap_or_else(|| panic!("RNG site history does not contain {range:?}"))
            .to_vec()
    }

    pub fn diagnostics(&self, range: std::ops::Range<usize>) -> OriginalRngDiagnostics {
        let script_rand_contexts = self
            .script_rand_contexts
            .get(range.clone())
            .unwrap_or_else(|| panic!("RNG diagnostic history does not contain {range:?}"))
            .to_vec();
        let script_zone_queries = self
            .script_zone_queries
            .iter()
            .filter(|query| (range.start..=range.end).contains(&query.rng_cursor))
            .cloned()
            .collect();
        OriginalRngDiagnostics {
            script_rand_contexts,
            script_zone_queries,
        }
    }

    fn draw(&mut self, site: RngSite) -> u32 {
        let index = self.cursor;
        let value = *self.draws.get(index).unwrap_or_else(|| {
            panic!("original RNG replay exhausted at draw {index} requested by {site:?}")
        });
        self.cursor += 1;
        self.sites.push(site);
        self.script_rand_contexts.push(None);
        value
    }

    fn attach_script_rand_context(&mut self, context: ScriptVmDiagnosticContext) {
        self.script_rand_contexts
            .last_mut()
            .expect("ScriptRand diagnostic recorded before its RNG draw")
            .replace(context);
    }

    fn record_script_zone_query(
        &mut self,
        vm: ScriptVmDiagnosticContext,
        location_handle: i32,
        occupant_handles: Vec<i32>,
    ) {
        self.script_zone_queries.push(ScriptZoneQueryDiagnostic {
            rng_cursor: self.cursor,
            vm,
            location_handle,
            occupant_handles,
        });
    }

    pub(crate) fn state_hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        std::hash::Hash::hash(&self.draws, hasher);
        std::hash::Hash::hash(&self.cursor, hasher);
        for site in &self.sites {
            std::hash::Hash::hash(&std::mem::discriminant(site), hasher);
        }
    }
}

impl SimulationContext {
    pub(crate) fn record_script_zone_query(
        &self,
        vm: &ScriptVmDiagnosticContext,
        location_handle: i32,
        occupant_handles: Vec<i32>,
    ) {
        if let Some(replay) = &self.original_replay {
            replay
                .lock()
                .expect("original RNG replay mutex poisoned")
                .record_script_zone_query(vm.clone(), location_handle, occupant_handles);
        }
    }
}

/// Construct an explicit deterministic context for one focused call chain.
/// Unlike the removed legacy helper, this installs no thread-local state.
pub fn with_seed<R>(seed: u64, f: impl FnOnce(&SimulationContext) -> R) -> R {
    let context = SimulationContext::with_seed(seed);
    f(&context)
}

#[cfg(test)]
pub(crate) fn test_context() -> SimulationContext {
    SimulationContext::with_seed(1)
}

/// Reviewed authoritative gameplay RNG entry points.
///
/// Every production draw must name one of these sites.  The source-audit
/// test at the bottom of this module compares their structural use against
/// `docs/RNG_AUDIT.md`, so adding or moving gameplay randomness requires an
/// explicit review rather than an unlabelled call to a generic RNG helper.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, strum_macros::EnumIter,
)]
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
    RescuePcFirstName,
    RescuePcSurname,
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
    DelayedSoundTimer,
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

fn with_rng<R>(
    context: &SimulationContext,
    site: RngSite,
    f: impl FnOnce(&mut fastrand::Rng) -> R,
) -> R {
    #[cfg(test)]
    DRAW_TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.push(site);
        }
    });
    #[cfg(not(test))]
    let _ = site;
    f(&mut context.rng.lock().expect("simulation RNG mutex poisoned"))
}

fn original_draw(context: &SimulationContext, site: RngSite) -> Option<u32> {
    context.original_replay.as_ref().map(|replay| {
        let mut replay = replay.lock().expect("original RNG replay mutex poisoned");
        let index = replay.cursor();
        let value = replay.draw(site);

        // This is intentionally generic parity tooling rather than a
        // site-specific diagnostic. An Original replay can expose its complete
        // global draw stream without another code change.
        let trace_from = std::env::var("ROBIN_TRACE_RNG_FROM")
            .ok()
            .map(|value| {
                value
                    .parse::<usize>()
                    .expect("ROBIN_TRACE_RNG_FROM must be a non-negative integer")
            })
            .unwrap_or(0);
        let trace_through = std::env::var("ROBIN_TRACE_RNG_THROUGH")
            .ok()
            .map(|value| {
                value
                    .parse::<usize>()
                    .expect("ROBIN_TRACE_RNG_THROUGH must be a non-negative integer")
            })
            .unwrap_or(usize::MAX);
        if let Some(mode) = std::env::var_os("ROBIN_TRACE_RNG")
            && (trace_from..=trace_through).contains(&index)
        {
            eprintln!("simulation RNG draw index={index} site={site:?} raw={value}");
            if mode == "backtrace" {
                eprintln!("{}", std::backtrace::Backtrace::force_capture());
            }
        }

        value
    })
}

fn unsigned_bounds<T>(range: &impl RangeBounds<T>, max: u64) -> (u64, u64)
where
    T: Copy + Into<u64>,
{
    let start = match range.start_bound() {
        Bound::Included(value) => (*value).into(),
        Bound::Excluded(value) => (*value).into() + 1,
        Bound::Unbounded => 0,
    };
    let end = match range.end_bound() {
        Bound::Included(value) => (*value).into() + 1,
        Bound::Excluded(value) => (*value).into(),
        Bound::Unbounded => max + 1,
    };
    assert!(start < end, "empty RNG range {start}..{end}");
    (start, end)
}

fn replay_unsigned(raw: u32, start: u64, end: u64) -> u64 {
    start + u64::from(raw) % (end - start)
}

// ─── Range helpers (mirror fastrand's API) ───────────────────────────

pub fn u32(context: &SimulationContext, site: RngSite, range: impl RangeBounds<u32>) -> u32 {
    if let Some(raw) = original_draw(context, site) {
        let (start, end) = unsigned_bounds(&range, u64::from(u32::MAX));
        return replay_unsigned(raw, start, end) as u32;
    }
    with_rng(context, site, |rng| rng.u32(range))
}

pub fn i32(context: &SimulationContext, site: RngSite, range: impl RangeBounds<i32>) -> i32 {
    if let Some(raw) = original_draw(context, site) {
        let start = match range.start_bound() {
            Bound::Included(value) => i64::from(*value),
            Bound::Excluded(value) => i64::from(*value) + 1,
            Bound::Unbounded => i64::from(i32::MIN),
        };
        let end = match range.end_bound() {
            Bound::Included(value) => i64::from(*value) + 1,
            Bound::Excluded(value) => i64::from(*value),
            Bound::Unbounded => i64::from(i32::MAX) + 1,
        };
        assert!(start < end, "empty RNG range {start}..{end}");
        return (start + i64::from(raw) % (end - start)) as i32;
    }
    with_rng(context, site, |rng| rng.i32(range))
}

pub fn u16(context: &SimulationContext, site: RngSite, range: impl RangeBounds<u16>) -> u16 {
    if let Some(raw) = original_draw(context, site) {
        let (start, end) = unsigned_bounds(&range, u64::from(u16::MAX));
        return replay_unsigned(raw, start, end) as u16;
    }
    with_rng(context, site, |rng| rng.u16(range))
}

pub fn u8(context: &SimulationContext, site: RngSite, range: impl RangeBounds<u8>) -> u8 {
    if let Some(raw) = original_draw(context, site) {
        let (start, end) = unsigned_bounds(&range, u64::from(u8::MAX));
        return replay_unsigned(raw, start, end) as u8;
    }
    with_rng(context, site, |rng| rng.u8(range))
}

pub fn i16(context: &SimulationContext, site: RngSite, range: impl RangeBounds<i16>) -> i16 {
    if let Some(raw) = original_draw(context, site) {
        let start = match range.start_bound() {
            Bound::Included(value) => i32::from(*value),
            Bound::Excluded(value) => i32::from(*value) + 1,
            Bound::Unbounded => i32::from(i16::MIN),
        };
        let end = match range.end_bound() {
            Bound::Included(value) => i32::from(*value) + 1,
            Bound::Excluded(value) => i32::from(*value),
            Bound::Unbounded => i32::from(i16::MAX) + 1,
        };
        assert!(start < end, "empty RNG range {start}..{end}");
        return (start + (raw % (end - start) as u32) as i32) as i16;
    }
    with_rng(context, site, |rng| rng.i16(range))
}

pub fn usize(context: &SimulationContext, site: RngSite, range: impl RangeBounds<usize>) -> usize {
    if let Some(raw) = original_draw(context, site) {
        let start = match range.start_bound() {
            Bound::Included(value) => *value,
            Bound::Excluded(value) => value.checked_add(1).expect("RNG range start overflow"),
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Included(value) => value.checked_add(1).expect("RNG range end overflow"),
            Bound::Excluded(value) => *value,
            Bound::Unbounded => usize::MAX,
        };
        assert!(start < end, "empty RNG range {start}..{end}");
        return start + raw as usize % (end - start);
    }
    with_rng(context, site, |rng| rng.usize(range))
}

pub fn bool(context: &SimulationContext, site: RngSite) -> bool {
    if let Some(raw) = original_draw(context, site) {
        return raw % 2 != 0;
    }
    with_rng(context, site, |rng| rng.bool())
}

pub fn f32(context: &SimulationContext, site: RngSite) -> f32 {
    if let Some(raw) = original_draw(context, site) {
        return raw as f32 / 2_147_483_647.0;
    }
    with_rng(context, site, |rng| rng.f32())
}

/// Shuffle a slice in-place using the simulation RNG.
pub fn shuffle<T>(context: &SimulationContext, site: RngSite, slice: &mut [T]) {
    if context.original_replay.is_some() {
        for upper in (1..slice.len()).rev() {
            let selected = usize(context, site, 0..=upper);
            slice.swap(upper, selected);
        }
        return;
    }
    with_rng(context, site, |rng| rng.shuffle(slice));
}

/// Original's MSVC-era `rand() / RAND_MAX` fraction, including both 0 and 1.
///
/// The shipped code assumes `RAND_MAX == 32767` at the two authoritative
/// floating-point call sites.  We retain those range semantics without
/// attempting to reproduce the libc output sequence.
pub fn c_rand_unit_inclusive(context: &SimulationContext, site: RngSite) -> f32 {
    if let Some(raw) = original_draw(context, site) {
        return raw as f32 / 2_147_483_647.0;
    }
    u16(context, site, 0..=32767) as f32 / 32767.0
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

pub fn script_rand(
    context: &SimulationContext,
    site: RngSite,
    max: i32,
) -> Result<i32, ScriptRandError> {
    if max <= 0 {
        return Err(ScriptRandError::NonPositiveMaximum(max));
    }
    Ok(i32(context, site, 0..max))
}

pub fn script_rand_with_context(
    context: &SimulationContext,
    site: RngSite,
    max: i32,
    mut diagnostic: ScriptVmDiagnosticContext,
) -> Result<i32, ScriptRandError> {
    if max <= 0 {
        return Err(ScriptRandError::NonPositiveMaximum(max));
    }
    diagnostic.native_max = Some(max);
    if let Some(raw) = original_draw(context, site) {
        context
            .original_replay
            .as_ref()
            .expect("Original draw lost its replay owner")
            .lock()
            .expect("original RNG replay mutex poisoned")
            .attach_script_rand_context(diagnostic);
        return Ok((raw % max as u32) as i32);
    }
    Ok(with_rng(context, site, |rng| rng.i32(0..max)))
}

/// `serde` adapters for `fastrand::Rng`.
///
/// Use with `#[serde(with = "crate::sim_rng::serde_rng")]` on any
/// `fastrand::Rng` field. The RNG is serialized as a single `u64` via
/// [`fastrand::Rng::get_seed`] / [`fastrand::Rng::with_seed`], which
/// preserves the full internal state (fastrand's PRNG state IS the seed).
///
/// Used by the Engine-owned [`crate::engine::SimulationRng`], so save files,
/// rollback snapshots, network state-sync, and desync dumps preserve the exact
/// next simulation roll.
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
        ("DoorFightDispersion", 2),
        ("DoorFightTarget", 1),
        ("DrunkCombatFreeze", 2),
        ("DrunkenPathDeviation", 2),
        ("EnemySeekDirectionShuffle", 1),
        ("EnemySeekLook", 2),
        ("EnemyWonderingLook", 5),
        ("HeroSpeech", 1),
        ("LevelBonusInitialFrame", 2),
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
        ("RescuePcFirstName", 1),
        ("RescuePcSurname", 1),
        ("RuntimeBuildingExitWait", 4),
        ("ScriptRand", 2),
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

    const REVIEWED_AUXILIARY_SITE_USES: &[(&str, usize)] =
        &[("DelayedSoundTimer", 1), ("PeasantNames", 1)];

    const REVIEWED_PUBLIC_ENTRY_POINTS: &[&str] = &[
        "bool",
        "c_rand_unit_inclusive",
        "f32",
        "i16",
        "i32",
        "script_rand",
        "script_rand_with_context",
        "shuffle",
        "u16",
        "u32",
        "u8",
        "usize",
        "with_auxiliary_seed",
        "with_seed",
    ];

    const REVIEWED_AMBIENT_RNG_USES: &[(&str, usize)] = &[
        (
            "crates/robin_engine/src/engine/types.rs|fastrand::Rng::with_seed",
            2,
        ),
        (
            "crates/robin_rs/src/game_session/interactive.rs|fastrand::Rng::new",
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
        let a = with_seed(42, |sim| {
            (0..10)
                .map(|_| u32(sim, RngSite::TitbitUpdate, ..))
                .collect::<Vec<_>>()
        });
        let b = with_seed(42, |sim| {
            (0..10)
                .map(|_| u32(sim, RngSite::TitbitUpdate, ..))
                .collect::<Vec<_>>()
        });
        assert_eq!(a, b);
    }

    #[test]
    fn simulation_rng_serde_roundtrip_preserves_state() {
        // Advance the real serialized owner to a non-trivial state, then
        // verify the restored owner continues with the same draws.
        let original = crate::engine::SimulationRng::with_seed(0xABCD_EF01);
        let original_context = original.context(crate::engine::SimConfig::default());
        let _ = u32(&original_context, RngSite::TitbitUpdate, ..);
        let _ = u32(&original_context, RngSite::TitbitUpdate, ..);
        let encoded = serde_json::to_string(&original).expect("serialize simulation RNG owner");
        let restored: crate::engine::SimulationRng =
            serde_json::from_str(&encoded).expect("deserialize simulation RNG owner");
        let restored_context = restored.context(crate::engine::SimConfig::default());

        assert_eq!(
            u32(&original_context, RngSite::TitbitUpdate, ..),
            u32(&restored_context, RngSite::TitbitUpdate, ..)
        );
        assert_eq!(
            u32(&original_context, RngSite::TitbitUpdate, ..),
            u32(&restored_context, RngSite::TitbitUpdate, ..)
        );
    }

    #[test]
    fn explicit_context_advances_one_owned_stream() {
        let first = SimulationContext::with_seed(7);
        let _ = u32(&first, RngSite::TitbitUpdate, ..);
        let x1 = u32(&first, RngSite::TitbitUpdate, ..);
        let second = SimulationContext::with_seed(7);
        let _ = u32(&second, RngSite::TitbitUpdate, ..);
        let x2 = u32(&second, RngSite::TitbitUpdate, ..);
        assert_eq!(x1, x2);
    }

    #[test]
    fn script_rand_range_and_invalid_bounds() {
        with_seed(0xA036, |sim| {
            assert_eq!(script_rand(sim, RngSite::ScriptRand, 1), Ok(0));
            for _ in 0..4096 {
                let value =
                    script_rand(sim, RngSite::ScriptRand, 7).expect("positive script bound");
                assert!((0..7).contains(&value));
            }
        });

        for invalid in [0, -1, i32::MIN] {
            let (result, trace) = with_seed(1, |sim| {
                with_draw_trace(|| script_rand(sim, RngSite::ScriptRand, invalid))
            });
            assert_eq!(result, Err(ScriptRandError::NonPositiveMaximum(invalid)));
            assert!(trace.is_empty(), "invalid Rand must not consume a draw");
        }
    }

    #[test]
    fn original_replay_diagnostics_do_not_affect_authoritative_hash() {
        use std::hash::Hasher;

        let vm = ScriptVmDiagnosticContext {
            vm_key: "Global".into(),
            class_name: "StartUp".into(),
            method_name: "Hourglass".into(),
            native_max: Some(3),
        };
        let mut plain = OriginalRngReplay::new(vec![5]);
        let mut diagnosed = plain.clone();
        assert_eq!(plain.draw(RngSite::ScriptRand), 5);
        assert_eq!(diagnosed.draw(RngSite::ScriptRand), 5);
        diagnosed.attach_script_rand_context(vm.clone());
        diagnosed.record_script_zone_query(vm.clone(), 17, vec![4, 9]);

        let diagnostics = diagnosed.diagnostics(0..1);
        assert_eq!(diagnostics.script_rand_contexts, vec![Some(vm.clone())]);
        assert_eq!(diagnostics.script_zone_queries.len(), 1);
        assert_eq!(diagnostics.script_zone_queries[0].occupant_handles, [4, 9]);

        let mut plain_hash = std::collections::hash_map::DefaultHasher::new();
        let mut diagnosed_hash = std::collections::hash_map::DefaultHasher::new();
        plain.state_hash(&mut plain_hash);
        diagnosed.state_hash(&mut diagnosed_hash);
        assert_eq!(plain_hash.finish(), diagnosed_hash.finish());
    }

    #[test]
    fn integer_and_float_helpers_preserve_reviewed_range_shapes() {
        with_seed(0x3600, |sim| {
            let mut saw_inclusive_min = false;
            let mut saw_inclusive_max = false;
            for _ in 0..4096 {
                let half_open = i32(sim, RngSite::SoldierFreedRotation, -8..9);
                assert!((-8..9).contains(&half_open));

                let inclusive = u16(sim, RngSite::SwordDamageProtection, 1..=3);
                assert!((1..=3).contains(&inclusive));
                saw_inclusive_min |= inclusive == 1;
                saw_inclusive_max |= inclusive == 3;

                let unit = f32(sim, RngSite::LuaMathRandom);
                assert!((0.0..1.0).contains(&unit));

                let c_unit = c_rand_unit_inclusive(sim, RngSite::ReinforcementJitter);
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

        let simulation = SimulationContext::with_seed(0xA036);
        let _ = generate();
        let actual_next = u32(&simulation, RngSite::TitbitUpdate, ..);

        let expected = SimulationContext::with_seed(0xA036);
        let expected_next = u32(&expected, RngSite::TitbitUpdate, ..);
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
            // Diagnostic macros may name RNG *sites* (for the parity RNG
            // owner traces) without drawing; only a draw entry point hidden
            // inside a macro body is a violation.
            let tokens = node
                .tokens
                .to_string()
                .replace("sim_rng :: RngSite", "")
                .replace("sim_rng :: AuxiliaryRngSite", "");
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
                    .iter()
                    .nth(1)
                    .and_then(|expr| Self::site_name(expr, "RngSite"))
                    .is_some();
                let forwarded_sprite_site = self.file.ends_with("sprite.rs")
                    && helper == "u16"
                    && node.args.iter().nth(1).is_some_and(
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
