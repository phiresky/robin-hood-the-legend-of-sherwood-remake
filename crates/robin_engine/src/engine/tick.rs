//! Main per-frame update tick (`perform_hourglass`).

use super::movement::{CompletedPathWork, MovementContext};
#[cfg(test)]
use super::sequence_runtime::{
    DirectAbilityCommandContext, LiftWaitCommandContext, NpcAttentionCommandContext,
    NpcStateCommandContext, OwnerActionBarrier, PositionAssertionContext, StealthCommandContext,
    TurnCommandContext, WaitCommandContext,
};
use super::sequence_runtime::{required_canonical_door, required_canonical_door_mut};
use super::*;
use crate::abilities;
use crate::element::{Command, Entity, EntityId};
use crate::entities::EntitySlots;
use crate::game_operation::GameCode;
use crate::messenger::{Message, MessageType, SimpleMessage};
use crate::profiles::MissionType;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NpcHourglassPhase {
    SoldierPrelude,
    Patrol,
    BaseHuman,
    Broadcasts,
    View,
    Detection,
    Ambush,
    Busy,
    Ladder,
    LockGate,
    SixteenthFrame,
    NormalTimer,
    MacroTimer,
    QueuedStimuli,
}

#[cfg(test)]
thread_local! {
    static NPC_HOURGLASS_PHASE_TRACE: std::cell::RefCell<Option<Vec<NpcHourglassPhase>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn observe_npc_hourglass_phase(phase: NpcHourglassPhase) {
    NPC_HOURGLASS_PHASE_TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.push(phase);
        }
    });
}

#[cfg(not(test))]
fn observe_npc_hourglass_phase(_phase: ()) {}

#[cfg(test)]
pub(super) fn capture_npc_hourglass_phases<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<NpcHourglassPhase>) {
    NPC_HOURGLASS_PHASE_TRACE.with(|trace| {
        assert!(trace.borrow().is_none(), "phase capture is not re-entrant");
        *trace.borrow_mut() = Some(Vec::new());
    });
    let result = f();
    let phases = NPC_HOURGLASS_PHASE_TRACE.with(|trace| {
        trace
            .borrow_mut()
            .take()
            .expect("phase capture must remain active")
    });
    (result, phases)
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ActorAnimationBoundaryPhase {
    WaitReady(EntityId),
    GenericExecute(EntityId),
    CompletionEffects(EntityId),
    CombatInjuryThink(EntityId),
    ActionChange(EntityId),
}

#[cfg(test)]
thread_local! {
    static ACTOR_ANIMATION_BOUNDARY_TRACE: std::cell::RefCell<Option<Vec<ActorAnimationBoundaryPhase>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn observe_actor_animation_boundary(phase: ActorAnimationBoundaryPhase) {
    ACTOR_ANIMATION_BOUNDARY_TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.push(phase);
        }
    });
}

#[cfg(test)]
pub(super) fn capture_actor_animation_boundary<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<ActorAnimationBoundaryPhase>) {
    ACTOR_ANIMATION_BOUNDARY_TRACE.with(|trace| {
        assert!(
            trace.borrow_mut().replace(Vec::new()).is_none(),
            "actor animation boundary capture is not re-entrant"
        );
    });
    let result = f();
    let phases = ACTOR_ANIMATION_BOUNDARY_TRACE.with(|trace| {
        trace
            .borrow_mut()
            .take()
            .expect("actor animation boundary capture must remain active")
    });
    (result, phases)
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ActorOwnerEnvelopePhase {
    SoldierPrelude(EntityId),
    Patrol(EntityId),
    HumanPrelude(EntityId),
    BaseActor(EntityId),
    MovementExecute(EntityId),
    HumanNoise(EntityId),
    HumanTiredness(EntityId),
    PcTail(EntityId),
    NpcTail(EntityId),
}

#[cfg(test)]
thread_local! {
    static ACTOR_OWNER_ENVELOPE_TRACE: std::cell::RefCell<Option<Vec<ActorOwnerEnvelopePhase>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn observe_actor_owner_envelope(phase: ActorOwnerEnvelopePhase) {
    ACTOR_OWNER_ENVELOPE_TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.push(phase);
        }
    });
}

#[cfg(test)]
pub(super) fn capture_actor_owner_envelope<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<ActorOwnerEnvelopePhase>) {
    ACTOR_OWNER_ENVELOPE_TRACE.with(|trace| {
        assert!(
            trace.borrow_mut().replace(Vec::new()).is_none(),
            "actor-owner envelope capture is not re-entrant"
        );
    });
    let result = f();
    let phases = ACTOR_OWNER_ENVELOPE_TRACE.with(|trace| {
        trace
            .borrow_mut()
            .take()
            .expect("actor-owner envelope capture must remain active")
    });
    (result, phases)
}

// ─── Per-tick timing instrumentation ─────────────────────────────────
//
// Records the wall-clock duration of every `perform_hourglass` call
// and emits a periodic summary so we can see where the rollback
// checker's 25-replays-per-frame cost actually goes. Lives in a
// thread-local so the live tick and the rollback-replay ticks each get
// their own histogram (rollback runs on the same thread but typically
// happens in bursts of 25, so they'll dominate any window they hit).
thread_local! {
    static HOURGLASS_STATS: std::cell::RefCell<HourglassStats> =
        std::cell::RefCell::new(HourglassStats::default());
}

/// Number of `perform_hourglass` calls between log lines.
const HOURGLASS_LOG_INTERVAL: u32 = 100;

/// Coarse, ordered phases of [`EngineInner::perform_hourglass_inner`].
///
/// Keep these deliberately broader than individual systems: the phase trace is
/// an ordering contract for the tick spine, not a second scheduler.  In
/// particular, `Paths` names the Rust port's prior-tick retry maintenance;
/// path construction itself is synchronous during `Sequences` (see the parity
/// audit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HourglassPhase {
    DeferredEffectsStart,
    MissionAndMessages,
    NpcOrders,
    Paths,
    Entities,
    EntitySystems,
    Npcs,
    GameplaySystems,
    Sequences,
    DeferredEffectsEnd,
}

#[cfg(test)]
thread_local! {
    static CAPTURED_HOURGLASS_PHASES: std::cell::RefCell<Option<Vec<HourglassPhase>>> =
        const { std::cell::RefCell::new(None) };
}

fn trace_hourglass_phase(phase: HourglassPhase) {
    tracing::trace!(
        target: "robin_engine::engine::tick::phases",
        ?phase,
        "perform_hourglass phase"
    );
    #[cfg(test)]
    CAPTURED_HOURGLASS_PHASES.with(|captured| {
        if let Some(phases) = captured.borrow_mut().as_mut() {
            phases.push(phase);
        }
    });
}

#[cfg(test)]
pub(super) fn begin_hourglass_phase_capture() {
    CAPTURED_HOURGLASS_PHASES.with(|captured| {
        let previous = captured.borrow_mut().replace(Vec::new());
        assert!(previous.is_none(), "hourglass phase capture already active");
    });
}

#[cfg(test)]
pub(super) fn end_hourglass_phase_capture() -> Vec<HourglassPhase> {
    CAPTURED_HOURGLASS_PHASES.with(|captured| {
        captured
            .borrow_mut()
            .take()
            .expect("hourglass phase capture was not active")
    })
}

