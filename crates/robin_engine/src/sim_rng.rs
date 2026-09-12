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
        config
            .validate()
            .expect("cannot create simulation context with invalid difficulty rules");
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode)]
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
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ScriptVmDiagnosticContext {
    pub vm_key: String,
    pub class_name: String,
    pub method_name: String,
    pub native_max: Option<i32>,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct ScriptZoneQueryDiagnostic {
    /// RNG cursor at the instant the query ran. This associates a query with
    /// the ScriptRand draw it can conditionally enable without consuming RNG.
    pub rng_cursor: usize,
    pub vm: ScriptVmDiagnosticContext,
    pub location_handle: i32,
    pub occupant_handles: Vec<i32>,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
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

    fn last_draw(&self) -> Option<u32> {
        self.cursor
            .checked_sub(1)
            .and_then(|index| self.draws.get(index).copied())
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
/// test in `tests/rng_inventory.rs` checks their structural use, so adding gameplay randomness requires an
/// explicit review rather than an unlabelled call to a generic RNG helper.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    strum_macros::EnumIter,
    bitcode::Encode,
    bitcode::Decode,
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
    // This observes the ordinary fastrand path, unlike OriginalRngReplay's
    // sites(), which selects a different generator. Always clear the observer
    // even when a test deliberately exercises an invariant panic.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    let trace = DRAW_TRACE.with(|trace| trace.borrow_mut().take().expect("draw trace missing"));
    match result {
        Ok(result) => (result, trace),
        Err(payload) => std::panic::resume_unwind(payload),
    }
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
        let trace = rng_trace_config();
        if trace.enabled && (trace.from..=trace.through).contains(&index) {
            eprintln!("simulation RNG draw index={index} site={site:?} raw={value}");
            if trace.backtrace {
                eprintln!("{}", std::backtrace::Backtrace::force_capture());
            }
        }

        value
    })
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RngTraceConfig {
    enabled: bool,
    backtrace: bool,
    from: usize,
    through: usize,
}

fn rng_trace_config() -> &'static RngTraceConfig {
    static CONFIG: std::sync::OnceLock<RngTraceConfig> = std::sync::OnceLock::new();
    CONFIG.get_or_init(|| {
        let bound = |name, default| match std::env::var(name) {
            Ok(value) => value
                .parse()
                .unwrap_or_else(|_| panic!("{name} must be a non-negative integer")),
            Err(std::env::VarError::NotPresent) => default,
            Err(error) => panic!("cannot read {name}: {error}"),
        };
        let mode = std::env::var_os("ROBIN_TRACE_RNG");
        RngTraceConfig {
            enabled: mode.is_some(),
            backtrace: mode.as_deref().is_some_and(|mode| mode == "backtrace"),
            from: bound("ROBIN_TRACE_RNG_FROM", 0),
            through: bound("ROBIN_TRACE_RNG_THROUGH", usize::MAX),
        }
    })
}

/// Return the raw value of the most recent Original replay draw without
/// advancing either RNG stream. This is intended only for opt-in diagnostics
/// that call it immediately after their own draw.
pub(crate) fn last_original_raw_draw(context: &SimulationContext) -> Option<u32> {
    context.original_replay.as_ref().and_then(|replay| {
        replay
            .lock()
            .expect("original RNG replay mutex poisoned")
            .last_draw()
    })
}

/// Return the current Original replay cursor without advancing either RNG
/// stream. This is parity-diagnostic state only and must not gate behavior.
pub(crate) fn original_replay_cursor(context: &SimulationContext) -> Option<usize> {
    context.original_replay.as_ref().map(|replay| {
        replay
            .lock()
            .expect("original RNG replay mutex poisoned")
            .cursor()
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
    // Match the 64-bit native stream on every target, including wasm32.
    // fastrand::usize uses a different reduction algorithm on 32-bit targets.
    let widen = |bound: Bound<&usize>| match bound {
        Bound::Included(value) => Bound::Included(*value as u64),
        Bound::Excluded(value) => Bound::Excluded(*value as u64),
        Bound::Unbounded => Bound::Unbounded,
    };
    let bounds = (widen(range.start_bound()), widen(range.end_bound()));
    with_rng(context, site, |rng| {
        usize::try_from(rng.u64(bounds)).expect("random index does not fit this target")
    })
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
    with_rng(context, site, |rng| {
        // Preserve fastrand native shuffle order with fixed-width index draws.
        for index in 1..slice.len() {
            slice.swap(index, rng.u64(..=index as u64) as usize);
        }
    });
}

/// Original-game random fraction, including both 0 and 1.
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
/// The original game takes a random value modulo the requested maximum; zero is fatal and
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
    #[test]
    fn index_draws_and_shuffle_match_fixed_width_stream() {
        let context = super::SimulationContext::with_seed(17);
        let mut reference = fastrand::Rng::with_seed(17);
        for upper in [1usize, 22, 257, 65_537] {
            for _ in 0..20 {
                assert_eq!(
                    super::usize(&context, super::RngSite::ScriptRand, 0..upper),
                    reference.u64(0..upper as u64) as usize
                );
            }
        }
        let mut actual: Vec<_> = (0..100).collect();
        let mut expected = actual.clone();
        super::shuffle(&context, super::RngSite::ScriptRand, &mut actual);
        for index in 1..expected.len() {
            expected.swap(index, reference.u64(..=index as u64) as usize);
        }
        assert_eq!(actual, expected);
        assert_eq!(context.seed(), reference.get_seed());
    }

    use super::*;

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
        assert_eq!(32767_f32 / 32767.0, 1.0);
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
}