#[cfg(test)]
thread_local! {
    static CAPTURED_ORDERED_GAMEPLAY_ENTITIES: std::cell::RefCell<Option<Vec<EntityId>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn capture_ordered_gameplay_entities<T>(f: impl FnOnce() -> T) -> (T, Vec<EntityId>) {
    CAPTURED_ORDERED_GAMEPLAY_ENTITIES.with(|captured| {
        assert!(
            captured.borrow_mut().replace(Vec::new()).is_none(),
            "ordered gameplay capture is not re-entrant"
        );
    });
    let result = f();
    let entities = CAPTURED_ORDERED_GAMEPLAY_ENTITIES.with(|captured| {
        captured
            .borrow_mut()
            .take()
            .expect("ordered gameplay capture must remain active")
    });
    (result, entities)
}

/// Move exclamations whose decoded-duration deadline has arrived into
/// the callback queue consumed as the first mutation of the next
/// `PerformHourglass` deferred-effects phase.
pub(super) fn drain_matured_exclamations(
    sound_sim: &mut crate::sound::SoundSimState,
    cur_frame: u32,
) {
    let mut still_playing = Vec::new();
    let mut finished = Vec::new();
    for p in sound_sim.playing_exclamations.drain(..) {
        if p.finish_frame <= cur_frame {
            finished.push((p.actor_id, p.exclamation_id));
        } else {
            still_playing.push(p);
        }
    }
    sound_sim.playing_exclamations = still_playing;
    sound_sim.finished_exclamations = finished;
}

#[derive(Default)]
struct HourglassStats {
    count: u32,
    total_us: u128,
    min_us: u128,
    max_us: u128,
}

impl HourglassStats {
    fn record(&mut self, us: u128) {
        if self.count == 0 {
            self.min_us = us;
            self.max_us = us;
        } else {
            self.min_us = self.min_us.min(us);
            self.max_us = self.max_us.max(us);
        }
        self.count += 1;
        self.total_us += us;
    }

    fn flush(&mut self) {
        if self.count == 0 {
            return;
        }
        let avg = self.total_us / self.count as u128;
        tracing::info!(
            target: "robin_engine::engine::tick::perf",
            count = self.count,
            avg_us = avg,
            min_us = self.min_us,
            max_us = self.max_us,
            "perform_hourglass timing"
        );
        *self = Self::default();
    }
}

/// RAII guard: timer.start() at construction, records on drop. Logs a
/// summary every `HOURGLASS_LOG_INTERVAL` ticks.
struct HourglassTimer {
    start: web_time::Instant,
}

impl HourglassTimer {
    fn start() -> Option<Self> {
        if !tracing::enabled!(target: "robin_engine::engine::tick::perf", tracing::Level::INFO) {
            return None;
        }
        Some(Self {
            start: web_time::Instant::now(),
        })
    }
}

impl Drop for HourglassTimer {
    fn drop(&mut self) {
        let us = self.start.elapsed().as_micros();
        HOURGLASS_STATS.with(|cell| {
            let mut s = cell.borrow_mut();
            s.record(us);
            if s.count >= HOURGLASS_LOG_INTERVAL {
                s.flush();
            }
        });
    }
}

impl EngineInner {
    // ─── Main update tick ────────────────────────────────────────

    /// The main per-frame logic update.
    ///
    /// Returns the game state code — normally `LevelInProgress`, but can
    /// return `LevelSucceeded`, `LevelFailed`, or `LevelInterrupted` to
    /// signal that the mission is over.
    ///
    /// Called once per frame from the game loop, gated by:
    /// - console not displayed
    /// - no UI transition in progress
    /// - not paused
    /// - not in LEVEL_NEXT or LEVEL_LOAD state
    ///
    /// Supplies [`EngineInner::perform_hourglass_inner`] with an explicit
    /// simulation context and drains the deferred sound queue so all
    /// gameplay-affecting randomness is pulled from the engine-owned stream
    /// (deterministic across clients) and all audio is
    /// flushed *after* the sim is done (letting rollback replay the tick
    /// without duplicating playback).
    pub fn perform_hourglass(
        &mut self,
        display: &mut HostDisplayState,
        assets: &LevelAssets,
        dev: &mut DevState,
    ) -> super::SideEffects {
        let _hourglass_timer = HourglassTimer::start();

        // RHScript::FadeToBlack presents its ramp in a tight loop without
        // calling PerformHourglass. Drain the corresponding presentation
        // count before lending the explicit simulation context or touching any simulation,
        // display-state, or sound timer. A frame-counter deadline cannot
        // represent this: advancing that clock would mature every deadline
        // that is supposed to remain frozen during the blocking native.
        if self.consume_fade_freeze_frame() {
            let mut fx = self.feedback.drain_side_effects();
            fx.code = GameCode::LevelInProgress;
            // Fast-forward render skipping must not strand the host fade.
            fx.skip_render = false;
            return fx;
        }

        let sim = self.control.simulation_context();
        let sim = &sim;

        let code = self.perform_hourglass_inner(sim, display, assets, dev);

        // Post-tick sim mutations that used to live in `game_session`
        // between the hourglass and the render pass. They have to run
        // inside `perform_hourglass` for rollback determinism: replay
        // only re-runs `perform_hourglass`, so anything advancing engine
        // state outside it would diverge from the live timeline.
        self.update_overall_villain_alert(&assets.profile_manager);
        display.minimap.tick_transition();
        // Advance the delayed-reveal highlight state machine.  Run it
        // once per hourglass (rather than from the draw loop) so
        // rollback replays the reveal timing deterministically.
        display.minimap.tick_highlights();
        // Advance per-PC QA macro-icon shift-fall phase so host
        // renderers can read via `macro_shift_phase` without mutating
        // engine state at draw time.
        display.tick_macro_shift_phases(&self.world.pc_ids, &self.players.macro_store);
        // Advance per-PC QA titbit fizzle-blink phase.  Host renderer
        // reads visibility via `macro_titbit_blink_hidden`.
        display.tick_macro_blink_phases(&self.world.pc_ids);
        // Advance destination-marker animation and retire finished
        // marks.  Used to run during rendering, which broke rollback
        // determinism — the render path is now read-only.
        {
            let view_pos = self.feedback.cutscene_camera.view_position;
            let zoom = self.feedback.cutscene_camera.zoom_factor;
            let screen = Self::director_camera_view_size();
            let screen_w = screen.x as i32;
            let screen_h = screen.y as i32;
            let frame_counter = self.control.frame_counter;
            self.feedback.ground_mark.tick(
                view_pos.to_geo(),
                zoom,
                screen_w,
                screen_h,
                frame_counter,
            );
        }
        // Sound-source delay state machine — fully sim-side now: engine
        // ticks the timer down, fires a `PlayDelayedSource` side-effect
        // when it hits zero, and re-rolls the next delay using
        // `sim_rng`. The host just consumes the command to kick off
        // audio playback. Previously the timer reset lived host-side
        // (driven by audio-backend completion + a host RNG), which
        // broke rollback determinism.
        let num_sources = self.feedback.sound_sim.sources.num_sources();
        for i in 0..num_sources {
            let Some(src) = self.feedback.sound_sim.sources.get_mut(i) else {
                continue;
            };
            if !src.active || src.source_kind != crate::sound_source::SoundSourceKind::Delayed {
                continue;
            }
            if src.timer > 0 {
                src.timer -= 1;
            }
            if src.timer == 0 {
                // Re-roll the next play delay before queueing the
                // play command — the per-source delay is always reset
                // immediately after a play decision.
                if src.delay_stepping > 0 && src.max_delay > src.min_delay {
                    let step = crate::sim_rng::u32(
                        sim,
                        crate::sim_rng::RngSite::DelayedSoundTimer,
                        0..src.delay_stepping as u32,
                    ) as u16;
                    let range = src.max_delay - src.min_delay;
                    src.timer = (step as u32 * range as u32 / src.delay_stepping as u32) as u16
                        + src.min_delay;
                } else {
                    src.timer = src.min_delay;
                }
                self.feedback
                    .pending_side_effects
                    .sounds
                    .push(super::SoundCommand::PlayDelayedSource(i));
            }
        }

        let skip_render = self.tick_camera_display_state();

        // Reset per-frame scroll dedupe after the camera display tick.
        // Host-local viewport scroll is host-side and never enters engine
        // state, so peer-2's held scroll doesn't gate the host's, and vice
        // versa.
        self.feedback.cutscene_camera.display.frame_scrolled = [false; 4];
        display.frame_scrolled = [false; 4];

        let mut fx = self.feedback.drain_side_effects();
        fx.code = code;
        // The trigger tick supplies the first FadeToBlack presentation.
        // Force that render even when the camera state machine requested a
        // fast-forward skip; the remaining presentations are forced by the
        // early-return path above.
        let starts_fade = matches!(fx.fade_to_black, Some(Some(_)));
        fx.skip_render = !starts_fade && skip_render != 0;
        fx
    }

    /// Run the one-shot mission-script `PostInitialize` stage.
    ///
    /// The original `RHGame::GameLoop` calls this after the first
    /// `Refresh(true, true)` and `RHSound::Hourglass`, not from inside
    /// `RHEngine::PerformHourglass`.  The host therefore invokes this
    /// explicit stage after its first refresh/sound boundary.  Rollback
    /// replay invokes the same stage after replaying frame zero so the
    /// resulting pre-frame-one simulation state remains deterministic.
    pub fn perform_post_initialize(
        &mut self,
        display: &mut HostDisplayState,
        assets: &LevelAssets,
    ) -> Option<super::SideEffects> {
        let needs_post_initialize = self
            .scripts
            .mission
            .as_ref()
            .is_some_and(|script| !script.post_initialized);
        if !needs_post_initialize {
            return None;
        }

        // PostInitialize can call randomising natives, so keep it on the same
        // engine-owned deterministic stream while moving only the scheduling
        // boundary.
        let sim = self.control.simulation_context();
        let sim = &sim;

        self.run_post_initialize_if_needed(sim, assets);
        self.drain_pending_immediate_actions_sync(sim, display, assets);

        let mut fx = self.feedback.drain_side_effects();
        fx.code = GameCode::LevelInProgress;
        Some(fx)
    }

    /// Whether any PC is currently guarded.
    pub fn is_pc_guarded(&self) -> bool {
        for &pc_id in &self.world.pc_ids {
            if let Some(Entity::Pc(pc)) = self.get_entity(pc_id)
                && pc.pc.guard.is_some()
            {
                return true;
            }
        }
        false
    }

    fn perform_hourglass_inner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        assets: &LevelAssets,
        dev: &mut DevState,
    ) -> GameCode {
        trace_hourglass_phase(HourglassPhase::DeferredEffectsStart);
        let pc_guarded = self.hourglass_phase_deferred_effects_start(sim, assets);

        trace_hourglass_phase(HourglassPhase::MissionAndMessages);
        if let Some(code) =
            self.hourglass_phase_mission_and_messages(sim, display, assets, dev, pc_guarded)
        {
            return code;
        }

        trace_hourglass_phase(HourglassPhase::NpcOrders);
        self.hourglass_phase_npc_orders(sim, assets);

        trace_hourglass_phase(HourglassPhase::Paths);
        self.hourglass_phase_paths(sim, assets);

        trace_hourglass_phase(HourglassPhase::Entities);
        let was_swordfighting = self.hourglass_phase_entities(sim, assets);

        trace_hourglass_phase(HourglassPhase::EntitySystems);
        let (positions_before_movement, processed_projectiles) =
            self.hourglass_phase_entity_systems(sim, assets);

        trace_hourglass_phase(HourglassPhase::Npcs);
        self.hourglass_phase_npcs(sim, assets, &positions_before_movement);

        trace_hourglass_phase(HourglassPhase::GameplaySystems);
        self.hourglass_phase_gameplay_systems(sim, display, assets, &processed_projectiles);

        trace_hourglass_phase(HourglassPhase::Sequences);
        self.hourglass_phase_sequences(sim, display, assets);

        // `RHSequenceElement::SetState(Terminated)` calls the owner's
        // `SendCondolationCard`, then `RHSequence::Ready`, synchronously
        // (`original-code/RHsequenceelement.cpp:280-296`). `Ready` immediately
        // starts the next command level (`RHsequence.cpp:304-313`), so an
        // immediate Timer successor must be installed before RHEngine reaches
        // its anonymous-timer scan at RHengine.cpp:3755-3771. Rust defers the
        // borrow-reentrant card itself, but this barrier must stay on the
        // sequence-manager side of that scan.
        self.dispatch_condolations(sim, assets);

        // `RHSequenceManager::Hourglass` runs before the anonymous-timer
        // scan. If a deferred command terminates and advances its sequence
        // to an immediate Timer, C++ executes that Timer re-entrantly, adds
        // it to `mlistTimerElements`, and decrements it later in this same
        // tick. Drain that immediate continuation here so Rust preserves the
        // same launch-frame decrement. Waiting until DeferredEffectsEnd's
        // final drain makes every such timer one frame late.
        self.drain_pending_immediate_actions_sync(sim, display, assets);

        trace_hourglass_phase(HourglassPhase::DeferredEffectsEnd);
        self.hourglass_phase_deferred_effects_end(sim, display, assets, was_swordfighting);

        GameCode::LevelInProgress
    }

    /// Drain effects deferred by the preceding tick before any mission,
    /// entity, path, NPC, or sequence work observes this frame's state.
    ///
    /// Original provenance: `original-code/RHengine.cpp:3446-3548` starts
    /// `RHEngine::PerformHourglass` with host/widget and mission-state work.
    /// These Rust-owned queues have no one-to-one original equivalent; their
    /// relative placement is retained from the pre-decomposition Rust tick.
    pub(super) fn hourglass_phase_deferred_effects_start(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) -> bool {
        // Sound Hourglass completes after the preceding Engine frame in the
        // Original. Its callbacks therefore finish before the next
        // PerformHourglass begins and must be the first mutation here.
        let cur_frame = self.control.frame_counter;
        drain_matured_exclamations(&mut self.feedback.sound_sim, cur_frame);
        self.settle_npc_speech_completions(sim, assets);

        // Drain deferred console-cheat / death reinforcement spawns and
        // scroll-reveal amulet spawns. Both used to live in
        // `Game::run_engine_tick` because they needed `&mut LevelAssets`
        // to load sprites; the two sprite families are now preloaded at
        // mission start (`preload_campaign_peasant_sprites`,
        // `preload_scroll_amulet_sprite`) so the spawn paths read the
        // scriptor cache via `&LevelAssets` and the whole flow lives
        // inside `perform_hourglass` — keeping the "sim mutation only
        // during perform_hourglass" invariant intact.
        self.drain_pending_reinforcements(sim, assets);
        self.drain_pending_scroll_amulets(sim, assets);
        self.drain_pending_hero_speeches(assets);
        self.drain_pending_hades_kills(sim, assets);
        self.drain_pending_concussion_side_effects(sim, assets);

        // Drain matured sound-source finishes.  Replaces the
        // `stop_sound_source` logic the Rust host used to run on
        // Audio-backend playback-completion events: for each scheduled
        // source whose sim-frame deadline has arrived, `Single` sources
        // flip to `active = false` and `Volatile` sources are deleted
        // from the manager.  `Delayed` / `Looped` never land in
        // `playing_sources` (Delayed re-rolls itself below; Looped
        // doesn't terminate on its own), so this drain only ever sees
        // Single/Volatile; still match exhaustively to fail loudly if
        // a kind ever leaks into the queue.
        let mut still_playing_sources = Vec::new();
        let mut source_deactivations: Vec<usize> = Vec::new();
        let mut source_deletions: Vec<usize> = Vec::new();
        for p in self.feedback.sound_sim.playing_sources.drain(..) {
            if p.finish_frame > cur_frame {
                still_playing_sources.push(p);
                continue;
            }
            let Some(src) = self.feedback.sound_sim.sources.get(p.source_index as usize) else {
                // Slot already cleared (e.g. Destroy command ran this
                // tick); drop the stale entry silently.
                continue;
            };
            match src.source_kind {
                crate::sound_source::SoundSourceKind::Single => {
                    source_deactivations.push(p.source_index as usize);
                }
                crate::sound_source::SoundSourceKind::Volatile => {
                    source_deletions.push(p.source_index as usize);
                }
                crate::sound_source::SoundSourceKind::Looped
                | crate::sound_source::SoundSourceKind::Delayed => {
                    tracing::warn!(
                        source_index = p.source_index,
                        kind = ?src.source_kind,
                        "sound source scheduled finish fired for Looped/Delayed kind — \
                         should never happen (schedule_source_finish skips them)"
                    );
                }
            }
        }
        self.feedback.sound_sim.playing_sources = still_playing_sources;
        for idx in source_deactivations {
            if let Some(src) = self.feedback.sound_sim.sources.get_mut(idx) {
                src.active = false;
            }
        }
        for idx in source_deletions {
            self.feedback.sound_sim.sources.delete(idx);
        }

        // PC-guarded state drives start/quit mission widget enable and
        // guard-portrait blinking.  The
        // widget-enable side is applied from `Game::run_engine_tick`
        // before `perform_hourglass` runs so both consumers see the
        // same value for this tick.  The guard-portrait blink is
        // rendered live by `ui_panel.rs` directly from
        // `mission.mission_won` + `PcData::guard`, so there's nothing
        // to do here for (b).

        self.is_pc_guarded()
    }

    /// Run mission gates, the once-per-second script, clock advancement, and
    /// the tick's messenger drain. Returning a code short-circuits every later
    /// phase exactly where the monolithic implementation did.
    ///
    /// Original provenance: `original-code/RHengine.cpp:3470-3664` performs
    /// mission/UI gates, script callbacks, counter advancement, lock checks,
    /// loss checks, and reinforcement notification in this order.
    fn hourglass_phase_mission_and_messages(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        assets: &LevelAssets,
        dev: &mut DevState,
        pc_guarded: bool,
    ) -> Option<GameCode> {
        // ── Projectile cheat rain ────────────────────────────────
        // The original `ProjectileRain` cheat was wired up but never
        // implemented in the shipped build.  Preserve the drain so the
        // dev flag resets, but don't invent gameplay that never existed.
        if dev.projectile_cheat_rain >= 0 {
            dev.projectile_cheat_rain = -1;
        }

        // ── Anti-chorus timer ────────────────────────────────────
        if self.control.chorus_timer > 0 {
            self.control.chorus_timer -= 1;
        }

        // ── First-time mission-won message ───────────────────────
        // Fire the mission-state banner ("leave mission now" / quit
        // mission popup) and disable the quit-mission widget once the
        // player has reached a guarded exit AND no PC is currently
        // being guarded (guarded PCs can't lead everyone out yet).
        // We signal both via `SideEffects.pending_mission_state_notice`;
        // the host flips the widget-enable flag and shows the popup.
        if self.mission_domain.state.mission_won_first_time && !pc_guarded {
            self.mission_domain.state.mission_won_first_time = false;
            self.feedback
                .pending_side_effects
                .pending_mission_state_notice = true;
        }

        // ── Check quit conditions ────────────────────────────────
        // Each of the three quit branches displays the full minimap.
        if self.mission_domain.state.quit_won {
            display.minimap.display_map(false, true);
            self.finalize_mission_script(sim, assets, false);
            return Some(GameCode::LevelSucceeded);
        }
        if self.mission_domain.state.quit_lost {
            display.minimap.display_map(false, true);
            self.quit_mission();
            return Some(GameCode::LevelFailed);
        }
        if self.mission_domain.state.quit_interrupted {
            display.minimap.display_map(false, true);
            self.finalize_mission_script(sim, assets, true);
            return Some(GameCode::LevelInterrupted);
        }

        // ── Cheat display all dialogs/briefings ──────────────────
        // After the engine/host carve-out (Decision 9) level descriptors
        // live host-side.  `all_dialogues`, `all_popup_texts` and
        // `all_debriefings` are expanded by `game_session` after the
        // tick returns — it has the descriptor on hand and pushes every
        // registered ID straight onto the host-side pending queues.

        // ── Script tick (once per game-second) ──────────────────────
        // The main loop runs at 25 Hz (40 ms frame time), and the
        // script's Hourglass fires only when
        // `frame_counter % 25 == 0` — i.e. once per real second — with
        // the game-second index as its argument.
        if self.control.frame_counter.is_multiple_of(FRAMES_PER_SECOND) {
            let game_seconds = self.control.frame_counter / FRAMES_PER_SECOND;

            if let Err(error) = self.call_script_vm(
                sim,
                assets,
                super::ScriptVmKey::Global,
                "Hourglass",
                &[game_seconds as i32],
                crate::natives::ScriptCallFrame::default(),
            ) {
                tracing::warn!("Script Hourglass error: {error}");
            }

            // Check victory/defeat conditions every 3 game-seconds
            // (or immediately if force_check was set by a native call).
            if game_seconds.is_multiple_of(VICTORY_CHECK_INTERVAL)
                || self.script_domains.mission_ui.force_check
            {
                self.script_domains.mission_ui.force_check = false;

                {
                    let victory_result = self.call_script_vm(
                        sim,
                        assets,
                        super::ScriptVmKey::Global,
                        "CheckVictoryCondition",
                        &[game_seconds as i32],
                        crate::natives::ScriptCallFrame::default(),
                    );
                    match victory_result {
                        Ok(1) => {
                            // Mission won!
                            if !self.mission_domain.state.mission_won {
                                // Don't show the "leave mission" message for
                                // ambush or tactical missions (they end immediately).
                                let show_window = !matches!(
                                    self.mission_type(&assets.profile_manager),
                                    Some(MissionType::Ambush | MissionType::Tactical)
                                );
                                self.win(show_window);
                            }
                        }
                        Ok(2) => {
                            // Script says mission lost
                            self.quit_mission();
                            return Some(GameCode::LevelFailed);
                        }
                        Ok(_) => {} // 0 or other = still in progress
                        Err(e) => {
                            tracing::warn!("Script CheckVictoryCondition error: {e}");
                        }
                    }
                }
            }
        }

        // ── Increment frame counter ──────────────────────────────
        self.advance_mission_clock();

        // ── Skip logic if engine is locked (zoom, sequence, etc) ─
        if self
            .feedback
            .cutscene_camera
            .display
            .background_transform
            .zoom_to_up
            || self
                .feedback
                .cutscene_camera
                .display
                .background_transform
                .zoom_to_down
            || self.engine_locked()
        {
            return Some(GameCode::LevelInProgress);
        }

        // ── Default lose condition check ─────────────────────────
        // Guarded by `ignore_default_loose`.
        // Missions that keep-an-NPC-alive (e.g. "protect the cart")
        // set this flag to true so the default "all PCs dead/guarded /
        // dead-PC / civilian-killed" loss checks are skipped; the
        // script's `CheckVictoryCondition` is the authority instead.
        let ignore_default_loose = self.control.sim_config.ignore_default_loose;
        if !ignore_default_loose {
            // Original: RHEngine::PerformHourglass checks the PC's explicit
            // IsPlayable() flag and guard state. Death paths are responsible
            // for clearing playability; do not substitute an HP/posture test.
            if !self.world.pc_ids.is_empty() {
                let any_playable_and_free = self.world.pc_ids.iter().any(|&pc_id| {
                    if let Some(Entity::Pc(pc)) = self.world.entities.get(pc_id) {
                        let guarded = pc.pc.guard.is_some();
                        pc.pc.playable && !guarded
                    } else {
                        false
                    }
                });
                if !any_playable_and_free {
                    tracing::info!("No playable, unguarded PC remains; mission lost");
                    self.quit_mission();
                    return Some(GameCode::LevelFailed);
                }
            }

            // Check if a dead PC was flagged for mission failure
            if let Some(dead_id) = self.mission_domain.dead_pc.take() {
                if let Some(entity) = self.get_entity(dead_id) {
                    let pos = entity.element_data().position_map();
                    self.center_on_point(0, pos);
                }
                self.quit_mission();
                return Some(GameCode::LevelFailed);
            }

            // Check if any civilian was killed (not by accident) → mission failure
            let mut killed_civilian = None;
            for (npc_id, civilian) in self.world.entities.civilians() {
                if civilian.element.posture.is_dead() {
                    let npc_id: EntityId = npc_id.into();
                    // Check killed_by_accident via the civilian's human data
                    let accident = civilian.human.killed_by_accident;
                    if !accident {
                        killed_civilian = Some(npc_id);
                        break;
                    }
                }
            }
            if let Some(civ_id) = killed_civilian {
                if let Some(entity) = self.get_entity(civ_id) {
                    let pos = entity.element_data().position_map();
                    self.center_on_point(0, pos);
                }
                self.quit_mission();
                return Some(GameCode::LevelFailed);
            }
        }

        // ── Send reinforcement messages ──────────────────────────
        //
        // For every PC, decrement `time_till_reinforcement` and, the
        // tick it hits zero, enqueue a reinforcement spawn directly
        // (skipping the messenger round-trip the original used).
        // `drain_pending_reinforcements` already handles the
        // `&mut LevelAssets` needed for sprite loading, and the
        // intermediate message was never observed by anything else.
        let pc_ids_for_reinf: Vec<EntityId> = self.world.pc_ids.clone();
        for pc_id in pc_ids_for_reinf {
            let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id) else {
                continue;
            };
            let arrived = match pc.pc.time_till_reinforcement {
                0xFFFF_FFFF => false,
                0 => {
                    pc.pc.time_till_reinforcement = 0xFFFF_FFFF;
                    true
                }
                ref mut t => {
                    *t -= 1;
                    false
                }
            };
            if arrived {
                self.orders.pending_reinforcements.push(Some(pc_id));
            }
        }

        // ── Process messenger (engine-state messages) ────────────
        // Handle pending messages that mutate engine state. Other
        // messages (UI/mission flow) are left in the queue for their
        // respective consumers (UI layer, tests, etc.) to observe.
        // We only consume the ones that actually affect engine state.
        {
            // `RHMessenger::ForwardMessage` is synchronous and recursive:
            // a message emitted while handling another message completes
            // before the outer call resumes.  Keep host/UI-only messages for
            // their downstream consumer, but prepend newly emitted messages
            // to the remaining engine work so their observable state changes
            // happen depth-first in this frame.
            let mut messages: std::collections::VecDeque<_> = self.orders.messenger.drain().into();
            let mut downstream = std::collections::VecDeque::new();
            while let Some(msg) = messages.pop_front() {
                match msg.msg_type {
                    MessageType::Simple(SimpleMessage::LockAlt) => {
                        self.players.seats[0].is_lock_alt = true;
                    }
                    MessageType::Simple(SimpleMessage::UnlockAlt) => {
                        self.players.seats[0].is_lock_alt = false;
                    }
                    // Macro recording state machine.  The PC id is
                    // passed via the message: a present id targets one
                    // specific PC; an absent id arms every currently-
                    // selected PC.
                    MessageType::Pc(crate::messenger::PcMessage::StartRecordingMacro, pc) => {
                        let slot = self.players.qa_recording_slot;
                        let targets: Vec<crate::element::EntityId> = match pc {
                            Some(id) => vec![id],
                            None => self.players.seats[0].selection.clone(),
                        };
                        for pc_id in &targets {
                            self.players
                                .macro_store
                                .get_or_insert(*pc_id)
                                .begin_recording(slot);
                        }
                        self.players.qa_recording_for = targets;
                        // Snapshot the currently-armed action so the
                        // MSG_STOP_RECORDING_MACRO post-process can
                        // restore it.
                        self.players.action_before_recording_macro = self.get_selected_action();
                    }
                    MessageType::Pc(crate::messenger::PcMessage::StopRecordingMacro, _) => {
                        // Suppress the post-process restore unless
                        // something was actually recording.
                        let was_recording = !self.players.qa_recording_for.is_empty();
                        for pc_id in self.players.qa_recording_for.clone() {
                            if let Some(state) = self.players.macro_store.get_mut(pc_id) {
                                state.stop_recording();
                            }
                        }
                        self.players.qa_recording_for.clear();

                        // Post-process: re-select the action that was
                        // armed before recording started.  Apply the
                        // saved action to each selected PC directly —
                        // we do not route MSG_SELECT_ACTION through
                        // the messenger drain.
                        if was_recording {
                            let restore = self.players.action_before_recording_macro;
                            self.players.action_before_recording_macro =
                                crate::profiles::Action::NoAction;
                            for id in self.players.seats[0].selection.clone() {
                                if let Some(entity) = self.get_entity_mut(id)
                                    && let Some(pc) = entity.pc_data_mut()
                                {
                                    pc.current_action = restore;
                                }
                            }
                            // Emit the message for script /
                            // edge-subscriber observation.
                            self.orders
                                .messenger
                                .send(crate::messenger::Message::pc_with_value(
                                    crate::messenger::PcMessage::SelectAction,
                                    None,
                                    restore as u32,
                                ));
                        }
                    }
                    MessageType::Pc(crate::messenger::PcMessage::UpdateRecordingMacro, _) => {
                        // When a recording is live, end it on PCs no
                        // longer selected and start it on any newly-
                        // selected PC — keeping the slot index stable
                        // across selection changes.
                        if !self.players.qa_recording_for.is_empty() {
                            let slot = self.players.qa_recording_slot;
                            let selected: Vec<crate::element::EntityId> =
                                self.players.seats[0].selection.clone();
                            // End on PCs that left the selection.
                            let current = self.players.qa_recording_for.clone();
                            for pc_id in &current {
                                if !selected.contains(pc_id)
                                    && let Some(state) = self.players.macro_store.get_mut(*pc_id)
                                {
                                    state.stop_recording();
                                }
                            }
                            // Start on PCs newly selected.
                            for pc_id in &selected {
                                if !current.contains(pc_id) {
                                    self.players
                                        .macro_store
                                        .get_or_insert(*pc_id)
                                        .begin_recording(slot);
                                }
                            }
                            self.players.qa_recording_for = selected;
                        }
                    }
                    MessageType::Pc(crate::messenger::PcMessage::SendReinforcement, pc) => {
                        // `MSG_SEND_REINFORCEMENT` plays the "new peasant
                        // called" jingle and sets the PC's cooldown to
                        // 100 ticks.  The cooldown poll in the tick
                        // above spawns the replacement when the counter
                        // hits zero.
                        if let Some(pc_id) = pc
                            && let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id)
                        {
                            pc.pc.time_till_reinforcement = 100;
                        }
                        self.feedback.pending_side_effects.sounds.push(
                            super::SoundCommand::Jingle(crate::sound::Jingle::NewPeasantCalled),
                        );
                    }
                    // PC-info hover popup is HQ-only (Sherwood) — go
                    // through `request_pc_info_overlay` so that gate
                    // is honored.
                    //
                    // UI-has-focus: another UI widget grabbed input
                    // focus — hide any live PC-info hover popup.
                    // Emitted from the minimap drag handler
                    // (commands.rs) and should be emitted from any
                    // future in-game widget that grabs focus.
                    //
                    // The Rust port keeps the mouse focus gate on
                    // host-owned `InputState`; `run_engine_tick_core`
                    // consumes the side effect below and clears that
                    // latch before later mouse dispatch can see it.
                    MessageType::Simple(crate::messenger::SimpleMessage::UiHasFocus) => {
                        self.request_pc_info_overlay(assets, None);
                        // Raise the host-side per-frame `ui_focus`
                        // latch; the host clears it at end of
                        // `update_mouse`.
                        self.feedback.pending_side_effects.ui_has_focus = true;
                    }
                    MessageType::Pc(crate::messenger::PcMessage::ShowPcInformation, pc) => {
                        self.request_pc_info_overlay(assets, pc);
                    }
                    MessageType::Pc(crate::messenger::PcMessage::HidePcInformation, _) => {
                        self.request_pc_info_overlay(assets, None);
                    }
                    // The four `SelectCharacter[Add][WithEcho]` arms
                    // all route through `select_pc` with the
                    // appropriate (multi-select, speak) flags.
                    MessageType::Pc(crate::messenger::PcMessage::SelectCharacter, Some(pc_id)) => {
                        // Tick messenger drains: ambient single-seat
                        // semantics; LOCAL seat for now.
                        self.select_pc(assets, 0, pc_id, false, false);
                        self.emit_character_selection_followups();
                    }
                    MessageType::Pc(
                        crate::messenger::PcMessage::SelectCharacterWithEcho,
                        Some(pc_id),
                    ) => {
                        self.select_pc(assets, 0, pc_id, false, true);
                        self.emit_character_selection_followups();
                    }
                    MessageType::Pc(
                        crate::messenger::PcMessage::SelectAddCharacter,
                        Some(pc_id),
                    ) => {
                        self.select_pc(assets, 0, pc_id, true, false);
                        self.emit_character_selection_followups();
                    }
                    MessageType::Pc(
                        crate::messenger::PcMessage::SelectAddCharacterWithEcho,
                        Some(pc_id),
                    ) => {
                        self.select_pc(assets, 0, pc_id, true, true);
                        self.emit_character_selection_followups();
                    }
                    // `pc == None` drops the whole selection;
                    // otherwise remove the specific PC.  Producers:
                    // `tick.rs:L4279` (dying / KO'd PC), `LockUser`,
                    // `DisableCharacter` (below).
                    MessageType::Pc(crate::messenger::PcMessage::UnselectCharacter, pc) => {
                        // Sherwood-only: on `pc == None`, mark every
                        // PC's interface hidden; otherwise hide just
                        // that PC's.  Engine side clears the selection
                        // list separately.
                        if self.is_sherwood(&assets.profile_manager) {
                            match pc {
                                None => {
                                    let ids = self.world.pc_ids.clone();
                                    for id in ids {
                                        if let Some(crate::element::Entity::Pc(pc)) =
                                            self.get_entity_mut(id)
                                        {
                                            pc.pc.interface_hidden = true;
                                        }
                                    }
                                }
                                Some(pc_id) => {
                                    if let Some(crate::element::Entity::Pc(pc)) =
                                        self.get_entity_mut(pc_id)
                                    {
                                        pc.pc.interface_hidden = true;
                                    }
                                }
                            }
                        }
                        match pc {
                            None => self.unselect_all_pcs(0),
                            Some(pc_id) => self.unselect_single_pc(pc_id),
                        }
                        self.emit_character_selection_followups();
                    }
                    // The engine drops the PC from the selection and
                    // (outside Sherwood) removes the portrait.  The
                    // portrait strip in Rust immediate-mode renders
                    // from `pc_ids` filtered by `pc.playable`, so the
                    // "portrait disappears" side effect is covered by
                    // the native already writing `pc.playable = false`
                    // at `natives/mod.rs:1546`.  Here we only need the
                    // selection-drop plus the Sherwood interface flag.
                    MessageType::Pc(crate::messenger::PcMessage::DisableCharacter, pc) => {
                        if let Some(pc_id) = pc {
                            self.unselect_single_pc(pc_id);
                            // Net effect: flip the interface-hidden
                            // flag only when we are NOT in Sherwood.
                            // Previously the gate was inverted; the
                            // effect was masked because
                            // `interface_hidden` is not read by the
                            // HUD path, but parity still matters for
                            // the `STATUS PC` cheat and future HUD
                            // wiring.
                            if !self.is_sherwood(&assets.profile_manager)
                                && let Some(crate::element::Entity::Pc(pc)) =
                                    self.get_entity_mut(pc_id)
                            {
                                pc.pc.interface_hidden = true;
                            }
                        }
                    }
                    // The portrait widget is re-added only outside
                    // Sherwood.  In Rust, the live HUD reads
                    // `pc.interface_hidden`; clear it whenever the
                    // portrait would have been re-added.  Sherwood
                    // also gets the same clear so the HUD panel
                    // re-shows the PC when re-activated mid-Sherwood.
                    MessageType::Pc(crate::messenger::PcMessage::EnableCharacter, pc) => {
                        if let Some(pc_id) = pc
                            && let Some(crate::element::Entity::Pc(pc)) = self.get_entity_mut(pc_id)
                        {
                            pc.pc.interface_hidden = false;
                        }
                    }
                    // After a modal (dialogue, popup, Sherwood report)
                    // closes, zero the cached mouse/keyboard state,
                    // clear the rubber-band selection and
                    // pending-drag / click suppression flags, and drop
                    // pressed-key edges queued during the modal.  The
                    // Rust equivalents live host-side across two
                    // InputState groups: ThreadedInput pressed-key
                    // cache (`pending_reset_input`) and the
                    // rubber-band / click-suppression flags
                    // (`reset_input`).
                    MessageType::Simple(crate::messenger::SimpleMessage::ResetInput) => {
                        self.feedback.pending_side_effects.pending_reset_input = true;
                        self.feedback.pending_side_effects.reset_input = true;
                        // Clear the alt-lock latch along with the
                        // modifier cache; without this, an alt-lock
                        // toggled before a console-hide / task-switch
                        // / save-load / unlock-user would persist
                        // past the reset.
                        self.players.seats[0].is_lock_alt = false;
                    }
                    // Ctrl-press saves the current action on every
                    // selected PC so the follow-on move command can
                    // run without the action overriding it (and the
                    // action is restored on ctrl-release).  Emitted
                    // by the host input layer when
                    // `GameAction::KeyControl` fires.
                    MessageType::Simple(crate::messenger::SimpleMessage::KeyControl) => {
                        self.save_action_for_selected_pcs(0);
                    }
                    // `LockUser` / `UnlockUser` flip `user_locked`.
                    // Scripts already set it directly via
                    // `Command::LockUser` (see tick.rs sequence-manager
                    // handler), but wiring the messenger variants
                    // keeps any non-script producer in sync with the
                    // engine-side flag that gates mouse events in
                    // `handle_mouse_input`.  Unlock also raises the
                    // `pending_reset_input` side-effect so held-key
                    // edges from the locked period are dropped.
                    MessageType::Simple(crate::messenger::SimpleMessage::LockUser) => {
                        self.players.user_locked = true;
                    }
                    MessageType::Simple(crate::messenger::SimpleMessage::UnlockUser) => {
                        self.players.user_locked = false;
                        self.feedback.pending_side_effects.pending_reset_input = true;
                    }
                    // After hiding the console or switching task,
                    // emit `MSG_RESET_INPUT` so the held-key edges
                    // and modifier latches don't bleed across the
                    // task boundary.
                    MessageType::Simple(crate::messenger::SimpleMessage::HideConsole)
                    | MessageType::Simple(crate::messenger::SimpleMessage::SwitchTask) => {
                        self.feedback.pending_side_effects.pending_reset_input = true;
                        self.feedback.pending_side_effects.reset_input = true;
                        // Same `is_lock_alt` clear as the explicit
                        // `ResetInput` arm above.
                        self.players.seats[0].is_lock_alt = false;
                    }
                    // `SelectActionSimple` and `DisableAction` both
                    // clear the aim-trajectory preview so a dropped /
                    // replaced action doesn't leave a stale trajectory
                    // overlay on screen.  `valid_trajectory` lives on
                    // `host` in the Rust split, so raise the
                    // side-effect flag.
                    MessageType::Pc(crate::messenger::PcMessage::SelectActionSimple, _)
                    | MessageType::Pc(crate::messenger::PcMessage::DisableAction, _) => {
                        self.feedback
                            .pending_side_effects
                            .invalidate_trajectory_preview = true;
                    }
                    // A macro fizzled on a PC's QA slot, so arm the
                    // per-slot titbit blink strobe.  Typed `pc` slot
                    // carries the PC id; `msg.value` is the QA slot
                    // index.  A `None` PC is treated as a no-op with
                    // a warning (the producer must always set one).
                    MessageType::Pc(crate::messenger::PcMessage::FizzleMacro, pc) => {
                        let slot = msg.value as usize;
                        match pc {
                            None => tracing::warn!(
                                "FizzleMacro received with no PC; \
                                 producer must set the PC id"
                            ),
                            Some(pc_id) => {
                                display.blink_qa(pc_id, slot);
                            }
                        }
                    }
                    // `QaFocus` flashes the macro titbit for the
                    // focused QA slot.  Typed `pc` slot carries the
                    // PC (None = all PCs); `msg.value` encodes the
                    // slot index.
                    MessageType::Pc(crate::messenger::PcMessage::QaFocus, pc) => {
                        let slot = msg.value as usize;
                        match pc {
                            None => {
                                let pc_ids = self.world.pc_ids.clone();
                                for pc_id in pc_ids {
                                    self.set_blinking_for_slot(pc_id, slot);
                                }
                            }
                            Some(pc_id) => self.set_blinking_for_slot(pc_id, slot),
                        }
                    }
                    // Bulk-flip `disabled_actions_temp` on a specific
                    // PC (`Some(pc_id)`) or every selected PC
                    // (`None`).
                    MessageType::Pc(crate::messenger::PcMessage::DisableAllActionsTemp, pc) => {
                        // Tick messenger drain: ambient single-seat
                        // semantics; LOCAL seat for now.
                        self.apply_disable_all_actions_temp(0, pc, true);
                    }
                    MessageType::Pc(crate::messenger::PcMessage::EnableAllActionsTemp, pc) => {
                        self.apply_disable_all_actions_temp(0, pc, false);
                    }
                    // Other messages are consumed by downstream systems
                    // (UI layer, mission flow). Re-enqueue so those
                    // consumers can still observe them.
                    _ => downstream.push_back(msg),
                }

                // Preserve the send order of recursive calls while placing
                // them ahead of pre-existing sibling messages.
                for nested in self.orders.messenger.drain().into_iter().rev() {
                    messages.push_front(nested);
                }
            }
            for msg in downstream {
                self.orders.messenger.send(msg);
            }
        }

        None
    }

    /// Promote queued NPC intents before entity refresh and sequence dispatch.
    ///
    /// Original provenance: NPC AI was primarily reached through each NPC's
    /// `RHElement::Hourglass` in the original entity loop
    /// (`original-code/RHengine.cpp:3715-3723`). The Rust pre-pass is an
    /// architectural split; its exact parity remains audited separately.
    fn hourglass_phase_npc_orders(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        // ── Sequence manager cleanup ─────────────────────────────
        // Run every 256 frames (or every frame in debug).
        if self.control.frame_counter.is_multiple_of(256) {
            self.orders.sequence_manager.friday_evening_cleanup();
        }

        // ── Process pending AI orders ─────────────────────────────
        //
        // AI Move intents collected by `launch_pending_orders_for_npc`
        // route through `launch_ai_move`, which just enqueues into
        // `pending_move_requests` (dedup-per-actor).  The drain below
        // promotes one Move sequence element per unique actor this
        // tick — absorbing redundant re-fires that would otherwise
        // launch a fresh Move each frame and `InterruptCurrent` the
        // in-flight one. A*-requiring elements enter the frame-paced
        // path-request queue advanced by the following `Paths` phase.
        self.process_pending_ai_orders();
        self.drain_pending_move_requests();

        // ── Dispatch per-waypoint ReachPoint scripts ─────────────
        // When the AI reaches a scripted waypoint it queues the
        // dispatch on `pending_waypoint_script_reach_point`; we drain
        // the queue here, call `ReachPoint(actor)` on the waypoint's
        // VM, and push `EventAfterScriptGoOn` as a self-stimulus
        // unless the script pulled the NPC into `DefaultScriptDriven`.
        // Runs before `process_pending_cross_npc_actions` so the
        // self-stimulus drain at the end of that pass picks up the
        // `EventAfterScriptGoOn` in the same tick.
        self.dispatch_pending_waypoint_scripts(sim, assets);

        // ── Process cross-NPC actions (phalanx coordination) ────
        self.process_pending_cross_npc_actions(sim, assets);

        // ── Process NPC turn orders ──────────────────────────────
        // Turning orders (from face_direction / face_position) are queued
        // by process_pending_ai_orders into actor.order_queue. Process
        // them here: set entity direction and dispatch EventDone back to
        // the AI so the state machine can advance (e.g. from
        // DefaultGotoRouteTurn → DefaultEnroute).
        // Turn: instant turn → SendCondolationCard(EventDone).
        self.process_turn_orders();

        // ── Process AI animation orders ─────────────────────────
        // Drain Pointing/RaisingShield/etc orders from NPC order queues
        // and start them as active_ai_anim. EventDone fires when the
        // sprite animation completes (detected in tick_actor_animation_for).
        self.process_animation_orders();

        // TODO(original-parity): determine which queued NPC-order effects must
        // remain inside an individual NPC's creation-ordered Hourglass call.
    }

    /// Refresh every entity in stable entity-table (creation) order.
    ///
    /// Original provenance: `original-code/RHengine.cpp:3715-3723` iterates
    /// `marrayElements`, which `SortForEngine` orders by creation order at
    /// `original-code/RHengine.cpp:7909-7944`, and removes dead elements inline.
    fn hourglass_phase_entities(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) -> bool {
        // Snapshot pre-hourglass swordfight state so we can detect a
        // swordfight→non-swordfight transition across this tick and
        // raise the ignore-mouse-event bracket on the falling edge.
        // The per-element / sequence-manager hourglass passes below may
        // flip the selected PC out of `Swordfighting`; when that
        // happens mid-drag the in-flight drag must be suppressed so it
        // doesn't bleed into the next click-release action.
        let was_swordfighting = self.is_selected_pc_swordfighting();

        // RHElementActorSoldier::Hourglass performs its subclass prelude
        // before delegating to RHElementActorNPC::Hourglass: apple smell,
        // primary-target tracking, and the reaction-time EnemyNear test.
        // In particular, keep the target snap introduced by 24c43efde ahead
        // of RefreshView without moving it into the base NPC phases.
        #[cfg(test)]
        observe_npc_hourglass_phase(NpcHourglassPhase::SoldierPrelude);
        #[cfg(not(test))]
        observe_npc_hourglass_phase(());
        // Work runs at each soldier's live owner slot below.

        // First base-NPC phase in RHElementActorNPC::Hourglass. Patrol
        // history observes the actor before RHElementActorHuman::Hourglass
        // executes its movement/order work.
        #[cfg(test)]
        observe_npc_hourglass_phase(NpcHourglassPhase::Patrol);
        #[cfg(not(test))]
        observe_npc_hourglass_phase(());
        // Work runs before the Human/Actor slices of each NPC owner below.

        // ── Element hourglass (per-element update) ───────────────
        #[cfg(test)]
        observe_npc_hourglass_phase(NpcHourglassPhase::BaseHuman);
        #[cfg(not(test))]
        observe_npc_hourglass_phase(());
        // Human concussion healing runs synchronously in each owner's
        // pre-Actor hook below.
        let mut to_remove = Vec::new();
        for (id, entity) in self.world.entities.occupied_mut() {
            if !entity.hourglass() {
                to_remove.push(id);
            }
        }
        for id in to_remove {
            self.remove_entity(id);
        }

        // ── PC selection outline fade ────────────────────────────
        // The hulk state-machine block runs during the per-element
        // refresh pass.
        self.refresh_pc_selection_hulk();

        // Tick the cheat-teleport hulk-rebuild fade counter on every
        // PC.  Decrementing here (rather than from the per-PC render
        // path) lets rollback / replay see bit-identical state (the
        // counter is serde'd `PcData`).
        self.tick_pc_teleport_fades();

        // `RefreshSeek` is actor-Hourglass behavior in the
        // original, not part of the engine's ProcessPathRequests pre-pass.
        // Seek refresh dispatch is in
        // `original-code/RHelementactor.cpp:2720-2728`. WAIT_TIMER now runs
        // after Execute in the live actor-slot coordinator below.
        self.tick_refresh_seeks(sim, assets);

        was_swordfighting
    }

    /// Advance queued pathfinding and failed-path deadlines before any entity
    /// refresh observes their state.
    ///
    /// Original provenance: `original-code/RHengine.cpp:3697-3702` calls
    /// `ProcessPathRequests` once before collision and entity hourglasses;
    /// `original-code/RHpathfinder.cpp:710-765` returns at most one completed
    /// request and begins at most one successor at that scheduling point.
    fn hourglass_phase_paths(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        // Rust computes A* synchronously, but the queue retains the original
        // one-call latency and one-completion-per-frame observation order.
        let completed = MovementContext::new(
            self.control.frame_counter,
            &mut self.world,
            &mut self.orders,
        )
        .take_completed();
        match completed {
            Some(CompletedPathWork::Ready { request, waypoints }) => {
                if let Some(element) = self
                    .orders
                    .sequence_manager
                    .get_element_mut(request.seq_id, request.elem_idx)
                {
                    element.command = crate::element::Command::MoveOk;
                }
                let _ = self.finish_move_path(sim, request, waypoints);
            }
            Some(CompletedPathWork::Failed(request)) => {
                tracing::warn!(
                    actor = ?request.owner,
                    seq_id = ?request.seq_id,
                    elem_idx = request.elem_idx,
                    src_x = request.source.x,
                    src_y = request.source.y,
                    dst_x = request.dest.x,
                    dst_y = request.dest.y,
                    layer = request.layer,
                    sector = request.sector,
                    "path scheduling barrier: pathfind FAILED",
                );
                self.orders
                    .failed_path_requests
                    .push(super::movement::FailedPathRequest {
                        owner: request.owner,
                        seq_id: request.seq_id,
                        elem_idx: request.elem_idx,
                        first_fail_frame: self.control.frame_counter,
                    });
            }
            None => {}
        }

        MovementContext::new(
            self.control.frame_counter,
            &mut self.world,
            &mut self.orders,
        )
        .start_next(assets);

        // ── Failed-path retry ────────────────────────────────────
        // Move / Seek elements whose pathfind failed on a previous
        // tick stay in `InProgress` with empty orders for up to 100
        // frames while the engine retries.  Successful retries
        // populate orders; timeouts mark the element `Impossible` and
        // fire `HERO_UNABLE_TO_DO_SOMETHING` for PCs.  Runs before the
        // hourglass dispatch so same-tick failures & retries both age
        // correctly.
        let expired = MovementContext::new(
            self.control.frame_counter,
            &mut self.world,
            &mut self.orders,
        )
        .take_expired_failures();
        for expired in expired {
            let request = expired.request;
            if expired.owner_is_pc {
                self.hero_speaking(
                    assets,
                    request.owner,
                    crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                );
            }

            if let Some(element) = self
                .orders
                .sequence_manager
                .get_element_mut(request.seq_id, request.elem_idx)
            {
                element.command = crate::element::Command::MoveOk;
            }
            self.orders
                .sequence_manager
                .element_impossible(request.seq_id, request.elem_idx);
            tracing::debug!(
                actor = ?request.owner,
                seq_id = ?request.seq_id,
                elem_idx = request.elem_idx,
                age = expired.age,
                "failed_path: 100-frame timeout expired — marking Impossible",
            );
        }

        // Original `CheckForCollision` follows ProcessPathRequests. Its only
        // implemented response is a human standing inside a non-stopped
        // mobile's motion polygon: launch RECEIVE_MOBILE_DAMAGE for 50/50
        // while the mobile moved last tick, otherwise 10/10.
        let mut humans: Vec<(EntityId, crate::coordinates::MapPoint)> = self
            .world
            .entities
            .humans()
            .map(|(id, human)| (id.into(), human.element_data().position_map()))
            .collect();
        humans.reverse();
        let mut impacts = Vec::new();
        for (human_id, position) in humans {
            for mobile in &self.world.mobile_elements {
                if !mobile.stopped && mobile.contains_point(position) {
                    let amount = if mobile.is_moving() { 50 } else { 10 };
                    impacts.push((human_id, mobile.sprite_ids[0], amount));
                }
            }
        }
        for (human_id, mobile_child, amount) in impacts {
            self.launch_element(crate::sequence::SequenceElement::new_damage(
                1,
                Command::ReceiveMobileDamage,
                Some(human_id),
                Some(mobile_child),
                amount,
                amount,
            ));
        }
    }

    /// Advance movement, animations, scripts, and the NPC-facing state that
    /// must be refreshed before the main AI pass.
    ///
    /// Original provenance: these responsibilities were distributed across
    /// individual `RHElement::Hourglass` implementations inside the original
    /// creation-ordered entity loop (`original-code/RHengine.cpp:3715-3723`).
    fn hourglass_phase_entity_systems(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) -> (
        EntitySlots<Option<crate::coordinates::MapPoint>>,
        Vec<EntityId>,
    ) {
        // Preserve the position each element exposed before the globally
        // batched movement pass. The original does not have this batch:
        // RHElementActorNPC::Hourglass calls RHElementActorHuman::Hourglass
        // (and therefore the observer's own movement) before RefreshView,
        // while actors with a later creation order have not run yet.
        let mut positions_before_movement = EntitySlots::filled(self.world.entities.len(), None);
        for (entity_id, entity) in self.world.entities.occupied() {
            positions_before_movement[entity_id] = Some(entity.element_data().position_map());
        }

        // ── Per-frame movement tick ─────────────────────────────
        // Actor movement runs later, inside the live legacy-slot owner walk.

        // RHElementMobile masters precede their RHElementFXMasked children in
        // the original creation-ordered Hourglass loop. Advance the shipped
        // chariot masters now, then translate/update their child FX before
        // the generic FX animation pass below.
        for mobile_index in 0..self.world.mobile_elements.len() {
            let path_index = self.world.mobile_elements[mobile_index].path_index;
            let path = assets
                .hiking_paths
                .get(usize::from(path_index))
                .unwrap_or_else(|| panic!("mobile {mobile_index} lost hiking path {path_index}"));
            let tick = self.world.mobile_elements[mobile_index]
                .hourglass(sim, path)
                .unwrap_or_else(|e| panic!("mobile {mobile_index} hourglass failed: {e}"));
            let active = self.world.mobile_elements[mobile_index].active;
            let animation_speed = self.world.mobile_elements[mobile_index].animation_speed();
            let layer = self.world.mobile_elements[mobile_index].layer;
            let sector = self.world.mobile_elements[mobile_index].sector;
            let sprite_ids = self.world.mobile_elements[mobile_index].sprite_ids.clone();
            for sprite_id in sprite_ids {
                let fx = self
                    .world
                    .entities
                    .get_mut(sprite_id)
                    .and_then(crate::element::Entity::as_fx_mut)
                    .unwrap_or_else(|| {
                        panic!("mobile {mobile_index} child {sprite_id} is missing or non-FX")
                    });
                if tick.movement != crate::coordinates::MapVec::ZERO {
                    let position = fx.element.position_map() + tick.movement;
                    fx.element.set_position_map(position);
                }
                fx.element.active = active;
                fx.fx.animation_speed = animation_speed;
                fx.element.set_layer(layer);
                fx.element
                    .set_sector(crate::position_interface::SectorHandle::new(sector));
            }
            self.check_mobile_line_crossing(assets, mobile_index);
        }

        // `quit_swordfight_with_far_opponents` is called ONLY during
        // walking-with-sword movement, NOT for stationary entities.
        // Only check entities actively moving in sword state.
        // Owned by the selected sword-movement Execute arm in
        // `tick_entity_movement_owner`.

        // ── PC sword-walk pinch abort ───────────────────────────
        // During `WalkingWithSword` / `RunningWithSword`, after the
        // per-frame sprite motion the PC checks whether two opponents
        // are pinching its forward corridor and, if so, marks the
        // current sequence element `Impossible`.  Runs only on PCs in
        // sword movement with an active movement element and an
        // in-flight position delta (`is_moving_map()`).
        // `element_impossible` itself silently no-ops when the
        // element is `NonInterruptable`, which is the desired
        // behaviour.
        // Owned by the selected PC sword-movement Execute arm too.

        // ── Dispatch EventReachPoint to NPCs that just finished walking ──
        // Fires `Think(EVENT_REACHPOINT)` when a MOVE sequence
        // element terminates.

        // ── Dispatch EventGaloppLoopEnd to riders with RIDER_CHARGE flag ──
        // When a rider's running animation reaches half/end frame
        // with RIDER_CHARGE, fire `Think(EVENT_GALOPP_LOOP_END)` so
        // the AI can check whether to begin the actual charge pass.

        // Separate Rust reconciliation boundary: the cited Original actor
        // Execute arms do not establish zone occupancy as owner-local work.
        // Fires EnterZone/ExitZone on zone scripts when occupancy changes.

        // ── Per-frame animation tick ────────────────────────────
        // Advance sprite animations for idle actors, FX, and other entities.
        // Moving actors are animated inside their live owner Execute arm.
        // Advance line-jump sequences: interpolate 3D position for
        // actors currently mid-jump.  Runs before the animation tick
        // so the sprite drawn this frame reflects the new position.
        self.tick_active_jumps(assets);

        // ── PC `Execute` per-arm validity pre-tick gate ─────────
        // Run the init-phase validity guards for TAKING / EATING /
        // SEARCHING / HEALING / HELPING-CLIMB transitions /
        // corpse-carry transitions / jump-init arms before the
        // animation driver so failing init-phase arms are aborted /
        // terminated synchronously instead of running their first
        // frame and then being marked Impossible from inside the
        // entity-iter borrow.
        self.pre_tick_pc_execute_validity(assets);

        // Preserve the pre-PA-013 batching boundary for nonactors: patch/FX/
        // object animation completes before any actor ActionChange callback,
        // and a callback-spawned nonactor cannot advance until next tick.
        self.tick_nonactor_entity_animations(sim, assets);
        let processed_projectiles =
            self.tick_actor_owner_envelopes(sim, assets, &positions_before_movement);
        // Separate Rust reconciliation boundary after owner movement.
        self.tick_zone_occupants(sim, assets);

        // ── Corpse-intersection repulsion hook ────────────────────
        // Scan for lying↔non-lying posture transitions and fire
        // `update_intersecting_corpses` so stacked corpses get the
        // smaller repulsive radius and don't shove each other out
        // of their hitboxes.  Runs after animations have had a
        // chance to change postures this frame and before the next
        // frame's movement (which reads `small_repulsive_radius`
        // via `compute_repulsive_force`).
        self.process_corpse_intersection_updates();

        // ── Per-frame animation sound dispatch ──────────────────
        // Now that every sprite has advanced (both movement-driven
        // and idle/one-shot animations), check each entity's current
        // sprite frame for an attached sound ID and queue it as an
        // FX (the `current_sound_id()` block every element type
        // runs during refresh / execute).
        self.dispatch_frame_sounds();

        // ── Per-scroll script Hourglass dispatch ────────────────
        // Every active scroll with a bound script bumps a per-scroll
        // `script_hourglass_timeout` counter; on every 25th active
        // frame the scroll's `IScrollScript::Hourglass(0)` fires
        // (bracketed by `SetScrollExecutingScript` / reset).
        self.dispatch_scroll_hourglasses(sim, assets);

        // TODO(original-parity): the followed-target position oracle below
        // proves one movement/NPC-refresh interleaving, but the rest of this
        // system-oriented pass still lacks per-entity dispatch boundaries.
        // Keep those responsibilities batched until each consumer has the
        // mixed pre/post inputs required at an individual creation slot.

        (positions_before_movement, processed_projectiles)
    }

    /// Run the bounded base-actor Hourglass slice in live legacy creation
    /// order: generic animation/Execute, synchronous combat-injury Think,
    /// completion/priority effects, then `ActionChange`.
    ///
    /// The loop deliberately rereads the live slot-array length and resolves
    /// each slot through `id_at_legacy_slot`, matching the original engine's
    /// mutable element-array walk. Generic animation eligibility does not gate
    /// `ActionChange`; inactive, frozen, moving, active-shot, and active-melee
    /// actors still reach the callback boundary.
    #[cfg(test)]
    pub(super) fn tick_actor_animation_action_change_slots(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        self.tick_actor_animation_action_change_slots_with_hooks(
            sim,
            assets,
            |_, _| {},
            |_, _| {},
            |_, _, _| {},
            |_, _| {},
        );
    }

    #[cfg(test)]
    pub(super) fn tick_actor_animation_action_change_slots_with_after_slot(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        after_slot: impl FnMut(&mut Self, EntityId),
    ) {
        self.tick_actor_animation_action_change_slots_with_hooks(
            sim,
            assets,
            |_, _| {},
            |_, _| {},
            |_, _, _| {},
            after_slot,
        );
    }

    pub(super) fn tick_actor_animation_action_change_slots_with_hooks(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        mut non_actor_slot: impl FnMut(&mut Self, EntityId),
        mut before_actor: impl FnMut(&mut Self, EntityId),
        mut execute_owner_arm: impl FnMut(
            &mut Self,
            EntityId,
            Option<super::movement::MovementOwnerSelection>,
        ),
        mut after_slot: impl FnMut(&mut Self, EntityId),
    ) {
        let mut slot = 0;
        while slot < self.world.entities.len() {
            if let Some(entity_id) = self.world.entities.id_at_legacy_slot(slot as u32) {
                let entity = self
                    .world
                    .entities
                    .get(entity_id)
                    .unwrap_or_else(|| {
                        panic!(
                            "actor animation coordinator lost entity {entity_id:?} resolved from legacy slot {slot}"
                        )
                    });
                let actor_enters_hourglass = entity.actor_data().is_some()
                    && !matches!(entity, Entity::Pc(pc) if pc.pc.fried_psykokwack);
                if actor_enters_hourglass {
                    // Detach work that predates this actor slot. Lazy Wait
                    // initialization and completion callbacks below may drain
                    // only work they synchronously create; they must not steal
                    // a global/later-owner continuation.
                    let preexisting_sequence_work = self
                        .orders
                        .sequence_manager
                        .take_pending_synchronous_actions();

                    before_actor(self, entity_id);
                    #[cfg(test)]
                    observe_actor_owner_envelope(ActorOwnerEnvelopePhase::BaseActor(entity_id));

                    let actor_is_active = self
                        .world
                        .entities
                        .get(entity_id)
                        .unwrap_or_else(|| {
                            panic!(
                                "actor {entity_id:?} vanished before Wait initialization at legacy slot {slot}"
                            )
                        })
                        .is_active();
                    if actor_is_active {
                        self.ensure_wait_element(entity_id);
                        self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
                            .unwrap_or_else(|error| {
                                panic!(
                                    "actor {entity_id:?} Wait initialization at legacy slot {slot} failed to drain its synchronous sequence work: {error:?}"
                                )
                            });
                    }
                    #[cfg(test)]
                    observe_actor_animation_boundary(ActorAnimationBoundaryPhase::WaitReady(
                        entity_id,
                    ));

                    let movement_selection = self
                        .orders
                        .sequence_manager
                        .current_order_for_actor(entity_id)
                        .and_then(|(seq_id, elem_idx, order)| {
                            self.orders
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .filter(|element| {
                                    element.data.is_movement()
                                        && !matches!(
                                            element.command,
                                            crate::element::Command::WaitTimer
                                                | crate::element::Command::WaitFreeLift
                                        )
                                })
                                .map(|_| super::movement::MovementOwnerSelection {
                                    seq_id,
                                    elem_idx,
                                    order_id: order.order_id,
                                })
                        });
                    #[cfg(test)]
                    observe_actor_owner_envelope(ActorOwnerEnvelopePhase::MovementExecute(
                        entity_id,
                    ));
                    execute_owner_arm(self, entity_id, movement_selection);

                    #[cfg(test)]
                    observe_actor_animation_boundary(ActorAnimationBoundaryPhase::GenericExecute(
                        entity_id,
                    ));
                    let frozen_wait_execute = self
                        .actors_frozen()
                        .then(|| {
                            self.orders
                                .sequence_manager
                                .current_order_for_actor(entity_id)
                                .and_then(|(seq_id, elem_idx, order)| {
                                    let element = self
                                        .orders
                                        .sequence_manager
                                        .get_element(seq_id, elem_idx)?;
                                    matches!(
                                        element.command,
                                        crate::element::Command::WaitTimer
                                            | crate::element::Command::WaitFreeLift
                                    )
                                    .then_some(
                                        super::animation::ActorExecuteResult {
                                            order_type: order.order_type,
                                            entry_seq_id: seq_id,
                                            entry_elem_idx: elem_idx,
                                            motion: crate::sprite::MotionState::InProgress,
                                        },
                                    )
                                })
                        })
                        .flatten();
                    let (combat_injury_terminated, mut outcomes, mut execute_result) =
                        if movement_selection.is_some() {
                            (Vec::new(), Default::default(), None)
                        } else if frozen_wait_execute.is_some() {
                            (Vec::new(), Default::default(), frozen_wait_execute)
                        } else {
                            self.tick_actor_animation_for(sim, assets, entity_id)
                        };
                    for injured_id in combat_injury_terminated.iter().copied() {
                        self.dispatch_combat_injury_think_for_actor_hourglass(
                            sim, injured_id, assets,
                        );
                    }
                    self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
                        .unwrap_or_else(|error| {
                            panic!(
                                "actor {entity_id:?} combat-injury Think at legacy slot {slot} failed to drain synchronous sequence work: {error:?}"
                            )
                        });
                    #[cfg(test)]
                    for injured_id in combat_injury_terminated {
                        observe_actor_animation_boundary(
                            ActorAnimationBoundaryPhase::CombatInjuryThink(injured_id),
                        );
                    }

                    // RHElementActorHuman::Execute performs this work inside
                    // the WaitingSword arm, after PerformAction and before
                    // returning its motion result to Actor::Hourglass. Keep
                    // launches and cross-actor mutations live so later slots
                    // observe them and earlier slots do not.
                    if execute_result.as_ref().is_some_and(|result| {
                        result.order_type == crate::order::OrderType::WaitingSword
                    }) {
                        self.tick_waiting_sword_execute_for(sim, assets, entity_id);
                        self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
                            .unwrap_or_else(|error| {
                                panic!(
                                    "actor {entity_id:?} WaitingSword Execute at legacy slot {slot} failed to drain synchronous sequence work: {error:?}"
                                )
                            });
                    }

                    // Original Actor::Hourglass modifies the just-produced
                    // Execute result for WAIT_TIMER / WAIT_FREE_LIFT before
                    // completion/DoNextOrder. Sampling the current element
                    // here is intentional: WaitingSword callbacks above may
                    // have synchronously replaced it.
                    if let Some(mut result) = execute_result.take() {
                        self.apply_actor_post_execute_wait_modifier(entity_id, &mut result);
                        self.stage_actor_execute_completion(entity_id, result, &mut outcomes);
                    }

                    // Original soldier Execute calls Think before returning
                    // Terminated to the base Actor Hourglass. Only after that
                    // synchronous Think finishes may DoNextOrder/completion
                    // promote the actor's successor order.
                    self.process_anim_completion_outcomes(sim, outcomes, assets);
                    self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
                        .unwrap_or_else(|error| {
                            panic!(
                                "actor {entity_id:?} completion at legacy slot {slot} failed to drain synchronous sequence work: {error:?}"
                            )
                        });
                    #[cfg(test)]
                    observe_actor_animation_boundary(
                        ActorAnimationBoundaryPhase::CompletionEffects(entity_id),
                    );

                    // Release every animation/completion borrow before the VM:
                    // ActionChange can synchronously replace this or a later
                    // actor's order and the next slot must sample that live.
                    #[cfg(test)]
                    observe_actor_animation_boundary(ActorAnimationBoundaryPhase::ActionChange(
                        entity_id,
                    ));
                    self.dispatch_actor_action_change_for(sim, assets, entity_id);
                    after_slot(self, entity_id);

                    let leaked_slot_work = self
                        .orders
                        .sequence_manager
                        .take_pending_synchronous_actions();
                    assert!(
                        leaked_slot_work.is_empty(),
                        "actor {entity_id:?} leaked synchronous sequence work after ActionChange at legacy slot {slot}: {leaked_slot_work:?}"
                    );
                    self.orders
                        .sequence_manager
                        .restore_pending_synchronous_actions(preexisting_sequence_work);
                } else {
                    non_actor_slot(self, entity_id);
                }
            }
            slot += 1;
        }

        // Movement, active melee, bow, abilities, and riders remain separate
        // owners. The production caller installs the human/PC/NPC tail hook
        // to close the supported actor envelope before this loop advances.
    }

    /// Fuse the supported Actor → Human → PC/NPC Hourglass slices into one
    /// live legacy-slot walk. The underlying actor coordinator owns the
    /// `while slot < entities.len()` loop, including holes and callback-spawned
    /// later slots; this hook closes the derived tail before it increments the
    /// slot.
    pub(super) fn tick_actor_owner_envelopes(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        positions_before_movement: &EntitySlots<Option<crate::coordinates::MapPoint>>,
    ) -> Vec<EntityId> {
        let prepared = self.prepare_npc_owner_pass(sim, assets);
        let prepared_movement = self.prepare_movement_frame();
        let mut processed_projectiles = Vec::new();
        self.tick_actor_animation_action_change_slots_with_hooks(
            sim,
            assets,
            |engine, owner| {
                let Some(Entity::Projectile(projectile)) = engine.get_entity(owner) else {
                    return;
                };
                if matches!(
                    projectile.object.object_type,
                    crate::element::ObjectType::Purse
                        | crate::element::ObjectType::Coin
                        | crate::element::ObjectType::WaspNest
                        | crate::element::ObjectType::BonusWaspNest
                        | crate::element::ObjectType::Wasp
                ) {
                    return;
                }
                engine.tick_existing_projectile(sim, assets, owner);
                processed_projectiles.push(owner);
            },
            |engine, owner| {
                if matches!(owner, EntityId::Soldier(_)) {
                    #[cfg(test)]
                    observe_actor_owner_envelope(ActorOwnerEnvelopePhase::SoldierPrelude(owner));
                    engine.tick_apple_smell_for(owner);
                    engine.tick_soldier_track_primary_target_for(owner);
                    let scratch = engine.build_owner_context_scratch_without_forecast(assets);
                    engine.tick_attacking_reactiontime_enemy_near_for(sim, assets, &scratch, owner);
                }
                if matches!(owner, EntityId::Soldier(_) | EntityId::Civilian(_))
                    && !engine.actors_frozen()
                {
                    #[cfg(test)]
                    observe_actor_owner_envelope(ActorOwnerEnvelopePhase::Patrol(owner));
                    engine.tick_patrol_coordination_for_npc(
                        sim,
                        assets,
                        owner,
                        positions_before_movement,
                    );
                }
                if engine
                    .world
                    .entities
                    .get(owner)
                    .is_some_and(|entity| entity.human_data().is_some())
                {
                    #[cfg(test)]
                    observe_actor_owner_envelope(ActorOwnerEnvelopePhase::HumanPrelude(owner));
                    engine.tick_concussion_healing_for(sim, owner, assets);
                    engine.process_shoot_list_for(owner);
                }
            },
            |engine, owner, selected| {
                engine.tick_entity_movement_owner(sim, assets, owner, &prepared_movement, selected)
            },
            |engine, owner| {
                let is_human = engine
                    .world
                    .entities
                    .get(owner)
                    .unwrap_or_else(|| {
                        panic!(
                            "actor owner {} disappeared before its derived Hourglass tail",
                            owner.index()
                        )
                    })
                    .human_data()
                    .is_some();
                if !is_human {
                    return;
                }
                match owner {
                    EntityId::Pc(_) => {
                        engine.refresh_pc_produced_noise_for(owner);
                        #[cfg(test)]
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::HumanNoise(owner));
                        engine.tick_tiredness_for(owner, assets);
                        #[cfg(test)]
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::HumanTiredness(
                            owner,
                        ));
                        engine.tick_pc_auto_heal_for(sim, owner);
                        #[cfg(test)]
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::PcTail(owner));
                    }
                    EntityId::Soldier(_) | EntityId::Civilian(_) => {
                        engine.tick_tiredness_for(owner, assets);
                        #[cfg(test)]
                        {
                            // NPC humans have no produced-noise refresh, so
                            // their Human tail begins at tiredness.
                            observe_actor_owner_envelope(ActorOwnerEnvelopePhase::HumanTiredness(
                                owner,
                            ));
                        }
                        engine.tick_npc_owner_pass(
                            sim,
                            assets,
                            positions_before_movement,
                            prepared,
                            owner,
                        );
                        #[cfg(test)]
                        observe_actor_owner_envelope(ActorOwnerEnvelopePhase::NpcTail(owner));
                    }
                    _ => panic!(
                        "human actor owner {} has unsupported entity kind",
                        owner.index()
                    ),
                }
            },
        );
        self.finish_npc_owner_pass();
        processed_projectiles
    }

    /// Apply the two sequence-command motion modifiers owned by
    /// `RHElementActor::Hourglass` after one actor's Execute call.
    fn apply_actor_post_execute_wait_modifier(
        &mut self,
        owner: EntityId,
        execute_result: &mut super::animation::ActorExecuteResult,
    ) {
        let Some((seq_id, elem_idx)) = self
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
        else {
            return;
        };
        let command = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .unwrap_or_else(|| {
                panic!(
                    "actor {owner:?} post-Execute modifier lost current sequence element {seq_id:?}/{elem_idx}"
                )
            })
            .command;

        match command {
            crate::element::Command::WaitTimer => {
                let actor = self
                    .world
                    .entities
                    .get_mut(owner)
                    .unwrap_or_else(|| panic!("WAIT_TIMER post-Execute owner {owner:?} is missing"))
                    .actor_data_mut()
                    .unwrap_or_else(|| {
                        panic!("WAIT_TIMER post-Execute owner {owner:?} is not an actor")
                    });
                if actor.wait_time == 0 {
                    execute_result.motion = crate::sprite::MotionState::Terminated;
                } else {
                    actor.wait_time -= 1;
                }
            }
            crate::element::Command::WaitFreeLift => {
                let authorized = super::sequence_runtime::LiftWaitCommandContext {
                    entities: &mut self.world.entities,
                    fast_grid: &mut self.world.fast_grid,
                    doors: self.script_domains.interactables.doors.as_slice(),
                    sequence_manager: &mut self.orders.sequence_manager,
                }
                .authorize_and_reserve(owner, seq_id, elem_idx);
                if authorized {
                    execute_result.motion = crate::sprite::MotionState::Terminated;
                }
            }
            _ => {}
        }
    }

    /// Resolve the retained base-Actor motion after derived Execute callbacks
    /// and wait modifiers. Original TERMINATED calls DoNextOrder through the
    /// owner's live `mpSequenceElement`; ABORTED alone uses the sequence
    /// element snapshot captured before Execute.
    fn stage_actor_execute_completion(
        &self,
        owner: EntityId,
        execute_result: super::animation::ActorExecuteResult,
        outcomes: &mut super::animation::AnimCompletionOutcomes,
    ) {
        match execute_result.motion {
            crate::sprite::MotionState::Aborted => outcomes
                .seq_impossible
                .push((execute_result.entry_seq_id, execute_result.entry_elem_idx)),
            crate::sprite::MotionState::Terminated => {
                let Some((seq_id, elem_idx, order)) =
                    self.orders.sequence_manager.current_order_for_actor(owner)
                else {
                    return;
                };
                match order.completion.clone() {
                    crate::order::OrderCompletion::AdvanceElement => {
                        outcomes.seq_advance.push((seq_id, elem_idx));
                    }
                    crate::order::OrderCompletion::UnlockDoor { door_id } => {
                        outcomes.unlock_door.push((door_id, seq_id, elem_idx));
                    }
                    crate::order::OrderCompletion::ResumeDoorPass => {
                        outcomes.resume_door_pass.push(owner);
                    }
                    crate::order::OrderCompletion::NextJumpStep => {
                        outcomes.next_jump_step.push(owner);
                    }
                    crate::order::OrderCompletion::WaspStruggleCycle { cycles_remaining } => {
                        if cycles_remaining <= 1 {
                            outcomes.seq_terminate.push((seq_id, elem_idx));
                        } else {
                            outcomes
                                .wasp_next_cycle
                                .push((seq_id, elem_idx, cycles_remaining - 1));
                        }
                    }
                }
            }
            crate::sprite::MotionState::Done
            | crate::sprite::MotionState::Start
            | crate::sprite::MotionState::InProgress => {}
            crate::sprite::MotionState::Error => panic!(
                "actor {owner:?} Execute returned MotionState::Error from entry {:?}/{}",
                execute_result.entry_seq_id, execute_result.entry_elem_idx
            ),
        }
    }

    /// Run the NPC Hourglass tail and its immediately adjacent notification
    /// passes in the exact order established by the original implementation.
    ///
    /// Original provenance: `RHElementActorNPC::Hourglass` in
    /// `original-code/RHelementactornpc.cpp:3495-3614`.
    fn hourglass_phase_npcs(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        _positions_before_movement: &EntitySlots<Option<crate::coordinates::MapPoint>>,
    ) {
        // Listen and object/FX blip discovery are not owned by an NPC
        // Hourglass. Keep this mutating work out of the prepare-once owner
        // inputs; those must stay immutable and RNG-free.
        if !self.actors_frozen() {
            self.tick_non_npc_blip_detection(sim, assets);
        }
        // ── Creation-ordered pre-detection boundary ──────────────
        // These observations remain coarse labels for the original nested
        // order. The coordinator below interleaves the actual operations per
        // NPC: own synchronous FITAGAIN + resurrection/eye apply, own body
        // broadcast, own view refresh, then that same NPC's RefreshDetection.
        #[cfg(test)]
        observe_npc_hourglass_phase(NpcHourglassPhase::Broadcasts);
        #[cfg(not(test))]
        observe_npc_hourglass_phase(());

        #[cfg(test)]
        observe_npc_hourglass_phase(NpcHourglassPhase::View);
        #[cfg(not(test))]
        observe_npc_hourglass_phase(());

        #[cfg(test)]
        observe_npc_hourglass_phase(NpcHourglassPhase::Detection);
        #[cfg(not(test))]
        observe_npc_hourglass_phase(());
        // Production work already ran inside the live actor-owner walk in the
        // preceding EntitySystems phase. Keep these coarse observations for
        // the PA-016 tick-spine contract only.

        // The phase observations below retain the coarse PA-016 ordering
        // contract. Production work no longer runs here: PA-013 executes the
        // complete post-detection tail inside each NPC's creation slot before
        // the next NPC enters RefreshDetection.
        #[cfg(test)]
        observe_npc_hourglass_phase(NpcHourglassPhase::Ambush);
        #[cfg(not(test))]
        observe_npc_hourglass_phase(());

        // ── Per-tick AILOCK_BUSY edge detector ─────────────────
        // Lock or unlock AILOCK_BUSY based on the live
        // `is_very_very_busy` predicate (posture or active PassDoor /
        // Fall element).  Runs after the view refresh.
        #[cfg(test)]
        observe_npc_hourglass_phase(NpcHourglassPhase::Busy);
        #[cfg(not(test))]
        observe_npc_hourglass_phase(());

        // ── Stuck-on-ladder emergency counter ──────────────────
        // Bump per frame for non-script-locked NPCs on outdoor
        // ladders idling in CMD_WAIT/CMD_MOVE_WAITING; after 25
        // frames force a ReturnToDuty so the actor can self-recover.
        // Runs after the BUSY edge detector.
        #[cfg(test)]
        observe_npc_hourglass_phase(NpcHourglassPhase::Ladder);
        #[cfg(not(test))]
        observe_npc_hourglass_phase(());

        // ── Locked-frame timer bumps ───────────────────────────
        // When any lock is held the entire Hourglass tail
        // short-circuits while the three timer ring-frames
        // (`when_does_timer_ring`, `when_does_macro_timer_ring`,
        // `emoticon_expiration_date`) tick forward by +1.  This both
        // keeps the relative timer offset stable across the lock
        // window and acts as the "skip the fire" gate for the
        // downstream macro-timer / EVENT_TIMER fire checks (which
        // compare against the live `frame_counter`).
        #[cfg(test)]
        observe_npc_hourglass_phase(NpcHourglassPhase::LockGate);
        #[cfg(not(test))]
        observe_npc_hourglass_phase(());

        // The unlocked tail is ordered exactly like the original callee:
        // The16thFrame, normal EVENT_TIMER, macro timer, then stimuli held
        // by a prior AI/script lock.
        #[cfg(test)]
        observe_npc_hourglass_phase(NpcHourglassPhase::SixteenthFrame);
        #[cfg(not(test))]
        observe_npc_hourglass_phase(());

        #[cfg(test)]
        observe_npc_hourglass_phase(NpcHourglassPhase::NormalTimer);
        #[cfg(not(test))]
        observe_npc_hourglass_phase(());

        // ── Macro-timer hourglass ──────────────────────────────
        // Poll the macro-specific timer each frame and, when it
        // rings, call `execute_next_macro_command` directly —
        // bypassing the stimulus queue so CMD_WAIT / CMD_BEND
        // resume cleanly. Any resulting movement-order / substate change
        // is visible to the queued-stimulus drain in the same frame.
        #[cfg(test)]
        observe_npc_hourglass_phase(NpcHourglassPhase::MacroTimer);
        #[cfg(not(test))]
        observe_npc_hourglass_phase(());

        #[cfg(test)]
        observe_npc_hourglass_phase(NpcHourglassPhase::QueuedStimuli);
        #[cfg(not(test))]
        observe_npc_hourglass_phase(());

        // Every engine-entered AI call closes its ordered owner-local
        // SetState/Say boundary before returning to effects/orders. Nothing
        // may survive into the obsolete post-NPC speech batch position.
        for (npc_id, entity) in self.world.entities.npcs() {
            let leaked = entity
                .ai_controller()
                .is_some_and(|ai| !ai.outbox.reentrant.owner_work.is_empty());
            assert!(
                !leaked,
                "NPC {} leaked owner-local AI work past its Hourglass slot",
                npc_id.index()
            );
        }

        // ── HUD speech-log decay ────────────────────────────────
        // Decrement the per-remark display timer and evict expired
        // entries every frame regardless of `speech_display` so the
        // Vec does not grow unbounded when the overlay is off.
        self.tick_screen_remarks();

        // TODO(PA-013): pure-Rust handlers still enqueue until their AI borrow
        // returns, so arbitrary reads between Say/SetState statements cannot
        // yet observe the Original's fully inline engine/audio call stack.
    }

    /// Advance combat, projectiles, abilities, and other gameplay systems that
    /// consume the entity/sequence/NPC state established above.
    pub(super) fn hourglass_phase_gameplay_systems(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        assets: &LevelAssets,
        processed_projectiles: &[EntityId],
    ) {
        // The original loop is literally:
        //
        // `for (i = 0; i < marrayElements.Size(); ++i) element[i]->Hourglass()`
        //
        // so both cross-type effects and entities appended during a virtual
        // call are observable in creation order. These are the gameplay
        // systems with proven cross-batch differences; keep the slot count
        // live rather than snapshotting ids.
        let mut slot = 0;
        while slot < self.world.entities.len() {
            let Some(entity_id) = self.world.entities.id_at_legacy_slot(slot as u32) else {
                slot += 1;
                continue;
            };

            #[cfg(test)]
            CAPTURED_ORDERED_GAMEPLAY_ENTITIES.with(|captured| {
                if let Some(entities) = captured.borrow_mut().as_mut() {
                    entities.push(entity_id);
                }
            });

            match entity_id {
                EntityId::Pc(_) | EntityId::Soldier(_) | EntityId::Civilian(_) => {
                    // `RHElementActor::Hourglass` executes the actor's active
                    // order before `RHElementActorPC::Hourglass` applies its
                    // auto-heal tail. Only one of bow/melee/ability can own
                    // that active order, but dispatching each narrow driver
                    // keeps stale-state cleanup behavior intact.
                    self.tick_bow_shot_for(sim, assets, entity_id);
                    self.tick_straight_melee_for(sim, assets, entity_id);
                    self.tick_melee_completion_for(sim, assets, entity_id);
                    let initialized_sweep = self.tick_nonstraight_melee_for(sim, assets, entity_id);
                    self.tick_sweep_for(sim, assets, entity_id, initialized_sweep);
                    self.tick_ability_for(sim, display, assets, entity_id);
                }
                EntityId::Projectile(_) => {
                    let object_type = match self.get_entity(entity_id) {
                        Some(Entity::Projectile(projectile)) => projectile.object.object_type,
                        _ => unreachable!("projectile id must resolve to a projectile entity"),
                    };
                    match object_type {
                        crate::element::ObjectType::Purse | crate::element::ObjectType::Coin => {
                            self.tick_purse_or_coin(sim, assets, entity_id)
                        }
                        crate::element::ObjectType::WaspNest
                        | crate::element::ObjectType::BonusWaspNest
                        | crate::element::ObjectType::Wasp => {
                            self.tick_wasp_nest_or_wasp(sim, assets, entity_id)
                        }
                        _ if !processed_projectiles.contains(&entity_id) => {
                            self.tick_existing_projectile(sim, assets, entity_id)
                        }
                        _ => {}
                    }
                }
                EntityId::Net(_) => self.tick_net(sim, assets, entity_id),
                _ => {}
            }
            slot += 1;
        }

        // ── Beggar-solicitation tick ────────────────────────────
        // For each PC currently in `SimulatingBeggar` posture,
        // iterate civilians and toss a coin to the beggar if a
        // donor passes the full predicate chain.
        self.tick_beggar_bids(assets);

        // Combat progression without a proven cross-subsystem ordering
        // discrepancy remains batched. Fallback-timed completions already
        // cleared at their owning actor slots above and are skipped here.
        self.tick_melee_combat(sim, assets);

        // ── Per-actor `Order::done` propagation ────────────────
        // Runs after every per-system sprite-advance tick this frame
        // (movement, jumps, animations, bow shots, melee, abilities),
        // each of which has already stashed its result on the sprite
        // via `Sprite::record_motion_state`.  The pass flips
        // `Order::done` on every actor whose sprite reported
        // `MotionState::Done`, then clears `last_motion_state` so the
        // next tick starts fresh.  Read by the postpone-race guard in
        // `EngineInner::engine_postpone`.
        self.propagate_done_to_current_orders();

        // ── Carried entity position sync ───────────────────────
        // Keep bodies carried by Little John positioned on the carrier
        // and drive their sprite animation (BeingLifted/BeingCarried/
        // BeingDropped) synchronized with the carrier.  Needs the
        // campaign profile manager to look up LittleJohnCarry contextual
        // actions on the carrier.
        if true {
            abilities::sync_carried_positions(&mut self.world.entities, &assets.profile_manager);
        }

        // TODO(original-parity): move further gameplay maintenance into the
        // ordered pass only when a concrete observable discrepancy is proven.
    }

    /// Apply work intentionally deferred until every entity, path, sequence,
    /// NPC, and gameplay-system update has completed.
    ///
    /// Original provenance: `original-code/RHengine.cpp:3729-3775` performs the
    /// swordfight falling-edge check, titbit update, dead-selection scan, and
    /// anonymous timers after the sequence manager. Rust adds deterministic
    /// condolation, self-stimulus, and immediate-action drains.
    fn hourglass_phase_deferred_effects_end(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        assets: &LevelAssets,
        was_swordfighting: bool,
    ) {
        // ── Swordfight-drag IgnoreMouseEvent bracket ────────────
        // If the selected PC was swordfighting at entry to
        // `perform_hourglass` but is no longer swordfighting after
        // the per-element / sequence-manager hourglass, raise the
        // ignore-mouse-event bracket so a drag in flight when the
        // swordfight ended this tick is suppressed.  We push the
        // request as a side effect; the host gates it on
        // `InputState::is_dragging` in `apply_side_effects`.
        if was_swordfighting && !self.is_selected_pc_swordfighting() {
            self.feedback
                .pending_side_effects
                .pending_swordfight_drag_ignore = true;
        }

        // ── Titbit sync + per-frame update ──────────────────────
        // First, sync persistent titbits (emoticons, unconscious
        // stars, alert indicators) with current entity state.
        self.sync_titbits(assets);

        // Then run the titbit update to advance animations and
        // expire finished titbits.
        {
            let query = EntityTitbitQuery {
                sim,
                entities: &self.world.entities,
                sequence_manager: &self.orders.sequence_manager,
                follow_element: self.players.seats[0].follow_element,
            };
            self.feedback.titbit_manager.update(&query);
            // PrepareRefresh: advance blink counter, sort by
            // display order using each supplier entity's Y position
            // as a stand-in (we don't compute display order yet).
            self.feedback.titbit_manager.prepare_refresh(|handle| {
                self.world
                    .entities
                    .id_at_legacy_slot(handle.0)
                    .and_then(|entity_id| self.world.entities.get(entity_id))
                    .map(|e| e.element_data().position_map().y)
            });
        }

        // ── Ground mark animation ────────────────────────────────
        // Advanced after `perform_hourglass_inner` by `ground_mark.tick`,
        // using the deterministic director view. That helper preserves the
        // original on-screen guard, so off-screen marks freeze. The renderer
        // remains read-only; see the wrapper at the start of this file.

        // Selection ring animation lives host-side now —
        // `Game::run_engine_tick` advances `host.selection_mark`
        // once per frame, gated on the same `should_run_hourglass`
        // check as this function, so pause / console still freeze
        // the ring.

        // ── Check selected PCs are still alive ───────────────────
        {
            let mut deselect = Vec::new();
            for &pc_id in &self.players.seats[0].selection {
                if let Some(entity) = self.world.entities.get(pc_id) {
                    let should_deselect = match entity {
                        Entity::Pc(pc) => pc.pc.life_points <= 0 || pc.human.unconscious,
                        _ => false,
                    };
                    if should_deselect {
                        deselect.push(pc_id);
                    }
                }
            }
            for pc_id in deselect {
                self.orders.messenger.send(Message::pc(
                    crate::messenger::PcMessage::UnselectCharacter,
                    Some(pc_id),
                ));
            }
        }

        // ── Anonymous timers ─────────────────────────────────────
        // Decrement each timer; remove entries that reach 0 and
        // mark the backing sequence element `Terminated` so the
        // sequence advances.
        let mut expired: Vec<crate::sequence::SequenceElementRef> = Vec::new();
        self.orders.timer_elements.retain_mut(|timer| {
            if timer.remaining <= 1 {
                expired.push(timer.element_ref);
                false
            } else {
                timer.remaining -= 1;
                true
            }
        });
        for r in expired {
            self.orders
                .sequence_manager
                .element_terminated(r.sequence_id, r.element_index);
        }

        // ── Post-timer SendCondolationCard dispatch ──────────────
        // The pre-timer pass after `hourglass_phase_sequences` preserves the
        // original SetState -> SendCondolationCard -> Ready ordering for work
        // that completed before this scan. This second pass is still required:
        // timer expiry above can itself terminate an owned sequence element
        // and queue another card. Its continuation and immediate successors
        // drain below, after this frame's timer iteration has finished.
        self.dispatch_condolations(sim, assets);

        // ── Same-tick re-entrant stimulus dispatch ───────────────
        // The condolation drain calls `Think(EVENT_DONE)` /
        // `Think(EVENT_IMPOSSIBLE)` / etc. synchronously and
        // re-entrantly on the same tick — so e.g. a patrol Turn
        // that gets interrupted when `SetAttentiveMode(true)`
        // launches `ENTER_ATTENTIVE_MODE` during
        // `EventViewStandardProcedure` fires its `EVENT_DONE`
        // *during that same* `EventView` Think, advancing
        // `SUBSTATE_ATTACKING_REACTIONTIME_TURNING` →
        // `REACTIONTIME` before the frame ends.  We can't nest
        // `&mut AiController` borrows mid-think, so
        // `send_condolation_card` queues the stimulus via
        // `fire_self_stimulus` (→ `pending_self_stimuli`).  Drain
        // that queue here — after `dispatch_condolations` has
        // populated it — so the redispatch happens on the same
        // tick as the condolation, keeping
        // `REACTIONTIME_TURNING → REACTIONTIME` timing correct.
        // Without this the substate waits for the full
        // `LaunchTimer(20)` upper bound regardless of which
        // sequence actually completed.
        self.drain_pending_self_stimuli(sim, assets);

        // ── End-of-tick immediate-action drain ──────────────────────
        // Catch any `register_element_to_go` calls that happened
        // in post-action passes (condolation fan-out, self-stimulus
        // drains, etc.) without piggybacking on the
        // hourglass action-loop drain.  Close the immediate-side-
        // effect window before returning control to the host
        // renderer so post-tick state reads see the immediate side
        // effects.
        self.drain_pending_immediate_actions_sync(sim, display, assets);
    }

    /// Auto-leave disguise/stealth posture if the entity is in one and
    /// the incoming command requires Upright posture.
    ///
    /// **Superseded.**  The transition logic now lives in
    /// `engine/transitions.rs` and runs at launch time via
    /// `launch_element_for_owner` / the stamped single-order
    /// wrapper.  Posture transitions resolve before the element
    /// becomes `InProgress`, so the dispatch pipeline no longer
    /// needs to peek at posture.
    ///
    /// This helper remains as `#[cfg(test)]` so the legacy edge-case
    /// tests in `engine/tests.rs` that document the partial-port
    /// behaviour still compile.  Those tests cross-check commands the
    /// transitions module also covers; once they're migrated to call
    /// `generate_transition` directly, this function can be deleted.
    #[cfg(test)]
    pub(super) fn auto_leave_disguise_if_needed(
        &mut self,
        owner: EntityId,
        command: Command,
    ) -> bool {
        use crate::stealth;
        use crate::titbit::{ElementHandle, TitbitKind};

        if !stealth::command_requires_upright(command) {
            return false;
        }

        let posture = match self.world.entities.get(owner) {
            Some(e) => e.element_data().posture,
            None => return false,
        };

        // Honor the `CAN_BE_LEANING_OUT` /
        // `CAN_BE_ANONYMOUS_ARCHER` flags that pair with
        // `MUST_BE_UPRIGHT` on a handful of bow commands: the actor
        // keeps its lean-out / anonymous-archer pose rather than
        // unsticking before the shot (e.g. `SHOOT_BOW` from a
        // lean-out window preserves the lean).
        if posture == crate::element::Posture::LeaningOut
            && stealth::command_allows_leaning_out(command)
        {
            return false;
        }
        if posture == crate::element::Posture::AnonymousArcher
            && stealth::command_allows_anonymous_archer(command)
        {
            return false;
        }

        // ENTER_LEISURE permits CAN_BE_LEISURING, letting an
        // already-leisuring NPC re-enter leisure without standing
        // up first.  Skip the auto-leave in that case so the
        // animation pipeline doesn't churn through Upright.
        if command == Command::EnterLeisure && posture == crate::element::Posture::Leisure {
            return false;
        }

        let transition = match stealth::leave_disguise(posture) {
            Some(t) => t,
            None => {
                // Also handle Crouched → Upright for commands that need it.
                if posture == crate::element::Posture::Crouched {
                    stealth::crouch_up()
                } else {
                    return false;
                }
            }
        };

        // Snap posture + action state.  Pre-existing behavior for
        // disguise / crouched transitions is silent (no transition
        // anim queued); the soldier-specific `LeaningOut → Upright`
        // branch additionally queues
        // `TransitionLeaningOutWaitingAlerted` on the actor's
        // order_queue so the lean-out-window soldier plays the
        // visible unstick transition.  Sitting/Leisure are also
        // visible transitions (NPC standing up out of a chair / out
        // of leisure pose), so they queue their animation too.
        let queue_anim = matches!(
            posture,
            crate::element::Posture::LeaningOut
                | crate::element::Posture::Sitting
                | crate::element::Posture::Leisure
        );
        // Look up the sequence element that's currently dispatching
        // this command so the queued transition animation can be
        // tagged with its owner — if the element is later
        // interrupted (injury mid-transition),
        // `send_condolation_card` scrubs the pending order so no
        // ghost animation plays.  The order lives on the sequence
        // element and goes away with it.
        let dispatching = self.find_dispatching_element(owner, command);

        if let Some(entity) = self.world.entities.get_mut(owner) {
            entity.set_posture(transition.result_posture);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = transition.result_action_state;
            }
        }
        if queue_anim {
            // `compute_direction = false` on the transition
            // order — direction is preserved so the soldier
            // finishes facing the same way it was leaning.
            let mut order = crate::order::Order::new(
                transition.animation,
                0.0,
                0.0,
                self.orders.allocate_order_id(),
            );
            order.compute_direction = false;
            if let Some((seq_id, elem_idx)) = dispatching {
                self.orders
                    .sequence_manager
                    .push_order_on(seq_id, elem_idx, order);
            } else {
                // No dispatching element found — spawn a single-
                // order generic sequence so the visible unstick
                // transition still plays.  Without a host element
                // we launch a tiny one just to carry this animation.
                self.launch_single_order_sequence_stamped(owner, Command::Generic, order);
            }
        }

        // Set `posture_after_transition` so downstream dispatch
        // (e.g. `NpcAttentionCommandContext`) decides whether to
        // run the command's real transition or snap.
        if let Some((seq_id, elem_idx)) = dispatching
            && let Some(elem) = self
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
        {
            elem.posture_after_transition = transition.result_posture;
            elem.action_state_after_transition = transition.result_action_state;
        }

        // Remove HIDDEN titbit when leaving a hidden posture.
        if posture.is_hidden() {
            self.feedback
                .titbit_manager
                .remove_titbit(TitbitKind::Hidden, ElementHandle(owner.index()));
        }

        tracing::debug!(
            ?owner,
            ?command,
            old_posture = ?posture,
            new_posture = ?transition.result_posture,
            "auto-leave disguise before command"
        );
        true
    }

    /// Find the sequence element currently being dispatched for
    /// `(owner, command)` so auto-leave can update its
    /// `posture_after_transition` / `action_state_after_transition`
    /// fields.
    ///
    /// Only reachable from `auto_leave_disguise_if_needed`, which is
    /// itself `#[cfg(test)]` after the transitions-port migration.
    #[cfg(test)]
    fn find_dispatching_element(
        &self,
        owner: EntityId,
        command: Command,
    ) -> Option<(crate::sequence::SequenceId, usize)> {
        use crate::sequence::SequenceState;
        self.orders
            .sequence_manager
            .live_element_for_actor_matching(owner, |elem| {
                elem.command == command
                    && matches!(elem.state, SequenceState::Todo | SequenceState::InProgress)
            })
    }

    /// Whether `owner` is a beggar civilian that refuses this command.
    ///
    /// Beggars accept only `RECEIVE_PURSE`, `BEGGAR_SHOW_FACE`, and
    /// `WAIT`.  Every other sequence command on a beggar is
    /// rejected — `sequence_manager.element_impossible` fires.
    pub(super) fn beggar_rejects_command(&self, owner: EntityId, cmd: Command) -> bool {
        let is_beggar = self.get_entity(owner).is_some_and(|e| {
            matches!(e, crate::element::Entity::Civilian(c)
                if c.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar)
        });
        is_beggar
            && !matches!(
                cmd,
                Command::ReceivePurse | Command::BeggarShowFace | Command::Wait
            )
    }

    pub(super) fn apply_door_pass_transition_done_side_effects(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        use crate::coordinates::MapPoint;
        use crate::element::{ActionState, Posture};
        use crate::order::OrderType as OT;

        let Some((door_index, action, is_pc)) = self.get_entity(entity_id).and_then(|entity| {
            entity.actor_data().and_then(|actor| {
                actor
                    .active_door_pass
                    .as_ref()
                    .map(|dp| (dp.door_index, dp.current_action, entity.is_pc()))
            })
        }) else {
            return;
        };

        let door = required_canonical_door(
            &self.script_domains.interactables.doors,
            door_index,
            "PassDoor transition side effects",
        );
        let (layer_in, layer_out, sector_in, sector_out, point_in, point_mid, point_out) = (
            door.layer_in,
            door.layer_out,
            door.sector_in,
            door.sector_out,
            MapPoint {
                x: door.point_in.x,
                y: door.point_in.y,
            },
            MapPoint {
                x: door.point_mid.x,
                y: door.point_mid.y,
            },
            MapPoint {
                x: door.point_out.x,
                y: door.point_out.y,
            },
        );

        let lift_direction = self
            .grid_sector_by_number(crate::sector::SectorNumber::new(i16::from(sector_in)))
            .and_then(|sector| {
                if sector.lift_type == Some(crate::sector::LiftType::Wall) {
                    Some(sector.lift_direction)
                } else {
                    None
                }
            });

        match action {
            OT::TransitionWaitingUprightClimbingWallUp => {
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.set_posture(Posture::OnWall);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Moving;
                    }
                }
            }
            OT::TransitionWaitingCrouchedClimbingWallDown => {
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.set_posture(Posture::OnWall);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Moving;
                    }
                }
                self.set_transition_position_map_and_compute_position_all(
                    assets,
                    entity_id,
                    crate::coordinates::MapPoint {
                        x: point_in.x,
                        y: point_in.y,
                    },
                );
            }
            OT::TransitionWaitingCrouchedClimbingWallDownCrenel => {
                let point_in = crate::coordinates::MapPoint::new(point_in.x, point_in.y);
                self.finalize_special_move_position(
                    assets,
                    entity_id,
                    super::special_motion::SpecialMovePosition::Map(point_in),
                    Some(layer_in),
                    Some(u16::from(sector_in)),
                    Some(point_in),
                    "crenel climb-down transition",
                );
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.set_posture(Posture::OnWall);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Moving;
                    }
                    let elem = entity.element_data_mut();
                    if let Some(dir) = lift_direction {
                        elem.set_direction_instantly(dir);
                    }
                }
            }
            OT::TransitionClimbingWallUpWaitingCrouched => {
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.set_posture(if is_pc {
                        Posture::Crouched
                    } else {
                        Posture::Upright
                    });
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Waiting;
                    }
                }
                self.set_transition_position_map_and_compute_position_all(
                    assets,
                    entity_id,
                    crate::coordinates::MapPoint {
                        x: point_mid.x,
                        y: point_mid.y,
                    },
                );
            }
            OT::TransitionClimbingWallUpWaitingCrouchedCrenel => {
                let point_out_probe = crate::coordinates::MapPoint::new(point_out.x, point_out.y);
                let point_mid_map = crate::coordinates::MapPoint::new(point_mid.x, point_mid.y);
                self.finalize_special_move_position(
                    assets,
                    entity_id,
                    super::special_motion::SpecialMovePosition::Map(point_mid_map),
                    Some(layer_out),
                    Some(u16::from(sector_out)),
                    Some(point_out_probe),
                    "crenel climb-up transition",
                );
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.set_posture(Posture::Flying);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Moving;
                    }
                    {
                        let pi = &mut entity.element_data_mut().sprite.position_iface;
                        let point_out = crate::coordinates::MapPoint {
                            x: point_out.x,
                            y: point_out.y,
                        };
                        pi.set_old_map_position(point_out);
                        pi.set_map_goal(point_out);
                        pi.compute_increment_all(true);
                    }
                }
            }
            OT::TransitionClimbingWallDownWaitingUpright => {
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.set_posture(Posture::Upright);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Waiting;
                    }
                }
            }
            _ => {}
        }
    }

    fn set_transition_position_map_and_compute_position_all(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        point: crate::coordinates::MapPoint,
    ) {
        self.finalize_special_move_position(
            assets,
            entity_id,
            super::special_motion::SpecialMovePosition::Map(point),
            None,
            None,
            Some(point),
            "wall transition",
        );
    }

    fn apply_door_pass_transition_completion_side_effects(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        use crate::coordinates::MapPoint;
        use crate::element::{ActionState, Posture};
        use crate::order::OrderType as OT;

        let Some((door_index, action, is_pc)) = self.get_entity(entity_id).and_then(|entity| {
            entity.actor_data().and_then(|actor| {
                actor
                    .active_door_pass
                    .as_ref()
                    .map(|dp| (dp.door_index, dp.current_action, entity.is_pc()))
            })
        }) else {
            return;
        };

        let Some((snap_point, posture, action_state, sector_in)) = (|| {
            let door = required_canonical_door(
                &self.script_domains.interactables.doors,
                door_index,
                "PassDoor transition completion",
            );
            let snap = match action {
                OT::TransitionWaitingUprightClimbingWallUp => Some(MapPoint {
                    x: door.point_mid.x,
                    y: door.point_mid.y,
                }),
                OT::TransitionClimbingWallDownWaitingUpright
                | OT::TransitionClimbingLadderDownWaitingUpright
                | OT::TransitionClimbingLadderDownWaitingUprightAlerted
                | OT::TransitionClimbingWallUpWaitingCrouchedCrenel => None,
                _ => return None,
            };
            let (posture, action_state) = match action {
                OT::TransitionWaitingUprightClimbingWallUp => {
                    (Posture::OnWall, ActionState::Moving)
                }
                OT::TransitionClimbingWallDownWaitingUpright => {
                    (Posture::Upright, ActionState::Waiting)
                }
                OT::TransitionClimbingLadderDownWaitingUpright
                | OT::TransitionClimbingLadderDownWaitingUprightAlerted => {
                    (Posture::Upright, ActionState::Waiting)
                }
                OT::TransitionClimbingWallUpWaitingCrouchedCrenel => {
                    let posture = if is_pc {
                        Posture::Crouched
                    } else {
                        Posture::Upright
                    };
                    (posture, ActionState::Waiting)
                }
                _ => return None,
            };
            Some((snap, posture, action_state, door.sector_in))
        })() else {
            return;
        };
        let lift_direction = self
            .grid_sector_by_number(crate::sector::SectorNumber::new(i16::from(sector_in)))
            .and_then(|sector| {
                if sector.lift_type == Some(crate::sector::LiftType::Wall) {
                    Some(sector.lift_direction)
                } else {
                    None
                }
            });

        if let Some(snap_point) = snap_point {
            self.set_transition_position_map_and_compute_position_all(
                assets,
                entity_id,
                crate::coordinates::MapPoint {
                    x: snap_point.x,
                    y: snap_point.y,
                },
            );
        }

        let Some(entity) = self.world.entities.get_mut(entity_id) else {
            return;
        };
        let elem = entity.element_data_mut();
        if let Some(dir) = lift_direction {
            elem.set_direction_instantly(dir);
        }
        elem.update_grid_cell();
        entity.set_posture(posture);
        if let Some(actor) = entity.actor_data_mut() {
            actor.action_state = action_state;
        }
    }

    /// Post-animation hook that drains outcomes collected by
    /// [`EngineInner::tick_actor_animation_for`] for non-`EventDone`
    /// completion variants.
    ///
    /// - `seq_terminate`: terminate the associated sequence element
    ///   (Turn / any plain `SequenceElement` booking).
    /// - `unlock_door`:   flip `door.locked_pc = false`, then terminate
    ///   the lockpick sequence element.  The lock release is tied
    ///   to the end of the `UnlockingDoor` order.
    /// - `resume_door_pass`: re-enter `advance_door_pass` for the actor
    ///   so the next step in the door-pass chain (PassingDoor trigger,
    ///   next Walk step, or Done) can fire.
    pub(super) fn process_anim_completion_outcomes(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        outcomes: super::animation::AnimCompletionOutcomes,
        assets: &LevelAssets,
    ) {
        use super::movement::DoorPassAdvance;

        for (seq_id, elem_idx) in outcomes.non_interruptable_lifts.iter().copied() {
            self.orders.sequence_manager.set_element_priority(
                seq_id,
                elem_idx,
                crate::sequence::SequencePriority::NonInterruptable,
            );
        }

        for (seq_id, elem_idx) in outcomes.seq_advance {
            // `do_next_order` semantics: pop the just-completed
            // order; advance to the next if one exists, otherwise
            // terminate the element.
            self.do_next_order(seq_id, elem_idx);
        }

        // Wasp struggle-cycle refill: push a fresh `GettingFreeFromWasp`
        // order with the decremented counter, then pop the current one
        // via `do_next_order` so the new order takes over cleanly.
        for (seq_id, elem_idx, cycles_remaining) in outcomes.wasp_next_cycle {
            let order = crate::order::Order::new(
                crate::order::OrderType::GettingFreeFromWasp,
                0.0,
                0.0,
                self.orders.allocate_order_id(),
            )
            .with_completion(crate::order::OrderCompletion::WaspStruggleCycle { cycles_remaining });
            self.orders
                .sequence_manager
                .push_order_on(seq_id, elem_idx, order);
            self.do_next_order(seq_id, elem_idx);
        }

        for (seq_id, elem_idx) in outcomes.seq_terminate {
            self.orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
        }

        for (actor, command_level, anim) in outcomes.play_anim_frozen {
            let mut elem = crate::sequence::SequenceElement::new_generic(
                command_level,
                crate::element::Command::PlayAnimFrozen,
                Some(actor),
            );
            elem.set_property(
                crate::sequence::Field::AnimationId,
                crate::sequence::FieldValue::Animation(anim),
            );
            self.orders.sequence_manager.launch_element(elem);
        }

        // ABORTED motion result: set the sequence element to
        // IMPOSSIBLE.
        for (seq_id, elem_idx) in outcomes.seq_impossible {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
        }

        for (door_id, seq_id, elem_idx) in outcomes.unlock_door {
            let door = required_canonical_door_mut(
                &mut self.script_domains.interactables.doors,
                door_id,
                "UnlockDoor completion",
            );
            door.locked_pc = false;
            tracing::debug!(
                door_id = %door_id,
                "UnlockDoor: lockpick animation complete, door unlocked"
            );
            self.orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
        }

        for entity_id in outcomes.next_jump_step {
            self.advance_jump_step(entity_id);
        }

        for entity_id in outcomes.resume_door_pass {
            self.apply_door_pass_transition_completion_side_effects(assets, entity_id);
            // Advance through Transition / PassingDoor / Walk steps.
            // PassingDoor triggers fired here need to run through
            // `execute_pass_door` with `&mut self`, so we collect them
            // and drain after the borrow on the actor ends.
            let mut door_triggers: Vec<(EntityId, crate::gate::DoorIndex, bool, u8)> = Vec::new();
            let mut select_triggers: Vec<(EntityId, f32)> = Vec::new();
            let (advance, arrived_movement, completed_pass) = {
                let Some(entity) = self.world.entities.get_mut(entity_id) else {
                    continue;
                };
                let Some(actor) = entity.actor_data_mut() else {
                    continue;
                };
                let adv = Self::advance_door_pass(
                    actor,
                    entity_id,
                    &mut door_triggers,
                    &mut select_triggers,
                    &mut self.orders.next_order_id,
                );
                // If the door pass is done (no more steps), mirror the
                // arrival teardown performed by the movement tick.
                let arrived = if let DoorPassAdvance::Done { completed } = &adv {
                    let am = actor.active_movement;
                    actor.clear_path();
                    actor.action_state = if actor.action_state.is_sword() {
                        crate::element::ActionState::WaitingSword
                    } else {
                        crate::element::ActionState::Waiting
                    };
                    actor.active_movement.clear();
                    actor.active_door_pass = None;
                    Some((am, *completed))
                } else {
                    None
                };
                let (arrived, completed) = match arrived {
                    Some((am, completed)) => (Some(am), completed),
                    None => (None, None),
                };
                (adv, arrived, completed)
            };

            // Fire any PassingDoor triggers that came up during this resume.
            for (eid, door_index, direct, trigger_num) in door_triggers {
                self.execute_pass_door(sim, assets, eid, door_index, direct, trigger_num);
            }
            for (eid, speed) in select_triggers {
                self.apply_select_hulk(eid, speed);
            }
            if let Some((door_index, direct)) = completed_pass {
                tracing::debug!(
                    entity = ?entity_id,
                    door = %door_index,
                    direct,
                    "DoorPass: completed after transition resume"
                );
                self.apply_completed_door_pass_lift_entry_state(entity_id, door_index, direct);
            }
            // If the advance yielded another Walk or Transition step,
            // append it behind the completed transition order, then pop
            // that completed transition so the new order becomes the
            // front order.  This mirrors the movement-tick door-pass
            // path, where `transition_pushes` are drained before
            // `order_pops`.
            if let Some((seq_id, elem_idx)) = self
                .orders
                .sequence_manager
                .current_element_for_actor(entity_id)
            {
                match advance.clone() {
                    DoorPassAdvance::Continue {
                        destination,
                        action,
                        reverse,
                        compute_direction,
                        tolerance,
                    } => {
                        tracing::debug!(
                            entity = ?entity_id,
                            ?action,
                            target_x = destination.x,
                            target_y = destination.y,
                            "DoorPass: resumed with movement order after transition"
                        );
                        self.install_special_walk_order(
                            entity_id,
                            seq_id,
                            elem_idx,
                            destination,
                            action,
                            reverse,
                            compute_direction,
                            tolerance,
                            None,
                            "PassDoor resumed walk",
                        );
                        self.do_next_order(seq_id, elem_idx);
                    }
                    DoorPassAdvance::Paused { transition_order } => {
                        self.orders.sequence_manager.push_order_on(
                            seq_id,
                            elem_idx,
                            transition_order,
                        );
                        self.do_next_order(seq_id, elem_idx);
                    }
                    DoorPassAdvance::NoActive => {
                        tracing::warn!(
                            entity = ?entity_id,
                            "DoorPass: resume callback had no active pass"
                        );
                        self.do_next_order(seq_id, elem_idx);
                    }
                    DoorPassAdvance::Done { .. } => {}
                }
            }

            // If the door pass completed, notify the sequence manager
            // and dispatch EventReachPoint, matching the handling in
            // `tick_entity_movement` for normal arrival.
            if let Some(am) = arrived_movement {
                if let Some(seq_id) = am.sequence_id {
                    self.orders
                        .sequence_manager
                        .element_terminated(seq_id, am.element_index);
                }
                self.dispatch_reach_point_events(sim, assets, &[entity_id]);
            }

            let _ = advance;
        }

        // ── Soldier `Execute` cross-entity side effects ──────────
        // Collected by `apply_soldier_execute_side_effects` as the
        // animation tick walks each `active_ai_anim` booking.  Each
        // block below fires a cross-entity effect (bottle hide,
        // coin pickup, remarks, blood-alcohol bump).
        let sides = outcomes.execute_sides;

        for entity_id in sides.weak_stunned_start {
            self.add_weak_stunned_combat(entity_id);
        }

        for entity_id in sides.hidden_titbit_removals {
            self.feedback.titbit_manager.remove_titbit(
                crate::titbit::TitbitKind::Hidden,
                crate::titbit::ElementHandle(entity_id.index()),
            );
        }

        for actor_id in sides.smalltalk_swipes {
            let (target_id, position, weapon1) = {
                let Some(entity) = self.get_entity(actor_id) else {
                    continue;
                };
                let Some(target_id) = entity
                    .human_data()
                    .and_then(|h| h.opponents.first().copied())
                else {
                    continue;
                };
                let target_mutual = self
                    .get_entity(target_id)
                    .and_then(|e| e.human_data())
                    .and_then(|h| h.opponents.first().copied())
                    .map(|id| id == actor_id)
                    .unwrap_or(false);
                if !target_mutual {
                    continue;
                }
                let pos = entity.element_data().position_map();
                let weapon1 =
                    super::melee::weapon_material_from_profile(entity, &assets.profile_manager);
                (target_id, pos, weapon1)
            };
            let weapon2 = self
                .get_entity(target_id)
                .map(|e| super::melee::weapon_material_from_profile(e, &assets.profile_manager))
                .unwrap_or(crate::profiles::WeaponMaterial::SteelAndWood);
            self.feedback
                .pending_side_effects
                .sounds
                .push(super::SoundCommand::StrikeFx {
                    strike_kind: crate::sound::StrikeKind::Swipe,
                    weapon1,
                    weapon2,
                    position,
                });
        }

        for (victim_id, killer_id) in sides.killed_at_bottom {
            let mut elem = crate::sequence::SequenceElement::new_interaction(
                1,
                crate::element::Command::GetKilledAtBottom,
                Some(victim_id),
                Some(killer_id),
            );
            elem.priority = crate::sequence::SequencePriority::Lethal;
            self.launch_element(elem);
        }

        // DRINKING_ALE DONE — deactivate the antagonist to hide
        // the ale bottle.
        for antag in sides.deactivate_entities {
            if let Some(entity) = self.world.entities.get_mut(antag) {
                entity.element_data_mut().active = false;
            }
        }

        for (pc, target, activation_cmd) in sides.pc_target_activations {
            let target_is_fx = self
                .get_entity(target)
                .is_some_and(|e| e.kind().is_fx_target());
            if !target_is_fx {
                tracing::warn!(
                    ?pc,
                    ?target,
                    ?activation_cmd,
                    "PC target animation DONE but antagonist is not an FX target"
                );
                continue;
            }
            let mut activation =
                crate::sequence::SequenceElement::new(1, activation_cmd, Some(target));
            activation.data = crate::sequence::SequenceElementData::Interaction {
                antagonist: Some(pc),
            };
            self.launch_element(activation);
        }

        for (rescuer, target) in sides.waking_up_done {
            let target_entity = self.get_entity(target).unwrap_or_else(|| {
                panic!(
                    "WakingUp DONE from rescuer {rescuer:?} references missing required target {target:?}"
                )
            });
            if !target_entity.is_human() {
                panic!(
                    "WakingUp DONE from rescuer {rescuer:?} requires human target {target:?}, found {:?}",
                    target_entity.kind()
                );
            }

            let target_is_dead = target_entity.is_dead();
            let target_is_pc = target_entity.is_pc();
            if !target_is_dead {
                if let Some(target_entity) = self.get_entity_mut(target) {
                    target_entity.set_posture(crate::element::Posture::Lying);
                    if let Some(actor) = target_entity.actor_data_mut() {
                        actor.action_state = crate::element::ActionState::Waiting;
                    }
                }
                self.apply_concussion(sim, assets, target, 0, false);
                self.stop_owner(target, crate::sequence::SequencePriority::Normal);
                self.ensure_wait_element(target);
            }

            if target_is_pc {
                self.hero_speaking(assets, target, crate::engine::melee::HERO_RECOVER);
            }
        }

        // TAKING DONE — dispatches by taker + object_type.
        //
        // * PC takers route through `apply_pc_take_object` which
        //   covers amulet, purse, coin, ransom, relics, and the
        //   default ammo-bonus fall-through.
        //
        // * Net takers (PC or NPC) hit the shared net-release path.
        //
        // * NPC soldiers picking up Coin/Purse use the short
        //   money-bump path.
        //
        // * Scrolls route through `take_scroll` which fires
        //   `IScrollScript::IsTaken`.
        for (taker, object) in sides.pickups {
            // Scrolls are not ObjectData carriers — they have their
            // own Entity::Scroll variant and a script-driven
            // `IsTaken` dispatch.
            let is_scroll = matches!(
                self.world.entities.get(object),
                Some(crate::element::Entity::Scroll(_))
            );
            if is_scroll {
                self.scroll_is_taken(sim, assets, object, taker);
                continue;
            }

            let object_type = self
                .world
                .entities
                .get(object)
                .and_then(|e| e.object_data())
                .map(|o| o.object_type);
            let taker_is_pc = self.get_entity(taker).map(|e| e.is_pc()).unwrap_or(false);

            match object_type {
                Some(obj_type)
                    if obj_type == crate::element::ObjectType::Net
                        || (taker_is_pc && obj_type == crate::element::ObjectType::BonusNet) =>
                {
                    self.unapply_net_effect(object);
                    if taker_is_pc {
                        self.increase_ammo_and_enable(
                            assets,
                            taker,
                            crate::profiles::Action::Net,
                            1,
                        );
                    }
                    self.remove_entity(object);
                }
                // Scroll — PC click-to-take path.  Flips `taken`,
                // sets status to Opened, forces the BonusThree
                // sprite row, then (when a script is bound) invokes
                // `IScrollScript::IsTaken(pc)` on the bound class.
                // When the script returns non-zero the status
                // advances to Taken; otherwise it rests at Opened.
                Some(crate::element::ObjectType::Scroll) => {
                    self.take_scroll(sim, assets, taker, object);
                }
                Some(obj_type) if taker_is_pc => {
                    // Snapshot the object's position/layer/quantity/
                    // associated-action before mutating the engine.
                    let Some(obj_entity) = self.get_entity(object) else {
                        continue;
                    };
                    let obj_data = obj_entity.object_data();
                    let (quantity, assoc_action) = match obj_data {
                        Some(o) => (o.quantity, o.associated_action),
                        None => continue,
                    };
                    let elem = obj_entity.element_data();
                    let (bx, by, blayer) =
                        (elem.position_map().x, elem.position_map().y, elem.layer());
                    self.apply_pc_take_object(
                        assets,
                        taker,
                        object,
                        obj_type,
                        assoc_action,
                        quantity,
                        bx,
                        by,
                        blayer,
                    );
                }
                Some(crate::element::ObjectType::Purse)
                | Some(crate::element::ObjectType::Coin) => {
                    // NPC soldier picking up a dropped purse/coin:
                    // add the money to the soldier's purse and
                    // remove the element.  PCs went through the
                    // branch above.
                    let value = match object_type {
                        Some(crate::element::ObjectType::Purse) => {
                            crate::inventory::COINS_PER_PURSE as u32 * crate::inventory::COIN_VALUE
                        }
                        Some(crate::element::ObjectType::Coin) => crate::inventory::COIN_VALUE,
                        _ => 0,
                    };
                    if value > 0 {
                        if let Some(entity) = self.world.entities.get_mut(taker)
                            && let Some(npc) = entity.npc_data_mut()
                        {
                            npc.money = npc.money.saturating_add(value);
                        }
                        // Deactivate the object (clearing `active`
                        // is our equivalent of unlinking from the
                        // engine's active-element list).
                        if let Some(entity) = self.world.entities.get_mut(object) {
                            entity.element_data_mut().active = false;
                        }
                    }
                }
                _ => {}
            }
        }

        // DRINKING_ALE TERMINATED — add the profile's beer value
        // to the soldier's blood alcohol (clamped to 100).
        // `blood_alcohol` lives on the `AiController` attached to
        // the soldier's NPC data via `ai_brain`; `profile.beer` is
        // the per-profile increment (see profiles.rs).
        for soldier in sides.drink_done {
            let profile_idx = self
                .world
                .entities
                .get(soldier)
                .and_then(|e| e.soldier_data())
                .map(|sd| sd.soldier_profile_index);
            let beer = profile_idx
                .and_then(|idx| assets.profile_manager.get_soldier(idx))
                .map(|prof| prof.beer)
                .unwrap_or(0);
            if beer == 0 {
                continue;
            }
            if let Some(entity) = self.world.entities.get_mut(soldier)
                && let Some(npc) = entity.npc_data_mut()
                && let Some(base) = npc.ai_brain.base_mut()
            {
                let new_val = (base.blood_alcohol as u16 + beer).min(100);
                base.blood_alcohol = new_val as u8;
            }
        }

        // SEARCHING DONE — NPC-on-NPC pickpocket money transfer:
        // thief.money += victim.money; victim.money = 0.
        for (thief, victim) in sides.pickpockets {
            let stolen = self
                .world
                .entities
                .get(victim)
                .and_then(|e| e.npc_data())
                .map(|n| n.money)
                .unwrap_or(0);
            if stolen == 0 {
                continue;
            }
            if let Some(entity) = self.world.entities.get_mut(victim)
                && let Some(npc) = entity.npc_data_mut()
            {
                npc.money = 0;
            }
            if let Some(entity) = self.world.entities.get_mut(thief)
                && let Some(npc) = entity.npc_data_mut()
            {
                npc.money = npc.money.saturating_add(stolen);
            }
        }

        // GETTING_FREE_FROM_WASP START — `Say(REMARK_WASP_STING)`.
        // Plain `say` on the AI base.
        for speaker in sides.wasp_sting_remark {
            if let Some(entity) = self.world.entities.get_mut(speaker)
                && let Some(npc) = entity.npc_data_mut()
                && let Some(base) = npc.ai_brain.base_mut()
            {
                base.say(crate::ai::Remark::WaspSting);
            }
            self.drain_ai_owner_work_for(sim, assets, speaker);
        }

        // SPECIAL START — `make_special_action_remark`.  Branches
        // on `IsShieldBearer`: shield-bearers always speak,
        // everyone else only speaks at 1-in-3 odds and only when
        // currently silent.  `IsShieldBearer` = sword is a shield
        // weapon AND the sprite has the `WaitingShield` animation —
        // the same two-gate check used by the per-tick
        // FighterSnapshot build (engine/ai/snapshots.rs:619-632).
        for speaker in sides.special_remark {
            // Two-step: read weapon/sprite info immutably, then
            // dispatch the remark mutably.  Splitting avoids holding
            // an immutable borrow on `self.world.entities` across the
            // mutable `npc.ai_brain.enemy_mut()` call.
            let is_shield_bearer = self
                .world
                .entities
                .get(speaker)
                .map(|entity| {
                    let hth_weapon_id = entity
                        .npc_data()
                        .and_then(|npc| npc.ai_brain.enemy())
                        .map(|e| e.hth_weapon_id)
                        .unwrap_or(0);
                    let weapon_is_shield = assets
                        .profile_manager
                        .get_hth_weapon(hth_weapon_id)
                        .map(|w| w.shield)
                        .unwrap_or(false);
                    let has_shield_anim = entity
                        .element_data()
                        .sprite
                        .has_animation(crate::order::OrderType::WaitingShield);
                    weapon_is_shield && has_shield_anim
                })
                .unwrap_or(false);
            if let Some(entity) = self.world.entities.get_mut(speaker)
                && let Some(npc) = entity.npc_data_mut()
                && let Some(enemy) = npc.ai_brain.enemy_mut()
            {
                enemy.make_special_action_remark(sim, is_shield_bearer);
            }
            self.drain_ai_owner_work_for(sim, assets, speaker);
        }

        // LYING_STUCK_UNDER_NET 1/31 cycle — NPCs say
        // `UnderNet` (soldier) or `CivUnderNet` (civilian) plus a
        // HEEELP noise at the entity's 2D position (volume
        // `NOISE_VOLUME_HEEELP`, = 200).
        for speaker in sides.cry_for_help_under_net {
            let (remark, origin, layer, elevation) = {
                let Some(entity) = self.world.entities.get(speaker) else {
                    continue;
                };
                let is_soldier = matches!(entity, Entity::Soldier(_));
                let remark = if is_soldier {
                    crate::ai::Remark::UnderNet
                } else {
                    crate::ai::Remark::CivUnderNet
                };
                let elem = entity.element_data();
                let pos3d = elem.position();
                (
                    remark,
                    elem.position_map(),
                    elem.layer(),
                    pos3d.z.max(0.0) as u16,
                )
            };
            if let Some(entity) = self.world.entities.get_mut(speaker)
                && let Some(npc) = entity.npc_data_mut()
                && let Some(base) = npc.ai_brain.base_mut()
            {
                base.say(remark);
            }
            self.drain_ai_owner_work_for(sim, assets, speaker);
            self.broadcast_noise(
                crate::ai::NoiseType::Heeelp,
                origin,
                layer,
                crate::parameters_ai::NOISE_VOLUME_HEEELP as u16,
                elevation,
                Some(speaker),
            );
        }
    }

    fn advance_mission_clock(&mut self) {
        self.control.frame_counter += 1;
        if self.control.frame_counter.is_multiple_of(FRAMES_PER_SECOND)
            && let Some(campaign) = Some(&mut self.mission_domain.campaign)
        {
            campaign.add_value(crate::campaign::CampaignValue::MissionLength, 1);
        }
    }
}

/// Insert randomised midpoint detours into a pathfinder-returned
/// waypoint list (drunken soldier post-process path).
///
/// Walks the waypoint list in passes (one pass per
/// `blood_alcohol / increment` increments) and for every segment
/// tries up to 3 random deviation vectors; the first reachable one
/// gets inserted as a new intermediate waypoint.  Running soldiers
/// use a lower increment + factor (they don't wobble as much per
/// step) than walking soldiers.
///
/// The RNG is drained deterministically from the explicit caller context, so
/// replays reproduce the same deviation sequence. Original provenance:
/// `RHElementActorSoldier::PostProcessPath` in
/// `original-code/RHelementactorsoldier.cpp:1688-1771` uses two draws for
/// each of up to three candidate deviations per segment.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_drunken_path_deviation(
    sim: &crate::sim_rng::SimulationContext,

    mut waypoints: Vec<crate::coordinates::MapPoint>,
    origin: crate::coordinates::MapPoint,
    blood_alcohol: u8,
    is_running: bool,
    layer: u16,
    move_box: &crate::coordinates::MoveBox,
    half_diagonal: crate::coordinates::MoveBoxHalfDiagonal,
    grid: &crate::fast_find_grid::FastFindGrid,
) -> Vec<crate::coordinates::MapPoint> {
    const DRUNKEN_DEVIATION_FACTOR: f32 = 0.03;

    // Max of (30, blood_alcohol) — the minimum ensures even mildly
    // tipsy soldiers still show some wobble.
    let clamped_ba = blood_alcohol.max(30) as f32;
    let (factor, increment) = if is_running {
        (0.003 * clamped_ba, 60u8)
    } else {
        (0.01 * clamped_ba, 30u8)
    };

    let mut iterator = 0u8;
    while iterator < blood_alcohol {
        let mut new_path: Vec<crate::coordinates::MapPoint> =
            Vec::with_capacity(waypoints.len() * 2);
        let mut prev = origin;
        for next in &waypoints {
            let straight = crate::coordinates::MapVec::new(next.x - prev.x, next.y - prev.y);
            let max_norm = straight.x.abs().max(straight.y.abs());
            // Midpoint of the current segment.
            let midpoint = crate::coordinates::MapPoint::new(
                prev.x + 0.5 * straight.x,
                prev.y + 0.5 * straight.y,
            );
            let mut inserted: Option<crate::coordinates::MapPoint> = None;
            for _try in 0..3 {
                // `rand() & 15` — pick a random 16-sector direction
                // and scale by another 0..15 random magnitude.
                let dir_sector =
                    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::DrunkenPathDeviation, 0..16)
                        as i16;
                let magnitude =
                    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::DrunkenPathDeviation, 0..16)
                        as f32;
                let (dx, dy) = crate::element_kinds::direction_vector_16(dir_sector);
                let scale = magnitude * max_norm * DRUNKEN_DEVIATION_FACTOR * factor;
                let candidate = crate::coordinates::MapPoint::new(
                    midpoint.x + dx * scale,
                    midpoint.y + dy * scale,
                );
                if grid.is_straight_movement_authorized(prev, candidate, layer, move_box)
                    && grid.is_reachable_thick(candidate, *next, layer, half_diagonal)
                {
                    inserted = Some(candidate);
                    break;
                }
            }
            if let Some(ip) = inserted {
                new_path.push(ip);
            }
            new_path.push(*next);
            prev = *next;
        }
        waypoints = new_path;
        iterator = iterator.saturating_add(increment);
    }

    waypoints
}

// ─── Titbit update query ─────────────────────────────────────────

/// Real implementation of [`crate::titbit::TitbitUpdateQuery`] that
/// queries live entity state.  Replaces the old `StubQuery` that kept
/// all titbits alive unconditionally.
struct EntityTitbitQuery<'a> {
    sim: &'a crate::sim_rng::SimulationContext,
    entities: &'a crate::entities::Entities,
    sequence_manager: &'a crate::sequence::SequenceManager,
    follow_element: Option<EntityId>,
}

impl crate::titbit::TitbitUpdateQuery for EntityTitbitQuery<'_> {
    /// True when the entity should keep its weak-stunned titbit.
    ///
    /// - Soldiers in `WonderingAppleSauceInTheVisor` always keep stars.
    /// - Otherwise, stars stay only while the current animation is
    ///   `BeingWeakSword` or `BeingStunnedSword`.
    fn is_weak_or_stunned(&self, element: crate::titbit::ElementHandle) -> bool {
        use crate::ai::Substate;
        use crate::order::OrderType;

        let Some(entity_id) = self.entities.id_at_legacy_slot(element.0) else {
            return false;
        };
        let Some(entity) = self.entities.get(entity_id) else {
            return false;
        };

        // Soldiers in apple-sauce substate keep stars unconditionally.
        if let Entity::Soldier(s) = entity
            && s.npc.ai_substate() == Substate::WonderingAppleSauceInTheVisor
        {
            return true;
        }

        // Otherwise, check if the current animation is weak/stunned sword.
        // Orders live on the owning `SequenceElement.orders` now —
        // look up via the actor's current in-progress element.
        matches!(
            self.sequence_manager
                .current_order_for_actor(entity_id)
                .map(|(_, _, o)| o.order_type),
            Some(OrderType::BeingWeakSword | OrderType::BeingStunnedSword)
        )
    }

    fn is_unconscious_and_alive(&self, element: crate::titbit::ElementHandle) -> bool {
        let Some(entity_id) = self.entities.id_at_legacy_slot(element.0) else {
            return false;
        };
        let Some(entity) = self.entities.get(entity_id) else {
            return false;
        };
        match entity {
            Entity::Pc(pc) => pc.human.unconscious && pc.pc.life_points > 0,
            Entity::Soldier(s) => s.human.unconscious && s.npc.life_points > 0,
            Entity::Civilian(c) => c.human.unconscious && c.npc.life_points > 0,
            _ => false,
        }
    }

    fn is_follow_element(&self, element: crate::titbit::ElementHandle) -> bool {
        // The entity the camera is currently locked onto (via
        // `SelectFollowElement` / `LockCameraOn`).
        self.follow_element
            .is_some_and(|id| id.index() == element.0)
    }

    fn is_hidden_posture(&self, element: crate::titbit::ElementHandle) -> bool {
        use crate::element::Posture;
        let Some(entity_id) = self.entities.id_at_legacy_slot(element.0) else {
            return false;
        };
        let Some(entity) = self.entities.get(entity_id) else {
            return false;
        };
        matches!(
            entity.element_data().posture,
            Posture::Spy | Posture::Tree | Posture::AnonymousArcher
        )
    }

    fn random_u32(&self) -> u32 {
        crate::sim_rng::u32(self.sim, crate::sim_rng::RngSite::TitbitUpdate, ..)
    }
}

#[cfg(test)]
mod bow_command_body_parity_tests {
    use super::*;
    use crate::element::{
        ActionState, ActorData, ActorPc, ActorSoldier, ElementData, ElementKind, Entity, HumanData,
        NpcData, PcData, Posture, SoldierData,
    };
    use crate::order::OrderType;
    use crate::sequence::{SequenceElement, SequenceId, SequenceState};

    fn make_aiming_pc(action_state: ActionState) -> Entity {
        Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData {
                action_state,
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        })
    }

    fn launch_bow_command_and_tick(command: Command, action_state: ActionState) -> EngineInner {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let pc_id = engine.add_entity(make_aiming_pc(action_state));
        engine.launch_element(SequenceElement::new(1, command, Some(pc_id)));

        let mut display = HostDisplayState::default();
        let mut dev = DevState::default();
        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.perform_hourglass(&mut display, &assets, &mut dev);
        engine
    }

    fn command_order_types(engine: &EngineInner) -> Vec<OrderType> {
        engine
            .orders
            .sequence_manager
            .get_element(SequenceId(1), 0)
            .unwrap()
            .orders
            .iter()
            .map(|order| order.order_type)
            .collect()
    }

    fn make_bow_soldier(posture: Posture, action_state: ActionState) -> Entity {
        Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                active: true,
                posture,
                ..ElementData::default()
            },
            actor: ActorData {
                action_state,
                ..ActorData::default()
            },
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        })
    }

    fn install_test_lift_sector(
        engine: &mut EngineInner,
        owner: EntityId,
        sector_number: crate::sector::SectorNumber,
    ) {
        engine
            .world
            .entities
            .get_mut(owner)
            .expect("test lift owner exists")
            .element_data_mut()
            .set_sector(crate::position_interface::SectorHandle::new(0));
        let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid.level);
        level.sector_number_map.insert(sector_number, 0);
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::LIFT,
            layer: 0,
            sector_number,
            door_index: None,
            lift_type: Some(crate::sector::LiftType::Ladder),
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        });
    }

    #[test]
    fn bow_lean_out_commands_keep_transition_order_live() {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let soldier_id = engine.add_entity(make_bow_soldier(
            Posture::Upright,
            ActionState::AimingWithBow,
        ));
        let seq_id = engine.launch_element(SequenceElement::new(
            1,
            Command::LowerBowLeanOut,
            Some(soldier_id),
        ));

        let mut display = HostDisplayState::default();
        let mut dev = DevState::default();
        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.perform_hourglass(&mut display, &assets, &mut dev);

        let elem = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(
            elem.state,
            SequenceState::InProgress,
            "C++ LOWER_BOW_LEAN_OUT keeps its translated transition order live"
        );
        assert_eq!(
            elem.current_order().map(|order| order.order_type),
            Some(OrderType::TransitionLoweringBowLeaningOut)
        );
    }

    #[test]
    fn equip_bow_terminates_when_actor_is_already_aiming() {
        let engine = launch_bow_command_and_tick(Command::EquipBow, ActionState::AimingWithBow);
        let elem = engine
            .orders
            .sequence_manager
            .get_element(SequenceId(1), 0)
            .unwrap();

        assert_eq!(elem.state, SequenceState::Terminated);
        assert!(
            elem.orders.is_empty(),
            "redundant EquipBow must not queue equip/load orders"
        );
    }

    #[test]
    fn pre_timer_condolation_starts_successor_timer_before_the_scan() {
        use crate::sequence::{Field, FieldValue, Sequence};

        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let pc_id = engine.add_entity(make_aiming_pc(ActionState::AimingWithBow));
        let mut sequence = Sequence::new();
        sequence.append_element(SequenceElement::new(1, Command::EquipBow, Some(pc_id)));
        let mut timer = SequenceElement::new_generic(2, Command::Timer, None);
        timer.set_property(Field::Timer, FieldValue::Integer(2));
        sequence.append_element(timer);
        engine.orders.sequence_manager.launch_sequence(sequence);

        let mut display = HostDisplayState::default();
        let mut dev = DevState::default();
        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.perform_hourglass(&mut display, &assets, &mut dev);

        assert_eq!(engine.orders.timer_elements.len(), 1);
        assert_eq!(
            engine.orders.timer_elements[0].remaining, 1,
            "the immediate Timer successor must launch before the same frame's timer scan"
        );
    }

    #[test]
    fn timer_expiry_condolation_starts_successor_after_the_scan() {
        use crate::sequence::{Field, FieldValue, Sequence};

        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let pc_id = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let mut sequence = Sequence::new();
        let mut expiring = SequenceElement::new_generic(1, Command::Timer, Some(pc_id));
        expiring.set_property(Field::Timer, FieldValue::Integer(1));
        sequence.append_element(expiring);
        let mut successor = SequenceElement::new_generic(2, Command::Timer, None);
        successor.set_property(Field::Timer, FieldValue::Integer(2));
        sequence.append_element(successor);
        engine.orders.sequence_manager.launch_sequence(sequence);

        let mut display = HostDisplayState::default();
        let mut dev = DevState::default();
        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.perform_hourglass(&mut display, &assets, &mut dev);

        assert_eq!(engine.orders.timer_elements.len(), 1);
        assert_eq!(
            engine.orders.timer_elements[0].remaining, 2,
            "a successor launched by timer expiry belongs to the final condolation drain and must not re-enter the timer scan in progress"
        );
    }

    #[test]
    fn equip_bow_down_terminates_when_actor_is_already_aiming_up() {
        let engine =
            launch_bow_command_and_tick(Command::EquipBowDown, ActionState::AimingWithBowUp);
        let elem = engine
            .orders
            .sequence_manager
            .get_element(SequenceId(1), 0)
            .unwrap();

        assert_eq!(elem.state, SequenceState::Terminated);
        assert!(
            elem.orders.is_empty(),
            "redundant EquipBowDown must not queue equip/load/lower orders"
        );
    }

    #[test]
    fn raise_bow_from_waiting_queues_equip_load_then_raise() {
        let engine = launch_bow_command_and_tick(Command::RaiseBow, ActionState::Waiting);

        assert_eq!(
            command_order_types(&engine),
            vec![
                OrderType::TransitionEquipBow,
                OrderType::TransitionLoadingBow,
                OrderType::TransitionRaisingBow,
            ],
            "C++ TestBowAimUp expects RaiseBow from waiting to equip, load, then raise"
        );
    }

    #[test]
    fn unequip_bow_from_aiming_up_queues_lower_unload_then_unequip() {
        let engine = launch_bow_command_and_tick(Command::UnequipBow, ActionState::AimingWithBowUp);

        assert_eq!(
            command_order_types(&engine),
            vec![
                OrderType::TransitionLoweringBow,
                OrderType::TransitionUnloadBow,
                OrderType::TransitionUnequipBow,
            ],
            "C++ TestBowAimUp expects UnequipBow from bow-up to lower, unload, then unequip"
        );
    }

    #[test]
    fn turn_context_sets_goal_without_snapping_and_books_turning() {
        use crate::sequence::{Field, FieldValue};

        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        let mut turn = SequenceElement::new_generic(1, Command::Turn, Some(owner));
        turn.set_property(Field::Direction, FieldValue::Integer(5));
        let seq_id = engine.orders.sequence_manager.launch_element(turn);

        let barrier = TurnCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
        }
        .dispatch(owner, Command::Turn, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Reach);
        let entity = engine.world.entities.get(owner).unwrap();
        assert_eq!(entity.element_data().direction(), 0);
        assert_eq!(
            u8::from(
                entity
                    .element_data()
                    .sprite
                    .position_iface
                    .get_direction_goal()
            ),
            5,
            "Turn must set the progressive direction goal, not snap direction"
        );
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(
            element.current_order().map(|order| order.order_type),
            Some(OrderType::Turning)
        );
        assert!(element.current_order().unwrap().compute_direction);
    }

    #[test]
    fn wait_timer_context_arms_actor_and_books_upright_idle() {
        use crate::sequence::{Field, FieldValue};

        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let mut wait = SequenceElement::new_generic(1, Command::WaitTimer, Some(owner));
        wait.set_property(Field::Timer, FieldValue::Integer(7));
        let seq_id = engine.orders.sequence_manager.launch_element(wait);

        let barrier = WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WaitTimer, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Reach);
        assert_eq!(
            engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .wait_time,
            7
        );
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(
            element.current_order().map(|order| order.order_type),
            Some(OrderType::WaitingUprightBored)
        );
        assert!(!element.current_order().unwrap().compute_direction);
    }

    #[test]
    fn frozen_all_wait_timer_still_completes_in_owner_slot() {
        use crate::sequence::{Field, FieldValue};

        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let mut wait = SequenceElement::new_generic(1, Command::WaitTimer, Some(owner));
        wait.set_property(Field::Timer, FieldValue::Integer(0));
        let seq_id = engine.orders.sequence_manager.launch_element(wait);
        WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WaitTimer, seq_id, 0);
        let _ = engine
            .orders
            .sequence_manager
            .take_pending_synchronous_actions();
        engine.set_actors_frozen(true);

        engine.tick_actor_animation_action_change_slots(&crate::sim_rng::test_context(), &assets);

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .expect("wait timer remains inspectable")
                .state,
            SequenceState::Terminated
        );
    }

    #[test]
    fn npc_state_context_preserves_menace_order_and_reaches_splice_barrier() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        let seq_id = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::StartMenace, Some(owner)));

        let barrier = NpcStateCommandContext {
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
        }
        .dispatch(Command::StartMenace, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Reach);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::Terminated);
        assert_eq!(
            element
                .orders
                .iter()
                .map(|order| order.order_type)
                .collect::<Vec<_>>(),
            vec![
                OrderType::TransitionRaisingSword,
                OrderType::TransitionWaitingSwordMenacing,
            ]
        );
        assert!(element.orders.iter().all(|order| !order.compute_direction));
    }

    #[test]
    fn npc_attention_context_uses_alerted_look_and_reaches_splice_barrier() {
        let mut engine = EngineInner::new();
        let mut soldier_entity = make_bow_soldier(Posture::Upright, ActionState::Waiting);
        let Entity::Soldier(soldier) = &mut soldier_entity else {
            unreachable!();
        };
        soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
        let owner = engine.add_entity(soldier_entity);
        engine
            .world
            .entities
            .get_mut(owner)
            .and_then(Entity::enemy_ai_mut)
            .expect("test soldier has enemy AI")
            .attentive = true;
        let seq_id = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::LookLeft, Some(owner)));

        let barrier = NpcAttentionCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
        }
        .dispatch(owner, Command::LookLeft, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Reach);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(
            element.current_order().map(|order| order.order_type),
            Some(OrderType::LookingLeftAlerted)
        );
        assert!(!element.current_order().unwrap().compute_direction);
    }

    #[test]
    fn stealth_context_crouches_and_preserves_terminated_order() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let seq_id = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::CrouchDown, Some(owner)));

        let barrier = StealthCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            titbit_manager: &mut engine.feedback.titbit_manager,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::CrouchDown, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Reach);
        let entity = engine.world.entities.get(owner).unwrap();
        assert_eq!(entity.element_data().posture, Posture::Crouched);
        assert_eq!(
            entity.actor_data().unwrap().action_state,
            ActionState::Waiting
        );
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::Terminated);
        assert_eq!(
            element.current_order().map(|order| order.order_type),
            Some(OrderType::TransitionCrouchingDown),
            "the pre-split path terminates synchronously but retains the transition order"
        );
    }

    #[test]
    #[should_panic(expected = "WAIT_TIMER owner")]
    fn wait_timer_context_rejects_missing_timer_contextually() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let wait = SequenceElement::new_generic(1, Command::WaitTimer, Some(owner));
        let seq_id = engine.orders.sequence_manager.launch_element(wait);

        WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WaitTimer, seq_id, 0);
    }

    #[test]
    #[should_panic(expected = "Wait translation owner")]
    fn wait_context_rejects_stale_owner_contextually() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let wait = SequenceElement::new(1, Command::Wait, Some(owner));
        let seq_id = engine.orders.sequence_manager.launch_element(wait);
        engine.remove_entity(owner);

        WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::Wait, seq_id, 0);
    }

    #[test]
    fn stealth_termination_splices_timer_successor_before_same_tick_scan() {
        use crate::sequence::{Field, FieldValue, Sequence};

        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let mut sequence = Sequence::new();
        sequence.append_element(SequenceElement::new(1, Command::CrouchDown, Some(owner)));
        let mut timer = SequenceElement::new_generic(2, Command::Timer, None);
        timer.set_property(Field::Timer, FieldValue::Integer(2));
        sequence.append_element(timer);
        engine.orders.sequence_manager.launch_sequence(sequence);

        let mut display = HostDisplayState::default();
        let mut dev = DevState::default();
        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.perform_hourglass(&mut display, &assets, &mut dev);

        assert_eq!(engine.orders.timer_elements.len(), 1);
        assert_eq!(
            engine.orders.timer_elements[0].remaining, 1,
            "the stealth context must reach the synchronous splice before the timer scan"
        );
    }

    #[test]
    fn direct_ability_context_starts_whistle_and_reaches_splice_barrier() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let seq_id = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::WhistleCmd, Some(owner)));

        let barrier = DirectAbilityCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WhistleCmd, true, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Reach);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::InProgress);
        assert_eq!(
            element.current_order().map(|order| order.order_type),
            Some(OrderType::Whistling)
        );
        assert_eq!(
            engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .whistle_wait_time,
            25
        );
    }

    #[test]
    fn direct_ability_context_preserves_eat_no_ammo_skip_barrier() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let seq_id = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::EatCmd, Some(owner)));

        let barrier = DirectAbilityCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::EatCmd, false, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Skip);
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(element.state, SequenceState::Terminated);
        assert!(element.orders.is_empty());
    }

    #[test]
    fn direct_ability_context_preserves_missing_throw_target_skip_barrier() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_aiming_pc(ActionState::Waiting));
        let seq_id = engine
            .orders
            .sequence_manager
            .launch_element(SequenceElement::new(1, Command::ThrowApple, Some(owner)));

        let barrier = DirectAbilityCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::ThrowApple, true, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Skip);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .state,
            SequenceState::Impossible
        );
    }

    #[test]
    fn position_assertion_context_interrupts_at_tolerance_boundary() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        let mut assertion = SequenceElement::new_movement(
            1,
            Command::AssertPosition,
            Some(owner),
            OrderType::WalkingUpright,
        );
        if let crate::sequence::SequenceElementData::Movement {
            destination,
            tolerance,
            ..
        } = &mut assertion.data
        {
            *destination = crate::coordinates::MapPoint::new(5.0, 0.0);
            *tolerance = 0.0;
        }
        let seq_id = engine.orders.sequence_manager.launch_element(assertion);

        let barrier = PositionAssertionContext {
            entities: &engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
        }
        .dispatch(owner, seq_id, 0);

        assert_eq!(barrier, OwnerActionBarrier::Reach);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .state,
            SequenceState::Interrupted,
            "RHelementactor.cpp uses >= tolerance + 5 for the max-norm mismatch"
        );
    }

    #[test]
    fn lift_wait_context_keeps_blocked_lift_in_progress_and_reaches_splice() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        let sector_number = crate::sector::SectorNumber::new(42);
        install_test_lift_sector(&mut engine, owner, sector_number);
        engine.world.fast_grid.lift_state_mut(0).wait_time = 2;
        let door = crate::gate::Door {
            door_type: crate::gate::DoorType::LiftHigh,
            sector_in: sector_number,
            ..crate::gate::Door::default()
        };
        let mut wait = SequenceElement::new_movement(
            1,
            Command::WaitFreeLift,
            Some(owner),
            OrderType::WalkingUpright,
        );
        if let crate::sequence::SequenceElementData::Movement {
            gate_id, sector, ..
        } = &mut wait.data
        {
            *gate_id = Some(crate::gate::DoorIndex(0));
            *sector = crate::position_interface::SectorHandle::new(42);
        }
        let seq_id = engine.orders.sequence_manager.launch_element(wait);

        WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WaitFreeLift, seq_id, 0);

        let authorized = LiftWaitCommandContext {
            entities: &mut engine.world.entities,
            fast_grid: &mut engine.world.fast_grid,
            doors: std::slice::from_ref(&door),
            sequence_manager: &mut engine.orders.sequence_manager,
        }
        .authorize_and_reserve(owner, seq_id, 0);

        assert!(!authorized);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .state,
            SequenceState::InProgress
        );
        assert_eq!(engine.world.fast_grid.lift_state_mut(0).wait_time, 1);
        assert!(
            engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .actor_data()
                .unwrap()
                .active_lift
                .is_none()
        );
    }

    #[test]
    #[should_panic(expected = "must be LiftHigh or LiftLow")]
    fn lift_wait_context_rejects_crenel_lift_type_contextually() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        let sector_number = crate::sector::SectorNumber::new(42);
        install_test_lift_sector(&mut engine, owner, sector_number);
        let door = crate::gate::Door {
            door_type: crate::gate::DoorType::LiftHighCrenel,
            sector_in: sector_number,
            ..crate::gate::Door::default()
        };
        let mut wait = SequenceElement::new_movement(
            1,
            Command::WaitFreeLift,
            Some(owner),
            OrderType::WalkingUpright,
        );
        if let crate::sequence::SequenceElementData::Movement {
            gate_id, sector, ..
        } = &mut wait.data
        {
            *gate_id = Some(crate::gate::DoorIndex(0));
            *sector = crate::position_interface::SectorHandle::new(42);
        }
        let seq_id = engine.orders.sequence_manager.launch_element(wait);
        WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WaitFreeLift, seq_id, 0);

        LiftWaitCommandContext {
            entities: &mut engine.world.entities,
            fast_grid: &mut engine.world.fast_grid,
            doors: std::slice::from_ref(&door),
            sequence_manager: &mut engine.orders.sequence_manager,
        }
        .authorize_and_reserve(owner, seq_id, 0);
    }

    #[test]
    fn lift_wait_context_reserves_direction_before_terminating() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        let sector_number = crate::sector::SectorNumber::new(42);
        install_test_lift_sector(&mut engine, owner, sector_number);
        let door = crate::gate::Door {
            door_type: crate::gate::DoorType::LiftHigh,
            sector_in: sector_number,
            ..crate::gate::Door::default()
        };
        let mut wait = SequenceElement::new_movement(
            1,
            Command::WaitFreeLift,
            Some(owner),
            OrderType::WalkingUpright,
        );
        if let crate::sequence::SequenceElementData::Movement {
            gate_id, sector, ..
        } = &mut wait.data
        {
            *gate_id = Some(crate::gate::DoorIndex(0));
            *sector = crate::position_interface::SectorHandle::new(42);
        }
        let seq_id = engine.orders.sequence_manager.launch_element(wait);

        WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WaitFreeLift, seq_id, 0);

        let authorized = LiftWaitCommandContext {
            entities: &mut engine.world.entities,
            fast_grid: &mut engine.world.fast_grid,
            doors: std::slice::from_ref(&door),
            sequence_manager: &mut engine.orders.sequence_manager,
        }
        .authorize_and_reserve(owner, seq_id, 0);

        assert!(authorized);
        engine.do_next_order(seq_id, 0);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .unwrap()
                .state,
            SequenceState::Terminated
        );
        let lift = engine.world.fast_grid.lift_state_mut(0);
        assert_eq!(lift.occupants, 1);
        assert!(lift.occupied_downwards);
        assert_eq!(lift.wait_time, 100);
        let active_lift = engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap()
            .active_lift
            .expect("authorized actor records its active lift");
        assert_eq!(active_lift.sector_number, 42);
        assert!(!active_lift.upwards);
    }

    #[test]
    fn frozen_all_lift_wait_rechecks_and_promotes_successor_in_authorizing_slot() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let owner = engine.add_entity(make_bow_soldier(Posture::Upright, ActionState::Waiting));
        let sector_number = crate::sector::SectorNumber::new(42);
        install_test_lift_sector(&mut engine, owner, sector_number);
        engine.world.fast_grid.lift_state_mut(0).wait_time = 2;
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                door_type: crate::gate::DoorType::LiftHigh,
                sector_in: sector_number,
                ..crate::gate::Door::default()
            });
        let mut wait = SequenceElement::new_movement(
            1,
            Command::WaitFreeLift,
            Some(owner),
            OrderType::WalkingUpright,
        );
        if let crate::sequence::SequenceElementData::Movement {
            gate_id, sector, ..
        } = &mut wait.data
        {
            *gate_id = Some(crate::gate::DoorIndex(0));
            *sector = crate::position_interface::SectorHandle::new(42);
        }
        let seq_id = engine.orders.sequence_manager.launch_element(wait);
        WaitCommandContext {
            entities: &mut engine.world.entities,
            sequence_manager: &mut engine.orders.sequence_manager,
            next_order_id: &mut engine.orders.next_order_id,
            profiles: &assets.profile_manager,
        }
        .dispatch(owner, Command::WaitFreeLift, seq_id, 0);
        engine.set_actors_frozen(true);
        let _ = engine
            .orders
            .sequence_manager
            .take_pending_synchronous_actions();
        let sim = crate::sim_rng::test_context();

        engine.tick_actor_animation_action_change_slots(&sim, &assets);
        assert_eq!(engine.world.fast_grid.lift_state_mut(0).wait_time, 1);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .expect("blocked lift wait remains installed")
                .state,
            SequenceState::InProgress
        );

        engine.tick_actor_animation_action_change_slots(&sim, &assets);
        assert_eq!(engine.world.fast_grid.lift_state_mut(0).wait_time, 0);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .expect("zeroed lift wait remains installed")
                .state,
            SequenceState::InProgress,
            "authorization returns false on the frame that decrements the cooldown to zero"
        );

        engine.tick_actor_animation_action_change_slots(&sim, &assets);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(seq_id, 0)
                .expect("authorized lift wait remains inspectable")
                .state,
            SequenceState::Terminated
        );
        let lift = engine.world.fast_grid.lift_state_mut(0);
        assert_eq!(lift.occupants, 1);
        assert!(lift.occupied_downwards);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_element_for_actor(owner)
                .and_then(|(sequence, element)| engine
                    .orders
                    .sequence_manager
                    .get_element(sequence, element))
                .map(|element| element.command),
            Some(Command::Wait),
            "DoNext/Wait translation must finish in the same owner slot"
        );
    }
}

#[cfg(test)]
mod soldier_take_drink_parity_tests {
    use super::*;
    use crate::coordinates::WorldPoint3D;
    use crate::element::{
        ActorData, ActorSoldier, ElementBonus, ElementData, ElementKind, ElementProjectile,
        HumanData, NpcData, ObjectData, ObjectType, Posture, ProjectileData, SoldierData,
    };
    use crate::sequence::SequenceElement;

    fn make_soldier_at(x: f32, y: f32) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position(WorldPoint3D { x, y, z: 0.0 });
        element.set_position_map(crate::coordinates::MapPoint { x, y });
        element.set_direction_instantly(0);
        Entity::Soldier(ActorSoldier {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        })
    }

    fn make_pc_at(x: f32, y: f32) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position(WorldPoint3D { x, y, z: 0.0 });
        element.set_position_map(crate::coordinates::MapPoint { x, y });
        Entity::Soldier(ActorSoldier {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData::default(),
        })
    }

    fn make_projectile_object_at(object_type: ObjectType, x: f32, y: f32) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            ..ElementData::default()
        };
        element.set_position(WorldPoint3D { x, y, z: 0.0 });
        element.set_position_map(crate::coordinates::MapPoint { x, y });
        Entity::Projectile(ElementProjectile {
            element,
            object: ObjectData {
                object_type,
                ..ObjectData::default()
            },
            projectile: ProjectileData::default(),
        })
    }

    fn make_bonus_object_at(object_type: ObjectType, x: f32, y: f32) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ObjectBonus,
            active: true,
            ..ElementData::default()
        };
        element.set_position(WorldPoint3D { x, y, z: 0.0 });
        element.set_position_map(crate::coordinates::MapPoint { x, y });
        Entity::Bonus(ElementBonus {
            element,
            object: ObjectData {
                object_type,
                ..ObjectData::default()
            },
        })
    }

    fn launch_interaction_and_tick(
        command: Command,
        actor: Entity,
        antagonist: Entity,
    ) -> (EngineInner, EntityId) {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let actor_id = engine.add_entity(actor);
        let antagonist_id = engine.add_entity(antagonist);
        engine.launch_element(SequenceElement::new_interaction(
            1,
            command,
            Some(actor_id),
            Some(antagonist_id),
        ));

        let mut dev = DevState::default();
        let mut display = HostDisplayState::default();
        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.perform_hourglass(&mut display, &assets, &mut dev);
        assert_eq!(
            engine
                .get_entity(actor_id)
                .expect("interaction actor present")
                .element_data()
                .direction(),
            0,
            "the sequence-manager dispatch follows the entity loop, so its new order cannot turn the actor on the launch frame"
        );
        engine.perform_hourglass(&mut display, &assets, &mut dev);
        (engine, actor_id)
    }

    #[test]
    fn soldier_taking_sets_goal_and_turns_toward_antagonist() {
        let (engine, actor_id) = launch_interaction_and_tick(
            Command::Take,
            make_soldier_at(0.0, 0.0),
            make_projectile_object_at(ObjectType::Purse, 10.0, 0.0),
        );

        let actor = engine.get_entity(actor_id).unwrap();
        assert_eq!(actor.element_data().direction(), 1);
    }

    #[test]
    fn soldier_drinking_ale_sets_goal_and_turns_toward_antagonist() {
        let (engine, actor_id) = launch_interaction_and_tick(
            Command::DrinkAle,
            make_soldier_at(0.0, 0.0),
            make_bonus_object_at(ObjectType::Ale, 100.0, 0.0),
        );

        let actor = engine.get_entity(actor_id).unwrap();
        assert_eq!(actor.element_data().direction(), 1);
    }

    #[test]
    fn nearby_pc_does_not_pick_up_bonus_without_take_command() {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        engine.add_entity(make_pc_at(100.0, 100.0));
        let bonus_id =
            engine.add_entity(make_bonus_object_at(ObjectType::BonusPurse, 100.0, 100.0));

        let mut dev = DevState::default();
        let mut display = HostDisplayState::default();
        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        engine.perform_hourglass(&mut display, &assets, &mut dev);

        let bonus = engine.get_entity(bonus_id).unwrap();
        assert!(bonus.element_data().active);
        assert!(!bonus.object_data().unwrap().taken);
    }
}

#[cfg(test)]
mod drop_ammo_merge_tests {
    use super::*;
    use crate::campaign::{Campaign, PcDescription};
    use crate::element::{ActorPc, ElementData, ElementKind, EntityId, Posture};
    use crate::profiles::{Action, CharacterProfileIdx};
    use crate::sequence::{Field, FieldValue, SequenceElement};

    fn count_bonuses(engine: &EngineInner, action: Action) -> Vec<(EntityId, u16)> {
        engine
            .world
            .entities
            .bonuses()
            .filter_map(|(entity_id, bonus)| {
                if bonus.element.active && bonus.object.associated_action == action {
                    Some((entity_id.into(), bonus.object.quantity))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Build an engine with one PC at the origin, a campaign with one
    /// PcDescription whose status starts with `bow_ammo` arrows, and a
    /// move-box that lets `find_authorized_position_toward` return a
    /// valid drop position on the empty FastFindGrid.
    fn build_engine_with_pc(bow_ammo: u16) -> (EngineInner, EntityId, LevelAssets) {
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        let pm = std::sync::Arc::make_mut(&mut assets.profile_manager);
        pm.characters.push(crate::profiles::CharacterProfile {
            index: 0,
            filename: "TEST_PC".into(),
            profile_name: "TEST".into(),
            ..Default::default()
        });

        let mut campaign = Campaign::default();
        let mut desc = PcDescription {
            character_profile_idx: Some(CharacterProfileIdx(0)),
            ..Default::default()
        };
        desc.status.set_ammo(Action::Bow, bow_ammo);
        campaign.characters.push(desc);
        engine.mission_domain.campaign = campaign;

        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position_map(crate::coordinates::MapPoint { x: 100.0, y: 100.0 });
        element.set_direction_instantly(0);
        // Seed a non-empty move box so try_get_drop_position's
        // is_somewhere check passes.  The exact dims don't matter on
        // an empty grid.
        element
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                crate::coordinates::MapVec::new(-5.0, -5.0),
                crate::coordinates::MapVec::new(5.0, 5.0),
            ));

        let pc_id = engine.add_entity(crate::element::Entity::Pc(ActorPc {
            element,
            actor: Default::default(),
            human: Default::default(),
            pc: crate::element::PcData {
                profile_index: CharacterProfileIdx(0),
                ..Default::default()
            },
        }));

        super::complete_test_runtime_fixture(&mut engine, &mut assets);
        (engine, pc_id, assets)
    }

    fn drop_ammo_and_tick(
        engine: &mut EngineInner,
        pc_id: EntityId,
        amount: u32,
        assets: &LevelAssets,
    ) {
        let mut elem =
            SequenceElement::new_generic(1, crate::element::Command::DropAmmo, Some(pc_id));
        elem.set_property(Field::ActionId, FieldValue::Integer(Action::Bow as u32));
        elem.set_property(Field::Amount, FieldValue::Integer(amount));
        engine.launch_element(elem);

        let mut display = HostDisplayState::default();
        let mut dev = DevState::default();
        engine.perform_hourglass(&mut display, assets, &mut dev);
    }

    #[test]
    fn three_drops_at_same_position_merge_into_one_pile() {
        let (mut engine, pc_id, assets) = build_engine_with_pc(/* bow_ammo */ 10);

        drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);
        drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);
        drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);

        let bonuses = count_bonuses(&engine, Action::Bow);
        assert_eq!(
            bonuses.len(),
            1,
            "three same-position drops should leave one merged pile, got {bonuses:?}"
        );
        assert_eq!(bonuses[0].1, 3, "merged quantity");

        // last_dropped_ammo should point at the surviving pile.
        let pc = engine.get_entity(pc_id).unwrap();
        let pc_data = match pc {
            crate::element::Entity::Pc(p) => &p.pc,
            _ => unreachable!(),
        };
        assert_eq!(pc_data.last_dropped_ammo, Some(bonuses[0].0));
        assert_eq!(pc_data.last_ammo_dropping_position.x, 100.0);
    }

    #[test]
    fn drop_over_pile_cap_spawns_fresh_and_bumps_facing() {
        let (mut engine, pc_id, assets) = build_engine_with_pc(20);

        // Fill a pile to the cap (5).
        for _ in 0..5 {
            drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);
        }
        let bonuses = count_bonuses(&engine, Action::Bow);
        assert_eq!(bonuses.len(), 1, "five drops merge into one pile");
        assert_eq!(bonuses[0].1, 5, "pile capped at 5");

        let dir_before = engine.get_entity(pc_id).unwrap().element_data().direction();

        // Sixth drop overflows the cap → new pile, facing rotates +1.
        drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);

        let bonuses = count_bonuses(&engine, Action::Bow);
        assert_eq!(
            bonuses.len(),
            2,
            "cap-overflow drop should spawn a fresh pile, got {bonuses:?}"
        );
        // The fresh pile is the one with quantity 1.
        let fresh_qty = bonuses.iter().find(|(_, q)| *q == 1).map(|(_, q)| *q);
        assert_eq!(fresh_qty, Some(1));

        let dir_after = engine.get_entity(pc_id).unwrap().element_data().direction();
        assert_eq!(
            dir_after,
            (dir_before + 1).rem_euclid(16),
            "PC facing should rotate +1 sector on cap overflow"
        );
    }

    #[test]
    fn moving_between_drops_breaks_merge() {
        let (mut engine, pc_id, assets) = build_engine_with_pc(10);

        drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);

        // Teleport the PC sideways before the second drop — same as
        // walking off the original tile.
        if let Some(entity) = engine.world.entities.get_mut(pc_id) {
            entity
                .element_data_mut()
                .set_position_map(crate::coordinates::MapPoint { x: 200.0, y: 200.0 });
        }

        drop_ammo_and_tick(&mut engine, pc_id, 1, &assets);

        let bonuses = count_bonuses(&engine, Action::Bow);
        assert_eq!(
            bonuses.len(),
            2,
            "moving between drops invalidates the merge gate, got {bonuses:?}"
        );
    }
}
